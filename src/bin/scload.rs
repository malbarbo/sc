//! scload: runs wasm shared objects from files (see sc::loader).
//!
//! usage: scload [-L dir]... [--dir host::guest]... [--stack bytes] first.so [entry.so]... [-- args...]

use sc::loader::native::Wasmtime;
use sc::loader::{Error, Loader, STACK};
use std::path::PathBuf;
use wasmtime_wasi::{FsPerms, WasiCtxBuilder};

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut libdirs, mut mods, mut dirs, mut prog_args) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut stack = STACK;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-L" => libdirs.push(PathBuf::from(args.next().expect("-L dir"))),
            "--dir" => dirs.push(args.next().expect("--dir host::guest")),
            "--stack" => stack = args.next().and_then(|s| s.parse().ok()).expect("--stack bytes"),
            "--" => { prog_args.extend(args.by_ref()); }
            _ if a.starts_with("-L") => libdirs.push(PathBuf::from(&a[2..])),
            _ => mods.push(PathBuf::from(a)),
        }
    }
    if mods.is_empty() {
        eprintln!("usage: scload [-L dir]... [--dir host::guest]... [--stack bytes] first.so [entry.so]... [-- args...]");
        std::process::exit(2);
    }
    let code = match run(libdirs, mods, dirs, prog_args, stack) {
        Ok(c) => c,
        Err(e) => match e.downcast_ref::<Error>() {
            Some(Error::Exit(c)) => *c,
            _ => {
                eprintln!("scload: {e}");
                134
            }
        },
    };
    std::process::exit(code);
}

fn run(libdirs: Vec<PathBuf>, mods: Vec<PathBuf>, dirs: Vec<String>, args: Vec<String>, stack: u32)
       -> Result<i32, Box<dyn std::error::Error>> {
    let mut b = WasiCtxBuilder::new();
    b.inherit_stdio().inherit_env().arg(mods[0].to_string_lossy()).args(&args);
    for d in dirs {
        let (h, g) = d.split_once("::").unwrap_or((&d, &d));
        b.preopened_dir(h, g, FsPerms::ReadWrite)?;
    }
    let mut l = Loader::new(Wasmtime::new(b.build_p1())?, sc::loader::libdirs(libdirs), stack)?;
    for (i, m) in mods.iter().enumerate() {
        l.load(m, i > 0)?;
    }
    Ok(l.run()?)
}
