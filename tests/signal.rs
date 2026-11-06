//! Ctrl-C stops a loop of the REPL (docs/plan-repl.md §5.3, step 5).
//! Unix only: the signal is sent with pthread_kill.
#![cfg(unix)]

mod common;

use std::time::Duration;
use sc::loader::{Error, TrapKind, Val};

/// `sc`'s SIGINT handler ([`sc::loader::native::Wasmtime::on_ctrl_c`]), on the loader
/// of the second session, as after a %restart: the first one's is replaced.
#[test]
fn signal_stops_a_loop() {
    common::loader().runtime().on_ctrl_c(|| {});
    let mut l = common::loader();
    l.runtime().on_ctrl_c(|| {});
    // to this thread, in the loop's code, as a Ctrl-C to the REPL
    // as a number: on musl and macOS pthread_t is a pointer, which is not Send
    let me = unsafe { libc::pthread_self() } as usize;
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        unsafe { libc::pthread_kill(me as libc::pthread_t, libc::SIGINT) };
    });
    // each call is armed when it starts: a signal before it would be lost.
    // A loop the signal does not stop would hang the test. The process,
    // with only this test, ends with it
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(30));
        eprintln!("signal_stops_a_loop: the signal did not stop the loop");
        std::process::exit(1);
    });
    let e = l.call("spin", &[]).unwrap_err();
    t.join().unwrap();
    assert!(matches!(&e, Error::Trap(t) if t.kind == TrapKind::Interrupt), "{e:?}");
    // the REPL goes on: the next call runs
    assert_eq!(l.call("add", &[Val::I32(2), Val::I32(3)]).unwrap().unwrap().unwrap_i32(), 5);
}
