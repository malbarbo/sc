#!/usr/bin/env python3
"""Initializer functions (llvm::initialize<X>Pass) needed by the wasm O0 pipeline.

Pass arguments come from `llc -O0 -debug-pass=Arguments` (plus extra args on the
command line); each is mapped to its class through INITIALIZE_PASS* in the
sources, and INITIALIZE_PASS_DEPENDENCY is followed transitively.
usage: o0passes.py <args...>  -> prints initializer names, one per line"""
import glob, re, sys
import os
SRC = os.path.join(os.path.dirname(os.path.dirname(os.path.realpath(__file__))), 'build/src/llvm/lib')
byarg, deps = {}, {}
for f in glob.glob(f'{SRC}/**/*.cpp', recursive=True):
    s = open(f, errors='replace').read()
    if 'INITIALIZE_PASS' not in s: continue
    dt = re.findall(r'#define DEBUG_TYPE\s+"([^"]+)"', s)
    for m in re.finditer(r'INITIALIZE_(?:PASS|PASS_BEGIN|PASS_WITH_OPTIONS|PASS_WITH_OPTIONS_BEGIN|AG_PASS|AG_PASS_BEGIN)\(\s*(\w+)\s*,\s*("([^"]*)"|\w+)', s):
        cls, arg = m.group(1), m.group(3) if m.group(3) is not None else (dt[0] if m.group(2) == 'DEBUG_TYPE' and dt else m.group(2))
        byarg.setdefault(arg, cls)
    for m in re.finditer(r'INITIALIZE_PASS_BEGIN\(\s*(\w+).*?INITIALIZE_PASS_END\(', s, re.S):
        deps.setdefault(m.group(1), set()).update(re.findall(r'INITIALIZE_PASS_DEPENDENCY\((\w+)\)', m.group(0)))
need, todo = set(), [byarg[a.lstrip('-')] for a in sys.argv[1:] if a.lstrip('-') in byarg]
missing = [a for a in sys.argv[1:] if a.lstrip('-') not in byarg]
while todo:
    c = todo.pop()
    if c in need: continue
    need.add(c); todo += deps.get(c, ())
for c in sorted(need): print(f'llvm::initialize{c}Pass(llvm::PassRegistry&)')
print('unmapped:', ' '.join(missing), file=sys.stderr)
