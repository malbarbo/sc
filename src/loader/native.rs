//! The loader's [`Runtime`] on wasmtime, for `sc` and `scload`. A call
//! stops at Ctrl-C ([`Wasmtime::on_ctrl_c`]).

use super::config;
use super::runtime::{Error, Frame, Import, Runtime, Trap, TrapKind, H};
use std::collections::HashMap;
use std::sync::Arc;
use wasm_encoder as we;
use wasmtime::{Engine, Extern, ExternType, Func, FuncType, Global, GlobalType, Instance, Linker, Memory, Module,
               Mutability, Store, ThrownException, Val, ValType, WasmBacktrace};
use wasmtime_wasi::p1::{self, WasiP1Ctx};
use wasmtime_wasi::I32Exit;

struct Host {
    wasi: WasiP1Ctx,
    stacks: Stacks,
}

/// The stacks `env.wasm_stack` keeps: each once, under a number from 1.
#[derive(Default)]
struct Stacks {
    ids: HashMap<Vec<(usize, usize)>, u32>, // (module, offset in it) of each frame
    traces: Vec<Arc<WasmBacktrace>>,
}

#[derive(Clone)]
enum Item {
    Extern(Extern),
    Instance(Instance),
}

pub struct Wasmtime {
    engine: Engine,
    store: Store<Host>,
    linker: Linker<Host>,
    handles: Vec<Item>, // handle k + 1
    instances: Vec<(Module, H)>, // for the instance of a frame
    wasi: HashMap<(H, String), Func>, // the WASI functions on each memory
    stack: Option<Func>,
}

impl Wasmtime {
    pub fn new(wasi: WasiP1Ctx) -> wasmtime::Result<Wasmtime> {
        let engine = Engine::new(&config::config())?;
        let store = Store::new(&engine, Host { wasi, stacks: Stacks::default() });
        let mut linker = Linker::new(&engine);
        p1::add_to_linker_sync(&mut linker, |h: &mut Host| &mut h.wasi)?;
        Ok(Wasmtime { engine, store, linker, handles: Vec::new(), instances: Vec::new(), wasi: HashMap::new(), stack: None })
    }

    fn handle(&mut self, e: Extern) -> H {
        self.handles.push(Item::Extern(e));
        self.handles.len() as H
    }

    fn get(&self, h: H) -> Option<&Extern> {
        match h.checked_sub(1).and_then(|k| self.handles.get(k as usize)) {
            Some(Item::Extern(e)) => Some(e),
            _ => None,
        }
    }

    fn memory(&self, h: H) -> Option<Memory> {
        self.get(h).and_then(|e| e.clone().into_memory())
    }

    /// The WASI functions `need` on the memory `mem`, through a shim that
    /// exports it: wasmtime-wasi reads the memory from its caller.
    fn wasi(&mut self, mem: H, need: &[(String, FuncType)]) -> wasmtime::Result<()> {
        let need: Vec<_> = need.iter().filter(|(n, _)| !self.wasi.contains_key(&(mem, n.clone()))).collect();
        if need.is_empty() {
            return Ok(());
        }
        let ty = |t: ValType| match t {
            ValType::I32 => we::ValType::I32, ValType::I64 => we::ValType::I64,
            ValType::F32 => we::ValType::F32, ValType::F64 => we::ValType::F64, _ => we::ValType::EXTERNREF,
        };
        // function k calls import k
        let (mut types, mut imports, mut funcs, mut exports, mut code) = (we::TypeSection::new(), we::ImportSection::new(),
            we::FunctionSection::new(), we::ExportSection::new(), we::CodeSection::new());
        let memory = we::MemoryType { minimum: 0, maximum: None, memory64: false, shared: false, page_size_log2: None };
        imports.import("env", "memory", memory);
        exports.export("memory", we::ExportKind::Memory, 0);
        for (k, (n, ft)) in need.iter().enumerate() {
            let k = k as u32;
            types.ty().function(ft.params().map(ty), ft.results().map(ty));
            imports.import("wasi_snapshot_preview1", n, we::EntityType::Function(k));
            funcs.function(k);
            exports.export(n, we::ExportKind::Func, need.len() as u32 + k);
            let mut f = we::Function::new([]);
            for i in 0..ft.params().len() as u32 {
                f.instructions().local_get(i);
            }
            f.instructions().call(k).end();
            code.function(&f);
        }
        let mut m = we::Module::new();
        m.section(&types).section(&imports).section(&funcs).section(&exports).section(&code);
        let m = Module::new(&self.engine, m.finish())?;
        let memory = self.memory(mem).ok_or_else(|| wasmtime::format_err!("WASI without a memory"))?;
        let mut l = self.linker.clone();
        l.define(&mut self.store, "env", "memory", memory)?;
        let inst = l.instantiate(&mut self.store, &m)?;
        for (n, _) in need {
            let f = inst.get_func(&mut self.store, n).unwrap();
            self.wasi.insert((mem, n.clone()), f);
        }
        Ok(())
    }

    /// `env.wasm_stack`: keeps the stack of its caller.
    fn stack_func(&mut self) -> Func {
        if let Some(f) = self.stack {
            return f;
        }
        let ty = FuncType::new(&self.engine, [], [ValType::I32]);
        let f = Func::new(&mut self.store, ty, |mut caller: wasmtime::Caller<'_, Host>, _, r| {
            let bt = WasmBacktrace::capture(&caller);
            let key: Vec<(usize, usize)> = bt.frames().iter()
                .map(|f| (f.module().image_range().start as usize, f.module_offset().unwrap_or(0))).collect();
            let st = &mut caller.data_mut().stacks;
            let n = st.traces.len() as u32 + 1;
            let id = *st.ids.entry(key).or_insert(n);
            if id == n {
                st.traces.push(Arc::new(bt));
            }
            r[0] = Val::I32(id as i32);
            Ok(())
        });
        self.stack = Some(f);
        f
    }

    fn frames(&self, bt: &WasmBacktrace) -> Vec<Frame> {
        bt.frames().iter().map(|f| Frame {
            instance: self.instances.iter().find(|(m, _)| Module::same(m, f.module())).map(|(_, h)| *h),
            func: f.func_name().map(str::to_string),
            offset: f.module_offset(),
        }).collect()
    }

    /// What `e`, of a call, says.
    fn error(&mut self, e: wasmtime::Error) -> Error {
        if let Some(x) = e.downcast_ref::<I32Exit>() {
            return Error::Exit(x.0);
        }
        let frames = e.downcast_ref::<WasmBacktrace>().map(|bt| self.frames(bt)).unwrap_or_default();
        let fault = e.chain().find_map(|c| {
            // wasmtime keeps the address to itself as a type
            let s = c.to_string();
            let hex = s.strip_prefix("memory fault at wasm address 0x")?.split(' ').next()?;
            u64::from_str_radix(hex, 16).ok()
        });
        use wasmtime::Trap as T;
        let (kind, message) = match e.downcast_ref::<T>() {
            Some(t) => (match t {
                T::Interrupt => TrapKind::Interrupt,
                T::StackOverflow => TrapKind::StackOverflow,
                T::MemoryOutOfBounds => TrapKind::MemoryOutOfBounds,
                T::IntegerDivisionByZero => TrapKind::IntegerDivisionByZero,
                T::IntegerOverflow => TrapKind::IntegerOverflow,
                T::BadSignature => TrapKind::BadSignature,
                T::IndirectCallToNull => TrapKind::IndirectCallToNull,
                T::TableOutOfBounds => TrapKind::TableOutOfBounds,
                T::UnreachableCodeReached => TrapKind::Unreachable,
                _ => TrapKind::Other,
            }, t.to_string()),
            None => {
                if e.is::<ThrownException>() {
                    // an exception no one caught, not a C++ one
                    let _ = self.store.take_pending_exception();
                }
                if frames.is_empty() {
                    return Error::Fail(e.root_cause().to_string());
                }
                (TrapKind::Other, e.root_cause().to_string())
            }
        };
        Error::Trap(Trap { kind, message, fault, frames })
    }

    /// From now on, Ctrl-C stops the code this runtime runs, in place of
    /// the runtime's before, and calls `also`, which must be safe in a
    /// signal handler (`sc::engine::interrupt`: the compile). The first
    /// call installs the SIGINT handler.
    pub fn on_ctrl_c(&self, also: fn()) {
        let _ = CTRL_C_ALSO.set(also);
        if CTRL_C.lock().unwrap().replace(self.engine.clone()).is_none() {
            // SAFETY: the handler only increments the epoch and calls `also`
            unsafe { libc::signal(libc::SIGINT, on_sigint as *const () as libc::sighandler_t) };
        }
    }
}

impl Runtime for Wasmtime {
    fn instantiate(&mut self, bytes: &[u8], compiled: Option<&[u8]>, imports: &mut [Import]) -> super::runtime::Result<H> {
        let fail = |e: wasmtime::Error| Error::Fail(e.root_cause().to_string());
        // SAFETY: a .cwasm comes from the binary, which build.rs made with
        // the same wasmtime; one that does not fit this Engine fails
        let module = match compiled.and_then(|c| unsafe { Module::deserialize(&self.engine, c) }.ok()) {
            Some(m) => m,
            None => Module::new(&self.engine, bytes).map_err(fail)?,
        };
        let decl: Vec<_> = module.imports().map(|i| (i.module().to_string(), i.name().to_string(), i.ty())).collect();
        if decl.len() != imports.len() {
            return Err(Error::Fail(format!("{} imports for {}", imports.len(), decl.len())));
        }
        // the memory the WASI functions work on
        let mem = decl.iter().zip(imports.iter()).find_map(|((_, _, t), i)| match (t, i) {
            (ExternType::Memory(_), Import::Handle(h)) => Some(*h),
            _ => None,
        });
        let need: Vec<(String, FuncType)> = decl.iter().zip(imports.iter()).filter_map(|((m, n, t), i)| match (m.as_str(), t, i) {
            ("wasi_snapshot_preview1", ExternType::Func(ft), Import::Default) => Some((n.clone(), ft.clone())),
            _ => None,
        }).collect();
        if !need.is_empty() {
            let mem = mem.ok_or_else(|| Error::Fail("WASI without a memory".into()))?;
            self.wasi(mem, &need).map_err(fail)?;
        }
        let mut externs = Vec::new();
        for ((m, n, t), i) in decl.iter().zip(imports.iter_mut()) {
            externs.push(match *i {
                Import::Handle(h) => self.get(h).cloned().ok_or_else(|| Error::Fail(format!("{m}.{n}: no handle {h}")))?,
                Import::I32 { value, mutable } => {
                    let ty = GlobalType::new(ValType::I32, if mutable { Mutability::Var } else { Mutability::Const });
                    let g = Global::new(&mut self.store, ty, Val::I32(value)).map_err(fail)?;
                    *i = Import::Handle(self.handle(g.into()));
                    g.into()
                }
                Import::Default => match (m.as_str(), n.as_str(), t) {
                    ("wasi_snapshot_preview1", _, _) => self.wasi[&(mem.unwrap(), n.clone())].into(),
                    ("env", "wasm_stack", _) => self.stack_func().into(),
                    _ => return Err(Error::Fail(format!("no default for {m}.{n}"))),
                },
            });
        }
        let inst = Instance::new(&mut self.store, &module, &externs).map_err(fail)?;
        self.handles.push(Item::Instance(inst));
        let h = self.handles.len() as H;
        self.instances.push((module, h));
        Ok(h)
    }

    fn export(&mut self, inst: H, name: &str) -> H {
        let Some(Item::Instance(i)) = inst.checked_sub(1).and_then(|k| self.handles.get(k as usize)).cloned() else { return 0 };
        match i.get_export(&mut self.store, name) {
            Some(e) => self.handle(e),
            None => 0,
        }
    }

    fn call(&mut self, func: H, sig: &str, vals: &mut [u64]) -> super::runtime::Result<()> {
        let Some(Extern::Func(f)) = self.get(func).cloned() else { return Err(Error::Fail(format!("no function {func}"))) };
        let (params, results) = sig.split_once(':').unwrap_or((sig, ""));
        let mut args = Vec::new();
        for (c, &v) in params.chars().zip(vals.iter()) {
            args.push(match c {
                'i' => Val::I32(v as i32),
                'j' => Val::I64(v as i64),
                'f' => Val::F32(v as u32),
                'd' => Val::F64(v),
                _ => match self.get(v as H) {
                    Some(Extern::Func(f)) => Val::FuncRef(Some(*f)),
                    _ => Val::FuncRef(None),
                },
            });
        }
        let mut res = vec![Val::I32(0); results.len()];
        // the epoch does not move until a Ctrl-C: armed at each call, one
        // stops the call it comes in
        self.store.set_epoch_deadline(1);
        if let Err(e) = f.call(&mut self.store, &args, &mut res) {
            return Err(self.error(e));
        }
        for (r, v) in res.iter().zip(vals.iter_mut()) {
            *v = match r {
                Val::I32(x) => *x as u32 as u64,
                Val::I64(x) => *x as u64,
                Val::F32(x) => *x as u64,
                Val::F64(x) => *x,
                Val::FuncRef(Some(f)) => self.handle((*f).into()) as u64,
                _ => 0,
            };
        }
        Ok(())
    }

    fn read(&mut self, mem: H, addr: u32, buf: &mut [u8]) -> bool {
        self.memory(mem).is_some_and(|m| m.read(&self.store, addr as usize, buf).is_ok())
    }

    fn write(&mut self, mem: H, addr: u32, data: &[u8]) -> bool {
        self.memory(mem).is_some_and(|m| m.write(&mut self.store, addr as usize, data).is_ok())
    }

    fn stack(&self, id: u32) -> Vec<Frame> {
        let st = &self.store.data().stacks;
        id.checked_sub(1).and_then(|i| st.traces.get(i as usize)).map(|bt| self.frames(bt)).unwrap_or_default()
    }
}

/// The engine Ctrl-C interrupts: the last runtime's ([`Wasmtime::on_ctrl_c`]).
static CTRL_C: std::sync::Mutex<Option<Engine>> = std::sync::Mutex::new(None);
static CTRL_C_ALSO: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

extern "C" fn on_sigint(_: libc::c_int) {
    if let Some(also) = CTRL_C_ALSO.get() {
        also();
    }
    // the main thread holds the lock only to put a new engine there
    if let Ok(e) = CTRL_C.try_lock() {
        if let Some(e) = e.as_ref() {
            e.increment_epoch();
        }
    }
}
