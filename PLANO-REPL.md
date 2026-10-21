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
  Nesse modo o wasmtime nem é iniciado. **Decidido na fase 2: o laço é em
  Rust**, sobre a API segura (`sc_proto::serve`), e o lado do pai também
  (`sc_proto::Child` e `Repl`, com o log e o repasse do §3.6). Assim o laço
  não é escrito duas vezes: o `sc --engine` da fase 6b usa o mesmo código
  que o `build/sc-engine-full` de hoje.
- **Nomes no build:**
  - o crate do `loader/` passa a gerar o `sc`, que deixa de se chamar
    `scload`;
  - o executável só de C++ de hoje passa a ser `build/sc-engine` (cortado) e
    `build/sc-engine-full` (sem cortes). Ele existe só para o `make learn` e
    para os testes;
  - `build/sc` passa a ser o binário entregue;
  - o wasm continua `sc.wasm`.
- Antes de cada célula o filho faz `fork` de si mesmo (§3.3). Se a célula
  falha ou derruba o compilador, a cópia segue no lugar dele, sem repasse.
  Timeout ou Ctrl-C matam o filho. O REPL lança outro e faz o repasse
  (§3.6).
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

**Bindgen.** O crate `sc-proto` (`proto/`) roda o bindgen sobre `engine/sc.h` no build:

- Gera os tipos do protocolo: o enum de status, os códigos de mensagem e o
  cabeçalho de cada mensagem.
- O REPL nativo e o host Rust de testes do `sc.wasm` usam esses mesmos tipos.
- Nenhuma constante nem layout é mantido à mão.
- As declarações das funções também saem dele. Com a feature `engine`, o
  crate liga o engine e dá uma API segura sobre elas (`Session`, `main`;
  ANALISE.md §23). As chamadas só acontecem no processo filho.

### 1.2 Wasm: `sc.wasm` vira reactor que também exporta `_start` (R8, R9, T-B2)

- **Reactor e comando no mesmo módulo.** `-mexec-model=reactor`, mais um
  `_start` nosso que chama `__main_void`. Um experimento confirmou que o
  módulo roda como comando e como biblioteca (exports).
- **Testes inalterados.** O corpus (`wasmtime run sc.cwasm`), o `link.py
  --learn` e as bases **não mudam**.
- **Exports:** a lista atual está no §5.2, passo 3.
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

**Decidido na fase 0 (2026-10-13): A.** Ver ANALISE.md §22.

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
- O protótipo `scload` tinha 26,7 MB, com as features padrão do wasmtime
  (27,5 MB no início da fase 6a). O conjunto final está no §5.3. O `sc` final ≈ 24,5 MB do engine + 5,7 MB do runtime. Medido na fase 6a, o `scload` com o perfil final tem 8,1 MB, e não 5,7 MB (ANALISE.md §49).
- O `sc` acrescenta o `sc-proto`, o `bpaf`, o `rustyline`, o `dirs` e a
  `libc` (0,9 MB, ANALISE.md).

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
  SC_OK = 0,    /* pronto */
  SC_ERROR = 1, /* erro no código do aluno; a sessão volta ao estado anterior à célula */
  SC_LOST = 2,  /* erro fatal, falha de link depois do codegen ou undo:
                   a sessão deve ser recriada e repassada */
};
enum sc_part { SC_DIAGS = 0, SC_SO = 1, SC_SYMS = 2 /* exports do .so, para o repasse */ };
enum sc_cell_mode { SC_CELL_SO = 0, SC_CELL_REPLAY = 1 /* só IR, para o repasse */ };

typedef struct sc_session sc_session;
/* flags de sessão (§3.5), -x c|c++, depois de "--" os argumentos do wasm-ld;
   *out sempre recebe a sessão, também em SC_ERROR (para ler SC_DIAGS) */
int sc_session_new(int argc, const char *const *argv, sc_session **out);
void sc_session_free(sc_session *s);
int sc_session_cell(sc_session *s, const char *src, unsigned len, int mode);
int sc_session_undo(sc_session *s);                             /* SC_LOST (§3.3) */
const unsigned char *sc_result_ptr(sc_session *s, int part);
unsigned sc_result_len(sc_session *s, int part);
int sc_main(int argc, const char **argv);                       /* o engine de hoje */

/* protocolo do filho nativo: cabeçalho de cada mensagem nos pipes */
enum sc_msg { SC_MSG_NEW = 1, SC_MSG_CELL = 2, SC_MSG_UNDO = 3, SC_MSG_REPLY = 4 };
struct sc_msg_header { unsigned kind, status, len[3]; };
```

Feito na fase 1 (2026-10-13; ANALISE.md §23). Em relação ao rascunho:
`sc_session_new` devolve o status, `sc_session_free` existe, o `codegen` virou
`mode`, e o engine recebe os argumentos do link em vez de deduzir onde estão
a libc e os builtins.

- **Entradas do engine:**
  - o `main` dos executáveis só de C++ é `engine/sc_entry.cpp`, que chama
    `sc_main` (`-cc1`/`ld`);
  - o `build/sc-engine-full` (fase 2) é Rust (`proto/src/bin/sc-engine.rs`):
    `-cc1`/`ld` vão para o `sc_main`, `session` é o filho e `repl` é o
    teste do lado do pai;
  - no wasm, o `_start` chama o mesmo `sc_main`.
- **O protocolo** (comentário do `sc.h`): o cabeçalho na ordem de bytes do
  host e as três partes. `SC_MSG_NEW` leva os argumentos separados por NUL,
  `SC_MSG_CELL` a fonte (o modo vai em `status`), e cada pedido recebe um
  `SC_MSG_REPLY` com as partes na ordem do `enum sc_part`. O filho acaba no
  fim da entrada e escreve o protocolo numa cópia do stdout; o fd 1 passa a
  ser o stderr, para que nada do engine quebre uma mensagem.
- **Diagnósticos:**
  - a sessão usa um `TextDiagnosticPrinter` sobre uma string;
  - o que vai direto para o fd 2 (`sc_cut_called`, "LLVM ERROR", lld em
    erro fatal) o pai lê do stderr do filho (nativo) ou do stderr WASI (wasm);
  - os nomes `input_line_N` e a nota "In file included from `<<< inputs >>>`"
    viram o nome que o REPL dá à célula, por um mapeamento no printer (C-M1).

## 3. A sessão (candidata A)

**Fase 2 (2026-10-14; ANALISE.md §24).** O `split-` (`make test-split`)
iguala o `pic-` em 1.941 dos 1.947 programas do corpus. Os 6 restantes são
limites conhecidos (`KNOWN` no `test/split.py`). Para chegar lá, a sessão
ganhou quatro correções (§3.8) e o loader, as referências para a frente.
Os itens que faltavam foram fechados em 2026-10-17: o §3.4 (o aviso de não
usado pelo §29 do ANALISE.md; a redefinição, sem nota), as combinações
incrementais no `checkinv` (§3.5) e o `return` no topo, por um patch no
Sema (§3.7). O `#include` depois de uma
célula que falhou foi resolvido pelo snapshot por `fork` (§3.3).

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
- **Isso não basta** (fase 2). O `CleanUpPTU` deixa restos de cabeçalhos
  lidos pela célula que falhou:
  - as guardas de inclusão continuam definidas (o `#include` seguinte não
    lê nada);
  - o `Sema` guarda o `namespace std` da célula;
  - o `operator new` implícito e enumeradores continuam no lookup.

  Tapar cache por cache não termina. **Decidido (2026-10-14): snapshot por
  `fork`.** No `serve` (`proto/src/engine.rs`), antes de cada célula
  `SC_CELL_SO`, o filho faz `fork`. A cópia espera num pipe. Se a célula é
  aceita, a cópia acaba. Se falha, o original passa a resposta à cópia e
  acaba, e a cópia segue com a sessão de antes da célula. Se o original
  morre na célula (crash), a cópia responde `SC_ERROR` com "the engine
  crashed" e segue. O log do `Repl` guarda só as células aceitas. O pid do
  filho muda a cada erro. No navegador o equivalente é copiar a memória do
  wasm antes da célula (fase 6b).
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
- Uma expressão final sem `;` é aceita em silêncio, como um valor descartado.

**Decisões para a v1:**

- ~~O aviso de não usado só sai na célula que define a declaração. Isso é
  filtrado no printer, pela PTU de origem.~~ Mudou no protótipo (ANALISE.md
  §29): a sessão esvazia o `UnusedFileScopedDecls` e o `UndefinedButUsed` a
  cada célula, porque uma célula seguinte pode usar ou definir o nome; no
  REPL o fim de célula não avisa nada. `sc FILE` e `sc check` usam
  `-sc-one-cell` e avisam como o compilador de uma vez só.
- ~~`warn_unused_expr` fica ignorado; uma expressão sem `;` dá "falta
  `;`" e a impressão de valor fica fora da v1.~~ Mudou no protótipo
  (ANALISE.md §26): a expressão sem `;` mostra o valor, se for escalar, e
  não é mais um valor descartado (`llvm/incremental-stmts.patch`), então
  o `warn_unused_expr` voltou.
- Fica para depois mostrar struct, union, array e enum pelo nome. O motor
  passa a descrição do tipo (campos, offsets, tamanhos, enumeradores) e o
  endereço de uma temporária, e o `sc` lê a memória do wasm, sem gerar
  código na célula. Os tipos da biblioteca do C++ (`std::string`,
  `vector`) ficam fora: o formato deles é da libc++.

**Redefinição (C-M3, T-M7).**

- O clang rejeita a redefinição, e o upstream não a suporta (o clang-repl
  usa `%undo`).
- Remover do lookup não basta. O `DeferredDecls` do CodeGen é indexado pelo
  nome mangled e emitiria o corpo antigo.
- **Decidido: a v1 rejeita.** ~~A mensagem do clang ganha uma nota do sc que
  sugere `undo` da célula anterior.~~ Decidido em 2026-10-17: sem nota. O
  erro do clang já aponta a definição anterior ("previous definition is
  here", com a `cell_N`), e o `:undo` só desfaz a última célula, então a
  nota só serviria num caso. O `:undo` está no `:help`.
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
  e coberto pelos testes de sessão. Feito em 2026-10-17 (ANALISE.md §44):
  o `checkinv` roda os argumentos das 12 sessões (`build/repl/*.args`), e
  o que a sessão força está em `scIncremental`.

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

1. O host guarda o log das células aceitas, menos as desfeitas. Uma célula
   que falhou não deixa nada no filho (§3.3).
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
| entrada que falha vaza IR | #226252, aberto | §3.3 (snapshot por `fork`) |
| `#include` depois de uma entrada que falhou ou de `undo` | aberto | resolvido: snapshot por `fork` (§3.3); o `undo` já era repasse |
| `return` no topo derruba o compilador | aberto | resolvido: `llvm/repl-outside-function.patch` dá o erro no Sema (ANALISE.md §43) |
| `static` local e lambda num comando de topo derrubam o compilador (C++) | aberto | resolvido: `llvm/repl-outside-function.patch` (ANALISE.md §45) |
| entrada que termina dentro de um `{`, `(` ou `[`, ou num `_Pragma(`, trava o compilador | aberto | resolvido: `llvm/incremental-input-end.patch` (ANALISE.md §46) |
| cada entrada tem o seu namespace anônimo | aberto | resolvido: `llvm/incremental-anon-namespace.patch` (ANALISE.md §46) |
| `B::C::C() {}` com `C` privada de `B` dá erro de acesso | aberto | resolvido: `llvm/incremental-access.patch` (ANALISE.md §46) |

A cópia do `IncrementalParser` (§3.1) é feita a partir do 23.1.3, não do pin
atual.

### 3.8 O que o `split-` achou (fase 2)

- **C: o escopo de bloco vazava** (bug do upstream). No modo incremental,
  `Sema::ActOnPopScope` não tira nome nenhum do `IdResolver` em C, de
  nenhum escopo. A intenção é manter os nomes do topo entre as PTUs, mas
  parâmetros, locais e rótulos também ficavam. `int f(int f)` dava
  "redefinition", e um rótulo ou um nome repetido em blocos diferentes
  dava erro. O `llvm/incremental-c-scopes.patch` limita isso ao escopo do
  TU, o único sem pai.
- **Função sem protótipo** (C antes do C23). A chamada declara a função
  como variádica. No TU inteiro a definição troca essa declaração; na
  célula seguinte não. O pós-passo dá à declaração o tipo LLVM da definição
  de uma célula anterior.
- **`static` usado antes da definição.** A declaração usa o nome promovido
  (`f.sc`), e a célula que define a função a emite mesmo sem uso nela, como
  o uso no TU inteiro faria.
- **`inline` com `always_inline`** (`available_externally`). O backend
  descarta essa definição, então ela não é exportada. Depois de cada célula
  ela volta para os adiados do CodeGen, para ser emitida de novo onde outra
  célula a usar.
- **Os erros do backend** (`__builtin_return_address`) vão para os
  diagnósticos da sessão, como no one-shot, e a célula dá `SC_LOST`.
- **No loader**, uma célula pode usar o que uma célula posterior define,
  como uma declaração antes da definição num arquivo:
  - a entrada `GOT.mem` é preenchida quando o símbolo chega;
  - uma função ainda sem definição ganha na hora uma entrada da tabela, que
    a definição preenche (o mesmo endereço para todos os módulos);
  - uma chamada direta passa por um stub em wasm com `call_indirect` para
    essa entrada. Os trampolins do host acabavam com a pilha nativa numa
    recursão entre células.

### 3.9 O que as fatias da fase 3 acharam

As fatias `ses-`, `seq-`, `crash-` e `mem-` estão no `test/session.py`
(`make test-session`), e não no `corpus.py`: rodam sobre o
`sc-engine repl`, que só existe no nativo, e o `corpus.py` roda qualquer
engine one-shot. Na fase 4 o mesmo script roda sobre o engine cortado,
contra o `summary.json` do sem cortes. O `ses-` achou três bugs do
clang-repl (tabela do §3.7). A lista do que não pode ser igual ao
compile inteiro, com o motivo, está no `KNOWN` do script (ANALISE.md
§46).

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
    64 KB/célula. Os limites são revistos com o primeiro dado. Medido na
    fase 3 (nativo): 2,6 KB/célula em C e 5,2 KB/célula em C++. O
    `memory.size` espera o reactor (fase 5).
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
| 1. API | `sc.h`, `sc_entry.cpp`, `sc_main`; `sc_api.cpp` (a sessão da API sobre o spike A) e o crate `sc-proto` com a API segura em Rust (`make test-proto`) | `make test`, `test-native`, `test-warn` e `checkinv` sem diferença; `test-proto` verde |
| 2. sessão | §3 completo no `sc-host-full`; `build/sc-engine-full session` (laço do protocolo, em Rust); um modo de teste `build/sc-engine-full repl <dir> <células...> -- <args>` que grava os `.so` e faz o repasse | `split-` (`make test-split`) igual ao `pic-` no engine sem cortes, fora os `KNOWN` |
| 3. testes | as fatias `ses-`, `split-`, `seq-`, `crash-` e `mem-` no `corpus.py` (§6) | todas verdes no engine sem cortes |
| 4. cortes | `make learn` e `learn-native` dirigindo o `build/sc-engine repl` com as fatias novas; `test-warn` delas | `make test` 0 diferenças com as fatias novas; tempo do learn medido; tamanho dentro do orçamento (§5.1) |
| 5. reactor (feita, §5.2) | `sc.wasm` reactor + `_start`; as exports | antes de trocar, congelar `res/base/summary.json`; depois, o `sc-full.wasm` novo reproduz a base hash por hash (T-B2) |
| 6a. loader mínimo (feita, §5.3) | o loader com as features mínimas do §1.4, exceções ligadas e tags compartilhadas, `epoch_interruption` | `pic-` e `repl-` com a mesma saída; tamanho do loader registrado |
| 6b. hosts (nativo feito, ANALISE.md §50) | crate `sc-proto` (bindgen sobre `sc.h`), binário único `build/sc` (engine ligado pelo `--emit-link`), filho `sc --engine`, timeout, válvula, repasse; host JS documentado | `crash-` pelo REPL nativo: o REPL segue vivo e a célula seguinte funciona |
| 7. docs e distribuição | ANALISE.md (§20), README, AGENTS (alvos novos), Makefile (`.PHONY`, `clean` de `res/ses*` etc.); o `sc` fora do checkout (ANALISE.md §51): os `.args` sem caminhos absolutos, relativos a uma raiz, e a escolha entre uma árvore ao lado do binário e o sysroot embutido | o `sc` copiado para fora do `build/` roda |

### 5.1 Orçamento de tamanho (T-m1)

- No máximo **+1% em zstd** sobre os 5.452.450 do `sc.wasm`.
- Acima disso, o plano passa a gerar dois engines, one-shot e sessão, a
  partir dos mesmos objetos.
- Os 5.452.450 eram o `sc.wasm` antes da fase E+N. A base da fase 5 é o
  `sc.wasm` da fase 4, com 5.641.258 em zstd (ANALISE.md §47). Pela
  estimativa do §47, a sessão no wasm custa +0,47% sobre o `sc.wasm` de
  antes da fase 4.

### 5.2 Fase 5 em detalhe: o reactor

**Entrega.** O `sc.wasm` (e o `sc-full.wasm`) passa a ser um módulo só:
- roda como comando (`wasmtime run`), exatamente como hoje;
- exporta a sessão do `sc.h` para um host.

Um host nativo de teste, o `build/sc-engine-wasm`, dirige essas exports com
as mesmas fatias da fase 3.

**Fora da fase 5:**
- o host do navegador (Worker, JS);
- a válvula de memória;
- o REPL usar o `sc.wasm`;
- o snapshot de memória para o navegador (passo 6 só mede).

Tudo isso é da fase 6.

**O que não muda:**
- o `build/sc` one-shot nativo e o `build/sc-engine`, nem os objetos deles;
- o `corpus.py`, com os mesmos argumentos;
- o `link.py --learn`, que segue rodando o corpus one-shot.

#### Passo 1. O reactor com `_start` (já verificado)

A revisão do plano verificou, com o wasi-sdk 29, o `wasm-ld` do pin e o
wasmtime 48:
- um módulo ligado com `-mexec-model=reactor` e com o nosso `_start`, que
  faz `exit(__main_void())`, roda como comando. A saída e os códigos de
  saída (0 e 3) são os de antes;
- o `wasmtime run` chama o `_initialize` antes do `_start`, também com
  `--invoke` e com um `.cwasm`, e o construtor global roda uma vez;
- o `_initialize` do `crt1-reactor.o` tem uma guarda: uma segunda chamada
  cai num `unreachable`. Todo host chama o `_initialize` uma vez por
  instância.

**O `node:wasi` recusa** um módulo que exporta o `_initialize` e o `_start`.
O `wasi.start` exige que o `_initialize` não exista, e o `wasi.initialize`
exige o contrário. Isso quebra o `bench/cc.mjs`. O contorno, verificado, é
passar um objeto no lugar da instância:
- comando: `wasi.start({exports: {memory, _start() { e._initialize(); e._start(); }}})`;
- biblioteca: `wasi.initialize({exports: {memory, _initialize: e._initialize}})`.

O contorno fica documentado no README, e o `bench/cc.mjs` passa a usá-lo.

**Alternativa sem o reactor, também verificada.**
- O módulo é ligado com `-nostartfiles` e exporta um `sc_init`, que chama
  o `__wasi_init_tp` e o `__wasm_call_ctors` uma vez.
- O nosso `_start` chama o `sc_init` e depois faz `exit(__main_void())`.
- Assim o `wasi.start` do Node funciona sem mudança.

A recomendação é o reactor padrão, porque não reimplementa o crt. A
alternativa fica registrada.

#### Passo 2. Congelar as bases

As bases dependem dos engines no Makefile. Religar um engine regera a base
dele, e o critério do T-B2 se perde. Antes de qualquer mudança, as bases
vão para `build/res/<nome>-pre5/`, e os hashes dos `summary.json` vão
para o ANALISE. São seis bases:
- `base` e `wev-base` (`sc-full.wasm`);
- `native-base` e `native-wev-base` (`sc-host-full`);
- `session-base` e `session-wev-base` (`sc-engine-full`).

Os critérios comparam com essas cópias.

**O que não pode mudar.** O plano não muda os objetos nativos:
- o `_start` vai para um arquivo só do wasm, o `engine/sc_start.cpp`, e não
  para o `sc_entry.cpp`;
- a nota do diretório temporário vai para o README, e não para o `sc.h`,
  que todo objeto inclui;
- a mudança no `proto/` (passo 4) muda o código do `sc-engine-full`, e as
  bases da sessão são comparadas com as congeladas para mostrar que ele não
  mudou de comportamento.

#### Passo 3. O módulo

1. **Objetos.**
   - O wasm ganha uma lista própria no Makefile:
     `WENGINE := $(ENGINE) sc_session sc_api sc_start`.
   - O `WENGINE` vale nas regras `engine`, `sc-full.wasm`, `sc-cut.wasm` e
     `learn`. O `ENGINE` continua sendo a lista do nativo.
   - A regra `$(B)/engine/%.o` passa a depender do `engine/sc_session.h`.
   - O `link.py` tem a lista no código (`ENGINE = [...]`). Sem `--native`,
     ela ganha os três objetos.
2. **Modelo.**
   - O `link.py` repete o comando de link do clang. Com
     `-mexec-model=reactor` nos `flags` do wasm, o modelo chega a todo link:
     os stubs, o trace e o link final.
   - O `sc_start.cpp` tem o `_start` do passo 1.
3. **Exports.** São ligadas com `-Wl,--export=` a partir de uma lista
   única, no `link.py`:
   - `sc_session_new`, `sc_session_free`, `sc_session_cell`,
     `sc_session_undo` e `sc_session_depth`;
   - `sc_result_ptr` e `sc_result_len`;
   - `malloc` e `free`, para o host pôr o argv e o fonte na memória;
   - `chdir` (do wasi-libc). O guest começa no `/` qualquer que seja o cwd
     do host (verificado), e a sessão acha os `#include "..."` pelo cwd: o
     `session.py` roda cada sessão no diretório do fonte. O host chama o
     `chdir` depois do `_initialize`;
   - o `memory`, que já é exportado.

   O `sc_main` não é exportado: o one-shot entra pelo `_start`. A lista
   substitui a do §1.2, que era anterior ao `sc.h`.

   A revisão confirmou que o wrap e o `noop:` não alcançam as exports: os
   candidatos vêm só dos membros `.obj` do LLVM. As exports viram raízes
   do `--gc-sections`, e os nomes delas passam pelo `wasm-opt`.
4. **Diretório temporário.**
   - O `sc_session_new` cria o diretório das células pelo
     `system_temp_directory` do LLVM. No WASI, esse diretório é o `TMPDIR`
     (ou `TMP`, `TEMP`, `TEMPDIR`), ou então o `P_tmpdir` do wasi-libc,
     que é o `/tmp`.
   - O host dá ao guest um `/tmp` gravável: `--dir` no wasmtime, e no
     navegador um sistema de arquivos em memória. Isso vai para o README.

**Critério do passo 3:**
- O `sc-full.wasm` novo reproduz `base-pre5` e `wev-base-pre5` hash por
  hash.
- `make test` e `test-warn` dão 0 diferenças no `sc.wasm` cortado.
- `checkinv` imprime SAME.
- Os objetos nativos não mudam: o `make -n` do `sc-host-full` não religa
  nada.

#### Passo 4. O host de teste: `build/sc-engine-wasm`

As fatias da fase 3 são o oráculo. Para rodá-las sem mudar o `session.py`
nem o `split.py`, o `sc-engine-wasm` é o `sc-engine` compilado com outra
feature e faz o mesmo que ele:
- `session`, o protocolo;
- `repl`;
- o one-shot (`-cc1 ...`, `ld ...`), que o `std_pcm()` do
  `test/spike.py` usa para o `.pcm` do `import std;`.

1. **Onde fica.**
   - O `proto/` ganha a feature `wasm`. Ela troca só a camada crua: as sete
     chamadas `sys::sc_*` atrás de `Session` e `Reply`, por chamadas das
     exports pelo wasmtime. Não precisa de trait.
   - O `serve`, o `Copy`, o `Repl` e o `sc-engine.rs` ficam como estão. O
     `Reply::part` empresta o `memory.data(&store)` como hoje empresta o
     ponteiro nativo.
   - O `sc_main` do `sc-engine.rs`, com `wasm`, vira uma instância nova
     que roda o `_start`: uma instância por compilação one-shot (§4).
   - O `wasmtime` e o `wasmtime-wasi` entram nas versões exatas do
     `loader/` (48.0.3). O `proto/Cargo.lock` e o `loader/Cargo.lock` são
     regerados, porque o `loader` depende do `sc-proto` e todo build usa
     `--locked`.
2. **Instância.**
   - Uma `Store` por sessão (`SC_MSG_NEW`): `_initialize`, `chdir` com o
     cwd do processo, e depois `sc_session_new`. Antes de descartar a
     `Store`, o host chama o `sc_session_free`, que apaga o diretório
     temporário.
   - O argv e o fonte vão para a memória pelo `malloc` exportado. Os
     resultados são lidos pelo `sc_result_ptr` e pelo `sc_result_len`.
   - O `.cwasm` do CLI não serve, porque o CLI é a versão 48.0.0 e o crate
     é a 48.0.3: o wasmtime recusa um módulo de outra versão. O host
     compila o módulo com o próprio `Engine` e guarda o resultado num
     cache, como faz o `loader/src/lib.rs`.
   - O `max_wasm_stack` fica no padrão (512 KiB, o mesmo do CLI), de onde
     vêm os "call stack exhausted".
3. **Diretórios e saída.**
   - Um `--dir` da raiz do repositório no mesmo caminho do guest. O
     `build/` está dentro dela, e o `session.py` passa caminhos absolutos
     do host (sysroot, `-I`, `-L`, o `.pcm`).
   - Um diretório temporário do host mapeado como `/tmp`.
   - O stdout do guest vai para o stderr do host, como o `dup2(2, 1)` de
     hoje, para não quebrar as mensagens do protocolo.
   - O stdin não é herdado: o wasmtime-wasi o leria numa thread.
4. **Fork.** O `serve` faz um fork por célula, e o fork só é seguro com uma
   thread. Sem cuidado, o wasmtime tem 57 threads depois de compilar o
   módulo (o rayon) e 114 depois das primeiras chamadas WASI (o tokio do
   p1 síncrono). A receita verificada fica em 1 thread mesmo com arquivos:
   - `Config::parallel_compilation(false)`, ou um módulo desserializado;
   - um runtime tokio próprio, `new_current_thread().enable_all()`, com
     `enter()`;
   - `WasiCtxBuilder::allow_blocking_current_thread(true)` chamado **antes**
     do `preopened_dir`, que captura o valor.

   Antes de cada fork, um assert de que o `/proc/self/task` tem uma
   entrada. Com essa receita, um trap no processo depois do fork voltou
   como `Err`, e a `Store` do pai continuou usável.
5. **Falhas.** No `serve`, o original roda a célula e a cópia espera.
   - Um trap (erro fatal, pilha esgotada) ou o `proc_exit` (o `I32Exit`
     do wasmtime) volta como `Err` da chamada.
   - Com um `Err`, o host faz `_exit` no original, sem ler resultado. A
     cópia então responde "the engine crashed", como no SIGSEGV do nativo.
   - Nos modos sem fork (`REPLAY`, `NEW`, `UNDO`, `DEPTH`), o mesmo `Err`
     encerra o processo, como uma queda nativa.
6. **`mem-`.** O `session.py` lê o RSS no `/proc`. Com `wasm`, o host
   escreve no stderr uma linha com o `memory.size` depois de cada célula,
   e o `session.py` a lê.

**Alvos novos no Makefile:**
- `$(B)/sc-engine-wasm` (sobre o `sc.wasm`) e
  `$(B)/sc-engine-wasm-full` (sobre o `sc-full.wasm`);
- `test-session-wasm` (ses-, seq-, crash-, mem-), contra a
  `session-base`;
- `test-split-wasm`;
- `test-session-wasm-warn`, contra a `session-wev-base`.

**Critério do passo 4:**
- `test-session-wasm` e `test-split-wasm` com 0 ruins, no `sc-full.wasm` e
  no `sc.wasm`.
- `test-session` e `test-split` nativos com 0 ruins contra as bases
  congeladas.
- As exceções conhecidas são listadas com o motivo, como os `KNOWN`:
  - os estouros de pilha, porque o limite de pilha do wasm não é o do
    nativo (§1.2);
  - as diferenças de texto do trap.

#### Passo 5. Cortes do wasm com a sessão

Um resultado ruim no `sc.wasm` que não aparece no `sc-full.wasm` é uma
função cortada. No wasm, a função cortada é um `unreachable`, sem o
`sc_cut_called` do nativo. O nome só aparece no backtrace do trap, e o
`sc.wasm` não tem a seção de nomes (o `wasm-opt --strip-debug` a tira).
Então:
- as fatias rodam também sobre o `sc-cut.wasm`, que tem os nomes, num
  `sc-engine-wasm-cut`;
- o host escreve o trap com o backtrace no stderr;
- o `session.py` e o `learn_session.py` leem o frame 0 `__wrap_X`, além
  da linha `sc: cut function called:`.

Os casos aparecem onde o código do wasm difere do nativo (`#ifdef __wasi__`
no LLVM, a libc).
- Primeiro, rodar e contar.
- Se houver casos, o `learn_session.py` ganha um `--wasm`: liga com o
  `link.py` sem `--native`, usa o `sc-engine-wasm-cut` e escreve na mesma
  lista.
- Se não houver, nada muda, e isso fica registrado.

#### Passo 6. Medidas

- **Tamanho:** `sc.wasm` cru e zstd contra os 5.641.258 da fase 4.
  Orçamento: +1% (§5.1).
- **`mem-`:** a inclinação do `memory.size` (limite de 64 KB/célula, §4) e
  o tempo por célula no wasm, contra os 33 ms em C e 41 ms em C++ do
  nativo cortado.
- **Snapshot**, para a fase 6 decidir se o navegador recupera um
  `SC_ERROR` por snapshot ou por repasse.
  - Mede o custo de copiar a memória linear depois de 100 células, em MB e
    em ms.
  - O que um snapshot precisa, segundo a revisão:
    - a memória linear;
    - o `__stack_pointer`, exportado com `--export`, o que foi verificado.
      Depois de um trap, ele fica onde o trap aconteceu e precisa ser
      restaurado;
    - a memória só cresce: ao restaurar, o trecho que cresceu é zerado;
    - os arquivos abertos na célula ficam abertos na `WasiCtx`, um
      vazamento pequeno.
- **Tempo** do `make test`, para ver se o módulo maior pesa no
  `wasmtime compile`.

#### Passo 7. Documentação

- ANALISE §48: o reactor, as bases congeladas, as exports, o host de
  teste, a receita do fork, as medidas e os cortes.
- README:
  - o `sc.wasm` como reactor e as exports;
  - o `_initialize` uma vez por instância;
  - o `chdir` e o `/tmp`;
  - o contorno do `node:wasi`.
- `bench/cc.mjs`: o contorno do `node:wasi`.
- AGENTS: os alvos novos, e a regra de que `test-session-wasm` não mostra
  diferenças.
- Makefile: `.PHONY` e `clean`.
- O §1.2 passa a apontar para cá.

**Critério de saída da fase 5:**
- os critérios dos passos 3 e 4;
- tamanho dentro do orçamento.

**Feita** (ANALISE.md §48). O que mudou em relação a este plano:
- o `sc.wasm` é ligado também com `-lc-printscan-long-double`: o
  demangler do `wasm-ld` imprime literais `long double`, e o `printf` do
  wasi-libc sem eles aborta;
- as fatias no wasm são comparadas com as bases do `sc-engine-wasm-full`
  (`session-wasm-base` e `session-wasm-wev-base`), e o `ses-` e o
  `crash-` decidem pelo one-shot wasm, e não pela `session-base` nativa:
  onde o nativo cai, o wasm nem sempre cai;
- o host compila o módulo com `--precompile`, em vez do cache do
  wasmtime, e escreve ele mesmo a linha `sc: cut function called:`;
- o learn no wasm é o alvo `learn-session-wasm`. Achou uma função;
- o `loader/Cargo.lock` não precisou mudar;
- o `ses-` do `dcl.decomp/p3` com `-Weverything` caía às vezes: a
  limpeza da célula com erro lia depois da tabela de lookup, um bug de
  antes da fase 5. Corrigido pelo `llvm/incremental-remove-decl.patch`
  (ANALISE.md §48).

### 5.3 Fase 6a em detalhe: o loader mínimo

**Ponto de partida.** Quase tudo do §1.4 já está no `loader/`, desde as
fases E+N e o protótipo do `sc`:
- as tags `__cpp_exception` e `__c_longjmp` são criadas uma vez e
  compartilhadas, e a exceção não capturada vira o `std::terminate`
  (`Loader::uncaught`);
- o `epoch_interruption` existe e é ligado só no REPL interativo
  (`Loader::arm`, `Loader::on_ctrl_c`). O `sc FILE` e o `scload` rodam sem ele.

Falta o wasmtime mínimo. Hoje o `loader/Cargo.toml` usa as features
padrão: o `scload` tem 27,5 MB e o `sc` 28,8 MB. A fase troca isso pelo
mínimo que o loader usa, mede cada feature e registra o tamanho.

**O que o loader usa do wasmtime** (lido em `loader/src`):

| feature | uso | |
| --- | --- | --- |
| `runtime`, `std`, `cranelift` | rodar e compilar as células | fica |
| `gc`, `gc-drc` | a proposta de exceções só é ligada com `gc` (`config.rs:2551`); `Tag`, `ThrownException`. Sem `gc` o loader compila e só falha ao rodar. O `gc-null` nunca libera: um laço que lança acabaria com o heap | fica |
| `cache` | `Cache::from_file`, para as bibliotecas do sysroot | a decidir (passo 3) |
| `parallel-compilation` | implícito: o rayon compila as funções de um módulo em paralelo | a decidir (passo 3) |
| `wat` | os dois módulos gerados em texto: o calço do WASI (`wasi_shim`) e os stubs de `call_indirect` (`stubs`) | sai (passo 2) |
| `addr2line`, `demangle` | não: o loader lê o DWARF com o próprio `gimli` e demangla com o `cpp_demangle` (`wasm_backtrace_details` desligado) | sai |
| `backtrace` | não: é o `std::backtrace` do Rust dentro do `wasmtime::Error`. O `WasmBacktrace` (`keep_stacks`, `blamed`) não depende de feature | sai |
| `async`, `component-model` | só porque o `wasmtime-wasi` `p1` liga o `p2`, que liga os dois | ficam, à força |
| `pooling-allocator`, `profiling`, `coredump`, `threads`, `stack-switching`, `debug*`, `gc-copying`, `gc-null`, `wit-parser`, `compile-time-builtins`, `anyhow` | nenhum | saem |

O `wasmtime-wasi` fica com `default-features = false, features = ["p1"]`.
O `sc-proto` não traz as features padrão de volta: ele só usa o wasmtime
na feature `wasm`, que o `repl` não liga (conferido com `cargo tree -e
features`). Um `cargo check` com esse conjunto compila o `scload` e o `sc`.

**O coletor muda.** Com as features padrão, o `Collector::Auto` escolhe o
`Copying` (`config.rs:3620`), que a documentação do wasmtime diz "not yet
functional". As bases rodaram com ele. Só com o `gc-drc`, o `Auto` escolhe
o DRC. É uma mudança de comportamento: um teste novo lança e pega uma
exceção num laço longo, para ver o heap do GC não acabar.

**Medido antes da fase** (`sc FILE`, load ~54, features padrão):

| programa | cache quente | cache frio |
| --- | ---: | ---: |
| C, `puts` | 0,09 s | 0,41 s |
| C++, `vector::at` com exceção | 2,65 s | 3,72 s |

O cache frio custa a compilação da `libc.so` (1,9 MB) e da `libc++.so`
(0,9 MB) pelo Cranelift, em paralelo.

**Passos** (todos feitos, ANALISE.md §49):

1. **Features.** `default-features = false` no `wasmtime` com `runtime`,
   `std`, `cranelift`, `gc`, `gc-drc`, `cache`, `parallel-compilation` e
   `wat`. O `loader/Cargo.lock` muda (o Makefile usa `--locked`) e vai no
   commit. Medir o `scload`: tamanho e os tempos da tabela acima. Depois
   tirar uma por vez as três a decidir e medir de novo. Uma tabela no
   ANALISE com o custo de cada uma.
2. **Sem `wat`.** Os dois módulos gerados passam a ser montados pelo
   `wasm-encoder`, que o compilador do wasmtime já traz. O `wat_sig` some.
   O ganho foi de 0,86 MB depois do LTO.
3. **`cache` e `parallel-compilation`.** Decididos pela tabela do passo 1.
   O padrão é manter os dois: o cache paga a `libc++.so` uma vez por
   máquina, e sem o rayon o cache frio fica serial.
4. **Perfil de release.** `lto = true`, `codegen-units = 1`,
   `strip = true`, e `opt-level` medido: o `z` do §1.4 deixa o binário
   menor, mas o Cranelift fica mais lento, e isso pesa no cache frio e em
   cada célula. Medir `3`, `s` e `z` no cache frio do C++ e no
   `make test-small`. `panic = "abort"` só se nada depender de unwind:
   não há `catch_unwind` no loader. O wasmtime retoma o panic de uma
   função do host, sem transformá-lo em erro (conferido, ANALISE.md §49).
   Feito no perfil `dist` (`make dist`). O `release`, que as baterias
   usam, ficou sem LTO para compilar rápido.
5. **`epoch_interruption` e o coletor.** Um alvo novo, `make
   test-loader` (`cargo test` no loader, com o `wat` como
   dev-dependency se o passo 2 o tirar): um laço infinito, `arm`, e o
   `increment_epoch` chamado de um handler de sinal, como o Ctrl-C do
   `sc` (`machine.rs:94-101`); o laço tem de parar com `Trap::Interrupt`.
   E o laço que lança do coletor. E medir o custo das checagens no laço de 3·10⁸
   iterações do §1.4, para confirmar que o `sc FILE` sem elas faz
   sentido. Medido: abaixo de 1%, então desligá-las não economiza nada
   que se veja.
6. **O `proto/`.** O `sc-engine-wasm` é só host de teste e fica com as
   features padrão. Registrar isso no ANALISE.

**Critério de saída:**
- `make test`, `test-warn`, `test-native`, `test-split`, `test-split-wasm`,
  `test-session*`, `test-aluno` e `test-checks` sem diferença. Todos rodam
  pelo `scload` ou pelo `sc` com as features novas;
- o `make test-loader` verde;
- as bases não são refeitas: o `$(SCLOAD)` é só de ordem nas regras das
  bases, que guardam a saída do loader antigo para comparar;
- tamanho do `scload` e do `sc` e tempos de cache frio e quente no
  ANALISE.md (§49), comparados com os 5,7 MB do §1.4.

**Riscos:**
- uma feature tirada que só falta num caminho raro. A falta aparece como
  erro do wasmtime na hora, não como saída diferente, mas só no caminho
  que a usa. As baterias passam pelas exceções, pelo `setjmp`, pelos traps
  e pelos vazamentos;
- `opt-level` menor deixar o Cranelift lento demais: o passo 4 mede antes
  de escolher.

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
- **Feito na fase 4 de outro jeito (ANALISE.md §47):** o wasm só tem a
  sessão na fase 5, então quem aprende é o `make learn-session`, no
  nativo, e o resultado vai para o `cuts/symbols.session.used`, que os dois
  engines mantêm. O `ses-` roda o corpus inteiro: o learn refaz só o que
  difere.

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

O protótipo `sc` (ANALISE.md §26) tomou uma decisão provisória para cada
um deles, para que o REPL possa ser usado antes da fase 6b.
