//! The REPL's side over sc.wasm's session on wasmtime (Guest, the sc.wasm
//! that sc embeds): a cell with an error, a crash, the replay, undo, the
//! limits, Enter, the shown values and `:type`. The shared objects run in
//! the loader (build/cargo/release/scload).
use sc::engine::{Guest, Repl, Status};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::{fs, io::Write};

fn build() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("build").canonicalize().unwrap()
}

/// The session's arguments: cc1 for -fPIC, then the link (scripts/flags.py).
fn args(cxx: bool) -> Vec<String> {
    let b = build();
    let out = Command::new(b.join("../scripts/flags.py"))
        .args(["--session", if cxx { "c++" } else { "c" }])
        .arg(&b)
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().lines().map(String::from).collect()
}

/// Runs the shared objects in the loader; its output.
fn run(name: &str, sos: &[Vec<u8>]) -> String {
    let b = build();
    let dir = std::env::temp_dir().join(format!("sc-proto-{}-{name}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let mut cmd = Command::new(b.join("cargo/release/scload"));
    cmd.arg("-L").arg(b.join("sysroot/lib/wasm32-wasip1"));
    for (i, so) in sos.iter().enumerate() {
        let p = dir.join(format!("{}.so", i + 1));
        fs::write(&p, so).unwrap();
        cmd.arg(p);
    }
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"").unwrap();
    let out = child.wait_with_output().unwrap();
    fs::remove_dir_all(&dir).unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
}

const C: &[&str] = &[
    "#include <stdio.h>",
    "static int cnt;",
    "int next(void) { return ++cnt; }",
    "int y = ;",
    "int main(void) { next(); printf(\"%d\\n\", next()); return 0; }",
];

/// A session in C or C++, without limits.
fn repl(cxx: bool) -> Repl<Guest> {
    Repl::start(Guest::new(None, None).unwrap(), args(cxx)).unwrap()
}

#[test]
fn cells() {
    let mut r = repl(false);
    let mut sos = vec![];
    fn cell(r: &mut Repl<Guest>, sos: &mut Vec<Vec<u8>>, src: &str) -> sc::engine::Answer {
        let a = r.cell(src.as_bytes()).unwrap();
        if a.status == Status::Ok {
            sos.push(a.so.clone());
        }
        a
    }
    for src in &C[..3] {
        assert_eq!(cell(&mut r, &mut sos, src).status, Status::Ok);
    }
    assert_eq!(cell(&mut r, &mut sos, "int y = ;").status, Status::Error);
    // a failed cell leaves nothing behind, not even the guard of a header
    let a = cell(&mut r, &mut sos, "#include <stdio.h>\nint z = ;");
    assert_eq!(a.status, Status::Error);
    assert_eq!(cell(&mut r, &mut sos, "#include <stdio.h>\nint p(void) { return EOF; }").status, Status::Ok);
    sos.pop();
    assert_eq!(r.undo().unwrap().status, Status::Ok);
    // a crash traps, and the copy from before the cell puts the session back
    let a = cell(&mut r, &mut sos, "#pragma clang __debug crash");
    assert_eq!(a.status, Status::Error);
    assert!(a.diags.contains("the engine crashed"), "{a:?}");
    assert_eq!(r.len(), 3);
    // undo replays the log without the last accepted cell
    assert_eq!(cell(&mut r, &mut sos, "int bad(void) { return 1; }").status, Status::Ok);
    assert_eq!(r.undo().unwrap().status, Status::Ok);
    sos.pop();
    assert_eq!(r.len(), 3);
    // the static of the first instance is the one of the cells after the replays
    assert_eq!(cell(&mut r, &mut sos, C[4]).status, Status::Ok);
    assert_eq!(run("repl", &sos), "2\n");
    drop(r);

    // nothing to undo
    let mut r = repl(false);
    assert_eq!(r.undo().unwrap().status, Status::Error);
}

/// Above a limit (a timeout of 0, Ctrl-C, the memory) the call stops, and
/// the copy puts the session back: the cell is an error. A replay that
/// fails so (an undo) is told in the answer, and the session stays.
#[test]
fn limits() {
    let mut r = repl(true);
    assert_eq!(r.cell(b"int x1 = 1;").unwrap().status, Status::Ok);
    r.limit(Some(std::time::Duration::ZERO), None);
    let a = r.cell(b"#include <regex>").unwrap();
    assert_eq!(a.status, Status::Error);
    assert!(a.diags.contains("the compiler took more than 0 s"), "{a:?}");
    assert_eq!(r.len(), 1);
    r.limit(None, None);
    assert_eq!(r.cell(b"#include <regex>").unwrap().status, Status::Ok);
    assert_eq!(r.cell(b"int x2 = x1;").unwrap().status, Status::Ok);
    // the replay of <regex> takes more than the watchdog's tick
    r.limit(Some(std::time::Duration::ZERO), None);
    let a = r.undo().unwrap();
    assert_eq!(a.status, Status::Error);
    assert!(a.diags.contains("the cell was not undone"), "{a:?}");
    assert_eq!(r.len(), 3);
    r.limit(None, None);
    assert_eq!(r.cell(b"int x3 = x2;").unwrap().status, Status::Ok);
    assert_eq!(r.len(), 4);

    let mut r = repl(true);
    r.limit(None, Some(1));
    let a = r.cell(b"#include <regex>").unwrap();
    assert_eq!(a.status, Status::Error);
    assert!(a.diags.contains("the compiler took more than 0 MB"), "{a:?}");
}

/// Enter in the REPL: a cell whose first error is at its end stops short and
/// leaves nothing; the depth comes from the lexer.
#[test]
fn whole() {
    let mut r = repl(false);
    let mut st = |src: &str| r.cell_if_whole(src.as_bytes()).unwrap().status;
    for src in [
        "int main(void)",
        "int x = 5",
        "for (int i = 0; i < 3; i++)",
        "if (1) ; else",
        "struct ponto",
        "int y = 1 +",
        "int main(void) // the body comes next",
    ] {
        assert_eq!(st(src), Status::Incomplete, "{src:?}");
    }
    for (src, want) in [
        ("#include <stdio.h>", Status::Ok),
        ("int x = 5;", Status::Ok),
        ("x", Status::Ok),
        ("x * 2", Status::Ok),
        ("printf(\"%d\\n\", x)", Status::Ok),
        ("y", Status::Error),
        ("int z = );", Status::Error),
        ("int w = 1 + nada", Status::Error),
    ] {
        assert_eq!(st(src), want, "{src:?}");
    }
    // a type the session knows: a declaration, not a product
    assert_eq!(st("typedef int T;"), Status::Ok);
    assert_eq!(st("T *f(T n)"), Status::Incomplete);
    assert_eq!(st("x * f(2)"), Status::Error);
    let a = r.cell_if_whole(b"int main(void)").unwrap();
    assert!(a.diags.is_empty(), "{a:?}");
    assert_eq!(r.len(), 6);

    for (src, d) in [
        ("int f(void) {", 1),
        ("puts(\"{\"); // {", 0),
        ("f((", 2),
        ("/* a comment", 1),
        ("/* a comment */ x", 0),
        ("#define F(x) \\", 1),
        ("char c = '{';", 0),
    ] {
        assert_eq!(r.depth(src.as_bytes()).unwrap(), d, "{src:?}");
    }
}

#[test]
fn show() {
    let mut r = repl(false);
    let mut cell = |src: &str| r.cell(src.as_bytes()).unwrap();
    let has = |so: &[u8], s: &str| so.windows(s.len()).any(|w| w == s.as_bytes());
    assert_eq!(cell("int x = 3; struct p { int a; } q;").status, Status::Ok);
    // an expression without its `;` calls the host with its type and value
    for (src, call, ty) in [("x", "__sc_show_int", "int\0"), ("x / 2.0", "__sc_show_float", "double\0"),
                            ("sizeof x", "__sc_show_int", "size_t\0"), ("-x", "__sc_show_int", "int\0")] {
        let a = cell(src);
        assert_eq!(a.status, Status::Ok, "{src:?}: {a:?}");
        assert!(has(&a.so, call) && has(&a.so, ty), "{src:?}");
        assert!(a.diags.is_empty(), "{src:?}: {}", a.diags);
    }
    // its value is not discarded; with the `;`, it is
    assert!(cell("x > 2").diags.is_empty());
    assert!(cell("x > 2;").diags.contains("-Wunused-comparison"));
    assert!(cell("x + 1;").diags.contains("-Wunused-value"));
    // a value that is not a scalar is not shown
    let a = cell("q");
    assert_eq!(a.status, Status::Ok);
    assert!(!has(&a.so, "__sc_show"));
    // a placeholder is an error, as with the `;`
    let a = cell("__builtin_abs");
    assert_eq!(a.status, Status::Error);
    assert!(a.diags.contains("must be directly called"), "{}", a.diags);

    // C++: the temporaries die after the call
    let mut r = repl(true);
    let mut cell = |src: &str| r.cell(src.as_bytes()).unwrap();
    assert_eq!(cell("struct T { ~T() {} int n() { return 1; } }; T t;").status, Status::Ok);
    let a = cell("T().n()");
    assert_eq!(a.status, Status::Ok, "{a:?}");
    assert!(has(&a.so, "__sc_show_int") && has(&a.so, "_ZN1TD"));
    let a = cell("t.n");
    assert_eq!(a.status, Status::Error);
    assert!(a.diags.contains("must be called"), "{}", a.diags);
}

/// `:type`: the type of an expression, as a shown value writes it; nothing
/// of the expression stays in the session.
#[test]
fn type_of() {
    let mut r = repl(false);
    assert_eq!(r.cell(b"int x = 3; struct p { int a; } q;").unwrap().status, Status::Ok);
    let mut ty = |src: &str| {
        let a = r.type_of(src.as_bytes()).unwrap();
        (a.status, a.syms.join(""), a.diags)
    };
    for (src, want) in [("x", "int"), ("q", "struct p"), ("&q.a", "int *"), ("sizeof x", "size_t"),
                        ("\"ab\"", "char[3]"), ("x / 2.0", "double"), ("(struct r { int b; }){1}", "struct r")] {
        let (st, t, d) = ty(src);
        assert_eq!((st, t.as_str()), (Status::Ok, want), "{src:?}: {d}");
        assert!(d.is_empty(), "{src:?}: {d}");
    }
    for src in ["int", "x;", "1; 2", "y", ""] {
        let (st, t, d) = ty(src);
        assert_eq!((st, t.as_str()), (Status::Error, ""), "{src:?}");
        assert!(d.contains("error:"), "{src:?}: {d}");
    }
    // the struct of the expression is gone, and the log has the one cell
    assert_eq!(r.cell(b"struct r { char c; };").unwrap().status, Status::Ok);
    assert_eq!(r.len(), 2);

    // C++: the names of a class or a template's inside are left out; the
    // copy constructor Sema defines goes with the expression
    let mut r = repl(true);
    let src = "#include <string>\n#include <vector>\nstd::string s; std::vector<int> v; struct T { std::string a; } t;";
    assert_eq!(r.cell(src.as_bytes()).unwrap().status, Status::Ok);
    for (src, want) in [("s", "std::string"), ("s + \"c\"", "std::string"), ("s.size()", "size_t"),
                        ("v[0]", "int"), ("v.begin()", "std::__wrap_iter<int *>"), ("T(t)", "T")] {
        let a = r.type_of(src.as_bytes()).unwrap();
        assert_eq!((a.status, a.syms.join("")), (Status::Ok, want.to_string()), "{src:?}: {}", a.diags);
    }
    assert_eq!(r.cell(b"T u = t;").unwrap().status, Status::Ok);
}
