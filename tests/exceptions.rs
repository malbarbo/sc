//! Many exceptions on wasmtime's DRC collector (PLANO-REPL.md §5.3, step
//! 5), in a process of their own, so that only they count in its memory.

mod common;

use sc::loader::Loader;
use wasmtime::Val;

/// The resident memory of this process, in bytes.
fn rss() -> u64 {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap();
    let pages: u64 = statm.split_whitespace().nth(1).unwrap().parse().unwrap();
    // SAFETY: sysconf only reads a value
    pages * unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64
}

#[test]
fn many_exceptions() {
    // each throw allocates the exception in the GC heap, which DRC frees
    let mut l = common::loader();
    let n = 100_000;
    let throws = |l: &mut Loader| l.call("throws", &[Val::I32(n)]).unwrap().unwrap().unwrap_i32();
    assert_eq!(throws(&mut l), (n as i64 * (n as i64 + 1) / 2) as i32);
    let before = rss();
    for _ in 0..10 {
        throws(&mut l);
    }
    let grew = rss().saturating_sub(before);
    assert!(grew < 16 << 20, "{n}0 more exceptions took {grew} bytes");
}
