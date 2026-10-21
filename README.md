# sc

sc is the C and C++ engine for students, in the family of
[sgleam](https://github.com/malbarbo/sgleam) and spython. The engine is clang
and wasm-ld built for `wasm32-wasip1` as one module, cut down to what C (gnu23)
and C++ (gnu++26, with exceptions) at `-O0` need: 19.4 MB, 7.1 MB gzip, 5.5 MB
zstd. Exceptions and `setjmp`/`longjmp` use wasm's exception handling, in the
standard form (exnref), with libc++, libc++abi and libunwind built from the
LLVM pin into `build/sysroot` (`make sysroot`). Warnings come from the `-W` flags, as in clang (`-Wall`, `-Wshadow`,
`-Weverything`...); none is on beyond clang's defaults.

```sh
sc.wasm -cc1 <cc1 flags> -o out.o file.c    # compile one file (.cpp, .cc, .cxx: C++)
sc.wasm ld out.o crt1-command.o -lsetjmp -lc ...  # link with wasm-ld (C++: -lc++ -lc++abi)
sc.wasm -cc1 <cc1 flags> -emit-module-interface -o std.pcm std.cppm
sc.wasm -cc1 <cc1 flags> -fmodule-file=std=std.pcm -o out.o file.cpp  # import std;
```

`sc.wasm` is also a library: a reactor that exports the REPL's session
(`engine/sc.h`) besides `_start`. As a command (`wasmtime run`) it runs as
above. A host of the session:
- calls `_initialize` once per instance (a second call traps), and never
  `_start`;
- calls the exported `chdir` with the directory where a cell's
  `#include "..."` files are: the guest starts in `/`;
- puts the arguments and the cells in the guest's memory with the exported
  `malloc` and `free`, and reads a result at `sc_result_ptr`;
- gives the guest a writable `/tmp` (or `TMPDIR`), where a session keeps
  its cells' objects.

`node:wasi` refuses a module with both `_initialize` and `_start`, so pass
it an object in place of the instance (`e` its exports):
`wasi.start({exports: {memory: e.memory, _start() { e._initialize(); e._start(); }}})`
for the command, `wasi.initialize({exports: {memory: e.memory, _initialize: e._initialize}})`
for the library (`bench/cc.mjs`).

The same engine is also built natively (x86-64 Linux), with the same cuts:
24.5 MB (7.6 MB zstd) against 52 MB uncut. It still compiles to wasm, so the
student's code runs in wasm either way. `build/sc` is what ships: the REPL
(`sc`, `sc FILE`, `sc help`) with that engine and the session linked in.
Its compiler is a child, `sc --engine`, and `sc --cc1 ...` and `sc --ld ...`
are the one-shot engine (`--`: a student's program may be called `ld`).

`import std;` takes a hello from 2.8 s to 0.14 s in V8. The engine builds
`std.pcm` once (6.8 s) from libc++'s `std.cppm`, without `<csignal>`, which
WASI's libc rejects.

`scload` (`src/bin/scload.rs`) runs each input with the REPL's loader as a
wasm shared object on top of the ones before:

```sh
sc.wasm -cc1 <cc1 flags for -fPIC> -o in1.o in1.c
sc.wasm ld -shared --experimental-pic --unresolved-symbols=import-dynamic in1.o -lc \
  libclang_rt.builtins.a -o in1.so
scload -L build/sysroot/lib/wasm32-wasip1 in1.so in2.so ...   # then main, if any
```

`ANALISE.md` (Portuguese) records how the build was chosen and measured.

## Layout

- `engine/` is the code of the engine. `sc.h` is its C API (`PLANO-REPL.md`
  §2). `sc_main.cpp` dispatches `-cc1` and `ld` (`sc_main`), and
  `sc_entry.cpp` is the executables' `main`. `sc_inv.cpp` builds clang's invocation in code rather than through the
  cc1 option parser. `sc_backend.cpp` replaces clang's `BackendUtil` with the
  O0 pipeline (the always-inliner, the coroutine lowering and codegen), so
  the optimization passes stay out. `sc_cuts.cpp`
  replaces a few LLVM functions sc does not need. `sc_session.cpp` is the
  REPL's session (one clang instance, each cell parsed on top of the ones
  before), `sc_api.cpp` the session part of `sc.h` over it (each cell linked
  into a shared object). `sc_start.cpp` is
  `sc.wasm`'s `_start`. `sc` links the native engine, with the session too.
- `Cargo.toml`, `build.rs` and `src/` are the Rust crate `sc`
  (`ANALISE.md` §51). `cargo build --release` makes
  `build/cargo/release/sc` and `scload`: without `SC_LINK_ARGS`, `build.rs`
  runs `make sc-deps` (the cut engine's link arguments, and what `sc`
  reads at run time) and links `build/link-session-cut/engine.args`.
  From a clean checkout that is the LLVM pin built twice (native tools,
  host libraries) and the sysroot, without `make learn`: the cuts are in
  `cuts/`. It took 14 min on 56 cores (7.3 h of CPU) and 6.6 GB.
  - `src/engine/` (the `engine` feature, the default) is bindgen over
    `engine/sc.h`, the protocol between the REPL and its child (`wire`),
    the REPL's side of it (`Child`, and `Repl`, which replays a lost
    session) and a safe Rust API over the linked engine (`Session`,
    `main`, `serve`; `ANALISE.md` §23 and §24). `make test-proto` runs its
    tests (`tests/session.rs`) on the uncut engine. With the `wasm`
    feature, the same API calls `sc.wasm`'s exports through wasmtime
    (`src/engine/wasm.rs`): `build/sc-wasm`, the test host of the
    module (`ANALISE.md` §48).
  - `src/loader.rs` loads wasm shared objects the way the REPL does:
    `libc.so` and `libc++.so` from wasi-sdk, then each input, sharing one
    memory and one function table. A definition in an input hides an
    older one for the inputs after it, and an input may use what a later
    one defines. `ANALISE.md` §18 has the design.
  - `src/bin/sc/` is the REPL, `src/bin/scload.rs` the loader alone, for
    the batteries. `sc --replay` is the test of the REPL's side
    (`test/split.py`, `test/session.py`); `build/sc-full` is `sc` with the
    uncut engine, their base.
- `llvm/` has the patches over the LLVM pin (`Makefile` has the commit) and the
  CMake toolchain file for wasi-sdk. `wasi-host.patch` makes LLVM build for
  WASI, from YoWASP's patch. `no-veclibs.patch` lets `scripts/trim.py` build
  `TargetLibraryInfo` without the vector library tables. `dylink-weak.patch`
  makes wasm-ld list the weak exports of a shared object, so the loader keeps
  the first copy of an inline function, a vtable or a typeinfo.
  `libcxx-locale.patch` keeps libc++'s z/OS locale shims off WASI.
  `incremental-c-scopes.patch` keeps a REPL cell's block scopes in C from
  leaking names into the cells after it (`ANALISE.md` §24).
  `incremental-stmts.patch` lets a REPL cell start with `-` or `+`, and
  keeps the value of an expression without its `;` at the end of a cell,
  which the REPL shows, from being discarded (`ANALISE.md` §26).
  `asan-cleanupret.patch` keeps ASan from clearing a frame's shadow at
  every exception cleanup inside the function, which made `main` of a C++
  program many times bigger (`ANALISE.md` §34). `pattern-init.patch`
  gives an uninitialized integer 0xAA… on wasm32 too, where clang gives
  it -1, which looks initialized; a pointer keeps 0xFFFFFFFF (§42).
  `repl-outside-function.patch` is for what a REPL cell has outside a
  function: an error for a `return` there, which clang dropped without
  one, crashing CodeGen (§43), and for a `static` local there, which has
  no function to be named after; a lambda or a class local to a statement
  there is mangled as one in a function `__stmts_N` (§45).
  `incremental-anon-namespace.patch` makes the anonymous namespace of
  every cell the one of the first, as in one file: each cell has a
  `TranslationUnitDecl` of its own, and its anonymous namespace was
  another, so a name redefined there was ambiguous. It is in the
  `TranslationUnitDecl`'s accessors, as `NamespaceDecl` does it (§46).
  `incremental-input-end.patch` ends the loops that stop at the end of the
  file at the end of a cell too: a cell that ends inside a bracket (`int
  f() {`) or inside a `_Pragma(` hung the engine. In the parser, the end of
  the cell inside a bracket is the end of the file, and the counts of
  brackets are 0 again at the end of each cell. In the preprocessor, the
  loops of `_Pragma(`, of a macro's arguments and of `__has_*(` stop there
  too; the end of the cell lexed 65536 times is the end of the file, a
  safety net (§46).
  `incremental-access.patch` keeps the guess between a statement and a
  declaration at the top of a cell from checking access early: it
  rejected `B::C::C() {}` for a private class `C` of `B` (§46).
  `incremental-remove-decl.patch` lets `DeclContext::removeDecl`, which
  takes the decls of a cell that failed back out, skip a name with no
  lookup entry: the variable of a tuple-like binding is added hidden, and
  a decl added to a namespace from a module is not in its lazy table; the
  release build read past the table there (§48).
  `runtimes.cmake.in` is the toolchain file for libc++ with exceptions.
- `cuts/` lists what the link leaves out. See [Cuts](#cuts).
- `scripts/link.py` links the engine from the LLVM libraries and applies the
  cuts; `--native` links `build/sc` from the host build of LLVM. `scripts/trim.py` builds the trimmed intrinsics table and
  `TargetLibraryInfo`. `attrib.py`, `cat2.py` and `o0passes.py` are for
  measuring (see `ANALISE.md` §10). `scripts/flags.py` has the engine's flags:
  `[c++|gnu17] [pic] [pcm|mod]` prints the cc1 flags, `--session c|c++
  <build>` a REPL session's arguments.
- `test/corpus.py` runs the corpus through an engine (`WFLAGS=-Weverything`
  adds warnings).
  `test/checkinv.cpp` checks `sc_inv.cpp` against clang's option parser.
  `test/samples/` (C), `test/cxx/` (C++) and `test/mod/` (C++ with
  `import std;`) have small student-style programs. `test/split.py` cuts
  every program of the corpus into REPL cells (`test/cells.py`) and runs
  them through `build/sc --replay`. `test/session.py` has the other slices of
  the session: every source of the corpus as one cell (`ses-`), the
  sequences of cells, undos and restarts in `test/seq/` (`seq-`), the
  sources that crash the engine between good cells (`crash-`) and the
  memory over 1000 cells (`mem-`). `test/repl/` has
  sequences of REPL inputs (`c1.c`, `c2.c`...).
- `rt/sc_rt.c` is `libsc_rt`, in the sysroot: the handlers of UBSan's
  checks and of libc++'s hardening for the levels `on` and `strict` of
  `--safety`, which say the error and trap (`ANALISE.md` §32).
  `rt/sc_asan.c`, in the same library, is ASan's runtime: the shadow, the
  allocator with its redzones and quarantine, and the checked libc
  functions, those that write (`scanf` among them, §39) and those that read
  the program's strings (§41), named as wasm-ld's `--wrap` names them
  (`ANALISE.md` §34).
  It asks the loader for the stacks where each block was allocated and
  freed, which the reports show (§36), and finds the blocks nothing
  reaches, the leaks of `strict` and of `:leaks` (§37). `rt/c_longjmp.s` is the tag of
  `setjmp`/`longjmp`, defined strong in the sysroot's `libsetjmp.so`
  (§35). `rt/ignorelist.txt` keeps the checks of `strict` alone (U-estrito)
  out of the sysroot's headers (§40).
- `bench/` times the engine in V8 (`cc.mjs`) and compares O0/O1/O2 code.
- `build/` holds everything generated and is not tracked.

## Build

Needs cmake, ninja, a native clang and lld, `wasm-opt` (binaryen),
`wasm-tools`, `wasmtime`, `llvm-cxxfilt` and, for `sc` and the loader,
cargo and libclang (bindgen).

```sh
make             # build/sc.wasm; the first run builds LLVM twice (~1 h)
make test        # the corpus on build/sc.wasm against the uncut engine
make test-small  # only the ~240 programs that are linked and run
make test-warn   # the corpus with -Weverything against the uncut engine
make checkinv    # the invocation of sc_inv.cpp against clang's parser
make size        # raw, gzip and zstd sizes
make loader      # build/cargo/release/scload
make learn       # rebuild what the cuts keep, from the corpus
make native      # build/sc, the REPL with the native engine (builds LLVM once more)
make test-native # the corpus on build/sc against the uncut native engine
make learn-native # after make learn: what only the native engine calls
make test-proto  # the Rust API (src/engine) over the uncut native engine
make learn-session # after make learn-native: what only the REPL's session calls
make learn-session-wasm # then what only sc.wasm's session calls
make test-split  # the corpus's programs as REPL cells (build/sc)
make test-session # the session's other slices (test/session.py) against the uncut engine
make test-session-warn # ses-, seq- and crash- with -Weverything
make test-session-wasm # the same slices on sc.wasm's session (build/sc-wasm)
make test-session-wasm-warn
make test-split-wasm
```

The first `make` fetches wasi-sdk 29, the LLVM pin with the patches, and the
corpus (c-testsuite and gcc's torture tests). It then builds LLVM natively for
tablegen and wasm-ld, and cross-builds it for wasm.

## Cuts

The link keeps only what the corpus uses. There are two kinds of cut:

- **Objects** (`cuts/objects.txt`): regexes over archive member paths. The
  archives are rebuilt without the matching objects, and each function left
  undefined gets a stub that traps. Lines starting with `!` keep an object.
- **Functions** (`cuts/symbols.txt`): every global function selected is
  redirected with `wasm-ld --wrap` to a stub that traps, so `--gc-sections`
  drops its body. `cuts/symbols.used` lists the ones the corpus calls, which
  stay. Pass initializers that O0 does not use become empty functions instead
  (`noop:` rules). `cuts/o0init.txt` lists the ones O0 does use.

`make learn` rebuilds `cuts/symbols.used` and appends to `cuts/objects.txt`,
running the corpus with `-Weverything` so the code of every warning stays.
First it links with every wrapper recording its first call and forwarding to
the original function, and runs the corpus once. Then it checks the cut engine
against the uncut one and puts back anything that traps. Run it after changing
a cut rule or bumping LLVM, then `make test`.

A C or C++ feature the corpus does not cover can reach a stub. The engine then
traps with `unreachable`.

The native engine (the one in `build/sc`) takes the same cuts from a host build of LLVM
(`build/host`, from the same patched sources). The function names learned on
wasm are matched by their demangled names, where `int64_t` is `long long`
on wasm and `long` on x86-64, and `std::` is libc++'s `std::__2` on wasm and
libstdc++'s `std::__cxx11` natively. What inlines differently on x86-64 adds
the functions in `cuts/symbols.host.used` (`make learn-native`, which runs
the corpus on `sc --cc1` and `sc --ld` as `make test-native` does). A cut
function prints its name and aborts; cut data is zeros, as at address 0 in
`sc.wasm`.

The session in that engine is cut the same way. What only it calls is in
`cuts/symbols.session.used`, by demangled name, which both engines keep:
`sc.wasm` has the session too. `make learn-session`
(`scripts/learn_session.py`) learns it on the native engine, from the
functions that abort in the session's slices (`ses-` and `seq-` and
`crash-` with `-Weverything`, and `split-`), and puts back the objects they
are in.

The corpus has 11,736 files: c-testsuite, gcc's torture tests (`execute/` is
linked and run, `compile/` only compiled), clang's own C and C++ tests
(compiled only) and the programs in `test/samples`, `test/cxx`, `test/mod`
and `test/repl`.
The engine first builds the `std` module, which `test/mod` imports. The engine
itself writes each object to a file and links the programs with `sc.wasm ld`,
so the cuts keep what the linker needs. The programs are also compiled with
`-fPIC`, linked as shared objects and run by the loader (`pic-` results), as
the REPL will do with each input, and the sequences in `test/repl` run as
REPL sessions (`repl-`). The
compile-only files compare the diagnostics and the md5 of the object. The
reference is `build/sc-full.wasm`, the same engine without cuts.
