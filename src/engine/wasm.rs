//! The raw functions of `engine/sc.h` over `sc.wasm`'s exports, through
//! wasmtime (the `wasm` feature, PLANO-REPL.md §5.2): what `sys` is with the
//! linked engine, for build/sc-engine-wasm, the test host of the module.
//!
//! The module is `<the program>.wasm`, precompiled to `<the program>.cwasm`
//! by `<the program> --precompile`. Each session is a new instance in a
//! store of its own: `_initialize`, `chdir` to the process's directory,
//! `sc_session_new`; the one-shot engine is a new instance that runs
//! `_start`. The guest sees the repository at the same path and a directory
//! of this process as `/tmp`, where the session keeps its cells.
//!
//! [`super::serve`] forks before each cell, which is safe only with one
//! thread: the module is compiled in another process, and WASI runs on a
//! tokio runtime of this thread that blocks in place.
//!
//! A trap or an exit of the guest ends the process, as a crash of the
//! linked engine does: the trap, and the cut function (frame 0 `__wrap_`,
//! `sc-cut.wasm`) as scripts/link.py's native stubs say it, go to stderr.
use super::sys::sc_session;
use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_uint, CStr};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use wasmtime::{Config, Engine, Instance, Linker, Memory, Module, Store, TypedFunc, WasmBacktrace};
use wasmtime_wasi::p1::{self, WasiP1Ctx};
use wasmtime_wasi::{FsPerms, I32Exit, WasiCtxBuilder};

fn exe() -> PathBuf {
    std::env::current_exe().expect("the program's path")
}

/// `<the program>.wasm` compiled to `<the program>.cwasm`, in this process,
/// which may use threads.
pub fn precompile() -> wasmtime::Result<()> {
    let (wasm, cwasm) = (exe().with_extension("wasm"), exe().with_extension("cwasm"));
    let bytes = Engine::new(&config(true))?.precompile_module(&std::fs::read(&wasm)?)?;
    let tmp = cwasm.with_extension("cwasm.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &cwasm)?;
    Ok(())
}

fn config(parallel: bool) -> Config {
    let mut c = Config::new();
    c.parallel_compilation(parallel);
    // the frames of a trap: the default of 20 may not reach the engine's
    c.wasm_backtrace_max_frames(std::num::NonZeroUsize::new(256));
    c
}

struct Shared {
    engine: Engine,
    module: Module,
    linker: Linker<WasiP1Ctx>,
    rt: tokio::runtime::Runtime,
    root: PathBuf,
    tmp: PathBuf,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

fn shared() -> &'static Shared {
    SHARED.get_or_init(|| {
        let engine = Engine::new(&config(false)).unwrap();
        let cwasm = exe().with_extension("cwasm");
        // the file is ours: `<the program> --precompile` wrote it
        let module = unsafe { Module::deserialize_file(&engine, &cwasm) }
            .unwrap_or_else(|e| fail(&format!("{}: {e} (run {} --precompile)", cwasm.display(), exe().display())));
        let mut linker = Linker::new(&engine);
        p1::add_to_linker_sync(&mut linker, |c| c).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").canonicalize().unwrap();
        let tmp = std::env::temp_dir().join(format!("sc-engine-wasm-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        Shared { engine, module, linker, rt, root, tmp }
    })
}

/// Removes the directory of `/tmp`: at the end of the process that goes on
/// to the end (the copies of a fork share it).
pub fn cleanup() {
    if let Some(s) = SHARED.get() {
        let _ = std::fs::remove_dir_all(&s.tmp);
    }
}

thread_local! {
    static GUEST: RefCell<Option<Guest>> = const { RefCell::new(None) };
}

fn fail(msg: &str) -> ! {
    eprintln!("sc: {msg}");
    std::process::exit(1)
}

/// One thread, before a fork.
pub fn assert_one_thread() {
    let n = std::fs::read_dir("/proc/self/task").map(|d| d.count()).unwrap_or(0);
    assert_eq!(n, 1, "fork with {n} threads");
}

/// A new instance; `args` its argv.
fn instance(args: &[&str]) -> (Store<WasiP1Ctx>, Instance) {
    let s = shared();
    let _g = s.rt.enter();
    let mut b = WasiCtxBuilder::new();
    // before the preopens, which take it
    b.allow_blocking_current_thread(true);
    b.args(args).inherit_stdout().inherit_stderr();
    let open = |b: &mut WasiCtxBuilder, host: &Path, guest: &str| {
        if let Err(e) = b.preopened_dir(host, guest, FsPerms::ReadWrite) {
            fail(&format!("{}: {e}", host.display()))
        }
    };
    open(&mut b, &s.root, s.root.to_str().unwrap());
    open(&mut b, &s.tmp, "/tmp");
    let mut store = Store::new(&s.engine, b.build_p1());
    let inst = s.linker.instantiate(&mut store, &s.module).unwrap_or_else(|e| fail(&format!("{e:?}")));
    (store, inst)
}

/// A trap or an exit of the guest ends the process.
fn check<T>(r: wasmtime::Result<T>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => {
            if let Some(I32Exit(code)) = e.downcast_ref::<I32Exit>() {
                unsafe { _exit(*code) }
            }
            let frame0 = e.downcast_ref::<WasmBacktrace>().and_then(|b| b.frames().first()).and_then(|f| f.func_name());
            if let Some(name) = frame0.filter(|n| n.starts_with("__wrap_")) {
                eprintln!("sc: cut function called: {name}");
            }
            eprintln!("{e:?}");
            unsafe { _exit(134) }
        }
    }
}

extern "C" {
    fn _exit(code: c_int) -> !;
}

fn func<P: wasmtime::WasmParams, R: wasmtime::WasmResults>(
    store: &mut Store<WasiP1Ctx>,
    inst: &Instance,
    name: &str,
) -> TypedFunc<P, R> {
    inst.get_typed_func(store, name).unwrap_or_else(|e| fail(&format!("{name}: {e}")))
}

/// A session's instance.
struct Guest {
    store: Store<WasiP1Ctx>,
    memory: Memory,
    malloc: TypedFunc<u32, u32>,
    free: TypedFunc<u32, ()>,
    cell: TypedFunc<(u32, u32, u32, i32), i32>,
    depth: TypedFunc<(u32, u32, u32), i32>,
    undo: TypedFunc<u32, i32>,
    ptr: TypedFunc<(u32, i32), u32>,
    len: TypedFunc<(u32, i32), u32>,
    del: TypedFunc<u32, ()>,
}

impl Guest {
    /// bytes in the guest's memory, from its malloc
    fn put(&mut self, bytes: &[u8]) -> u32 {
        let p = check(self.malloc.call(&mut self.store, bytes.len().max(1) as u32));
        if p == 0 {
            fail("the guest's malloc failed");
        }
        self.memory.write(&mut self.store, p as usize, bytes).unwrap();
        p
    }
}

fn with<R>(f: impl FnOnce(&mut Guest) -> R) -> R {
    let _g = shared().rt.enter();
    GUEST.with(|g| f(g.borrow_mut().as_mut().expect("a session")))
}

/// `sc_main`: a new instance runs `_start`.
pub unsafe fn sc_main(argc: c_int, argv: *mut *const c_char) -> c_int {
    let args: Vec<String> =
        (0..argc as usize).map(|i| CStr::from_ptr(*argv.add(i)).to_string_lossy().into_owned()).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (mut store, inst) = instance(&args);
    let _g = shared().rt.enter();
    let start: TypedFunc<(), ()> = func(&mut store, &inst, "_start");
    match start.call(&mut store, ()) {
        Ok(()) => 0,
        Err(e) => match e.downcast_ref::<I32Exit>() {
            Some(I32Exit(code)) => *code,
            None => check(Err(e)),
        },
    }
}

/// `sc_session_new` in a new instance, which replaces the one before.
pub unsafe fn sc_session_new(argc: c_int, argv: *const *const c_char, out: *mut *mut sc_session) -> c_int {
    let (mut store, inst) = instance(&["sc"]);
    let _g = shared().rt.enter();
    check(func::<(), ()>(&mut store, &inst, "_initialize").call(&mut store, ()));
    let memory = inst.get_memory(&mut store, "memory").unwrap_or_else(|| fail("no memory"));
    let mut g = Guest {
        malloc: func(&mut store, &inst, "malloc"),
        free: func(&mut store, &inst, "free"),
        cell: func(&mut store, &inst, "sc_session_cell"),
        depth: func(&mut store, &inst, "sc_session_depth"),
        undo: func(&mut store, &inst, "sc_session_undo"),
        ptr: func(&mut store, &inst, "sc_result_ptr"),
        len: func(&mut store, &inst, "sc_result_len"),
        del: func(&mut store, &inst, "sc_session_free"),
        store,
        memory,
    };
    let new: TypedFunc<(i32, u32, u32), i32> = func(&mut g.store, &inst, "sc_session_new");
    let chdir: TypedFunc<u32, i32> = func(&mut g.store, &inst, "chdir");
    // the session finds a cell's "..." includes from the directory
    if let Some(cwd) = std::env::current_dir().ok().and_then(|d| d.to_str().map(String::from)) {
        let p = g.put(format!("{cwd}\0").as_bytes());
        check(chdir.call(&mut g.store, p));
        check(g.free.call(&mut g.store, p));
    }
    // the arguments stay with the session
    let mut ptrs = vec![];
    for i in 0..argc as usize {
        ptrs.extend(g.put(CStr::from_ptr(*argv.add(i)).to_bytes_with_nul()).to_le_bytes());
    }
    ptrs.extend(0u32.to_le_bytes());
    let pv = g.put(&ptrs);
    let pout = g.put(&[0; 4]);
    let st = check(new.call(&mut g.store, (argc, pv, pout)));
    let mut s = [0; 4];
    g.memory.read(&g.store, pout as usize, &mut s).unwrap();
    check(g.free.call(&mut g.store, pout));
    *out = u32::from_le_bytes(s) as usize as *mut sc_session;
    GUEST.with(|x| *x.borrow_mut() = Some(g));
    st
}

pub unsafe fn sc_session_free(s: *mut sc_session) {
    with(|g| check(g.del.call(&mut g.store, s as usize as u32)));
    GUEST.with(|x| *x.borrow_mut() = None);
}

pub unsafe fn sc_session_cell(s: *mut sc_session, src: *const c_char, len: c_uint, mode: c_int) -> c_int {
    with(|g| {
        let p = g.put(std::slice::from_raw_parts(src.cast(), len as usize));
        let st = check(g.cell.call(&mut g.store, (s as usize as u32, p, len, mode)));
        check(g.free.call(&mut g.store, p));
        // for mem- (test/session.py), in place of the process's RSS
        if std::env::var_os("SC_MEMORY_SIZE").is_some() {
            eprintln!("sc: memory.size {}", g.memory.data_size(&g.store));
        }
        st
    })
}

pub unsafe fn sc_session_depth(s: *mut sc_session, src: *const c_char, len: c_uint) -> c_int {
    with(|g| {
        let p = g.put(std::slice::from_raw_parts(src.cast(), len as usize));
        let d = check(g.depth.call(&mut g.store, (s as usize as u32, p, len)));
        check(g.free.call(&mut g.store, p));
        d
    })
}

pub unsafe fn sc_session_undo(s: *mut sc_session) -> c_int {
    with(|g| check(g.undo.call(&mut g.store, s as usize as u32)))
}

/// A pointer into the guest's memory, which does not move: wasmtime
/// reserves its whole range up front.
pub unsafe fn sc_result_ptr(s: *mut sc_session, part: c_int) -> *const u8 {
    with(|g| match check(g.ptr.call(&mut g.store, (s as usize as u32, part))) {
        0 => std::ptr::null(),
        p => g.memory.data_ptr(&g.store).add(p as usize),
    })
}

pub unsafe fn sc_result_len(s: *mut sc_session, part: c_int) -> c_uint {
    with(|g| check(g.len.call(&mut g.store, (s as usize as u32, part))))
}
