#!/usr/bin/env python3
"""What the REPL's session calls of what the cuts remove, docs/plan-repl.md
phase 4: rewrites cuts/symbols.session.used (demangled names, which
scripts/link.py keeps).

usage: learn_session.py   (make learn-session, after make learn)

From the list as it is (empty it to learn it again from nothing), the cut
sc.wasm with its names (link.py, as sc-cut.wasm) is sc's compiler
(SC_CWASM, src/engine/guest.rs), which runs the session's slices; each
function it calls that the cuts removed says its name (frame 0 of the trap,
`__wrap_`), goes in the list (or its object goes back, in
cuts/objects.txt), and the slices that differ run again until they pass,
then all of them again. The slices:
  test/session.py ses-, seq- and crash-, with -Weverything, against the
  uncut engine's (build/res/session-wev-base);
  test/split.py, against pic-.
"""
import json, os, re, shutil, subprocess, sys, time

R = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
sys.path.insert(0, f'{R}/test')
from corpuslib import IGNORE, norm_hash

B = f'{R}/build'
D = f'{B}/link-session-learn'
ENG = f'{B}/cargo/release/sc'  # its compiler is D/engine.cwasm (SC_CWASM)
FILE = f'{R}/cuts/symbols.session.used'
BASE = json.load(open(f'{B}/res/session-wev-base/summary.json'))
BASE = {k: v for k, v in BASE.items() if k.split('-', 1)[1] not in IGNORE}
os.makedirs(D, exist_ok=True)
log = open(f'{D}/learn.log', 'w')


def say(msg):
    print(msg, flush=True)
    log.write(msg + '\n')
    log.flush()


def write(names):
    txt = ('# functions of cuts/symbols.txt that only the REPL\'s session calls, demangled\n'
           '# (scripts/learn_session.py), on top of symbols.used\n'
           + ''.join(f'{s}\n' for s in sorted(names)))
    # the same list, untouched: the engines that depend on it are not relinked
    if not os.path.exists(FILE) or open(FILE).read() != txt:
        open(FILE, 'w').write(txt)


def link():
    """the cut engine with the session, D/engine.cwasm"""
    subprocess.check_call([f'{R}/scripts/link.py', f'{D}/engine.wasm'], env=dict(os.environ, SC_LINKDIR=D),
                          stdout=subprocess.DEVNULL)
    subprocess.check_call([ENG, '--precompile', f'{D}/engine.wasm', f'{D}/engine.cwasm'])


def slice_run(cmd, out, only, env={}):
    """runs a slice's script on the names only (None: all); the names of
    its results"""
    if only == []:
        return []
    if only is None:
        shutil.rmtree(out, ignore_errors=True)
    e = dict(os.environ, SC_CWASM=f'{D}/engine.cwasm', **env)
    if only:
        open(f'{out}.only', 'w').write(''.join(f'{k}\n' for k in only))
        e['ONLY'] = f'{out}.only'
    return subprocess.run(cmd + [ENG, out], env=e, capture_output=True, text=True).stdout


def ses(only):
    """the results of ses-, seq- and crash- that differ from the base"""
    out = f'{D}/ses'
    slice_run([f'{R}/test/session.py'], out, only, {'WFLAGS': '-Weverything', 'SLICES': 'ses,seq,crash'})
    names = BASE if only is None else only
    return [k for k in names if not os.path.exists(f'{out}/{k}.txt') or norm_hash(open(f'{out}/{k}.txt').read()) != BASE[k]]


def split(only):
    """the programs of split- that differ from pic-"""
    return re.findall(r'^DIFF (\S+)', slice_run([f'{R}/test/split.py'], f'{D}/split', only) or '', re.M)


def traps(out, bad):
    t = set()
    for k in bad:
        if os.path.exists(f'{out}/{k}.txt'):
            t |= set(re.findall(r'^sc: cut function called: (\S+)$', open(f'{out}/{k}.txt', errors='replace').read(), re.M))
    return t


def put_back(syms):
    """the cut objects that define syms go back (cuts/objects.txt), as
    link.py --learn does on the wasm engine; their paths"""
    rm, keep = [], []
    for l in open(f'{R}/cuts/objects.txt'):
        l = l.split('#', 1)[0].strip()
        if l:
            (keep if l.startswith('!') else rm).append(re.compile(l.lstrip('!')))
    back = []
    for p in (o for ms in json.load(open(f'{B}/link-members.json')).values() for o in ms):
        if any(r.search(p) for r in rm) and not any(k.search(p) for k in keep):
            nm = subprocess.run(['llvm-nm', '--defined-only', '-g', '-j', f'{B}/wasm/{p}'], capture_output=True, text=True)
            if syms & set(nm.stdout.split()):
                back.append(p)
    if not back:
        sys.exit(f'not in a cut object: {sorted(syms)[:5]}')
    with open(f'{R}/cuts/objects.txt', 'a') as f:
        f.write('# put back by learn_session.py\n' + ''.join(f'!{re.escape(p)}$\n' for p in back))
    return back


names = {l.strip() for l in open(FILE) if l.strip() and not l.startswith('#')} if os.path.exists(FILE) else set()
write(names)
start = time.time()
only_ses = only_split = None
for it in range(100):
    link()
    bad_ses, bad_split = ses(only_ses), split(only_split)
    say(f'{it}: {len(names)} functions; ses {"all" if only_ses is None else len(only_ses)}: {len(bad_ses)} differ, '
        f'split {"all" if only_split is None else len(only_split)}: {len(bad_split)} differ')
    if not bad_ses and not bad_split:
        if only_ses is None and only_split is None:
            break
        only_ses = only_split = None
        continue
    t = traps(f'{D}/ses', bad_ses) | traps(f'{D}/split', bad_split)
    if not t:
        sys.exit(f'differ without a cut function: {(bad_ses + bad_split)[:10]}')
    objs = {x for x in t if not x.startswith('__wrap_')}
    if objs:
        back = put_back(objs)
        say('   put back: ' + ', '.join(x.split('.dir/')[-1] for x in back))
        t -= objs
    dem = subprocess.run(['llvm-cxxfilt'], input='\n'.join(x[len('__wrap_'):] for x in sorted(t)),
                         capture_output=True, text=True).stdout.splitlines() if t else []
    names |= set(dem)
    write(names)
    say('   ' + '\n   '.join(sorted(dem)))
    only_ses, only_split = bad_ses, bad_split
else:
    sys.exit('no end after 100 rounds')
say(f'{len(names)} functions, {(time.time() - start) / 60:.0f} min')
