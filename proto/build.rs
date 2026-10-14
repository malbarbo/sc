// The bindings of engine/sc.h, and with the engine feature the arguments
// that link the engine (SC_LINK_ARGS: scripts/link.py --emit-link).
use std::{env, fs, path::PathBuf};

fn main() {
    let header = "../engine/sc.h";
    println!("cargo:rerun-if-changed={header}");
    bindgen::Builder::default()
        .header(header)
        .allowlist_item("sc_.*|SC_.*")
        .default_enum_style(bindgen::EnumVariation::ModuleConsts)
        .generate()
        .expect("bindgen engine/sc.h")
        .write_to_file(PathBuf::from(env::var("OUT_DIR").unwrap()).join("sc.rs"))
        .unwrap();

    if env::var_os("CARGO_FEATURE_ENGINE").is_none() {
        return;
    }
    println!("cargo:rerun-if-env-changed=SC_LINK_ARGS");
    let file = env::var("SC_LINK_ARGS").expect("SC_LINK_ARGS: the file of scripts/link.py --emit-link");
    println!("cargo:rerun-if-changed={file}");
    // rustc-link-arg reaches only this crate's own programs, tests and
    // examples; a program in another crate passes DEP_SC_LINK_ARGS on in its
    // build.rs
    println!("cargo:link_args={file}");
    for arg in fs::read_to_string(&file).unwrap().lines().chain(["-lstdc++"]) {
        println!("cargo:rustc-link-arg={arg}");
    }
}
