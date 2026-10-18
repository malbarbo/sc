// Time clang.wasm (cc1) in V8: module compile, then N compiles of a C file.
import { readFile } from 'node:fs/promises';
import { WASI } from 'node:wasi';
const [wasmPath, src, n = '3'] = process.argv.slice(2);
const R = new URL('..', import.meta.url).pathname, B = `${R}build`, W = `${B}/wasi-sdk-29.0-x86_64-linux`;
let t = performance.now();
const mod = await WebAssembly.compile(await readFile(wasmPath));
console.error(`compile module: ${(performance.now() - t).toFixed(0)} ms`);
for (let i = 0; i < +n; i++) {
  const args = ['clang', '-cc1', '-triple', 'wasm32-unknown-wasip1', '-emit-obj', '-Wall',
    '-isysroot', '/sysroot', '-internal-isystem', '/res/include',
    '-internal-externc-isystem', '/sysroot/include/wasm32-wasip1', '-internal-externc-isystem', '/sysroot/include',
    '-o', '-', `/c/${src}`];
  const wasi = new WASI({ version: 'preview1', args, env: {}, returnOnExit: true,
    preopens: { '/c': `${R}test/samples`, '/sysroot': `${W}/share/wasi-sysroot`, '/res': `${W}/lib/clang/21`, '/dev': '/dev' } });
  t = performance.now();
  const inst = await WebAssembly.instantiate(mod, wasi.getImportObject());
  // sc.wasm is a reactor with a _start, which wasi.start refuses: _start
  // through an object without _initialize, which it calls first
  const e = inst.exports;
  const rc = e._initialize ? wasi.start({ exports: { memory: e.memory, _start() { e._initialize(); e._start(); } } })
    : wasi.start(inst);
  console.error(`run ${i}: rc=${rc} ${(performance.now() - t).toFixed(0)} ms`);
}
