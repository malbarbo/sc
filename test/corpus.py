#!/usr/bin/env python3
"""Run the C corpus through an sc engine: sc.wasm under wasmtime, or the
native engine (build/sc) under bwrap, with the same directories mapped.

usage: corpus.py <engine> <outdir> [<base summary.json>]
Programs (test/samples, c-testsuite, gcc torture execute/) are compiled with
-cc1, linked with the engine's own ld and run. The rest (gcc torture compile/,
clang's C tests) are only compiled: diagnostics and the object's md5.
Writes <outdir>/<name>.txt and <outdir>/summary.json {name: normalized md5}.
With a base summary, lists the files whose result differs and fails if any.
CORPUS=small runs only the programs, ONLY=<file> the results it names;
JOBS sets the parallelism (48);
WFLAGS adds warning flags (e.g. -Weverything).
C (gnu23; gnu17 for c-testsuite and the gcc torture tests) and C++ (gnu++26)
have exceptions and setjmp/longjmp. C++ goes the same way: the programs in test/cxx,
linked with libc++, and clang's C++ tests. The engine first builds libc++'s
std module (std-pcm), which the programs in test/mod import.
The programs are also compiled with -fPIC, linked as shared objects and run
by the REPL's loader, build/loader/release/scload (pic-), and test/repl has
sequences of REPL inputs, each loaded on top of the ones before (repl-).
"""
from corpuslib import norm_hash, diff
import hashlib, re, json, os, subprocess, sys, glob
from concurrent.futures import ThreadPoolExecutor
R = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
B = os.environ.get('SC_BUILD', f'{R}/build')
W = f'{B}/wasi-sdk-29.0-x86_64-linux'; RES = f'{W}/lib/clang/21'
SR = f'{B}/sysroot'  # wasi-sdk's libc with libc++, libc++abi and libunwind built from the pin with exceptions
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
CXXEXT = ('.cpp', '.cc', '.cxx')
SCLOAD = f'{B}/loader/release/scload'  # the REPL's loader (loader/)
LIB = f'{SR}/lib/wasm32-wasip1'  # libc.so, libc++.so...
W_EXTRA = os.environ.get('WFLAGS', '').split()  # e.g. WFLAGS=-Weverything


# libc++'s std module, built as the driver does for --precompile, and its use
STD_CPPM = f'{SR}/share/libc++/v1/std.cppm'
MODFILE = ['-fmodule-file=std=/m/std.pcm']


def iface(fl):
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


if sys.argv[1] == '--flags':  # the cc1 flags, for test/checkinv: --flags [c++] [pic] [pcm|mod]
    fl = CXXFLAGS if 'c++' in sys.argv else gnu17(FLAGS) if 'gnu17' in sys.argv else FLAGS
    fl = pic(fl) if 'pic' in sys.argv else fl
    fl = iface(fl) if 'pcm' in sys.argv else fl + MODFILE if 'mod' in sys.argv else fl
    sys.exit(print(' '.join(fl[1:])))
eng, out = os.path.realpath(sys.argv[1]), sys.argv[2]
# objects and programs go to a directory of their own: it stays small, so
# looking for the temporary a failed write leaves is cheap
tmp = f'{out}/o'
OUT = os.path.realpath(out) + '/'
os.makedirs(tmp, exist_ok=True)
M = f'{out}/m'  # the std module, built by the engine under test
os.makedirs(M, exist_ok=True)
NATIVE = not eng.endswith('.wasm')
if not NATIVE:
    cw = f'{out}/engine.cwasm'
    subprocess.check_call(['wasmtime', 'compile', eng, '-o', cw])


def engine(dirs, args):
    """the command that runs the engine with dirs {host: guest} mapped"""
    if NATIVE:
        return ['bwrap', '--ro-bind', '/usr', '/usr', '--symlink', 'usr/lib', '/lib', '--symlink', 'usr/lib64', '/lib64',
                '--dev', '/dev', '--ro-bind', eng, '/sc'] + [x for h, g in dirs.items() for x in ('--bind', h, g)] + ['/sc'] + args
    return ['wasmtime', 'run', '--allow-precompiled'] + [x for h, g in dirs.items() for x in ('--dir', f'{h}::{g}')] + [cw] + args

# (path, kind) — True: link and execute; False: compile only (diagnostics + object
# hash); 'pic': compile with -fPIC and link a shared object, as the REPL does
# for each input (diagnostics + the object's md5)
progs = [(p, True) for p in sorted(glob.glob(f'{R}/test/samples/*.c')) + sorted(glob.glob(f'{B}/c-testsuite/tests/single-exec/*.c'))]
if os.environ.get('CORPUS', 'full') == 'full':
    T = f'{B}/src/clang/test'
    progs += [(p, True) for p in sorted(glob.glob(f'{B}/gcc-ts/gcc/testsuite/gcc.c-torture/execute/*.c'))]
    progs += [(p, False) for p in sorted(glob.glob(f'{B}/gcc-ts/gcc/testsuite/gcc.c-torture/compile/*.c'))]
    for sub in ['Sema', 'Parser', 'CodeGen', 'Lexer', 'Preprocessor', 'Misc', 'Frontend', 'AST', 'C/*', 'C/*/*']:
        progs += [(p, False) for p in sorted(glob.glob(f'{T}/{sub}/*.c'))]
progs += [(p, True) for p in sorted(glob.glob(f'{R}/test/cxx/*.cpp')) + sorted(glob.glob(f'{R}/test/mod/*.cpp'))]
if os.environ.get('CORPUS', 'full') == 'full':
    for sub in ['SemaCXX', 'SemaTemplate', 'CodeGenCXX', 'CXX/**', 'Parser', 'Lexer', 'AST', 'Misc']:
        progs += [(p, False) for p in sorted(glob.glob(f'{T}/{sub}/*.cpp', recursive=True))]
progs += [(p, 'pic') for p, k in list(progs) if k is True]

def name(pr):
    """the name of a result: <outdir>/<name>.txt"""
    p, kind = pr
    d, f = os.path.dirname(p), os.path.basename(p)
    rel = os.path.relpath(p, B)
    stem, rel = os.path.splitext(f)[0], os.path.splitext(rel)[0]
    n = stem if d == f'{R}/test/samples' else 'cxx-' + stem if d == f'{R}/test/cxx' else 'mod-' + stem if d == f'{R}/test/mod' else (
        'ct-' + stem if 'c-testsuite' in d else rel.replace('/', '_'))
    if len(n) > 200: n = n[-200:]
    return 'pic-' + n if kind == 'pic' else n


def one(pr):
    p, kind = pr
    d, f = os.path.dirname(p), os.path.basename(p)
    n = name(pr)
    try:
        return n, one1(p, d, f, n, kind)
    except subprocess.TimeoutExpired:
        open(f'{out}/{n}.txt', 'w').write('TIMEOUT\n'); return n, 'TIMEOUT'

def cc1(d, f, n, fl, mod):
    """compiles d/f to tmp/n.o: the result text, the object and whether it succeeded"""
    dirs = {d: '/c', tmp: '/o', SR: '/sysroot', RES: '/res'} | ({M: '/m'} if mod else {})
    r = subprocess.run(engine(dirs, fl + MODFILE * mod + W_EXTRA + ['-o', f'/o/{n}.o', f'/c/{f}']),
                       capture_output=True, timeout=300)
    obj = f'{tmp}/{n}.o'
    o = open(obj, 'rb').read() if os.path.exists(obj) else b''
    err = r.stderr.decode(errors='replace')
    txt = err.split('wasm trap')[0] + ('<TRAP>\n' + err.split('wasm trap', 1)[1] if 'wasm trap' in err else '') + f'cc1 exit={r.returncode}\n'
    return txt, o, r.returncode == 0

def ld_so(n, cxx):
    """links tmp/n.o as a shared object, as the REPL does with each input"""
    l = subprocess.run(engine({tmp: '/o', SR: '/sysroot', RES: '/res'}, [
                        'ld', '-shared', '--experimental-pic', '--unresolved-symbols=import-dynamic', f'/o/{n}.o',
                        '-L/sysroot/lib/wasm32-wasip1'] + (['-lc++', '-lc++abi'] if cxx else []) +
                       ['-lsetjmp', '-lc', '/res/lib/wasm32-unknown-wasip1/libclang_rt.builtins.a', '-o', f'/o/{n}.so']),
                       capture_output=True, text=True, timeout=300)
    so = f'{tmp}/{n}.so'
    txt = l.stderr + f'ld exit={l.returncode}\n'
    if l.returncode == 0:
        txt += 'so ' + hashlib.md5(open(so, 'rb').read()).hexdigest() + '\n'
    return txt, l.returncode == 0

def run(cmd):
    try:
        x = subprocess.run(cmd, input=b'linha1\nlinha2\n', capture_output=True, timeout=20)
    except subprocess.TimeoutExpired as e:
        x = subprocess.CompletedProcess(e.cmd, 'timeout', e.stdout or b'', e.stderr or b'')
    return x.stdout.decode(errors='replace') + x.stderr.decode(errors='replace') + f'run exit={x.returncode}\n'

def one1(p, d, f, n, kind):
    cxx = f.endswith(CXXEXT)
    obj = f'{tmp}/{n}.o'
    fl = CXXFLAGS if cxx else gnu17(FLAGS) if p.startswith((f'{B}/c-testsuite/', f'{B}/gcc-ts/')) else FLAGS
    fl = pic(fl) if kind == 'pic' else fl
    txt, o, ok = cc1(d, f, n, fl, d == f'{R}/test/mod')
    if o and kind is not True:
        txt += 'obj ' + hashlib.md5(o).hexdigest() + '\n'
    if ok and o and kind == 'pic':  # the shared object, run by the REPL's loader
        t, ok = ld_so(n, cxx)
        txt += t + run([SCLOAD, '-L', LIB, f'{tmp}/{n}.so']) * ok
        if ok: os.remove(f'{tmp}/{n}.so')
    if ok and o and kind is True:
        l = subprocess.run(engine({tmp: '/o', SR: '/sysroot', RES: '/res'}, ['ld', f'/o/{n}.o', '/sysroot/lib/wasm32-wasip1/crt1-command.o',
                            '-L/sysroot/lib/wasm32-wasip1', '-lc-printscan-long-double'] + (['-lc++', '-lc++abi'] if cxx else []) +
                           ['-lsetjmp', '-lc', '-lm', '/res/lib/wasm32-unknown-wasip1/libclang_rt.builtins.a', '-o', f'/o/{n}.wasm']),
                           capture_output=True, text=True, timeout=300)
        txt += l.stderr
        if l.returncode == 0:
            txt += run(['wasmtime', f'{tmp}/{n}.wasm'])
            os.remove(f'{tmp}/{n}.wasm')
    for t in [obj] * os.path.exists(obj) + glob.glob(f'{tmp}/{glob.escape(n)}-{"[0-9a-f]" * 8}.o.tmp'):
        os.remove(t)  # the object and the temporary a failed write leaves
    txt = re.sub(r'main module `[^`]*/', 'main module `', txt)
    txt = txt.replace(OUT, 'OUT/').replace(out + '/', 'OUT/')
    open(f'{out}/{n}.txt', 'w').write(txt)
    return norm_hash(txt)

def repl(seq):
    """a sequence of REPL inputs in test/repl (c1.c, c2.c...), each a shared
    object loaded on top of the ones before"""
    d = f'{R}/test/repl'
    files = sorted((f for f in os.listdir(d) if re.fullmatch(seq + r'\d+\.\w+', f)), key=lambda f: int(re.sub(r'\D', '', f)))
    txt, sos = '', []
    for f in files:
        n = f'repl-{os.path.splitext(f)[0]}'
        cxx = f.endswith(CXXEXT)
        t, o, ok = cc1(d, f, n, pic(CXXFLAGS if cxx else FLAGS), 'import std;' in open(f'{d}/{f}').read())
        txt += f'== {f}\n' + t
        if os.path.exists(f'{tmp}/{n}.o'):
            if ok and o:
                t, ok = ld_so(n, cxx)
                txt += t
                sos.append(f'{tmp}/{n}.so')
            os.remove(f'{tmp}/{n}.o')
        if not (ok and o):
            break
    else:
        txt += run([SCLOAD, '-L', LIB] + sos)
    for so in sos:
        if os.path.exists(so): os.remove(so)
    txt = txt.replace(OUT, 'OUT/').replace(out + '/', 'OUT/')
    open(f'{out}/repl-{seq}.txt', 'w').write(txt)
    return f'repl-{seq}', norm_hash(txt)

def std_pcm():
    """libc++'s std module, without <csignal>, which WASI's libc rejects"""
    src = [l for l in open(STD_CPPM) if 'csignal' not in l]
    open(f'{M}/std.cppm', 'w').writelines(src)
    os.utime(f'{M}/std.cppm', (0, 0))  # the .pcm records it
    if os.path.exists(f'{M}/std.pcm'):
        os.remove(f'{M}/std.pcm')
    r = subprocess.run(engine({M: '/m', SR: '/sysroot', RES: '/res'}, iface(CXXFLAGS) + W_EXTRA + ['-o', '/m/std.pcm', '/m/std.cppm']),
                       capture_output=True, timeout=600)
    txt = r.stderr.decode(errors='replace') + f'cc1 exit={r.returncode}\n'
    if os.path.exists(f'{M}/std.pcm'):
        txt += 'pcm ' + hashlib.md5(open(f'{M}/std.pcm', 'rb').read()).hexdigest() + '\n'
    open(f'{out}/std-pcm.txt', 'w').write(txt)
    return norm_hash(txt)

SKIP = ('warn-date-time', 'SOURCE_DATE_EPOCH', 'pr133574', 'builtin_redef')  # depend on the clock
progs = [p for p in progs if not p[0].endswith(tuple(x + '.c' for x in SKIP))]
seqs = sorted({re.match(r'[a-z]+', f)[0] for f in os.listdir(f'{R}/test/repl')})
if os.environ.get('ONLY'):  # ONLY=<file>: only the results named there, one per line
    only = set(open(os.environ['ONLY']).read().split())
    progs = [pr for pr in progs if name(pr) in only]
    seqs = [s for s in seqs if f'repl-{s}' in only]
res = {'std-pcm': std_pcm()}
with ThreadPoolExecutor(int(os.environ.get('JOBS', '48'))) as ex:
    res.update(ex.map(one, progs))
    res.update(ex.map(repl, seqs))
if not NATIVE:
    os.remove(cw)
json.dump(res, open(f'{out}/summary.json', 'w'), indent=0, sort_keys=True)
print(hashlib.md5(json.dumps(res, sort_keys=True).encode()).hexdigest())
if len(sys.argv) > 3:
    base = json.load(open(sys.argv[3]))
    if os.environ.get('CORPUS', 'full') != 'full' or os.environ.get('ONLY'):  # only the files that ran
        base = {k: v for k, v in base.items() if k in res}
    bad = diff(base, res)
    for k in bad:
        print('DIFF', k)
    print(f'{len(res)} files, {len(bad)} differ from the base')
    sys.exit(1 if bad else 0)
