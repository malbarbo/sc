"""The corpus's programs as REPL cells, for test/split.py and test/session.py.

The programs of test/samples, test/cxx and test/mod are cut into one cell
per top-level declaration (a preprocessor directive, a declaration ending
in ';', a function or namespace body), and the output of the cells must be
the one of the program compiled whole (pic-, from the one-shot sc.wasm's
base, build/res/base). Phase 0 of PLANO-REPL.md chose, with these cells,
one session over chained PCHs (ANALISE.md §22).
"""
import os, re, subprocess, sys
sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
from corpuslib import norm_hash

R = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
sys.path.insert(0, f'{R}/scripts')
from flags import CXXFLAGS, iface, pic, real  # the engine's flags
B = f'{R}/build'
SR = f'{B}/sysroot'
LIB = f'{SR}/lib/wasm32-wasip1'
SCLOAD = os.environ.get('SCLOAD', f'{B}/cargo/release/scload')  # test/node/scload: the JS loader; scload-rs, sc.wasm's
BASE = f'{B}/res/base'
# on V8, with the JS loader (test/node/scload, PLANO-REPL.md §5.5) or sc.wasm's
# (test/node/scload-rs), the programs whose trap V8 says less of than wasmtime
JS = {
    'ct-00187': 'a call through a null slot: V8 says "null function or function signature mismatch",'
                ' wasmtime "uninitialized element"',
    'gcc-ts_gcc_testsuite_gcc.c-torture_execute_pr23135': 'out of bounds: wasmtime says the address and the size'
                                                         ' of the memory too',
} if SCLOAD.endswith(('/node/scload', '/node/scload-rs')) else {}


def std_pcm(out):
    """<out>/std.pcm, libc++'s std module as corpus.py builds it, by the
    one-shot sc.wasm under wasmtime"""
    src = [l for l in open(f'{SR}/share/libc++/v1/std.cppm') if 'csignal' not in l]
    open(f'{out}/std.cppm', 'w').writelines(src)
    # the loader's primitives (src/loader/host.rs), which the one-shot engine never calls
    subprocess.check_call(['wasmtime', 'run', '-W', 'unknown-imports-trap=y', '--dir', '/', f'{B}/sc.wasm', '-cc1']
                          + real(iface(pic(CXXFLAGS))[1:], B) + ['-o', f'{out}/std.pcm', f'{out}/std.cppm'])


# --- one cell per top-level declaration

COMMENT = re.compile(r'//[^\n]*|/\*.*?\*/', re.S)
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
                head = COMMENT.sub('', cur)
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
        elif c == ';' and depth == 0 and not re.match(KR, COMMENT.sub('', cur)):
            end()
    cells.append(cur)
    # comments and blank lines go with the next declaration
    out, pend = [], ''
    for c in cells:
        if COMMENT.sub('', c).strip() == '':
            pend += c
        else:
            out.append(pend + c)
            pend = ''
    if pend.strip() and out:
        out[-1] += pend
    return out


def run(sos):
    try:
        x = subprocess.run([SCLOAD, '-L', LIB] + sos, input=b'linha1\nlinha2\n', capture_output=True, timeout=20)
        return x.stdout.decode(errors='replace') + x.stderr.decode(errors='replace') + f'run exit={x.returncode}\n'
    except subprocess.TimeoutExpired:
        return 'TIMEOUT\n'


def expected(name):
    """the run output of the program compiled whole (pic-), None if it does not compile"""
    txt = open(f'{BASE}/pic-{name}.txt').read()
    m = re.search(r'^so [0-9a-f]{32}\n', txt, re.M)
    return txt[m.end():] if m else None


def same(out, exp):
    """the output of the cells is the program's, up to a trap's backtrace"""
    return out is None if exp is None else out is not None and norm_hash(out) == norm_hash(exp)
