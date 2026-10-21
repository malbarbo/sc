// The bindings of engine/sc.h, and with the engine feature the arguments
// that link the engine into sc and the tests (scripts/link.py --emit-link):
// the file SC_LINK_ARGS names, else build/link-session-cut/engine.args, the
// cut engine. Without SC_LINK_ARGS and outside make, make sc-deps builds it
// and what sc reads at run time, so that cargo build alone makes sc.
use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    // cargo watches build.rs anyway: this only keeps it from running again
    // on a change to any file of the package, as it does when nothing is said
    println!("cargo:rerun-if-changed=build.rs");
    if env::var_os("CARGO_FEATURE_ENGINE").is_none() {
        return;
    }
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let header = root.join("engine/sc.h");
    println!("cargo:rerun-if-changed={}", header.display());
    #[cfg(feature = "engine")]
    bindgen::Builder::default()
        .header(header.to_str().unwrap())
        .allowlist_item("sc_.*|SC_.*")
        .default_enum_style(bindgen::EnumVariation::ModuleConsts)
        .generate()
        .expect("bindgen engine/sc.h")
        .write_to_file(PathBuf::from(env::var("OUT_DIR").unwrap()).join("sc.rs"))
        .unwrap();

    if env::var_os("CARGO_FEATURE_WASM").is_some() {
        return;
    }
    println!("cargo:rerun-if-env-changed=SC_LINK_ARGS");
    let file = match env::var_os("SC_LINK_ARGS") {
        Some(file) => PathBuf::from(file),
        None => {
            watch(&root);
            // under make, the rule that runs cargo has built it
            if env::var_os("MAKELEVEL").is_none() {
                make(&root);
            }
            root.join("build/link-session-cut/engine.args")
        }
    };
    println!("cargo:rerun-if-changed={}", file.display());
    for arg in fs::read_to_string(&file).unwrap().lines().chain(["-lstdc++"]) {
        println!("cargo:rustc-link-arg-bin=sc={arg}");
        println!("cargo:rustc-link-arg-tests={arg}");
    }
}

/// What make sc-deps builds from.
fn watch(root: &PathBuf) {
    for dir in ["engine", "cuts", "llvm", "rt", "Makefile"] {
        println!("cargo:rerun-if-changed={}", root.join(dir).display());
    }
    // the scripts, without the __pycache__ that running them writes
    for e in fs::read_dir(root.join("scripts")).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "py") {
            println!("cargo:rerun-if-changed={}", p.display());
        }
    }
}

/// make sc-deps, with cargo's jobs. The first time it builds LLVM; cargo
/// shows its output with -vv.
fn make(root: &PathBuf) {
    let mut cmd = Command::new("make");
    cmd.arg("-C").arg(root).arg("sc-deps");
    if let Some(flags) = env::var_os("CARGO_MAKEFLAGS") {
        cmd.env("MAKEFLAGS", flags);
    }
    let st = cmd.status().expect("make: the engine is built by the Makefile");
    assert!(st.success(), "make sc-deps failed: cargo build -vv shows its output");
}
