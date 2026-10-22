// The REPL's loader in JS (PLANO-REPL.md §5.5): src/loader.rs over the JS
// API of WebAssembly, for Node and, later, a browser's worker. It loads
// wasm shared objects the same way: one memory, one table and the stack
// pointer for all modules, dylink.0 read from the bytes, GOT imports as
// mutable globals set once the group is loaded, a stub that calls through
// the table for a function not defined yet, and one exception tag for all
// modules. See the comment at the top of src/loader.rs.
//
// What the JS API does not give, the bytes do: the type of an import (no
// type reflection), and whether an exported global is a constant. A
// function that goes into the table must be a wasm one, so the stubs and
// the "undefined symbol" functions are small modules made here. Not here
// yet: ASan's runtime (libsc_rt) and the file and line of a frame (DWARF).
import fs from 'node:fs';
import path from 'node:path';

export const STACK = 1 << 20;
const PAGE = 1 << 16;
const UNDEFINED = 0xC000_0000; // as in src/loader.rs: a variable not defined yet
const UNDEFINED_SIZE = 1 << 20;
const INTERNAL = new Set(['__wasm_call_ctors', '__wasm_apply_data_relocs', '_initialize', '__wasm_init_memory']);
const WEAK = 1; // BINDING_WEAK of dylink.0's flags
const enc = new TextEncoder();
const dec = new TextDecoder();

// --- reading a module

const VAL = { 0x7f: 'i32', 0x7e: 'i64', 0x7d: 'f32', 0x7c: 'f64', 0x7b: 'v128', 0x70: 'anyfunc', 0x6f: 'externref', 0x69: 'exnref' };

class Reader {
  constructor(b, at = 0, end = b.length) {
    Object.assign(this, { b, at, end });
  }
  byte() {
    return this.b[this.at++];
  }
  u32() {
    let r = 0, s = 0, x;
    do {
      x = this.byte();
      r |= (x & 0x7f) << s;
      s += 7;
    } while (x & 0x80);
    return r >>> 0;
  }
  name() {
    const n = this.u32();
    return dec.decode(this.b.subarray(this.at, (this.at += n)));
  }
  val() {
    const t = VAL[this.byte()];
    if (!t) throw new Error(`a value type of byte 0x${this.b[this.at - 1].toString(16)}`);
    return t;
  }
  vec(f) {
    return Array.from({ length: this.u32() }, () => f());
  }
  /// skips a constant expression
  expr() {
    for (let op; (op = this.byte()) !== 0x0b;) {
      if (op === 0x41 || op === 0x42 || op === 0x23 || op === 0xd2) this.u32(); // the s(33|64)leb of a const skips as one
      else if (op === 0x43) this.at += 4;
      else if (op === 0x44) this.at += 8;
      else if (op === 0xd0) this.byte();
      else if (op < 0x6a || op > 0x6c) throw new Error(`a constant expression with 0x${op.toString(16)}`);
    }
  }
  limits() {
    const f = this.byte();
    this.u32();
    if (f & 1) this.u32();
  }
}

/// What the loader needs of a module: the types of its function and tag
/// imports, the mutability of its globals, and its dylink.0.
function parse(bytes) {
  const r = new Reader(bytes, 8);
  const m = { types: [], imports: [], globals: [], exports: new Map(), dylink: null };
  while (r.at < r.end) {
    const id = r.byte(), size = r.u32(), end = r.at + size;
    const s = new Reader(bytes, r.at, end);
    if (id === 0) {
      if (s.name() === 'dylink.0') m.dylink = dylink(s);
    } else if (id === 1) {
      m.types = s.vec(() => {
        if (s.byte() !== 0x60) throw new Error('a type other than a function');
        return { params: s.vec(() => s.val()), results: s.vec(() => s.val()) };
      });
    } else if (id === 2) {
      m.imports = s.vec(() => {
        const i = { module: s.name(), name: s.name(), kind: s.byte() };
        if (i.kind === 0) i.type = m.types[s.u32()];
        else if (i.kind === 1) s.byte(), s.limits();
        else if (i.kind === 2) s.limits();
        else if (i.kind === 3) s.val(), m.globals.push(s.byte() === 1);
        else if (i.kind === 4) s.byte(), (i.type = m.types[s.u32()]);
        return i;
      });
    } else if (id === 6) {
      for (let n = s.u32(); n--;) {
        s.val();
        m.globals.push(s.byte() === 1);
        s.expr();
      }
    } else if (id === 7) {
      for (let n = s.u32(); n--;) {
        const name = s.name(), kind = s.byte(), index = s.u32();
        m.exports.set(name, { kind, index });
      }
    }
    r.at = end;
  }
  if (!m.dylink) throw new Error('no dylink.0 section: not a shared object');
  return m;
}

function dylink(s) {
  const d = { memSize: 0, memAlign: 0, tableSize: 0, tableAlign: 0, needed: [], weak: new Set(), weakDef: new Set() };
  while (s.at < s.end) {
    const id = s.byte(), end = s.u32() + s.at;
    if (id === 1) [d.memSize, d.memAlign, d.tableSize, d.tableAlign] = [s.u32(), s.u32(), s.u32(), s.u32()];
    else if (id === 2) d.needed = s.vec(() => s.name());
    else if (id === 3) s.vec(() => [s.name(), s.u32()]).forEach(([n, f]) => f & WEAK && d.weakDef.add(n));
    else if (id === 4) s.vec(() => [s.name(), s.name(), s.u32()]).forEach(([, n, f]) => f & WEAK && d.weak.add(n));
    s.at = end;
  }
  return d;
}

// --- writing a small module

const CODE = Object.fromEntries(Object.entries(VAL).map(([k, v]) => [v, +k]));

function leb(x) {
  const out = [];
  do {
    let b = x & 0x7f;
    x >>>= 7;
    if (x) b |= 0x80;
    out.push(b);
  } while (x);
  return out;
}

function sleb(x) {
  const out = [];
  for (;;) {
    const b = x & 0x7f;
    x >>= 7;
    if ((x === 0 && !(b & 0x40)) || (x === -1 && b & 0x40)) return out.push(b), out;
    out.push(b | 0x80);
  }
}

const str = (s) => [...leb(enc.encode(s).length), ...enc.encode(s)];
const vec = (xs) => [...leb(xs.length), ...xs.flat()];
const section = (id, xs) => (xs.length ? [id, ...leb(vec(xs).length), ...vec(xs)] : []);

/// A module of functions: `funcs` are [type, body], `imports` [module,
/// name, descriptor bytes], each function k is exported as f<k>.
function module({ types = [], imports = [], funcs = [] }) {
  const nimp = imports.filter((i) => i[2][0] === 0).length;
  const bytes = [
    0, 0x61, 0x73, 0x6d, 1, 0, 0, 0,
    ...section(1, types.map((t) => [0x60, ...vec(t.params.map((v) => [CODE[v]])), ...vec(t.results.map((v) => [CODE[v]]))])),
    ...section(2, imports.map(([m, n, d]) => [...str(m), ...str(n), ...d])),
    ...section(3, funcs.map(([t]) => leb(t))),
    ...section(7, funcs.map((_, k) => [...str(`f${k}`), 0, ...leb(nimp + k)])),
    ...section(10, funcs.map(([, body]) => {
      const b = [0, ...body, 0x0b];
      return [...leb(b.length), ...b];
    })),
  ];
  return new WebAssembly.Module(new Uint8Array(bytes));
}

const params = (t) => t.params.flatMap((_, i) => [0x20, ...leb(i)]);
const TABLE = [1, 0x70, 0, 0]; // an import of a funcref table, from 0

// --- the loader

/// libc's exit, which the loader turns into a trap (Loader.failed)
export class Exit extends Error {
  constructor(code) {
    super(`exit ${code}`);
    this.code = code;
  }
}

export class Loader {
  /// wasi: a node:wasi WASI (its proc_exit is replaced); libs(name): the
  /// bytes of a library, or null.
  constructor(wasi, libs, stack = STACK) {
    this.memory = new WebAssembly.Memory({ initial: Math.ceil(stack / PAGE), maximum: UNDEFINED / PAGE });
    this.table = new WebAssembly.Table({ element: 'anyfunc', initial: 1 });
    this.sp = new WebAssembly.Global({ value: 'i32', mutable: true }, stack);
    this.libs = libs;
    this.syms = new Map(); // name: { kind: func, data or tag, v, slot }
    this.loaded = new Set();
    this.bump = stack;
    this.later = []; // GOT.mem imports of symbols not defined yet: [name, global]
    this.undefined = [];
    this.slots = new Map();
    this.failed = null; // an Exit, or the Error of a trap the loader made: wasm code cannot catch a trap
    const trap = new WebAssembly.Instance(module({ types: [{ params: [], results: [] }], funcs: [[0, [0x00]]] }));
    this.trap = trap.exports.f0;
    const proc_exit = (code) => {
      this.failed = new Exit(code);
      this.trap();
    };
    wasi.initialize({ exports: { memory: this.memory } });
    this.wasi = { ...wasi.wasiImport, proc_exit };
    this.define('__stack_low', { kind: 'data', v: 16 }, false);
    this.define('__stack_high', { kind: 'data', v: stack }, false);
  }

  define(name, sym, shadow) {
    if (shadow || !this.syms.has(name)) this.syms.set(name, { ...sym, slot: undefined });
  }

  defines(name) {
    return this.syms.get(name)?.kind === 'func';
  }

  /// `__wrap_name` when a module defines it, `name` for `__real_name`
  target(name) {
    if (name.startsWith('__real_')) return name.slice(7);
    return this.syms.has(`__wrap_${name}`) ? `__wrap_${name}` : name;
  }

  call(name, ...args) {
    const s = this.syms.get(name);
    if (s?.kind !== 'func') throw new Error(`undefined symbol: ${name}`);
    // JS would drop the arguments a function does not take
    if (s.v.length !== args.length) throw new Error(`expected ${s.v.length} arguments, got ${args.length}`);
    return s.v(...args);
  }

  collect(name, bytes, seen, out) {
    if (this.loaded.has(name) || seen.has(name)) return;
    seen.add(name);
    let info;
    try {
      info = parse(bytes);
    } catch (e) {
      throw new Error(`${name}: ${e.message}`);
    }
    for (const n of info.dylink.needed) {
      if (this.loaded.has(n) || seen.has(n)) continue;
      const lib = this.libs(n);
      if (!lib) throw new Error(`${n}: not found (use -L)`);
      this.collect(n, lib, seen, out);
    }
    out.push({ name, module: new WebAssembly.Module(bytes), info });
  }

  alloc(size, align) {
    if (this.bump === null) {
      const p = this.call('aligned_alloc', align, alignUp(Math.max(size, 1), align)) >>> 0;
      if (!p) throw new Error('out of memory loading a module');
      new Uint8Array(this.memory.buffer, p, size).fill(0);
      return p;
    }
    const p = alignUp(this.bump, align);
    this.bump = p + size;
    this.reserve(p + size);
    return p;
  }

  reserve(end) {
    const have = this.memory.buffer.byteLength;
    if (end > have) this.memory.grow(Math.ceil((end - have) / PAGE));
  }

  undefinedAddress(name) {
    let k = this.undefined.indexOf(name);
    if (k < 0) {
      if (this.undefined.length >= (2 ** 32 - UNDEFINED) / UNDEFINED_SIZE) return 0;
      k = this.undefined.push(name) - 1;
    }
    return UNDEFINED + k * UNDEFINED_SIZE;
  }

  /// A function of type t that says `name` is not defined and traps.
  missing(name, t) {
    const fail = () => {
      this.failed = new Error(`undefined symbol: ${name}`);
    };
    const m = module({ types: [{ params: [], results: [] }, t], imports: [['env', 'fail', [0, 0]]], funcs: [[1, [0x10, 0, 0x00]]] });
    return new WebAssembly.Instance(m, { env: { fail } }).exports.f0;
  }

  reserveSlot(name, t) {
    const s = this.slots.get(name);
    if (s !== undefined) {
      // reserved by an address (GOT.func), which left it null
      if (t && this.table.get(s) === null) this.table.set(s, this.missing(name, t));
      return s;
    }
    const k = this.table.grow(1, t ? this.missing(name, t) : null);
    this.slots.set(name, k);
    return k;
  }

  /// stubs for the function imports of p with no definition yet: each
  /// calls through the symbol's table slot
  stubs(p) {
    const need = p.info.imports.filter((i) => i.module === 'env' && i.kind === 0 && !this.defines(this.target(i.name)));
    if (!need.length) return new Map();
    const funcs = need.map((i, k) => {
      const slot = this.reserveSlot(this.target(i.name), i.type);
      return [k, [...params(i.type), 0x41, ...sleb(slot), 0x11, ...leb(k), 0]];
    });
    const m = module({ types: need.map((i) => i.type), imports: [['env', 't', TABLE]], funcs });
    const x = new WebAssembly.Instance(m, { env: { t: this.table } }).exports;
    return new Map(need.map((i, k) => [i.name, x[`f${k}`]]));
  }

  slot(name) {
    const s = this.syms.get(name);
    if (s.slot === undefined) {
      if (s.kind !== 'func') throw new Error(`${name}: GOT.func of data`);
      s.slot = this.table.grow(1, s.v);
    }
    return s.slot;
  }

  /// what the modules before referred to and the group just loaded defines
  forward() {
    for (const [n, s] of this.slots) {
      const sym = this.syms.get(n);
      if (sym?.kind === 'func' && sym.slot === undefined) {
        this.table.set(s, sym.v);
        sym.slot = s;
        this.slots.delete(n);
      }
    }
    this.later = this.later.filter(([n, g]) => {
      const sym = this.syms.get(n);
      if (sym?.kind !== 'data') return true;
      g.value = sym.v;
      return false;
    });
  }

  load(file, shadow) {
    this.loadBytes(file, fs.readFileSync(file), shadow);
  }

  /// the module `name` and what it needs, with their constructors run.
  /// shadow: its definitions replace the older ones (a REPL entry)
  loadBytes(name, bytes, shadow) {
    const group = [];
    this.collect(name, bytes, new Set(), group);
    if (group.some((p) => p.info.exports.has('__asan_init'))) throw new Error("ASan's runtime: not in the JS loader yet");
    const got = []; // name, global, func?, weak?
    const insts = [];
    const konst = (v) => new WebAssembly.Global({ value: 'i32', mutable: false }, v);
    for (const p of group) {
      const sh = shadow && p.name === name;
      const d = p.info.dylink;
      const memBase = this.alloc(d.memSize, 2 ** d.memAlign);
      const size = this.table.length;
      const tableBase = alignUp(size, 2 ** d.tableAlign);
      this.table.grow(tableBase + d.tableSize - size);
      const stubs = this.stubs(p);
      const imports = {};
      for (const i of p.info.imports) {
        const [m, n] = [i.module, i.name];
        let v;
        if (m === 'env' && n === 'memory') v = this.memory;
        else if (m === 'env' && n === '__indirect_function_table') v = this.table;
        else if (m === 'env' && n === '__stack_pointer') v = this.sp;
        else if (m === 'env' && n === '__memory_base') v = konst(memBase);
        else if (m === 'env' && n === '__table_base') v = konst(tableBase);
        else if ((m === 'GOT.mem' || m === 'GOT.func') && i.kind === 3) {
          v = new WebAssembly.Global({ value: 'i32', mutable: true }, 0);
          got.push([this.target(n), v, m === 'GOT.func', d.weak.has(n)]);
        } else if (m === 'wasi_snapshot_preview1' && this.wasi[n]) v = this.wasi[n];
        else if (m === 'env' && i.kind === 0) {
          const s = this.syms.get(this.target(n));
          v = s?.kind === 'func' ? s.v : stubs.get(n);
        } else if (m === 'env' && i.kind === 4) {
          const s = this.syms.get(n);
          if (s?.kind === 'tag') v = s.v;
          else {
            v = new WebAssembly.Tag({ parameters: i.type.params });
            this.define(n, { kind: 'tag', v }, false);
          }
        } else throw new Error(`${p.name}: unsupported import ${m}.${n}`);
        (imports[m] ??= {})[n] = v;
      }
      let inst;
      try {
        inst = new WebAssembly.Instance(p.module, imports);
      } catch (e) {
        throw new Error(`${p.name}: ${e.message}`);
      }
      for (const [n, e] of Object.entries(inst.exports)) {
        if (INTERNAL.has(n)) continue;
        // an inline function, a vtable or a typeinfo is one for all modules
        const s = sh && !d.weakDef.has(n);
        if (typeof e === 'function') this.define(n, { kind: 'func', v: e }, s);
        else if (e instanceof WebAssembly.Tag) this.define(n, { kind: 'tag', v: e }, false);
        else if (e instanceof WebAssembly.Global && !p.info.globals[p.info.exports.get(n).index]) {
          this.define(n, { kind: 'data', v: (memBase + e.value) >>> 0 }, s);
        }
      }
      insts.push(inst);
    }
    this.forward();
    if (this.bump !== null) {
      // the heap starts after the first group and has the rest of its page
      const base = alignUp(this.bump, 16);
      this.reserve(base + 1);
      this.define('__heap_base', { kind: 'data', v: base }, false);
      this.define('__heap_end', { kind: 'data', v: this.memory.buffer.byteLength }, false);
      this.bump = null;
    }
    for (const [n, g, func, weak] of got) {
      const s = this.syms.get(n);
      let v;
      if (s?.kind === 'data' && !func) v = s.v;
      else if (s?.kind === 'func' && func) v = this.slot(n);
      else if (s) throw new Error(`${n}: GOT.${func ? 'func' : 'mem'} of a ${func ? 'data' : 'function'}`);
      else if (weak) v = 0;
      else if (func) v = this.reserveSlot(n, null);
      else {
        this.later.push([n, g]);
        v = this.undefinedAddress(n);
      }
      g.value = v | 0;
    }
    group.forEach((p, k) => {
      this.loaded.add(p.name);
      const x = insts[k].exports;
      x.__wasm_apply_data_relocs?.();
      (x._initialize ?? x.__wasm_call_ctors)?.();
    });
  }

  /// main, then libc's exit
  run() {
    let ret;
    if (this.syms.has('__main_argc_argv')) ret = this.call('__main_void');
    else if (this.syms.has('__original_main')) ret = this.call('__original_main');
    else if (this.syms.has('main')) ret = this.call('main', 0, 0);
    this.call('exit', ret ?? 0);
    return ret ?? 0;
  }

  /// a C++ exception no one caught, as on a native target: catch it and
  /// terminate, which prints its type and what() and aborts
  uncaught(e) {
    const cpp = this.syms.get('__cpp_exception');
    if (cpp?.kind === 'tag' && e.is(cpp.v)) {
      this.call('__cxa_begin_catch', e.getArg(cpp.v, 0));
      this.call('_ZSt9terminatev');
    }
    throw e;
  }
}

function alignUp(x, a) {
  return Math.ceil(x / a) * a;
}

/// the libraries in dirs, from the first one that has each
export function libdirs(dirs) {
  return (lib) => {
    for (const d of dirs) {
      try {
        return fs.readFileSync(path.join(d, lib));
      } catch {}
    }
    return null;
  };
}
