//! The log that recreates a lost session (PLANO-REPL.md §3.6), over any
//! [`Compiler`]: the engine in a child process ([`super::Child`]), or the
//! session in the same module (sc.wasm).
use super::{Mode, Status};
use std::fmt;
use std::io;

/// The answer to a request, owned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub status: Status,
    /// The diagnostics, as clang and wasm-ld print them.
    pub diags: String,
    /// The cell as a wasm shared object.
    pub so: Vec<u8>,
    /// The names the cell exports; [`Mode::Type`]: the type.
    pub syms: Vec<String>,
}

impl Answer {
    /// An answer with diagnostics alone.
    pub fn bare(status: Status, diags: String) -> Answer {
        Answer { status, diags, so: vec![], syms: vec![] }
    }
}

/// A compiler with one session at a time, which a [`Repl`] drives. A
/// failed cell leaves nothing in it: the session is as before the cell. An
/// `Err` of kind [`io::ErrorKind::BrokenPipe`] is a compiler that ended:
/// the session is lost.
pub trait Compiler: Sized {
    /// Another compiler as this one, without a session.
    fn another(&self) -> io::Result<Self>;
    /// A session of `args`, in place of the one before: see
    /// [`super::sys::sc_session_new`].
    fn new_session(&mut self, args: &[String]) -> io::Result<Answer>;
    fn cell(&mut self, src: &[u8], mode: Mode) -> io::Result<Answer>;
    /// See [`super::sys::sc_session_depth`].
    fn depth(&mut self, src: &[u8]) -> io::Result<u32>;
    /// Whether it takes more memory than it may: the [`Repl`] replays the
    /// log in another.
    fn over(&self) -> bool {
        false
    }
}

/// Why a [`Repl`] cannot go on.
#[derive(Debug)]
pub enum ReplError {
    Io(io::Error),
    /// The session did not start: the diagnostics.
    Start(String),
    /// The replay of the cell at this index of the log did not give what it
    /// gave the first time: the student must restart the session.
    Diverged(usize),
}

impl fmt::Display for ReplError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ReplError::Io(e) => write!(f, "{e}"),
            ReplError::Start(d) => write!(f, "the session did not start: {d}"),
            ReplError::Diverged(i) => write!(f, "the replay of cell {} diverged", i + 1),
        }
    }
}

impl std::error::Error for ReplError {}

impl From<io::Error> for ReplError {
    fn from(e: io::Error) -> ReplError {
        ReplError::Io(e)
    }
}

/// An accepted cell of the log: its source and the names it exported.
struct Entry {
    src: Vec<u8>,
    syms: Vec<String>,
}

/// A session, with the log of its accepted cells. A failed cell leaves
/// nothing in the compiler, so the log does not keep it. When the session
/// is lost (the compiler ended, an undo), another compiler replays the log
/// without the backend, and each cell must export the same names (§3.6).
pub struct Repl<C: Compiler> {
    args: Vec<String>,
    pub(super) compiler: C,
    log: Vec<Entry>,
}

impl<C: Compiler> Repl<C> {
    /// A session of `args` in `compiler`.
    pub fn start(compiler: C, args: Vec<String>) -> Result<Repl<C>, ReplError> {
        let compiler = open(compiler, &args)?;
        Ok(Repl { args, compiler, log: vec![] })
    }

    /// Another compiler replays the log. It replaces the one before only
    /// once the whole log is replayed: on an error, the compiler and the
    /// log are as they were. The REPL calls it when the compiler takes too
    /// much memory (PLANO-REPL.md §4).
    pub fn restart(&mut self) -> Result<(), ReplError> {
        let mut compiler = open(self.compiler.another()?, &self.args)?;
        for (i, e) in self.log.iter().enumerate() {
            let a = compiler.cell(&e.src, Mode::Replay)?;
            if a.status != Status::Ok || a.syms != e.syms {
                return Err(ReplError::Diverged(i));
            }
        }
        self.compiler = compiler;
        Ok(())
    }

    /// The accepted cells.
    pub fn len(&self) -> usize {
        self.log.len()
    }

    /// No cell accepted yet.
    pub fn is_empty(&self) -> bool {
        self.log.is_empty()
    }

    /// The names the accepted cells export.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.log.iter().flat_map(|e| &e.syms).map(String::as_str)
    }

    /// Compiles a cell. [`Status::Lost`]: the cell ended the session (its
    /// diagnostics, or how the compiler ended), which is back as before it,
    /// or will be at the next request if the replay failed.
    pub fn cell(&mut self, src: &[u8]) -> Result<Answer, ReplError> {
        self.compile(src, Mode::So)
    }

    /// [`Repl::cell`], or [`Status::Incomplete`] if the cell stops short
    /// ([`Mode::Whole`]): nothing of it is kept.
    pub fn cell_if_whole(&mut self, src: &[u8]) -> Result<Answer, ReplError> {
        self.compile(src, Mode::Whole)
    }

    /// The type of `src`, an expression without its `;`, if
    /// [`Status::Ok`]. Nothing of it is kept.
    pub fn type_of(&mut self, src: &[u8]) -> Result<Answer, ReplError> {
        self.compile(src, Mode::Type)
    }

    /// See [`super::sys::sc_session_depth`].
    pub fn depth(&mut self, src: &[u8]) -> Result<u32, ReplError> {
        match self.compiler.depth(src) {
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {
                self.restart()?;
                Ok(self.compiler.depth(src)?)
            }
            r => Ok(r?),
        }
    }

    fn compile(&mut self, src: &[u8], mode: Mode) -> Result<Answer, ReplError> {
        let mut a = match self.compiler.cell(src, mode) {
            Ok(a) => a,
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Answer::bare(Status::Lost, format!("sc: {e}\n")),
            Err(e) => return Err(e.into()),
        };
        match a.status {
            Status::Ok if mode != Mode::Type => self.log.push(Entry { src: src.to_vec(), syms: a.syms.clone() }),
            Status::Ok | Status::Error | Status::Incomplete => {}
            // a replay that fails (interrupted too, or above the limits)
            // leaves the compiler that ended: the next request replays again
            Status::Lost => {
                if let Err(e) = self.restart() {
                    a.diags += &format!("sc: the session was not restored ({e}): the next cell tries again\n");
                }
            }
        }
        // the cell is in the log: on an error, the compiler stays, and its
        // next reply above the limit loses it
        if self.compiler.over() {
            let _ = self.restart();
        }
        Ok(a)
    }

    /// Undoes the last accepted cell: [`Status::Ok`] once the session is
    /// back without it, [`Status::Error`] if there is none or the replay
    /// failed (the session stays as it was).
    pub fn undo(&mut self) -> Result<Answer, ReplError> {
        let Some(last) = self.log.pop() else {
            return Ok(Answer::bare(Status::Error, "sc: no cell to undo\n".into()));
        };
        // the generated code cannot be undone in the session (§3.3)
        if let Err(e) = self.restart() {
            self.log.push(last);
            return Ok(Answer::bare(Status::Error, format!("sc: the cell was not undone: {e}\n")));
        }
        Ok(Answer::bare(Status::Ok, String::new()))
    }
}

/// `compiler` with a session of `args`.
fn open<C: Compiler>(mut compiler: C, args: &[String]) -> Result<C, ReplError> {
    let a = compiler.new_session(args)?;
    if a.status != Status::Ok {
        return Err(ReplError::Start(a.diags));
    }
    Ok(compiler)
}
