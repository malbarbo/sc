//! The safe API over the linked engine.
use super::{sys, Mode, Status};
// the raw functions: the linked engine's, or sc.wasm's exports
#[cfg(not(feature = "wasm"))]
use super::sys as raw;
#[cfg(feature = "wasm")]
use super::wasm as raw;
use std::borrow::Cow;
use std::ffi::{c_char, c_int, CString};
use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::ptr::{self, NonNull};
use std::sync::OnceLock;
use std::thread::{self, ThreadId};

// the child's side of the protocol, which forks: the native engine's
#[cfg(feature = "engine")]
mod serve;
#[cfg(feature = "engine")]
pub use serve::{serve, serve_child};

/// Why a call did not reach the engine, or a session did not start.
#[derive(Debug)]
pub enum Error {
    /// An argument has a NUL byte.
    Nul,
    /// The engine was first used from another thread.
    OtherThread,
    /// A cell of 4 GiB or more.
    TooLong,
    /// The session could not start: the diagnostics.
    Start(String),
    /// The protocol's pipes ([`serve`]).
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Nul => write!(f, "an argument has a NUL byte"),
            Error::OtherThread => write!(f, "the engine is used from another thread"),
            Error::TooLong => write!(f, "the cell is too long"),
            Error::Start(d) => write!(f, "the session did not start: {d}"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

/// The engine is not thread safe (LLVM is built without threads): only the
/// first thread that uses it may.
fn claim() -> Result<(), Error> {
    static OWNER: OnceLock<ThreadId> = OnceLock::new();
    let me = thread::current().id();
    if *OWNER.get_or_init(|| me) == me {
        Ok(())
    } else {
        Err(Error::OtherThread)
    }
}

/// argv for C: the strings and the pointers to them, null terminated.
struct Argv {
    _strings: Vec<CString>,
    ptrs: Vec<*const c_char>,
}

impl Argv {
    fn new<I, S>(args: I) -> Result<Argv, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let strings = args
            .into_iter()
            .map(|a| CString::new(a.as_ref()).map_err(|_| Error::Nul))
            .collect::<Result<Vec<_>, _>>()?;
        let mut ptrs: Vec<_> = strings.iter().map(|s| s.as_ptr()).collect();
        ptrs.push(ptr::null());
        Ok(Argv { _strings: strings, ptrs })
    }

    fn argc(&self) -> c_int {
        (self.ptrs.len() - 1) as c_int
    }
}

/// The one-shot engine, `sc_main`: `args[0]` is the program, then `-cc1 ...`
/// or `ld ...`. Returns the exit code.
pub fn main<I, S>(args: I) -> Result<i32, Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    claim()?;
    let mut argv = Argv::new(args)?;
    // sc_main does not write to argv
    Ok(unsafe { raw::sc_main(argv.argc(), argv.ptrs.as_mut_ptr()) })
}

/// The files the sessions read in place of the disk (`sc_set_files`): the
/// paths under `files.root`, for their clang and wasm-ld; with `sc.wasm`,
/// its own pack at the same root (`engine/sc_pack.cpp`).
pub fn set_files(files: &'static sys::sc_files) {
    // SAFETY: what files points to lives until the process ends
    unsafe { raw::sc_set_files(files) }
}

/// A REPL session: each cell is compiled on top of the ones before.
pub struct Session {
    raw: NonNull<sys::sc_session>,
    // the engine stays on its thread: neither Send nor Sync
    _thread: PhantomData<*mut ()>,
}

impl Session {
    /// A session from cc1 flags without an input file, `-x c` or `-x c++`
    /// (C by default), then, after `"--"`, the wasm-ld arguments that link a
    /// cell (`-L`, the libraries, the builtins).
    pub fn new<I, S>(args: I) -> Result<Session, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        claim()?;
        let argv = Argv::new(args)?;
        let mut raw = ptr::null_mut();
        // argv outlives the call; *out is always set
        let st = unsafe { raw::sc_session_new(argv.argc(), argv.ptrs.as_ptr(), &mut raw) };
        let session = Session {
            raw: NonNull::new(raw).expect("sc_session_new sets *out"),
            _thread: PhantomData,
        };
        if st as u32 == sys::sc_status::SC_OK {
            Ok(session)
        } else {
            let reply = Reply { session: &session, status: Status::Error };
            Err(Error::Start(reply.diags().into_owned()))
        }
    }

    /// Compiles a cell on top of the ones before.
    pub fn cell(&mut self, src: impl AsRef<[u8]>, mode: Mode) -> Result<Reply<'_>, Error> {
        let src = src.as_ref();
        let len = u32::try_from(src.len()).map_err(|_| Error::TooLong)?;
        // the engine copies src[0, len)
        let st = unsafe { raw::sc_session_cell(self.raw.as_ptr(), src.as_ptr().cast(), len, mode.to_raw() as c_int) };
        Ok(self.reply(st))
    }

    /// The brackets `src` leaves open, one more if it ends inside a comment
    /// or with a `\` that joins the next line. It does not touch the session.
    pub fn depth(&mut self, src: impl AsRef<[u8]>) -> Result<u32, Error> {
        let src = src.as_ref();
        let len = u32::try_from(src.len()).map_err(|_| Error::TooLong)?;
        // the engine copies src[0, len)
        Ok(unsafe { raw::sc_session_depth(self.raw.as_ptr(), src.as_ptr().cast(), len) } as u32)
    }

    /// Undoes the last accepted cell. Its code is already generated, so the
    /// session is lost and the host replays the log without it; an error if
    /// no cell was accepted.
    pub fn undo(&mut self) -> Reply<'_> {
        let st = unsafe { raw::sc_session_undo(self.raw.as_ptr()) };
        self.reply(st)
    }

    fn reply(&self, st: c_int) -> Reply<'_> {
        Reply { session: self, status: Status::from_raw(st as u32) }
    }

    fn part(&self, part: u32) -> &[u8] {
        // the result lives in the session until the next call, which needs
        // &mut self and so outlives this borrow
        unsafe {
            let p = raw::sc_result_ptr(self.raw.as_ptr(), part as c_int);
            let len = raw::sc_result_len(self.raw.as_ptr(), part as c_int) as usize;
            if p.is_null() || len == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(p, len)
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe { raw::sc_session_free(self.raw.as_ptr()) }
    }
}

/// The result of a call on a [`Session`], valid until the next one.
pub struct Reply<'s> {
    session: &'s Session,
    status: Status,
}

impl<'s> Reply<'s> {
    pub fn status(&self) -> Status {
        self.status
    }

    /// The diagnostics, as clang and wasm-ld print them.
    pub fn diags(&self) -> Cow<'s, str> {
        String::from_utf8_lossy(self.session.part(sys::sc_part::SC_DIAGS))
    }

    /// The cell as a wasm shared object (empty unless [`Mode::So`] and
    /// [`Status::Ok`]).
    pub fn so(&self) -> &'s [u8] {
        self.session.part(sys::sc_part::SC_SO)
    }

    /// The names the cell exports, to check a replay; [`Mode::Type`]: the
    /// type.
    pub fn syms(&self) -> impl Iterator<Item = Cow<'s, str>> {
        self.session.part(sys::sc_part::SC_SYMS).split(|&b| b == b'\n').filter(|l| !l.is_empty()).map(String::from_utf8_lossy)
    }
}

impl fmt::Debug for Reply<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Reply")
            .field("status", &self.status)
            .field("diags", &self.diags())
            .field("so", &self.so().len())
            .finish()
    }
}
