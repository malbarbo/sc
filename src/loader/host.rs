//! The loader's [`Runtime`] inside sc.wasm: the primitives are imports of
//! the host, `env.wasm_*` (ANALISE.md §64). test/node/wasm.mjs gives them on
//! Node, with the JS API of WebAssembly.
//!
//! - `wasm_instantiate(bytes, len, imports, n, out) -> status`: `imports`
//!   are `n` [`Slot`]s, one for each import of the module in its order; the
//!   host writes the handle of a global it made back into its slot, and the
//!   instance into `*out`;
//! - `wasm_export(inst, name, len) -> handle`, 0 if nothing; `inst` 0 is
//!   the module that imports the primitives (sc.wasm itself);
//! - `wasm_call(func, sig, sig_len, vals) -> status`, as [`Runtime::call`];
//! - `wasm_error(buf, cap) -> len`: the message of the last failure, or of
//!   the last trap; `len` is its whole length;
//! - `wasm_frames(id, buf, cap) -> len`: the frames of the stack that
//!   `env.wasm_stack` kept under `id`, or of the last trap with 0, a line
//!   each, innermost first: the offset in the module in hex (or `-`), a
//!   space, the function's name (or nothing);
//! - `wasm_memory_read(mem, addr, dst, len)` and `wasm_memory_write(mem,
//!   addr, src, len) -> 0 | -1`;
//! - `wasm_drop(handle)`: the host forgets it; a number is never reused.
//!
//! A status is [`OK`], or else what stopped the call, with its detail in
//! `vals[0]` (in `*out` for an instantiation): [`TRAP`] and its code
//! ([`kind`]), [`EXIT`] and the exit code, [`STOP`], [`ERROR`].

use super::runtime::{Error, Frame, Import, Result, Runtime, Trap, TrapKind, H};

/// An import of `wasm_instantiate`: `kind` 0 the host's default, 1 the
/// handle in `value`, 2 a constant i32 global of `value`, 3 a mutable one.
#[repr(C)]
struct Slot {
    kind: u32,
    value: i32,
}

pub const OK: i32 = 0;
pub const TRAP: i32 = 1;
pub const EXIT: i32 = 3;
pub const STOP: i32 = 4;
pub const ERROR: i32 = 5;

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn wasm_instantiate(bytes: *const u8, len: usize, imports: *mut Slot, n: usize, out: *mut u32) -> i32;
    fn wasm_export(inst: H, name: *const u8, len: usize) -> H;
    fn wasm_call(func: H, sig: *const u8, sig_len: usize, vals: *mut u64) -> i32;
    fn wasm_error(buf: *mut u8, cap: usize) -> usize;
    fn wasm_frames(id: u32, buf: *mut u8, cap: usize) -> usize;
    fn wasm_memory_read(mem: H, addr: u32, dst: *mut u8, len: usize) -> i32;
    fn wasm_memory_write(mem: H, addr: u32, src: *const u8, len: usize) -> i32;
    fn wasm_drop(handle: H);
}

/// The kind of a trap's code, and wasmtime's words for it, which the
/// loader's users print (a host's differ from one to another).
fn kind(code: u64) -> (TrapKind, Option<&'static str>) {
    match code {
        1 => (TrapKind::Interrupt, Some("interrupt")),
        2 => (TrapKind::StackOverflow, Some("call stack exhausted")),
        3 => (TrapKind::MemoryOutOfBounds, Some("out of bounds memory access")),
        4 => (TrapKind::IntegerDivisionByZero, Some("integer divide by zero")),
        5 => (TrapKind::IntegerOverflow, Some("integer overflow")),
        6 => (TrapKind::Other, Some("invalid conversion to integer")),
        7 => (TrapKind::BadSignature, Some("indirect call type mismatch")),
        8 => (TrapKind::IndirectCallToNull, Some("uninitialized element")),
        9 => (TrapKind::TableOutOfBounds, Some("undefined element: out of bounds table access")),
        10 => (TrapKind::Unreachable, Some("wasm `unreachable` instruction executed")),
        _ => (TrapKind::Other, None),
    }
}

/// A text the host writes: it says its length, and a longer one is asked
/// again.
fn text(f: impl Fn(*mut u8, usize) -> usize) -> String {
    let mut buf = vec![0; 256];
    loop {
        let n = f(buf.as_mut_ptr(), buf.len());
        if n <= buf.len() {
            buf.truncate(n);
            return String::from_utf8_lossy(&buf).into_owned();
        }
        buf.resize(n, 0);
    }
}

fn message() -> String {
    // SAFETY: the host writes at most cap bytes to buf
    text(|b, cap| unsafe { wasm_error(b, cap) })
}

fn frames(id: u32) -> Vec<Frame> {
    // SAFETY: as in message
    let t = text(|b, cap| unsafe { wasm_frames(id, b, cap) });
    t.lines().map(|l| {
        let (off, name) = l.split_once(' ').unwrap_or((l, ""));
        Frame { instance: None, func: (!name.is_empty()).then(|| name.to_string()), offset: usize::from_str_radix(off, 16).ok() }
    }).collect()
}

/// What a status other than OK says, with its detail.
pub fn error(status: i32, detail: u64) -> Error {
    match status {
        EXIT => Error::Exit(detail as i32),
        TRAP | STOP => {
            let (kind, words) = if status == STOP { (TrapKind::Interrupt, Some("interrupt")) } else { kind(detail) };
            let message = words.map_or_else(message, |w| format!("wasm trap: {w}"));
            Error::Trap(Trap { kind, message, fault: None, frames: frames(0) })
        }
        _ => Error::Fail(message()),
    }
}

/// `wasm_call` itself: the status, with the detail in `vals[0]`, which
/// [`error`] reads. Nothing is allocated: the machine inside sc.wasm puts
/// the memory back after a cell that trapped before it allocates.
pub fn call(func: H, sig: &str, vals: &mut [u64]) -> i32 {
    let (params, results) = sig.split_once(':').unwrap_or((sig, ""));
    assert!(vals.len() >= params.len().max(results.len()).max(1), "{sig}: {} values", vals.len());
    // SAFETY: vals has a slot for each parameter and result, and the detail
    unsafe { wasm_call(func, sig.as_ptr(), sig.len(), vals.as_mut_ptr()) }
}

/// `wasm_export` itself: a handle that nothing drops.
pub fn export(inst: H, name: &str) -> H {
    // SAFETY: the host reads len bytes
    unsafe { wasm_export(inst, name.as_ptr(), name.len()) }
}

/// The host's primitives. The host keeps what they make, until a drop: a
/// `Host` drops every handle it got when it goes, with the loader that owns
/// it, so a session the page runs again leaves nothing behind in the host.
#[derive(Default)]
pub struct Host {
    made: Vec<H>,
}

impl Drop for Host {
    fn drop(&mut self) {
        for &h in &self.made {
            // SAFETY: a handle of the host's
            unsafe { wasm_drop(h) }
        }
    }
}

impl Runtime for Host {
    fn instantiate(&mut self, bytes: &[u8], _compiled: Option<&[u8]>, imports: &mut [Import]) -> Result<H> {
        let mut slots: Vec<Slot> = imports.iter().map(|i| match *i {
            Import::Default => Slot { kind: 0, value: 0 },
            Import::Handle(h) => Slot { kind: 1, value: h as i32 },
            Import::I32 { value, mutable } => Slot { kind: 2 + mutable as u32, value },
        }).collect();
        let mut out = 0;
        // SAFETY: the host reads len bytes and n slots, and writes the slots
        // and *out
        let st = unsafe { wasm_instantiate(bytes.as_ptr(), bytes.len(), slots.as_mut_ptr(), slots.len(), &mut out) };
        if st != OK {
            return Err(error(st, out as u64));
        }
        for (i, s) in imports.iter_mut().zip(&slots) {
            if let Import::I32 { .. } = i {
                *i = Import::Handle(s.value as H);
                self.made.push(s.value as H);
            }
        }
        self.made.push(out);
        Ok(out)
    }

    fn export(&mut self, inst: H, name: &str) -> H {
        let h = export(inst, name);
        if h != 0 {
            self.made.push(h);
        }
        h
    }

    fn call(&mut self, func: H, sig: &str, vals: &mut [u64]) -> Result<()> {
        let (params, results) = sig.split_once(':').unwrap_or((sig, ""));
        if vals.len() < params.len().max(results.len()) {
            return Err(Error::Fail(format!("{sig}: {} values", vals.len())));
        }
        // the detail of a status needs a slot
        let mut one = [0];
        let vals = if vals.is_empty() { &mut one[..] } else { vals };
        match call(func, sig, vals) {
            OK => Ok(()),
            st => Err(error(st, vals[0])),
        }
    }

    fn read(&mut self, mem: H, addr: u32, buf: &mut [u8]) -> bool {
        // SAFETY: the host writes len bytes to buf
        unsafe { wasm_memory_read(mem, addr, buf.as_mut_ptr(), buf.len()) == 0 }
    }

    fn write(&mut self, mem: H, addr: u32, data: &[u8]) -> bool {
        // SAFETY: the host reads len bytes of data
        unsafe { wasm_memory_write(mem, addr, data.as_ptr(), data.len()) == 0 }
    }

    fn stack(&self, id: u32) -> Vec<Frame> {
        if id == 0 { Vec::new() } else { frames(id) }
    }
}
