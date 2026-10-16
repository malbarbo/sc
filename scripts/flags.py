#!/usr/bin/env python3
"""The engine's flags: the -cc1 flags clang's driver would set for
wasm32-wasip1 (C gnu23, C++ gnu++26, with exceptions and setjmp/longjmp) and
the libraries a shared object is linked with. The paths are the engine's
view: the sysroot at /sysroot, clang's resource directory at /res and libc++'s
std module at /m/std.pcm; real() puts the build's in their place.

The level of --safety (PLANO-VERIFICACOES.md §3, §4) adds to these flags the
warnings, and the warnings that are errors, of what the student sees.

usage: flags.py [c++|gnu17] [pic] [pcm|mod] [off|on|strict] [g]
                                    the -cc1 flags (make checkinv)
       flags.py --session c|c++ <build> [off|on|strict] [file]
                                    a REPL session's arguments, one per line
                                    (build/repl, sc.h); file: of `sc FILE`
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


# PLANO-VERIFICACOES.md §4.1, E: undefined behaviour for certain, or an error
# with no use, which no correct program of the measures set off; an error
# from the level on. -Werror=G is also the error of G's subgroups: the
# second list keeps those that are a warning
E = ['uninitialized', 'return-type', 'return-stack-address', 'dangling', 'array-bounds', 'division-by-zero',
     'shift-count-overflow', 'shift-count-negative', 'integer-overflow', 'fortify-source', 'format',
     'string-compare', 'unsequenced', 'mismatched-new-delete', 'delete-abstract-non-virtual-dtor',
     'free-nonheap-object']
E_WARN = ['sometimes-uninitialized', 'uninitialized-const-pointer', 'return-type-c-linkage', 'format-security',
          'format-extra-args', 'format-zero-length', 'format-truncation']
# §4.2, R: in -Wall or -Wextra, but noise in a beginner's correct code; strict only
R = ['unused-parameter', 'missing-field-initializers', 'sign-compare']
# §4.3 and §4.4, A: off in clang, a student's common error, no false positive
# measured. -Wshadow only in C and only in a whole program (§4.3)
A = ['float-overflow-conversion', 'float-zero-conversion', 'string-conversion', 'shift-bool',
     'tautological-unsigned-zero-compare', 'tautological-unsigned-enum-zero-compare',
     'tautological-value-range-compare', 'unreachable-code', 'implicit-fallthrough', 'comma', 'main',
     'main-return-type', 'array-bounds-pointer-arithmetic']
A_C = ['shift-sign-overflow', 'default-const-init-var', 'default-const-init-field']
A_CXX = ['shadow-field-in-constructor-modified', 'non-virtual-dtor', 'lifetime-safety-permissive']
# §4, S: style, conversions, portability; strict only. P: the GNU and clang
# extensions, with -Wpedantic (decision 6, §11)
S = ['alloca', 'anon-enum-enum-conversion', 'assign-enum', 'bad-function-cast', 'c++-compat', 'c++-hidden-decl',
     'c++-keyword', 'c++-unterminated-string-initialization', 'cast-align', 'cast-function-type-strict', 'cast-qual',
     'class-varargs', 'conditional-uninitialized', 'conversion', 'ctad-maybe-unsupported',
     'deprecated-copy-with-dtor', 'deprecated-copy-with-user-provided-dtor', 'double-promotion', 'dtor-name',
     'duplicate-decl-specifier', 'duplicate-enum', 'enum-compare-conditional', 'enum-enum-conversion',
     'enum-float-conversion', 'expansion-to-defined',
     'extra-semi', 'extra-semi-stmt', 'float-conversion', 'float-equal', 'format-nonliteral', 'format-signedness',
     'format-type-confusion', 'four-char-constants', 'header-hygiene', 'ignored-base-class-qualifiers',
     'implicit-float-conversion', 'implicit-int-conversion', 'implicit-int-conversion-on-negation',
     'implicit-int-enum-cast', 'implicit-int-float-conversion', 'implicit-void-ptr-cast',
     'incompatible-function-pointer-types-strict', 'inconsistent-missing-destructor-override', 'jump-misses-init',
     'keyword-macro', 'lifetime-safety-invalidation', 'old-style-cast',
     'range-loop-bind-reference', 'reserved-attribute-identifier', 'reserved-identifier',
     'reserved-macro-identifier', 'shadow-field', 'shadow-uncaptured-local', 'shorten-64-to-32', 'sign-conversion',
     'static-in-inline', 'suggest-destructor-override', 'suggest-override', 'switch-default', 'switch-enum',
     'tautological-type-limit-compare', 'tautological-unsigned-char-zero-compare', 'tentative-definition-compat',
     'tentative-definition-incomplete-type', 'undef', 'undefined-internal-type', 'undefined-reinterpret-cast',
     'unneeded-member-function', 'unreachable-code-break', 'unreachable-code-return', 'unused-exception-parameter',
     'unused-macros', 'unused-member-function', 'unused-template', 'vla', 'zero-as-null-pointer-constant']
LEVELS = ['off', 'on', 'strict']
# §5.1, U: the checks of -fsanitize=undefined on wasm32, as the driver lists
# them; a report stops the program (no -fsanitize-recover). The handlers are
# libsc_rt's (rt/)
U = ['alignment', 'array-bounds', 'bool', 'builtin', 'enum', 'float-cast-overflow', 'integer-divide-by-zero',
     'nonnull-attribute', 'null', 'pointer-overflow', 'return', 'returns-nonnull-attribute', 'shift-base',
     'shift-exponent', 'signed-integer-overflow', 'unreachable', 'vla-bound']
# §5.3: libc++'s hardening; off: none (the sysroot is built with fast)
HARDENING = {'off': 'NONE', 'on': 'EXTENSIVE', 'strict': 'EXTENSIVE'}
# §5.2: ASan's shadow memory is at SHADOW, after the stack: 512 MiB, the
# shadow of the 4 GiB, and the program after it. Not a power of 2, which
# the pass would OR in (libsc_rt and the loader know it too)
SHADOW = 0x110000
# what libsc_rt puts in place of libc's and libc++abi's functions (rt/sc_asan.c),
# with the names of wasm-ld's --wrap: the allocator, the functions of libc
# that write the program's memory, and the throws of libc++
WRAP = ['malloc', 'free', 'calloc', 'realloc', 'reallocarray', 'aligned_alloc', 'posix_memalign',
        'malloc_usable_size', 'getdelim', 'getline', 'strcpy', 'strncpy', 'strcat', 'strncat', 'sprintf',
        'snprintf', 'fgets', 'fread', 'read', '__cxa_throw', '__cxa_rethrow']
# use-after-return would take a fake stack; the stack safety analysis takes
# ScalarEvolution, which the engine leaves out
ASAN = ['-fsanitize-address-use-after-scope', '-fsanitize-address-use-after-return=never',
        '-fsanitize-address-globals-dead-stripping', '-mllvm', '-asan-use-stack-safety=0',
        '-mllvm', f'-asan-mapping-offset={SHADOW:#x}']


def checks(fl, level, cxx):
    """fl with the checks at run time of a level of --safety (§5): ASan,
    UBSan, libc++'s hardening and the locals' pattern; off has none"""
    fl = fl + ['-D', f'_LIBCPP_HARDENING_MODE=_LIBCPP_HARDENING_MODE_{HARDENING[level]}'] * cxx
    if level == 'off':
        return fl
    u = ','.join(U)
    return fl + [f'-fsanitize=address,{u}', f'-fsanitize-merge={u}', '-fno-sanitize-memory-param-retval',
                 '-fno-sanitize-address-use-odr-indicator', '-ftrivial-auto-var-init=pattern'] + ASAN


def safety(fl, level, cxx, file=False, errors=True):
    """fl at a level of --safety: off has clang's warnings, on and strict
    those of their sets, and the checks at run time; file: in a whole
    program, not in the REPL. Without errors, E stays warnings: for the
    tests that have undefined behaviour on purpose, which would stop in the
    frontend"""
    fl = checks([x for x in fl if x not in ('-Wall', '-Wextra')], level, cxx)
    if level == 'off':
        return fl
    w = ['all', 'extra'] + A + (A_CXX if cxx else A_C + ['shadow'] * file)
    w += [f'no-{x}' for x in R] if level == 'on' else S + ['pedantic']
    w += [f'error={x}' for x in E] + [f'no-error={x}' for x in E_WARN] if errors else []
    return fl + [f'-W{x}' for x in w]


def lines(fl):
    """-gline-tables-only, as the driver sets it: where a trap is (§5.6)"""
    return fl + ['-debug-info-kind=line-tables-only', '-dwarf-version=4', '-debugger-tuning=gdb']


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


def solibs(cxx, level=None):
    """what a shared object is linked with, as the REPL links each input; at
    a level of --safety with checks, libsc_rt, their handlers"""
    return ['-L/sysroot/lib/wasm32-wasip1'] + ['-lsc_rt'] * (level in ('on', 'strict')) + ['-lc++', '-lc++abi'] * cxx + [
        '-lsetjmp', '-lc', '/res/lib/wasm32-unknown-wasip1/libclang_rt.builtins.a']


def rt_static(level):
    """what a static program is linked with at a level of --safety with
    checks (the corpus's): libsc_rt with its wraps, and the memory of the
    REPL's loader, the stack of 1 MiB first and the data after ASan's shadow"""
    if level not in ('on', 'strict'):
        return []
    return ['-lsc_rt', '--stack-first', '-z', 'stack-size=1048576', f'--global-base={SHADOW + (512 << 20)}'] + [
        f'--wrap={x}' for x in WRAP]


def res(build):
    """clang's resource directory in the build"""
    return f'{build}/wasi-sdk-29.0-x86_64-linux/lib/clang/21'


def real(fl, build, pcm=None):
    """fl with the build's paths in place of the engine's view"""
    fl = [x.replace('/sysroot', f'{build}/sysroot').replace('/res/', res(build) + '/') for x in fl]
    return [x.replace('/m/std.pcm', pcm) for x in fl] if pcm else fl


def session(cxx, build, c17=False, mod=False, pcm=None, level=None, file=False):
    """a REPL session's arguments (sc_session_new): the -cc1 flags, then after
    -- the link's, with the build's paths; at a level of --safety, or with
    the engine's flags. With the lines of the code, for where a trap is"""
    fl = pic(CXXFLAGS if cxx else gnu17(FLAGS) if c17 else FLAGS)[1:] + MODFILE * mod
    fl = lines(safety(fl, level, cxx, file) if level else fl)
    return real(fl + ['-x', 'c++' if cxx else 'c', '--'] + solibs(cxx, level), build, pcm)


if __name__ == '__main__':
    if sys.argv[1:2] == ['--session']:
        a = sys.argv[4:]
        level = next((x for x in a if x in LEVELS), None)
        sys.exit(print('\n'.join(session(sys.argv[2] == 'c++', sys.argv[3], level=level, file='file' in a))))
    a = sys.argv[1:]
    fl = CXXFLAGS if 'c++' in a else gnu17(FLAGS) if 'gnu17' in a else FLAGS
    fl = pic(fl) if 'pic' in a else fl
    fl = iface(fl) if 'pcm' in a else fl + MODFILE if 'mod' in a else fl
    level = next((x for x in a if x in LEVELS), None)
    fl = safety(fl, level, 'c++' in a) if level else fl
    fl = lines(fl) if 'g' in a else fl
    print(' '.join(fl[1:]))
