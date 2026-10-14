//! sc: C and C++ for students, with a REPL. The command line is the one of
//! sgleam and spython: `sc` is the REPL, `sc FILE` runs a program.
//!
//! A prototype ahead of phase 6b of PLANO-REPL.md: the compiler is the
//! child `sc-engine-full session` of the build directory (`SC_BUILD`, by
//! default the one this binary is in), with the session's arguments that
//! `make repl` writes to `build/repl`.

mod config;
mod machine;
mod reader;
mod shell;
mod show;

use bpaf::{Bpaf, Parser};
use machine::{Lang, Machine, Outcome};
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

#[derive(Debug, Clone, Bpaf)]
enum Command {
    /// Start the interactive REPL (default).
    #[bpaf(command)]
    Repl {
        #[bpaf(external(lang_arg))]
        lang: Option<Lang>,
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
    let file = bpaf::positional::<PathBuf>("FILE");
    let args = bpaf::positional::<String>("ARG").many();
    let file_as_run = bpaf::construct!(Command::Run { lang, file, args });

    let lang = lang_arg();
    let quiet = bpaf::short('q').help("Suppress the welcome message.").switch();
    let file = bpaf::pure(None);
    let options_as_repl = bpaf::construct!(Command::Repl { lang, quiet, file });

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

fn main() -> ExitCode {
    let parsed = match cli().run_inner(bpaf::Args::current_args().set_name(NAME)) {
        Ok(parsed) => parsed,
        Err(err) => {
            err.print_message(80);
            return ExitCode::from(err.exit_code() as u8);
        }
    };
    let result = match parsed.unwrap_or(Command::Repl { lang: None, quiet: false, file: None }) {
        Command::Help { command: None } => return show_help(cli(), &["--help"]),
        // The commands alone, without the shorthands, so that a name that is
        // not a command lists the commands and does not run as a file.
        Command::Help { command: Some(name) } => return show_help(command().to_options(), &[&name, "--help"]),
        Command::Repl { lang, quiet, file } => repl(lang, quiet, file.as_deref()),
        Command::Run { lang, file, args } => run(lang, &file, args),
        Command::Check { lang, file } => check(lang, &file),
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

/// The language of `file`: `-x`, else its extension.
fn lang_of(lang: Option<Lang>, file: Option<&Path>) -> Lang {
    lang.unwrap_or_else(|| match file.and_then(Path::extension).and_then(|e| e.to_str()) {
        Some("cpp" | "cc" | "cxx" | "C") => Lang::Cxx,
        _ => Lang::C,
    })
}

fn read(file: &Path) -> Result<String> {
    std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()).into())
}

fn repl(lang: Option<Lang>, quiet: bool, file: Option<&Path>) -> Result<i32> {
    let lang = lang_of(lang, file);
    let src = file.map(read).transpose()?;
    let mut machine = Machine::new(lang, vec![NAME.into()], true)?;
    if !quiet {
        print!("{}", shell::welcome_message(lang));
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

fn run(lang: Option<Lang>, file: &Path, args: Vec<String>) -> Result<i32> {
    let src = read(file)?;
    let argv = std::iter::once(file.display().to_string()).chain(args).collect();
    let mut machine = Machine::new(lang_of(lang, Some(file)), argv, false)?;
    match machine.cell(&src)? {
        Outcome::Ok => Ok(machine.main()),
        Outcome::Error => Ok(1),
        Outcome::Exit(code) => Ok(code),
    }
}

fn check(lang: Option<Lang>, file: &Path) -> Result<i32> {
    let src = read(file)?;
    let mut machine = Machine::new(lang_of(lang, Some(file)), vec![NAME.into()], false)?;
    Ok(if machine.compile(&src)? { 0 } else { 1 })
}
