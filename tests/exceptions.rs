//! Many exceptions on wasmtime's DRC collector (docs/plan-repl.md §5.3, step
//! 5), in a process of their own, so that only they count in its memory.
//! Unix only: the memory is read with getrusage.
#![cfg(unix)]

mod common;

use sc::loader::{Loader, Val};

/// The peak resident memory of this process, in bytes: after a first run,
/// what more runs add to it is what they kept. getrusage, not /proc, which
/// only Linux has.
fn rss() -> u64 {
    // SAFETY: getrusage writes the struct it is given
    let ru = unsafe {
        let mut ru = std::mem::zeroed::<libc::rusage>();
        assert_eq!(libc::getrusage(libc::RUSAGE_SELF, &mut ru), 0);
        ru
    };
    // kilobytes on Linux, bytes on macOS
    let unit = if cfg!(target_os = "macos") { 1 } else { 1024 };
    ru.ru_maxrss as u64 * unit
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
