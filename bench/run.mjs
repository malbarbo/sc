import { readFile } from 'node:fs/promises';
import { WASI } from 'node:wasi';
const f = process.argv[2];
const wasi = new WASI({ version: 'preview1', args: [f], env: {} });
const m = await WebAssembly.compile(await readFile(f));
const inst = await WebAssembly.instantiate(m, wasi.getImportObject());
const t = performance.now(); wasi.start(inst); console.error(`${f}: ${(performance.now() - t).toFixed(0)} ms`);
