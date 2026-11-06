//! Ctrl-C ([`super::Wasmtime::on_ctrl_c`]): it stops the code the last
//! runtime runs and calls what sc gave (the compile). [`InRawMode`] lets it
//! through while a line editor holds the terminal in raw mode.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use wasmtime::Engine;

/// The engine Ctrl-C interrupts: the last runtime's.
pub(super) static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);
static ALSO: OnceLock<fn()> = OnceLock::new();
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// A Ctrl-C came while an [`InRawMode`] lived.
#[cfg(windows)]
static CAME: AtomicBool = AtomicBool::new(false);

/// What a Ctrl-C does: in a signal handler on Unix, on a thread of its own
/// on Windows.
fn ctrl_c() {
    if let Some(also) = ALSO.get() {
        also();
    }
    // the main thread holds the lock only to put a new engine there
    if let Ok(e) = ENGINE.try_lock() {
        if let Some(e) = e.as_ref() {
            e.increment_epoch();
        }
    }
    #[cfg(windows)]
    CAME.store(true, Ordering::Relaxed);
}

/// The handler, once, with `also`, which must be safe in a signal handler.
pub(super) fn install(also: fn()) {
    let _ = ALSO.set(also);
    if INSTALLED.swap(true, Ordering::Relaxed) {
        return;
    }
    // SAFETY: the handler only touches atomics and the epoch
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGINT, on_sigint as *const () as libc::sighandler_t);
    }
    // the CRT's signal puts SIG_DFL back before it calls the handler: a
    // second Ctrl-C would end the process
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(on_ctrl), 1);
    }
}

#[cfg(unix)]
extern "C" fn on_sigint(_: libc::c_int) {
    ctrl_c();
}

/// Ctrl-C only: Ctrl-Break and closing the console end the process.
#[cfg(windows)]
unsafe extern "system" fn on_ctrl(kind: u32) -> windows_sys::core::BOOL {
    if kind != windows_sys::Win32::System::Console::CTRL_C_EVENT {
        return 0;
    }
    ctrl_c();
    1
}

/// While it lives, Ctrl-C reaches the handler, also in a terminal that a
/// line editor (rustyline) holds in raw mode, where Ctrl-C is a key it
/// reads later: the compile Enter does in sc's REPL. What was typed in the
/// meantime is thrown away with the Ctrl-C. Ctrl-\ and Ctrl-Z do nothing.
/// Without the handler or a terminal in raw mode it does nothing.
pub struct InRawMode {
    #[cfg(unix)]
    saved: Option<(libc::termios, libc::sigaction)>,
    #[cfg(windows)]
    saved: Option<(windows_sys::Win32::Foundation::HANDLE, u32)>,
}

impl InRawMode {
    #[cfg(unix)]
    pub fn new() -> InRawMode {
        let saved = INSTALLED.load(Ordering::Relaxed).then(|| unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut t) != 0 || t.c_lflag & libc::ISIG != 0 {
                return None;
            }
            // the handler first: a Ctrl-C between the two would go to the
            // editor's (rustyline's writes to a pipe it reads later)
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_sigint as *const () as libc::sighandler_t;
            sa.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut sa.sa_mask);
            let mut old: libc::sigaction = std::mem::zeroed();
            libc::sigaction(libc::SIGINT, &sa, &mut old);
            let mut raw = t;
            raw.c_lflag |= libc::ISIG;
            raw.c_cc[libc::VQUIT] = libc::_POSIX_VDISABLE;
            raw.c_cc[libc::VSUSP] = libc::_POSIX_VDISABLE;
            // Ctrl-T, SIGINFO's line
            #[cfg(target_vendor = "apple")]
            {
                raw.c_cc[libc::VSTATUS] = libc::_POSIX_VDISABLE;
            }
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw);
            Some((t, old))
        });
        InRawMode { saved: saved.flatten() }
    }

    #[cfg(windows)]
    pub fn new() -> InRawMode {
        use windows_sys::Win32::System::Console::*;
        let saved = INSTALLED.load(Ordering::Relaxed).then(|| unsafe {
            let h = GetStdHandle(STD_INPUT_HANDLE);
            let mut mode = 0;
            if GetConsoleMode(h, &mut mode) == 0 || mode & ENABLE_PROCESSED_INPUT != 0 {
                return None;
            }
            CAME.store(false, Ordering::Relaxed);
            SetConsoleMode(h, mode | ENABLE_PROCESSED_INPUT);
            Some((h, mode))
        });
        InRawMode { saved: saved.flatten() }
    }
}

impl Default for InRawMode {
    fn default() -> InRawMode {
        InRawMode::new()
    }
}

impl Drop for InRawMode {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some((t, old)) = self.saved.take() {
            // the terminal first, as in new
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
                libc::sigaction(libc::SIGINT, &old, std::ptr::null_mut());
            }
        }
        #[cfg(windows)]
        if let Some((h, mode)) = self.saved.take() {
            use windows_sys::Win32::System::Console::*;
            unsafe {
                SetConsoleMode(h, mode);
                // as ISIG does on Unix
                if CAME.swap(false, Ordering::Relaxed) {
                    FlushConsoleInputBuffer(h);
                }
            }
        }
    }
}
