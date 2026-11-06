//! The compiler of the native `sc`: sc.wasm's session in this process, on
//! wasmtime (ANALISE.md §68), as the browser runs it. A cell that fails
//! leaves nothing: the session copies its memory before the cell and puts
//! it back (SC_SNAPSHOT, §65). A trap or an exit of the compiler (a crash,
//! a fatal error, Ctrl-C, the timeout, the memory) stops the call, and
//! `sc_session_recover` puts the copy back too: the cell is an error, and
//! the session goes on. Without the copy (a replay) the session is lost,
//! and the [`super::Repl`] replays the log in another [`Guest`].
//!
//! The module is sc.wasm compiled for this target and the loader's
//! [`Config`](crate::loader::config), with zstd, which build.rs embeds. It
//! sees the pack inside it at [`ROOT`] (engine/sc_pack.cpp), and the host's
//! files at their paths: the session finds a cell's "..." includes from the
//! process's directory.
//!
//! `SC_CWASM` names another module, compiled by [`precompile`]: the
//! batteries run the uncut engine (sc-full.wasm) and the cuts being learned
//! so. The trap of a function the cuts removed says its name on stderr.
use super::{sys, Answer, Compiler, Mode, Status};
use crate::machine::ROOT;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::thread::{self, Thread};
use std::time::{Duration, Instant};
use wasmtime::{Engine, Instance, Linker, Memory, Module, ResourceLimiter, Store, TypedFunc, UpdateDeadline, WasmBacktrace};
use wasmtime_wasi::p1::{self, WasiP1Ctx};
use wasmtime_wasi::{FsPerms, WasiCtxBuilder};

// clippy (the CI's make check) builds without the engine (build.rs)
#[cfg(not(clippy))]
static CWASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sc.cwasm.zst"));
#[cfg(clippy)]
static CWASM: &[u8] = &[];

const CRASHED: &str = "sc: the engine crashed on this cell; the session is as it was before it\n";

/// The Ctrl-Cs so far ([`interrupt`]): a call stops at one after it began.
static INTERRUPTS: AtomicU64 = AtomicU64::new(0);
/// The calls running with a deadline: while there is one, the watchdog
/// moves the epoch, at which each store looks at its own.
static TIMED: AtomicUsize = AtomicUsize::new(0);
/// How often the watchdog moves the epoch.
const TICK: Duration = Duration::from_millis(100);

/// Why a call stopped.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    Interrupted,
    TimedOut,
}

struct Shared {
    engine: Engine,
    module: Module,
    linker: Linker<Ctx>,
    watchdog: Thread,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

fn shared() -> io::Result<&'static Shared> {
    if let Some(s) = SHARED.get() {
        return Ok(s);
    }
    let fail = |e: wasmtime::Error| io::Error::other(format!("sc.wasm: {e}"));
    let engine = Engine::new(&crate::loader::config::config()).map_err(fail)?;
    // SAFETY: build.rs, or precompile, compiled it with this wasmtime and
    // Config; one that does not fit fails
    let module = match std::env::var_os("SC_CWASM") {
        Some(p) => unsafe { Module::deserialize_file(&engine, p) },
        None => unsafe { Module::deserialize(&engine, zstd::decode_all(CWASM)?) },
    }
    .map_err(fail)?;
    let mut linker = Linker::new(&engine);
    p1::add_to_linker_sync(&mut linker, |c: &mut Ctx| &mut c.wasi).map_err(fail)?;
    // the loader's primitives (src/loader/host.rs), which the session does
    // not use
    linker.define_unknown_imports_as_traps(&module).map_err(fail)?;
    let e = engine.clone();
    let watchdog = thread::Builder::new().name("sc-watchdog".into()).spawn(move || loop {
        if TIMED.load(Ordering::Relaxed) == 0 {
            thread::park();
        } else {
            thread::sleep(TICK);
            e.increment_epoch();
        }
    })?.thread().clone();
    Ok(SHARED.get_or_init(|| Shared { engine, module, linker, watchdog }))
}

/// `wasm` compiled to `cwasm`, for `SC_CWASM`.
pub fn precompile(wasm: &Path, cwasm: &Path) -> io::Result<()> {
    let fail = |e: wasmtime::Error| io::Error::other(format!("{}: {e}", wasm.display()));
    let engine = Engine::new(&crate::loader::config::config()).map_err(fail)?;
    std::fs::write(cwasm, engine.precompile_module(&std::fs::read(wasm)?).map_err(fail)?)
}

/// Stops the call of the compiler running, if any: the cell is an error,
/// and the session is as before it. Safe in a signal handler (Ctrl-C): an
/// atomic increment, and the epoch's.
pub fn interrupt() {
    INTERRUPTS.fetch_add(1, Ordering::Relaxed);
    if let Some(s) = SHARED.get() {
        s.engine.increment_epoch();
    }
}

/// The store's data: WASI, the memory the module may grow to, and the call
/// running.
struct Ctx {
    wasi: WasiP1Ctx,
    max: Option<usize>,
    /// A growth above `max` was refused.
    over: bool,
    /// The call's deadline, and the Ctrl-Cs before it if Ctrl-C stops it.
    deadline: Option<Instant>,
    interrupts: Option<u64>,
    /// Why the call stopped, if it was stopped.
    stop: Option<Stop>,
}

/// At each move of the epoch: whether the call goes on.
fn tick(c: &mut Ctx) -> UpdateDeadline {
    c.stop = if c.interrupts.is_some_and(|i| INTERRUPTS.load(Ordering::Relaxed) != i) {
        Some(Stop::Interrupted)
    } else if c.deadline.is_some_and(|d| Instant::now() >= d) {
        Some(Stop::TimedOut)
    } else {
        return UpdateDeadline::Continue(1);
    };
    UpdateDeadline::Interrupt
}

impl ResourceLimiter for Ctx {
    fn memory_growing(&mut self, _: usize, desired: usize, _: Option<usize>) -> wasmtime::Result<bool> {
        let ok = self.max.is_none_or(|m| desired <= m);
        self.over |= !ok;
        Ok(ok)
    }

    fn table_growing(&mut self, _: usize, _: usize, _: Option<usize>) -> wasmtime::Result<bool> {
        Ok(true)
    }
}

/// A session of sc.wasm in an instance of its own.
pub struct Guest {
    timeout: Option<Duration>,
    memory: Option<u64>,
    store: Store<Ctx>,
    mem: Memory,
    f: Funcs,
    /// The session, once started: its address in the module.
    session: u32,
    /// The instance can no longer be called: a trap without the copy.
    dead: bool,
    /// The module's TMPDIR, removed with the instance: the session never
    /// ends as clang would remove the .pcm `#pragma clang module build`
    /// writes there (ANALISE.md §70).
    _tmp: Option<TempDir>,
}

/// A new directory in the host's temporary one, removed with it.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Option<TempDir> {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("sc-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&d).ok().map(|()| TempDir(d))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Funcs {
    malloc: TypedFunc<u32, u32>,
    free: TypedFunc<u32, ()>,
    new: TypedFunc<(i32, u32, u32), i32>,
    del: TypedFunc<u32, ()>,
    cell: TypedFunc<(u32, u32, u32, i32), i32>,
    recover: TypedFunc<u32, i32>,
    depth: TypedFunc<(u32, u32, u32), i32>,
    ptr: TypedFunc<(u32, i32), u32>,
    len: TypedFunc<(u32, i32), u32>,
}

/// The guest's path of a host path: the same on Unix; on Windows, `C:\a`
/// is `/C:/a`, under the drive's preopen.
fn guest_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    if cfg!(windows) { format!("/{}", s.replace('\\', "/")) } else { s.into_owned() }
}

fn lost(what: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, format!("the engine ended ({what})"))
}

impl Guest {
    /// A new instance, without a session. Each call is limited to
    /// `timeout`, and the module's memory, with the copy, to twice
    /// `memory`; above `memory`, after an accepted cell, a [`super::Repl`]
    /// replays the log in another ([`Compiler::over`], PLANO-REPL.md §4).
    pub fn new(timeout: Option<Duration>, memory: Option<u64>) -> io::Result<Guest> {
        let s = shared()?;
        let fail = |e: wasmtime::Error| io::Error::other(format!("sc.wasm: {e:?}"));
        let mut b = WasiCtxBuilder::new();
        b.args(&["sc"]).env("SC_SNAPSHOT", "1").inherit_stderr();
        let tmp = TempDir::new();
        if let Some(t) = &tmp {
            b.env("TMPDIR", guest_path(&t.0));
        }
        let cwd = std::env::current_dir()?;
        // the files of the host, as the native engine saw them
        let root = cwd.ancestors().last().unwrap_or(&cwd);
        let at = if cfg!(windows) { guest_path(root).trim_end_matches('/').to_string() } else { "/".into() };
        b.preopened_dir(root, at, FsPerms::ReadWrite).map_err(fail)?;
        let max = memory.map(|m| (2 * m).min(u32::MAX as u64 + 1) as usize);
        let ctx = Ctx { wasi: b.build_p1(), max, over: false, deadline: None, interrupts: None, stop: None };
        let mut store = Store::new(&s.engine, ctx);
        store.limiter(|c| c);
        store.epoch_deadline_callback(|mut c| Ok(tick(c.data_mut())));
        store.set_epoch_deadline(1);
        let inst = s.linker.instantiate(&mut store, &s.module).map_err(fail)?;
        let mem = inst.get_memory(&mut store, "memory").ok_or_else(|| io::Error::other("sc.wasm: no memory"))?;
        let f = funcs(&mut store, &inst).map_err(fail)?;
        let mut g = Guest { timeout, memory, store, mem, f, session: 0, dead: false, _tmp: tmp };
        g.init(&inst, &cwd).map_err(fail)?;
        Ok(g)
    }

    fn init(&mut self, inst: &Instance, cwd: &Path) -> wasmtime::Result<()> {
        inst.get_typed_func::<(), ()>(&mut self.store, "_initialize")?.call(&mut self.store, ())?;
        let pack: TypedFunc<u32, ()> = inst.get_typed_func(&mut self.store, "sc_pack")?;
        let p = self.put(format!("{ROOT}\0").as_bytes())?;
        pack.call(&mut self.store, p)?;
        self.f.free.call(&mut self.store, p)?;
        let chdir: TypedFunc<u32, i32> = inst.get_typed_func(&mut self.store, "chdir")?;
        let p = self.put(format!("{}\0", guest_path(cwd)).as_bytes())?;
        chdir.call(&mut self.store, p)?;
        self.f.free.call(&mut self.store, p)
    }

    /// `bytes` in the module's memory, from its malloc.
    fn put(&mut self, bytes: &[u8]) -> wasmtime::Result<u32> {
        let p = self.f.malloc.call(&mut self.store, bytes.len().max(1) as u32)?;
        if p == 0 {
            wasmtime::bail!("the module's malloc failed");
        }
        self.mem.write(&mut self.store, p as usize, bytes)?;
        Ok(p)
    }

    /// `f` within the limits: armed when it starts, as a Ctrl-C before it
    /// is for the student's code. Ctrl-C stops it if `ctrl_c`: a cell's
    /// compile, which [`Compiler::cell`] recovers from.
    fn call<R>(&mut self, ctrl_c: bool, f: impl FnOnce(&mut Guest) -> wasmtime::Result<R>) -> Result<R, (wasmtime::Error, Option<Stop>)> {
        let c = self.store.data_mut();
        let interrupts = ctrl_c.then(|| INTERRUPTS.load(Ordering::Relaxed));
        (c.deadline, c.interrupts, c.stop) = (self.timeout.map(|t| Instant::now() + t), interrupts, None);
        self.store.set_epoch_deadline(1);
        if self.timeout.is_some() {
            TIMED.fetch_add(1, Ordering::Relaxed);
            SHARED.get().expect("an instance").watchdog.unpark();
        }
        let r = f(self);
        if self.timeout.is_some() {
            TIMED.fetch_sub(1, Ordering::Relaxed);
        }
        let c = self.store.data_mut();
        // the recover after it runs to its end
        (c.deadline, c.interrupts) = (None, None);
        r.map_err(|e| (e, c.stop.take()))
    }

    /// The part `part` of the last result.
    fn part(&mut self, part: u32) -> wasmtime::Result<Vec<u8>> {
        let s = self.session;
        let p = self.f.ptr.call(&mut self.store, (s, part as i32))?;
        let n = self.f.len.call(&mut self.store, (s, part as i32))?;
        let mut v = vec![0; if p == 0 { 0 } else { n as usize }];
        self.mem.read(&self.store, p as usize, &mut v)?;
        Ok(v)
    }

    fn answer(&mut self, st: i32) -> wasmtime::Result<Answer> {
        let [diags, so, syms] = [sys::sc_part::SC_DIAGS, sys::sc_part::SC_SO, sys::sc_part::SC_SYMS].map(|p| self.part(p));
        Ok(Answer {
            status: Status::from_raw(st as u32),
            diags: String::from_utf8_lossy(&diags?).into_owned(),
            so: so?,
            syms: String::from_utf8_lossy(&syms?).lines().map(String::from).collect(),
        })
    }

    /// Why a call stopped, for the student.
    fn why(&self, e: &wasmtime::Error, stop: Option<Stop>) -> String {
        let t = self.timeout.unwrap_or_default().as_secs();
        let m = self.memory.unwrap_or_default() >> 20;
        match stop {
            Some(Stop::Interrupted) => "sc: interrupted\n".into(),
            Some(Stop::TimedOut) => format!("sc: the compiler took more than {t} s\n"),
            _ if self.store.data().over => format!("sc: the compiler took more than {m} MB\n"),
            _ => {
                // the cut function a crash called (frame 0 `__wrap_`,
                // scripts/link.py), for make learn-session; the engine has
                // written the rest itself ("LLVM ERROR: ...")
                let frame0 = e.downcast_ref::<WasmBacktrace>().and_then(|b| b.frames().first()).and_then(|f| f.func_name());
                if let Some(name) = frame0.filter(|n| n.starts_with("__wrap_")) {
                    eprintln!("sc: cut function called: {name}");
                }
                CRASHED.into()
            }
        }
    }
}

fn funcs(store: &mut Store<Ctx>, inst: &Instance) -> wasmtime::Result<Funcs> {
    Ok(Funcs {
        malloc: inst.get_typed_func(&mut *store, "malloc")?,
        free: inst.get_typed_func(&mut *store, "free")?,
        new: inst.get_typed_func(&mut *store, "sc_session_new")?,
        del: inst.get_typed_func(&mut *store, "sc_session_free")?,
        cell: inst.get_typed_func(&mut *store, "sc_session_cell")?,
        recover: inst.get_typed_func(&mut *store, "sc_session_recover")?,
        depth: inst.get_typed_func(&mut *store, "sc_session_depth")?,
        ptr: inst.get_typed_func(&mut *store, "sc_result_ptr")?,
        len: inst.get_typed_func(&mut *store, "sc_result_len")?,
    })
}

impl super::Repl<Guest> {
    /// Each call within `timeout`, the module's memory within `memory`
    /// ([`Guest::new`]), from the next call on.
    pub fn limit(&mut self, timeout: Option<Duration>, memory: Option<u64>) {
        let g = &mut self.compiler;
        (g.timeout, g.memory) = (timeout, memory);
        g.store.data_mut().max = memory.map(|m| (2 * m).min(u32::MAX as u64 + 1) as usize);
    }
}

impl Compiler for Guest {
    fn another(&self) -> io::Result<Guest> {
        Guest::new(self.timeout, self.memory)
    }

    fn new_session(&mut self, args: &[String]) -> io::Result<Answer> {
        if self.dead {
            return Err(lost("a session before"));
        }
        let r = self.call(false, |g| {
            if g.session != 0 {
                let s = std::mem::take(&mut g.session);
                g.f.del.call(&mut g.store, s)?;
            }
            // the arguments stay with the session
            let mut ptrs = vec![];
            for a in args {
                ptrs.extend(g.put(format!("{a}\0").as_bytes())?.to_le_bytes());
            }
            ptrs.extend(0u32.to_le_bytes());
            let argv = g.put(&ptrs)?;
            let out = g.put(&[0; 4])?;
            let st = g.f.new.call(&mut g.store, (args.len() as i32, argv, out))?;
            let mut s = [0; 4];
            g.mem.read(&g.store, out as usize, &mut s)?;
            g.f.free.call(&mut g.store, out)?;
            g.session = u32::from_le_bytes(s);
            g.answer(st)
        });
        r.map_err(|(e, _)| {
            self.dead = true;
            lost(e)
        })
    }

    fn cell(&mut self, src: &[u8], mode: Mode) -> io::Result<Answer> {
        if self.dead || self.session == 0 {
            return Err(lost("no session"));
        }
        let len = u32::try_from(src.len()).map_err(|_| io::Error::other("the cell is too long"))?;
        let r = self.call(true, |g| {
            let p = g.put(src)?;
            let st = g.f.cell.call(&mut g.store, (g.session, p, len, mode.to_raw() as i32))?;
            g.f.free.call(&mut g.store, p)?;
            g.answer(st)
        });
        let mut a = match r {
            Ok(a) => a,
            Err((e, stop)) => {
                let why = self.why(&e, stop);
                self.store.data_mut().over = false;
                // the deadline that stopped it stays where it was
                self.store.set_epoch_deadline(1);
                // the copy back, before any other call: what the cell left
                // is gone with it, the source too
                let st = self.f.recover.call(&mut self.store, self.session);
                if st.is_ok_and(|st| st as u32 == sys::sc_status::SC_ERROR) {
                    return Ok(Answer::bare(Status::Error, why));
                }
                self.dead = true;
                return Err(lost(why.trim_end().trim_start_matches("sc: ")));
            }
        };
        // the compiler may say so itself ("Out of memory"), as an error
        if std::mem::take(&mut self.store.data_mut().over) {
            a.diags += &format!("sc: the compiler took more than {} MB\n", self.memory.unwrap_or_default() >> 20);
        }
        // for mem- (test/session.py), in place of the process's RSS
        if std::env::var_os("SC_MEMORY_SIZE").is_some() {
            eprintln!("sc: memory.size {}", self.mem.data_size(&self.store));
        }
        Ok(a)
    }

    fn depth(&mut self, src: &[u8]) -> io::Result<u32> {
        if self.dead || self.session == 0 {
            return Ok(0);
        }
        let len = u32::try_from(src.len()).map_err(|_| io::Error::other("the cell is too long"))?;
        let r = self.call(false, |g| {
            let p = g.put(src)?;
            let d = g.f.depth.call(&mut g.store, (g.session, p, len))?;
            g.f.free.call(&mut g.store, p)?;
            Ok(d)
        });
        r.map(|d| d as u32).map_err(|(e, _)| {
            self.dead = true;
            lost(e)
        })
    }

    fn over(&self) -> bool {
        self.memory.is_some_and(|m| self.mem.data_size(&self.store) as u64 > m)
    }
}

/// Never moves between threads while a call runs: the REPL's machine is
/// behind a mutex.
const _: fn() = || {
    fn send<T: Send>() {}
    send::<Guest>();
};
