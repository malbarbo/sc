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
make native      # build/sc, the same engine for x86-64 Linux
make test-native # the corpus on build/sc against the uncut native engine
make learn-native # after make learn: what only the native engine calls
make spike       # PLANO-REPL.md phase 0: REPL cells through build/sc-spike
make test-proto  # proto/, the Rust API over engine/sc.h, after changing either
make test-split  # the corpus as REPL cells, after changing the session or the loader
make test-session # the session's slices: ses-, seq-, crash-, mem- (test/seq/expected.txt)
make repl        # build/loader/release/sc, the REPL (sc, sc FILE, sc help)
make test-aluno  # test/aluno, correct code, at each --safety level (scripts/flags.py)
make test-checks # test/checks, one bug per program: what sc says of it at each level
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
  difference. For an intended change in behaviour, add the file to `IGNORE` in
  `test/corpuslib.py` with the reason.
- `engine/sc_inv.cpp` sets by hand the defaults that clang's option parser
  would set. `make checkinv` must print SAME.
