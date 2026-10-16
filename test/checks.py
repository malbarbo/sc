#!/usr/bin/env python3
"""One bug per program (PLANO-VERIFICACOES.md §9): what sc (make repl) says
of each program in test/checks at each level of --safety must be what
test/checks/expected.txt says: the output, the diagnostics and the exit
status. A NAME.c or NAME.cpp runs as `sc NAME`, with NAME.c.in or
NAME.cpp.in as its input if there is one; a NAME.c.repl or
NAME.cpp.repl goes, a line at a time, to the REPL.

usage: checks.py [--update]   --update writes expected.txt
"""
import difflib, glob, os, subprocess, sys
from concurrent.futures import ThreadPoolExecutor

R = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
SC = f'{R}/build/loader/release/sc'
D = f'{R}/test/checks'
EXPECTED = f'{D}/expected.txt'
LEVELS = ('off', 'on', 'strict')


def one(job):
    prog, level = job
    name = os.path.basename(prog)
    if name.endswith('.repl'):
        lang = name.split('.')[-2]
        cmd, stdin = [SC, '-q', f'-x={"c++" if lang == "cpp" else lang}', f'--safety={level}'], open(prog, 'rb').read()
    else:
        inp = f'{prog}.in'
        cmd, stdin = [SC, f'--safety={level}', name], open(inp, 'rb').read() if os.path.exists(inp) else b''
    r = subprocess.run(cmd, cwd=D, input=stdin, capture_output=True, timeout=120, env=os.environ | {'NO_COLOR': '1'})
    out = (r.stdout + r.stderr).decode(errors='replace')
    return f'== {name} {level} exit={r.returncode}\n{out}'


def main():
    progs = sorted(glob.glob(f'{D}/*.c') + glob.glob(f'{D}/*.cpp') + glob.glob(f'{D}/*.repl'))
    with ThreadPoolExecutor(os.cpu_count()) as ex:
        got = ''.join(ex.map(one, [(p, l) for p in progs for l in LEVELS]))
    if sys.argv[1:] == ['--update']:
        open(EXPECTED, 'w').write(got)
        return
    exp = open(EXPECTED).read() if os.path.exists(EXPECTED) else ''
    diff = list(difflib.unified_diff(exp.splitlines(), got.splitlines(), 'expected.txt', 'got', lineterm=''))
    print('\n'.join(diff))
    print(f'{len(progs)} programs, {len(progs) * len(LEVELS)} runs, {"differ" if diff else "as expected.txt"}')
    sys.exit(1 if diff else 0)


if __name__ == '__main__':
    main()
