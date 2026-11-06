# Plan: checks for the student

Version 2 (2026-10-15), with the teacher's decisions (§11). Three
adversarial reviewers reviewed version 1, and each change cites the finding
behind it:

- **W:** warnings and the student's experience;
- **R:** runtime, wasm and loader;
- **T:** engine, cuts, tests and phases.

The experiments are in the session's scratchpad (`rev-avisos`, `rev-runtime`,
`rev-engine`).

The plan covers what `sc` checks in the student's code:

- clang's warnings (`-W`);
- the sanitizers (`-fsanitize`);
- libc++'s hardening;
- the initialization of local variables;
- wasm's traps.

It groups the checks into sets, proposes which ones are on by default and
says how to turn them off.

## 0. Principles

- **On by default.** The student sees the error where it is, with the line
  and in words. An error that goes by in silence is the worst case.
- **Can be turned off.** With the checks off, the teacher shows that the
  same program "works", gives a different result or corrupts data. That is
  what happens with an ordinary compiler. The `off` level turns off every
  run-time check.
- **No false positive at the default level.** A warning that fires on
  correct code teaches the student to ignore `sc`. What can be wrong stays
  in strict. The false positives the reviewers measured took several
  warnings out of the default (§4).
- **One message per error, and the program stops.** The message gives the
  kind of error, the file, the line and the stack
  (`-fno-sanitize-recover=all`). In the REPL, only the cell ends.
- **The project's rules.**
  - The engine does not change its defaults. `sc` chooses the checks.
  - The corpus also runs with the levels (§9).
  - A new option in `sc_inv.cpp` requires `make checkinv` SAME.

## 1. What the student sees today

Measured with `sc FILE` and `sc check` at commit a0f2310, on 2026-10-15:

| Program | Warnings today (`-Wall -Wextra`) | Run today |
|---|---|---|
| `int *p = NULL; *p = 3;` | nothing | **nothing**: writes to address 0 |
| `p[0]` in memory already freed | nothing | **nothing**: prints the old value |
| `free(p); free(p);` | nothing | **nothing** |
| `malloc` of 4 `int`, write to `p[4]` | nothing | **nothing** |
| `int a[3]; ... a[i] = 0` with `i` up to 3 | nothing | **nothing**: writes to another variable |
| `strcpy(a, "hello world")` with `char a[4]` | `-Wfortify-source` (W-A5) | **runs**: writes to `b` |
| `x = INT_MAX; x + 1` | nothing; only a literal `INT_MAX + 1` gives `-Winteger-overflow` (W-A5) | **nothing**: prints -2147483648 |
| `int s; s += i` | `-Wuninitialized` | prints an arbitrary value |
| `int` function without `return` | `-Wreturn-type` | prints 0 |
| return of a local `&x` | `-Wreturn-stack-address` | **nothing**: prints 5 |
| `printf("%d", 2.5)` | `-Wformat` | prints 0 |
| `v[5]` in a `vector` of 3 | nothing | `aborted`, without saying where |
| `*o` on an empty `optional`, `s[10]` | nothing | `aborted` |
| `1/z` with `z == 0` | nothing | `wasm trap: integer divide by zero`, without the line |
| endless recursion | `-Winfinite-recursion` | `stack overflow`, without the line |

The warnings exist, but the program runs anyway. The memory errors go by in
silence.

## 2. The layers

| Layer | What it catches | Cost | Where |
|---|---|---|---|
| 0. wasm traps | access outside memory, integer division by zero and `INT_MIN/-1`, exhausted call stack, indirect call with the wrong type, function still undefined, `unreachable` | none, it already exists | host: message and location |
| 1. Warnings | what can be seen without running | none at run time | `sc`'s flags |
| 2. UBSan | null, null with offset, signed overflow, shift, index into an array of known size, invalid `bool`/`enum`, out-of-range `float`→`int`, missing `return` (C++) | code 1.35–2.15×, depending on the mode (§5.1) | flags + handlers |
| 3. ASan | heap, stack and globals out of bounds, use after `free` and after scope, double or invalid `free`, the student's `str*` | code 1.5×; *shadow* of 512 MiB | new pass in the engine + `libsc_rt` + loader |
| 4. libc++ | `vector[]`, `string[]`, `optional`, `span` and iterators | only on the accesses | macros + one `std.pcm` per mode |
| 5. Initialization | an uninitialized local has a fixed value | one `store` per variable | `-ftrivial-auto-var-init=pattern` |
| 6. Location | file, line and column of each frame | code 1.2× (DWARF) | `-gline-tables-only` + wasmtime |

Layer 6 serves all the others. The traps of layers 0 and 4 and the ASan
reports say where they occurred only through it. UBSan has the location in
the handler's data and does not depend on it (T6).

## 3. Levels and control

There are three levels:

| | `off` | `on` (default) | `strict` |
|---|---|---|---|
| warnings | clang's default | `-Wall -Wextra` − R + A (§4) | + R + S + P, `-Wpedantic` |
| errors | clang's | + E (§4.1) | + E |
| UBSan | — | U (§5.1) | U + U-strict |
| ASan | — | yes (F3) | + leaks at the end of `sc FILE` |
| libc++ | `NONE` | `EXTENSIVE` | `DEBUG` |
| locals | no initialization | `pattern` | `pattern` |
| error location | yes | yes | yes |

- **Command line.** `sc --safety=off|on|strict FILE` and
  `sc repl --safety=...`. `check` accepts the option, but only the warnings
  and the errors matter to it.
- **In the REPL, the level holds for the whole session (T7).**
  - Today's `restart` redoes the log with parse and IR only
    (`proto/src/child.rs:185-193`), without generating a `.so` or running.
  - Changing the level would require recompiling and **rerunning** every
    cell. That would repeat the prints, consume stdin and change `rand` and
    `time`.
  - A cell accepted at `off` can be rejected at `on` because of E.
  - So `:safety LEVEL` restarts the session **empty**, with a warning. The
    level can also be chosen at startup (`sc repl --safety=off`).
- **At `off`, E goes back to being a warning.** That way the undefined
  behaviour example runs.
- **The error location holds at every level.** It only changes the message.
- **Fine tuning**, for the teacher: `sc ... -- -Wno-shadow -fno-sanitize=alignment`.
  - This requires the engine to **reject** a flag it does not know. Today
    `scParseArgs` (`sc_inv.cpp:190-227`) silently ignores everything that is
    not `-W` (T2).
  - Until then, `sc` only passes on `-W…`.

## 4. Warnings

**The numbers.**

- With `-Weverything`, the pin's clang 23.1.3 turns on 2265 diagnostics
  (`diagtool show-enabled`, wasm32-wasip1).
- 1450 are on without any flag. `-Wall` adds 59, and `-Wextra` 19 more.
- The other 737 are in **343 groups** that are off by default. Appendix A
  classifies each one of them.

**The sets:**

| Set | Criterion | Level |
|---|---|---|
| **E**: error | certain undefined behaviour, or an error with no legitimate use, with no measured false positive | `on`, `strict` |
| **D**: default | clang's default, `-Wall` and `-Wextra`, minus R | all (`off` stays at clang's default) |
| **R**: removed | from `-Wall` or `-Wextra`, but noise on a beginner's correct code | `strict` |
| **A**: added | off in clang; catches a common student error, with no measured false positive | `on`, `strict` |
| **L**: lifetime | `-Wlifetime-safety-*`, new in clang 23 | §4.4 |
| **S**: strict | style, conversions, portability | `strict` |
| **P**: pedantic | GNU and clang extensions (`-Wpedantic`) | `strict` |
| **N**: never | compatibility with old standards, ABI, large project style (`-Wunsafe-buffer-usage`) | — |
| **X**: does not apply | ObjC, CUDA, HLSL, OpenMP, Microsoft, modules, *thread safety*, Doxygen | — |

**`-Werror=G` also applies to the subgroups of G (W-A1).** Each entry of E
says what it takes out with `-Wno-error=`.

### 4.1 E: warnings that become errors

All of them are already on in clang or in `-Wall`:

| Error | Without the subgroups | Reason |
|---|---|---|
| `-Wuninitialized` | `-Wno-error=sometimes-uninitialized`, `-Wno-error=uninitialized-const-pointer` | `sometimes` rejects `if (x>0) r=1; else if (x<=0) r=2;` (W-A1) |
| `-Wreturn-type` | — | also rejects an `if/else if` that covers every case; the fix is trivial (W-A13) |
| `-Wreturn-stack-address`, `-Wdangling` | — | |
| `-Warray-bounds` | — | constant index |
| `-Wdivision-by-zero`, `-Wshift-count-overflow`, `-Wshift-count-negative`, `-Winteger-overflow` | — | constants (W-A5) |
| `-Wfortify-source` | — | a `strcpy` that certainly overflows (W-A5) |
| `-Wformat` | `-Wno-error=format-security`, `-Wno-error=format-extra-args`, `-Wno-error=format-zero-length` | `scanf("%d", x)` runs inside libc, which not even ASan sees. The warning is the only defence. `%d` with `sizeof` is UB, and the fix, `%zu`, is what we teach (W-A6). `-Wnonnull` is a subgroup of it |
| `-Wstring-compare` | — | `name == "ana"` (W-A5) |
| `-Wunsequenced` | — | `i = i++ + 1` in C. In C++17 it is defined and does not fire (W-A13) |
| `-Wmismatched-new-delete`, `-Wdelete-abstract-non-virtual-dtor`, `-Wfree-nonheap-object` | — | The non-abstract case (`delete d` with the exact type) is correct and stays a warning (W-A4) |

**Out of version 1's E:**

- `-Wnull-dereference`: it only catches `*(int*)0`, and the message
  confuses. UBSan covers the case (W-A13).
- `-Wdelete-non-abstract-non-virtual-dtor` (W-A4).

**Already errors in clang 23:**

- `-Wint-conversion`, `-Wincompatible-pointer-types`;
- `-Wimplicit-function-declaration`, `-Wimplicit-int`.

### 4.2 R: remove from the default

- `-Wunused-parameter`: fires on `main(int argc, char *argv[])` and on
  callbacks.
- `-Wmissing-field-initializers`: `struct S s = {1}` zeroes the rest.
- `-Wsign-compare`: it is the most frequent warning in a beginner's correct
  code, with `i < v.size()` and `i < strlen(s)`. It was the only `-Wall`
  warning in the typical C++ program.
  - Version 1's justification was wrong. `-1 < 1u` with literals does not
    fire the warning. Only the form with variables fires it (W-A10).
  - By the principle of §0, it goes to R (decision 2, §11).

### 4.3 A: add to the default

The number in parentheses is the number of corpus programs in which the
warning fires with `-Weverything`. The corpus is not student code, so the
number only indicates the noise.

- **Conversions:**
  - `-Wfloat-overflow-conversion` (1), `-Wfloat-zero-conversion`;
  - `-Wstring-conversion`: `"abc"` as `bool`;
  - `-Wshift-bool`;
  - `-Wshift-sign-overflow` (8): only in C. In C++20, `1 << 31` is defined
    (W-A13).
  - `int i = 2.7;` is already caught by default (`-Wliteral-conversion`).
  - `-Wfloat-conversion` went to S. `int lim = sqrt(n)` is common and
    intentional (W-A9; decision 3, §11).
- **Comparisons and loops:**
  - `-Wtautological-unsigned-zero-compare` (9):
    `for (size_t i = n - 1; i >= 0; i--)`;
  - `-Wtautological-unsigned-enum-zero-compare`;
  - `-Wtautological-value-range-compare` (13);
  - `-Wunreachable-code` (75), with the subgroups `-loop-increment` (7) and
    `-fallthrough`. The `-break` and `-return` forms stay out with `-Wno-`
    (W-A12).
- **Flow:** `-Wimplicit-fallthrough` (18) and the subgroup `-per-function`
  (W-A12).
- **Names:**
  - `-Wshadow` (54), **only in C and only in `sc FILE`**. In C++, the setter
    `this->x = x` fires, and clang has no group for locals alone (W-A7).
  - In the REPL, a global from an earlier cell would make every
    `for (int i …)` warn.
  - `-Wshadow-field-in-constructor-modified`.
  - `-Wcomma` (57).
- **C:** `-Wdefault-const-init-var` (6), `-Wdefault-const-init-field`,
  `-Wmain` (4), `-Wmain-return-type`. `-Wmain-return-type` already fires on
  `void main()` (W-A12).
- **C++:**
  - `-Wnon-virtual-dtor` (1);
  - `-Wdeprecated-copy-with-user-provided-dtor` (rule of three). After F3,
    it goes to S. It fires in the lifecycle class, and ASan catches the real
    double `free` with a better message (W-A11).
  - `-Warray-bounds-pointer-arithmetic`.
- **Went to S:**
  - `-Wconditional-uninitialized`: 5 false positives in 6 correct functions
    (W-A8). `pattern` shows the real error at run time.
  - `-Wclass-varargs`: the `std::string` case is already an error through
    `-Wnon-pod-varargs` (W-A12).

### 4.4 L: lifetime (C++ only)

- **`-Wlifetime-safety-permissive` goes to A.** It caught every bug in the
  test, with no false positive on linked lists, `unique_ptr::get` and local
  `static` (W-A2). It includes:
  - `use-after-scope`;
  - `return-stack-addr`;
  - `use-after-free`.
- **`invalidation` goes to S.** It fires on
  `for (auto &row : g) row.push_back(1)` and on `string &f = v[0]; f += "!"`
  (W-A2).
- **`dangling-field` and `dangling-global`** remain to be measured in F1.
- **In C**, the analysis only runs with `-fexperimental-lifetime-safety-c`
  (cc1), and even then it does not see `free(p); *p` (W-A3). It stays out
  until ASan.

### 4.5 Differences by mode

§29 of NOTES.md already takes the end-of-unit warnings out of the REPL.

In this proposal, only `-Wshadow` changes by mode. It stays in `sc FILE` in
C. E holds in both modes.

An error in one function hides the analysis warnings of the functions that
follow (W-A14). That is why each program in `test/checks` has a single bug.

## 5. Run-time checks

### 5.1 UBSan

| Check | Catches | Decision |
|---|---|---|
| `null` | access through a null pointer, including `s->field` (tested, R3) | **U** |
| `pointer-overflow` | `p[1000]` with a null `p` (tested, R3), arithmetic that wraps around | **U** |
| `alignment` | misaligned access; wasm accepts it, but it is UB | **U** |
| `array-bounds` | index out of an array of known size | **U** |
| `signed-integer-overflow` | `INT_MAX + 1`, `-INT_MIN` | **U** |
| `integer-divide-by-zero` | `/` and `%` by 0, with the line | **U** |
| `shift-base`, `shift-exponent` | `1 << 40`, `x << -1` | **U** |
| `vla-bound` | VLA with size ≤ 0 | **U** |
| `return` (C++) | end of a non-`void` function without `return` | **U** |
| `unreachable` | `__builtin_unreachable()` reached | **U** |
| `bool`, `enum` | invalid value loaded | **U** |
| `builtin` | `__builtin_clz(0)` | **U** |
| `float-cast-overflow` | `(int)1e10` | **U** |
| `nonnull-attribute`, `returns-nonnull-attribute` | `NULL` for a `nonnull` parameter of libc | **U** |
| `local-bounds` | — | emits nothing at `-O0` (measured) |
| `object-size` | — | has no effect at `-O0` (clang warns) |
| `unsigned-integer-overflow`, `unsigned-shift-base`, `implicit-conversion`, `float-divide-by-zero` | `unsigned` wraparound, implicit truncation, `1.0/0` | **U-strict**: they fire in the headers' generators and in the macros of `ctype.h`, which stay out (NOTES.md §40) |
| `nullability-*` | — | only with `_Nonnull` |
| `function` | — | not supported on wasm, and wasm already traps |
| `vptr` | — | only in Emscripten, needs a C++ runtime |

**The three modes (R6).** The measurement in F2 makes the decision:

| Mode | `c1_basics.cpp` | What the student sees |
|---|---|---|
| full handlers | 200,520 bytes (2.15×) | the line, plus the values of the operands and the types (`2147483647 + 1 cannot be represented in type 'int'`) |
| minimal runtime | 134,499 (1.44×) | the kind of error; the line comes from the stack (F0) |
| `-fsanitize-trap` | 125,988 (1.35×) | the kind of error, which goes in the DWARF (`__clang_trap_msg$Undefined Behavior Sanitizer$signed integer addition overflow in 'a + b'`); the line comes from F0 |

With `trap`, there is no runtime at all. The values of the operands are the
only saving of the handlers.

**If they are handlers, they stay in C, in `libsc_rt` (T10).**

- That way they serve `sc`, the corpus's `scload`, the wasmtime CLI and the
  browser.
- They call a single host import, for the stack and the trap.
- The symbols emitted with U (R6):
  - `add/sub/mul/negate_overflow`, `divrem_overflow`, `shift_out_of_bounds`;
  - `out_of_bounds`, `type_mismatch_v1`, `pointer_overflow`;
  - `vla_bound_not_positive`, `load_invalid_value`, `float_cast_overflow`;
  - `invalid_builtin`, `nonnull_arg`, `nonnull_return_v1`;
  - `alignment_assumption`, which comes from libc++'s headers.

  Each one has the `_abort` form, except `builtin_unreachable` and
  `missing_return`. `SourceLocation` is {ptr, u32, u32}, and an operand
  wider than 32 bits comes by pointer.

### 5.2 ASan

| Item | Decision |
|---|---|
| `address`, `use-after-scope` | **on** |
| `use-after-return` | `=never`: it needs the *fake stack*, and `-Wreturn-stack-address` catches the common case |
| `pointer-compare`, `pointer-subtract` | no: the driver rejects them on wasm |
| `leak` | **strict**, at the end of `sc FILE`; `:leaks` at `on` and at `strict`. Only the block that nothing reaches (NOTES.md §37) |
| `init-order` | no. `__asan_before_dynamic_init` and `__asan_after_dynamic_init` are emitted anyway and become no-ops (R7) |

**The pass has to go into the engine (T1).**

- `engine/sc_backend.cpp` replaces `BackendUtil.cpp` and runs only the
  AlwaysInliner, the coroutines and the codegen. `addSanitizers` exists in
  neither engine.
- `sc_backend` now runs `AddressSanitizerPass` in the new PM, like
  `runCoroPasses` (NOTES.md §16).
- With `-asan-use-stack-safety=0`. Without it, `StackSafetyGlobalAnalysis`
  pulls in ScalarEvolution (436 KB, cut today).
- The `-asan-*` options are `cl::opt`. `sc_inv` sets them in `LLVMArgs`
  (`sc_inv.cpp:173`), and `checkinv` covers them.

**Memory layout (R2, R3).**

- The stack still comes first, in [16, 1 MiB). That way an overflow still
  traps below 0.
  - Version 1 put the stack after the *shadow* with a guard zone. That does
    not work. The prologue of each instrumented frame rewrites the frame's
    own *shadow* and erases the guard's poison (R2).
- The *shadow* moves away from address 0 with `-mllvm -asan-mapping-offset=S`.
  - S cannot be a power of 2. With a power of 2, the pass uses OR
    (`AddressSanitizer.cpp:624-627`).
  - For example, S = 0x110000. The *shadow* sits in [S, S + 512 MiB) and the
    program starts after it.
- The memory starts out with S + 512 MiB. Every read of the *shadow* has to
  be inside the memory, including the read for an undefined address above
  3 GiB (R12).
- **No large poisoned regions.** Poisoning [3 GiB, 4 GiB) and
  [0, 64 MiB) would cost 192 MiB of writes, which become real RSS (R3).
  - UBSan already catches null with an offset.
  - The undefined symbol already works. The *shadow* read falls inside the
    memory, the access traps, and `undefined_at` gives the name.

**The runtime, `libsc_rt.so`.** It is C, PIC, and the loader loads it before
libc.

- **The ABI** (R7):
  - `__asan_init` and `__asan_version_mismatch_check_v8`;
  - `__asan_report_{load,store}{1,2,4,8,16,N}`, which can be host
    functions;
  - `__asan_set_shadow_00`, `__asan_alloca_poison` and
    `__asan_allocas_unpoison` (VLA);
  - `__asan_memcpy`, `__asan_memset` and `__asan_memmove`. The student's
    `memcpy` becomes these, so they need no interceptor.
  - `__asan_register_globals` and `__asan_unregister_globals`;
  - `__asan_handle_no_return`.

  Each global exports `__odr_asan_gen_<name>`, and these names are filtered
  out.
- **The allocator (R8).**
  - It is a wrapper around libc's `malloc`, with red zones, a header and a
    FIFO quarantine.
  - `libc.so` calls its own `malloc` directly, with 61 internal `call`s.
    `libc++.so` and `libc++abi.so` import `malloc`, `free`, `realloc`,
    `aligned_alloc` and `_Znwm`. The loader resolves them to the wrapper, so
    `new` also gets red zones.
  - A pointer from libc, like the one from `strdup`, has no header, and
    `free` passes it on.
  - `getline` and `getdelim` are intercepted, because they call `realloc` on
    a student pointer.
  - **The loader** allocates each cell's data with `aligned_alloc`
    (`lib.rs:222-230`). It now uses `__real_aligned_alloc`. Otherwise the
    globals would become heap blocks, and `free(&global)` would pass.
  - **The quarantine** unpoisons the block when it gives it back to libc.
    Otherwise the next `strdup` would report a false positive.
- **Interceptors:** `strcpy`, `strncpy`, `strcat`, `strncat`, `sprintf`,
  `snprintf`, `fgets`, `fread` and `read`.
  - The loader hands libc to the runtime as `__real_*`.
  - A call from inside libc is not intercepted.
  - `scanf("%s")` was left for F4 (NOTES.md §39).
- **Exceptions and `longjmp` (R9).**
  - `__asan_handle_no_return` is emitted before the student's `__cxa_throw`
    and before `longjmp` (tested).
  - The `throw`s from inside `libc++.so`, like `at` and `stoi`, do not go
    through it. The loader forwards the `__cxa_throw` and `__cxa_rethrow`
    that `libc++.so` imports to a wrapper in the runtime.
  - The risk is smaller than it looked. The prologue rewrites the frame's
    *shadow*.
- **The reports.**
  - The host reads the *shadow* to classify the error: heap, stack, global,
    freed, scope.
  - The allocation stack is only the top frames, compact (module and
    offset), captured in the host. The cost will be measured with a program
    that uses `std::map` (R12).
- **REPL.** After a trap, the machine restores the *stack pointer* and
  clears the *shadow* of the abandoned stack.

**For later: the instrumented libc.** Building wasi-libc with ASan and
`MALLOC_IMPL=none` would catch `scanf` and `printf("%s")` inside libc. That
requires building libc from source. In F4, it stayed with the interceptors
of the functions that read (NOTES.md §41).

### 5.3 libc++ (R4, R5)

- The sysroot already has `LIBCXX_HARDENING_MODE=fast`. Its semantics is
  `QUICK_ENFORCE`. That is a trap with no message, today's `aborted`.
- **`on`:** `EXTENSIVE`, still with `QUICK_ENFORCE`.
  - With `-g`, the trap's message is already in the DWARF:
    `__clang_trap_msg$libc++$vector.h:414: libc++ Hardening assertion __n < size() failed: vector[] index out of bounds`
    (R4).
  - F0 shows that message and the student's line. The wasmtime CLI did not
    show the inline frame of `__clang_trap_msg`, so F0 reads the DWARF with
    `addr2line` directly, if needed (R10).
- **`strict`:** `DEBUG`, which already uses `ENFORCE`, with `verbose_abort`
  and the message.
- **`off`:** `NONE`.
- **Out:** `_LIBCPP_ASSERTION_SEMANTIC`. It is experimental, gives `#error`
  without `-fexperimental-library`, and that flag changes the `.pcm`.
- **One `std.pcm` per mode is mandatory.** The module accepts a different
  macro without complaint and uses the value from when it was built (R5,
  tested with `-DFOO`). Without one `.pcm` per mode, the hardening would
  have no effect with `import std`.
  - `-fsanitize`, `-g` and `pattern` do not conflict with the `.pcm`
    (tested).
  - Mixing cells of different modes is safe. Each mode has its own ODR
    signature (T7).

### 5.4 Initialization of locals (R1)

**`on` and `strict`:** `-ftrivial-auto-var-init=pattern`. On wasm32,
32-bit pointers make `PatternInit.cpp:17-20` choose `0xFF…`:

| Type | Value |
|---|---|
| integer | -1 |
| pointer | 0xFFFFFFFF; above 3 GiB, so it already traps in F2, without ASan |
| `double` | NaN |
| `char[]` | `\xFF` repeated |

-1 is less visible than a pattern. A one-line patch would put 0xAAAAAAAA in
the integers and leave the pointer at 0xFFFFFFFF. It waits until after a
measurement. **Measured and done** (NOTES.md §42). The counter the student
forgot to zero came out one short. `llvm/pattern-init.patch` puts 0xAA… in
the integers, without changing the code size.

**A trap near 0xFFFFFFxx is ambiguous.** It can be an uninitialized pointer
or the overflow of the linear stack below 0. `runtime_error` tells the two
cases apart by `stack_pointer()`. If it wrapped around, it is an overflow.

**`off`:** the local keeps whatever was on the stack.

**Discarded:**

- `zero`, which hides the error;
- MSan, which needs an instrumented libc and libc++ and has no support on
  wasm.

### 5.5 Sanitizers out of scope

| Sanitizer | Why |
|---|---|
| `memory` | §5.4 |
| `thread` | the REPL has no threads |
| `hwaddress`, `memtag-*`, `kernel-*` | hardware or kernel |
| `cfi-*`, `kcfi`, `safe-stack`, `shadow-call-stack` | wasm already checks the indirect call and keeps the return stack separate |
| `type` | *strict aliasing* changes nothing at `-O0` |
| `dataflow`, `numerical`, `realtime`, `fuzzer`, `scudo`, `alloc-token` | another purpose |
| `-fstack-protector`, `_FORTIFY_SOURCE` | ASan is better; wasi-libc has no *fortify* |

**Note for the teacher.** On wasm, overflowing a local array corrupts other
variables, but never the return address. At `off`, classic
*stack smashing* does not bring the program down as it does on x86.

### 5.6 The error location (layer 6; T6, R10)

wasmtime already does almost everything:

- `Config::wasm_backtrace_details(Enable)`, with the default `addr2line`
  feature, gives file, line and column per frame. The default only reads
  `WASMTIME_BACKTRACE_DETAILS`.
- The `.debug_line` of a PIC `.so` is relative to the module. The reviewer
  saw `d.c:2:22` in a `.so` loaded by `scload`.
- The host formats the student's frames with `frame.symbols()`.

**Pitfalls:**

- wasmtime demangles `f` as `float`. Use the raw name, and demangle only the
  `_Z` names, as the loader already does.
- The default is 20 frames. In a recursion, merge the repeated frames.
- **In the incremental session, the DWARF has never been exercised (T8).**
  Each cell is a new module, named `cell_N`. A `test-split` pass with
  `-gline-tables-only` goes into F0.

**The trap messages:**

| Trap | Message |
|---|---|
| `out of bounds memory access` | `invalid memory access` or `stack overflow`, by `stack_pointer()` (§5.4) |
| `integer divide by zero` | `division by zero` |
| `integer overflow` | `INT_MIN / -1` |
| `call stack exhausted` | `stack overflow (infinite recursion?)` |
| `indirect call type mismatch` | `call through a function pointer of the wrong type` |
| `uninitialized element` | `call through a null function pointer`, or `undefined symbol` if it is the slot of `reserve_slot` (R11) |

## 6. Architecture

- **Flags.**
  - The sets live in `scripts/flags.py`, in a function
    `safety(level, cxx, mode)`.
  - `flags.py --session` receives the level and generates
    `build/repl/<lang>-<level>.args`. Today there is one file per
    language (`machine.rs:86`) (T3).
- **`sc_inv.cpp`.**
  - **Strict parse (T2).** `scParseArgs` now rejects with an error a flag it
    does not know.
  - **New flags:** `-D`, for libc++'s macros; today there is no
    `PreprocessorOptions` code.
  - **New flags:** `-fsanitize=`, `-fsanitize-recover=`, `-fsanitize-trap=`,
    `-fsanitize-merge=`, `-fsanitize-address-use-after-scope`,
    `-fsanitize-address-globals-dead-stripping`.
  - **New flags:** `-ftrivial-auto-var-init=`, `-debug-info-kind=`,
    `-dwarf-version=`, `-debugger-tuning=`.
  - **`-mllvm -asan-*`.**
  - **`sc_inv.cpp:107-108`** today sets the opposite defaults of
    `SanitizeMemoryParamRetval` and `UseOdrIndicator`. `flags.py` decides
    each one.
- **`checkinv` (T3).**
  - The *round trip* already serializes `Sanitize*`, `DebugInfo` and `-D`.
  - `TrivialAutoVarInit` enters the field by field comparison.
  - It gains some 8 lines: {C pic, C++ pic, C++ pic mod, C++ pcm} ×
    {on, strict}.
- **`sc_backend.cpp`:** the ASan pass (§5.2, T1).
- **Cuts (T5).**
  - **The REPL and `sc FILE` use `sc-engine-full`**, native and without cuts
    (`machine.rs:85`). F0, F1 and F2 in the REPL do not depend on `learn`.
  - `learn` only matters for `sc.wasm` and `build/sc`. Measured in the
    `engine.map` files:
    - `CGDebugInfo`: 376 KB in the full one, 18 KB in the cut one;
    - AsmPrinter's DWARF: 366 KB and 54 KB;
    - ASan: 95 KB and 0;
    - UBSan (`EmitCheck*`) and `pattern`: not cut.
  - The ceiling is about 0.8 MB in 24 MB, about 3%.
- **Loader (`loader/src/lib.rs`).**
  - It loads `libsc_rt.so` first and resolves libc++'s imports to it
    (§5.2).
  - It hands libc over as `__real_*`.
  - It sets up the memory with the *shadow*.
  - It turns on `wasm_backtrace_details`.
  - It stays in `lib.rs`, and not in `sc`, because `split.py` and
    `corpus.py` run through `scload` (T9).
- **Sysroot.** `make sysroot` now builds `libsc_rt.so` (PIC and static) and
  the `std.pcm` of each hardening mode.

## 7. Costs

**Measured** with the pin's native clang, which runs the full pipeline. The
engine only runs the ASan passes after T1. They are wasm32 PIC objects at
`-O0`, in bytes:

| Program | nothing | UBSan (handlers) | ASan | both | `-gline-tables-only` |
|---|---|---|---|---|---|
| `test/cxx/c3_stl.cpp` | 442,544 | 1,003,900 (2.27×) | 644,268 (1.46×) | 1,211,950 (2.74×) | 530,941 (1.20×) |
| `test/cxx/c1_basics.cpp` | 93,205 | 199,129 (2.14×) | 138,593 (1.49×) | 244,068 (2.62×) | — |

UBSan with handlers weighs more than ASan at `-O0`, because libc++'s
templates get every check. In `trap` mode, the factor drops to 1.35×
(§5.1).

**To measure:**

- the latency per cell in `sc-engine-full`, with UBSan and ASan separately
  (T11);
- the run time of the corpus at each level;
- the stack capture in each `malloc`;
- the engine after `learn`.

## 8. Phases (reordered by T)

Each phase closes with `make test` at 0 differences, `test-split`,
`checkinv` SAME and the record in NOTES.md.

| Phase | Delivers | Size |
|---|---|---|
| **F1** | the strict parse of `sc_inv` (T2); the levels in `flags.py`, in `sc` and in `checkinv` (T3); the corpus at several levels (T4); the E, R, A and L warnings (§4); measure L and the noise on the student corpus | medium |
| **F0** | `wasm_backtrace_details`; traps in words; `-gline-tables-only` in the native REPL, plus `test-split` with `-g` (T8) | small |
| **F2** | UBSan with the mode the measurement chooses (§5.1); libc++ `EXTENSIVE` and one `std.pcm` per mode; `pattern` | medium |
| **F2b** | `learn` for `sc.wasm`: DWARF and whatever else is missing | small |
| **F3** | the ASan pass in `sc_backend` (T1); `libsc_rt`; the layout with an offset (R2); the allocator and the loader (R8); `__cxa_throw` (R9); the report with a stack | large |
| **F4** | `strict`: U-strict, leaks, libc++ `DEBUG`, `scanf("%s")`; optional: instrumented libc | medium |

**F1 is done** (NOTES.md §30).
- The cell now counts as the main file
  (`llvm/incremental-main-file.patch`). Before that, `-Wunreachable-code`
  did not fire, and `-Wheader-hygiene` gave a false positive.
- `-Wmissing-prototypes` moved from S to N. It fired on 16 of the 20
  correct programs of `test/student` at `strict`.

**F0 is done** (NOTES.md §31). It has the error in words and the student's
frames, with `-gline-tables-only` in every session, `test-split` with `-g`
(T8) and `make test-checks`.

**F2 is done** (NOTES.md §32). It has UBSan with handlers in `libsc_rt`
(`rt/`), libc++'s `EXTENSIVE` with the `enforce` semantics in the sysroot,
`pattern`, and one `std.pcm` per level in the corpus. A cell takes 102 ms
at `on` against 61 ms at `off`.

**F2b is done** (NOTES.md §33). The corpus compiles the `pic-` and `repl-`
programs and the levels with `-gline-tables-only`, and `learn` gave the
DWARF back to `sc.wasm` (+211 KB).

**F3 is done** (NOTES.md §34). It has the ASan pass in `sc_backend`, the
runtime in `rt/sc_asan.c`, the *shadow* at 0x110000, the allocator with a
quarantine, the `--wrap` names in the loader and `__cxa_throw`.
`llvm/asan-cleanupret.patch` fixes ASan with wasm exceptions.
`-Wdeprecated-copy-with-user-provided-dtor` went to S. A cell takes 115 ms
at `on` against 68 ms at `off`, and `sc.wasm` grew by 78 KB.

**F3b is done** (NOTES.md §36). The report says where the block was freed
and allocated, with stacks that the loader captures and symbolizes only for
the report. Only the first 20 thousand blocks keep their stack, and a
`std::map` of 100 thousand keys takes the same time as before (R12).
`sc FILE` with `setjmp`/`longjmp` was also fixed (§35).

**F4's leaks are done** (NOTES.md §37). They show at the end of `sc FILE`
at `strict` and in the REPL's `:leaks`.

**F4's libc++ `DEBUG` is done** (NOTES.md §38). `strict` catches a
comparator that is not a strict weak order, like `<=` in `std::sort`.

**F4's `scanf("%s")` is done** (NOTES.md §39). It works at `on` and at
`strict`, like the other interceptors.

**F4's U-strict is done** (NOTES.md §40). It works at `strict`, outside the
sysroot's headers (an ignorelist), and without musl's `isdigit`... macros,
which wrap around in the student's code.

**F4's instrumented libc stayed with the interceptors** (NOTES.md §41). The
functions that read the program's strings (`strlen`, `strcmp`,
`printf("%s")`, `puts`, `atoi`...) check what they read, with the same
`--wrap` as before, without a second libc. F4 is done.

F1 comes first, because it is the foundation:

- `sc_inv` now accepts and checks the flags;
- the corpus now tests the levels.

## 9. Tests

- **`test/student/`: correct student code (T9, W).**
  - These are introductory exercises in C and in C++ with the STL: loops,
    vectors, strings, structs, lists, matrices, graphs, classes, `map`.
  - **It is the gate of the `on` level:** no warning from E or A and no
    report at run time.
  - The correct programs of reviewer W (`rev-avisos/ok/`) are the start.
  - Today's corpus does not serve for this. Of its 1947 programs, 1698 come
    from gcc torture, which has UB on purpose. `test/cxx` plus
    `test/samples` are only 25 files.
- **`test/checks/`: one bug per program (W-A14).** Each program carries the
  expected output at each level. At `off` that is the silent behaviour, and
  at `on` the message and the line. `make test-checks` compares.
- **The corpus at several levels (T4).**
  - `corpus.py` runs each set of flags in the same round, with suffixed
    names (`-on`, `-strict`).
  - The base comes from `sc-full`.
  - `learn` learns the union.
  - E is kept apart from the run-time tests. Otherwise the gnu17 programs of
    torture stop at compilation.
- **`test-split` with a level.** `split.py` passes the level to
  `session(...)`. The difference from the base without checks becomes a list
  of reports for triage. Each one is either real UB in the test or a false
  positive, which moves the check down to strict.
- **The static programs** of the corpus, which run on the wasmtime CLI,
  link with `libsc_rt.a` (T10).

## 10. Risks

1. **ASan with dynamic linking.** Registering the globals cell by cell
   works. The `.so` links and the constructor runs with the relocated
   metadata (R11). `SC_SYMS` has only names, so the size of the globals
   does not matter (R11). Undo and replay with ASan remain to be tested.
2. **The `throw`s from inside libc++** without `__asan_handle_no_return`
   (R9, §5.2).
3. **DWARF in the incremental session** (T8). The test goes into F0.
4. **512 MiB of memory.** It comes with the memory from the start and
   cannot grow later (R12).
   - On native, that is only `mprotect`.
   - On Windows, committed memory counts.
   - In the browser it can fail. In that case, ASan starts off in
     `sc.wasm`.
5. **The REPL's latency.** If `on` makes the cell too slow, the REPL uses
   UBSan in `trap` mode and leaves ASan only for `sc FILE`. The measurement
   decides (T11).
6. **False positives in libc++'s headers.** The gate is `test/student` in
   C++ (§9).

Risk 4 of version 1, the `std.pcm`, is solved. One `.pcm` per mode is
mandatory (R5).

## 11. The teacher's decisions (2026-10-15)

1. **E becomes an error** at the `on` and `strict` levels, with `-Wformat`
   and with the exceptions of §4.1. At `off`, it goes back to being a
   warning.
2. **`-Wsign-compare` leaves the default** and goes to R, only at `strict`.
3. **`-Wfloat-conversion` stays only at `strict`.** The default keeps
   `-Wfloat-overflow-conversion` and `-Wfloat-zero-conversion`.
4. **UBSan uses the full handlers**, if the latency measured in F2 allows
   it. If it does not, the REPL uses `trap` mode, and `sc FILE` keeps the
   handlers.
5. **Leaks show only at `strict`.** The REPL's `:leaks` works at `on` and at
   `strict`. `off` does not have the allocator that knows the blocks
   (2026-10-16). A leak is a block that nothing reaches, as in LSan, and it
   changes a status 0 to 1 (NOTES.md §37).
6. **`strict` only turns on `-Wpedantic`.** The dialect stays `gnu23` and
   `gnu++26`.
7. **The control is called `--safety=off|on|strict`**, in the REPL `:safety
   LEVEL`. It restarts the session empty, with a warning. The name says what
   you lose by turning it off and is not confused with the `check` command.

## Appendix A: the 343 groups off by default

The groups are classified by the sets of §4. The number in parentheses is
the number of corpus programs in which the group fires. With no number, it
fires in none.

The list comes from `diagtool show-enabled` with and without
`-Weverything`, from the pin's native clang. The script is `classify.py`, in
the session's scratchpad.

**A: turn on at the default level (`-Wshadow` only in C; `-Wdeprecated-copy-with-user-provided-dtor` goes back to S after F3)** (23):

`-Warray-bounds-pointer-arithmetic`, `-Wcomma` (57), `-Wdefault-const-init-field`, `-Wdefault-const-init-var` (6), `-Wdeprecated-copy-with-user-provided-dtor`, `-Wfloat-overflow-conversion` (1), `-Wfloat-zero-conversion`, `-Wimplicit-fallthrough` (18), `-Wimplicit-fallthrough-per-function`, `-Wmain` (4), `-Wmain-return-type` (5), `-Wnon-virtual-dtor` (1), `-Wshadow` (54), `-Wshadow-field-in-constructor-modified`, `-Wshift-bool`, `-Wshift-sign-overflow` (8), `-Wstring-conversion`, `-Wtautological-unsigned-enum-zero-compare`, `-Wtautological-unsigned-zero-compare` (9), `-Wtautological-value-range-compare` (13), `-Wunreachable-code` (75), `-Wunreachable-code-fallthrough`, `-Wunreachable-code-loop-increment` (7)

**L: lifetime analysis, C++ only; the `-permissive` form (use-after-scope, return-stack-addr, use-after-free) goes to A; dangling-field/global to be measured** (9):

`-Wlifetime-safety-dangling-field`, `-Wlifetime-safety-dangling-field-moved`, `-Wlifetime-safety-dangling-global`, `-Wlifetime-safety-dangling-global-moved`, `-Wlifetime-safety-return-stack-addr`, `-Wlifetime-safety-return-stack-addr-moved`, `-Wlifetime-safety-use-after-free`, `-Wlifetime-safety-use-after-scope`, `-Wlifetime-safety-use-after-scope-moved`

**S: turn on only at the strict level** (76):

`-Walloca` (12), `-Wanon-enum-enum-conversion`, `-Wassign-enum` (6), `-Wbad-function-cast` (8), `-Wc++-compat` (68), `-Wc++-hidden-decl` (3), `-Wc++-keyword` (27), `-Wc++-unterminated-string-initialization` (7), `-Wcast-align` (36), `-Wcast-function-type-strict` (7), `-Wcast-qual` (24), `-Wclass-varargs` (21), `-Wconditional-uninitialized` (48), `-Wconversion` (5), `-Wctad-maybe-unsupported`, `-Wdeprecated-copy-with-dtor`, `-Wdouble-promotion` (66), `-Wdtor-name`, `-Wduplicate-decl-specifier`, `-Wduplicate-enum`, `-Wenum-compare-conditional`, `-Wenum-enum-conversion`, `-Wenum-float-conversion`, `-Wexpansion-to-defined`, `-Wextra-semi` (12), `-Wextra-semi-stmt` (42), `-Wfloat-conversion` (58), `-Wfloat-equal` (82), `-Wformat-nonliteral` (1), `-Wformat-signedness` (21), `-Wformat-type-confusion`, `-Wfour-char-constants`, `-Wheader-hygiene`, `-Wignored-base-class-qualifiers`, `-Wimplicit-float-conversion` (29), `-Wimplicit-int-conversion` (203), `-Wimplicit-int-conversion-on-negation` (9), `-Wimplicit-int-enum-cast` (9), `-Wimplicit-int-float-conversion` (22), `-Wimplicit-void-ptr-cast` (110), `-Wincompatible-function-pointer-types-strict` (8), `-Winconsistent-missing-destructor-override`, `-Wjump-misses-init` (9), `-Wkeyword-macro` (2), `-Wlifetime-safety-invalidation`, `-Wmissing-prototypes` (3002), `-Wold-style-cast` (1), `-Wrange-loop-bind-reference`, `-Wreserved-attribute-identifier`, `-Wreserved-identifier` (188), `-Wreserved-macro-identifier` (6), `-Wshadow-field`, `-Wshadow-uncaptured-local`, `-Wshorten-64-to-32` (44), `-Wsign-conversion` (442), `-Wstatic-in-inline` (2), `-Wsuggest-destructor-override`, `-Wsuggest-override`, `-Wswitch-default` (89), `-Wswitch-enum` (8), `-Wtautological-type-limit-compare` (3), `-Wtautological-unsigned-char-zero-compare`, `-Wtentative-definition-compat` (9), `-Wtentative-definition-incomplete-type` (1), `-Wundef` (7), `-Wundefined-internal-type`, `-Wundefined-reinterpret-cast`, `-Wunneeded-member-function`, `-Wunreachable-code-break` (6), `-Wunreachable-code-return` (34), `-Wunused-exception-parameter` (1), `-Wunused-macros` (26), `-Wunused-member-function`, `-Wunused-template`, `-Wvla` (66), `-Wzero-as-null-pointer-constant`

**P: extensions (only at strict, with -Wpedantic)** (50):

`-Wattribute-preprocessor-tokens`, `-Wauto-decl-extensions`, `-Wbit-int-extension`, `-Wc++20-designator`, `-Wc11-extensions`, `-Wc23-extensions` (73), `-Wc2y-extensions` (15), `-Wc99-designator`, `-Wc99-extensions`, `-Wcomplex-component-init`, `-Wdelimited-escape-sequence-extension`, `-Wdollar-in-identifier-extension`, `-Wembedded-directive`, `-Wempty-translation-unit` (3), `-Wflexible-array-extensions` (4), `-Wformat-non-iso`, `-Wformat-pedantic`, `-Wgnu-anonymous-struct`, `-Wgnu-auto-type`, `-Wgnu-case-range`, `-Wgnu-complex-integer` (21), `-Wgnu-compound-literal-initializer` (1), `-Wgnu-conditional-omitted-operand` (26), `-Wgnu-designator` (15), `-Wgnu-empty-struct` (52), `-Wgnu-flexible-array-initializer` (8), `-Wgnu-flexible-array-union-member`, `-Wgnu-folding-constant`, `-Wgnu-imaginary-constant`, `-Wgnu-include-next`, `-Wgnu-label-as-value` (56), `-Wgnu-line-marker`, `-Wgnu-pointer-arith` (20), `-Wgnu-redeclared-enum`, `-Wgnu-statement-expression` (39), `-Wgnu-statement-expression-from-macro-expansion` (1), `-Wgnu-union-cast` (2), `-Wgnu-zero-line-directive`, `-Wgnu-zero-variadic-macro-arguments`, `-Wimport-preprocessor-directive-pedantic`, `-Wlanguage-extension-token` (118), `-Wnested-anon-types`, `-Woctal-prefix-extension`, `-Woverlength-strings` (2), `-Wpedantic` (28), `-Wpointer-arith` (2), `-Wvariadic-macro-arguments-omitted`, `-Wvariadic-macros` (4), `-Wvla-extension`, `-Wzero-length-array` (23)

**N: never (style, ABI, compatibility with old standards, noise)** (84):

`-Wabstract-vbase-init`, `-Watomic-implicit-seq-cst` (8), `-Wbind-to-temporary-copy`, `-Wbitfield-enum-conversion`, `-Wc++11-compat`, `-Wc++11-compat-reserved-user-defined-literal`, `-Wc++11-extensions`, `-Wc++11-extra-semi`, `-Wc++11-long-long`, `-Wc++11-narrowing`, `-Wc++14-attribute-extensions`, `-Wc++14-binary-literal`, `-Wc++17-attribute-extensions`, `-Wc++17-extensions`, `-Wc++20-attribute-extensions`, `-Wc++20-compat`, `-Wc++20-extensions` (1), `-Wc++23-attribute-extensions`, `-Wc++26-extensions`, `-Wc++98-c++11-compat-binary-literal`, `-Wc++98-compat` (23), `-Wc++98-compat-bind-to-temporary-copy`, `-Wc++98-compat-extra-semi`, `-Wc++98-compat-local-type-template-args` (5), `-Wc++98-compat-pedantic`, `-Wc++98-compat-unnamed-type-template-args`, `-Wc23-compat` (13), `-Wc99-compat`, `-Wcompound-token-split-by-space` (7), `-Wcovered-switch-default` (1), `-Wdate-time`, `-Wdeclaration-after-statement` (143), `-Wdeprecated-dynamic-exception-spec`, `-Wdeprecated-redundant-constexpr-static-def`, `-Wdisabled-macro-expansion` (12), `-Wexit-time-destructors` (1), `-Wexpansion-stmt-missing-braces`, `-Wgcc-compat`, `-Wglobal-constructors` (2), `-Widiomatic-parentheses`, `-Winvalid-utf8`, `-Wlifetime-safety-cross-tu-constructor-suggestions`, `-Wlifetime-safety-cross-tu-misplaced-lifetimebound`, `-Wlifetime-safety-cross-tu-suggestions`, `-Wlifetime-safety-inapplicable-lifetimebound`, `-Wlifetime-safety-intra-tu-constructor-suggestions` (1), `-Wlifetime-safety-intra-tu-misplaced-lifetimebound`, `-Wlifetime-safety-intra-tu-suggestions` (2), `-Wlifetime-safety-lifetimebound-violation`, `-Wlifetime-safety-noescape`, `-Wlong-long`, `-Wmissing-format-attribute` (4), `-Wmissing-noreturn` (90), `-Wmissing-variable-declarations` (1279), `-Wnewline-eof` (3), `-Wnrvo` (2), `-Wover-aligned`, `-Wpacked` (7), `-Wpadded` (244), `-Wpadded-bitfield` (14), `-Wpre-c++14-compat` (6), `-Wpre-c++17-compat` (8), `-Wpre-c++17-compat-pedantic`, `-Wpre-c++20-compat` (4), `-Wpre-c++20-compat-pedantic` (2), `-Wpre-c++23-compat` (1), `-Wpre-c++26-compat` (1), `-Wpre-c11-compat` (3), `-Wpre-c23-compat` (2), `-Wpre-c2y-compat`, `-Wredundant-parens` (4), `-Wshadow-field-in-constructor` (4), `-Wsigned-enum-bitfield`, `-Wstrict-prototypes` (1567), `-Wundefined-func-template`, `-Wunsafe-buffer-usage` (862), `-Wunsafe-buffer-usage-in-container`, `-Wunsafe-buffer-usage-in-format-attr-call`, `-Wunsafe-buffer-usage-in-libc-call` (3), `-Wunsafe-buffer-usage-in-static-sized-array`, `-Wunsafe-buffer-usage-in-unique-ptr-array-access`, `-Wused-but-marked-unused` (1), `-Wweak-template-vtables`, `-Wweak-vtables` (7)

**X: does not apply (ObjC, CUDA, HLSL, OpenMP, MS, modules, threads, Doxygen...)** (101):

`-Waarch64-sme-attributes`, `-Wamdgpu-unguarded-builtin-usage`, `-Warc-maybe-repeated-use-of-weak`, `-Warc-repeated-use-of-weak`, `-Wcompletion-handler`, `-Wconsumed`, `-Wcstring-format-directive`, `-Wcuda-compat`, `-Wcustom-atomic-properties`, `-Wdecls-in-multiple-modules`, `-Wdeprecated-implementations`, `-Wdirect-ivar-access`, `-Wdocumentation`, `-Wdocumentation-deprecated-sync`, `-Wdocumentation-html`, `-Wdocumentation-pedantic`, `-Wdocumentation-unknown-command`, `-Wduplicate-header-ownership`, `-Wduplicate-method-arg`, `-Wduplicate-method-match`, `-Wexplicit-ownership-type`, `-Wfunction-effect-redeclarations`, `-Wfunction-effects`, `-Whip-deprecated-builtins`, `-Whlsl-implicit-binding`, `-Wimplicit-atomic-properties`, `-Wimplicit-overflow-behavior-conversion`, `-Wimplicit-overflow-behavior-conversion-assignment`, `-Wimplicit-overflow-behavior-conversion-assignment-pedantic`, `-Wimplicit-overflow-behavior-conversion-function-boundary`, `-Wimplicit-overflow-behavior-conversion-function-boundary-pedantic`, `-Wimplicit-overflow-behavior-conversion-pedantic`, `-Wimplicit-retain-self`, `-Winvalid-or-nonexistent-directory`, `-Wmax-tokens`, `-Wmethod-signatures`, `-Wmicrosoft-anon-tag`, `-Wmicrosoft-charize`, `-Wmicrosoft-comment-paste`, `-Wmicrosoft-cpp-macro`, `-Wmicrosoft-empty-enum`, `-Wmicrosoft-end-of-file`, `-Wmicrosoft-enum-value`, `-Wmicrosoft-exception-spec`, `-Wmicrosoft-fixed-enum`, `-Wmicrosoft-flexible-array`, `-Wmicrosoft-redeclare-static`, `-Wmissing-include-dirs`, `-Wmmap-deprecated-symlink-to-modular-header`, `-Wmodule-file-mapping-mismatch`, `-Wmodule-map-path-outside-directory`, `-Wms-bitfield-padding` (10), `-Wnon-modular-include-in-framework-module`, `-Wnon-modular-include-in-module`, `-Wnonportable-include-path-separator`, `-Wnonportable-private-system-apinotes-path`, `-Wnonportable-system-include-path`, `-Wnullability-extension`, `-Wnullable-to-nonnull-conversion`, `-Wnvcc-compat`, `-Wobjc-interface-ivars`, `-Wobjc-messaging-id`, `-Wobjc-missing-property-synthesis`, `-Wobjc-property-assign-on-object-type`, `-Wobjc-signed-char-bool-implicit-int-conversion`, `-Wopenacc-extension`, `-Woverriding-method-mismatch`, `-Wpedantic-core-features`, `-Wperf-constraint-implies-noexcept`, `-Wpoison-system-directories`, `-Wpragma-pack-suspicious-include`, `-Wpre-openmp-51-compat`, `-Wprofile-instr-missing`, `-Wquoted-include-in-framework-header`, `-Wreceiver-forward-class`, `-Wretained-language-linkage`, `-Wselector`, `-Wselector-type-mismatch`, `-Wshadow-header`, `-Wsource-uses-openacc`, `-Wsource-uses-openmp` (1), `-Wspir-compat`, `-Wstrict-potentially-direct-selector`, `-Wstrict-selector-match`, `-Wsuper-class-method-mismatch`, `-Wthread-safety-analysis`, `-Wthread-safety-attributes`, `-Wthread-safety-beta`, `-Wthread-safety-negative`, `-Wthread-safety-pointer`, `-Wthread-safety-precise`, `-Wthread-safety-reference`, `-Wthread-safety-reference-return`, `-Wthread-safety-verbose`, `-Wunaligned-access` (1), `-Wundeclared-selector`, `-Wundef-prefix`, `-Wunguarded-availability`, `-Wunique-object-duplication`, `-Wunsupported-dll-base-class-template`, `-Wvector-conversion` (14)
