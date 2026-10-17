"""Normalized results of the corpus (test/corpus.py).

usage: corpuslib.py <outdir>   recompute <outdir>/summary.json from its .txt files
"""
import glob, hashlib, json, os, re, signal, subprocess, sys

# depend on the clock
SKIP = ('warn-date-time', 'SOURCE_DATE_EPOCH', 'pr133574', 'builtin_redef')

# tests whose result is not a function of the compiler's behaviour
# (order of pointer-keyed maps, stack overflow in the baseline)
# c2x-fallthrough: in C23 the order of its [[fallthrough]] errors, and so which
# source lines are shown, changes from run to run in the native engine
IGNORE = {'src_clang_test_C_C99_n590', 'src_clang_test_CodeGen_alias', 'src_clang_test_Sema_c2x-fallthrough'}


def sources(R, B, full=True):
    """(path, runs) of each source of the corpus: runs, a program to link and
    execute; else only compiled. full: with gcc torture and clang's tests"""
    T = f'{B}/src/clang/test'
    ps = [(p, True) for p in sorted(glob.glob(f'{R}/test/samples/*.c')) + sorted(glob.glob(f'{B}/c-testsuite/tests/single-exec/*.c'))]
    if full:
        ps += [(p, True) for p in sorted(glob.glob(f'{B}/gcc-ts/gcc/testsuite/gcc.c-torture/execute/*.c'))]
        ps += [(p, False) for p in sorted(glob.glob(f'{B}/gcc-ts/gcc/testsuite/gcc.c-torture/compile/*.c'))]
        for sub in ['Sema', 'Parser', 'CodeGen', 'Lexer', 'Preprocessor', 'Misc', 'Frontend', 'AST', 'C/*', 'C/*/*']:
            ps += [(p, False) for p in sorted(glob.glob(f'{T}/{sub}/*.c'))]
    ps += [(p, True) for p in sorted(glob.glob(f'{R}/test/cxx/*.cpp')) + sorted(glob.glob(f'{R}/test/mod/*.cpp'))]
    if full:
        for sub in ['SemaCXX', 'SemaTemplate', 'CodeGenCXX', 'CXX/**', 'Parser', 'Lexer', 'AST', 'Misc']:
            ps += [(p, False) for p in sorted(glob.glob(f'{T}/{sub}/*.cpp', recursive=True))]
    skip = tuple(s + '.c' for s in SKIP)
    return [x for x in ps if not x[0].endswith(skip)]


def source_name(p, R, B):
    """the name of the results of a source"""
    d, f = os.path.dirname(p), os.path.basename(p)
    stem, rel = os.path.splitext(f)[0], os.path.splitext(os.path.relpath(p, B))[0]
    n = stem if d == f'{R}/test/samples' else 'cxx-' + stem if d == f'{R}/test/cxx' else 'mod-' + stem if d == f'{R}/test/mod' else (
        'ct-' + stem if 'c-testsuite' in d else rel.replace('/', '_'))
    return n[-200:]


def only_names():
    """the names of ONLY: <name>,... or a file of them, one per line; None
    without ONLY"""
    o = os.environ.get('ONLY')
    if not o:
        return None
    return open(o).read().split() if os.path.isfile(o) else o.split(',')


def run_all(cmd, timeout, cwd=None):
    """subprocess.run(cmd, capture_output=True), in a session of its own: on
    a timeout, every process of the session is killed, the children it put
    in process groups of their own too (the REPL's engine)"""
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True, cwd=cwd)
    try:
        out, err = p.communicate(timeout=timeout)
        return subprocess.CompletedProcess(cmd, p.returncode, out, err)
    except subprocess.TimeoutExpired:
        while kill_session(p.pid):  # again: one may have forked during the scan
            pass
        p.communicate()
        raise


def kill_session(sid):
    """SIGKILL to each live process of the session sid (not a zombie, which
    is already dead): how many"""
    n = 0
    for d in os.listdir('/proc'):
        try:  # the fields after the name: state, ppid, pgrp, session
            f = d.isdigit() and open(f'/proc/{d}/stat').read().rsplit(')', 1)[1].split()
            if f and f[0] != 'Z' and int(f[3]) == sid:
                os.kill(int(d), signal.SIGKILL)
                n += 1
        except (OSError, ValueError):
            pass
    return n


def norm_hash(txt):
    """md5 of a result without addresses, backtraces and line order (the order
    of some diagnostics depends on pointer-keyed maps)"""
    if txt == 'TIMEOUT\n':
        return 'TIMEOUT'
    # a trap's backtrace depends on the names section and the stack depth
    # (wasmtime prints the note only when the module has names)
    norm = re.sub(r'^note: using the `WASMTIME_BACKTRACE_DETAILS=1`.*\n', '', txt, flags=re.M)
    norm = re.sub(r'error while executing at wasm backtrace:.*?(?=\n\S|\Z)', 'TRAP', norm, flags=re.S)
    norm = re.sub(r'0x[0-9a-f]+', '0x?', norm)
    norm = re.sub(r'-[0-9a-f]{6}\.pcm\b', '-?.pcm', norm)  # random temporary names
    norm = re.sub(r'/tmp/sc-\w{6}/', '/tmp/sc-?/', norm)  # a REPL cell's link (sc_session.cpp)
    return hashlib.md5('\n'.join(sorted(norm.splitlines())).encode()).hexdigest()


def diff(base, got):
    return [k for k in base if k not in IGNORE and got.get(k) != base[k]]


if __name__ == '__main__':
    d = sys.argv[1]
    res = {f[:-4]: norm_hash(open(f'{d}/{f}').read()) for f in os.listdir(d) if f.endswith('.txt')}
    json.dump(res, open(f'{d}/summary.json', 'w'), indent=0, sort_keys=True)
