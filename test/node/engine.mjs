// `sc --engine` on Node: the child of the protocol (engine/sc.h) over
// sc.wasm's session, as src/engine/wasm.rs and serve (src/engine/session.rs)
// are in build/sc-wasm. The prototype of the JS host (PLANO-REPL.md §5.5):
// in place of the fork before a cell, a snapshot of the linear memory and
// of __stack_pointer, restored when the cell fails, is a type or traps.
//
// usage: node engine.mjs --engine (SC_WASM: the module, build/sc.wasm)
// The guest sees the repository's parent at its path, and the session the
// pack in the module at /sc (engine/sc_pack.cpp), which the snapshot leaves
// out: the session only reads it. The guest's stdout goes to stderr.
// SC_MEMORY_SIZE writes
// the memory's size after each cell, as wasm.rs does; SC_SNAPSHOT_LOG
// appends to that file, for each snapshot, the memory's size and the ms to
// copy and to restore it (or -).
import fs from 'node:fs';
import path from 'node:path';
import { WASI } from 'node:wasi';

const R = path.resolve(import.meta.dirname, '../..');
const ROOT = path.dirname(R);
const WASM = process.env.SC_WASM ?? `${R}/build/sc.wasm`;
const [OK, ERROR, LOST, INCOMPLETE] = [0, 1, 2, 3];
const [NEW, CELL, UNDO, REPLY, DEPTH] = [1, 2, 3, 4, 5];
const [SO, REPLAY, WHOLE, TYPE] = [0, 1, 2, 3];
const CRASHED = 'sc: the engine crashed on this cell; the session is as it was before it\n';
const enc = new TextEncoder();

const module = await WebAssembly.compile(fs.readFileSync(WASM));

// --- the protocol, on fds 0 and 1

function readFull(n) {
  const b = Buffer.alloc(n);
  for (let got = 0; got < n;) {
    let k;
    try {
      k = fs.readSync(0, b, got, n - got, null);
    } catch (e) {
      if (e.code === 'EAGAIN') continue;
      throw e;
    }
    if (k === 0) {
      if (got === 0) return null;
      throw new Error('the input ends in a message');
    }
    got += k;
  }
  return b;
}

/// kind, status, parts; null at the end of the input
function readMsg() {
  const h = readFull(20);
  if (!h) return null;
  const w = (i) => h.readUInt32LE(i * 4);
  const parts = [2, 3, 4].map((i) => (w(i) ? readFull(w(i)) : Buffer.alloc(0)));
  return { kind: w(0), status: w(1), parts };
}

function writeAll(b) {
  for (let off = 0; off < b.length;) {
    try {
      off += fs.writeSync(1, b, off);
    } catch (e) {
      if (e.code !== 'EAGAIN') throw e;
    }
  }
}

function reply(status, parts) {
  const ps = [0, 1, 2].map((i) => parts[i] ?? Buffer.alloc(0));
  const h = Buffer.alloc(20);
  [REPLY, status, ...ps.map((p) => p.length)].forEach((x, i) => h.writeUInt32LE(x, i * 4));
  writeAll(Buffer.concat([h, ...ps]));
}

// --- the guest

class Exit extends Error {
  constructor(code) {
    super(`the engine exited with ${code}`);
  }
}

class Guest {
  constructor() {
    const wasi = new WASI({
      version: 'preview1', args: ['sc'], env: {}, stdout: 2,
      preopens: { [ROOT]: ROOT },
    });
    // an exit of the guest (a fatal error) unwinds to the cell, as a trap
    const proc_exit = (code) => {
      throw new Exit(code);
    };
    const imports = { wasi_snapshot_preview1: { ...wasi.wasiImport, proc_exit } };
    this.inst = new WebAssembly.Instance(module, imports);
    this.x = this.inst.exports;
    // sc.wasm exports _start too (the one-shot engine), which initialize refuses
    wasi.initialize({ exports: { memory: this.x.memory, _initialize: this.x._initialize } });
    const root = this.put(enc.encode('/sc\0'));
    this.x.sc_pack(root);
    this.x.free(root);
    const size = this.put(new Uint8Array(4));
    const image = this.x.sc_pack_image(size);
    this.image = [image, image + new DataView(this.x.memory.buffer).getUint32(size, true)];
    this.x.free(size);
    // the session finds a cell's "..." includes from the directory
    const p = this.put(enc.encode(process.cwd() + '\0'));
    this.x.chdir(p);
    this.x.free(p);
  }

  bytes() {
    return new Uint8Array(this.x.memory.buffer);
  }

  /// bytes in the guest's memory, from its malloc
  put(b) {
    const p = this.x.malloc(Math.max(b.length, 1));
    if (!p) throw new Error("the guest's malloc failed");
    this.bytes().set(b, p);
    return p;
  }

  part(s, i) {
    const p = this.x.sc_result_ptr(s, i);
    return p ? Buffer.from(this.bytes().slice(p, p + this.x.sc_result_len(s, i))) : Buffer.alloc(0);
  }

  parts(s) {
    return [0, 1, 2].map((i) => this.part(s, i));
  }

  /// sc_session_new: the session, or the diagnostics
  session(args) {
    const ptrs = new Uint32Array(args.length + 1);
    args.forEach((a, i) => (ptrs[i] = this.put(enc.encode(a + '\0'))));
    const pv = this.put(new Uint8Array(ptrs.buffer));
    const pout = this.put(new Uint8Array(4));
    const st = this.x.sc_session_new(args.length, pv, pout);
    const s = new DataView(this.x.memory.buffer).getUint32(pout, true);
    this.x.free(pout);
    return st === OK ? { s } : { diags: this.part(s, 0) };
  }

  with(src, f) {
    const p = this.put(src);
    try {
      return f(p, src.length);
    } finally {
      this.x.free(p);
    }
  }

  /// the memory but the pack, and the stack pointer, in a buffer kept from
  /// one cell to the next: a new one each time costs its allocation too
  snapshot() {
    const t = performance.now();
    const m = this.bytes();
    const [a, b] = this.image;
    if (!this.buf || this.buf.length < m.length) this.buf = new Uint8Array(m.length);
    this.buf.set(m.subarray(0, a));
    this.buf.set(m.subarray(b), b);
    const snap = { mem: this.buf.subarray(0, m.length), sp: this.x.__stack_pointer.value };
    snap.ms = performance.now() - t;
    return snap;
  }

  /// back to the snapshot: what the memory grew since is zeroed, as it was
  restore(snap) {
    const t = performance.now();
    const m = this.bytes();
    const [a, b] = this.image;
    m.set(snap.mem.subarray(0, a));
    m.set(snap.mem.subarray(b), b);
    m.fill(0, snap.mem.length);
    this.x.__stack_pointer.value = snap.sp;
    return performance.now() - t;
  }
}

/// what a trap or an exit of the guest says, on stderr
function report(e) {
  const frame0 = /^\s*at (\S+) \(wasm:/m.exec(e?.stack ?? '')?.[1];
  if (frame0?.startsWith('__wrap_')) process.stderr.write(`sc: cut function called: ${frame0}\n`);
  process.stderr.write(`${e?.stack ?? e}\n`);
}

function log(snap, restored) {
  if (!process.env.SC_SNAPSHOT_LOG) return;
  const r = restored === undefined ? '-' : restored.toFixed(2);
  fs.appendFileSync(process.env.SC_SNAPSHOT_LOG, `${snap.mem.length} ${snap.ms.toFixed(2)} ${r}\n`);
}

let guest = null, session = null;
for (let m; (m = readMsg());) {
  if (m.kind === NEW) {
    session = null;
    const args = m.parts[0].toString().split('\0').slice(0, -1);
    try {
      guest = new Guest();
      const r = guest.session(args);
      if (r.diags) reply(ERROR, [r.diags]);
      else (session = r.s), reply(OK, []);
    } catch (e) {
      report(e);
      process.exit(134);
    }
  } else if ((m.kind === CELL || m.kind === UNDO) && session === null) {
    reply(ERROR, [Buffer.from('sc: no session\n')]);
  } else if (m.kind === CELL) {
    const mode = m.status;
    if (![SO, REPLAY, WHOLE, TYPE].includes(mode)) {
      reply(ERROR, [Buffer.from('sc: unknown cell mode\n')]);
      continue;
    }
    // a replayed cell is one that was accepted
    const snap = mode === REPLAY ? null : guest.snapshot();
    let st, parts;
    try {
      st = guest.with(m.parts[0], (p, n) => guest.x.sc_session_cell(session, p, n, mode));
      parts = guest.parts(session);
    } catch (e) {
      report(e);
      if (!snap) process.exit(134);
      log(snap, guest.restore(snap));
      reply(ERROR, [Buffer.from(CRASHED)]);
      continue;
    }
    if (snap) st = st === OK || st === INCOMPLETE ? st : ERROR;
    // what Sema made of the expression goes too
    const back = snap && (st !== OK || mode === TYPE);
    if (snap) log(snap, back ? guest.restore(snap) : undefined);
    if (process.env.SC_MEMORY_SIZE) process.stderr.write(`sc: memory.size ${guest.x.memory.buffer.byteLength}\n`);
    reply(st, parts);
  } else if (m.kind === UNDO) {
    const st = guest.x.sc_session_undo(session);
    reply(st, guest.parts(session));
  } else if (m.kind === DEPTH) {
    // the reply's status is the depth: with no session, nothing is open
    const d = session === null ? 0 : guest.with(m.parts[0], (p, n) => guest.x.sc_session_depth(session, p, n));
    reply(d, []);
  } else {
    reply(ERROR, [Buffer.from('sc: unknown message\n')]);
  }
}
