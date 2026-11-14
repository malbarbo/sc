#!/usr/bin/env python3
"""A Stop of the host in sc's REPL inside sc.wasm (test/node/sc-rs, whose
check_interrupt says 1 at the SC_STOP_AT-th check): a loop in a cell stops
with "interrupted", in C and in a C++ catch, and the cell after it sees
what the cells before defined. Without a Stop, a loop of more iterations
than a tick period and one in a catch run to their end.

usage: stop.py
"""
import os, subprocess, sys

R = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SC = f'{R}/test/node/sc-rs'

# (language, SC_STOP_AT, the cells, what the output has to contain)
CASES = [
    ('c', '1', 'int x = 41;\nwhile (1);\nx + 1\n', ['interrupted', '42']),
    ('c++', '1', 'int x = 41;\ntry { throw 1; } catch (...) { while (1); }\nx + 1\n', ['interrupted', '42']),
    ('c', '', 'long long s = 0;\nfor (int i = 0; i < 1000000; i++) s += i;\ns\n', ['499999500000']),
    ('c++', '', 'long long s = 0;\ntry { throw 1; } catch (...) { for (int i = 0; i < 1000000; i++) s += i; }\ns\n',
     ['499999500000']),
]

failed = 0
for lang, at, cells, want in CASES:
    env = dict(os.environ, SC_STOP_AT=at, NO_COLOR='1')
    out = subprocess.run([SC, '-q', '-x', lang], input=cells, capture_output=True, text=True, env=env, timeout=120)
    text = out.stdout + out.stderr
    missing = [w for w in want if w not in text]
    if missing or (not at and 'interrupted' in text):
        failed += 1
        print(f'FAIL {lang} SC_STOP_AT={at!r}: {cells!r}\n{text}')
print(f'{len(CASES)} cases, {failed} failed')
sys.exit(failed > 0)
