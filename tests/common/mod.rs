//! The module of the tests: one per file, so each runs in its own process
//! (the signal handler, the resident memory).

use sc::loader::{Loader, STACK};
use wasmtime_wasi::WasiCtxBuilder;

/// A shared object as the cells are: the C++ exceptions' tag comes from
/// `env`.
pub const MODULE: &str = r#"
(module
  (@dylink.0 (mem-info (memory 0 0) (table 0 0)))
  (import "env" "__cpp_exception" (tag $e (param i32)))
  (func (export "spin") (loop $l (br $l)))
  (func (export "add") (param i32 i32) (result i32)
    (i32.add (local.get 0) (local.get 1)))
  ;; throws and catches n times; the sum of what was thrown
  (func (export "throws") (param $n i32) (result i32) (local $sum i32)
    (block $done
      (loop $l
        (br_if $done (i32.eqz (local.get $n)))
        (block $h (result i32)
          (try_table (catch $e $h) (throw $e (local.get $n)))
          (unreachable))
        (local.set $sum (i32.add (local.get $sum)))
        (local.set $n (i32.sub (local.get $n) (i32.const 1)))
        (br $l)))
    (local.get $sum)))
"#;

pub fn loader(interruptible: bool) -> Loader {
    let mut l = Loader::new(WasiCtxBuilder::new().build_p1(), Vec::new(), STACK, interruptible).unwrap();
    l.load_bytes("cell", &wat::parse_str(MODULE).unwrap(), true).unwrap();
    l
}
