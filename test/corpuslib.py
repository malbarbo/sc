"""Normalized results of the corpus (test/corpus.py).

usage: corpuslib.py <outdir>   recompute <outdir>/summary.json from its .txt files
"""
import hashlib, json, os, re, sys

# tests whose result is not a function of the compiler's behaviour
# (order of pointer-keyed maps, stack overflow in the baseline)
IGNORE = {'src_clang_test_C_C99_n590', 'src_clang_test_CodeGen_alias'}


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
    return hashlib.md5('\n'.join(sorted(norm.splitlines())).encode()).hexdigest()


def diff(base, got):
    return [k for k in base if k not in IGNORE and got.get(k) != base[k]]


if __name__ == '__main__':
    d = sys.argv[1]
    res = {f[:-4]: norm_hash(open(f'{d}/{f}').read()) for f in os.listdir(d) if f.endswith('.txt')}
    json.dump(res, open(f'{d}/summary.json', 'w'), indent=0, sort_keys=True)
