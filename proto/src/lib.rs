//! The engine's API (`engine/sc.h`, PLANO-REPL.md §2) for Rust.
//!
//! [`sys`] is what bindgen generates from the header: the protocol's types
//! and the raw functions. With the `engine` feature the engine is linked, and
//! [`Session`] and [`main`] are a safe API over the raw functions:
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
    /// An error in the student's code; the session is as before the call.
    Error,
    /// The session must be recreated and its log replayed.
    Lost,
}

impl Status {
    /// The status of a raw result; anything unknown is [`Status::Lost`].
    pub fn from_raw(s: u32) -> Status {
        match s {
            sys::sc_status::SC_OK => Status::Ok,
            sys::sc_status::SC_ERROR => Status::Error,
            _ => Status::Lost,
        }
    }

    pub fn to_raw(self) -> u32 {
        match self {
            Status::Ok => sys::sc_status::SC_OK,
            Status::Error => sys::sc_status::SC_ERROR,
            Status::Lost => sys::sc_status::SC_LOST,
        }
    }
}

#[cfg(feature = "engine")]
mod engine;
#[cfg(feature = "engine")]
pub use engine::*;
