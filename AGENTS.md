# sc

A version of C and C++ for students: a REPL, a runner for single files, and
checks that catch the student's bugs where they happen. The compiler is clang
and wasm-ld built for wasm32-wasip1 as one module, `sc.wasm`, cut to what C23
and C++26 (with exceptions) at -O0 need. `engine/` is the C++ around clang
that `sc.wasm` exports, `cuts/` and `llvm/` make the cut LLVM, and `rt/` is
the runtime of the student's program. `src/` is the `sc` binary, which runs
the session of `sc.wasm` on wasmtime, and `wasm/` is the Rust that `sc.wasm`
links. `scripts/` builds and checks the cuts, and `test/` holds the corpus
and the batteries. `README.md` has the layout, the build and how the cuts
work. `NOTES.md` records the measurements behind each decision. Add to it
when a size or a decision changes.

## Build and test

```sh
make             # build/sc.wasm
make sysroot     # build/sysroot: wasi-sdk's libc, libc++ with exceptions from the pin
make test        # the full corpus (~4 min on 48 cores) against build/sc-full.wasm
make test-small  # the programs only
make checkinv    # after changing engine/sc_inv.cpp
make test-warn   # the full corpus with -Weverything
make learn       # after changing cuts/ or the LLVM pin, then make test and test-warn
                 # (and learn-session)
make learn-session # after make learn: what only the REPL's session calls
make test-proto  # src/engine, the Repl over sc.wasm's session (tests/session.rs), and sc's modules
make test-split  # the corpus as REPL cells, after changing the session or the loader
make test-session # the session's slices on build/sc against the uncut sc.wasm (SC_CWASM)
make test-session-warn # ses-, seq- and crash- with -Weverything
make test-split-node # test-split with the JS loader (test/node/loader.mjs) in place of scload
make test-split-node-rs # test-split with sc.wasm's loader on Node's primitives (test/node/wasm.mjs)
make test-student-node-rs # test-student with sc inside sc.wasm (the machine) on Node's primitives
make repl        # build/cargo/release/sc, the REPL (sc, sc FILE, sc help), unstripped:
                 # sc.wasm's session on wasmtime is its compiler (NOTES.md §68)
cargo build --release # the same sc (build.rs runs make sc-deps)
make dist        # build/cargo/dist/scload and sc: small, LTO (~4 min)
make dist-windows # the same sc.exe and scload.exe for x86_64-pc-windows-msvc (cargo-xwin)
make test-student  # test/student, correct code, at each --safety level (scripts/flags.py)
make test-checks # test/checks, one bug per program: what sc says of it at each level
make test-loader # src/loader.rs: Ctrl-C, many exceptions, after changing the loader or wasmtime (no engine)
make test-ctrl-c # Ctrl-C in sc's REPL on a pty (test/ctrlc.py), after changing the reader or Ctrl-C
make check       # clippy with and without the engine (no LLVM: under clippy build.rs embeds nothing) and on wasm/: the CI's (.github/workflows/ci.yaml)
make rs-wasm     # the Rust that sc.wasm links, alone
make test-dist   # before shipping: test-split, test-session, test-student, test-checks on the dist sc
```

The cuts keep the code of every warning. `make learn` runs the corpus with
`-Weverything` (`NOTES.md` §13).

A change to the engine or the cuts is done when `make test` reports no
difference, and `make test-session` none for the session of `sc.wasm`. For
an intended change in behaviour, add the file to `IGNORE` in
`test/corpuslib.py` with the reason. A change to `src/` is done when `make
check` and `make test-proto` pass. Do not leave test files in the root of
the repository.

## Scope

Change what was asked and nothing else. A problem you see on the way is
something to report, not to fix in the same change. Before you start, write
the subject of the commit. If it does not fit in one concrete line, the
change is more than one, and it becomes more than one commit.

## Conventions in the code

- `build/` is generated. Change the sources in `engine/`, `llvm/`, `cuts/`,
  `scripts/` and `test/`, never the copies in `build/`. The one exception is
  `build/src`, which is the LLVM pin with `llvm/*.patch` applied. When you
  change it there, regenerate the patch with `git diff` in `build/src`.
- Prefer a cut in `cuts/` to a patch in `llvm/`. A cut does not conflict
  when LLVM is rebased. Patch only where the saving is worth it.
- `engine/sc_inv.cpp` sets by hand the defaults that the option parser of
  clang would set. `make checkinv` must print SAME.
- The release (`.github/workflows/release.yaml`) builds `sc.wasm` once, on
  the LLVM that `.github/workflows/llvm.yaml` keeps in a cache, and each
  target compiles `sc` around it. A change to the pin or to `llvm/` costs a
  new LLVM build in the CI, about 1h20.
- Colour is decided in `src/bin/sc/reader.rs`, and nowhere else. A
  non-empty `NO_COLOR` keeps the escape codes out, as
  <https://no-color.org> asks, and a non-empty `FORCE_COLOR` asks for them
  where no terminal is watching.

# Writing

These rules cover every piece of English in the repository: comments, doc
comments, commit messages, and the docs.

## Comments

Delete first. Most comments should not exist. The name, the type, and the line
below already say it. Ask what the comment adds before you ask how it reads.

- Say why, not what. The reason for an assert, the case a branch guards, the
  invariant a caller has to keep.
- If the reader can see it in the line below, drop the comment.
- Never comment history. What the code used to do, a bug that was fixed, a
  function that is gone, none of that belongs in a comment about the code as
  it stands.
- Put the comment on the thing it describes. `// from the gleam project`
  belongs on the function that came from there, not at the top of the file.
- Check that the comment is true in every case the code covers. "only
  Windows reaches this" read well and was false.
- Keep it short. One or two lines is the norm. A paragraph needs a reason.
- A function that answers yes or no says so first: `Returns \`true\` if ...,
  \`false\` otherwise.` The details come after.

## How a sentence reads

Plain, active English. Subject, verb, object. The reader takes it in one word
at a time, left to right, without going back.

Avoid the constructions that make the reader do the grammar:

- **Passive voice.** It hides who acts.
  - No: `Nothing else is filtered.`
  - Yes: `Everything else stays.`
- **A preposition left at the end of a relative clause.** The object of the
  preposition sits at the head of the clause, so the reader only finds out at
  the end what to fill the gap with. Portuguese has no such construction,
  which makes it doubly hard to read.
  - No: `The path the compiled JavaScript names a source file by`
  - Yes: `The path the compiled JavaScript uses for a source file`
- **An elided verb.** Write the verb out.
  - No: `so only a relative path of plain names does`
  - Yes: `so the path has to be relative and made of plain names`
- **A pronoun for a noun that is far away.** Name the noun again.
  - No: `the head of the ones with a body`
  - Yes: `the head of a definition that has a body`
- **A verb and preposition that do not go together.** Use the ordinary one.
  - No: `What the definition takes of the input`
  - Yes: `How much of the input the definition covers`
- **A clause that ends in its verb.** The reader holds the noun until the
  verb arrives, which is the same trouble as a stranded preposition.
  - No: `A JavaScript context, and the runtime it runs in`
  - Yes: `A JavaScript context together with its runtime`
  - No: `the origin an svg text element takes`
  - Yes: `the origin of an svg text element`
- **A colon in the middle of a sentence.** It joins two halves and asks the
  reader to hold the first one. Write two sentences. A colon before a list
  or after a label is fine.
  - No: `One flag for the whole process: whatever runs stops at the next check.`
  - Yes: `One flag serves the whole process. Whatever runs stops at the next
    check.`
- **A dash in the middle of a sentence.** Same trouble as the colon. Write
  two sentences, or use a comma.
- **A metaphor for something the code states plainly.**
  - No: `a name that climbs out`, `however far the chain goes`
  - Yes: `a name with ..`, `directly or not`
- **A word that sounds clever.** `owe`, `pay for`, `travel`, `land`, `reach
  out`. Say `cost`, `go`, `is`, `call`.

## Commit messages

One change per commit. A refactor and the fix it prepares are two commits.
`engine: say each thing once`, which touched a macro, a cursor, the import
names, and the file list of a compile, should have been five.

The subject says what the change does to the code, in one line:

- A prefix for the part of the tree, then a lowercase sentence: `engine:`
  (`engine/`, `rt/`), `cuts:`, `llvm:`, `loader:` (`src/loader*`), `repl:`
  (`src/bin`, `src/engine`, `src/machine`), `wasm:`, `scripts:`, `test:`,
  `docs:`, `ci:`, and `all:` when it crosses them.
- Imperative or present, active voice. `repl: say that a directory is not a
  source file`.
- Name the concrete thing that changed and what happened to it. A function,
  a command, a flag, a message. The reader should know what to look for in
  the diff.
  - No: `all: ask the shell about the cursor, not just the text`
  - Yes: `all: change repl_ready to receive the cursor position`
  - No: `engine: tell the interrupt by the mark quickjs puts on it`
  - Yes: `engine: use JS_IsUncatchableError to detect an interrupt`
- No rhetorical contrast, no metaphor, no joke. `not just`, `instead of`, and
  `rather than` in a subject usually mean the subject is describing the old
  code instead of the new one.
- Under about 65 characters. No period at the end.

The body is optional. Write one when the subject leaves something out, which
is usually why:

- What was wrong, then what the change does. Two to four sentences, one
  paragraph, wrapped at 72 columns.
- No bullet lists, no list of files, no narration of the process (`also`,
  `while at it`, `cleaned up`). What is not worth a sentence is in the diff.
- No trailers. The message ends at the last line of the body. No
  `Co-Authored-By`, no `Signed-off-by`.

A good one:

```
ci: install wasm-tools and mksquashfs for the release

The first release run found the LLVM cache and stopped in link.py,
because the runner has neither wasm-tools nor mksquashfs. The engine job
installs both now, and link.py takes llvm-cxxfilt from wasi-sdk, as it
already did for llvm-nm.
```

## Docs, the README, and release notes

Prose, not lists of features. Show a thing where it is used, once. No counts
of modules or colors. No bold label followed by a dash.

Release notes open with the first sentence of the README, then the list of
changes. Each change is the subject of a commit, or the subjects of a few
commits joined, and not a rewrite of them.
