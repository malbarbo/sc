// sc links the engine (PLANO-REPL.md §1.1): the arguments that sc-proto's
// build.rs read from SC_LINK_ARGS (scripts/link.py --emit-link), for sc
// alone, as scload does not use it.
use std::{env, fs};

fn main() {
    let Some(file) = env::var_os("DEP_SC_LINK_ARGS") else {
        return;
    };
    for arg in fs::read_to_string(&file).unwrap().lines().chain(["-lstdc++"]) {
        println!("cargo:rustc-link-arg-bin=sc={arg}");
    }
}
