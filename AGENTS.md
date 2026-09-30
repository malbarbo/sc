# sc

The C and C++ engine: clang and wasm-ld built for wasm32-wasip1 as one module,
cut to what C and C++20 (no exceptions) at -O0 need. `README.md` has the layout, the build and how the
cuts work. `ANALISE.md` (Portuguese) records the measurements behind each
decision. Add to it when a size or a decision changes.

## Build and test

```sh
make             # build/sc.wasm
make test        # the full corpus (~4 min on 48 cores) against build/sc-full.wasm
make test-small  # the programs only
make checkinv    # after changing engine/sc_inv.cpp
make learn       # after changing cuts/ or the LLVM pin, then make test
```

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
