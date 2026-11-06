//! What of the Rust crate sc.wasm links: the loader (`sc::loader`) and the
//! machine (`sc::machine`) on the host's primitives, with the session in
//! this module, and their exports.
//!
//! The exports are unsafe for the host that calls them: ENGINE.md, in
//! simplecode, says what each one asks of its pointers.
#![allow(clippy::missing_safety_doc)]

use sc::engine::{sys, Answer, Compiler, Mode, Session, Status};
use sc::loader::host::{self, Host};
use sc::loader::{libdirs, Error, Loader, STACK};
use sc::machine::shell::{self, Step, COMMANDS};
use sc::machine::{Enter, Lang, Machine, Outcome, Platform, Safety, INDENT, ROOT};
use std::ffi::{c_char, c_int, c_ulonglong, CString};
use std::io::{self, BufRead};
use std::path::PathBuf;
use std::sync::OnceLock;

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

// --- the machine

extern "C" {
    // engine/sc_pack.cpp
    fn sc_pack(root: *const c_char);
    fn sc_pack_read(path: *const c_char, size: *mut c_ulonglong) -> *const c_char;
    // engine/sc_api.cpp
    fn sc_session_snapshot(on: c_int);
    fn sc_session_restore(s: *mut sys::sc_session) -> c_int;
}

const CRASHED: &str = "sc: the engine crashed on this cell; the session is as it was before it\n";

/// The session in this module (engine/sc_api.cpp), with the copy of the
/// memory before each cell (NOTES.md §65). A cell is called through the
/// host, `wasm_call` of this module's own `sc_session_cell` (the handle of
/// `wasm_export(0, ...)`): a trap or an exit of the compiler comes back
/// as an error to the machine, whose frames are above the cell's, and the
/// copy puts the memory back.
#[derive(Default)]
struct Local {
    session: Option<Session>,
}

/// `sc_session_cell` as the host calls it.
fn cell_export() -> u32 {
    static H: OnceLock<u32> = OnceLock::new();
    *H.get_or_init(|| host::export(0, "sc_session_cell"))
}

impl Compiler for Local {
    fn another(&self) -> io::Result<Local> {
        Ok(Local::default())
    }

    fn new_session(&mut self, args: &[String]) -> io::Result<Answer> {
        self.session = None;
        // SAFETY: only sets a flag
        unsafe { sc_session_snapshot(1) };
        Ok(match Session::new(args) {
            Ok(s) => {
                self.session = Some(s);
                Answer::bare(Status::Ok, String::new())
            }
            Err(sc::engine::Error::Start(d)) => Answer::bare(Status::Error, d),
            Err(e) => Answer::bare(Status::Error, format!("sc: {e}\n")),
        })
    }

    fn cell(&mut self, src: &[u8], mode: Mode) -> io::Result<Answer> {
        let Some(s) = self.session.as_mut() else {
            return Ok(Answer::bare(Status::Error, "sc: no session\n".into()));
        };
        let len = u32::try_from(src.len()).map_err(|_| io::Error::other("the cell is too long"))?;
        let raw = s.as_raw();
        let mut vals = [raw as u64, src.as_ptr() as u64, len as u64, mode.to_raw() as u64];
        match host::call(cell_export(), "iiii:i", &mut vals) {
            host::OK => {
                let r = s.reply(vals[0] as u32 as c_int);
                Ok(Answer { status: r.status(), diags: r.diags().into_owned(), so: r.so().to_vec(), syms: r.syms().map(String::from).collect() })
            }
            // SAFETY: the session is the one the cell stopped in
            // the memory back first: what the host's error allocates would
            // be lost with what the cell allocated
            st => match unsafe { sc_session_restore(raw) } as u32 {
                sys::sc_status::SC_ERROR => Ok(Answer::bare(Status::Error, CRASHED.into())),
                _ => {
                    // what the cell left of it cannot be freed
                    std::mem::forget(self.session.take());
                    let e = host::error(st, vals[0]);
                    Err(io::Error::new(io::ErrorKind::BrokenPipe, format!("the engine ended ({e})")))
                }
            },
        }
    }

    fn depth(&mut self, src: &[u8]) -> io::Result<u32> {
        Ok(self.session.as_mut().and_then(|s| s.depth(src).ok()).unwrap_or(0))
    }
}

/// The machine in sc.wasm: the session in this module, the code on the
/// host's primitives, the pack in the module (engine/sc_pack.cpp).
#[derive(Clone)]
struct Inside {
    color: bool,
}

impl Platform for Inside {
    type Compiler = Local;
    type Runtime = Host;

    fn compiler(&self) -> io::Result<Local> {
        Ok(Local::default())
    }

    // the code's arguments and stdio are those the host gives a module
    fn runtime(&self, _argv: &[String], _interactive: bool) -> sc::loader::Result<Host> {
        Ok(Host::default())
    }

    fn read(&self, path: &str) -> Option<&'static [u8]> {
        static PACK: OnceLock<()> = OnceLock::new();
        let root = CString::new(ROOT).unwrap();
        // SAFETY: sc_pack copies the root
        PACK.get_or_init(|| unsafe { sc_pack(root.as_ptr()) });
        let path = CString::new(format!("{ROOT}/{path}")).ok()?;
        let mut size = 0;
        // SAFETY: the pack keeps what it read until the end
        let p = unsafe { sc_pack_read(path.as_ptr(), &mut size) };
        (!p.is_null()).then(|| unsafe { std::slice::from_raw_parts(p.cast(), size as usize) })
    }

    fn color(&self) -> bool {
        self.color
    }
}

/// `sc` inside sc.wasm, with the arguments of its WASI: `[-q] [-x LANG]
/// [--safety LEVEL] [FILE [ARG]...]` (`-x=LANG`, `--safety=LEVEL` too).
/// With a file, runs the program; else the REPL, on stdin, without a
/// terminal. Returns the exit code.
#[no_mangle]
pub extern "C" fn sc_cli() -> i32 {
    let mut args = std::env::args().skip(1);
    let (mut lang, mut safety, mut file) = (None, Safety::On, None);
    while let Some(a) = args.next() {
        let (name, value) = match a.split_once('=') {
            Some((n, v)) => (n.to_string(), Some(v.to_string())),
            None => (a.clone(), None),
        };
        match name.as_str() {
            "-q" => {}
            "-x" => lang = value.or_else(|| args.next()).map(|l| if l == "c++" { Lang::Cxx } else { Lang::C }),
            "--safety" => safety = value.or_else(|| args.next()).and_then(|l| Safety::parse(&l)).unwrap_or(Safety::On),
            _ => {
                file = Some(PathBuf::from(a));
                break;
            }
        }
    }
    let platform = Inside { color: false };
    let r = match file {
        Some(f) => sc::machine::run(platform, lang, safety, &f, args.collect()),
        None => repl(platform, lang.unwrap_or(Lang::C), safety),
    };
    r.unwrap_or_else(|e| {
        eprintln!("sc: {e}");
        1
    })
}

/// The REPL on stdin, without a terminal, as the native `sc`'s line editor
/// reads it: an input goes on, a line at a time, while the compiler says
/// it stops short ([`Machine::enter`]); one that the end cuts off still
/// runs, for the student to read what is wrong with it.
fn repl(platform: Inside, lang: Lang, safety: Safety) -> sc::machine::Result<i32> {
    let mut m = Machine::new(platform, lang, safety, vec!["sc".into()], true)?;
    let mut status = 0;
    let (mut input, mut pending) = (String::new(), String::new());
    let mut stdin = io::stdin().lock();
    loop {
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            if pending.trim().is_empty() {
                break;
            }
            input = std::mem::take(&mut pending);
        } else {
            let line = line.strip_suffix('\n').map_or(line.as_str(), |l| l.strip_suffix('\n').unwrap_or(l));
            input += line;
            if m.enter(&input, input.len()) != Enter::Run {
                pending = input.clone();
                input.push('\n');
                continue;
            }
            pending.clear();
        }
        match shell::line(&mut m, &std::mem::take(&mut input), COMMANDS, |_, _| Step::Same)? {
            Step::Status(s) => status = s,
            Step::Same | Step::Names => {}
            Step::Quit => break,
            Step::Exit(code) => return Ok(code),
        }
    }
    Ok(status)
}

// --- simplecode's API (its ENGINE.md): the REPL of a page

/// A REPL of the page.
pub struct Repl {
    m: Machine<Inside>,
}

/// The text the host wrote at `p`.
unsafe fn text<'a>(p: *const u8, len: usize) -> std::borrow::Cow<'a, str> {
    if len == 0 {
        return "".into();
    }
    String::from_utf8_lossy(std::slice::from_raw_parts(p, len))
}

/// A REPL with `code` loaded, as `sc FILE` loads it: its declarations are
/// there for the cells, its `main` does not run. `config`: `lang=c|c++`,
/// `safety=off|on|strict`, `color=0|1` (1: the page renders the ANSI
/// codes). Null if the code has an error, which is written.
#[no_mangle]
pub unsafe extern "C" fn repl_new(code: *const u8, code_len: usize, config: *const u8, config_len: usize) -> *mut Repl {
    let (code, config) = (text(code, code_len), text(config, config_len));
    let (mut lang, mut safety, mut color) = (Lang::C, Safety::On, true);
    for kv in config.split_whitespace() {
        match kv.split_once('=') {
            Some(("lang", "c")) => lang = Lang::C,
            Some(("lang", "c++")) => lang = Lang::Cxx,
            Some(("safety", l)) => safety = Safety::parse(l).unwrap_or(safety),
            Some(("color", c)) => color = c == "1",
            _ => {}
        }
    }
    let new = || Machine::new(Inside { color }, lang, safety, vec!["sc".into()], true);
    let mut m = match new() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("sc: {e}");
            return std::ptr::null_mut();
        }
    };
    if !code.trim().is_empty() {
        let file = if lang == Lang::C { "user.c" } else { "user.cpp" };
        match m.cell(&format!("#line 1 \"{file}\"\n{code}")) {
            Ok(Outcome::Ok) => {}
            Ok(Outcome::Error) => return std::ptr::null_mut(),
            Err(e) => {
                eprintln!("sc: {e}");
                return std::ptr::null_mut();
            }
            // the protocol has no session that is over: an empty one
            Ok(Outcome::Exit(_)) => match new() {
                Ok(n) => m = n,
                Err(_) => return std::ptr::null_mut(),
            },
        }
    }
    Box::into_raw(Box::new(Repl { m }))
}

/// A cell or a command: 0 done, 1 an error (written), 2 the session is
/// over (`exit`, `:quit`).
#[no_mangle]
pub unsafe extern "C" fn repl_run(r: *mut Repl, p: *const u8, len: usize) -> u32 {
    let r = &mut *r;
    match shell::line(&mut r.m, &text(p, len), COMMANDS, |_, _| Step::Same) {
        Ok(Step::Status(0) | Step::Same | Step::Names) => 0,
        Ok(Step::Status(_)) => 1,
        Ok(Step::Quit | Step::Exit(_)) => 2,
        Err(e) => {
            eprintln!("sc: {e}");
            1
        }
    }
}

/// What Enter does with the input, the caret at `cursor`: -1 runs it, else
/// a line opens with this many spaces ([`Machine::enter`]).
#[no_mangle]
pub unsafe extern "C" fn repl_ready(r: *mut Repl, p: *const u8, len: usize, cursor: usize) -> i32 {
    let input = text(p, len);
    let mut at = cursor.min(input.len());
    while !input.is_char_boundary(at) {
        at -= 1;
    }
    match (*r).m.enter(&input, at) {
        Enter::Run => -1,
        Enter::Open(depth) => (depth * INDENT) as i32,
    }
}

/// Tab at byte `cursor`: `c <start>` and a candidate a line, as in
/// [`shell::complete`].
#[no_mangle]
pub unsafe extern "C" fn repl_complete(r: *mut Repl, p: *const u8, len: usize, cursor: usize) -> *mut c_char {
    let input = text(p, len);
    let mut at = cursor.min(input.len());
    while !input.is_char_boundary(at) {
        at -= 1;
    }
    let (start, words) = shell::complete(&shell::candidates((*r).m.names(), COMMANDS), &input, at);
    let mut out = format!("c {start}");
    for w in words {
        out.push('\n');
        out.push_str(&w);
    }
    cstr(out)
}

/// sc formats nothing: 0, and the page keeps the code as it is.
#[no_mangle]
pub extern "C" fn format(_p: *const u8, _len: usize) -> *mut c_char {
    std::ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn version() -> *mut c_char {
    cstr(format!("sc {}", env!("CARGO_PKG_VERSION")))
}

fn cstr(s: String) -> *mut c_char {
    CString::new(s).map_or(std::ptr::null_mut(), CString::into_raw)
}

#[no_mangle]
pub unsafe extern "C" fn cstr_deallocate(p: *mut c_char) {
    if !p.is_null() {
        drop(CString::from_raw(p));
    }
}

#[no_mangle]
pub unsafe extern "C" fn repl_destroy(r: *mut Repl) {
    drop(Box::from_raw(r));
}

#[no_mangle]
pub extern "C" fn string_allocate(size: usize) -> *mut u8 {
    let mut b = Vec::<u8>::with_capacity(size);
    let p = b.as_mut_ptr();
    std::mem::forget(b);
    p
}

#[no_mangle]
pub unsafe extern "C" fn string_deallocate(p: *mut u8, size: usize) {
    drop(Vec::from_raw_parts(p, 0, size));
}
