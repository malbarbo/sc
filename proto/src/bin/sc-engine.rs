//! The engine with the REPL's protocol (PLANO-REPL.md §1.1):
//!
//!   sc-engine -cc1 ... | ld ...   the one-shot engine (sc_main)
//!   sc-engine session             the child: the protocol on stdin and stdout
//!   sc-engine repl <dir> <cell>... -- <session args>
//!       a test of the REPL's side: each cell (a file, or %undo) goes to a
//!       child session through a Repl, which replays the log when the session
//!       is lost. Prints "== <cell> <status>" and the diagnostics of each, and
//!       writes the shared object of each accepted cell as <dir>/<k>.so (k:
//!       the cell's position, from 1, in three digits); an undo removes the
//!       last one. Exits with 1 if the session cannot go on.
use sc_proto::{main as sc_main, serve, Repl, Status};
use std::ffi::c_int;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::process::ExitCode;

extern "C" {
    fn dup2(old: c_int, new: c_int) -> c_int;
}

/// The child: the protocol goes on a copy of stdout, and the engine's own
/// writes to stdout go to stderr, so they cannot break a message.
fn session() -> Result<(), Box<dyn std::error::Error>> {
    let out = File::from(io::stdout().as_fd().try_clone_to_owned()?);
    if unsafe { dup2(2, 1) } < 0 {
        return Err(io::Error::last_os_error().into());
    }
    serve(&mut BufReader::new(io::stdin().lock()), &mut BufWriter::new(out))?;
    Ok(())
}

fn repl(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let sep = args.iter().position(|a| a == "--").ok_or("repl <dir> <cell>... -- <session args>")?;
    let (dir, cells) = args[..sep].split_first().ok_or("repl <dir> <cell>... -- <session args>")?;
    let mut r = Repl::new(std::env::current_exe()?, args[sep + 1..].to_vec())?;
    let mut sos: Vec<PathBuf> = vec![];
    let mut out = io::stdout().lock();
    for (k, cell) in cells.iter().enumerate() {
        let a = if cell == "%undo" { r.undo() } else { r.cell(&fs::read(cell)?) };
        let a = a.map_err(|e| {
            let _ = writeln!(out, "== {cell} restart: {e}");
            e
        })?;
        let st = match a.status {
            Status::Ok => "ok",
            Status::Error => "error",
            Status::Lost => "lost",
            Status::Incomplete => "incomplete",
        };
        writeln!(out, "== {cell} {st}")?;
        out.write_all(a.diags.as_bytes())?;
        if cell == "%undo" && a.status == Status::Ok {
            fs::remove_file(sos.pop().ok_or("undo without a shared object")?)?;
        } else if a.status == Status::Ok {
            let so = PathBuf::from(dir).join(format!("{:03}.so", k + 1));
            fs::write(&so, &a.so)?;
            sos.push(so);
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args_os().map(|a| a.to_string_lossy().into_owned()).collect();
    let res = match args.get(1).map(String::as_str) {
        Some("session") => session(),
        Some("repl") => repl(&args[2..]),
        _ => return match sc_main(&args) {
            Ok(code) => ExitCode::from(code as u8),
            Err(e) => {
                eprintln!("sc: {e}");
                ExitCode::from(1)
            }
        },
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sc: {e}");
            ExitCode::from(1)
        }
    }
}
