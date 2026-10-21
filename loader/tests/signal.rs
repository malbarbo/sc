//! Ctrl-C stops a loop of the REPL (PLANO-REPL.md §5.3, step 5).

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use wasmtime::{Engine, Trap, Val};

/// The engine the signal interrupts, as `sc`'s SIGINT handler.
static ENGINE: OnceLock<Engine> = OnceLock::new();

extern "C" fn interrupt(_: libc::c_int) {
    if let Some(e) = ENGINE.get() {
        e.increment_epoch();
    }
}

#[test]
fn signal_stops_a_loop() {
    let mut l = common::loader(true);
    ENGINE.set(l.interrupter()).unwrap();
    // SAFETY: the handler only bumps an atomic counter
    unsafe { libc::signal(libc::SIGUSR1, interrupt as *const () as libc::sighandler_t) };
    // armed before the signal can come: one before would be lost
    l.arm();
    // to this thread, in the loop's code, as a Ctrl-C to the REPL
    let me = unsafe { libc::pthread_self() };
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        unsafe { libc::pthread_kill(me, libc::SIGUSR1) };
    });
    // a loop the signal does not stop would hang the test
    static STOPPED: AtomicBool = AtomicBool::new(false);
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(30));
        if !STOPPED.load(Ordering::Relaxed) {
            eprintln!("signal_stops_a_loop: the signal did not stop the loop");
            std::process::exit(1);
        }
    });
    let e = l.call("spin", &[]).unwrap_err();
    STOPPED.store(true, Ordering::Relaxed);
    t.join().unwrap();
    assert_eq!(e.downcast_ref::<Trap>(), Some(&Trap::Interrupt), "{e:?}");
    // the REPL goes on: armed again, the next call runs
    l.arm();
    assert_eq!(l.call("add", &[Val::I32(2), Val::I32(3)]).unwrap().unwrap().unwrap_i32(), 5);
}
