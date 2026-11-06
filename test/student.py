#!/usr/bin/env python3
"""A student's correct code (docs/plan-checks.md §9): the programs in
test/student through sc (make repl) at each level of --safety. What each one
gets, its exit status and its warnings by flag, must be what
test/student/expected.txt says; at the level on, no error.

usage: student.py [--update] [SC]   --update writes expected.txt; SC: by
default build/cargo/release/sc (make test-dist: the dist one)
"""
import collections, glob, os, re, subprocess, sys
from concurrent.futures import ThreadPoolExecutor

R = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
ARGS = [a for a in sys.argv[1:] if a != '--update']
SC = os.path.realpath(ARGS[0]) if ARGS else f'{R}/build/cargo/release/sc'
EXPECTED = f'{R}/test/student/expected.txt'
LEVELS = ('off', 'on', 'strict')
STDIN = b'1\n2\n2\n9\n'  # the menu's options, lines for those that read them


def one(job):
    prog, level = job
    r = subprocess.run([SC, f'--safety={level}', prog], input=STDIN, capture_output=True, timeout=120,
                       env=os.environ | {'NO_COLOR': '1'})
    err = r.stderr.decode(errors='replace')
    flags = collections.Counter(re.findall(r'^\S+: (?:warning|error): .* \[(?:-Werror,)?(-W[^],]+)', err, re.M))
    errors = len(re.findall(r'^\S+: error: ', err, re.M))
    w = ' '.join(f'{f}:{n}' if n > 1 else f for f, n in sorted(flags.items()))
    return f'{os.path.basename(prog)} {level} exit={r.returncode}' + (f' errors={errors}' if errors else '') + (f' {w}' if w else ''), errors


def main():
    progs = sorted(glob.glob(f'{R}/test/student/*.c') + glob.glob(f'{R}/test/student/*.cpp'))
    with ThreadPoolExecutor(os.cpu_count()) as ex:
        rs = list(ex.map(one, [(p, l) for p in progs for l in LEVELS]))
    lines = [l for l, _ in rs]
    if '--update' in sys.argv[1:]:
        open(EXPECTED, 'w').write('\n'.join(lines) + '\n')
        return
    bad = [l for l, e in rs if e and ' on ' in l]
    for l in bad:
        print('ERROR at on:', l)
    exp = open(EXPECTED).read().splitlines() if os.path.exists(EXPECTED) else []
    diff = [('-', l) for l in exp if l not in lines] + [('+', l) for l in lines if l not in exp]
    for s, l in diff:
        print(s, l)
    print(f'{len(progs)} programs, {len(lines)} runs, {len(diff)} lines differ from expected.txt')
    sys.exit(1 if bad or diff else 0)


if __name__ == '__main__':
    main()
