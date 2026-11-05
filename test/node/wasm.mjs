// The host's primitives for wasm (ANALISE.md §64) over the JS API of
// WebAssembly: the imports `env.wasm_*` of a module that loads others, as
// sc.wasm's loader does (src/loader/host.rs, which says what each does).
// Nothing here knows sc: a host such as simplecode's gives the same.
//
// Handles are numbers from 1, never reused, until wasm_drop; the instance 0
// is the module bound, the one that imports the primitives: a call of its
// own export through wasm_call comes back with what stopped it, as any
// other. A module's import of the default (0) is WASI on the memory it
// imports (on the `memory` it exports if it imports none, bound after the
// instantiation), `env.check_interrupt` (0 here) or `env.wasm_stack`. WASI's
// proc_exit and a trap the host makes stop the code with a trap of a module
// of its own, which wasm code cannot catch (a JS exception would run the
// C++ cleanups); the call says EXIT.
//
// usage: const api = wasmApi(memory => new WASI(...)); instantiate the
// module with { env: api.env }, then api.bind(instance).

const [OK, TRAP, EXIT, ERROR] = [0, 1, 3, 5];
const enc = new TextEncoder();
const dec = new TextDecoder();

// the code of a trap (host.rs's kind) by V8's message
const TRAPS = [
  [/^Maximum call stack size exceeded/, 2],
  [/^memory access out of bounds/, 3],
  [/^(divide|remainder) by zero/, 4],
  [/^(integer overflow|divide result unrepresentable)/, 5],
  [/^float unrepresentable/, 6],
  [/^null function or function signature mismatch/, 7],
  [/^table index is out of bounds/, 9],
  [/^unreachable/, 10],
];

/// wasi(memory): a node:wasi WASI for a module on that memory, not
/// initialized
export function wasmApi(wasi) {
  const handles = [null];
  let engine = null; // the memory of the module that imports the primitives
  let self = null; // and its instance
  let error = '';
  let frames = ''; // of the last trap
  let exit = null; // the code of a proc_exit
  const stacks = [];
  const ids = new Map();
  const wasis = new Map(); // memory: its WASI functions
  const unreachable = new WebAssembly.Instance(new WebAssembly.Module(new Uint8Array([
    0, 0x61, 0x73, 0x6d, 1, 0, 0, 0, 1, 4, 1, 0x60, 0, 0, 3, 2, 1, 0, 7, 5, 1, 1, 0x66, 0, 0, 10, 5, 1, 3, 0, 0, 0x0b,
  ]))).exports.f;

  const put = (x) => handles.push(x) - 1;
  const get = (h) => (h > 0 ? handles[h] : undefined);
  const view = () => new DataView(engine.buffer);
  const bytes = (p, n) => new Uint8Array(engine.buffer, p, n);
  const str = (p, n) => dec.decode(bytes(p, n));
  /// writes text to buf, cap bytes at most; its whole length
  const text = (t, buf, cap) => {
    const b = enc.encode(t);
    bytes(buf, Math.min(cap, b.length)).set(b.subarray(0, cap));
    return b.length;
  };

  /// the wasm frames of a JS stack, as host.rs reads them. V8 writes a
  /// function as `<module>.<name>`, the module's name from its name section
  /// (in the URL, before `-<hash>`), and the name demangled, with spaces
  function wasmFrames(stack) {
    const re = /^\s*at (?:(.+) \()?wasm:\/\/wasm\/([^:]*):wasm-function\[\d+\]:0x([0-9a-f]+)\)?$/gm;
    return [...(stack ?? '').matchAll(re)].map(([, name, url, off]) => {
      const mod = /^(.*)-[0-9a-f]+$/.exec(url)?.[1];
      if (name && mod !== undefined && name.startsWith(`${mod}.`)) name = name.slice(mod.length + 1);
      return `${off} ${name ?? ''}\n`;
    }).join('');
  }

  /// the status and detail of what a call or an instantiation threw
  function failure(e) {
    if (exit !== null) {
      const c = exit;
      exit = null;
      return [EXIT, c];
    }
    error = e?.message ?? String(e);
    frames = wasmFrames(e?.stack);
    if (e instanceof WebAssembly.RuntimeError || (e instanceof RangeError && /^Maximum call stack/.test(error)))
      return [TRAP, TRAPS.find(([re]) => re.test(error))?.[1] ?? 0];
    if (e instanceof WebAssembly.Exception) {
      error = 'uncaught wasm exception';
      return [TRAP, 0];
    }
    return [ERROR, 0];
  }

  function wasiOn(memory) {
    let w = wasis.get(memory);
    if (!w) {
      const x = wasi(memory);
      x.initialize({ exports: { memory } });
      const proc_exit = (code) => {
        exit = code;
        unreachable();
      };
      w = { ...x.wasiImport, proc_exit };
      wasis.set(memory, w);
    }
    return w;
  }

  function wasm_stack() {
    const limit = Error.stackTraceLimit;
    Error.stackTraceLimit = 1000;
    const key = wasmFrames(new Error().stack);
    Error.stackTraceLimit = limit;
    let id = ids.get(key);
    if (id === undefined) {
      id = stacks.push(key);
      ids.set(key, id);
    }
    return id;
  }

  const env = {
    wasm_instantiate(p, len, slots, n, out) {
      let module;
      try {
        module = new WebAssembly.Module(bytes(p, len).slice());
      } catch (e) {
        error = e.message;
        return ERROR;
      }
      const decl = WebAssembly.Module.imports(module);
      if (decl.length !== n) {
        error = `${n} imports for ${decl.length}`;
        return ERROR;
      }
      const v = view();
      const slot = (k) => [v.getUint32(slots + 8 * k, true), v.getInt32(slots + 8 * k + 4, true)];
      let memory = decl.map((d, k) => d.kind === 'memory' && slot(k)[0] === 1 && get(slot(k)[1])).find((m) => m);
      let own = null; // the instance, whose memory a WASI of its own reads
      const lazy = (name) => (...a) => wasiOn(own.exports.memory)[name](...a);
      const imports = {};
      for (const [k, d] of decl.entries()) {
        const [kind, value] = slot(k);
        let x;
        if (kind === 0) {
          if (d.module === 'wasi_snapshot_preview1') x = memory ? wasiOn(memory)[d.name] : lazy(d.name);
          else if (d.module === 'env' && d.name === 'wasm_stack') x = wasm_stack;
          else if (d.module === 'env' && d.name === 'check_interrupt') x = () => 0;
        } else if (kind === 1) x = get(value);
        else {
          x = new WebAssembly.Global({ value: 'i32', mutable: kind === 3 }, value);
          v.setInt32(slots + 8 * k + 4, put(x), true);
        }
        if (x === undefined) {
          error = `${d.module}.${d.name}: ${kind ? `no handle ${value}` : 'no default'}`;
          return ERROR;
        }
        const m = (imports[d.module] ??= {});
        if (d.name in m && m[d.name] !== x) {
          error = `${d.module}.${d.name}: two values`;
          return ERROR;
        }
        m[d.name] = x;
      }
      try {
        own = new WebAssembly.Instance(module, imports);
        view().setUint32(out, put(own), true);
        return OK;
      } catch (e) {
        const [st, d] = failure(e);
        view().setUint32(out, d >>> 0, true);
        return st;
      }
    },

    wasm_export(inst, p, len) {
      const x = (inst === 0 ? self : get(inst))?.exports?.[str(p, len)];
      return x === undefined ? 0 : put(x);
    },

    wasm_call(func, sp, slen, vals) {
      const f = get(func);
      if (typeof f !== 'function') {
        error = `no function ${func}`;
        return ERROR;
      }
      const [ps, rs] = str(sp, slen).split(':');
      let v = view();
      const args = [...ps].map((c, k) => {
        const at = vals + 8 * k;
        if (c === 'i') return v.getInt32(at, true);
        if (c === 'j') return v.getBigInt64(at, true);
        if (c === 'f') return v.getFloat32(at, true);
        if (c === 'd') return v.getFloat64(at, true);
        return get(v.getUint32(at, true)) ?? null;
      });
      let r;
      try {
        r = f(...args);
      } catch (e) {
        const [st, d] = failure(e);
        view().setBigUint64(vals, BigInt(d >>> 0), true);
        return st;
      }
      v = view();
      [...(rs ?? '')].forEach((c, k) => {
        const x = rs.length > 1 ? r[k] : r;
        const at = vals + 8 * k;
        v.setBigUint64(at, 0n, true);
        if (c === 'i') v.setInt32(at, x, true);
        else if (c === 'j') v.setBigInt64(at, x, true);
        else if (c === 'f') v.setFloat32(at, x, true);
        else if (c === 'd') v.setFloat64(at, x, true);
        else v.setUint32(at, x === null ? 0 : put(x), true);
      });
      return OK;
    },

    wasm_error: (buf, cap) => text(error, buf, cap),
    wasm_frames: (id, buf, cap) => text(id ? (stacks[id - 1] ?? '') : frames, buf, cap),

    wasm_memory_read(mem, addr, dst, len) {
      const m = get(mem);
      if (!(m instanceof WebAssembly.Memory) || addr + len > m.buffer.byteLength) return -1;
      bytes(dst, len).set(new Uint8Array(m.buffer, addr, len));
      return 0;
    },

    wasm_drop(h) {
      if (h > 0) handles[h] = undefined;
    },

    wasm_memory_write(mem, addr, src, len) {
      const m = get(mem);
      if (!(m instanceof WebAssembly.Memory) || addr + len > m.buffer.byteLength) return -1;
      new Uint8Array(m.buffer, addr, len).set(bytes(src, len));
      return 0;
    },
  };

  // every argument is a pointer, a length, a handle or a number: unsigned
  for (const [k, f] of Object.entries(env)) env[k] = (...a) => f(...a.map((x) => x >>> 0));
  return {
    env,
    bind(instance) {
      engine = instance.exports.memory;
      self = instance;
    },
  };
}
