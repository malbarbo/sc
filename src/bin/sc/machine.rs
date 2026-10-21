//! A cell from source to run: the compiler in its child ([`sc::engine::Repl`])
//! makes the cell a shared object, and the loader, which lives as long as the
//! REPL, loads it on top of the cells before. Loading runs the cell's
//! statements at the top level, which the session's incremental mode accepts.
use sc::engine::{Answer, Repl, Status};
use crate::show;
use sc::loader::{source_name, Loader, STACK};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
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

/// How much the compiler checks (PLANO-VERIFICACOES.md §3): `off` is a
/// common compiler's, `on` the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Safety {
    Off,
    On,
    Strict,
}

impl Safety {
    pub const ALL: [Safety; 3] = [Safety::Off, Safety::On, Safety::Strict];

    pub fn name(self) -> &'static str {
        match self {
            Safety::Off => "off",
            Safety::On => "on",
            Safety::Strict => "strict",
        }
    }

    pub fn parse(s: &str) -> Option<Safety> {
        Safety::ALL.into_iter().find(|l| l.name() == s)
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
    lang: Lang,
    safety: Safety,
    argv: Vec<String>,
    repl: Repl,
    /// The build directory, whose headers are not the student's code.
    build: PathBuf,
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

/// The engine Ctrl-C interrupts: the last session's.
static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);

/// How long a cell may take to compile, and how much memory the compiler
/// may keep: above it, a new one replays the cells (PLANO-REPL.md §4).
const TIMEOUT: Duration = Duration::from_secs(60);
const MEMORY: u64 = 1 << 30;

/// A Ctrl-C stops the compile or the code that runs, whichever it is.
extern "C" fn interrupt(_: libc::c_int) {
    sc::engine::interrupt();
    // the main thread holds the lock only to put a new engine there
    if let Ok(e) = ENGINE.try_lock() {
        if let Some(e) = e.as_ref() {
            e.increment_epoch();
        }
    }
}

/// The build directory: `SC_BUILD`, else the first one above this binary
/// with the session's arguments (`build/sc`, `build/cargo/release/sc`).
fn build_dir() -> std::io::Result<PathBuf> {
    if let Some(b) = std::env::var_os("SC_BUILD") {
        return Ok(b.into());
    }
    let exe = std::env::current_exe()?.canonicalize()?;
    Ok(exe.ancestors().skip(1).find(|d| d.join("repl").is_dir()).map(PathBuf::from).unwrap_or_default())
}

impl Machine {
    /// A session in `lang` at `safety`, whose programs see `argv`. One that
    /// is not `interactive` takes one cell, the whole program. Only an
    /// `interactive` one is stopped by Ctrl-C: the epoch checks cost in
    /// every loop and call.
    pub fn new(lang: Lang, safety: Safety, argv: Vec<String>, interactive: bool) -> crate::Result<Machine> {
        let b = build_dir()?;
        let file = if interactive { "" } else { "-file" };
        let args_file = b.join("repl").join(format!("{}-{}{file}.args", lang.name(), safety.name()));
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
        let mut repl = Repl::new(std::env::current_exe()?, args)?;
        repl.limit(Some(TIMEOUT), Some(MEMORY));

        let mut wasi = WasiCtxBuilder::new();
        wasi.inherit_stdio().inherit_env().args(&argv);
        let libdir = b.join("sysroot/lib/wasm32-wasip1");
        let mut loader = Loader::new(wasi.build_p1(), vec![libdir], STACK, interactive)?;
        loader.keep_stacks();
        let shown = Arc::new(Mutex::new(None));
        let (s, null) = (Arc::clone(&shown), if lang == Lang::C { "NULL" } else { "nullptr" });
        loader.host("__sc_show_int", &[ValType::I32, ValType::I32, ValType::I64], move |mem, a| {
            *s.lock().unwrap() = Some(show::int(mem, a[0].unwrap_i32() as u32, a[1].unwrap_i32() as u32, a[2].unwrap_i64(), null));
        });
        let s = Arc::clone(&shown);
        loader.host("__sc_show_float", &[ValType::I32, ValType::I32, ValType::F64], move |mem, a| {
            *s.lock().unwrap() = Some(show::float(mem, a[0].unwrap_i32() as u32, a[1].unwrap_i32() as u32, a[2].unwrap_f64()));
        });
        if interactive && ENGINE.lock().unwrap().replace(loader.interrupter()).is_none() {
            // SAFETY: the handler only bumps an atomic counter
            unsafe { libc::signal(libc::SIGINT, interrupt as *const () as libc::sighandler_t) };
        }
        Ok(Machine { lang, safety, argv, repl, build: b, loader, loaded: 0, pending: None, shown })
    }

    pub fn safety(&self) -> Safety {
        self.safety
    }

    /// Starts over, empty, at `safety`: the cells before would have to run
    /// again to be compiled at another level.
    pub fn restart(&mut self, safety: Safety) -> crate::Result<()> {
        *self = Machine::new(self.lang, safety, self.argv.clone(), true)?;
        Ok(())
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
    /// left open, or an error where it ends ([`sc::engine::Mode::Whole`]: a
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
        let out = self.exec(|l| l.load_bytes(&name, &a.so, true));
        if !self.loader.is_loaded(&name) {
            // nothing of it is in the loader (an import of another type):
            // nor is it in the compiler, so that it can be written again
            self.repl.undo()?;
        }
        Ok(out)
    }

    /// Undoes the last accepted cell in the compiler: its names can be
    /// defined again. What its code did stays done.
    pub fn undo(&mut self) -> crate::Result<bool> {
        let a = self.repl.undo()?;
        eprint!("{}", a.diags);
        Ok(a.status == Status::Ok)
    }

    /// Runs `main` and exits.
    pub fn main(&mut self) -> Outcome {
        self.exec(|l| l.run().map(drop))
    }

    /// The status of a program that exits with `code`: at `strict`, the
    /// blocks it lost are told, and make a 0 a 1.
    pub fn exit(&mut self, code: i32) -> i32 {
        if self.safety == Safety::Strict && self.leaks() > 0 && code == 0 { 1 } else { code }
    }

    /// Tells the blocks of the heap that nothing reaches any more (at `on`
    /// and `strict`, whose allocator keeps them): how many groups of them,
    /// by where they were allocated.
    pub fn leaks(&mut self) -> usize {
        const MOST: usize = 10;
        self.loader.arm();
        let leaks = match self.loader.leaks() {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{}", runtime_error(&e, &mut self.loader, &self.build));
                return 0;
            }
        };
        let plural = |n: u32, what: &str| if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") };
        for l in leaks.iter().take(MOST) {
            eprintln!("error: memory leak: {} in {}, never freed", plural(l.bytes, "byte"), plural(l.blocks, "block"));
            let fs = l.allocated.as_ref().map(|bt| frames(bt, &self.loader, &self.build, false)).unwrap_or_default();
            if l.allocated.is_none() {
                eprintln!("sc keeps where only the first blocks of a program were allocated");
            } else if !fs.is_empty() {
                eprintln!("{} allocated", if l.blocks == 1 { "the block was" } else { "the blocks were" });
                fs.iter().for_each(|f| eprintln!("    {f}"));
            }
        }
        if leaks.len() > MOST {
            let rest = &leaks[MOST..];
            let bytes = rest.iter().map(|l| l.bytes).sum();
            let blocks = rest.iter().map(|l| l.blocks).sum();
            eprintln!("... and {} in {} more, allocated elsewhere", plural(bytes, "byte"), plural(blocks, "block"));
        }
        leaks.len()
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
                eprintln!("{}", runtime_error(&e, &mut self.loader, &self.build));
                let _ = self.loader.set_stack_pointer(sp);
                // the frames the trap left below sp keep their shadow (ASan)
                if let (true, Some(top)) = (self.loader.defines("__asan_unpoison_memory_region"), sp.i32()) {
                    self.loader.arm();
                    let _ = self.loader.call("__asan_unpoison_memory_region", &[Val::I32(16), Val::I32(top - 16)]);
                }
                Outcome::Error
            }
        }
    }
}

/// What stopped the code, and where: the frames of the student's code.
fn runtime_error(e: &Error, loader: &mut Loader, build: &Path) -> String {
    let msg = message(e, loader);
    let library = !msg.is_empty() && msg != "aborted";
    let mut out: Vec<String> = Some(msg).filter(|m| !m.is_empty()).into_iter().collect();
    let bt = e.downcast_ref::<wasmtime::WasmBacktrace>();
    out.extend(bt.map(|bt| frames(bt, loader, build, library)).unwrap_or_default().iter().map(|f| format!("    {f}")));
    // the block of a report of ASan's: where it was freed and allocated
    let (allocated, freed) = loader.blamed();
    let and = if freed.is_some() { "and allocated" } else { "the block was allocated" };
    for (what, bt) in [("the block was freed", freed), (and, allocated)] {
        let fs = bt.map(|bt| frames(&bt, loader, build, false)).unwrap_or_default();
        if !fs.is_empty() {
            out.push(what.into());
            out.extend(fs.iter().map(|f| format!("    {f}")));
        }
    }
    out.join("\n")
}

/// What stopped the code; empty when libsc_rt has said it (a check of
/// UBSan or of libc++'s hardening, rt/sc_rt.c).
fn message(e: &Error, loader: &mut Loader) -> String {
    let bt = e.downcast_ref::<wasmtime::WasmBacktrace>();
    if bt.is_some_and(|bt| bt.frames().iter().any(|f| f.func_name() == Some("__sc_rt_report"))) {
        return String::new();
    }
    let fault = fault_address(e);
    if let Some(n) = fault.and_then(|a| loader.undefined_at(a)) {
        return format!("error: undefined symbol: {n}");
    }
    // the stack grows down from STACK: below 0 it wraps around
    let below = loader.stack_pointer().i32().is_some_and(|sp| sp as u32 > STACK);
    // -ftrivial-auto-var-init=pattern's pointer, plus an offset: a local
    // the code never set (--safety=on)
    let uninit = fault.is_some_and(|a| (0xffff_ffff..0x1_0000_ffff).contains(&a));
    match e.downcast_ref::<Trap>() {
        Some(Trap::Interrupt) => "interrupted".into(),
        Some(Trap::StackOverflow) => "error: stack overflow (infinite recursion?)".into(),
        Some(Trap::MemoryOutOfBounds) if below => "error: stack overflow (infinite recursion?)".into(),
        Some(Trap::MemoryOutOfBounds) if uninit => "error: invalid memory access through an uninitialized pointer".into(),
        Some(Trap::MemoryOutOfBounds) => "error: invalid memory access".into(),
        Some(Trap::IntegerDivisionByZero) => "error: division by zero".into(),
        Some(Trap::IntegerOverflow) => "error: integer overflow in a division (INT_MIN / -1)".into(),
        Some(Trap::BadSignature) => "error: call through a function pointer of the wrong type".into(),
        Some(Trap::IndirectCallToNull) => "error: call through a null function pointer".into(),
        Some(Trap::TableOutOfBounds) => "error: call through an invalid function pointer".into(),
        Some(Trap::UnreachableCodeReached) => "aborted".into(),
        Some(t) => format!("error: {t}"),
        None => format!("error: {}", e.root_cause()),
    }
}

/// The frames of `bt`, innermost first, as `at f (file:line:column)`: those
/// of the student's code, after the innermost frame if it is not (a
/// function of the libraries, where the code stopped; not when `abort`
/// stopped it, which says it was called). A frame repeated in a row, as in
/// a recursion, comes once, with how many times; a cycle of 2 to 4 frames
/// too. A long list keeps its start and its end.
fn frames(bt: &wasmtime::WasmBacktrace, loader: &Loader, build: &Path, library: bool) -> Vec<String> {
    let build = build.to_string_lossy();
    // (function, where in the student's code), inlined functions first
    let entries = bt.frames().iter().flat_map(|f| {
        let name = f.func_name().map(source_name).unwrap_or_default();
        let syms: Vec<_> = loader.symbols(f).into_iter().map(|s| {
            // the libraries' own sources are wasi-sdk's (wasisdk://...)
            let file = s.file.unwrap_or_default();
            let user = !file.is_empty() && !file.starts_with(&*build) && !file.contains("://");
            let at = s.line.filter(|_| user).map(|l| format!("{file}:{l}:{}", s.column.unwrap_or(0)));
            (s.name.as_deref().map(source_name).unwrap_or_else(|| name.clone()), at)
        }).collect();
        if syms.is_empty() { vec![(name, None)] } else { syms }
    });
    let mut fs: Vec<(String, u32)> = Vec::new();
    for (k, (n, at)) in entries.enumerate() {
        let line = match at {
            Some(at) if n.starts_with("__stmts__") => format!("at {at}"),
            Some(at) => format!("at {n} ({at})"),
            None if k == 0 && library && !n.is_empty() => format!("in {n}"),
            None => continue,
        };
        match fs.last_mut() {
            Some((l, times)) if *l == line => *times += 1,
            _ => fs.push((line, 1)),
        }
    }
    // a stack overflow stops a function before its first line
    if let [(first, 1), rest @ ..] = &fs[..] {
        let name = first.strip_prefix("in ").unwrap_or_default();
        if rest.iter().any(|(l, _)| l.starts_with(&format!("at {name} ("))) {
            fs.remove(0);
        }
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < fs.len() {
        // a recursion through more than one function: its cycle once
        let cycle = (2..=4).filter_map(|p| {
            let k = (1..).take_while(|&k| fs.get(i + (k + 1) * p - 1).is_some()
                && fs[i + k * p..i + (k + 1) * p] == fs[i..i + p]).count() + 1;
            (k >= 3).then_some((p, k))
        }).next();
        let (p, k) = cycle.unwrap_or((1, 1));
        out.extend(fs[i..i + p].iter()
            .map(|(l, times)| if *times > 1 { format!("{l}, {times} times") } else { l.clone() }));
        if k > 1 {
            out.push(format!("(the {p} frames above, {k} times)"));
        }
        i += p * k;
    }
    // what is left of a long recursion: its start and its end
    const HEAD: usize = 10;
    const TAIL: usize = 3;
    if out.len() > HEAD + TAIL + 1 {
        let more = out.len() - HEAD - TAIL;
        out.splice(HEAD..out.len() - TAIL, [format!("... {more} more")]);
    }
    out
}

/// The address of an access out of the memory, from wasmtime's description
/// of the fault, which it keeps to itself as a type.
fn fault_address(e: &Error) -> Option<u64> {
    e.chain().find_map(|c| {
        let s = c.to_string();
        let hex = s.strip_prefix("memory fault at wasm address 0x")?.split(' ').next()?;
        u64::from_str_radix(hex, 16).ok()
    })
}
