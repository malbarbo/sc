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
by the REPL's loader, build/cargo/release/scload (pic-), and test/repl has
sequences of REPL inputs, each loaded on top of the ones before (repl-).
The programs run again at each level of --safety but off (<name>-on,
<name>-strict; PLANO-VERIFICACOES.md §9): those that have undefined
behaviour on purpose (c-testsuite, the gcc torture tests) with the errors of
the levels as warnings. Their checks at run time take libsc_rt, and the
programs in test/mod a std module of their level (std-pcm-on...): it keeps
the macros it was built with, libc++'s hardening among them. As a REPL
session does, pic-, repl- and the levels compile with -gline-tables-only:
the debug info, for where a trap is (ANALISE.md §31, §33).
"""
from corpuslib import norm_hash, diff, sources, source_name
import hashlib, re, json, os, subprocess, sys, glob
from concurrent.futures import ThreadPoolExecutor
R = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
sys.path.insert(0, f'{R}/scripts')
from flags import FLAGS, CXXFLAGS, MODFILE, iface, gnu17, pic, lines, safety, checks, solibs, rt_static, res  # the engine's flags
B = os.environ.get('SC_BUILD', f'{R}/build')
RES = res(B)
SR = f'{B}/sysroot'  # wasi-sdk's libc with libc++, libc++abi and libunwind built from the pin with exceptions
CXXEXT = ('.cpp', '.cc', '.cxx')
SCLOAD = f'{B}/cargo/release/scload'  # the REPL's loader (src/loader.rs)
LIB = f'{SR}/lib/wasm32-wasip1'  # libc.so, libc++.so...
W_EXTRA = os.environ.get('WFLAGS', '').split()  # e.g. WFLAGS=-Weverything
STD_CPPM = f'{SR}/share/libc++/v1/std.cppm'  # libc++'s std module


eng, out = os.path.realpath(sys.argv[1]), sys.argv[2]
# objects and programs go to a directory of their own: it stays small, so
# looking for the temporary a failed write leaves is cheap
tmp = f'{out}/o'
OUT = os.path.realpath(out) + '/'
os.makedirs(tmp, exist_ok=True)
M = f'{out}/m'  # the std module, built by the engine under test
os.makedirs(M, exist_ok=True)
NATIVE = not eng.endswith('.wasm')
# a native engine is sc, the REPL, which names the engine's modes --cc1 and
# --ld (sc ld would be a student's program called ld); sc.wasm, -cc1 and ld
if not NATIVE:
    cw = f'{out}/engine.cwasm'
    subprocess.check_call(['wasmtime', 'compile', eng, '-o', cw])


def engine(dirs, args):
    """the command that runs the engine with dirs {host: guest} mapped"""
    if NATIVE:
        args = ['--' + args[0].lstrip('-')] + args[1:]
    if NATIVE:
        return ['bwrap', '--ro-bind', '/usr', '/usr', '--symlink', 'usr/lib', '/lib', '--symlink', 'usr/lib64', '/lib64',
                '--dev', '/dev', '--ro-bind', eng, '/sc'] + [x for h, g in dirs.items() for x in ('--bind', h, g)] + ['/sc'] + args
    # the loader's primitives (src/loader/host.rs), which the one-shot engine never calls
    return ['wasmtime', 'run', '--allow-precompiled', '-W', 'unknown-imports-trap=y'] + [x for h, g in dirs.items() for x in ('--dir', f'{h}::{g}')] + [cw] + args

# (path, kind) — True: link and execute; False: compile only (diagnostics + object
# hash); 'pic': compile with -fPIC and link a shared object, as the REPL does
# for each input (diagnostics + the object's md5); 'on', 'strict': as True, at
# that level of --safety
progs = sources(R, B, os.environ.get('CORPUS', 'full') == 'full')
SAFETY = ('on', 'strict')
progs += [(p, k) for p, t in list(progs) if t is True for k in ('pic',) + SAFETY]

def name(pr):
    """the name of a result: <outdir>/<name>.txt"""
    p, kind = pr
    n = source_name(p, R, B)
    return 'pic-' + n if kind == 'pic' else f'{n}-{kind}' if kind in SAFETY else n


def one(pr):
    p, kind = pr
    d, f = os.path.dirname(p), os.path.basename(p)
    n = name(pr)
    try:
        return n, one1(p, d, f, n, kind)
    except subprocess.TimeoutExpired:
        open(f'{out}/{n}.txt', 'w').write('TIMEOUT\n'); return n, 'TIMEOUT'

def cc1(d, f, n, fl, mod, level=None):
    """compiles d/f to tmp/n.o: the result text, the object and whether it
    succeeded; mod: with the std module of the level"""
    dirs = {d: '/c', tmp: '/o', SR: '/sysroot', RES: '/res'} | ({pcm_dir(level): '/m'} if mod else {})
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
                        'ld', '-shared', '--experimental-pic', '--unresolved-symbols=import-dynamic', f'/o/{n}.o'] +
                       solibs(cxx) + ['-o', f'/o/{n}.so']),
                       capture_output=True, text=True, timeout=300)
    so = f'{tmp}/{n}.so'
    txt = l.stderr + f'ld exit={l.returncode}\n'
    if l.returncode == 0:
        txt += 'so ' + hashlib.md5(open(so, 'rb').read()).hexdigest() + '\n'
    return txt, l.returncode == 0

def run(cmd):
    # a few programs under ASan (gcc torture's VLAs) take more than 20 s when
    # the base is made with 48 jobs (ANALISE.md §51), and 20040811-1-strict
    # more than 60 s on a loaded machine (§70); none loops for ever
    try:
        x = subprocess.run(cmd, input=b'linha1\nlinha2\n', capture_output=True, timeout=300)
    except subprocess.TimeoutExpired as e:
        x = subprocess.CompletedProcess(e.cmd, 'timeout', e.stdout or b'', e.stderr or b'')
    return x.stdout.decode(errors='replace') + x.stderr.decode(errors='replace') + f'run exit={x.returncode}\n'

def one1(p, d, f, n, kind):
    cxx = f.endswith(CXXEXT)
    obj = f'{tmp}/{n}.o'
    ub = p.startswith((f'{B}/c-testsuite/', f'{B}/gcc-ts/'))  # undefined behaviour on purpose
    fl = CXXFLAGS if cxx else gnu17(FLAGS) if ub else FLAGS
    fl = lines(pic(fl)) if kind == 'pic' else fl
    level = kind if kind in SAFETY else None
    if level:
        fl, kind = lines(safety(fl, level, cxx, file=True, errors=not ub)), True
    txt, o, ok = cc1(d, f, n, fl, d == f'{R}/test/mod', level)
    if o and kind is not True:
        txt += 'obj ' + hashlib.md5(o).hexdigest() + '\n'
    if ok and o and kind == 'pic':  # the shared object, run by the REPL's loader
        t, ok = ld_so(n, cxx)
        txt += t + run([SCLOAD, '-L', LIB, f'{tmp}/{n}.so']) * ok
        if ok: os.remove(f'{tmp}/{n}.so')
    if ok and o and kind is True:
        l = subprocess.run(engine({tmp: '/o', SR: '/sysroot', RES: '/res'}, ['ld', f'/o/{n}.o', '/sysroot/lib/wasm32-wasip1/crt1-command.o',
                            '-L/sysroot/lib/wasm32-wasip1', '-lc-printscan-long-double'] + rt_static(level) +
                           (['-lc++', '-lc++abi'] if cxx else []) +
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
        t, o, ok = cc1(d, f, n, lines(pic(CXXFLAGS if cxx else FLAGS)), 'import std;' in open(f'{d}/{f}').read())
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

def pcm_dir(level):
    """the directory of the std module of a level of --safety, or of none"""
    return M + (f'-{level}' if level else '')

def std_pcm(level=None):
    """libc++'s std module, without <csignal>, which WASI's libc rejects; at
    a level, with its checks"""
    m = pcm_dir(level)
    os.makedirs(m, exist_ok=True)
    src = [l for l in open(STD_CPPM) if 'csignal' not in l]
    open(f'{m}/std.cppm', 'w').writelines(src)
    os.utime(f'{m}/std.cppm', (0, 0))  # the .pcm records it
    if os.path.exists(f'{m}/std.pcm'):
        os.remove(f'{m}/std.pcm')
    fl = iface(checks(CXXFLAGS, level, True) if level else CXXFLAGS)
    r = subprocess.run(engine({m: '/m', SR: '/sysroot', RES: '/res'}, fl + W_EXTRA + ['-o', '/m/std.pcm', '/m/std.cppm']),
                       capture_output=True, timeout=600)
    txt = r.stderr.decode(errors='replace') + f'cc1 exit={r.returncode}\n'
    if os.path.exists(f'{m}/std.pcm'):
        txt += 'pcm ' + hashlib.md5(open(f'{m}/std.pcm', 'rb').read()).hexdigest() + '\n'
    n = 'std-pcm' + (f'-{level}' if level else '')
    open(f'{out}/{n}.txt', 'w').write(txt)
    return n, norm_hash(txt)

seqs = sorted({re.match(r'[a-z]+', f)[0] for f in os.listdir(f'{R}/test/repl')})
if os.environ.get('ONLY'):  # ONLY=<file>: only the results named there, one per line
    only = set(open(os.environ['ONLY']).read().split())
    progs = [pr for pr in progs if name(pr) in only]
    seqs = [s for s in seqs if f'repl-{s}' in only]
with ThreadPoolExecutor(int(os.environ.get('JOBS', '48'))) as ex:
    res = dict(ex.map(std_pcm, (None,) + SAFETY))
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
