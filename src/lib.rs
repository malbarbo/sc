//! sc's library: the [`loader`] runs the wasm shared objects the compiler
//! makes, and the [`engine`] (the `engine` feature) is the compiler, linked
//! in, with the protocol between the REPL and its child.

pub mod loader;
#[cfg(feature = "engine")]
pub mod engine;
