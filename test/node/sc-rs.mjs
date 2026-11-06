// sc inside sc.wasm on Node: the machine (src/machine.rs, its export
// sc_cli in wasm/lib.rs), with the session in the same module and the
// host's primitives of wasm.mjs, which is all the JS here gives it. For
// test/student.py and test/checks.py (SC=test/node/sc-rs).
//
// usage: node sc-rs.mjs [-q] [-x LANG] [--safety LEVEL] [FILE [ARG]...]
// (SC_WASM: the module, build/sc.wasm)
import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { WASI } from 'node:wasi';
import { wasmApi } from './wasm.mjs';

const R = path.resolve(import.meta.dirname, '../..');
const WASM = process.env.SC_WASM ?? `${R}/build/sc.wasm`;
const args = process.argv.slice(2);
// the program sees the file and what follows it, as sc gives them
const file = args.findIndex((a, i) => !a.startsWith('-') && !['-x', '--safety'].includes(args[i - 1]));
const argv = file < 0 ? ['sc'] : args.slice(file);

const api = wasmApi(() => new WASI({ version: 'preview1', args: argv, env: process.env, preopens: { '/': '/' } }));
// sc.wasm reads the file: it sees the whole file system, from the directory
const wasi = new WASI({ version: 'preview1', args: ['sc', ...args], env: {}, preopens: { '/': '/' } });
const inst = new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync(WASM)),
  { wasi_snapshot_preview1: wasi.wasiImport, env: api.env });
api.bind(inst);
// sc.wasm exports _start too (the one-shot engine), which initialize refuses
wasi.initialize({ exports: { memory: inst.exports.memory, _initialize: inst.exports._initialize } });
const x = inst.exports;
const cwd = new TextEncoder().encode(process.cwd() + '\0');
const p = x.malloc(cwd.length);
new Uint8Array(x.memory.buffer).set(cwd, p);
x.chdir(p);
x.free(p);
process.exit(x.sc_cli());
