#!/usr/bin/env python3
"""Build the trimmed objects that link.py puts in place of their archive members.

  * Intrinsics.cpp.obj: compiled against a copy of the tablegen'd
    IntrinsicImpl.inc with only the wasm intrinsic names (trim_intrinsics.py);
  * TargetLibraryInfo.cpp.obj: compiled with -DSC_NO_VECLIBS (see
    llvm/no-veclibs.patch), without the vector library tables.

Each uses the compile command of the wasm build, from ninja.
"""
import os, re, subprocess, sys

R = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
B = os.environ.get('SC_BUILD', f'{R}/build')
WB, T = f'{B}/wasm', f'{B}/trim'
os.makedirs(f'{T}/llvm/IR', exist_ok=True)
env = dict(os.environ, PATH=f'{B}/tools:' + os.environ['PATH'])
subprocess.check_call([sys.executable, f'{R}/scripts/trim_intrinsics.py',
                       f'{WB}/include/llvm/IR/IntrinsicImpl.inc', f'{T}/llvm/IR/IntrinsicImpl.inc'])


def compile(obj, edit):
    cmd = subprocess.check_output(['ninja', '-C', WB, '-t', 'commands', obj], text=True, env=env).splitlines()[-1]
    cmd = re.sub(r' -MD -MT \S+ -MF \S+', '', cmd)
    cmd = cmd.replace(f' -o {obj} ', f' -o {T}/{os.path.basename(obj)} ')
    subprocess.check_call(edit(cmd), shell=True, cwd=WB)


compile('lib/IR/CMakeFiles/LLVMCore.dir/Intrinsics.cpp.obj',
        lambda c: c.replace(f' -I{WB}/include ', f' -I{T} -I{WB}/include ', 1))
compile('lib/Analysis/CMakeFiles/LLVMAnalysis.dir/TargetLibraryInfo.cpp.obj',
        lambda c: c.replace(' -o ', ' -DSC_NO_VECLIBS -o ', 1))
