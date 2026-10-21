//! The engine's API (`engine/sc.h`, PLANO-REPL.md §2) for Rust.
//!
//! [`sys`] is what bindgen generates from the header: the protocol's types
//! and the raw functions. [`wire`] reads and writes the protocol's messages,
//! and [`Child`] and [`Repl`] are the REPL's side of it: the engine in a
//! child process and the log that replays a lost session.
//!
//! The engine is linked (the `engine` feature), and [`Session`] and
//! [`main`] are a safe API over the raw functions, and [`serve`] the child's
//! side of the protocol:
//!
//! - a [`Reply`] borrows its session, so the engine's buffers it points to
//!   cannot be used after the next call, which replaces them;
//! - the engine is not thread safe: only the first thread that uses it may,
//!   and a [`Session`] is neither `Send` nor `Sync`;
//! - arguments are checked before they cross (no NUL bytes, a cell under
//!   4 GiB).
//!
//! The compiler still ends the process on a fatal error or a crash: the
//! native REPL runs the engine in a child process (PLANO-REPL.md §1.1).
//!
//! With the `wasm` feature, the same API calls `sc.wasm`'s exports through
//! wasmtime in place of the linked engine ([`wasm`]).

#[allow(non_camel_case_types, non_upper_case_globals, dead_code)]
pub mod sys {
    include!(concat!(env!("OUT_DIR"), "/sc.rs"));
}

/// The header of each message between the REPL and its child.
pub use sys::sc_msg_header as MsgHeader;

/// The kinds of message ([`MsgHeader::kind`]).
pub use sys::sc_msg as msg;

/// What a call left (`enum sc_status`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Done.
    Ok,
    /// An error in the student's code. From a `Session`, which keeps part
    /// of a failed cell (PLANO-REPL.md §3.3); from a [`Child`], the session
    /// is as before the cell.
    Error,
    /// The session must be recreated and its log replayed.
    Lost,
    /// [`Mode::Whole`]: the cell stops short, where the parser wanted more.
    /// It is an [`Status::Error`] without the diagnostics, and so is what
    /// the session keeps of it.
    Incomplete,
}

impl Status {
    /// The status of a raw result; anything unknown is [`Status::Lost`].
    pub fn from_raw(s: u32) -> Status {
        match s {
            sys::sc_status::SC_OK => Status::Ok,
            sys::sc_status::SC_ERROR => Status::Error,
            sys::sc_status::SC_INCOMPLETE => Status::Incomplete,
            _ => Status::Lost,
        }
    }

    pub fn to_raw(self) -> u32 {
        match self {
            Status::Ok => sys::sc_status::SC_OK,
            Status::Error => sys::sc_status::SC_ERROR,
            Status::Lost => sys::sc_status::SC_LOST,
            Status::Incomplete => sys::sc_status::SC_INCOMPLETE,
        }
    }
}

/// What a cell does (`enum sc_cell_mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Compile and link: the shared object and the exported names.
    So,
    /// Parse and generate IR only, as in a replay: the exported names.
    Replay,
    /// [`Mode::So`], or [`Status::Incomplete`] if the cell stops short: its
    /// first error is at its end (a declaration without its `;`, a header
    /// without its body). The REPL asks so on Enter.
    Whole,
    /// The cell is an expression without its `;`: its type, in place of the
    /// exported names. Nothing is generated, but as a failed cell it leaves
    /// part of itself in a [`Session`]; a [`Child`] goes back to before it.
    Type,
}

impl Mode {
    pub fn from_raw(m: u32) -> Option<Mode> {
        match m {
            sys::sc_cell_mode::SC_CELL_SO => Some(Mode::So),
            sys::sc_cell_mode::SC_CELL_REPLAY => Some(Mode::Replay),
            sys::sc_cell_mode::SC_CELL_WHOLE => Some(Mode::Whole),
            sys::sc_cell_mode::SC_CELL_TYPE => Some(Mode::Type),
            _ => None,
        }
    }

    pub fn to_raw(self) -> u32 {
        match self {
            Mode::So => sys::sc_cell_mode::SC_CELL_SO,
            Mode::Replay => sys::sc_cell_mode::SC_CELL_REPLAY,
            Mode::Whole => sys::sc_cell_mode::SC_CELL_WHOLE,
            Mode::Type => sys::sc_cell_mode::SC_CELL_TYPE,
        }
    }
}

pub mod wire;
mod child;
pub use child::{interrupt, on_ctrl_c, Answer, Child, Repl, ReplError};

mod session;
#[cfg(feature = "wasm")]
pub mod wasm;
pub use session::*;
