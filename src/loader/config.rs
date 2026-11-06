// wasmtime's configuration of the loader, which build.rs also compiles the
// pack's libraries with (docs/plan-repl.md §5.4): Module::deserialize takes a
// .cwasm only from an Engine with the same settings.
use wasmtime::{Config, WasmBacktraceDetails};

/// The epoch checks are always there, so that one .cwasm serves `sc FILE`
/// and the REPL: they cost under 1% (NOTES.md §49), and without Ctrl-C
/// the epoch never moves.
pub fn config() -> Config {
    let mut config = Config::new();
    config.epoch_interruption(true);
    // a trap's file and line come from Loader::symbols. Frames: the
    // default of 20 would lose the main of a recursion. A frame takes 16
    // bytes or more of max_wasm_stack's 512 KiB; more would make each
    // backtrace reserve memory that malloc takes from mmap
    config.wasm_backtrace_details(WasmBacktraceDetails::Disable);
    config.wasm_backtrace_max_frames(std::num::NonZeroUsize::new(1 << 15));
    config
}
