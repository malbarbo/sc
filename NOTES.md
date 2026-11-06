# sc: notes on a C engine built on clang/LLVM

Date: 2026-09-29. LLVM at the pin of `rustports/llvm/upstream`
(`5fe235b9862f`, 23.0.0git); since 2026-10-13, `llvmorg-23.1.3`
(`0d261d1ca552`, §21). Build toolchain: wasi-sdk 29 (clang 21).
Everything here was measured. The `README.md` describes the build, and §9
lists the old paths.

## 1. Summary

- **Whole clang + wasm-ld can run as a single `wasm32-wasip1`
  module**, with no threads, no processes and no signals. I compiled it and
  ran it on wasmtime and V8. It compiles C with `<stdio.h>`, links against
  wasi-libc, and the program runs. The diagnostics match native clang.
- LLVM **has no upstream support** for hosting on WASI. A ready patch exists
  (YoWASP, PR #92677, open since 2024, never merged). It has ~170 lines in
  25 files, almost all `#ifdef __wasi__` in `llvm/lib/Support`. I ported it
  to the pin with 4 trivial adjustments.
- **Size with CMake options alone** (1 target, no analyzer, no asserts,
  MinSizeRel): `clang.wasm` is **46.8 MB** (16.6 MB gzip). The prototype
  `sc.wasm`, cc1 + wasm-ld in one module, is **46.0 MB** (16.3 MB gzip).
  For comparison, spython is 18.6 MB (7.4 MB gzip) and sgleam 3.4 MB
  (1.4 MB).
- **C++ weighs little.** Cutting what is C++-only saves **1.9 MB (4 %)**
  verified, 3.2 MB at most. C++ is woven into the shared code
  (`SemaExpr`, `ExprConstant`, `CodeGenModule`...), and only a rewrite of
  clang would take that weave apart. The weight is in the **LLVM
  optimization pipeline** (−6.7 MB), the **other targets built into clang**
  (−3.9 MB) and the **modules/PCH/dumpers tooling** (−3.6 MB). None of these
  has a CMake option.
- **With all the cuts together**, verified against a C corpus,
  `sc.wasm` (cc1 + wasm-ld) is **26.9 MB, 9.8 MB gzip, 7.6 MB zstd**. It
  compiles and links inside its own module, and the programs run. With a
  source patch that turns constructors into stubs, the ceiling is **22.9 MB
  (8.4 MB gzip)**. What remains is the C frontend (27 %), the LLVM backend
  (27 %) and IR/Analysis (17 %). That is the floor short of rewriting clang.
- **None of this requires editing clang.** The cuts work per object, at link
  time, with stubs generated automatically. The only source edit outside
  the WASI patch was 1 line in lld.
- Performance on V8: loading the module takes ~0.45 s, the first compilation
  ~1.1 s, and after that **~0.23 s per student program**. That is acceptable
  for "run" and even for a REPL.
- **Round 2 (§10): 15.75 MB, 5.8 MB gzip, 4.6 MB zstd**, identical to clang
  (diagnostics, object bytes and execution) on a corpus of **7,462
  C files** (clang's tests, gcc torture, c-testsuite). It is smaller than
  spython. The larger corpus showed that the round 1 number was optimistic
  (the honest one was 28.6 MB). The saving came from three things. sc has
  its own invocation and backend in `sc_main` (the option parser and the
  `PassBuilder` go out at the root). The cuts work per **symbol** with
  `--wrap` (not only per object). And the tablegen tables keep only the
  wasm target.

## 2. Build: what already exists as an option

Options that **already exist** and were used (none requires a code change):

| option | effect |
| --- | --- |
| `LLVM_TARGETS_TO_BUILD=WebAssembly` | only the wasm backend of **LLVM** |
| `LLVM_DEFAULT_TARGET_TRIPLE=wasm32-wasip1` | host = target |
| `CMAKE_BUILD_TYPE=MinSizeRel`, `LLVM_ENABLE_ASSERTIONS=OFF` | `-Os`, no asserts |
| `LLVM_ENABLE_THREADS=OFF`, `LLVM_ENABLE_PIC=OFF`, `LLVM_BUILD_STATIC=ON` | required by WASI |
| `CLANG_ENABLE_STATIC_ANALYZER=OFF`, `CLANG_ENABLE_OBJC_REWRITER=OFF` | drops the analyzer |
| `CLANG_PLUGIN_SUPPORT=OFF`, `LLVM_ENABLE_PLUGINS=OFF` | no `dlopen` |
| `LLVM_ENABLE_{ZLIB,ZSTD,LIBXML2,LIBEDIT}=OFF`, `LLVM_ENABLE_BACKTRACES=OFF` | no deps |
| `-Wl,-z,stack-size=8388608`, `-Wl,--max-memory=4294967296` | 8 MB stack (the parser is recursive) |

What **does not exist** as an option (and that is where the weight is):

- `LLVM_TARGETS_TO_BUILD` does not affect **clang**. `clang/lib/Basic/Targets/*`,
  `clang/lib/CodeGen/Targets/*` and `TargetBuiltins/*`, `Sema{ARM,X86,RISCV,...}`
  and `Driver/ToolChains/*` are always compiled and linked (factories with a
  `switch` on the triple).
- clang has no way to turn off C++, Objective-C, OpenMP, OpenACC, CUDA/HIP,
  HLSL, SYCL or OpenCL. It is a single language with `LangOptions`, decided
  at run time. The `Sema` constructor builds `SemaOpenMP`, `SemaObjC`,
  `SemaCUDA`, `SemaHLSL`... unconditionally.
- There is no way to turn off optimization passes. The `PassBuilder` (which
  `clang/lib/CodeGen/BackendUtil.cpp` uses even at `-O0`) references every
  pass through `PassRegistry.def`, so every pass gets linked.
- `lld` always links ELF, COFF, MachO, MinGW and Wasm. And `lldWasm` pulls
  in `LLVMLTO` (which pulls in the whole optimization pipeline) to accept
  bitcode.

## 3. Hosting on WASI: the patch

`build/wasi-host.patch` is the YoWASP commit (`llvmorg-22.1.0+wasm`).
Applying it to the 23git pin needed:

- 2 hunks redone by hand (`InitLLVM.cpp`, `Process.inc`, where the context
  changed);
- `Signals.cpp` and `CrashRecoveryContext.cpp`, whose signatures gained the
  `NeedsPOSIXUtilitySignalHandling` parameter;
- `clang/tools/driver/driver.cpp`, which has new code that uses
  `raise`/`<signal.h>`;
- `clang/lib/AST/ByteCode/Interp.cpp`. The new constexpr interpreter uses
  `musttail`, which requires `-mtail-call`. I turned off `USE_TAILCALLS` on
  wasm without tail-call.

Total: 25 files, +169/−33. The maintenance cost is rebasing this at every
LLVM bump (YoWASP did that from 18 to 22; its repository was archived in
2026-03).

Behaviour observed on WASI:

- Writing the `.o` to a file fails (`IO failure on output stream`). LLVM
  writes to a `.tmp` and renames it. With `-o -` it works. In the engine
  the output goes to memory (`raw_svector_ostream`), so it does not matter.
- There is no `fork`/`exec`. The driver has to run cc1 in process (that is
  the default, `CLANG_SPAWN_CC1=OFF`). In the engine, I call `cc1_main`
  directly (§5).
- wasi-sdk's wasm-ld runs `wasm-opt` on its own when the link gets `-O`
  and finds `wasm-opt` in the `PATH`. That erases the `name` section. It is
  a build trap, not a runtime one.

## 4. How much each part weighs

### 4.1 Attribution by file (upper bound)

Linker map (`-Map`), code+data, grouped by file name. It is an upper bound.
The shared code calls these files, so not all of it goes away.

| category | native x86-64 | wasm |
| --- | ---: | ---: |
| clang: C core (Lex/Parse/Sema/AST/CodeGen/Frontend/Basic) | 18.4 % | 17.8 % |
| llvm: backend (CodeGen/SelectionDAG/MC/wasm) | 15.3 % | 15.9 % |
| llvm: IR optimizations (Vectorize/IPO/Scalar/InstCombine/Instrumentation...) | 15.4 % | 15.5 % |
| llvm: IR/Analysis/TransformUtils | 12.3 % | 12.3 % |
| clang: other targets (Basic/CodeGen/Sema/Driver) | 8.0 % | 7.3 % |
| clang: **C++** (C++-only Sema/Parse/AST/CodeGen) | **6.4 %** | **6.3 %** |
| clang: modules/PCH/APINotes/ExtractAPI | 5.0 % | 5.4 % |
| clang: OpenMP/OpenACC/CUDA/HLSL/SYCL/OpenCL | 4.3 % | 4.3 % |
| clang: dumpers/code-complete/rewrite | 2.7 % | 2.8 % |
| clang: new constexpr interpreter | 2.6 % | 2.8 % |
| llvm: Support/TargetParser/Option | 2.5 % | 2.3 % |
| clang: ObjC/Blocks | 1.2 % | 1.2 % |
| llvm: GlobalISel (wasm does not use it) | 0.9 % | 1.1 % |
| clang: extra analyses (lifetime, unsafe-buffer, thread-safety) | 0.9 % | 0.9 % |
| clang: PGO/coverage/debug info | 0.9 % | 1.0 % |

### 4.2 Verified cuts

Method (`build/scripts/exp2.py`): I rebuild the `.a` files without the
objects of one category. I link once with `--allow-undefined` to find the
wasm signature of each symbol left missing, generate an `unreachable` stub
with the symbol's name, and link again. Then I compile the corpus
(`build/corpus/`, 5 programs with structs, unions, VLA, varargs, `_Generic`,
`_Static_assert`, bitfields, float/long double, stdio/stdlib/string/math,
and 2 programs with errors and warnings from `-Wall -Wextra`). I compare the
diagnostics, the generated code when run, and the output with full clang.
If a stub gets called, the trap names the symbol. The object that defines it
comes back, and I repeat until the result is identical.

Base: `clang.wasm` 46.78 MB (`wasm-opt -Oz`, no debug). Sizes after
`wasm-opt -Oz`:

| cut (alone) | objects removed (final) / returned | size | verified saving | maximum saving* |
| --- | ---: | ---: | ---: | ---: |
| LLVM IR optimizations (Vectorize, IPO, Scalar, InstCombine, Instrumentation, Coroutines, ObjCARC) + GlobalISel | 439 / 23 | 40.07 MB | **−6.71 MB** | −7.1 |
| other targets (clang Basic/CodeGen/Sema/Driver, except WebAssembly) | 235 / 16 | 42.92 MB | **−3.86 MB** | −4.4 |
| modules/PCH, APINotes, ExtractAPI, dumpers, code-complete, extra analyses | 133 / 8(+3) | 43.22 MB | **−3.56 MB** | −4.0 |
| ObjC, OpenMP, OpenACC, CUDA/HIP, HLSL, SYCL, OpenCL | 83 / 14 | 44.66 MB | **−2.12 MB** | −3.7 |
| **C++** (templates, `DeclCXX`, `ExprCXX`, lambdas, coroutines, vtables, MS mangling...) | 31 / 15 | 44.93 MB | **−1.85 MB** | −3.2 |
| clang driver (the engine calls cc1 directly) | 76 / 3 | 45.43 MB | **−1.35 MB** | — |
| new constexpr interpreter (`AST/ByteCode`, used only with an experimental flag) | 26 / 1 | 45.54 MB | **−1.24 MB** | −1.4 |

\* maximum saving = removing everything, before returning any objects (the
corpus breaks).

Almost every returned object is a **constructor or a registration**, not
logic. `Sema::Sema` builds `SemaOpenMP`, `SemaObjC`, `SemaCUDA`, `SemaHLSL`,
`SemaSYCL`, `SemaX86`, `SemaARM`, ... (16 targets and 9 languages).
`initializeCodeGen`/`initializeGlobalISel` register passes that never run.
The wasm subtarget builds GlobalISel's `LegalizerInfo`. The C
`CodeGenModule` builds `CodeGenVTables` and the `ItaniumVTableContext`. On
the C++ side, some predicates of the shared code go through C++ files
(`Sema::AdjustDeclIfTemplate`, `DiagnoseUnexpandedParameterPack`,
`RequireNonAbstractType`, `ResolveExceptionSpec`,
`PerformImplicitConversion` in `SemaExprCXX`). The full list of traps is in
`build/exp/*/loop.log`.

### 4.3 Everything together, in the engine's shape

Prototype `build/proto/sc_main.cpp` (30 lines): a `main` that dispatches
`-cc1 ...` to `cc1_main` and `ld ...` to `lld::lldMain` with only the wasm
driver. It is a single module, with no clang driver and no ELF/COFF/MachO in
lld.

| variant | raw | gzip | zstd -19 |
| --- | ---: | ---: | ---: |
| YoWASP `@yowasp/clang` 22 (clang+lld+tools, with asserts) | 75.5 MB | 22.7 MB | 15.8 MB |
| `clang.wasm` with CMake options only | 46.8 MB | 16.6 MB | 12.4 MB |
| `sc.wasm` (cc1 + wasm-ld), no cuts | 46.0 MB | 16.3 MB | — |
| **`sc.wasm` with all the verified cuts** | **26.9 MB** | **9.8 MB** | **7.6 MB** |
| `sc.wasm` ceiling (cuts without returning anything; needs stubs in the source) | 22.9 MB | 8.4 MB | — |
| *spython today* | *18.6 MB* | *7.4 MB* | |
| *sgleam today* | *3.4 MB* | *1.4 MB* | |

The cut engine compiles and links **inside itself** (p1, p2 and p5 of the
corpus run with the right output). That took 1 line in lld.
`SymbolTable::compileBitcodeFiles` builds the `BitcodeCompiler` (LTO) even
with no bitcode at all. An `if (ctx.bitcodeFiles.empty()) return;` fixes it
and removes the whole LTO.

What remains in the 26.9 MB (share from the linker map):

| part | % |
| --- | ---: |
| LLVM backend (CodeGen 3.7, SelectionDAG 3.0, wasm 0.9, AsmPrinter, MC, Object) | 27 % |
| clang's C core (Lex/Parse/Sema/AST/CodeGen/Frontend/Basic) | 27 % |
| IR, Analysis, TransformUtils | 17 % |
| C++ that the shared code still calls | 6 % |
| Support, Option, TargetParser | 4 % |
| the rest (OpenMP/ObjC/targets still called, dumpers, debug info, libc++) | 19 % |

Second-round cuts in the backend (MIR printer, CodeView, ELF and MachO
readers, pipeliner, register allocators unused at -O0) are in
`build/exp/all2`. They hit the same pattern. `initializeCodeGen` registers
every pass, and each iteration returns one object. In 25 iterations it did
not converge. It stopped at 25.35 MB with the corpus still different. The
real saving there is **less than 1.5 MB**, and it only comes with a codegen
pipeline built by hand (§7.2).

## 5. How much "bringing C++ along" weighs

Direct answer: **~2 MB verified (4 %), ~3–4 MB at most.** The C++ parser and
Sema are not "glued" onto the C ones. C and C++ share the same parser and
the same Sema, with `if (getLangOpts().CPlusPlus)` scattered around. What
can go is the C++-only files (templates, `DeclCXX`, lambdas, coroutines,
concepts, exceptions, MS mangling, vtables). Even so, 15 of them come back
because the shared code calls their predicates or constructors.

A real separation (taking the C++ branches out of `SemaExpr`, `SemaDecl`,
`ExprConstant`, `ParseDecl`, `CodeGenModule`...) would be a fork of clang,
and the estimated saving is small next to the cheap cuts. **It is not worth
it.** The large savings, in order, are LLVM's optimizations, the other
targets, modules/PCH and tooling, the extensions (OpenMP/ObjC/...), and the
driver.

## 6. Optimization: -O0 or "the basics"

`bench/b.c` (8 queens ×20, bubble sort on 6000, `fib(30)`, harmonic series
2·10⁷), on V8 (node 24). The numbers are noisy (busy machine), but the
order is stable:

| student code | time |
| --- | ---: |
| -O0 | 280–400 ms |
| -O0 + sroa + simplifycfg + instcombine | ~190 ms |
| -O1 | 170–235 ms |
| -O2 | 110–155 ms |

-O0 is 1.5–2× slower than -O1. For a student program this does not matter,
and V8 (TurboFan) optimizes the wasm anyway. If we want "the basics",
SROA + InstCombine + EarlyCSE + SimplifyCFG (+ GVN/LICM) cost **~1.1 MB**.
mem2reg alone is already in TransformUtils and comes for free. This
requires building the pipeline by hand instead of using the `PassBuilder`
(§7), which is good anyway.

## 7. How we would do it

### 7.1 Build

An `sc` repository with:

1. `llvm-project` as a submodule on our fork: pin + WASI patch + the few
   source patches (today only 1 line in lld). That is what YoWASP did.
2. A two-stage build of LLVM/clang/lld. Native tablegen first, then a cross
   build for `wasm32-wasip1` with wasi-sdk (§2).
3. **The engine's own link**, outside LLVM's CMake. It takes the object
   libraries (`obj.clangSema` etc.), filters out the list of cut objects
   (`build/exp/specs/all.txt` with the returned ones), generates the stubs
   (link with `--allow-undefined` → read the imports → `stubs.s`) and links
   the engine. The script already exists (`exp2.py`). It only needs to
   become the build.
4. Instead of `unreachable`, the stubs call an engine function that reports
   "feature not supported by sc: <symbol>". That way an untested path
   becomes a readable error, not a crash.

Why per object and not `#ifdef` in the source: zero conflicts when LLVM is
rebased. The cuts live in a list of files, verified by a corpus. Source
patches go only where the saving is worth it. The largest of them would
build the sub-Semas on demand (`Sema::OpenMP()` is already an accessor;
today the constructor creates them all), which takes the size from 26.9 to
about 23 MB.

### 7.2 Engine

- `repl_new`/`repl_run` in C++ (wasi-sdk), with the exports of `ENGINE.md`.
- Compilation: `CompilerInvocation::CreateFromArgs` + `CompilerInstance`
  with the student's file in memory (`InMemoryFileSystem` laid over the
  VFS), and with the wasi-libc headers and the ~20 headers of clang's
  resource dir (`stddef.h`, `stdarg.h`, `stdbool.h`...) built in.
  Diagnostics go through our own `DiagnosticConsumer` (no color, with the
  column).
- Code generation without `BackendUtil`/`PassBuilder`: `CodeGenerator`
  (`ModuleBuilder`) gives the `llvm::Module`. We run our passes
  (AlwaysInliner, mem2reg or "the basics") and the `legacy::PassManager`
  with `addPassesToEmitFile` into a `raw_svector_ostream`. This removes the
  `PassBuilder` and the registration of every pass at the root, with no
  stubs.
- Link: `lld::wasm::link` with a reduced `libc.a`. The student's program is
  1 object + libc, and that is fast.
- **Execution**: the student's program is another wasm module. Today the
  host gives the engine no way to ask for a module to be instantiated (the
  same gap as in sjava, `env.wasm_run` in `sjava/PLANO.md`). It needs that
  import. Interruption: an LLVM pass of ours inserts
  `call env.check_interrupt` on loop back edges and at function entry (the
  same plan as sjava's for jwonder).
- REPL: C has no natural REPL. The cheapest option is the sgleam model.
  Accumulate declarations and recompile at each input (0.2 s per
  compilation allows it).

### 7.3 Measured performance (V8, node 24)

| step | time |
| --- | ---: |
| compile the cut `sc.wasm` module | ~0.19 s (46 MB: ~0.45 s) |
| 1st compilation of a program (lazy tier-up) | ~1.0 s |
| later compilations (p1.c, 36 lines, 10 headers) | **~0.16–0.20 s** |
| native clang, same file | ~0.07 s |

## 8. Risks

- **WASI patch outside upstream.** PR #92677 has been open for 2 years and
  YoWASP was archived. We are the ones who rebase it, but it is small and
  touches almost only `Support/Unix`.
- **Per-object cuts depend on the corpus.** A C path the corpus does not
  cover can fall into a stub. Mitigation: a large corpus (c-testsuite, gcc
  torture `execute/`, real submissions from na-programacao), and stubs that
  turn into a readable diagnostic. It has to run at every bump.
- **Size still larger than spython.** 9.8 MB gzip against 7.4 MB. With an
  HTTP cache this is a single download, but it is the largest engine.
- **Memory.** clang uses a deep stack (8 MB reserved) and a few tens of MB
  of heap per compilation. Nothing serious for student programs.

## 9. Reproduction

The project was organized after round 2. The `README.md` describes the
layout and the build (`make`, `make test`, `make learn`). Rounds 1 and 2
used experiment scripts in `build/`. The paths this document cites map as
follows:

| experiment (`build/`) | now |
| --- | --- |
| `src/` (pin + WASI patch) | `llvm/wasi-host.patch`, `llvm/no-veclibs.patch`; pin in the `Makefile` |
| `Toolchain-WASI-LLVM.cmake` | `llvm/toolchain.cmake.in` |
| `proto2/sc_*.cpp`, `checkinv.cpp` | `engine/`, `test/checkinv.cpp` |
| `scripts/exp2.py` + `exp3.py` | `scripts/link.py` (deterministic link; `--learn` redoes the lists) |
| `scripts/corpus2.py`, `corpuslib.py` | `test/corpus.py`, `test/corpuslib.py` |
| `exp/specs/all5.txt`, `sym2.txt`, `o0init.txt` | `cuts/objects.txt`, `cuts/symbols.txt`, `cuts/o0init.txt` |
| `exp/sym2/unwrapped.json` | `cuts/symbols.used` |
| `trim/`, `.a`/`.obj` swapped in `wasm/` | `scripts/trim.py` generates `build/trim/`; `link.py` replaces the members and `wasm/` stays intact |
| `res/base3/` (reference: prototype with `cc1_main`) | `build/res/base/` (reference: `build/sc-full.wasm`, the engine without cuts) |
| `corpus/`, `bench/` | `test/samples/`, `bench/` |

## 10. Round 2: measure what was left and cut more

### 10.1 A real corpus

The 5 programs of round 1 were too few. The corpus now has **7,462 files**:

- `c-testsuite` (220 programs with expected output);
- gcc torture: `execute/` (1,698, linked and run) and `compile/` (2,016);
- clang's own C tests: `Sema`, `Parser`, `CodeGen`, `Lexer`,
  `Preprocessor`, `C/*`, `Misc`, `Frontend`, `AST` (3.5 thousand). They
  exercise almost every C diagnostic.

`scripts/corpus2.py` compiles each file with the engine under wasmtime.
Executable programs are linked and run. The others compare diagnostics
**and the md5 of the object**. A run takes ~3.5 min on 48 cores.
`scripts/corpuslib.py` normalizes addresses, backtraces and line order (the
order of some diagnostics depends on maps keyed by pointer).

With this corpus, the "26.9 MB" build of round 1 failed on 138 files:

- C11 atomics (`LowerAtomicPass`);
- `__builtin_object_size` (`LowerConstantIntrinsics`);
- comments (`RawComment`);
- struct fields (`SemaAccess`);
- blocks/lambdas, among others.

With those 12 objects returned, the honest number of round 1 is **28.55 MB
(10.5 MB gzip)**.

### 10.2 Measuring what was left

Method: `wasm-opt -Oz -g` to keep the names, then `twiggy` for the size of
each function, and the linker map to tie each function to its object
(`scripts/attrib.py`, `scripts/cat2.py`). Of the 22.5 MB of code:

| part | MB |
| --- | ---: |
| C++/ObjC/OpenMP/other targets still called (SemaOpenMP 450 KB alone) | 2.83 |
| optimization passes + `PassBuilder` | 1.95 |
| textual IR, bitcode, AutoUpgrade, verifier | 1.07 |
| debug info (clang and LLVM) | 0.98 |
| non-wasm formats, `CompilerInvocation`, PGO | 1.14 |
| core: SelectionDAG 2.6, C Sema 2.4, IR/Analysis 2.1, CodeGen/MC 1.7, AST 1.3, clang CodeGen 1.3, Lex/Parse 1.2, wasm 0.5, lld 0.3 | ~13.5 |

Data (4.4 MB):

- intrinsic names of every target: 850 KB;
- diagnostic text: 720 KB;
- cc1 option table: 430 KB;
- Unicode names for `\N{}`: 240 KB.

### 10.3 What was done

1. **`sc_main` v2** (`build/proto2/`):
   - `sc_inv.cpp` builds the `CompilerInvocation` in code (~120 lines),
     with no `CreateFromArgs` and no option table.
   - `sc_backend.cpp` replaces `BackendUtil.cpp`. It runs AlwaysInliner and
     then the codegen pipeline in a `legacy::PassManager`. That way the
     `PassBuilder` (which references every pass) leaves the call tree.
   - `checkinv` (native) verifies the equivalence. It compares
     `generateCC1CommandLine()` and the predefined macro buffer of the two
     invocations. The comparison needed ~60 fields whose struct default
     differs from the parser's default (`MathErrno`, `NoCommon`,
     `ShowOptionNames`, `Inlining=OnlyAlwaysInlining`…).
   - Result: **0 differences** on the corpus, including the object bytes.
2. **Per-symbol cuts** (`scripts/exp3.py`):
   - `wasm-ld --wrap=X` redirects `X` to a stub, and `--gc-sections` drops
     the original body. This cuts *inside* the objects that the per-object
     cuts had to return.
   - Nobody needs to pick the symbols one by one. The script runs a
     **discovery**. Each `__wrap_X` records the first call and forwards to
     `__real_X`. One pass of the corpus lists everything in use. The rest
     becomes a stub with `unreachable`, and a second pass validates it.
   - In round 2, 369 of the ~6 thousand candidate functions were in use.
   - Only strong symbols (`T`) qualify. Template instances (`W`) do not
     work with `--wrap` in wasm-ld.
   - The pass initializers that the wasm O0 pipeline does not use become
     no-ops (`scripts/o0passes.py` lists the used ones from
     `llc -O0 -debug-pass=Arguments`). With no registration, the pass's
     constructor and vtable go away.
   - It needs wasm-ld 23. The 21 of wasi-sdk 29 segfaults with `--wrap` +
     `--allow-undefined`.
3. **Tables and data**:
   - `scripts/trim_intrinsics.py` rewrites the generated `IntrinsicImpl.inc`.
     Intrinsic names of other targets become `""`, and the builtin→intrinsic
     tables of other targets go away. IDs and indexed tables do not change.
     The object drops from 925 KB to 246 KB.
   - `TargetLibraryInfo.cpp` under `SC_NO_VECLIBS`. The vectorization
     tables (SVML, SLEEF, ArmPL…) had 145 KB of static initializer.
   - `sc_cuts.cpp` defines the Unicode name functions to return "not
     found". `\N{NAME}` now gives a clean compile error (3 clang tests
     change, on purpose).

### 10.4 Results

| step | raw | gzip |
| --- | ---: | ---: |
| round 1 with the full corpus (per-object cuts) | 28.55 MB | 10.47 MB |
| + sc_main v2 (own invocation and backend) | 26.46 MB | 9.74 MB |
| + wasm-only intrinsics, TLI without veclibs, no Unicode names | 25.24 MB | 9.32 MB |
| + per-symbol cuts (sym1) | 16.92 MB | 6.27 MB |
| **+ non-O0 passes without registration, more objects (sym2)** | **15.75 MB** | **5.81 MB** (zstd 4.56 MB) |
| *spython* | *18.6 MB* | *7.4 MB* |

`sym2` passes on the 7,462 files, and the internal `ld` links and runs p1,
p2 and p5. On V8, the module compiles in 0.11 s. The 1st compilation takes
0.87 s and the later ones 0.14–0.18 s.

What remains (sym1, 14.7 MB of code):

- SelectionDAG 2.6 MB (the FastISel fallback; it cannot go);
- C Sema 1.9 MB;
- IR/Analysis 1.65 MB;
- CodeGen/MC 1.4 MB;
- AST 1.2 MB;
- clang CodeGen 1.1 MB;
- Lex/Parse 1.1 MB;
- C++ residue 0.9 MB, almost all of it template instances (`W`) or code
  that really gets called.

In the data, the largest item is the diagnostic text (720 KB, ~200 KB
gzip). From here on, each MB requires a real change to clang.

### 10.5 A new risk

The stubs trap on paths that the corpus does not cover. With 7.5 thousand
files, including clang's diagnostic tests, the C coverage is good, but it is
not complete. A trap is recoverable. The engine reports "internal error:
feature not supported by sc" and instantiates the module again. For
production:

- add real student submissions to the corpus;
- keep the list of cut symbols with the build;
- make the stub print the symbol's name before the `unreachable`.

### 10.6 Files

See §9. The files of round 2 became `engine/`, `cuts/`, `scripts/link.py`
and `test/`. With the organized build, `make` gives a `build/sc.wasm` of
15,746,919 bytes (659 more than round 2's, because of the order of the
files in the link), and `make test` passes on the 7,462 files.

## 11. What bringing C++ back costs (in the cut engine)

§5 measured C++ against the whole of clang. Here the question is different.
Starting from the 15.75 MB `sc.wasm`, how much larger is an engine that also
compiles C++, cut by the same method (the cuts in `cuts/` learned again with
a corpus that includes C++)?

C++ configuration: gnu++17 (clang's default), `-fno-exceptions`, and the
libc++ and libc++abi of the sysroot. The libc++ of wasi-sdk 29 comes without
exceptions. With the driver's default, `-fcxx-exceptions`, the link fails on
`__cxa_throw`. `sc_inv` picks the language from the extension (`.cpp`,
`.cc`, `.cxx`). `checkinv` gives SAME for C and for C++.

Corpus: the 7,462 C files plus 4,270 C++ ones. Of those, 9 are student
programs in `test/cxx` (iostream, STL, classes, templates, lambdas,
optional/variant, iomanip, typical errors and warnings), linked and run.
The rest are clang's C++ tests (`SemaCXX`, `SemaTemplate`, `CodeGenCXX`,
`CXX/**`, `Parser`, `Lexer`, `AST`, `Misc`), only compiled. `learn`
returned 11 objects (`CGExprCXX`, `CXXInheritance`, `DeclFriend`,
`StmtCXX`, `SemaTemplateDeductionGuide`, `ASTConcept`, `CGCXX`, `CGVTT`,
`VTTBuilder`, `ASTDumper`, `CGHLSLBuiltins`), and the used functions went
from 369 to 1,520.

| engine | raw | gzip | zstd |
| --- | ---: | ---: | ---: |
| `sc.wasm` (C only) | 15,746,919 | 5,811,230 | 4,564,766 |
| C only, with the C++ branch in `sc_inv` | 15,747,508 | 5,810,163 | 4,564,972 |
| **C + C++** | **17,800,572** | **6,624,798** | **5,087,490** |
| difference | **+2.05 MB (+13 %)** | **+0.81 MB (+14 %)** | **+0.52 MB (+11 %)** |

The C + C++ engine passes on the 11,731 files, with no difference from the
whole engine.

Where the bytes come from (code before `wasm-opt`, +2.42 MB): Sema 1.75 MB
(`SemaTemplate*` 1.06 MB, `SemaDeclCXX` 260 KB, `SemaConcept` 150 KB,
`SemaOverload` 91 KB, `SemaExprCXX` 89 KB), AST 345 KB, CodeGen 167 KB and
Parse 140 KB. Almost all of it is templates.

About 200 KB come only from clang's tests. `ASTDumper` and `TextNodeDumper`
(170 KB, for `#pragma clang __debug dump`) and `CGHLSLBuiltins` (33 KB). They
can be cut by putting those tests in `IGNORE`, which brings the cost close
to 1.85 MB.

Limits of the measurement: the coverage is gnu++17. A student who uses
`-std=c++20` (concepts, ranges, `<=>`, coroutines) can fall into stubs, and
the corpus would need C++20 tests. Exceptions stayed out. They would need
the libc++ with exceptions and wasm EH, and `CGException` and
`ItaniumCXXABI` would come back whole.

Reproduction: `build/exp-cxx` (engine sources in `src/`, learned cuts in
`cuts/`), with `CXX=1` in `test/corpus.py` and `SC_CUTS`, `SC_ENGINE`,
`SC_LINKDIR` and `SC_BASE` in `scripts/link.py`.

## 12. C++20 in the engine

C++ stayed in the main engine (`engine/`, `cuts/`, `test/corpus.py` without
`CXX=1`), and the default went from gnu++17 to gnu++20. The flags are the
ones the driver passes with `-std=c++20` for wasm32-wasip1, plus
`-fno-exceptions`: `-std=gnu++20 -fno-implicit-modules -fskip-odr-check-in-gmf`.

`checkinv` gained a field-by-field comparison of the `LangOptions`. It
caught three options that the round trip through the command line does not
show. With C++20, the parser turns on `Modules` (default =
`CPlusPlusModules`) and `ModulesLocalVisibility` (implied by
`-fcxx-modules`), and `-fno-implicit-modules` turns off `ImplicitModules`.
`checkinv` gives SAME for C and for C++.

The corpus gained 5 programs in `test/cxx`: concepts, `requires` and
`consteval` (c10); ranges, views, `span`, `<=>` and designated initializers
(c11); `std::format`, coroutines, `char8_t`, `using enum` and `constinit`
(c12); typical C++20 errors (c13); and warnings (c14). The clang tests stay
the same, now compiled as gnu++20. That makes 11,736 files.

With `Modules` on, clang creates the module reader (`ASTReader`) for every
C++ file, and every one of them fell into a stub. `learn` returned
`ASTReader`, `ASTReaderDecl` and `ModuleManager` whole (+0.67 MB raw). A
function rule in `cuts/symbols.txt` for these objects keeps only what gets
called. `learn` also returned `ExprConcepts` and `SemaCoroutine`.

| engine | raw | gzip | zstd |
| --- | ---: | ---: | ---: |
| C + C++17 (§11) | 17,800,572 | 6,624,798 | 5,087,490 |
| C + C++20, whole `ASTReader` | 18,471,846 | 6,844,449 | 5,252,055 |
| **C + C++20** | **17,993,849** | **6,702,950** | **5,144,933** |

C++20 costs 0.19 MB raw and 0.06 MB zstd more than C++17. Against the
C-only engine, the difference is +2.25 MB raw, +0.89 MB gzip and +0.58 MB
zstd. The used functions (`cuts/symbols.used`) number 1,732. `make test`
passes on the 11,736 files.

`-Wall -Wextra` is already fixed in `sc_inv` (`DO.Warnings`) and applies to
C++ (until §13, which moved the warnings to the flags). The
warnings come from Sema, which the cut keeps, and the output of `c9_warn`
and `c14_warn20` matches the whole engine's: `-Wuninitialized`,
`-Wunused-*`, `-Wreturn-type`, `-Wparentheses`, `-Wsign-compare`,
`-Wstring-plus-int`, `-Wambiguous-reversed-operator`. These are not in
`-Wall -Wextra`: `-Wimplicit-fallthrough`, `-Wconversion` (`int k = 3.7`)
and `-Wshadow`.

Modules (`import std;`) do not work. They would need libc++'s BMIs and the
`ASTWriter`, which is cut.

## 13. All the warnings

Up to here `-Wall -Wextra` was fixed in `sc_inv` and the other `-W` flags
were ignored. Now the warnings come only from the `-W` flags
(`DO.Warnings`), as in clang. With no flag, only the warnings that clang
turns on by default show. The corpus still passes `-Wall -Wextra`, and
`checkinv` gives SAME.

`test/corpus.py` accepts `WFLAGS` (for example `WFLAGS=-Weverything`). With
`-Weverything`, the cut engine of §12 differed from the whole one on 7,378
of the 11,736 files, almost all by trap. `learn` with `-Weverything`
(against a base of the whole engine, also with `-Weverything`) returned
whole objects for four groups of warnings:

| warning | returned objects |
| --- | --- |
| `-Wdocumentation` | `Comment`, `CommentLexer`, `CommentParser`, `CommentSema` |
| `-Wthread-safety` | `ThreadSafetyCommon`, `ThreadSafetyTIL` |
| `-Wunsafe-buffer-usage` | `UnsafeBufferUsage` |
| `-Wlifetime-safety` | 8 objects of `Analysis/LifetimeSafety` |

and about 90 functions, almost all from these groups and from `-Wconsumed`
(`cuts/symbols.used` went from 1,732 to 1,822). The other warnings come
from Sema, and their code was already in the engine.

| engine | raw | gzip | zstd |
| --- | ---: | ---: | ---: |
| C + C++20, only `-Wall -Wextra` (§12) | 17,993,849 | 6,702,950 | 5,144,933 |
| **C + C++20, all the warnings** | **18,267,827** | **6,786,231** | **5,204,882** |

All the warnings cost +0.27 MB raw, +0.08 MB gzip and +0.06 MB zstd. The
corpus passes with and without `-Weverything` (`make test` and
`make test-warn`, 0 differences on the 11,736 files). `make learn` now runs
with `-Weverything`, so that a new cut does not remove the code of any
warning.

Which warnings to turn on by default for students is left for later. The
engine turns on none beyond clang's.

## 14. The corpus links with the engine itself

The corpus linked the programs with wasi-sdk's native wasm-ld, so the `ld`
of the cut engine never ran in the test. When it ran, `sc.wasm ld` trapped
when linking any C++ program. wasm-ld calls the demangler, and
`LLVMDemangle` is cut (`cuts/symbols.txt`). The C programs linked.

`test/corpus.py` now links with `sc.wasm ld`. With the whole engine, the
result is the same as native wasm-ld's on the 239 programs, and `learn`
returned 4 demangler functions (`cuts/symbols.used` went from 1,821 to
1,825).

The corpus also started writing the object to a file (`-o /o/x.o`) instead
of `-o -`, and that found a second bug, this time in `llvm/wasi-host.patch`.
On WASI, `Process::SafelyCloseFileDescriptor` ignored the return of `close`
and returned `errno`. When the file had an `#include`, `errno` still held
the `EINVAL` of a `readlink`, and `cc1` failed with "IO failure on output
stream: Invalid argument", leaving a `.o.tmp` behind. With `-o -` the error
did not show. The patch now returns `errno` only when `close` fails.

| engine | raw | gzip | zstd |
| --- | ---: | ---: | ---: |
| C + C++20, all the warnings (§13) | 18,267,827 | 6,786,231 | 5,204,882 |
| **+ demangler for `ld`** | **18,270,532** | **6,787,704** | **5,205,722** |

`make test` and `make test-warn` give 0 differences on the 11,736 files,
and `checkinv` gives SAME.

`learn` did not finish in 2 hours. The discovery did not see the
`TextNodeDumper` functions that `#pragma clang __debug dump` calls, and the
loop returned them one per iteration, running the whole corpus each time.
The `SCWRAP n` mark that each wrapper writes to stderr fell in the middle of
a line of the dump (`FunctionDeclSCWRAP 3998`), and the regex, anchored at
the start of the line, did not see it. Now the mark starts with a newline.
The discovery finds the 1,825 functions, the verification loop closes on
the first round, and `symbols.used` comes out the same as before.

Each corpus run still took ~13 min, with the machine almost idle. Only 0 to
6 wasmtime processes were alive at a time, for 48 threads. Sampling the
threads of `corpus.py` showed 86% of the time in the `glob` that looks for
the `.o.tmp` of a failed write. It lists the output directory, which
collects one `.txt` per file (11,737 at the end), and runs under the GIL.
The objects and programs now go to an `o/` subdirectory, which holds only
the ones in use.

| | before | after |
| --- | ---: | ---: |
| one corpus run | ~13 min | 2.5–3.5 min |
| `make learn` | > 2 h (57 min with the mark alone) | 11.4 min |

wasmtime itself costs little. 480 compilations of a trivial file, 48 at a
time, take 1 s.

## 15. Shared objects (PIC) for the REPL

The REPL will compile each input as a shared object, loaded on top of the
earlier ones (`-fPIC -fvisibility=default`, `wasm-ld -shared
--experimental-pic`). The corpus only compiled with `-mrelocation-model static`, so
nothing ensured that the cuts kept the PIC code in the backend and `-shared`
in `ld`.

`sc_inv.cpp` now accepts `-mrelocation-model pic -pic-level 2 -target-feature
+mutable-globals -fvisibility=default`, the flags that the driver passes for
`-fPIC -fvisibility=default`. With PIC the default of
`-fdirect-access-external-data` changes to false. `checkinv` compares the
four combinations (C and C++, static and PIC) and gives SAME.

The corpus gained a `pic-` slice. The 1,937 programs that are linked and
run are also compiled with PIC and linked with `sc.wasm ld -shared
--experimental-pic --unresolved-symbols=import-dynamic` against wasi-sdk's
`libc.so` (and `libc++.so`). The result is the diagnostics and the md5 of
the object and of the `.so`. They do not run yet. That is left for the
loader.

The cut engine gave 0 differences on the 13,673 files, with and without
`-Weverything`, with no change to the cuts. The code for PIC and for
`-shared` was all there already. The size changes only because of
`sc_inv.cpp`:

| engine | raw | gzip | zstd |
| --- | ---: | ---: | ---: |
| §14 | 18,270,532 | 6,787,704 | 5,205,722 |
| **+ PIC in `sc_inv`** | **18,270,869** | **6,787,594** | **5,205,596** |

A separate finding: `test/cxx/c12_format.cpp` also fails in the static
compile, with "Do not know how to promote this operator!". `sc_backend.cpp`
does not run the coroutine passes (CoroEarly, CoroSplit, CoroCleanup) that
LLVM's O0 pipeline runs, and the `llvm.coro.*` intrinsics reach the backend.
The corpus did not catch it because the whole engine, which is the base,
uses the same backend.
## 16. Coroutines in the backend

LLVM's O0 pipeline (`buildO0DefaultPipeline`) ends with
`buildCoroWrapper`. It holds CoroEarly, CoroSplit (a CGSCC pass),
CoroCleanup and GlobalDCE, which run only if the module declares
`llvm.coro.*` intrinsics (`CoroConditionalWrapper`). `sc_backend.cpp` now
runs these passes in the new PassManager, between the AlwaysInliner and the
codegen, which stay in the legacy one. The analysis managers are built by
hand, with only what these passes ask for (TTI, TargetLibraryInfo, ORE,
AssumptionCache, DominatorTree, LazyCallGraph and the proxies). There is no
`PassBuilder`, which would bring in every pass.

Against the old base, only 23 files change, and all of them have coroutines
(`cxx-c12_format`, its `pic-` and 21 clang tests, such as `coreturn` and
`co_await-ast`). Before, they stopped with "Do not know how to promote this
operator!". Now they compile, and `c12_format` runs correctly. The other
13,650 stay identical, so moving the AlwaysInliner into a PassManager of its
own changes no object.

`make learn` brought back 8 objects (CoroEarly, CoroCleanup, CoroFrame,
SuspendCrossingInfo, MaterializationUtils, SpillUtils, SimplifyCFGPass,
GlobalDCE) and 40 functions. After that, 0 differences with and without
`-Weverything`.

| engine | raw | gzip | zstd |
| --- | ---: | ---: | ---: |
| §15 | 18,270,869 | 6,787,594 | 5,205,596 |
| **+ coroutines** | **18,471,730** | **6,867,480** | **5,268,583** |

That is +201 KB raw and +63 KB zstd, for a part of C++20 that used to break.

## 17. `import std` and the `.pcm`

Processing `<iostream>` takes almost all the time of a compilation. On V8,
a hello takes 2.8 s, against 21 ms for an empty `main`. `<iostream>` is 841
headers, 82 thousand preprocessed lines in C++20, and the time goes to parse
and Sema (with `-fsyntax-only` it is the same). I compared the options on the
pin's native clang with the libc++ 21 headers from wasi-sdk:

| form | raw | zstd | hello | 5 headers |
| --- | ---: | ---: | ---: | ---: |
| plain headers | 4.9 MB | 0.53 MB | 1.98 s | 2.18 s |
| PCH of `<iostream>` | 17.2 MB | 8.2 MB | 0.12 s | — |
| PCH of the 5 headers | 17.8 MB | 8.6 MB | — | 0.31 s |
| `import std;` (`std.pcm`) | 28.4 MB | 13.4 MB | 0.08 s | 0.31 s |
| Clang modules (cache) | 44.9 MB | 19.9 MB | 0.13 s | 0.37 s |
| snapshot with Wizer (estimated) | ~64 MB | ~14.3 MB | ~0 | ~0 |

The PCH and the snapshot come in before the first line of the file. A
`#define` before the `#include` is ignored without a warning, and whatever
the prelude includes stays visible. A module is compiled in isolation, so
this does not happen, and libc++ splits `std` into submodules (`std::map`
without `<map>` gives "declaration of 'map' must be imported from module
'std.map'"). `wasm-ctor-eval` does not work for the snapshot. It stops at
the first `memory.grow` and does not call WASI. `wasmtime wizer` works.

I chose `import std;`. It is what the student learns first, and the engine
itself generates the `.pcm` on the first run and keeps it in a cache. The
`std.cppm` from wasi-sdk does not compile for wasip1 as it is. `<csetjmp>`
and `<csignal>` give `#error` in the WASI libc, so the corpus removes those
two lines.

`sc_inv.cpp` accepts `-emit-module-interface -fmodules-reduced-bmi`, `-I` and
`-fmodule-file=<name>=<pcm>`, and `checkinv` compares the three new
combinations (SAME). The driver's reduced BMI only applies with
`-fmodule-output`. With `--precompile` the `.pcm` is the complete one, and
the reduced one would be almost the same size (27.8 MB). The corpus
generates `std.pcm` with the engine under test (`std-pcm`; the md5 is
stable between the whole engine and the cut one) and compiles the programs
in `test/mod`, which import `std`, also as PIC with the same `.pcm`.

`make learn` brought back 9 objects from `clangSerialization` (the
`ASTWriter` and the rest of the `ASTReader`; `ASTReader.cpp` and `ASTReaderDecl.cpp` were already kept) and 352 functions. After that, 0
differences in the 13,680 files, with and without `-Weverything`.

| engine | raw | gzip | zstd |
| --- | ---: | ---: | ---: |
| §16 | 18,471,730 | 6,867,480 | 5,268,583 |
| **+ `import std`** | **19,293,131** | **7,106,557** | **5,452,461** |

On V8, with the cut engine:

| | time |
| --- | ---: |
| generate `std.pcm` (once) | 6.8 s |
| hello with `import std;` | 0.14 s |
| 5 headers with `import std;` | 0.48 s |
| hello with `#include <iostream>` | 2.8 s |

That is +184 KB zstd in the engine, and the `.pcm` (28 MB) is not
downloaded. The browser generates it.

## 18. The REPL loader

The REPL will load each input as a shared object (§15) on top of the
previous ones. The loader prototype is in `loader/` (Rust and wasmtime,
`build/loader/release/scload`). It runs the `.so` of each program in the
corpus and sequences of inputs in the style of the REPL.

**Design.**

- **No main module.** The plan was to link libc and libc++ into a main
  module that exported everything. That was not needed. The loader loads
  `libc.so`, `libc++abi.so` and `libc++.so` from wasi-sdk like any other
  `.so`, in the order of `needed` in `dylink.0`.
- **Shared resources.** The host creates the memory, the function table and
  `__stack_pointer`, and every module imports them.
- **Memory of the modules.**
  - The stack sits at the start, as with `--stack-first`, and an overflow
    traps.
  - The first group of modules (the libraries and the program) sits right
    after the stack, and the libc heap (`__heap_base`) starts where that
    group ends.
  - The later inputs get memory from libc's `aligned_alloc`.
- **Symbol binding.**
  - The `GOT.mem` and `GOT.func` imports are mutable globals, filled in
    once the whole group has been instantiated. This is necessary because
    `libc++abi.so` and `libc++.so` depend on each other.
  - An imported function is bound directly to the definition already
    loaded. If there is no definition yet, the call goes through a host
    trampoline, which is fixed at the end of the group. This happens, for
    example, when libc calls one of its own symbols that can be replaced,
    such as `lseek`.
- **WASI.** The wasmtime WASI reads memory through the `memory` export of
  the caller, and no `.so` exports its memory. The loader generates a bridge
  module that imports the memory, exports it and forwards the calls.
- **Execution.** At the end, the loader calls `main` (through libc's
  `__main_void`, if the program takes `argc` and `argv`) and libc's `exit`,
  which flushes `stdio`.

**The rule for redefinitions.**

- A new definition in an input hides the old one from the inputs loaded
  later. Whatever was already bound keeps the old one. A pointer to the old
  `f` keeps calling the old one, and the old `f` keeps reading the old `x`.
- Inline functions, template instances, vtables and typeinfo come in each
  input that uses them. For them, the first definition must hold in every
  input. Otherwise a `static` inside an inline function gets duplicated and
  `dynamic_cast` between inputs fails.
- `wasm-ld` does not tell the loader which definitions are weak, because
  the `dylink.0` section only lists the TLS exports. `llvm/dylink-weak.patch`
  adds the weak exports, with the `BINDING_WEAK` flag. A cut does not solve
  this. Emscripten only looks at the TLS flag in this list, so the change
  does not affect it. The engine grows by 41 bytes: 19,293,172 raw,
  7,106,613 gzip, 5,452,450 zstd.
- Weak functions already worked without the patch, because `wasm-ld` makes
  the module itself import them and the loader binds the import to the first
  definition. Weak data (typeinfo, local `static`s) goes through the GOT,
  and the patch exists for that data.

**The `.so` carries compiler-rt.** The static link includes
`libclang_rt.builtins.a`, and `-shared` did not. `long double` and
`__int128` called `__netf2`, `__multi3` and other functions that were in no
`.so`. Now the corpus passes `libclang_rt.builtins.a` to `ld
-shared`, and each `.so` carries the functions it needs.

**Results.**

- The 1,743 programs that run statically give the same result as a `.so`.
  The output and exit code are equal, or both trap. The only difference is
  the stack. The loader uses 1 MiB, against 64 KiB from `wasm-ld`. With
  `--stack 65536`, the 8 programs that overflow the stack statically also
  trap in the loader, and `return-addr` prints the same addresses.
- The corpus now runs the `pic-` files through the loader, and `test/repl`
  has two sequences of inputs (`repl-c`, `repl-x`):
  - C: redefinition of a function and of a global, a function pointer
    across inputs, the heap across inputs;
  - C++: a `std::vector<std::string>` alive across inputs, a shared
    `static` of an inline function, `dynamic_cast` across inputs, and an
    input with `import std;` on top of inputs compiled with `#include`.

Loader times (native, 48 cores, wasmtime cache):

| program | cold | with cache |
| --- | ---: | ---: |
| C (`libc.so`) | 0.15 s | 0.03 s |
| C++ (`libc++.so`) | 0.45 s (4.1 s with 1 core) | 0.10 s |

The libraries the REPL needs at hand:

| | raw | zstd |
| --- | ---: | ---: |
| `libc.so` | 1,906,118 | 645,773 |
| `libc++abi.so` | 1,074,125 | 230,638 |
| `libc++.so` | 5,314,463 | 1,111,133 |

On the web, the browser will download the `.so` files (2 MB zstd in total),
and the loader will be rewritten in JS with `WebAssembly.instantiate`,
following the same logic.

## 19. The native engine

The native REPL will run the compiler as native code and only the student's
code in wasm. For this reason the same engine is now also linked for x86-64
Linux (`build/sc`, `make native`), with the same cuts. It still generates
wasm32-wasip1.

**Build.**

- The host LLVM lives in `build/host`. It is compiled from the same patched
  sources (`build/src`), with the options of the wasm build: `MinSizeRel`,
  no PIC and no threads (as in wasm). The `tblgen` tools come from
  `build/native`.
- `build/native` does not work for this. It exists only for the tools, and
  here it was configured from another tree, without our patches.
- `scripts/link.py --native` links the engine with `ld.lld`. The rules in
  `cuts/` are the same. The `.cpp.o` members are compared as `.cpp.obj`.
  - A cut function becomes a stub that prints its name
    (`sc: cut function called: ...`) and calls `abort`.
  - A cut datum becomes zeros. Before, the stub of a datum was code, and the
    `cl::opt` `EnableSingleByteCoverage`, read as true, changed the code
    generated for the `?:` operator. In `sc.wasm`, `wasm-ld` leaves an
    undefined datum at address 0, where memory is zero.
- The `--icf=safe` of `ld.lld` merges identical functions whose address is
  not used. This removes another 11%.

**The functions the corpus uses.**

- `cuts/symbols.used` was learned in wasm. The native build matches the
  demangled names, with two ABI differences:
  - `int64_t` is `long long` in wasm32 and `long` in x86-64;
  - `std::` is `std::__2` (wasi-sdk's libc++) in wasm and `std::__cxx11`
    (libstdc++), with `[abi:cxx11]`, in the native build.
- That leaves 14 functions that only the native build calls
  (`cuts/symbols.host.used`, `make learn-native`). `-Os` inlined most of
  them in wasm and not in x86-64, like the `TypeLocReader::Visit*` of the
  `.pcm`. One changes name. `SmallVector<Literal, N>` has `N` 13 in wasm and
  12 in x86-64, because the default `N` depends on the pointer size.
- `link.py --learn` now runs again only the files that differ, until they
  pass, and then the whole corpus. `make learn-native` took 20 min.

**Native against wasm, without cuts.** The two differ in 45 of the 13,682
files, all because of the host:

- **The stack.** In deep recursion (`instantiation-depth`, `limits-*`,
  `deep_recursion`), the wasm stack and the 8 MB of the native one overflow
  at different points, or only one of them overflows.
- **Memory.** One file gives "Out of memory" in wasm32 and compiles in the
  native build.
- **The sandbox.** An `#include "../x"` outside the mapped folder gives
  "Operation not permitted" in wasmtime and "file not found" in bwrap.
- **`std.pcm`.** The bytes differ (224 bytes fewer in the native one), but
  the programs with `import std;` give the same result, and `sc.wasm` uses
  the `.pcm` generated by the native build.

Apart from that, the objects and the linked `.wasm` files are identical
byte for byte. So `make test-native` compares `build/sc` with the uncut
native build (`build/sc-host-full`): 13,682 files, 0 differences.
`corpus.py` runs the native build in `bwrap`, with the same folders that
wasmtime maps.

**Size** (without symbols):

| | raw | gzip | zstd |
| --- | ---: | ---: | ---: |
| native without cuts | 51,748,552 | | 15,776,120 |
| native cut (`build/sc`) | 24,545,480 | 9,950,566 | 7,637,727 |
| `sc.wasm` | 19,293,172 | 7,106,613 | 5,452,450 |

The cuts remove 53% of the native build, almost the same fraction they
remove from wasm.

**Time** (the lowest of three, on a machine with load 10 to 20, wasmtime
with the engine precompiled):

| | native | wasmtime | ratio |
| --- | ---: | ---: | ---: |
| small C (`p1.c`) | 0.038 s | 0.071 s | 1.9 |
| `#include <iostream>` | 1.88 s | 2.87 s | 1.5 |
| `c3_stl.cpp` | 2.55 s | 4.26 s | 1.7 |
| C++ `ld` | 0.094 s | 0.137 s | 1.5 |
| compile `sc.wasm` for wasmtime, once | | 2.8 s (29 s with 1 core) | |

In the native REPL the biggest saving is not the time of each compilation.
It is not having to compile the engine on the first run, and not needing
Cranelift for that. The host only needs a runtime for the student's code.
The native build is `-Os`, like wasm. An `-O2` build would be faster and
bigger. That has not been measured yet.

## 20. Exceptions, `setjmp` and the newest standards

The REPL will need exceptions in C++ (a requirement from the teacher). C's
`setjmp`/`longjmp` uses the same wasm mechanism, so it came in too. In the
same round, the defaults became the newest standards: **gnu23** for C and
**gnu++26** for C++ (`docs/plan-repl.md` §1.5 and §1.6).

**The form of the exceptions.**

- The form is the standardized one (exnref, `try_table`). The pin's LLVM
  still generates the legacy one by default (`-wasm-use-legacy-eh`,
  `cl::init(true)`).
- wasmtime 48 refuses the legacy form with "legacy_exceptions feature
  required for try instruction". The standardized form runs on wasmtime 48
  and node 24.
- The flags are those of the driver with `-fwasm-exceptions` and `-mllvm
  -wasm-enable-sjlj`, plus `-mllvm -wasm-use-legacy-eh=false`:
  - target features `+exception-handling`, `+multivalue`, `+reference-types`;
  - `-exception-model=wasm`;
  - `-fcxx-exceptions -fexceptions` and `-mllvm -wasm-enable-eh` in C++.
- sc does not have the `-mllvm` parser. `sc_inv` puts the values in
  `FrontendOptions::LLVMArgs`, which `checkinv` compares, and
  `scSetLLVMArgs` applies them directly to the registered `cl::opt`.
- The backend (`sc_backend.cpp`) now uses `ExceptionHandling::Wasm`.
- `checkinv` gained the gnu17 case and gives SAME in all 8 cases. It caught
  a difference. From C++23 on, the default of `CheckConstexprFunctionBodies`
  is false.

**The sysroot** (`make sysroot`, `build/sysroot`).

- The libc++ of wasi-sdk 29 comes without exceptions. libc++, libc++abi and
  libunwind are now compiled from the pin (`build/src`), with the pin's
  clang (`build/native`) and `-fwasm-exceptions`.
  - The options follow wasi-sdk's `__config_site`: ABI 2, musl, pthread,
    `fast` hardening.
  - wasi-sdk's libc stays, including `libsetjmp`.
- The pin gained the whole `libcxx`, `libcxxabi`, `libunwind` and `runtimes`
  trees. The pin's libc++ also needs some headers from llvm-libc
  (`libc/shared`, `libc/src/__support`…), for the floating point
  `from_chars`.
- Three build details:
  - The pin's driver passes `-fvisibility=hidden` by default on wasm, and
    libunwind exported nothing. It is compiled with
    `-fvisibility=default`. On wasm, its `__declspec(dllexport)` requires
    `-fdeclspec`.
  - On wasm, libunwind calls `__gxx_personality_wasm0` (libc++abi), and
    libc++abi calls `_Unwind_RaiseException` (libunwind). A separate `.so`
    for each does not link with `-z defs`. So libunwind goes inside
    libc++abi, in the `.a` and in the `.so`, and the corpus still links
    with `-lc++ -lc++abi`.
  - `LIBCXXABI_SILENT_TERMINATE` stays off (wasi-sdk turns it on), so that
    the student sees the message from `terminate`.
- Sizes: `libc++.so` 788 KB and `libc++abi.so` 129 KB, without debug
  information. The wasi-sdk ones have 5.3 MB and 1.1 MB, with it.

**The loader.**

- `libc++abi.so` defines and exports the `__cpp_exception` tag.
  `libsetjmp.so` imports `__c_longjmp`. The loader keeps tags as symbols.
  The first definition holds for every module, and the loader creates a tag
  that a module imports before anyone defines it. In this way the exception
  and `longjmp` cross modules (`repl-e`, `repl-j`).
- **Uncaught exception.** In wasm there is no search phase. libc++abi never
  calls `std::terminate`, and the exception goes out to the host ("thrown
  Wasm exception") without saying which one it was. The loader does what
  libc++abi does natively. It takes the pending exnref with the
  `__cpp_exception` tag, reads the pointer (field 0), and calls
  `__cxa_begin_catch` and `std::terminate`. The output becomes "libc++abi:
  terminating due to uncaught exception of type std::out_of_range: vector".
  The statically linked program, run by wasmtime, still prints only
  "thrown Wasm exception".

**The standards.**

- C: gnu23. C++: gnu++26. `sc_inv` accepts `-std=` only for the corpus.
- C23 rejects K&R definitions and implicit `int`, and the c-testsuite and
  the gcc torture tests would stop in the frontend without reaching the
  backend. Those slices stay in gnu17 (`gnu17()` in `corpus.py`). The rest
  (programs, REPL, clang tests) uses the new default.
- `views::enumerate` does not exist yet in the pin's libc++. `std::print`,
  `std::expected`, `ranges::to`, deducing this, pack indexing, `=
  delete("…")` and `_` work (`c18_cxx23`).

**The corpus.**

- New programs:
  - exceptions (`c15`): by value and by reference, hierarchy, the library
    ones, RAII during unwinding, rethrow, function-try-block,
    `exception_ptr`, `throw_with_nested`;
  - `terminate` in `noexcept` (`c16`) and an uncaught exception (`c17`);
  - C++23/26 (`c18`), C23 (`p7`), `setjmp` in C (`p6`), `import std` with
    exceptions (`m4`);
  - REPL: an exception thrown in one input and caught in another (`repl-e`),
    and a `longjmp` from one input to the `setjmp` of another (`repl-j`).
- `std.pcm` now has `<csetjmp>` (only `<csignal>` is left out).
- There are 13,698 files. 7,940 changed their result against the previous
  base. The objects change with the new features, and C++ uses the new
  libc++.
  - The pin now compiles the gcc torture tests with `setjmp` (`pr56982`,
    `20210505-1`).
  - Two clang tests started to crash the compiler. One is a computed `goto`
    (`ast-dump-APValue-addrlabeldiff`, now with C23's `constexpr`) and one
    is a recursion that overflows the stack (`cxx2b-deducing-this`). They
    are the same classes as before, and the `const-init*` tests stopped
    crashing.

**The cuts.**

- `make learn` took 12 min. It brought back 46 functions
  (`cuts/symbols.used` went from 2,218 to 2,261). They are the EH of CodeGen
  and Sema (`EmitCXXTryStmt`, `EmitCXXThrowExpr`, `getEHDispatchBlock`,
  `handlerCanCatch`, the exception specs), `WinEHPrepare` (which wasm EH
  uses) and `ASTStructuralEquivalence`, because C23 accepts redeclaring an
  identical struct. The object cuts did not change.

| `sc.wasm` | raw | gzip | zstd |
| --- | ---: | ---: | ---: |
| C gnu17 + C++20 without exceptions (§19) | 19,293,172 | 7,106,613 | 5,452,450 |
| **C gnu23 + C++26 with exceptions** | **19,388,589** | **7,141,165** | **5,479,984** |

The exceptions and the new standards cost 95 KB raw and 27.5 KB zstd
(+0.5%). `make test` and `make test-warn` pass on the 13,698 files.
`checkinv` gives SAME.

**The native build.** `make learn-native` took 8 min, and
`cuts/symbols.host.used` went from 14 to 17 functions. `build/sc` has
24,633,560 bytes (+88 KB) and 7,668,404 zstd (+31 KB). `make test-native`
passes on the 13,698 files with one exception. `Sema/c2x-fallthrough` went
into `IGNORE`. In C23, the order of its `[[fallthrough]]` errors changes from
one run to the next in the native build (ASLR changes the order of a map
keyed by pointer), and with it the source lines that appear change.

**Time.** `c3_stl.cpp` compiles in 2.72 s natively, against 2.55 s in
§19, with the same load. That is 7% more, which comes from the new libc++
in gnu++26 and from the EH code.

**Pending:** `learn` has to be redone after pin 23.1.3 (phase P).

## 21. The pin moves to 23.1.3

The pin moved from the `main` of April 8 (`5fe235b9862f`) to the tag
`llvmorg-23.1.3` (`0d261d1ca552`), before the REPL work
(`docs/plan-repl.md`, phase P). 23.1.3 brings three clang-repl fixes that
the REPL needs. They are the vtable duplication (#185648), the `Undo` that
turns off the removed PTU (#213235, backport `ea5cc83`) and the earlier
declarations kept when an input fails (#218149).

**The patches.**

- `wasi-host.patch`: three hunks no longer applied and were redone by hand
  (`clang/Support/Compiler.h`, the includes of `CrashRecoveryContext.cpp`,
  `lockFile`/`unlockFile` in `Unix/Path.inc`). 23.1.3 brought
  `mapped_file_region::randomAccessImpl`, which calls `posix_madvise`, and
  wasi-libc does not have that function. It gained the same
  `defined(__wasi__)` as its neighbours.
- `libcxx-locale.patch` (new): in 23.x, libc++'s `locale_base_api.h`
  includes `locale_base_api/ibm.h` on every platform without its own
  support, and not only on z/OS. On wasm, clang defines `_GNU_SOURCE` in
  C++, wasi-libc's `stdlib.h` declares `strtod_l`, and `ibm.h` redefines it
  with `abi_tag`. The libc++ build stops, and any `<iostream>` of the
  student would stop as well. The patch restores the old
  `#if defined(__MVS__)`. LLVM's `main` is still like this.
- `no-veclibs.patch` applied cleanly. `dylink-weak.patch` only changed
  context.

**`sc_inv`.** `LangOptions::EnableLifetimeSafety` is gone (the analysis now
follows `-Wlifetime-safety`). `DiagnosticOptions::ShowColors` became an
enum, and cc1 now leaves it at `Auto`, resolved against the stream at print
time. Before, the parser already resolved it to false. `sc_inv` no longer
touches either of them. `checkinv` gives SAME in all 8 cases. In a terminal,
the native build may now color the diagnostics. In the corpus the output is
redirected and nothing changes.

**The corpus and the cuts.**

- There are 13,819 files (there were 13,698). They include the clang tests
  of the new pin.
- `make learn` took 26 min, counting the base and the build of `scload`.
  `cuts/symbols.used` went from 2,261 to 2,281 functions. Most of the
  difference is renaming (`UnsignedOrNone` became `OptionalUnsigned`, the
  `LateParsed*` classes moved out of `Parser`). What is new is the C++26
  *expansion statements* (`CXXExpansionStmt*`) and
  `mayVTableBeDuplicated` (#185648). In the object cuts, `Facts.cpp` from
  `LifetimeSafety` came back.
- `make learn-native` took 12 min and brought back one function
  (`TypeLocReader::VisitDependentBitIntTypeLoc`, 17 to 18).
- `libc++.so` went from 788 KB to 856 KB (`<text_encoding>` came in).
  `libc++abi.so` has 130 KB.

| | raw | gzip | zstd |
| --- | ---: | ---: | ---: |
| `sc.wasm`, April pin (§20) | 19,388,589 | 7,141,165 | 5,479,984 |
| **`sc.wasm`, 23.1.3** | **19,660,880** | **7,246,200** | **5,552,971** |
| `build/sc`, April pin (§20) | 24,633,560 | | 7,668,404 |
| **`build/sc`, 23.1.3** | **25,122,800** | **10,195,747** | **7,803,852** |

The new pin costs 272 KB raw and 73 KB zstd in `sc.wasm` (+1.3%), and 135 KB
zstd in the native build (+1.8%). `make test`, `make test-warn` and
`make test-native` pass on the 13,819 files with no difference. `checkinv`
gives SAME.

## 22. REPL phase 0: a stateful session against chained PCH

Phase 0 of `docs/plan-repl.md` (§1.3) compares two ways to compile the
cells of the REPL, each carried all the way to the loader. The spike does
not go into the usual engines. `make spike` links `build/sc-spike`, which
is the uncut native engine with `engine/sc_session.cpp` and
`engine/sc_spike.cpp`. The spike code sits behind `SC_SPIKE`. `sc.wasm`,
`build/sc`, `sc-full.wasm` and `sc-host-full` come out identical byte for
byte.

- **A, session.** One live process with a copy of the core of the
  23.1.3 `IncrementalParser` without `clangInterpreter`. It has a plain
  `CodeGenerator` and the backend of `sc_backend.cpp` over the module of
  each cell. The cells arrive through a pipe.
- **B, stateless.** Each cell is a process. The cell becomes a PCH chained
  onto the previous one (`-fpch-codegen`), and the cell's object comes out
  of that PCH.

**How it was measured (`test/spike.py`).**

- The 29 programs in `test/samples` (7 in C), `test/cxx` (18) and `test/mod`
  (4, with `import std;`) were cut into 222 cells, one per top-level
  declaration.
- Each cell is linked as a `.so`, and `scload` runs them all.
- The output has to be that of the `pic-` of the native base, normalized as
  in the corpus.
- In the crash test, a fatal cell (`#pragma clang __debug crash`) goes into
  the middle of the program. The session dies, another one is created, and
  the previous cells are replayed without the backend. The names each cell
  exports have to be the same as the first time, and the following cells
  have to give the same output.

| | A | B |
| --- | ---: | ---: |
| programs with the right output | **29/29** | 26/29 (`import std` fails) |
| crash, replay and next cell | **23/23** | stateless |
| 1st cell, C++ (median) | **0.91 s** | 1.49 s |
| following cells, C++: median, p90, max. | **8 ms, 0.40 s, 3.0 s** | 164 ms, 0.59 s, 4.4 s |
| following cells, C: median | **3 ms** | 22 ms |
| sum of the cells, C++ (the whole program at once takes 39.1 s) | **40.5 s** | 78.4 s |
| peak memory | 283 MB (the session) | 278 MB (one cell) |

**Long session (300 small cells).**

| | A | B |
| --- | ---: | ---: |
| C++, cells 1–50 | 3.0 ms | 122 ms |
| C++, cells 250–300 | 2.6 ms | 285 ms |
| C, cells 1–50 | 1.6 ms | 21 ms |
| C, cells 250–300 | 2.5 ms | 51 ms |
| memory, C++ | 101 MB (session peak) | 159 MB (per cell) and 28 MB of PCH on disk |

In B, each cell loads the whole chain of PCHs, so the cost grows with the
session. In A it stays constant. In A, the sum of the cells costs the same
as compiling the whole program. The session adds nothing.

**What B needed, and why it is not simpler.**

- **Re-emission.** The object of a cell carries again the variables and
  the tentative definitions of the previous cells, with the dynamic
  initializers. This is the defect from §1.3. The copies become
  declarations through the same IR post-pass as in A.
- **`static`.** The problem is the same as in A (§3.2), with the same
  post-pass.
- **libc++ inline functions.** `-fpch-codegen` generates the inline
  functions of a header only in the object of the cell that included the
  header. libc++ marks almost all of them as hidden
  (`_LIBCPP_HIDE_FROM_ABI`), and the following cells did not find them. The
  spike exports them (hidden becomes default).
- **`import std;`** The `import` of a cell does not stay visible in the
  following ones through the chained PCH ("declaration of 'string' must be
  imported").

The saving of B would be to have no state, so no replay and no
`CleanUpPTU`. B needs the IR post-pass, which is the delicate part, just the
same.

**What the spike showed about A.**

- **The post-pass of §3.2.** It works for:
  - a `static` with a dynamic initializer;
  - a local `static` of a `static` function;
  - an anonymous namespace, with a vtable;
  - a template `static inline`;
  - a global with a dynamic initializer used in later cells.

  One defect found is that the promoted global cannot stay `dso_local`.
  wasm-ld does not put an exported `dso_local` function in the table of the
  `.so`, and a call through a pointer (the `qsort` of p1, the
  `std::function` of c4) gave "out of bounds table access".
- **Error and redefinition.**
  - A cell with an error followed by a good cell works.
  - An `#include` after a cell with an error works.
  - A redefinition is rejected, as decided (§3.4).
- **A top-level `return`** brings the session down (the open bug of §3.7).
  It recovers through the replay. Phase 2 needs the diagnostic before
  CodeGen.
- **Where the code comes out.** CodeGen defers emission. The code of a
  function comes out in the object of the first cell that uses it, usually
  the one with `main`. The declaration cells come out almost empty, and the
  cost goes to the cell that uses the code.

**Decision: A.** It is faster from the first cell, constant in a long
session, and accepts `import std;`. The state that B would avoid (replay,
`CleanUpPTU`) already worked in the spike. `engine/sc_session.cpp` is the
base of phase 2. `sc_spike.cpp` and `test/spike.py` stay until the `split-`
and `crash-` slices of phase 3 replace them.

## 23. REPL phase 1: the C API and the safe Rust API

Date: 2026-10-13.

**What came in.**

- `engine/sc.h`, the C API from the plan (§2), with four differences from
  the draft:
  - `sc_session_new` returns the status and always hands over the session,
    so that the diagnostics of a session that did not start can be read;
  - `sc_session_free`;
  - the `codegen` of the cell became `mode` (`SC_CELL_SO` or
    `SC_CELL_REPLAY`);
  - the wasm-ld arguments (`-L`, the libraries, the builtins) come after
    `--`. The engine does not need to know where libc and the builtins are.
- `main` became `sc_main`, and `engine/sc_entry.cpp` is the `main` of the
  executables (`sc.wasm`, `build/sc`, the uncut engines).
- `engine/sc_api.cpp`, the API session on top of the spike's `ScSession`:
  - the diagnostics go into a string;
  - `lldMain` links the object of each cell in a temporary directory of the
    session, and the `.so` is read back (§3.1 of the plan);
  - a link failure after codegen gives `SC_LOST`, and so does `undo`
    (§3.3).
- The session goes only into the uncut native engine that Rust links.
  `sc.wasm` and `build/sc` stay as they were until phases 4 and 5.

**The link resets the LLVM options.** The wasm-ld driver calls
`cl::ResetAllOptionOccurrences` on each link. In the session, the first
cell linked and the second lost `-wasm-enable-sjlj` and `-wasm-enable-eh`,
and the backend aborted with "-exception-model=wasm only allowed with at
least one of -wasm-enable-eh or -wasm-enable-sjlj". The spike did not see
this because it linked each cell in another process. Now `scSetLLVMArgs`
runs before each backend run of the session and skips the options already
given, which avoids the warning "may only occur zero or one times". In
one-shot mode it runs only once, as before.

**The `sc-proto` crate (`proto/`).**

- **bindgen** over `engine/sc.h`. The enums become modules of constants,
  and `sc_msg_header` comes with the layout check. No constant is written
  by hand.
- **Linking** with the `engine` feature:
  - `build.rs` reads the file from `link.py --native --full --emit-link`.
    It has the same objects and archives as the engine, without
    `sc_entry.o`, plus the stubs and the link flags (`-fuse-ld=lld`,
    `--gc-sections`, `--icf=safe`);
  - `build.rs` adds `-lstdc++`;
  - `rustc-link-arg` only covers the programs, tests and examples of the
    crate itself. For the `sc` binary of phase 6b, which lives in another
    crate, the crate declares `links = "sc"` and passes the file on as
    `DEP_SC_LINK_ARGS`.
- **Safe API:**
  - `Session::new`, `cell` and `undo` return a `Reply`, which borrows the
    session. The engine buffers it exposes (`diags`, `so`, `syms`) do not
    survive the next call, because that call takes `&mut`;
  - the engine is not thread safe (LLVM without threads). Only the first
    thread that uses it can use it, and the others get `Error::OtherThread`.
    `Session` is neither `Send` nor `Sync`;
  - a NUL in the arguments and a cell of 4 GiB or more are refused before
    the call;
  - `main` wraps `sc_main`.
- **What Rust does not guarantee:** a fatal error or a compiler crash still
  ends the process (the top-level `return`, §22). So the native REPL keeps
  the child (§1.1 of the plan).
- **Test (`make test-proto`):**
  - a C session with a shared `static` and a cell with an error;
  - the replay, which gives the same symbols;
  - `undo`;
  - a C++ session with an exception and libc++, run in `scload`;
  - `main` and the thread rule.

  It takes 2.1 s.

**Regressions.**

- `make test`, `make test-native` and `make test-warn`: 13,819 files, 0
  differences.
- `make checkinv`: SAME.
- `make spike`: the same as in §22 (A 29/29, B 26/29, replay 23/23).
- Size: `sc.wasm` is at 5,552,196 bytes zstd (−775 bytes) and `build/sc` at
  7.80 MB zstd. The session is not in them yet.

## 24. REPL phase 2: the child's loop and the corpus in cells

Date: 2026-10-14.

**The loop is in Rust** (decision of §1.1 of the plan). `sc-proto` gained:

- `wire`: the header from `sc.h` and the three parts, in the host's byte
  order;
- `serve`, with the `engine` feature: the child's side, one session at a
  time, until the end of the input;
- `Child`: the parent's side. A child that dies (crash, `SIGILL` from
  `__debug crash`) becomes an I/O error, with the exit status;
- `Repl`: the log of the cells, including the ones that failed. When the
  session is lost, a new child replays the log without the backend, and each
  accepted cell has to export the same names (§3.6 of the plan). `undo`
  removes the last accepted cell from the log and replays the rest.

`build/sc-engine-full` (`proto/src/bin/sc-engine.rs`) is the uncut engine
with this loop. `-cc1`/`ld` go to `sc_main`, `session` is the child, and
`repl <dir> <cells...> -- <args>` is a test of the parent's side, which
writes the `.so` of each accepted cell. The child writes the protocol to a
copy of stdout and puts stderr on fd 1, so that nothing from the engine
breaks a message. Eight cells with a crash and an `undo` (three children,
two replays) take 0.12 s.

**`split-` (`test/split.py`, `make test-split`).** The 1,947 programs the
corpus runs (samples, c-testsuite, gcc torture `execute/`, `test/cxx` and
`test/mod`) are cut into cells, one per top-level declaration (the cutting
of `spike.py`). The cells go through `sc-engine-full repl`, and `scload`
runs the `.so` files. The output has to be that of the whole program
(`pic-` of the native base). There are 10,541 cells, and the run takes 29 s
on 48 cores.

The runs, with the differences left after each fix:

| run | fix | differences |
| --- | --- | --- |
| 1 | — | 213 (91 with a cell that fails) |
| 2 | the cutting (K&R definitions, `typedef struct __attribute__((...)) {`, `-I` of the program's directory) and block scope in C | 135 |
| 3 | the type of the declarations of functions without a prototype | 23 |
| 4 | in the loader, the GOT of a symbol that a later cell defines; the `static` used before the cell that defines it; `always_inline` | 13 |
| 5 | the backend's diagnostic handler; the loader stubs; a `//` inside `/* */` in the cutting | 7, of which 6 known |
| 6 | the bases redone with the new loader | only the 6 known ones |

**Block scope in C** is an upstream bug. In `Sema::ActOnPopScope`, in
incremental mode in C, no name leaves the `IdResolver`. The comment says
this is so that the top-level names hold in the following PTUs, but the
condition holds for every scope, so parameters, locals and labels stay
visible after the block. `int f(int f) { return f; }` in a session gave
"redefinition of 'f' as different kind of symbol". The patch adds
`|| S->getParent()`, so that only the TU scope keeps the names. A cut does
not solve this. Outside incremental mode the condition does not change, and
the one-shot engines give the same result.

**The session's post-pass** (`sc_session.cpp`) gained three things:

- **Retyped declarations.** The session keeps the LLVM type of each function
  a cell exports (the cells share the `LLVMContext`). A declaration with
  another type is rebuilt with the type of the definition, and the
  backend's `FixFunctionBitcasts` adjusts the calls, as in the whole TU.
- **`static` before the definition.** The declaration of an internal name
  becomes the promoted name (`f.sc`). The session keeps these names while
  they have no definition. Before the cell that defines them closes the
  module, it asks CodeGen for their address, and this takes them out of the
  deferred ones. Without this, CodeGen would only emit them if the cell
  itself used them.
- **`available_externally`.** `scExports` no longer lists these definitions.
  The backend discards them, so they are not exported. A later cell that
  needed the body for inlining got only a declaration. After each cell they
  go back to the deferred ones (`HandleTopLevelDecl`), as CodeGen already
  does in incremental mode for the internal and `linkonce` ones
  (`addEmittedDeferredDecl`).

**The diagnostic handler.** In one-shot mode, the `BackendConsumer` of
`CodeGenAction` installs a handler in the `LLVMContext`. The session has no
`CodeGenAction`, and LLVM's default handler printed the backend error
without clang seeing it. The cell gave `SC_OK` with a bad object. Now
backend errors and warnings go into the session's diagnostics
(`err_fe_backend_unsupported`), and the cell gives `SC_LOST`, because the
code has already been generated.

**The loader** (`loader/src/main.rs`):

- **Forward references:**
  - a `GOT.mem` without a definition stays pending and is filled in when
    the symbol arrives;
  - a `GOT.func` gets a table entry right away, filled in when the function
    arrives. The pointer is the same for every module.
- **Calls to a function that has no definition yet.** Before, they went
  through a host trampoline (`Func::new`). Each call went through the native
  stack, and a recursion of 500 levels across cells (`nestfunc-4`)
  overflowed the stack. Now they go through stubs in wasm, a
  `call_indirect` to the same table entry. Before the definition arrives,
  the entry holds a host function that gives "undefined symbol: X".
- This also changes the `pic-` and the `repl-` of a whole program with an
  undefined symbol. The error moves from load time to the first call
  (`bcp-1`). The bases were redone, and the difference between the old and
  the new bases is below.

**Known limits** (`KNOWN` in `split.py`; 6 programs):

- a `#line` does not carry over from one cell to the next;
- a directive in the middle of a declaration (the cutting splits it);
- an `alias` to a name from another cell;
- an `#include "../../gcc.dg/..."` that the `-I` of the cells finds and the
  whole compilation (in bwrap) does not;
- **a function with the name of a libc function and another signature**
  (`div` in `20090711-1`). The link of the next cell resolves the name
  through `libc.so` and gives "function signature mismatch". The loader
  already hides the libc one behind the cell. Fixing it in the link would
  require passing the `.so` files of the previous cells to wasm-ld, and
  they do not exist in a child that has just replayed the log. This is left
  for later.

Warnings that the cells give and the whole TU does not, such as "unused
function" for each cell and "has internal linkage but is not defined" for a
`static` used before its definition, are §3.4 of the plan and are not
filtered yet.

**Regressions.**

- `make test`, `make test-native` and `make test-warn`: 13,819 files, 0
  differences, against the redone bases.
- The four bases, old against new: only `pic-bcp-1` changes (the loader).
- `make checkinv`: SAME ×8.
- `make test-proto`: green, with the new `Repl` test (crash, replay,
  `undo`).
- `make test-split`: 1,947 programs, 0 differences apart from the 6 known
  ones.
- `make spike`: A 29/29, B 26/29, replay 24/24. The replay covers one more
  program (there were 23) because the cutting changed.
- Size: `sc.wasm` 5,552,276 bytes zstd (+80, the patch) and `build/sc`
  7,799,434 zstd. The session is not in them yet.
## 25. The cell that fails: a snapshot by `fork`

**The problem.** A cell with an error goes through `CleanUpPTU`, copied
from clang-repl. It removes the cell's declarations from lookup, but not
the rest. These cases break the next cell, all with `int x = ;` in the one
that fails:

| cell that fails | the next one, the same but without the error |
| --- | --- |
| `#include <stdio.h>` (C) | `printf` not declared. The `_STDIO_H` guard stayed defined and the header is not read |
| `#include <vector>` (C++) | "reference to ... is ambiguous". The `Sema` keeps the cell's `namespace std`, and the new `std` reopens the dead one |
| `new int(3)` (C++, no header) | "no matching function for call to 'operator new'". The implicit `operator new` stayed in the dead TU |
| `#include <vector>` with `<cstdio>` before it | "redefinition of enumerator". The enumerators stay in the lookup of `std`, which survives |

Undoing the macros and the `#pragma once` fixes C. Clearing the `Sema`'s
pointers to `std` trades the ambiguity for `operator new` and the
comparison categories. Patching one cache after another never ends.

**Decision: the child calls `fork` before each cell** (`serve`, in
`proto/src/engine.rs`):

- the copy waits on a pipe;
- if the cell is accepted, the original sends `d` and the copy exits;
- if it fails, the original sends `t` and the reply, and exits. The copy
  sends the reply and goes on with the session from before the cell;
- if the original dies in the cell (the test's
  `#pragma clang __debug crash`), the pipe closes with nothing. The copy
  replies `SC_ERROR` ("the engine crashed on this cell") and goes on,
  without a replay;
- if `fork` fails, a cell with an error becomes `SC_LOST`, and the `Repl`
  replays the log without it.

So the `Repl`'s log keeps only the accepted cells. The child's pid changes
on each error, and `Child` no longer relies on it to know how the child
exited. `CleanUpPTU` stays. It is what makes `sc_session_cell` usable
without the child, in the API tests.

**Cost**, on `sc-engine-full`, with the headers `iostream`, `vector`,
`string`, `map` and `algorithm` plus 20 functions, one per cell:

| | median of 5 |
| --- | --- |
| without `fork` | 3.27 s |
| with `fork` | 3.62 s |

That is ~17 ms per accepted cell. The cost is the copy of the page tables
and the copies on write while the copy lives. 20 cells with an error cost
3 ms each. Replaying the log on each error, the other correct option,
would cost ~2 s per error in this session.

**In the browser** there is no `fork`. The equivalent is to copy the wasm
linear memory before the cell and restore it if the cell fails. That waits
for phase 6b.

**Regressions.** `make test-proto` is green. The `Repl` test covers a
repeated `#include` after a cell with an error, and the crash is now
`Error` without a replay. `make test-split`: 1,947 programs, 0 differences
besides the 6 known ones.

## 26. Prototype of the interactive REPL (`sc`)

Before phase 6b, an interactive `sc` to use the REPL for real. The command
line interface is the one of sgleam and spython:
- `sc` opens the REPL;
- `sc FILE [ARG...]` runs a program;
- `sc repl [-q] [FILE]`, `sc run`, `sc check` and `sc help`;
- `-x c|c++` chooses the language. Without it, the file extension decides,
  and C is the last fallback;
- the commands `:help`, `:quit`, `:undo` and `:theme`;
- the prompt uses rustyline, with the history in `data_dir()/sc/history`
  and the theme in `config_dir()/sc/config`.

**Assembly:**
- the `loader/` crate became a library (`Loader`) with two binaries, the
  earlier `scload` and `sc` (`make repl`, `build/loader/release/sc`);
- the compiler is the child `build/sc-engine-full session`, through
  `sc_proto::Repl`;
- the session's arguments are those of `scripts/flags.py --session`,
  written to `build/repl/{c,c++}.args`. The engine's flags live only in
  `scripts/flags.py`, and `corpus.py`, `spike.py`, `split.py` and
  `checkinv` read them from there. Before, `sc` read flags that a test
  script generated;
- the engine is not yet linked into `sc`. That stays for phase 6b;
- `sc` and its dependencies sit behind the crate's `repl` feature.
  `scload` (`make test`) does not need `sc-proto`, whose bindgen needs
  libclang.

**Provisional decisions of the front end** (docs/plan-repl.md §8):
- **The cell:** top-level declarations and statements, which the
  incremental mode accepts. The statements run when the cell's `.so` is
  loaded.
- **The `main`:** runs only in `sc FILE`. In the REPL it is a function like
  the others, and the student calls it in a cell.
- **The stdin** is the terminal's. Rustyline lets go of it while the cell
  runs.
- **Trap:** the message is short (`error: wasm trap: integer divide by
  zero`, `aborted`, `error: stack overflow`), the `__stack_pointer` goes
  back to its value from before the cell and the REPL goes on. What the
  cell defined stays.
- **Uncaught C++ exception:** libc++abi prints "terminating due to
  uncaught exception…" and the cell ends as `aborted`. The REPL goes on.
- **`exit(n)`** ends the REPL with status `n`.
- **Ctrl-C:** the SIGINT handler calls `increment_epoch` and the cell stops
  with "interrupted" (`epoch_interruption`). The child sits in its own
  process group, so that Ctrl-C does not kill it. A Ctrl-C during a read
  from stdin only takes effect after the read returns. The `fflush` after
  the interrupted cell resets the deadline. Otherwise the flush would
  stop too, and what stayed in the buffer would come out after the next
  cell. This holds only in the REPL. `sc FILE` runs without the epoch
  checks, which cost something in every loop and call, and Ctrl-C kills
  the process, as in `scload`.
- **`sc check`** compiles in `SC_CELL_SO`, with the backend and the link,
  and runs nothing. `SC_CELL_REPLAY` would be cheaper, but it would miss
  the errors that only the backend or `wasm-ld` give (an undefined
  symbol).
- **`:undo`** undoes the declarations in the compiler. What the cell
  already ran stays done.
- **Enter asks clang** (see below).

**Size:** `sc` is 28.1 MB, against 27.2 MB for `scload` (wasmtime with the
default features. The minimum of §1.4 waits for phase 6a). Rustyline, bpaf
and `sc-proto` add up to 0.9 MB.

**Enter asks clang.** A first version decided with a heuristic in Rust. It
counted the brackets and guessed a function header from the shape of the
line. It does not know what a type is (`T * f(x)`). Now the engine
decides, with two new things in `sc.h`:
- `SC_CELL_WHOLE`: compiles the cell like `SC_CELL_SO`, but replies
  `SC_INCOMPLETE`, without diagnostics, when the first error is at the end
  of the cell. This holds for an error after the cell's last token and for
  an error on the end-of-input token of the incremental mode, which sits
  in the main file, which is empty. It is the parser saying that it
  wanted more. Examples are a declaration without `;`, a header without a
  body, a `for` or `else` without its statement, and `1 +`. An error in
  the middle (`x` undefined) is a real error. `serve` calls `fork` as for
  an ordinary cell, so the incomplete cell leaves nothing behind;
- `sc_session_depth` (`SC_MSG_DEPTH`): the brackets that the input leaves
  open, from clang's lexer in raw mode. It counts one more if the input
  ends inside a comment or with `\` at the end. This gives the indent of
  the new line and does not touch the session.

On Enter with the cursor at the end, the REPL does the following:
- it asks for the depth. With an open bracket, it opens a new line;
- a blank line runs whatever is there;
- otherwise, it sends the cell in `SC_CELL_WHOLE` and keeps the reply. If
  the cell is complete, it uses that reply. The compilation on Enter is
  the cell itself, with no double work.

Measured cost, counting compile, link and load: about 10 ms per cell in C
and about 22 ms in C++ with `iostream`. `int x = 5` without `;` waits for
the next line. The blank line shows the error.

**The value of an expression without `;`.** The plan (§3.4) left value
printing out of v1. It went into the prototype, in clang-repl's format.
`x * 2` shows `(int) 42`. How it works:
- the session parses a prelude with two declarations, `__sc_show_int(const
  char *type, int kind, long long)` and `__sc_show_float(..., double)`. It
  is a system file, so it gives no warning, and it does not count in
  `cell_N`;
- when the last statement of a cell is an expression without `;`
  (`TopLevelStmtDecl::isSemiMissing`), the session replaces it in the AST,
  before CodeGen, with a call to one of the two. The call takes the type
  as a string, the kind of value (`enum sc_show` in `sc.h`) and the value
  converted to `long long` or `double`;
- the host defines the two functions. `Loader::host` puts a wasmtime
  function in the symbol table, and the cell imports it like any other.
  `sc` keeps the text and prints it after the `fflush`, in the right order
  relative to what the cell wrote;
- the host formats the value. A `double` gets the fewest digits that read
  back as the same number (`0.30000000000000004`), a `char` goes between
  single quotes, a `char *` or `char` array prints as a string (up to
  1,000 bytes, with escapes), a pointer prints in hexadecimal, and null
  prints as `NULL` in C and `nullptr` in C++;
- if the expression has temporaries (`std::string("abc").size()`), the new
  call sits inside the original's `ExprWithCleanups`, and they die after
  it;
- only scalars show. A struct, class, array that is not of `char`,
  function and `void` show nothing, so `std::cout << x` comes out as
  before;
- the type name is clang's, with two changes. `__size_t` (the type of
  `sizeof`) becomes `size_t`, and a class member typedef
  (`std::string::size_type`) becomes the type it names.

Two changes in the incremental parser, in `llvm/incremental-stmts.patch`:
- the expression without `;` at the end of the input is no longer a
  discarded value. So `x > 2` does not warn "comparison result unused",
  and `warn_unused_expr`, which the session ignored like clang-repl, is
  back. Now `x + 1;` warns as in the whole-program compiler. Since the
  value is not discarded, `CheckPlaceholderExpr` would not run, and
  `s.size` (without the parentheses) or `__builtin_abs` would reach CodeGen
  and crash the engine. The patch calls it earlier;
- `-x` or `+x` at the top level was "expected external declaration". The
  parser reserved `-` and `+` for Objective-C methods. In the incremental
  mode, they start a statement.

Both change only the incremental mode. `make test` did not change (0
differences), `checkinv` gave SAME, and `sc.wasm` grew by 53 bytes.

**What remained visible** goes to the engine (fixed in §29):
- the first diagnostic of each cell comes with "In file included from
  <<< inputs >>>:1:";
- the cell is called `cell_N` in the diagnostics (in `sc FILE` it should
  be the file name);
- the diagnostics have no color;
- the "the rest is missing" error, seen when a blank line forces the
  cell, points to `<<< inputs >>>:1:1` instead of the end of the cell.
- `static int f(void);` in one cell and its use in another warn "unused
  function" and "has internal linkage but is not defined". Each cell ends
  as a translation unit, but the definition comes later.

## 27. Simplification review of the REPL

A review (reuse, simplification, efficiency and altitude) of phases 2 and
later. No behaviour changed. `test-split` gives 0 differences, `spike`
gives A 29/29, B 26/29 and replay 24/24, `make test` gives 0 differences
and `checkinv` gives SAME.

**Changes:**
- **Engine flags:** they moved to `scripts/flags.py`, which is the single
  source. Before, `sc` read `build/repl/*.args`, which
  `test/split.py --args` generated by calling `test/corpus.py`. The
  product depended on the harness. Now `corpus.py` and `spike.py` import
  the module. `split.py`, the `proto` tests and `make repl` use
  `--session`, and `checkinv` uses the same script. The generated
  arguments are identical to the earlier ones.
- **Libraries:** the loader does not read again from disk the libraries it
  already loaded. Before, each C++ cell read about 2.9 MB (libc.so,
  libc++.so and libc++abi.so) only to discard it.
- **Reply to an accepted cell:** the child sends the reply before it waits
  for the `fork` copy to exit.

**Discarded experiment:** emit the internal definitions right away in the
incremental mode. It was a condition in `ASTContext::DeclMustBeEmitted`.
With `IncrementalExtensions`, a `GVA_Internal` definition is required.
So each cell would emit its `static` definitions, and `Wanted` and the
`BeforeEnd` hook would leave `sc_session.cpp`, 28 fewer lines. Measured on
`test-split`:

| | before | with the patch |
|---|---|---|
| `.so` of the cells, sum | 18,324,012 B | 18,583,858 B (+1.4%) |
| programs that grow / shrink | | 254 / 17 |
| largest increases | | `memclr` +118 KB (from 1 MB), `c11_ranges` +28 KB (+15%), `m2_stl` +35 KB |
| programs that differ | 0 | 1 (`alias-3`) |

C++ grows because the namespace-scope constants in the headers have
internal linkage and get emitted in every cell that includes the header.
`alias-3` breaks because the alias of the next cell names a `static` that
the previous cell already emitted, the same case as `alias-2` and
`alias-4`. Before, the `static` was deferred and emitted together with the
alias. A patch in `llvm/`, more size and one program fewer are not worth
28 lines. `Wanted` stays.

## 28. `:type`

`:type EXPR` shows the type of an expression without running it. In
`sc.h`, it is a new cell mode, `SC_CELL_TYPE`, which returns the type in
place of the exported names (`SC_SYMS`). So the protocol and the message
do not change.

**In the engine** (`ScSession::type`):
- the cell goes through the parser and Sema like the others, but no
  declaration goes to CodeGen;
- what Sema hands over along the way, such as the implicit definition of
  a copy constructor, goes out with the cell's module, which is discarded;
- the cell has to be an expression without `;`. Anything else (`int`,
  `x;`, `1; 2`, nothing) gives "expected expression";
- as in a cell that fails, what Sema did stays in the session. In the
  child, the `fork` copy taken before the cell always takes the place of
  the original, even with `SC_OK`, and the `Repl` does not put the cell
  in the log.

**The type name** is the same as for the shown value (§26), with a new
rule that holds for both:
- a namespace-scope typedef (`size_t`, `std::string`, `FILE`, one of the
  student's) and the `__size_t` of `sizeof` keep the name;
- a class member typedef becomes the type it names, as before;
- the rest comes out in canonical form.

Before, the rest came out as written, and what came from inside a
template was unreadable outside it:

| expression | as written | canonical |
|---|---|---|
| `s + "c"` | `basic_string<char, std::char_traits<char>, std::allocator<char>>` | `std::string` |
| `v.begin()` (`vector<int>`) | `__wrap_iter<pointer>` | `std::__wrap_iter<int *>` |
| `m.find("a")` (`map<string,int>`) | `__map_iterator<typename __base::iterator>` | `std::__map_iterator<std::__tree_iterator<...>>` |

The canonical form comes out as `std::string` because libc++ marks
`basic_string<char>` with `preferred_name`. We measured and discarded
`TypeName::getFullyQualifiedType`, which clang-repl uses. It writes
`std::basic_string<char, std::char_traits<char>,
std::allocator<char>>` and `std::vector<...>::iterator`, with all the
template arguments.

The session is only in `sc-engine-full`, so `sc.wasm` does not change. In
the shell, the argument of `:type` is colored as code.

## 29. The REPL's diagnostics

The items that §26 left visible, fixed without a patch:

- **"In file included from <<< inputs >>>:1:"** came out before the first
  diagnostic of each cell. Each cell is a file that the empty main file
  includes. `FirstError` now receives the output of
  `TextDiagnosticPrinter` in a buffer and removes that line. The
  alternative, creating the cell without an include location, would change
  `isInMainFile` for each declaration, and `isBeforeInTranslationUnit`
  between two cells would have no common ancestor (`llvm_unreachable`). An
  `#include` inside a cell still shows the stack up to the cell.
- **The "the rest is missing" error** (`1 +` forced by a blank line,
  `:type int`) pointed to `<<< inputs >>>:1:1`. The end-of-input token
  belongs to the main file. A diagnostic in the main file now goes to the
  end of the cell's last token (`CellEnd`, from the raw lexer), with the
  same message. This also simplifies the `SC_INCOMPLETE` test, which
  looked at both places.
- **Color:** the engine accepts `-fcolor-diagnostics` (`ScOptions::Colors`,
  `ShowColorsKind::On`. Without it, the default stays, and `checkinv` does
  not change). The `FirstError` buffer is a string, so it needs
  `enable_colors`. `sc` passes the flag when stderr is a terminal, with
  the same rules as the prompt (`NO_COLOR`, `FORCE_COLOR`).
- **The file name:** in `sc FILE`, `sc check FILE` and `sc repl FILE`,
  `sc` puts `#line 1 "FILE"` before the code. With `ShowPresumedLoc`, the
  diagnostics and `__FILE__` give the file name. In the REPL, the cell is
  still `cell_N`.
- **End of translation unit warnings:** `static int f(void);` in one cell,
  its use in another and the definition in a third warned "unused
  function" and "has internal linkage but is not defined". Worse, the
  "unused function" of a `static` function repeated in every later cell
  until the function was used, because Sema's list
  (`UnusedFileScopedDecls`) never emptied. The session now empties that
  list and `UndefinedButUsed` on each declaration that the parser returns,
  so the end of each cell has nothing to say. The local variable warnings
  (`-Wunused-variable` in a function body) stay.

  This holds only where a cell comes next. In `sc FILE` and
  `sc check FILE`, the file is the whole program, and `sc` creates the
  session with `-sc-one-cell` (`ScOptions::OneCell`). The lists stay, and
  the warnings come out as in the whole-program compiler. `sc repl FILE`
  does not pass the flag, because the next cells can use and define what
  the file left.

**Undefined symbol.** A function or variable used and never defined:
- in `sc FILE` or `sc check`, it ran. The call stopped with "undefined
  symbol: h.sc" (the name the session gives a `static`) or `_Z1fv` (the
  C++ name without demangling). A variable read address 0 and the program
  exited with 0. Now the one-cell session links with
  `--unresolved-symbols=report-all`, does not promote the `static`
  definitions and does not show values, because nothing comes after.
  `wasm-ld` rejects the cell as in the whole-program link:
  `wasm-ld: error: undefined symbol: f()`;
- the `wasm-ld` messages no longer give the temporary path of the cell's
  object (`/tmp/sc-XXXX/cell_1.o: `), which the student does not
  recognize;
- in the REPL, the loader keeps looking for the function until it is
  called, because a later cell can define it. The loader's stub now gives
  the name as the code writes it. It drops the `.sc`, and it demangles
  with `cpp_demangle`, which wasmtime already brought in, only for names
  that start with `_Z`. Without that filter, `h` became `unsigned char`.

**The `extern` variable not yet defined in the REPL** read 0, and a write
to it went to address 0, without a warning. The `GOT.mem` entry without a
definition was 0 until the definition arrived. Now each such variable has
its own address from 3 GiB up (`UNDEFINED`), in a 1 MiB block, so that a
field or an element falls in the same block. The memory has a maximum of
3 GiB, so a read or write there always traps. Wasmtime describes the trap
with the address ("memory fault at wasm address 0x..."), but it does not
export the type (`WasmFault`). So `sc` reads the address from the text,
and `Loader::undefined_at` says whose address it is:
`error: undefined symbol: v`. A `weak` variable still has address 0, and
`&v` is no longer null, as it should be for a variable that is not
`weak`. A pointer taken before the definition still points to the
provisional address, and the trap then comes out without the name.

## 30. `--safety`: the levels and the warnings (F1 of docs/plan-checks.md)

**The levels.** `sc` has `--safety=off|on|strict` (the default is `on`) in
the REPL, in `sc FILE` and in `sc check`. In the REPL, `:safety LEVEL`
restarts the session empty, because the earlier cells would have to run
again to compile at another level. The sets are in `scripts/flags.py`
(`safety()`), and `make repl` generates one argument file per language,
level and mode: `build/repl/<lang>-<level>[-file].args`. The `-file` one
is for `sc FILE`, which turns on `-Wshadow` in C (docs/plan-checks.md
§4.3). The engine did not change its defaults. `flags.py` chooses.

| Level | Warnings | Errors |
|---|---|---|
| `off` | clang's, without `-Wall -Wextra` | clang's |
| `on` | `-Wall -Wextra` − R + A | + E, with the exceptions of `E_WARN` |
| `strict` | `-Wall -Wextra` + A + S + `-Wpedantic` | + E |

**The strict parse.** `scParseArgs` silently rejected what it did not
know. Now it:
- rejects an unknown flag (`sc: unknown argument: '-fsanitize=address'`);
- requires the fixed value of the flags that do not vary (`-triple`,
  `-target-cpu`, `-mllvm`, `-target-feature`...);
- rejects `-x` and `-std=` values that do not exist.

A session that receives such a flag does not start. `-mllvm -asan-*` and
the sanitizer flags come in F2 and F3.

**Fields without an initial value.** The `checkinv` of the levels hit a
segfault inside `generateCC1CommandLine`. Valgrind showed reads of
uninitialized memory already in the old `checkinv`. Six fields have no
value in the constructor, because the option parser always writes them.
With more arguments, the heap garbage changed, and the
`denormalizeSimpleEnum` of `GPUDefaultStream` crashed. `sc_inv.cpp` now
gives the parser's value to:
- `GPUDefaultStream` and `SanitizeKcfiHash`;
- `PrintSupportedCPUs`, `PrintSupportedExtensions` and
  `PrintEnabledExtensions`;
- `ASTDumpDeclTypes` and `IsSystemModule`.

The engine read the same fields without a value. Valgrind reports nothing
now.

**`checkinv`** gained 9 lines: {C pic, C++ pic, C++ pic mod} × {off, on,
strict}. That makes 17 lines, all SAME.

**The corpus at the levels.** Each program that runs also runs at levels
`on` and `strict` (`<name>-on`, `<name>-strict`), linked as a whole, as in
`sc FILE`. The c-testsuite and the gcc torture tests have UB on purpose.
In them E stays a warning (`safety(..., errors=False)`), so that the
program does not stop at compile time.

**The session and `-Wpedantic`.** The session's main file is empty, and
the cells come after it. `-Wpedantic` counted the "ISO C requires a
translation unit to contain at least one declaration" without showing it,
and `strict` printed only "1 warning generated.". The session now ignores
`ext_empty_translation_unit`.

**Correct student code (`test/student`, `make test-student`).**
- There are 20 programs. 11 were written for this (lists, matrices,
  strings, structs with `qsort`, recursion, a menu with `fgets`, `vector`,
  `map`, classes with exceptions, BFS, a stack template), and 9 are the
  correct ones from reviewer W.
- `expected.txt` gives the exit code and the warnings, by flag, of each
  program at each level. At `on`, no program may have an error.

Measured:
- `off`: no warning.
- `on`: 14 of the 20 programs have no warning at all. Of the 6 warnings,
  the `-Wunreachable-code` in `rev-ur.c` is the false positive described
  below. The plan already expected the other 5:
  - `-Wshadow` in `rev-c2.c`;
  - `-Wdeprecated-copy-with-user-provided-dtor` in `rev-dtor.cpp`;
  - `-Wsometimes-uninitialized` in `rev-u.c`, as a warning;
  - `-Wunused-but-set-variable` in `p2` and `p4`, which is true.
- `-Wlifetime-safety-permissive`, with `dangling-field` and
  `dangling-global`, did not fire in any program. It stays in A.
- `strict`: `-Wmissing-prototypes` fires in 16 of the 20 programs, up to
  11 times in a single one. Each student function without a prototype
  before it fires the warning. It is a style for projects with several
  files. It moved from S to N (the user's decision). It asks for `static`
  or a prototype that adds nothing for someone with a single file, and it
  hid the other `strict` warnings.
- `-Wsign-conversion` fires in 7 programs, up to 9 times in a single one
  (the BFS).

**The cells were not the main file.** Each cell comes in as a file
included at the start of the main one (as in clang-repl).
`SourceManager::isInMainFile` then said that the cell was not the main
file, and two clang checks that depend on it failed:
- `-Wunreachable-code` (A) **never fired**, neither in the REPL nor in
  `sc FILE`. `CheckUnreachable` only analyzes the main file. The `puts`
  after a `return` passed without a warning;
- `-Wheader-hygiene` (S) gave a **false positive** on
  `using namespace std;` ("in header"), in the 10 C++ programs of
  `test/student`.

A cell without an include location would count as the main file. But
`isBeforeInTranslationUnit` between two cells with no common ancestor
reaches `llvm_unreachable("Unsortable locations found")`.

The fix is the 4-line patch `llvm/incremental-main-file.patch`.
`isInMainFile` also accepts the file included at the start of the main
file. An `#include` never sits at that point, because the include
location is that of the file name, after `#include`. Only an incremental
input sits there. After the patch:
- the two warnings behave as in clang on the whole program.
  `-Wunreachable-code` fires, and `-Wheader-hygiene` no longer fires;
- in the other calls to `isInMainFile` (unused macros, `#pragma hdrstop`,
  modules, PGO, the form of the message for an unused `static`), the cell
  is now treated as the student's code, which is what it is.

**A measured false positive of `-Wunreachable-code`.** Reviewer W's
`rev-ur.c` has `enum { MAX = 10 }` and `if (MAX > 100) printf(...)`. The
pin's clang also warns when it compiles the file as a whole. An `enum`
constant initialized with a literal does not count as a configuration
value. The reviewer had measured the group without this case (W-A12). The
other cases in `rev-ur.c` do not fire at `on`:
- `#define DEBUG 0`;
- `const bool`;
- `break` after `return` and `return` after `exit`;
- `while (1)` with a `return` inside.

`-Wunreachable-code` stays in A. Clang's note shows how to silence it:
`if ((MAX > 100))`.

## 31. The location of a runtime error (F0 of docs/plan-checks.md)

Before, a runtime error gave only the kind, without the line:
`error: wasm trap: integer divide by zero`. Now `sc` gives the error in
words and the frames of the student's code:

```
error: division by zero
    at average (div-zero.c:4:16)
    at main (div-zero.c:10:20)
```

**The pieces:**
- **`-gline-tables-only` in every session.** `sc_inv` accepts
  `-debug-info-kind=line-tables-only`, `-dwarf-version=4` and
  `-debugger-tuning=gdb`, as the driver passes them. `flags.py --session`
  always adds them, at every level. The error location only changes the
  message (§3 of the plan). `checkinv` gained two lines with them (19
  SAME), and it reports a missing one.
- **`wasm_backtrace_details` in the loader** (`lib.rs`), with
  `wasm_backtrace_max_frames` of 2^20. The default of 20 frames would lose
  the `main` of a recursion.
- **The frames.** `machine.rs` shows the frames with a line in a student
  file. It leaves out the build's headers (libc++) and the libc sources,
  which come with DWARF from wasi-sdk (`wasisdk://...`). If the error
  stops in a library function, that function comes first, without a line
  (`in strlen`). The names come from the name section, which wasm-ld
  already demangles. Adjustments:
  - `__original_main` and `__main_argc_argv` are `main`;
  - the `.sc` of a cell's `static` goes away, also in the demangled form
    `f(int) (.sc)`;
  - `std::__2::` becomes `std::`, and the `[abi:...]` tags go away;
  - a lambda is `a lambda in main`;
  - the statements of a cell (`__stmts__N`) are only the location:
    `at cell_3:1:1`, the name that the diagnostics give the cell.
- **Recursion.** A repeated frame shows once, with the count
  (`at factorial (recursion.c:4:16), 10916 times`). A cycle of 2 to 4
  frames, such as `even` and `odd`, shows once, with
  `(the 2 frames above, 8187 times)`. What remains, if very long, shows
  the first 10 and the last 3.
- **The messages:**

  | Trap | Message |
  |---|---|
  | `call stack exhausted` | `stack overflow (infinite recursion?)` |
  | `out of bounds memory access` with `__stack_pointer` below 0 | `stack overflow (infinite recursion?)` |
  | `out of bounds memory access` | `invalid memory access` |
  | `integer divide by zero` | `division by zero` |
  | `integer overflow` | `integer overflow in a division (INT_MIN / -1)` |
  | `indirect call type mismatch` | `call through a function pointer of the wrong type` |
  | `uninitialized element` | `call through a null function pointer` |
  | `undefined element` | `call through an invalid function pointer` |
  | `unreachable` | `aborted`, with the student's frames, without `abort` |
  | interrupt (Ctrl-C) | `interrupted`, with the frames, which show where the loop was |

  The message uses the `stack_pointer` from before `exec` restores it.

**Measured costs:**
- compiling with `-g` costs about 2% per cell (`graph.cpp`: 1.94 s against
  1.91 s; `rev-c1.c`: 0.058 s against 0.060 s). The `.so` grows by 20% to
  40% (127 KB against 105 KB; 7.5 KB against 5.3 KB);
- `wasm_backtrace_details` has no measurable cost. `sc run graph.cpp`
  takes about 2 s with or without it, and `list.c` 0.09 s;
- capturing and symbolizing all the frames of an infinite recursion takes
  less than 0.1 s. That is 16 thousand frames, and the whole `sc` run
  takes 0.14 s.

**DWARF in the incremental session (T8).** `test-split` now compiles the
10541 cells with `-g`, because it goes through `session()`: 0
differences. Each cell has its own line (`cell_N`). In `sc FILE`, the
`#line` gives the file name to the DWARF too.

**`test/checks`.** There are 15 programs, with one error in each, run with
`make test-checks` at the three levels. `expected.txt` keeps the whole
output. That is the warnings, the message, the frames and the status. Two
of them are REPL sessions (`*.repl`, one line at a time).

**An old defect, found by `test-checks`.** Sometimes, at the end of a REPL
session, `sc: Broken pipe (os error 32)` came out, 2 to 4 times in 48
parallel sessions. The engine replied to an accepted cell **before** it
told its copy (the cell's `fork`) that the copy could exit. `sc` received
the reply, reached the end of the input and killed the engine. Without
the notice, the copy assumed that the engine had crashed and replied on
the pipe, which was already closed. Now the engine notifies the copy
before it replies and waits for it afterwards (`proto/src/engine.rs`). In
5 rounds of 48, the error did not appear.

**Outside F0:**
- the libc++ hardening message (`__clang_trap_msg`) only exists with the
  `EXTENSIVE` of F2. Today `v[5]` gives `aborted` with the line;
- an `assert` in a cell statement warns
  `-Wpredefined-identifier-outside-function` (the `__func__` of `assert.h`
  outside a function) and prints `(cell_6: : 1)`.

## 32. Runtime checks (F2 of docs/plan-checks.md)

At levels `on` and `strict`, the student's code is now compiled with
UBSan, libc++'s `EXTENSIVE` hardening and locals initialized with a
pattern. At `off`, none of this happens. The same program runs silently,
as in an ordinary compiler.

```
10! = 3628800
11! = 39916800
12! = 479001600
error: signed integer overflow: 479001600 * 13 cannot be represented in type 'int'
    at factorial (overflow.c:6:15)
    at main (overflow.c:13:33)
```

**The flags** (`checks()` in `flags.py`, which `safety()` calls):
- **U:** `-fsanitize=` with the 17 checks that the driver turns on with
  `-fsanitize=undefined` on wasm32 (§5.1 of the plan), and
  `-fsanitize-merge=` with the same ones, as the driver does. There is no
  `-fsanitize-recover`. The report stops the program. The driver also
  passes `-fno-sanitize-memory-param-retval` and
  `-fno-sanitize-address-use-odr-indicator` when there is a sanitizer.
- **libc++:** `-D _LIBCPP_HARDENING_MODE=_LIBCPP_HARDENING_MODE_EXTENSIVE`
  at `on` and `strict` (the `DEBUG` of `strict` waits for F4), and `NONE`
  at `off`.
- **Locals:** `-ftrivial-auto-var-init=pattern`.

`sc_inv` now accepts these flags and `-D`, and it rejects a check that it
does not know. `checkinv` gained the `std` interface at `on`: 20 SAME.

**The UBSan mode: full handlers (decision 4 of the plan).** Measured with
the pin's native clang, a PIC object with `-gline-tables-only`:

| | none | handlers | minimal runtime | `trap` |
|---|---|---|---|---|
| `c3_stl.cpp`, time | 1.85 s | 2.24 s (+21%) | 2.07 s | 2.14 s |
| `c3_stl.cpp`, object | 527 KB | 1,122 KB | 755 KB | 852 KB |
| `c1_basics.cpp`, time | 0.82 s | 0.92 s (+13%) | 0.90 s | 0.92 s |

The time of the handlers is almost that of `trap`. The cost is in
generating the checks, not in the call. The handlers give the values
(`479001600 * 13`), and `trap` would not.

**The latency in `sc` (T11)**, the best of 3:

| | `off` | `on` |
|---|---|---|
| `sc c3_stl.cpp` | 3.09 s | 3.74 s (+21%) |
| `sc c1_basics.cpp` | 1.85 s | 1.99 s (+7%) |
| `sc p1.c` | 0.098 s | 0.107 s |
| REPL: 40 small cells with `vector`, `map` and `string`, per cell | 61 ms | 102 ms |

A cell stays below 0.1 s. The REPL keeps the handlers, without the
fallback to `trap` that decision 4 foresaw.

**The handlers live in `libsc_rt` (`rt/sc_rt.c`).**
- It is C, in the sysroot: `libsc_rt.so` for the REPL and `libsc_rt.a`
  for the static programs of the corpus. `solibs()` links `-lsc_rt` only
  at the levels with checks, before libc++.
- Each handler builds the message with UBSan's words and calls
  `__sc_rt_report`. That function flushes `stdout` (what the program
  printed comes first), writes `error: ...` to `stderr` and traps.
- `sc` recognizes `__sc_rt_report` in the frames. It does not add the
  trap message (`aborted`) and shows only the student's frames. In the
  wasmtime CLI, its own message and backtrace come out. There is no host
  import.
- The layouts are those of compiler-rt's `ubsan_handlers.h` for wasm32. A
  value of up to 32 bits comes in the argument. A larger one comes by
  pointer.

**The libc++ message: `enforce` instead of `quick_enforce`.**
- In `EXTENSIVE`, the default semantic is `quick_enforce`. That is
  `__builtin_verbose_trap`, which keeps the message only in the DWARF, as
  an inline function (`__clang_trap_msg$libc++$...`).
- Wasmtime does not find it. Cranelift merges the `br_if` and the
  `unreachable` into one conditional trap instruction and gives the
  address of the `br_if`, 5 bytes before the range of the inline function.
  Reading the DWARF in the host would not help, because the address is a
  different one.
- So the sysroot is now built with `LIBCXX_ASSERTION_SEMANTIC=enforce`. A
  check that fails calls `std::__libcpp_verbose_abort` with the message.
- libc++ defines that function as *weak*, precisely so that the program
  can put its own in its place. `libsc_rt` defines its own. The loader
  and wasm-ld keep the first definition, and `libsc_rt` comes before
  libc++. It removes the prefix
  (`vector.h:414: libc++ Hardening assertion __n < size() failed:`)
  and says `error: vector[] index out of bounds`.
- Without `libsc_rt` (the default `fast` level, outside `sc`), libc++
  prints the whole message and aborts.

**The uninitialized pointer.** On wasm32, `pattern` puts 0xFFFFFFFF in a
pointer. An access through it hits UBSan's `alignment` check (an `int` at
0xFFFFFFFF is misaligned). The handler says
`store to uninitialized pointer 0xffffffff of type 'int'`, and for
arithmetic on it, `pointer arithmetic on uninitialized pointer`. A
`char *` has no alignment to check. The access falls outside memory, and
`sc` says `invalid memory access through an uninitialized pointer` when
the fault address is in [0xFFFFFFFF, 0x1_0000_FFFF).

**One `std.pcm` per level in the corpus.** The module keeps the macros it
was built with (§5.3 of the plan). `corpus.py` builds `std-pcm-on` and
`std-pcm-strict` with the flags of `checks()`, and the `test/mod` programs
of each level import their own. The REPL does not use `import std` (§22).

**`test/checks`** gained 9 programs:

| Program | `off` | `on` |
|---|---|---|
| `overflow.c` | `13! = 1932053504` | `signed integer overflow: 479001600 * 13 ...` |
| `shift.c` | `1` | `shift exponent 32 is too large for 32-bit type 'int'` |
| `array-index.c` | `6` | `index 3 out of bounds for type 'int[3]'` |
| `null.c` | `0` | `member access within null pointer of type 'struct node'` |
| `uninit-pointer.c` | `42` | `store to uninitialized pointer 0xffffffff of type 'int'` |
| `uninit-string.c` | a null byte | `invalid memory access through an uninitialized pointer` |
| `float-cast.c` | `2147483647` | `8.1e+09 is outside the range of representable values of type 'int'` |
| `optional.cpp` | `4352` | `optional operator* called on a disengaged value` |
| `string-index.cpp` | `abc` and garbage | `string index out of bounds` |

`test/student` (the gate for `on`, §9 of the plan) did not change. There is
no report in the 20 correct programs, at any level.

**The cuts.** The corpus at the levels now generates the checks, and
`make learn` gave back to `sc.wasm` only 1 object (`NSAPI.cpp`) and 2
symbols (`SanitizerMetadata::disableSanitizerForGlobal` and
`hasBranchWeightOrigin`). `sc.wasm` has 19,668,486 bytes. The DWARF of F0
waits for F2b (§33).

Also, `int-min.c` and `vector-index.cpp` changed. At `on`, the first says
`division of -2147483648 by -1 cannot be represented in type 'int'`
(UBSan catches it before the trap), and the second says
`vector[] index out of bounds`. At `off`, `v[5]` now prints 0 silently
(`NONE`).
## 33. DWARF in `sc.wasm` (F2b of docs/plan-checks.md)

The session always compiles with `-gline-tables-only` (§31), but the corpus
did not use `-g`, and `learn` had cut the DWARF code. The cut `sc.wasm`
stopped with `unreachable` when it compiled with `-g`. The REPL did not
notice, because it uses `sc-engine-full`.

Now the corpus compiles with `-gline-tables-only`, like a session, the
variants that play the session's role: the `pic-` ones, the `repl-` ones and
the levels (`-on`, `-strict`). The programs without a suffix still go
without `-g`, so that the code generated without DWARF stays covered.

`make learn` brought back about 155 symbols and no object. They are clang's
`CGDebugInfo` and the AsmPrinter's DWARF (`DwarfDebug`, `DwarfCompileUnit`,
`DwarfUnit`, `DIE`...). `learn-native` changed nothing. `sc.wasm` went from
19,668,486 to 19,879,645 bytes (+211 KB, +1.1%), below the ceiling of 0.8 MB
that the plan estimated (§6). An object compiled with `-g` in `sc.wasm` is
the same as the one from `sc-full.wasm`.

`make test`, `test-warn` and `test-native` gave 0 differences, and
`checkinv` gave SAME.

## 34. ASan (F3 of docs/plan-checks.md)

In `on` and in `strict`, `sc` now catches memory errors. These are the
overflow of a block, of a local variable, of a global or of a VLA, the use
after `free` or after the scope ends, a double `free` or one of
something that did not come from `malloc`, and a vector that reallocates
under a reference. The report goes out through F2's `__sc_rt_report`, with
the student's frames:

```
error: heap-use-after-free: read of 4 bytes, 4 bytes into a freed block of 8 bytes
    at free_list (use-after-free.c:12:22)
    at main (use-after-free.c:24:5)
```

**The flags** (`checks()` and `ASAN` in `flags.py`):
- `-fsanitize=address` together with UBSan;
- `-fsanitize-address-use-after-scope`;
- `-fsanitize-address-use-after-return=never`. The fake stack would need a
  frame allocator in the runtime, and the case is rare in student code
  (`-Wreturn-stack-address` is already an error);
- `-fsanitize-address-globals-dead-stripping`, as the driver does on wasm;
- `-mllvm -asan-use-stack-safety=0`. With the analysis, the pass stops
  instrumenting the accesses the analysis proves safe, and at -O0 the
  analysis costs more than it saves;
- `-mllvm -asan-mapping-offset=0x110000`, the layout below.

`sc_inv` accepts these flags, and with ASan it sets `DiscardValueNames` to
false, as cc1 does. The variable names go into the frame description, and
the report uses them. `sc_backend` runs `AddressSanitizerPass` after the
coroutines, at the same point in the pipeline as clang. `checkinv` gives 20
SAME, and the engine's objects are the same as those of the pin's clang.

**The memory.** The stack comes first, in [16, 1 MiB). The *shadow*, of
512 MiB (one byte for every 8 of the 4 GiB), starts at 0x110000. The program
comes after it, at 0x20110000. With offset 0, the *shadow* would fall on the
stack and the data. With the stack first, the shadow fits right after it.
`__asan_init` poisons the *shadow* of the null page (`fe`), and sc reports an
access through a null pointer as such. The loader reserves the *shadow* when
the first group exports `__asan_init`. The corpus's static programs link
with `--stack-first` and `--global-base` (`rt_static()` in `flags.py`).

**The runtime** is `rt/sc_asan.c`, in `libsc_rt` (about 700 lines):
- the pass's callbacks (`__asan_report_*`, `__asan_set_shadow_*`, the
  globals, the `alloca` calls, `__asan_handle_no_return`);
- the allocator. Each block has on its left a red zone with a header of
  32 bytes and on its right a red zone that grows with the block, like
  compiler-rt's. A freed block waits in a FIFO quarantine of 64 MiB
  before it goes back to libc;
- the annotations of libc++'s containers (`vector` and `deque`);
- the interceptors that check the arguments: `strcpy`, `strncpy`,
  `strcat`, `strncat`, `sprintf`, `snprintf`, `fgets`, `fread`, `read`,
  `getline` and `getdelim`, besides `memcpy`, `memmove` and `memset` (with
  the overlap in `memcpy`). The `scanf` family came in F4 (§39);
- `__cxa_throw` and `__cxa_rethrow`, which clear the *shadow* of the
  abandoned stack before they throw.

**The `--wrap` names.** The runtime replaces a libc function with the name
`__wrap_X` and calls the original as `__real_X`, like wasm-ld's `--wrap`.
The static programs link with `--wrap=X`. The loader does the same
resolution for the REPL's objects. An import of `X` goes to `__wrap_X`, if
some module defines it, and `__real_X` goes to `X`. Libc calls its own
`malloc` and does not go through the wrap in the REPL. A block that libc
allocates (`strdup`) has no header, and `free` recognizes it and returns it
directly. The runtime does not call `snprintf`, because that would be its
own wrap. It uses a `format()` on top of `vsnprintf`.

**The REPL.** After a trap, the machine clears the *shadow* of the abandoned
stack (`__asan_unpoison_memory_region` from 16 to the top). The REPL
registers the globals of each cell when it loads the object. A session goes
on after a report (`test/checks/heap.c.repl`).

**A defect of ASan with wasm exceptions**
(`llvm/asan-cleanupret.patch`). The pass treats every `cleanupret` as an
exit from the function, and at each one it rewrites the *shadow* of the
whole frame, even when the exception goes to another cleanup block of the
same function. At -O0, with libc++'s `always_inline` functions inside
`main`, the frame of a program with `std::format` has 2.4 KB of *shadow*,
rewritten in 180 cleanup blocks. `make learn` timed out on `c18_cxx23.cpp`.
The patch (4 lines) counts only the `cleanupret` that leaves the function.
Besides the size, the old behaviour removed the red zones of a frame that
was still live. On `c18_cxx23.cpp`, in `on`, with the pin's clang:

| | UBSan only (F2) | ASan, without the patch | ASan, with the patch |
|---|---|---|---|
| code | 545 KB | 2,516 KB | 1,103 KB |
| `main` | 27 KB | 1,470 KB | 82 KB |
| compilation | 5.9 s | 9.5 s | 6.6 s |
| 1st run (wasmtime compiles) | — | 6.0 s | 2.1 s |

Without *use-after-scope*, `main` without the patch would still have
368 KB. What remains with the patch is the normal cost of ASan at -O0.
That is about 2x the code of each function.

**The cost**, the best of 3, with `sc`:

| | `off` | `on` |
|---|---|---|
| `sc c3_stl.cpp` | 3.57 s | 4.12 s (+16%) |
| `sc c1_basics.cpp` | 1.89 s | 2.12 s (+12%) |
| `sc c18_cxx23.cpp` | 5.41 s | 9.31 s (+72%) |
| `sc p1.c` | 0.106 s | 0.118 s |
| REPL: 42 small cells with `vector`, `map` and `string`, per cell | 68 ms | 115 ms |

F2 measured 61 ms against 102 ms per cell on a less loaded machine. ASan
adds little to UBSan in the REPL. A cell stays near 0.1 s.

**The cuts.** `make learn` brought back to `sc.wasm` the object
`AddressSanitizer.cpp` and 2 symbols (`AddressSanitizerPass::run` and
`removeASanIncompatibleFnAttributes`). `sc.wasm` went from 19,879,645 to
19,957,337 bytes (+78 KB, +0.4%). `learn-native` changed nothing.

**The rule-of-three warning** (`-Wdeprecated-copy-with-user-provided-dtor`)
moved from A to S, as the plan foresaw (§4). ASan catches the real double
`delete[]` (`test/checks/rule-of-three.cpp`). In `test/student`,
`rev-dtor.cpp` no longer warns in `on`.

**The tests.** `test/checks` gained 12 programs, one per kind of error.
Each one gives the report with the student's lines in `on` and in `strict`.
`test/student` still has no report in the 20 correct programs, at any level.
`make test`, `test-warn`, `test-native` and `test-split` gave 0
differences.

**Left for later:**
- the stack of the allocation and of the `free` in the report. This needs a
  host function that returns the current frames (an F3b, done in §36);
- the leaks at the end of `sc FILE` (F4);
- libc is not instrumented. An overflow inside one of its functions that
  has no interceptor goes unseen.

An old defect, from before F3. In `sc FILE`, a program with `setjmp` and
`longjmp` fails to link with `undefined symbol: __c_longjmp`, in `off` as
well. §35 fixed it.

## 35. `__c_longjmp` in `sc FILE`

`sc FILE` links each cell with `--unresolved-symbols=report-all`, and a
program with `setjmp`/`longjmp` failed at every level with
`undefined symbol: __c_longjmp`. That is the wasm exception *tag* that
`longjmp` throws. wasi-sdk's `libsetjmp.so` imports the *tag* and does not
define it. The REPL did not see the defect. It links with `import-dynamic`,
and the loader creates the *tag* that no module defines.

Relinking `libsetjmp.a` with our wasm-ld was not enough. `libsetjmp.a`
defines the *tag* as weak, and `dylink-weak.patch` makes wasm-ld export it.
The library, however, throws its own local *tag*, and not the one the
loader gives the other modules, so `setjmp` did not catch the `longjmp`.

**The fix:** `rt/c_longjmp.s` defines the *tag* as strong, in assembly, and
the Makefile rebuilds the sysroot's `libsetjmp.so` from it and from
wasi-sdk's `libsetjmp.a` (`$(SR)/.setjmp`). The functions stay exported with
`--export`, because they are hidden in the object. `test/student` gained
`errors.c`, a calculator that handles errors with `setjmp`/`longjmp`, and it
runs at all three levels.

## 36. The block's stack in the ASan report (F3b)

A heap report now also says where the program freed the block and where it
allocated the block:

```
error: heap-use-after-free: read of 4 bytes, 4 bytes into a freed block of 8 bytes
    at free_list (use-after-free.c:12:22)
    at main (use-after-free.c:24:5)
the block was freed
    at free_list (use-after-free.c:11:9)
    at main (use-after-free.c:24:5)
and allocated
    at main (use-after-free.c:19:26)
```

**The pieces:**
- **The runtime.** The block's header of 32 bytes uses two of its free
  fields for the numbers of the stacks of the allocation and of the `free`.
  `sc_asan.c` imports two weak functions from the host:
  - `__sc_rt_stack()` stores the caller's stack and returns a number for
    it;
  - `__sc_rt_blame(alocado, liberado)` comes before `__sc_rt_report`. It
    gives the block's stacks in heap access errors, in a double `free` and
    in the `free` of an address in the middle of a block.

  Without `sc`'s loader, as in the corpus, the number is 0.
- **The loader** (`Loader::keep_stacks`). It captures a `WasmBacktrace` and
  stores each stack once, keyed by the frames' (module, offset). `machine.rs`
  shows the stacks with the same filter as the error's frames.

**The cost, and what solved it.** The programs are a list of 300 thousand
nodes (`malloc` and `free` of each) and a `std::map<string, int>` of 100
thousand keys, in `on`:

| | list | `std::map` |
|---|---|---|
| without the stacks | 0.49 s | 4.61 s |
| with wasmtime's `wasm_backtrace_details`, 2^16 frames | 10.0 s | 1,529 s |
| without the details, 2^16 frames | 2.74 s | 22.7 s |
| the final form, with no block limit | 2.32 s | 11.3 s |
| **the final form** (20 thousand blocks) | **0.36 s** | **4.74 s** |

The time of the final form comes from another run. There `off` gave 0.22 s
and 3.45 s.

Three things weighed:
- **Symbolization.** With the details on, wasmtime 48 rebuilds the
  addr2line context from DWARF at every frame of every capture.
  - Now the details stay off, and the loader symbolizes by itself
    (`Loader::symbols`, with the `addr2line` and `gimli` crates in
    wasmtime's versions).
  - It keeps each module's `.debug_*` sections and builds the context once
    per module, at the first lookup.
  - Symbolization happens only when a report is shown.
  - The error's frames also moved here, and `test/checks` gave the same
    lines in the 108 cases.
- **`max_frames`.** Each capture reserves a vector of `max_frames`
  entries. With 2^20, the vector goes past the limit of 32 MB under which
  glibc still reuses the heap, and each capture goes to `mmap`.
  - It went to 2^15. A frame takes at least 16 bytes of the 512 KiB of
    `max_wasm_stack`, so 2^15 frames still cover the whole stack, and the
    `main` of a recursion does not get lost (§31).
- **The capture.** Even so, each one costs a few microseconds.
  - Only the first 20 thousand blocks keep the stack (`STACKS` in
    `sc_asan.c`), and `free` keeps it only for a block that has the
    allocation's stack.
  - A student program almost never goes past that, and one that does only
    loses the stacks of the blocks after the limit. The report stays the
    same.

**The tests.** `test/checks/expected.txt` gained the stack lines in the 7
programs with a heap error, in `on` and in `strict`. In the REPL, the stack
points to the cell (`at cell_4:1:5`).

## 37. Leaks (F4)

In `strict`, `sc FILE` reports at the end the blocks that the program lost,
and a status 0 becomes 1. The REPL's `:leaks` does the same at any moment,
in `on` and in `strict`. `off` does not load `libsc_rt`, and without it no
allocator knows the blocks.

```
error: memory leak: 24 bytes in 3 blocks, never freed
the blocks were allocated
    at insert (leak.c:10:22)
    at sum (leak.c:19:16)
    at main (leak.c:29:20)
```

**What counts (decision of 2026-10-16).** A leak is a block that nothing
reaches, as in LSan. A list kept in a global until the end is not a leak,
and neither is what libc++ allocates for itself, which its globals reach.
So there is no need to filter by the student's frame, as the plan foresaw.

**The pieces:**
- **The runtime.** The block's header gained the field `prev` in place of
  `pad`, and the live blocks sit in a doubly linked list. `next` serves the
  list and, after `free`, the quarantine.
  - `__sc_rt_leaks(sp)` sorts the live blocks by address and scans as
    roots the words of the stack above `sp` and those of the modules' data.
  - A value that falls in `[begin, end)` of a block reaches it, even when
    it points to the middle. The scan covers the reached blocks in turn.
  - The lost ones go to the loader through `__sc_rt_leak`, grouped by the
    allocation's stack.
- **The loader.**
  - It calls `__sc_rt_leaks` after `exit`. By then the `atexit` handlers
    and the destructors of the C++ globals have run, and the stack that
    remains is the one of `exit`. If the student called `exit` from inside
    a function, that function's frames are still there, and a pointer in a
    local variable still reaches the block, as in LSan.
  - After a runtime error, an `abort` or an uncaught exception, there is
    no report.
  - It shows at most 10 groups, the ones with the most bytes first.
  - The blocks after the 20 thousand that keep the stack (§36) form a group
    without it, and the report says so.

**The roots in the data: only what changed.** The first version scanned
the whole data of each module.
- A lost list of 100 thousand nodes gave 9,737 lost blocks in one run and
  55,656 in another.
- The heap starts right after the data of `libc.so`, near `0x2014_0000`,
  and some constants in its `.rodata` (`0x20152014`) fell in the middle of a
  node. From there the whole list counted as reached.
- Skipping `.rodata` by name is not enough. `libc.so` names its segments,
  but `libc++.so` has no `name` section.
- So the loader keeps the image of each module's data after the
  relocations and before the constructors, without the trailing zeros (the
  `.bss`).
  - Only the words that changed since then are roots.
  - A pointer to the heap only gets into the data through a write during
    the run, and a constant never changes.
  - With this, the 100 thousand blocks show up, and the two groups (20
    thousand with the stack and 80 thousand without) are the expected ones.

**The cost.** The measurement swapped only `libsc_rt.so` (HEAD's against the
new one), in `on`, alternating the two, with the lowest of 4 times. The
machine was loaded, so only the comparison counts:

| | list of 300 thousand nodes | `std::map` of 100 thousand keys | 100 thousand lost blocks |
|---|---|---|---|
| without the list of live blocks | 0.23 s | 4.48 s | 0.16 s |
| with it | 0.24 s | 4.50 s | 0.15 s |

The scan in `strict` cost about 0.1 s on the 100 thousand lost blocks
(0.27 s against 0.17 s in `on`), and nothing on the other two programs.
They end with little on the heap.

**The tests.** `test/checks` gained four programs:
- `leak.c`, a list lost inside a function;
- `leak-global.c`, a list in a global, which gets no warning;
- `leak.cpp`, a `new[]` without `delete[]` next to `std::map`,
  `std::string` and `std::vector`, with no false positive from libc++;
- `leaks.c.repl`, `:leaks` before and after the block is lost.

`test-student` stayed the same. None of the 21 correct programs loses memory.

## 38. libc++'s `DEBUG` in `strict` (F4)

`strict` moved from libc++'s `EXTENSIVE` hardening to its `DEBUG` mode
(`HARDENING` in `flags.py`). `on` stays at `EXTENSIVE`, and `off` at `NONE`.

```
error: Comparator does not induce a strict weak ordering
    at main (sort-cmp.cpp:7:5)
```

**What `DEBUG` adds.** In the pin, it adds three things:
- **the comparator:** `__debug_less` wraps the algorithms' comparator and,
  when `comp(a, b)` is true, checks that `comp(b, a)` is false;
- **the ordering:** after `sort`, `__check_strict_weak_ordering_sorted`
  checks that the range came out sorted and tests every pair of the
  first 100 elements;
- **libc++'s internal assertions** (`_LIBCPP_ASSERT_INTERNAL`).

The student's classic error, `<=` as the comparator of `std::sort`, passes
in silence in `on` and stops the program in `strict`.

**Nothing else changes.** `DEBUG` exists only in the headers:
- the sysroot's `libc++.so` stays with `fast`;
- the semantics stay `enforce`, the sysroot's;
- the message arrives through the same `__libcpp_verbose_abort` of
  `libsc_rt` (§32).

The corpus already builds one `std.pcm` per level (`std-pcm-strict`), and
`checkinv` stays SAME, because it already accepted `-D`.

**The cost** lies in the `sort` calls of up to 100 elements, because of the
tested pairs. Measured with `sc`, the lowest of 3 times, compilation
included and the machine loaded:

| | `on` | `strict` |
|---|---|---|
| one `sort` of 1 million `int` | 1.58 s | 1.64 s |
| 20 thousand `sort` calls of 100 `int` | 3.38 s | 6.21 s |
| `std::map` of 100 thousand keys | 5.03 s | 4.72 s |

**The tests.** `test/checks` gained `sort-cmp.cpp`, a `sort` with `<=`.

## 39. `scanf` (F4)

`libsc_rt` now intercepts `scanf`, `fscanf`, `sscanf` and the versions with
`va_list`. Like the other interceptors, it applies in `on` and in `strict`
(the names are in `WRAP` in `flags.py`):

```
Name and age: error: stack-buffer-overflow: write of 12 bytes by scanf, 0 bytes past the end of the variable 'name' (8 bytes)
    at main (scanf.c:7:9)
```

**How it checks.** The write happens inside libc, which is not
instrumented. So the interceptor reads the format and checks the pointers
at two moments.

*Before the call*, what the format gives the size of:
- the numbers, by the modifier: `%d`, `%hd`, `%lld`, `%f`, `%lf`, `%p`, `%n`;
- `%c`, with the width or 1;
- `%ms`, a `char *`.

The interceptor checks `%9s` and `%9[` against the space the program says
it has, 10 bytes, like `fgets`, and the report says "write of up to".

*After the call*, `%s` and `%[` without a width, by what was read. That is
the `strlen` plus 1. The interceptor checks only the conversions that the
return value counts. compiler-rt's ASan does the same.

**The limits.**
- A `%s` without a width has already written when the report comes out. The
  program stops right after, before it uses the corrupted memory.
- A positional argument (`%1$d`) stops the check.
- The interceptor reports a null pointer before the call, even if the input
  ends before the conversion. It is always an error.

**`%9s` into an array of 4** does not even get to run in `on`.
`-Wfortify-source` is in E (§4.1 of the plan) and gives a compilation
error. So the test uses a heap block, which clang does not see.

**The tests.** `checks.py` now gives the program the input of
`NAME.c.in`, when that file exists. `test/checks` gained:
- `scanf.c`, a `%s` into an array of 8 with "Maximiliano";
- `scanf-width.c`, a `%9s` into a block of 4 bytes, with a short input.

In `off`, `scanf.c` corrupts the stack and exits with an invalid status.

## 40. U-strict (F4)

`strict` gained the UBSan checks that the plan called U-strict (§5.1 of
the plan). They catch what is defined behaviour but is rarely what the
student meant:

| Check | Catches |
|---|---|
| `unsigned-integer-overflow` | `balance - withdrawal` with `withdrawal > balance`, `size_t i = n - 1` with `n = 0` |
| `unsigned-shift-base` | `1u << 31 << 1` |
| `implicit-conversion` | `short p = 50000`, `uint8_t b = 250; b += 10`, `unsigned u = -1` |
| `float-divide-by-zero` | `sum / n` with `n = 0` in `double` |

`implicit-conversion` is a group. `-cc1` accepts the checks only one by
one, so `U_STRICT` in `flags.py` expands it as the driver does:
`implicit-unsigned-integer-truncation`, `implicit-signed-integer-truncation`,
`implicit-integer-sign-change` and `implicit-bitfield-conversion`.

`libsc_rt` gained the conversion handler, with compiler-rt's words:

```
error: implicit conversion from type 'int' of value 50000 (32-bit, signed) to type 'short' changed the value to -15536 (16-bit, signed)
    at main (truncation.c:5:23)
```

The other three already had a handler. These are the overflow handler
(which says "unsigned"), the shift handler and the division handler (which
says "division by zero").

**The headers.** Measured with a harness that recovers and counts each
report:
- `std::shuffle` with an `mt19937` fired 293 times, all in
  `mersenne_twister_engine.h:318` (`unsigned-shift-base`).
- `std::hash` does not fire. libc++ already marks murmur with
  `no_sanitize`.

The wraparound is intentional in both cases. So `strict` passes
`-fsanitize-ignorelist=/sysroot/share/sc/ignorelist.txt`, which comes from
`rt/ignorelist.txt`:
- It removes `src:*/sysroot/include/*` only in the sections of the seven
  U-strict checks. ASan and U stay on in the headers' code.
- The pattern with a leading `*` matches both paths. These are the engine's
  `/sysroot` and the real path that the REPL uses.
- `sc_inv` now accepts the flag (`LangOptions::NoSanitizeFiles`).
  `checkinv` gives SAME and the cut engine did not need `make learn`.

**The `ctype.h` macros.** musl defines `isdigit`, `isalpha`, `islower`,
`isupper`, `isprint` and `isgraph` (and `iswdigit`, in `wchar.h` and in
`wctype.h`) as `((unsigned)(a)-'0') < 10`. The macro expands in the
student's code, where the ignorelist does not apply, and `isdigit(' ')`
stopped `test/student/errors.c`, which is correct. The Makefile's
`$(SR)/.ctype` step deletes these macros from the sysroot's headers. They
become libc's functions, with the same result, at every level. `isspace`
uses `__isspace`, a function from the header, which the ignorelist already
covers.

**What stops in the corpus.** Of the 1951 runs in `strict`, 179 stop on
U-strict:
- 174 gcc torture tests;
- 3 from c-testsuite;
- `p2.c` (a djb2 hash, `h * 33 + c`) and `p5.c` (`uint8_t by += 10`).

None stops inside a header. The torture tests wrap around on purpose.

The hash in `p2.c` is the case that costs. A correct program stops in
`strict`. This was a decision. `strict` is the demanding level, like the
`-Wconversion` and `-Wsign-conversion` that it already turns on. Whoever
wants the wraparound writes the cast or uses `on`.

**The cost** is the size of the object in `strict`, without and with
U-strict:

| Program | without | with | |
|---|---|---|---|
| `student/matrix.c` | 13,925 | 13,925 | 1.00× |
| `student/text.c` | 14,365 | 15,341 | 1.07× |
| `student/students.c` | 13,272 | 14,312 | 1.08× |
| `student/vector.cpp` | 341,016 | 342,695 | 1.00× |
| `student/graph.cpp` | 426,604 | 428,430 | 1.00× |
| `cxx/c1_basics.cpp` | 321,533 | 322,100 | 1.00× |

C++ barely changes. The headers' code stays out.

**The tests.**
- `test/checks` gained `unsigned.c`, `truncation.c` and `float-div-zero.c`.
  In `off` and in `on` they print the value that wrapped around. In
  `strict`, they print the report.
- `test/student` gained `draw.cpp`, correct code with `mt19937`,
  `shuffle`, `uniform_int_distribution` and an `unordered_map<char, int>`
  with "Fábio". It must not stop in `strict`.

## 41. libc's reads (F4)

The plan left the instrumented libc as optional (§5.2 of the plan), to
catch what libc reads and writes on its own. Today, a string without the
`'\0'` passes in silence through `strlen`, `printf("%s")`, `strcmp` and
`puts`. It is a common student error: `char name[5] = "Maria"`.

**Decided: more interceptors, not a new libc.** Building wasi-libc with
ASan would require:
- the source and a new build;
- two libcs in the sysroot, one per level;
- the loader and the corpus to choose between them.

The interceptors use the `--wrap` that already exists, as compiler-rt does.
`libsc_rt` now checks what these functions read:

| Functions | How far |
|---|---|
| `strlen`, `strnlen`, `puts`, `fputs`, `strrchr`, `strstr`, `strspn`, `strcspn`, `strpbrk`, `strtok` | the whole string, with the 0 (`strnlen`, up to `n`) |
| `strcmp`, `strncmp` | up to the first difference or the 0 |
| `strchr`, `memchr` | up to the character (C11 guarantees that `memchr` stops at it) |
| `memcmp` | the `n` bytes of both, like compiler-rt's `strict_memcmp` |
| `printf`, `fprintf`, `vprintf`, `vfprintf`, and the `%s` of `sprintf` and `snprintf` | the format, each `%s` up to the precision (`%.3s` reads at most 3), `%ls`. `%n` is a write |
| `atoi`, `atol`, `atoll`, `atof`, `strtol`... `strtold` | up to the character that ends the number, which libc gives in `endptr` |
| `strdup`, `strndup` | the string. The copy is now a block of the allocator, with redzones and in `:leaks` |

The earlier interceptors now also read the source string this way:
`strcpy`, `strcat`, `strncpy`, `strncat` and `sscanf`.

```
error: stack-buffer-overflow: read of 6 bytes by strcmp, 0 bytes past the end of the variable 'name' (5 bytes)
    at main (unterminated.c:6:9)
```

In C++, `std::cout << acronym` and `std::string(acronym)` call `strlen` in the
student's code, from the headers, and the interceptors catch them too.

**How it reads.** `libsc_rt` cannot call libc's `strlen`. The call would go
to `__wrap_strlen`, both in wasm-ld's `--wrap` and in the loader.

`read_str` walks the string as libc does, but it checks the *shadow* of
each granule before it reads:
- in a fully valid granule, it tests the 8 bytes at once for the 0, musl's
  trick;
- the report says how many bytes it read up to the first invalid one.

It is `always_inline`, and the frame's name loses the `__wrap_`. A pointer
outside the memory, as in `in-libc.c`, still gets reported `in strlen`.

**The cost.** A loop with `strlen` in the condition, over 20 thousand
characters (4·10⁸ bytes read):

| Version | `on` |
|---|---|
| without the new interceptors | 0.19 s |
| checking byte by byte | 0.78 s |
| 8 bytes at once | 0.19 to 0.25 s |

With 200 thousand calls of `snprintf`, `atoi` and `strcmp`, it stayed at
0.20 to 0.24 s, against 0.23 s before.

**The limits.**
- A function without an interceptor goes unseen: `fopen`, `getenv`, the
  `wcs*` functions.
- `strstr` and `strspn` check both whole strings, even if libc stops
  earlier.
- A call from inside libc goes unchecked (the `printf` that calls `strlen`,
  for example). The check covers the argument that the program passes.
- A positional argument (`%1$s`) stops the check of the format, as in
  `scanf`.

**In the corpus**, none of the runs in `on` and in `strict` gained a report
from these functions. `test/student` still has no report either.

**The tests.** `test/checks` gained:
- `unterminated.c`, a `strcmp` with `char name[5] = "Maria"`;
- `unterminated.cpp`, a `std::cout << acronym` of a `char[3]`.

In `off`, the first one says "not found" and the second prints "UEM" by
luck.

## 42. The pattern for uninitialized integers (§5.4 of the plan)

`on` and `strict` use `-ftrivial-auto-var-init=pattern`. On a 32-bit target,
clang's `PatternInit.cpp` uses 0xFF… for pointers, because only page 0 is
guaranteed to be invalid on every system. It uses the same value for
integers, for simplicity. An uninitialized `int` is then -1, and -1 looks
like a right value.

The compiler already catches an uninitialized scalar local
(`-Wuninitialized`, an error in E). What remains for the pattern is arrays,
struct fields and conditional initializations:

| Case | Correct | `on` with -1 | `on` with 0xAA… |
|---|---|---|---|
| `int votes[3]; votes[k]++` | 1 1 3 | 0 0 2 | -1431655765 -1431655765 -1431655763 |
| `sum[i] += 10 * (i + 1)` | 10 20 | 9 19 | -1431655756 -1431655746 |
| `struct student a; a.absences++` | 1 | 0 | -1431655765 |

With -1, the forgotten counter comes out one short, and `absences` comes out
0, which looks right.

**`llvm/pattern-init.patch`** (4 lines) separates the two values:
- integers and `char` values become 0xAA… on every target, as on 64-bit
  ones (-1431655766, `170`, `-21846`);
- pointers stay 0xFFFFFFFF. `libsc_rt` recognizes that value as
  "uninitialized pointer" (§32).

`double` values were already NaN and stay NaN.

It took a patch, not a cut, because it is a change in CodeGen's behaviour.
The cut engine did not need `make learn`.

**The code size does not change.** Measured with the clang without the
patch (`build/native`) against the engine with it, in `on`:
- `student/matrix.c`, `students.c`, `list.c`, `vector.cpp` and `graph.cpp` keep
  the same bytes;
- `cxx/c1_basics.cpp` gains 4 bytes.

The pattern's `memset` only changes the byte.

**The tests.** `make test`, `test-warn`, `test-native`, `test-split` and
`test-student` gave 0 differences. `test/checks` gained `uninit-count.c`, the
vote counter without `= {0}`. `uninit-pointer.c` still says "uninitialized
pointer".

## 43. `return` at the top of a cell (§3.7 of docs/plan-repl.md)

In a REPL cell, `return 1;` at the top, or inside an `if` at the top,
crashed the engine. `Sema::BuildReturnStmt` sees that there is no function
and returns `StmtError()` **without a diagnostic**. In a file this does not
happen, because the parser never accepts a statement outside a function.
With no error emitted, the cell goes on, the `TopLevelStmtDecl` ends up
with a null statement and CodeGen falls on it.

**`llvm/repl-return.patch`** (3 lines; today in
`repl-outside-function.patch`, §45) emits the error in that branch. It
reuses `err_coroutine_outside_function` ("'return' cannot be used outside a
function"), which already exists with that text. The cell fails like any
other with an error and the session goes on, in C and in C++. A cut would
not solve it. Code is missing, not left over.

The plan foresaw a diagnostic of our own before CodeGen, but there the
information about where the `return` is has already been lost. Sema is the
place to give the error.

**The tests.** `test/checks` gained `return.c.repl` and
`return.cpp.repl`. The `return` at the top and in the `if` give the error,
and the next cell shows the value. `uninit-count.c` (§42) printed stack
garbage in `off`, which changed with the new engine (4525, 4513). Now it
prints "impossible count" for a value outside 0..5, and the output stays
stable. `make test`, `test-native`, `test-split`, `test-student` and
`test-checks` gave 0 differences.

## 44. The end of the REPL's phase 2: §3.4 and §3.5 of the plan

**The unused warning (§3.4).** The plan wanted the warning only in the cell
that defines the declaration, filtered in the printer. §29 had already
solved it another way. The session empties Sema's lists at each cell, so in
the REPL the end of a cell warns about nothing, and `sc FILE`
(`-sc-one-cell`) warns like the compiler.

**Redefinition (§3.4).** Decided: no note from sc. clang's error already
points to the earlier definition, with its `cell_N`:

```
cell_4:1:5: error: redefinition of 'g'
cell_3:1:5: note: previous definition is here
```

`:undo` only undoes the last cell, so a note that suggests `:undo` would
only help when the earlier definition belongs to that cell. A
`:undo cell_N` (a replay without the cell, and without the cells that
depend on it) stayed out.

**The session's `checkinv` (§3.5).** The session builds the invocation with
`scBuildInvocation` and then forces four fields. These fields now live in a
function, `scIncremental`:
- `IncrementalExtensions`;
- CodeGen's `DisableFree` and Frontend's, turned off;
- `ClearASTBeforeBackend`, turned off.

This is what clang's parser builds with `-fincremental-extensions`, without
`-disable-free` and without `-clear-ast-before-backend`.
`-fincremental-extensions` is a pure flag. Only the `Preprocessor` reads it,
and it has no derived options.

`make checkinv` now runs the arguments of each of the 12 sessions
(`flags.py --session`: c and c++, each level, REPL and `-file`). It removes
the link part (after `--`) and the two flags, and adds
`-fincremental-extensions`. With that flag, `checkinv` calls
`scIncremental` on sc's side. All 12 give SAME. With `-disable-free` left
on the parser's side, the result is DIFFERENT, so the comparison sees these
fields. `TU_Incremental` stays out. It is not a cc1 flag.
## 45. The review of phase 2 of the REPL

A review of the phase 2 code (from `4b0325d` to `8425ca8`) found eight
problems.

**The engine left its temporary directory behind (`proto/src/child.rs`).**
The `Drop` of `Child` killed the engine with SIGKILL while the engine
waited on stdin. The session's destructor did not run, and its
`/tmp/sc-XXXXXX` stayed. This happened on every session, every `:undo` and
every `:safety`, and `/tmp` already held more than 31 thousand of these
directories. Now the `Drop`:
- closes stdin;
- waits for the engine to close stdout, up to 5 s;
- only then kills the process.

A cell that fails hands the session to the `fork` copy (§3.3), which is not
a child of the REPL. So the `Child` kills the **process group**. Before, the
copy became an orphan. After the process is reaped, the group may be gone
and its number free, so the `Child` marks `gone` and sends no more signals.
The `test-split` (10541 cells) and the other batteries now leave the number
of `/tmp/sc-*` unchanged.

**A local `static` and a lambda in a top-level statement.** In C++, the
cell `{ static int n = 5; }` crashed the engine. So did a lambda in a
top-level `for`, such as `for (...) [&] { ... }();`. The Itanium mangler
went up through the contexts to the `TopLevelStmtDecl` and tried to give it
a name, but it has none. In C the cell worked by chance. The name of the
`static` hit an `llvm_unreachable` in `getStaticDeclName`, which in release
is undefined behaviour. The `llvm/repl-return.patch` became
`llvm/repl-outside-function.patch`, with two more parts.

- **Sema (`ActOnVariableDeclarator`).** A `static` or `thread_local`
  variable whose context is a `TopLevelStmtDecl` is an error, with the same
  `err_coroutine_outside_function` as `return` ("'static' cannot be used
  outside a function"). A top-level statement has no function to give the
  variable a name. A `static` variable inside a function or a lambda still
  works.
- **Mangler (`ItaniumMangle.cpp`).** The `TopLevelStmtDecl` is a local
  container, like a function named `__stmts_N`, where N encodes the
  position of the statement. Each cell is a new file in the session's
  `SourceManager`, so N is unique in the session. N also repeats in the
  replay, because a cell that fails leaves nothing in the copy that goes
  on.

  The simpler alternative was to skip the context and treat the statement
  as if it were at the top of the file. We tried it and dropped it. Two
  local `struct S`, from different cells or blocks, collided in
  `_ZN1S1fEv`.

The loader removes `__stmts_N()::` from frame names, so an error shows
`at S::f(int)`. The `test/checks` gained three tests:
- `static-stmt.c.repl` and `static-stmt.cpp.repl`, with the error;
- `cells-local.cpp.repl`, with lambdas, a `static` in a lambda and repeated
  local `struct S`, ending in a trap inside the method.

**A slot without a function (`loader/src/lib.rs`).** A `&g` reserved the
slot of `g` (GOT.func) empty. A call to `g` in a later cell reused that
slot without putting in it the function that says "undefined symbol: g".
The error came out as "call through a null function pointer". Now
`reserve_slot` fills an empty slot when the call carries the type.

**A cell that fails in the loader (`machine.rs`).** A cell that the
compiler accepts and that fails later stayed in the compiler's log.
Decided:
- if the loader does not even instantiate the cell, the REPL undoes it in
  the compiler, and the student can rewrite it. `Loader::is_loaded` says
  whether the cell was instantiated;
- if the trap comes later, for example in an initializer, the cell stays.
  The names exist, and what the cell did stays done. A new `int y = 5;`
  gives "redefinition", and the student uses `:undo`.

We found no case that reaches the first branch. The wasm-ld already
rejects a signature that differs from libc's.

**`Repl::restart` was not atomic (`child.rs`).** It swapped the child
before the replay. A replay that diverged left a child with part of the
log, while the log had everything. Now the new child replaces the old one
only after the whole replay. If `:undo` fails, the cell goes back to the
log.

**`retypeDecls` before `scPromoteInternals` (`sc_session.cpp`).** The
`Types` keeps the exported names, such as `f.sc` for a `static f`, but the
declaration only gets that name at promotion. The order is now to promote
first and retype after. We did not reproduce the predicted failure. Clang
gives the call the type of the K&R definition, even in a later cell.

**The two `source_name`.** There were two, with different rules. One was
in `lib.rs` and the other in `machine.rs`. Now there is only one,
`scload::source_name`. It takes the symbol or the name that the wasm-ld
demangles.

**The tests.** With the patch, `make test` and `test-native` gave 0
differences in 17715 files, and the corpus does not change. `test-split`,
`test-student` and `test-proto` passed, and `test-checks` gained only the new
entries.

## 46. Phase 3 of the REPL: the session's slices

The slices of §6 of docs/plan-repl.md went into `test/session.py`
(`make test-session`), not into `corpus.py`. They run on
`sc-engine repl`, which exists only in the native build, and `corpus.py`
runs any one-shot engine. The script takes the engine and a base, so
phase 4 runs the same script on the cut engine, against the
`summary.json` of the uncut one. The list of corpus files moved to
`corpuslib.sources()`, which both scripts use.

- **`ses-`.** Each corpus source (11,867) is the only cell of a session.
  The cell is accepted if and only if the whole source compiles on the
  native base (the `pic-` of a program), and a program gives the output of
  the `pic-`. A source that the corpus only compiles can fail at link. The
  session links each cell, and the corpus does not link those sources. The
  session runs in the source's directory, where its `#include "..."` files
  are.
- **`seq-`.** Fixed sequences in `test/seq/`, with the output in
  `test/seq/expected.txt`:
  - an error and then a good cell;
  - `undo`;
  - `%restart` (a new child that replays the log; `sc-engine repl` gained
    the step, and `Repl::restart` became public for the valve of §4);
  - `static` and an anonymous namespace across cells;
  - `import std;` after `#include`;
  - cells that end inside a block;
  - the nested private class.
- **`crash-`.** The 40 sources on which the one-shot crashes or gives a
  fatal error (an exit code that is neither 0 nor 1 on the native base). 29
  are fatal errors (70), 3 are crashes of `#pragma clang __debug` (132) and
  8 are stack overflows (139). Each one goes between two good cells, and
  then come a `%restart` and a cell with `main`. The session goes on in all
  40, the replay exports the same names, and the program prints 42. In the
  session, 39 really crash ("the engine crashed on this cell"). The `#pragma clang __debug
  parser_crash` does not reach the parser in a cell and gives "expected
  expression".
- **`mem-`.** 1,000 small cells in one session, in C and in C++. After the
  first 100, the slope of the RSS is 2.6 KB per cell in C and 5.2 KB in
  C++, below the plan's 50 KB. The largest rise over 100 cells, each one a
  link, is 440 KB and 860 KB, below 5 MB. Each cell takes 12 ms in C and
  19 ms in C++. The `memory.size` waits for the reactor (phase 5).

**What `ses-` found.** On the first run, 130 sources gave a result
different from the whole compile. Three were bugs of clang-repl, each with
a patch.

- **A cell that ends inside a bracket hangs the engine
  (`llvm/incremental-input-end.patch`).** `int f() {`, `(x + 1;` or an
  unclosed `_Pragma(` left the engine in an infinite loop. This held for 25
  corpus sources and for the student too. `sc arquivo.c` with a missing `}`
  hung, and so did the REPL fed from a pipe. In incremental mode, the end
  of an input is the `annot_repl_input_end` token, not `eof`, and many loops
  of the parser and the preprocessor stop only at `eof`. The parser's loop
  does not consume the token, so it spins in place. The patch has two
  parts:
  - **Parser.** The end of the input with an open parenthesis, bracket or
    brace (`ParenCount`, `BracketCount`, `BraceCount`) becomes `eof`. The
    loops stop, and the error is clang's own: "expected '}'" with the note
    "to match this '{'". The counts go back to zero at the end of each
    input (`ParseTopLevelDecl`), so the next cell starts outside any
    bracket, with or without the `fork`. The engine skips an `eof` at the
    start of a cell. That token is the end of an earlier cell that ended
    inside a bracket. The raw lexer's depth test would not work for the
    decision, because it counts the braces of `#if 0` and of `#define`.
  - **Preprocessor.** The loops that stopped only at `eof` now stop at the
    end of the input too. These are the loop of `_Pragma(` (`Pragma.cpp`),
    the loop over the arguments of a macro `F(` and the loop of
    `__has_c_attribute(`, `__has_warning(` and the other `__has_*(`
    (`PPMacroExpansion.cpp`). Upstream already does this in
    `LexTokensUntilEOF`. As a safety net, the end of the same input read
    65,536 times in a row becomes `eof`, for a loop that the corpus does
    not exercise.

  The first version of the net gave `eof` on the second read already and
  broke the REPL. The normal flow reads the end again a few times, for
  example in `ConsumeAnnotationToken` after a printed value and in the
  `Parser` that the engine creates after `ParseAST`. The second version had
  the limit at 64 and was what produced the errors of these loops. In the
  review (`/simplify`), the loops started to stop on their own. With the
  limit removed entirely, `ses-`, `seq-` and `crash-` had no timeout,
  which means no corpus source depends on the net.
- **An anonymous namespace per cell
  (`llvm/incremental-anon-namespace.patch`).** Each input has its own
  `TranslationUnitDecl`, and Sema kept the anonymous namespace in it. So
  each cell had a new anonymous namespace. A repeated
  `namespace { int x = 9; }` was accepted, and `x` was ambiguous from then
  on. Now the anonymous namespace is the one of the first TU, as in a file,
  and the repeated `x` is a redefinition. The change is in the accessors of
  `TranslationUnitDecl` (`getFirstDecl()`, as `NamespaceDecl` already does
  for its own), and not in Sema. That way it holds for the ASTReader, the
  ASTWriter and the ASTImporter too.
- **Access in the guess between statement and declaration
  (`llvm/incremental-access.patch`).** At the top of a cell, clang-repl
  parses the start in tentative mode to decide whether it is a statement or
  a declaration. The branch for `identifier` and `::` read the scope
  `B::C::` without suspending the access check, so `B::C::C() {}` gave
  "'C' is a private member of 'B'". The `default` branch just below already
  did this. This is common in student code, such as `struct List::Node`
  defined outside the class. Now the branch does what `default` does. It
  suspends the check and goes back with the tokens unannotated, and the
  access is checked in the context of the declaration. Using `List::Node`
  outside the class still gives an error.

**The harness.** On a timeout, `subprocess.run` killed `sc-engine repl`,
but its children, in their own process groups, kept running. There were 55
processes after the first run. `corpuslib.run_all` runs the command in its
own session (`setsid`) and, on a timeout, kills every process of the
session. It repeats the scan of `/proc` until it finds none alive, because
a process can start during the scan and hold stdout. `split.py` uses it
now too. The slices read and write the sources as bytes. In text mode,
Python turned `\r\n` into `\n`, and 11 corpus sources have CRLF. No result
changed, but now the cell is the source that the base compiled. In `mem-`,
a child that dies or hangs (600 s) is a failure of the slice and does not
bring down the script.

**What cannot be equal (the script's `KNOWN`, 39 sources).**
- `__FILE__` in `#include`, `#embed` or `__has_embed` names the cell, which
  is not a file (10).
- A module declaration cannot be in a cell (13).
- An expression at the top level is a statement in a cell (5).
- The source's directory holds what `../` names, and the corpus maps only
  the source (5).
- A check at the end of the TU, which the session never gets to (2):
  - "inline variable is not defined";
  - "used but not defined in this translation unit".
- `#pragma clang module build` writes to `/tmp`, which the corpus does not
  give the engine (1).
- The wasm-ld fails on the shared object, outside the session too (2):
  - a segfault, with an annotation on an undefined function;
  - "duplicate symbol name", in an `"X"(f)` operand of `asm`. In the
    session it is a fatal lld error, which exits the process. The `fork`
    copy answers that the engine crashed.
- `void (*__funcref f)();` is taken as a statement (1).

`pr29160` and `mangle-ms-back-references-pr13207` do not run. Their
demangled names grow exponentially, and the wasm-ld demangles the names
for the name section on every cell link. The corpus only compiles these
two.

**The batteries, with phase 3 done.**
- `make test` and `make test-native`: 17,715 files, 0 differences.
- `make test-split`: 1,947 programs and 10,541 cells, 0 differences (6
  known).
- `make test-student` and `make test-checks`: as in `expected.txt`.
- `make test-proto`: passes. `make checkinv`: SAME.
- `make test-session`: 11,917 results, 0 bad (39 known).

With the machine busy with the other slices, `mem-` measures 30 ms per
cell in C and 40 ms in C++. The slope is 2.6 KB and 6.6 KB per cell, and
the largest rise over 100 cells is 404 KB and 1.8 MB. The plan's limits
are still far away.

After the review (`/simplify`), with the preprocessor's loops fixed and the
net at 65,536, every battery gave the same, with 0 differences. The
patch's `LexTok` is inline in `Parser.h`. Out of line, in `Parser.cpp`,
`sc.wasm` has the same 19,953,807 bytes. The inline costs nothing, and it
stays as it is.

## 47. Phase 4 of the REPL: the session on the cut engine

The REPL used `sc-engine-full`, without cuts. Now it uses
`build/sc-engine`. This is the same `proto/src/bin/sc-engine.rs`, linked
with `link.py --native` (the usual cuts) and with `sc_session.o` and
`sc_api.o`, with `llvm-strip`. It is 26.1 MB, against 75.5 MB for
`sc-engine-full` (37.2 MB before the strip).

**Where to learn.** The plan had `make learn` drive `sc-engine repl`. But
`sc.wasm` has no session before phase 5 (the reactor), and the wasm learn
runs `corpus.py` on a one-shot engine. So the session learns on the native
build, like `learn-native`. `make learn-session`
(`scripts/learn_session.py`) links the cut engine with the session and
runs the slices, and each cut function that the engine calls
(`sc: cut function called: ...`, `sc_cuts.cpp`) goes into the list. The
slices that differ run again until they pass, and then all of them run
once more. The learn's slices:
- `ses-`, `seq-` and `crash-` with `-Weverything`, against the same
  `session.py` on `sc-engine-full` (`build/res/session-wev-base`);
- `split-`, against `pic-`.

The list is `cuts/symbols.session.used`, with demangled names. `link.py`
compares it by the same normalized name as in §19 (`long long` and
`std::__2` against `long` and `std::__cxx11`), so both engines keep the
list. In phase 5, `sc.wasm` already has what the session calls. A function
from a cut object puts the object back into `cuts/objects.txt`, as
`link.py --learn` does for wasm. `session.py` now records these stderr
lines of the engine in the result, and `split.py` already recorded stderr.

**What the session calls.** Before the learn, the cut `sc-engine` had 205
bad results in the slices. The learn brought in 63 functions and one
object:
- 44 from the debug information (`CGDebugInfo` and `DwarfUnit`). The
  session compiles every cell with `-gline-tables-only`, for the line of a
  trap (§3.5), and so `ses-` puts through the clang tests with it, which the
  corpus compiles without debug;
- `threadSafetyCleanup`, which the end of every session calls. Almost
  every result differed because of it;
- the module functions (`ModuleManager`, `GenerateModuleAction`, the
  `ActOnAnnotModule*`) and the `ModuleFile.cpp` object, for the sources
  with `#pragma clang module build` and for `import std;` in a session;
- a few others (`TemplateParamObjectDecl::printName`,
  `SemaOpenMP::DestroyDataSharingAttributesStack`).

**Two adjustments to the tests.**
- With `WFLAGS`, `ses-` decides whether the whole source compiles by
  `native-wev-base`. `-Weverything` makes 8 sources compile (`cc1 exit=0`)
  that give an error without it. It turns on warnings such as
  `-Wincompatible-function-pointer-types-strict`, which, where they apply,
  take the place of the `-Wincompatible-function-pointer-types` error.
- The link of a cell fails with the name of the temporary directory
  (`/tmp/sc-XXXXXX/cell_1.o`), which changes on every run. `norm_hash`
  replaces that name with `/tmp/sc-?/`, as it already did with the `.pcm`
  files. No result of the corpus bases has this name, so they do not
  change.

**Time of the learn.** From scratch it took 26 rounds and about 1 hour.
Each round takes about 2 minutes, between the link and `cargo build`. The
native build lacks the wasm's discovery, with the wrapper that records and
forwards the call. Each source finds one function per round, and 44 debug
sources found the 20 of `CGDebugInfo` one by one. A round that finds
nothing takes 4 minutes. So the learn starts from the list as it is, and
emptying the file learns everything again. `ses-` runs the whole corpus,
and not the sample that the plan expected (T-M2), because the later rounds
run only what differs.

**Size.** `sc.wasm` keeps the 63 functions and the object, even without a
session, where the one-shot code calls them:

| | before | after | |
|---|---|---|---|
| `sc.wasm` | 19,953,807 | 20,010,260 | +56 KB, +0.28% |
| `sc.wasm` zstd | 5,624,066 | 5,641,258 | +0.31% |
| `build/sc` | 25,500,032 | 25,554,032 | +54 KB |

Phase 5 will link the session into `sc.wasm`. An estimate: `link.py` with
`sc_session.o` and `sc_api.o` for wasm, exporting `sc.h`, after the usual
`wasm-opt`, gives 20,035,364 bytes and 5,650,733 in zstd. That is +0.47%
over the `sc.wasm` from before phase 4. The budget in §5.1 of the plan is
+1% in zstd. The 5,452,450 that the plan cites was `sc.wasm` before
exceptions and the new standards (phase E+N), which had their own budget.

**Memory and time of the session.** In `mem-`, the cut engine uses less
memory than the uncut one. It has 25 MB of RSS after 100 cells in C
(before, 40 MB) and 56 MB in C++ (before, 70 MB). The slope is the same,
2.6 KB and 5.3 KB per cell. A cell takes 33 ms in C and 41 ms in C++.

**The batteries.** `make test`, `test-native` and `test-warn` gave 0
differences in 17,715 files. `test-session`, on `sc-engine` against
`sc-engine-full`, gave 11,917 results and 0 bad (39 known), and
`test-session-warn` (ses-, seq- and crash- with `-Weverything`) gave 11,916
and 0 bad. `test-split` gave 0 differences in 1,947 programs, and
`test-student` and `test-checks` now run on the cut engine, equal to
`expected.txt`. `checkinv` and `test-proto` passed.

The session's slices stay out of `make test`. They need the native engine
with the session, and `make test` runs `sc.wasm`. The criterion for a
change to the engine or the cuts now includes `make test-session`
(AGENTS.md).

## 48. Phase 5 of the REPL: `sc.wasm` as a reactor with the session

`sc.wasm` became a single module, which runs as a command and exports the
session of `sc.h` (docs/plan-repl.md §5.2):
- it is linked with `-mexec-model=reactor`, and the `_start` is ours
  (`engine/sc_start.cpp`, wasm only): `exit(__main_void())`. The
  `_initialize` of `crt1-reactor.o` runs the constructors, and
  `wasmtime run` calls it before `_start`;
- the wasm has its own list of objects in the Makefile,
  `WENGINE := $(ENGINE) sc_session sc_api sc_start`, and `link.py` repeats
  it without `--native`;
- the exports come from a list in `link.py`. They are the seven session
  functions, `malloc`, `free` and `chdir`, plus `memory`, `_initialize`
  and `_start`. The guest starts at `/`, and the host calls `chdir` for
  the cell's directory.

**The frozen bases.** Before the change, the six bases were copied to
`build/res/<name>-pre5`. The `summary.json` files:

| base | sha256 |
|---|---|
| `base` | `a6d409a7d3ef5055...` |
| `wev-base` | `6826119050f0fb04...` |
| `native-base` | `11d513f42dc7e492...` |
| `native-wev-base` | `4d5113a1a078fb59...` |
| `session-base` | `1a196cba99b5713e...` |
| `session-wev-base` | `5197704e56cadfb2...` |

Results:
- the new `sc-full.wasm` gave `base` and `wev-base` equal to the frozen
  ones, file by file (17,715 each);
- `sc-host-full`, `build/sc` and the link arguments of the relinked
  `sc-engine` are identical byte for byte, so the native bases hold;
- `sc-engine-full`, recompiled with the change in `proto/`, gave
  `session-base` and `session-wev-base` equal to the frozen ones.

One caveat: this machine had a `ulimit -s` of 16 MB. With it, six native
session results that crash on the stack with 8 MB
(`instantiation-depth-*`, `limits-exprparen`) pass. The frozen bases are
from 8 MB, and with `ulimit -s 8192` all of them reproduce. The native
batteries depend on the shell's stack limit.

**A `long double` in `printf`.** The `wasm-ld` writes the name section of
each cell's `.so` with demangled names. LLVM's demangler prints a
`long double` literal of a name (`FloatLiteralImpl<long
double>`) with `snprintf`, and wasi-libc's `printf` without `long
double` support calls `abort`. In the wasm session, three clang mangling
tests (`mangle-template`, `mangle-class-nttp`, `mangle-literal-suffix`)
crashed the engine. The one-shot `sc.wasm ld` already had the problem. The
corpus does not link programs with these names. `sc.wasm` is now linked
with `-lc-printscan-long-double`, for +11.7 KB raw and +3.5 KB in zstd.
The corpus bases did not change.

**The test host.** `build/sc-engine-wasm` is
`proto/src/bin/sc-engine.rs` with the `wasm` feature. The feature swaps
only the raw calls (`proto/src/wasm.rs`). `engine.rs` uses `raw::`, which
is `sys` with the engine linked in or `wasm` with the feature. The module
is `<programa>.wasm`, a symbolic link to `sc.wasm`, `sc-full.wasm` or
`sc-cut.wasm`. `<programa> --precompile` compiles it, with threads, to
`<programa>.cwasm`, which the process deserializes. That way the process
that forks compiles nothing.
- **Instances.** Each session is a new instance, in its own `Store`:
  `_initialize`, `chdir`, `sc_session_new`. The one-shot (`-cc1`, `ld`,
  used by `std_pcm()`) is another instance, which runs `_start`.
- **Directories.** The guest sees the repository root at the same path and
  a directory of the process as `/tmp`.
- **Fork.** The plan's recipe works: `parallel_compilation(false)`, a tokio
  `current_thread` runtime with `enter()`, and
  `allow_blocking_current_thread(true)` before the preopens. Before each
  fork, an assert checks that there is 1 thread, and it never failed in the
  batteries.
- **Failures.** A trap or a `proc_exit` of the guest ends the process, with
  `_exit`. The fork copy then answers "the engine crashed", as in the
  native build.
- **Cut function.** The host writes `sc: cut function called: __wrap_X`
  when frame 0 of the trap is a `__wrap_`, the same line as the native
  stubs. `session.py` and `learn_session.py` did not change for this.
  Only `sc-cut.wasm` has the names, in an `sc-engine-wasm-cut`.

**Against what.** The plan compared the wasm slices with the native
`session-base`. But the wasm engine is not the native one where the native
one crashes. Three `crash-` sources (`static-compound-literals-crash`,
`builtins-eh-wasm`, `friend2`) do not crash in the wasm one-shot, and
`crash-` picks the sources by the one-shot. So, with a wasm engine,
`session.py`:
- decides `ses-` and `crash-` by the bases of the wasm one-shot (`base`
  and `wev-base`);
- in `mem-`, reads the `memory.size` that the host writes after each cell
  (with `SC_MEMORY_SIZE`).

The wasm session bases come from `sc-engine-wasm-full`
(`session-wasm-base` and `session-wasm-wev-base`), and the cut
`sc-engine-wasm` is compared with them, as in the native build. `split-`
compares with `pic-`, as always.

**Cuts (step 5).** `make learn-session-wasm`
(`learn_session.py --wasm`) learns on the cut `sc.wasm` with names,
through the host, and writes to the same list `cuts/symbols.session.used`.
It found one function, `clang::ASTReader::GetHeaderFileInfo(FileEntryRef)`,
which `import std;` in a cell calls on wasm (`seq-import-std` and the three
`mod-` of `split-`). It took 5 minutes.

**An unstable result.** The `ses-` of `CXX/dcl.decl/dcl.decomp/p3`, with
`-Weverything`, crashes in the cleanup of the cell with the error.
`IncrementalParser::Fail`, the `ASTDeclUnmerger` and
`DeclContext::removeDecl` read a bad pointer. On wasm, this is "memory
fault at wasm address 0x8004010". Whether it crashes depends on the engine
and the stack:
- on wasm, it crashes on the uncut engine and not on the cut one;
- on native, it crashes with 8 MB of stack and on the cut one with 16 MB.

It was a session bug, from before phase 5, and it is fixed now. The
`auto [y] = UsingGet();` at namespace scope, tuple-like, creates an
implicit variable `y` for the binding. Clang puts it in the context with
`addHiddenDecl`, outside the lookup table. `DeclContext::removeDecl`
assumes that every named decl is in the table. The `BindingDecl` `y`,
removed earlier, erased the `y` entry. So the `Map->find` of the variable
returned `end()`, and `removeDecl` read the list past the end of the table.
The `assert` that would catch this does not exist in release, and what was
there depended on the memory layout. The same happens with a decl added to
a namespace that comes from a module (the `std` of `import std;`). Its
table is lazy, and the new decl does not go into it.

The first fix, without a patch, was in `ASTDeclUnmerger`. Before
`removeDecl`, an empty entry took the place of the name. For every named
decl, it called `getPrimaryContext()` on the specializations too, which
`removeDecl` skips. This deserialized the module, and the cut engine
crashed in a cut function (`mod-m3_errors`, `CXX/module/cpp.pre/p1`).
Restricted to the implicit variables, the fix changed the layout and
exposed the module case. The same two tests crashed on every engine, and
the bases, generated with it, hid the crash. To cover both cases without a
patch, `sc_session.cpp` would have to copy `shouldBeHidden`, which is
`static` in clang. So the fix is a 3-line patch,
`llvm/incremental-remove-decl.patch`. `removeDecl` goes on without
touching the table when the name has no entry, in place of the `assert`.
With valgrind, `p3` and the two module tests gave "Invalid read", and now
they do not. Without the bad read, the cell with an error in `import std;`
goes on and deserializes more of the module. `make learn-session` put 4
functions of the AST reader into `symbols.session.used`
(`ExplicitInstantiationDecl`, `OffsetOfExpr`, `PackIndexingTypeLoc`).
`test/seq/binding-error.cpp` undoes an `auto [a, b] = std::pair{...}` in a
cell with an error and redefines it in the next one. `UNSTABLE` left
`corpuslib.py`.

**`test-checks`.** `optional.cpp` and another program with UB at `off`
printed a value read from uninitialized memory. That value depended on the
size of the environment, which the loader passes to the program
(`inherit_env`). It was 512 with `env -i` and 2,560 with a variable of
2,000 bytes. `checks.py` now runs `sc` with an empty environment, with
only `NO_COLOR`, and `expected.txt` was updated in those two lines.

**Size.**

| | phase 4 | phase 5 | |
|---|---|---|---|
| `sc.wasm` | 20,010,260 | 20,047,936 | +37.7 KB |
| `sc.wasm` zstd | 5,641,258 | 5,654,725 | +0.24% |

The budget in §5.1 of the plan is +1%. Without the `long double` it would
be 5,650,654 (+0.17%), close to the estimate in §47 (5,650,733).
`build/sc` did not change.

**Memory and time of the session on wasm.** The `mem-` of the cut
`sc-engine-wasm` measures `memory.size`:

| | C | C++ |
|---|---|---|
| after 100 cells | 16 MB | 38 MB |
| after 1,000 cells | 17 MB | 41 MB |
| slope | 0.5 KB/cell | 3.5 KB/cell |
| largest jump over 100 cells | 320 KB | 512 KB |
| time per cell | 20.9 ms | 30.0 ms |

The slope stays below the limit of 64 KB per cell in §4. The time per cell
is close to the cut native engine (23.5 ms and 29.1 ms in this run),
without counting the fork.

**Snapshot, for phase 6.** After 100 cells, the linear memory has 19 MB in
C and 42 MB in C++ (on the uncut engine). On V8 (Node 24):

| linear memory | copy (`slice`) | restore (`set`) |
|---|---|---|
| 19 MB | 3.7 ms | 2.5 ms |
| 42 MB | 16.9 ms | 4.4 ms |

A snapshot before each cell would cost from 15% to 50% of a cell. Besides
the memory, it needs:
- the `__stack_pointer`, which can be exported;
- to zero the part that the memory grew;
- to deal with the files that the cell left open in the `WasiCtx`.

**Time of `make test`.** With the phase 4 `sc.wasm`, relinked with the old
`link.py` (the same sha256), the corpus took 2m31s. With the new one it
took 2m34s, and 2m22s on a second run. `wasmtime compile` went from 2.06 s
to 2.25 s. No difference matters.

**The batteries.**

| battery | result |
|---|---|
| `make test` | 17,715 files, 0 differences |
| `test-warn` | 17,715 files, 0 differences |
| `test-native` | 17,715 files, 0 differences |
| `test-session-wasm` | 11,925 results, 0 bad (38 known) |
| `test-session-wasm-warn` | 11,924 results, 0 bad |
| `test-split-wasm` | 1,947 programs, 0 differences |
| `test-session` | 11,917 results, 0 bad |
| `test-session-warn` | 11,916 results, 0 bad |
| `test-split` | 1,947 programs, 0 differences |
| `checkinv` | SAME |
| `test-proto` | passed |
| `test-student` | passed |
| `test-checks` | passed |

`node:wasi` rejects a module with both `_initialize` and `_start`.
`bench/cc.mjs` uses the README's workaround and runs.

## 49. Phase 6a of the REPL: the minimal loader

The plan is in §5.3 of docs/plan-repl.md, and its six steps are done:
- the features;
- the removal of `wat`;
- `cache` and `parallel-compilation`;
- the release profile;
- `make test-loader`;
- the record of `proto/`.

**Size.** Each row adds one change to the row before:

| loader | `scload` | `sc` |
| --- | ---: | ---: |
| default features (start of the phase) | 27.5 MB | 28.8 MB |
| the minimal set of §5.3 | 24.1 MB | 25.5 MB |
| `lto`, `codegen-units = 1`, `strip`, `opt-level = "s"` | 9.9 MB | 10.6 MB |
| `panic = "abort"` | 9.0 MB | 9.6 MB |
| without `wat`: `wasm-encoder` builds the two modules | 8.1 MB | 8.7 MB |

The loader moved from `wasmparser` 0.239 to 0.254, the same as wasmtime's,
and the binary ended up with a single copy. The saving was 5 KB in
`scload` and 9 KB in `sc`, because LTO had already removed almost all of
the old copy.

§1.4 of the plan expected 5.7 MB, but for another scenario. There it was
`opt-level = "z"`, without `cache` and without `parallel-compilation`, and
only a C loop. That means without the loader's `wasmparser`, `gimli` and
`cpp_demangle`. Here, `s` costs 1.4 MB more than `z`, and `cache` costs
0.7 MB. The rest is the loader's code.

Almost all of the saving comes from `lto` with `strip`. The features
remove only 3.4 MB, because the `wasmtime-wasi` `p1` turns on `p2`, which
brings `component-model` and `async` back.

**Each feature, with the final profile.** Removing `wat` saves 0.86 MB,
and removing `cache` saves 0.69 MB. Without the profile,
`parallel-compilation` costs 0.3 MB.
- `wat` went away without hand-written bytes. The `wasm-encoder` 0.254 is
  already in the binary through the `compile` feature of
  `wasmtime-environ`, which `cranelift` always turns on.
  `Loader::wasi_shim` and `Loader::stubs` build the modules with it, and
  `wat_sig` is gone.
- `cache` and `parallel-compilation` stay. Without the cache, each
  `sc FILE` recompiles `libc.so` and `libc++.so`. The table in §5.3 shows
  the cost. It is 0.3 s in C and 1 s in C++.

**`opt-level`.** The times are for `sc FILE`, with three or four
interleaved runs per binary, at a load of 16 to 18. "Cold" is an empty
`XDG_CACHE_HOME` on each run. Every binary has the minimal set of
features.

| program | without lto | 3 | s | z |
| --- | --- | --- | --- | --- |
| C, `puts`, cold | 0.18–0.45 s | 0.16–0.19 s | 0.20–0.25 s | 0.23–0.27 s |
| C++, `map` and `iostream`, cold | 2.30–2.46 s | 2.25–2.50 s | 2.31–2.40 s | 2.43–2.53 s |
| C++, `map` and `iostream`, warm | 1.90–2.00 s | 1.92–2.01 s | 1.96–2.09 s | 1.95–2.03 s |
| C++, 200 thousand exceptions, warm | 1.54–1.73 s | 1.47–1.58 s | 1.49–1.61 s | 1.71–1.89 s |

- Clang inside `sc.wasm` dominates the warm cache, and its code is the
  same in every binary.
- `z` loses 5% to 8% when cold and about 15% on the exceptions, which go
  through the host. `s` ties with `3`. Hence `s`.
- With `lto`, the build of `sc` takes 3 to 6 minutes, against 1 to 2
  minutes without it.

**Two profiles.** The batteries use `scload` and `sc` through `$(SCLOAD)`
and `make repl`. With LTO in the `release` profile, each change to the
loader cost those minutes before any battery, and the batteries gain
nothing from the smaller binary. So `release` kept only `debug = false`.
With it, `scload` is 22.1 MB and `sc` 23.5 MB, and both compile in
2.5 min. The `dist` profile inherits `release` and adds `lto`,
`codegen-units = 1`, `strip`, `opt-level = "s"` and `panic = "abort"`.
`make dist` builds `build/loader/dist/scload` and `build/loader/dist/sc`,
with the sizes in the table above, in a single `cargo` run with the
`repl` feature. The library compiles once, and the two LTO steps run
together (2.4 min). `scload` does not use what the feature brings, and LTO
removes it (−4 KB). The batteries do not test the exact `dist` binary, but
the two versions differ only in the profile, not in the code, and `dist`
went through the batteries when it was `release`.

**`panic = "abort"`.** Neither the loader nor `proto/` has
`catch_unwind`. Wasmtime resumes the panic of a host function after it
unwinds the wasm frames, but it does not turn the panic into an error. So
a panic only exists in a loader bug. With `abort`, the message is still
printed, and the process ends with SIGABRT instead of exit code 101.

**The collector.** With only `gc-drc`, `Collector::Auto` picks DRC in
place of `Copying`. A program that throws and catches 200 thousand
`std::out_of_range` counts them all and ends the same way. `make
test-loader` checks the memory (below).

**The depth of the stack overflow.** The wasmtime limit
(`max_wasm_stack`) measures the native stack from the entry into wasm, and
the host frames count. With the `dist` profile, the host uses less stack,
and the recursion in `test/checks` overflows 3 frames later. `factorial`
goes from 10,916 to 10,919 calls, and `even`/`odd` from 8,187 to 8,190.
With two profiles, the count also differs between the batteries' binary
and the shipped one. So `checks.py` replaces the count with `N` before it
compares (`at factorial (...), N times`). The test still checks the message
and the frames, only not the number. `expected.txt` was regenerated, and
only those 6 lines changed.

Pinning the number with a larger stack is not worth it. With a
`max_wasm_stack` of 4 MiB (wasmtime then requires a larger
`async_stack_size`), the recursion goes past the backtrace's cap
(`wasm_backtrace_max_frames`, 2^15). The output becomes
`at factorial (...), 32767 times`, without `at main`, and the number is
fixed only because it is the cap. A thread with a large stack would still
be needed so as not to depend on the student's `ulimit -s`. The 1 MiB
stack in linear memory never becomes the limit. `N` stays, and in
`expected.txt` it appears only in the overflows.

**The batteries** use the new loader against the old one's bases:

| battery | result |
| --- | --- |
| `make test` | 17,715 files, 1 difference (base with a timeout) |
| `test-warn` | 17,715 files, 1 difference (base with a timeout) |
| `test-native` | 17,715 files, 0 differences |
| `test-split`, `test-split-wasm` | 1,947 programs, 0 differences |
| `test-session`, `test-session-warn` | 0 bad |
| `test-session-wasm`, `test-session-wasm-warn` | 0 bad |
| `test-student` | passed |
| `test-checks` | passed, after the `--update` above |

The two differences come from bases made under high load:
- `20040811-1-strict` in `base`;
- `vla-dealloc-1-on` in `wev-base`.

Both bases recorded `run exit=timeout`, and now the programs run
(`run exit=0`). The other timeouts of the first run (load 85) passed when
run again with `ONLY`.

**`make test-loader`** (`cargo test --release` in `loader/`, with the
`repl` feature, which reuses the wasmtime of `make repl`; in the debug
profile, the 10⁶ throws took 17 s, and in release 0.4 s) covers what the
batteries do not get to. The modules in `loader/tests/` are written in
WAT, with `wat` as a dev-dependency, which does not go into the binary:
- **Ctrl-C:** an infinite loop, armed with `arm`, stops with
  `Trap::Interrupt` when a `SIGUSR1` handler calls `increment_epoch`, like
  the SIGINT handler of `sc`. The signal goes to the thread that runs the
  loop. Once rearmed, the loader runs the next call. If the loop does not
  stop, a watchdog ends the test with an error after 30 s, instead of
  hanging. With the loader built without the checks, the test fails this
  way;
- **the collector:** after 10⁵ exceptions, 10⁶ more do not increase the
  resident memory, because DRC frees each exception.

Each test is in its own file, and cargo runs each file in a separate
process. That way the signal handler of one test does not affect the
other, and the resident memory measures only the exceptions.

The target takes about 1 min with the debug build of wasmtime already
done.

**The cost of the epoch checks** was measured with the same program
through `sc FILE`, without the checks, and through the REPL
(`sc repl FILE`), with them. Each cell of the table holds the smallest of
5 times, measured inside the program with `clock_gettime`, so the
compilation does not count:

| program | `--safety` | `sc FILE` | REPL |
| --- | --- | ---: | ---: |
| loop of 3·10⁸ iterations | `off` | 0.931 s | 0.937 s |
| loop of 3·10⁸ iterations | `on` | 0.937 s | 0.940 s |
| loop of 3·10⁸ iterations | `strict` | 0.946 s | 0.964 s |
| recursive `fib(36)` | `off` | 0.228 s | 0.229 s |

The difference stays below 1% in the loop and in the program made of
calls, and it disappears in the noise of the loaded machine. Cranelift
checks the epoch with one load and one compare at the start of each
function and each loop, and code at `-O0` does much more than that on
each iteration. Turning off the checks in `sc FILE` saves nothing visible.
The only difference is Ctrl-C, which kills the process in `sc FILE`. If
`sc FILE` ever has something to do with a Ctrl-C, turning the checks on
there costs nothing.

**`proto/` stays as it is.** `build/sc-engine-wasm` runs `sc.wasm` on
wasmtime through the `wasm` feature of `proto/`. It is only a test host,
for `test-session-wasm` and `test-split-wasm`, and it does not go to the
student. So it keeps wasmtime's default features and the previous release
profile (25.3 MB). Cutting the features there shrinks nothing that ships,
and LTO would add minutes to each build of the batteries. The binary that
the student gets is the `sc` of phase 6b, which uses the loader with the
minimal set above.

## 50. REPL phase 6b: one binary

`build/sc` becomes the shipped binary. It holds the REPL, the loader and
the cut engine with the session in one executable (docs/plan-repl.md
§1.1). Before there were two. The Rust `sc` launched `build/sc-engine` (C++
with the protocol loop) from the build directory as its child.

**Linking.** `proto/build.rs` already exported the arguments of
`link.py --native --emit-link` as `DEP_SC_LINK_ARGS` for the crates that
depend on it. The new `loader/build.rs` passes them to the link of `sc`
with `rustc-link-arg-bin=sc`, which does not affect `scload`. `sc` uses the
same arguments as `build/sc-engine` (`build/link-session-cut/engine.args`),
and so the same engine, object by object. Without `ld -r`, `--icf=safe` and
`--gc-sections` apply to the whole binary.

**The modes.** The `main` of `sc` looks at the first argument before
`bpaf`:
- `--engine`: the child, `serve` over stdin and a copy of stdout. The code
  moved from `sc-engine.rs` to `proto` (`serve_child`), and both binaries
  use it. The child's subcommand changed from `session` to `--engine` in
  `sc-engine` and in `test/session.py` too;
- `-cc1` and `ld`: the engine in one pass (`sc_main`), as in the old
  `build/sc`. A student file named `ld` needs `sc ./ld`;
- the rest is the student's command line.

The child is the executable itself (`current_exe`). The build directory is
`SC_BUILD`, or the first directory above the binary that has `repl/`. This
works for both `build/sc` and `build/loader/release/sc`.

**Size:**

| binary | raw | zstd -19 |
| --- | ---: | ---: |
| `build/loader/dist/sc` (`make dist`) | 34.3 MB | 10.7 MB |
| `build/sc` (release, `llvm-strip`) | 42.6 MB | 12.4 MB |
| the old `build/sc-engine` (the engine only) | 26.1 MB | 8.1 MB |
| `build/loader/dist/scload` | 8.1 MB | |

The plan estimated 24.5 MB for the engine plus 5.7 MB for the runtime
(§1.4). With the 8.1 MB of the loader measured in phase 6a, `dist` comes
out 1.7 MB above the sum of the two separate binaries, which the child and
the parent already loaded together. `release` is larger because it
compiles wasmtime without LTO. A hello through `sc FILE` takes 0.07 s and
28 MB, the same in both profiles. The child alone, with no cell, takes
14 MB.

**Timeout, Ctrl-C and valve** (`Repl::limit`, in `proto/src/child.rs`).
The parent waited for the answer in a blocking `read`, and a Ctrl-C during
compilation did nothing until the compilation ended. Now `Child` waits with
`poll`:
- **Ctrl-C:** the SIGINT handler of `sc` also calls
  `sc_proto::interrupt`, which sets an `AtomicBool`. The signal wakes
  `poll` with `EINTR`, even with `SA_RESTART`. The parent kills the
  child's group, and `Repl` replays the log in a new child
  (`Status::Lost`, "sc: interrupted"). The flag is cleared at the start of
  each request. So a Ctrl-C in the student's code does not bring down the
  next compilation;
- **timeout:** 60 s per answer ("the compiler took more than 60 s");
- **valve:** 1 GB, on the RSS of the largest process in the child's group
  (the `fork` copies share pages with it, and the child's pid changes with
  every cell that fails). The parent checks it every 200 ms during
  compilation, and after each accepted cell, when a new child replays the
  log.

A cell with a macro that expands to 2²⁷ elements of an array shows all
three:

| case | before | now |
| --- | --- | --- |
| Ctrl-C after 3 s | nothing until the compilation ends | "sc: interrupted", the next cell runs |
| no Ctrl-C | the child grows to 13.6 GB | killed at 1 GB, in 6 s |
| only the timeout (no valve) | — | killed at 60 s |

If the replayed log already exceeds the valve, each accepted cell replays
the log again. This does not happen with a student's cells (`mem-` shows
5 KB per cell), and the replay runs without the backend.

**The child dies with the parent.** With the parent idle, the end of stdin
already ended the child. In the middle of a cell, without the parent's
timeout, the child would go on to the end of the cell. `serve_child` now
asks for `PR_SET_PDEATHSIG` with `SIGKILL`. This closes the point left
open in phase 6a. With `panic = "abort"`, a panic in `sc` does not run the
`Drop` of `Child`, and still the child is not left an orphan. Tested by
killing the parent with `SIGKILL` in the middle of the large cell. No
`sc --engine` remains.

**The phase's criterion (`crash-` through the native REPL).**
`test/checks` gained `compiler-crash.c.repl`, with
`#pragma clang __debug crash` and
`#pragma clang __debug llvm_fatal_error` between a declaration and its
use. At all three levels the REPL says "the engine crashed on this cell;
the session is as it was before it", and the next cell shows `(int) 42`.
The `crash-` slices of `test-session` still go through `build/sc-engine`,
which has the same engine and the same `serve`. The `repl` test mode
exists only there.

**Batteries.** `test-native` (the whole corpus through the new `build/sc`,
with `sc -cc1` and `sc ld`), `test-split`, `test-session`,
`test-session-wasm`, `test-proto`, `test-loader`, `test-student` and
`test-checks` show no difference.

**Left for later:** the documented JS host (`sc.wasm` in the browser, with
a copy of the memory before the cell in place of `fork`). Documented on
2026-10-22, as a proposal, in docs/plan-repl.md §5.5.

## 51. One crate, and `cargo build` builds `sc`

Until phase 6b there were two Cargo projects. `loader/` was the crate
`scload`, with the binaries `scload` and `sc`. `proto/` was the crate
`sc-proto`, the API over `engine/sc.h`, with the binary `sc-engine`. Each
had its own `Cargo.lock`, and the two were almost equal. Only make could
build `sc`. The `repl` feature and `SC_LINK_ARGS` were required, and
`proto/build.rs` failed without the variable.

**One crate at the root.** The `Cargo.toml`, `build.rs` and `src/` at the
root are the crate `sc`:
- `src/loader.rs` is the old `loader/src/lib.rs`, and `src/engine/` is the
  old `proto/src` (its `engine.rs` became `session.rs`);
- the binaries are `src/bin/sc/` and `src/bin/scload.rs`, and the tests are
  `tests/` (those of the loader and the `session.rs` of proto);
- the `engine` feature (the default) brings the engine, bindgen and the
  command line. The `wasm` feature replaces the linked engine with
  `sc.wasm`. The batteries' `scload` is built with
  `--no-default-features`;
- `links = "sc"` and the pass-through through `DEP_SC_LINK_ARGS` are gone.
  `build.rs` passes the arguments directly to `sc`
  (`rustc-link-arg-bin=sc`) and to the tests (`rustc-link-arg-tests`). A
  `scload` with the default features links without them, because nothing
  in it calls the engine;
- one `Cargo.lock`. bindgen stayed at 0.72.1, and tokio went from 1.53.1
  to 1.53.3, the version proto already used.

**`sc-engine` is gone.** Since 6b, `build/sc-engine` had the same link as
`sc`. Only the `repl` test mode was its own. That mode became
`sc --replay` (`repl` is already a student command). As a result:
- `test-split` and `test-session` run on `build/sc` itself, and the
  `crash-` slices go through the shipped binary (the point §50 left open);
- `build/sc-engine-full` became `build/sc-full`. It is the same `sc`
  linked with the whole engine (`link-session`), in `build/cargo-full`. It
  is the base of `test-session`;
- the test host of `sc.wasm` is `sc` with `--features wasm`
  (`build/cargo-wasm`), copied to `build/sc-engine-wasm*`.
  `test/session.py` recognized the host by the existence of
  `<engine>.wasm`, which would now match `build/sc` with `build/sc.wasm`.
  It now recognizes the host by its name.

**`cargo build` builds the engine.** Without `SC_LINK_ARGS`, `build.rs`
links `build/link-session-cut/engine.args`. Outside make, it first runs
`make sc-deps`, which builds those arguments and what `sc` reads at run
time (`build/repl/*.args` and the sysroot). The first time, this is the
LLVM build, and cargo shows its output only with `-vv`. Details:
- **no recursion:** make exports `MAKELEVEL`. Under make, `build.rs` does
  not call make, because the rule that runs cargo has already built what
  it needs;
- **jobs:** make gets cargo's jobserver (`CARGO_MAKEFLAGS`);
- **when to rebuild:** `build.rs` watches `engine/`, `cuts/`, `llvm/`,
  `rt/`, the `Makefile` and `scripts/*.py`. With everything up to date,
  `make sc-deps` does nothing;
- **one directory:** `.cargo/config.toml` puts the build in
  `build/cargo`, where make also builds the default `sc`, without
  `SC_LINK_ARGS`. `cargo build` and `make native` do not rebuild each
  other's output, and `sc` finds `build/repl` by going up from the binary.
  A `target/` at the root would not find it.

The other links (the whole engine of `test-proto` and of `build/sc-full`,
and the link of `make learn-session`) pass `SC_LINK_ARGS`, and each has its
own `--target-dir`.

**An old Makefile bug.** `SCLOAD` was defined after the rules of `spike`,
`test-split` and `test-session`, which listed it as a prerequisite. make
expands the prerequisites when it reads the rule, and at that point the
variable was empty. The crate's variables moved to the top.

**Batteries.** `test-native`, `test-split`, `test-session` (with the base
rebuilt by `build/sc-full`, equal to the old one, result by result),
`test-session-wasm` (the same), `test-split-wasm`, `test-proto`,
`test-loader`, `test-student` and `test-checks` show no difference.
`build/cargo/dist/sc` is 34.3 MB, like the `build/loader/dist/sc` of §50.

**`sc --cc1` and `sc --ld`.** The engine modes in `sc` were `-cc1` and
`ld`, the ones of `sc_main`, and a student program named `ld` became a
wasm-ld link (`sc ld`). In `sc` they become `--cc1` and `--ld`, like
`--engine` and `--replay`, and `main` translates them for `sc_main`.
`sc_main` does not change. `sc.wasm` and the `build/sc-host-*` binaries
keep `-cc1` and `ld`, since no student can collide with them there, and
`test/corpus.py` translates only when the engine is `build/sc`.
`test-native` and `test-loader` show no difference.

**The bases of `make test` and `test-warn` rebuilt.** The change in
`corpus.py` rebuilds them (§49: two of them held `run exit=timeout` from a
run under load). Rebuilt one at a time, with no other battery alongside,
neither has a timeout, and both batteries give 17,715 files, 0
differences.

**The Ctrl-C handler in the library.** It was in the `machine.rs` of `sc`,
and `tests/signal.rs` tested a simplified copy (a `OnceLock`, SIGUSR1). Now
it is `Loader::on_ctrl_c(also)`, in `sc::loader` (it was in `sc::engine`,
next to `interrupt`; see below). It replaces the `Engine` that Ctrl-C
interrupts and, the first time, installs the SIGINT handler. The test calls
the same function twice, as in a `%restart`, and sends SIGINT to the loop's
thread.

**The batteries on the distributed `sc`.** `build/cargo/dist/sc` (LTO,
`"s"`, `panic=abort`) is what the student gets, and it had never gone
through the batteries. `make test-dist` runs `test-split`, `test-session`,
`test-student` and `test-checks` on it, against the same bases (`student.py`
and `checks.py` take the binary as an argument). It stays out of the
batteries for each change. It is the check before distributing. The first
time: 1,947 programs, 0 differences; 11,918 results, 0 bad; student and
checks as in `expected.txt`. `spike.py` also called `-cc1` on `sc` (the
`std.pcm` of the sessions). `std_pcm` now takes the mode.

**A failed replay does not end the REPL.** When the session is lost
(Ctrl-C, timeout, memory), `Repl::compile` replays the log in a new child.
If that replay also failed (a second Ctrl-C, a replayed cell above the
limit), the error reached `shell.rs` as `Err`, and the REPL exited and lost
the session. Now the failure goes as a diagnostic in the answer ("the
session was not restored ... the next cell tries again"). The child that
ended stays in place, and the next request gets `BrokenPipe` and replays
again. `undo` answers `Error` and keeps the session. After an accepted cell
above the memory limit, a failed replay keeps the child, because the cell
is already in the log. The test `replay_fails` (in `tests/session.rs`)
forces the failure with a timeout of 0.

**The child and the copies end with `sc`, the copy that takes over too.**
The `PR_SET_PDEATHSIG` of `serve_child` applied only to the process that
`sc` creates. Linux clears it on `fork`, and the copy that takes over after
a cell with an error is a child of the original, which exits in
`hand_over`. The copy then becomes a child of init. So `PDEATHSIG` could
not point to `sc`. In the copy, it would kill the copy exactly in
`hand_over`. If `sc` died in the middle of a heavy cell (`panic=abort`,
SIGKILL), that copy went on compiling with no timeout and no memory limit.
Now the end of `sc` arrives through stdin, which is the pipe that comes from it.
With `O_ASYNC` and `F_SETOWN` on the child's group, the kernel sends SIGIO
to every process in the group when `sc` closes the pipe, even when it
dies. The handler exits if `poll` gives `POLLHUP`. The writes of each
request also send SIGIO, but without `POLLHUP`. In a test on a pty, a cell
with an error made the copy take over, and `sc` died by SIGKILL while the
copy compiled a `std::regex`. With `PDEATHSIG`, the orphaned copy kept
running. With SIGIO, the whole group exited. The mechanism works on Unix,
like `fork`. `PDEATHSIG` was Linux only. Windows has no `fork`, and there
the session will go through `sc.wasm` in the process itself, copying and
restoring the linear memory at each cell, as in the JS host. Without a
child process, there is no orphan, and the timeout, Ctrl-C and memory are
left to wasmtime's epoch and limits.

**`kill` still kills the whole group.** The review suggested that, on a
timeout, a Ctrl-C or memory above the limit, the parent kill only the process
that compiles. The copy from before the cell would then see the end of the
pipe and go on with the session as it was, without a replay. For that,
`sc` would need to know who serves after a `hand_over`. The clean way would
be for each answer to carry the pid of the process that wrote it, which
changes the protocol (`MsgHeader`, `wire.rs`). It was left out. The saving
is small. The compilation on Enter runs with the terminal in raw mode, and
Ctrl-C arrives as a key, not as SIGINT. The timeout and the memory limit
are rare. The replay already works and no longer ends the REPL.

**Memory is checked only after the first `poll`.** `rss()` scans all of
`/proc`. There are 834 processes on this machine, about 27 ms measured in
Python. `wait()` called it before the first `poll` of each request, and
each Enter makes 2 to 4 requests (`depth`, `cell_if_whole`). Now memory is
checked only after a 200 ms `poll` expires with no answer. The requests
that answer earlier never scan. The check after each accepted cell stays
(one scan per cell). The test `memory` (in `tests/session.rs`) sets a
limit of 1 byte on a C++ cell with `<regex>`, `<format>` and `<iostream>`,
which takes longer than one `poll`.

**Ctrl-C in the loader, not in the engine.** `on_ctrl_c` was in
`sc::engine`, which compiles only with the `engine` feature. So
`tests/signal.rs` needed the feature, and `make test-loader` compiled and
linked the cut engine only to test the loader. Now it is
`Loader::on_ctrl_c(also)`. The loader keeps the `Engine` and installs the
handler, and `sc` passes `sc::engine::interrupt` as `also`, which the
handler calls first. `libc` becomes a common dependency, with no feature.
`test-loader` runs with `--no-default-features`, without the engine.

**Where `sc` finds `build/` is left for phase 7.** `build_dir()` goes up
from the binary to the first directory that has `repl/`. If none has it,
it falls back to the current directory, and an unrelated `~/repl` would be
taken for it. This exists because `sc` reads from `build/` at run time. It
reads the `.args` of each language and level (`build/repl`, with absolute
paths to `build/sysroot` and to clang's resource include), the sysroot
(headers and the `.so` files that the loader loads: 41 MB, 7.7 MB in zstd
-19) and the path of `build/` itself, used in backtraces. Fixing only the
search does not make `sc` work outside the checkout. This is a
distribution decision. The first step will remove the absolute paths from
the `.args`, and make them relative to a root that `sc` resolves in one
place. Then it remains to choose between a fixed tree next to the binary
(`<prefixo>/lib/sc`, like clang's resource directory) and the sysroot
embedded in the binary, extracted on the first run to
`~/.cache/sc/<version>`. The second is one file, simpler for the student
and for a future Windows. Until then, the search works in the checkout and
with `SC_BUILD`.

**The phase 0 spike is gone.** The choice of §22 (one session, not chained
PCHs) dates from 2026-10-13. `engine/sc_spike.cpp`, the `#ifdef SC_SPIKE`
in `sc_main.cpp` and `sc_backend.cpp`, `build/sc-spike` and `make spike`
served only to measure the two candidates. `split.py` and `session.py`
still imported `test/spike.py` for the split into cells, the `run` on
`scload`, the `expected` of `pic-` and `std_pcm`. Those helpers are now in
`test/cells.py`, and the rest is gone. Since the code behind `SC_SPIKE`
went into no other engine, the objects do not change.

**The corpus's run limit goes from 20 s to 60 s.** Removing the spike
changed `sc_main.cpp`, and with it `sc-full.wasm`, and the Makefile
regenerated the base. Three gcc torture programs with VLAs under ASan
(`pr43220-strict`, `vla-dealloc-1-on` and `-strict`) recorded
`run exit=timeout` in the base generated with 48 jobs, and passed in the
next `make test`. Alone (`JOBS=1`), the three also pass. It is the same
cause as the two bases of §49. The 20 s limit was close to the time of
these programs on a busy machine. No program in the corpus loops forever,
so the larger limit makes no run slower.

**The one-pass native engines (`build/sc-host-cut` and `sc-host-full`) are
gone.** `sc --cc1` and `sc --ld` do what they did, with the same engine.
They came from the time when the REPL did not yet link the engine into
itself. Two uses remained:
- the native bases (`native-base` and `native-wev-base`), which now come
  from `build/sc-full` (the REPL with the uncut engine). They came out
  identical to those of `sc-host-full`: 17715 hashes each, none different;
- `make learn-native`. `link.py --native --learn` links the engine, builds
  `sc` with it through cargo (like `learn_session.py`) and runs the corpus
  on it. Rebuilt from scratch, `cuts/symbols.host.used` came out the same,
  with the same 18 functions, in 15 min (11 rounds).

Since every native engine has the session, `link.py` always links
`sc_session.o` and `sc_api.o`. The `SC_EXTRA` of the Makefiles is gone,
and the `engine.args` of the cut `sc` did not change. `corpus.py`
recognized `sc` by name to use `--cc1` and `--ld`. Now every native engine
is `sc`.

**The test host of `sc.wasm` is now called `build/sc-wasm`** (before,
`sc-engine-wasm`, from the time of `build/sc-engine`), and the variants
become `sc-wasm-full` and `sc-wasm-cut`. `session.py` recognized the host
by name. Testing for `<engine>.wasm`, where the host looks for its engine,
does not work, because `build/sc.wasm` sits next to the native
`build/sc`. So `session.py` now takes `--wasm`, like `learn_session.py`,
and the host of `learn_session.py --wasm` became any `sc`.
`docs/plan-repl.md` §5 gained a table that maps the old names to today's.

The limits of `Repl` (60 s per answer and 1 GB) now apply to the first
child too. Before, `Repl::new` started the session with no limits, and only
the next `limit` set them. An engine that hung when starting the session
left `sc` stuck forever. Now `Repl::new` takes the limits. `limit` still
exists, and the tests use it to change the limits afterwards.

**`cargo build` from scratch.** It does not need `make learn`, because the
lists of the cuts are in `cuts/`. It does not need the wasm build of LLVM
or the corpus either. It needs the pin, the native LLVM (the tools), the
host LLVM (the libraries that the engine links) and the sysroot.
`build/native` built all the LLVM tools (141 in `bin/`). Today it builds
only the ones in use. These are the tablegens (`llvm-tblgen`,
`clang-tblgen` and `llvm-min-tblgen`, for the wasm and host builds),
`clang` (the one for the sysroot) and `lld` (the `wasm-ld` of every link).
14 binaries remain, and the clang libraries that `checkinv` uses come with
them.

| | before | with the restricted targets |
|---|---|---|
| steps of `build/native` | 5674 | 3707 |
| CPU of `build/native` | ~7 h | ~4.5 h |
| `build/native` on disk | 2.7 GB | 1.2 GB |

On a clean clone, `cargo build --release` took 13 min 45 s on 56 cores
(7.3 h of CPU) and used 6.6 GB in `build/`. The steps were:

| step | time |
|---|---|
| `build/native` | 5 min |
| `build/host` | 4.4 min |
| sysroot | 15 s |
| the rest: fetching the pin, link and cargo | ~4 min |

The resulting `sc` compiles C and C++ with exceptions, and the REPL works.
On the same clone, `make build/sc.wasm checkinv` passed, and `checkinv`
printed SAME. `sc.wasm` differs from the checkout's only in the
`build/src` paths recorded in it. The code is the same, and `wasm-opt`
merged the data into one segment fewer.

## 52. REPL phase 7, step 1: the pack embedded in `sc`

`docs/plan-repl.md` §5.4 has the proposal and the decisions. In this step,
`sc` carries the pack inside itself, but nothing reads it yet. `sc` still
reads the `.args` from `build/repl` and the files from disk.

**The `.args` as the pack sees them.** `scripts/flags.py --session`
accepts `/sc` in place of the build directory and writes
`/sc/sysroot/...` and `/sc/res/...`. make writes these `.args` to
`build/pack/repl`, and those in `build/repl` keep the real paths until
step 2.

**The pack.** `build.rs` builds a squashfs in `$OUT_DIR/sc.sqfs`, with
`backhand` 0.25.5 as a build dependency (`default-features = false`, only
`zstd`). The parameters are zstd -19, 128 KiB blocks, no padding, and
owner and dates set to zero. `sc` embeds it with `include_bytes!`, and a
`black_box` keeps it in the binary until the readers arrive. The `sc` of
the `wasm` feature carries no pack.

The contents:
- `sysroot/include`;
- `res/include`, without the `*intrin*.h` and `arm_*.h` at the top level;
- the five `.so` files (libc without the `.debug*` sections, which
  `build.rs` strips);
- the ignorelist;
- `libclang_rt.builtins.a`;
- the 12 `.args` in `repl/`.

That is 2,030 files and 18.7 MB raw. These are the 2,035 of the
measurement in §5.4 minus the five `.cwasm`, which arrive in step 4.
Extracted with `unsquashfs`, the pack equals the measurement's directory,
file by file. The libc without DWARF came out byte for byte equal to the
output of `wasm-tools strip -d '.debug.*'`.

| | bytes |
|---|---|
| the pack (`backhand`) | 2,612,568 |
| the same contents with `mksquashfs` (with 4 KiB padding) | 2,617,344 |
| the dist `sc`, before | 34,336,352 |
| the dist `sc`, with the pack | 36,950,432 (+2,614,080) |

The table in §5.4 gives 4.01 MB with the `.cwasm`. `mksquashfs` of these
contents with the `.cwasm` gave 4,014,080 bytes, and without them it gives
the 2,617,344 above. The 302 KB of the reader come in step 2.

**Build time.** Compressing the pack takes 63 s with `opt-level` 0, which
is the default for what compiles build scripts. `build-override` with
`opt-level = 2` in the `dev` and `release` profiles (`dist` inherits the
one from `release`) brings this down to 12 s. It is the same setting that
step 4 asks for the wasmtime of `build.rs`. Turning it on recompiled
everything once (4.5 min), because the proc-macros are shared.

`build.rs` also runs again on each change in `engine/`, `cuts/` and the
other directories that `make sc-deps` reads. For these cases, it writes a
stamp with the path, size and date of each entry (`sc.stamp`), and does
not rebuild the pack if the stamp has not changed.

`Cargo.lock` gained `backhand` and its dependencies (19 crates).
`toml_parser` went from 1.1.3 to 1.1.5, because of the `toml_edit` that
`deku` brings.

`test-student` and `test-checks` passed with no difference.

## 53. REPL phase 7, steps 2 to 4: `sc` reads from the pack

`docs/plan-repl.md` §5.4 planned clang's VFS (step 2), the hook in wasm-ld
(step 3) and the loader (step 4) as separate steps. They were done
together (decided on 2026-10-21), because the pack's `.args` also give
`-L/sc/...` to the link, and the loader read the `.so` files from
`build/sysroot`. None of the three works on its own. The `.cwasm`, the
always-on epoch and the removal of wasmtime's `cache` are left for later.

**The interface.** `sc.h` gained `struct sc_files` (the root `/sc` and
three functions: `status`, `read`, `list`) and `sc_set_files`. What `read`
and `list` return stays valid until the process ends, with a NUL after the
bytes, for the `MemoryBuffer` that clang requests with a terminator.

**clang.** `engine/sc_files.cpp` has a `ProxyFileSystem` over the real FS.
It answers for the paths under the root (`status`, `exists`,
`openFileForRead`, `dir_begin`, `getRealPath`, `isLocal`) and passes the
rest on. The `UniqueID` of each file is the hash of its path, because
`FileManager` tells files apart by it. Without `sc_set_files` (in
`sc.wasm`, which does not export it), `scFileSystem()` is the real FS.

**wasm-ld.** `llvm/lld-files.patch` (15 lines) adds a hook,
`lld::wasm::fileHook`, to `readFile` and `findFile`, which asks the hook
before the disk. `sc_set_files` sets it. The `stat` of the `.imports` of
`libclang_rt.builtins.a` still goes to disk, as §5.4 predicted.

**Rust.** `src/bin/sc/pack.rs` reads the squashfs with `backhand` (now a
normal dependency too). It keeps an index of the 2,151 nodes (files and
directories), built on the first read, and decompresses each file once and
keeps it. `sc --engine` calls `pack::install()` before serving. `Machine`
reads the `.args` and the loader's libraries from the pack. The loader
receives a `Libs` function (name → bytes) in place of the directories, and
`scload` and `tests/` pass `libdirs(...)`. `build_dir()` is gone, and
`build/repl` with it. `frames` treats `/sc/` as outside the student's code
(the `/src/` of step 5 is still missing).

The `sc` of the `wasm` feature also embeds the pack, so that the code is
the same, but `sc.wasm` reads from disk. It is only the test host
(`--replay` with the build's paths).

Measured on a loaded machine: the index takes 3 to 4 ms, `stdio.h`
0.35 ms, `libc.so` 2.2 ms, and decompressing the whole pack 31 ms.

**Outside the checkout.** With `sc` copied to a temporary directory and
`build/` hidden (`bwrap --tmpfs build`), `test-student` (66 runs) and
`test-checks` (165) give 0 differences. `strace -f -e trace=%file` shows
no path from the checkout, and the only `/sc/...` on disk is the `access`
of the `.imports`. The same holds for the dist `sc`, which also runs the
REPL (`x * 21` shows `(int) 42`, a `std::vector` in the next cell) and a
trap with its line (`at main (b.c:2:46)`).

**The bases.** `test-session` and `test-split` move to the `.args` with
`/sc/`, except `sc-wasm` and the programs with `import std;`, whose
`std.pcm` `sc --cc1` builds from disk. Of the 11,916 results of
`session-base`, 120 changed, all of them expected:
- 74 only because of the paths: `/sc/sysroot/...` and `/sc/res/...` in
  place of the build's paths, in the notes and in the wasm-ld errors;
- 46 include x86 or ARM intrinsics headers, which stayed out of the pack
  (§52). They already failed (`#error "This header is only meant to
  be used on x86..."`), and now they fail with "file not found". The
  status is the same.

`test-split` (10,541 cells, 0 differences from `pic-`), `make test`
(17,715 files, 0 differences: the patch goes into `sc.wasm` with a null
hook), `test-session-wasm` and `test-loader` passed.

**Size.**

| | before | after | |
| --- | --- | --- | --- |
| dist `sc` | 36,950,432 (§52) | 37,167,200 | +216,768, the reader |
| `sc.wasm` | 20,047,936 (§48) | 20,052,756 | +4,820 |
| `sc.wasm` zstd | 5,654,725 (§48) | 5,657,074 | +0.04% |

The reader came out below the 302 KB measured in §5.4, probably because
`zstd` was already in the binary, through wasmtime's `cache`.

## 54. REPL phase 7, steps 4 to 6: the `.cwasm` in the pack, no cache

**There is one `Config`.** `src/loader/config.rs` has the loader's
wasmtime `Config`, and `build.rs` includes it (`#[path]`) to compile the
libraries with the same options. `Module::deserialize` only accepts a
`.cwasm` from an equal `Engine`. The epoch is always on (§49: below 1%),
and `interruptible` is gone from `Loader::new`, `Machine`, `scload` and
`tests/`. Without Ctrl-C the epoch does not advance. `Loader::new` already
sets the deadline to 1.

**The `.cwasm`.** `build.rs` compiles each `.so` of the pack with wasmtime
48.0.3, now also a build dependency (the same features, and the
`build-override` of `opt-level` 2 from §52), for cargo's `TARGET`. The
result is code for the base CPU, which works on any machine of the target.
Each one goes next to its library
(`sysroot/lib/wasm32-wasip1/libc.so.cwasm`). `libc.so` is compiled already
without DWARF, which is what the pack carries. The pack's stamp includes
the compiled `build.rs` itself (wasmtime and `Config` are in it) and the
`TARGET`.

The loader receives a `Lib { wasm, cwasm }` from `Libs`. With the `cwasm`,
it tries `Module::deserialize` and, if that fails, compiles. The bytes of
the `.so` are still needed, because `dylink.0` and DWARF come from them.
`scload` and `tests/` have no `.cwasm`. Checked with a temporary
`eprintln`. In `sc` only the cells and the shims go through Cranelift (17
ms in total, for a C `hello`). The five libraries come from the `.cwasm`.

| `.cwasm` | bytes |
| --- | ---: |
| `libc++.so` | 2,590,328 |
| `libc.so` | 1,835,368 |
| `libc++abi.so` | 496,360 |
| `libsc_rt.so` | 205,144 |
| `libsetjmp.so` | 14,944 |
| total | 5,142,144 |

That is 11% more than the 4.62 MB from `wasmtime compile` measured in
§5.4. The numbers are not comparable. That measurement had no epoch checks,
and its code was not for the base CPU.

**The `cache` goes.** The `cache` feature is gone from wasmtime, and with
it 130 lines of `Cargo.lock`. `sc` no longer writes anything to `~/.cache`
except the REPL's history and theme.

**Step 5.** `frames` also hides the compiler-rt inside `libc++.so`, which
has `/src/src/llvm-project/...` and `/src/build/...` (the
`DW_AT_comp_dir` of the wasi-sdk build). The filter uses these two
prefixes and not all of `/src/`, which would hide the code of a student in
`/src`. An `__int128` divided by zero showed three compiler-rt frames
(`__divti3`, `__divXi3`, `__udivmodti4` with their lines) before the
student's code. Now it shows `in __udivmodti4`, followed by the student's
function.

**Size and time.**

| | 182cc2d (§53) | now | |
| --- | ---: | ---: | --- |
| pack | 2,612,568 | 4,177,083 | +1,564,515, the `.cwasm` |
| dist `sc` | 37,167,200 | 38,199,744 | +1,032,544 |
| dist `scload` | | 7,489,488 | without the `cache` |

`sc` grew 0.5 MB less than the pack, because the `cache` and its
dependencies are gone. `make dist` went from 2m51s to 4m10s, because
wasmtime is also compiled as a build dependency.

Time of `sc FILE` (dist, on a loaded machine; the warm times by
`hyperfine`):

| program | before, cold cache | before, warm cache | now |
| --- | ---: | ---: | ---: |
| C `hello` | 0.23 s | 84 ms | 97 ms |
| `std::vector` and `iostream` | 2.14 s | 1.84 s | 1.88 s |

The first run on a machine dropped from 0.23 s to 0.10 s in C and from
2.14 s to 1.86 s in C++, and there is no longer a difference between the
first run and the later ones. Against the warm cache, the new `sc` is 13
to 40 ms slower. The `cache` kept every module, cells and shims included,
and `sc` now compiles those every time. This is what §5.4 accepted when it
chose not to keep the cells.

**Outside the checkout.** The dist `sc` was copied to an empty directory,
with `build/` and `~/.cache/wasmtime` hidden (`bwrap --tmpfs`). It ran C++
with `std::vector` and exceptions, the REPL (`x * 21` shows `(int) 42`)
and a trap with its line (`at f (t.c:1:24)`). `strace -f -e trace=%file`
shows no path from the checkout. The only `/sc/...` on disk is the
`access` of lld's `.imports`. Nothing was written to `HOME`.

**The stack of recursions.** The epoch checks make Cranelift's native
frames larger, and in a mutual recursion of small functions
`max_wasm_stack` (512 KiB, native) runs out before the stack in linear
memory. `test/checks/mutual.c` in `sc FILE` went to 8,190 cycles of 2
frames and now goes to 5,459. This is what the REPL, which already had the
epoch, always gave. The recursion of a single function did not change
(10,920 frames). The limit stays as it is. `config.rs` explains why it
does not go up.

The point where the stack runs out varies with the build, and so does the
innermost frame. The release `sc` overflowed in `odd`, after 5,458
cycles. The dist one overflowed in `even`, after 5,459. Then the cycle,
found from the inside, started one frame earlier, and one `even` was left
over at the end. `test-checks` (release) and `test-dist` (dist) use the
same `expected.txt`, and one of the two failed. Before, they agreed by
chance. **Decided on 2026-10-22:** `frames` finds the cycles from the
outside (from `main`), which does not depend on the build. It omits the
piece of a cycle that the overflow cut on the inside, because those frames
equal the cycle's. A trap inside the cycle keeps its line, which is a
different one. Both builds now give `at odd / at even / (the 2 frames
above, N times) / at main`, as `expected.txt` already had.

**The batteries.** `make test-dist`:
- `test-split`: 1,947 programs, 10,541 cells, 0 differences (6
  known);
- `test-session`: 11,918 results, 0 bad (39 known);
- `test-student`: 66 runs, 0 differences;
- `test-checks`: 165 runs, equal to `expected.txt`, on the dist `sc` and
  on the release one.

`make test-loader` passed. The engine and the cuts did not change, so
`make test` and `test-session-wasm` do not need to run again.

## 55. wasm-ld and an annotation on a function that is only declared

The test `ses-src_clang_test_CodeGen_annotations-decl-use-decl` was a
known case of `test-session`. The native wasm-ld segfaulted in the cell's
link. The cell has `__attribute__((annotate("bar")))` on a function that it
only declares. The object gets the section `llvm.func_attr.annotate.bar`,
with an `R_WASM_FUNCTION_INDEX_I32` relocation for that function. In the
link `-shared --unresolved-symbols=import-dynamic`, a function used only
in a custom section is not imported and has no index.
`FunctionSymbol::getFunctionIndex` then casts an `UndefinedFunction` to a
`DefinedFunction`. Natively this is a segfault, in a command line link too
(`sc --cc1` and `sc --ld`). In `sc.wasm` the link passes and writes index
0, which belongs to another function.

**Decided on 2026-10-22:** a patch, `llvm/lld-undef-index.patch` (9 lines
in `ObjFile::calcNewValue`). An undefined function with no index and no
stub gives the same value that a dead symbol already gives there, the
section's tombstone or the addend. A cut does not work, because the code
that breaks is in use. Both engines now give the same result on this test,
and the entry leaves `KNOWN`.

**The batteries.**
- `make test`: 17,715 files, 0 differences;
- `make test-session`: 11,918 results, 0 bad (38 known, down from
  39);
- `make test-session-wasm`: 11,926 results, 0 bad (38 known).

The cuts did not need `make learn`. The batteries on the cut engines
(`build/sc`, `build/sc-wasm`) passed.

## 56. The JS host, step 1: the memory snapshot in Node

The first step of the JS host (docs/plan-repl.md §5.5) measures the
recommendation of §5.5. That is to undo a cell that fails by copying the
linear memory, in place of `fork`.

**What was done.**
- `scripts/link.py` exports the `__stack_pointer` of `sc.wasm`, the only
  global of the module. `sc.wasm` grew 86 bytes (20,052,842; zstd
  5,656,992).
- `test/node/engine.mjs` is the `sc --engine` child in Node. It is
  `wasm.rs` and `serve` in JS, with Node's WASI (`node:wasi`). Before each
  cell that is not a pass-through, it copies the memory and the
  `__stack_pointer`. If the cell fails, is a type, traps or calls `exit`,
  it restores the copy and zeroes what the memory grew.
- `sc --replay` accepts `SC_CHILD`, the child's program.
  `test/node/sc-node` is `build/sc-wasm` with that child, and
  `test/session.py --wasm` uses it like any host. The target is `make
  test-session-node`.

**Three adjustments.**
- Node's `WASI.initialize` refuses a module that exports `_start`, and
  `sc.wasm` exports both. The host passes only the memory and
  `_initialize`.
- With `returnOnExit`, an `exit` of the engine (an `LLVM ERROR`) ended the
  process. `proc_exit` now throws an exception, which the cell treats as a
  trap.
- A copy into a buffer kept from one cell to the next costs a half to a
  third of a new `slice()` (C++: 6.7 ms against 22.9).

**The batteries.** `make test-session-node`: 11,926 results, 0 bad (41
known). The 3 extra known cases are Node only. They are objects of
hundreds of MB (`arr[100000000]`). wasmtime's WASI fails to write them
("IO failure on output stream: Out of memory"), and so does the base made
with it. Node's writes them. In 47 of the 48 `crash-`, as in wasmtime, the
engine trapped or exited, and the session went on restored, with the
pass-through exporting the same names. `make test` (0 differences) and
`make test-session-wasm` (0 bad) passed with the export.

**The stack.** With Node's default stack (984 KB), 6 results differed from
wasmtime. The engine went deeper and compiled what overflows the stack in
wasmtime (`many-logical-ops`, `cxx2b-deducing-this`). With
`--stack-size=300`, V8 overflows where wasmtime overflows, and the target
uses that value. The browser, like default Node, goes deeper.

**Time.** One session, with no load, after the first 100 cells of
`mem-`:

| | memory | copy (median) | cell, Node | cell, wasmtime and `fork` |
| --- | ---: | ---: | ---: | ---: |
| C | 16 MB | 1.8 ms | 11.1 ms | 17.1 ms |
| C++ | 41 MB | 6.7 ms | 23.0 ms | 28.8 ms |

The restore, in `seq-` and `crash-` (62 times, 14 MB median, in a new
session), cost 11 ms median and 16 ms at most. The first touch of the
pages weighs on it, as on the first copy. The snapshot costs less than
`fork`, and the Node host is faster than `sc-wasm`.

With 48 jobs, each Node child compiles `sc.wasm` again, with no `.cwasm`.
The battery took 17 min, with 728 min of CPU. In one of the three runs, 3
large programs went over the 120 s of `ses-` and failed. Alone, they pass.

**Tier-up.** The compilation of the module is lazy (64 ms). Liftoff
compiles each function on its first call, and TurboFan compiles the hot
ones again, in background threads. This second compilation, redone in
each child, was almost all of the cost. On a sample of 215 cases of
`ses-`:

| V8 | time | CPU |
| --- | ---: | ---: |
| default (Liftoff and TurboFan) | 28 s | 14.1 min |
| `--liftoff-only` | 13 s | 3.6 min |
| `--no-liftoff` (TurboFan only) | 129 s | 78.5 min |
| wasmtime (`sc-wasm`, `.cwasm`) | 10 s | 1.3 min |

With `--liftoff-only`, the whole battery takes 5.5 min, with 221 min of
CPU, and gives the same results (0 bad, 41 known). The target uses the
flag. In a long session the Liftoff code is slower. After 1000 cells it
takes 18.3 ms per cell in C (10.9 with TurboFan) and 33.7 ms in C++
(23.9). The real host keeps the default, which compiles once per page. The
battery's sessions are short.

**Left for later.** A trap leaves open the fds that the cell opened in
Node's WASI, which the restore does not reach. In the browser the shim is
ours and closes them (§5.5).

## 57. The JS host, step 2: the loader in JS

`test/node/loader.mjs` is `src/loader.rs` on the JS API of WebAssembly,
and `test/node/scload` is `scload` on top of it, with the same arguments
and the same output. It is 559 lines, with no dependencies, and it serves
Node and, later, the browser's worker.

**What the JS API does not give, the bytes give.** Without type
reflection (`--experimental-wasm-type-reflection`, off), the API does not
tell the type of an import, or whether an exported global is constant.
The loader reads the type, import, global and export sections from the
module, as well as `dylink.0`. A function that goes into the table has to
be wasm, not JS. So the stubs (a `call_indirect` through the slot) and the
"undefined symbol" functions are small modules built from bytes, like the
ones `wasm-encoder` makes in the native loader. WASI does not need the
native loader's shim module. The `wasiImport` of `node:wasi` goes straight
into the imports, after a `wasi.initialize` with the shared memory.
Exnref is already on in V8 13.6 of Node 24, and the `__cpp_exception` tag
is a single `WebAssembly.Tag`.

**An `exit` must not be caught.** A JS exception that crosses wasm is
caught by a `catch_all`, and the destructors would run during the `exit`.
`proc_exit` stores the code and calls a wasm function that is only
`unreachable`. The V8 trap crosses JS and wasm without being caught, as in
wasmtime. We checked this with an `exit` inside a `try` with a destructor.
The same holds for "undefined symbol". The function writes the message
and traps. Another difference from JS is that a call with extra arguments
does not fail. `call` compares with the function's `length` and says what
wasmtime says (`alias-4`: `main(int)` called with two).

**The trap.** `scload.mjs` writes the trap in wasmtime's format, with
wasmtime's words for each V8 message. Normalization in `norm_hash` drops
the stack. In two programs V8 says less, and they are known only with the
JS loader (`JS` in `test/cells.py`):
- `ct-00187`: V8 has a single message, "null function or function
  signature mismatch", for a null slot and for a wrong type. Wasmtime
  says "uninitialized element";
- `pr23135`: wasmtime also gives the address and the size of the memory.

**Out for now:** the ASan runtime (`libsc_rt` at the `--safety` levels
with checks), the file and line of a frame (DWARF), the stacks of the
blocks and the leaks. The loader rejects a module that exports
`__asan_init`.

**The batteries.**
- `make test-split-node` compiles in `build/sc` and runs with the JS
  loader. It gives 1,947 programs, 10,541 cells, 0 differences (8 known,
  the 6 of `test-split` and the 2 above), in 5.5 min.
- `make test-session-node` now uses the JS loader too. The compiler and
  the programs run in Node. It gives 11,926 results, 0 bad (43 known), in
  7 min.
- `make test-split` still has 0 differences.

**The start of a program.** V8 compiles lazily, and `scload` without
`.cwasm` compiles everything with Cranelift (3 runs each):

| | `scload.mjs` | `scload` (`-L`, no `.cwasm`) |
| --- | ---: | ---: |
| `int main(void){return 0;}` | 0.11 s, 0.1 s of CPU | 0.12 s, 2.0 s of CPU |
| hello with `iostream` | 0.14 s, 0.2 s of CPU | 0.29 s, 5.4 s of CPU |


## 58. The session's files without a disk: the package through the host and the cells in memory

For the REPL in simplecode, `sc.wasm` reads the files as the native `sc`
does, and not through an FS in the page's WASI shim. The simplecode shim
has no files (`path_open` fails), and it stays that way.

**The cells in memory.** The session used to write `cell_N.o` to a
temporary directory, wasm-ld wrote `cell_N.so` next to it, and the
session read it back. Now `ScLink` (`sc_files.cpp`) gives wasm-ld the
object during linking through the read hook of `lld-files.patch`. It
receives the output of `-o -` through a new hook in the same patch,
`outputHook` in `Writer.cpp`. Without the hook, `-o -` would go to stdout
(`InMemoryBuffer` of `FileOutputBuffer`). This holds for native too. No
session opens `/tmp` any more, and `src/engine/wasm.rs` and
`test/node/engine.mjs` do not give it to the guest.

**The package through the host.** An `sc_files` with function pointers
cannot cross the wasm boundary. `engine/sc_host_files.cpp`, only in
`sc.wasm`, implements `sc_files` on two imports:
- `sc_files.status(path, n, *size)`: 0, 1 (file) or 2 (directory,
  with the size of the names);
- `sc_files.read(path, n, dest)`: the bytes of the file or the
  names.

The host calls the exported `sc_host_files("/sc")` after `_initialize`.
Each file is copied once into the guest's memory and kept, as `sc_files`
asks. With this, the `sc.wasm` session reads the package embedded in
`build/sc-wasm` (the same as the native one) and, in Node, the build
directories at the package's paths. `engine.mjs` removes the intrinsics
of other targets as `build.rs` does. Without the filter, 46 results
differed, all from programs that include `immintrin.h` or `arm_neon.h`.
In a large C case, it took 9 calls and 1.5 ms.

The corpus's `wasmtime run` (`test/corpus.py`) passes
`-W unknown-imports-trap=y`, because the one-shot never calls the
imports. `sc.wasm` grew 1.4 KB (20,054,224 bytes).

**What changed in the batteries.**
- The 3 objects of hundreds of MB (§56) are now accepted in wasmtime too,
  because the session does not write them through WASI. The whole
  compilation of the corpus still fails when it writes them, and they are
  known on every host of `sc.wasm`, not only in Node.
- `microsoft-abi-emit-dependent` (`#pragma clang module build`, which
  writes to `/tmp`) now agrees with the corpus on the `sc.wasm` hosts,
  which no longer have `/tmp`, and it is known only in native.
- `make test`: 0 differences. `make test-session`: 11,918 results, 0 bad
  (38 known). `make test-session-wasm`: 11,926, 0 bad (40).
  `make test-split-node`: 0 differences (8). `make test-session-node`:
  11,926, 0 bad (42), in 4.7 min. In one of the runs,
  `limits-externdecl` went past 120 s with the machine full. Alone it
  takes 63 s in Node (Liftoff) and 41 s in wasmtime, and it passed in the
  other two.

## 59. The package inside `sc.wasm`: an uncompressed squashfs

`sc.wasm` now carries the package of §58 inside itself, and the page
downloads only the module (decided on 2026-10-22). The simplecode WASI
shim gives nothing beyond what it already gives, and the imports
`sc_files.status` and `sc_files.read` go away.

**The format.** The same 2,030 entries as the `sc` package (§52), without
the `.cwasm` files, which the browser does not use (18.7 MB raw), in two
`mksquashfs` images. We compressed them afterwards as the simplecode
server does (`server/src/static_files.rs`: zstd 19 and gzip 9) and with
brotli 11:

| | bytes | zstd 19 | gzip 9 | brotli 11 |
| --- | ---: | ---: | ---: | ---: |
| the earlier `sc.wasm` | 20,054,224 | 5,656,340 | 7,459,299 | 5,314,157 |
| + the uncompressed squashfs | 38,925,223 | 7,549,291 | 10,324,980 | 7,096,188 |
| + the squashfs with zstd -19 (the one in `sc`) | 22,667,547 | 8,269,744 | 10,073,117 | 7,922,753 |

The uncompressed image arrives smaller at the browser, because the server
compresses the whole module at once, and squashfs compresses independent
blocks of 128 KiB. On its own, it gives 1.9 MB in zstd 19 and 1.8 MB in
brotli, against 2.6 MB for the image with zstd. It also removes the need
for a decompressor in the guest. `zstddeclib` gives from 90 to 99 KB of
wasm, and it decompresses `libc.so` in 2.7 ms in Node. The native `sc`
keeps the zstd image. It has no compression on the way, and the other
image would make it grow by 16 MB.

**The reader.** The squashfuse 0.6.5 library (BSD-2), fetched by the
`Makefile` at a fixed commit (`build/squashfuse`) and compiled for wasm32
in `build/squashfuse-wasm/libsquashfuse.a`. It has no patch.
`engine/sc_pack.cpp` gives the two functions the library asks of the
system. `sqfs_pread` copies from the image, and `sqfs_decompressor_get`
gives a decompressor that always fails (nothing in the image is
compressed). `decompress.c` and `nonstd-pread.c` stay out.

The image is an `#embed` of `build/pack/sc.sqfs`, which `scripts/pack.py`
builds with `mksquashfs` (`-noI -noId -noD -noF -noX`, owner and dates
zeroed). It starts from a copy of the tree with the same files as
`build.rs`. These are the same directories, without the intrinsics of
other targets, and `libc.so` without DWARF. Extracted, it is equal, file
by file, to the `sc` package without the `.cwasm` files. The build now
needs `mksquashfs` (squashfs-tools).

The host calls the exported `sc_pack("/sc")` after `_initialize`, in place
of `sc_host_files`. Each file read is copied once to the heap and kept,
as `sc_files` asks (with a NUL after it). `sc_pack_image` gives where the
image is in memory, and `engine.mjs` leaves it out of the snapshot and the
restore. The session only reads it. The corpus's `wasmtime run` no longer
needs `-W unknown-imports-trap`.

**The size.** `sc.wasm` has 38,913,173 bytes. The code grew 9.3 KB
(squashfuse and the reader, minus `sc_host_files`), and the data 18.8 MB.
Through the server, it is 7,548,235 bytes in zstd 19 (+1.9 MB) and
7,100,519 in brotli.

**The time, in Node.** A session without `--liftoff-only`, with 100 small
cells and then 21 measurements, compared with the one in §58, where the
host read the files from disk through the imports. The machine was
loaded, and other runs varied up to 3 times, so the numbers count only as
an order of magnitude:

| | before, C | now, C | before, C++ | now, C++ |
| --- | ---: | ---: | ---: | ---: |
| instantiate | 9.1 ms | 18.7 ms | 9.7 ms | 21.5 ms |
| `sc_host_files` / `sc_pack` | 0.12 ms | 0.49 ms | 0.14 ms | 0.48 ms |
| the cell with the `#include` lines | 140 ms | 141 ms | 3.2 s | 3.4 s |
| one cell, median | 6.2 ms | 7.4 ms | 8.9 ms | 9.0 ms |
| the memory | 16.1 MB | 33.4 MB | 65.8 MB | 82.7 MB |
| the snapshot copy | 2.4 ms | 1.3 ms (3.9 with the image) | 10.4 ms | 10.3 ms (13.5) |

Instantiating copies the 19 MB of the image into memory, about 10 ms,
once per worker. Reading the image's files through squashfuse does not
change the time of the `#include` lines. The memory grows by the size of
the image, and the snapshot that leaves it out costs what it cost before.

**The batteries.** `make test`: 17,715 files, 0 differences. `make
test-session-wasm`: 11,926 results, 0 bad (40 known). `make
test-session-node`: 11,926, 0 bad (42). `make test-split-node`: 0
differences (8). The native engine did not change.

## 60. One instantiation per module: the GOT computed from the bytes

**Why.** To run the REPL in simplecode, the proposal is that the host
gives the engine only generic primitives to run wasm, with no JS of sc's
own. Such a host instantiates a module with the imports it receives, and
it should need nothing else. The loader wrote the `GOT.mem` and
`GOT.func` entries after it instantiated the group (§57). The question
was whether the values can be known before.

**Without PIC it does not work.** I tried to link a static cell with the
pin's `wasm-ld`, with fixed addresses (`--import-memory --import-table
--global-base --table-base --allow-undefined`). Three things prevent it:
- the address of data in another module becomes 0, with no warning, and
  there is no `--defsym`. An absolute symbol in assembly
  (`.set x, 0x12340`) gives `absolute addressing not supported!`;
- the module defines its own `__stack_pointer` instead of importing it;
- `&f` of a function from another module gets a slot in the cell's own
  table, different from the slot in the module that defines it.

It would need a patch in lld. PIC stays.

**With PIC it works.** In a `.so`, the data comes in through active
segments at `__memory_base`, and the table through an active `elem` at
`__table_base`. An exported data symbol is an immutable global
`i32.const offset`. `loadBytes` in `test/node/loader.mjs` now has two
phases:
1. With the bytes only, it reserves the memory and the table of each
   module in the group, defines the data (`__memory_base` + the
   `i32.const`) and `__heap_base`, and leaves the functions pending. The
   slot of a function in the table can be given before the function
   exists.
2. It instantiates each module once, with each GOT entry at its final
   value, and fills the slots of the module's functions in the table.

Only the `GOT.mem` of a variable not yet defined is written later, when a
module defines it, as before.

**The stubs.** An import of a function that does not exist yet goes
through a stubs module, which calls through the table. In the
`test-split-node` battery:

| | loaded | with a stubs module |
| --- | ---: | ---: |
| cells | 9,743 | 107 (1.1%) |
| libc, libc++abi, libc++ | | all |

The libc imports its own exported functions (`clock_gettime`, `lseek`,
`isalpha_l`...), the interposition of a `.so`. Before it is
instantiated, only a stub works. This happens once per session. Of the
107 cells, 26 import only their own functions, and the others call a
function declared and not yet defined. The old loader made the same
stubs.

**The batteries.** `make test-split-node`: 1,947 programs, 10,541 cells,
0 differences (8 known), in 39.0 s, against 38.9 s for the old loader,
with the machine loaded.
`make test-session-node`: 11,926 results, 0 bad (42 known), apart from
`limits-externdecl`, which went past 120 s with the machine full, as in
§59, and passes alone in 61 s.

## 61. The loader inside `sc.wasm` in Rust: the cost of linking the std

**The decision.** In simplecode, the loader, the `Machine` and the `Repl`
live inside `sc.wasm` and call only the host's generic primitives (§60).
We chose Rust, not C++. The native `sc` already has this code in Rust on
wasmtime (`src/loader.rs`, `src/bin/sc/machine.rs`: about 1,500 lines of
the REPL front end), and few places touch wasmtime. With a runtime trait,
the same code serves both hosts, and sgleam, which is Rust, reuses the
API bindings. In C++, the REPL would have two implementations.

**The measured risk.** A Rust staticlib (`wasm32-wasip1`, rustc 1.98.1,
`opt-level="z"`, LTO, `panic="abort"`) with a wasm section reader, which
uses `Vec`, `String`, `HashMap`, `format!` and `eprintln!`, linked to the
engine through `SC_EXTRA` of `scripts/link.py`, with one more export:
- it links with no conflict with wasi-sdk's wasi-libc, and the imports of
  `sc.wasm` do not change;
- the function reads `libc++.so` inside `sc.wasm` in Node and counts
  3,319 imports and exports, as `wasm-tools` does. `eprintln!` goes out
  through the engine's WASI.

| | `sc.wasm` | zstd -19 |
| --- | ---: | ---: |
| no Rust | 38,913,173 | 7,548,235 |
| with the staticlib | 38,952,148 (+39 KB) | 7,564,854 (+17 KB) |

The Rust std weighs little. The real loader will add its own code, not a
whole std.

## 62. The native loader in the model of §60

First step to put the loader behind a runtime trait (§61). `src/loader.rs`
now also instantiates each module once, with the GOT at the final values,
like `test/node/loader.mjs`. `dylink` reads from the bytes the
`i32.const` of the exported immutable globals. A function of the group
stays pending (`Sym::Func(None)`) until its module is instantiated, and
then fills its slot in the table. The `GOT.mem` of a variable that only a
later module defines is still written later, when the variable comes.

**The difference from JS.** If a module fails to instantiate (an import
with another type), the loader undoes what the group defined from the
bytes, and the reserved slots it took become reserved again. Without
this, a rejected cell would leave pending functions in place of the old
ones. `tests/undo.rs` covers this case (in `make test-loader`) and fails
without the undo. `loader.mjs` does not undo.

**The batteries.** `make test-split`: 1,947 programs, 10,541 cells, 0
differences (6 known). `make test-session`: 11,918 results, 0 bad
(38 known). `make test-student`, `make test-checks` (the levels with ASan)
and `make test-loader` pass.

## 63. The loader on a runtime trait: the primitives of Draft 4

Second step (§61, §62). The core of `src/loader.rs` no longer uses
wasmtime. It asks the runtime only for what the host will give `sc.wasm`
as `env.wasm_*` (`src/loader/runtime.rs`, the `Runtime` trait):

| primitive | what it does |
| --- | --- |
| `instantiate(bytes, compilado, imports)` | compiles and instantiates. An import is a handle, the runtime's default (WASI, `env.wasm_stack`) or an i32 that becomes a new global, whose handle comes back in the list |
| `export(inst, name)` | the handle of what the instance exports |
| `call(f, assinatura, vals)` | calls. The error is `Exit`, `Trap` (kind, message, fault address, frames) or `Fail` |
| `read`, `write` | the memory |
| `stack(id)` | the frames of a stack that `env.wasm_stack` stored |

`src/loader/native.rs` implements the trait on wasmtime. The handles are
a vector of `Extern` and of instances. WASI goes through the same shim
that exports the memory (one per memory and per instantiation that asks
for new functions). Each `call` arms the epoch, so Ctrl-C stops the call
it falls on without the old `arm()`. The `Machine` gets the trap, the
output and the stacks through the loader's types (`loader::Error`,
`Trap`, `Frame`), and it finds the DWARF through the frame's instance.

**What the loader generates** (`src/loader/gen.rs`, with `wasm-encoder`):
- **the world**, instantiated once. It holds the memory, the table,
  `__stack_pointer`, and functions to grow the memory and the table, put
  a function in the table, and read and write the stack pointer;
- **the stubs**, as before, and in the same module the functions that do
  not exist yet. Instead of a host function that fails with the name,
  each is a function that writes its number into the world's `missing`
  global and executes `unreachable`. The `Machine` reads the name after
  the trap (`Loader::missing`);
- **a tag**, when a module imports one that nobody defined;
- **a module that sets globals** for the `GOT.mem` entries of variables
  that come later (one per group that has any);
- **the `try`**, per signature. It calls the given function through the
  table inside a `try_table` with the `__cpp_exception` tag. The
  constructors, `main` and `exit` go through it. A C++ exception that
  nobody catches comes back as a number, and the loader calls
  `__cxa_begin_catch` and `std::terminate`, as `uncaught()` did with
  `take_pending_exception`. It exists only after a module defines the tag
  (the one in libc++abi), so a C program does not use it.

**The host functions became functions of the world**, with no callbacks.
This was the user's decision. Draft 4 still has no `wasm_func_new`, and
it comes back to the table with sgleam:
- `__sc_show_int` and `__sc_show_float` write the arguments into globals.
  The `Machine` reads them right after the cell, before `fflush`, and
  reads the type name and the string from memory;
- `__sc_rt_blame` writes the two numbers;
- `__sc_rt_data` reads the data ranges from a table that the loader
  writes to memory before `__sc_rt_leaks`, and `__sc_rt_leak` writes each
  group of lost blocks into another (up to 16,384 groups). Both live in a
  block from libc's `aligned_alloc`, which libsc_rt does not see, and the
  loader frees it afterwards. The libsc_rt protocol did not change;
- `__sc_rt_stack` is the runtime's `env.wasm_stack`, which the world
  imports and re-exports. Only capturing the stack needs the host.

**The batteries.** `make test-split`: 1,947 programs, 10,541 cells, 0
differences (6 known), 5m10s (5m05s before). `make test-session`: 11,918
results, 0 bad (38 known), 4m45s (4m41s before). `make test-student`,
`make test-checks`, `make test-loader`, `make test-proto` and
`make test-session-wasm` (11,926 results, 0 bad, 40 known) pass. The
extra calls (the world, the `try`) do not show in the time.

**Seen on the way.** `libc++.so` and `libc++abi.so` each define their own
`__cpp_exception` tag. The loader gives everyone the first one, the one
in libc++abi, which is the one `__cxa_throw` uses. A `catch (...)` inside
`libc++.so` works (it falls into `catch_all`, and a `streambuf` that
throws sets `badbit`), but a typed `catch` compiled into `libc++.so`
would not catch the exception. It was already like this before this step.

## 64. The loader inside `sc.wasm`: the primitives as host imports

Third step (§61 to §63). The same `src/loader.rs` runs inside `sc.wasm`,
on primitives that the host gives as `env.wasm_*` imports, and Node gives
them with the JS API of WebAssembly, with nothing from sc.

**The crate.** Wasmtime became the `native` feature (`scload` and `sc`
with the engine turn it on). Without it the crate is the core of the
loader, which compiles to `wasm32-wasip1` (`wasmparser`, `wasm-encoder`,
`cpp_demangle`, `gimli`, `addr2line`). `src/loader/host.rs` implements
`Runtime` on seven imports:

| import | what it does |
| --- | --- |
| `wasm_instantiate(bytes, len, imports, n, out) -> status` | compiles and instantiates. An import is the host's default (0), a handle (1) or a new i32 global, constant (2) or mutable (3), whose handle the host writes back |
| `wasm_export(inst, name, len) -> h` | 0 if there is none |
| `wasm_call(f, sig, sig_len, vals) -> status` | `vals`: 8 bytes per value. The results come back in them |
| `wasm_error(buf, cap) -> len` | the message of the last failure or trap |
| `wasm_frames(id, buf, cap) -> len` | the frames of a stack from `env.wasm_stack` (or of the last trap, with 0). One line each, the offset and the name |
| `wasm_memory_read`, `wasm_memory_write` | the memory of a child |

The status is OK, TRAP (with a code for the kinds of `TrapKind` and the
conversion to integer), EXIT (with the code), STOP or ERROR, and the
detail comes in `vals[0]`. From the code, `host.rs` writes wasmtime's
message (`wasm trap: out of bounds memory access`...). V8's texts change
from one browser to another, and this way the output of `scload` is the
same on both hosts. The address of a fault does not come (V8 does not
give it), and the frames do not name the instance.

`wasm/` is the staticlib that `sc.wasm` links (`scripts/link.py`, the
Makefile's `$(RS_WASM)`). It holds the loader and the `sc_scload` export,
which is `scload` with the arguments of the WASI of `sc.wasm` (`-L`,
`--stack`, the modules). The program's arguments, the directories and the
stdio are those of the default WASI that the host gives the children.
`link.py` makes no stubs for the `wasm_*` imports, because they are real
imports. `sc-wasm` (`src/engine/wasm.rs`) and the corpus
(`wasmtime run -W unknown-imports-trap=y`) make them functions that trap,
because the sessions and the one-shot engine do not call them.

**The host in Node.** `test/node/wasm.mjs` gives the primitives with the
JS API. The handles are a vector that never reuses a number. The default
WASI is one `node:wasi` per memory (the one the child imports).
`proc_exit` records the code and executes the `unreachable` of a host
module, which C++ does not catch. The code of a trap comes from V8's
message. `env.wasm_stack` stores the wasm frames of a
`new Error().stack`, one id per distinct stack. `test/node/scload-rs`
instantiates `sc.wasm` with them and calls `sc_scload`. `engine.mjs` also
gives them, for `sc.wasm` to instantiate.

**The measurements.**
- `make test-split-node-rs`: 1,947 programs, 10,541 cells, 0 differences
  (7 known, the 6 of `test-split` and `ct-00187`, whose trap message V8
  gives differently). `make test-split`, `test-session` (0 bad, 38
  known), `test-session-wasm` (0 bad, 40), `test-split-node`,
  `test-loader`, `test-student` and `test-checks` pass.
- `make test-session-node`, with `scload.mjs` or with `scload-rs` in its
  place: the same 11,926 results, 2 bad (41 known). These are the 2 the
  battery already gave before §61. `limits-externdecl` runs out of time,
  and in `n590` the engine crashes in Node where in wasmtime it does not
  find the `#include` (it is in the corpus's `IGNORE`).
- ASan (`libsc_rt`, which calls `env.wasm_stack`) works. A
  use-after-free comes out the same in the native `scload` and in
  `scload-rs`.
- A program takes ~0.3 s in `scload-rs`, against 0.15 s in `scload` and
  0.1 s in `scload.mjs`. Node compiles the whole `sc.wasm` (39 MB) each
  time. A loop of 100 thousand `malloc` calls with ASan costs 0.03 s more
  than without.
- `sc.wasm` grows by 424 KB (38.91 → 39.34 MB, +1.1%). With zstd -19, it
  goes from 7.55 → 7.67 MB (+122 KB). Of what comes in, `cpp_demangle`
  (the C++ names in the messages) is 180 KB, the loader 75 KB,
  `wasmparser` 74 KB, and the std and `hashbrown` about 90 KB. `gimli`
  and `addr2line` drop out with LTO, because `sc_scload` does not read
  DWARF.

**Seen on the way.**
- §63 changed what `scload` says about a trap (`scload: wasm trap: ...`,
  without wasmtime's backtrace) and, by accident, lost the "undefined
  symbol: X" of a call to a function that does not exist yet. The name
  now comes from the world's `missing` global, which only the `Machine`
  read. The bases rebuilt today stored the new output, and only
  `test-session-node` caught it (`scload.mjs` still spoke like wasmtime).
  Now `Loader::why` gives the name to both `scload` binaries,
  `scload.mjs` says the same as they do, and the bases (`native-base`,
  `session-base`, `session-wasm-base`) were rebuilt.
- `loader.mjs` called `main(0, 0)`, and V8 rejects the wrong arity when
  the call comes from JS. A `main(int argc)` (`alias-4`) failed. Now it
  passes only the arguments that `main` receives.


## 65. The cell that fails without `fork` and without the host: the snapshot inside the session

**The question.** With the `Machine` and the loader inside `sc.wasm`, a
memory snapshot taken by the host (`engine.mjs`, §56) would also undo
their state, and we do not want anything specific to sc in `env`. Making
clang undo everything the cell left does not terminate (§25). Decided.
The copy stays inside `sc.wasm`.

**How it works.** `sc_session_cell` itself (`engine/sc_api.cpp`, only in
wasm, with `SC_SNAPSHOT` in the environment) copies the linear memory
before each cell that is not a replay. It puts the copy back before it
returns if the cell fails or is a type, and keeps only the diagnostics
and the names. During the call only the engine runs, and the rest of
what lives in memory goes back to the same values. Two things stay out.
These are the stack (the cell's frames have already ended at the return)
and the package image, which is only read.

The copy sits at the top of memory, above the heap. The `sbrk` of
`sc_api.cpp` replaces the one in wasi-libc, gives `malloc` the memory
below the copy, and raises the copy when the heap grows. The heap sees
what it would see without the copy (and with the host's snapshot), the
same addresses. The copy has the exact size of what it copies, and what
the cell said goes right after it, with the memory grown to fit (before,
a `malloc` buffer with 1 MiB of slack cut the 66 MB of diagnostics of
`limits-fndefn`).

A trap or an `exit` from clang interrupts the call, but the copy
survives. The host calls `sc_session_recover`, which sets
`__stack_pointer` to the top of the stack (a `global.set` in asm, before
any frame) and puts the memory back, and it answers `SC_ERROR`. Without a
copy it answers `SC_LOST`, and the `Repl` replays the log. `engine.mjs`
with `SC_SNAPSHOT=engine` takes no snapshot and does this.

**The battery.** `test-session-node` in this mode gives the same result
as the run with the host's snapshot (11926 results. The only bad one is
`limits-externdecl`, which already ran out of time). Comparing the raw
files, only these differ:
- 2 `mem-`: `memory.size`, with the copy;
- 2 `ses-` with `/tmp/<name>-<random>.pcm` in the message;
- 3 `ses-` (`fallthrough`, `noderef`) with the diagnostics in another
  order. The battery compares the sorted lines, and the order already
  differs between the cut and the full `sc.wasm` (`session-wasm` ×
  `session-wasm-base`). It depends on the layout, not on the copy.

The first prototype lost the session in the 131 cases with a trap or an
`exit` (47 `crash-`, 84 `ses-`). With `sc_session_recover` they come out
equal, byte for byte, to the host's snapshot.

**The time and the memory** (default Node, 100 cells of `mem-`, then 20
pairs of a cell with an error and a good cell. The machine was loaded,
load 35–66, and the numbers vary up to 2× between runs):

| | memory | accepted cell | cell with an error | copy | restore |
| --- | ---: | ---: | ---: | ---: | ---: |
| C, host | 33 MB | 10–23 ms | 6–14 ms | 2.8–6.4 ms | 2.8–6.0 ms |
| C, engine | 40 MB | 11 ms | 3–3.5 ms | 1.3 ms | 1.4 ms |
| C++, host | 55 MB | 18–52 ms | 13–48 ms | 6–22 ms | 6–22 ms |
| C++, engine | 85 MB | 23–26 ms | 9–20 ms | 4.5–8 ms | 4.1–8.3 ms |

Only the heap above the package image is copied. That is +7 MB in C and
+30 MB in C++ (the prototype, with the buffer in the heap and 25% of
slack, reached +37 MB). The copy inside wasm (`memcpy`) costs less than
the one in JS. Native keeps `fork`.

## 66. The `Machine` inside `sc.wasm`

**The step.** The `Machine` moved from the `sc` binary to the library
(`src/machine.rs`, the `machine` feature). It is generic over a
`Platform`, which gives the compiler, the runtime of the code, the
package files and the colors:
- in the native `sc` (`src/bin/sc/platform.rs`), the child with `fork`,
  wasmtime and the embedded package;
- in `sc.wasm` (`wasm/lib.rs`), the session in the module itself with the
  copy of §65 on (`sc_session_snapshot`), the host's primitives and the
  module's package (`sc_pack_read`).

The `Repl` (the log that replays a lost session) became generic over a
`Compiler`. The `Child` is one of them. The child's side of the protocol,
which uses `fork`, went to `src/engine/session/serve.rs`, and the engine
API compiles to wasm32-wasip1. Bindgen runs for the target, with
`-fvisibility=default`. The wasm clang hides what it declares, and bindgen
drops the hidden functions.

The REPL loop (`sc::machine::shell`) is also a single one. The native
`sc` reads it with rustyline and adds `:theme`. `sc.wasm` reads it from
stdin.

**The compiler's trap.** With the `Machine` inside, a trap or an `exit`
from clang would also undo its frames. The host sees only the
outer call, and we do not want it to know about sc. So the `Machine`
calls the cell *through the host*, with `wasm_call` on the module's own
`sc_session_cell`. This needs one more generic primitive. In
`wasm_export`, instance 0 is the module that imports the primitives. The
trap comes back as an error to `wasm_call`, and the `Machine`'s frames,
above the cell's, stay alive. `sc_session_restore` puts the memory back
without touching `__stack_pointer`, and the caller's epilogue restores
it.

One caveat is that the copy has to go back before any allocation. The
error that `Host::call` builds (the message and the frames) lives in the
heap, and putting the memory back afterwards would free it underneath.
The first test gave an out-of-bounds memory access. `Local` uses
`host::call`, which does not allocate, puts the copy back, and only then
reads the error.

**The exports.**
- `sc_cli`: `sc` with the WASI arguments (`[-q] [-x LANG] [--safety
  LEVEL] [FILE [ARG]...]`). With a file, it runs the program. Without
  one, it runs the REPL on stdin, as `sc`'s rustyline reads it without a
  terminal. `test/node/sc-rs` calls it in Node with only `wasm.mjs`.
- `repl_new`, `repl_run`, `repl_ready`, `repl_destroy`,
  `string_allocate` and `string_deallocate`: the API of simplecode's
  `ENGINE.md`, with the configuration `lang=c|c++ safety=… color=0|1`.

**The batteries.**
- `test/student.py test/node/sc-rs`: 66 runs, equal to `expected.txt`.
- `test/checks.py test/node/sc-rs`: only the frames differ (`at f
  (arquivo:linha:coluna)`, "the block was freed …"). The rest comes out
  the same, and so does the cell that crashes clang
  (`compiler-crash.c.repl`). V8's frames give the function and the
  offset, but not the instance, and `Loader::symbols` finds the DWARF
  through the instance. This is the open item of the frames in the
  browser. `wasm.mjs` now strips from the names the `<module>.` that V8
  puts in front and accepts C++ names with spaces, and so it finds
  `__sc_rt_report` (before, it said "aborted").

In native, `test-split`, `test-session`, `test-student`, `test-checks`,
`test-proto` and `test-loader` pass, and so do `test-split-wasm`,
`test-session-wasm` and `test-split-node-rs`. `make test-student-node-rs`
runs the first battery above.

**The time** is not measured yet. The machine had load 100–550. Each run
of `sc-rs` compiles the whole `sc.wasm` in Node, like `scload-rs` (§64).

**The size.** `sc.wasm` goes from 39.34 MB (§64) to 39.55 MB (+202 KB).
With zstd -19, it goes from 7.67 → 7.74 MB. `gimli` and `addr2line` come
in now, because the `Machine` reads the DWARF.

## 67. The simplecode API for engines that make modules, with sgleam

Sgleam will replace QuickJS with Gleam's WebAssembly backend, which makes
one module per REPL entry. This is the same need as sc's. The two agents
agreed on a single set of generic primitives in the `env` of simplecode's
`ENGINE.md` (section "Modules"). It is the set of `src/loader/host.rs`
with these changes:

- **`wasm_drop(handle)`**, new. The `Host` keeps each handle it receives
  (instances, exports, created globals) and releases them in `Drop`,
  together with the `Loader`. Without this, each Run of the page left the
  previous session's instances and memory in the host. In Node, a
  `repl_destroy` releases 1555 handles. `host::export` is `wasm_export`
  without registration, for the `sc_session_cell` that stays forever in a
  `OnceLock`.
- **The defaults (kind 0)** are the engine's WASI on the memory that the
  module imports or, if it imports none, on the `memory` it exports (a
  request from sgleam, whose modules have their own memory and GC types),
  plus `env.check_interrupt` and `env.wasm_stack`. A WASI function that
  the page's polyfill lacks answers `ENOSYS`. A program's libc imports
  `clock_res_get` and others that almost none call.
- **A handle of kind 1 can have a GC type**, and the host passes it with
  no check beyond the instantiation. This is how Gleam's entry N+1
  imports entry N.
- **Stop**: the normal way is for the module to poll
  `env.check_interrupt` (Gleam will have a counter in the generated
  code). Sc does not instrument the cells' code, because that would shift
  the offsets that the DWARF of the crash reports uses. Plan B stays, in
  the page. 1 s after a Stop with no answer, the page ends the worker,
  starts another, keeps the level, panels and history, and says that the
  REPL lost the entries. It repeats nothing by itself. The entries would
  print and read again, and a Run that enters a loop would enter it
  again.

**The missing exports** for `ENGINE.md`: `format` (it returns 0, sc does
not format), `version`, `cstr_deallocate` and `repl_complete`. Completion
and the keywords moved from `src/bin/sc/reader.rs` to `sc::machine::shell`
(`KEYWORDS`, `candidates`, `complete`), which the native `sc` and
`sc.wasm` use.

**In simplecode** (not committed, for the user's review):
`src/wasm_host.ts` (the primitives on the browser's WebAssembly. The
modules use the same WASI object as the engine, and `host.buffer()` says
which memory the current WASI call reads), the C and C++ languages
(`lang=c` and `lang=c++` in the new `LangConfig.config`), `fd_pread` in
the polyfill, the `_initialize` of a reactor engine, and Plan B for Stop.
Simplecode's `make test-all` gives 636 tests, 0 failures, with sc in
`engine_test.ts`. `check-imports` accepts the 8 `env` imports of
`sc.wasm`.

In sc, `test-proto`, `test-student`, `test-checks`, `test-split-node-rs`
(0 differ), `test-student-node-rs` and `test-session-node` (11926 results,
0 bad) pass.

**The size**: 39.56 MB (+11 KB over §66). With zstd -19, 7.75 MB.

## 68. `sc` with only `sc.wasm`: the size and the time

**The question.** The native `sc` compiles with the engine in a child,
with `fork` before each cell, a process group and `SIGIO`. All of this is
Unix only. Running the session through `sc.wasm` in the process itself,
with the snapshot of §65, as in the browser, would remove the need for
the native engine and would bring `sc` (and Ctrl-C) to Windows. It is not
decided. These are the measurements to decide.

**The size.** The `.cwasm` of `sc.wasm` (`build/sc-wasm.cwasm`):

| | MB |
| --- | ---: |
| `.text` | 42.7 |
| `.rodata.wasm` (the data, with the package image, §59) | 21.4 |
| `.wasmtime.addrmap` | 6.9 |
| the rest (`eh_frame`, `traps`, ...) | 4.3 |
| total | 77.3 |
| total, zstd -19 | 20.2 |

Decompressing the 20 MB takes 0.23–0.46 s on each run. Without the
package image in the data (the native `sc` already has it, and would give
it through the host, §58) the `.cwasm` would drop to ~56 MB, estimated.
Today `build/sc` is 46.2 MB (16.3 with zstd), the dist `sc` 38.2 MB, and
the dist `scload` (wasmtime and the loader, no engine) 7.5 MB.

**The time.** One session through `--replay`, with the `#include` lines in
one cell and 20 functions, and a cell with an error every 4. `build/sc`
(the cut native engine) against `build/sc-wasm` (the cut `sc.wasm` on
wasmtime, with `.cwasm`). Both use the child and `fork`, so the
difference is the engine's. The snapshot of §65 would replace `fork` with
a copy of 1.3–8 ms. Two rounds of 5, alternated, on a machine with load
25–40 on 28 cores:

| | native | wasm | ratio |
| --- | ---: | ---: | ---: |
| C, whole session (minimum) | 0.23–0.25 s | 0.29–0.31 s | 1.25 |
| C, 1st cell with the start (minimum) | 36–39 ms | 42–44 ms | 1.15 |
| C, accepted cell (median) | 10–12 ms | 13–15 ms | 1.3 |
| C, cell with an error (median) | 3.1–3.7 ms | 4.6 ms | 1.3 |
| C++, whole session (minimum) | 3.54–3.57 s | 5.69–5.74 s | 1.6 |
| C++, 1st cell, 5 headers (minimum) | 1.92–1.94 s | 3.34–3.36 s | 1.75 |
| C++, accepted cell (median) | 77–79 ms | 106–108 ms | 1.4 |
| C++, cell with an error (median) | 10 ms | 10–11 ms | 1.0 |

The engine in wasm is 1.3–1.75× slower. What the user feels is the cell
with the C++ headers, 1.4 s more. In the first round, with the `.cwasm`
out of the page cache, the first C cell took 301 ms in wasm (73 in
native).

## 69. `sc` with only `sc.wasm`: done

**The decision** (after §68): a single form. `sc` no longer has a native
engine, a child or `fork`. The compiler is the `sc.wasm` session in the
process itself, on wasmtime (`src/engine/guest.rs`, the `Guest`), and the
`Machine` stays native, with the reader and the loader on wasmtime. This
is option A. Option B, everything inside `sc.wasm` as in simplecode,
would need the `env.wasm_*` primitives on wasmtime and the reader through
the exports. The `.cwasm` of `sc.wasm` goes into the binary with zstd -19
(the choice between compressed and not, §68).

**How the `Guest` works:**
- the session with `SC_SNAPSHOT` (§65). A cell with an error, a crash or
  an `exit` from clang go back to the state before the cell through
  `sc_session_recover`. Without the copy (a replay) the session is lost
  and the `Repl` replays the log in another instance;
- Ctrl-C, the timeout and the memory use the epoch and a
  `ResourceLimiter`. The epoch belongs to the `Engine`, which the
  instances share, so each store decides in the callback whether the call
  stops. It stops if there was a Ctrl-C since the call started, or if the
  call's deadline passed. A watchdog moves the epoch every 100 ms only
  while there is a call with a deadline. Stopped this way, the cell is an
  error and the session goes on. Before, the child died and the log was
  replayed. The module's memory, with the copy, goes up to twice the
  limit. Above the limit, after an accepted cell, the `Repl` replays the
  log in another instance (`over`);
- the files. `sc.wasm` reads the session's headers and libraries from the
  package inside it (`sc_pack`) and sees the host's disk through `/` (on
  Windows, the drive of the directory as `/C:`), as the native engine
  did;
- `SC_CWASM` puts another `sc.wasm` in place of the embedded one,
  compiled by `sc --precompile`. The uncut one makes the bases, and
  `learn-session` uses the one it is learning. The trap of a cut function
  gives its name on stderr (frame 0, `__wrap_`).

**What went away:** the child (`child.rs`), the protocol (`wire.rs` and
the one in `sc.h`), `serve` with `fork` and `SIGIO`, the test host
`build/sc-wasm` (`wasm.rs`, the `wasm` feature, tokio), the engine linked
into `sc` (`SC_LINK_ARGS`, `sc --cc1`, `sc --ld`, `sc --engine`), the
host build of LLVM (`build/host`, `engine-host`, `trim-host`,
`link.py --native`, `cuts/symbols.host.used`), `make native`,
`test-native`, `learn-native`, `test-session-wasm*`, `test-split-wasm`,
`test-session-node` and the child in Node (`test/node/engine.mjs`,
`sc-node`). The `sc` package kept only the `.args` files and the loader's
libraries. The headers are in `sc.wasm`. The corpus's `native-base` (the
native one-shot) gave way to `base` (`sc-full.wasm`) in `split-` and
`ses-`. The batteries' `std.pcm` comes from `sc.wasm` in `wasmtime`. It
was 24 files, +450 −916 lines, not counting the removed ones.
`cuts/symbols.session.used` still has what the native engine called
(`unll` matches the names). Learning it from scratch in wasm only would
shrink it.

**The batteries** (machine with load 20–240 on 28 cores, from other
sessions and from the batteries themselves):
- `make test`: 17,715 files, 0 differences;
- `test-session`: 11,926 results, 0 differences from the new base (made
  with `sc-full.cwasm`). `test-session-warn`: the same;
- `test-split`: 1,947 programs, 10,541 cells, 0 differences from `pic-`;
- `test-student` (66 runs) and `test-checks` (165) equal to the expected
  output;
- `test-proto` (`tests/session.rs` ported to the `Guest`, covering cells,
  crash, undo, limits, Enter, values, `:type`) and `test-loader` pass.
  So does clippy with and without the `engine` feature.

**The CI's clippy with the `engine` feature.** `guest.rs` compiles only
with that feature, and the CI has no LLVM. Under clippy (`cfg(clippy)`,
which `clippy-driver` itself turns on, also in `build.rs`) `build.rs` does
not call make or build the pack and the `.cwasm`, and `PACK` and `CWASM`
stay empty. Outside clippy nothing changes. `make check` runs both forms.
In a copy of the repository without `build/`, the one with `engine` takes
3 min, almost all of it compiling wasmtime.

The new base differs from the one of the old `sc-wasm` (child and `fork`,
with only the repository root open) in 5 results. One is
`limits-externdecl`, which goes past 120 s with the machine loaded (49 s
alone). The corpus's `wev-base` already has TIMEOUT for it, so in
`test-session-warn` it comes out bad whenever the session ends. It is now
in `SKIP` of `test/session.py`, together with the ones that wasm-ld is
slow to demangle, so that the battery does not depend on the machine's
load. The other 4 change because of the host's disk. Three write modules
to `/tmp` (`#pragma clang module build`) and one reads `/dev` in the
`#embed`, as in native. `microsoft-abi-emit-dependent` goes back to
`KNOWN` (the corpus runs without `/tmp`).

**The size and the time:**

| | before (§68) | now |
| --- | ---: | ---: |
| dist `sc` | 38.2 MB | 32.6 MB |
| dist `sc`, zstd -19 | | 26.6 MB |
| release `sc` (not stripped) | 46.2 MB | 47.7 MB |

`sc hello.c` takes 0.35–0.41 s, with a 194 MB peak (load ~18).
Decompressing the 20 MB of the `.cwasm` (0.2–0.4 s, §68) and copying it
to executable memory weigh on this. `sc m.cpp` with `<iostream>` takes
3.6 s. What §68 measured of the cells still holds. The engine in wasm is
1.3–1.75× slower, more so in the cell with the C++ headers.

**Ctrl-C** reaches the `Guest` through the loader's handler
(`on_ctrl_c`), and in a pipe it stops the compilation (`sc: interrupted`,
the session goes on). In the terminal it does not reach the compilation
that Enter runs (`Machine::enter`, inside `readline`). Rustyline swaps
the SIGINT handler and the terminal is in raw mode. It was like this
with the child too.

## 70. Ctrl-C in the compilation on Enter, and the session that a trap corrupted

**`make learn-session` with wasm only.** Rebuilt from scratch, the list
`cuts/symbols.session.used` has 67 functions, all with the names of
`sc.wasm`. The `unll()` of `scripts/link.py`, which translated the names
of the native `sc-engine`, went away. `sc.wasm` keeps the same size
(39,561,349 bytes). `make test`, `test-session` and `test-split` show no
differences.

**Ctrl-C in the terminal.** Enter compiles the cell inside rustyline's
`readline` (`EnterHandler` → `Machine::enter`), with the terminal in raw
mode. On Unix that means no `ISIG` (and rustyline's SIGINT handler, which
writes to a pipe). On Windows it means no `ENABLE_PROCESSED_INPUT`.
Ctrl-C becomes a key, read only after the compilation (3.6 s for
`#include <iostream>`). Now a guard, `sc::loader::native::InRawMode`
(`src/loader/native/ctrl_c.rs`), wraps `enter`:

- Unix: it puts sc's handler in place of rustyline's and only then turns
  on `ISIG`, with `VQUIT`, `VSUSP` (and `VSTATUS` on macOS) off, so that
  Ctrl-\ and Ctrl-Z do not kill or suspend sc. On the way out, it undoes
  these in reverse order (so a Ctrl-C in the window does not fall into
  rustyline's pipe). What was typed during the compilation is lost with
  the Ctrl-C, as in a shell.
- Windows: it turns on `ENABLE_PROCESSED_INPUT` and, if there was a
  Ctrl-C, flushes the input on the way out (`FlushConsoleInputBuffer`).
  The handler is now the console's (`SetConsoleCtrlHandler`, TRUE only
  for `CTRL_C_EVENT`), not the CRT's `signal`, which resets SIGINT to
  `SIG_DFL` before it calls the handler. Because of that, the 2nd Ctrl-C
  killed sc (in a pipe too). `windows-sys` 0.61, the version rustyline
  uses, is in the `native` feature.

In the `Guest`, only the compilation of a cell stops on Ctrl-C (it has
`recover`). `depth` and `new_session` do not, and `recover` does not
either (before, a 2nd Ctrl-C in it lost the session).

`make test-ctrl-c` (`test/ctrlc.py`, on a pty, `TERM=xterm`, because with
`TERM=dumb` rustyline does not use raw mode) checks five cases, and after
each one `1+2` still gives `(int) 3`:
- Ctrl-C in the compilation on Enter gives `sc: interrupted`;
- two Ctrl-C in a row during it;
- Ctrl-\ and Ctrl-Z before it do nothing;
- Ctrl-C at the prompt still discards the line;
- two Ctrl-C in the student's code (`while (1);`).

On Windows, `test/ctrlc.py ssh:HOST` runs the VM's `sc.exe` through
OpenSSH, which gives it a ConPTY (`^C` becomes `CTRL_C_EVENT`). All five
pass on a Windows 10 VM, with `sc.exe` compiled here (`cargo xwin build
--target x86_64-pc-windows-msvc`). Before, `sc` did not even compile on
Windows (the CI compiles only the loader there), and getting here took
three fixes:
- Bindgen gave the enum constants of `engine/sc.h` the type `i32`,
  because in the MSVC ABI an enum is an `int`. For MSVC, `build.rs` passes
  the GNU target to clang. The layouts are the same, and the constants
  stay `u32` as on the other targets.
- The pack's paths on Windows came with `\` (backhand's `fullpath` is a
  `PathBuf`), and `repl/c-on.args` was not found. `pack.rs` now joins the
  components with `/`.
- `sc.exe` depended on `VCRUNTIME140.dll`, which a Windows without the
  VC++ Redistributable lacks (`STATUS_DLL_NOT_FOUND`). With `-C
  target-feature=+crt-static`, only the UCRT remains, and it comes with
  Windows 10. This is now in `.cargo/config.toml`, for both msvc targets.

**The session that a trap corrupted.** This test failed sometimes. After
the Ctrl-C, `1+2` returned `Lost` with no message and no result. It also
happened in a pipe, so it was already like this before (and with the
time limit, which also stops the compilation at any point). With no load,
it failed ~1 in 30. With the machine loaded (96 busy loops), 3 in 14. The
cause is that the snapshot's `sbrk` (§65) raises the copy (`memmove`,
~7.9 MB in the `<iostream>` cell) and only then advances `Brk`. A trap
between the two let `recover` restore from where the copy no longer was.
With instrumentation, the 3 of 18 that failed were exactly the ones that
stopped inside `sbrk`.

The fix (`engine/sc_api.cpp`):
- `Brk`, `Snap` and the state of the move (`Mv`) are no longer in the
  copy (they are holes, like the stack and the pack), so putting the copy
  back does not change them. Before, a trap in the middle of the
  `restore` of a failed cell could leave `Brk` with the copy's value.
- `sbrk` moves the copy in pieces of at most `N` bytes, from the top
  down, and records how much it moved. Each piece goes where there is
  nothing still to move, so the piece the trap stopped can be moved
  again. `recover` finishes the move if `Brk` has not advanced yet.
- `__atomic_signal_fence` between the steps, so the compiler does not
  reorder the writes.

With the machine loaded, 0 in 30 in the pipe and 0 in 15 on the pty.
`sc.wasm` grows by 400 bytes (39,561,749).

**The `.pcm` files in `/tmp`.** `#pragma clang module build X` (only in
clang's tests, and `test-session` and `test-split` go through three of
them) writes `/tmp/X-*.pcm`, and clang deletes it when the
`CompilerInstance` ends (`clearOutputFiles`). The sc session does not end
that way. The process ends with the session alive, and each battery left
4 files in `/tmp` (the native clang left none). Now the `Guest` gives
`sc.wasm` a `TMPDIR` of its own (`$TMPDIR/sc-<pid>-<n>`, only in the
guest's environment) and deletes it in `Drop`, which also covers the
cells that fail and go back through the snapshot. An sc killed by a
signal (SIGKILL, the terminal closed) still leaves the directory.
`test/ctrlc.py` now ends sc with Ctrl-D.

**The run time in the corpus.** `20040811-1-strict` went past the 60 s of
run time of `test/corpus.py` with the machine loaded (load ~180. Alone it
takes 47 s, compilation and all). The limit becomes 300 s, the same as
for compilation and linking. No program in the corpus loops, so only one
that did would pay this cost.

**The start of `sc`.** `sc hello.c` takes 0.39–0.50 s (194 MB peak). Most
of it is decompressing the embedded `.cwasm` (84 MB decompressed),
0.19–0.31 s with the machine loaded (load 28–85). Compared on the same
machine (`zstd -d`/`lz4 -d` in memory):

| format | size | decompress |
| --- | ---: | ---: |
| zstd -19 (the current one) | 21 MB | 0.19 s |
| zstd -9 | 24 MB | 0.22 s |
| zstd -3 | 27 MB | 0.24 s |
| zstd -22 --long | 21 MB | 0.36 s |
| lz4 -9 | 32 MB | 0.14 s |

Zstd -19 stays. Lowering the level does not speed it up (the larger file
costs more than the level saves), and lz4 saves ~0.05 s for 11 MB more.
Only a disk cache (the decompressed `.cwasm`, through mmap) would bring
the start close to zero, and §52–54 ruled that out.

**sc for Windows.** `make dist-windows` uses `cargo xwin` and the `dist`
profile to build `build/cargo/x86_64-pc-windows-msvc/dist/sc.exe`
(31.7 MB, 41.5 MB in the release profile) and `scload.exe` (6.7 MB). The
Linux ones are 32.6 MB and 7.5 MB. The build takes 5.5 min. The VM's
sc.exe passes `test/ctrlc.py ssh:win` and the C++ exceptions, and exits with 134 on an uncaught
exception. `sc hello.c` takes 0.8–0.9 s on the VM. The
aarch64-pc-windows-msvc target stays out, because there is nowhere to
test it.

**The release in the CI.** `.github/workflows/release.yaml`, like
sgleam's. A `v*` tag (which has to be the version in `Cargo.toml`)
builds, on each native target, `sc-VERSION-TARGET.tar.gz` or `.zip`, with
the sha256, and a draft release. There are six targets, sgleam's five
and aarch64-pc-windows-msvc (`windows-11-arm`, which also joins the
matrix of `ci.yaml`). What sc embeds and does not depend on the target
(`sc.wasm`, `build/pack/repl` and the five libraries of the pack, 8 MB
in `tar.zst`) comes from a single job and goes to the others as an
artifact. Each target compiles sc around it with `MAKELEVEL=1`, as under
make, so that `build.rs` does not call `make sc-deps`. Before it goes to the release, the binary from the archive
compiles and runs a C program and a C++ program with an exception.

The pin's LLVM costs, by the `.ninja_log` files here (the sum of the
tasks, only for the targets that make asks for), ~274 min in the native
tools and ~267 min in the wasm libraries, plus ~6 min in the runtimes.
That is ~2.3 h on a 4-core runner (the public repository's
`ubuntu-24.04`) and ~4.6 h on a 2-core one (the private repository's).
So it lives in `.github/workflows/llvm.yaml`, in two jobs (each well
below the 6 h limit), with a cache keyed by `make llvm-key` and `llvm/*`.
The cache holds `build/native/bin` and `lib/clang` (~150 MB) and all of
`build/wasm` (~1 GB, because `link.py` rebuilds the `.a` archives from
the objects). A cache saved on a tag serves only that tag, and GitHub
deletes what goes 7 days without use. So `llvm.yaml` runs on main when
the pin changes and every week, and the release calls it. In a clean
clone, with only the cache, `make -n sc-deps` calls no LLVM `ninja`.
We also ran here the `cargo build --profile dist` of a single target with the artifact, the
archive and the test of the binary.

The first run in the CI showed two things. With the private repository,
no job even started ("recent account payments have failed or your
spending limit needs to be increased"). The private repository's quota
of minutes had run out, and the LLVM jobs would not even fit in it. With
the public repository, the minutes of the standard runners do not count.
And `llvm.yaml` stopped at lld's `patch`. `llvm/lld-files.patch` had been
regenerated (d34ad05) with `lld-undef-index.patch` already applied, and
it carried that patch's hunk in `lld/wasm/InputFiles.cpp` again. Here
`build/src` is not rebuilt from scratch, and nobody noticed. Without the
hunk (1a4fc6e), the 16 patches, in the Makefile's order, on the pin's
files give exactly `build/src`, and a patch covers every file that
`build/src` changes. In the same run `ci.yaml` passed in all 9 jobs, with
`windows-11-arm` (loader tests and clippy with the engine on
aarch64-pc-windows-msvc). Its image has 7-Zip, OpenSSL and LLVM 20, which
the release uses.

In the actual CI (run 42262528778, 4-core `ubuntu-24.04`), the native
tools job took 48 min and the wasm libraries job 34 min. That is 1h22 in
all, much less than the `.ninja_log` estimate, which comes from a
machine with a different load. The caches take 46 MiB (`llvm-native-`)
and 277 MiB (`llvm-wasm-`) compressed, far from the 10 GB limit.

The first release run (run 42275091835) found the cache (the two jobs of
`llvm.yaml` took seconds) and stopped in `link.py`. The runner has
neither `wasm-tools`, which it calls by name, nor the pack's
`mksquashfs`, nor `llvm-cxxfilt`. The job installs the first two
(`wasm-tools` 1.220.0, the version used here). `link.py` now takes
`llvm-cxxfilt` from wasi-sdk, as it already does with `llvm-nm`. On the
12,777 names in `build/link-nm.json` it demangles the same as the
system's.

With this the whole release passed (run 42280578981, 30 min). The cache
took seconds, `sc.wasm` 5 min, and the six targets ran in parallel, from
6 min (aarch64 musl) to 24 min (macOS x86_64), each with the test of the
binary. The `sc-deps` artifact is 11.7 MB, and the archives (zip
artifact, close to the size of the `.tar.gz`/`.zip`) are 27.4 MB (x86_64
musl), 24.0 MB (aarch64 musl), 27.1 MB (x86_64 Windows), 23.6 MB
(aarch64 Windows), 27.2 MB (x86_64 macOS), 23.9 MB (aarch64 macOS).

## 71. A cut that only the size of a cell reaches

`make test-split` failed on `mod-m2_stl`. The last cell crashed the cut
`sc.wasm` and not the uncut one. On `build/sc-cut.wasm`, which keeps its
names, the trap is in the stub of
`clang::serialization::reader::HeaderFileInfoTrait::EqualKey`, called by
`IncrementalParser::Parse` through `HeaderSearch::getFileInfo` and
`ASTReader::GetHeaderFileInfo` on `std.pcm`. The table of a module's
headers hashes only the size of a file and its time
(`HeaderFileInfoTrait::ComputeHash`), and the module has no times. So
`EqualKey` runs only when the cell has the size of a header that
`std.pcm` knows. The cell had 1,092 bytes. With 5 bytes more the same
session passes. No corpus can learn a call like that, so
`cuts/symbols.txt` keeps the whole trait (`EqualKey` calls `getFile`,
which was cut too). `sc.wasm` grows from 39,561,749 to 39,563,151 bytes
(+1,402).
