//! The modules the loader makes for itself (NOTES.md §63): the world (the
//! memory, the table and the stack pointer of all modules, with functions
//! that change them), the stubs of functions not defined yet, a tag, a
//! module that sets globals, and the one that calls a function and catches
//! a C++ exception.

use super::runtime::{FuncType, ValType};
use wasm_encoder as we;
use we::{BlockType, Catch, ExportKind, HeapType, InstructionSink, MemArg};

fn val(t: ValType) -> we::ValType {
    match t {
        ValType::I32 => we::ValType::I32,
        ValType::I64 => we::ValType::I64,
        ValType::F32 => we::ValType::F32,
        ValType::F64 => we::ValType::F64,
        ValType::FuncRef => we::ValType::FUNCREF,
    }
}

/// A module being made. The imports come before the definitions of their
/// kind.
#[derive(Default)]
struct Module {
    types: Vec<FuncType>,
    imports: we::ImportSection,
    funcs: we::FunctionSection,
    tables: we::TableSection,
    memories: we::MemorySection,
    tags: we::TagSection,
    globals: we::GlobalSection,
    exports: we::ExportSection,
    code: we::CodeSection,
    nfuncs: u32,
    nglobals: u32,
}

const FUNCREF: we::TableType = we::TableType { element_type: we::RefType::FUNCREF, table64: false, minimum: 0, maximum: None, shared: false };

impl Module {
    fn ty(&mut self, ft: &FuncType) -> u32 {
        match self.types.iter().position(|t| t == ft) {
            Some(k) => k as u32,
            None => {
                self.types.push(ft.clone());
                self.types.len() as u32 - 1
            }
        }
    }

    fn import_func(&mut self, m: &str, n: &str, ft: &FuncType) -> u32 {
        let t = self.ty(ft);
        self.imports.import(m, n, we::EntityType::Function(t));
        self.nfuncs += 1;
        self.nfuncs - 1
    }

    fn import_table(&mut self, m: &str, n: &str) {
        self.imports.import(m, n, FUNCREF);
    }

    fn import_global(&mut self, m: &str, n: &str) -> u32 {
        let g = we::GlobalType { val_type: we::ValType::I32, mutable: true, shared: false };
        self.imports.import(m, n, g);
        self.nglobals += 1;
        self.nglobals - 1
    }

    fn import_tag(&mut self, m: &str, n: &str, ft: &FuncType) {
        let t = self.ty(ft);
        self.imports.import(m, n, we::TagType { kind: we::TagKind::Exception, func_type_idx: t });
    }

    fn global(&mut self, t: ValType, init: i64) -> u32 {
        let g = we::GlobalType { val_type: val(t), mutable: true, shared: false };
        let init = match t {
            ValType::I64 => we::ConstExpr::i64_const(init),
            _ => we::ConstExpr::i32_const(init as i32),
        };
        self.globals.global(g, &init);
        self.nglobals += 1;
        self.nglobals - 1
    }

    /// A function of type `ft`, with `locals` after its parameters.
    fn func(&mut self, ft: &FuncType, locals: &[ValType], body: impl FnOnce(&mut InstructionSink)) -> u32 {
        let t = self.ty(ft);
        self.funcs.function(t);
        let mut f = we::Function::new_with_locals_types(locals.iter().map(|&l| val(l)));
        body(&mut f.instructions());
        f.instructions().end();
        self.code.function(&f);
        self.nfuncs += 1;
        self.nfuncs - 1
    }

    fn export_func(&mut self, name: &str, ft: &FuncType, locals: &[ValType], body: impl FnOnce(&mut InstructionSink)) {
        let f = self.func(ft, locals, body);
        self.exports.export(name, ExportKind::Func, f);
    }

    fn finish(&self) -> Vec<u8> {
        let mut types = we::TypeSection::new();
        for t in &self.types {
            types.ty().function(t.params.iter().map(|&p| val(p)), t.results.iter().map(|&r| val(r)));
        }
        let mut m = we::Module::new();
        m.section(&types).section(&self.imports).section(&self.funcs).section(&self.tables).section(&self.memories)
            .section(&self.tags).section(&self.globals).section(&self.exports).section(&self.code);
        m.finish()
    }
}

use ValType::{F64, FuncRef, I32, I64};

fn ft(params: &[ValType], results: &[ValType]) -> FuncType {
    FuncType::new(params.iter().copied(), results.iter().copied())
}

fn mem(offset: u64, align: u32) -> MemArg {
    MemArg { offset, align, memory_index: 0 }
}

/// The world: exports `memory` (`pages` at first, at most `max`), `table`,
/// `__stack_pointer` (at `stack`) and `missing` (which function not defined
/// yet was called: see [`stubs`]), with functions that change them, and the
/// functions the loader gives the modules: libsc_rt's (`__sc_rt_*`, which
/// keep what it says) and the REPL's (`__sc_show_*`, the value it shows).
/// `__sc_rt_stack` is `env.wasm_stack`, of the runtime.
pub fn world(pages: u64, max: u64, stack: u32) -> Vec<u8> {
    let mut m = Module::default();
    let stack_fn = m.import_func("env", "wasm_stack", &ft(&[], &[I32]));
    m.exports.export("__sc_rt_stack", ExportKind::Func, stack_fn);
    m.memories.memory(we::MemoryType { minimum: pages, maximum: Some(max), memory64: false, shared: false, page_size_log2: None });
    m.exports.export("memory", ExportKind::Memory, 0);
    m.tables.table(we::TableType { minimum: 1, ..FUNCREF });
    m.exports.export("table", ExportKind::Table, 0);
    let sp = m.global(I32, stack as i64);
    m.exports.export("__stack_pointer", ExportKind::Global, sp);
    let missing = m.global(I32, 0);
    m.exports.export("missing", ExportKind::Global, missing);
    let [blame_a, blame_f, shown, show_ty, show_k] = [(); 5].map(|_| m.global(I32, 0));
    let show_v = m.global(I64, 0);
    let [data_at, data_n, leak_at, leak_cap, leak_n] = [(); 5].map(|_| m.global(I32, 0));

    m.export_func("grow", &ft(&[I32], &[I32]), &[], |s| { s.local_get(0).memory_grow(0); });
    m.export_func("size", &ft(&[], &[I32]), &[], |s| { s.memory_size(0); });
    m.export_func("table_grow", &ft(&[I32], &[I32]), &[], |s| { s.ref_null(HeapType::FUNC).local_get(0).table_grow(0); });
    m.export_func("table_set", &ft(&[I32, FuncRef], &[]), &[], |s| { s.local_get(0).local_get(1).table_set(0); });
    m.export_func("get_sp", &ft(&[], &[I32]), &[], |s| { s.global_get(sp); });
    m.export_func("set_sp", &ft(&[I32], &[]), &[], |s| { s.local_get(0).global_set(sp); });
    m.export_func("take_missing", &ft(&[], &[I32]), &[], |s| {
        s.global_get(missing).i32_const(0).global_set(missing);
    });
    m.export_func("__sc_rt_blame", &ft(&[I32, I32], &[]), &[], |s| {
        s.local_get(0).global_set(blame_a).local_get(1).global_set(blame_f);
    });
    m.export_func("take_blamed", &ft(&[], &[I32, I32]), &[], |s| {
        s.global_get(blame_a).global_get(blame_f).i32_const(0).global_set(blame_a).i32_const(0).global_set(blame_f);
    });
    m.export_func("__sc_show_int", &ft(&[I32, I32, I64], &[]), &[], |s| {
        s.i32_const(1).global_set(shown).local_get(0).global_set(show_ty).local_get(1).global_set(show_k)
            .local_get(2).global_set(show_v);
    });
    m.export_func("__sc_show_float", &ft(&[I32, I32, F64], &[]), &[], |s| {
        s.i32_const(2).global_set(shown).local_get(0).global_set(show_ty).local_get(1).global_set(show_k)
            .local_get(2).i64_reinterpret_f64().global_set(show_v);
    });
    m.export_func("take_shown", &ft(&[], &[I32, I32, I32, I64]), &[], |s| {
        s.global_get(shown).global_get(show_ty).global_get(show_k).global_get(show_v).i32_const(0).global_set(shown);
    });
    // the data of the modules for libsc_rt's leaks: n (start, end) pairs at `at`
    m.export_func("set_data", &ft(&[I32, I32], &[]), &[], |s| {
        s.local_get(0).global_set(data_at).local_get(1).global_set(data_n);
    });
    m.export_func("__sc_rt_data", &ft(&[I32], &[I64]), &[], |s| {
        s.local_get(0).global_get(data_n).i32_lt_u().if_(BlockType::Result(we::ValType::I64))
            .global_get(data_at).local_get(0).i32_const(3).i32_shl().i32_add().i64_load(mem(0, 3))
            .else_().i64_const(0).end();
    });
    // its leaks: 12 bytes each at `at`, room for `cap`; leak_count says
    // how many it told
    m.export_func("set_leaks", &ft(&[I32, I32], &[]), &[], |s| {
        s.local_get(0).global_set(leak_at).local_get(1).global_set(leak_cap).i32_const(0).global_set(leak_n);
    });
    m.export_func("__sc_rt_leak", &ft(&[I32, I32, I32], &[]), &[I32], |s| {
        s.global_get(leak_n).global_get(leak_cap).i32_lt_u().if_(BlockType::Empty)
            .global_get(leak_at).global_get(leak_n).i32_const(12).i32_mul().i32_add().local_set(3);
        for k in 0..3 {
            s.local_get(3).local_get(k).i32_store(mem(4 * k as u64, 2));
        }
        s.end().global_get(leak_n).i32_const(1).i32_add().global_set(leak_n);
    });
    m.export_func("leak_count", &ft(&[], &[I32]), &[], |s| { s.global_get(leak_n); });
    m.finish()
}

/// Imports `env.table` and `env.missing`, the world's. Exports `s{k}` for
/// each of `stubs`, of its type, which calls through its table slot, and
/// `u{k}` for each of `undefined`, which sets `missing` to its number and
/// traps.
pub fn stubs(stubs: &[(&FuncType, u32)], undefined: &[(&FuncType, u32)]) -> Vec<u8> {
    let mut m = Module::default();
    m.import_table("env", "table");
    let missing = m.import_global("env", "missing");
    for (k, &(t, slot)) in stubs.iter().enumerate() {
        let ty = m.ty(t);
        m.export_func(&format!("s{k}"), t, &[], |s| {
            for i in 0..t.params.len() as u32 {
                s.local_get(i);
            }
            s.i32_const(slot as i32).call_indirect(0, ty);
        });
    }
    for (k, &(t, id)) in undefined.iter().enumerate() {
        m.export_func(&format!("u{k}"), t, &[], |s| {
            s.i32_const(id as i32).global_set(missing).unreachable();
        });
    }
    m.finish()
}

/// Exports `t`, a tag of type `ft`.
pub fn tag(ft: &FuncType) -> Vec<u8> {
    let mut m = Module::default();
    let t = m.ty(ft);
    m.tags.tag(we::TagType { kind: we::TagKind::Exception, func_type_idx: t });
    m.exports.export("t", ExportKind::Tag, 0);
    m.finish()
}

/// Imports `n` mutable i32 globals, `g.0`, `g.1`..., and exports `set`,
/// which gives each its value in `values`.
pub fn setter(values: &[i32]) -> Vec<u8> {
    let mut m = Module::default();
    for k in 0..values.len() {
        m.import_global("g", &k.to_string());
    }
    m.export_func("set", &ft(&[], &[]), &[], |s| {
        for (k, &v) in values.iter().enumerate() {
            s.i32_const(v).global_set(k as u32);
        }
    });
    m.finish()
}

/// Imports `env.table` and `env.tag`, the C++ exceptions' tag, and exports
/// `try`: its parameters are those of `callee`'s type and a table slot; it
/// calls the slot's function and gives 0 and its results, or, if it throws
/// a C++ exception, the exception and zeros.
pub fn catcher(callee: &FuncType) -> Vec<u8> {
    let mut m = Module::default();
    m.import_table("env", "table");
    m.import_tag("env", "tag", &ft(&[I32], &[]));
    let ty = m.ty(callee);
    let n = callee.params.len() as u32;
    let mut params = callee.params.clone();
    params.push(I32);
    let mut results = vec![I32];
    results.extend(&callee.results);
    let locals = callee.results.clone();
    m.export_func("try", &FuncType::new(params, results), &locals, |s| {
        let r = |k: usize| n + 1 + k as u32; // the local of result k
        s.block(BlockType::Result(we::ValType::I32))
            .try_table(BlockType::Empty, [Catch::One { tag: 0, label: 0 }]);
        for i in 0..=n {
            s.local_get(i);
        }
        s.call_indirect(0, ty);
        for k in (0..callee.results.len()).rev() {
            s.local_set(r(k));
        }
        s.end().i32_const(0);
        for k in 0..callee.results.len() {
            s.local_get(r(k));
        }
        s.return_().end();
        for k in 0..callee.results.len() {
            s.local_get(r(k));
        }
    });
    m.finish()
}
