//! Ctrl-C stops a loop of the REPL (PLANO-REPL.md §5.3, step 5).

mod common;

use std::time::Duration;
use wasmtime::{Trap, Val};

/// `sc`'s SIGINT handler ([`sc::engine::on_ctrl_c`]), on the loader of the
/// second session, as after a %restart: the first one's engine is replaced.
#[test]
fn signal_stops_a_loop() {
    sc::engine::on_ctrl_c(common::loader(true).interrupter());
    let mut l = common::loader(true);
    sc::engine::on_ctrl_c(l.interrupter());
    // armed before the signal can come: one before would be lost
    l.arm();
    // to this thread, in the loop's code, as a Ctrl-C to the REPL
    let me = unsafe { libc::pthread_self() };
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        unsafe { libc::pthread_kill(me, libc::SIGINT) };
    });
    // a loop the signal does not stop would hang the test. The process,
    // with only this test, ends with it
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(30));
        eprintln!("signal_stops_a_loop: the signal did not stop the loop");
        std::process::exit(1);
    });
    let e = l.call("spin", &[]).unwrap_err();
    t.join().unwrap();
    assert_eq!(e.downcast_ref::<Trap>(), Some(&Trap::Interrupt), "{e:?}");
    // the REPL goes on: armed again, the next call runs
    l.arm();
    assert_eq!(l.call("add", &[Val::I32(2), Val::I32(3)]).unwrap().unwrap().unwrap_i32(), 5);
}
