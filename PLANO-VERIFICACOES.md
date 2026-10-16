# Plano: verificações para o aluno

Versão 2 (2026-10-15), com as decisões do professor (§11). A versão 1 foi revisada por três revisores
adversários, e cada mudança cita o achado que a motivou:

- **W:** avisos e a experiência do aluno;
- **R:** runtime, wasm e loader;
- **T:** engine, cortes, testes e fases.

Os experimentos estão no scratchpad da sessão (`rev-avisos`, `rev-runtime`,
`rev-engine`).

O plano cobre o que o `sc` verifica no código do aluno:

- os avisos do clang (`-W`);
- os sanitizers (`-fsanitize`);
- o hardening da libc++;
- a inicialização das variáveis locais;
- os traps do wasm.

Ele agrupa as verificações em conjuntos, propõe quais ficam ligadas por
padrão e diz como desligá-las.

## 0. Princípios

- **Ligado por padrão.** O aluno vê o erro onde ele está, com a linha e em
  palavras. Um erro que passa em silêncio é o pior caso.
- **Desligável.** Com as verificações desligadas, o professor mostra que o
  mesmo programa "funciona", dá um resultado diferente ou corrompe dados.
  É o que acontece num compilador comum. O nível `off` desliga todas as
  verificações de tempo de execução.
- **Sem falso positivo no nível padrão.** Um aviso que dispara em código
  correto ensina o aluno a ignorar o `sc`. O que pode errar fica no
  estrito. Os falsos positivos medidos pelos revisores tiraram vários
  avisos do padrão (§4).
- **Uma mensagem por erro, e o programa para.** A mensagem diz o tipo do
  erro, o arquivo, a linha e a pilha (`-fno-sanitize-recover=all`). No REPL,
  só a célula termina.
- **As regras do projeto.**
  - O engine não muda os seus padrões: quem escolhe as verificações é o
    `sc`.
  - O corpus passa a rodar também com os níveis (§9).
  - Uma opção nova em `sc_inv.cpp` exige `make checkinv` SAME.

## 1. O que o aluno vê hoje

Medido com `sc FILE` e `sc check` no commit a0f2310, em 2026-10-15:

| Programa | Avisos hoje (`-Wall -Wextra`) | Execução hoje |
|---|---|---|
| `int *p = NULL; *p = 3;` | nada | **nada**: escreve no endereço 0 |
| `p[0]` em memória já liberada | nada | **nada**: imprime o valor antigo |
| `free(p); free(p);` | nada | **nada** |
| `malloc` de 4 `int`, escrita em `p[4]` | nada | **nada** |
| `int a[3]; ... a[i] = 0` com `i` até 3 | nada | **nada**: escreve em outra variável |
| `strcpy(a, "hello world")` com `char a[4]` | `-Wfortify-source` (W-A5) | **roda**: escreve em `b` |
| `x = INT_MAX; x + 1` | nada; só `INT_MAX + 1` literal dá `-Winteger-overflow` (W-A5) | **nada**: imprime -2147483648 |
| `int s; s += i` | `-Wuninitialized` | imprime um valor qualquer |
| função `int` sem `return` | `-Wreturn-type` | imprime 0 |
| retorno de `&x` local | `-Wreturn-stack-address` | **nada**: imprime 5 |
| `printf("%d", 2.5)` | `-Wformat` | imprime 0 |
| `v[5]` num `vector` de 3 | nada | `aborted`, sem dizer onde |
| `*o` num `optional` vazio, `s[10]` | nada | `aborted` |
| `1/z` com `z == 0` | nada | `wasm trap: integer divide by zero`, sem a linha |
| recursão sem fim | `-Winfinite-recursion` | `stack overflow`, sem a linha |

Os avisos existem, mas o programa roda mesmo assim. Os erros de memória
passam em silêncio.

## 2. As camadas

| Camada | O que pega | Custo | Onde |
|---|---|---|---|
| 0. Traps do wasm | acesso fora da memória, divisão inteira por zero e `INT_MIN/-1`, pilha de chamadas esgotada, chamada indireta com tipo errado, função ainda indefinida, `unreachable` | nenhum, já existe | host: mensagem e local |
| 1. Avisos | o que dá para ver sem rodar | nenhum em execução | flags do `sc` |
| 2. UBSan | nulo, nulo com deslocamento, overflow com sinal, shift, índice de array de tamanho conhecido, `bool`/`enum` inválido, `float`→`int` fora da faixa, `return` faltando (C++) | código 1,35–2,15×, conforme o modo (§5.1) | flags + handlers |
| 3. ASan | heap, pilha e globais fora dos limites, uso depois do `free` e depois do escopo, `free` duplo ou inválido, `str*` do aluno | código 1,5×; *shadow* de 512 MiB | passe novo no engine + `libsc_rt` + loader |
| 4. libc++ | `vector[]`, `string[]`, `optional`, `span` e iteradores | só nos acessos | macros + um `std.pcm` por modo |
| 5. Inicialização | a local não inicializada tem um valor fixo | um `store` por variável | `-ftrivial-auto-var-init=pattern` |
| 6. Local | arquivo, linha e coluna de cada frame | código 1,2× (DWARF) | `-gline-tables-only` + wasmtime |

A camada 6 serve a todas: os traps das camadas 0 e 4 e os relatos do ASan
só dizem onde ocorreram por ela. O UBSan tem o local nos dados do handler e
não depende dela (T6).

## 3. Níveis e controle

Há três níveis:

| | `off` | `on` (padrão) | `strict` |
|---|---|---|---|
| avisos | padrão do clang | `-Wall -Wextra` − R + A (§4) | + R + S + P, `-Wpedantic` |
| erros | os do clang | + E (§4.1) | + E |
| UBSan | — | U (§5.1) | U + U-estrito |
| ASan | — | sim (F3) | + vazamentos no fim do `sc FILE` |
| libc++ | `NONE` | `EXTENSIVE` | `DEBUG` |
| locais | sem inicialização | `pattern` | `pattern` |
| local do erro | sim | sim | sim |

- **Linha de comando.** `sc --safety=off|on|strict FILE` e
  `sc repl --safety=...`. O `check` aceita a opção, mas só os avisos e os
  erros importam para ele.
- **No REPL, o nível vale para a sessão inteira (T7).**
  - O `restart` de hoje refaz o log só com parse e IR
    (`proto/src/child.rs:185-193`), sem gerar `.so` nem executar.
  - Trocar de nível exigiria recompilar e **reexecutar** todas as células.
    Isso repetiria os prints, consumiria a stdin e mudaria `rand` e `time`.
  - Uma célula aceita no `off` pode ser recusada no `on` por causa do E.
  - Por isso `:safety NÍVEL` recomeça a sessão **vazia**, com um aviso. O
    nível também pode ser escolhido na partida (`sc repl --safety=off`).
- **No `off`, o E volta a ser aviso.** Assim o exemplo de comportamento
  indefinido roda.
- **O local do erro vale em todos os níveis.** Ele só muda a mensagem.
- **Ajuste fino**, para o professor: `sc ... -- -Wno-shadow -fno-sanitize=alignment`.
  - Isso exige que o engine **recuse** a flag que não conhece. Hoje o
    `scParseArgs` (`sc_inv.cpp:190-227`) ignora em silêncio tudo o que não é
    `-W` (T2).
  - Até lá, o `sc` só repassa `-W…`.

## 4. Avisos

**Os números.**

- Com `-Weverything`, o clang 23.1.3 do pin liga 2265 diagnósticos
  (`diagtool show-enabled`, wasm32-wasip1).
- 1450 ficam ligados sem flag nenhuma. O `-Wall` acrescenta 59, e o
  `-Wextra` mais 19.
- Os 737 restantes ficam em **343 grupos** desligados por padrão. O
  apêndice A classifica cada um deles.

**Os conjuntos:**

| Conjunto | Critério | Nível |
|---|---|---|
| **E**: erro | comportamento indefinido certo, ou erro sem uso legítimo, sem falso positivo medido | `on`, `strict` |
| **D**: padrão | o padrão do clang, `-Wall` e `-Wextra`, menos R | todos (o `off` fica no padrão do clang) |
| **R**: retirado | do `-Wall` ou do `-Wextra`, mas ruído em código correto de iniciante | `strict` |
| **A**: acrescentado | desligado no clang; pega erro comum de aluno, sem falso positivo medido | `on`, `strict` |
| **L**: tempo de vida | `-Wlifetime-safety-*`, novo no clang 23 | §4.4 |
| **S**: estrito | estilo, conversões, portabilidade | `strict` |
| **P**: pedante | extensões GNU e clang (`-Wpedantic`) | `strict` |
| **N**: nunca | compatibilidade com padrões antigos, ABI, estilo de projeto grande (`-Wunsafe-buffer-usage`) | — |
| **X**: não se aplica | ObjC, CUDA, HLSL, OpenMP, Microsoft, módulos, *thread safety*, Doxygen | — |

**`-Werror=G` vale também para os subgrupos de G (W-A1).** Cada entrada do
E diz o que ela tira com `-Wno-error=`.

### 4.1 E: avisos que viram erro

Todos já estão ligados no clang ou no `-Wall`:

| Erro | Sem os subgrupos | Motivo |
|---|---|---|
| `-Wuninitialized` | `-Wno-error=sometimes-uninitialized`, `-Wno-error=uninitialized-const-pointer` | o `sometimes` rejeita `if (x>0) r=1; else if (x<=0) r=2;` (W-A1) |
| `-Wreturn-type` | — | rejeita também um `if/else if` que cobre todos os casos; a correção é trivial (W-A13) |
| `-Wreturn-stack-address`, `-Wdangling` | — | |
| `-Warray-bounds` | — | índice constante |
| `-Wdivision-by-zero`, `-Wshift-count-overflow`, `-Wshift-count-negative`, `-Winteger-overflow` | — | constantes (W-A5) |
| `-Wfortify-source` | — | `strcpy` que certamente estoura (W-A5) |
| `-Wformat` | `-Wno-error=format-security`, `-Wno-error=format-extra-args`, `-Wno-error=format-zero-length` | `scanf("%d", x)` roda dentro da libc, que nem o ASan vê. O aviso é a única defesa. `%d` com `sizeof` é UB, e a correção, `%zu`, é o que se ensina (W-A6). O `-Wnonnull` é subgrupo dele |
| `-Wstring-compare` | — | `nome == "ana"` (W-A5) |
| `-Wunsequenced` | — | `i = i++ + 1` em C. Em C++17 é definido e não dispara (W-A13) |
| `-Wmismatched-new-delete`, `-Wdelete-abstract-non-virtual-dtor`, `-Wfree-nonheap-object` | — | O caso não abstrato (`delete d` com o tipo exato) é correto e fica como aviso (W-A4) |

**Saem do E da versão 1:**

- `-Wnull-dereference`: só pega `*(int*)0`, e a mensagem confunde. O UBSan
  cobre o caso (W-A13).
- `-Wdelete-non-abstract-non-virtual-dtor` (W-A4).

**Já são erro no clang 23:**

- `-Wint-conversion`, `-Wincompatible-pointer-types`;
- `-Wimplicit-function-declaration`, `-Wimplicit-int`.

### 4.2 R: retirar do padrão

- `-Wunused-parameter`: dispara em `main(int argc, char *argv[])` e em
  callbacks.
- `-Wmissing-field-initializers`: `struct S s = {1}` zera o resto.
- `-Wsign-compare`: é o aviso mais frequente em código correto de
  iniciante, com `i < v.size()` e `i < strlen(s)`. Foi o único aviso do
  `-Wall` no programa C++ típico.
  - A justificativa da versão 1 estava errada: `-1 < 1u` com literais não
    dispara o aviso. Só a forma com variáveis dispara (W-A10).
  - Pelo princípio do §0, vai para o R (decisão 2, §11).

### 4.3 A: acrescentar ao padrão

O número entre parênteses é a quantidade de programas do corpus em que o
aviso dispara com `-Weverything`. O corpus não é de aluno, então o número só
indica o ruído.

- **Conversões:**
  - `-Wfloat-overflow-conversion` (1), `-Wfloat-zero-conversion`;
  - `-Wstring-conversion`: `"abc"` como `bool`;
  - `-Wshift-bool`;
  - `-Wshift-sign-overflow` (8): só em C; em C++20, `1 << 31` é definido
    (W-A13).
  - O `int i = 2.7;` já é pego por padrão (`-Wliteral-conversion`).
  - O `-Wfloat-conversion` foi para o S: `int lim = sqrt(n)` é comum e
    intencional (W-A9; decisão 3, §11).
- **Comparações e laços:**
  - `-Wtautological-unsigned-zero-compare` (9):
    `for (size_t i = n - 1; i >= 0; i--)`;
  - `-Wtautological-unsigned-enum-zero-compare`;
  - `-Wtautological-value-range-compare` (13);
  - `-Wunreachable-code` (75), com os subgrupos `-loop-increment` (7) e
    `-fallthrough`. As formas `-break` e `-return` ficam de fora com
    `-Wno-` (W-A12).
- **Fluxo:** `-Wimplicit-fallthrough` (18) e o subgrupo `-per-function`
  (W-A12).
- **Nomes:**
  - `-Wshadow` (54), **só em C e só no `sc FILE`**. Em C++, o setter
    `this->x = x` dispara, e o clang não tem um grupo só para locais (W-A7).
  - No REPL, uma global de uma célula anterior faria todo `for (int i …)`
    avisar.
  - `-Wshadow-field-in-constructor-modified`.
  - `-Wcomma` (57).
- **C:** `-Wdefault-const-init-var` (6), `-Wdefault-const-init-field`,
  `-Wmain` (4), `-Wmain-return-type`. O `-Wmain-return-type` já dispara em
  `void main()` (W-A12).
- **C++:**
  - `-Wnon-virtual-dtor` (1);
  - `-Wdeprecated-copy-with-user-provided-dtor` (regra dos três). Depois da
    F3, ele vai para o S: dispara na aula de ciclo de vida, e o ASan pega o
    `free` duplo real com uma mensagem melhor (W-A11).
  - `-Warray-bounds-pointer-arithmetic`.
- **Foram para o S:**
  - `-Wconditional-uninitialized`: 5 falsos positivos em 6 funções corretas
    (W-A8). O `pattern` mostra o erro real na execução.
  - `-Wclass-varargs`: o caso do `std::string` já é erro por
    `-Wnon-pod-varargs` (W-A12).

### 4.4 L: tempo de vida (só C++)

- **`-Wlifetime-safety-permissive` vai para o A.** Ele pegou todos os bugs
  do teste, sem falso positivo em listas encadeadas, `unique_ptr::get` e
  `static` local (W-A2). Inclui:
  - `use-after-scope`;
  - `return-stack-addr`;
  - `use-after-free`.
- **`invalidation` vai para o S.** Ele dispara em
  `for (auto &row : g) row.push_back(1)` e em `string &f = v[0]; f += "!"`
  (W-A2).
- **`dangling-field` e `dangling-global`** ficam para medir na F1.
- **Em C**, a análise só roda com `-fexperimental-lifetime-safety-c` (cc1),
  e mesmo assim não vê `free(p); *p` (W-A3). Fica fora até o ASan.

### 4.5 Diferenças por modo

O §29 do ANALISE.md já tira do REPL os avisos de fim de unidade.

Nesta proposta, só `-Wshadow` muda por modo: fica no `sc FILE` em C. O E vale
nos dois modos.

Um erro numa função esconde os avisos de análise das funções seguintes
(W-A14). Por isso cada programa do `test/checks` tem um bug só.

## 5. Verificações em tempo de execução

### 5.1 UBSan

| Verificação | Pega | Decisão |
|---|---|---|
| `null` | acesso por ponteiro nulo, inclusive `s->campo` (testado, R3) | **U** |
| `pointer-overflow` | `p[1000]` com `p` nulo (testado, R3), aritmética que dá a volta | **U** |
| `alignment` | acesso desalinhado; o wasm aceita, mas é UB | **U** |
| `array-bounds` | índice fora de um array de tamanho conhecido | **U** |
| `signed-integer-overflow` | `INT_MAX + 1`, `-INT_MIN` | **U** |
| `integer-divide-by-zero` | `/` e `%` por 0, com a linha | **U** |
| `shift-base`, `shift-exponent` | `1 << 40`, `x << -1` | **U** |
| `vla-bound` | VLA com tamanho ≤ 0 | **U** |
| `return` (C++) | fim de uma função não `void` sem `return` | **U** |
| `unreachable` | `__builtin_unreachable()` alcançado | **U** |
| `bool`, `enum` | valor inválido carregado | **U** |
| `builtin` | `__builtin_clz(0)` | **U** |
| `float-cast-overflow` | `(int)1e10` | **U** |
| `nonnull-attribute`, `returns-nonnull-attribute` | `NULL` para um parâmetro `nonnull` da libc | **U** |
| `local-bounds` | — | não emite nada a `-O0` (medido) |
| `object-size` | — | não tem efeito a `-O0` (o clang avisa) |
| `unsigned-integer-overflow`, `unsigned-shift-base`, `implicit-conversion`, `float-divide-by-zero` | volta de `unsigned`, truncamento implícito, `1.0/0` | **U-estrito**: disparam em `std::hash` e nos geradores dos headers |
| `nullability-*` | — | só com `_Nonnull` |
| `function` | — | não suportado no wasm, e o wasm já faz trap |
| `vptr` | — | só no Emscripten, precisa de um runtime C++ |

**Os três modos (R6).** A decisão sai da medida na F2:

| Modo | `c1_basics.cpp` | O que o aluno vê |
|---|---|---|
| handlers completos | 200 520 bytes (2,15×) | a linha, mais os valores dos operandos e os tipos (`2147483647 + 1 cannot be represented in type 'int'`) |
| minimal runtime | 134 499 (1,44×) | o tipo do erro; a linha vem da pilha (F0) |
| `-fsanitize-trap` | 125 988 (1,35×) | o tipo do erro, que vai no DWARF (`__clang_trap_msg$Undefined Behavior Sanitizer$signed integer addition overflow in 'a + b'`); a linha vem da F0 |

Com o `trap`, não há runtime nenhum. Os valores dos operandos são o único
ganho dos handlers.

**Se forem handlers, eles ficam em C, na `libsc_rt` (T10).**

- Assim eles servem ao `sc`, ao `scload` do corpus, ao wasmtime CLI e ao
  navegador.
- Eles chamam um único import do host, para a pilha e o trap.
- Os símbolos emitidos com o U (R6):
  - `add/sub/mul/negate_overflow`, `divrem_overflow`, `shift_out_of_bounds`;
  - `out_of_bounds`, `type_mismatch_v1`, `pointer_overflow`;
  - `vla_bound_not_positive`, `load_invalid_value`, `float_cast_overflow`;
  - `invalid_builtin`, `nonnull_arg`, `nonnull_return_v1`;
  - `alignment_assumption`, que vem dos headers da libc++.

  Cada um tem a forma `_abort`, menos `builtin_unreachable` e
  `missing_return`. O `SourceLocation` é {ptr, u32, u32}, e um operando com
  mais de 32 bits chega por ponteiro.

### 5.2 ASan

| Item | Decisão |
|---|---|
| `address`, `use-after-scope` | **on** |
| `use-after-return` | `=never`: precisa da *fake stack*, e o `-Wreturn-stack-address` pega o caso comum |
| `pointer-compare`, `pointer-subtract` | não: o driver recusa no wasm |
| `leak` | **strict**, no fim do `sc FILE`; o `:leaks` no `on` e no `strict`. Só o bloco que nada alcança (ANALISE §37) |
| `init-order` | não. `__asan_before_dynamic_init` e `__asan_after_dynamic_init` são emitidos mesmo assim e viram no-op (R7) |

**O passe precisa entrar no engine (T1).**

- O `engine/sc_backend.cpp` substitui o `BackendUtil.cpp` e roda só o
  AlwaysInliner, as corrotinas e o codegen. O `addSanitizers` não existe em
  nenhum dos engines.
- O `sc_backend` passa a rodar o `AddressSanitizerPass` no novo PM, como o
  `runCoroPasses` (ANALISE §16).
- Com `-asan-use-stack-safety=0`. Sem isso, o `StackSafetyGlobalAnalysis`
  puxa o ScalarEvolution (436 KB, cortado hoje).
- As opções `-asan-*` são `cl::opt`. O `sc_inv` as fixa em `LLVMArgs`
  (`sc_inv.cpp:173`), e o `checkinv` as cobre.

**Layout da memória (R2, R3).**

- A pilha continua primeiro, em [16, 1 MiB). Assim o estouro continua dando
  trap abaixo de 0.
  - A versão 1 punha a pilha depois da *shadow* com uma zona de guarda.
    Isso não funciona: o prólogo de cada frame instrumentado reescreve a
    *shadow* do próprio frame e apaga o veneno da guarda (R2).
- A *shadow* sai do endereço 0 com `-mllvm -asan-mapping-offset=S`.
  - S não pode ser potência de 2: com potência de 2, o passe usa OR
    (`AddressSanitizer.cpp:624-627`).
  - Por exemplo, S = 0x110000. A *shadow* fica em [S, S + 512 MiB) e o
    programa começa depois dela.
- A memória já nasce com S + 512 MiB. Toda leitura da *shadow* precisa
  estar dentro da memória, inclusive a de um endereço indefinido acima de
  3 GiB (R12).
- **Nada de regiões envenenadas grandes.** Envenenar [3 GiB, 4 GiB) e
  [0, 64 MiB) custaria 192 MiB gravados, que viram RSS de verdade (R3).
  - O nulo com deslocamento já é pego pelo UBSan.
  - O símbolo indefinido já funciona: a leitura da *shadow* cai na memória,
    o acesso dá trap, e `undefined_at` dá o nome.

**O runtime, `libsc_rt.so`.** É C, PIC, e o loader o carrega antes da libc.

- **A ABI** (R7):
  - `__asan_init` e `__asan_version_mismatch_check_v8`;
  - `__asan_report_{load,store}{1,2,4,8,16,N}`, que podem ser funções do
    host;
  - `__asan_set_shadow_00`, `__asan_alloca_poison` e
    `__asan_allocas_unpoison` (VLA);
  - `__asan_memcpy`, `__asan_memset` e `__asan_memmove`. O `memcpy` do aluno
    vira esses, então eles não precisam de interceptor.
  - `__asan_register_globals` e `__asan_unregister_globals`;
  - `__asan_handle_no_return`.

  Cada global exporta `__odr_asan_gen_<nome>`, e esses nomes são filtrados.
- **O alocador (R8).**
  - É um invólucro do `malloc` da libc, com zonas vermelhas, cabeçalho e
    uma quarentena FIFO.
  - A `libc.so` chama o próprio `malloc` direto, com 61 `call` internos. A
    `libc++.so` e a `libc++abi.so` importam `malloc`, `free`, `realloc`,
    `aligned_alloc` e `_Znwm`. O loader as resolve para o invólucro, então o
    `new` também ganha zonas vermelhas.
  - Um ponteiro da libc, como o de `strdup`, não tem cabeçalho, e o `free`
    o repassa.
  - `getline` e `getdelim` são interceptados, porque dão `realloc` num
    ponteiro do aluno.
  - **O loader** aloca os dados de cada célula com `aligned_alloc`
    (`lib.rs:222-230`). Ele passa a usar o `__real_aligned_alloc`. Senão os
    globais virariam blocos do heap, e `free(&global)` passaria.
  - **A quarentena** desenvenena o bloco ao devolvê-lo à libc. Senão o
    `strdup` seguinte acusaria um falso positivo.
- **Interceptores:** `strcpy`, `strncpy`, `strcat`, `strncat`, `sprintf`,
  `snprintf`, `fgets`, `fread` e `read`.
  - O loader entrega a libc ao runtime como `__real_*`.
  - Uma chamada de dentro da libc não é interceptada.
  - `scanf("%s")` fica para a F4.
- **Exceções e `longjmp` (R9).**
  - O `__asan_handle_no_return` é emitido antes do `__cxa_throw` do aluno e
    do `longjmp` (testado).
  - Os `throw` de dentro da `libc++.so`, como `at` e `stoi`, não passam por
    ele. O loader encaminha os `__cxa_throw` e `__cxa_rethrow` importados
    pela `libc++.so` a um invólucro no runtime.
  - O risco é menor do que parecia: o prólogo reescreve a *shadow* do frame.
- **Os relatos.**
  - O host lê a *shadow* para classificar o erro: heap, pilha, global,
    liberado, escopo.
  - A pilha da alocação são só os frames do topo, compactos (módulo e
    offset), capturados no host. O custo vai ser medido com um programa que
    usa `std::map` (R12).
- **REPL.** Depois de um trap, a máquina restaura o *stack pointer* e limpa
  a *shadow* da pilha abandonada.

**Para depois: a libc instrumentada.** Construir a wasi-libc com ASan e
`MALLOC_IMPL=none` pegaria `scanf` e `printf("%s")` dentro da libc. Isso exige
construir a libc a partir do fonte.

### 5.3 libc++ (R4, R5)

- O sysroot já tem `LIBCXX_HARDENING_MODE=fast`. A semântica dele é
  `QUICK_ENFORCE`: um trap sem mensagem, o `aborted` de hoje.
- **`on`:** `EXTENSIVE`, ainda com `QUICK_ENFORCE`.
  - Com `-g`, a mensagem do trap já está no DWARF:
    `__clang_trap_msg$libc++$vector.h:414: libc++ Hardening assertion __n < size() failed: vector[] index out of bounds`
    (R4).
  - A F0 mostra essa mensagem e a linha do aluno. O wasmtime CLI não
    mostrou o frame inline do `__clang_trap_msg`, então a F0 lê o DWARF com
    o `addr2line` direto, se preciso (R10).
- **`strict`:** `DEBUG`, que já usa `ENFORCE`, com `verbose_abort` e a
  mensagem.
- **`off`:** `NONE`.
- **Fora:** `_LIBCPP_ASSERTION_SEMANTIC`. Ele é experimental, dá `#error`
  sem `-fexperimental-library`, e essa flag muda o `.pcm`.
- **Um `std.pcm` por modo é obrigatório.** O módulo aceita uma macro
  diferente sem reclamar e usa o valor de quando foi construído (R5,
  testado com `-DFOO`). Sem um `.pcm` por modo, o hardening não teria efeito
  com `import std`.
  - `-fsanitize`, `-g` e `pattern` não conflitam com o `.pcm` (testado).
  - Misturar células de modos diferentes é seguro: cada modo tem uma
    assinatura ODR própria (T7).

### 5.4 Inicialização das locais (R1)

**`on` e `strict`:** `-ftrivial-auto-var-init=pattern`. No wasm32, ponteiros
de 32 bits fazem o `PatternInit.cpp:17-20` escolher `0xFF…`:

| Tipo | Valor |
|---|---|
| inteiro | -1 |
| ponteiro | 0xFFFFFFFF; acima dos 3 GiB, então dá trap já na F2, sem ASan |
| `double` | NaN |
| `char[]` | `\xFF` repetido |

O -1 é menos visível que um padrão. Um patch de uma linha poria 0xAAAAAAAA
nos inteiros e deixaria o ponteiro em 0xFFFFFFFF. Ele fica para depois de
medir.

**O trap perto de 0xFFFFFFxx é ambíguo.** Pode ser um ponteiro não
inicializado ou o estouro da pilha linear abaixo de 0. O `runtime_error`
separa os dois casos pelo `stack_pointer()`: se ele deu a volta, é estouro.

**`off`:** a local fica com o que estava na pilha.

**Descartados:**

- o `zero`, que esconde o erro;
- o MSan, que precisa da libc e da libc++ instrumentadas e não tem suporte
  no wasm.

### 5.5 Sanitizers fora do escopo

| Sanitizer | Por quê |
|---|---|
| `memory` | §5.4 |
| `thread` | o REPL não tem threads |
| `hwaddress`, `memtag-*`, `kernel-*` | hardware ou kernel |
| `cfi-*`, `kcfi`, `safe-stack`, `shadow-call-stack` | o wasm já verifica a chamada indireta e separa a pilha de retorno |
| `type` | *strict aliasing* não muda nada a `-O0` |
| `dataflow`, `numerical`, `realtime`, `fuzzer`, `scudo`, `alloc-token` | outro propósito |
| `-fstack-protector`, `_FORTIFY_SOURCE` | o ASan é melhor; a wasi-libc não tem *fortify* |

**Observação para o professor.** No wasm, estourar um array local corrompe
outras variáveis, mas nunca o endereço de retorno. No `off`, o
*stack smashing* clássico não derruba o programa como no x86.

### 5.6 O local do erro (camada 6; T6, R10)

O wasmtime já faz quase tudo:

- `Config::wasm_backtrace_details(Enable)`, com a feature `addr2line`
  padrão, dá arquivo, linha e coluna por frame. O padrão só lê
  `WASMTIME_BACKTRACE_DETAILS`.
- O `.debug_line` de um `.so` PIC é relativo ao módulo. O revisor viu
  `d.c:2:22` num `.so` carregado pelo `scload`.
- O host formata os frames do aluno com `frame.symbols()`.

**Armadilhas:**

- O wasmtime demangla `f` como `float`. Usar o nome cru, e demanglar só os
  nomes `_Z`, como o loader já faz.
- O padrão é 20 frames: na recursão, juntar os frames repetidos.
- **Na sessão incremental, o DWARF nunca foi exercitado (T8).** Cada célula
  é um módulo novo, com o nome `cell_N`. Um passe do `test-split` com
  `-gline-tables-only` entra na F0.

**As mensagens dos traps:**

| Trap | Mensagem |
|---|---|
| `out of bounds memory access` | `invalid memory access` ou `stack overflow`, pelo `stack_pointer()` (§5.4) |
| `integer divide by zero` | `division by zero` |
| `integer overflow` | `INT_MIN / -1` |
| `call stack exhausted` | `stack overflow (infinite recursion?)` |
| `indirect call type mismatch` | `call through a function pointer of the wrong type` |
| `uninitialized element` | `call through a null function pointer`, ou `undefined symbol` se for o slot de `reserve_slot` (R11) |

## 6. Arquitetura

- **Flags.**
  - Os conjuntos ficam em `scripts/flags.py`, numa função
    `safety(level, cxx, mode)`.
  - O `flags.py --session` recebe o nível e gera
    `build/repl/<linguagem>-<nível>.args`. Hoje é um arquivo por linguagem
    (`machine.rs:86`) (T3).
- **`sc_inv.cpp`.**
  - **Parse estrito (T2).** O `scParseArgs` passa a recusar com erro a
    flag que não conhece.
  - **Flags novas:** `-D`, para as macros da libc++; hoje não há código de
    `PreprocessorOptions`.
  - **Flags novas:** `-fsanitize=`, `-fsanitize-recover=`, `-fsanitize-trap=`,
    `-fsanitize-merge=`, `-fsanitize-address-use-after-scope`,
    `-fsanitize-address-globals-dead-stripping`.
  - **Flags novas:** `-ftrivial-auto-var-init=`, `-debug-info-kind=`,
    `-dwarf-version=`, `-debugger-tuning=`.
  - **`-mllvm -asan-*`.**
  - **`sc_inv.cpp:107-108`** hoje fixa os padrões contrários de
    `SanitizeMemoryParamRetval` e `UseOdrIndicator`. O `flags.py` decide
    cada um.
- **`checkinv` (T3).**
  - O *round trip* já serializa `Sanitize*`, `DebugInfo` e `-D`.
  - `TrivialAutoVarInit` entra na comparação campo a campo.
  - Ele ganha umas 8 linhas: {C pic, C++ pic, C++ pic mod, C++ pcm} ×
    {on, strict}.
- **`sc_backend.cpp`:** o passe do ASan (§5.2, T1).
- **Cortes (T5).**
  - **O REPL e o `sc FILE` usam o `sc-engine-full`**, nativo e sem cortes
    (`machine.rs:85`). A F0, a F1 e a F2 no REPL não dependem do `learn`.
  - O `learn` só importa para o `sc.wasm` e o `build/sc`. Medido nos
    `engine.map`:
    - `CGDebugInfo`: 376 KB no completo, 18 KB no cortado;
    - DWARF do AsmPrinter: 366 KB e 54 KB;
    - ASan: 95 KB e 0;
    - UBSan (`EmitCheck*`) e `pattern`: não estão cortados.
  - O teto é de uns 0,8 MB em 24 MB, cerca de 3%.
- **Loader (`loader/src/lib.rs`).**
  - Carrega a `libsc_rt.so` primeiro e resolve os imports da libc++ para
    ela (§5.2).
  - Entrega a libc como `__real_*`.
  - Monta a memória com a *shadow*.
  - Liga `wasm_backtrace_details`.
  - Fica no `lib.rs`, e não no `sc`, porque o `split.py` e o `corpus.py`
    rodam pelo `scload` (T9).
- **Sysroot.** O `make sysroot` passa a construir a `libsc_rt.so` (PIC e
  estática) e o `std.pcm` de cada modo de hardening.

## 7. Custos

**Medidos** com o clang nativo do pin, que roda o pipeline completo. O
engine só roda os passes do ASan depois do T1. São objetos wasm32 PIC
a `-O0`, em bytes:

| Programa | nada | UBSan (handlers) | ASan | os dois | `-gline-tables-only` |
|---|---|---|---|---|---|
| `test/cxx/c3_stl.cpp` | 442 544 | 1 003 900 (2,27×) | 644 268 (1,46×) | 1 211 950 (2,74×) | 530 941 (1,20×) |
| `test/cxx/c1_basics.cpp` | 93 205 | 199 129 (2,14×) | 138 593 (1,49×) | 244 068 (2,62×) | — |

O UBSan com handlers pesa mais que o ASan a `-O0`, porque os templates da
libc++ ganham todas as verificações. No modo `trap`, o fator cai para 1,35×
(§5.1).

**A medir:**

- a latência por célula no `sc-engine-full`, com o UBSan e o ASan
  separados (T11);
- o tempo de execução do corpus em cada nível;
- a captura da pilha em cada `malloc`;
- o engine depois do `learn`.

## 8. Fases (reordenadas por T)

Cada fase fecha com o `make test` com 0 diferenças, o `test-split`, o
`checkinv` SAME e o registro no ANALISE.md.

| Fase | Entrega | Tamanho |
|---|---|---|
| **F1** | o parse estrito do `sc_inv` (T2); os níveis no `flags.py`, no `sc` e no `checkinv` (T3); o corpus em vários níveis (T4); os avisos E, R, A e L (§4); medir L e o ruído no corpus de aluno | média |
| **F0** | `wasm_backtrace_details`; traps em palavras; `-gline-tables-only` no REPL nativo, mais o `test-split` com `-g` (T8) | pequena |
| **F2** | UBSan com o modo escolhido pela medida (§5.1); libc++ `EXTENSIVE` e um `std.pcm` por modo; `pattern` | média |
| **F2b** | `learn` para o `sc.wasm`: DWARF e o que mais faltar | pequena |
| **F3** | o passe do ASan no `sc_backend` (T1); o `libsc_rt`; o layout com offset (R2); o alocador e o loader (R8); o `__cxa_throw` (R9); o relato com pilha | grande |
| **F4** | `strict`: U-estrito, vazamentos, libc++ `DEBUG`, `scanf("%s")`; opcional: libc instrumentada | média |

**A F1 está feita** (ANALISE.md §30).
- A célula passou a contar como o arquivo principal
  (`llvm/incremental-main-file.patch`). Antes disso, o `-Wunreachable-code`
  não disparava, e o `-Wheader-hygiene` dava falso positivo.
- O `-Wmissing-prototypes` saiu do S para o N: disparava em 16 dos 20
  programas corretos do `test/aluno` no `strict`.

**A F0 está feita** (ANALISE.md §31): o erro em palavras e os frames do
aluno, com `-gline-tables-only` em toda sessão; o `test-split` com `-g`
(T8) e o `make test-checks`.

**A F2 está feita** (ANALISE.md §32): o UBSan com handlers na `libsc_rt`
(`rt/`), o `EXTENSIVE` da libc++ com a semântica `enforce` no sysroot, o
`pattern`, e um `std.pcm` por nível no corpus. Uma célula leva 102 ms no
`on` contra 61 ms no `off`.

**A F2b está feita** (ANALISE.md §33): o corpus compila os `pic-`, os
`repl-` e os níveis com `-gline-tables-only`, e o `learn` devolveu o DWARF
ao `sc.wasm` (+211 KB).

**A F3 está feita** (ANALISE.md §34): o passe do ASan no `sc_backend`, o
runtime em `rt/sc_asan.c`, a *shadow* em 0x110000, o alocador com
quarentena, os nomes do `--wrap` no loader e o `__cxa_throw`. O
`llvm/asan-cleanupret.patch` corrige o ASan com as exceções do wasm. O
`-Wdeprecated-copy-with-user-provided-dtor` foi para o S. Uma célula leva
115 ms no `on` contra 68 ms no `off`, e o `sc.wasm` cresceu 78 KB.

**A F3b está feita** (ANALISE.md §36). O relato diz onde o bloco foi
liberado e alocado, com pilhas capturadas pelo loader e simbolizadas por
ele só no relato. Só os primeiros 20 mil blocos guardam a pilha, e o
`std::map` de 100 mil chaves fica no tempo de antes (R12). O `sc FILE` com
`setjmp`/`longjmp` também foi corrigido (§35).

**Os vazamentos da F4 estão feitos** (ANALISE.md §37): no fim do `sc FILE`
no `strict` e no `:leaks` do REPL.

**O libc++ `DEBUG` da F4 está feito** (ANALISE.md §38): o `strict` pega o
comparador que não é uma ordem estrita fraca, como o `<=` no `std::sort`.
Faltam o `scanf("%s")` e o U-estrito.

A F1 vem primeiro, porque é o alicerce:

- o `sc_inv` passa a aceitar e conferir as flags;
- o corpus passa a testar os níveis.

## 9. Testes

- **`test/aluno/`: código correto de aluno (T9, W).**
  - São exercícios de introdução em C e em C++ com STL: laços, vetores,
    strings, structs, listas, matrizes, grafos, classes, `map`.
  - **É o portão do nível `on`:** nenhum aviso do E ou do A e nenhum relato
    em execução.
  - Os programas corretos do revisor W (`rev-avisos/ok/`) são o começo.
  - O corpus de hoje não serve para isso. Dos 1947 programas, 1698 são do
    gcc torture, que tem UB de propósito; o `test/cxx` mais o
    `test/samples` são só 25 arquivos.
- **`test/checks/`: um bug por programa (W-A14).** Cada programa traz a
  saída esperada em cada nível: no `off`, o comportamento silencioso; no
  `on`, a mensagem e a linha. `make test-checks` compara.
- **O corpus em vários níveis (T4).**
  - O `corpus.py` roda cada conjunto de flags na mesma rodada, com os nomes
    sufixados (`-on`, `-strict`).
  - A base sai do `sc-full`.
  - O `learn` aprende a união.
  - O E é separado dos testes de tempo de execução. Senão, os programas
    gnu17 do torture param na compilação.
- **O `test-split` com um nível.** O `split.py` passa o nível a
  `session(...)`. A diferença com a base sem verificação vira uma lista de
  relatos para triagem: UB real do teste, ou falso positivo, que derruba a
  verificação para o estrito.
- **Os programas estáticos** do corpus, que rodam no wasmtime CLI, se ligam
  com a `libsc_rt.a` (T10).

## 10. Riscos

1. **O ASan com a ligação dinâmica.** O registro dos globais célula a
   célula funciona: o `.so` liga e o construtor roda com os metadados
   relocados (R11). O `SC_SYMS` só tem nomes, então o tamanho dos globais
   não importa (R11). Sobra testar o undo e o replay com o ASan.
2. **Os `throw` de dentro da libc++** sem `__asan_handle_no_return` (R9,
   §5.2).
3. **O DWARF na sessão incremental** (T8). O teste vai na F0.
4. **512 MiB de memória.** Ela nasce com a memória e não pode crescer
   depois (R12).
   - No nativo, isso é só `mprotect`.
   - No Windows, a memória comprometida conta.
   - No navegador pode falhar. Nesse caso, o ASan começa desligado no
     `sc.wasm`.
5. **A latência do REPL.** Se o `on` deixar a célula lenta demais, o REPL
   usa o UBSan em modo `trap` e deixa o ASan só para o `sc FILE`. A medida
   decide (T11).
6. **Os falsos positivos nos headers da libc++.** O portão é o
   `test/aluno` em C++ (§9).

O risco 4 da versão 1, o `std.pcm`, foi resolvido: um `.pcm` por modo é
obrigatório (R5).

## 11. Decisões do professor (2026-10-15)

1. **O E vira erro** nos níveis `on` e `strict`, com `-Wformat` e com as
   exceções da §4.1. No `off`, volta a ser aviso.
2. **`-Wsign-compare` sai do padrão** e vai para o R, só no `strict`.
3. **`-Wfloat-conversion` fica só no `strict`.** No padrão ficam
   `-Wfloat-overflow-conversion` e `-Wfloat-zero-conversion`.
4. **O UBSan usa os handlers completos**, se a latência medida na F2
   permitir. Se não permitir, o REPL usa o modo `trap`, e o `sc FILE` fica
   com os handlers.
5. **Os vazamentos aparecem só no `strict`.** O `:leaks` do REPL funciona no
   `on` e no `strict`: o `off` não tem o alocador que conhece os blocos
   (2026-10-16). Vazamento é o bloco que nada alcança, como no LSan, e ele
   muda um status 0 para 1 (ANALISE §37).
6. **O `strict` só liga `-Wpedantic`.** O dialeto continua `gnu23` e
   `gnu++26`.
7. **O controle se chama `--safety=off|on|strict`**, no REPL `:safety
   NÍVEL`. Ele recomeça a sessão vazia, com um aviso. O nome diz o que se
   perde ao desligar e não se confunde com o comando `check`.

## Apêndice A: os 343 grupos desligados por padrão

Os grupos estão classificados pelos conjuntos da §4. O número entre
parênteses é a quantidade de programas do corpus em que o grupo dispara; sem
número, não dispara em nenhum.

A lista vem de `diagtool show-enabled` com e sem `-Weverything`, do clang
nativo do pin. O script é o `classify.py`, no scratchpad da sessão.

**A: ligar no nível padrão (`-Wshadow` só em C; `-Wdeprecated-copy-with-user-provided-dtor` volta ao S depois da F3)** (23):

`-Warray-bounds-pointer-arithmetic`, `-Wcomma` (57), `-Wdefault-const-init-field`, `-Wdefault-const-init-var` (6), `-Wdeprecated-copy-with-user-provided-dtor`, `-Wfloat-overflow-conversion` (1), `-Wfloat-zero-conversion`, `-Wimplicit-fallthrough` (18), `-Wimplicit-fallthrough-per-function`, `-Wmain` (4), `-Wmain-return-type` (5), `-Wnon-virtual-dtor` (1), `-Wshadow` (54), `-Wshadow-field-in-constructor-modified`, `-Wshift-bool`, `-Wshift-sign-overflow` (8), `-Wstring-conversion`, `-Wtautological-unsigned-enum-zero-compare`, `-Wtautological-unsigned-zero-compare` (9), `-Wtautological-value-range-compare` (13), `-Wunreachable-code` (75), `-Wunreachable-code-fallthrough`, `-Wunreachable-code-loop-increment` (7)

**L: análise de tempo de vida, só C++; a forma `-permissive` (use-after-scope, return-stack-addr, use-after-free) vai para A; dangling-field/global a medir** (9):

`-Wlifetime-safety-dangling-field`, `-Wlifetime-safety-dangling-field-moved`, `-Wlifetime-safety-dangling-global`, `-Wlifetime-safety-dangling-global-moved`, `-Wlifetime-safety-return-stack-addr`, `-Wlifetime-safety-return-stack-addr-moved`, `-Wlifetime-safety-use-after-free`, `-Wlifetime-safety-use-after-scope`, `-Wlifetime-safety-use-after-scope-moved`

**S: ligar só no nível estrito** (76):

`-Walloca` (12), `-Wanon-enum-enum-conversion`, `-Wassign-enum` (6), `-Wbad-function-cast` (8), `-Wc++-compat` (68), `-Wc++-hidden-decl` (3), `-Wc++-keyword` (27), `-Wc++-unterminated-string-initialization` (7), `-Wcast-align` (36), `-Wcast-function-type-strict` (7), `-Wcast-qual` (24), `-Wclass-varargs` (21), `-Wconditional-uninitialized` (48), `-Wconversion` (5), `-Wctad-maybe-unsupported`, `-Wdeprecated-copy-with-dtor`, `-Wdouble-promotion` (66), `-Wdtor-name`, `-Wduplicate-decl-specifier`, `-Wduplicate-enum`, `-Wenum-compare-conditional`, `-Wenum-enum-conversion`, `-Wenum-float-conversion`, `-Wexpansion-to-defined`, `-Wextra-semi` (12), `-Wextra-semi-stmt` (42), `-Wfloat-conversion` (58), `-Wfloat-equal` (82), `-Wformat-nonliteral` (1), `-Wformat-signedness` (21), `-Wformat-type-confusion`, `-Wfour-char-constants`, `-Wheader-hygiene`, `-Wignored-base-class-qualifiers`, `-Wimplicit-float-conversion` (29), `-Wimplicit-int-conversion` (203), `-Wimplicit-int-conversion-on-negation` (9), `-Wimplicit-int-enum-cast` (9), `-Wimplicit-int-float-conversion` (22), `-Wimplicit-void-ptr-cast` (110), `-Wincompatible-function-pointer-types-strict` (8), `-Winconsistent-missing-destructor-override`, `-Wjump-misses-init` (9), `-Wkeyword-macro` (2), `-Wlifetime-safety-invalidation`, `-Wmissing-prototypes` (3002), `-Wold-style-cast` (1), `-Wrange-loop-bind-reference`, `-Wreserved-attribute-identifier`, `-Wreserved-identifier` (188), `-Wreserved-macro-identifier` (6), `-Wshadow-field`, `-Wshadow-uncaptured-local`, `-Wshorten-64-to-32` (44), `-Wsign-conversion` (442), `-Wstatic-in-inline` (2), `-Wsuggest-destructor-override`, `-Wsuggest-override`, `-Wswitch-default` (89), `-Wswitch-enum` (8), `-Wtautological-type-limit-compare` (3), `-Wtautological-unsigned-char-zero-compare`, `-Wtentative-definition-compat` (9), `-Wtentative-definition-incomplete-type` (1), `-Wundef` (7), `-Wundefined-internal-type`, `-Wundefined-reinterpret-cast`, `-Wunneeded-member-function`, `-Wunreachable-code-break` (6), `-Wunreachable-code-return` (34), `-Wunused-exception-parameter` (1), `-Wunused-macros` (26), `-Wunused-member-function`, `-Wunused-template`, `-Wvla` (66), `-Wzero-as-null-pointer-constant`

**P: extensões (só no estrito, com -Wpedantic)** (50):

`-Wattribute-preprocessor-tokens`, `-Wauto-decl-extensions`, `-Wbit-int-extension`, `-Wc++20-designator`, `-Wc11-extensions`, `-Wc23-extensions` (73), `-Wc2y-extensions` (15), `-Wc99-designator`, `-Wc99-extensions`, `-Wcomplex-component-init`, `-Wdelimited-escape-sequence-extension`, `-Wdollar-in-identifier-extension`, `-Wembedded-directive`, `-Wempty-translation-unit` (3), `-Wflexible-array-extensions` (4), `-Wformat-non-iso`, `-Wformat-pedantic`, `-Wgnu-anonymous-struct`, `-Wgnu-auto-type`, `-Wgnu-case-range`, `-Wgnu-complex-integer` (21), `-Wgnu-compound-literal-initializer` (1), `-Wgnu-conditional-omitted-operand` (26), `-Wgnu-designator` (15), `-Wgnu-empty-struct` (52), `-Wgnu-flexible-array-initializer` (8), `-Wgnu-flexible-array-union-member`, `-Wgnu-folding-constant`, `-Wgnu-imaginary-constant`, `-Wgnu-include-next`, `-Wgnu-label-as-value` (56), `-Wgnu-line-marker`, `-Wgnu-pointer-arith` (20), `-Wgnu-redeclared-enum`, `-Wgnu-statement-expression` (39), `-Wgnu-statement-expression-from-macro-expansion` (1), `-Wgnu-union-cast` (2), `-Wgnu-zero-line-directive`, `-Wgnu-zero-variadic-macro-arguments`, `-Wimport-preprocessor-directive-pedantic`, `-Wlanguage-extension-token` (118), `-Wnested-anon-types`, `-Woctal-prefix-extension`, `-Woverlength-strings` (2), `-Wpedantic` (28), `-Wpointer-arith` (2), `-Wvariadic-macro-arguments-omitted`, `-Wvariadic-macros` (4), `-Wvla-extension`, `-Wzero-length-array` (23)

**N: nunca (estilo, ABI, compatibilidade com padrões antigos, ruído)** (84):

`-Wabstract-vbase-init`, `-Watomic-implicit-seq-cst` (8), `-Wbind-to-temporary-copy`, `-Wbitfield-enum-conversion`, `-Wc++11-compat`, `-Wc++11-compat-reserved-user-defined-literal`, `-Wc++11-extensions`, `-Wc++11-extra-semi`, `-Wc++11-long-long`, `-Wc++11-narrowing`, `-Wc++14-attribute-extensions`, `-Wc++14-binary-literal`, `-Wc++17-attribute-extensions`, `-Wc++17-extensions`, `-Wc++20-attribute-extensions`, `-Wc++20-compat`, `-Wc++20-extensions` (1), `-Wc++23-attribute-extensions`, `-Wc++26-extensions`, `-Wc++98-c++11-compat-binary-literal`, `-Wc++98-compat` (23), `-Wc++98-compat-bind-to-temporary-copy`, `-Wc++98-compat-extra-semi`, `-Wc++98-compat-local-type-template-args` (5), `-Wc++98-compat-pedantic`, `-Wc++98-compat-unnamed-type-template-args`, `-Wc23-compat` (13), `-Wc99-compat`, `-Wcompound-token-split-by-space` (7), `-Wcovered-switch-default` (1), `-Wdate-time`, `-Wdeclaration-after-statement` (143), `-Wdeprecated-dynamic-exception-spec`, `-Wdeprecated-redundant-constexpr-static-def`, `-Wdisabled-macro-expansion` (12), `-Wexit-time-destructors` (1), `-Wexpansion-stmt-missing-braces`, `-Wgcc-compat`, `-Wglobal-constructors` (2), `-Widiomatic-parentheses`, `-Winvalid-utf8`, `-Wlifetime-safety-cross-tu-constructor-suggestions`, `-Wlifetime-safety-cross-tu-misplaced-lifetimebound`, `-Wlifetime-safety-cross-tu-suggestions`, `-Wlifetime-safety-inapplicable-lifetimebound`, `-Wlifetime-safety-intra-tu-constructor-suggestions` (1), `-Wlifetime-safety-intra-tu-misplaced-lifetimebound`, `-Wlifetime-safety-intra-tu-suggestions` (2), `-Wlifetime-safety-lifetimebound-violation`, `-Wlifetime-safety-noescape`, `-Wlong-long`, `-Wmissing-format-attribute` (4), `-Wmissing-noreturn` (90), `-Wmissing-variable-declarations` (1279), `-Wnewline-eof` (3), `-Wnrvo` (2), `-Wover-aligned`, `-Wpacked` (7), `-Wpadded` (244), `-Wpadded-bitfield` (14), `-Wpre-c++14-compat` (6), `-Wpre-c++17-compat` (8), `-Wpre-c++17-compat-pedantic`, `-Wpre-c++20-compat` (4), `-Wpre-c++20-compat-pedantic` (2), `-Wpre-c++23-compat` (1), `-Wpre-c++26-compat` (1), `-Wpre-c11-compat` (3), `-Wpre-c23-compat` (2), `-Wpre-c2y-compat`, `-Wredundant-parens` (4), `-Wshadow-field-in-constructor` (4), `-Wsigned-enum-bitfield`, `-Wstrict-prototypes` (1567), `-Wundefined-func-template`, `-Wunsafe-buffer-usage` (862), `-Wunsafe-buffer-usage-in-container`, `-Wunsafe-buffer-usage-in-format-attr-call`, `-Wunsafe-buffer-usage-in-libc-call` (3), `-Wunsafe-buffer-usage-in-static-sized-array`, `-Wunsafe-buffer-usage-in-unique-ptr-array-access`, `-Wused-but-marked-unused` (1), `-Wweak-template-vtables`, `-Wweak-vtables` (7)

**X: não se aplica (ObjC, CUDA, HLSL, OpenMP, MS, módulos, threads, Doxygen...)** (101):

`-Waarch64-sme-attributes`, `-Wamdgpu-unguarded-builtin-usage`, `-Warc-maybe-repeated-use-of-weak`, `-Warc-repeated-use-of-weak`, `-Wcompletion-handler`, `-Wconsumed`, `-Wcstring-format-directive`, `-Wcuda-compat`, `-Wcustom-atomic-properties`, `-Wdecls-in-multiple-modules`, `-Wdeprecated-implementations`, `-Wdirect-ivar-access`, `-Wdocumentation`, `-Wdocumentation-deprecated-sync`, `-Wdocumentation-html`, `-Wdocumentation-pedantic`, `-Wdocumentation-unknown-command`, `-Wduplicate-header-ownership`, `-Wduplicate-method-arg`, `-Wduplicate-method-match`, `-Wexplicit-ownership-type`, `-Wfunction-effect-redeclarations`, `-Wfunction-effects`, `-Whip-deprecated-builtins`, `-Whlsl-implicit-binding`, `-Wimplicit-atomic-properties`, `-Wimplicit-overflow-behavior-conversion`, `-Wimplicit-overflow-behavior-conversion-assignment`, `-Wimplicit-overflow-behavior-conversion-assignment-pedantic`, `-Wimplicit-overflow-behavior-conversion-function-boundary`, `-Wimplicit-overflow-behavior-conversion-function-boundary-pedantic`, `-Wimplicit-overflow-behavior-conversion-pedantic`, `-Wimplicit-retain-self`, `-Winvalid-or-nonexistent-directory`, `-Wmax-tokens`, `-Wmethod-signatures`, `-Wmicrosoft-anon-tag`, `-Wmicrosoft-charize`, `-Wmicrosoft-comment-paste`, `-Wmicrosoft-cpp-macro`, `-Wmicrosoft-empty-enum`, `-Wmicrosoft-end-of-file`, `-Wmicrosoft-enum-value`, `-Wmicrosoft-exception-spec`, `-Wmicrosoft-fixed-enum`, `-Wmicrosoft-flexible-array`, `-Wmicrosoft-redeclare-static`, `-Wmissing-include-dirs`, `-Wmmap-deprecated-symlink-to-modular-header`, `-Wmodule-file-mapping-mismatch`, `-Wmodule-map-path-outside-directory`, `-Wms-bitfield-padding` (10), `-Wnon-modular-include-in-framework-module`, `-Wnon-modular-include-in-module`, `-Wnonportable-include-path-separator`, `-Wnonportable-private-system-apinotes-path`, `-Wnonportable-system-include-path`, `-Wnullability-extension`, `-Wnullable-to-nonnull-conversion`, `-Wnvcc-compat`, `-Wobjc-interface-ivars`, `-Wobjc-messaging-id`, `-Wobjc-missing-property-synthesis`, `-Wobjc-property-assign-on-object-type`, `-Wobjc-signed-char-bool-implicit-int-conversion`, `-Wopenacc-extension`, `-Woverriding-method-mismatch`, `-Wpedantic-core-features`, `-Wperf-constraint-implies-noexcept`, `-Wpoison-system-directories`, `-Wpragma-pack-suspicious-include`, `-Wpre-openmp-51-compat`, `-Wprofile-instr-missing`, `-Wquoted-include-in-framework-header`, `-Wreceiver-forward-class`, `-Wretained-language-linkage`, `-Wselector`, `-Wselector-type-mismatch`, `-Wshadow-header`, `-Wsource-uses-openacc`, `-Wsource-uses-openmp` (1), `-Wspir-compat`, `-Wstrict-potentially-direct-selector`, `-Wstrict-selector-match`, `-Wsuper-class-method-mismatch`, `-Wthread-safety-analysis`, `-Wthread-safety-attributes`, `-Wthread-safety-beta`, `-Wthread-safety-negative`, `-Wthread-safety-pointer`, `-Wthread-safety-precise`, `-Wthread-safety-reference`, `-Wthread-safety-reference-return`, `-Wthread-safety-verbose`, `-Wunaligned-access` (1), `-Wundeclared-selector`, `-Wundef-prefix`, `-Wunguarded-availability`, `-Wunique-object-duplication`, `-Wunsupported-dll-base-class-template`, `-Wvector-conversion` (14)
