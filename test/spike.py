#!/usr/bin/env python3
"""Phase 0 of PLANO-REPL.md: the two candidates for compiling REPL cells,
on build/sc-spike (the uncut native engine with engine/sc_spike.cpp).

usage: spike.py [<outdir>]   (build/res/spike)
ONLY=<name>,... runs only those programs; JOBS sets the parallelism (1, for the times)

The programs of test/samples, test/cxx and test/mod are cut into one cell
per top-level declaration (a preprocessor directive, a declaration ending
in ';', a function or namespace body). Each candidate compiles the cells in
order, each cell is linked as a shared object and the loader runs them all;
the output must be the one of the program compiled whole (pic-, from the
native base, build/res/native-base).

  A  one session (engine/sc_session.cpp), cells sent over a pipe
  B  stateless: each cell a PCH chained on the one before, and its object
     compiled from that PCH (-fpch-codegen)

crash: a fatal cell (#pragma clang __debug crash) in the middle of each
program. A loses the session, starts another and replays the cells before
(without the backend); the names each replayed cell exports must be the
ones of the first run, and the cells after must export the same names and
give the same output as without the crash.

Writes <outdir>/report.txt (and a line per program on stdout), with the
time per cell and the memory of each candidate.
"""
import json, os, re, shutil, subprocess, sys, time
from concurrent.futures import ThreadPoolExecutor
sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
from corpuslib import norm_hash

R = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
B = f'{R}/build'
ENG = f'{B}/sc-spike'
SR = f'{B}/sysroot'
RES = f'{B}/wasi-sdk-29.0-x86_64-linux/lib/clang/21'
LIB = f'{SR}/lib/wasm32-wasip1'
SCLOAD = f'{B}/loader/release/scload'
BASE = f'{B}/res/native-base'
OUT = os.path.realpath(sys.argv[1] if len(sys.argv) > 1 else f'{B}/res/spike')
CRASH = '#pragma clang __debug crash\n'


def flags(cxx, mod):
    fl = subprocess.check_output([f'{R}/test/corpus.py', '--flags'] + ['c++'] * cxx + ['pic'] + ['mod'] * mod,
                                 text=True).split()
    return [x.replace('/sysroot', SR).replace('/res/', RES + '/').replace('/m/std.pcm', f'{OUT}/std.pcm') for x in fl]


# --- one cell per top-level declaration

# a struct, union or enum with attributes, whose '{' is not a function body
TAG = re.compile(r'\s*(typedef\s+)?(struct|union|enum)\b(\s*__attribute__\s*\(\(.*?\)\))*\s*\w*\s*\{$', re.S)
# the head of a K&R definition, up to a declaration of its parameters
KR = re.compile(r'\s*[\w\s*]*?\b\w+\s*\(\s*(?!void\s*\))\w+(\s*,\s*\w+)*\s*\)\s*(?!__attribute__|asm\b|__asm)[A-Za-z_]')


def split(src):
    """the top-level declarations of a C or C++ source, each with the
    comments and blank lines before it"""
    cells, cur, i, n = [], '', 0, len(src)
    depth, body = 0, False  # body: the '{' at depth 0 opens a function or namespace body

    def end():
        nonlocal cur, body
        cells.append(cur)
        cur, body = '', False

    while i < n:
        c = src[i]
        if c == '#' and depth == 0 and src[src.rfind('\n', 0, i) + 1:i].strip() == '':
            # a directive (with its continuations); #if ... #endif in one cell
            j, nest = i, 0
            while True:
                e = src.find('\n', j)
                e = n if e < 0 else e
                while e < n and src[e - 1] == '\\':
                    e = src.find('\n', e + 1)
                    e = n if e < 0 else e
                d = src[j:e].strip()
                if re.match(r'#\s*if', d):
                    nest += 1
                elif re.match(r'#\s*endif', d):
                    nest -= 1
                j = e + 1
                if nest <= 0 or j >= n:
                    break
            cur += src[i:j]
            i = j
            end()
            continue
        if src.startswith('//', i):
            e = src.find('\n', i)
            e = n if e < 0 else e
            cur += src[i:e]
            i = e
            continue
        if src.startswith('/*', i):
            e = src.find('*/', i + 2) + 2
            cur += src[i:e]
            i = e
            continue
        m = re.match(r'R"([^(\s]*)\(', src[i:i + 20]) if c == 'R' and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == '_')) else None
        if m:
            e = src.find(')' + m[1] + '"', i) + len(m[1]) + 2
            cur += src[i:e]
            i = e
            continue
        if c in '"\'':
            j = i + 1
            while j < n and src[j] != c:
                j += 2 if src[j] == '\\' else 1
            cur += src[i:j + 1]
            i = j + 1
            continue
        cur += c
        i += 1
        if c in '([':
            depth += 1
        elif c in ')]':
            depth -= 1
        elif c == '{':
            if depth == 0:
                head = re.sub(r'//[^\n]*|/\*.*?\*/', '', cur, flags=re.S)
                body = ')' in head and not re.match(TAG, head) or re.match(r'\s*(namespace\b|extern\s*"C")', head) is not None
            depth += 1
        elif c == '}':
            depth -= 1
            if depth == 0 and body:
                k = i
                while k < n and src[k] in ' \t':
                    k += 1
                if k < n and src[k] == ';':
                    cur += src[i:k + 1]
                    i = k + 1
                end()
        elif c == ';' and depth == 0 and not re.match(KR, re.sub(r'//[^\n]*|/\*.*?\*/', '', cur, flags=re.S)):
            end()
    cells.append(cur)
    # comments and blank lines go with the next declaration
    out, pend = [], ''
    for c in cells:
        if re.sub(r'//[^\n]*|/\*.*?\*/', '', c, flags=re.S).strip() == '':
            pend += c
        else:
            out.append(pend + c)
            pend = ''
    if pend.strip() and out:
        out[-1] += pend
    return out


# --- running the cells

def ld(obj, so, cxx):
    r = subprocess.run([ENG, 'ld', '-shared', '--experimental-pic', '--unresolved-symbols=import-dynamic', obj, f'-L{LIB}'] +
                       ['-lc++', '-lc++abi'] * cxx + ['-lsetjmp', '-lc', f'{RES}/lib/wasm32-unknown-wasip1/libclang_rt.builtins.a',
                                                     '-o', so], capture_output=True, text=True)
    return r.returncode == 0, r.stderr


def run(sos):
    try:
        x = subprocess.run([SCLOAD, '-L', LIB] + sos, input=b'linha1\nlinha2\n', capture_output=True, timeout=20)
        return x.stdout.decode(errors='replace') + x.stderr.decode(errors='replace') + f'run exit={x.returncode}\n'
    except subprocess.TimeoutExpired:
        return 'TIMEOUT\n'


class SessionA:
    """a session process of candidate A"""

    def __init__(self, fl, cxx, log):
        self.p = subprocess.Popen([ENG, 'spike-a'] + fl + ['-x', 'c++' if cxx else 'c'], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=log, text=True)

    def send(self, cmd):
        """the answer: ('ok', names) | ('error', None) | ('lost', None)"""
        try:
            self.p.stdin.write(cmd + '\n')
            self.p.stdin.flush()
            a = self.p.stdout.readline()
        except BrokenPipeError:
            a = ''
        if not a:  # the process died
            self.p.wait()
            return 'lost', None
        w = a.split()
        return w[0], w[1:] if w[0] == 'ok' else None

    def hwm(self):
        """peak RSS of the session, in KB"""
        try:
            return int(re.search(r'VmHWM:\s+(\d+)', open(f'/proc/{self.p.pid}/status').read())[1])
        except (OSError, TypeError):
            return 0

    def close(self):
        if self.p.poll() is None:
            self.p.stdin.write('quit\n')
            self.p.stdin.close()
            self.p.wait()


def cand_a(files, d, fl, cxx, crash_at=None):
    """runs the cells through one session; with crash_at, a fatal cell before
    that index, then a new session and the replay. Returns per cell (status,
    names, object), the time of each cell, the peak RSS and the replay check"""
    log = open(f'{d}/a.log', 'w')
    s = SessionA(fl, cxx, log)
    res, times, replay_ok, hwm, history = [], [], None, 0, []
    for k, f in enumerate(files):
        if k == crash_at:
            open(f'{d}/crash.c', 'w').write(CRASH)
            st, _ = s.send(f'cell {d}/crash.c {d}/crash.o')
            assert st == 'lost', st
            s = SessionA(fl, cxx, log)
            t = time.time()
            replay_ok = all(s.send(f'replay {h}') == (r[0], r[1]) for h, r in zip(history, res))
            res_replay_time = time.time() - t
        obj = f'{f}.a.o'
        t = time.time()
        st, names = s.send(f'cell {f} {obj}')
        times.append(time.time() - t)
        hwm = max(hwm, s.hwm())
        res.append((st, names, obj))
        history.append(f)
        if st == 'lost':
            break
    hwm = max(hwm, s.hwm())
    s.close()
    log.close()
    return res, times, hwm, replay_ok


def cand_b(files, d, fl):
    res, times, rss = [], [], 0
    known, prev = f'{d}/known', None
    open(known, 'w').close()
    log = open(f'{d}/b.log', 'w')
    for k, f in enumerate(files):
        pch, obj = f'{f}.pch', f'{f}.b.o'
        t = time.time()
        p = subprocess.Popen([ENG, 'spike-b'] + fl + (['-include-pch', prev] if prev else []) +
                             ['-emit-pch', pch, '-known', known, '-o', obj, f], stdout=subprocess.PIPE, stderr=log, text=True)
        out = p.stdout.read()
        _, status, ru = os.wait4(p.pid, 0)
        times.append(time.time() - t)
        rss = max(rss, ru.ru_maxrss)
        code = os.waitstatus_to_exitcode(status)
        if code:  # stateless: the cell leaves nothing behind
            res.append(('error' if code == 1 else 'lost', None, obj))
            continue
        names = out.split()
        open(known, 'a').write(''.join(x + '\n' for x in names))
        res.append(('ok', names, obj))
        prev = pch
    log.close()
    return res, times, rss


def link_run(res, d, cxx, tag):
    """links each object as a shared object and runs them; None if a cell failed"""
    if any(st != 'ok' for st, _, _ in res):
        return None
    sos = []
    for i, (_, _, obj) in enumerate(res):
        so = f'{d}/{tag}{i + 1}.so'
        ok, err = ld(obj, so, cxx)
        if not ok:
            return 'ld: ' + err
        sos.append(so)
    return run(sos)


def expected(name):
    """the run output of the program compiled whole (pic-), None if it does not compile"""
    txt = open(f'{BASE}/pic-{name}.txt').read()
    m = re.search(r'^so [0-9a-f]{32}\n', txt, re.M)
    return txt[m.end():] if m else None


def same(out, exp):
    """the output of the cells is the program's, up to a trap's backtrace"""
    return out is None if exp is None else out is not None and norm_hash(out) == norm_hash(exp)


def one(prog):
    p, name = prog
    cxx, mod = p.endswith('.cpp'), '/test/mod/' in p
    d = f'{OUT}/{name}'
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    ext = os.path.splitext(p)[1]
    cells = split(open(p).read())
    files = []
    for i, c in enumerate(cells):
        f = f'{d}/c{i + 1:03}{ext}'
        open(f, 'w').write(c)
        files.append(f)
    fl = flags(cxx, mod)
    exp = expected(name)
    r = {'name': name, 'cells': len(cells), 'expected_compiles': exp is not None}

    t = time.time()
    one_shot = subprocess.run([ENG, '-cc1'] + fl + ['-o', f'{d}/whole.o', p], capture_output=True)
    r['whole_s'] = time.time() - t

    ra, ta, hwm, _ = cand_a(files, d, fl, cxx)
    out_a = link_run(ra, d, cxx, 'a')
    r['a'] = {'ok': same(out_a, exp),
              'status': [x[0] for x in ra], 'times': ta, 'rss_kb': hwm}
    if not same(out_a, exp):
        open(f'{d}/a.out', 'w').write(str(out_a))

    rb, tb, rss = cand_b(files, d, fl)
    out_b = link_run(rb, d, cxx, 'b')
    r['b'] = {'ok': same(out_b, exp),
              'status': [x[0] for x in rb], 'times': tb, 'rss_kb': rss}
    if not same(out_b, exp):
        open(f'{d}/b.out', 'w').write(str(out_b))

    if exp is not None and len(files) > 2:  # the crash and the replay
        k = len(files) // 2
        rc, _, _, replay_ok = cand_a(files, d, fl, cxx, crash_at=k)
        out_c = link_run(rc, d, cxx, 'c')
        names = [x[:2] for x in rc] == [x[:2] for x in ra]
        r['crash'] = {'ok': bool(replay_ok) and names and same(out_c, exp), 'replay': replay_ok, 'same_names': names,
                      'out': same(out_c, exp)}
    return r


def main():
    os.makedirs(OUT, exist_ok=True)
    progs = [(p, os.path.splitext(os.path.basename(p))[0]) for p in sorted(os.listdir(f'{R}/test/samples'))]
    progs = [(f'{R}/test/samples/{p}', n) for p, n in progs]
    for sub, pre in [('cxx', 'cxx-'), ('mod', 'mod-')]:
        progs += [(f'{R}/test/{sub}/{p}', pre + os.path.splitext(p)[0]) for p in sorted(os.listdir(f'{R}/test/{sub}'))]
    if os.environ.get('ONLY'):  # ONLY=p1,cxx-c3_stl
        progs = [x for x in progs if x[1] in os.environ['ONLY'].split(',')]
    if any('/test/mod/' in p for p, _ in progs):  # the std module, as corpus.py builds it
        src = [l for l in open(f'{SR}/share/libc++/v1/std.cppm') if 'csignal' not in l]
        open(f'{OUT}/std.cppm', 'w').writelines(src)
        fl = [{'-emit-obj': '-emit-module-interface'}.get(x, x) for x in flags(True, False)]
        subprocess.check_call([ENG, '-cc1'] + fl + ['-fmodules-reduced-bmi', '-I', f'{SR}/share/libc++/v1',
                               '-Wno-reserved-module-identifier', '-o', f'{OUT}/std.pcm', f'{OUT}/std.cppm'])
    with ThreadPoolExecutor(int(os.environ.get('JOBS', '1'))) as ex:
        rs = list(ex.map(one, progs))
    json.dump(rs, open(f'{OUT}/report.json', 'w'), indent=1)
    for r in rs:
        a, b, c = r['a'], r['b'], r.get('crash')
        print(f"{r['name']:24} {r['cells']:3} cells  A {'ok ' if a['ok'] else 'BAD'} {sum(a['times']):5.2f}s {a['rss_kb'] // 1024:4}MB"
              f"  B {'ok ' if b['ok'] else 'BAD'} {sum(b['times']):5.2f}s {b['rss_kb'] // 1024:4}MB"
              f"  whole {r['whole_s']:4.2f}s  crash {'-' if c is None else 'ok' if c['ok'] else 'BAD ' + str(c)}")
    print('A', sum(r['a']['ok'] for r in rs), '/', len(rs), ' B', sum(r['b']['ok'] for r in rs), '/', len(rs),
          ' crash', sum(r['crash']['ok'] for r in rs if 'crash' in r), '/', sum('crash' in r for r in rs))


if __name__ == '__main__':
    main()
