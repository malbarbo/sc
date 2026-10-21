//! The REPL's side: the engine in a child process ([`Child`]), and the log
//! that recreates the session after it is lost ([`Repl`], PLANO-REPL.md §3.6).
use crate::{msg, sys, wire, Mode, Status};
use std::fmt;
use std::io::{self, BufReader, BufWriter, Read};
use std::os::fd::AsRawFd;
use std::os::raw::{c_int, c_short};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{self, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Set by [`interrupt`]: the request waiting for the child gives up.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Stops the request a [`Child`] waits on, if any: its child is killed, and
/// a [`Repl`] replays the log in another. Safe in a signal handler (Ctrl-C).
pub fn interrupt() {
    INTERRUPTED.store(true, Ordering::Relaxed);
}

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

/// The engine in a child process (`sc --engine`). Its stderr is the
/// parent's: what the engine writes there directly (a crash, "LLVM ERROR").
/// An `Err` is a child that ended or does not follow the protocol.
///
/// After a failed cell the process spawned hands the session over to its
/// copy ([`crate::serve`]) and ends with 0: the copy, which is not ours to
/// wait for, is reached through the process group, which they share.
pub struct Child {
    proc: process::Child,
    /// None once closed, in [`Drop`].
    to: Option<BufWriter<ChildStdin>>,
    from: BufReader<ChildStdout>,
    /// How long a reply may take, and the memory it may take ([`Repl::limit`]).
    timeout: Option<Duration>,
    memory: Option<u64>,
    /// Set by [`Child::ended`]: the process spawned is reaped, and its
    /// process group may be gone, its id free for another.
    gone: bool,
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
        Ok(Child { proc, to: Some(to), from, timeout: None, memory: None, gone: false })
    }

    fn call(&mut self, kind: u32, status: u32, part: &[u8]) -> io::Result<Answer> {
        self.request(kind, status, part).map(Answer::from_msg)
    }

    fn request(&mut self, kind: u32, status: u32, part: &[u8]) -> io::Result<wire::Msg> {
        if self.gone {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "the engine ended"));
        }
        let to = self.to.as_mut().expect("open until dropped");
        // a Ctrl-C before is for the student's code
        INTERRUPTED.store(false, Ordering::Relaxed);
        let res = wire::write(to, kind, status, [part, b"", b""]).and_then(|_| self.wait()).and_then(|_| wire::read(&mut self.from));
        match res {
            Ok(Some(m)) if m.kind == msg::SC_MSG_REPLY => Ok(m),
            Ok(_) => Err(self.ended()),
            Err(e) if matches!(e.kind(), io::ErrorKind::BrokenPipe | io::ErrorKind::UnexpectedEof) => Err(self.ended()),
            // why it was killed, not how it ended
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                self.ended();
                Err(io::Error::new(io::ErrorKind::BrokenPipe, e.to_string()))
            }
            Err(e) => Err(e),
        }
    }

    /// The child is gone: how it ended. If the copy was serving, how it
    /// ended is not known.
    fn ended(&mut self) -> io::Error {
        self.kill();
        self.gone = true;
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

    /// Waits for the reply: [`io::ErrorKind::TimedOut`] if it is
    /// interrupted, takes longer than the timeout or more memory.
    fn wait(&mut self) -> io::Result<()> {
        let deadline = self.timeout.map(|t| Instant::now() + t);
        let fd = self.from.get_ref().as_raw_fd();
        while self.from.buffer().is_empty() {
            if INTERRUPTED.swap(false, Ordering::Relaxed) {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "interrupted"));
            }
            let mut left = deadline.map_or(-1, |d| d.saturating_duration_since(Instant::now()).as_millis() as c_int);
            if let Some(m) = self.memory {
                if self.rss() > m {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, format!("the compiler took more than {} MB", m >> 20)));
                }
                // how often the memory is looked at
                left = if left < 0 { 200 } else { left.min(200) };
            }
            let mut p = PollFd { fd, events: POLLIN, revents: 0 };
            // a signal ends the poll with EINTR, even with SA_RESTART
            match unsafe { poll(&mut p, 1, left) } {
                0 if deadline.map_or(true, |d| Instant::now() < d) => continue,
                0 => {
                    let t = self.timeout.unwrap_or_default().as_secs();
                    return Err(io::Error::new(io::ErrorKind::TimedOut, format!("the compiler took more than {t} s")));
                }
                n if n < 0 => continue,
                // readable, or closed: the read says which
                _ => break,
            }
        }
        Ok(())
    }

    /// The resident memory of the child: of the largest process of its
    /// group, which is the one that serves (the copies share its pages).
    fn rss(&self) -> u64 {
        let pgid = self.proc.id().to_string();
        let Ok(dir) = std::fs::read_dir("/proc") else { return 0 };
        let page = unsafe { sysconf(SC_PAGESIZE) } as u64;
        dir.flatten()
            .filter_map(|e| std::fs::read_to_string(e.path().join("stat")).ok())
            .filter_map(|st| {
                // the fields after the name: state, ppid, pgrp, ..., rss (the 22nd)
                let f: Vec<&str> = st.rsplit_once(") ")?.1.split(' ').collect();
                (f.get(2) == Some(&pgid.as_str())).then(|| f.get(21)?.parse::<u64>().ok()).flatten()
            })
            .max()
            .unwrap_or(0)
            * page
    }

    /// Kills the process spawned and its copy, if any.
    fn kill(&mut self) {
        unsafe { kill(-(self.proc.id() as c_int), SIGKILL) };
    }

    /// Whether the child closed its output within `ms` milliseconds, which
    /// it does when it ends. What it writes until then is dropped.
    fn closes_within(&mut self, ms: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(ms);
        let mut buf = [0; 4096];
        loop {
            let left = deadline.saturating_duration_since(Instant::now()).as_millis() as c_int;
            let mut p = PollFd { fd: self.from.get_ref().as_raw_fd(), events: POLLIN, revents: 0 };
            match unsafe { poll(&mut p, 1, left) } {
                0 => return false,
                n if n < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => continue,
                n if n < 0 => return false,
                _ => match self.from.get_mut().read(&mut buf) {
                    Ok(0) => return true,
                    Ok(_) => {}
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => return false,
                },
            }
        }
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        // the end of its input ends the child, which frees its session (and
        // its temporary directory); one that does not end in time is killed
        drop(self.to.take());
        if !self.gone && !self.closes_within(5000) {
            self.kill();
        }
        let _ = self.proc.wait();
    }
}

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: c_short,
    revents: c_short,
}

const POLLIN: c_short = 1;
const SIGKILL: c_int = 9;
const SC_PAGESIZE: c_int = 30;

extern "C" {
    fn poll(fds: *mut PollFd, nfds: u64, timeout: c_int) -> c_int;
    fn kill(pid: c_int, sig: c_int) -> c_int;
    fn sysconf(name: c_int) -> std::os::raw::c_long;
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
    timeout: Option<Duration>,
    memory: Option<u64>,
}

impl Repl {
    /// A child `program --engine` with a session of `args`.
    pub fn new(program: PathBuf, args: Vec<String>) -> Result<Repl, ReplError> {
        let child = Repl::start(&program, &args, None, None)?;
        Ok(Repl { program, args, child, log: vec![], timeout: None, memory: None })
    }

    /// Limits each reply of the child to `timeout`, and its memory to
    /// `memory` bytes (PLANO-REPL.md §4). A reply that takes longer or
    /// more, or is [`interrupt`]ed, loses the session ([`Status::Lost`]),
    /// which a new child replays. After an accepted cell that leaves the
    /// child above `memory`, a new child replays the log too.
    pub fn limit(&mut self, timeout: Option<Duration>, memory: Option<u64>) {
        (self.timeout, self.memory) = (timeout, memory);
        (self.child.timeout, self.child.memory) = (timeout, memory);
    }

    fn start(program: &Path, args: &[String], timeout: Option<Duration>, memory: Option<u64>) -> Result<Child, ReplError> {
        let mut child = Child::spawn(program, &["--engine"])?;
        (child.timeout, child.memory) = (timeout, memory);
        let a = child.new_session(args)?;
        if a.status != Status::Ok {
            return Err(ReplError::Start(a.diags));
        }
        Ok(child)
    }

    /// A new child replays the log. It replaces the one before only once
    /// the whole log is replayed: on an error, the child and the log are
    /// as they were. The REPL calls it when the child takes too much memory
    /// (PLANO-REPL.md §4).
    pub fn restart(&mut self) -> Result<(), ReplError> {
        let mut child = Repl::start(&self.program, &self.args, self.timeout, self.memory)?;
        for (i, e) in self.log.iter().enumerate() {
            let a = child.cell(&e.src, Mode::Replay)?;
            if a.status != Status::Ok || a.syms != e.syms {
                return Err(ReplError::Diverged(i));
            }
        }
        self.child = child;
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

    /// The type of `src`, an expression without its `;`, if
    /// [`Status::Ok`]. Nothing of it is kept.
    pub fn type_of(&mut self, src: &[u8]) -> Result<Answer, ReplError> {
        self.compile(src, Mode::Type)
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
            Status::Ok if mode != Mode::Type => self.log.push(Entry { src: src.to_vec(), syms: a.syms.clone() }),
            Status::Ok | Status::Error | Status::Incomplete => {}
            Status::Lost => self.restart()?,
        }
        if self.memory.is_some_and(|m| self.child.rss() > m) {
            self.restart()?;
        }
        Ok(a)
    }

    /// Undoes the last accepted cell: [`Status::Ok`] once the session is
    /// back without it, [`Status::Error`] if there is none.
    pub fn undo(&mut self) -> Result<Answer, ReplError> {
        let Some(last) = self.log.pop() else {
            return Ok(Answer::bare(Status::Error, "sc: no cell to undo\n".into()));
        };
        // the generated code cannot be undone in the session (§3.3)
        if let Err(e) = self.restart() {
            self.log.push(last);
            return Err(e);
        }
        Ok(Answer::bare(Status::Ok, String::new()))
    }
}

fn lost(e: io::Error) -> Answer {
    Answer::bare(Status::Lost, format!("sc: {e}\n"))
}

// Answer::from_msg takes the parts in this order
const _: () = assert!(sys::sc_part::SC_DIAGS == 0 && sys::sc_part::SC_SO == 1 && sys::sc_part::SC_SYMS == 2);
