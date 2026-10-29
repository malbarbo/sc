//! What of the Rust crate sc.wasm links: the loader (`sc::loader`) on the
//! host's primitives, and its exports.

use sc::loader::{libdirs, Error, Loader, STACK};
use std::path::PathBuf;

/// scload inside sc.wasm: runs wasm shared objects from files, with the
/// arguments of sc.wasm's WASI, `[-L dir]... [--stack bytes] first.so
/// [entry.so]...`. The program's own arguments, its directories and its
/// stdio are those the host gives the modules by default. Returns the exit
/// code.
#[no_mangle]
pub extern "C" fn sc_scload() -> i32 {
    let mut args = std::env::args().skip(1);
    let (mut dirs, mut mods) = (Vec::new(), Vec::new());
    let mut stack = STACK;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-L" => dirs.extend(args.next().map(PathBuf::from)),
            "--stack" => stack = args.next().and_then(|s| s.parse().ok()).unwrap_or(STACK),
            _ if a.starts_with("-L") => dirs.push(PathBuf::from(&a[2..])),
            _ => mods.push(PathBuf::from(a)),
        }
    }
    let run = || -> sc::loader::Result<i32> {
        let mut l: Loader = Loader::new(Default::default(), libdirs(dirs), stack)?;
        let mut go = || -> sc::loader::Result<i32> {
            for (i, m) in mods.iter().enumerate() {
                l.load(m, i > 0)?;
            }
            l.run()
        };
        go().map_err(|e| l.why(e))
    };
    match run() {
        Ok(c) | Err(Error::Exit(c)) => c,
        Err(e) => {
            eprintln!("scload: {e}");
            134
        }
    }
}
