//! The REPL's loader: runs wasm shared objects (`wasm-ld -shared
//! --experimental-pic`) the way the REPL runs each input. `scload` runs them
//! from files, and `sc` runs the cells of its REPL.
//!
//! The first module is loaded with what its `dylink.0` section lists as
//! needed (libc.so, libc++.so...), which [`Libs`] finds: in the `-L`
//! directories for `scload`, in the pack for `sc`. The
//! entries after it are loaded one at a time on top of the ones before, as
//! REPL inputs: a definition in an entry shadows an older one with the same
//! name for the entries loaded after it. A weak definition (an inline
//! function, a template, a vtable, a typeinfo) does not: the first one is
//! the one for all modules. Each module runs its constructors
//! when it is loaded; at the end the last `main` defined runs and the
//! program exits through libc's `exit`.
//!
//! The loader asks the wasm runtime for little ([`Runtime`]: instantiate a
//! module, an export, a call, the memory; ANALISE.md §63), so that it runs
//! on wasmtime (`native::Wasmtime`) and inside sc.wasm (`host::Host`). The rest comes
//! from modules it makes ([`gen`]): the world has the memory, the function
//! table and the stack pointer that all modules share, and the functions
//! that grow and set them.
//!
//! Memory for the first group comes from a bump allocator below the heap;
//! for the entries, from libc's `aligned_alloc`. A function import is bound
//! to the definition visible when its group is loaded, or, when there is
//! none yet (a library that calls one of its own preemptible symbols, a weak
//! undefined symbol, a function a later entry defines), to a stub that calls
//! through the symbol's table slot. Each module is instantiated once, with
//! its GOT imports at the values the bytes give ([`Loader::load_bytes`],
//! ANALISE.md §60). An exception tag is one for all
//! modules: the first module that defines it, or the loader when a module
//! imports it first, so a C++ exception or a longjmp crosses modules.
//!
//! With ASan's runtime (libsc_rt, at the levels of `--safety` with checks),
//! the memory has its shadow after the stack, and the modules after it: see
//! [`SHADOW`]. The runtime puts its functions in place of libc's (the
//! allocator, the functions that write the program's memory) with the names
//! of wasm-ld's `--wrap`: an import of `malloc` binds to `__wrap_malloc`
//! when a module defines it, and an import of `__real_malloc` to `malloc`.
//! libc calls its own functions directly, so they stay libc's for it.
//!
//! An entry may refer to what a later entry defines, as a declaration before
//! its definition in one file: a GOT.mem import is set when the symbol comes,
//! and a GOT.func import or a stub gets a table slot at once, which the
//! function fills when it comes (the same slot for all modules). Until then
//! a GOT.mem import is an address past the memory, so a read or a write
//! traps, and a call through the slot traps ([`Loader::missing`]).

use std::borrow::Cow;
use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wasmparser::{Dylink0Subsection, KnownCustom, Parser, Payload, SymbolFlags, TypeRef};

#[cfg(feature = "native")]
pub mod config;
pub mod gen;
// the imports of sc.wasm's host: a native build would leave them undefined,
// which MSVC's linker refuses even unused
#[cfg(not(feature = "native"))]
pub mod host;
#[cfg(feature = "native")]
pub mod native;
pub mod runtime;

use runtime::{fail, FuncType, Import, Runtime, H};
pub use runtime::{Error, Frame, Result, Trap, TrapKind, ValType};

/// The runtime of a [`Loader`] when it does not say: wasmtime with the
/// native feature, else the host's primitives, as inside sc.wasm.
#[cfg(feature = "native")]
pub type Native = native::Wasmtime;
#[cfg(not(feature = "native"))]
pub type Native = host::Host;

pub const STACK: u32 = 1 << 20; // the default; the stack is first, so an overflow traps below 0
const PAGE: u32 = 1 << 16;
/// ASan's shadow memory: one byte for each 8 of the 4 GiB, from here
/// (`-asan-mapping-offset`, scripts/flags.py), after the stack. The memory
/// starts with all of it: a check of any address reads it.
pub const SHADOW: u32 = 0x11_0000;
const SHADOW_END: u32 = SHADOW + (1 << 29);
// A variable not defined yet has an address from here up, each its own MiB,
// above the most the memory grows to: reading or writing it traps, and
// [`Loader::undefined_at`] names it.
const UNDEFINED: u32 = 0xC000_0000;
const UNDEFINED_SIZE: u32 = 1 << 20;
/// The most groups of lost blocks [`Loader::leaks`] tells.
const LEAKS: u32 = 1 << 14;

/// A value of a call ([`Loader::call`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Val {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
}

impl Val {
    pub fn i32(self) -> Option<i32> {
        match self {
            Val::I32(x) => Some(x),
            _ => None,
        }
    }

    pub fn unwrap_i32(self) -> i32 {
        self.i32().expect("an i32")
    }

    fn bits(self) -> u64 {
        match self {
            Val::I32(x) => x as u32 as u64,
            Val::I64(x) => x as u64,
            Val::F32(x) => x.to_bits() as u64,
            Val::F64(x) => x.to_bits(),
        }
    }

    fn of(t: ValType, v: u64) -> Val {
        match t {
            ValType::I64 => Val::I64(v as i64),
            ValType::F32 => Val::F32(f32::from_bits(v as u32)),
            ValType::F64 => Val::F64(f64::from_bits(v)),
            _ => Val::I32(v as i32),
        }
    }
}

#[derive(Clone, Copy)]
enum Sym {
    Func(Option<H>, u32), // the function, None until its module is instantiated, and its type
    Data(u32), // an absolute address
    Tag(H),    // an exception tag: __cpp_exception (libc++abi), __c_longjmp (libsetjmp)
}

/// The data of a module, at an ASan level: where, and what was there
/// before its constructors ran, without the zeros at the end.
struct Data {
    start: u32,
    end: u32,
    image: Vec<u8>,
}

/// Blocks of libsc_rt's allocator that the program lost ([`Loader::leaks`]).
pub struct Leak {
    /// Where they were allocated; none if libsc_rt did not keep it.
    pub allocated: Option<Vec<Frame>>,
    pub bytes: u32,
    pub blocks: u32,
}

/// The value a REPL cell shows ([`Loader::show`]): the arguments of the
/// last `__sc_show_int` or `__sc_show_float`.
pub struct Shown {
    pub float: bool,
    pub ty: u32,
    pub kind: u32,
    /// The value: the i64 or the f64's bits.
    pub bits: u64,
}

#[derive(Clone)]
enum Kind {
    Func(FuncType),
    Global,
    Memory,
    Table,
    Tag(FuncType),
}

/// What a module says of itself: its `dylink.0` section, its imports and
/// its exports.
#[derive(Default)]
struct Info {
    mem_size: u32,
    mem_align: u32,
    table_size: u32,
    table_align: u32,
    needed: Vec<String>,
    weak: HashSet<String>, // weak undefined symbols (the section names GOT imports by env)
    weak_def: HashSet<String>, // weak definitions (llvm/dylink-weak.patch)
    consts: HashMap<String, Option<i32>>, // exported immutable i32 globals: the i32.const of their init
    imports: Vec<(String, String, Kind)>,
    exports: Vec<(String, Kind)>,
}

struct Pending<'a> {
    name: String,
    bytes: Cow<'a, [u8]>,
    compiled: Option<&'static [u8]>,
    info: Info,
    debug: Option<Debug>,
}

/// The world's handles ([`gen::world`]).
struct World {
    memory: H,
    table: H,
    sp: H,
    missing: H,
    funcs: HashMap<&'static str, (H, FuncType)>,
}

/// A symbol of the loader's and its table slot.
type Entry = (Sym, Option<u32>);

pub struct Loader<R: Runtime = Native> {
    rt: R,
    world: World,
    syms: HashMap<String, Entry>,              // the symbol and its table slot
    types: Vec<FuncType>,                      // of the function symbols
    libs: Libs,
    loaded: HashSet<String>,
    bump: Option<u32>,            // the next free address, until the heap is set up
    table_size: u32,
    later: Vec<(String, H)>,      // GOT.mem imports of symbols not defined yet
    undefined: Vec<String>,       // what the addresses from UNDEFINED stand for
    missing: Vec<String>,         // the functions not defined yet that a call may reach, from 1
    slots: HashMap<String, (u32, bool)>, // table slots of functions not defined yet: whether filled
    asan: bool,                   // the memory has ASan's shadow
    debug: Vec<Debug>,            // of the modules with DWARF
    data: Vec<Data>,
    journal: Option<Vec<(String, Option<Entry>)>>, // what a group being defined replaced
    catchers: HashMap<(String, H), H>, // gen::catcher's try, by signature and tag
    scratch: Option<u32>,         // the table slot of the function a catcher calls
}

/// Where the loader finds the libraries a module needs: by name.
pub type Libs = Box<dyn Fn(&str) -> Option<Lib> + Send + Sync>;

/// A library: its module and, if there is one, the module compiled for
/// this loader's runtime ([`config::config`]), which saves compiling it.
pub struct Lib {
    pub wasm: Cow<'static, [u8]>,
    pub cwasm: Option<&'static [u8]>,
}

/// The libraries in `dirs`, from the first one that has each.
pub fn libdirs(dirs: Vec<PathBuf>) -> Libs {
    Box::new(move |lib| {
        let wasm = dirs.iter().find_map(|d| std::fs::read(d.join(lib)).ok())?;
        Some(Lib { wasm: Cow::Owned(wasm), cwasm: None })
    })
}

type Dwarf = gimli::EndianArcSlice<gimli::LittleEndian>;

/// The DWARF of a module, for the file and line of its frames
/// ([`Loader::symbols`]). wasmtime's (`wasm_backtrace_details`) parses it
/// again at each frame of each backtrace, which made keeping the stacks of
/// ASan's blocks hundreds of times slower.
struct Debug {
    instance: H,
    code: u64, // where the code section's contents start, which DWARF's addresses count from
    sections: HashMap<String, Arc<[u8]>>,
    cx: OnceCell<Option<addr2line::Context<Dwarf>>>, // made at the first frame
}

/// A frame's function (an inlined one first), file and line, from DWARF.
pub struct Symbol {
    pub name: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

fn debug(bytes: &[u8]) -> Result<Option<Debug>> {
    let (mut code, mut sections) = (0, HashMap::new());
    for p in Parser::new(0).parse_all(bytes) {
        match p? {
            Payload::CodeSectionStart { range, .. } => code = range.start as u64,
            Payload::CustomSection(c) if c.name().starts_with(".debug_") => {
                sections.insert(c.name().to_string(), Arc::from(c.data()));
            }
            _ => {}
        }
    }
    Ok((!sections.is_empty()).then(|| Debug { instance: 0, code, sections, cx: OnceCell::new() }))
}

impl From<wasmparser::BinaryReaderError> for Error {
    fn from(e: wasmparser::BinaryReaderError) -> Error {
        Error::Fail(e.message().to_string())
    }
}

fn val_type(t: wasmparser::ValType) -> Result<ValType> {
    use wasmparser::ValType as V;
    Ok(match t {
        V::I32 => ValType::I32,
        V::I64 => ValType::I64,
        V::F32 => ValType::F32,
        V::F64 => ValType::F64,
        V::Ref(r) if r.is_func_ref() => ValType::FuncRef,
        _ => fail!("a value of type {t}"),
    })
}

fn info(bytes: &[u8]) -> Result<Info> {
    let mut d = Info::default();
    let mut found = false;
    let mut types: Vec<Option<FuncType>> = Vec::new(); // None: of a value the loader does not pass (exnref)
    let mut funcs: Vec<u32> = Vec::new(); // the type of each function
    let mut tags: Vec<u32> = Vec::new();
    let mut globals: Vec<Option<Option<i32>>> = Vec::new(); // by index: None if imported or mutable
    let ty = |types: &[Option<FuncType>], k: u32| types.get(k as usize).cloned().flatten()
        .ok_or_else(|| Error::Fail("an import or export of a type the loader does not pass".into()));
    for p in Parser::new(0).parse_all(bytes) {
        let c = match p? {
            Payload::TypeSection(r) => {
                for t in r.into_iter_err_on_gc_types() {
                    let t = t?;
                    let params = t.params().iter().map(|&p| val_type(p)).collect::<Result<Vec<_>>>();
                    let results = t.results().iter().map(|&p| val_type(p)).collect::<Result<Vec<_>>>();
                    types.push(params.and_then(|p| Ok(FuncType::new(p, results?))).ok());
                }
                continue;
            }
            Payload::ImportSection(r) => {
                for i in r.into_imports() {
                    let i = i?;
                    let kind = match i.ty {
                        TypeRef::Func(k) => {
                            funcs.push(k);
                            Kind::Func(ty(&types, k)?)
                        }
                        TypeRef::Global(_) => {
                            globals.push(None);
                            Kind::Global
                        }
                        TypeRef::Memory(_) => Kind::Memory,
                        TypeRef::Table(_) => Kind::Table,
                        TypeRef::Tag(t) => {
                            tags.push(t.func_type_idx);
                            Kind::Tag(ty(&types, t.func_type_idx)?)
                        }
                        _ => fail!("an import {}.{}", i.module, i.name),
                    };
                    d.imports.push((i.module.to_string(), i.name.to_string(), kind));
                }
                continue;
            }
            Payload::FunctionSection(r) => {
                for f in r {
                    funcs.push(f?);
                }
                continue;
            }
            Payload::TagSection(r) => {
                for t in r {
                    tags.push(t?.func_type_idx);
                }
                continue;
            }
            Payload::GlobalSection(r) => {
                for g in r {
                    let g = g?;
                    let mut ops = g.init_expr.get_operators_reader();
                    let v = match (ops.read()?, ops.read()?) {
                        (wasmparser::Operator::I32Const { value }, wasmparser::Operator::End) => Some(value),
                        _ => None,
                    };
                    let konst = !g.ty.mutable && g.ty.content_type == wasmparser::ValType::I32;
                    globals.push(konst.then_some(v));
                }
                continue;
            }
            Payload::ExportSection(r) => {
                for e in r {
                    let e = e?;
                    let kind = match e.kind {
                        wasmparser::ExternalKind::Func => Kind::Func(ty(&types, funcs[e.index as usize])?),
                        wasmparser::ExternalKind::Global => {
                            if let Some(Some(v)) = globals.get(e.index as usize) {
                                d.consts.insert(e.name.to_string(), *v);
                            }
                            Kind::Global
                        }
                        wasmparser::ExternalKind::Tag => Kind::Tag(ty(&types, tags[e.index as usize])?),
                        wasmparser::ExternalKind::Memory => Kind::Memory,
                        _ => Kind::Table,
                    };
                    d.exports.push((e.name.to_string(), kind));
                }
                continue;
            }
            Payload::CustomSection(c) => c,
            _ => continue,
        };
        let KnownCustom::Dylink0(r) = c.as_known() else { continue };
        found = true;
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
    }
    if !found {
        fail!("no dylink.0 section: not a shared object");
    }
    Ok(d)
}

/// A symbol as the source names it: without the `.sc` of a static a REPL
/// cell exports for the cells after it, and demangled.
/// A function as the source names it, from a symbol (`_Z1fi.sc`) or from
/// wasm-ld's demangled name (`f(int) (.sc)`): demangled, `main` for libc's
/// names of it, without the `.sc` of a static of a REPL cell, the function
/// a REPL statement is in (`__stmts_N()::`), libc++'s inline namespace and
/// ABI tags. A lambda is one in the function around it, and libsc_rt's
/// `__wrap_strlen` is the `strlen` it checks.
pub fn source_name(name: &str) -> String {
    let n = name.strip_suffix(" (.sc)").or(name.strip_suffix(".sc")).unwrap_or(name);
    let n = n.strip_prefix("__wrap_").unwrap_or(n);
    let demangled = || cpp_demangle::Symbol::new(n).ok()?.demangle().ok();
    let n = n.starts_with("_Z").then(demangled).flatten().unwrap_or_else(|| n.to_string());
    let n = match n.strip_prefix("__stmts_").and_then(|r| r.split_once("()::")) {
        Some((_, rest)) => rest,
        None => &n,
    };
    match n {
        "__original_main" | "__main_argc_argv" => "main".into(),
        _ if n.contains("::$_") => format!("a lambda in {}", source_name(&n[..n.find("::$_").unwrap()])),
        _ => n.replace("std::__2::", "std::").split("[abi:").enumerate()
            .map(|(k, p)| if k == 0 { p } else { p.split_once(']').map_or("", |x| x.1) })
            .collect(),
    }
}

fn align(x: u32, a: u32) -> u32 {
    (x + a - 1) & !(a - 1)
}

// symbols of the loading protocol, which stay out of the symbol table
const INTERNAL: &[&str] = &["__wasm_call_ctors", "__wasm_apply_data_relocs", "_initialize", "__wasm_init_memory"];

/// The world's functions, with their types ([`gen::world`]).
fn world_funcs() -> Vec<(&'static str, FuncType)> {
    use ValType::*;
    let f = |p: &[ValType], r: &[ValType]| FuncType::new(p.iter().copied(), r.iter().copied());
    vec![
        ("grow", f(&[I32], &[I32])), ("size", f(&[], &[I32])), ("table_grow", f(&[I32], &[I32])),
        ("table_set", f(&[I32, FuncRef], &[])), ("get_sp", f(&[], &[I32])), ("set_sp", f(&[I32], &[])),
        ("take_missing", f(&[], &[I32])), ("take_blamed", f(&[], &[I32, I32])), ("take_shown", f(&[], &[I32, I32, I32, I64])),
        ("set_data", f(&[I32, I32], &[])), ("set_leaks", f(&[I32, I32], &[])), ("leak_count", f(&[], &[I32])),
        ("__sc_rt_stack", f(&[], &[I32])), ("__sc_rt_blame", f(&[I32, I32], &[])), ("__sc_rt_data", f(&[I32], &[I64])),
        ("__sc_rt_leak", f(&[I32, I32, I32], &[])), ("__sc_show_int", f(&[I32, I32, I64], &[])),
        ("__sc_show_float", f(&[I32, I32, F64], &[])),
    ]
}

impl<R: Runtime> Loader<R> {
    pub fn new(mut rt: R, libs: Libs, stack: u32) -> Result<Loader<R>> {
        let bytes = gen::world(stack.div_ceil(PAGE) as u64, (UNDEFINED / PAGE) as u64, stack);
        let inst = rt.instantiate(&bytes, None, &mut [Import::Default])?;
        let mut funcs = HashMap::new();
        for (n, t) in world_funcs() {
            funcs.insert(n, (rt.export(inst, n), t));
        }
        let world = World {
            memory: rt.export(inst, "memory"),
            table: rt.export(inst, "table"),
            sp: rt.export(inst, "__stack_pointer"),
            missing: rt.export(inst, "missing"),
            funcs,
        };
        let mut l = Loader { rt, world, syms: HashMap::new(), types: Vec::new(), libs, loaded: HashSet::new(),
                             bump: Some(stack), table_size: 1, later: Vec::new(), undefined: Vec::new(),
                             missing: Vec::new(), slots: HashMap::new(), asan: false, debug: Vec::new(),
                             data: Vec::new(), journal: None, catchers: HashMap::new(), scratch: None };
        l.define("__stack_low", Sym::Data(16), false);
        l.define("__stack_high", Sym::Data(stack), false);
        Ok(l)
    }

    /// The runtime the loader runs on.
    pub fn runtime(&mut self) -> &mut R {
        &mut self.rt
    }

    fn ty(&mut self, ft: &FuncType) -> u32 {
        match self.types.iter().position(|t| t == ft) {
            Some(k) => k as u32,
            None => {
                self.types.push(ft.clone());
                self.types.len() as u32 - 1
            }
        }
    }

    /// Whether `name` is now `sym`: not if an older definition stays.
    fn define(&mut self, name: &str, sym: Sym, shadow: bool) -> bool {
        if !shadow && self.syms.contains_key(name) {
            return false;
        }
        let old = self.syms.insert(name.to_string(), (sym, None));
        if let Some(j) = &mut self.journal {
            j.push((name.to_string(), old));
        }
        true
    }

    /// Calls `f`, of type `ty`, with `args`: its results.
    fn invoke(&mut self, f: H, ty: &FuncType, args: &[u64]) -> Result<Vec<u64>> {
        let mut vals = args.to_vec();
        vals.resize(ty.params.len().max(ty.results.len()), 0);
        self.rt.call(f, &ty.sig(), &mut vals)?;
        vals.truncate(ty.results.len());
        Ok(vals)
    }

    /// Calls the world's function `name`.
    fn world(&mut self, name: &str, args: &[u64]) -> Result<Vec<u64>> {
        let (f, ty) = self.world.funcs[name].clone();
        self.invoke(f, &ty, args)
    }

    /// The function `name` the modules define, and its type.
    fn func(&self, name: &str) -> Result<(H, FuncType)> {
        match self.syms.get(name) {
            Some((Sym::Func(Some(f), t), _)) => Ok((*f, self.types[*t as usize].clone())),
            _ => fail!("undefined symbol: {}", source_name(name)),
        }
    }

    /// Calls the function `name` the modules define.
    pub fn call(&mut self, name: &str, args: &[Val]) -> Result<Option<Val>> {
        let (f, ty) = self.func(name)?;
        let r = self.invoke(f, &ty, &args.iter().map(|a| a.bits()).collect::<Vec<_>>())?;
        Ok(ty.results.first().map(|&t| Val::of(t, r[0])))
    }

    /// Calls `f` as [`Loader::invoke`] does, but a C++ exception that no one
    /// catches does what libc++abi does on a native target: wasm has no
    /// search phase, so it never calls `std::terminate`. It is caught and
    /// terminates, which prints its type and `what()` and aborts.
    fn guarded(&mut self, f: H, ty: &FuncType, args: &[u64]) -> Result<Vec<u64>> {
        let Some((Sym::Tag(tag), _)) = self.syms.get("__cpp_exception").copied() else { return self.invoke(f, ty, args) };
        let key = (ty.sig(), tag);
        let catcher = match self.catchers.get(&key) {
            Some(&c) => c,
            None => {
                let mut imports = [Import::Handle(self.world.table), Import::Handle(tag)];
                let inst = self.rt.instantiate(&gen::catcher(ty), None, &mut imports)?;
                let c = self.rt.export(inst, "try");
                self.catchers.insert(key, c);
                c
            }
        };
        let scratch = match self.scratch {
            Some(s) => s,
            None => {
                let s = self.table_grow(1)?;
                self.scratch = Some(s);
                s
            }
        };
        self.world("table_set", &[scratch as u64, f as u64])?;
        let mut params = ty.params.clone();
        params.push(ValType::I32);
        let mut results = vec![ValType::I32];
        results.extend(&ty.results);
        let mut vals = args.to_vec();
        vals.push(scratch as u64);
        let r = self.invoke(catcher, &FuncType::new(params, results), &vals)?;
        if r[0] != 0 {
            self.call("__cxa_begin_catch", &[Val::I32(r[0] as i32)])?;
            self.call("_ZSt9terminatev", &[])?;
            fail!("std::terminate returned");
        }
        Ok(r[1..].to_vec())
    }

    /// The module `name` and the libraries it needs, not loaded yet,
    /// dependencies first.
    fn collect<'a>(&self, name: &str, bytes: Cow<'a, [u8]>, compiled: Option<&'static [u8]>, seen: &mut HashSet<String>,
                   out: &mut Vec<Pending<'a>>) -> Result<()> {
        if self.loaded.contains(name) || !seen.insert(name.to_string()) {
            return Ok(());
        }
        let info = info(&bytes)?;
        for n in &info.needed {
            if self.loaded.contains(n) || seen.contains(n) {
                continue;
            }
            let Some(lib) = (self.libs)(n) else { fail!("{n}: not found (use -L)") };
            self.collect(n, lib.wasm, lib.cwasm, seen, out)?;
        }
        let debug = debug(&bytes)?;
        out.push(Pending { name: name.to_string(), bytes, compiled, info, debug });
        Ok(())
    }

    /// The symbol an import of `name` binds to: `__wrap_name` when a module
    /// defines it, `name` for `__real_name` (wasm-ld's `--wrap`).
    fn target(&self, name: &str) -> String {
        if let Some(n) = name.strip_prefix("__real_") {
            return n.to_string();
        }
        let wrap = format!("__wrap_{name}");
        if self.syms.contains_key(&wrap) { wrap } else { name.to_string() }
    }

    /// The size of the memory, in bytes.
    fn memory_size(&mut self) -> Result<u64> {
        Ok(self.world("size", &[])?[0] * PAGE as u64)
    }

    /// `size` bytes of memory aligned to `align`.
    fn alloc(&mut self, size: u32, align_: u32) -> Result<u32> {
        let Some(b) = self.bump else {
            let r = self.call("aligned_alloc", &[Val::I32(align_ as i32), Val::I32(align(size.max(1), align_) as i32)])?;
            let p = r.and_then(|v| v.i32()).unwrap_or(0) as u32;
            if p == 0 {
                fail!("out of memory loading a module");
            }
            // the memory of a module starts zeroed (.bss)
            self.rt.write(self.world.memory, p, &vec![0; size as usize]);
            return Ok(p);
        };
        let p = align(b, align_);
        self.bump = Some(p + size);
        self.reserve(p + size)?;
        Ok(p)
    }

    fn reserve(&mut self, end: u32) -> Result<()> {
        let have = self.memory_size()?;
        if (end as u64) > have {
            let r = self.world("grow", &[(end as u64 - have).div_ceil(PAGE as u64)])?;
            if r[0] as u32 == u32::MAX {
                fail!("out of memory loading a module");
            }
        }
        Ok(())
    }

    /// Grows the table by `n` null slots: the first of them.
    fn table_grow(&mut self, n: u32) -> Result<u32> {
        let r = self.world("table_grow", &[n as u64])?[0] as u32;
        if r == u32::MAX {
            fail!("the function table cannot grow");
        }
        self.table_size = r + n;
        Ok(r)
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

    /// The function not defined yet whose call stopped the code, as the
    /// source names it, if one did since the last time.
    pub fn missing(&mut self) -> Option<String> {
        let id = self.world("take_missing", &[]).ok()?[0] as usize;
        id.checked_sub(1).and_then(|k| self.missing.get(k)).cloned()
    }

    /// `e`, or, for a trap of a call to a function not defined yet, the
    /// failure that names it, as `scload` says it.
    pub fn why(&mut self, e: Error) -> Error {
        match (&e, self.missing()) {
            (Error::Trap(_), Some(n)) => Error::Fail(format!("undefined symbol: {n}")),
            _ => e,
        }
    }

    /// The table slot reserved for a function not defined yet, and whether
    /// it is to be filled now: with its type, if known (a call, not only an
    /// address), a function that says which one is missing.
    fn reserve_slot(&mut self, name: &str, ft: bool) -> Result<(u32, bool)> {
        if let Some(&(s, filled)) = self.slots.get(name) {
            // reserved by an address (GOT.func), which left it null
            if ft && !filled {
                self.slots.insert(name.to_string(), (s, true));
                return Ok((s, true));
            }
            return Ok((s, false));
        }
        let s = self.table_grow(1)?;
        self.slots.insert(name.to_string(), (s, ft));
        Ok((s, ft))
    }

    /// Stubs for the function imports of a module with no function yet:
    /// each calls through the symbol's table slot, which a module of the
    /// group fills once instantiated, or a later one.
    fn stubs(&mut self, imports: &[(String, String, Kind)]) -> Result<HashMap<String, H>> {
        let mut need: Vec<(String, FuncType, u32)> = Vec::new();
        let mut undefined: Vec<(FuncType, u32, u32)> = Vec::new(); // the function's slot and number
        for (m, n, k) in imports {
            let Kind::Func(ft) = k else { continue };
            let t = self.target(n);
            if m != "env" || self.defines(&t) {
                continue;
            }
            let slot = if matches!(self.syms.get(&t), Some((Sym::Func(None, _), _))) {
                self.slot(&t)?
            } else {
                let (s, fill) = self.reserve_slot(&t, true)?;
                if fill {
                    let id = match self.missing.iter().position(|x| *x == source_name(&t)) {
                        Some(k) => k,
                        None => {
                            self.missing.push(source_name(&t));
                            self.missing.len() - 1
                        }
                    };
                    undefined.push((ft.clone(), s, id as u32 + 1));
                }
                s
            };
            need.push((n.clone(), ft.clone(), slot));
        }
        if need.is_empty() {
            return Ok(HashMap::new());
        }
        let s: Vec<_> = need.iter().map(|(_, t, s)| (t, *s)).collect();
        let u: Vec<_> = undefined.iter().map(|(t, _, id)| (t, *id)).collect();
        let mut imports = [Import::Handle(self.world.table), Import::Handle(self.world.missing)];
        let inst = self.rt.instantiate(&gen::stubs(&s, &u), None, &mut imports)?;
        for (k, (_, slot, _)) in undefined.iter().enumerate() {
            let f = self.rt.export(inst, &format!("u{k}"));
            self.world("table_set", &[*slot as u64, f as u64])?;
        }
        let mut out = HashMap::new();
        for (k, (n, _, _)) in need.into_iter().enumerate() {
            out.insert(n, self.rt.export(inst, &format!("s{k}")));
        }
        Ok(out)
    }

    /// The table slot of a function symbol, the same for every module.
    fn slot(&mut self, name: &str) -> Result<u32> {
        let (sym, slot) = self.syms[name];
        if let Some(s) = slot {
            return Ok(s);
        }
        let Sym::Func(f, _) = sym else { fail!("{name}: GOT.func of data") };
        let s = self.table_grow(1)?;
        if let Some(f) = f {
            self.world("table_set", &[s as u64, f as u64])?;
        }
        self.syms.get_mut(name).unwrap().1 = Some(s);
        Ok(s)
    }

    /// The slots reserved for functions now defined become theirs: a
    /// function of a group being loaded fills its slot once its module is
    /// instantiated. Returns the slots taken.
    fn forward(&mut self) -> Result<Vec<(String, (u32, bool))>> {
        let slots: Vec<(String, (u32, bool), Option<H>)> = self.slots.iter()
            .filter_map(|(n, &s)| match self.syms.get(n) { Some((Sym::Func(f, _), None)) => Some((n.clone(), s, *f)), _ => None })
            .collect();
        let mut taken = Vec::new();
        for (n, s, f) in slots {
            if let Some(f) = f {
                self.world("table_set", &[s.0 as u64, f as u64])?;
            }
            self.syms.get_mut(&n).unwrap().1 = Some(s.0);
            self.slots.remove(&n);
            taken.push((n, s));
        }
        Ok(taken)
    }

    /// The GOT.mem imports of variables the group just loaded defines.
    fn resolve_later(&mut self) -> Result<()> {
        let mut set = Vec::new();
        for (n, g) in std::mem::take(&mut self.later) {
            match self.syms.get(&n).copied() {
                Some((Sym::Data(a), _)) => set.push((g, a as i32)),
                _ => self.later.push((n, g)),
            }
        }
        if set.is_empty() {
            return Ok(());
        }
        let values: Vec<i32> = set.iter().map(|s| s.1).collect();
        let mut imports: Vec<Import> = set.iter().map(|s| Import::Handle(s.0)).collect();
        let inst = self.rt.instantiate(&gen::setter(&values), None, &mut imports)?;
        let f = self.rt.export(inst, "set");
        self.invoke(f, &FuncType::new([], []), &[])?;
        Ok(())
    }

    /// Loads `path` and what it needs: see [`Loader::load_bytes`].
    pub fn load(&mut self, path: &Path, shadow: bool) -> Result<()> {
        let bytes = std::fs::read(path).map_err(|e| Error::Fail(format!("{}: {e}", path.display())))?;
        // entries are told apart by path, libraries by name
        self.load_bytes(&path.to_string_lossy(), &bytes, shadow)
    }

    /// Loads the module `name` and what it needs, and runs their
    /// constructors. `shadow`: its definitions replace the older ones (a REPL
    /// entry); otherwise the first definition wins.
    ///
    /// Each module is instantiated once, with every import at its value
    /// (ANALISE.md §60): the bytes give the layout, the data symbols and
    /// the functions the group defines, whose slots are filled once their
    /// module is instantiated. Only the GOT.mem of a variable not defined
    /// yet is set later, when a module defines it. If a module fails to
    /// instantiate, what the group defined is undone.
    pub fn load_bytes(&mut self, name: &str, bytes: &[u8], shadow: bool) -> Result<()> {
        let mut group = Vec::new();
        self.collect(name, Cow::Borrowed(bytes), None, &mut HashSet::new(), &mut group)?;
        if group.iter().any(|p| p.info.exports.iter().any(|(n, _)| n == "__asan_init")) && !self.asan {
            // the shadow, after the stack: the first group has the memory to itself
            match self.bump {
                Some(b) if b <= SHADOW => self.bump = Some(SHADOW_END),
                _ => fail!("ASan's runtime comes after the first modules, or the stack is over {SHADOW} bytes"),
            }
            self.reserve(SHADOW_END)?;
            self.asan = true;
        }
        self.journal = Some(Vec::new());
        let mut taken = Vec::new();
        let r = self.instantiate(name, &mut group, shadow, &mut taken);
        let journal = self.journal.take().unwrap_or_default();
        let insts = match r {
            Ok(insts) => insts,
            Err(e) => {
                // the slots the group took are reserved again, for whatever defines them
                for (n, old) in journal.into_iter().rev() {
                    match old {
                        Some(o) => self.syms.insert(n, o),
                        None => self.syms.remove(&n),
                    };
                }
                self.slots.extend(taken);
                return Err(e);
            }
        };
        self.resolve_later()?;
        for (p, (inst, base)) in group.iter().zip(insts) {
            self.loaded.insert(p.name.clone());
            let relocs = self.rt.export(inst, "__wasm_apply_data_relocs");
            if relocs != 0 {
                self.invoke(relocs, &FuncType::new([], []), &[])?;
            }
            self.image(base)?;
            let init = match self.rt.export(inst, "_initialize") {
                0 => self.rt.export(inst, "__wasm_call_ctors"),
                f => f,
            };
            if init != 0 {
                self.guarded(init, &FuncType::new([], []), &[])?;
            }
        }
        Ok(())
    }

    /// The group's symbols from the bytes, then each module instantiated:
    /// the instances and their memory bases. `taken`: the reserved slots
    /// the group's functions took.
    fn instantiate(&mut self, name: &str, group: &mut [Pending], shadow: bool, taken: &mut Vec<(String, (u32, bool))>)
                   -> Result<Vec<(H, u32)>> {
        let mut bases = Vec::new();
        let mut owner: HashMap<String, usize> = HashMap::new(); // the module of each function defined
        for (k, p) in group.iter().enumerate() {
            let shadow = shadow && p.name == name;
            let mem_base = self.alloc(p.info.mem_size, 1 << p.info.mem_align)?;
            if p.info.mem_size > 0 && self.asan {
                self.data.push(Data { start: mem_base, end: mem_base + p.info.mem_size, image: Vec::new() });
            }
            let table_base = align(self.table_size, 1 << p.info.table_align);
            let grow = table_base + p.info.table_size - self.table_size;
            if grow > 0 {
                self.table_grow(grow)?;
            }
            bases.push((mem_base, table_base));
            for (n, kind) in &p.info.exports {
                if INTERNAL.contains(&n.as_str()) {
                    continue;
                }
                // an inline function, a vtable or a typeinfo is one for all modules
                let shadow = shadow && !p.info.weak_def.contains(n);
                match kind {
                    Kind::Func(ft) => {
                        let t = self.ty(ft);
                        if self.define(n, Sym::Func(None, t), shadow) {
                            owner.insert(n.to_string(), k);
                        }
                    }
                    Kind::Global => match p.info.consts.get(n) {
                        Some(Some(v)) => { self.define(n, Sym::Data(mem_base.wrapping_add(*v as u32)), shadow); }
                        Some(None) => fail!("{}: {n}, a global that is not a constant", p.name),
                        None => {}
                    },
                    _ => {}
                }
            }
        }
        if let Some(b) = self.bump.take() {
            // the heap starts after the first group and has the rest of its page
            let base = align(b, 16);
            self.reserve(base + 1)?;
            let end = self.memory_size()? as u32;
            self.define("__heap_base", Sym::Data(base), false);
            self.define("__heap_end", Sym::Data(end), false);
        }
        *taken = self.forward()?;
        let mut insts = Vec::new();
        for (k, (p, (mem_base, table_base))) in group.iter_mut().zip(bases).enumerate() {
            let stubs = self.stubs(&p.info.imports)?;
            let mut imports = Vec::new();
            let mut later = Vec::new(); // the GOT.mem imports to set later
            for (m, n, kind) in &p.info.imports {
                let weak = p.info.weak.contains(n);
                imports.push(match (m.as_str(), n.as_str(), kind) {
                    ("env", "memory", _) => Import::Handle(self.world.memory),
                    ("env", "__indirect_function_table", _) => Import::Handle(self.world.table),
                    ("env", "__stack_pointer", _) => Import::Handle(self.world.sp),
                    ("env", "__memory_base", _) => Import::I32 { value: mem_base as i32, mutable: false },
                    ("env", "__table_base", _) => Import::I32 { value: table_base as i32, mutable: false },
                    ("GOT.mem" | "GOT.func", _, Kind::Global) => {
                        let t = self.target(n);
                        let (v, l) = self.got(&t, m == "GOT.func", weak)?;
                        if l {
                            later.push((imports.len(), t));
                        }
                        Import::I32 { value: v as i32, mutable: true }
                    }
                    ("wasi_snapshot_preview1", _, _) => Import::Default,
                    ("env", _, Kind::Func(_)) => match self.syms.get(&self.target(n)) {
                        Some((Sym::Func(Some(f), _), _)) => Import::Handle(*f),
                        _ => Import::Handle(stubs[n]),
                    },
                    ("env", _, Kind::Tag(ft)) => match self.syms.get(n) {
                        Some((Sym::Tag(t), _)) => Import::Handle(*t),
                        _ => {
                            let inst = self.rt.instantiate(&gen::tag(ft), None, &mut [])?;
                            let t = self.rt.export(inst, "t");
                            self.define(n, Sym::Tag(t), false);
                            Import::Handle(t)
                        }
                    },
                    _ => fail!("{}: unsupported import {m}.{n}", p.name),
                });
            }
            let inst = self.rt.instantiate(&p.bytes, p.compiled, &mut imports)?;
            for (i, n) in later {
                if let Import::Handle(g) = imports[i] {
                    self.later.push((n, g));
                }
            }
            for (n, kind) in &p.info.exports {
                match kind {
                    Kind::Func(_) if owner.get(n) == Some(&k) => {
                        let f = self.rt.export(inst, n);
                        let s = self.syms.get_mut(n).unwrap();
                        if let Sym::Func(_, t) = s.0 {
                            s.0 = Sym::Func(Some(f), t);
                        }
                        if let Some(slot) = s.1 {
                            self.world("table_set", &[slot as u64, f as u64])?;
                        }
                    }
                    Kind::Tag(_) if !INTERNAL.contains(&n.as_str()) => {
                        let t = self.rt.export(inst, n);
                        self.define(n, Sym::Tag(t), false);
                    }
                    _ => {}
                }
            }
            if let Some(mut d) = p.debug.take() {
                d.instance = inst;
                self.debug.push(d);
            }
            insts.push((inst, mem_base));
        }
        Ok(insts)
    }

    /// The value of a GOT entry, an address or a table slot, and whether
    /// it is to be set when the variable comes.
    fn got(&mut self, n: &str, func: bool, weak: bool) -> Result<(u32, bool)> {
        Ok(match self.syms.get(n).copied() {
            Some((Sym::Data(a), _)) if !func => (a, false),
            Some((Sym::Func(..), _)) if func => (self.slot(n)?, false),
            Some(_) => fail!("{n}: GOT.{} of a {}", if func { "func" } else { "mem" }, if func { "data" } else { "function" }),
            None if weak => (0, false),
            None if func => (self.reserve_slot(n, false)?.0, false),
            None => (self.undefined_address(n), true),
        })
    }

    /// Runs `main` and exits through libc's `exit`.
    pub fn run(&mut self) -> Result<i32> {
        let main = ["__main_argc_argv", "__original_main", "main"].into_iter().find(|n| self.syms.contains_key(*n));
        let ret = match main {
            Some("__main_argc_argv") => self.guarded_call("__main_void", &[])?,
            Some("__original_main") => self.guarded_call("__original_main", &[])?,
            Some(_) => self.guarded_call("main", &[0, 0])?,
            None => Vec::new(),
        };
        let code = ret.first().map_or(0, |&v| v as i32);
        self.guarded_call("exit", &[code as u32 as u64])?;
        Ok(code)
    }

    fn guarded_call(&mut self, name: &str, args: &[u64]) -> Result<Vec<u64>> {
        let (f, ty) = self.func(name)?;
        self.guarded(f, &ty, args)
    }

    /// Defines `__sc_show_int` and `__sc_show_float`, which a REPL cell
    /// that ends in an expression calls: [`Loader::shown`] says with what.
    pub fn show(&mut self) {
        for n in ["__sc_show_int", "__sc_show_float"] {
            self.define_world(n);
        }
    }

    fn define_world(&mut self, n: &'static str) {
        let (f, ty) = self.world.funcs[n].clone();
        let t = self.ty(&ty);
        self.define(n, Sym::Func(Some(f), t), true);
    }

    /// What the last call to `__sc_show_*` was given since the last time.
    pub fn shown(&mut self) -> Option<Shown> {
        let r = self.world("take_shown", &[]).ok()?;
        (r[0] != 0).then(|| Shown { float: r[0] == 2, ty: r[1] as u32, kind: r[2] as u32, bits: r[3] })
    }

    /// The memory from `addr`, at most `max` bytes; none if `addr` is
    /// outside it.
    pub fn read(&mut self, addr: u32, max: usize) -> Option<Vec<u8>> {
        let size = self.memory_size().ok()?;
        let n = (size.checked_sub(addr as u64)? as usize).min(max);
        let mut buf = vec![0; n];
        self.rt.read(self.world.memory, addr, &mut buf).then_some(buf)
    }

    /// What the data of the module at `base` is now, before its
    /// constructors: words that change after it may point to the heap.
    fn image(&mut self, base: u32) -> Result<()> {
        let Some(k) = self.data.iter().rposition(|d| d.start == base) else { return Ok(()) };
        let (start, end) = (self.data[k].start, self.data[k].end);
        let mut all = vec![0; (end - start) as usize];
        self.rt.read(self.world.memory, start, &mut all);
        all.truncate(all.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1));
        self.data[k].image = all;
        Ok(())
    }

    /// libsc_rt's imports for the stacks of its blocks:
    /// `uint32_t __sc_rt_stack(void)` keeps the stack of its caller and
    /// says its number, and `void __sc_rt_blame(uint32_t allocated,
    /// uint32_t freed)` names the stacks of the block of the report that
    /// follows ([`Loader::blamed`]); 0 is none. And those for its leaks
    /// ([`Loader::leaks`]): `uint64_t __sc_rt_data(uint32_t i)`, where the
    /// data of the i-th module is, and `void __sc_rt_leak(uint32_t
    /// allocated, uint32_t bytes, uint32_t blocks)`. They are the world's,
    /// and `__sc_rt_stack` the runtime's `env.wasm_stack`.
    pub fn keep_stacks(&mut self) {
        for n in ["__sc_rt_stack", "__sc_rt_blame", "__sc_rt_data", "__sc_rt_leak"] {
            self.define_world(n);
        }
    }

    /// The blocks of libsc_rt's allocator that nothing reaches from the
    /// stack or the data of the modules, by the stack they were allocated
    /// at, the most bytes first; none without libsc_rt.
    pub fn leaks(&mut self) -> Result<Vec<Leak>> {
        if !self.defines("__sc_rt_leaks") {
            return Ok(Vec::new());
        }
        let mut roots = Vec::new();
        for k in 0..self.data.len() {
            let (start, end) = (self.data[k].start, self.data[k].end);
            let from = start & !3;
            let mut mem = vec![0; (end - from) as usize + 3];
            self.rt.read(self.world.memory, from, &mut mem);
            let d = &self.data[k];
            let mut run = None;
            for a in (from..end).step_by(4) {
                let i = (a - from) as usize;
                let then = |k: usize| a.checked_sub(d.start).and_then(|o| d.image.get(o as usize + k)).copied().unwrap_or(0);
                if (0..4).any(|k| mem[i + k] != then(k)) {
                    run.get_or_insert(a);
                } else if let Some(s) = run.take() {
                    roots.push((s, a));
                }
            }
            roots.extend(run.map(|s| (s, d.end)));
        }
        // the data's ranges and the leaks go in memory of libc's allocator,
        // which libsc_rt does not see
        let bytes: Vec<u8> = roots.iter().flat_map(|&(s, e)| ((e as u64) << 32 | s as u64).to_le_bytes()).collect();
        let size = bytes.len() as u32 + LEAKS * 12;
        let buf = self.call("aligned_alloc", &[Val::I32(8), Val::I32(align(size.max(8), 8) as i32)])?
            .and_then(|v| v.i32()).unwrap_or(0) as u32;
        if buf == 0 {
            fail!("out of memory looking for leaks");
        }
        let at = buf + bytes.len() as u32;
        self.rt.write(self.world.memory, buf, &bytes);
        self.world("set_data", &[buf as u64, roots.len() as u64])?;
        self.world("set_leaks", &[at as u64, LEAKS as u64])?;
        let sp = self.stack_pointer();
        let r = self.call("__sc_rt_leaks", &[Val::I32(sp)]);
        self.world("set_data", &[0, 0])?;
        let n = (self.world("leak_count", &[])?[0] as u32).min(LEAKS);
        let mut leaks = vec![0; n as usize * 12];
        self.rt.read(self.world.memory, at, &mut leaks);
        self.call("free", &[Val::I32(buf as i32)])?;
        r?;
        let word = |b: &[u8]| u32::from_le_bytes(b.try_into().unwrap());
        let mut out: Vec<Leak> = leaks.chunks(12).map(|l| {
            let a = word(&l[0..4]);
            Leak { allocated: (a != 0).then(|| self.rt.stack(a)), bytes: word(&l[4..8]), blocks: word(&l[8..12]) }
        }).collect();
        out.sort_by_key(|l| std::cmp::Reverse(l.bytes));
        Ok(out)
    }

    /// The stacks where the block of libsc_rt's last report was allocated
    /// and freed, if it named them ([`Loader::keep_stacks`]).
    pub fn blamed(&mut self) -> (Option<Vec<Frame>>, Option<Vec<Frame>>) {
        let r = self.world("take_blamed", &[]).unwrap_or_else(|_| vec![0, 0]);
        let get = |id: u64| (id != 0).then(|| self.rt.stack(id as u32));
        (get(r[0]), get(r[1]))
    }

    /// The function, file and line of `frame`, inlined functions first,
    /// from its module's DWARF; none without it.
    pub fn symbols(&self, frame: &Frame) -> Vec<Symbol> {
        let Some(d) = self.debug.iter().find(|d| Some(d.instance) == frame.instance) else { return Vec::new() };
        let cx = d.cx.get_or_init(|| {
            let empty: Arc<[u8]> = Arc::from([]);
            let dwarf = gimli::Dwarf::load(|id| -> std::result::Result<Dwarf, gimli::Error> {
                let data = d.sections.get(id.name()).unwrap_or(&empty);
                Ok(Dwarf::new(Arc::clone(data), gimli::LittleEndian))
            }).ok()?;
            addr2line::Context::from_dwarf(dwarf).ok()
        });
        let (Some(cx), Some(at)) = (cx, frame.offset) else { return Vec::new() };
        let mut out = Vec::new();
        if let Ok(mut fs) = cx.find_frames(at as u64 - d.code).skip_all_loads() {
            while let Ok(Some(f)) = fs.next() {
                let loc = f.location.as_ref();
                out.push(Symbol {
                    name: f.function.as_ref().and_then(|n| n.raw_name().ok()).map(|n| n.to_string()),
                    file: loc.and_then(|l| l.file).map(str::to_string),
                    line: loc.and_then(|l| l.line),
                    column: loc.and_then(|l| l.column),
                });
            }
        }
        out
    }

    /// Whether the module `name` is loaded: instantiated, even if its
    /// constructors then stopped with an error.
    pub fn is_loaded(&self, name: &str) -> bool {
        self.loaded.contains(name)
    }

    /// Whether the modules define the function `name`.
    pub fn defines(&self, name: &str) -> bool {
        matches!(self.syms.get(name), Some((Sym::Func(Some(_), _), _)))
    }

    /// The stack pointer, which a trap leaves where the code that trapped
    /// had it.
    pub fn stack_pointer(&mut self) -> i32 {
        self.world("get_sp", &[]).map_or(0, |r| r[0] as i32)
    }

    pub fn set_stack_pointer(&mut self, sp: i32) -> Result<()> {
        self.world("set_sp", &[sp as u32 as u64]).map(drop)
    }
}
