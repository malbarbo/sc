#!/usr/bin/env python3
"""The engine's flags: the -cc1 flags clang's driver would set for
wasm32-wasip1 (C gnu23, C++ gnu++26, with exceptions and setjmp/longjmp) and
the libraries a shared object is linked with. The paths are the engine's
view: the sysroot at /sysroot, clang's resource directory at /res and libc++'s
std module at /m/std.pcm; real() puts the build's in their place.

usage: flags.py [c++|gnu17] [pic] [pcm|mod]   the -cc1 flags (make checkinv)
       flags.py --session c|c++ <build>        a REPL session's arguments, one
                                               per line (build/repl, sc.h)
"""
import sys

FLAGS = ['-cc1', '-triple', 'wasm32-unknown-wasip1', '-emit-obj', '-Wall', '-Wextra',
         '-disable-free', '-clear-ast-before-backend', '-disable-llvm-verifier', '-discard-value-names',
         '-mrelocation-model', 'static', '-mframe-pointer=none', '-ffp-contract=on', '-fno-rounding-math',
         '-target-cpu', 'generic', '-fvisibility=hidden', '-ferror-limit', '19', '-fgnuc-version=4.2.1',
         '-isysroot', '/sysroot', '-internal-isystem', '/res/include',
         '-internal-externc-isystem', '/sysroot/include/wasm32-wasip1', '-internal-externc-isystem', '/sysroot/include',
         # -fwasm-exceptions with -mllvm -wasm-enable-sjlj, in the standard form (exnref)
         '-target-feature', '+exception-handling', '-target-feature', '+multivalue', '-target-feature', '+reference-types',
         '-exception-model=wasm', '-mllvm', '-wasm-enable-sjlj', '-mllvm', '-wasm-use-legacy-eh=false', '-std=gnu23']
# C++ as the driver sets it for wasm32-wasip1, with libc++ first in the search path
CXXFLAGS = FLAGS[:FLAGS.index('/res/include') - 1] + [
    '-internal-isystem', '/sysroot/include/wasm32-wasip1/c++/v1', '-internal-isystem', '/sysroot/include/c++/v1'] + \
    FLAGS[FLAGS.index('/res/include') - 1:-1] + ['-std=gnu++26', '-mconstructor-aliases', '-fdeprecated-macro', '-fno-implicit-modules',
    '-fskip-odr-check-in-gmf', '-fcxx-exceptions', '-fexceptions', '-mllvm', '-wasm-enable-eh']
# the use of libc++'s std module
MODFILE = ['-fmodule-file=std=/m/std.pcm']


def iface(fl):
    """the std module's interface, built as the driver does for --precompile"""
    return [{'-emit-obj': '-emit-module-interface'}.get(x, x) for x in fl] + [
        '-fmodules-reduced-bmi', '-I', '/sysroot/share/libc++/v1', '-Wno-reserved-module-identifier']


def gnu17(fl):
    """the C of c-testsuite and the gcc torture tests: K&R definitions and implicit
    int, which C23 rejects; without them these tests would stop in the frontend"""
    return [{'-std=gnu23': '-std=gnu17'}.get(x, x) for x in fl]


def pic(fl):
    """the flags for a shared object, as the driver sets them for -fPIC -fvisibility=default"""
    fl = [{'static': 'pic', '-fvisibility=hidden': '-fvisibility=default'}.get(x, x) for x in fl]
    return fl + ['-pic-level', '2', '-target-feature', '+mutable-globals']


def solibs(cxx):
    """what a shared object is linked with, as the REPL links each input"""
    return ['-L/sysroot/lib/wasm32-wasip1'] + ['-lc++', '-lc++abi'] * cxx + [
        '-lsetjmp', '-lc', '/res/lib/wasm32-unknown-wasip1/libclang_rt.builtins.a']


def res(build):
    """clang's resource directory in the build"""
    return f'{build}/wasi-sdk-29.0-x86_64-linux/lib/clang/21'


def real(fl, build, pcm=None):
    """fl with the build's paths in place of the engine's view"""
    fl = [x.replace('/sysroot', f'{build}/sysroot').replace('/res/', res(build) + '/') for x in fl]
    return [x.replace('/m/std.pcm', pcm) for x in fl] if pcm else fl


def session(cxx, build, c17=False, mod=False, pcm=None):
    """a REPL session's arguments (sc_session_new): the -cc1 flags, then after
    -- the link's, with the build's paths"""
    fl = pic(CXXFLAGS if cxx else gnu17(FLAGS) if c17 else FLAGS)[1:] + MODFILE * mod
    return real(fl + ['-x', 'c++' if cxx else 'c', '--'] + solibs(cxx), build, pcm)


if __name__ == '__main__':
    if sys.argv[1:2] == ['--session']:
        sys.exit(print('\n'.join(session(sys.argv[2] == 'c++', sys.argv[3]))))
    a = sys.argv[1:]
    fl = CXXFLAGS if 'c++' in a else gnu17(FLAGS) if 'gnu17' in a else FLAGS
    fl = pic(fl) if 'pic' in a else fl
    fl = iface(fl) if 'pcm' in a else fl + MODFILE if 'mod' in a else fl
    print(' '.join(fl[1:]))
