# Plano: compilação incremental para o REPL

Versão 4 (2026-10-01):
- o REPL nativo usa o wasmtime mínimo, com JIT (§1.4);
- exceções e `setjmp`/`longjmp` entram no escopo (§1.5);
- o compilador roda num processo filho (§1.1).

Decisões do professor (2026-10-01):
- a redefinição é rejeitada por enquanto (§3.4);
- um binário só, `sc`, que se reexecuta como filho (§1.1);
- a linguagem usa os padrões mais novos (§1.6).

A versão 2 foi revisada por três revisores adversários: internals do clang,
runtime e build, testes e sequência. Cada mudança cita o achado que a
motivou (C = clang, R = runtime, T = testes). Os experimentos estão no
scratchpad da sessão (`adv1`–`adv3`) e em `~/.cache/sc-wtsize`, `sc-wmi` e
`sc-wamr`.

## 0. Objetivo e restrições

- **O que a célula vê.** O REPL compila cada célula sobre as anteriores.
  Tipos, funções, variáveis, macros e `#include` das células anteriores valem
  na seguinte, sem o aluno repetir declarações. Os headers são lidos uma vez
  por sessão.
- **Execução.** Cada célula vira um `.so` wasm, carregado pelo loader (§18)
  sobre os anteriores. O código do aluno é sempre wasm.
- **Dois alvos.** O mesmo engine e os mesmos cortes servem ao nativo (x86-64
  Linux) e ao wasm (`sc.wasm`, web).
- **API.** A API do engine é C, em `engine/sc.h`. O Rust usa o que o bindgen
  gera a partir desse header, sem nada mantido à mão (ver §1.1 e §2).
- **Linguagem.** C++ **com exceções**, e C com `setjmp`/`longjmp` (§1.5).
- **Regras do AGENTS.md:**
  - `make test` com 0 diferenças;
  - `make checkinv` SAME;
  - cortes antes de patches;
  - ANALISE.md atualizado.

## 1. Decisões

### 1.1 Nativo: sessão num processo filho (R1–R4, R16, T-M5)

O código do aluno roda no wasmtime (§1.4). O wasmtime trata os traps por
sinais, e compilar dentro do processo do REPL não é seguro:

- **Os handlers de sinal brigam.** O `CrashRecoveryContext` instala
  handlers sem encadear. Um trap do código do aluno (SIGILL, SIGFPE ou
  SIGSEGV) chega a ele sem contexto ativo. Ele então desliga a recuperação, e
  daí em diante `Process::Exit` mata o REPL.
- **O contexto é global.** Com `LLVM_ENABLE_THREADS=OFF`, o contexto
  "corrente" do CRC é um global do processo.
- **`llvm_unreachable` é comportamento indefinido** no nosso build.
- **SIGSEGV no `malloc`.** Ele deixa a trava da arena presa, num processo
  com threads.
- **Vazamento.** Uma sessão abandonada vaza mais de 100 MB.
- **Mais handlers.** O LLVM instala os próprios handlers
  (`RemoveFileOnSignal`).

**Decisão.** A sessão vive num processo filho de longa duração:

- **Um binário só, chamado `sc`.** Ele junta o REPL, o loader e o engine:
  - `sc` é o REPL;
  - `sc --engine` é o filho, reexecutado pelo próprio `sc`;
  - `sc -cc1 ...` e `sc ld ...` são o engine one-shot de hoje, que o corpus
    usa para testar o binário entregue.
- **O filho.** Ele conversa por pipes com mensagens prefixadas pelo tamanho.
  Nesse modo o wasmtime nem é iniciado: o `main` do Rust chama
  `sc_entry(argc, argv)`, e o laço do protocolo é escrito em C++
  (`engine/sc_entry.cpp`).
- **Nomes no build:**
  - o crate do `loader/` passa a gerar o `sc`, que deixa de se chamar
    `scload`;
  - o executável só de C++ de hoje passa a ser `build/sc-engine` (cortado) e
    `build/sc-engine-full` (sem cortes). Ele existe só para o `make learn` e
    para os testes;
  - `build/sc` passa a ser o binário entregue;
  - o wasm continua `sc.wasm`.
- Crash, timeout ou Ctrl-C matam o filho. O REPL lança outro e faz o
  repasse (§3.6).
- Lançar o filho custa 4,5 ms e 11 MB, e matá-lo libera de verdade a
  memória.
- **Ligação do engine ao Rust, sem `ld -r`** (R7, que perderia o ICF):
  - o `link.py --native` ganha o modo `--emit-link`, que grava a lista dos
    objetos e arquivos já cortados, os stubs e o `wrap.rsp`;
  - o `build.rs` passa a lista ao link do Rust (`cargo:rustc-link-arg`),
    com `-fuse-ld=lld`, `--gc-sections`, `--icf=safe` e `-lstdc++`;
  - o `ld.lld` 19 está em `/usr/bin`.
- **Teste do binário entregue:**
  - o `make learn` dirige o `build/sc-engine`, que tem os mesmos objetos do
    engine; a parte em Rust não tem código do LLVM;
  - o `make test-native` roda o corpus também pelo `build/sc` (`sc -cc1`,
    `sc ld` e a sessão pelo `sc --engine`), contra a mesma base nativa.

**Bindgen.** O crate `sc-proto` roda o bindgen sobre `engine/sc.h` no build:

- Gera os tipos do protocolo: o enum de status, os códigos de mensagem e o
  cabeçalho de cada mensagem.
- O REPL nativo e o host Rust de testes do `sc.wasm` usam esses mesmos tipos.
- Nenhuma constante nem layout é mantido à mão.
- As declarações das funções também saem dele: o `main` do Rust chama
  `sc_entry` pelo binding gerado. É a única chamada FFI, e ela só acontece no
  processo filho.

### 1.2 Wasm: `sc.wasm` vira reactor que também exporta `_start` (R8, R9, T-B2)

- **Reactor e comando no mesmo módulo.** `-mexec-model=reactor`, mais um
  `_start` nosso que chama `__main_void`. Um experimento confirmou que o
  módulo roda como comando e como biblioteca (exports).
- **Testes inalterados.** O corpus (`wasmtime run sc.cwasm`), o `link.py
  --learn` e as bases **não mudam**.
- **Exports:** `sc_session_new`, `sc_session_cell`, `sc_session_undo`,
  `sc_result_ptr`, `sc_result_len` e `malloc`.
- **Navegador:** o engine roda num Web Worker. Recuperar uma falha é
  `terminate` + worker novo + repasse.
- **Pilha:** os 16 "call stack exhausted" do corpus vêm do limite de pilha
  wasm do motor. No navegador esse limite (~1 MB no V8) é fixo, e esses casos
  continuam falhando lá. Isso fica documentado.

### 1.3 Compilador com estado ou sem estado: decidido por um spike (T-M9)

O revisor de testes mediu a alternativa sem estado:

- **`import std`.** Uma célula com `import std;` + vector/cout compila em
  0,12 s e 53 MB no nativo. O §17 dá 0,14 s no V8.
- **PCH encadeado** (`-fpch-codegen`):
  - a célula 1, com headers e definições, leva 2,3 s e gera um PCH de 17 MB;
  - cada célula seguinte leva 0,04 s de PCH mais 0,09 s de objeto;
  - defeito achado: a célula 2 reemitiu o inicializador dinâmico da célula 1.

Sem estado não há sessão viva, nem `CleanUpPTU`, nem repasse, nem crash
recovery. Cada célula é um processo ou instância curto. Por isso a fase 0
(§5) implementa as duas candidatas no engine sem cortes e decide com dados:

- **A. Sessão** (`IncrementalParser`, §3): o caminho principal deste plano.
- **B. Sem estado:** cada célula é compilada com um prefixo vindo das células
  anteriores, por PCH encadeado. As restrições do §3.4 valem para as duas.

### 1.4 Runtime nativo do código do aluno: wasmtime mínimo com JIT

Medido em 2026-10-01: x86-64, LTO, `opt-level=z`, strip, com um laço C de
3·10⁸ iterações compilado em `-O0`.

| runtime | exceções | tamanho | tempo |
| --- | :---: | ---: | ---: |
| **wasmtime 48, Cranelift x86, features mínimas + exceções + WASI p1** | sim | **5,7 MB** | **0,81 s** |
| wasmtime 48, só Pulley (interpretador) + exceções + WASI p1 | sim | 4,9 MB | 18 s (29 s com `-Oz`) |
| wasmi 2.0 + WASI | **não** | 2,6 MB | 4,1 s |
| WAMR, interpretador rápido | **não** | 0,30 MB | 7,5 s |
| WAMR, interpretador clássico + exceções (legadas) | só legadas | 0,29 MB | 15 s |

**Por que os outros ficam de fora:**

- **Pulley.** As células são geradas em tempo de execução, então o
  Cranelift continua no binário. Sem o backend x86, que exige um patch no
  Cargo, o binário diminui só 0,8 MB e fica 20 vezes mais lento.
- **wasmi.** Rejeita a proposta de exceções (`module/import.rs:86`).
- **WAMR.** Não compartilha entre instâncias uma memória, uma tabela e um
  `__stack_pointer` criados pelo host:
  - a wasm-c-api ignora memória e tabela importadas;
  - as globais são copiadas por valor;
  - o `call_indirect` sempre chama a própria instância;
  - exceções só existem no interpretador clássico, na forma legada, e não
    atravessam módulos;
  - fechar isso dá de 1,5 a 2,5 meses de fork.

**Configuração do loader:**

- **Features:** `default-features = false` e `["runtime", "std",
  "cranelift", "gc", "gc-drc"]`. O `gc` é exigido por
  `Config::wasm_exceptions`.
- **WASI:** `wasmtime-wasi` só com `p1`.
- **Interrupção:** `epoch_interruption`, para interromper um laço infinito
  do aluno sem custo por instrução.
- O protótipo `scload` de hoje tem 26,7 MB, com as features padrão do wasmtime. O `sc` final ≈ 24,5 MB do engine + 5,7 MB do runtime.
- O crate novo `sc-proto` é a única dependência a mais.

### 1.5 Exceções e `setjmp`/`longjmp`

Hoje o C++ é `-fno-exceptions`. A libc++ do wasi-sdk 29 vem sem exceções, e
os cortes tiraram o `CGException` e o EH do `ItaniumCXXABI` (ANALISE §12). O
`setjmp.h` do WASI dá `#error` sem a proposta de exceções, e o `std.cppm`
tira o `<csetjmp>` (§17). As duas coisas usam o mesmo mecanismo, então
entram juntas.

- **Forma:** a padronizada (exnref, `try_table`). Os navegadores atuais e o
  wasmtime 48 a suportam.
  - O LLVM do pin ainda gera a forma legada por padrão
    (`wasm-use-legacy-eh`, `cl::init(true)`, `WebAssemblyTargetMachine.cpp:80`).
  - O engine liga a forma padronizada ao iniciar. O sc não lê `-mllvm`, então
    o valor é posto direto no `cl::opt` registrado.
  - Confirmado na fase E: o wasmtime 48 recusa a forma legada ("legacy_exceptions
    feature required for try instruction"), e a padronizada roda nele e no
    node 24 (V8). O valor vai no `FrontendOptions::LLVMArgs`, que o `checkinv`
    compara, e o `scSetLLVMArgs` o põe no `cl::opt` sem o parser.
- **Runtime C/C++:**
  - feito: `make sysroot` monta o `build/sysroot` (ANALISE §20). A
    libunwind vai dentro da libc++abi (as duas se chamam no wasm), e o
    `SILENT_TERMINATE` fica desligado, para o aluno ver o tipo e o `what()`;
  - a libc++, a libc++abi e a libunwind são construídas a partir do
    `build/src` (o pin) com `-fwasm-exceptions -mllvm
    -wasm-use-legacy-eh=false`, para `wasm32-wasip1`, em `.a` e `.so`;
  - a libc do wasi-sdk 29 continua, inclusive a `libsetjmp`, que já vem no
    sysroot (`libsetjmp.a`, `libsetjmp.so`);
  - o `std.pcm` é gerado de novo, agora com `<csetjmp>`;
  - novo alvo no Makefile: `build/runtimes`, com a receita registrada no
    ANALISE.
- **`sc_inv.cpp`:**
  - C++: `-fcxx-exceptions -fexceptions -exception-model=wasm`;
  - C: `-mllvm -wasm-enable-sjlj` equivalente, posto direto no `cl::opt`;
  - os dois com `+exception-handling` nas target features;
  - o `checkinv` ganha as combinações com exceções e confere contra o driver
    com `-fwasm-exceptions`.
- **Ligação e loader:**
  - o `.so` de cada célula importa a tag `__cpp_exception`, e o `__c_longjmp`
    para `setjmp`;
  - o loader cria cada tag uma vez e a passa a todos os módulos, como faz com
    a memória e a tabela. Na prática, a `libc++abi.so` define e exporta o
    `__cpp_exception`, e a `libsetjmp.so` importa o `__c_longjmp`, que o
    loader cria;
  - **exceção não capturada:** no wasm não há fase de busca, então a
    libc++abi nunca chama o `std::terminate`, e a exceção sai para o host
    ("thrown Wasm exception"). O host faz o que a libc++abi faz no nativo:
    lê o ponteiro da exceção (campo 0 da exnref com a tag
    `__cpp_exception`), chama `__cxa_begin_catch` e `std::terminate`, que
    imprime "terminating due to uncaught exception of type …: …" e aborta.
    Feito no loader; o host JS faz o mesmo com `WebAssembly.Exception.getArg`;
  - uma exceção lançada na libc++abi e pega numa célula atravessa os módulos
    pela tag compartilhada.
- **Corpus:**
  - programas de aluno com exceções: `throw`/`catch` por valor e por
    referência, hierarquia, `std::exception` da biblioteca
    (`std::vector::at`, `std::stoi`), RAII durante o desenrolar, `noexcept`
    com `std::terminate`, exceção que cruza células do REPL;
  - em C, `setjmp`/`longjmp`, inclusive entre células;
  - os testes do clang que hoje dão "exceptions disabled" passam a compilar.
    A base muda, e cada mudança vai para o `IGNORE` com motivo, ou para a base
    nova, como mudança intencional;
  - depois, o `make learn` traz de volta o código de EH. O tamanho é medido,
    e o orçamento do §5.1 é revisto só para esta fase, porque exceção é
    requisito.

### 1.6 Padrões mais novos

- **O padrão:**
  - C passa de gnu17 para **gnu23**, o padrão C mais novo publicado. O
    `c2y` do pin é rascunho.
  - C++ passa de gnu++20 para **gnu++26** (`LangStandards.def:188`).
  - A libc++ do pin tem só parte do C++26, então o que falta é medido e
    registrado.
- **No REPL e para o aluno:** o padrão é fixo, sem opção.
- **No `sc_inv`:**
  - passa a aceitar `-std=`, só para o corpus;
  - o padrão default muda;
  - o `checkinv` confere as combinações novas contra o driver com
    `-std=gnu23` e `-std=gnu++26`.
- **Cobertura dos cortes:**
  - o C23 rejeita as definições K&R e o `int` implícito, comuns nos testes
    gcc torture e na c-testsuite. No padrão novo, esses testes param no
    frontend e deixam de cobrir o backend;
  - por isso as fatias desses testes continuam com `-std=gnu17`. Os
    programas de aluno, o `test/mod` e os testes do clang usam o padrão
    novo;
  - testes C23 e C++23/26 entram em `test/samples` e `test/cxx`: `nullptr`,
    `constexpr` e `typeof` em C; `std::print`, `std::expected`, deducing this
    e `std::ranges::to` em C++;
  - o `std.pcm` é gerado de novo com gnu++26.
- **Junto da fase E:** as bases e os cortes mudam de uma vez (§5).

## 2. A API (`engine/sc.h`)

Só escalares e ponteiros, para que a assinatura seja a mesma em wasm32 e
x86-64. O resultado pertence ao engine e vale até a próxima chamada (R13).
Não há `sc_buf` alocado pelo host.

```c
enum sc_status {
  SC_OK = 0,    /* .so pronto */
  SC_ERROR = 1, /* erro no código do aluno; a sessão volta ao estado anterior à célula */
  SC_LOST = 2,  /* erro fatal, falha de link depois do codegen ou estado inconsistente:
                   a sessão deve ser recriada e repassada */
};
enum sc_part { SC_DIAGS = 0, SC_SO = 1, SC_SYMS = 2 /* exports do .so, para o repasse */ };

typedef struct sc_session sc_session;
sc_session *sc_session_new(int argc, const char *const *argv); /* flags de sessão, §3.5 */
int sc_session_cell(sc_session *s, const char *src, unsigned len, int codegen);
int sc_session_undo(sc_session *s);                             /* desfaz a última célula aceita */
const unsigned char *sc_result_ptr(sc_session *s, int part);
unsigned sc_result_len(sc_session *s, int part);
int sc_main(int argc, const char **argv);                       /* o engine de hoje */

/* protocolo do filho nativo: cabeçalho de cada mensagem nos pipes */
enum sc_msg { SC_MSG_NEW = 1, SC_MSG_CELL = 2, SC_MSG_UNDO = 3, SC_MSG_REPLY = 4 };
struct sc_msg_header { unsigned kind, status, len[3]; };
```

- **Entradas do engine:**
  - o `main` de `build/sc-engine` passa a ser `engine/sc_entry.cpp`, com
    `sc_main` para `-cc1`/`ld` e o laço do protocolo para `session`;
  - no wasm, o `_start` chama o mesmo `sc_main`.
- **Diagnósticos:**
  - a sessão usa um `TextDiagnosticPrinter` sobre uma string;
  - o que vai direto para o fd 2 (`sc_cut_called`, "LLVM ERROR", lld em
    erro fatal) o pai lê do stderr do filho (nativo) ou do stderr WASI (wasm);
  - os nomes `input_line_N` e a nota "In file included from `<<< inputs >>>`"
    viram o nome que o REPL dá à célula, por um mapeamento no printer (C-M1).

## 3. A sessão (candidata A)

### 3.1 Montagem (C-B3, C-m1, C-m3)

- **Nada de `clangInterpreter` nem de `IncrementalAction`.** Para `EmitObj`,
  o `IncrementalAction` embrulha um `EmitLLVMOnlyAction`. Ele chamaria o
  `emitBackendOutput` com `Backend_EmitNothing`, que o `sc_backend.cpp`
  rejeita.
- **`engine/sc_session.cpp`:**
  - um `ASTFrontendAction` mínimo, com `TU_Incremental`, `EndSourceFile`
    vazio e `CurContext = nullptr` depois do `ExecuteAction`;
  - o consumer é um `CreateLLVMCodeGen` puro;
  - um arquivo principal vazio, `<<< inputs >>>`, remapeado;
  - depois do `ParseAST` inicial, o primeiro módulo é guardado vivo (o
    CodeGen guarda referências a ele) e entra um `StartModule`.
- **Por célula, o núcleo do `IncrementalParser` copiado** (`Parse`,
  `ParseOrWrapTopLevelDecl`, `CleanUpPTU`):
  - ~310 linhas com a ação;
  - commit de origem no cabeçalho;
  - conferido a cada troca de pin com `git diff` dos arquivos de origem. A
    troca para 23.1.3 está pendente.
- **Depois do parse:** `GenModule` (`ReleaseModule` + `StartModule`), o
  pós-passo do §3.2 e `emitBackendOutput(..., Backend_EmitObj, ...)` direto
  no módulo da PTU. Os passes por módulo e a checagem de data layout
  funcionam assim.
- **Link:** o `lld` lê e escreve arquivos (R5).
  - A sessão tem um diretório temporário próprio: `/tmp` no nativo, um
    preopen gravável no wasm, um FS em memória no shim WASI do navegador.
  - O `.so` é lido de volta para o resultado.
  - Se o `lldMain` devolve `canRunAgain=false`, o status é `SC_LOST`.

### 3.2 Estado interno duplicado em cada célula (C-B1, bloqueador da versão 1)

**O problema.** No modo incremental, o CodeGen reemite em toda PTU as
declarações com ligação interna: `static`, namespace anônimo e `static`
local de função `static`. Cada `.so` ganha uma cópia própria. Confirmado no
`clang-repl-17`: `static int cnt` incrementado numa célula aparece como 0 na
seguinte.

**A correção** é um pós-passo de IR no engine, sem patch:

1. Na primeira vez que uma global interna ligada a uma declaração aparece
   (`CodeGenerator::GetDeclForMangledName`), ela é promovida a externa com
   visibilidade default, sob um nome único da sessão.
2. A sessão guarda o conjunto dos nomes promovidos. Nos módulos seguintes,
   uma cópia reemitida vira declaração (`deleteBody`, sem inicializador).
3. Continuam internos `__cxx_global_var_init*`, `__stmts__*`,
   `_GLOBAL__sub_I_*` e `.str*`.

Testes: os casos `cnt`, `nx` (`static` local de função `static`) e `an`
(namespace anônimo), em C e C++, no `repl-`.

### 3.3 Célula com erro (C-B2, C-M4)

- **O problema.** O CodeGen só para em erro *irrecuperável*. As declarações
  vistas antes do erro vazam para o módulo corrente.
- **Ao falhar:**
  1. `Consumer->HandleTranslationUnit` (vê o erro, faz `clear` e `M.reset`);
  2. `StartModule`;
  3. só então `CleanUpPTU` e `Diags.Reset`.

  Status `SC_ERROR`.
- **Desfazer depois do codegen** (link ou carga falhou, ou `undo` pedido
  pelo aluno): o `CleanUpPTU` restaura só o lookup. O estado do CodeGen e as
  especializações não voltam. Então `undo` de uma célula já gerada é
  `SC_LOST` + repasse sem ela. O `undo` barato existe só para a célula que
  falhou.

### 3.4 Semântica do modo incremental (C-m2)

**O que muda em relação a um TU normal:**

- O fim de TU roda por célula, e as definições tentativas são completadas
  por célula. Em C, `int x;` seguido de `int x = 5;` vira duas definições
  fortes, e o loader esconde a primeira.
- `UnusedFileScopedDecls` não é limpo: um `static` não usado é avisado de
  novo em toda célula.
- Uma expressão final sem `;` é aceita em silêncio.

**Decisões para a v1:**

- O aviso de não usado só sai na célula que define a declaração. Isso é
  filtrado no printer, pela PTU de origem.
- `warn_unused_expr` fica ignorado, como no `Interpreter::Parse`.
- Uma expressão sem `;` dá um erro do sc: "falta `;`". A impressão de valor
  fica fora da v1.

**Redefinição (C-M3, T-M7).**

- O clang rejeita a redefinição, e o upstream não a suporta (o clang-repl
  usa `%undo`).
- Remover do lookup não basta. O `DeferredDecls` do CodeGen é indexado pelo
  nome mangled e emitiria o corpo antigo.
- **Decidido: a v1 rejeita.** A mensagem do clang ganha uma nota do sc que
  sugere `undo` da célula anterior. Esse `undo` de uma célula já gerada é
  `SC_LOST` + repasse sem ela (§3.3).
- Aceitar a redefinição fica para depois. Seria um patch que apaga o nome de
  `DeferredDecls` e `EmittedDeferredDecls` e a declaração do lookup.

### 3.5 Flags da sessão (C-m1, T-m3)

- `scParseArgs` hoje exige um arquivo e deduz C/C++ pela extensão. A sessão
  ganha `-x c`/`-x c++` explícito e dispensa o arquivo.
- A sessão força:
  - PIC;
  - `-fvisibility=default`;
  - `-fincremental-extensions`;
  - `DisableFree=false`;
  - `ClearASTBeforeBackend=false`.
- O `checkinv` ganha as combinações incrementais: c, c++, pic e c++ pic
  mod. O `TU_Incremental` não é flag de cc1, então fica fora do `checkinv`
  e coberto pelos testes de sessão.

### 3.6 Repasse depois de `SC_LOST` (C-B4, R6, T-M4)

**Sem codegen não funciona.** O `moveLazyEmissionStates` carrega de um
módulo para o outro:
- `DeferredDecls`;
- `MangleCtx`;
- `Manglings`;
- as vtables adiadas.

Sem esse estado, funções `_LIBCPP_HIDE_FROM_ABI` usadas depois viram só
declarações, e elas não são exportadas pelo `.so` antigo.

**O repasse:**

1. O host guarda o **log exato**: todas as células, inclusive as que falharam,
   e os undos.
2. Sessão nova. O log é reaplicado com codegen completo e com o pós-passo do
   §3.2 (reconstrói os nomes promovidos), mas sem backend e sem link
   (`codegen=1`, com o objeto descartado).
3. Para cada célula aceita, o conjunto dos símbolos exportados (`SC_SYMS`)
   tem que bater com o gravado na compilação original. Se não bater, o REPL
   pede para reiniciar a sessão.
4. O `CrashRecoveryContextCleanupRegistrar<Sema>` copiado é removido: no filho
   não há recuperação dentro do processo.

### 3.7 Bugs do clang-repl e o pin 23.1.3

Decidido em 2026-09-30: o pin vai para `llvmorg-23.1.3` (previsto para
~2026-10-06) **antes** do trabalho de REPL, num commit próprio. A versão
traz três correções de que a sessão precisa:

| bug | upstream | no plano |
| --- | --- | --- |
| vtable duplicada entre PTUs | #185648, no 23.1.3 | vem com o pin |
| `Undo` não desliga a PTU | #213235, no 23.1.3 | vem com o pin |
| entrada que falha apaga declarações anteriores | #218149, no release/23.x depois do 23.1.2 | vem com o pin |
| reemissão de `static` | aberto | §3.2 (pós-passo de IR, sem patch) |
| entrada que falha vaza IR | #226252, aberto | §3.3 |
| `#include` depois de uma entrada que falhou ou de `undo` | aberto | teste `seq-` + correção nossa (patch se preciso) |
| `return` no topo derruba o compilador | aberto | teste `seq-` + diagnóstico nosso antes do CodeGen |

A cópia do `IncrementalParser` (§3.1) é feita a partir do 23.1.3, não do pin
atual.

## 4. Memória e estado global

- **Nativo:** a memória da sessão é do filho. Uma válvula (RSS acima de um
  limite, por exemplo 1 GB) leva a matar o filho e fazer o repasse.
- **Wasm:** a mesma válvula sobre o `memory.size`. Recuperar é descartar o
  `Store` ou o Worker.
- **One-shot (`sc_main`):** continua com `-disable-free` e `BuryPointer`.
  Por isso os hosts garantem **uma instância por compilação one-shot**
  (T-M6). Nada de reaproveitar o reactor para compilações avulsas.
- **Estado global** (R11, verificado):
  - `LLVMInitialize*` é idempotente;
  - o `lld` faz `Ctx::reset` em todo link;
  - o assert do `install_fatal_error_handler` não existe no build.
  - Dentro de uma sessão, o que importa é o lld depois de erro fatal
    (`canRunAgain`), já tratado como `SC_LOST`.
- **Critérios mensuráveis** (T-m4):
  - `mem-`: 1.000 células pequenas. Depois de aquecer, a inclinação do RSS
    tem que ficar abaixo de 50 KB/célula e a do `memory.size` abaixo de
    64 KB/célula. Os limites são revistos com o primeiro dado.
  - Memória do `lld` por link: RSS antes e depois de 100 links seguidos,
    diferença abaixo de 5 MB.

## 5. Fases

**Regra geral (T-B1):** o desenvolvimento das fases 0 a 3 roda sobre os
engines **sem cortes** (`sc-host-full`, `sc-full.wasm`). O engine cortado
só entra na fase 4, depois do aprendizado.

| fase | entrega | critério de saída |
| --- | --- | --- |
| P. pin | `llvmorg-23.1.3`: rebase de `llvm/*.patch`, rebuild, `make learn`/`test`/`test-warn`/`checkinv`, tamanho no ANALISE | os critérios de sempre. As fases 0 em diante só começam depois dela |
| E+N. exceções e padrões | §1.5 e §1.6 completos, no fluxo one-shot de hoje (antes da sessão, porque mudam os cortes e as bases). Pode começar antes da P, mas o `learn` é refeito depois dela | `make test`, `test-warn`, `test-native` e `checkinv` 0 diferenças contra as bases novas; programas com exceções e `setjmp` rodando pelo `scload` (wasmtime) e no V8; tamanho registrado no ANALISE |
| 0. spike | as candidatas A e B (§1.3) mínimas sobre o `sc-host-full`; programas de `test/cxx` e `test/samples` cortados automaticamente em uma célula por declaração do topo, ligados e rodados pelo `scload`; uma célula fatal → repasse → célula seguinte | A: saída igual à do `pic-`; símbolos da célula após o repasse iguais aos sem crash. B: saída igual, sem reemitir inicializadores. Escolha registrada no ANALISE.md (tempo por célula, complexidade, memória) |
| 1. API | `sc.h`, `sc_entry.cpp`, `sc_main` | `make test`, `test-native`, `test-warn` e `checkinv` sem diferença |
| 2. sessão | §3 completo no `sc-host-full`; `build/sc-engine session` (laço do protocolo); um modo de teste `build/sc-engine repl <células...>` que grava os `.so` | `split-` (abaixo) igual ao `pic-` no engine sem cortes |
| 3. testes | as fatias `ses-`, `split-`, `seq-`, `crash-` e `mem-` no `corpus.py` (§6) | todas verdes no engine sem cortes |
| 4. cortes | `make learn` e `learn-native` dirigindo o `build/sc-engine repl` com as fatias novas; `test-warn` delas | `make test` 0 diferenças com as fatias novas; tempo do learn medido; tamanho dentro do orçamento (§5.1) |
| 5. reactor | `sc.wasm` reactor + `_start`; as exports | antes de trocar, congelar `res/base/summary.json`; depois, o `sc-full.wasm` novo reproduz a base hash por hash (T-B2) |
| 6a. loader mínimo | o loader com as features mínimas do §1.4, exceções ligadas e tags compartilhadas, `epoch_interruption` | `pic-` e `repl-` com a mesma saída; tamanho do loader registrado |
| 6b. hosts | crate `sc-proto` (bindgen sobre `sc.h`), binário único `build/sc` (engine ligado pelo `--emit-link`), filho `sc --engine`, timeout, válvula, repasse; host JS documentado | `crash-` pelo REPL nativo: o REPL segue vivo e a célula seguinte funciona |
| 7. docs | ANALISE.md (§20), README, AGENTS (alvos novos), Makefile (`.PHONY`, `clean` de `res/ses*` etc.) | — |

### 5.1 Orçamento de tamanho (T-m1)

- No máximo **+1% em zstd** sobre os 5.452.450 do `sc.wasm`.
- Acima disso, o plano passa a gerar dois engines, one-shot e sessão, a
  partir dos mesmos objetos.

## 6. Testes

| fatia | o que é | oráculo |
| --- | --- | --- |
| `ses-` | **todos** os arquivos do corpus, inclusive os com erro (exercitam `CleanUpPTU`), como uma célula numa sessão | base do mesmo modo no engine sem cortes |
| `split-` | os ~1.743 programas executados, cortados em uma célula por declaração do topo | **critério principal:** saída e código de saída iguais aos do `pic-` one-shot (T-M3, C-M2) |
| `seq-` | sequências geradas: [erro, boa], [boa, undo, boa], [`static`/namespace anônimo entre células], [`import std;` depois de `#include`] | saída esperada fixa |
| `crash-` | os 36 fatais e os estouros de pilha como células, seguidos de uma célula boa | o REPL vivo; a célula boa funciona; os símbolos após o repasse batem com os sem crash |
| `repl-` | o atual (`.so` explícitos, testa a redefinição no loader) | sem mudança |
| `mem-` | 1.000 células | limites do §4 |

- O `make learn` passa a dirigir o mesmo `build/sc-engine repl`, então os caminhos
  raros (`CleanUpPTU`, undo, repasse) entram no `symbols.used` (T-M1).
- O `ses-` usa uma amostra dos testes do clang, não o corpus inteiro, para
  não dobrar o tempo do learn (T-M2).

## 7. Decisões do professor

1. Redefinição na v1: **rejeitar** (§3.4).
2. Pacote: **um binário só, `sc`**, com o REPL, o loader e o engine; o filho é `sc --engine` (§1.1).
3. Padrão da linguagem: **os mais novos**, gnu23 e gnu++26 (§1.6).

## 8. Fora do escopo deste plano (T-M8)

O front end do REPL:
- divisão das células;
- quando roda `main`;
- stdin;
- ordem entre statements do topo e `constructor`;
- política quando o código do aluno dá trap;
- interrupção de laço infinito: no nativo, `epoch_interruption` do wasmtime; no navegador, `check_interrupt` (§7.2) ou matar o worker.

Eles afetam a premissa de que "o estado do aluno vive no loader" e precisam
de um plano próprio antes da fase 6.
