# sc

sc is the C and C++ engine for students, in the family of
[sgleam](https://github.com/malbarbo/sgleam) and spython. The engine is clang
and wasm-ld built for `wasm32-wasip1` as one module, cut down to what C (gnu17)
and C++ (gnu++20, no exceptions) at `-O0` need: 19.3 MB, 7.1 MB gzip, 5.5 MB
zstd. Warnings come from the `-W` flags, as in clang (`-Wall`, `-Wshadow`,
`-Weverything`...); none is on beyond clang's defaults.

```sh
sc.wasm -cc1 <cc1 flags> -o out.o file.c    # compile one file (.cpp, .cc, .cxx: C++)
sc.wasm ld out.o crt1-command.o -lc ...     # link with wasm-ld (C++: -lc++ -lc++abi)
sc.wasm -cc1 <cc1 flags> -emit-module-interface -o std.pcm std.cppm
sc.wasm -cc1 <cc1 flags> -fmodule-file=std=std.pcm -o out.o file.cpp  # import std;
```

`import std;` takes a hello from 2.8 s to 0.14 s in V8. The engine builds
`std.pcm` once (6.8 s) from libc++'s `std.cppm`, without `<csetjmp>` and
`<csignal>`, which WASI's libc rejects.

`loader/` is the prototype of the REPL's loader, which runs each input as a
wasm shared object on top of the ones before:

```sh
sc.wasm -cc1 <cc1 flags for -fPIC> -o in1.o in1.c
sc.wasm ld -shared --experimental-pic --unresolved-symbols=import-dynamic in1.o -lc \
  libclang_rt.builtins.a -o in1.so
scload -L <sysroot>/lib/wasm32-wasip1 in1.so in2.so ...   # then main, if any
```

`ANALISE.md` (Portuguese) records how the build was chosen and measured.

## Layout

- `engine/` is the code of the engine. `sc_main.cpp` dispatches `-cc1` and
  `ld`. `sc_inv.cpp` builds clang's invocation in code rather than through the
  cc1 option parser. `sc_backend.cpp` replaces clang's `BackendUtil` with the
  O0 pipeline (the always-inliner, the coroutine lowering and codegen), so
  the optimization passes stay out. `sc_cuts.cpp`
  replaces a few LLVM functions sc does not need.
- `llvm/` has the patches over the LLVM pin (`Makefile` has the commit) and the
  CMake toolchain file for wasi-sdk. `wasi-host.patch` makes LLVM build for
  WASI, from YoWASP's patch. `no-veclibs.patch` lets `scripts/trim.py` build
  `TargetLibraryInfo` without the vector library tables. `dylink-weak.patch`
  makes wasm-ld list the weak exports of a shared object, so the loader keeps
  the first copy of an inline function, a vtable or a typeinfo.
- `cuts/` lists what the link leaves out. See [Cuts](#cuts).
- `scripts/link.py` links the engine from the LLVM libraries and applies the
  cuts. `scripts/trim.py` builds the trimmed intrinsics table and
  `TargetLibraryInfo`. `attrib.py`, `cat2.py` and `o0passes.py` are for
  measuring (see `ANALISE.md` §10).
- `test/corpus.py` runs the corpus through an engine (`--flags [c++]
  [pic] [pcm|mod]` prints the cc1 flags; `WFLAGS=-Weverything` adds warnings).
  `test/checkinv.cpp` checks `sc_inv.cpp` against clang's option parser.
  `test/samples/` (C), `test/cxx/` (C++) and `test/mod/` (C++ with
  `import std;`) have small student-style programs. `test/repl/` has
  sequences of REPL inputs (`c1.c`, `c2.c`...).
- `loader/` (Rust, wasmtime) loads wasm shared objects the way the REPL will:
  `libc.so` and `libc++.so` from wasi-sdk, then each input, sharing one
  memory and one function table. A definition in an input hides an older one
  for the inputs after it. `ANALISE.md` §18 has the design.
- `bench/` times the engine in V8 (`cc.mjs`) and compares O0/O1/O2 code.
- `build/` holds everything generated and is not tracked.

## Build

Needs cmake, ninja, a native clang and lld, `wasm-opt` (binaryen),
`wasm-tools`, `wasmtime`, `llvm-cxxfilt` and, for the loader, cargo.

```sh
make             # build/sc.wasm; the first run builds LLVM twice (~1 h)
make test        # the corpus on build/sc.wasm against the uncut engine
make test-small  # only the ~240 programs that are linked and run
make test-warn   # the corpus with -Weverything against the uncut engine
make checkinv    # the invocation of sc_inv.cpp against clang's parser
make size        # raw, gzip and zstd sizes
make loader      # build/loader/release/scload
make learn       # rebuild what the cuts keep, from the corpus
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
