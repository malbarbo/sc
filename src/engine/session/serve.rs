//! The child's side of the protocol (`engine/sc.h`, [`super::super::Child`]),
//! with a copy of the process forked before each cell.
use super::{claim, Error, Reply, Session};
use crate::engine::{msg, sys, wire, Mode, Status};
use std::ffi::c_int;
use std::io::{self, PipeWriter, Read, Write};
use std::ptr;

impl Reply<'_> {
    /// The reply as a message of the protocol, with this status.
    fn write(&self, output: &mut impl Write, status: Status) -> io::Result<()> {
        let s = self.session;
        let parts = [sys::sc_part::SC_DIAGS, sys::sc_part::SC_SO, sys::sc_part::SC_SYMS].map(|p| s.part(p));
        wire::write(output, msg::SC_MSG_REPLY, status.to_raw(), parts)
    }
}

/// The child's side of the protocol (`engine/sc.h`, [`super::Child`]): one
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
                    // the engine's diagnostics, which the parent says did not start
                    Err(Error::Start(d)) => reply(output, Status::Error, d.as_bytes())?,
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
                        // the copy is told before the reply, which the
                        // parent may answer by killing the original (the
                        // end of the REPL); the reply does not wait for
                        // the copy to end
                        let pid = copy.map(Copy::discard);
                        output.write_all(&answer)?;
                        output.flush()?;
                        if let Some(pid) = pid {
                            unsafe { waitpid(pid, ptr::null_mut(), 0) };
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

/// The child (`--engine`): [`serve`] on stdin and a copy of stdout. The
/// engine's own writes to stdout go to stderr, so they cannot break a
/// message. It ends with its parent, and so do its copies: at the end of
/// its input, or in the middle of a cell (the parent's timeout no longer
/// runs; a panic in `sc` aborts, with no `Drop`). A copy that goes on in
/// the original's place is no longer the parent's child, so the end is
/// told by stdin, the parent's pipe: with `O_ASYNC`, the kernel sends
/// SIGIO to the child's process group when the parent closes it
/// ([`on_sigio`]). Unix alone, as `fork` is (ANALISE.md §51).
pub fn serve_child() -> Result<(), Error> {
    use std::os::fd::AsFd;
    // SAFETY: the handler only polls and exits
    unsafe {
        libc::signal(libc::SIGIO, on_sigio as *const () as libc::sighandler_t);
        libc::fcntl(0, libc::F_SETOWN, -libc::getpgrp());
        libc::fcntl(0, libc::F_SETFL, libc::fcntl(0, libc::F_GETFL) | libc::O_ASYNC);
    }
    // the parent may have ended before
    on_sigio(0);
    let out = std::fs::File::from(io::stdout().as_fd().try_clone_to_owned()?);
    if unsafe { dup2(2, 1) } < 0 {
        return Err(io::Error::last_os_error().into());
    }
    serve(&mut io::BufReader::new(io::stdin().lock()), &mut io::BufWriter::new(out))
}

/// Each write of the parent to stdin sends SIGIO too: the process ends
/// only if the parent closed it.
extern "C" fn on_sigio(_: c_int) {
    let mut p = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
    if unsafe { libc::poll(&mut p, 1, 0) } > 0 && p.revents & libc::POLLHUP != 0 {
        unsafe { libc::_exit(0) };
    }
}

extern "C" {
    fn fork() -> c_int;
    fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    fn _exit(code: c_int) -> !;
    fn dup2(old: c_int, new: c_int) -> c_int;
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
        #[cfg(feature = "wasm")]
        crate::engine::wasm::assert_one_thread();
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

    /// The cell was accepted: the copy ends. Its pid, to wait for.
    fn discard(mut self) -> c_int {
        let _ = self.to.write_all(b"d");
        self.pid
    }

    /// The cell failed, or was a type: the copy goes on with the answer, and
    /// the original, whose session has the cell's leftovers, ends.
    fn hand_over(mut self, answer: &[u8]) -> ! {
        let _ = self.to.write_all(b"t").and_then(|_| self.to.write_all(answer));
        unsafe { _exit(0) }
    }
}
