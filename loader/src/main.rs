//! scload: runs wasm shared objects (`wasm-ld -shared --experimental-pic`)
//! the way the REPL will run each input.
//!
//! usage: scload [-L dir]... [--dir host::guest]... [--stack bytes] first.so [entry.so]... [-- args...]
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
//! to the definition visible when its group is loaded, or to a host
//! trampoline when there is none yet (a library that calls one of its own
//! preemptible symbols, a weak undefined symbol). GOT imports are mutable
//! globals set once the group is loaded. An exception tag is one for all
//! modules: the first module that defines it, or the loader when a module
//! imports it first, so a C++ exception or a longjmp crosses modules. WASI calls go through a shim module
//! that exports the memory, because wasmtime-wasi reads it from the caller.

use wasmtime::error::Context;
use wasmtime::{bail, format_err, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use wasmparser::{Dylink0Subsection, KnownCustom, Parser, Payload, SymbolFlags};
use wasmtime::*;
use wasmtime_wasi::p1::{self, WasiP1Ctx};
use wasmtime_wasi::{FsPerms, I32Exit, WasiCtxBuilder};

const STACK: u32 = 1 << 20; // the default; the stack is first, so an overflow traps below 0
const PAGE: u32 = 1 << 16;

#[derive(Clone, Copy)]
enum Sym {
    Func(Func),
    Data(u32), // an absolute address
    Tag(Tag),  // an exception tag: __cpp_exception (libc++abi), __c_longjmp (libsetjmp)
}

struct Host {
    wasi: WasiP1Ctx,
    syms: HashMap<String, (Sym, Option<u32>)>, // the symbol and its table slot
    tramps: Vec<(String, Option<Func>)>,        // the target of each trampoline
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

struct Loader {
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

fn align(x: u32, a: u32) -> u32 {
    (x + a - 1) & !(a - 1)
}

// symbols of the loading protocol, which stay out of the symbol table
const INTERNAL: &[&str] = &["__wasm_call_ctors", "__wasm_apply_data_relocs", "_initialize", "__wasm_init_memory"];

impl Loader {
    fn new(wasi: WasiP1Ctx, libdirs: Vec<PathBuf>, stack: u32) -> Result<Loader> {
        let mut config = Config::new();
        if let Ok(c) = Cache::from_file(None) {
            config.cache(Some(c));
        }
        let engine = Engine::new(&config)?;
        let mut store = Store::new(&engine, Host { wasi, syms: HashMap::new(), tramps: Vec::new() });
        let memory = Memory::new(&mut store, MemoryType::new(stack.div_ceil(PAGE), None))?;
        let table = Table::new(&mut store, TableType::new(RefType::FUNCREF, 1, None), Ref::Func(None))?;
        let sp = Global::new(&mut store, GlobalType::new(ValType::I32, Mutability::Var), Val::I32(stack as i32))?;
        let mut linker = Linker::new(&engine);
        p1::add_to_linker_sync(&mut linker, |h: &mut Host| &mut h.wasi)?;
        linker.define(&mut store, "env", "memory", memory)?;
        let mut l = Loader { engine, store, linker, memory, table, sp, wasi: HashMap::new(), libdirs,
                             loaded: HashSet::new(), bump: Some(stack) };
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

    /// `path` and the libraries it needs, not loaded yet, dependencies first.
    fn collect(&self, path: &Path, name: &str, seen: &mut HashSet<String>, out: &mut Vec<Pending>) -> Result<()> {
        if self.loaded.contains(name) || !seen.insert(name.to_string()) {
            return Ok(());
        }
        let bytes = std::fs::read(path).with_context(|| format!("{}", path.display()))?;
        let info = dylink(&bytes).with_context(|| format!("{}", path.display()))?;
        for n in &info.needed {
            let p = self.find(n)?;
            self.collect(&p, n, seen, out)?;
        }
        let module = Module::new(&self.engine, &bytes).with_context(|| format!("{}", path.display()))?;
        out.push(Pending { name: name.to_string(), module, info });
        Ok(())
    }

    fn call(&mut self, name: &str, args: &[Val]) -> Result<Option<Val>> {
        let Some((Sym::Func(f), _)) = self.store.data().syms.get(name).copied() else {
            bail!("undefined symbol: {name}")
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
        let ty = |t: &ValType| match t {
            ValType::I32 => "i32", ValType::I64 => "i64", ValType::F32 => "f32", ValType::F64 => "f64", _ => "?",
        };
        let (mut wat, mut funcs) = (String::from("(module (import \"env\" \"memory\" (memory 0)) (export \"memory\" (memory 0))\n"), String::new());
        for (k, (n, ft)) in need.iter().enumerate() {
            let sig = format!("(param {}) (result {})",
                              ft.params().map(|t| ty(&t)).collect::<Vec<_>>().join(" "),
                              ft.results().map(|t| ty(&t)).collect::<Vec<_>>().join(" "));
            let args: String = (0..ft.params().len()).map(|i| format!("local.get {i} ")).collect();
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

    fn trampoline(&mut self, name: &str, ft: FuncType) -> Func {
        let idx = self.store.data().tramps.len();
        self.store.data_mut().tramps.push((name.to_string(), None));
        Func::new(&mut self.store, ft, move |mut caller, params, results| {
            let h = caller.data();
            let f = match h.tramps[idx] {
                (_, Some(f)) => f,
                (ref n, None) => match h.syms.get(n) {
                    Some((Sym::Func(f), _)) => *f,
                    _ => bail!("undefined symbol: {n}"),
                },
            };
            f.call(&mut caller, params, results)
        })
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

    /// Loads `path` and what it needs. `shadow`: its definitions replace the
    /// older ones (a REPL entry); otherwise the first definition wins.
    fn load(&mut self, path: &Path, shadow: bool) -> Result<()> {
        let name = path.to_string_lossy().to_string(); // entries are told apart by path, libraries by name
        let mut group = Vec::new();
        self.collect(path, &name, &mut HashSet::new(), &mut group)?;
        self.wasi_shim(&group)?;
        let mut got: Vec<(String, Global, bool, bool)> = Vec::new(); // name, global, func?, weak?
        let tramp0 = self.store.data().tramps.len();
        let mut insts = Vec::new();
        for p in &group {
            let shadow = shadow && p.name == name;
            let mem_base = self.alloc(p.info.mem_size, 1 << p.info.mem_align)?;
            let size = self.table.size(&self.store) as u32;
            let table_base = align(size, 1 << p.info.table_align);
            self.table.grow(&mut self.store, (table_base + p.info.table_size - size) as u64, Ref::Func(None))?;
            let konst = GlobalType::new(ValType::I32, Mutability::Const);
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
                    ("env", _, ExternType::Func(ft)) => match self.store.data().syms.get(n) {
                        Some((Sym::Func(f), _)) => (*f).into(),
                        _ => self.trampoline(n, ft).into(),
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
                None => bail!("undefined symbol: {n}"),
            };
            g.set(&mut self.store, Val::I32(v as i32))?;
        }
        let h = self.store.data_mut();
        for k in tramp0..h.tramps.len() {
            if let Some((Sym::Func(f), _)) = h.syms.get(&h.tramps[k].0) {
                h.tramps[k].1 = Some(*f);
            }
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
    fn run(&mut self) -> Result<i32> {
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
    fn uncaught(&mut self, e: Error) -> Result<i32> {
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
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut libdirs, mut mods, mut dirs, mut prog_args) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut stack = STACK;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-L" => libdirs.push(PathBuf::from(args.next().expect("-L dir"))),
            "--dir" => dirs.push(args.next().expect("--dir host::guest")),
            "--stack" => stack = args.next().and_then(|s| s.parse().ok()).expect("--stack bytes"),
            "--" => { prog_args.extend(args.by_ref()); }
            _ if a.starts_with("-L") => libdirs.push(PathBuf::from(&a[2..])),
            _ => mods.push(PathBuf::from(a)),
        }
    }
    if mods.is_empty() {
        eprintln!("usage: scload [-L dir]... [--dir host::guest]... [--stack bytes] first.so [entry.so]... [-- args...]");
        std::process::exit(2);
    }
    let code = match run(libdirs, mods, dirs, prog_args, stack) {
        Ok(c) => c,
        Err(e) => match e.downcast_ref::<I32Exit>() {
            Some(x) => x.0,
            None => {
                eprintln!("scload: {e:?}");
                134
            }
        },
    };
    std::process::exit(code);
}

fn run(libdirs: Vec<PathBuf>, mods: Vec<PathBuf>, dirs: Vec<String>, args: Vec<String>, stack: u32) -> Result<i32> {
    let mut b = WasiCtxBuilder::new();
    b.inherit_stdio().inherit_env().arg(mods[0].to_string_lossy()).args(&args);
    for d in dirs {
        let (h, g) = d.split_once("::").unwrap_or((&d, &d));
        b.preopened_dir(h, g, FsPerms::ReadWrite)?;
    }
    let mut l = Loader::new(b.build_p1(), libdirs, stack)?;
    let r = (|| {
        for (i, m) in mods.iter().enumerate() {
            l.load(m, i > 0)?;
        }
        l.run()
    })();
    match r {
        Err(e) if e.is::<ThrownException>() => l.uncaught(e),
        r => r,
    }
}
