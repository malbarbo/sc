//! The REPL's side: the engine in a child process ([`Child`]), and the log
//! that recreates the session after it is lost ([`Repl`], PLANO-REPL.md §3.6).
use crate::{msg, sys, wire, Mode, Status};
use std::fmt;
use std::io::{self, BufReader, BufWriter};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{self, ChildStdin, ChildStdout, Command, Stdio};

/// The answer to a request, owned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub status: Status,
    /// The diagnostics, as clang and wasm-ld print them.
    pub diags: String,
    /// The cell as a wasm shared object.
    pub so: Vec<u8>,
    /// The names the cell exports.
    pub syms: Vec<String>,
}

impl Answer {
    /// An answer with diagnostics alone.
    fn bare(status: Status, diags: String) -> Answer {
        Answer { status, diags, so: vec![], syms: vec![] }
    }

    fn from_msg(m: wire::Msg) -> Answer {
        let [diags, so, syms] = m.parts;
        Answer {
            status: Status::from_raw(m.status),
            diags: String::from_utf8_lossy(&diags).into_owned(),
            so,
            syms: String::from_utf8_lossy(&syms).lines().map(String::from).collect(),
        }
    }
}

/// The engine in a child process (`sc-engine session`). Its stderr is the
/// parent's: what the engine writes there directly (a crash, "LLVM ERROR").
/// An `Err` is a child that ended or does not follow the protocol.
pub struct Child {
    proc: process::Child,
    to: BufWriter<ChildStdin>,
    from: BufReader<ChildStdout>,
}

impl Child {
    /// Starts `program args...`, which speaks the protocol on stdin and stdout.
    /// It is in a process group of its own: a Ctrl-C at the terminal is for
    /// the student's code, which the REPL runs.
    pub fn spawn(program: &Path, args: &[&str]) -> io::Result<Child> {
        let mut proc = Command::new(program).args(args).stdin(Stdio::piped()).stdout(Stdio::piped())
            .process_group(0).spawn()?;
        let to = BufWriter::new(proc.stdin.take().unwrap());
        let from = BufReader::new(proc.stdout.take().unwrap());
        Ok(Child { proc, to, from })
    }

    fn call(&mut self, kind: u32, status: u32, part: &[u8]) -> io::Result<Answer> {
        self.request(kind, status, part).map(Answer::from_msg)
    }

    fn request(&mut self, kind: u32, status: u32, part: &[u8]) -> io::Result<wire::Msg> {
        let res = wire::write(&mut self.to, kind, status, [part, b"", b""]).and_then(|_| wire::read(&mut self.from));
        match res {
            Ok(Some(m)) if m.kind == msg::SC_MSG_REPLY => Ok(m),
            Ok(_) => Err(self.ended()),
            Err(e) if matches!(e.kind(), io::ErrorKind::BrokenPipe | io::ErrorKind::UnexpectedEof) => Err(self.ended()),
            Err(e) => Err(e),
        }
    }

    /// The child is gone: how it ended.
    fn ended(&mut self) -> io::Error {
        // after a failed cell the process spawned may have handed over to
        // its copy ([`crate::serve`]) and ended with 0
        let _ = self.proc.kill();
        let how = match self.proc.wait() {
            Ok(s) if s.success() => String::new(),
            Ok(s) => format!(" ({s})"),
            Err(e) => format!(" ({e})"),
        };
        io::Error::new(io::ErrorKind::BrokenPipe, format!("the engine ended{how}"))
    }

    /// A session (it replaces the one before): see [`crate::sys::sc_session_new`].
    pub fn new_session<S: AsRef<str>>(&mut self, args: &[S]) -> io::Result<Answer> {
        let mut part = vec![];
        for a in args {
            part.extend(a.as_ref().as_bytes());
            part.push(0);
        }
        self.call(msg::SC_MSG_NEW, 0, &part)
    }

    pub fn cell(&mut self, src: &[u8], mode: Mode) -> io::Result<Answer> {
        self.call(msg::SC_MSG_CELL, mode.to_raw(), src)
    }

    pub fn undo(&mut self) -> io::Result<Answer> {
        self.call(msg::SC_MSG_UNDO, 0, b"")
    }

    /// See [`crate::sys::sc_session_depth`].
    pub fn depth(&mut self, src: &[u8]) -> io::Result<u32> {
        Ok(self.request(msg::SC_MSG_DEPTH, 0, src)?.status)
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        // the end of its input ends the child; a lost one is killed
        if matches!(self.proc.try_wait(), Ok(None)) {
            let _ = self.proc.kill();
        }
        let _ = self.proc.wait();
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

/// A session in a child, with the log of its accepted cells. A failed cell
/// leaves nothing in the child ([`crate::serve`]), so the log does not keep
/// it. When the session is lost (the child ended, an undo), a new child
/// replays the log without the backend, and each cell must export the same
/// names (§3.6).
pub struct Repl {
    program: PathBuf,
    args: Vec<String>,
    child: Child,
    log: Vec<Entry>,
}

impl Repl {
    /// A child `program session` with a session of `args`.
    pub fn new(program: PathBuf, args: Vec<String>) -> Result<Repl, ReplError> {
        let child = Repl::start(&program, &args)?;
        Ok(Repl { program, args, child, log: vec![] })
    }

    fn start(program: &Path, args: &[String]) -> Result<Child, ReplError> {
        let mut child = Child::spawn(program, &["session"])?;
        let a = child.new_session(args)?;
        if a.status != Status::Ok {
            return Err(ReplError::Start(a.diags));
        }
        Ok(child)
    }

    /// A new child replays the log.
    fn restart(&mut self) -> Result<(), ReplError> {
        self.child = Repl::start(&self.program, &self.args)?;
        for (i, e) in self.log.iter().enumerate() {
            let a = self.child.cell(&e.src, Mode::Replay)?;
            if a.status != Status::Ok || a.syms != e.syms {
                return Err(ReplError::Diverged(i));
            }
        }
        Ok(())
    }

    /// The accepted cells.
    pub fn len(&self) -> usize {
        self.log.len()
    }

    /// The names the accepted cells export.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.log.iter().flat_map(|e| &e.syms).map(String::as_str)
    }

    /// Compiles a cell. [`Status::Lost`]: the cell ended the session (its
    /// diagnostics, or how the child ended), which is back as before it.
    pub fn cell(&mut self, src: &[u8]) -> Result<Answer, ReplError> {
        self.compile(src, Mode::So)
    }

    /// [`Repl::cell`], or [`Status::Incomplete`] if the cell stops short
    /// ([`Mode::Whole`]): nothing of it is kept.
    pub fn cell_if_whole(&mut self, src: &[u8]) -> Result<Answer, ReplError> {
        self.compile(src, Mode::Whole)
    }

    /// See [`crate::sys::sc_session_depth`].
    pub fn depth(&mut self, src: &[u8]) -> Result<u32, ReplError> {
        match self.child.depth(src) {
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {
                self.restart()?;
                Ok(self.child.depth(src)?)
            }
            r => Ok(r?),
        }
    }

    fn compile(&mut self, src: &[u8], mode: Mode) -> Result<Answer, ReplError> {
        let a = match self.child.cell(src, mode) {
            Ok(a) => a,
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => lost(e),
            Err(e) => return Err(e.into()),
        };
        match a.status {
            Status::Ok => self.log.push(Entry { src: src.to_vec(), syms: a.syms.clone() }),
            Status::Error | Status::Incomplete => {}
            Status::Lost => self.restart()?,
        }
        Ok(a)
    }

    /// Undoes the last accepted cell: [`Status::Ok`] once the session is
    /// back without it, [`Status::Error`] if there is none.
    pub fn undo(&mut self) -> Result<Answer, ReplError> {
        if self.log.pop().is_none() {
            return Ok(Answer::bare(Status::Error, "sc: no cell to undo\n".into()));
        }
        // the generated code cannot be undone in the session (§3.3)
        self.restart()?;
        Ok(Answer::bare(Status::Ok, String::new()))
    }
}

fn lost(e: io::Error) -> Answer {
    Answer::bare(Status::Lost, format!("sc: {e}\n"))
}

// Answer::from_msg takes the parts in this order
const _: () = assert!(sys::sc_part::SC_DIAGS == 0 && sys::sc_part::SC_SO == 1 && sys::sc_part::SC_SYMS == 2);
