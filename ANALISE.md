# sc: análise de um engine C baseado em clang/LLVM

Data: 2026-09-29. LLVM no pin do `rustports/llvm/upstream`
(`5fe235b9862f`, 23.0.0git); desde 2026-10-13, `llvmorg-23.1.3`
(`0d261d1ca552`, §21). Toolchain de build: wasi-sdk 29 (clang 21).
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

## 19. O engine nativo

O REPL nativo vai rodar o compilador como código nativo e só o código do
aluno em wasm. Por isso o mesmo engine agora também é ligado para x86-64
Linux (`build/sc`, `make native`), com os mesmos cortes. Ele continua
gerando wasm32-wasip1.

**Build.**

- O LLVM do host fica em `build/host`. Ele é compilado das mesmas fontes
  com patches (`build/src`), com as opções do build wasm: `MinSizeRel`, sem
  PIC e sem threads (como no wasm). Os `tblgen` vêm do `build/native`.
- O `build/native` não serve para isso. Ele só existe para as ferramentas,
  e aqui ele foi configurado de outra árvore, sem os nossos patches.
- O `scripts/link.py --native` liga o engine com o `ld.lld`. As regras de
  `cuts/` são as mesmas: os membros `.cpp.o` são comparados como `.cpp.obj`.
  - Uma função cortada vira um stub que imprime o nome
    (`sc: cut function called: ...`) e chama `abort`.
  - Um dado cortado vira zeros. Antes, o stub de um dado era código, e a
    `cl::opt` `EnableSingleByteCoverage`, lida como verdadeira, mudava o
    código gerado para o operador `?:`. No `sc.wasm`, o `wasm-ld` deixa um
    dado indefinido no endereço 0, onde a memória é zero.
- O `--icf=safe` do `ld.lld` funde as funções idênticas cujo endereço não é
  usado. Isso tira mais 11%.

**As funções que o corpus usa.**

- O `cuts/symbols.used` foi aprendido no wasm. O nativo casa os nomes
  desmanglados, com duas diferenças de ABI:
  - o `int64_t` é `long long` no wasm32 e `long` no x86-64;
  - o `std::` é `std::__2` (libc++ do wasi-sdk) no wasm e `std::__cxx11`
    (libstdc++), com `[abi:cxx11]`, no nativo.
- Sobram 14 funções que só o nativo chama (`cuts/symbols.host.used`,
  `make learn-native`). A maioria o `-Os` inlinou no wasm e não no x86-64,
  como os `TypeLocReader::Visit*` do `.pcm`. Uma muda de nome:
  `SmallVector<Literal, N>` tem `N` 13 no wasm e 12 no x86-64, porque o `N`
  padrão depende do tamanho do ponteiro.
- O `link.py --learn` agora roda de novo só os arquivos que diferem, até
  passarem, e depois o corpus inteiro. O `make learn-native` levou 20 min.

**O nativo contra o wasm, sem cortes.** Os dois diferem em 45 dos 13.682
arquivos, todos por causa do host:

- **A pilha:** em recursão profunda (`instantiation-depth`, `limits-*`,
  `deep_recursion`), a pilha do wasm e os 8 MB do nativo estouram em pontos
  diferentes, ou só um deles estoura.
- **A memória:** um arquivo dá "Out of memory" no wasm32 e compila no
  nativo.
- **O sandbox:** um `#include "../x"` fora da pasta mapeada dá "Operation
  not permitted" no wasmtime e "file not found" no bwrap.
- **O `std.pcm`:** os bytes diferem (224 bytes a menos no nativo), mas os
  programas com `import std;` dão o mesmo resultado, e o `sc.wasm` usa o
  `.pcm` gerado pelo nativo.

Fora isso, os objetos e os `.wasm` ligados são idênticos byte a byte. Por
isso o `make test-native` compara o `build/sc` com o nativo sem cortes
(`build/sc-host-full`): 13.682 arquivos, 0 diferenças. O `corpus.py` roda o
nativo no `bwrap`, com as mesmas pastas que o wasmtime mapeia.

**Tamanho** (sem símbolos):

| | bruto | gzip | zstd |
| --- | ---: | ---: | ---: |
| nativo sem cortes | 51.748.552 | | 15.776.120 |
| nativo cortado (`build/sc`) | 24.545.480 | 9.950.566 | 7.637.727 |
| `sc.wasm` | 19.293.172 | 7.106.613 | 5.452.450 |

Os cortes tiram 53% do nativo, quase a mesma fração que tiram do wasm.

**Tempo** (o menor de três, máquina com carga 10 a 20, wasmtime com o
engine pré-compilado):

| | nativo | wasmtime | razão |
| --- | ---: | ---: | ---: |
| C pequeno (`p1.c`) | 0,038 s | 0,071 s | 1,9 |
| `#include <iostream>` | 1,88 s | 2,87 s | 1,5 |
| `c3_stl.cpp` | 2,55 s | 4,26 s | 1,7 |
| `ld` de C++ | 0,094 s | 0,137 s | 1,5 |
| compilar o `sc.wasm` para o wasmtime, uma vez | | 2,8 s (29 s com 1 núcleo) | |

No REPL nativo o ganho maior não é o tempo de cada compilação. É não
precisar compilar o engine na primeira execução, nem do Cranelift para
isso: o host só precisa de um runtime para o código do aluno. O nativo é
`-Os`, como o wasm. Um `-O2` seria mais rápido e maior; isso ainda não foi
medido.

## 20. Exceções, `setjmp` e os padrões mais novos

O REPL vai precisar de exceções em C++ (requisito do professor). O
`setjmp`/`longjmp` do C usa o mesmo mecanismo do wasm, então entrou junto.
Na mesma rodada, os padrões passaram a ser os mais novos: **gnu23** para C
e **gnu++26** para C++ (`PLANO-REPL.md` §1.5 e §1.6).

**A forma das exceções.**

- A forma é a padronizada (exnref, `try_table`). O LLVM do pin ainda gera a
  legada por padrão (`-wasm-use-legacy-eh`, `cl::init(true)`).
- O wasmtime 48 recusa a forma legada: "legacy_exceptions feature required
  for try instruction". A padronizada roda no wasmtime 48 e no node 24.
- As flags são as do driver com `-fwasm-exceptions` e `-mllvm
  -wasm-enable-sjlj`, mais `-mllvm -wasm-use-legacy-eh=false`:
  - target features `+exception-handling`, `+multivalue`, `+reference-types`;
  - `-exception-model=wasm`;
  - `-fcxx-exceptions -fexceptions` e `-mllvm -wasm-enable-eh` em C++.
- O sc não tem o parser do `-mllvm`. O `sc_inv` põe os valores em
  `FrontendOptions::LLVMArgs`, que o `checkinv` compara, e o
  `scSetLLVMArgs` os aplica direto no `cl::opt` registrado.
- O backend (`sc_backend.cpp`) passa a usar `ExceptionHandling::Wasm`.
- O `checkinv` ganhou o caso gnu17 e dá SAME nos 8 casos. Ele pegou uma
  diferença: no C++23 em diante, o padrão de `CheckConstexprFunctionBodies`
  é falso.

**O sysroot** (`make sysroot`, `build/sysroot`).

- A libc++ do wasi-sdk 29 vem sem exceções. A libc++, a libc++abi e a
  libunwind agora são compiladas do pin (`build/src`), com o clang do pin
  (`build/native`) e `-fwasm-exceptions`.
  - As opções seguem o `__config_site` do wasi-sdk: ABI 2, musl, pthread,
    hardening `fast`.
  - A libc do wasi-sdk continua, inclusive a `libsetjmp`.
- O pin ganhou as árvores `libcxx`, `libcxxabi`, `libunwind` e `runtimes`
  inteiras. A libc++ do pin também precisa de alguns headers da llvm-libc
  (`libc/shared`, `libc/src/__support`…), para o `from_chars` de ponto
  flutuante.
- Três detalhes de build:
  - O driver do pin passa `-fvisibility=hidden` por padrão no wasm, e a
    libunwind não exportava nada. Ela é compilada com
    `-fvisibility=default`; no wasm, o `__declspec(dllexport)` dela pede
    `-fdeclspec`.
  - No wasm, a libunwind chama `__gxx_personality_wasm0` (libc++abi), e a
    libc++abi chama `_Unwind_RaiseException` (libunwind). Uma `.so` de
    cada não liga com `-z defs`. Por isso a libunwind vai dentro da
    libc++abi, na `.a` e na `.so`, e o corpus continua ligando com
    `-lc++ -lc++abi`.
  - `LIBCXXABI_SILENT_TERMINATE` fica desligado (o wasi-sdk o liga), para
    o aluno ver a mensagem do `terminate`.
- Tamanhos: `libc++.so` 788 KB e `libc++abi.so` 129 KB, sem informação de
  depuração. As do wasi-sdk têm 5,3 MB e 1,1 MB, com ela.

**O loader.**

- A `libc++abi.so` define e exporta a tag `__cpp_exception`. A
  `libsetjmp.so` importa a `__c_longjmp`. O loader guarda as tags como
  símbolos: a primeira definição vale para todos os módulos, e uma tag que
  um módulo importa antes de alguém a definir é criada pelo loader. Assim a
  exceção e o `longjmp` cruzam módulos (`repl-e`, `repl-j`).
- **Exceção não capturada.** No wasm não há fase de busca. A libc++abi
  nunca chama o `std::terminate`, e a exceção sai para o host ("thrown Wasm
  exception"), sem dizer qual era. O loader faz o que a libc++abi faz no
  nativo: pega a exnref pendente com a tag `__cpp_exception`, lê o ponteiro
  (campo 0), chama `__cxa_begin_catch` e `std::terminate`. A saída fica
  "libc++abi: terminating due to uncaught exception of type
  std::out_of_range: vector". O programa ligado estático, rodado pelo
  wasmtime, continua só com "thrown Wasm exception".

**Os padrões.**

- C: gnu23. C++: gnu++26. O `sc_inv` aceita `-std=` só para o corpus.
- O C23 rejeita definições K&R e `int` implícito, e a c-testsuite e os
  testes gcc torture parariam no frontend sem chegar ao backend. Essas
  fatias continuam em gnu17 (`gnu17()` no `corpus.py`). O resto (programas,
  REPL, testes do clang) usa o padrão novo.
- O `views::enumerate` ainda não existe na libc++ do pin. `std::print`,
  `std::expected`, `ranges::to`, deducing this, pack indexing, `=
  delete("…")` e `_` funcionam (`c18_cxx23`).

**O corpus.**

- Programas novos:
  - exceções (`c15`): por valor e por referência, hierarquia, as da
    biblioteca, RAII no desenrolar, rethrow, function-try-block,
    `exception_ptr`, `throw_with_nested`;
  - `terminate` em `noexcept` (`c16`) e exceção não capturada (`c17`);
  - C++23/26 (`c18`), C23 (`p7`), `setjmp` em C (`p6`), `import std` com
    exceções (`m4`);
  - REPL: exceção lançada numa entrada e pega em outra (`repl-e`), e
    `longjmp` de uma entrada para o `setjmp` de outra (`repl-j`).
- O `std.pcm` agora tem o `<csetjmp>` (sai só o `<csignal>`).
- São 13.698 arquivos. 7.940 mudaram de resultado contra a base anterior:
  os objetos mudam com as features novas, e o C++ usa a libc++ nova.
  - O pin compila agora os testes gcc torture com `setjmp` (`pr56982`,
    `20210505-1`).
  - Dois testes do clang passaram a quebrar o compilador: um `goto`
    computado (`ast-dump-APValue-addrlabeldiff`, agora com o `constexpr`
    do C23) e uma recursão que estoura a pilha
    (`cxx2b-deducing-this`). São as mesmas classes de antes, e os
    `const-init*` deixaram de quebrar.

**Os cortes.**

- O `make learn` levou 12 min. Ele trouxe de volta 46 funções
  (`cuts/symbols.used` foi de 2.218 para 2.261): o EH do CodeGen e da Sema
  (`EmitCXXTryStmt`, `EmitCXXThrowExpr`, `getEHDispatchBlock`,
  `handlerCanCatch`, as exception specs), o `WinEHPrepare` (que o EH do
  wasm usa) e a `ASTStructuralEquivalence`, porque o C23 aceita redeclarar
  uma struct igual. Os cortes de objetos não mudaram.

| `sc.wasm` | bruto | gzip | zstd |
| --- | ---: | ---: | ---: |
| C gnu17 + C++20 sem exceções (§19) | 19.293.172 | 7.106.613 | 5.452.450 |
| **C gnu23 + C++26 com exceções** | **19.388.589** | **7.141.165** | **5.479.984** |

As exceções e os padrões novos custam 95 KB bruto e 27,5 KB zstd (+0,5%).
`make test` e `make test-warn` passam nos 13.698 arquivos. O `checkinv` dá
SAME.

**O nativo.** O `make learn-native` levou 8 min, e o
`cuts/symbols.host.used` passou de 14 para 17 funções. O `build/sc` tem
24.633.560 bytes (+88 KB) e 7.668.404 zstd (+31 KB). O `make test-native`
passa nos 13.698 arquivos com uma exceção: o `Sema/c2x-fallthrough` foi
para o `IGNORE`. Em C23, a ordem dos seus erros de `[[fallthrough]]` muda
de uma execução para outra no nativo (o ASLR muda a ordem de um mapa por
ponteiro), e com ela muda quais linhas de código aparecem.

**Tempo.** O `c3_stl.cpp` compila em 2,72 s no nativo, contra 2,55 s na
§19, com a mesma carga. São 7% a mais, que vêm da libc++ nova em gnu++26 e
do código de EH.

**Pendente:** o `learn` precisa ser refeito depois do pin 23.1.3 (fase P).

## 21. O pin vai para a 23.1.3

O pin saiu do `main` de 8 de abril (`5fe235b9862f`) para a tag
`llvmorg-23.1.3` (`0d261d1ca552`), antes do trabalho do REPL
(`PLANO-REPL.md`, fase P). A 23.1.3 traz três correções do clang-repl de
que o REPL precisa: a duplicação de vtable (#185648), o `Undo` que
desliga a PTU retirada (#213235, backport `ea5cc83`) e as declarações
anteriores mantidas quando uma entrada falha (#218149).

**Os patches.**

- `wasi-host.patch`: três hunks não aplicavam mais e foram refeitos à mão
  (`clang/Support/Compiler.h`, os includes do `CrashRecoveryContext.cpp`,
  o `lockFile`/`unlockFile` do `Unix/Path.inc`). A 23.1.3 trouxe o
  `mapped_file_region::randomAccessImpl`, que chama `posix_madvise`, que a
  wasi-libc não tem; ganhou o mesmo `defined(__wasi__)` dos vizinhos.
- `libcxx-locale.patch` (novo): na 23.x, o `locale_base_api.h` da libc++
  inclui o `locale_base_api/ibm.h` em toda plataforma sem suporte próprio,
  e não só no z/OS. No wasm, o clang define `_GNU_SOURCE` em C++, a
  `stdlib.h` da wasi-libc declara `strtod_l` e o `ibm.h` a redefine com
  `abi_tag`: o build da libc++ para, e qualquer `<iostream>` do aluno
  pararia também. O patch devolve o `#if defined(__MVS__)` de antes. O
  `main` do LLVM ainda está assim.
- `no-veclibs.patch` aplicou limpo; `dylink-weak.patch` só mudou de
  contexto.

**O `sc_inv`.** O `LangOptions::EnableLifetimeSafety` sumiu (a análise
agora segue o `-Wlifetime-safety`). O `DiagnosticOptions::ShowColors`
virou um enum, e o cc1 agora o deixa em `Auto`, resolvido contra o stream
na hora de imprimir; antes o parser já o resolvia para falso. O `sc_inv`
deixa de mexer nos dois. O `checkinv` dá SAME nos 8 casos. Num terminal, o
nativo pode passar a colorir os diagnósticos; no corpus a saída é
redirecionada e nada muda.

**O corpus e os cortes.**

- São 13.819 arquivos (eram 13.698): os testes do clang do pin novo.
- O `make learn` levou 26 min, contando a base e o build do `scload`. O
  `cuts/symbols.used` foi de 2.261 para 2.281 funções. A maior parte da
  diferença é renomeação (`UnsignedOrNone` virou `OptionalUnsigned`, os
  `LateParsed*` saíram de dentro do `Parser`). O que é novo: as
  *expansion statements* do C++26 (`CXXExpansionStmt*`) e o
  `mayVTableBeDuplicated` (#185648). Nos cortes de objetos, o `Facts.cpp`
  da `LifetimeSafety` voltou.
- O `make learn-native` levou 12 min e trouxe uma função
  (`TypeLocReader::VisitDependentBitIntTypeLoc`, 17 para 18).
- A `libc++.so` foi de 788 KB para 856 KB (o `<text_encoding>` entrou); a
  `libc++abi.so` tem 130 KB.

| | bruto | gzip | zstd |
| --- | ---: | ---: | ---: |
| `sc.wasm`, pin de abril (§20) | 19.388.589 | 7.141.165 | 5.479.984 |
| **`sc.wasm`, 23.1.3** | **19.660.880** | **7.246.200** | **5.552.971** |
| `build/sc`, pin de abril (§20) | 24.633.560 | | 7.668.404 |
| **`build/sc`, 23.1.3** | **25.122.800** | **10.195.747** | **7.803.852** |

O pin novo custa 272 KB bruto e 73 KB zstd no `sc.wasm` (+1,3%), e 135 KB
zstd no nativo (+1,8%). `make test`, `make test-warn` e `make test-native`
passam nos 13.819 arquivos sem diferença; o `checkinv` dá SAME.

## 22. Fase 0 do REPL: sessão com estado contra PCH encadeado

A fase 0 do `PLANO-REPL.md` (§1.3) compara duas formas de compilar as
células do REPL, cada uma levada até o loader. O spike não entra nos
engines de sempre. `make spike` liga o `build/sc-spike`: o engine nativo
sem cortes, com `engine/sc_session.cpp` e `engine/sc_spike.cpp`. O código
do spike fica atrás de `SC_SPIKE`; o `sc.wasm`, o `build/sc`, o
`sc-full.wasm` e o `sc-host-full` saem idênticos byte a byte.

- **A, sessão.** Um processo vivo, uma cópia do núcleo do
  `IncrementalParser` da 23.1.3 sem o `clangInterpreter`, com um
  `CodeGenerator` puro e o backend do `sc_backend.cpp` sobre o módulo de
  cada célula. As células chegam por um pipe.
- **B, sem estado.** Cada célula é um processo: ela vira um PCH
  encadeado no anterior (`-fpch-codegen`), e o objeto da célula sai desse
  PCH.

**Como foi medido (`test/spike.py`).**

- Os 29 programas de `test/samples` (7 em C), `test/cxx` (18) e `test/mod`
  (4, com `import std;`) foram cortados em 222 células, uma por declaração
  do topo.
- Cada célula é ligada como `.so`, e o `scload` roda todas.
- A saída tem que ser a do `pic-` da base nativa, normalizada como no
  corpus.
- No teste de crash, uma célula fatal (`#pragma clang __debug crash`) entra
  no meio do programa. A sessão morre, outra é criada e as células
  anteriores são repassadas sem backend. Os nomes que cada célula exporta
  têm que ser os da primeira vez, e as células seguintes têm que dar a
  mesma saída.

| | A | B |
| --- | ---: | ---: |
| programas com a saída certa | **29/29** | 26/29 (falha `import std`) |
| crash, repasse e célula seguinte | **23/23** | sem estado |
| 1ª célula, C++ (mediana) | **0,91 s** | 1,49 s |
| células seguintes, C++: mediana, p90, máx. | **8 ms, 0,40 s, 3,0 s** | 164 ms, 0,59 s, 4,4 s |
| células seguintes, C: mediana | **3 ms** | 22 ms |
| soma das células, C++ (o programa inteiro de uma vez leva 39,1 s) | **40,5 s** | 78,4 s |
| pico de memória | 283 MB (a sessão) | 278 MB (uma célula) |

**Sessão longa (300 células pequenas).**

| | A | B |
| --- | ---: | ---: |
| C++, células 1–50 | 3,0 ms | 122 ms |
| C++, células 250–300 | 2,6 ms | 285 ms |
| C, células 1–50 | 1,6 ms | 21 ms |
| C, células 250–300 | 2,5 ms | 51 ms |
| memória, C++ | 101 MB (pico da sessão) | 159 MB (por célula) e 28 MB de PCH em disco |

Em B, cada célula carrega a cadeia inteira de PCHs, então o custo cresce
com a sessão. Em A ele fica constante. Em A, a soma das células custa o
mesmo que compilar o programa inteiro: a sessão não acrescenta nada.

**O que B precisou, e por que não é mais simples.**

- **Reemissão.** O objeto de uma célula traz de novo as variáveis e as
  definições tentativas das células anteriores, com os inicializadores
  dinâmicos. É o defeito do §1.3. As cópias viram declarações pelo mesmo
  pós-passo de IR de A.
- **`static`.** O problema é o mesmo de A (§3.2), com o mesmo pós-passo.
- **Inline da libc++.** O `-fpch-codegen` gera as funções inline de um
  header só no objeto da célula que incluiu o header. A libc++ marca quase
  todas como hidden (`_LIBCPP_HIDE_FROM_ABI`), e as células seguintes não
  as achavam. O spike as exporta (hidden vira default).
- **`import std;`** O `import` de uma célula não fica visível nas
  seguintes pelo PCH encadeado ("declaration of 'string' must be
  imported").

O ganho de B seria não ter estado: nada de repasse nem de `CleanUpPTU`. O
pós-passo de IR, que é a parte delicada, B precisa do mesmo jeito.

**O que o spike mostrou de A.**

- **Pós-passo do §3.2.** Funciona para:
  - `static` com inicializador dinâmico;
  - `static` local de função `static`;
  - namespace anônimo, com vtable;
  - `static inline` de template;
  - global com inicializador dinâmico usado em células depois.

  Um defeito achado: o global promovido não pode ficar `dso_local`. O
  wasm-ld não põe uma função exportada `dso_local` na tabela do `.so`, e a
  chamada por ponteiro (o `qsort` do p1, o `std::function` do c4) dava
  "out of bounds table access".
- **Erro e redefinição.**
  - Uma célula com erro seguida de uma célula boa funciona.
  - Um `#include` depois de uma célula com erro funciona.
  - A redefinição é rejeitada, como decidido (§3.4).
- **`return` no topo** derruba a sessão (o bug aberto do §3.7). Volta pelo
  repasse; a fase 2 precisa do diagnóstico antes do CodeGen.
- **Onde o código sai.** O CodeGen adia a emissão: o código de uma função
  sai no objeto da célula que primeiro a usa, em geral a do `main`. As
  células de declaração saem quase vazias, e o custo vai para a célula que
  usa.

**Decisão: A.** É mais rápida desde a primeira célula, constante numa
sessão longa e aceita `import std;`. O estado que B evitaria (repasse,
`CleanUpPTU`) já funcionou no spike. O `engine/sc_session.cpp` é a base da
fase 2. O `sc_spike.cpp` e o `test/spike.py` ficam até as fatias `split-`
e `crash-` da fase 3 os substituírem.

## 23. Fase 1 do REPL: a API em C e a API segura em Rust

Data: 2026-10-13.

**O que entrou.**

- `engine/sc.h`, a API em C do plano (§2), com quatro diferenças do
  rascunho:
  - `sc_session_new` devolve o status e sempre entrega a sessão, para que os
    diagnósticos de uma sessão que não começou possam ser lidos;
  - `sc_session_free`;
  - o `codegen` da célula virou `mode` (`SC_CELL_SO` ou `SC_CELL_REPLAY`);
  - os argumentos do wasm-ld (`-L`, as bibliotecas, os builtins) vêm depois
    de `--`. O engine não precisa saber onde ficam a libc e os builtins.
- `main` virou `sc_main`, e `engine/sc_entry.cpp` é o `main` dos
  executáveis (`sc.wasm`, `build/sc`, os engines sem cortes).
- `engine/sc_api.cpp`, a sessão da API sobre o `ScSession` do spike:
  - os diagnósticos vão para uma string;
  - o objeto de cada célula é ligado pelo `lldMain` num diretório
    temporário da sessão, e o `.so` é lido de volta (§3.1 do plano);
  - uma falha de link depois do codegen dá `SC_LOST`, e o `undo` também
    (§3.3).
- A sessão entra só no engine nativo sem cortes que o Rust liga. O
  `sc.wasm` e o `build/sc` ficam como estavam até as fases 4 e 5.

**O link reinicia as opções do LLVM.** O driver do wasm-ld chama
`cl::ResetAllOptionOccurrences` a cada link. Na sessão, a primeira célula
ligava e a segunda perdia `-wasm-enable-sjlj` e `-wasm-enable-eh`, e o
backend abortava com "-exception-model=wasm only allowed with at least one
of -wasm-enable-eh or -wasm-enable-sjlj". O spike não via isso porque ligava
cada célula em outro processo. Agora o `scSetLLVMArgs` roda antes de cada
backend da sessão e pula as opções já dadas, o que evita o aviso "may only
occur zero or one times". No one-shot ele roda uma vez só, como antes.

**O crate `sc-proto` (`proto/`).**

- **bindgen** sobre `engine/sc.h`: os enums viram módulos de constantes, e
  o `sc_msg_header` vem com a checagem de layout. Nenhuma constante é
  escrita à mão.
- **Ligação** com a feature `engine`:
  - o `build.rs` lê o arquivo do `link.py --native --full --emit-link`: os
    mesmos objetos e arquivos do engine, sem o `sc_entry.o`, mais os stubs e
    as flags de link (`-fuse-ld=lld`, `--gc-sections`, `--icf=safe`);
  - o `build.rs` soma `-lstdc++`;
  - o `rustc-link-arg` só alcança os programas, testes e exemplos do
    próprio crate. Para o binário `sc` da fase 6b, que fica em outro crate,
    o crate declara `links = "sc"` e passa o arquivo adiante como
    `DEP_SC_LINK_ARGS`.
- **API segura:**
  - `Session::new`, `cell` e `undo` devolvem um `Reply`, que empresta a
    sessão. Os buffers do engine que ele expõe (`diags`, `so`, `syms`) não
    sobrevivem à chamada seguinte, porque ela pede `&mut`;
  - o engine não é thread safe (LLVM sem threads). Só a primeira thread que
    o usa pode usá-lo, e as outras recebem `Error::OtherThread`.
    `Session` não é `Send` nem `Sync`;
  - um NUL nos argumentos e uma célula de 4 GiB ou mais são recusados antes
    da chamada;
  - `main` embrulha o `sc_main`.
- **O que o Rust não garante:** um erro fatal ou um crash do compilador
  ainda encerra o processo (o `return` no topo, §22). Por isso o REPL
  nativo continua com o filho (§1.1 do plano).
- **Teste (`make test-proto`):**
  - uma sessão C com `static` compartilhado e uma célula com erro;
  - o repasse, que dá os mesmos símbolos;
  - o `undo`;
  - uma sessão C++ com exceção e libc++, rodada no `scload`;
  - o `main` e a regra de thread.

  Leva 2,1 s.

**Regressões.**

- `make test`, `make test-native` e `make test-warn`: 13.819 arquivos, 0
  diferenças.
- `make checkinv`: SAME.
- `make spike`: igual ao §22 (A 29/29, B 26/29, repasse 23/23).
- Tamanho: o `sc.wasm` fica com 5.552.196 bytes zstd (−775 bytes) e o
  `build/sc` com 7,80 MB zstd. A sessão ainda não está neles.

## 24. Fase 2 do REPL: o laço do filho e o corpus em células

Data: 2026-10-14.

**O laço é em Rust** (decisão do §1.1 do plano). O `sc-proto` ganhou:

- `wire`: o cabeçalho do `sc.h` e as três partes, na ordem de bytes do host;
- `serve`, com a feature `engine`: o lado do filho, uma sessão por vez, até
  o fim da entrada;
- `Child`: o lado do pai. O filho que morre (crash, `SIGILL` do
  `__debug crash`) vira um erro de E/S, com o status de saída;
- `Repl`: o log das células, inclusive as que falharam. Quando a sessão se
  perde, um filho novo repassa o log sem backend, e cada célula aceita tem
  que exportar os mesmos nomes (§3.6 do plano). O `undo` tira do log a
  última célula aceita e repassa o resto.

O `build/sc-engine-full` (`proto/src/bin/sc-engine.rs`) é o engine sem cortes
com esse laço: `-cc1`/`ld` vão para o `sc_main`, `session` é o filho e
`repl <dir> <células...> -- <args>` é um teste do lado do pai, que grava o
`.so` de cada célula aceita. O filho escreve o protocolo numa cópia do
stdout e põe o stderr no fd 1, para que nada do engine quebre uma mensagem.
Oito células com um crash e um `undo` (três filhos, dois repasses) levam
0,12 s.

**O `split-` (`test/split.py`, `make test-split`).** Os 1.947 programas que
o corpus executa (samples, c-testsuite, gcc torture `execute/`, `test/cxx` e
`test/mod`) são cortados em células, uma por declaração do topo (o corte do
`spike.py`). As células passam pelo `sc-engine-full repl`, e o `scload` roda
os `.so`. A saída tem que ser a do programa inteiro (`pic-` da base nativa).
São 10.541 células, e a rodada leva 29 s em 48 núcleos.

As rodadas, com as diferenças que sobraram depois de cada correção:

| rodada | correção | diferenças |
| --- | --- | --- |
| 1 | — | 213 (91 com célula que falha) |
| 2 | o corte (definições K&R, `typedef struct __attribute__((...)) {`, `-I` do diretório do programa) e o escopo de bloco em C | 135 |
| 3 | o tipo das declarações de funções sem protótipo | 23 |
| 4 | no loader, o GOT de um símbolo que uma célula posterior define; o `static` usado antes da célula que o define; o `always_inline` | 13 |
| 5 | o tratador de diagnósticos do backend; os stubs do loader; um `//` dentro de `/* */` no corte | 7, dos quais 6 conhecidos |
| 6 | as bases refeitas com o loader novo | só os 6 conhecidos |

**O escopo de bloco em C** é um bug do upstream. Em
`Sema::ActOnPopScope`, com o modo incremental em C, nenhum nome sai do
`IdResolver`. O comentário diz que é para os nomes do topo valerem nas PTUs
seguintes, mas a condição vale para todo escopo, então parâmetros, locais e
rótulos ficam visíveis depois do bloco. `int f(int f) { return f; }` numa
sessão dava "redefinition of 'f' as different kind of symbol". O patch
acrescenta `|| S->getParent()`, para que só o escopo do TU guarde os nomes.
Um corte não resolve isso. Fora do modo incremental a condição não muda, e os
engines one-shot dão o mesmo resultado.

**O pós-passo da sessão** (`sc_session.cpp`) ganhou três coisas:

- **Declarações retipadas.** A sessão guarda o tipo LLVM de cada função que
  uma célula exporta (as células dividem o `LLVMContext`). Uma declaração
  com outro tipo é refeita com o da definição, e o `FixFunctionBitcasts` do
  backend ajusta as chamadas, como no TU inteiro.
- **`static` antes da definição.** A declaração de um nome interno vira o
  nome promovido (`f.sc`). A sessão guarda esses nomes ainda sem definição.
  Antes de a célula que os define fechar o módulo, ela pede ao CodeGen o
  endereço deles, e isso os tira dos adiados. Sem isso, o CodeGen só os
  emitiria se a própria célula os usasse.
- **`available_externally`.** O `scExports` não lista mais essas definições:
  o backend as descarta, então elas não são exportadas. Uma célula
  posterior que precisasse do corpo para inlinar ganhava só uma
  declaração. Depois de cada célula elas voltam aos adiados
  (`HandleTopLevelDecl`), como o CodeGen já faz em modo incremental para as
  internas e as `linkonce` (`addEmittedDeferredDecl`).

**O tratador de diagnósticos.** No one-shot, o `BackendConsumer` do
`CodeGenAction` instala um tratador no `LLVMContext`. A sessão não tem
`CodeGenAction`, e o tratador padrão do LLVM imprimia o erro do backend sem
que o clang o visse: a célula dava `SC_OK` com um objeto ruim. Agora erros e
avisos do backend vão para os diagnósticos da sessão
(`err_fe_backend_unsupported`), e a célula dá `SC_LOST`, porque o código já
foi gerado.

**O loader** (`loader/src/main.rs`):

- **Referências para a frente:**
  - um `GOT.mem` sem definição fica pendente e é preenchido quando o
    símbolo chega;
  - um `GOT.func` ganha na hora uma entrada da tabela, preenchida quando a
    função chega. O ponteiro é o mesmo para todos os módulos.
- **Chamadas para uma função ainda sem definição.** Antes, iam por um
  trampolim do host (`Func::new`). Cada chamada passava pela pilha nativa,
  e uma recursão de 500 níveis entre células (`nestfunc-4`) estourava a
  pilha. Agora vão por stubs em wasm, um `call_indirect` para a mesma
  entrada da tabela. Antes de a definição chegar, a entrada tem uma função
  do host que dá "undefined symbol: X".
- Isso também muda o `pic-` e o `repl-` de um programa inteiro com um
  símbolo indefinido: o erro passa do carregamento para a primeira chamada
  (`bcp-1`). As bases foram refeitas, e a diferença entre as bases antigas
  e as novas está abaixo.

**Limites conhecidos** (`KNOWN` no `split.py`; 6 programas):

- um `#line` não passa de uma célula para a seguinte;
- uma diretiva no meio de uma declaração (o corte a parte);
- um `alias` para um nome de outra célula;
- um `#include "../../gcc.dg/..."` que o `-I` das células acha e a
  compilação inteira (no bwrap) não;
- **uma função com o nome de uma da libc e outra assinatura** (`div` em
  `20090711-1`). O link da célula seguinte resolve o nome pela `libc.so` e
  dá "function signature mismatch". O loader já esconde a da libc atrás da
  célula. Corrigir no link exigiria passar os `.so` das células anteriores
  ao wasm-ld, e eles não existem num filho que acabou de repassar o log.
  Fica para depois.

Avisos que as células dão e o TU inteiro não, como "unused function" a
cada célula e "has internal linkage but is not defined" para um `static`
usado antes da definição, são o §3.4 do plano e ainda não foram filtrados.

**Regressões.**

- `make test`, `make test-native` e `make test-warn`: 13.819 arquivos, 0
  diferenças, contra as bases refeitas.
- As quatro bases, antigas contra novas: só o `pic-bcp-1` muda (o loader).
- `make checkinv`: SAME ×8.
- `make test-proto`: verde, com o teste novo do `Repl` (crash, repasse,
  `undo`).
- `make test-split`: 1.947 programas, 0 diferenças fora os 6 conhecidos.
- `make spike`: A 29/29, B 26/29, repasse 24/24. O repasse cobre um
  programa a mais (eram 23) porque o corte mudou.
- Tamanho: `sc.wasm` 5.552.276 bytes zstd (+80, o patch) e `build/sc`
  7.799.434 zstd. A sessão ainda não está neles.


## 25. A célula que falha: snapshot por `fork`

**O problema.** Uma célula com erro passa pelo `CleanUpPTU`, copiado do
clang-repl. Ele tira do lookup as declarações da célula, mas não o resto.
Casos que quebram a célula seguinte, todos com `int x = ;` na que falha:

| célula que falha | a seguinte, igual mas sem erro |
| --- | --- |
| `#include <stdio.h>` (C) | `printf` não declarado: a guarda `_STDIO_H` ficou definida e o cabeçalho não é lido |
| `#include <vector>` (C++) | "reference to ... is ambiguous": o `Sema` guarda o `namespace std` da célula, e o `std` novo reabre o morto |
| `new int(3)` (C++, sem cabeçalho) | "no matching function for call to 'operator new'": o `operator new` implícito ficou no TU morto |
| `#include <vector>` com `<cstdio>` antes | "redefinition of enumerator": os enumeradores ficam no lookup do `std`, que sobrevive |

Desfazer as macros e o `#pragma once` resolve o C. Zerar os ponteiros do
`Sema` para o `std` troca a ambiguidade pelo `operator new` e pelas
categorias de comparação. Tapar cache por cache não termina.

**Decisão: o filho faz `fork` antes de cada célula** (`serve`, em
`proto/src/engine.rs`):

- a cópia espera num pipe;
- se a célula é aceita, o original manda `d` e a cópia acaba;
- se falha, o original manda `t` e a resposta, e acaba. A cópia envia a
  resposta e segue com a sessão de antes da célula;
- se o original morre na célula (o `#pragma clang __debug crash` do
  teste), o pipe fecha sem nada. A cópia responde `SC_ERROR` ("the engine
  crashed on this cell") e segue, sem repasse;
- se o `fork` falha, uma célula com erro vira `SC_LOST`, e o `Repl` repassa
  o log sem ela.

Assim o log do `Repl` guarda só as células aceitas. O pid do filho muda a
cada erro, e o `Child` não conta mais com ele para saber como o filho
acabou. O `CleanUpPTU` continua: é ele que deixa o `sc_session_cell`
utilizável sem o filho, nos testes da API.

**Custo**, no `sc-engine-full`, com os cabeçalhos `iostream`, `vector`,
`string`, `map` e `algorithm` mais 20 funções, uma por célula:

| | mediana de 5 |
| --- | --- |
| sem `fork` | 3,27 s |
| com `fork` | 3,62 s |

São ~17 ms por célula aceita: a cópia das tabelas de páginas e as cópias
na escrita enquanto a cópia vive. 20 células com erro custam 3 ms cada.
Repassar o log a cada erro, a outra opção correta, custaria ~2 s por erro
nessa sessão.

**No navegador** não há `fork`. O equivalente é copiar a memória linear do
wasm antes da célula e restaurá-la se ela falhar. Fica para a fase 6b.

**Regressões.** `make test-proto` verde: o teste do `Repl` cobre um
`#include` repetido depois de uma célula com erro, e o crash agora é
`Error` sem repasse. `make test-split`: 1.947 programas, 0 diferenças fora
os 6 conhecidos.

## 26. Protótipo do REPL interativo (`sc`)

Antes da fase 6b, um `sc` interativo para usar o REPL de verdade. A
interface de linha de comando é a do sgleam e do spython:
- `sc` abre o REPL;
- `sc FILE [ARG...]` roda um programa;
- `sc repl [-q] [FILE]`, `sc run`, `sc check` e `sc help`;
- `-x c|c++` escolhe a linguagem; sem ele, vale a extensão do arquivo e,
  por fim, C;
- os comandos `:help`, `:quit`, `:undo` e `:theme`;
- o prompt usa o rustyline, com histórico em `data_dir()/sc/history` e o
  tema em `config_dir()/sc/config`.

**Montagem:**
- o crate do `loader/` virou biblioteca (`Loader`) com dois binários, o
  `scload` de antes e o `sc` (`make repl`, `build/loader/release/sc`);
- o compilador é o filho `build/sc-engine-full session`, pelo
  `sc_proto::Repl`;
- os argumentos da sessão são os do `test/split.py --args`, gravados em
  `build/repl/{c,c++}.args`;
- o engine ainda não está ligado ao `sc`; isso continua na fase 6b;
- o `sc` e suas dependências ficam atrás da feature `repl` do crate. O
  `scload` (`make test`) não precisa do `sc-proto`, cujo bindgen pede a
  libclang.

**Decisões provisórias do front end** (PLANO-REPL.md §8):
- **A célula:** declarações e statements do topo, que o modo incremental
  aceita. Os statements rodam quando o `.so` da célula é carregado.
- **O `main`:** roda só em `sc FILE`. No REPL ele é uma função como as
  outras, e o aluno a chama numa célula.
- **O stdin** é o do terminal: o rustyline o larga enquanto a célula roda.
- **Trap:** a mensagem é curta (`error: wasm trap: integer divide by
  zero`, `aborted`, `error: stack overflow`), o `__stack_pointer` volta
  ao de antes da célula e o REPL segue. O que a célula definiu fica.
- **Exceção C++ não capturada:** a libc++abi imprime "terminating due to
  uncaught exception…" e a célula termina como `aborted`; o REPL segue.
- **`exit(n)`** encerra o REPL com o status `n`.
- **Ctrl-C:** o handler de SIGINT chama `increment_epoch` e a célula para
  com "interrupted" (`epoch_interruption`). O filho fica num grupo de
  processos próprio, para o Ctrl-C não matá-lo. Um Ctrl-C durante uma
  leitura do stdin só tem efeito depois que a leitura volta. O `fflush`
  depois da célula interrompida rearma o prazo, senão ele também pararia e
  o que ficou no buffer sairia depois da célula seguinte. Isso vale só no
  REPL: `sc FILE` roda sem as checagens de epoch, que custam em todo laço e
  chamada, e o Ctrl-C mata o processo, como no `scload`.
- **`sc check`** compila em `SC_CELL_SO`, com o backend e o link, e não
  roda nada. O `SC_CELL_REPLAY` seria mais barato, mas perderia os erros
  que só o backend ou o `wasm-ld` dão (um símbolo indefinido).
- **`:undo`** desfaz as declarações no compilador; o que a célula já rodou
  continua feito.
- **O Enter pergunta ao clang** (ver abaixo).

**Tamanho:** `sc` 28,1 MB, contra 27,2 MB do `scload` (wasmtime com as
features padrão; o mínimo do §1.4 fica para a fase 6a). O rustyline, o
bpaf e o `sc-proto` somam 0,9 MB.

**O Enter pergunta ao clang.** Uma primeira versão decidia com uma
heurística em Rust: contava os colchetes e adivinhava um cabeçalho de
função pela forma da linha. Ela não sabe o que é tipo (`T * f(x)`). Agora
quem decide é o engine, com duas coisas novas no `sc.h`:
- `SC_CELL_WHOLE`: compila a célula como `SC_CELL_SO`, mas responde
  `SC_INCOMPLETE`, sem diagnósticos, quando o primeiro erro está no fim da
  célula. Isso vale para um erro depois do último token da célula e para
  um erro no token de fim de entrada do modo incremental, que fica no
  arquivo principal, vazio. É o parser dizendo que queria mais: uma
  declaração sem `;`, um cabeçalho sem corpo, um `for` ou `else` sem a
  instrução, `1 +`. Um erro no meio (`x` indefinido) é um erro de
  verdade. O `serve` faz o `fork` como numa célula comum, então a célula
  incompleta não deixa nada;
- `sc_session_depth` (`SC_MSG_DEPTH`): os colchetes que a entrada deixa
  abertos, pelo lexer do clang em modo raw. Conta um a mais se a entrada
  termina dentro de um comentário ou com `\` no fim. Isso dá o recuo da
  linha nova e não mexe na sessão.

No Enter com o cursor no fim, o REPL faz o seguinte:
- pede a profundidade; com colchete aberto, abre uma linha;
- uma linha em branco roda o que houver;
- senão, manda a célula em `SC_CELL_WHOLE` e guarda a resposta. Se ela for
  completa, a célula usa essa resposta: a compilação do Enter é a própria
  célula, sem trabalho em dobro.

Custo medido, contando compilar, ligar e carregar: cerca de 10 ms por
célula em C e cerca de 22 ms em C++ com `iostream`. `int x = 5` sem `;`
espera a linha seguinte; a linha em branco mostra o erro.

**O valor de uma expressão sem `;`.** O plano (§3.4) deixava a impressão
de valor fora da v1. Ela entrou no protótipo, no formato do clang-repl:
`x * 2` mostra `(int) 42`. Como funciona:
- a sessão analisa um prelúdio com duas declarações, `__sc_show_int(const
  char *type, int kind, long long)` e `__sc_show_float(..., double)`. Ele
  é um arquivo de sistema, então não dá aviso, e não conta no `cell_N`;
- quando a última instrução de uma célula é uma expressão sem `;`
  (`TopLevelStmtDecl::isSemiMissing`), a sessão a troca no AST, antes do
  CodeGen, pela chamada a uma das duas. A chamada leva o tipo como string,
  o tipo de valor (`enum sc_show` no `sc.h`) e o valor convertido para
  `long long` ou `double`;
- quem define as duas funções é o host. O `Loader::host` põe uma função do
  wasmtime na tabela de símbolos, e a célula a importa como qualquer
  outra. O `sc` guarda o texto e o imprime depois do `fflush`, na ordem
  certa em relação ao que a célula escreveu;
- o host formata o valor: um `double` com os dígitos mínimos que leem de
  volta o mesmo número (`0.30000000000000004`), um `char` entre aspas
  simples, um `char *` ou array de `char` como string (até 1.000 bytes,
  com escapes), um ponteiro em hexadecimal, o nulo como `NULL` em C e
  `nullptr` em C++;
- se a expressão tem temporários (`std::string("abc").size()`), a chamada
  nova fica dentro do `ExprWithCleanups` da original, e eles morrem depois
  dela;
- só os escalares aparecem. Struct, classe, array que não é de `char`,
  função e `void` não mostram nada, então `std::cout << x` sai como antes;
- o nome do tipo é o do clang, com duas trocas: `__size_t` (o tipo do
  `sizeof`) vira `size_t`, e um typedef membro de classe
  (`std::string::size_type`) vira o tipo que ele nomeia.

Dois ajustes no parser incremental, no `llvm/incremental-stmts.patch`:
- a expressão sem `;` no fim da entrada não é mais um valor descartado.
  Assim, `x > 2` não avisa "comparison result unused", e o
  `warn_unused_expr`, que a sessão ignorava como o clang-repl, voltou.
  Agora `x + 1;` avisa como no compilador de uma vez só. Como o valor não
  é descartado, o `CheckPlaceholderExpr` não rodaria, e `s.size` (sem os
  parênteses) ou `__builtin_abs` chegariam ao CodeGen e derrubariam o
  engine. O patch o chama antes;
- `-x` ou `+x` no topo era "expected external declaration": o parser
  reservava `-` e `+` para os métodos de Objective-C. No modo incremental,
  eles começam uma instrução.

Os dois só mudam o modo incremental. O `make test` não mudou (0
diferenças), o `checkinv` deu SAME, e o `sc.wasm` cresceu 53 bytes.

**O que ficou à vista** e vai para o engine:
- o primeiro diagnóstico de cada célula vem com "In file included from
  <<< inputs >>>:1:";
- a célula se chama `cell_N` nos diagnósticos (no `sc FILE` deveria ser o
  nome do arquivo);
- os diagnósticos não têm cor;
- o erro de "faltou o resto", visto quando uma linha em branco força a
  célula, aponta para `<<< inputs >>>:1:1` em vez do fim da célula.
