# sc

The C and C++ engine: clang and wasm-ld built for wasm32-wasip1 as one module,
cut to what C23 and C++26 (with exceptions) at -O0 need. `README.md` has the
layout, the build and how the cuts work. `ANALISE.md` (Portuguese) records the measurements behind each
decision. Add to it when a size or a decision changes.

## Build and test

```sh
make             # build/sc.wasm
make sysroot     # build/sysroot: wasi-sdk's libc, libc++ with exceptions from the pin
make test        # the full corpus (~4 min on 48 cores) against build/sc-full.wasm
make test-small  # the programs only
make checkinv    # after changing engine/sc_inv.cpp
make test-warn   # the full corpus with -Weverything
make learn       # after changing cuts/ or the LLVM pin, then make test and test-warn
                 # (and learn-native, learn-session, learn-session-wasm)
make native      # build/sc, what ships: the REPL with the engine linked in
make test-native # the corpus on build/sc against the uncut native engine
make learn-native # after make learn: what only the native engine calls
make learn-session # after make learn-native: what only the REPL's session calls
make learn-session-wasm # after learn-session: what only sc.wasm's session calls
make test-proto  # src/engine, the Rust API over engine/sc.h, and sc's modules, after changing them
make test-split  # the corpus as REPL cells, after changing the session or the loader
make test-session # the session's slices on build/sc against the uncut engine (build/sc-full)
make test-session-warn # ses-, seq- and crash- with -Weverything
make test-session-wasm # the same slices on sc.wasm's session (build/sc-wasm)
make test-session-wasm-warn # and test-split-wasm
make test-split-node # test-split with the JS loader (test/node/loader.mjs) in place of scload
make test-split-node-rs # test-split with sc.wasm's loader on Node's primitives (test/node/wasm.mjs)
make test-aluno-node-rs # test-aluno with sc inside sc.wasm (the machine) on Node's primitives
make test-session-node # the same slices with the child and the loader on Node (test/node)
make repl        # build/cargo/release/sc, the REPL (sc, sc FILE, sc help), unstripped
cargo build --release # the same sc (build.rs runs make sc-deps)
make dist        # build/cargo/dist/scload and sc: small, LTO (~4 min)
make test-aluno  # test/aluno, correct code, at each --safety level (scripts/flags.py)
make test-checks # test/checks, one bug per program: what sc says of it at each level
make test-loader # src/loader.rs: Ctrl-C, many exceptions, after changing the loader or wasmtime (no engine)
make test-dist   # before shipping: test-split, test-session, test-aluno, test-checks on the dist sc
```

The cuts keep the code of every warning: `make learn` runs the corpus with
`-Weverything` (`ANALISE.md` §13).

## Rules

- `build/` is generated. Change the sources in `engine/`, `llvm/`, `cuts/`,
  `scripts/` and `test/`, never the copies in `build/`. The one exception is
  `build/src`, which is the LLVM pin with `llvm/*.patch` applied. When you
  change it there, regenerate the patch with `git diff` in `build/src`.
- Prefer a cut in `cuts/` to a patch in `llvm/`. A cut does not conflict
  when LLVM is rebased. Patch only where the saving is worth it.
- A change to the engine or the cuts is done when `make test` reports no
  difference, and `make test-session` and `test-session-wasm` none for the
  REPL's engine and `sc.wasm`'s session. For an
  intended change in behaviour, add the file to `IGNORE` in
  `test/corpuslib.py` with the reason.
- `engine/sc_inv.cpp` sets by hand the defaults that clang's option parser
  would set. `make checkinv` must print SAME.
