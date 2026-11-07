//! The log that recreates a lost session (docs/plan-repl.md §3.6), over any
//! [`Compiler`]: sc.wasm's session on wasmtime (`Guest`), or the session in
//! the same module (inside sc.wasm).
use super::{CellStatus, Mode};
use std::fmt;
use std::io;

/// The answer to a request, owned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub status: CellStatus,
    /// The diagnostics, as clang and wasm-ld print them.
    pub diags: String,
    /// The cell as a wasm shared object.
    pub so: Vec<u8>,
    /// The names the cell exports; [`Mode::Type`]: the type, which
    /// [`Repl::type_of`] takes out.
    pub names: Vec<String>,
}

impl Answer {
    /// An answer with diagnostics alone.
    pub fn bare(status: CellStatus, diags: String) -> Answer {
        Answer { status, diags, so: vec![], names: vec![] }
    }
}

/// The type of an expression ([`Repl::type_of`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeOf {
    /// None if the source is not an expression.
    pub ty: Option<String>,
    /// The diagnostics, as clang prints them.
    pub diags: String,
}

/// A compiler with one session at a time, which a [`Repl`] drives. A
/// failed cell leaves nothing in it: the session is as before the cell.
pub trait Compiler: Sized {
    /// Another compiler as this one, without a session.
    fn another(&self) -> Result<Self, CompilerError>;
    /// A session of `args`, in place of the one before: see
    /// [`super::sys::sc_session_new`]. [`CompilerError::Start`] if it does
    /// not start.
    fn new_session(&mut self, args: &[String]) -> Result<(), CompilerError>;
    fn cell(&mut self, src: &[u8], mode: Mode) -> Result<Answer, CompilerError>;
    /// See [`super::sys::sc_session_depth`].
    fn depth(&mut self, src: &[u8]) -> Result<u32, CompilerError>;
    /// Whether it takes more memory than it may: the [`Repl`] replays the
    /// log in another.
    fn over(&self) -> bool {
        false
    }
}

/// Why a [`Compiler`] did not answer.
#[derive(Debug)]
pub enum CompilerError {
    /// The compiler ended, and the session with it: why.
    Lost(String),
    /// The session did not start: the diagnostics.
    Start(String),
    /// The compiler did not start, or it refused the request.
    Io(io::Error),
}

impl fmt::Display for CompilerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CompilerError::Lost(why) => write!(f, "the engine ended ({why})"),
            CompilerError::Start(d) => write!(f, "the session did not start: {d}"),
            CompilerError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CompilerError {}

impl From<io::Error> for CompilerError {
    fn from(e: io::Error) -> CompilerError {
        CompilerError::Io(e)
    }
}

/// Why a [`Repl`] cannot go on.
#[derive(Debug)]
pub enum ReplError {
    Compiler(CompilerError),
    /// The replay of the cell at this index of the log did not give what it
    /// gave the first time: the student must restart the session.
    Diverged(usize),
}

impl fmt::Display for ReplError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ReplError::Compiler(e) => write!(f, "{e}"),
            ReplError::Diverged(i) => write!(f, "the replay of cell {} diverged", i + 1),
        }
    }
}

impl std::error::Error for ReplError {}

impl From<CompilerError> for ReplError {
    fn from(e: CompilerError) -> ReplError {
        ReplError::Compiler(e)
    }
}

/// An accepted cell of the log: its source and the names it exported.
struct Entry {
    src: Vec<u8>,
    names: Vec<String>,
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
    /// much memory (docs/plan-repl.md §4).
    pub fn restart(&mut self) -> Result<(), ReplError> {
        let mut compiler = open(self.compiler.another()?, &self.args)?;
        for (i, e) in self.log.iter().enumerate() {
            let a = compiler.cell(&e.src, Mode::Replay)?;
            if a.status != CellStatus::Ok || a.names != e.names {
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
        self.log.iter().flat_map(|e| &e.names).map(String::as_str)
    }

    /// Compiles a cell. [`CellStatus::Lost`]: the cell ended the session (its
    /// diagnostics, or how the compiler ended), which is back as before it,
    /// or will be at the next request if the replay failed.
    pub fn cell(&mut self, src: &[u8]) -> Result<Answer, ReplError> {
        self.compile(src, Mode::So)
    }

    /// [`Repl::cell`], or [`CellStatus::Incomplete`] if the cell stops short
    /// ([`Mode::Whole`]): nothing of it is kept.
    pub fn cell_if_whole(&mut self, src: &[u8]) -> Result<Answer, ReplError> {
        self.compile(src, Mode::Whole)
    }

    /// The type of `src`, an expression without its `;`. Nothing of it is
    /// kept.
    pub fn type_of(&mut self, src: &[u8]) -> Result<TypeOf, ReplError> {
        let a = self.compile(src, Mode::Type)?;
        let ty = if a.status == CellStatus::Ok { a.names.into_iter().next() } else { None };
        Ok(TypeOf { ty, diags: a.diags })
    }

    /// See [`super::sys::sc_session_depth`].
    pub fn depth(&mut self, src: &[u8]) -> Result<u32, ReplError> {
        match self.compiler.depth(src) {
            Err(CompilerError::Lost(_)) => {
                self.restart()?;
                Ok(self.compiler.depth(src)?)
            }
            r => Ok(r?),
        }
    }

    fn compile(&mut self, src: &[u8], mode: Mode) -> Result<Answer, ReplError> {
        let mut a = match self.compiler.cell(src, mode) {
            Ok(a) => a,
            Err(e @ CompilerError::Lost(_)) => Answer::bare(CellStatus::Lost, format!("sc: {e}\n")),
            Err(e) => return Err(e.into()),
        };
        match a.status {
            CellStatus::Ok if mode != Mode::Type => self.log.push(Entry { src: src.to_vec(), names: a.names.clone() }),
            CellStatus::Ok | CellStatus::Error | CellStatus::Incomplete => {}
            // a replay that fails (interrupted too, or above the limits)
            // leaves the compiler that ended: the next request replays again
            CellStatus::Lost => {
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

    /// Undoes the last accepted cell. An `Err` says why not, for the
    /// student: there is none, or the replay failed, and the session stays
    /// as it was.
    pub fn undo(&mut self) -> Result<(), String> {
        let Some(last) = self.log.pop() else {
            return Err("sc: no cell to undo\n".into());
        };
        // the generated code cannot be undone in the session (§3.3)
        if let Err(e) = self.restart() {
            self.log.push(last);
            return Err(format!("sc: the cell was not undone: {e}\n"));
        }
        Ok(())
    }
}

/// `compiler` with a session of `args`.
fn open<C: Compiler>(mut compiler: C, args: &[String]) -> Result<C, ReplError> {
    compiler.new_session(args)?;
    Ok(compiler)
}
