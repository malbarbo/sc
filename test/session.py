#!/usr/bin/env python3
"""Phase 3 of PLANO-REPL.md (§6): the REPL's session through a native engine
with the protocol's loop (build/sc), as the REPL drives it,
`<engine> --replay` (src/bin/sc/main.rs). split- is test/split.py.

usage: session.py [--update] [<engine> [<outdir> [<base summary.json>]]]
       (build/sc, build/res/session)
SLICES=ses,seq,crash,mem picks the slices (all); ONLY=<name>,... (or a
file of names) the results; JOBS sets the parallelism (48); WFLAGS adds warning flags to the
session's (e.g. -Weverything; then seq- is not checked against expected.txt)

ses-    each source of the corpus (test/corpuslib.py) as the one cell of a
        session: accepted if and only if it compiles whole (the native base,
        build/res/native-base: pic- for a program), and a program gives the
        output of pic-. A source the corpus only compiles may fail to link.
        With WFLAGS, the base is build/res/native-wev-base.
        The session runs in the source's directory. KNOWN lists where a
        session cannot be the whole compile, with the reason.
seq-    test/seq/NAME.c or NAME.cpp, cells separated by a line `//%%`; a
        cell `//%undo` or `//%restart` (a new child that replays the log)
        is that step. What the cells and the run give must be
        test/seq/expected.txt (--update writes it).
crash-  each source on which the one-shot engine crashes or stops with a
        fatal error (the native base), as a cell between two good ones, then
        a %restart and a good cell with main: the session goes on, the
        replay exports the same names and the program runs. In the session
        the cell may only fail: `#pragma clang __debug parser_crash` is an
        error where a cell is parsed.
mem-    1000 small cells, C and C++, each in one session: after the first
        100, the child's RSS grows by less than 50 KB a cell, and by less
        than 5 MB over any 100 cells, each a link (§4).

A host of sc.wasm (build/sc-engine-wasm, the engine <engine>.wasm,
PLANO-REPL.md §5.2) is checked against the one-shot sc.wasm (build/res/base
and wev-base), where the native engine differs (a crash, the stack), and in
mem- its memory.size stands for the RSS.

Writes <outdir>/<name>.txt and <outdir>/summary.json (ses-, seq- and crash-,
normalized as the corpus's); with a base summary, lists the results that
differ from it. Fails if a slice's own check fails. A function the cuts
removed that the engine calls (scripts/link.py, native) is a line
`sc: cut function called: <name>` of the result.
"""
import difflib, glob, json, os, re, shutil, statistics, struct, subprocess, sys, threading, time
from concurrent.futures import ThreadPoolExecutor
sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
from corpuslib import IGNORE, diff, kill_session, norm_hash, only_names, run_all, sources, source_name
from spike import expected, run, same, std_pcm

R = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
sys.path.insert(0, f'{R}/scripts')
from flags import session  # the engine's flags
B = f'{R}/build'
SEQ = f'{R}/test/seq'
W_EXTRA = os.environ.get('WFLAGS', '').split()
# the one-shot engine's results, with the same warnings: -Weverything turns
# some errors into warnings
UPDATE = sys.argv[1:2] == ['--update']
ARGS = sys.argv[1 + UPDATE:]
ENG = os.path.realpath(ARGS[0] if ARGS else f'{B}/sc')
WASM = os.path.basename(ENG).startswith('sc-engine-wasm')  # sc with the feature wasm
BASE = f'{B}/res/{"" if WASM else "native-"}{"wev-" if W_EXTRA else ""}base'
OUT = os.path.realpath(ARGS[1] if len(ARGS) > 1 else f'{B}/res/session')
# where a session is not the whole compile (ANALISE.md §46)
FILE = '__FILE__ (#include, #embed, __has_embed) names the cell, which is no file'
MODULE = 'a module declaration cannot be in a cell'
STMT = 'an expression at the top is a statement in a cell'
DIR = 'the session runs in the source\'s directory, which has what "../" names; the corpus maps only the source'
TMP = 'the corpus runs the engine without /tmp, which #pragma clang module build writes to'
TU = 'a check at the end of the translation unit, which a session does not reach'
LD = 'wasm-ld fails on the shared object, outside the session too'
G = 'gcc-ts_gcc_testsuite_gcc.c-torture_'
C = 'src_clang_test_'
KNOWN = {f'ses-{k}': v for k, v in {
    G + 'execute_pr98304-2': DIR, G + 'compile_pr88423': DIR, C + 'SemaCXX_attr-callback': DIR,
    C + 'CodeGenCXX_alloc-token-pointer': DIR, C + 'CodeGenCXX_alloc-token': DIR,
    C + 'CodeGenCXX_microsoft-abi-emit-dependent': TMP,
    C + 'Preprocessor_cl-pathmap': STMT, C + 'Preprocessor_dumptokens_phyloc': STMT,
    C + 'Preprocessor_macro_rparen_scan': STMT, C + 'Misc_diag-macro-backtrace': STMT, C + 'Frontend_rewrite-macros': STMT,
    C + 'CXX_basic_basic.def_p4': TU, C + 'CXX_basic_basic.link_p8': TU,
    C + 'Sema_embed_compound_literal': FILE, C + 'Sema_excess-embed-data': FILE, C + 'CodeGen_excess-embed-data': FILE,
    C + 'Preprocessor_embed___has_embed_supported': FILE, C + 'SemaCXX_embed-init-list': FILE,
    C + 'SemaCXX_uninitialized': FILE, C + 'SemaCXX_warn-deprecated-specializations-in-system-headers': FILE,
    C + 'SemaCXX_warn-sysheader-macro': FILE, C + 'SemaCXX_warn-using-namespace-in-header': FILE,
    C + 'Misc_warn-sysheader': FILE,
    C + 'SemaCXX_P2615': MODULE, C + 'SemaCXX_lookup-through-export': MODULE,
    C + 'CodeGenCXX_cxx20-module-decomp-1': MODULE, C + 'CodeGenCXX_cxx20-module-initializer-pacbti': MODULE,
    C + 'CodeGenCXX_cxx20-module-part-1a': MODULE, C + 'CodeGenCXX_cxx20-module-part-1b': MODULE,
    C + 'CodeGenCXX_module-extern-C': MODULE, C + 'CodeGenCXX_module-intializer-pmf': MODULE,
    **{C + f'CXX_module_module.unit_p7_t{i}': MODULE for i in range(1, 6)},
    C + 'CodeGen_annotations-decl-use-decl': LD + ': a segfault, for an annotation on a function the object does not define',
    C + 'AST_cc-modifier': LD + ': "duplicate symbol name" for an asm operand "X"(f); in the session a fatal error, which exits',
    C + 'CodeGenCXX_wasm-reftypes-mangle': 'the guess between a statement and a declaration at the top takes'
                                          ' `void (*__funcref f)();` for a statement',
}.items()}
# not run: names whose demangling grows exponentially, which wasm-ld does
# for each cell's link (the corpus only compiles them)
SKIP = {C + 'CodeGenCXX_mangle-ms-back-references-pr13207', C + 'CodeGenCXX_pr29160'}


def flags(p):
    """the session's flags for the source p"""
    cxx = p.endswith(('.cpp', '.cc', '.cxx'))
    c17 = p.startswith((f'{B}/c-testsuite/', f'{B}/gcc-ts/'))
    fl = session(cxx, B, c17, '/test/mod/' in p or 'import std;' in open(p, errors='replace').read(), f'{OUT}/std.pcm')
    return fl[:fl.index('--')] + W_EXTRA + fl[fl.index('--'):]


def status(txt):
    """the status of each cell"""
    return re.findall(r'^== \S+ (\w+)$', txt, re.M)


def repl(name, p, cells, runs):
    """the cells (bytes: a source's \\r\\n as it is; or the steps %undo and
    %restart) through `<engine> repl`, in the source's directory, where a
    cell's "..." includes are. When runs(txt, ok) (ok: the session went on
    to the end), the shared objects run. The output, ok and what the run
    printed (None if not run); the output is <OUT>/<name>.txt"""
    d = f'{OUT}/{name}'
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    files = []
    for i, c in enumerate(cells):
        if c in ('%undo', '%restart'):
            files.append(c)
            continue
        files.append(f'{d}/c{i + 1:03}{os.path.splitext(p)[1]}')
        open(files[-1], 'wb').write(c)
    try:
        r = run_all([ENG, '--replay', d] + files + ['--'] + flags(p), 120, cwd=os.path.dirname(p))
        # what the engine writes on a crash (a backtrace) is not the
        # session's, but for the cut function it called
        cut = ''.join(re.findall(r'^sc: cut function called: \S+\n', r.stderr.decode(errors='replace'), re.M))
        txt, ok = r.stdout.decode(errors='replace') + cut + f'repl exit={r.returncode}\n', r.returncode == 0
    except subprocess.TimeoutExpired:
        txt, ok = 'repl TIMEOUT\n', False
    out = run(sorted(glob.glob(f'{d}/*.so'))) if runs(txt, ok) else None
    txt = (txt + (out or '')).replace(d + '/', '').replace(OUT + '/', 'OUT/')
    open(f'{OUT}/{name}.txt', 'w').write(txt)
    shutil.rmtree(d)
    return txt, ok, out


# --- ses-

def whole(name, runs):
    """whether the source compiles whole: its pic- for a program"""
    t = open(f'{BASE}/{"pic-" * runs}{name}.txt').read()
    return 'cc1 exit=0\n' in t and (not runs or 'ld exit=0\n' in t)


def ses(name, src):
    p, runs, n = src
    txt, ok, out = repl(name, p, [open(p, 'rb').read()], lambda txt, ok: runs and status(txt) == ['ok'])
    acc = status(txt) == ['ok']
    # the corpus only compiles what it does not run: a cell that fails to link compiled
    linked = re.search(r'^wasm-ld: error:', txt, re.M) and not re.search(r'^\S+:\d+:\d+: (fatal )?error:', txt, re.M)
    good = ok and (acc or not runs and bool(linked)) == whole(n, runs)
    return txt, good and (out is None or same(out, expected(n)))


# --- seq-

def seq(name, p):
    cells = [c if c.strip() not in (b'//%undo', b'//%restart') else c.strip()[2:].decode()
             for c in re.split(rb'^//%%\n', open(p, 'rb').read(), flags=re.M)]
    return repl(name, p, cells, lambda txt, ok: ok)[0], None


# --- crash-

BEFORE = b'int sc_before(void) { return 41; }\n'
AFTER = b'int sc_after(void) { return sc_before() + 1; }\n'
MAIN = b'#include <stdio.h>\nint main(void) { printf("%d\\n", sc_after()); }\n'


def crashes(name):
    """whether the one-shot engine crashes on the source or stops with a fatal error"""
    m = re.search(r'^cc1 exit=(\d+)$', open(f'{BASE}/{name}.txt').read(), re.M)
    return m is not None and m[1] not in ('0', '1')


def crash(name, src):
    p = src[0]
    cells = [BEFORE, open(p, 'rb').read(), AFTER, '%restart', MAIN]
    # all but the source's cell accepted
    txt, _, out = repl(name, p, cells, lambda txt, ok: ok and (st := status(txt))[:1] + st[2:] == ['ok'] * 4)
    return txt, out == '42\nrun exit=0\n'


# --- mem-

def mem_cells(cxx, n):
    """n small cells: functions, globals and statements; in C++ with
    templates and std::string"""
    cs = ['#include <string>\n#include <vector>\n' if cxx else '#include <stdio.h>\n']
    for i in range(1, n):
        k = i % 4
        if k == 1:
            cs.append(f'int f{i}(int x) {{ return x * {i % 7} + {i}; }}\n')
        elif k == 2:
            cs.append(f'int g{i} = {i};\n')
        elif k == 3:
            cs.append(f'g{i - 1} += f{i - 2}(g{i - 1});\n')
        elif cxx:
            cs.append(f'template <class T> struct S{i} {{ T v; T get() const {{ return v; }} }};\n'
                      f'std::string s{i} = std::to_string(S{i}<int>{{{i}}}.get());\n')
        else:
            cs.append(f'struct S{i} {{ int a, b; }};\nint h{i}(struct S{i} s) {{ return s.a + s.b; }}\n')
    return cs


class Child:
    """`<engine> --engine`, through the protocol (engine/sc.h)"""
    NEW, CELL, REPLY, SO = 1, 2, 4, 0

    def __init__(self, timeout):
        """the child, in a session of its own with the copies it forks: all
        of them killed after timeout seconds, when a reply is an EOFError"""
        self.p = subprocess.Popen([ENG, '--engine'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE if WASM else None, start_new_session=True,
                                  env=dict(os.environ, SC_MEMORY_SIZE='1') if WASM else None)
        self.timer = threading.Timer(timeout, self.kill)
        self.timer.start()

    def kill(self):
        while kill_session(self.p.pid):
            pass

    def start(self, args):
        st, (diags, _, _) = self.send(self.NEW, 0, b''.join(a.encode() + b'\0' for a in args))
        if st != 0:
            raise ValueError(f'no session: {diags.decode(errors="replace")}')

    def send(self, kind, st, part):
        """the reply's status and parts; EOFError if the child ended"""
        self.p.stdin.write(struct.pack('=5I', kind, st, len(part), 0, 0) + part)
        self.p.stdin.flush()
        h = self.p.stdout.read(20)
        if len(h) != 20:
            raise EOFError(f'the child ended (exit {self.p.wait()})')
        k, st, *ls = struct.unpack('=5I', h)
        parts = [self.p.stdout.read(n) for n in ls]
        if k != self.REPLY or any(len(x) != n for x, n in zip(parts, ls)):
            raise ValueError(f'not a reply: kind {k}')
        return st, parts

    def rss(self):
        """the RSS of the process spawned, in KB: the session while its cells
        are accepted; in sc.wasm, the memory.size the host writes after each
        cell (one line a cell)"""
        while WASM:
            l = self.p.stderr.readline()
            if m := re.match(rb'sc: memory.size (\d+)$', l):
                return int(m[1]) // 1024
            if not l:
                raise EOFError('no memory.size')
            sys.stderr.buffer.write(l)
        return int(re.search(r'VmRSS:\s+(\d+)', open(f'/proc/{self.p.pid}/status').read())[1])

    def close(self):
        self.timer.cancel()
        try:
            self.p.stdin.close()
        except OSError:  # it ended
            pass
        self.p.wait()


def mem(name, cxx):
    rows, t0, bad = [], time.time(), []
    c = Child(600)
    try:
        c.start(session(cxx, B))
        for i, src in enumerate(mem_cells(cxx, 1000)):
            st, (diags, _, _) = c.send(Child.CELL, Child.SO, src.encode())
            if st != 0:
                bad.append(f'cell {i + 1}: {diags.decode(errors="replace")}')
            rows.append((i + 1, c.rss()))
    except (EOFError, ValueError, OSError) as e:
        txt = ''.join(bad) + f'after {len(rows)} cells: {e}\n'
        open(f'{OUT}/{name}.txt', 'w').write(txt)
        return txt, False
    finally:
        c.close()
    secs = time.time() - t0
    slope = statistics.linear_regression(*zip(*rows[100:])).slope
    step = max(rows[i + 100][1] - rows[i][1] for i in range(100, len(rows) - 100))
    good = not bad and slope < 50 and step < 5 * 1024
    txt = (''.join(bad) + f'{len(rows)} cells, {secs / len(rows) * 1000:.1f} ms a cell; RSS {rows[99][1] // 1024} MB after 100,'
           f' {rows[-1][1] // 1024} MB after {len(rows)}; slope {slope:.1f} KB a cell, at most {step} KB over 100 cells\n')
    open(f'{OUT}/{name}.txt', 'w').write(txt + ''.join(f'{x} {y}\n' for x, y in rows))
    return txt, good


def main():
    os.makedirs(OUT, exist_ok=True)
    slices = os.environ.get('SLICES', 'ses,seq,crash,mem').split(',')
    only = only_names()
    srcs = {}
    for p, runs in sources(R, B):
        srcs[source_name(p, R, B)] = (p, runs)  # the last of a name, as the corpus's results
    srcs = [(p, runs, n) for n, (p, runs) in srcs.items() if n not in SKIP]
    # (name, slice, its argument), the longest first: mem- is 1000 cells in a row
    jobs = []
    if 'mem' in slices:
        jobs += [(f'mem-{"cxx" if x else "c"}', mem, x) for x in (False, True)]
    if 'crash' in slices:
        jobs += [(f'crash-{s[2]}', crash, s) for s in srcs if crashes(s[2])]
    if 'ses' in slices:
        jobs += [(f'ses-{s[2]}', ses, s) for s in srcs]
    if 'seq' in slices:
        jobs += [('seq-' + os.path.splitext(os.path.basename(p))[0], seq, p)
                 for p in sorted(glob.glob(f'{SEQ}/*.c') + glob.glob(f'{SEQ}/*.cpp'))]
    if only:
        jobs = [j for j in jobs if j[0] in only]
    std_pcm(ENG, OUT)
    with ThreadPoolExecutor(int(os.environ.get('JOBS', '48'))) as ex:
        rs = list(ex.map(lambda j: (j[0], *j[1](j[0], j[2])), jobs))
    bad = [n for n, _, good in rs if good is False and n not in KNOWN]
    known = [n for n, _, good in rs if good is False and n in KNOWN]
    for n in (n for n, _, good in rs if good and n in KNOWN):
        print('KNOWN, now good (take it out of KNOWN):', n)
    for n, txt, _ in rs:
        if n.startswith('mem-'):
            print(n, txt.splitlines()[-1])
    # seq-: the expected output, all of them in one file
    got = ''.join(f'== {n}\n{txt}' for n, txt, _ in rs if n.startswith('seq-'))
    if 'seq' in slices and not only and not W_EXTRA:
        if UPDATE:
            open(f'{SEQ}/expected.txt', 'w').write(got)
        exp = open(f'{SEQ}/expected.txt').read() if os.path.exists(f'{SEQ}/expected.txt') else ''
        d = list(difflib.unified_diff(exp.splitlines(), got.splitlines(), 'expected.txt', 'got', lineterm=''))
        print('\n'.join(d))
        bad += ['seq- (expected.txt)'] * bool(d)
    summary = {n: norm_hash(txt) for n, txt, _ in rs if not n.startswith('mem-')}
    if not only:
        json.dump(summary, open(f'{OUT}/summary.json', 'w'), indent=0, sort_keys=True)
    for n in bad:
        print('BAD', n)
    if len(ARGS) > 2:
        # the corpus's IGNORE, in each slice
        base = {k: v for k, v in json.load(open(ARGS[2])).items()
                if k in summary and k.split('-', 1)[1] not in IGNORE}
        for k in diff(base, summary):
            print('DIFF', k)
            bad.append(k)
    print(f'{len(rs)} results, {len(bad)} bad ({len(known)} known)')
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
