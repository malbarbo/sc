//! A cell that fails to instantiate leaves nothing defined: what it
//! defined from its bytes before is undone (NOTES.md §60).

use sc::loader::native::Wasmtime;
use sc::loader::{libdirs, Loader, Val, STACK};
use wasmtime_wasi::WasiCtxBuilder;

/// `add`, and the allocator a cell after the first takes its memory from:
/// the cells here have none.
const BASE: &str = r#"
(module
  (@dylink.0 (mem-info (memory 0 0) (table 0 0)))
  (func (export "aligned_alloc") (param i32 i32) (result i32) (i32.const 16))
  (func (export "add") (param i32 i32) (result i32)
    (i32.add (local.get 0) (local.get 1))))
"#;

/// Calls `sub` through its address, before any cell defines it.
const CALLER: &str = r#"
(module
  (@dylink.0 (mem-info (memory 0 0) (table 0 0)))
  (import "env" "__indirect_function_table" (table 0 funcref))
  (import "GOT.func" "sub" (global $sub (mut i32)))
  (type $t (func (param i32 i32) (result i32)))
  (func (export "call_sub") (param i32 i32) (result i32)
    (call_indirect (type $t) (local.get 0) (local.get 1) (global.get $sub))))
"#;

/// Defines `sub`, but imports `add` with another type than the one defined.
const BAD: &str = r#"
(module
  (@dylink.0 (mem-info (memory 0 0) (table 0 0)))
  (import "env" "add" (func (param i64) (result i64)))
  (func (export "sub") (param i32 i32) (result i32) (i32.const 99)))
"#;

const SUB: &str = r#"
(module
  (@dylink.0 (mem-info (memory 0 0) (table 0 0)))
  (func (export "sub") (param i32 i32) (result i32)
    (i32.sub (local.get 0) (local.get 1))))
"#;

#[test]
fn failed_cell_is_undone() {
    let mut l = Loader::new(Wasmtime::new(WasiCtxBuilder::new().build_p1()).unwrap(), libdirs(Vec::new()), STACK).unwrap();
    l.load_bytes("base", &wat::parse_str(BASE).unwrap(), true).unwrap();
    let sub = |l: &mut Loader, f| l.call(f, &[Val::I32(7), Val::I32(2)]).map(|v| v.unwrap().unwrap_i32());
    l.load_bytes("caller", &wat::parse_str(CALLER).unwrap(), true).unwrap();
    assert!(l.load_bytes("bad", &wat::parse_str(BAD).unwrap(), true).is_err());
    assert!(!l.is_loaded("bad"));
    assert!(!l.defines("sub"));
    assert!(sub(&mut l, "call_sub").is_err());
    // the slot the failed cell took is reserved again: the next sub fills it
    l.load_bytes("sub", &wat::parse_str(SUB).unwrap(), true).unwrap();
    assert_eq!(sub(&mut l, "sub").unwrap(), 5);
    assert_eq!(sub(&mut l, "call_sub").unwrap(), 5);
    assert_eq!(l.call("add", &[Val::I32(2), Val::I32(3)]).unwrap().unwrap().unwrap_i32(), 5);
}
