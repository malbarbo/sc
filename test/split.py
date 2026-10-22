#!/usr/bin/env python3
"""Phase 2 of PLANO-REPL.md: the programs of the corpus as REPL cells, through
a native engine with the protocol's loop (sc --replay, src/bin/sc/main.rs).

usage: split.py [<engine> [<outdir>]]   (build/sc, build/res/split)
ONLY=<name>,... (or a file of names) runs only those programs; JOBS sets the parallelism (48)

Each program the corpus runs (test/samples, c-testsuite, gcc torture
execute/, test/cxx, test/mod) is cut into one cell per top-level declaration
(test/cells.py), the cells go through `<engine> --replay`, which compiles
them in one child session and links each as a shared object, and the loader
runs the shared objects. The output must be the one of the program compiled
whole as a shared object (pic-, in the native base, build/res/native-base);
a program that does not compile whole must not compile as cells either.
KNOWN lists the programs that cannot, with the reason.

Writes <outdir>/<name>.txt (the cells' statuses and diagnostics, then the
run) and <outdir>/report.txt, and lists the programs that differ.
"""
import glob, os, re, shutil, subprocess, sys
from concurrent.futures import ThreadPoolExecutor
sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
from cells import JS, SCLOAD, expected, same, split, std_pcm
from corpuslib import only_names, run_all, source_name, sources

R = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
sys.path.insert(0, f'{R}/scripts')
from flags import PACK, session  # the engine's flags
B = f'{R}/build'
ENG = os.path.realpath(sys.argv[1] if len(sys.argv) > 1 else f'{B}/sc')
LIB = f'{B}/sysroot/lib/wasm32-wasip1'
OUT = os.path.realpath(sys.argv[2] if len(sys.argv) > 2 else f'{B}/res/split')
G = 'gcc-ts_gcc_testsuite_gcc.c-torture_execute_'
# where cells cannot be the program (ANALISE.md §24)
KNOWN = {
    'ct-00152': '#line in one cell does not reach the next: each cell is a file',
    G + '20050215-1': 'a directive inside a declaration: the splitter cuts the declaration',
    G + 'alias-2': 'an alias names a definition of an earlier cell',
    G + 'alias-4': 'an alias names a definition of an earlier cell',
    G + 'pr98304-2': "#include \"../../gcc.dg/...\": the -I of the cells finds it, the whole compile does not",
    G + '20090711-1': "div, with another signature than libc's: the next cell links against libc.so's",
}
KNOWN.update(JS)

def flags(cxx, gnu17, mod):
    # the pack sc embeds (PLANO-REPL.md §5.4), but for a host of sc.wasm
    # (<engine>.wasm), which reads the disk, and with std.pcm, which sc --cc1
    # builds from the disk
    root = B if mod or os.path.exists(f'{ENG}.wasm') else PACK
    return session(cxx, root, gnu17, mod, f'{OUT}/std.pcm')


def programs():
    """(path, name of its pic- result) of each program the corpus runs"""
    return [(p, source_name(p, R, B)) for p, runs in sources(R, B) if runs]


def run(sos):
    try:
        x = subprocess.run([SCLOAD, '-L', LIB] + sos, input=b'linha1\nlinha2\n', capture_output=True, timeout=20)
    except subprocess.TimeoutExpired as e:
        x = subprocess.CompletedProcess(e.cmd, 'timeout', e.stdout or b'', e.stderr or b'')
    return x.stdout.decode(errors='replace') + x.stderr.decode(errors='replace') + f'run exit={x.returncode}\n'


def one(prog):
    p, name = prog
    cxx, mod = p.endswith('.cpp'), '/test/mod/' in p
    gnu17 = p.startswith((f'{B}/c-testsuite/', f'{B}/gcc-ts/'))
    d = f'{OUT}/{name}'
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    ext = os.path.splitext(p)[1]
    files = []
    for i, c in enumerate(split(open(p, errors='surrogateescape').read())):
        f = f'{d}/c{i + 1:03}{ext}'
        open(f, 'w', errors='surrogateescape').write(c)
        files.append(f)
    try:
        # -I: the program's own includes, which the cells' directory does not have
        r = run_all([ENG, '--replay', d] + files + ['--', '-I', os.path.dirname(p)] + flags(cxx, gnu17, mod), 300)
        txt = r.stdout.decode(errors='replace') + r.stderr.decode(errors='replace') + f'repl exit={r.returncode}\n'
        ok = r.returncode == 0 and not re.search(r'^== \S+ (error|lost)$', txt, re.M)
    except subprocess.TimeoutExpired:
        txt, ok = 'repl TIMEOUT\n', False
    exp = expected(name)
    out = run(sorted(glob.glob(f'{d}/*.so'))) if ok else None
    txt += out or ''
    eq = same(out, exp)
    txt = txt.replace(OUT + '/', 'OUT/')
    open(f'{OUT}/{name}.txt', 'w').write(txt)
    shutil.rmtree(d)
    return name, len(files), eq, ok, exp is not None


def main():
    os.makedirs(OUT, exist_ok=True)
    progs = programs()
    if only := only_names():
        progs = [x for x in progs if x[1] in only]
    if any('/test/mod/' in p for p, _ in progs):
        std_pcm(ENG, OUT)
    with ThreadPoolExecutor(int(os.environ.get('JOBS', '48'))) as ex:
        rs = list(ex.map(one, progs))
    bad = [r for r in rs if not r[2] and r[0] not in KNOWN]
    lines = [f'{n:60} {c:4} cells  {"ok" if s else "known" if n in KNOWN else "DIFF"}  {"" if ok else "cells fail"}'
             f'{"" if e else " (whole fails)"}' for n, c, s, ok, e in rs]
    open(f'{OUT}/report.txt', 'w').write('\n'.join(lines) + '\n')
    for n, c, s, ok, e in bad:
        print('DIFF', n, '' if ok else '(cells fail)')
    print(f'{len(rs)} programs, {sum(r[1] for r in rs)} cells, {len(bad)} differ from pic-'
          f' ({sum(not r[2] and r[0] in KNOWN for r in rs)} known)')
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
