//! sc: C and C++ for students, with a REPL. The command line is the one of
//! sgleam and spython: `sc` is the REPL, `sc FILE` runs a program.
//!
//! The engine is linked in (PLANO-REPL.md §1.1, phase 6b): the compiler is
//! the child `sc --engine`, and `sc --cc1 ...` and `sc --ld ...` are the
//! one-shot engine, `sc_main`'s `-cc1` and `ld` (a student's program may be
//! called `ld`). What a session reads, its arguments, the headers and the
//! libraries, is in the pack this binary embeds ([`pack`]): `sc` copied
//! anywhere runs. The one-shot engine reads the disk.
//!
//! `sc --replay <dir> <cell>... -- <session args>` is the test of the REPL's
//! side (test/split.py, test/session.py): each cell (a file, %undo, or
//! %restart, a new child that replays the log) goes to a child session
//! through a Repl, which replays the log when the session is lost. It prints
//! "== <cell> <status>" and the diagnostics of each, and writes the shared
//! object of each accepted cell as <dir>/<k>.so (k: the cell's position,
//! from 1, in three digits); an undo removes the last one. It exits with 1
//! if the session cannot go on.
//!
//! With the `wasm` feature (build/sc-wasm) the engine is
//! `<this program>.wasm`, which `<this program> --precompile` compiles
//! first ([`sc::engine::wasm`]): the test host of `sc.wasm`, whose sessions
//! read the pack too.

mod config;
mod pack;
mod platform;
mod reader;
mod shell;

use bpaf::{Bpaf, Parser};
use platform::{Machine, Native};
use sc::machine::{check, lang_of, run, source, Lang, Outcome, Safety};
use sc::engine::{Repl, Status};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const NAME: &str = "sc";

fn lang_arg() -> impl Parser<Option<Lang>> {
    bpaf::short('x')
        .help("The language, c or c++ (by default, the file's extension, else c)")
        .argument::<String>("LANG")
        .parse(|l| match l.as_str() {
            "c" => Ok(Lang::C),
            "c++" => Ok(Lang::Cxx),
            _ => Err("expected c or c++"),
        })
        .optional()
}

fn safety_arg() -> impl Parser<Safety> {
    bpaf::long("safety")
        .help("How much the compiler checks: off (as a common compiler), on (default) or strict")
        .argument::<String>("LEVEL")
        .parse(|l| Safety::parse(&l).ok_or("expected off, on or strict"))
        .fallback(Safety::On)
}

#[derive(Debug, Clone, Bpaf)]
enum Command {
    /// Start the interactive REPL (default).
    #[bpaf(command)]
    Repl {
        #[bpaf(external(lang_arg))]
        lang: Option<Lang>,
        #[bpaf(external(safety_arg))]
        safety: Safety,
        /// Suppress the welcome message.
        #[bpaf(short)]
        quiet: bool,
        /// File to load before the REPL starts.
        #[bpaf(positional("FILE"))]
        file: Option<PathBuf>,
    },
    /// Compile and run a program.
    #[bpaf(command)]
    Run {
        #[bpaf(external(lang_arg))]
        lang: Option<Lang>,
        #[bpaf(external(safety_arg))]
        safety: Safety,
        /// The program.
        #[bpaf(positional("FILE"))]
        file: PathBuf,
        /// The program's arguments.
        #[bpaf(positional("ARG"), many)]
        args: Vec<String>,
    },
    /// Compile a program without running it.
    #[bpaf(command)]
    Check {
        #[bpaf(external(lang_arg))]
        lang: Option<Lang>,
        #[bpaf(external(safety_arg))]
        safety: Safety,
        /// The program.
        #[bpaf(positional("FILE"))]
        file: PathBuf,
    },
    /// Show help information.
    #[bpaf(command)]
    Help {
        /// Command to show the help of.
        #[bpaf(positional("COMMAND"), optional)]
        command: Option<String>,
    },
}

/// The parser, with the two shorthands of sgleam and spython: a leading
/// `FILE` is a `run`, and bare options are a `repl`. `None` is the bare `sc`.
fn cli() -> bpaf::OptionParser<Option<Command>> {
    let lang = lang_arg();
    let safety = safety_arg();
    let file = bpaf::positional::<PathBuf>("FILE");
    let args = bpaf::positional::<String>("ARG").many();
    let file_as_run = bpaf::construct!(Command::Run { lang, safety, file, args });

    let lang = lang_arg();
    let safety = safety_arg();
    let quiet = bpaf::short('q').help("Suppress the welcome message.").switch();
    let file = bpaf::pure(None);
    let options_as_repl = bpaf::construct!(Command::Repl { lang, safety, quiet, file });

    let command = bpaf::construct!([command(), file_as_run, options_as_repl]).optional();
    bpaf::construct!(command)
        .to_options()
        .version(env!("CARGO_PKG_VERSION"))
        .descr("C and C++ for students, with a REPL")
}

/// Print what `parser` answers to `args`, which ask it for its help.
fn show_help<T>(parser: bpaf::OptionParser<T>, args: &[&str]) -> ExitCode {
    let Err(err) = parser.run_inner(bpaf::Args::from(args).set_name(NAME)) else {
        return ExitCode::SUCCESS;
    };
    err.print_message(80);
    ExitCode::from(err.exit_code() as u8)
}

/// The modes of the engine and of the tests, else the student's.
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args_os().map(|a| a.to_string_lossy().into_owned()).collect();
    let engine = match args.get(1).map(String::as_str) {
        Some("--engine") => {
            pack::install();
            sc::engine::serve_child().map(|()| 0).map_err(Into::into)
        }
        Some(mode @ ("--cc1" | "--ld")) => {
            let mode = if mode == "--cc1" { "-cc1" } else { "ld" };
            let args = [args[0].as_str(), mode].into_iter().chain(args[2..].iter().map(String::as_str));
            sc::engine::main(args).map_err(Into::into)
        }
        Some("--replay") => replay(&args[2..]).map(|()| 0),
        #[cfg(feature = "wasm")]
        Some("--precompile") => sc::engine::wasm::precompile().map(|()| 0).map_err(Into::into),
        _ => return sc(),
    };
    match engine {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("{NAME}: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `sc --replay`: the cells through a [`Repl`], as the REPL sends them.
fn replay(args: &[String]) -> Result<()> {
    let usage = "--replay <dir> <cell>... -- <session args>";
    let sep = args.iter().position(|a| a == "--").ok_or(usage)?;
    let (dir, cells) = args[..sep].split_first().ok_or(usage)?;
    // SC_CHILD: another engine, as test/node/sc-node
    let program = std::env::var_os("SC_CHILD").map_or_else(std::env::current_exe, |p| Ok(p.into()))?;
    let mut r = Repl::new(program, args[sep + 1..].to_vec(), None, None)?;
    let mut sos: Vec<PathBuf> = vec![];
    let mut out = io::stdout().lock();
    for (k, cell) in cells.iter().enumerate() {
        // None: a restart, which has no answer of its own
        let a = match cell.as_str() {
            "%undo" => r.undo().map(Some),
            "%restart" => r.restart().map(|()| None),
            _ => r.cell(&fs::read(cell)?).map(Some),
        };
        let a = a.map_err(|e| {
            let _ = writeln!(out, "== {cell} restart: {e}");
            e
        })?;
        let Some(a) = a else {
            writeln!(out, "== {cell} ok")?;
            continue;
        };
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

/// The command line of the student.
fn sc() -> ExitCode {
    let parsed = match cli().run_inner(bpaf::Args::current_args().set_name(NAME)) {
        Ok(parsed) => parsed,
        Err(err) => {
            err.print_message(80);
            return ExitCode::from(err.exit_code() as u8);
        }
    };
    let result = match parsed.unwrap_or(Command::Repl { lang: None, safety: Safety::On, quiet: false, file: None }) {
        Command::Help { command: None } => return show_help(cli(), &["--help"]),
        // The commands alone, without the shorthands, so that a name that is
        // not a command lists the commands and does not run as a file.
        Command::Help { command: Some(name) } => return show_help(command().to_options(), &[&name, "--help"]),
        Command::Repl { lang, safety, quiet, file } => repl(lang, safety, quiet, file.as_deref()),
        Command::Run { lang, safety, file, args } => run(Native, lang, safety, &file, args),
        Command::Check { lang, safety, file } => check(Native, lang, safety, &file),
    };
    match result {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("{NAME}: {e}");
            ExitCode::FAILURE
        }
    }
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn repl(lang: Option<Lang>, safety: Safety, quiet: bool, file: Option<&Path>) -> Result<i32> {
    let lang = lang_of(lang, file);
    let src = file.map(source).transpose()?;
    let mut machine = Machine::new(Native, lang, safety, vec![NAME.into()], true)?;
    if !quiet {
        print!("{}", shell::welcome_message(lang, safety));
    }
    if let Some(src) = src {
        // the file's declarations are there for the cells; its `main` does
        // not run, as a cell's does not
        match machine.cell(&src)? {
            Outcome::Ok => {}
            Outcome::Error => return Ok(1),
            Outcome::Exit(code) => return Ok(code),
        }
    }
    shell::run(machine)
}
