#!/usr/bin/env python3
"""Run the C corpus through an sc engine under wasmtime.

usage: corpus.py <engine.wasm> <outdir> [<base summary.json>]
Programs (test/samples, c-testsuite, gcc torture execute/) are compiled with
-cc1, linked with the engine's own ld and run. The rest (gcc torture compile/,
clang's C tests) are only compiled: diagnostics and the object's md5.
Writes <outdir>/<name>.txt and <outdir>/summary.json {name: normalized md5}.
With a base summary, lists the files whose result differs and fails if any.
CORPUS=small runs only the programs; JOBS sets the parallelism (48);
WFLAGS adds warning flags (e.g. -Weverything).
C++ (gnu++20, no exceptions) goes the same way: the programs in test/cxx,
linked with libc++, and clang's C++ tests.
"""
from corpuslib import norm_hash, diff
import hashlib, re, json, os, subprocess, sys, glob
from concurrent.futures import ThreadPoolExecutor
R = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
B = os.environ.get('SC_BUILD', f'{R}/build')
W = f'{B}/wasi-sdk-29.0-x86_64-linux'; RES = f'{W}/lib/clang/21'; SR = f'{W}/share/wasi-sysroot'
FLAGS = ['-cc1', '-triple', 'wasm32-unknown-wasip1', '-emit-obj', '-Wall', '-Wextra',
         '-disable-free', '-clear-ast-before-backend', '-disable-llvm-verifier', '-discard-value-names',
         '-mrelocation-model', 'static', '-mframe-pointer=none', '-ffp-contract=on', '-fno-rounding-math',
         '-target-cpu', 'generic', '-fvisibility=hidden', '-ferror-limit', '19', '-fgnuc-version=4.2.1',
         '-isysroot', '/sysroot', '-internal-isystem', '/res/include',
         '-internal-externc-isystem', '/sysroot/include/wasm32-wasip1', '-internal-externc-isystem', '/sysroot/include']
# C++ as the driver sets it for wasm32-wasip1, with libc++ first in the search path
CXXFLAGS = FLAGS[:FLAGS.index('/res/include') - 1] + [
    '-internal-isystem', '/sysroot/include/wasm32-wasip1/c++/v1', '-internal-isystem', '/sysroot/include/c++/v1'] + \
    FLAGS[FLAGS.index('/res/include') - 1:] + ['-std=gnu++20', '-mconstructor-aliases', '-fdeprecated-macro', '-fno-implicit-modules', '-fskip-odr-check-in-gmf']
CXXEXT = ('.cpp', '.cc', '.cxx')
W_EXTRA = os.environ.get('WFLAGS', '').split()  # e.g. WFLAGS=-Weverything
if sys.argv[1:3] == ['--flags', 'c++']:
    sys.exit(print(' '.join(CXXFLAGS[1:])))
if sys.argv[1] == '--flags':  # the cc1 flags, for test/checkinv
    sys.exit(print(' '.join(FLAGS[1:])))
eng, out = os.path.realpath(sys.argv[1]), sys.argv[2]
# objects and programs go to a directory of their own: it stays small, so
# looking for the temporary a failed write leaves is cheap
tmp = f'{out}/o'
OUT = os.path.realpath(out) + '/'
os.makedirs(tmp, exist_ok=True)
cw = f'{out}/engine.cwasm'
subprocess.check_call(['wasmtime', 'compile', eng, '-o', cw])
# (path, run?) — run: link and execute; otherwise compile only (diagnostics + object hash)
progs = [(p, True) for p in sorted(glob.glob(f'{R}/test/samples/*.c')) + sorted(glob.glob(f'{B}/c-testsuite/tests/single-exec/*.c'))]
if os.environ.get('CORPUS', 'full') == 'full':
    T = f'{B}/src/clang/test'
    progs += [(p, True) for p in sorted(glob.glob(f'{B}/gcc-ts/gcc/testsuite/gcc.c-torture/execute/*.c'))]
    progs += [(p, False) for p in sorted(glob.glob(f'{B}/gcc-ts/gcc/testsuite/gcc.c-torture/compile/*.c'))]
    for sub in ['Sema', 'Parser', 'CodeGen', 'Lexer', 'Preprocessor', 'Misc', 'Frontend', 'AST', 'C/*', 'C/*/*']:
        progs += [(p, False) for p in sorted(glob.glob(f'{T}/{sub}/*.c'))]
progs += [(p, True) for p in sorted(glob.glob(f'{R}/test/cxx/*.cpp'))]
if os.environ.get('CORPUS', 'full') == 'full':
    for sub in ['SemaCXX', 'SemaTemplate', 'CodeGenCXX', 'CXX/**', 'Parser', 'Lexer', 'AST', 'Misc']:
        progs += [(p, False) for p in sorted(glob.glob(f'{T}/{sub}/*.cpp', recursive=True))]

def one(pr):
    p, runit = pr
    d, f = os.path.dirname(p), os.path.basename(p)
    rel = os.path.relpath(p, B)
    stem, rel = os.path.splitext(f)[0], os.path.splitext(rel)[0]
    n = stem if d == f'{R}/test/samples' else 'cxx-' + stem if d == f'{R}/test/cxx' else (
        'ct-' + stem if 'c-testsuite' in d else rel.replace('/', '_'))
    if len(n) > 200: n = n[-200:]
    try:
        return n, one1(p, d, f, n, runit)
    except subprocess.TimeoutExpired:
        open(f'{out}/{n}.txt', 'w').write('TIMEOUT\n'); return n, 'TIMEOUT'

def one1(p, d, f, n, runit):
    cxx = f.endswith(CXXEXT)
    obj = f'{tmp}/{n}.o'
    r = subprocess.run(['wasmtime', 'run', '--allow-precompiled', '--dir', f'{d}::/c', '--dir', f'{tmp}::/o', '--dir', f'{SR}::/sysroot',
                        '--dir', f'{RES}::/res', cw] + (CXXFLAGS if cxx else FLAGS) + W_EXTRA + ['-o', f'/o/{n}.o', f'/c/{f}'],
                       capture_output=True, timeout=300)
    o = open(obj, 'rb').read() if os.path.exists(obj) else b''
    err = r.stderr.decode(errors='replace')
    txt = err.split('wasm trap')[0] + ('<TRAP>\n' + err.split('wasm trap', 1)[1] if 'wasm trap' in err else '') + f'cc1 exit={r.returncode}\n'
    if o and not runit:
        txt += 'obj ' + hashlib.md5(o).hexdigest() + '\n'
    if r.returncode == 0 and o and runit:
        l = subprocess.run(['wasmtime', 'run', '--allow-precompiled', '--dir', f'{tmp}::/o', '--dir', f'{SR}::/sysroot',
                            '--dir', f'{RES}::/res', cw, 'ld', f'/o/{n}.o', '/sysroot/lib/wasm32-wasip1/crt1-command.o',
                            '-L/sysroot/lib/wasm32-wasip1', '-lc-printscan-long-double'] + (['-lc++', '-lc++abi'] if cxx else []) +
                           ['-lc', '-lm', '/res/lib/wasm32-unknown-wasip1/libclang_rt.builtins.a', '-o', f'/o/{n}.wasm'],
                           capture_output=True, text=True, timeout=300)
        txt += l.stderr
        if l.returncode == 0:
            try:
                x = subprocess.run(['wasmtime', f'{tmp}/{n}.wasm'], input=b'linha1\nlinha2\n', capture_output=True, timeout=20)
            except subprocess.TimeoutExpired as e:
                x = subprocess.CompletedProcess(e.cmd, 'timeout', e.stdout or b'', e.stderr or b'')
            txt += x.stdout.decode(errors='replace') + x.stderr.decode(errors='replace') + f'run exit={x.returncode}\n'
            os.remove(f'{tmp}/{n}.wasm')
    for t in [obj] * os.path.exists(obj) + glob.glob(f'{tmp}/{glob.escape(n)}-{"[0-9a-f]" * 8}.o.tmp'):
        os.remove(t)  # the object and the temporary a failed write leaves
    txt = re.sub(r'main module `[^`]*/', 'main module `', txt)
    txt = txt.replace(OUT, 'OUT/').replace(out + '/', 'OUT/')
    open(f'{out}/{n}.txt', 'w').write(txt)
    return norm_hash(txt)

SKIP = ('warn-date-time', 'SOURCE_DATE_EPOCH', 'pr133574', 'builtin_redef')  # depend on the clock
progs = [p for p in progs if not p[0].endswith(tuple(x + '.c' for x in SKIP))]
with ThreadPoolExecutor(int(os.environ.get('JOBS', '48'))) as ex:
    res = dict(ex.map(one, progs))
os.remove(cw)
json.dump(res, open(f'{out}/summary.json', 'w'), indent=0, sort_keys=True)
print(hashlib.md5(json.dumps(res, sort_keys=True).encode()).hexdigest())
if len(sys.argv) > 3:
    base = json.load(open(sys.argv[3]))
    if os.environ.get('CORPUS', 'full') != 'full':  # only the files that ran
        base = {k: v for k, v in base.items() if k in res}
    bad = diff(base, res)
    for k in bad:
        print('DIFF', k)
    print(f'{len(res)} files, {len(bad)} differ from the base')
    sys.exit(1 if bad else 0)
