//! The REPL's side: the engine in a child process ([`Child`]), the
//! [`Compiler`] of the native REPL.
use super::{msg, sys, wire, Answer, Compiler, Mode, Repl, ReplError, Status};
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

fn answer(m: wire::Msg) -> Answer {
    let [diags, so, syms] = m.parts;
    Answer {
        status: Status::from_raw(m.status),
        diags: String::from_utf8_lossy(&diags).into_owned(),
        so,
        syms: String::from_utf8_lossy(&syms).lines().map(String::from).collect(),
    }
}

/// The engine in a child process (`sc --engine`). Its stderr is the
/// parent's: what the engine writes there directly (a crash, "LLVM ERROR").
/// An `Err` is a child that ended or does not follow the protocol.
///
/// After a failed cell the process spawned hands the session over to its
/// copy ([`super::serve`]) and ends with 0: the copy, which is not ours to
/// wait for, is reached through the process group, which they share.
pub struct Child {
    /// What was spawned, for [`Compiler::another`].
    program: PathBuf,
    args: Vec<String>,
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
        let args = args.iter().map(|a| a.to_string()).collect();
        Ok(Child { program: program.into(), args, proc, to: Some(to), from, timeout: None, memory: None, gone: false })
    }

    /// Limits each reply to `timeout`, and the memory to `memory` bytes
    /// (PLANO-REPL.md §4). A reply that takes longer or more, or is
    /// [`interrupt`]ed, ends the child; so does, in a [`Repl`], an
    /// accepted cell that leaves it above `memory`.
    pub fn limit(&mut self, timeout: Option<Duration>, memory: Option<u64>) {
        (self.timeout, self.memory) = (timeout, memory);
    }

    fn call(&mut self, kind: u32, status: u32, part: &[u8]) -> io::Result<Answer> {
        self.request(kind, status, part).map(answer)
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

    /// A session (it replaces the one before): see [`super::sys::sc_session_new`].
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

    /// See [`super::sys::sc_session_depth`].
    pub fn depth(&mut self, src: &[u8]) -> io::Result<u32> {
        Ok(self.request(msg::SC_MSG_DEPTH, 0, src)?.status)
    }

    /// Waits for the reply: [`io::ErrorKind::TimedOut`] if it is
    /// interrupted, takes longer than the timeout or more memory.
    fn wait(&mut self) -> io::Result<()> {
        let deadline = self.timeout.map(|t| Instant::now() + t);
        let fd = self.from.get_ref().as_raw_fd();
        // the memory is looked at (all of /proc) only once a poll found no
        // reply: most come within the first
        let mut first = true;
        while self.from.buffer().is_empty() {
            if INTERRUPTED.swap(false, Ordering::Relaxed) {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "interrupted"));
            }
            let mut left = deadline.map_or(-1, |d| d.saturating_duration_since(Instant::now()).as_millis() as c_int);
            if let Some(m) = self.memory {
                if !std::mem::take(&mut first) && self.rss() > m {
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

impl Compiler for Child {
    fn another(&self) -> io::Result<Child> {
        let args: Vec<&str> = self.args.iter().map(String::as_str).collect();
        let mut child = Child::spawn(&self.program, &args)?;
        child.limit(self.timeout, self.memory);
        Ok(child)
    }

    fn new_session(&mut self, args: &[String]) -> io::Result<Answer> {
        Child::new_session(self, args)
    }

    fn cell(&mut self, src: &[u8], mode: Mode) -> io::Result<Answer> {
        Child::cell(self, src, mode)
    }

    fn depth(&mut self, src: &[u8]) -> io::Result<u32> {
        Child::depth(self, src)
    }

    fn over(&self) -> bool {
        self.memory.is_some_and(|m| self.rss() > m)
    }
}

impl Repl<Child> {
    /// A child `program --engine` with a session of `args`, which starts
    /// within the limits ([`Repl::limit`]).
    pub fn new(program: PathBuf, args: Vec<String>, timeout: Option<Duration>, memory: Option<u64>) -> Result<Repl<Child>, ReplError> {
        let mut child = Child::spawn(&program, &["--engine"])?;
        child.limit(timeout, memory);
        Repl::start(child, args)
    }

    /// [`Child::limit`]: a reply above the limits loses the session
    /// ([`Status::Lost`]), which a new child replays. After an accepted
    /// cell that leaves the child above `memory`, a new child replays the
    /// log too.
    pub fn limit(&mut self, timeout: Option<Duration>, memory: Option<u64>) {
        self.compiler.limit(timeout, memory);
    }
}

// answer takes the parts in this order
const _: () = assert!(sys::sc_part::SC_DIAGS == 0 && sys::sc_part::SC_SO == 1 && sys::sc_part::SC_SYMS == 2);
