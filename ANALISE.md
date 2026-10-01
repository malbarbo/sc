# sc: análise de um engine C baseado em clang/LLVM

Data: 2026-09-29. LLVM no pin do `rustports/llvm/upstream`
(`5fe235b9862f`, 23.0.0git). Toolchain de build: wasi-sdk 29 (clang 21).
Tudo o que está aqui foi medido; o build está descrito no `README.md` e os caminhos antigos em §9.

## 1. Resumo

- **Dá para rodar clang + wasm-ld inteiros como um único módulo
  `wasm32-wasip1`**, sem threads, sem processos, sem sinais. Compilei, rodei
  no wasmtime e no V8: compila C com `<stdio.h>`, linka com a wasi-libc e o
  programa roda. Diagnósticos idênticos ao clang nativo.
- O LLVM **não tem suporte upstream** para hospedar em WASI. Há um patch
  pronto (YoWASP, PR #92677, aberto desde 2024, nunca mergeado):
  ~170 linhas em 25 arquivos, quase tudo `#ifdef __wasi__` em
  `llvm/lib/Support`. Portei para o pin com 4 ajustes triviais.
- **Tamanho, só com opções de CMake** (1 alvo, sem analyzer, sem asserts,
  MinSizeRel): `clang.wasm` **46,8 MB** (16,6 MB gzip). Protótipo
  `sc.wasm` = cc1 + wasm-ld num módulo só: **46,0 MB** (16,3 MB gzip).
  Para comparação: spython 18,6 MB (7,4 MB gzip), sgleam 3,4 MB (1,4 MB).
- **C++ pesa pouco.** Cortar o que é só-C++ economiza **1,9 MB (4 %)**
  verificado, no máximo 3,2 MB. O C++ está entrelaçado no código comum
  (`SemaExpr`, `ExprConstant`, `CodeGenModule`...), e esse entrelaçamento
  não sai sem reescrever o clang. O que pesa é o **pipeline de otimização do
  LLVM** (−6,7 MB), os **outros alvos embutidos no clang** (−3,9 MB) e o
  **ferramental de módulos/PCH/dumpers** (−3,6 MB). Nenhum desses tem opção
  de CMake.
- **Com todos os cortes juntos**, verificados contra um corpus de C:
  `sc.wasm` (cc1 + wasm-ld) **26,9 MB, 9,8 MB gzip, 7,6 MB zstd**. Ele
  compila e linka dentro do próprio módulo, e os programas rodam. Com um
  patch de fonte que troque construtores por stubs, o teto é **22,9 MB
  (8,4 MB gzip)**. O que sobra é o frontend C (27 %), o backend do LLVM
  (27 %) e o IR/Analysis (17 %): é o piso sem reescrever o clang.
- **Nada disso exige editar o clang.** Os cortes são por objeto, na hora do
  link, com stubs gerados automaticamente. A única edição de fonte fora do
  patch WASI foi 1 linha no lld.
- Desempenho no V8: carregar o módulo ~0,45 s; primeira compilação ~1,1 s;
  depois **~0,23 s por programa** de aluno. Aceitável para "rodar" e até
  para um REPL.
- **Rodada 2 (§10): 15,75 MB, 5,8 MB gzip, 4,6 MB zstd**, idêntico ao clang
  (diagnósticos, bytes do objeto e execução) em um corpus de **7.462
  arquivos C** (testes do clang, gcc torture, c-testsuite). Menor que o
  spython. O corpus maior mostrou que o número da rodada 1 era otimista
  (o honesto era 28,6 MB). O ganho veio de três coisas: invocação e backend
  próprios no `sc_main` (o parser de opções e o `PassBuilder` saem pela
  raiz), cortes por **símbolo** com `--wrap` (não só por objeto) e tabelas
  tablegen reduzidas ao alvo wasm.

## 2. Build: o que já existe como opção

Opções que **já existem** e foram usadas (nenhuma exige mudar código):

| opção | efeito |
| --- | --- |
| `LLVM_TARGETS_TO_BUILD=WebAssembly` | só o backend wasm do **LLVM** |
| `LLVM_DEFAULT_TARGET_TRIPLE=wasm32-wasip1` | host = alvo |
| `CMAKE_BUILD_TYPE=MinSizeRel`, `LLVM_ENABLE_ASSERTIONS=OFF` | `-Os`, sem asserts |
| `LLVM_ENABLE_THREADS=OFF`, `LLVM_ENABLE_PIC=OFF`, `LLVM_BUILD_STATIC=ON` | exigidos por WASI |
| `CLANG_ENABLE_STATIC_ANALYZER=OFF`, `CLANG_ENABLE_OBJC_REWRITER=OFF` | tira o analyzer |
| `CLANG_PLUGIN_SUPPORT=OFF`, `LLVM_ENABLE_PLUGINS=OFF` | sem `dlopen` |
| `LLVM_ENABLE_{ZLIB,ZSTD,LIBXML2,LIBEDIT}=OFF`, `LLVM_ENABLE_BACKTRACES=OFF` | sem deps |
| `-Wl,-z,stack-size=8388608`, `-Wl,--max-memory=4294967296` | pilha de 8 MB (o parser é recursivo) |

O que **não existe** como opção (e é onde está o peso):

- `LLVM_TARGETS_TO_BUILD` não afeta o **clang**: `clang/lib/Basic/Targets/*`,
  `clang/lib/CodeGen/Targets/*` e `TargetBuiltins/*`, `Sema{ARM,X86,RISCV,...}`
  e `Driver/ToolChains/*` são sempre compilados e linkados (fábricas com
  `switch` sobre o triple).
- Não há como desligar C++, Objective-C, OpenMP, OpenACC, CUDA/HIP, HLSL,
  SYCL ou OpenCL no clang. É uma linguagem só com `LangOptions`, decidida em
  tempo de execução; o `Sema` constrói `SemaOpenMP`, `SemaObjC`, `SemaCUDA`,
  `SemaHLSL`... incondicionalmente no construtor.
- Não há como desligar passes de otimização: o `PassBuilder` (usado por
  `clang/lib/CodeGen/BackendUtil.cpp` mesmo em `-O0`) referencia todos os
  passes via `PassRegistry.def`, então todos são linkados.
- O `lld` sempre linka ELF, COFF, MachO, MinGW e Wasm; e o `lldWasm` puxa
  `LLVMLTO` (que puxa o pipeline de otimização inteiro) para aceitar bitcode.

## 3. Hospedar em WASI: o patch

`build/wasi-host.patch` é o commit do YoWASP (`llvmorg-22.1.0+wasm`).
Aplicado no pin 23git precisou de:

- 2 hunks refeitos à mão (`InitLLVM.cpp`, `Process.inc`: contexto mudou);
- `Signals.cpp` e `CrashRecoveryContext.cpp`: assinaturas ganharam o
  parâmetro `NeedsPOSIXUtilitySignalHandling`;
- `clang/tools/driver/driver.cpp`: código novo que usa `raise`/`<signal.h>`;
- `clang/lib/AST/ByteCode/Interp.cpp`: o interpretador constexpr novo usa
  `musttail`, que exige `-mtail-call`; desliguei `USE_TAILCALLS` em wasm
  sem tail-call.

Total: 25 arquivos, +169/−33. O custo de manutenção é rebasear isso a cada
bump do LLVM (o YoWASP fez isso de 18 a 22; o repositório foi arquivado em
2026-03).

Comportamentos em WASI observados:

- Escrever o `.o` em arquivo falha (`IO failure on output stream`): o LLVM
  escreve num `.tmp` e renomeia. Com `-o -` funciona. No engine a saída vai
  para memória (`raw_svector_ostream`), então não importa.
- Sem `fork`/`exec`: o driver precisa rodar o cc1 em processo (é o padrão,
  `CLANG_SPAWN_CC1=OFF`). No engine, chamo `cc1_main` direto (§5).
- O wasm-ld do wasi-sdk roda `wasm-opt` sozinho quando o link recebe `-O`
  e encontra `wasm-opt` no `PATH`; isso apaga a seção `name`. Armadilha de
  build, não de runtime.

## 4. Quanto pesa cada parte

### 4.1 Atribuição por arquivo (limite superior)

Mapa do linker (`-Map`), código+dados, agrupado pelo nome do arquivo. É
limite superior: o código compartilhado chama esses arquivos, então nem tudo
sai.

| categoria | nativo x86-64 | wasm |
| --- | ---: | ---: |
| clang: núcleo C (Lex/Parse/Sema/AST/CodeGen/Frontend/Basic) | 18,4 % | 17,8 % |
| llvm: backend (CodeGen/SelectionDAG/MC/wasm) | 15,3 % | 15,9 % |
| llvm: otimizações IR (Vectorize/IPO/Scalar/InstCombine/Instrumentation...) | 15,4 % | 15,5 % |
| llvm: IR/Analysis/TransformUtils | 12,3 % | 12,3 % |
| clang: outros alvos (Basic/CodeGen/Sema/Driver) | 8,0 % | 7,3 % |
| clang: **C++** (Sema/Parse/AST/CodeGen só-C++) | **6,4 %** | **6,3 %** |
| clang: módulos/PCH/APINotes/ExtractAPI | 5,0 % | 5,4 % |
| clang: OpenMP/OpenACC/CUDA/HLSL/SYCL/OpenCL | 4,3 % | 4,3 % |
| clang: dumpers/code-complete/rewrite | 2,7 % | 2,8 % |
| clang: interpretador constexpr novo | 2,6 % | 2,8 % |
| llvm: Support/TargetParser/Option | 2,5 % | 2,3 % |
| clang: ObjC/Blocks | 1,2 % | 1,2 % |
| llvm: GlobalISel (wasm não usa) | 0,9 % | 1,1 % |
| clang: análises extras (lifetime, unsafe-buffer, thread-safety) | 0,9 % | 0,9 % |
| clang: PGO/coverage/debug info | 0,9 % | 1,0 % |

### 4.2 Cortes verificados

Método (`build/scripts/exp2.py`): refaço os `.a` sem os objetos de uma
categoria. Linko uma vez com `--allow-undefined` para descobrir a
assinatura wasm de cada símbolo que ficou faltando, gero um stub `unreachable`
com o nome do símbolo e relinko. Depois compilo o corpus (`build/corpus/`:
5 programas com structs, unions, VLA, varargs, `_Generic`, `_Static_assert`,
bitfields, float/long double, stdio/stdlib/string/math, e 2 programas com
erros e warnings de `-Wall -Wextra`). Comparo diagnósticos, código gerado
executado e saída com o clang completo. Se um stub for chamado, o trap diz
o símbolo; o objeto que o define volta e repito até dar idêntico.

Base: `clang.wasm` 46,78 MB (`wasm-opt -Oz`, sem debug). Tamanhos após
`wasm-opt -Oz`:

| corte (isolado) | objetos removidos (final) / devolvidos | tamanho | ganho verificado | ganho máximo* |
| --- | ---: | ---: | ---: | ---: |
| otimizações IR do LLVM (Vectorize, IPO, Scalar, InstCombine, Instrumentation, Coroutines, ObjCARC) + GlobalISel | 439 / 23 | 40,07 MB | **−6,71 MB** | −7,1 |
| outros alvos (clang Basic/CodeGen/Sema/Driver, exceto WebAssembly) | 235 / 16 | 42,92 MB | **−3,86 MB** | −4,4 |
| módulos/PCH, APINotes, ExtractAPI, dumpers, code-complete, análises extras | 133 / 8(+3) | 43,22 MB | **−3,56 MB** | −4,0 |
| ObjC, OpenMP, OpenACC, CUDA/HIP, HLSL, SYCL, OpenCL | 83 / 14 | 44,66 MB | **−2,12 MB** | −3,7 |
| **C++** (templates, `DeclCXX`, `ExprCXX`, lambdas, coroutines, vtables, mangling MS...) | 31 / 15 | 44,93 MB | **−1,85 MB** | −3,2 |
| driver do clang (o engine chama o cc1 direto) | 76 / 3 | 45,43 MB | **−1,35 MB** | — |
| interpretador constexpr novo (`AST/ByteCode`, só usado com flag experimental) | 26 / 1 | 45,54 MB | **−1,24 MB** | −1,4 |

\* ganho máximo = removendo tudo, antes de devolver objetos (corpus quebra).

Quase todo objeto devolvido é **construtor ou registro**, não lógica:
`Sema::Sema` constrói `SemaOpenMP`, `SemaObjC`, `SemaCUDA`, `SemaHLSL`,
`SemaSYCL`, `SemaX86`, `SemaARM`, ... (16 alvos e 9 linguagens);
`initializeCodeGen`/`initializeGlobalISel` registram passes que nunca rodam;
o subtarget wasm constrói o `LegalizerInfo` do GlobalISel; o `CodeGenModule`
de C constrói `CodeGenVTables` e o `ItaniumVTableContext`. Do lado de C++,
alguns predicados do código comum passam por arquivos de C++
(`Sema::AdjustDeclIfTemplate`, `DiagnoseUnexpandedParameterPack`,
`RequireNonAbstractType`, `ResolveExceptionSpec`,
`PerformImplicitConversion` em `SemaExprCXX`). A lista completa de traps está
em `build/exp/*/loop.log`.

### 4.3 Tudo junto, no formato do engine

Protótipo `build/proto/sc_main.cpp` (30 linhas): um `main` que despacha
`-cc1 ...` para `cc1_main` e `ld ...` para `lld::lldMain` só com o driver
wasm. É um único módulo, sem o driver do clang e sem ELF/COFF/MachO no lld.

| variante | bruto | gzip | zstd -19 |
| --- | ---: | ---: | ---: |
| YoWASP `@yowasp/clang` 22 (clang+lld+ferramentas, com asserts) | 75,5 MB | 22,7 MB | 15,8 MB |
| `clang.wasm` só com opções de CMake | 46,8 MB | 16,6 MB | 12,4 MB |
| `sc.wasm` (cc1 + wasm-ld), sem cortes | 46,0 MB | 16,3 MB | — |
| **`sc.wasm` com todos os cortes verificados** | **26,9 MB** | **9,8 MB** | **7,6 MB** |
| `sc.wasm` teto (cortes sem devolver nada; exige stubs no fonte) | 22,9 MB | 8,4 MB | — |
| *spython hoje* | *18,6 MB* | *7,4 MB* | |
| *sgleam hoje* | *3,4 MB* | *1,4 MB* | |

O cortado compila e linka **dentro dele mesmo** (p1, p2 e p5 do corpus
rodam com a saída certa). Para isso foi preciso 1 linha no lld:
`SymbolTable::compileBitcodeFiles` constrói o `BitcodeCompiler` (LTO) mesmo
sem nenhum bitcode; um `if (ctx.bitcodeFiles.empty()) return;` resolve e
tira o LTO inteiro.

O que resta nos 26,9 MB (proporção pelo mapa do linker):

| parte | % |
| --- | ---: |
| backend LLVM (CodeGen 3,7, SelectionDAG 3,0, wasm 0,9, AsmPrinter, MC, Object) | 27 % |
| núcleo C do clang (Lex/Parse/Sema/AST/CodeGen/Frontend/Basic) | 27 % |
| IR, Analysis, TransformUtils | 17 % |
| C++ que o código comum ainda alcança | 6 % |
| Support, Option, TargetParser | 4 % |
| resto (OpenMP/ObjC/alvos alcançados, dumpers, debug info, libc++) | 19 % |

Cortes de segunda rodada no backend (MIR printer, CodeView, leitores ELF e
MachO, pipeliner, alocadores de registrador não usados em -O0) estão em
`build/exp/all2`. Batem no mesmo padrão: `initializeCodeGen` registra
todos os passes, e cada iteração devolve um objeto. Em 25 iterações não
convergiu: parou em 25,35 MB com o corpus ainda diferente. O ganho real ali
é **menos de 1,5 MB** e só sai com o pipeline de codegen montado à mão
(§7.2).

## 5. Quanto pesa "carregar C++ junto"

Resposta direta: **~2 MB verificados (4 %), no máximo ~3–4 MB.** O parser e
o Sema de C++ não estão "colados" ao de C: C e C++ são o mesmo parser e o
mesmo Sema, com `if (getLangOpts().CPlusPlus)` espalhados. O que dá para
tirar são os arquivos só-C++ (templates, `DeclCXX`, lambdas, coroutines,
conceitos, exceções, mangling MS, vtables). Mesmo assim, 15 deles voltam
porque o código comum chama predicados ou construtores deles.

Separar de verdade (tirar os ramos C++ de `SemaExpr`, `SemaDecl`,
`ExprConstant`, `ParseDecl`, `CodeGenModule`...) seria um fork do clang, e o
ganho estimado é pequeno perto dos cortes baratos. **Não vale.** Os ganhos
grandes, em ordem: otimizações do LLVM, outros alvos, módulos/PCH e
ferramental, extensões (OpenMP/ObjC/...), driver.

## 6. Otimização: -O0 ou "o básico"

`bench/b.c` (8 rainhas ×20, bolha em 6000, `fib(30)`, série harmônica
2·10⁷), no V8 (node 24). Os números têm ruído (máquina ocupada), mas a ordem
é estável:

| código do aluno | tempo |
| --- | ---: |
| -O0 | 280–400 ms |
| -O0 + sroa + simplifycfg + instcombine | ~190 ms |
| -O1 | 170–235 ms |
| -O2 | 110–155 ms |

-O0 fica 1,5–2× mais lento que -O1. Para programa de aluno isso não importa,
e o V8 (TurboFan) otimiza o wasm de qualquer jeito. Se quisermos "o básico",
SROA + InstCombine + EarlyCSE + SimplifyCFG (+ GVN/LICM) custam **~1,1 MB**.
mem2reg sozinho já está em TransformUtils e é de graça. Isso exige montar o
pipeline à mão em vez de usar o `PassBuilder` (§7), o que é bom de qualquer
forma.

## 7. Como faríamos

### 7.1 Build

Um repositório `sc` com:

1. `llvm-project` como submodule no nosso fork: pin + patch WASI + os
   poucos patches de fonte (hoje só 1 linha no lld). É o que o YoWASP fazia.
2. Build do LLVM/clang/lld em dois estágios: tablegen nativo, depois cross
   para `wasm32-wasip1` com wasi-sdk (§2).
3. **Link próprio do engine**, fora do CMake do LLVM. Ele pega as bibliotecas
   objeto (`obj.clangSema` etc.), filtra a lista de objetos cortados
   (`build/exp/specs/all.txt` com as devoluções), gera os stubs
   (link com `--allow-undefined` → lê os imports → `stubs.s`) e linka o
   engine. O script já existe (`exp2.py`); é só torná-lo o build.
4. Os stubs, em vez de `unreachable`, chamam uma função do engine que
   reporta "recurso não suportado pelo sc: <símbolo>". Assim um caminho
   não testado vira erro legível, não crash.

Por que por objeto e não `#ifdef` no fonte: zero conflito ao rebasear o
LLVM. Os cortes vivem numa lista de arquivos, verificada por um corpus.
Patches de fonte só onde o ganho compensa. O maior deles seria construir os
sub-Semas sob demanda (`Sema::OpenMP()` já é um acessor; hoje o construtor
cria todos), o que leva de 26,9 para perto de 23 MB.

### 7.2 Engine

- `repl_new`/`repl_run` em C++ (wasi-sdk), com as exports do `ENGINE.md`.
- Compilação: `CompilerInvocation::CreateFromArgs` + `CompilerInstance` com
  o arquivo do aluno em memória (`InMemoryFileSystem` sobreposto ao VFS),
  cabeçalhos da wasi-libc e os ~20 do resource dir do clang (`stddef.h`,
  `stdarg.h`, `stdbool.h`...) embutidos. Diagnósticos por um
  `DiagnosticConsumer` próprio (sem cor, com a coluna).
- Geração de código sem o `BackendUtil`/`PassBuilder`: `CodeGenerator`
  (`ModuleBuilder`) dá o `llvm::Module`; rodamos nossos passes
  (AlwaysInliner, mem2reg ou o "básico") e o `legacy::PassManager` com
  `addPassesToEmitFile` num `raw_svector_ostream`. Isso tira o `PassBuilder`
  e o registro de todos os passes pela raiz, sem stubs.
- Link: `lld::wasm::link` com `libc.a` reduzida. O programa do aluno é 1
  objeto + libc, e isso é rápido.
- **Execução**: o programa do aluno é outro módulo wasm. Hoje o host não tem
  como o engine pedir para instanciar um módulo (mesma lacuna do sjava,
  `env.wasm_run` em `sjava/PLANO.md`); precisa desse import. Interrupção:
  um passo LLVM nosso insere `call env.check_interrupt` nas arestas de
  retorno de laço e na entrada de função (é o mesmo plano do sjava para o
  jwonder).
- REPL: C não tem REPL natural. O mais barato é o modelo do sgleam: acumular
  declarações e recompilar a cada entrada (0,2 s por compilação permite).

### 7.3 Desempenho medido (V8, node 24)

| etapa | tempo |
| --- | ---: |
| compilar o módulo `sc.wasm` cortado | ~0,19 s (46 MB: ~0,45 s) |
| 1ª compilação de um programa (lazy tier-up) | ~1,0 s |
| compilações seguintes (p1.c, 36 linhas, 10 headers) | **~0,16–0,20 s** |
| clang nativo, mesmo arquivo | ~0,07 s |

## 8. Riscos

- **Patch WASI fora do upstream.** O PR #92677 está aberto há 2 anos e o
  YoWASP foi arquivado. Somos nós que rebaseamos, mas é pequeno e mexe quase
  só em `Support/Unix`.
- **Cortes por objeto dependem do corpus.** Um caminho de C não coberto pode
  cair num stub. Mitigação: corpus grande (c-testsuite, gcc torture
  `execute/`, submissões reais de na-programacao), e stubs que viram
  diagnóstico legível. Precisa rodar a cada bump.
- **Tamanho ainda maior que o spython.** 9,8 MB gzip contra 7,4 MB. Com
  cache HTTP isso é um download só, mas é o maior engine.
- **Memória.** O clang usa pilha funda (8 MB reservados) e algumas dezenas de
  MB de heap por compilação; nada grave para programas de aluno.

## 9. Reprodução

O projeto foi organizado depois da rodada 2; o `README.md` descreve o layout e
o build (`make`, `make test`, `make learn`). As rodadas 1 e 2 foram feitas
com scripts de experimento em `build/`. Os caminhos citados neste documento
correspondem a:

| experimento (`build/`) | agora |
| --- | --- |
| `src/` (pin + patch WASI) | `llvm/wasi-host.patch`, `llvm/no-veclibs.patch`; pin no `Makefile` |
| `Toolchain-WASI-LLVM.cmake` | `llvm/toolchain.cmake.in` |
| `proto2/sc_*.cpp`, `checkinv.cpp` | `engine/`, `test/checkinv.cpp` |
| `scripts/exp2.py` + `exp3.py` | `scripts/link.py` (link determinístico; `--learn` refaz as listas) |
| `scripts/corpus2.py`, `corpuslib.py` | `test/corpus.py`, `test/corpuslib.py` |
| `exp/specs/all5.txt`, `sym2.txt`, `o0init.txt` | `cuts/objects.txt`, `cuts/symbols.txt`, `cuts/o0init.txt` |
| `exp/sym2/unwrapped.json` | `cuts/symbols.used` |
| `trim/`, `.a`/`.obj` trocados em `wasm/` | `scripts/trim.py` gera `build/trim/`; o `link.py` substitui os membros e `wasm/` fica intacto |
| `res/base3/` (referência: protótipo com `cc1_main`) | `build/res/base/` (referência: `build/sc-full.wasm`, o engine sem cortes) |
| `corpus/`, `bench/` | `test/samples/`, `bench/` |

## 10. Rodada 2: medir o que ficou e cortar mais

### 10.1 Corpus de verdade

Os 5 programas da rodada 1 eram pouco. O corpus agora tem **7.462 arquivos**:

- `c-testsuite` (220 programas com saída esperada);
- gcc torture: `execute/` (1.698, linkados e executados) e `compile/` (2.016);
- testes C do próprio clang: `Sema`, `Parser`, `CodeGen`, `Lexer`,
  `Preprocessor`, `C/*`, `Misc`, `Frontend`, `AST` (3.5 mil). Eles exercitam
  quase todos os diagnósticos de C.

`scripts/corpus2.py` compila cada arquivo com o engine sob wasmtime. Programas
executáveis são linkados e rodados; os outros comparam diagnósticos **e o
md5 do objeto**. Uma rodada leva ~3,5 min em 48 núcleos. Normalização em
`scripts/corpuslib.py`: endereços, backtraces e ordem das linhas (a ordem de
alguns diagnósticos depende de mapas indexados por ponteiro).

Com esse corpus, o build "26,9 MB" da rodada 1 falhava em 138 arquivos:

- C11 atômicos (`LowerAtomicPass`);
- `__builtin_object_size` (`LowerConstantIntrinsics`);
- comentários (`RawComment`);
- campos de struct (`SemaAccess`);
- blocks/lambdas, entre outros.

Devolvidos esses 12 objetos, o número honesto da rodada 1 é **28,55 MB
(10,5 MB gzip)**.

### 10.2 Medição do que sobrava

Método: `wasm-opt -Oz -g` para manter os nomes, depois `twiggy` para o
tamanho por função, e o mapa do linker para ligar cada função ao seu
objeto (`scripts/attrib.py`, `scripts/cat2.py`). Dos 22,5 MB de código:

| parte | MB |
| --- | ---: |
| C++/ObjC/OpenMP/outros alvos ainda alcançados (SemaOpenMP 450 KB sozinho) | 2,83 |
| passes de otimização + `PassBuilder` | 1,95 |
| IR textual, bitcode, AutoUpgrade, verifier | 1,07 |
| debug info (clang e LLVM) | 0,98 |
| formatos não-wasm, `CompilerInvocation`, PGO | 1,14 |
| núcleo: SelectionDAG 2,6, Sema C 2,4, IR/Analysis 2,1, CodeGen/MC 1,7, AST 1,3, CodeGen clang 1,3, Lex/Parse 1,2, wasm 0,5, lld 0,3 | ~13,5 |

Dados (4,4 MB):

- nomes de intrínsecos de todos os alvos: 850 KB;
- texto dos diagnósticos: 720 KB;
- tabela de opções do cc1: 430 KB;
- nomes Unicode para `\N{}`: 240 KB.

### 10.3 O que foi feito

1. **`sc_main` v2** (`build/proto2/`):
   - `sc_inv.cpp` monta o `CompilerInvocation` em código (~120 linhas), sem
     `CreateFromArgs` e sem a tabela de opções.
   - `sc_backend.cpp` substitui o `BackendUtil.cpp`: AlwaysInliner mais o
     pipeline de codegen num `legacy::PassManager`. Assim o `PassBuilder`
     (que referencia todos os passes) sai da árvore de chamadas.
   - A equivalência é verificada por `checkinv` (nativo). Ele compara o
     `generateCC1CommandLine()` e o buffer de macros predefinidas das duas
     invocações. A comparação precisou de ~60 campos cujo default da struct
     difere do default do parser (`MathErrno`, `NoCommon`,
     `ShowOptionNames`, `Inlining=OnlyAlwaysInlining`…).
   - Resultado: **0 diferenças** no corpus, inclusive nos bytes dos objetos.
2. **Cortes por símbolo** (`scripts/exp3.py`):
   - `wasm-ld --wrap=X` redireciona `X` para um stub, e o corpo original
     sai no `--gc-sections`. Isso corta *dentro* dos objetos que os cortes
     por objeto precisaram devolver.
   - Ninguém precisa escolher os símbolos um a um. O script roda uma
     **descoberta**: cada `__wrap_X` registra a primeira chamada e repassa
     para `__real_X`. Uma passada do corpus lista tudo o que é usado; o
     resto vira stub com `unreachable`, e uma segunda passada valida.
   - Na rodada 2, 369 das ~6 mil funções candidatas eram usadas.
   - Só entram símbolos fortes (`T`). Instâncias de template (`W`) não
     funcionam com `--wrap` no wasm-ld.
   - Os inicializadores de passes que o pipeline O0 do wasm não usa viram
     no-op (`scripts/o0passes.py` lista os usados a partir de
     `llc -O0 -debug-pass=Arguments`). Sem registro, o construtor e a vtable
     do passe saem.
   - Precisa do wasm-ld 23: o 21 do wasi-sdk 29 dá segfault com `--wrap` +
     `--allow-undefined`.
3. **Tabelas e dados**:
   - `scripts/trim_intrinsics.py` reescreve o `IntrinsicImpl.inc` gerado:
     nomes de intrínsecos de outros alvos viram `""` e as tabelas
     builtin→intrínseco de outros alvos saem. IDs e tabelas indexadas não
     mudam. O objeto cai de 925 KB para 246 KB.
   - `TargetLibraryInfo.cpp` sob `SC_NO_VECLIBS`: as tabelas de
     vetorização (SVML, SLEEF, ArmPL…) tinham 145 KB de inicializador
     estático.
   - `sc_cuts.cpp` define as funções de nomes Unicode retornando "não
     encontrado". `\N{NOME}` passa a dar erro de compilação limpo (3
     testes do clang mudam, de propósito).

### 10.4 Resultados

| etapa | bruto | gzip |
| --- | ---: | ---: |
| rodada 1 com corpus completo (cortes por objeto) | 28,55 MB | 10,47 MB |
| + sc_main v2 (invocação e backend próprios) | 26,46 MB | 9,74 MB |
| + intrínsecos só wasm, TLI sem veclibs, sem nomes Unicode | 25,24 MB | 9,32 MB |
| + cortes por símbolo (sym1) | 16,92 MB | 6,27 MB |
| **+ passes não-O0 sem registro, mais objetos (sym2)** | **15,75 MB** | **5,81 MB** (zstd 4,56 MB) |
| *spython* | *18,6 MB* | *7,4 MB* |

O `sym2` passa nos 7.462 arquivos, e o `ld` interno linka e roda p1, p2 e
p5. No V8, o módulo compila em 0,11 s; a 1ª compilação leva 0,87 s e as
seguintes 0,14–0,18 s.

O que sobra (sym1, código 14,7 MB):

- SelectionDAG 2,6 MB (o fallback do FastISel; não sai);
- Sema C 1,9 MB;
- IR/Analysis 1,65 MB;
- CodeGen/MC 1,4 MB;
- AST 1,2 MB;
- CodeGen clang 1,1 MB;
- Lex/Parse 1,1 MB;
- resíduo C++ 0,9 MB, que é quase todo instância de template (`W`) ou
  realmente chamado.

Nos dados, o maior item é o texto dos diagnósticos (720 KB, ~200 KB gzip).
A partir daqui, cada MB exige mudar o clang de verdade.

### 10.5 Risco novo

Os stubs dão trap em caminhos que o corpus não cobre. Com 7,5 mil arquivos,
incluindo os testes de diagnóstico do clang, a cobertura de C é boa, mas não
é total. Um trap é recuperável: o engine reporta "erro interno: recurso
não suportado pelo sc" e reinstancia o módulo. Para produção:

- adicionar ao corpus as submissões reais de alunos;
- guardar a lista de símbolos cortados junto com o build;
- fazer o stub imprimir o nome do símbolo antes do `unreachable`.

### 10.6 Arquivos

Ver §9: os arquivos da rodada 2 viraram `engine/`, `cuts/`, `scripts/link.py`
e `test/`. Pelo build organizado, `make` dá `build/sc.wasm` com 15.746.919
bytes (659 a mais que o da rodada 2, pela ordem dos arquivos no link), e
`make test` passa nos 7.462 arquivos.

## 11. Quanto custa trazer o C++ de volta (no engine cortado)

A §5 mediu o C++ contra o clang inteiro. Aqui a pergunta é outra: partindo do
`sc.wasm` de 15,75 MB, quanto cresce um engine que também compila C++,
cortado pelo mesmo método (cortes de `cuts/` reaprendidos com um corpus que
inclui C++).

Configuração do C++: gnu++17 (o padrão do clang), `-fno-exceptions` (a
libc++ do wasi-sdk 29 vem sem exceções: com o padrão do driver,
`-fcxx-exceptions`, o link falha em `__cxa_throw`), libc++ e libc++abi do
sysroot. O `sc_inv` escolhe a linguagem pela extensão (`.cpp`, `.cc`,
`.cxx`); o `checkinv` dá SAME para C e para C++.

Corpus: os 7.462 arquivos de C mais 4.270 de C++, sendo 9 programas de aluno
em `test/cxx` (iostream, STL, classes, templates, lambdas, optional/variant,
iomanip, erros e avisos típicos), linkados e executados, e os testes C++ do
clang (`SemaCXX`, `SemaTemplate`, `CodeGenCXX`, `CXX/**`, `Parser`,
`Lexer`, `AST`, `Misc`), só compilados. O `learn` devolveu 11 objetos
(`CGExprCXX`, `CXXInheritance`, `DeclFriend`, `StmtCXX`,
`SemaTemplateDeductionGuide`, `ASTConcept`, `CGCXX`, `CGVTT`, `VTTBuilder`,
`ASTDumper`, `CGHLSLBuiltins`), e as funções usadas passaram de 369 para
1.520.

| engine | bruto | gzip | zstd |
| --- | ---: | ---: | ---: |
| `sc.wasm` (só C) | 15.746.919 | 5.811.230 | 4.564.766 |
| só C, com o ramo C++ no `sc_inv` | 15.747.508 | 5.810.163 | 4.564.972 |
| **C + C++** | **17.800.572** | **6.624.798** | **5.087.490** |
| diferença | **+2,05 MB (+13 %)** | **+0,81 MB (+14 %)** | **+0,52 MB (+11 %)** |

O engine C + C++ passa nos 11.731 arquivos, sem diferença do engine inteiro.

De onde vêm os bytes (código antes do `wasm-opt`, +2,42 MB): Sema 1,75 MB
(`SemaTemplate*` 1,06 MB, `SemaDeclCXX` 260 KB, `SemaConcept` 150 KB,
`SemaOverload` 91 KB, `SemaExprCXX` 89 KB), AST 345 KB, CodeGen 167 KB e
Parse 140 KB. Quase tudo é template.

Uns 200 KB vêm só dos testes do clang: `ASTDumper` e `TextNodeDumper`
(170 KB, por `#pragma clang __debug dump`) e `CGHLSLBuiltins` (33 KB). Dá para
cortá-los pondo esses testes no `IGNORE`, o que deixa o custo perto de 1,85
MB.

Limites da medida: a cobertura é de gnu++17. Um aluno que use
`-std=c++20` (conceitos, ranges, `<=>`, corrotinas) pode cair em stubs, e o
corpus teria de ganhar testes em C++20. Exceções ficaram de fora: exigiriam a
libc++ com exceções e o EH do wasm, e voltariam `CGException` e
`ItaniumCXXABI` inteiros.

Reprodução: `build/exp-cxx` (fontes do engine em `src/`, cortes aprendidos
em `cuts/`), com `CXX=1` no `test/corpus.py` e `SC_CUTS`, `SC_ENGINE`,
`SC_LINKDIR` e `SC_BASE` no `scripts/link.py`.

## 12. C++20 no engine

O C++ ficou no engine principal (`engine/`, `cuts/`, `test/corpus.py` sem
`CXX=1`), e o padrão passou de gnu++17 para gnu++20. As flags são as que o
driver passa com `-std=c++20` para wasm32-wasip1, mais `-fno-exceptions`:
`-std=gnu++20 -fno-implicit-modules -fskip-odr-check-in-gmf`.

O `checkinv` ganhou uma comparação campo a campo das `LangOptions`. Ela pegou
três opções que a volta pela linha de comando não mostra: com C++20, o parser
liga `Modules` (padrão = `CPlusPlusModules`) e `ModulesLocalVisibility`
(implicada por `-fcxx-modules`), e `-fno-implicit-modules` desliga
`ImplicitModules`. O `checkinv` dá SAME para C e para C++.

O corpus ganhou 5 programas em `test/cxx`: conceitos, `requires` e
`consteval` (c10); ranges, views, `span`, `<=>` e inicializadores designados
(c11); `std::format`, corrotinas, `char8_t`, `using enum` e `constinit`
(c12); erros típicos de C++20 (c13); e avisos (c14). Os testes do clang
continuam os mesmos, agora compilados em gnu++20. São 11.736 arquivos.

Com `Modules` ligado, o clang cria o leitor de módulos (`ASTReader`) para
todo arquivo C++, e todos caíam em stub. O `learn` devolveu `ASTReader`,
`ASTReaderDecl` e `ModuleManager` inteiros (+0,67 MB bruto). Uma regra de
função em `cuts/symbols.txt` para esses objetos deixa só o que é chamado. O
`learn` também devolveu `ExprConcepts` e `SemaCoroutine`.

| engine | bruto | gzip | zstd |
| --- | ---: | ---: | ---: |
| C + C++17 (§11) | 17.800.572 | 6.624.798 | 5.087.490 |
| C + C++20, `ASTReader` inteiro | 18.471.846 | 6.844.449 | 5.252.055 |
| **C + C++20** | **17.993.849** | **6.702.950** | **5.144.933** |

O C++20 custa 0,19 MB bruto e 0,06 MB zstd a mais que o C++17. Contra o
engine só de C, a diferença é +2,25 MB bruto, +0,89 MB gzip e +0,58 MB zstd.
As funções usadas (`cuts/symbols.used`) são 1.732. `make test` passa nos
11.736 arquivos.

`-Wall -Wextra` já é fixo no `sc_inv` (`DO.Warnings`) e vale para C++ (até
a §13, que passou os avisos para as flags). Os
avisos são da Sema, que o corte mantém, e a saída de `c9_warn` e `c14_warn20`
é a mesma do engine inteiro: `-Wuninitialized`, `-Wunused-*`,
`-Wreturn-type`, `-Wparentheses`, `-Wsign-compare`, `-Wstring-plus-int`,
`-Wambiguous-reversed-operator`. Não entram em `-Wall -Wextra`:
`-Wimplicit-fallthrough`, `-Wconversion` (`int k = 3.7`) e `-Wshadow`.

Módulos (`import std;`) não funcionam: precisariam dos BMI da libc++ e do
`ASTWriter`, que está cortado.

## 13. Todos os avisos

Até aqui `-Wall -Wextra` era fixo no `sc_inv` e as outras flags `-W` eram
ignoradas. Agora os avisos vêm só das flags `-W` (`DO.Warnings`), como no
clang: sem flag, só os avisos ligados por padrão no clang. O corpus continua
passando `-Wall -Wextra`, e o `checkinv` dá SAME.

O `test/corpus.py` aceita `WFLAGS` (por exemplo `WFLAGS=-Weverything`). Com
`-Weverything`, o engine cortado da §12 diferia do inteiro em 7.378 dos 11.736
arquivos, quase todos por trap. O `learn` com `-Weverything` (contra uma base
do engine inteiro também com `-Weverything`) devolveu objetos inteiros de
quatro grupos de avisos:

| aviso | objetos devolvidos |
| --- | --- |
| `-Wdocumentation` | `Comment`, `CommentLexer`, `CommentParser`, `CommentSema` |
| `-Wthread-safety` | `ThreadSafetyCommon`, `ThreadSafetyTIL` |
| `-Wunsafe-buffer-usage` | `UnsafeBufferUsage` |
| `-Wlifetime-safety` | 8 objetos de `Analysis/LifetimeSafety` |

e cerca de 90 funções, quase todas desses grupos e de `-Wconsumed`
(`cuts/symbols.used` foi de 1.732 para 1.822). Os outros avisos são da Sema e
o código deles já estava no engine.

| engine | bruto | gzip | zstd |
| --- | ---: | ---: | ---: |
| C + C++20, só `-Wall -Wextra` (§12) | 17.993.849 | 6.702.950 | 5.144.933 |
| **C + C++20, todos os avisos** | **18.267.827** | **6.786.231** | **5.204.882** |

Todos os avisos custam +0,27 MB bruto, +0,08 MB gzip e +0,06 MB zstd. O
corpus passa com e sem `-Weverything` (`make test` e `make test-warn`, 0
diferenças nos 11.736 arquivos). O `make learn` agora roda com
`-Weverything`, para que um corte novo não tire o código de nenhum aviso.

Quais avisos ligar por padrão para os alunos fica para depois; o engine não
liga nenhum além dos do clang.

## 14. O corpus linka com o próprio engine

O corpus linkava os programas com o wasm-ld nativo do wasi-sdk, então o `ld`
do engine cortado nunca rodava no teste. Rodando, `sc.wasm ld` dava trap ao
linkar qualquer programa C++: o wasm-ld chama o demangler, e o
`LLVMDemangle` está cortado (`cuts/symbols.txt`). Os programas C linkavam.

O `test/corpus.py` agora linka com `sc.wasm ld`. Com o engine inteiro, o
resultado é o mesmo do wasm-ld nativo nos 239 programas, e o `learn` devolveu
4 funções do demangler (`cuts/symbols.used` foi de 1.821 para 1.825).

O corpus também passou a gravar o objeto em arquivo (`-o /o/x.o`) em vez de
`-o -`, e isso achou um segundo bug, desta vez no `llvm/wasi-host.patch`: no
WASI, `Process::SafelyCloseFileDescriptor` ignorava o retorno de `close` e
devolvia o `errno`. Quando o arquivo tinha um `#include`, o `errno` ainda
guardava o `EINVAL` de um `readlink`, e o `cc1` falhava com "IO failure on
output stream: Invalid argument", deixando um `.o.tmp`. Com `-o -` o erro não
aparecia. O patch agora só devolve o `errno` quando `close` falha.

| engine | bruto | gzip | zstd |
| --- | ---: | ---: | ---: |
| C + C++20, todos os avisos (§13) | 18.267.827 | 6.786.231 | 5.204.882 |
| **+ demangler para o `ld`** | **18.270.532** | **6.787.704** | **5.205.722** |

`make test` e `make test-warn` dão 0 diferenças nos 11.736 arquivos, e o
`checkinv` dá SAME.

O `learn` não terminou em 2 horas: a descoberta não via as funções do
`TextNodeDumper` que o `#pragma clang __debug dump` chama, e o laço as devolvia
uma por iteração, rodando o corpus inteiro a cada vez. A marca `SCWRAP n` que
cada wrapper escreve no stderr caía no meio de uma linha do dump
(`FunctionDeclSCWRAP 3998`) e a regex, ancorada no início da linha, não a via.
Agora a marca começa com uma quebra de linha. A descoberta acha as 1.825
funções, o laço de verificação fecha na primeira rodada, e o `symbols.used`
sai igual ao anterior.

Cada rodada do corpus ainda levava ~13 min, com a máquina quase parada: só
0 a 6 processos do wasmtime vivos de cada vez, para 48 threads. Amostrando as
threads do `corpus.py`, 86% do tempo estava no `glob` que procura o `.o.tmp`
de uma escrita falha: ele lista o diretório de saída, que acumula um `.txt`
por arquivo (11.737 no fim), e roda sob o GIL. Os objetos e programas agora
vão para um subdiretório `o/`, que só tem os que estão em uso.

| | antes | depois |
| --- | ---: | ---: |
| uma rodada do corpus | ~13 min | 2,5–3,5 min |
| `make learn` | > 2 h (57 min só com a marca) | 11,4 min |

O wasmtime em si custa pouco: 480 compilações de um arquivo trivial, 48 por
vez, levam 1 s.

## 15. Objetos compartilhados (PIC) para o REPL

O REPL vai compilar cada entrada como um objeto compartilhado, carregado sobre
as anteriores (`-fPIC -fvisibility=default`, `wasm-ld -shared
--experimental-pic`). O corpus só compilava com `-mrelocation-model static`, então
nada garantia que os cortes mantinham o código de PIC no backend e o `-shared`
no `ld`.

O `sc_inv.cpp` agora aceita `-mrelocation-model pic -pic-level 2 -target-feature
+mutable-globals -fvisibility=default`, as flags que o driver passa para
`-fPIC -fvisibility=default`. Com PIC o padrão de
`-fdirect-access-external-data` muda para falso. O `checkinv` compara as quatro
combinações (C e C++, estático e PIC) e dá SAME.

O corpus ganhou uma fatia `pic-`: os 1.937 programas que são ligados e
executados são compilados também com PIC e ligados com `sc.wasm ld -shared
--experimental-pic --unresolved-symbols=import-dynamic` contra o `libc.so` (e o
`libc++.so`) do wasi-sdk. O resultado são os diagnósticos e o md5 do objeto e
do `.so`. Ainda não são executados: isso fica para o loader.

O engine cortado deu 0 diferenças nos 13.673 arquivos, com e sem
`-Weverything`, sem mudar os cortes: o código de PIC e de `-shared` já
estava todo lá. O tamanho só muda pelo `sc_inv.cpp`:

| engine | bruto | gzip | zstd |
| --- | ---: | ---: | ---: |
| §14 | 18.270.532 | 6.787.704 | 5.205.722 |
| **+ PIC no `sc_inv`** | **18.270.869** | **6.787.594** | **5.205.596** |

Achado à parte: `test/cxx/c12_format.cpp` falha também na compilação estática,
com "Do not know how to promote this operator!". O `sc_backend.cpp` não roda os
passes de corrotinas (CoroEarly, CoroSplit, CoroCleanup) que o pipeline O0 do
LLVM roda, e as intrínsecas `llvm.coro.*` chegam ao backend. O corpus não
pegou porque o engine inteiro, que é a base, usa o mesmo backend.

## 16. Corrotinas no backend

O pipeline O0 do LLVM (`buildO0DefaultPipeline`) termina com o
`buildCoroWrapper`: CoroEarly, CoroSplit (um passe CGSCC), CoroCleanup e
GlobalDCE, rodados só se o módulo declara intrínsecas `llvm.coro.*`
(`CoroConditionalWrapper`). O `sc_backend.cpp` agora roda esses passes no
novo PassManager, entre o AlwaysInliner e o codegen, que continuam no legado.
Os gerenciadores de análise são montados à mão, só com o que esses passes
pedem (TTI, TargetLibraryInfo, ORE, AssumptionCache, DominatorTree,
LazyCallGraph e os proxies), sem o `PassBuilder`, que traria todos os passes.

Contra a base antiga, só 23 arquivos mudam e todos têm corrotinas (o
`cxx-c12_format`, o `pic-` dele e 21 testes do clang, como `coreturn` e
`co_await-ast`): antes paravam com "Do not know how to promote this
operator!", agora compilam, e o `c12_format` roda certo. Os outros 13.650
ficam idênticos, então separar o AlwaysInliner num PassManager próprio não
muda nenhum objeto.

O `make learn` devolveu 8 objetos (CoroEarly, CoroCleanup, CoroFrame,
SuspendCrossingInfo, MaterializationUtils, SpillUtils, SimplifyCFGPass,
GlobalDCE) e 40 funções. Depois, 0 diferenças com e sem `-Weverything`.

| engine | bruto | gzip | zstd |
| --- | ---: | ---: | ---: |
| §15 | 18.270.869 | 6.787.594 | 5.205.596 |
| **+ corrotinas** | **18.471.730** | **6.867.480** | **5.268.583** |

São +201 KB bruto e +63 KB zstd, por uma parte do C++20 que antes quebrava.

## 17. `import std` e o `.pcm`

Processar o `<iostream>` é quase todo o tempo de uma compilação: no V8, 2,8 s
para um hello, contra 21 ms para um `main` vazio. O `<iostream>` são 841 headers,
82 mil linhas pré-processadas em C++20, e o tempo é de parse e Sema (com
`-fsyntax-only` é o mesmo). Comparei, no clang nativo do pin com os headers
da libc++ 21 do wasi-sdk:

| forma | bruto | zstd | hello | 5 headers |
| --- | ---: | ---: | ---: | ---: |
| headers crus | 4,9 MB | 0,53 MB | 1,98 s | 2,18 s |
| PCH do `<iostream>` | 17,2 MB | 8,2 MB | 0,12 s | — |
| PCH dos 5 headers | 17,8 MB | 8,6 MB | — | 0,31 s |
| `import std;` (`std.pcm`) | 28,4 MB | 13,4 MB | 0,08 s | 0,31 s |
| Clang modules (cache) | 44,9 MB | 19,9 MB | 0,13 s | 0,37 s |
| snapshot com Wizer (estimado) | ~64 MB | ~14,3 MB | ~0 | ~0 |

O PCH e o snapshot entram antes da primeira linha do arquivo: um `#define`
antes do `#include` é ignorado sem aviso, e o que o prelúdio inclui fica
visível. Um módulo é compilado isolado, então isso não acontece, e a libc++
divide o `std` em submódulos (`std::map` sem `<map>` dá "declaration of 'map'
must be imported from module 'std.map'"). O `wasm-ctor-eval` não serve para
o snapshot: para no primeiro `memory.grow` e não chama WASI. O `wasmtime
wizer` serve.

Escolhi `import std;`: é o que o aluno aprende primeiro, e o `.pcm` é gerado
pelo próprio engine na primeira execução e guardado em cache. O
`std.cppm` do wasi-sdk não compila para wasip1 como está: `<csetjmp>` e
`<csignal>` dão `#error` na libc do WASI, então o corpus tira as duas linhas.

O `sc_inv.cpp` aceita `-emit-module-interface -fmodules-reduced-bmi`, `-I` e
`-fmodule-file=<nome>=<pcm>`, e o `checkinv` compara as três combinações novas
(SAME). O BMI reduzido do driver só vale com `-fmodule-output`; com
`--precompile` o `.pcm` é o completo, e o reduzido seria quase do mesmo tamanho
(27,8 MB). O corpus gera o `std.pcm` com o engine em teste (`std-pcm`, o md5
é estável entre o engine inteiro e o cortado) e compila os programas de
`test/mod`, que importam o `std`, também como PIC com o mesmo `.pcm`.

O `make learn` devolveu 9 objetos de `clangSerialization` (o `ASTWriter` e o
resto do `ASTReader`; o `ASTReader.cpp` e o `ASTReaderDecl.cpp` já ficavam) e 352 funções. Depois,
0 diferenças nos 13.680 arquivos, com e sem `-Weverything`.

| engine | bruto | gzip | zstd |
| --- | ---: | ---: | ---: |
| §16 | 18.471.730 | 6.867.480 | 5.268.583 |
| **+ `import std`** | **19.293.131** | **7.106.557** | **5.452.461** |

No V8, com o engine cortado:

| | tempo |
| --- | ---: |
| gerar o `std.pcm` (uma vez) | 6,8 s |
| hello com `import std;` | 0,14 s |
| 5 headers com `import std;` | 0,48 s |
| hello com `#include <iostream>` | 2,8 s |

São +184 KB zstd no engine, e o `.pcm` (28 MB) não é baixado: é gerado
no navegador.

## 18. O loader do REPL

O REPL vai carregar cada entrada como um objeto compartilhado (§15) por cima
das anteriores. O protótipo do loader está em `loader/` (Rust e wasmtime,
`build/loader/release/scload`) e roda o `.so` de cada programa do corpus e
sequências de entradas no estilo do REPL.

**Desenho.**

- **Sem módulo principal.** O plano era ligar a libc e a libc++ num módulo
  principal que exportasse tudo. Não foi preciso: o loader carrega o
  `libc.so`, o `libc++abi.so` e o `libc++.so` do wasi-sdk como qualquer
  outro `.so`, na ordem do `needed` do `dylink.0`.
- **Recursos compartilhados.** O host cria a memória, a tabela de funções e
  o `__stack_pointer`, e todos os módulos os importam.
- **Memória dos módulos.**
  - A pilha fica no começo, como no `--stack-first`, e um estouro dá trap.
  - O primeiro grupo de módulos (as bibliotecas e o programa) fica logo
    depois da pilha, e o heap da libc (`__heap_base`) começa onde ele termina.
  - As entradas seguintes recebem memória do `aligned_alloc` da libc.
- **Ligação de símbolos.**
  - Os imports `GOT.mem` e `GOT.func` são globais mutáveis, preenchidos
    quando o grupo inteiro já foi instanciado. Isso é necessário porque o
    `libc++abi.so` e o `libc++.so` dependem um do outro.
  - Uma função importada é ligada direto à definição já carregada. Se ainda
    não existe definição, ela passa por um trampolim do host, que é fixado
    no fim do grupo. Isso acontece, por exemplo, quando a libc chama um
    símbolo próprio que pode ser substituído, como `lseek`.
- **WASI.** O WASI do wasmtime lê a memória pelo export `memory` de quem
  chama, e nenhum `.so` exporta a memória. O loader gera um módulo de ponte
  que importa a memória, a exporta e repassa as chamadas.
- **Execução.** No fim, o loader chama o `main` (pelo `__main_void` da libc,
  se o programa recebe `argc` e `argv`) e o `exit` da libc, que esvazia o
  `stdio`.

**Regra das redefinições.**

- Uma definição nova numa entrada esconde a antiga para as entradas
  carregadas depois. Quem já estava ligado continua com a antiga: um ponteiro
  para o `f` antigo segue chamando o antigo, e o `f` antigo segue lendo o `x`
  antigo.
- Funções inline, instâncias de template, vtables e typeinfo vêm em cada
  entrada que as usa. Para elas, a primeira definição tem de valer em todas
  as entradas, senão uma `static` dentro de uma função inline se duplica e o
  `dynamic_cast` entre entradas falha.
- O `wasm-ld` não diz ao loader quais definições são fracas, porque a seção
  `dylink.0` só lista os exports de TLS. O `llvm/dylink-weak.patch` acrescenta
  os exports fracos, com a flag `BINDING_WEAK`. Um corte não resolve isso. O
  Emscripten só olha a flag TLS dessa lista, então a mudança não o afeta.
  O engine cresce 41 bytes: 19.293.172 bruto, 7.106.613 gzip, 5.452.450 zstd.
- As funções fracas já funcionavam sem o patch, porque o `wasm-ld` faz o
  próprio módulo importá-las e o loader liga o import à primeira definição.
  Os dados fracos (typeinfo, `static` locais) passam pelo GOT, e é para eles
  que o patch serve.

**O `.so` leva o compiler-rt.** O link estático inclui a
`libclang_rt.builtins.a`, e o `-shared` não incluía. O `long double` e o
`__int128` chamavam `__netf2`, `__multi3` e outras funções que não estavam
em nenhum `.so`. Agora o corpus passa a `libclang_rt.builtins.a` ao `ld
-shared`, e cada `.so` leva as funções de que precisa.

**Resultados.**

- Os 1.743 programas que rodam no estático dão o mesmo resultado como
  `.so`: saída e código de saída iguais, ou trap nos dois. A única diferença
  é a pilha: o loader usa 1 MiB, contra 64 KiB do `wasm-ld`. Com
  `--stack 65536`, os 8 programas que estouram a pilha no estático também
  dão trap no loader, e o `return-addr` imprime os mesmos endereços.
- O corpus agora executa os `pic-` pelo loader, e o `test/repl` traz duas
  sequências de entradas (`repl-c`, `repl-x`):
  - C: redefinição de função e de global, ponteiro de função entre
    entradas, heap entre entradas;
  - C++: `std::vector<std::string>` vivo entre entradas, `static` de função
    inline compartilhada, `dynamic_cast` entre entradas, e uma entrada com
    `import std;` por cima de entradas compiladas com `#include`.

Tempos do loader (nativo, 48 núcleos, cache do wasmtime):

| programa | a frio | com cache |
| --- | ---: | ---: |
| C (`libc.so`) | 0,15 s | 0,03 s |
| C++ (`libc++.so`) | 0,45 s (4,1 s com 1 núcleo) | 0,10 s |

As bibliotecas que o REPL precisa ter à mão:

| | bruto | zstd |
| --- | ---: | ---: |
| `libc.so` | 1.906.118 | 645.773 |
| `libc++abi.so` | 1.074.125 | 230.638 |
| `libc++.so` | 5.314.463 | 1.111.133 |

Na web, os `.so` vão ser baixados (2 MB zstd no total) e o loader vai ser
reescrito em JS com `WebAssembly.instantiate`, seguindo a mesma lógica.
