//! The safe API over the uncut engine: sessions in C and C++, a cell with an
//! error, the replay, undo, the one-shot engine and the thread rule. The
//! shared objects run in the loader (build/loader/release/scload).
//!
//! One test: the engine stays on the first thread that uses it.
use sc_proto::{main, Error, Mode, Session, Status};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::{fs, io::Write};

fn build() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../build").canonicalize().unwrap()
}

/// The session's flags: cc1 for -fPIC (test/corpus.py --flags), then the link.
fn args(cxx: bool) -> Vec<String> {
    let b = build();
    let sr = b.join("sysroot").display().to_string();
    let res = b.join("wasi-sdk-29.0-x86_64-linux/lib/clang/21").display().to_string();
    let mut cmd = Command::new(b.join("../test/corpus.py"));
    cmd.arg("--flags");
    if cxx {
        cmd.arg("c++");
    }
    let out = cmd.arg("pic").output().unwrap();
    assert!(out.status.success());
    let mut a: Vec<String> = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .map(|x| x.replace("/sysroot", &sr).replace("/res/", &format!("{res}/")))
        .collect();
    a.extend(["-x", if cxx { "c++" } else { "c" }, "--", &format!("-L{sr}/lib/wasm32-wasip1")].map(String::from));
    if cxx {
        a.extend(["-lc++", "-lc++abi"].map(String::from));
    }
    a.extend(["-lsetjmp", "-lc"].map(String::from));
    a.push(format!("{res}/lib/wasm32-unknown-wasip1/libclang_rt.builtins.a"));
    a
}

/// Runs the shared objects in the loader; its output.
fn run(name: &str, sos: &[Vec<u8>]) -> String {
    let b = build();
    let dir = std::env::temp_dir().join(format!("sc-proto-{}-{name}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let mut cmd = Command::new(b.join("loader/release/scload"));
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

/// Compiles the cells; per cell, the status and the exported names, and the
/// shared objects of the accepted ones.
fn cells(s: &mut Session, srcs: &[&str], mode: Mode) -> (Vec<(Status, Vec<String>)>, Vec<Vec<u8>>) {
    let (mut res, mut sos) = (vec![], vec![]);
    for src in srcs {
        let r = s.cell(src, mode).unwrap();
        if r.status() == Status::Ok && mode == Mode::So {
            assert!(r.so().starts_with(b"\0asm"), "{r:?}");
            sos.push(r.so().to_vec());
        }
        if r.status() != Status::Ok {
            assert!(r.so().is_empty() && r.syms().next().is_none());
        }
        res.push((r.status(), r.syms().map(|x| x.into_owned()).collect()));
    }
    (res, sos)
}

const C: &[&str] = &[
    "#include <stdio.h>",
    "static int cnt;",
    "int next(void) { return ++cnt; }",
    "int y = ;",
    "int main(void) { next(); printf(\"%d\\n\", next()); return 0; }",
];

const CXX: &[&str] = &[
    "#include <iostream>\n#include <string>",
    "std::string greet(std::string n) { return \"oi \" + n; }",
    "struct S { int v; S(int v) : v(v) {} };",
    "int main() { try { throw S(7); } catch (S &e) { std::cout << greet(std::to_string(e.v)) << '\\n'; } }",
];

#[test]
fn session() {
    // C: a static shared by the cells, a cell with an error
    let mut s = Session::new(args(false)).unwrap();
    let (res, sos) = cells(&mut s, C, Mode::So);
    let st: Vec<_> = res.iter().map(|r| r.0).collect();
    assert_eq!(st, [Status::Ok, Status::Ok, Status::Ok, Status::Error, Status::Ok]);
    assert!(res[2].1.contains(&"next".to_string()), "{res:?}");
    assert_eq!(run("c", &sos), "2\n");

    // the error cell's diagnostics; the session goes on
    let r = s.cell("int z = ;", Mode::So).unwrap();
    assert_eq!(r.status(), Status::Error);
    assert!(r.diags().contains("error: expected expression"), "{}", r.diags());
    assert_eq!(s.cell("int z = 1;", Mode::So).unwrap().status(), Status::Ok);

    // the replay: the same names, without the backend
    let mut s2 = Session::new(args(false)).unwrap();
    let (replay, _) = cells(&mut s2, C, Mode::Replay);
    assert_eq!(replay, res);

    // undo: the session is lost; a fresh one has nothing to undo
    assert_eq!(s.undo().status(), Status::Lost);
    let r = s.cell("int w;", Mode::So).unwrap();
    assert_eq!(r.status(), Status::Lost);
    assert!(r.diags().contains("lost"));
    drop(s);
    drop(s2);
    let mut s3 = Session::new(args(false)).unwrap();
    assert_eq!(s3.undo().status(), Status::Error);
    drop(s3);

    // C++ with exceptions and libc++
    let mut s = Session::new(args(true)).unwrap();
    let (res, sos) = cells(&mut s, CXX, Mode::So);
    assert!(res.iter().all(|r| r.0 == Status::Ok), "{res:?}");
    assert_eq!(run("cxx", &sos), "oi 7\n");

    // the one-shot engine: a usage error
    assert_eq!(main(["sc", "nope"]).unwrap(), 2);

    // arguments are checked before they cross
    assert!(matches!(Session::new(["-x", "c\0"]), Err(Error::Nul)));

    // the engine stays on this thread
    let other = std::thread::spawn(|| matches!(Session::new(args(false)), Err(Error::OtherThread)));
    assert!(other.join().unwrap());
}
