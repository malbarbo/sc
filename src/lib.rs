//! sc's library: the [`loader`] runs the wasm shared objects the compiler
//! makes; the [`engine`] (the `repl` feature) is the compiler's API, with
//! the protocol between the REPL and its child (the `engine` feature); the
//! [`repl`] takes a cell from source to run, over either, and the
//! [`shell`] runs the `:` commands over it.

pub mod loader;
#[cfg(feature = "repl")]
pub mod engine;
#[cfg(feature = "repl")]
pub mod repl;
#[cfg(feature = "repl")]
pub mod shell;
