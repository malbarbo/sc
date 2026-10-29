// scload inside sc.wasm on Node: sc.wasm's loader (src/loader.rs over
// src/loader/host.rs, its export sc_scload) with the host's primitives of
// wasm.mjs, which is all the JS here gives it. As scload.mjs, for
// test/split.py and test/session.py (SCLOAD=test/node/scload-rs).
//
// usage: node scload-rs.mjs [-L dir]... [--dir host::guest]... [--stack bytes] first.so [entry.so]... [-- args...]
// (SC_WASM: the module, build/sc.wasm)
import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { WASI } from 'node:wasi';
import { wasmApi } from './wasm.mjs';

const R = path.resolve(import.meta.dirname, '../..');
const WASM = process.env.SC_WASM ?? `${R}/build/sc.wasm`;
const USAGE = 'usage: scload [-L dir]... [--dir host::guest]... [--stack bytes] first.so [entry.so]... [-- args...]';
// the loader's arguments, and the program's: its own, its directories
const [loader, preopens, args] = [[], {}, []];
let first = null;
for (let a = process.argv.slice(2), i = 0; i < a.length; i++) {
  if (a[i] === '-L') loader.push('-L', path.resolve(a[++i]));
  else if (a[i] === '--dir') {
    const [h, g] = a[++i].split('::');
    preopens[g ?? h] = h;
  } else if (a[i] === '--stack') loader.push('--stack', a[++i]);
  else if (a[i] === '--') args.push(...a.slice(i + 1)), (i = a.length);
  else if (a[i].startsWith('-L')) loader.push('-L', path.resolve(a[i].slice(2)));
  else loader.push(path.resolve(a[i])), (first ??= a[i]);
}
if (first === null) {
  console.error(USAGE);
  process.exit(2);
}

const api = wasmApi(() => new WASI({ version: 'preview1', args: [first, ...args], env: process.env, preopens }));
// sc.wasm reads the modules from the files: it sees the whole file system
const wasi = new WASI({ version: 'preview1', args: ['scload', ...loader], env: {}, preopens: { '/': '/' } });
const inst = new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync(WASM)),
  { wasi_snapshot_preview1: wasi.wasiImport, env: api.env });
api.bind(inst);
// sc.wasm exports _start too (the one-shot engine), which initialize refuses
wasi.initialize({ exports: { memory: inst.exports.memory, _initialize: inst.exports._initialize } });
process.exit(inst.exports.sc_scload());
