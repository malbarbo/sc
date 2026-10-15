//! A cell from source to run: the compiler in its child ([`sc_proto::Repl`])
//! makes the cell a shared object, and the loader, which lives as long as the
//! REPL, loads it on top of the cells before. Loading runs the cell's
//! statements at the top level, which the session's incremental mode accepts.
use sc_proto::{Answer, Repl, Status};
use crate::show;
use scload::{Loader, STACK};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use wasmtime::{Engine, Error, ThrownException, Trap, Val, ValType};
use wasmtime_wasi::{I32Exit, WasiCtxBuilder};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    C,
    Cxx,
}

impl Lang {
    pub fn name(self) -> &'static str {
        match self {
            Lang::C => "c",
            Lang::Cxx => "c++",
        }
    }
}

/// What Enter does with the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enter {
    Run,
    /// Opens a line this many brackets in.
    Open(usize),
}

/// What became of a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    /// The compiler turned it down, or its code stopped with an error. The
    /// diagnostics are written.
    Error,
    /// The code called `exit`.
    Exit(i32),
}

pub struct Machine {
    repl: Repl,
    loader: Loader,
    /// Cells loaded, which names each module for the loader.
    loaded: usize,
    /// The input Enter compiled to know it whole, and the answer, which the
    /// cell takes.
    pending: Option<(String, Answer)>,
    /// The value the last cell shows, if it ends in an expression without
    /// its `;`.
    shown: Arc<Mutex<Option<String>>>,
}

/// The engine Ctrl-C interrupts.
static ENGINE: OnceLock<Engine> = OnceLock::new();

extern "C" fn interrupt(_: libc::c_int) {
    if let Some(e) = ENGINE.get() {
        e.increment_epoch();
    }
}

/// The build directory: `SC_BUILD`, else the one of `build/loader/release/sc`.
fn build_dir() -> std::io::Result<PathBuf> {
    if let Some(b) = std::env::var_os("SC_BUILD") {
        return Ok(b.into());
    }
    let exe = std::env::current_exe()?.canonicalize()?;
    Ok(exe.ancestors().nth(3).map(PathBuf::from).unwrap_or_default())
}

impl Machine {
    /// A session in `lang`, whose programs see `argv`. One that is not
    /// `interactive` takes one cell, the whole program. Only an `interactive`
    /// one is stopped by Ctrl-C: the epoch checks cost in every loop and call.
    pub fn new(lang: Lang, argv: Vec<String>, interactive: bool) -> crate::Result<Machine> {
        let b = build_dir()?;
        let engine = b.join("sc-engine-full");
        let args_file = b.join("repl").join(format!("{}.args", lang.name()));
        let mut args: Vec<String> = std::fs::read_to_string(&args_file)
            .map_err(|e| format!("{}: {e} (make repl)", args_file.display()))?
            .lines()
            .map(String::from)
            .collect();
        let link = args.iter().position(|a| a == "--").unwrap_or(args.len());
        if crate::reader::color(std::io::stderr().is_terminal()) {
            args.insert(link, "-fcolor-diagnostics".into());
        }
        if !interactive {
            // the file is the whole program: Sema judges what it leaves
            // unused as a one-shot compile does
            args.insert(link, "-sc-one-cell".into());
        }
        if !engine.exists() {
            return Err(format!("{}: not found (make repl)", engine.display()).into());
        }
        let repl = Repl::new(engine, args)?;

        let mut wasi = WasiCtxBuilder::new();
        wasi.inherit_stdio().inherit_env().args(&argv);
        let libdir = b.join("sysroot/lib/wasm32-wasip1");
        let mut loader = Loader::new(wasi.build_p1(), vec![libdir], STACK, interactive)?;
        let shown = Arc::new(Mutex::new(None));
        let (s, null) = (Arc::clone(&shown), if lang == Lang::C { "NULL" } else { "nullptr" });
        loader.host("__sc_show_int", &[ValType::I32, ValType::I32, ValType::I64], move |mem, a| {
            *s.lock().unwrap() = Some(show::int(mem, a[0].unwrap_i32() as u32, a[1].unwrap_i32() as u32, a[2].unwrap_i64(), null));
        });
        let s = Arc::clone(&shown);
        loader.host("__sc_show_float", &[ValType::I32, ValType::I32, ValType::F64], move |mem, a| {
            *s.lock().unwrap() = Some(show::float(mem, a[0].unwrap_i32() as u32, a[1].unwrap_i32() as u32, a[2].unwrap_f64()));
        });
        if interactive && ENGINE.set(loader.interrupter()).is_ok() {
            // SAFETY: the handler only bumps an atomic counter
            unsafe { libc::signal(libc::SIGINT, interrupt as *const () as libc::sighandler_t) };
        }
        Ok(Machine { repl, loader, loaded: 0, pending: None, shown })
    }

    /// Compiles `src` without running it: whether the compiler accepted it.
    pub fn compile(&mut self, src: &str) -> crate::Result<bool> {
        let a = self.repl.cell(src.as_bytes())?;
        eprint!("{}", a.diags);
        Ok(a.status == Status::Ok)
    }

    /// Writes the type of `expr` without running it: whether it is one.
    pub fn type_of(&mut self, expr: &str) -> crate::Result<bool> {
        let a = self.repl.type_of(expr.as_bytes())?;
        eprint!("{}", a.diags);
        match a.syms.first() {
            Some(t) if a.status == Status::Ok => println!("{t}"),
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// What Enter does with `input`, the caret at `cursor`, as the compiler
    /// sees it. With the caret at the end (or in an input of one line), it
    /// runs the input unless the input stops short: a bracket or a comment
    /// left open, or an error where it ends ([`sc_proto::Mode::Whole`]: a
    /// declaration without its `;`, a function's header without its body).
    /// A blank line runs any input with no bracket open, for the compiler to
    /// say what is wrong with it. With the caret back in an input of several
    /// lines, Enter opens a line there.
    pub fn enter(&mut self, input: &str, cursor: usize) -> Enter {
        if crate::shell::split(input).is_some() || input.trim().is_empty() {
            return Enter::Run;
        }
        if self.pending.as_ref().is_some_and(|(p, _)| p == input) {
            return Enter::Run;
        }
        let mut depth = |s: &str| self.repl.depth(s.as_bytes()).unwrap_or(0) as usize;
        let at_end = input[cursor..].trim().is_empty();
        if !at_end && input.contains('\n') {
            return Enter::Open(depth(&input[..cursor]));
        }
        let d = depth(input);
        if d > 0 {
            return Enter::Open(if at_end { d } else { depth(&input[..cursor]) });
        }
        if input.rsplit('\n').next().is_some_and(|l| l.trim().is_empty()) {
            return Enter::Run;
        }
        match self.repl.cell_if_whole(input.as_bytes()) {
            Ok(a) if a.status == Status::Incomplete => Enter::Open(0),
            Ok(a) => {
                self.pending = Some((input.to_string(), a));
                Enter::Run
            }
            // the cell says what went wrong
            Err(_) => Enter::Run,
        }
    }

    /// Compiles `src` (unless Enter did) and runs it.
    pub fn cell(&mut self, src: &str) -> crate::Result<Outcome> {
        let a = match self.pending.take() {
            Some((p, a)) if p == src => a,
            _ => self.repl.cell(src.as_bytes())?,
        };
        eprint!("{}", a.diags);
        if a.status != Status::Ok {
            return Ok(Outcome::Error);
        }
        self.loaded += 1;
        let name = format!("cell {}", self.loaded);
        Ok(self.exec(|l| l.load_bytes(&name, &a.so, true)))
    }

    /// Undoes the last accepted cell in the compiler: its names can be
    /// defined again. What its code did stays done.
    pub fn undo(&mut self) -> crate::Result<bool> {
        let a = self.repl.undo()?;
        eprint!("{}", a.diags);
        Ok(a.status == Status::Ok)
    }

    /// Runs `main` and exits: the program's exit status.
    pub fn main(&mut self) -> i32 {
        match self.exec(|l| l.run().map(drop)) {
            Outcome::Ok => 0,
            Outcome::Error => 134,
            Outcome::Exit(code) => code,
        }
    }

    /// The names the cells export.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.repl.names()
    }

    /// Runs `f` on the loader: what it printed is flushed, and a trap is
    /// told and leaves the loader ready for the next cell.
    fn exec(&mut self, f: impl FnOnce(&mut Loader) -> wasmtime::Result<()>) -> Outcome {
        let sp = self.loader.stack_pointer();
        self.loader.arm();
        let r = match f(&mut self.loader) {
            Err(e) if e.is::<ThrownException>() => self.loader.uncaught(e).map(drop),
            r => r,
        };
        if let Some(x) = r.as_ref().err().and_then(|e| e.downcast_ref::<I32Exit>()) {
            return Outcome::Exit(x.0);
        }
        if self.loader.defines("fflush") {
            self.loader.arm(); // after Ctrl-C, the deadline has passed
            let _ = self.loader.call("fflush", &[Val::I32(0)]);
        }
        let shown = self.shown.lock().unwrap().take();
        match r {
            Ok(()) => {
                if let Some(v) = shown {
                    println!("{v}");
                }
                Outcome::Ok
            }
            Err(e) => {
                let _ = self.loader.set_stack_pointer(sp);
                eprintln!("{}", runtime_error(&e));
                Outcome::Error
            }
        }
    }
}

fn runtime_error(e: &Error) -> String {
    match e.downcast_ref::<Trap>() {
        Some(Trap::Interrupt) => "interrupted".into(),
        Some(Trap::StackOverflow) => "error: stack overflow".into(),
        Some(Trap::UnreachableCodeReached) => "aborted".into(),
        Some(t) => format!("error: {t}"),
        None => format!("error: {}", e.root_cause()),
    }
}
