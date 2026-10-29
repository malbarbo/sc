//! sc's library: the [`loader`] runs the wasm shared objects the compiler
//! makes; the [`engine`] (the `machine` feature) is the compiler's API, with
//! the protocol between the REPL and its child (the `engine` feature); and
//! the [`machine`] takes a cell from source to run, over either.

pub mod loader;
#[cfg(feature = "machine")]
pub mod engine;
#[cfg(feature = "machine")]
pub mod machine;
