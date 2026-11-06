#!/usr/bin/env python3
"""The pack sc.wasm embeds (engine/sc_pack.cpp, docs/plan-repl.md §5.5): what
build.rs puts in the native sc's (§5.4), the same files at the same paths,
without the .cwasm, in a squashfs without compression. The page's server
compresses the module, and the whole of it compresses better than blocks
of 128 KiB (NOTES.md §59). mksquashfs makes it from a copy of the tree
in build/pack/tree.

usage: pack.py <out.sqfs>
"""
import os
import shutil
import subprocess
import sys

B = 'build'
RES = f'{B}/wasi-sdk-29.0-x86_64-linux/lib/clang/21'
LIB = 'lib/wasm32-wasip1'
BUILTINS = 'lib/wasm32-unknown-wasip1/libclang_rt.builtins.a'
# build.rs, LIBS
LIBS = ['libc.so', 'libc++.so', 'libc++abi.so', 'libsc_rt.so', 'libsetjmp.so']
TREES = [(f'{B}/sysroot/include', 'sysroot/include'), (f'{RES}/include', 'res/include'), (f'{B}/pack/repl', 'repl')]


def leb(b, i):
    n = shift = 0
    while True:
        x = b[i]
        i += 1
        n |= (x & 0x7f) << shift
        shift += 7
        if x < 0x80:
            return n, i


def strip_debug(wasm):
    """the module without its custom sections .debug_* (build.rs)"""
    out, i = bytearray(wasm[:8]), 8
    while i < len(wasm):
        start, sid = i, wasm[i]
        size, i = leb(wasm, i + 1)
        end = i + size
        if not (sid == 0 and (lambda n, j: wasm[j:j + n].startswith(b'.debug'))(*leb(wasm, i))):
            out += wasm[start:end]
        i = end
    return bytes(out)


def files():
    """the path in the pack of each file, and the file (build.rs, pack)"""
    fs = {}
    for d, at in TREES:
        for top, _, names in os.walk(d):
            for n in names:
                fs[f'{at}/{os.path.relpath(os.path.join(top, n), d)}'] = os.path.join(top, n)
    # out go the intrinsics of the other targets
    fs = {k: v for k, v in fs.items() if not (k.startswith('res/include/') and '/' not in k[12:]
                                              and ('intrin' in k[12:] or k[12:].startswith('arm_')))}
    fs.update({f'sysroot/{LIB}/{l}': f'{B}/sysroot/{LIB}/{l}' for l in LIBS})
    fs['sysroot/share/sc/ignorelist.txt'] = f'{B}/sysroot/share/sc/ignorelist.txt'
    fs[f'res/{BUILTINS}'] = f'{RES}/{BUILTINS}'
    return fs


def main():
    out = sys.argv[1]
    tree = f'{os.path.dirname(out)}/tree'
    shutil.rmtree(tree, ignore_errors=True)
    for to, frm in sorted(files().items()):
        os.makedirs(os.path.dirname(f'{tree}/{to}'), exist_ok=True)
        with open(frm, 'rb') as f:
            b = f.read()
        with open(f'{tree}/{to}', 'wb') as f:
            f.write(strip_debug(b) if to.endswith('/libc.so') else b)
    if os.path.exists(out):
        os.remove(out)
    subprocess.run(['mksquashfs', tree, out, '-noI', '-noId', '-noD', '-noF', '-noX', '-no-xattrs', '-all-root',
                    '-mkfs-time', '0', '-all-time', '0', '-nopad', '-quiet', '-no-progress'], check=True,
                   stdout=subprocess.DEVNULL)
    shutil.rmtree(tree)


main()
