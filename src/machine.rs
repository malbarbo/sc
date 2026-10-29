//! A cell from source to run: the compiler ([`crate::engine::Repl`]) makes
//! the cell a shared object, and the loader, which lives as long as the
//! REPL, loads it on top of the cells before. Loading runs the cell's
//! statements at the top level, which the session's incremental mode
//! accepts. Where the compiler and the code run is the [`Platform`]'s: the
//! native `sc` has the compiler in a child and wasmtime; sc.wasm, the
//! session in the same module and the host's primitives.
use crate::engine::{Answer, Compiler, Repl, Status};
use crate::loader::runtime::Runtime;
use crate::loader::{self, source_name, Error, Frame, Lib, Loader, TrapKind, Val, STACK};
use std::io;

mod show;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Where the engine sees the pack, as the session's arguments name it
/// (scripts/flags.py, PACK).
pub const ROOT: &str = "/sc";

/// What a machine needs of where it runs.
pub trait Platform: Clone + Send + Sync + 'static {
    type Compiler: Compiler;
    type Runtime: Runtime;
    /// A compiler without a session.
    fn compiler(&self) -> io::Result<Self::Compiler>;
    /// The runtime of the code, whose `main` sees `argv`. Only an
    /// `interactive` one is stopped by Ctrl-C.
    fn runtime(&self, argv: &[String], interactive: bool) -> loader::Result<Self::Runtime>;
    /// The file of the pack at `path` from its root (`repl/c-on.args`).
    fn read(&self, path: &str) -> Option<&'static [u8]>;
    /// Whether the diagnostics have colors.
    fn color(&self) -> bool;
}

/// The name of the command at the head of the input and what follows it, or
/// none for code, which starts with `:` only as C++'s `::x`.
pub fn split(input: &str) -> Option<(&str, &str)> {
    let trimmed = input.trim();
    if !trimmed.starts_with(':') || trimmed.starts_with("::") {
        return None;
    }
    let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let (name, arg) = trimmed.split_at(end);
    Some((name, arg.trim_start()))
}

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

pub struct Machine<P: Platform> {
    platform: P,
    lang: Lang,
    safety: Safety,
    argv: Vec<String>,
    repl: Repl<P::Compiler>,
    loader: Loader<P::Runtime>,
    /// Cells loaded, which names each module for the loader.
    loaded: usize,
    /// The input Enter compiled to know it whole, and the answer, which the
    /// cell takes.
    pending: Option<(String, Answer)>,
}

impl<P: Platform> Machine<P> {
    /// A session in `lang` at `safety`, whose programs see `argv`. One that
    /// is not `interactive` takes one cell, the whole program. Only an
    /// `interactive` one is stopped by Ctrl-C.
    pub fn new(platform: P, lang: Lang, safety: Safety, argv: Vec<String>, interactive: bool) -> Result<Machine<P>> {
        let file = if interactive { "" } else { "-file" };
        let args_file = format!("repl/{}-{}{file}.args", lang.name(), safety.name());
        let mut args: Vec<String> = String::from_utf8_lossy(platform.read(&args_file).ok_or(format!("{args_file}: not in the pack"))?)
            .lines()
            .map(String::from)
            .collect();
        let link = args.iter().position(|a| a == "--").unwrap_or(args.len());
        if platform.color() {
            args.insert(link, "-fcolor-diagnostics".into());
        }
        if !interactive {
            // the file is the whole program: Sema judges what it leaves
            // unused as a one-shot compile does
            args.insert(link, "-sc-one-cell".into());
        }
        let repl = Repl::start(platform.compiler()?, args)?;

        let p = platform.clone();
        let libs = Box::new(move |lib: &str| {
            let path = format!("sysroot/lib/wasm32-wasip1/{lib}");
            Some(Lib { wasm: p.read(&path)?.into(), cwasm: p.read(&format!("{path}.cwasm")) })
        });
        let mut loader = Loader::new(platform.runtime(&argv, interactive)?, libs, STACK)?;
        loader.keep_stacks();
        loader.show();
        Ok(Machine { platform, lang, safety, argv, repl, loader, loaded: 0, pending: None })
    }

    pub fn safety(&self) -> Safety {
        self.safety
    }

    /// Starts over, empty, at `safety`: the cells before would have to run
    /// again to be compiled at another level.
    pub fn restart(&mut self, safety: Safety) -> Result<()> {
        *self = Machine::new(self.platform.clone(), self.lang, safety, self.argv.clone(), true)?;
        Ok(())
    }

    /// Compiles `src` without running it: whether the compiler accepted it.
    pub fn compile(&mut self, src: &str) -> Result<bool> {
        let a = self.repl.cell(src.as_bytes())?;
        eprint!("{}", a.diags);
        Ok(a.status == Status::Ok)
    }

    /// Writes the type of `expr` without running it: whether it is one.
    pub fn type_of(&mut self, expr: &str) -> Result<bool> {
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
    /// left open, or an error where it ends ([`crate::engine::Mode::Whole`]: a
    /// declaration without its `;`, a function's header without its body).
    /// A blank line runs any input with no bracket open, for the compiler to
    /// say what is wrong with it. With the caret back in an input of several
    /// lines, Enter opens a line there.
    pub fn enter(&mut self, input: &str, cursor: usize) -> Enter {
        if split(input).is_some() || input.trim().is_empty() {
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
    pub fn cell(&mut self, src: &str) -> Result<Outcome> {
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
            eprint!("{}", self.repl.undo()?.diags);
        }
        Ok(out)
    }

    /// Undoes the last accepted cell in the compiler: its names can be
    /// defined again. What its code did stays done.
    pub fn undo(&mut self) -> Result<bool> {
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
        let leaks = match self.loader.leaks() {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{}", runtime_error(&e, &mut self.loader));
                return 0;
            }
        };
        let plural = |n: u32, what: &str| if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") };
        for l in leaks.iter().take(MOST) {
            eprintln!("error: memory leak: {} in {}, never freed", plural(l.bytes, "byte"), plural(l.blocks, "block"));
            let fs = l.allocated.as_ref().map(|fs| frames(fs, &self.loader, false)).unwrap_or_default();
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

    /// The value the last cell shows, if it ends in an expression without
    /// its `;`. Read before any other code runs: it may be a string on the
    /// stack.
    fn shown(&mut self) -> Option<String> {
        let s = self.loader.shown()?;
        let null = if self.lang == Lang::C { "NULL" } else { "nullptr" };
        let mut read = |a: u32, n: usize| self.loader.read(a, n);
        Some(if s.float {
            show::float(&mut read, s.ty, s.kind, f64::from_bits(s.bits))
        } else {
            show::int(&mut read, s.ty, s.kind, s.bits as i64, null)
        })
    }

    /// Runs `f` on the loader: what it printed is flushed, and a trap is
    /// told and leaves the loader ready for the next cell.
    fn exec(&mut self, f: impl FnOnce(&mut Loader<P::Runtime>) -> loader::Result<()>) -> Outcome {
        let sp = self.loader.stack_pointer();
        let r = f(&mut self.loader);
        if let Err(Error::Exit(c)) = r {
            return Outcome::Exit(c);
        }
        let shown = self.shown();
        if self.loader.defines("fflush") {
            let _ = self.loader.call("fflush", &[Val::I32(0)]);
        }
        match r {
            Ok(()) => {
                if let Some(v) = shown {
                    println!("{v}");
                }
                Outcome::Ok
            }
            Err(e) => {
                eprintln!("{}", runtime_error(&e, &mut self.loader));
                let _ = self.loader.set_stack_pointer(sp);
                // the frames the trap left below sp keep their shadow (ASan)
                if self.loader.defines("__asan_unpoison_memory_region") {
                    let _ = self.loader.call("__asan_unpoison_memory_region", &[Val::I32(16), Val::I32(sp - 16)]);
                }
                Outcome::Error
            }
        }
    }
}

/// What stopped the code, and where: the frames of the student's code.
fn runtime_error<R: Runtime>(e: &Error, loader: &mut Loader<R>) -> String {
    let msg = message(e, loader);
    let library = !msg.is_empty() && msg != "aborted";
    let mut out: Vec<String> = Some(msg).filter(|m| !m.is_empty()).into_iter().collect();
    if let Error::Trap(t) = e {
        out.extend(frames(&t.frames, loader, library).iter().map(|f| format!("    {f}")));
    }
    // the block of a report of ASan's: where it was freed and allocated
    let (allocated, freed) = loader.blamed();
    let and = if freed.is_some() { "and allocated" } else { "the block was allocated" };
    for (what, bt) in [("the block was freed", freed), (and, allocated)] {
        let fs = bt.map(|fs| frames(&fs, loader, false)).unwrap_or_default();
        if !fs.is_empty() {
            out.push(what.into());
            out.extend(fs.iter().map(|f| format!("    {f}")));
        }
    }
    out.join("\n")
}

/// What stopped the code; empty when libsc_rt has said it (a check of
/// UBSan or of libc++'s hardening, rt/sc_rt.c).
fn message<R: Runtime>(e: &Error, loader: &mut Loader<R>) -> String {
    let t = match e {
        Error::Trap(t) => t,
        Error::Fail(m) => return format!("error: {m}"),
        Error::Exit(c) => return format!("exit({c})"),
    };
    if t.frames.iter().any(|f| f.func.as_deref() == Some("__sc_rt_report")) {
        return String::new();
    }
    if let Some(n) = loader.missing() {
        return format!("error: undefined symbol: {n}");
    }
    if let Some(n) = t.fault.and_then(|a| loader.undefined_at(a)) {
        return format!("error: undefined symbol: {n}");
    }
    // the stack grows down from STACK: below 0 it wraps around
    let below = loader.stack_pointer() as u32 > STACK;
    // -ftrivial-auto-var-init=pattern's pointer, plus an offset: a local
    // the code never set (--safety=on)
    let uninit = t.fault.is_some_and(|a| (0xffff_ffff..0x1_0000_ffff).contains(&a));
    match t.kind {
        TrapKind::Interrupt => "interrupted".into(),
        TrapKind::StackOverflow => "error: stack overflow (infinite recursion?)".into(),
        TrapKind::MemoryOutOfBounds if below => "error: stack overflow (infinite recursion?)".into(),
        TrapKind::MemoryOutOfBounds if uninit => "error: invalid memory access through an uninitialized pointer".into(),
        TrapKind::MemoryOutOfBounds => "error: invalid memory access".into(),
        TrapKind::IntegerDivisionByZero => "error: division by zero".into(),
        TrapKind::IntegerOverflow => "error: integer overflow in a division (INT_MIN / -1)".into(),
        TrapKind::BadSignature => "error: call through a function pointer of the wrong type".into(),
        TrapKind::IndirectCallToNull => "error: call through a null function pointer".into(),
        TrapKind::TableOutOfBounds => "error: call through an invalid function pointer".into(),
        TrapKind::Unreachable => "aborted".into(),
        TrapKind::Other => format!("error: {}", t.message),
    }
}

/// The frames of `fs`, innermost first, as `at f (file:line:column)`: those
/// of the student's code, after the innermost frame if it is not (a
/// function of the libraries, where the code stopped; not when `abort`
/// stopped it, which says it was called). A frame repeated in a row, as in
/// a recursion, comes once, with how many times; a cycle of 2 to 4 frames
/// too. A long list keeps its start and its end.
fn frames<R: Runtime>(fs: &[Frame], loader: &Loader<R>, library: bool) -> Vec<String> {
    // the headers' code, compiled in the cell, is from the pack; the
    // compiler-rt in libc++.so from wasi-sdk's build, without -ffile-prefix-map
    let pack = format!("{ROOT}/");
    let theirs = [pack.as_str(), "/src/src/llvm-project/", "/src/build/"];
    // (function, where in the student's code), inlined functions first
    let entries = fs.iter().flat_map(|f| {
        let name = f.func.as_deref().map(source_name).unwrap_or_default();
        let syms: Vec<_> = loader.symbols(f).into_iter().map(|s| {
            // the libraries' own sources are wasi-sdk's (wasisdk://...)
            let file = s.file.unwrap_or_default();
            let user = !file.is_empty() && !theirs.iter().any(|p| file.starts_with(p)) && !file.contains("://");
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
    // from the outside in: where a recursion starts does not depend on
    // where the stack ran out, which changes with the build
    fs.reverse();
    let mut groups = Vec::new();
    let mut i = 0;
    while i < fs.len() {
        // a recursion through more than one function: its cycle once
        let cycle = (2..=4).filter_map(|p| {
            let k = (1..).take_while(|&k| fs.get(i + (k + 1) * p - 1).is_some()
                && fs[i + k * p..i + (k + 1) * p] == fs[i..i + p]).count() + 1;
            (k >= 3).then_some((p, k))
        }).next();
        let (p, k) = cycle.unwrap_or((1, 1));
        let mut g: Vec<String> = fs[i..i + p].iter().rev()
            .map(|(l, times)| if *times > 1 { format!("{l}, {times} times") } else { l.clone() })
            .collect();
        if k > 1 {
            g.push(format!("(the {p} frames above, {k} times)"));
        }
        groups.push(g);
        i += p * k;
        // the innermost frames, the cycle cut short: nothing more to say
        let rest = fs.len() - i;
        if k > 1 && rest < p && fs[i..] == fs[i - p..i - p + rest] {
            i = fs.len();
        }
    }
    let mut out: Vec<String> = groups.into_iter().rev().flatten().collect();
    // what is left of a long recursion: its start and its end
    const HEAD: usize = 10;
    const TAIL: usize = 3;
    if out.len() > HEAD + TAIL + 1 {
        let more = out.len() - HEAD - TAIL;
        out.splice(HEAD..out.len() - TAIL, [format!("... {more} more")]);
    }
    out
}
