//! The safe API over the linked engine.
use crate::{msg, sys, wire, Mode, Status};
use std::borrow::Cow;
use std::ffi::{c_char, c_int, CString};
use std::fmt;
use std::io::{self, PipeWriter, Read, Write};
use std::marker::PhantomData;
use std::ptr::{self, NonNull};
use std::sync::OnceLock;
use std::thread::{self, ThreadId};

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

/// The one-shot engine, `sc -cc1 ...` or `sc ld ...` (`sc_main`); `args[0]`
/// is the program. Returns the exit code.
pub fn main<I, S>(args: I) -> Result<i32, Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    claim()?;
    let mut argv = Argv::new(args)?;
    // sc_main does not write to argv
    Ok(unsafe { sys::sc_main(argv.argc(), argv.ptrs.as_mut_ptr()) })
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
        let st = unsafe { sys::sc_session_new(argv.argc(), argv.ptrs.as_ptr(), &mut raw) };
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
        let st = unsafe { sys::sc_session_cell(self.raw.as_ptr(), src.as_ptr().cast(), len, mode.to_raw() as c_int) };
        Ok(self.reply(st))
    }

    /// The brackets `src` leaves open, one more if it ends inside a comment
    /// or with a `\` that joins the next line. It does not touch the session.
    pub fn depth(&mut self, src: impl AsRef<[u8]>) -> Result<u32, Error> {
        let src = src.as_ref();
        let len = u32::try_from(src.len()).map_err(|_| Error::TooLong)?;
        // the engine copies src[0, len)
        Ok(unsafe { sys::sc_session_depth(self.raw.as_ptr(), src.as_ptr().cast(), len) } as u32)
    }

    /// Undoes the last accepted cell. Its code is already generated, so the
    /// session is lost and the host replays the log without it; an error if
    /// no cell was accepted.
    pub fn undo(&mut self) -> Reply<'_> {
        let st = unsafe { sys::sc_session_undo(self.raw.as_ptr()) };
        self.reply(st)
    }

    fn reply(&self, st: c_int) -> Reply<'_> {
        Reply { session: self, status: Status::from_raw(st as u32) }
    }

    fn part(&self, part: u32) -> &[u8] {
        // the result lives in the session until the next call, which needs
        // &mut self and so outlives this borrow
        unsafe {
            let p = sys::sc_result_ptr(self.raw.as_ptr(), part as c_int);
            let len = sys::sc_result_len(self.raw.as_ptr(), part as c_int) as usize;
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
        unsafe { sys::sc_session_free(self.raw.as_ptr()) }
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

impl Reply<'_> {
    /// The reply as a message of the protocol, with this status.
    fn write(&self, output: &mut impl Write, status: Status) -> io::Result<()> {
        let s = self.session;
        let parts = [sys::sc_part::SC_DIAGS, sys::sc_part::SC_SO, sys::sc_part::SC_SYMS].map(|p| s.part(p));
        wire::write(output, msg::SC_MSG_REPLY, status.to_raw(), parts)
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

/// The child's side of the protocol (`engine/sc.h`, [`crate::Child`]): one
/// session at a time, until the end of `input`. The parent waits for each
/// reply before it sends the next message.
///
/// A cell that fails leaves part of itself in clang (declarations in lookup
/// tables, Sema's caches, macros, IR: PLANO-REPL.md §3.3), so before each
/// cell the child forks a copy of itself ([`Copy`]), which goes on in its
/// place if the cell fails or brings the engine down. The session is then
/// as before the cell, and the reply says [`Status::Error`].
fn reply(output: &mut impl Write, st: Status, diags: &[u8]) -> io::Result<()> {
    wire::write(output, msg::SC_MSG_REPLY, st.to_raw(), [diags, b"", b""])
}

pub fn serve(input: &mut impl Read, output: &mut impl Write) -> Result<(), Error> {
    claim()?;
    let mut session: Option<Session> = None;
    while let Some(m) = wire::read(input)? {
        match (m.kind, &mut session) {
            (msg::SC_MSG_NEW, _) => {
                session = None;
                let text = String::from_utf8_lossy(&m.parts[0]);
                let args: Vec<&str> = text.split_terminator('\0').collect();
                match Session::new(args) {
                    Ok(s) => {
                        session = Some(s);
                        reply(output, Status::Ok, b"")?
                    }
                    Err(e) => reply(output, Status::Error, format!("sc: {e}\n").as_bytes())?,
                }
            }
            (msg::SC_MSG_CELL, Some(s)) => {
                let Some(mode) = Mode::from_raw(m.status) else {
                    reply(output, Status::Error, b"sc: unknown cell mode\n")?;
                    continue;
                };
                // a replayed cell is one that was accepted
                output.flush()?;
                let copy = match mode {
                    Mode::So | Mode::Whole | Mode::Type => match Copy::fork()? {
                        Fork::Original(c) => c,
                        Fork::Copy(answer) => {
                            output.write_all(&answer)?;
                            output.flush()?;
                            continue;
                        }
                    },
                    Mode::Replay => None,
                };
                let mut answer = vec![];
                let status = match s.cell(&m.parts[0], mode) {
                    Ok(r) => {
                        // without a copy, a failed cell loses the session
                        let st = match (r.status(), mode, &copy) {
                            (Status::Ok, _, _) | (_, Mode::Replay, _) => r.status(),
                            (Status::Incomplete, _, Some(_)) => Status::Incomplete,
                            (_, _, Some(_)) => Status::Error,
                            (_, _, None) => Status::Lost,
                        };
                        r.write(&mut answer, st)?;
                        st
                    }
                    Err(e) => {
                        reply(&mut answer, Status::Error, format!("sc: {e}\n").as_bytes())?;
                        Status::Error
                    }
                };
                match copy {
                    // what Sema made of the expression goes with the original
                    Some(c) if status != Status::Ok || mode == Mode::Type => c.hand_over(&answer),
                    copy => {
                        // the reply does not wait for the copy to end
                        output.write_all(&answer)?;
                        output.flush()?;
                        if let Some(c) = copy {
                            c.discard()
                        }
                    }
                }
            }
            (msg::SC_MSG_UNDO, Some(s)) => {
                let r = s.undo();
                r.write(output, r.status())?
            }
            (msg::SC_MSG_DEPTH, s) => {
                // the reply's status is the depth: with no session, nothing is open
                let d = s.as_mut().and_then(|s| s.depth(&m.parts[0]).ok()).unwrap_or(0);
                wire::write(output, msg::SC_MSG_REPLY, d, [b"", b"", b""])?
            }
            (msg::SC_MSG_CELL | msg::SC_MSG_UNDO, None) => reply(output, Status::Error, b"sc: no session\n")?,
            _ => reply(output, Status::Error, b"sc: unknown message\n")?,
        }
        output.flush()?;
    }
    Ok(())
}

extern "C" {
    fn fork() -> c_int;
    fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    fn _exit(code: c_int) -> !;
}

/// The copy of the child taken before a cell, as its original sees it. The
/// copy waits on a pipe for one of: `d` (the cell was accepted: the copy
/// ends), `t` and the answer (the copy goes on in the original's place and
/// sends the answer), or the end of the pipe alone (the original ended
/// during the cell: the copy goes on and answers with an error).
struct Copy {
    pid: c_int,
    to: PipeWriter,
}

enum Fork {
    /// In the original: its copy, or none if the fork failed.
    Original(Option<Copy>),
    /// In the copy, which goes on: the answer to send.
    Copy(Vec<u8>),
}

impl Copy {
    fn fork() -> io::Result<Fork> {
        let Ok((mut from, to)) = io::pipe() else {
            return Ok(Fork::Original(None));
        };
        match unsafe { fork() } {
            pid if pid < 0 => Ok(Fork::Original(None)),
            0 => {
                drop(to);
                let mut said = vec![];
                from.read_to_end(&mut said)?;
                match said.split_first() {
                    Some((b'd', _)) => unsafe { _exit(0) },
                    Some((b't', answer)) => Ok(Fork::Copy(answer.to_vec())),
                    _ => {
                        let diags: &[u8] = b"sc: the engine crashed on this cell; the session is as it was before it\n";
                        let mut answer = vec![];
                        reply(&mut answer, Status::Error, diags)?;
                        Ok(Fork::Copy(answer))
                    }
                }
            }
            pid => Ok(Fork::Original(Some(Copy { pid, to }))),
        }
    }

    /// The cell was accepted: the copy ends.
    fn discard(mut self) {
        let _ = self.to.write_all(b"d");
        drop(self.to);
        unsafe { waitpid(self.pid, ptr::null_mut(), 0) };
    }

    /// The cell failed, or was a type: the copy goes on with the answer, and
    /// the original, whose session has the cell's leftovers, ends.
    fn hand_over(mut self, answer: &[u8]) -> ! {
        let _ = self.to.write_all(b"t").and_then(|_| self.to.write_all(answer));
        unsafe { _exit(0) }
    }
}
