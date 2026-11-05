//! The machine of the native `sc`: the compiler is sc.wasm's session in
//! this process, the code on wasmtime, the pack embedded.
use crate::{pack, reader};
use sc::engine::Guest;
use sc::loader::native::Wasmtime;
use std::io::{self, IsTerminal};
use std::time::Duration;
use wasmtime_wasi::WasiCtxBuilder;

/// How long a cell may take to compile, and how much memory the compiler
/// may keep: above it, a new one replays the cells (PLANO-REPL.md §4).
const TIMEOUT: Duration = Duration::from_secs(60);
const MEMORY: u64 = 1 << 30;

#[derive(Clone)]
pub struct Native;

/// The machine of `sc`.
pub type Machine = sc::machine::Machine<Native>;

impl sc::machine::Platform for Native {
    type Compiler = Guest;
    type Runtime = Wasmtime;

    fn compiler(&self) -> io::Result<Guest> {
        Guest::new(Some(TIMEOUT), Some(MEMORY))
    }

    fn runtime(&self, argv: &[String], interactive: bool) -> sc::loader::Result<Wasmtime> {
        let mut wasi = WasiCtxBuilder::new();
        wasi.inherit_stdio().inherit_env().args(argv);
        let rt = Wasmtime::new(wasi.build_p1()).map_err(|e| sc::loader::Error::Fail(e.to_string()))?;
        if interactive {
            rt.on_ctrl_c(sc::engine::interrupt);
        }
        Ok(rt)
    }

    fn read(&self, path: &str) -> Option<&'static [u8]> {
        pack::read(path)
    }

    fn color(&self) -> bool {
        reader::color(io::stderr().is_terminal())
    }
}
