# Plan: incremental compilation for the REPL

Revision (2026-11-05, NOTES.md §68): the native `sc` no longer has a native
engine, a child or a `fork`. The compiler is the session of `sc.wasm` in
the same process, on wasmtime, as in the browser, with the memory copy of
§65 in place of the `fork`. What follows about the child (§1.1, §2, §3.3)
is the history of that decision.

Version 4 (2026-10-01):
- the native REPL uses the minimal wasmtime, with JIT (§1.4);
- exceptions and `setjmp`/`longjmp` come into scope (§1.5);
- the compiler runs in a child process (§1.1).

The teacher's decisions (2026-10-01):
- redefinition is rejected for now (§3.4);
- one binary, `sc`, which runs itself again as the child (§1.1);
- the language uses the newest standards (§1.6).

Three adversarial reviewers reviewed version 2. They covered clang
internals, runtime and build, and tests and sequence. Each change cites
the finding behind it (C = clang, R = runtime, T = tests). The experiments
are in the session's scratchpad (`adv1`–`adv3`) and in `~/.cache/sc-wtsize`,
`sc-wmi` and `sc-wamr`.

## 0. Goal and constraints

- **What the cell sees.** The REPL compiles each cell on top of the
  earlier ones. Types, functions, variables, macros and `#include` of the
  earlier cells hold in the next one, and the student does not repeat
  declarations. The headers are read once per session.
- **Execution.** Each cell becomes a wasm `.so`, which the loader (§18)
  loads on top of the earlier ones. The student's code is always wasm.
- **Two targets.** The same engine and the same cuts serve the native
  build (x86-64 Linux) and wasm (`sc.wasm`, web).
- **API.** The engine's API is C, in `engine/sc.h`. The Rust uses what
  bindgen generates from that header, with nothing kept by hand (see §1.1
  and §2).
- **Language.** C++ **with exceptions**, and C with `setjmp`/`longjmp`
  (§1.5).
- **Rules of AGENTS.md:**
  - `make test` with 0 differences;
  - `make checkinv` SAME;
  - cuts before patches;
  - NOTES.md kept up to date.

## 1. Decisions

### 1.1 Native: the session in a child process (R1–R4, R16, T-M5)

The student's code runs on wasmtime (§1.4). Wasmtime handles traps through
signals, and compiling inside the REPL's process is not safe:

- **The signal handlers fight.** The `CrashRecoveryContext` installs
  handlers without chaining them. A trap from the student's code (SIGILL,
  SIGFPE or SIGSEGV) arrives at it with no active context. It then turns
  recovery off, and from then on `Process::Exit` kills the REPL.
- **The context is global.** With `LLVM_ENABLE_THREADS=OFF`, the "current"
  context of the CRC is a global of the process.
- **`llvm_unreachable` is undefined behaviour** in our build.
- **SIGSEGV in `malloc`.** It leaves the arena lock held, in a process with
  threads.
- **Leak.** An abandoned session leaks more than 100 MB.
- **More handlers.** LLVM installs its own handlers (`RemoveFileOnSignal`).

None of this holds for the clang inside `sc.wasm`. It installs no handlers,
and a crash in it is a trap that stays in the instance. For this reason,
since NOTES.md §68, `sc` runs the session of `sc.wasm` in its own process
(`src/engine/guest.rs`), and the decision below was revised.

**Decision.** The session lives in a long-running child process:

- **One binary, called `sc`.** It joins the REPL, the loader and the
  engine:
  - `sc` is the REPL;
  - `sc --engine` is the child, which `sc` itself runs again;
  - `sc -cc1 ...` and `sc ld ...` are today's one-shot engine, which the
    corpus uses to test the shipped binary.
- **The child.** It talks over pipes with messages prefixed by their size.
  In this mode wasmtime does not even start. **Decided in phase 2: the
  loop is in Rust**, on top of the safe API (`sc_proto::serve`), and so is
  the parent's side (`sc_proto::Child` and `Repl`, with the log and the
  replay of §3.6). This way nobody writes the loop twice. The
  `sc --engine` of phase 6b uses the same code as today's
  `build/sc-engine-full`.
- **Names in the build:**
  - the crate of `loader/` now generates `sc`, which is no longer called
    `scload`;
  - today's C++-only executable becomes `build/sc-engine` (cut) and
    `build/sc-engine-full` (with no cuts). It exists only for `make learn`
    and for the tests;
  - `build/sc` becomes the shipped binary;
  - the wasm stays `sc.wasm`.
- Before each cell the child calls `fork` on itself (§3.3). If the cell
  fails or brings the compiler down, the copy takes its place, with no
  replay. A timeout or Ctrl-C kills the child. The REPL launches another
  one and replays (§3.6).
- Launching the child costs 4.5 ms and 11 MB, and killing it really frees
  the memory.
- **Linking the engine into the Rust, without `ld -r`** (R7, which would
  lose the ICF):
  - `link.py --native` gains the `--emit-link` mode, which writes the list
    of the objects and archives already cut, the stubs and `wrap.rsp`;
  - `build.rs` passes the list to the Rust link (`cargo:rustc-link-arg`),
    with `-fuse-ld=lld`, `--gc-sections`, `--icf=safe` and `-lstdc++`;
  - `ld.lld` 19 is in `/usr/bin`.
- **Testing the shipped binary:**
  - `make learn` drives `build/sc-engine`, which has the same objects as
    the engine. The Rust part has no LLVM code;
  - `make test-native` also runs the corpus through `build/sc` (`sc -cc1`,
    `sc ld` and the session through `sc --engine`), against the same
    native baseline.

**Bindgen.** The `sc-proto` crate (`proto/`) runs bindgen on `engine/sc.h` during the build:

- It generates the protocol's types. These are the status enum, the
  message codes and the header of each message.
- The native REPL and the Rust test host of `sc.wasm` use these same types.
- Nobody keeps a constant or a layout by hand.
- The function declarations come from it too. With the `engine` feature,
  the crate links the engine and gives a safe API over them (`Session`,
  `main`; NOTES.md §23). The calls happen only in the child process.

### 1.2 Wasm: `sc.wasm` becomes a reactor that also exports `_start` (R8, R9, T-B2)

- **Reactor and command in the same module.** `-mexec-model=reactor`, plus
  our own `_start` that calls `__main_void`. An experiment confirmed that
  the module runs as a command and as a library (exports).
- **Tests unchanged.** The corpus (`wasmtime run sc.cwasm`), `link.py
  --learn` and the baselines **do not change**.
- **Exports:** the current list is in §5.2, step 3.
- **Browser:** the engine runs in a Web Worker. Recovering from a failure
  means `terminate`, a new worker and a replay.
- **Stack:** the corpus's 16 "call stack exhausted" come from the engine's
  wasm stack limit. In the browser this limit (~1 MB in V8) is fixed, and
  these cases still fail there. The docs record this.

### 1.3 Compiler with or without state: decided by a spike (T-M9)

The test reviewer measured the stateless alternative:

- **`import std`.** A cell with `import std;` plus vector/cout compiles in
  0.12 s and 53 MB natively. §17 gives 0.14 s in V8.
- **Chained PCH** (`-fpch-codegen`):
  - cell 1, with headers and definitions, takes 2.3 s and produces a 17 MB
    PCH;
  - each later cell takes 0.04 s of PCH plus 0.09 s of object;
  - defect found. Cell 2 emitted the dynamic initializer of cell 1 again.

Without state there is no live session, no `CleanUpPTU`, no replay and no
crash recovery. Each cell is a short process or instance. For this reason
phase 0 (§5) implements both candidates in the engine with no cuts and
decides with data:

- **A. Session** (`IncrementalParser`, §3): the main path of this plan.
- **B. Stateless:** each cell compiles with a prefix that comes from the
  earlier cells, through chained PCH. The restrictions of §3.4 hold for
  both.

**Decided in phase 0 (2026-10-13): A.** See NOTES.md §22.

### 1.4 Native runtime for the student's code: minimal wasmtime with JIT

Measured on 2026-10-01: x86-64, LTO, `opt-level=z`, strip, with a C loop of
3·10⁸ iterations compiled at `-O0`.

| runtime | exceptions | size | time |
| --- | :---: | ---: | ---: |
| **wasmtime 48, Cranelift x86, minimal features + exceptions + WASI p1** | yes | **5.7 MB** | **0.81 s** |
| wasmtime 48, Pulley only (interpreter) + exceptions + WASI p1 | yes | 4.9 MB | 18 s (29 s with `-Oz`) |
| wasmi 2.0 + WASI | **no** | 2.6 MB | 4.1 s |
| WAMR, fast interpreter | **no** | 0.30 MB | 7.5 s |
| WAMR, classic interpreter + exceptions (legacy) | legacy only | 0.29 MB | 15 s |

**Why the others stay out:**

- **Pulley.** The cells are generated at run time, so Cranelift stays in
  the binary. Without the x86 backend, which needs a patch in Cargo, the
  binary shrinks by only 0.8 MB and becomes 20 times slower.
- **wasmi.** It rejects the exceptions proposal (`module/import.rs:86`).
- **WAMR.** It does not share across instances a memory, a table and a
  `__stack_pointer` that the host creates:
  - the wasm-c-api ignores an imported memory and table;
  - globals are copied by value;
  - `call_indirect` always calls its own instance;
  - exceptions exist only in the classic interpreter, in the legacy form,
    and do not cross modules;
  - closing these gaps costs 1.5 to 2.5 months of a fork.

**Configuration of the loader:**

- **Features:** `default-features = false` and `["runtime", "std",
  "cranelift", "gc", "gc-drc"]`. `Config::wasm_exceptions` requires `gc`.
- **WASI:** `wasmtime-wasi` with `p1` only.
- **Interruption:** `epoch_interruption`, to stop an infinite loop of the
  student with no cost per instruction.
- The `scload` prototype was 26.7 MB, with wasmtime's default features
  (27.5 MB at the start of phase 6a). §5.3 has the final set. The final `sc` ≈ 24.5 MB of engine + 5.7 MB of runtime. Measured in phase 6a, `scload` with the final profile is 8.1 MB, not 5.7 MB (NOTES.md §49).
- `sc` adds `sc-proto`, `bpaf`, `rustyline`, `dirs` and `libc` (0.9 MB,
  NOTES.md).

### 1.5 Exceptions and `setjmp`/`longjmp`

Today C++ is `-fno-exceptions`. The libc++ of wasi-sdk 29 comes without
exceptions, and the cuts removed `CGException` and the EH of
`ItaniumCXXABI` (NOTES.md §12). WASI's `setjmp.h` gives `#error` without
the exceptions proposal, and `std.cppm` drops `<csetjmp>` (§17). Both
features use the same mechanism, so they come in together.

- **Form:** the standard one (exnref, `try_table`). Current browsers and
  wasmtime 48 support it.
  - The pin's LLVM still generates the legacy form by default
    (`wasm-use-legacy-eh`, `cl::init(true)`, `WebAssemblyTargetMachine.cpp:80`).
  - The engine turns on the standard form at start-up. sc does not read
    `-mllvm`, so the engine puts the value directly into the registered
    `cl::opt`.
  - Confirmed in phase E. Wasmtime 48 refuses the legacy form
    ("legacy_exceptions feature required for try instruction"), and the
    standard form runs on it and on node 24 (V8). The value goes in
    `FrontendOptions::LLVMArgs`, which `checkinv` compares, and
    `scSetLLVMArgs` puts it into the `cl::opt` without the parser.
- **C/C++ runtime:**
  - done: `make sysroot` builds `build/sysroot` (NOTES.md §20). libunwind
    goes inside libc++abi (the two call each other in wasm), and
    `SILENT_TERMINATE` stays off, so the student sees the type and the
    `what()`;
  - libc++, libc++abi and libunwind are built from `build/src` (the pin)
    with `-fwasm-exceptions -mllvm
    -wasm-use-legacy-eh=false`, for `wasm32-wasip1`, as `.a` and `.so`;
  - the libc of wasi-sdk 29 stays, including `libsetjmp`, which the
    sysroot already ships (`libsetjmp.a`, `libsetjmp.so`);
  - `std.pcm` is generated again, now with `<csetjmp>`;
  - a new target in the Makefile, `build/runtimes`, with the recipe
    recorded in NOTES.md.
- **`sc_inv.cpp`:**
  - C++: `-fcxx-exceptions -fexceptions -exception-model=wasm`;
  - C: the equivalent of `-mllvm -wasm-enable-sjlj`, put directly into the
    `cl::opt`;
  - both with `+exception-handling` in the target features;
  - `checkinv` gains the combinations with exceptions and checks them
    against the driver with `-fwasm-exceptions`.
- **Linking and the loader:**
  - the `.so` of each cell imports the `__cpp_exception` tag, and
    `__c_longjmp` for `setjmp`;
  - the loader creates each tag once and passes it to every module, as it
    does with the memory and the table. In practice, `libc++abi.so`
    defines and exports `__cpp_exception`, and `libsetjmp.so` imports
    `__c_longjmp`, which the loader creates;
  - **uncaught exception:** wasm has no search phase, so libc++abi never
    calls `std::terminate`, and the exception goes out to the host
    ("thrown Wasm exception"). The host does what libc++abi does natively.
    It reads the exception's pointer (field 0 of the exnref with the
    `__cpp_exception` tag), calls `__cxa_begin_catch` and
    `std::terminate`, which prints "terminating due to uncaught exception
    of type …: …" and aborts. Done in the loader. The JS host does the
    same with `WebAssembly.Exception.getArg`;
  - an exception thrown in libc++abi and caught in a cell crosses the
    modules through the shared tag.
- **Corpus:**
  - student programs with exceptions: `throw`/`catch` by value and by
    reference, a hierarchy, `std::exception` from the library
    (`std::vector::at`, `std::stoi`), RAII during unwinding, `noexcept`
    with `std::terminate`, an exception that crosses REPL cells;
  - in C, `setjmp`/`longjmp`, including across cells;
  - the clang tests that today give "exceptions disabled" now compile. The
    baseline changes, and each change goes to `IGNORE` with a reason, or
    to the new baseline, as an intended change;
  - afterwards, `make learn` brings back the EH code. We measure the size,
    and we revise the budget of §5.1 only for this phase, because
    exceptions are a requirement.

### 1.6 Newer standards

- **The standard:**
  - C moves from gnu17 to **gnu23**, the newest published C standard. The
    pin's `c2y` is a draft.
  - C++ moves from gnu++20 to **gnu++26** (`LangStandards.def:188`).
  - The pin's libc++ has only part of C++26, so we measure and record what
    is missing.
- **In the REPL and for the student:** the standard is fixed, with no
  option.
- **In `sc_inv`:**
  - it now accepts `-std=`, only for the corpus;
  - the default standard changes;
  - `checkinv` checks the new combinations against the driver with
    `-std=gnu23` and `-std=gnu++26`.
- **Coverage of the cuts:**
  - C23 rejects K&R definitions and implicit `int`, which are common in
    the gcc torture tests and in c-testsuite. Under the new standard, these
    tests stop in the frontend and no longer cover the backend;
  - for this reason the slices of these tests stay with `-std=gnu17`. The
    student programs, `test/mod` and the clang tests use the new standard;
  - C23 and C++23/26 tests go into `test/samples` and `test/cxx`.
    `nullptr`, `constexpr` and `typeof` in C, and `std::print`,
    `std::expected`, deducing this and `std::ranges::to` in C++;
  - `std.pcm` is generated again with gnu++26.
- **Together with phase E:** the baselines and the cuts change at once
  (§5).

## 2. The API (`engine/sc.h`)

Only scalars and pointers, so that the signature is the same in wasm32 and
x86-64. The engine owns the result, and it is valid until the next call
(R13). There is no `sc_buf` allocated by the host.

```c
enum sc_status {
  SC_OK = 0,    /* done */
  SC_ERROR = 1, /* error in the student's code; the session goes back to before the cell */
  SC_LOST = 2,  /* fatal error, a link failure after codegen, or undo:
                   the session has to be made again and replayed */
};
enum sc_part { SC_DIAGS = 0, SC_SO = 1, SC_SYMS = 2 /* exports of the .so, for the replay */ };
enum sc_cell_mode { SC_CELL_SO = 0, SC_CELL_REPLAY = 1 /* IR only, for the replay */ };

typedef struct sc_session sc_session;
/* session flags (§3.5), -x c|c++, and after "--" the arguments of wasm-ld;
   *out always gets the session, in SC_ERROR too (to read SC_DIAGS) */
int sc_session_new(int argc, const char *const *argv, sc_session **out);
void sc_session_free(sc_session *s);
int sc_session_cell(sc_session *s, const char *src, unsigned len, int mode);
int sc_session_undo(sc_session *s);                             /* SC_LOST (§3.3) */
const unsigned char *sc_result_ptr(sc_session *s, int part);
unsigned sc_result_len(sc_session *s, int part);
int sc_main(int argc, const char **argv);                       /* the engine of today */

/* protocol of the native child: the header of each message on the pipes */
enum sc_msg { SC_MSG_NEW = 1, SC_MSG_CELL = 2, SC_MSG_UNDO = 3, SC_MSG_REPLY = 4 };
struct sc_msg_header { unsigned kind, status, len[3]; };
```

Done in phase 1 (2026-10-13; NOTES.md §23). Compared with the draft,
`sc_session_new` returns the status, `sc_session_free` exists, `codegen`
became `mode`, and the engine receives the link's arguments instead of
working out where the libc and the builtins are.

- **Entry points of the engine:**
  - the `main` of the C++-only executables is `engine/sc_entry.cpp`, which
    calls `sc_main` (`-cc1`/`ld`);
  - `build/sc-engine-full` (phase 2) is Rust (`proto/src/bin/sc-engine.rs`).
    `-cc1`/`ld` go to `sc_main`, `session` is the child and `repl` is the
    test of the parent's side;
  - in wasm, `_start` calls the same `sc_main`.
- **The protocol** (comment in `sc.h`): the header in the host's byte order
  and the three parts. `SC_MSG_NEW` carries the arguments separated by NUL,
  `SC_MSG_CELL` carries the source (the mode goes in `status`), and each
  request gets an `SC_MSG_REPLY` with the parts in the order of
  `enum sc_part`. The child ends at the end of its input and writes the
  protocol to a copy of stdout. Fd 1 becomes stderr, so that nothing from
  the engine breaks a message.
- **Diagnostics:**
  - the session uses a `TextDiagnosticPrinter` over a string;
  - the parent reads what goes straight to fd 2 (`sc_cut_called`, "LLVM
    ERROR", lld on a fatal error) from the child's stderr (native) or from
    the WASI stderr (wasm);
  - the names `input_line_N` and the note "In file included from
    `<<< inputs >>>`" become the name the REPL gives the cell, through a
    mapping in the printer (C-M1).

## 3. The session (candidate A)

**Phase 2 (2026-10-14; NOTES.md §24).** `split-` (`make test-split`)
matches `pic-` in 1,941 of the corpus's 1,947 programs. The other 6 are
known limits (`KNOWN` in `test/split.py`). To get there, the session
gained four fixes (§3.8) and the loader gained forward references. The
open items were closed on 2026-10-17. These were §3.4 (the unused warning
through NOTES.md §29, and redefinition with no note), the incremental
combinations in `checkinv` (§3.5), and a top-level `return`, through a
patch in Sema (§3.7). The `fork` snapshot (§3.3) solved an `#include`
after a cell that failed.

### 3.1 Assembly (C-B3, C-m1, C-m3)

- **No `clangInterpreter` and no `IncrementalAction`.** For `EmitObj`,
  `IncrementalAction` wraps an `EmitLLVMOnlyAction`. That would call
  `emitBackendOutput` with `Backend_EmitNothing`, which `sc_backend.cpp`
  rejects.
- **`engine/sc_session.cpp`:**
  - a minimal `ASTFrontendAction`, with `TU_Incremental`, an empty
    `EndSourceFile` and `CurContext = nullptr` after `ExecuteAction`;
  - the consumer is a plain `CreateLLVMCodeGen`;
  - an empty main file, `<<< inputs >>>`, remapped;
  - after the initial `ParseAST`, the session keeps the first module alive
    (CodeGen holds references to it) and calls `StartModule`.
- **Per cell, the core of `IncrementalParser`, copied** (`Parse`,
  `ParseOrWrapTopLevelDecl`, `CleanUpPTU`):
  - ~310 lines with the action;
  - the source commit in the header;
  - checked at each pin change with `git diff` of the source files. The
    switch to 23.1.3 is pending.
- **After the parse:** `GenModule` (`ReleaseModule` + `StartModule`), the
  post-pass of §3.2 and `emitBackendOutput(..., Backend_EmitObj, ...)`
  directly on the PTU's module. The per-module passes and the data layout
  check work this way.
- **Link:** `lld` reads and writes files (R5).
  - The session has its own temporary directory. It is `/tmp` natively, a
    writable preopen in wasm, and an in-memory FS in the browser's WASI
    shim.
  - The session reads the `.so` back into the result.
  - If `lldMain` returns `canRunAgain=false`, the status is `SC_LOST`.

### 3.2 Internal state duplicated in each cell (C-B1, blocker of version 1)

**The problem.** In incremental mode, CodeGen emits again in every PTU the
declarations with internal linkage. These are `static`, the anonymous
namespace, and a local `static` of a `static` function. Each `.so` gets
its own copy. Confirmed in `clang-repl-17`. A `static int cnt` incremented
in one cell shows up as 0 in the next.

**The fix** is an IR post-pass in the engine, with no patch:

1. The first time an internal global tied to a declaration appears
   (`CodeGenerator::GetDeclForMangledName`), the post-pass promotes it to
   external with default visibility, under a name unique to the session.
2. The session keeps the set of promoted names. In later modules, a copy
   emitted again becomes a declaration (`deleteBody`, no initializer).
3. `__cxx_global_var_init*`, `__stmts__*`, `_GLOBAL__sub_I_*` and `.str*`
   stay internal.

Tests: the cases `cnt`, `nx` (a local `static` of a `static` function) and
`an` (anonymous namespace), in C and C++, in `repl-`.

### 3.3 A cell with an error (C-B2, C-M4)

- **The problem.** CodeGen only stops on an *unrecoverable* error. The
  declarations seen before the error leak into the current module.
- **On failure:**
  1. `Consumer->HandleTranslationUnit` (it sees the error, calls `clear`
     and `M.reset`);
  2. `StartModule`;
  3. only then `CleanUpPTU` and `Diags.Reset`.

  Status `SC_ERROR`.
- **This is not enough** (phase 2). `CleanUpPTU` leaves remains of the
  headers that the failed cell read:
  - the include guards stay defined (the next `#include` reads nothing);
  - `Sema` keeps the cell's `namespace std`;
  - the implicit `operator new` and enumerators stay in lookup.

  Plugging one cache after another never ends. **Decided (2026-10-14): a
  `fork` snapshot.** In `serve` (`proto/src/engine.rs`), before each
  `SC_CELL_SO` cell, the child calls `fork`. The copy waits on a pipe. If
  the session accepts the cell, the copy ends. If the cell fails, the
  original passes the reply to the copy and ends, and the copy goes on
  with the session from before the cell. If the original dies in the cell
  (crash), the copy replies `SC_ERROR` with "the engine crashed" and goes
  on. The log of `Repl` keeps only the accepted cells. The child's pid
  changes at each error. In the browser the equivalent is to copy the
  wasm memory before the cell (phase 6b).
- **Undo after codegen** (the link or the load failed, or the student
  asked for `undo`): `CleanUpPTU` restores only lookup. The CodeGen state
  and the specializations do not come back. So `undo` of a cell that was
  already generated is `SC_LOST` plus a replay without it. The cheap
  `undo` exists only for the cell that failed.

### 3.4 Semantics of incremental mode (C-m2)

**What changes compared with a normal TU:**

- The end of TU runs per cell, and the tentative definitions are completed
  per cell. In C, `int x;` followed by `int x = 5;` becomes two strong
  definitions, and the loader hides the first.
- `UnusedFileScopedDecls` is not cleared. An unused `static` gets a
  warning again in every cell.
- A final expression without `;` is accepted silently, as a discarded
  value.

**Decisions for v1:**

- ~~The unused warning only comes out in the cell that defines the
  declaration. The printer filters this by the source PTU.~~ Changed in
  the prototype (NOTES.md §29). The session empties
  `UnusedFileScopedDecls` and `UndefinedButUsed` at each cell, because a
  later cell can use or define the name. In the REPL the end of a cell
  gives no warning. `sc FILE` and `sc check` use `-sc-one-cell` and warn
  like the compiler on a whole file.
- ~~`warn_unused_expr` stays ignored. An expression without `;` gives
  "missing `;`", and printing the value stays out of v1.~~ Changed in the
  prototype (NOTES.md §26). The expression without `;` shows the value,
  if it is a scalar, and is no longer a discarded value
  (`llvm/incremental-stmts.patch`), so `warn_unused_expr` came back.
- Showing a struct, union, array and enum by name stays for later. The
  engine passes the type's description (fields, offsets, sizes,
  enumerators) and the address of a temporary, and `sc` reads the wasm
  memory, without generating code in the cell. The C++ library's types
  (`std::string`, `vector`) stay out, because libc++ owns their layout.

**Redefinition (C-M3, T-M7).**

- Clang rejects the redefinition, and upstream does not support it
  (clang-repl uses `%undo`).
- Removing it from lookup is not enough. CodeGen's `DeferredDecls` is
  indexed by the mangled name and would emit the old body.
- **Decided: v1 rejects it.** ~~Clang's message gains a note from sc that
  suggests `undo` of the earlier cell.~~ Decided on 2026-10-17: no note.
  Clang's error already points to the earlier definition ("previous
  definition is here", with the `cell_N`), and `:undo` only undoes the
  last cell, so the note would help in one case only. `:undo` is in
  `:help`.
- Accepting the redefinition stays for later. It would be a patch that
  erases the name from `DeferredDecls` and `EmittedDeferredDecls` and the
  declaration from lookup.

### 3.5 Session flags (C-m1, T-m3)

- `scParseArgs` today requires a file and works out C/C++ from the
  extension. The session gains an explicit `-x c`/`-x c++` and needs no
  file.
- The session forces:
  - PIC;
  - `-fvisibility=default`;
  - `-fincremental-extensions`;
  - `DisableFree=false`;
  - `ClearASTBeforeBackend=false`.
- `checkinv` gains the incremental combinations: c, c++, pic and c++ pic
  mod. `TU_Incremental` is not a cc1 flag, so it stays out of `checkinv`,
  and the session tests cover it. Done on 2026-10-17 (NOTES.md §44).
  `checkinv` runs the arguments of the 12 sessions (`build/repl/*.args`),
  and `scIncremental` holds what the session forces.

### 3.6 Replay after `SC_LOST` (C-B4, R6, T-M4)

**It does not work without codegen.** `moveLazyEmissionStates` carries
from one module to the next:
- `DeferredDecls`;
- `MangleCtx`;
- `Manglings`;
- the deferred vtables.

Without this state, `_LIBCPP_HIDE_FROM_ABI` functions used later become
only declarations, and the old `.so` does not export them.

**The replay:**

1. The host keeps the log of the accepted cells, minus the undone ones. A
   cell that failed leaves nothing in the child (§3.3).
2. A new session. The host applies the log again with full codegen and
   with the post-pass of §3.2 (which rebuilds the promoted names), but
   without the backend and without the link (`codegen=1`, with the object
   thrown away).
3. For each accepted cell, the set of exported symbols (`SC_SYMS`) has to
   match the one recorded in the original compilation. If it does not
   match, the REPL asks to restart the session.
4. The copied `CrashRecoveryContextCleanupRegistrar<Sema>` is removed. The
   child has no in-process recovery.

### 3.7 Bugs of clang-repl and the 23.1.3 pin

Decided on 2026-09-30: the pin moves to `llvmorg-23.1.3` (expected around
~2026-10-06) **before** the REPL work, in a commit of its own. The release
brings three fixes that the session needs:

| bug | upstream | in the plan |
| --- | --- | --- |
| vtable duplicated across PTUs | #185648, in 23.1.3 | comes with the pin |
| `Undo` does not turn the PTU off | #213235, in 23.1.3 | comes with the pin |
| an input that fails erases earlier declarations | #218149, in release/23.x after 23.1.2 | comes with the pin |
| `static` emitted again | open | §3.2 (IR post-pass, no patch) |
| an input that fails leaks IR | #226252, open | §3.3 (`fork` snapshot) |
| `#include` after an input that failed or after `undo` | open | solved: `fork` snapshot (§3.3); `undo` was already a replay |
| a top-level `return` brings the compiler down | open | solved: `llvm/repl-outside-function.patch` gives the error in Sema (NOTES.md §43) |
| a local `static` and a lambda in a top-level statement bring the compiler down (C++) | open | solved: `llvm/repl-outside-function.patch` (NOTES.md §45) |
| an input that ends inside a `{`, `(` or `[`, or in a `_Pragma(`, hangs the compiler | open | solved: `llvm/incremental-input-end.patch` (NOTES.md §46) |
| each input has its own anonymous namespace | open | solved: `llvm/incremental-anon-namespace.patch` (NOTES.md §46) |
| `B::C::C() {}` with `C` private to `B` gives an access error | open | solved: `llvm/incremental-access.patch` (NOTES.md §46) |

The copy of `IncrementalParser` (§3.1) comes from 23.1.3, not from the
current pin.

### 3.8 What `split-` found (phase 2)

- **C: the block scope leaked** (an upstream bug). In incremental mode,
  `Sema::ActOnPopScope` removes no name from `IdResolver` in C, from any
  scope. The intent is to keep the top-level names across PTUs, but
  parameters, locals and labels stayed too. `int f(int f)` gave
  "redefinition", and a label or a name repeated in different blocks gave
  an error. `llvm/incremental-c-scopes.patch` limits this to the TU's
  scope, the only one without a parent.
- **A function without a prototype** (C before C23). The call declares the
  function as variadic. In a whole TU the definition replaces that
  declaration. In the next cell it does not. The post-pass gives the
  declaration the LLVM type of the definition from an earlier cell.
- **A `static` used before its definition.** The declaration uses the
  promoted name (`f.sc`), and the cell that defines the function emits it
  even with no use in that cell, as the use in a whole TU would.
- **`inline` with `always_inline`** (`available_externally`). The backend
  drops this definition, so the cell does not export it. After each cell
  it goes back to CodeGen's deferred list, so that CodeGen emits it again
  wherever another cell uses it.
- **The backend's errors** (`__builtin_return_address`) go to the
  session's diagnostics, as in the one-shot, and the cell gives `SC_LOST`.
- **In the loader**, a cell can use what a later cell defines, like a
  declaration before the definition in a file:
  - the `GOT.mem` entry is filled when the symbol arrives;
  - a function that has no definition yet gets a table entry right away,
    and the definition fills it (the same address for every module);
  - a direct call goes through a wasm stub with `call_indirect` to this
    entry. The host trampolines used up the native stack in a recursion
    across cells.

### 3.9 What the slices of phase 3 found

The slices `ses-`, `seq-`, `crash-` and `mem-` are in `test/session.py`
(`make test-session`), not in `corpus.py`. They run on `sc-engine repl`,
which exists only natively, and `corpus.py` runs any one-shot engine. In
phase 4 the same script runs on the cut engine, against the
`summary.json` of the uncut one. `ses-` found three clang-repl bugs (the
table of §3.7). The `KNOWN` list of the script holds what cannot match
the whole compile, with the reason (NOTES.md §46).

## 4. Memory and global state

- **Native:** the session's memory belongs to the child. A valve (RSS
  above a limit, for example 1 GB) leads to killing the child and
  replaying.
- **Wasm:** the same valve on `memory.size`. Recovering means discarding
  the `Store` or the Worker.
- **One-shot (`sc_main`):** it keeps `-disable-free` and `BuryPointer`.
  For this reason the hosts guarantee **one instance per one-shot
  compilation** (T-M6). The hosts never reuse the reactor for separate
  compilations.
- **Global state** (R11, checked):
  - `LLVMInitialize*` is idempotent;
  - `lld` calls `Ctx::reset` on every link;
  - the assert of `install_fatal_error_handler` does not exist in the build.
  - Inside a session, what matters is lld after a fatal error
    (`canRunAgain`), which the session already treats as `SC_LOST`.
- **Measurable criteria** (T-m4):
  - `mem-`: 1,000 small cells. After warm-up, the slope of RSS has to stay
    below 50 KB/cell and the slope of `memory.size` below 64 KB/cell. The
    limits get revised with the first data. Measured in phase 3 (native):
    2.6 KB/cell in C and 5.2 KB/cell in C++. `memory.size` waits for the
    reactor (phase 5).
  - Memory of `lld` per link: RSS before and after 100 links in a row,
    with a difference below 5 MB.

## 5. Phases

**General rule (T-B1).** Phases 0 to 3 develop on the engines **without
cuts** (`sc-host-full`, `sc-full.wasm`). The cut engine comes in only in
phase 4, after the learning step.

**Names.** The plan uses the names from the time of each phase. Today they
are:
- `sc-host-full` and `sc-host-cut` are `build/sc-full` and `build/sc`,
  with `sc --cc1` and `sc --ld`;
- `sc-engine-full` and `sc-engine` are `build/sc-full` and `build/sc`,
  with `sc --engine` and `sc --replay`;
- `sc-proto` (`proto/`, `sc_proto::`) and `loader/` are the `sc` crate,
  at the root (`src/engine/`, `sc::engine::`; `src/loader.rs`; `src/bin/`);
- `sc-engine-wasm*` is `build/sc-wasm*`;
- `build/repl` is the `repl/` of the pack that `sc` embeds (§5.4), written
  to `build/pack/repl`;
- `test/spike.py` is `test/cells.py`, since the spike left.

| phase | deliverable | exit criterion |
| --- | --- | --- |
| P. pin | `llvmorg-23.1.3`. Rebase of `llvm/*.patch`, rebuild, `make learn`/`test`/`test-warn`/`checkinv`, size in NOTES.md | The usual criteria. Phases 0 onward start only after this one |
| E+N. exceptions and standards | §1.5 and §1.6 complete, in today's one-shot flow (before the session, because they change the cuts and the bases). It can start before P, but `learn` runs again after P | `make test`, `test-warn`, `test-native` and `checkinv` with 0 differences against the new bases. Programs with exceptions and `setjmp` run through `scload` (wasmtime) and on V8. Size recorded in NOTES.md |
| 0. spike | Minimal candidates A and B (§1.3) on `sc-host-full`. Programs from `test/cxx` and `test/samples` split automatically into one cell per top-level declaration, linked and run by `scload`. A fatal cell, then a replay, then the next cell | A. Output equal to that of `pic-`. The cell's symbols after the replay equal those without a crash. B. Equal output, without emitting the initializers again. Choice recorded in NOTES.md (time per cell, complexity, memory) |
| 1. API | `sc.h`, `sc_entry.cpp`, `sc_main`. `sc_api.cpp` (the API's session on spike A) and the `sc-proto` crate with the safe Rust API (`make test-proto`) | `make test`, `test-native`, `test-warn` and `checkinv` with no difference. `test-proto` green |
| 2. session | §3 complete in `sc-host-full`. `build/sc-engine-full session` (the protocol loop, in Rust). A test mode `build/sc-engine-full repl <dir> <cells...> -- <args>` that writes the `.so` files and does the replay | `split-` (`make test-split`) equal to `pic-` on the engine without cuts, apart from the `KNOWN` |
| 3. tests | The slices `ses-`, `split-`, `seq-`, `crash-` and `mem-` in `corpus.py` (§6) | All green on the engine without cuts |
| 4. cuts | `make learn` and `learn-native` driving `build/sc-engine repl` with the new slices. Their `test-warn` | `make test` with 0 differences with the new slices. Learn time measured. Size within the budget (§5.1) |
| 5. reactor (done, §5.2) | `sc.wasm` as reactor + `_start`. The exports | Before the switch, freeze `res/base/summary.json`. After it, the new `sc-full.wasm` reproduces the base hash by hash (T-B2) |
| 6a. minimal loader (done, §5.3) | The loader with the minimal features of §1.4, exceptions on and shared tags, `epoch_interruption` | `pic-` and `repl-` with the same output. Loader size recorded |
| 6b. hosts (native done, NOTES.md §50; JS proposed in §5.5) | The `sc-proto` crate (bindgen on `sc.h`), the single binary `build/sc` (engine linked through `--emit-link`), the child `sc --engine`, timeout, valve, replay. JS host documented | `crash-` through the native REPL. The REPL stays alive and the next cell works |
| 7. docs and distribution | NOTES.md (§20), README, AGENTS (new targets), Makefile (`.PHONY`, `clean` of `res/ses*` etc.). `sc` outside the checkout (NOTES.md §51). The `.args` files without absolute paths, relative to a root, and the sysroot embedded, without extraction (§5.4) | `sc` copied out of `build/` runs (§5.4) |

### 5.1 Size budget (T-m1)

- At most **+1% in zstd** over the 5,452,450 of `sc.wasm`.
- Above that, the plan switches to building two engines, one-shot and
  session, from the same objects.
- The 5,452,450 was `sc.wasm` before phase E+N. The base for phase 5 is
  the `sc.wasm` of phase 4, with 5,641,258 in zstd (NOTES.md §47). By the
  estimate in §47, the session in the wasm costs +0.47% over the `sc.wasm`
  from before phase 4.

### 5.2 Phase 5 in detail: the reactor

**Deliverable.** `sc.wasm` (and `sc-full.wasm`) becomes a single module
that:
- runs as a command (`wasmtime run`), exactly as today;
- exports the session of `sc.h` to a host.

A native test host, `build/sc-engine-wasm`, drives these exports with the
same slices as phase 3.

**Outside phase 5:**
- the browser host (Worker, JS);
- the memory valve;
- the REPL using `sc.wasm`;
- the memory snapshot for the browser (step 6 only measures it).

All of that belongs to phase 6.

**What does not change:**
- the native one-shot `build/sc` and `build/sc-engine`, and their objects;
- `corpus.py`, with the same arguments;
- `link.py --learn`, which still runs the one-shot corpus.

#### Step 1. The reactor with `_start` (already checked)

The review of the plan checked the following with wasi-sdk 29, the pin's
`wasm-ld` and wasmtime 48:
- a module linked with `-mexec-model=reactor` and with our `_start`, which
  does `exit(__main_void())`, runs as a command. The output and the exit
  codes (0 and 3) are the same as before;
- `wasmtime run` calls `_initialize` before `_start`, also with
  `--invoke` and with a `.cwasm`, and the global constructor runs once;
- the `_initialize` of `crt1-reactor.o` has a guard. A second call hits an
  `unreachable`. Every host calls `_initialize` once per instance.

**`node:wasi` refuses** a module that exports both `_initialize` and
`_start`. `wasi.start` requires that `_initialize` does not exist, and
`wasi.initialize` requires the opposite. This breaks `bench/cc.mjs`. The
workaround, checked, is to pass an object in place of the instance:
- command: `wasi.start({exports: {memory, _start() { e._initialize(); e._start(); }}})`;
- library: `wasi.initialize({exports: {memory, _initialize: e._initialize}})`.

The README documents the workaround, and `bench/cc.mjs` switches to it.

**Alternative without the reactor, also checked.**
- The module links with `-nostartfiles` and exports an `sc_init`, which
  calls `__wasi_init_tp` and `__wasm_call_ctors` once.
- Our `_start` calls `sc_init` and then does `exit(__main_void())`.
- This way Node's `wasi.start` works without change.

The recommendation is the standard reactor, because it does not
reimplement the crt. The alternative stays on record.

#### Step 2. Freeze the bases

The bases depend on the engines in the Makefile. Relinking an engine
regenerates its base, and the T-B2 criterion is lost. Before any change,
the bases go to `build/res/<name>-pre5/`, and the hashes of the
`summary.json` files go to NOTES.md. There are six bases:
- `base` and `wev-base` (`sc-full.wasm`);
- `native-base` and `native-wev-base` (`sc-host-full`);
- `session-base` and `session-wev-base` (`sc-engine-full`).

The criteria compare against these copies.

**What must not change.** The plan does not change the native objects:
- `_start` goes into a wasm-only file, `engine/sc_start.cpp`, and not
  into `sc_entry.cpp`;
- the note on the temporary directory goes into the README, and not into
  `sc.h`, which every object includes;
- the change in `proto/` (step 4) changes the code of `sc-engine-full`,
  and the session bases are compared with the frozen ones to show that its
  behaviour did not change.

#### Step 3. The module

1. **Objects.**
   - The wasm gets its own list in the Makefile:
     `WENGINE := $(ENGINE) sc_session sc_api sc_start`.
   - `WENGINE` applies in the rules `engine`, `sc-full.wasm`, `sc-cut.wasm`
     and `learn`. `ENGINE` stays the native list.
   - The rule `$(B)/engine/%.o` now depends on `engine/sc_session.h`.
   - `link.py` has the list in the code (`ENGINE = [...]`). Without
     `--native`, the list gains the three objects.
2. **Model.**
   - `link.py` repeats clang's link command. With `-mexec-model=reactor`
     in the wasm's `flags`, the model reaches every link. That covers the
     stubs, the trace and the final link.
   - `sc_start.cpp` has the `_start` from step 1.
3. **Exports.** They are linked with `-Wl,--export=` from a single list,
   in `link.py`:
   - `sc_session_new`, `sc_session_free`, `sc_session_cell`,
     `sc_session_undo` and `sc_session_depth`;
   - `sc_result_ptr` and `sc_result_len`;
   - `malloc` and `free`, for the host to put the argv and the source in
     memory;
   - `chdir` (from wasi-libc). The guest starts at `/` whatever the host's
     cwd is (checked), and the session finds the `#include "..."` files
     through the cwd. `session.py` runs each session in the source's
     directory. The host calls `chdir` after `_initialize`;
   - `memory`, which is already exported.

   `sc_main` is not exported. The one-shot enters through `_start`. The
   list replaces the one in §1.2, which predates `sc.h`.

   The review confirmed that the wrap and `noop:` do not reach the
   exports. The candidates come only from LLVM's `.obj` members. The
   exports become roots of `--gc-sections`, and their names go through
   `wasm-opt`.
4. **Temporary directory.**
   - `sc_session_new` creates the cells' directory through LLVM's
     `system_temp_directory`. On WASI, that directory is `TMPDIR` (or
     `TMP`, `TEMP`, `TEMPDIR`), or else wasi-libc's `P_tmpdir`, which is
     `/tmp`.
   - The host gives the guest a writable `/tmp`. That is `--dir` in
     wasmtime, and an in-memory file system in the browser. This goes into
     the README.

**Criterion for step 3:**
- The new `sc-full.wasm` reproduces `base-pre5` and `wev-base-pre5` hash
  by hash.
- `make test` and `test-warn` give 0 differences on the cut `sc.wasm`.
- `checkinv` prints SAME.
- The native objects do not change. `make -n` of `sc-host-full` relinks
  nothing.

#### Step 4. The test host: `build/sc-engine-wasm`

The phase 3 slices are the oracle. To run them without changing
`session.py` or `split.py`, `sc-engine-wasm` is `sc-engine` compiled with
another feature. It does the same as the native one:
- `session`, the protocol;
- `repl`;
- the one-shot (`-cc1 ...`, `ld ...`), which `std_pcm()` in
  `test/spike.py` uses for the `.pcm` of `import std;`.

1. **Where it lives.**
   - `proto/` gains the `wasm` feature. It swaps only the raw layer. The
     seven `sys::sc_*` calls behind `Session` and `Reply` become calls to
     the exports through wasmtime. No trait is needed.
   - `serve`, `Copy`, `Repl` and `sc-engine.rs` stay as they are.
     `Reply::part` borrows `memory.data(&store)` as it borrows the native
     pointer today.
   - With `wasm`, the `sc_main` of `sc-engine.rs` becomes a new instance
     that runs `_start`. That is one instance per one-shot compilation
     (§4).
   - `wasmtime` and `wasmtime-wasi` come in at the exact versions of
     `loader/` (48.0.3). `proto/Cargo.lock` and `loader/Cargo.lock` are
     regenerated, because `loader` depends on `sc-proto` and every build
     uses `--locked`.
2. **Instance.**
   - One `Store` per session (`SC_MSG_NEW`). It runs `_initialize`, then
     `chdir` with the process's cwd, and then `sc_session_new`. Before
     dropping the `Store`, the host calls `sc_session_free`, which deletes
     the temporary directory.
   - The argv and the source go into memory through the exported `malloc`.
     The host reads the results through `sc_result_ptr` and
     `sc_result_len`.
   - The CLI's `.cwasm` does not work, because the CLI is version 48.0.0
     and the crate is 48.0.3. Wasmtime refuses a module from another
     version. The host compiles the module with its own `Engine` and keeps
     the result in a cache, as `loader/src/lib.rs` does.
   - `max_wasm_stack` stays at the default (512 KiB, the same as the
     CLI's), which is where the "call stack exhausted" errors come from.
3. **Directories and output.**
   - A `--dir` for the repository root, at the same path in the guest.
     `build/` is inside it, and `session.py` passes absolute host paths
     (sysroot, `-I`, `-L`, the `.pcm`).
   - A temporary host directory mapped as `/tmp`.
   - The guest's stdout goes to the host's stderr, like today's
     `dup2(2, 1)`, so that it does not break the protocol messages.
   - The stdin is not inherited. Wasmtime-wasi would read it in a thread.
4. **Fork.** `serve` forks once per cell, and the fork is safe only with
   one thread. Without care, wasmtime has 57 threads after it compiles the
   module (rayon) and 114 after the first WASI calls (the tokio of
   synchronous p1). The checked recipe stays at 1 thread even with files:
   - `Config::parallel_compilation(false)`, or a deserialized module;
   - a tokio runtime of its own, `new_current_thread().enable_all()`, with
     `enter()`;
   - `WasiCtxBuilder::allow_blocking_current_thread(true)` called
     **before** `preopened_dir`, which captures the value.

   Before each fork, an assert checks that `/proc/self/task` has one
   entry. With this recipe, a trap in the process after the fork came
   back as `Err`, and the parent's `Store` stayed usable.
5. **Failures.** In `serve`, the original runs the cell and the copy
   waits.
   - A trap (fatal error, exhausted stack) or `proc_exit` (wasmtime's
     `I32Exit`) comes back as an `Err` from the call.
   - On an `Err`, the host does `_exit` in the original, without reading
     a result. The copy then answers "the engine crashed", as with the
     native SIGSEGV.
   - In the modes without a fork (`REPLAY`, `NEW`, `UNDO`, `DEPTH`), the
     same `Err` ends the process, like a native crash.
6. **`mem-`.** `session.py` reads the RSS in `/proc`. With `wasm`, the
   host writes a line with `memory.size` to stderr after each cell, and
   `session.py` reads it.

**New targets in the Makefile:**
- `$(B)/sc-engine-wasm` (on `sc.wasm`) and
  `$(B)/sc-engine-wasm-full` (on `sc-full.wasm`);
- `test-session-wasm` (ses-, seq-, crash-, mem-), against
  `session-base`;
- `test-split-wasm`;
- `test-session-wasm-warn`, against `session-wev-base`.

**Criterion for step 4:**
- `test-session-wasm` and `test-split-wasm` with 0 bad, on `sc-full.wasm`
  and on `sc.wasm`.
- Native `test-session` and `test-split` with 0 bad against the frozen
  bases.
- The known exceptions are listed with the reason, like the `KNOWN`:
  - the stack overflows, because the wasm stack limit is not the native
    one (§1.2);
  - the differences in the trap's text.

#### Step 5. Wasm cuts with the session

A bad result on `sc.wasm` that does not show on `sc-full.wasm` is a cut
function. In the wasm, the cut function is an `unreachable`, without the
native `sc_cut_called`. The name shows only in the trap's backtrace, and
`sc.wasm` has no name section (`wasm-opt --strip-debug` removes it).
So:
- the slices also run on `sc-cut.wasm`, which has the names, in an
  `sc-engine-wasm-cut`;
- the host writes the trap with the backtrace to stderr;
- `session.py` and `learn_session.py` read frame 0 `__wrap_X`, besides
  the line `sc: cut function called:`.

The cases show up where the wasm code differs from the native code
(`#ifdef __wasi__` in LLVM, the libc).
- First, run and count.
- If there are cases, `learn_session.py` gains a `--wasm`. It links with
  `link.py` without `--native`, uses `sc-engine-wasm-cut` and writes to
  the same list.
- If there are none, nothing changes, and that goes on record.

#### Step 6. Measurements

- **Size:** raw and zstd `sc.wasm` against the 5,641,258 of phase 4.
  Budget: +1% (§5.1).
- **`mem-`:** the slope of `memory.size` (limit of 64 KB/cell, §4) and the
  time per cell in the wasm, against the 33 ms in C and 41 ms in C++ of
  the cut native engine.
- **Snapshot**, so that phase 6 can decide whether the browser recovers
  from an `SC_ERROR` by snapshot or by replay.
  - It measures the cost of copying the linear memory after 100 cells, in
    MB and in ms.
  - What a snapshot needs, according to the review:
    - the linear memory;
    - `__stack_pointer`, exported with `--export`, which was checked.
      After a trap, it stays where the trap happened and has to be
      restored;
    - the memory only grows. On restore, the host zeroes the part that
      grew;
    - the files opened in the cell stay open in the `WasiCtx`, a small
      leak.
- **Time** of `make test`, to see whether the larger module weighs on
  `wasmtime compile`.

#### Step 7. Documentation

- NOTES.md §48: the reactor, the frozen bases, the exports, the test host,
  the fork recipe, the measurements and the cuts.
- README:
  - `sc.wasm` as a reactor, and the exports;
  - `_initialize` once per instance;
  - `chdir` and `/tmp`;
  - the `node:wasi` workaround.
- `bench/cc.mjs`: the `node:wasi` workaround.
- AGENTS: the new targets, and the rule that `test-session-wasm` shows no
  differences.
- Makefile: `.PHONY` and `clean`.
- §1.2 now points here.

**Exit criterion for phase 5:**
- the criteria of steps 3 and 4;
- size within the budget.

**Done** (NOTES.md §48). What changed with respect to this plan:
- `sc.wasm` also links with `-lc-printscan-long-double`. The `wasm-ld`
  demangler prints `long double` literals, and wasi-libc's `printf`
  without them aborts;
- the wasm slices are compared with the bases of `sc-engine-wasm-full`
  (`session-wasm-base` and `session-wasm-wev-base`), and `ses-` and
  `crash-` decide by the wasm one-shot, and not by the native
  `session-base`. Where the native engine crashes, the wasm does not
  always crash;
- the host compiles the module with `--precompile`, instead of wasmtime's
  cache, and writes the line `sc: cut function called:` itself;
- the learn step in the wasm is the `learn-session-wasm` target. It found
  one function;
- `loader/Cargo.lock` did not need to change;
- the `ses-` of `dcl.decomp/p3` with `-Weverything` sometimes crashed.
  The cleanup of the cell with an error read past the lookup table, a bug
  from before phase 5. `llvm/incremental-remove-decl.patch` fixed it
  (NOTES.md §48).
### 5.3 Phase 6a in detail: the minimal loader

**Starting point.** Almost all of §1.4 is already in `loader/`, since
phases E+N and the `sc` prototype:
- the tags `__cpp_exception` and `__c_longjmp` are created once and
  shared, and an uncaught exception becomes `std::terminate`
  (`Loader::uncaught`);
- `epoch_interruption` exists and is on only in the interactive REPL
  (`Loader::arm`, `Loader::on_ctrl_c`). `sc FILE` and `scload` run without it.

The minimal wasmtime is missing. Today `loader/Cargo.toml` uses the default
features. `scload` is 27.5 MB and `sc` is 28.8 MB. The phase replaces them
with the minimum the loader uses, measures each feature and records the size.

**What the loader uses of wasmtime** (read in `loader/src`):

| feature | use | |
| --- | --- | --- |
| `runtime`, `std`, `cranelift` | run and compile the cells | stays |
| `gc`, `gc-drc` | the exceptions proposal is on only with `gc` (`config.rs:2551`); `Tag`, `ThrownException`. Without `gc` the loader compiles and fails only at run time. `gc-null` never frees, so a loop that throws would use up the heap | stays |
| `cache` | `Cache::from_file`, for the sysroot's libraries | to decide (step 3) |
| `parallel-compilation` | implicit. rayon compiles the functions of a module in parallel | to decide (step 3) |
| `wat` | the two modules generated as text, the WASI shim (`wasi_shim`) and the `call_indirect` stubs (`stubs`) | goes (step 2) |
| `addr2line`, `demangle` | no. The loader reads the DWARF with its own `gimli` and demangles with `cpp_demangle` (`wasm_backtrace_details` off) | goes |
| `backtrace` | no. It is Rust's `std::backtrace` inside `wasmtime::Error`. `WasmBacktrace` (`keep_stacks`, `blamed`) depends on no feature | goes |
| `async`, `component-model` | only because `wasmtime-wasi` `p1` turns on `p2`, which turns on both | stay, by force |
| `pooling-allocator`, `profiling`, `coredump`, `threads`, `stack-switching`, `debug*`, `gc-copying`, `gc-null`, `wit-parser`, `compile-time-builtins`, `anyhow` | none | go |

`wasmtime-wasi` gets `default-features = false, features = ["p1"]`.
`sc-proto` does not bring the default features back. It uses wasmtime only
in the `wasm` feature, which `repl` does not turn on (checked with `cargo tree -e
features`). A `cargo check` with this set compiles `scload` and `sc`.

**The collector changes.** With the default features, `Collector::Auto` picks
`Copying` (`config.rs:3620`), which the wasmtime documentation calls "not yet
functional". The baselines ran with it. With `gc-drc` alone, `Auto` picks
DRC. This is a change in behaviour. A new test throws and catches an
exception in a long loop, to see that the GC heap does not run out.

**Measured before the phase** (`sc FILE`, load ~54, default features):

| program | warm cache | cold cache |
| --- | ---: | ---: |
| C, `puts` | 0.09 s | 0.41 s |
| C++, `vector::at` with an exception | 2.65 s | 3.72 s |

The cold cache costs the compilation of `libc.so` (1.9 MB) and of `libc++.so`
(0.9 MB) by Cranelift, in parallel.

**Steps** (all done, NOTES.md §49):

1. **Features.** `default-features = false` on `wasmtime` with `runtime`,
   `std`, `cranelift`, `gc`, `gc-drc`, `cache`, `parallel-compilation` and
   `wat`. `loader/Cargo.lock` changes (the Makefile uses `--locked`) and goes
   in the commit. Measure `scload`, its size and the times of the table above.
   Then remove the three undecided ones one at a time and measure again. A
   table in NOTES.md with the cost of each one.
2. **Without `wat`.** The `wasm-encoder`, which wasmtime's compiler already
   brings, now assembles the two generated modules. `wat_sig` goes.
   The saving was 0.86 MB after LTO.
3. **`cache` and `parallel-compilation`.** Decided by the table of step 1.
   The default is to keep both. The cache pays for `libc++.so` once per
   machine, and without rayon the cold cache is serial.
4. **Release profile.** `lto = true`, `codegen-units = 1`,
   `strip = true`, and a measured `opt-level`. The `z` of §1.4 makes the
   binary smaller, but Cranelift gets slower, and that weighs on the cold
   cache and on each cell. Measure `3`, `s` and `z` on the cold cache of C++
   and on `make test-small`. `panic = "abort"` only if nothing depends on
   unwinding. The loader has no `catch_unwind`. Wasmtime resumes the panic of
   a host function, without turning it into an error (checked, NOTES.md §49).
   Done in the `dist` profile (`make dist`). The `release` profile, which the
   batteries use, has no LTO, so that it compiles fast.
5. **`epoch_interruption` and the collector.** A new target, `make
   test-loader` (`cargo test` in the loader, with `wat` as a
   dev-dependency if step 2 removes it): an infinite loop, `arm`, and
   `increment_epoch` called from a signal handler, like the Ctrl-C of
   `sc` (`machine.rs:94-101`). The loop has to stop with `Trap::Interrupt`.
   Also the collector's loop that throws. Also measure the cost of the checks
   in the loop of 3·10⁸ iterations of §1.4, to confirm that `sc FILE` without
   them makes sense. Measured below 1%, so turning them off saves nothing
   visible.
6. **`proto/`.** `sc-engine-wasm` is only a test host and keeps the
   default features. Record this in NOTES.md.

**Exit criterion:**
- `make test`, `test-warn`, `test-native`, `test-split`, `test-split-wasm`,
  `test-session*`, `test-student` and `test-checks` with no difference. All of
  them run through `scload` or `sc` with the new features;
- `make test-loader` green;
- the baselines are not redone. `$(SCLOAD)` is order-only in the baselines'
  rules, which keep the output of the old loader to compare against;
- the size of `scload` and `sc` and the cold and warm cache times in
  NOTES.md (§49), compared with the 5.7 MB of §1.4.

**Risks:**
- a removed feature that is missing only on a rare path. The lack shows up
  at once as a wasmtime error, not as different output, but only on the path
  that uses the feature. The batteries go through exceptions, `setjmp`, traps
  and leaks;
- a lower `opt-level` making Cranelift too slow. Step 4 measures before it
  chooses.

### 5.4 Phase 7 in detail: one binary (proposal)

**Goal.** `sc` is a single file. Copied anywhere, or to another machine
with the same system, it runs with no tree next to it, without extracting
anything to a temporary directory and without a cache on disk. The solution
cannot depend on features that only Linux has. The target includes Windows.

**What `sc` reads at run time today** (measured on 2026-10-21):

| what | who reads it | where it is | size |
| --- | --- | --- | --- |
| the session's `.args` (`<lang>-<level>[-file].args`, one argument per line) | `Machine::new` (`machine.rs`) | `build/repl`, found by `build_dir()` (`SC_BUILD` or the first ancestor of the binary with `repl/`) | 40 KB (68 KB on disk) |
| the headers, from the sysroot and from clang (`-internal-isystem`, `-internal-externc-isystem`) | the session's clang, through the VFS created in `sc_session.cpp:685` | `build/sysroot/include`, `build/wasi-sdk-*/lib/clang/21/include` | 18 MB + 7.8 MB (4.3 MB of them are x86 and ARM intrinsics) |
| the sanitizers' ignorelist (`-fsanitize-ignorelist`, only in `strict`) | clang, through the same VFS (`NoSanitizeList`, `clang/lib/Basic/NoSanitizeList.cpp:24`, with the VFS of `FileManager`) | `build/sysroot/share/sc/ignorelist.txt` | 659 bytes |
| the `.so` files for the link of each cell (`-L` and `-l`) and `libclang_rt.builtins.a` | wasm-ld, through `findFile` (`fs::exists`, `lld/wasm/Driver.cpp:227`), `readFile` (`MemoryBuffer::getFile`, `lld/wasm/InputFiles.cpp:68`) and, for the `.a`, an `fs::exists` of the `.imports` next to it (`Driver.cpp:312`, which returns false) | `build/sysroot/lib/wasm32-wasip1`, `build/wasi-sdk-*/lib/clang/21/lib/...` | 2.9 MB + 0.45 MB |
| the `.so` files loaded in wasmtime (`libc`, `libc++`, `libc++abi`, `libsc_rt`, `libsetjmp` and the ones they ask for) | `Loader::find` and `collect` (`loader.rs:317`, `std::fs::read`). The bytes also serve `dylink` and the DWARF (`debug`) | `build/sysroot/lib/wasm32-wasip1` | the same |
| the code compiled by Cranelift (the `.so` files, and also each cell and the shims) | wasmtime's `Cache::from_file(None)` (`loader.rs:291`), which reads `~/.config/wasmtime/config.toml` if it exists | `~/.cache/wasmtime` | written on first use |

These do not count: a `std.pcm` (the `.args` of `sc` have no `-fmodule-file`,
and `import std;` in `sc` gives "module 'std' not found"; only
`test/session.py` builds its own, of 38 MB), a `-resource-dir`, the `crt`
files (the link is `-shared`), the `.ctype` and `.setjmp` files (make's
stamps) and the source code of the libraries. Outside the build, `sc` reads
the theme (`~/.config/sc/config`) and the history
(`~/.local/share/sc/history`), which stay as they are.

In a tar, the first five rows add up to 26 MB. That is 2.5 MB with `zstd -19`
and 2.3 MB with `xz -9e`. With one zstd frame per file, it is 3.9 MB without
the intrinsics (2.9 MB of headers and 1.0 MB of libraries). The squashfs
that the proposal asks for stays close to the tar (the table at the end of
this section). The dist `sc` is 34 MB today.

`sc` also writes to a temporary directory, but not to extract anything. It
writes the `.o` and the `.so` of each cell (`sc_api.cpp`,
`createUniqueDirectory`). This is not part of this phase, see "Outside the
phase".

**The proposal.** A pack embedded in the binary and read in memory by
three readers: clang's VFS, a hook in wasm-ld and the loader.

1. **The pack.**
   - **It is a squashfs with zstd -19 and 128 KB blocks** (decided on
     2026-10-21). The index holds the directories and the original size of
     each file, which `status` asks for. The small files sit together
     in blocks ("fragments"), and reading a header decompresses only its
     block. `backhand` keeps the blocks it has already opened. A whole
     tar.zst does not work, because it would force decompressing everything
     to read one header. It is a standard format (`unsquashfs` and 7-Zip open
     it), and the smallest of the ones measured, reader included (the table at
     the end of this section). Zip compresses each file alone and has no
     dictionary. With a dictionary of our own inside it ("stored" entries
     with zstd frames), it would be 260 KB larger, and only `sc` would read
     it.
   - The pack holds the headers (without the x86 and ARM intrinsics), the
     ignorelist, the `.so` files, `libclang_rt.builtins.a`, the `.args` and
     the `.cwasm` files (item 6).
   - `build.rs` assembles it, with the `backhand` crate as a build
     dependency (the same `build.rs` that generates the `.cwasm` files, item
     6), from what `sc-deps` already produces. The root of the image has
     `sysroot/` and `res/`, which `sc` sees as `/sc/sysroot` and `/sc/res`.
     `sc` embeds it with
     `include_bytes!`, which works the same on Windows. The pack goes in the
     `sc` binary (`src/bin/sc`), not in the library, so that `scload` does
     not grow.
   - The paths inside the pack sit under a fixed virtual prefix,
     with one root for the sysroot and another for clang's directory:
     `/sc/sysroot/...` and `/sc/res/...`. A single root (`/sc/include`) does
     not work. Both have `limits.h`, `stdint.h`, `float.h` and others, which
     chain through `#include_next`.
2. **The `.args` without the machine's paths.** `scripts/flags.py` already
   writes the paths in a virtual view (`/sysroot`, `/res`, `/m/std.pcm`),
   which `real()` replaces with the build's. `--session` now replaces them
   with `/sc/sysroot` and `/sc/res` in place of `$(CURDIR)/build/...`, and
   `Machine::new` reads the `.args` from the pack, not from disk.
   - `build_dir()` leaves `sc`. It only found the `.args` and the sysroot,
     and served `frames` (item 8). No test passes `SC_BUILD` to `sc`
     (only `test/corpus.py` reads it, for itself).
3. **Clang: an `llvm::vfs::FileSystem` over the pack, with no patch to LLVM.**
   - In `sc_session.cpp:685`, `getRealFileSystem()` becomes an FS that
     answers for `/sc/` from the pack and passes the rest to the real FS,
     for the student's `#include "meu.h"` and for `sc FILE`. The simplest
     form is a subclass of `ProxyFileSystem` over the real FS. An
     `OverlayFileSystem` also works, but its `getRealPath` and `isLocal`
     ask the lower FS first (`VirtualFileSystem.cpp:532`
     and `:539`), which goes to disk looking for `/sc/...`, and its
     `setCurrentWorkingDirectory` requires the pack's FS to accept the
     directory.
   - The pack's FS implements `status`, `openFileForRead` and `dir_begin`
     (the header search asks for `status` of directories). Directories are
     implicit. They are the prefixes of the paths in the index.
   - This code lives in `engine/` (an `sc_files.cpp`), but the
     decompression stays on the Rust side. `status` needs the size
     without decompressing, and the index is the squashfs one, which only Rust
     reads. The C++ receives functions from Rust in an
     `sc_set_files(const struct sc_files *)`. One says whether a path is a
     file or a directory and gives its size, one lists a directory (for
     `dir_begin`) and one returns the bytes.
     The bytes that `read` returns stay valid until the process ends (Rust
     keeps them), because clang and lld keep pointers to them.
   - The C++ does not need zstd. With no reader registered, everything still
     goes to the real FS. This is the case of `sc.wasm`, which does not export
     `sc_set_files`, and `src/engine/wasm.rs` (`sc` with the `wasm` feature)
     implements it as nothing.
4. **Wasm-ld: a small patch, `llvm/lld-files.patch`.** Lld does not use
   the VFS, so the patch puts a global hook with the same read function
   in `readFile` (`InputFiles.cpp:68`), before
   `MemoryBuffer::getFile`, and in `findFile` (`Driver.cpp:227`), before
   `fs::exists`. The `fs::exists` of the `.imports` (`Driver.cpp:312`) can
   stay. It returns false for `/sc/...` and shows up only in `strace`.
   - Lld has no other access to the input files of this link. The
     members of the `.a` come from the buffer (`libclang_rt.builtins.a` is not
     thin), response files come only with `@` (`Driver.cpp:249`, and `sc`
     passes none), and the output (`tryCreateFile`, `Driver.cpp:1366`) is the
     cells' directory.
   - Why a patch and not a cut. Lld has no other input path. The
     alternative would be to pass the files to lld by real paths, which
     forces extraction.
   - The patch has some 10 to 15 lines (the hook, the declaration and the two
     calls), in functions that change little between versions. It also goes
     into `sc.wasm`, with a null hook, so `make test` and the budget of
     §5.1 apply.
5. **The loader.** `libdirs: Vec<PathBuf>` becomes a function that returns,
   by name, the bytes of the library and, if there is one, its `.cwasm`. The
   bytes of the `.so` are still needed with the `.cwasm`. The `dylink.0` and
   the DWARF come from them (`loader.rs:327` and `:338`), not from the
   compiled module. `sc` passes the pack, and `scload` and `tests/` keep the
   directories (`-L`).
6. **The code of the `.so` files, precompiled.**
   - The pack carries a `.cwasm` of each `.so`, and at run time
     `Module::deserialize` loads them without compiling. The cells and the
     shims (`wasi_shim`, `stubs`) are still compiled on the spot by Cranelift
     (`Module::new`), as today. The `Cache` never reused them. The
     `cache` feature leaves wasmtime (`Cargo.toml:16`, used only in
     `loader.rs:291`), and the writes to `~/.cache` go with it.
   - Measured with `wasmtime compile` 48: 4.7 MB (1.35 MB in zstd, one
     frame per file). Compiling is fast here (0.03 to 0.14 s per
     library on 56 cores), but not on a student's machine, and today the
     first use writes to `~/.cache`.
   - **Decided on 2026-10-21: embed, with no cache.** Keeping the `.so`
     alongside costs nothing, because wasm-ld reads it to link each cell.
     The cost is in the `.cwasm` files, 1.34 MB compressed (4.62 MB raw,
     against 2.96 MB for the `.so` files, 894 KB compressed). Filling a cache
     on first use would save those 1.34 MB, but the first cell on each
     machine and for each version would pay for the compilation, and `sc`
     would write to disk again. Wasmtime's `Cache` does not fit this. It keeps
     every module, cells included, and a thread cleans it now and then, which
     the short processes do not wait for. Here `~/.cache/wasmtime` holds
     1007 MB in 138 thousand files, above the default limits (512 MiB, 65,536
     files). A cache of only the five libraries, per version, would be bounded
     by construction, but embedding is simpler.
   - **The libc's DWARF leaves the pack.** It is 1.32 MB of the 1.91 MB of
     `libc.so`, and `frames()` already hides the `wasisdk://` lines. Without
     it (`wasm-tools strip -d '.debug.*'`), the compressed libc drops from
     646 KB to 287 KB. That is 359 KB less, a quarter of what the `.cwasm`
     files cost. The other libraries have almost no DWARF (libc++ 15 KB).
   - **Who compiles.** The `.cwasm` files have to exist before `sc` is
     compiled (`include_bytes!`), so they cannot come from `sc` itself,
     like the `--precompile` of `sc-wasm` (`src/engine/wasm.rs:34`), which
     writes a file next to the binary. The way out is for `build.rs` to
     compile them with wasmtime 48.0.3 as a build dependency, with the same
     features as `Cargo.toml:16`. The cost is wasmtime compiled twice, and at
     `opt-level` 0 if the profile has no `build-override`.
   - **Careful:** `Module::deserialize` checks (`wasmtime`,
     `engine/serialization.rs:204`) the wasmtime version, the architecture and
     the system, the Cranelift flags, the `Tunables` (among them
     `epoch_interruption`) and the wasm features (the module's have to
     be in the `Engine`). Today `epoch_interruption` differs between the
     interactive REPL and `sc FILE` (`Loader::new(..., interactive)`). The
     ways out are:
     - always turn the epoch on in `sc`. §5.3 measured the cost of the
       checks, below 1%. `sc FILE` does not register Ctrl-C, so the epoch
       never advances and the `arm()` of each call changes nothing;
     - or compile two variants, which doubles the 4.7 MB.
     The proposal is to always turn it on.
   - **The CPU.** An explicit `Config::target(...)` turns off the detection
     of the extensions of the compiling machine (`isa_builder.rs:33` of
     `wasmtime-internal-cranelift`). The output is code for the base CPU. At
     load time, a CPU flag that is on in the module has to exist on the
     machine (`engine.rs:529`). A flag that is off always works. In
     `build.rs`, the target is cargo's `TARGET`, which also works for a cross
     build for Windows.
   - `Module::deserialize` is `unsafe` because it trusts the bytes. Here the
     bytes come from the binary itself, generated in the same build, so this
     is acceptable.
7. **The child `sc --engine`** is the same binary, so it already has the
   pack. The `main` of the `--engine` mode calls `sc_set_files` before it
   opens the session. The children of `--replay` are the same `sc --engine`.
8. **The stack lines.** `frames` (`machine.rs`) treats as the student's
   a DWARF file that does not start with the build directory and has no
   `://`. The prefix hides the header code compiled inside the
   cell (inline functions and templates of libc++, which come from
   `build/sysroot/include`). The libraries' frames already hide in
   another way. `libc.so` and the `wasi-emulated` ones have
   `wasisdk://` paths, and libc++, libc++abi and `libsc_rt` have no DWARF
   of their own (measured with `llvm-dwarfdump`). So the criterion cannot
   be the module of the frame. A `std::vector` in the cell is the cell's code.
   - The prefix becomes the fixed `/sc/`, and `build_dir()` goes.
   - Along with it, `/src/`. The compiler-rt inside `libc.so` and `libc++.so`
     has `DW_AT_comp_dir` `/src/build/sysroot/compiler-rt-...`, from the
     wasi-sdk build. Today a trap in a `__divti3` would show up as the
     student's code. The project uses `-ffile-prefix-map` nowhere.

**What changes for the user.**
- A diagnostic note shows `/sc/sysroot/include/...` instead of
  `.../build/sysroot/include/...`. It is shorter and the same on every
  machine.
- The baselines do not change by themselves. `ses-`, `seq-`, `crash-` and
  `split-` pass their own arguments to `sc --replay`, with the
  real paths (`flags.session(...)`, `test/session.py:105`,
  `test/split.py:78`), and the real FS is still underneath. `test-student` and
  `test-checks` use the `.args` of `sc`, but their `expected.txt` has no
  build path.
- `sc` grows by some 4.3 MB, 4.01 MB for the squashfs and 302 KB for the
  reader (`backhand` without the default features, only `zstd`, and
  `zstd-sys`, in C, which the engine's build already knows how to compile;
  with `cache` out of wasmtime, only the pack uses it). Measured on
  2026-10-21 with the whole pack (2,035 files, 23.4 MB raw: headers,
  `.args`, ignorelist, `.so` files with the libc without DWARF, builtins and
  `.cwasm` files). The reader is the growth of a test binary with the `dist`
  profile over one without a reader. All three read the right 23,371,456
  bytes.

  | | pack | reader | total |
  | --- | --- | --- | --- |
  | squashfs, zstd -19, 128 KB blocks (`backhand`) | 4.01 MB | 302 KB | **4.31 MB** |
  | "stored" zip with zstd frames and a dictionary (`zip` + `zstd`) | 4.38 MB | 187 KB | 4.57 MB |
  | the same, with `ruzstd` | 4.38 MB | 178 KB | 4.56 MB |
  | zip with zstd, method 93, without a dictionary | 5.29 MB | ~190 KB | ~5.48 MB |

  Zip loses 403 KB in the headers of the 2,035 entries alone (each name
  twice, uncompressed). The dictionary, trained on the headers, does not
  change the libraries or the `.cwasm` files (1.988 MB against 1.990 MB).
  The headers alone, per file: 3.42 MB in zip deflate, 2.91 MB in zstd,
  1.88 MB with a dictionary, 1.95 MB in squashfs.

**Steps.**
1. `flags.py` with `/sc/sysroot` and `/sc/res`; the squashfs (`build.rs` and
   embedded), with no readers yet. Check the size against the table above.
   **Done on 2026-10-21** (NOTES.md §52). The pack's `.args` live in
   `build/pack/repl`, and the pack without the `.cwasm` files is 2.61 MB.
2. Clang's VFS and `sc_set_files`. Run `test-student` and `test-checks`,
   which are the ones that go through the `.args` of `sc`, with `sc` copied
   outside the checkout. `test-session` and `test-split` switch to the
   `.args` with `/sc/` (decided below) and redo the baselines once.
3. The lld patch; the link of the cells without the disk. `make test` (the
   patch goes into `sc.wasm`).
4. The loader reading from the pack; the `.cwasm` files from `build.rs`; the
   epoch always on; `cache` out. `make test-loader`.
   **Steps 2 and 3 and the loader of 4 were done together on 2026-10-21**
   (NOTES.md §53). Without the lld hook and without the loader on the pack,
   the `.args` with `/sc/` neither link nor load the cells. With them,
   `build_dir()` and `build/repl` went, and `frames` switched to `/sc/`.
   The rest of 4 was done on 2026-10-22 (NOTES.md §54), with 5 and 6.
5. `frames` with `/sc/` and `/src/`, without `build_dir()`. Done with the
   exact prefixes of compiler-rt (`/src/src/llvm-project/` and
   `/src/build/`). A whole `/src/` would hide the code of a student who
   works in `/src`, which is common in a container.
6. Docs: README (installing is copying `sc`), AGENTS, NOTES.md.

**Exit criterion.**
- `build/cargo/dist/sc`, copied to an empty directory, with the checkout
  out of reach (renamed), runs C, C++ with exceptions, the REPL and a trap
  with the stack line.
- `strace -f -e trace=%file` shows no path of the checkout and
  no `/sc/...` on disk, apart from lld's `stat` of the `.imports`. What
  may show up: the system libraries (`libstdc++.so.6`, `libc`),
  `/proc/self/exe`, the theme and the history, the student's
  `#include "..."` and the cells' directory.
- `make test-dist` with no difference.
- The size of `sc` and the time of the first run (before and after) in
  NOTES.md.

**Risks.**
- **Windows.** LLVM joins paths with the native separator
  (`path::append`), so clang and lld's `findFile` ask for
  `/sc/sysroot/include\stdio.h`. The pack's reader, which clang and lld
  share, has to accept both separators. And `/sc/...` is not
  absolute on Windows (the drive is missing). The VFS's `makeAbsolute`
  completes it with the drive of the current directory (`C:\sc\...`), which
  the pack's FS would also have to recognize, or else the prefix becomes one
  with a drive letter. This will be checked only once there is a Windows
  build. The pack is not the biggest obstacle. The `fork` of each cell
  (`session.rs:398`), the child's SIGIO (`session.rs:350`) and the loader's
  SIGINT (`loader.rs:870`) are Unix only (NOTES.md §51).
- **A forgotten reader.** A file that clang or lld opens outside the VFS
  or the hook shows up only outside the checkout. The `strace` of the
  criterion finds it.
- **An incompatible `.cwasm`.** If `Module::deserialize` fails (another
  CPU, another `Config`), compile the pack's `.so` on the spot instead of
  aborting. The bytes of the `.so` are in the pack anyway.
- **The system.** A single file does not mean no dependencies. The dist `sc`
  links `libstdc++.so.6` (up to `GLIBCXX_3.4.29`) and glibc (up to
  `GLIBC_2.39`) dynamically. "Another machine with the same system" means
  a glibc 2.39 or newer. A `-static-libstdc++` is a separate decision.

**Outside the phase.**
- The cells' temporary directory. Done later, on 2026-10-22
  (NOTES.md §58). Lld reads the `.o` and writes the `.so` in memory.
- The one-shot modes `sc --cc1` and `sc --ld` (`sc_main.cpp:61`, real FS).
  They belong to the tests (`test-native`, `learn-native`) and stay on disk.
- The JS host (6b, §5.5), which gives the same files to `sc.wasm`.
  Done later, on 2026-10-22, through the `sc_files` imports (NOTES.md §58).
- `import std;` in `sc` (decided on 2026-10-21: it stays out). Today it does
  not work. Embedded, the `std.pcm` (38 MB uncompressed, built with the
  session's flags) would far exceed the rest of the pack. Built on
  first use, it would bring back the writes to disk. It waits for a phase of
  its own, together with the PCH.
- A PCH of the headers to speed up the first cell. It would be one more
  file in the pack. It is a separate decision.

**Decisions.**
- **Decided on 2026-10-21:** `test-session` and `test-split` switch to
  `/sc/` (`sc-full` and `build/sc` reading from the pack), so that the corpus
  exercises the pack. A forgotten reader in clang or lld shows up only
  outside the checkout, and the corpus is the only battery large enough to
  catch it. The baselines of `session-*` and of `split-` change once (57
  results of `session-base` and 10 of `split` mention `build/sysroot`), and
  NOTES.md records the change as intended. `sc-wasm` keeps the
  real paths, because `sc.wasm` has no reader.
- **Decided on 2026-10-21:** the `.cwasm` files come from `build.rs`, with
  wasmtime 48.0.3 as a build dependency and a
  `[profile.*.build-override] opt-level = 2`. `cargo build` alone
  still makes `sc`, and the `Config` of the `Engine` comes from a single
  function, which `build.rs` and the loader both use. A separate step in make
  would compile wasmtime once, but `cargo build` would no longer be enough and
  the order would be circular (the `.cwasm` files go inside `sc`). Step 4
  measures the extra build time.
- **Decided on 2026-10-21:** the pack is a squashfs (item 1), written
  and read by `backhand`. It was the smallest of the ones measured with the
  reader included (4.31 MB), and it is a standard format. Zip was considered.
  Without a dictionary it is 5.5 MB. With a dictionary of our own inside it,
  it is 4.57 MB and only `sc` can read it.

### 5.5 The JS host (proposal)

The REPL in the browser, documented in phase 6b and not done yet. It
replaces the two halves of the native `sc`:
- the compiler (`sc --engine`) becomes `sc.wasm` as a library;
- the loader (`src/loader.rs`) becomes a loader in JS.

This plan gathers what phases 5 to 7 left noted (§1.2, §1.5, §3.6
and NOTES.md §25 and §48).

**Two Web Workers.** The page only shows the prompt and the output.
- The compiler's worker has `sc.wasm`. A long compilation does not freeze
  the page, and `terminate` stops any of them.
- The program's worker has the loader and the `.so` files. V8 has no epoch,
  and an infinite loop of the student's stops only with the worker's
  `terminate`.

The two talk through `postMessage`. The cell's `.so` goes from the compiler
to the program as a transferred `ArrayBuffer`, with no copy.

**The files.** Decided and done on 2026-10-22 (NOTES.md §58 and §59). They
are the native ones, with no FS in the WASI shim.
- The pack of §5.4 (headers, libraries, ignorelist, builtins and the
  `.args`) is in `sc.wasm` itself, an uncompressed squashfs that the
  squashfuse library reads (`engine/sc_pack.cpp`, NOTES.md §59). The
  host only calls the exported `sc_pack("/sc")`. The session reads through
  clang's VFS and lld's hook, as in the native build, and the `.args` of the
  native `sc` work unchanged. The page downloads only the module, which the
  server compresses.
- The session links the cells in memory. There is no `/tmp`. The worker's
  WASI only needs stdout, stderr, a clock and arguments.
- The `.cwasm` files do not work in the browser. V8 compiles the `.so` files
  and keeps the code of `WebAssembly.compileStreaming` in its HTTP cache.

**The session.** The contract is the README's, the same as `src/engine/wasm.rs`'s:
1. instantiate and call `_initialize` once;
2. call `chdir` (the guest starts in `/`);
3. put the arguments in place with `malloc` and call `sc_session_new`;
4. for each cell, `sc_session_cell` and the parts through `sc_result_ptr` and
   `sc_result_len`, read from `memory.buffer`.

The `Repl` of `src/engine/child.rs` becomes JS, with the log of the accepted
cells and the replay after an `SC_LOST` (§3.6).

**The cell that fails, without `fork`** (NOTES.md §25). The recommendation is
a snapshot of the linear memory before each cell, restored if the cell fails
or if the engine traps. In V8 it costs (NOTES.md §48, after 100
cells):

| memory | copy | restore |
| --- | ---: | ---: |
| 19 MB (C) | 3.7 ms | 2.5 ms |
| 42 MB (C++) | 16.9 ms | 4.4 ms |

That is 15% to 50% of a cell. The other option, replaying the log on each
error, cost ~2 s per error in the measured session (NOTES.md §25).

The snapshot also needs:
- `__stack_pointer`, which goes into the `EXPORTS` list of
  `scripts/link.py`. After a trap, it stays where the trap happened;
- zeroing the stretch by which the memory grew in the cell, because the
  memory does not shrink;
- closing in the shim the files the cell left open. With no FS in the
  shim (NOTES.md §58), there are none.

**Done on 2026-10-22, in Node (NOTES.md §56).** `__stack_pointer` is
exported. `test/node/engine.mjs` is the child `sc --engine` with the
snapshot and the restore, and the whole battery passes on it (`make
test-session-node`). A cell costs 11 ms in C and 23 ms in C++, against 17
and 29 ms for wasmtime with `fork`. The pack came later, through the
`sc_files` imports (NOTES.md §58).

**The limits.**
- **Timeout and Ctrl-C of the compilation:** the worker's `terminate`, a new
  worker and the replay of the log, like `SC_LOST`.
- **Memory:** the native 1 GB valve becomes a test of
  `memory.buffer.byteLength` after each cell. During the cell, the
  limit is that of wasm32 memory, 4 GB.
- **Stack:** V8 fixes the wasm stack (~1 MB, §1.2). The 16 cases of
  "call stack exhausted" in the corpus still fail in the browser.

**The loader in JS.** It follows `src/loader.rs`:
- one `WebAssembly.Memory` and one `WebAssembly.Table` for all the
  modules;
- `dylink.0` read from the bytes;
- the GOT entries as mutable `WebAssembly.Global` objects;
- the stubs through the table;
- `__cpp_exception` and `__c_longjmp` as a single `WebAssembly.Tag`.
  It needs exnref (§1.5).

The native loader's stubs and WASI shim are generated with `wasm-encoder`.
In JS they are host functions, or small modules assembled as bytes.

The JS loader handles an uncaught exception as the native loader does (§1.5):
`WebAssembly.Exception.getArg` with the `__cpp_exception` tag, then
`__cxa_begin_catch` and `std::terminate`.

The file and line of a trap (`frames`) depend on the DWARF of the `.so` files
and on the stack of the `Error`. V8 gives the stack with the offsets of the
functions. The rest is `Loader::symbols` in JS, or the trap goes without a
line.

**Done on 2026-10-22, in Node (NOTES.md §57).**
`test/node/loader.mjs` loads the `.so` files like `src/loader.rs`, without
ASan and without the DWARF. `make test-split-node` passes with 0 differences,
and `make test-session-node` runs the programs on it. The types of the
imports come from the bytes, and the stubs are modules assembled as bytes.
An `exit` becomes a trap, which wasm does not catch.

**To decide** (§8):
- The student's infinite loop. With `terminate`, the state of the cells is
  lost, and the accepted `.so` files are loaded again, which runs the
  top-level statements once more. The other option is a pass of our own that
  inserts `check_interrupt` into the loops (NOTES.md §7.2), read from a
  `SharedArrayBuffer`.
- The student's stdin. In a worker, it needs `Atomics.wait` on a
  `SharedArrayBuffer`, and that requires an isolated page (COOP and COEP).

## 6. Tests

| slice | what it is | oracle |
| --- | --- | --- |
| `ses-` | **all** the files of the corpus, including the ones with errors (they exercise `CleanUpPTU`), as one cell in a session | the baseline of the same mode on the uncut engine |
| `split-` | the ~1,743 executed programs, split into one cell per top-level declaration | **main criterion:** output and exit code equal to those of the one-shot `pic-` (T-M3, C-M2) |
| `seq-` | generated sequences: [error, good], [good, undo, good], [`static`/anonymous namespace across cells], [`import std;` after `#include`] | fixed expected output |
| `crash-` | the 36 fatal ones and the stack overflows as cells, followed by a good cell | the REPL alive; the good cell works; the symbols after the replay match the ones without a crash |
| `repl-` | the current one (explicit `.so` files, tests redefinition in the loader) | no change |
| `mem-` | 1,000 cells | the limits of §4 |

- `make learn` now drives the same `build/sc-engine repl`, so the rare
  paths (`CleanUpPTU`, undo, replay) get into `symbols.used` (T-M1).
- `ses-` uses a sample of clang's tests, not the whole corpus, so as
  not to double the time of learn (T-M2).
- **Done in phase 4 in another way (NOTES.md §47):** the wasm has the
  session only in phase 5, so the learning is done by `make learn-session`,
  on the native build, and the result goes into `cuts/symbols.session.used`,
  which both engines keep. `ses-` runs the whole corpus. Learn redoes only
  what differs.

## 7. The teacher's decisions

1. Redefinition in v1: **reject** (§3.4).
2. Pack: **a single binary, `sc`**, with the REPL, the loader and the engine; the child is `sc --engine` (§1.1).
3. Language standard: **the newest**, gnu23 and gnu++26 (§1.6).

## 8. Outside the scope of this plan (T-M8)

The REPL's front end:
- splitting into cells;
- when `main` runs;
- stdin;
- the order between top-level statements and `constructor`;
- the policy when the student's code traps;
- interrupting an infinite loop: in the native build, wasmtime's `epoch_interruption`; in the browser, `check_interrupt` (NOTES.md §7.2) or killing the worker (§5.5).

They affect the premise that "the student's state lives in the loader" and
need a plan of their own before phase 6.

The `sc` prototype (NOTES.md §26) made a provisional decision for each
of them, so that the REPL can be used before phase 6b.
