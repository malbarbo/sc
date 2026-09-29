#!/usr/bin/env python3
"""Attribute twiggy per-function sizes of an optimized wasm to (lib, obj) using the wasm-ld map.
usage: attrib.py clang.map twiggy.csv > funcs.tsv   (size lib obj name)"""
import re, sys, csv, collections
line_re = re.compile(r'^\s*(\S+)\s+([0-9a-f]+)\s+([0-9a-f]+) (\s*)(\S.*)$')
out = None; cur = None; owner = {}
for line in open(sys.argv[1]):
    m = line_re.match(line)
    if not m: continue
    ind = len(m.group(4)); rest = m.group(5)
    if ind == 0: out = rest.split()[0]; continue
    if out != 'CODE': continue
    if ind == 8:
        mm = re.match(r'(.*?)(?:\((.*?)\))?:\((.*)\)$', rest)
        if not mm: cur = None; continue
        path, obj = mm.group(1), mm.group(2)
        cur = ('<obj>', path.split('/')[-1]) if obj is None else (path.split('/')[-1], obj)
    elif ind == 16 and cur:
        owner.setdefault(rest, cur)
w = csv.writer(sys.stdout, delimiter='\t', lineterminator='\n')
for row in csv.DictReader(open(sys.argv[2])):
    n = row['Name']; s = int(row['ShallowSize'])
    if n.startswith('data segment') or n.startswith('"') or n.startswith('elem') or n.startswith('import') or n.startswith('custom') or 'subsection' in n:
        continue
    lib, obj = owner.get(n, ('?', '?'))
    w.writerow([s, lib, obj.replace('.cpp.obj', ''), n])
