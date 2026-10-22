// scload on Node: runs wasm shared objects from files with the JS loader
// (loader.mjs), as src/bin/scload.rs does with src/loader.rs, and says what
// stops them as scload does, for test/split.py and test/session.py.
//
// usage: node scload.mjs [-L dir]... [--dir host::guest]... [--stack bytes] first.so [entry.so]... [-- args...]
import process from 'node:process';
import { WASI } from 'node:wasi';
import { Exit, Loader, STACK, libdirs } from './loader.mjs';

const USAGE = 'usage: scload [-L dir]... [--dir host::guest]... [--stack bytes] first.so [entry.so]... [-- args...]';
const [dirs, mods, preopens, args] = [[], [], {}, []];
let stack = STACK;
for (let a = process.argv.slice(2), i = 0; i < a.length; i++) {
  if (a[i] === '-L') dirs.push(a[++i]);
  else if (a[i] === '--dir') {
    const [h, g] = a[++i].split('::');
    preopens[g ?? h] = h;
  } else if (a[i] === '--stack') stack = +a[++i];
  else if (a[i] === '--') args.push(...a.slice(i + 1)), (i = a.length);
  else if (a[i].startsWith('-L')) dirs.push(a[i].slice(2));
  else mods.push(a[i]);
}
if (!mods.length) {
  console.error(USAGE);
  process.exit(2);
}

// wasmtime's words for a trap, which test/corpuslib.py's norm_hash keeps
const TRAPS = [
  [/^unreachable/, 'wasm `unreachable` instruction executed'],
  [/^memory access out of bounds/, 'out of bounds memory access'],
  [/^(divide|remainder) by zero/, 'integer divide by zero'],
  [/^(integer overflow|divide result unrepresentable)/, 'integer overflow'],
  [/^float unrepresentable/, 'invalid conversion to integer'],
  [/^table index is out of bounds/, 'undefined element: out of bounds table access'],
  [/^null function or function signature mismatch/, 'indirect call type mismatch'],
  [/^Maximum call stack size exceeded/, 'call stack exhausted'],
];

/// as wasmtime prints a trap: the frames, then the cause
function trap(e, cause) {
  const frames = [...(e.stack ?? '').matchAll(/^\s*at (?:(\S+) )?\(?wasm:\/\/wasm\/[^:]*:wasm-function\[(\d+)\]:(0x[0-9a-f]+)/gm)];
  const bt = frames.map((f, k) => `${String(k).padStart(5)}: ${f[3].padStart(8)} - ${f[1] ?? `<wasm function ${f[2]}>`}`);
  if (!cause) {
    const t = TRAPS.find(([re]) => re.test(e.message));
    cause = t ? `wasm trap: ${t[1]}` : e.message;
  }
  console.error(`scload: error while executing at wasm backtrace:\n${bt.join('\n')}\n\nCaused by:\n    ${cause}\n`);
  return 134;
}

const wasi = new WASI({ version: 'preview1', args: [mods[0], ...args], env: process.env, preopens });
let l;
function main() {
  l = new Loader(wasi, libdirs(dirs), stack);
  try {
    mods.forEach((m, i) => l.load(m, i > 0));
    return l.run();
  } catch (e) {
    if (e instanceof WebAssembly.Exception) l.uncaught(e);
    throw e;
  }
}

let code;
try {
  code = main();
} catch (e) {
  const f = l?.failed;
  if (f instanceof Exit) code = f.code;
  else if (f) code = trap(e, f.message);
  else if (e instanceof WebAssembly.RuntimeError || e instanceof RangeError) code = trap(e);
  else {
    console.error(`scload: ${e?.message ?? e}`);
    code = 134;
  }
}
process.exit(code);
