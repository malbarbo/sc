#!/usr/bin/env python3
"""Link the sc engine from the LLVM wasm build, applying the cuts in cuts/.

usage: link.py [--full | --learn] <out.wasm>
  (default)  cut link: deterministic, from cuts/objects.txt, cuts/symbols.txt
             and cuts/symbols.used
  --full     no cuts and no trims: the reference engine for the corpus
  --learn    also run the corpus and rewrite what the cuts keep: objects that
             trap are appended to cuts/objects.txt as '!' lines, and
             cuts/symbols.used is rebuilt from a traced run

Two kinds of cut:
  * objects (cuts/objects.txt, one regex per line over member paths such as
    tools/clang/lib/Sema/CMakeFiles/obj.clangSema.dir/SemaOpenMP.cpp.obj;
    '!' keeps): the archives are rebuilt without them and every function
    left undefined gets a trapping stub;
  * functions (cuts/symbols.txt): each global function selected is
    redirected with wasm-ld --wrap to a trapping __wrap_ stub, so its body is
    dropped by --gc-sections. Functions in cuts/symbols.used (the ones the
    corpus calls) are left alone. Rules:
      obj:<re>      functions defined in objects whose path matches
      sym:<re>      functions whose demangled name matches
      !<re>         never wrap demangled names matching this
      noop:<re>     wrap with an empty body instead (void functions only)
      noopkeep:<f>  demangled names (one per line) that noop: must not touch
"""
import json, os, re, shlex, subprocess, sys

R = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
sys.path.insert(0, f'{R}/test')
import corpuslib

B = os.environ.get('SC_BUILD', f'{R}/build')
WB, W = f'{B}/wasm', f'{B}/wasi-sdk-29.0-x86_64-linux'
CUTS = f'{R}/cuts'
args = sys.argv[1:]
mode = args.pop(0)[2:] if args and args[0] in ('--full', '--learn') else 'cut'
out = os.path.realpath(args[0])
d = f'{B}/link-{mode}'
os.makedirs(f'{d}/lib', exist_ok=True)
env = dict(os.environ)
env['PATH'] = f'{B}/tools:' + ':'.join(x for x in env['PATH'].split(':') if '.local/bin' not in x)
log = open(f'{d}/link.log', 'w')


def say(msg):
    print(msg, flush=True)
    log.write(msg + '\n')
    log.flush()


def sh(cmd, **kw):
    r = subprocess.run(cmd, shell=isinstance(cmd, str), cwd=WB, capture_output=True, text=True, env=env, **kw)
    if r.returncode:
        sys.exit(f'{cmd if isinstance(cmd, str) else shlex.join(cmd)}\n{r.stderr[-3000:]}')
    return r


def ninja_cmd(target):
    return sh(['ninja', '-C', WB, '-t', 'commands', target]).stdout.splitlines()[-1]


# --- the link line: clang's, with the engine objects in place of the driver
ENGINE = [f'{B}/engine/{f}.o' for f in ('sc_main', 'sc_inv', 'sc_backend', 'sc_cuts')]
toks = shlex.split(ninja_cmd('bin/clang-23').split(': && ', 1)[1].removesuffix(' && :'))
flags = [t for t in toks if not t.endswith(('.a', '.obj')) and t not in ('-o', 'bin/clang-23', '-Os')]
archives = [t for t in shlex.split(ninja_cmd('bin/lld')) if t.startswith('lib/liblld') and t.endswith('.a')]
archives = list(dict.fromkeys(archives + [t for t in toks if t.endswith('.a')]))

# archive members, from the ninja rule that builds each archive
mfile = f'{B}/link-members.json'
members = json.load(open(mfile)) if os.path.exists(mfile) else {}
for a in archives:
    if a not in members:
        members[a] = shlex.split(ninja_cmd(a).split(' qc ', 1)[1].split(' && ')[0])[1:]
json.dump(members, open(mfile, 'w'), indent=0)

# trimmed tablegen/data objects (scripts/trim.py) replace their members
TRIMS = {} if mode == 'full' else {
    'lib/IR/CMakeFiles/LLVMCore.dir/Intrinsics.cpp.obj': f'{B}/trim/Intrinsics.cpp.obj',
    'lib/Analysis/CMakeFiles/LLVMAnalysis.dir/TargetLibraryInfo.cpp.obj': f'{B}/trim/TargetLibraryInfo.cpp.obj',
}


def rules(path):
    for l in open(path):
        l = l.split('#', 1)[0].strip()
        if l:
            yield l


# --- object cuts
rm, keep = [], []
if mode != 'full':
    for l in rules(f'{CUTS}/objects.txt'):
        (keep if l.startswith('!') else rm).append(re.compile(l.lstrip('!')))


def removed():
    return {o for a in archives for o in members[a]
            if any(r.search(o) for r in rm) and not any(k.search(o) for k in keep)}


def import_sigs(wasm, prefix=''):
    """(name, (params, results)) of the functions a module imports from env"""
    txt = subprocess.check_output(['wasm-tools', 'print', '--skeleton', wasm], text=True)
    types = {}
    for m in re.finditer(r'\(type \(;(\d+);\) \(func([^\n]*)\)\)\n', txt):
        ps = re.findall(r'\(param ([^)]*)\)', m.group(2))
        rs = re.findall(r'\(result ([^)]*)\)', m.group(2))
        types[m.group(1)] = (' '.join(ps).split(), ' '.join(rs).split())
    return [(m.group(1), types[m.group(2)]) for m in re.finditer(
        r'\(import "env" "(' + prefix + r'[^"]+)" \(func (?:\$(?:"[^"]*"|\S+) )?\(;\d+;\) \(type (\d+)\)\)\)', txt)]


def assemble(name, body):
    open(f'{d}/{name}.s', 'w').write(body)
    sh([f'{W}/bin/clang', '--target=wasm32-wasip1', '-c', f'{d}/{name}.s', '-o', f'{d}/{name}.o'])
    return f'{d}/{name}.o'


def trap_stubs(name, sigs):
    return assemble(name, ''.join(
        f'.globl {s}\n.type {s},@function\n{s}:\n  .functype {s} ({", ".join(ps)}) -> ({", ".join(rs)})\n'
        f'  unreachable\n  end_function\n\n' for s, (ps, rs) in sigs))


def link_objects():
    """archives without the removed objects; returns the base link command"""
    gone = removed()
    libs = []
    for a in archives:
        objs = [TRIMS.get(o, o) for o in members[a] if o not in gone]
        if objs == members[a]:
            libs.append(f'{WB}/{a}')
            continue
        na = f'{d}/{a}'
        if os.path.exists(na):
            os.remove(na)
        sh([f'{W}/bin/llvm-ar', 'qc', na] + objs)
        libs.append(na)
    # wasm-ld 21 (wasi-sdk 29) crashes on --wrap + --allow-undefined; use the one we built
    return shlex.join(flags + ENGINE + libs + [f'-fuse-ld={B}/native/bin/wasm-ld']), len(gone)


def link_with_stubs(base, extra, out_, name):
    """link; functions still undefined become trapping stubs <name>.o"""
    sh(f'{base} {extra} -Wl,--allow-undefined -o {d}/imports.wasm')
    stubs = import_sigs(f'{d}/imports.wasm')
    os.remove(f'{d}/imports.wasm')
    o = trap_stubs(name, stubs)
    cmd = f'{base} {extra} {o} -Wl,--warn-unresolved-symbols -Wl,-Map={d}/engine.map -o {out_}'
    open(f'{d}/link.sh', 'w').write(f'cd {WB} && {cmd}\n')
    sh(cmd)
    return o, len(stubs)


baseline = f'{B}/res/base/summary.json'


def corpus(wasm, res):
    subprocess.check_call([f'{R}/test/corpus.py', wasm, res], stdout=subprocess.DEVNULL)
    return corpuslib.diff(json.load(open(baseline)), json.load(open(f'{res}/summary.json')))


def traps(res, bad, prefix):
    txt = ''.join(open(f'{res}/{k}.txt').read() for k in bad)
    return set(re.findall(r'^\s+0: +0x[0-9a-f]+ - [^!\s]+!' + prefix + r'(.*)$', txt, re.M))


# --- stage 1: object cuts
base, ngone = link_objects()
if mode == 'learn':
    for it in range(25):
        stubs1, n = link_with_stubs(base, '', f'{d}/engine.wasm', 'stubs')
        bad = corpus(f'{d}/engine.wasm', f'{d}/res')
        say(f'objects {it}: removed={ngone} stubs={n} corpus={"OK" if not bad else f"DIFF {len(bad)}"}')
        if not bad:
            break
        frames = traps(f'{d}/res', bad, '')
        if not frames:
            sys.exit(f'corpus differs without a trap: {bad[:5]}')
        # the removed object that defines each trapping function goes back
        back = set()
        for o in sorted(removed()):
            p = TRIMS.get(o, f'{WB}/{o}')
            syms = sh([f'{W}/bin/llvm-nm', '-C', '--defined-only', '-g', '-j', p]).stdout.splitlines()
            if any(s.strip() in frames or s.split('(')[0] in {f.split('(')[0] for f in frames} for s in syms):
                back.add(o)
        if not back:
            sys.exit(f'trap not in a removed object: {sorted(frames)[:3]}')
        with open(f'{CUTS}/objects.txt', 'a') as f:
            f.write('# put back by link.py --learn\n' + ''.join(f'!{re.escape(o)}$\n' for o in sorted(back)))
        keep += [re.compile(re.escape(o) + '$') for o in back]
        say('   put back: ' + ', '.join(sorted(x.split('.dir/')[-1] for x in back)))
        base, ngone = link_objects()
else:
    stubs1, n = link_with_stubs(base, '', f'{d}/engine.wasm', 'stubs')
    say(f'objects: removed={ngone} stubs={n}')

if mode == 'full':
    os.replace(f'{d}/engine.wasm', out)
    sys.exit()

# --- stage 2: function cuts over what stage 1 linked
base += f' {stubs1} -Wl,--warn-unresolved-symbols'
objre, symre, symkeep, noopre, noopkeep = [], [], [], [], set()
for l in rules(f'{CUTS}/symbols.txt'):
    if l.startswith('!'): symkeep.append(re.compile(l[1:]))
    elif l.startswith('obj:'): objre.append(re.compile(l[4:]))
    elif l.startswith('sym:'): symre.append(re.compile(l[4:]))
    elif l.startswith('noop:'): noopre.append(re.compile(l[5:]))
    elif l.startswith('noopkeep:'): noopkeep |= set(x.strip() for x in open(f'{CUTS}/{l[9:]}'))

# linked functions (demangled) and the member defining each, from the map
bymember = {}
for a, ms in members.items():
    for o in ms:
        bymember.setdefault((a.split('/')[-1], o.split('/')[-1]), o)
line_re = re.compile(r'^\s*(\S+)\s+([0-9a-f]+)\s+([0-9a-f]+) (\s*)(\S.*)$')
sec = cur = None
linked = {}
for l in open(f'{d}/engine.map'):
    m = line_re.match(l)
    if not m: continue
    ind, rest = len(m.group(4)), m.group(5)
    if ind == 0: sec = rest.split()[0]; continue
    if sec != 'CODE': continue
    if ind == 8:
        mm = re.match(r'(.*?)(?:\((.*?)\))?:\((.*)\)$', rest)
        cur = None
        if mm and mm.group(2):
            cur = bymember.get((mm.group(1).split('/')[-1], mm.group(2)))
        elif mm:
            cur = mm.group(1)
    elif ind == 16 and cur:
        linked.setdefault(rest, cur)

# global functions of the selected objects (mangled, demangled)
nmfile = f'{B}/link-nm.json'
nm = json.load(open(nmfile)) if os.path.exists(nmfile) else {}
objs = {o for o in linked.values() if o.endswith('.obj') and any(r.search(o) for r in objre)}
objs |= {o for s, o in linked.items() if o.endswith('.obj') and any(r.search(s) for r in symre + noopre)}
for o in sorted(objs - nm.keys()):
    p = o if o.startswith('/') else f'{WB}/{o}'
    r = sh([f'{W}/bin/llvm-nm', '--defined-only', '-g', p])
    # only strong definitions: --wrap of a weak one leaves __real_ undefined
    mangled = [x.split()[-1] for x in r.stdout.splitlines() if x.split()[1:2] == ['T']]
    dem = subprocess.run(['llvm-cxxfilt'], input='\n'.join(mangled), capture_output=True, text=True).stdout.splitlines()
    nm[o] = list(zip(mangled, dem))
json.dump(nm, open(nmfile, 'w'))
noops, cands = {}, {}
for o in sorted(objs):
    for mg, dm in nm[o]:
        if dm not in linked or mg.startswith('__wrap_'):
            continue
        if any(r.search(dm) for r in noopre) and dm not in noopkeep:
            noops[mg] = dm
        elif (any(r.search(o) for r in objre) or any(r.search(dm) for r in symre)) and not any(k.search(dm) for k in symkeep):
            cands[mg] = dm
say(f'functions: {len(cands)} candidates in {len(objs)} objects, {len(noops)} no-ops')

o = assemble('noops', ''.join(f'.globl __wrap_{s}\n.type __wrap_{s},@function\n__wrap_{s}:\n'
                              f'  .functype __wrap_{s} (i32) -> ()\n  end_function\n\n' for s in sorted(noops)))
open(f'{d}/noops.rsp', 'w').write(''.join(f'--wrap={s}\n' for s in sorted(noops)) + f'{o}\n')
base += f' -Wl,@{d}/noops.rsp'

used_file = f'{CUTS}/symbols.used'
used = set(rules(used_file)) if os.path.exists(used_file) and mode != 'learn' else set()


def write_used():
    open(used_file, 'w').write('# functions of cuts/symbols.txt that the corpus calls (link.py --learn)\n'
                               + ''.join(f'{s}\n' for s in sorted(used)))


if mode == 'learn':
    # discovery: every wrapper records its first call and forwards to the original.
    # The originals (__real_) pull in more objects, which may reference more
    # wrapped functions: relink with the stubs so far until none is missing.
    open(f'{d}/wrap.rsp', 'w').write(''.join(f'--wrap={s}\n' for s in sorted(cands)))
    stubs = []

    def trace_objs():
        body = ''
        for i, (s, (ps, rs)) in enumerate(stubs):
            real = '__real_' + s[len('__wrap_'):]
            body += (f'.functype sc_trace (i32) -> ()\n.functype {real} ({", ".join(ps)}) -> ({", ".join(rs)})\n'
                     f'.globl {s}\n.type {s},@function\n{s}:\n  .functype {s} ({", ".join(ps)}) -> ({", ".join(rs)})\n'
                     f'  i32.const {i}\n  call sc_trace\n'
                     + ''.join(f'  local.get {k}\n' for k in range(len(ps))) + f'  call {real}\n  end_function\n\n')
        t = assemble('tstubs', body)
        open(f'{d}/trace.c', 'w').write(
            f'#include <stdio.h>\nstatic unsigned char seen[{len(stubs) + 1}];\n'
            'void sc_trace(int i) { if (!seen[i]) { seen[i] = 1; fprintf(stderr, "SCWRAP %d\\n", i); } }\n')
        sh([f'{W}/bin/clang', '--target=wasm32-wasip1', '-O2', '-c', f'{d}/trace.c', '-o', f'{d}/trace.o'])
        return f'{t} {d}/trace.o'

    while True:
        extra = f'-Wl,@{d}/wrap.rsp {trace_objs()}'
        sh(f'{base} {extra} -Wl,--allow-undefined -o {d}/imports.wasm')
        more = import_sigs(f'{d}/imports.wasm', '__wrap_')
        if not more:
            break
        stubs += more
    os.remove(f'{d}/imports.wasm')
    sh(f'{base} {extra} -o {d}/trace.wasm')
    subprocess.check_call([f'{R}/test/corpus.py', f'{d}/trace.wasm', f'{d}/tres'], stdout=subprocess.DEVNULL)
    hit = set()
    for f in os.listdir(f'{d}/tres'):
        if f.endswith('.txt'):
            hit |= {int(x) for x in re.findall(r'^SCWRAP (\d+)$', open(f'{d}/tres/{f}').read(), re.M)}
    used = {stubs[i][0][len('__wrap_'):] for i in hit}
    write_used()
    say(f'discovery: {len(stubs)} wrappers, {len(used)} called by the corpus')

for it in range(40):
    wrap = sorted(s for s in cands if s not in used)
    open(f'{d}/wrap.rsp', 'w').write(''.join(f'--wrap={s}\n' for s in wrap))
    _, n = link_with_stubs(base, f'-Wl,@{d}/wrap.rsp', f'{d}/engine.wasm', 'wstubs')
    if mode != 'learn':
        say(f'functions: wrapped={len(wrap)} stubs={n}')
        break
    bad = corpus(f'{d}/engine.wasm', f'{d}/res')
    say(f'functions {it}: wrapped={len(wrap)} stubs={n} corpus={"OK" if not bad else f"DIFF {len(bad)}"}')
    if not bad:
        break
    frames = traps(f'{d}/res', bad, '__wrap_')
    if not frames:
        sys.exit(f'corpus differs without a __wrap_ trap: {bad[:5]}')
    used |= frames
    write_used()
os.replace(f'{d}/engine.wasm', out)
