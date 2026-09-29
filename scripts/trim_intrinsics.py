#!/usr/bin/env python3
"""Trim the tablegen'd IntrinsicImpl.inc to the intrinsics a wasm-only compiler can use.

Intrinsic IDs, the per-target ID ranges and every ID-indexed table are kept, so
the result is ABI compatible with the rest of the build. Only strings go:
  * names of intrinsics of other targets become "" (lookupIntrinsicID of such a
    name then fails its binary search, as for any unknown llvm.* name);
  * the clang-builtin -> intrinsic tables of other targets are dropped.
usage: trim_intrinsics.py <in IntrinsicImpl.inc> <out> [keep-target ...]  (default: wasm)
"""
import re, sys
src, dst = sys.argv[1], sys.argv[2]
keep = set(sys.argv[3:] or ['wasm']) | {''}
L = open(src).read().split('\n')

def section(name):
    a = L.index(f'#ifdef {name}'); b = L.index(f'#endif // {name}')
    return a, b

# --- target ID ranges
a, b = section('GET_INTRINSIC_TARGET_DATA')
ranges = []
for l in L[a:b]:
    m = re.match(r'\s*\{"(\w*)", (\d+), (\d+)\},', l)
    if m and m.group(1) in keep:
        ranges.append((int(m.group(2)), int(m.group(2)) + int(m.group(3))))
kept_id = lambda i: i == 0 or any(lo <= i < hi for lo, hi in ranges)

def storage(lines):
    """[(offset, text)] of a "..\\0" string table body."""
    out, off = [], 0
    for l in lines:
        m = re.match(r'\s*"(.*)\\0"$', l)
        if m:
            out.append((off, m.group(1))); off += len(m.group(1)) + 1
    return out

def emit_storage(strings):
    body, off, pos = ['  "\\0"'], 1, {'': 0}
    for s in strings:
        if s in pos: continue
        pos[s] = off; off += len(s) + 1
        body.append(f'  "{s}\\0"')
    return body, pos

# --- name table: blank the names of other targets
a, b = section('GET_INTRINSIC_NAME_TABLE')
sa = next(i for i in range(a, b) if 'IntrinsicNameTableStorage[] =' in L[i])
se = next(i for i in range(sa, b) if L[i].strip() == ';')
ta = next(i for i in range(se, b) if 'IntrinsicNameOffsetTable[] = {' in L[i])
te = next(i for i in range(ta, b) if L[i].startswith('};'))
names = {off: s for off, s in storage(L[sa + 1:se])}
ids = []
for l in L[ta + 1:te]:
    m = re.match(r'\s*(\d+), // (.*)$', l)
    if m: ids.append(names[int(m.group(1))])
new_names = [n if kept_id(i) else '' for i, n in enumerate(ids)]
body, pos = emit_storage([n for n in new_names if n])
offs = [f'  {pos[n]}, // {n or "-"}' for n in new_names]
L[ta + 1:te] = offs
L[sa + 1:se] = body

# --- clang builtin -> intrinsic: keep the generic table and the kept targets
a, b = section('GET_LLVM_INTRINSIC_FOR_CLANG_BUILTIN')
sa = next(i for i in range(a, b) if 'BuiltinNamesStorage[] =' in L[i])
se = next(i for i in range(sa, b) if L[i].strip() == ';')
bnames = {off: s for off, s in storage(L[sa + 1:se])}
out, i, used = [], sa + 1, []
arrays = {}
j = se
res = L[:sa + 1]
res_tail = []
k = se
body_lines = L[se:b]
new_body, skip = [], False
for l in body_lines:
    m = re.match(r'\s*static constexpr BuiltinEntry (\w+)\[\] = \{', l)
    if m:
        tgt = '' if m.group(1) == 'Names' else m.group(1)[:-5]
        skip = tgt not in keep
    if skip:
        if re.match(r'\s*\}; // \w+', l): skip = False
        continue
    m = re.match(r'(\s*)\{(\w+), (\d+)\}, // (.*)$', l)
    if m:
        s = bnames[int(m.group(3))]; used.append(s)
        new_body.append((m, s)); continue
    m2 = re.match(r'\s*\{"(\w+)", (\w+)Names, "(\w*)"\},$', l)
    if m2 and m2.group(1) not in keep:
        continue
    new_body.append(l)
sbody, spos = emit_storage(used)
final = []
for x in new_body:
    if isinstance(x, tuple):
        m, s = x
        final.append(f'{m.group(1)}{{{m.group(2)}, {spos[s]}}}, // {m.group(4)}')
    else:
        final.append(x)
# an empty TargetTable is not valid C++: leave a dummy entry that never matches a builtin
for i, l in enumerate(final):
    if l.strip() == 'static constexpr TargetEntry TargetTable[] = {' and final[i + 1].strip() == '};':
        final.insert(i + 1, '    {"", {}, ""},')
        break
L = L[:sa + 1] + sbody + final + L[b:]
open(dst, 'w').write('\n'.join(L))
print(f'names kept: {sum(1 for n in new_names if n)}/{len(ids)}; builtin names kept: {len(used)}')
