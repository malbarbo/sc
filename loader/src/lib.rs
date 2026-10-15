//! The REPL's loader: runs wasm shared objects (`wasm-ld -shared
//! --experimental-pic`) the way the REPL runs each input. `scload` runs them
//! from files, and `sc` runs the cells of its REPL.
//!
//! The first module is loaded with what its `dylink.0` section lists as
//! needed (libc.so, libc++.so...), looked up in the `-L` directories. The
//! entries after it are loaded one at a time on top of the ones before, as
//! REPL inputs: a definition in an entry shadows an older one with the same
//! name for the entries loaded after it. A weak definition (an inline
//! function, a template, a vtable, a typeinfo) does not: the first one is
//! the one for all modules. Each module runs its constructors
//! when it is loaded; at the end the last `main` defined runs and the
//! program exits through libc's `exit`.
//!
//! All modules share one memory, one function table and the stack pointer.
//! Memory for the first group comes from a bump allocator below the heap;
//! for the entries, from libc's `aligned_alloc`. A function import is bound
//! to the definition visible when its group is loaded, or, when there is
//! none yet (a library that calls one of its own preemptible symbols, a weak
//! undefined symbol, a function a later entry defines), to a stub that calls
//! through the symbol's table slot. GOT imports are mutable globals set once
//! the group is loaded. An exception tag is one for all
//! modules: the first module that defines it, or the loader when a module
//! imports it first, so a C++ exception or a longjmp crosses modules. WASI calls go through a shim module
//! that exports the memory, because wasmtime-wasi reads it from the caller.
//!
//! An entry may refer to what a later entry defines, as a declaration before
//! its definition in one file: a GOT.mem import is set when the symbol comes,
//! and a GOT.func import or a stub gets a table slot at once, which the
//! function fills when it comes (the same slot for all modules). Until then
//! a GOT.mem import is an address past the memory, so a read or a write
//! traps, and a call through the slot traps.

use wasmtime::error::Context;
use wasmtime::{bail, format_err, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use wasmparser::{Dylink0Subsection, KnownCustom, Parser, Payload, SymbolFlags};
use wasmtime::*;
use wasmtime_wasi::p1::{self, WasiP1Ctx};

pub const STACK: u32 = 1 << 20; // the default; the stack is first, so an overflow traps below 0
const PAGE: u32 = 1 << 16;
// A variable not defined yet has an address from here up, each its own MiB,
// above the most the memory grows to: reading or writing it traps, and
// [`Loader::undefined_at`] names it.
const UNDEFINED: u32 = 0xC000_0000;
const UNDEFINED_SIZE: u32 = 1 << 20;

#[derive(Clone, Copy)]
enum Sym {
    Func(Func),
    Data(u32), // an absolute address
    Tag(Tag),  // an exception tag: __cpp_exception (libc++abi), __c_longjmp (libsetjmp)
}

struct Host {
    wasi: WasiP1Ctx,
    syms: HashMap<String, (Sym, Option<u32>)>, // the symbol and its table slot
}

/// What the `dylink.0` section says.
#[derive(Default)]
struct Dylink {
    mem_size: u32,
    mem_align: u32,
    table_size: u32,
    table_align: u32,
    needed: Vec<String>,
    weak: HashSet<String>, // weak undefined symbols (the section names GOT imports by env)
    weak_def: HashSet<String>, // weak definitions (llvm/dylink-weak.patch)
}

struct Pending {
    name: String,
    module: Module,
    info: Dylink,
}

pub struct Loader {
    engine: Engine,
    store: Store<Host>,
    linker: Linker<Host>,
    memory: Memory,
    table: Table,
    sp: Global,
    wasi: HashMap<String, Func>,
    libdirs: Vec<PathBuf>,
    loaded: HashSet<String>,
    bump: Option<u32>, // the next free address, until the heap is set up
    later: Vec<(String, Global)>, // GOT.mem imports of symbols not defined yet
    undefined: Vec<String>,       // what the addresses from UNDEFINED stand for
    slots: HashMap<String, u32>,  // table slots of functions not defined yet
}

fn dylink(bytes: &[u8]) -> Result<Dylink> {
    let mut d = Dylink::default();
    for p in Parser::new(0).parse_all(bytes) {
        let Payload::CustomSection(c) = p? else { continue };
        let KnownCustom::Dylink0(r) = c.as_known() else { continue };
        for s in r {
            match s? {
                Dylink0Subsection::MemInfo(m) => {
                    (d.mem_size, d.mem_align) = (m.memory_size, m.memory_alignment);
                    (d.table_size, d.table_align) = (m.table_size, m.table_alignment);
                }
                Dylink0Subsection::Needed(n) => d.needed.extend(n.iter().map(|s| s.to_string())),
                Dylink0Subsection::ExportInfo(v) => {
                    for e in v.iter().filter(|e| e.flags.contains(SymbolFlags::BINDING_WEAK)) {
                        d.weak_def.insert(e.name.to_string());
                    }
                }
                Dylink0Subsection::ImportInfo(v) => {
                    for i in v.iter().filter(|i| i.flags.contains(SymbolFlags::BINDING_WEAK)) {
                        d.weak.insert(i.field.to_string());
                    }
                }
                _ => {}
            }
        }
        return Ok(d);
    }
    bail!("no dylink.0 section: not a shared object")
}

/// A symbol as the source names it: without the `.sc` of a static a REPL
/// cell exports for the cells after it, and demangled.
fn source_name(name: &str) -> String {
    let n = name.strip_suffix(".sc").unwrap_or(name);
    let demangled = || cpp_demangle::Symbol::new(n).ok()?.demangle().ok();
    n.starts_with("_Z").then(demangled).flatten().unwrap_or_else(|| n.to_string())
}

fn align(x: u32, a: u32) -> u32 {
    (x + a - 1) & !(a - 1)
}

/// The WAT `(param ..) (result ..)` of `ft`, and the `local.get`s that pass
/// its parameters on.
fn wat_sig(ft: &FuncType) -> (String, String) {
    let ty = |t: ValType| match t {
        ValType::I32 => "i32", ValType::I64 => "i64", ValType::F32 => "f32", ValType::F64 => "f64", _ => "externref",
    };
    let sig = format!("(param {}) (result {})", ft.params().map(ty).collect::<Vec<_>>().join(" "),
                      ft.results().map(ty).collect::<Vec<_>>().join(" "));
    let args = (0..ft.params().len()).map(|i| format!("local.get {i} ")).collect();
    (sig, args)
}

// symbols of the loading protocol, which stay out of the symbol table
const INTERNAL: &[&str] = &["__wasm_call_ctors", "__wasm_apply_data_relocs", "_initialize", "__wasm_init_memory"];

impl Loader {
    /// `interruptible`: the code running can be stopped ([`Loader::arm`]),
    /// at a small cost in every loop and call.
    pub fn new(wasi: WasiP1Ctx, libdirs: Vec<PathBuf>, stack: u32, interruptible: bool) -> Result<Loader> {
        let mut config = Config::new();
        config.epoch_interruption(interruptible);
        // a trap's frames with their file and line, from the modules' DWARF
        config.wasm_backtrace_details(WasmBacktraceDetails::Enable);
        config.wasm_backtrace_max_frames(std::num::NonZeroUsize::new(1 << 20));
        if let Ok(c) = Cache::from_file(None) {
            config.cache(Some(c));
        }
        let engine = Engine::new(&config)?;
        let mut store = Store::new(&engine, Host { wasi, syms: HashMap::new() });
        let memory = Memory::new(&mut store, MemoryType::new(stack.div_ceil(PAGE), Some(UNDEFINED / PAGE)))?;
        let table = Table::new(&mut store, TableType::new(RefType::FUNCREF, 1, None), Ref::Func(None))?;
        let sp = Global::new(&mut store, GlobalType::new(ValType::I32, Mutability::Var), Val::I32(stack as i32))?;
        let mut linker = Linker::new(&engine);
        p1::add_to_linker_sync(&mut linker, |h: &mut Host| &mut h.wasi)?;
        linker.define(&mut store, "env", "memory", memory)?;
        let mut l = Loader { engine, store, linker, memory, table, sp, wasi: HashMap::new(), libdirs,
                             loaded: HashSet::new(), bump: Some(stack), later: Vec::new(), undefined: Vec::new(),
                             slots: HashMap::new() };
        l.define("__stack_low", Sym::Data(16), false);
        l.define("__stack_high", Sym::Data(stack), false);
        Ok(l)
    }

    fn define(&mut self, name: &str, sym: Sym, shadow: bool) {
        let syms = &mut self.store.data_mut().syms;
        if shadow || !syms.contains_key(name) {
            syms.insert(name.to_string(), (sym, None));
        }
    }

    fn find(&self, lib: &str) -> Result<PathBuf> {
        self.libdirs.iter().map(|d| d.join(lib)).find(|p| p.exists())
            .ok_or_else(|| format_err!("{lib}: not found (use -L)"))
    }

    /// The module `name` and the libraries it needs, not loaded yet,
    /// dependencies first.
    fn collect(&self, name: &str, bytes: &[u8], seen: &mut HashSet<String>, out: &mut Vec<Pending>) -> Result<()> {
        if self.loaded.contains(name) || !seen.insert(name.to_string()) {
            return Ok(());
        }
        let info = dylink(bytes).with_context(|| name.to_string())?;
        for n in &info.needed {
            if self.loaded.contains(n) || seen.contains(n) {
                continue;
            }
            let p = self.find(n)?;
            let lib = std::fs::read(&p).with_context(|| format!("{}", p.display()))?;
            self.collect(n, &lib, seen, out)?;
        }
        let module = Module::new(&self.engine, bytes).with_context(|| name.to_string())?;
        out.push(Pending { name: name.to_string(), module, info });
        Ok(())
    }

    /// Calls the function `name` the modules define.
    pub fn call(&mut self, name: &str, args: &[Val]) -> Result<Option<Val>> {
        let Some((Sym::Func(f), _)) = self.store.data().syms.get(name).copied() else {
            bail!("undefined symbol: {}", source_name(name))
        };
        let mut res = vec![Val::I32(0); f.ty(&self.store).results().len()];
        f.call(&mut self.store, args, &mut res)?;
        Ok(res.into_iter().next())
    }

    /// `size` bytes of memory aligned to `align`.
    fn alloc(&mut self, size: u32, align_: u32) -> Result<u32> {
        let Some(b) = self.bump else {
            let r = self.call("aligned_alloc", &[Val::I32(align_ as i32), Val::I32(align(size.max(1), align_) as i32)])?;
            let p = r.and_then(|v| v.i32()).unwrap_or(0) as u32;
            if p == 0 {
                bail!("out of memory loading a module");
            }
            // the memory of a module starts zeroed (.bss)
            self.memory.data_mut(&mut self.store)[p as usize..(p + size) as usize].fill(0);
            return Ok(p);
        };
        let p = align(b, align_);
        self.bump = Some(p + size);
        self.reserve(p + size)?;
        Ok(p)
    }

    fn reserve(&mut self, end: u32) -> Result<()> {
        let have = self.memory.data_size(&self.store) as u64;
        if (end as u64) > have {
            self.memory.grow(&mut self.store, (end as u64 - have).div_ceil(PAGE as u64))?;
        }
        Ok(())
    }

    /// The WASI functions the modules import, through a shim that exports
    /// the memory.
    fn wasi_shim(&mut self, group: &[Pending]) -> Result<()> {
        let mut need: Vec<(String, FuncType)> = Vec::new();
        for p in group {
            for i in p.module.imports() {
                if let (true, ExternType::Func(ft)) = (i.module() == "wasi_snapshot_preview1", i.ty()) {
                    if !self.wasi.contains_key(i.name()) && !need.iter().any(|(n, _)| n == i.name()) {
                        need.push((i.name().to_string(), ft));
                    }
                }
            }
        }
        if need.is_empty() {
            return Ok(());
        }
        let (mut wat, mut funcs) = (String::from("(module (import \"env\" \"memory\" (memory 0)) (export \"memory\" (memory 0))\n"), String::new());
        for (k, (n, ft)) in need.iter().enumerate() {
            let (sig, args) = wat_sig(ft);
            wat += &format!("(import \"wasi_snapshot_preview1\" \"{n}\" (func $i{k} {sig}))\n");
            funcs += &format!("(func (export \"{n}\") {sig} {args} call $i{k})\n");
        }
        wat += &funcs;
        wat += ")";
        let m = Module::new(&self.engine, &wat)?;
        let inst = self.linker.instantiate(&mut self.store, &m)?;
        for (n, _) in need {
            let f = inst.get_func(&mut self.store, &n).unwrap();
            self.wasi.insert(n, f);
        }
        Ok(())
    }

    /// The address of the variable `name` until it is defined: 0 once the
    /// addresses run out.
    fn undefined_address(&mut self, name: &str) -> u32 {
        let k = match self.undefined.iter().position(|n| n == name) {
            Some(k) => k,
            None if self.undefined.len() < ((u32::MAX - UNDEFINED) / UNDEFINED_SIZE) as usize => {
                self.undefined.push(name.to_string());
                self.undefined.len() - 1
            }
            None => return 0,
        };
        UNDEFINED + k as u32 * UNDEFINED_SIZE
    }

    /// The variable not defined yet whose address `addr` is in, as the
    /// source names it.
    pub fn undefined_at(&self, addr: u64) -> Option<String> {
        let k = addr.checked_sub(UNDEFINED as u64)? / UNDEFINED_SIZE as u64;
        let n = self.undefined.get(k as usize)?;
        self.later.iter().any(|(l, _)| l == n).then(|| source_name(n))
    }

    /// The table slot reserved for a function not defined yet.
    fn reserve_slot(&mut self, name: &str, ft: Option<&FuncType>) -> Result<u32> {
        if let Some(&s) = self.slots.get(name) {
            return Ok(s);
        }
        // until the function comes, a call says which one is missing
        let init = ft.map(|ft| {
            let n = source_name(name);
            Func::new(&mut self.store, ft.clone(), move |_, _, _| bail!("undefined symbol: {n}"))
        });
        let s = self.table.grow(&mut self.store, 1, Ref::Func(init))? as u32;
        self.slots.insert(name.to_string(), s);
        Ok(s)
    }

    /// Stubs for the function imports of `module` with no definition yet:
    /// each calls through the symbol's table slot.
    fn stubs(&mut self, module: &Module) -> Result<HashMap<String, Func>> {
        let need: Vec<(String, FuncType)> = module.imports()
            .filter_map(|i| match (i.module(), i.ty()) {
                ("env", ExternType::Func(ft)) if !self.defines(i.name()) =>
                    Some((i.name().to_string(), ft)),
                _ => None,
            })
            .collect();
        if need.is_empty() {
            return Ok(HashMap::new());
        }
        let mut wat = String::from("(module (import \"env\" \"__indirect_function_table\" (table 0 funcref))\n");
        for (k, (n, ft)) in need.iter().enumerate() {
            let slot = self.reserve_slot(n, Some(ft))?;
            let (sig, args) = wat_sig(ft);
            wat += &format!("(type $t{k} (func {sig})) (func (export \"s{k}\") (type $t{k}) {args} i32.const {slot} call_indirect (type $t{k}))\n");
        }
        wat += ")";
        let m = Module::new(&self.engine, &wat)?;
        let inst = Instance::new(&mut self.store, &m, &[self.table.into()])?;
        let mut out = HashMap::new();
        for (k, (n, _)) in need.into_iter().enumerate() {
            out.insert(n, inst.get_func(&mut self.store, &format!("s{k}")).unwrap());
        }
        Ok(out)
    }

    /// The table slot of a function symbol, the same for every module.
    fn slot(&mut self, name: &str) -> Result<u32> {
        let (sym, slot) = self.store.data().syms[name];
        if let Some(s) = slot {
            return Ok(s);
        }
        let Sym::Func(f) = sym else { bail!("{name}: GOT.func of data") };
        let s = self.table.grow(&mut self.store, 1, Ref::Func(Some(f)))? as u32;
        self.store.data_mut().syms.get_mut(name).unwrap().1 = Some(s);
        Ok(s)
    }

    /// What the modules before referred to and the group just loaded defines.
    fn forward(&mut self) -> Result<()> {
        let syms = &self.store.data().syms;
        let slots: Vec<(String, u32, Func)> = self.slots.iter()
            .filter_map(|(n, &s)| match syms.get(n) { Some((Sym::Func(f), None)) => Some((n.clone(), s, *f)), _ => None })
            .collect();
        for (n, s, f) in slots {
            self.table.set(&mut self.store, s as u64, Ref::Func(Some(f)))?;
            self.store.data_mut().syms.get_mut(&n).unwrap().1 = Some(s);
            self.slots.remove(&n);
        }
        for (n, g) in std::mem::take(&mut self.later) {
            match self.store.data().syms.get(&n).copied() {
                Some((Sym::Data(a), _)) => g.set(&mut self.store, Val::I32(a as i32))?,
                _ => self.later.push((n, g)),
            }
        }
        Ok(())
    }

    /// Loads `path` and what it needs: see [`Loader::load_bytes`].
    pub fn load(&mut self, path: &Path, shadow: bool) -> Result<()> {
        let bytes = std::fs::read(path).with_context(|| format!("{}", path.display()))?;
        // entries are told apart by path, libraries by name
        self.load_bytes(&path.to_string_lossy(), &bytes, shadow)
    }

    /// Loads the module `name` and what it needs, and runs their
    /// constructors. `shadow`: its definitions replace the older ones (a REPL
    /// entry); otherwise the first definition wins.
    pub fn load_bytes(&mut self, name: &str, bytes: &[u8], shadow: bool) -> Result<()> {
        let mut group = Vec::new();
        self.collect(name, bytes, &mut HashSet::new(), &mut group)?;
        self.wasi_shim(&group)?;
        let mut got: Vec<(String, Global, bool, bool)> = Vec::new(); // name, global, func?, weak?
        let mut insts = Vec::new();
        for p in &group {
            let shadow = shadow && p.name == name;
            let mem_base = self.alloc(p.info.mem_size, 1 << p.info.mem_align)?;
            let size = self.table.size(&self.store) as u32;
            let table_base = align(size, 1 << p.info.table_align);
            self.table.grow(&mut self.store, (table_base + p.info.table_size - size) as u64, Ref::Func(None))?;
            let konst = GlobalType::new(ValType::I32, Mutability::Const);
            let stubs = self.stubs(&p.module)?;
            let mut imports: Vec<Extern> = Vec::new();
            for i in p.module.imports() {
                let (m, n) = (i.module(), i.name());
                let weak = p.info.weak.contains(n);
                imports.push(match (m, n, i.ty()) {
                    ("env", "memory", _) => self.memory.into(),
                    ("env", "__indirect_function_table", _) => self.table.into(),
                    ("env", "__stack_pointer", _) => self.sp.into(),
                    ("env", "__memory_base", _) => Global::new(&mut self.store, konst.clone(), Val::I32(mem_base as i32))?.into(),
                    ("env", "__table_base", _) => Global::new(&mut self.store, konst.clone(), Val::I32(table_base as i32))?.into(),
                    ("GOT.mem" | "GOT.func", _, ExternType::Global(_)) => {
                        let g = Global::new(&mut self.store, GlobalType::new(ValType::I32, Mutability::Var), Val::I32(0))?;
                        got.push((n.to_string(), g, m == "GOT.func", weak));
                        g.into()
                    }
                    ("wasi_snapshot_preview1", _, _) => self.wasi[n].into(),
                    ("env", _, ExternType::Func(_)) => match self.store.data().syms.get(n) {
                        Some((Sym::Func(f), _)) => (*f).into(),
                        _ => stubs[n].into(),
                    },
                    ("env", _, ExternType::Tag(tt)) => match self.store.data().syms.get(n) {
                        Some((Sym::Tag(t), _)) => (*t).into(),
                        _ => {
                            let t = Tag::new(&mut self.store, &tt)?;
                            self.define(n, Sym::Tag(t), false);
                            t.into()
                        }
                    },
                    _ => bail!("{}: unsupported import {m}.{n}", p.name),
                });
            }
            let inst = Instance::new(&mut self.store, &p.module, &imports).with_context(|| p.name.clone())?;
            let exports: Vec<(String, Extern)> = inst.exports(&mut self.store)
                .map(|e| (e.name().to_string(), e.into_extern())).collect();
            for (n, e) in exports {
                if INTERNAL.contains(&n.as_str()) {
                    continue;
                }
                // an inline function, a vtable or a typeinfo is one for all modules
                let shadow = shadow && !p.info.weak_def.contains(&n);
                match e {
                    Extern::Func(f) => self.define(&n, Sym::Func(f), shadow),
                    Extern::Tag(t) => self.define(&n, Sym::Tag(t), false),
                    Extern::Global(g) if g.ty(&self.store).mutability() == Mutability::Const => {
                        if let Val::I32(v) = g.get(&mut self.store) {
                            self.define(&n, Sym::Data(mem_base.wrapping_add(v as u32)), shadow);
                        }
                    }
                    _ => {}
                }
            }
            insts.push(inst);
        }
        self.forward()?;
        if let Some(b) = self.bump.take() {
            // the heap starts after the first group and has the rest of its page
            let base = align(b, 16);
            self.reserve(base + 1)?;
            let end = self.memory.data_size(&self.store) as u32;
            self.define("__heap_base", Sym::Data(base), false);
            self.define("__heap_end", Sym::Data(end), false);
        }
        for (n, g, func, weak) in got {
            let v = match self.store.data().syms.get(&n).copied() {
                Some((Sym::Data(a), _)) if !func => a,
                Some((Sym::Func(_), _)) if func => self.slot(&n)?,
                Some(_) => bail!("{n}: GOT.{} of a {}", if func { "func" } else { "mem" }, if func { "data" } else { "function" }),
                None if weak => 0,
                None if func => self.reserve_slot(&n, None)?,
                None => {
                    self.later.push((n.clone(), g));
                    self.undefined_address(&n)
                }
            };
            g.set(&mut self.store, Val::I32(v as i32))?;
        }
        for (p, inst) in group.iter().zip(insts) {
            self.loaded.insert(p.name.clone());
            for f in ["__wasm_apply_data_relocs", if inst.get_func(&mut self.store, "_initialize").is_some() {
                "_initialize" } else { "__wasm_call_ctors" }] {
                if let Some(f) = inst.get_func(&mut self.store, f) {
                    f.call(&mut self.store, &[], &mut [])?;
                }
            }
        }
        Ok(())
    }

    /// Runs `main` and exits through libc's `exit`.
    pub fn run(&mut self) -> Result<i32> {
        let syms = &self.store.data().syms;
        let ret = if syms.contains_key("__main_argc_argv") {
            self.call("__main_void", &[])?
        } else if syms.contains_key("__original_main") {
            self.call("__original_main", &[])?
        } else if syms.contains_key("main") {
            self.call("main", &[Val::I32(0), Val::I32(0)])?
        } else {
            None
        };
        let code = ret.and_then(|v| v.i32()).unwrap_or(0);
        self.call("exit", &[Val::I32(code)])?;
        Ok(code)
    }

    /// A C++ exception no one caught reaches the host: wasm has no search
    /// phase, so libc++abi never calls `std::terminate`. Do what it does on
    /// a native target: catch it and terminate, which prints its type and
    /// `what()` and aborts.
    pub fn uncaught(&mut self, e: Error) -> Result<i32> {
        let Some(exn) = self.store.take_pending_exception() else { return Err(e) };
        let tag = exn.tag(&mut self.store)?;
        if let Some((Sym::Tag(cpp), _)) = self.store.data().syms.get("__cpp_exception").copied() {
            if Tag::eq(&cpp, &tag, &self.store) {
                if let Val::I32(p) = exn.field(&mut self.store, 0)? {
                    self.call("__cxa_begin_catch", &[Val::I32(p)])?;
                    self.call("_ZSt9terminatev", &[])?;
                }
            }
        }
        Err(e)
    }

    /// Defines the function `name`, which the modules loaded after may call,
    /// in the host: `f` takes the memory and the arguments, and returns
    /// nothing.
    pub fn host(&mut self, name: &str, params: &[ValType], f: impl Fn(&[u8], &[Val]) + Send + Sync + 'static) {
        let ty = FuncType::new(&self.engine, params.iter().cloned(), []);
        let memory = self.memory;
        let func = Func::new(&mut self.store, ty, move |caller, args, _| {
            f(memory.data(&caller), args);
            Ok(())
        });
        self.define(name, Sym::Func(func), true);
    }

    /// Whether the modules define the function `name`.
    pub fn defines(&self, name: &str) -> bool {
        matches!(self.store.data().syms.get(name), Some((Sym::Func(_), _)))
    }

    /// The stack pointer, which a trap leaves where the code that trapped
    /// had it.
    pub fn stack_pointer(&mut self) -> Val {
        self.sp.get(&mut self.store)
    }

    pub fn set_stack_pointer(&mut self, v: Val) -> Result<()> {
        self.sp.set(&mut self.store, v)
    }

    /// From now on, an `increment_epoch` of the [`Loader::interrupter`] stops
    /// the code running with a trap (in a loader made `interruptible`).
    pub fn arm(&mut self) {
        self.store.set_epoch_deadline(1);
    }

    /// The engine, whose `increment_epoch` may be called from another thread
    /// or a signal handler.
    pub fn interrupter(&self) -> Engine {
        self.engine.clone()
    }
}
