// The bindings of engine/sc.h (the machine feature), and with the engine
// feature the arguments that link the engine into sc and the tests
// (scripts/link.py --emit-link): the file SC_LINK_ARGS names, else
// build/link-session-cut/engine.args, the cut engine. Without SC_LINK_ARGS
// and outside make, make sc-deps builds it and what sc reads at run time, so
// that cargo build alone makes sc. And the pack sc embeds (PLANO-REPL.md
// §5.4), $OUT_DIR/sc.sqfs, also with the wasm feature.
use std::{env, fs, path::PathBuf, process::Command};
#[cfg(feature = "engine")]
use std::{collections::BTreeMap, io::Cursor, path::Path};

// the loader's wasmtime Config, which the libraries' .cwasm must have
#[cfg(feature = "engine")]
#[path = "src/loader/config.rs"]
mod config;

fn main() {
    // cargo watches build.rs anyway: this only keeps it from running again
    // on a change to any file of the package, as it does when nothing is said
    println!("cargo:rerun-if-changed=build.rs");
    if env::var_os("CARGO_FEATURE_MACHINE").is_none() {
        return;
    }
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let header = root.join("engine/sc.h");
    println!("cargo:rerun-if-changed={}", header.display());
    // for the target: sc.wasm's Rust is wasm32-wasip1 (wasm/), where clang
    // hides what it declares, and bindgen leaves out hidden functions
    #[cfg(feature = "machine")]
    bindgen::Builder::default()
        .header(header.to_str().unwrap())
        .clang_args([format!("--target={}", env::var("TARGET").unwrap()), "-fvisibility=default".into()])
        .allowlist_item("sc_.*|SC_.*")
        .default_enum_style(bindgen::EnumVariation::ModuleConsts)
        .generate()
        .expect("bindgen engine/sc.h")
        .write_to_file(PathBuf::from(env::var("OUT_DIR").unwrap()).join("sc.rs"))
        .unwrap();
    if env::var_os("CARGO_FEATURE_ENGINE").is_none() {
        return;
    }
    #[cfg(feature = "engine")]
    pack(&root, &PathBuf::from(env::var("OUT_DIR").unwrap()).join("sc.sqfs"));

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

/// The libraries a session links and loads (scripts/flags.py, solibs), with
/// those they need.
#[cfg(feature = "engine")]
const LIBS: [&str; 5] = ["libc.so", "libc++.so", "libc++abi.so", "libsc_rt.so", "libsetjmp.so"];

/// The pack: a squashfs (zstd -19, blocks of 128 KiB) with what a session
/// reads, at the paths of the pack's view (scripts/flags.py, PACK): the
/// sysroot's headers, libraries and ignorelist in sysroot/, clang's headers
/// and builtins in res/, the arguments in repl/. Each library has its
/// <name>.cwasm next to it, compiled for the loader and cargo's target, with
/// the CPU features that any of it has. Out of clang's headers go the
/// intrinsics of the other targets, and out of libc its DWARF, which frames
/// hides (wasisdk://).
#[cfg(feature = "engine")]
fn pack(root: &Path, out: &Path) {
    let b = root.join("build");
    let res = b.join("wasi-sdk-29.0-x86_64-linux/lib/clang/21");
    let (lib, builtins) = ("lib/wasm32-wasip1", "lib/wasm32-unknown-wasip1/libclang_rt.builtins.a");
    let trees = [
        (b.join("sysroot/include"), "sysroot/include"),
        (res.join("include"), "res/include"),
        (b.join("pack/repl"), "repl"),
    ];
    let mut inputs: Vec<(PathBuf, String)> =
        LIBS.iter().map(|l| (b.join("sysroot").join(lib).join(l), format!("sysroot/{lib}/{l}"))).collect();
    inputs.push((b.join("sysroot/share/sc/ignorelist.txt"), "sysroot/share/sc/ignorelist.txt".into()));
    inputs.push((res.join(builtins), format!("res/{builtins}")));
    for p in trees.iter().map(|t| &t.0).chain(inputs.iter().map(|i| &i.0)) {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    // the image's path of each file, and the file
    let mut files = BTreeMap::new();
    for (dir, at) in &trees {
        tree(dir, at, &mut files);
    }
    files.retain(|k: &String, _| {
        let name = k.strip_prefix("res/include/").unwrap_or("/");
        name.contains('/') || !(name.contains("intrin") || name.starts_with("arm_"))
    });
    files.extend(inputs.into_iter().map(|(from, to)| (to, from)));

    // build.rs also runs again for a change to the engine: then the 35 s of
    // zstd are not needed. This program, which wasmtime and the Config are
    // in, and the target make the .cwasm
    let me = env::current_exe().unwrap();
    let stamp: String = files
        .iter()
        .map(|(to, from)| (to.as_str(), from))
        .chain([("", &me)])
        .map(|(to, from)| {
            let m = fs::metadata(from).unwrap();
            format!("{to} {} {} {:?}\n", from.display(), m.len(), m.modified().unwrap())
        })
        .chain([env::var("TARGET").unwrap()])
        .collect();
    let stamp_file = out.with_extension("stamp");
    if out.exists() && fs::read_to_string(&stamp_file).is_ok_and(|s| s == stamp) {
        return;
    }

    use backhand::compression::{CompressionOptions, Compressor, Zstd};
    use backhand::{FilesystemCompressor, FilesystemWriter, NodeHeader};
    let mut fs = FilesystemWriter::default();
    let zstd = CompressionOptions::Zstd(Zstd { compression_level: 19 });
    fs.set_compressor(FilesystemCompressor::new(Compressor::Zstd, Some(zstd)).unwrap());
    fs.set_block_size(128 << 10);
    fs.set_no_padding();
    fs.set_root_mode(0o755);
    let mut cfg = config::config();
    cfg.target(&env::var("TARGET").unwrap()).unwrap();
    let engine = wasmtime::Engine::new(&cfg).unwrap();
    let file = NodeHeader::new(0o644, 0, 0, 0);
    for (to, from) in &files {
        fs.push_dir_all(Path::new(to).parent().unwrap(), NodeHeader::new(0o755, 0, 0, 0)).unwrap();
        let mut bytes = fs::read(from).unwrap_or_else(|e| panic!("{}: {e}", from.display()));
        if to.ends_with("/libc.so") {
            bytes = strip_debug(&bytes);
        }
        if to.ends_with(".so") {
            let cwasm = engine.precompile_module(&bytes).unwrap_or_else(|e| panic!("{to}: {e}"));
            fs.push_file(Cursor::new(cwasm), format!("{to}.cwasm"), file).unwrap();
        }
        fs.push_file(Cursor::new(bytes), to, file).unwrap();
    }
    fs.write(fs::File::create(out).unwrap()).unwrap();
    fs::write(stamp_file, stamp).unwrap();
}

/// The files under dir, as at/<their path under dir>.
#[cfg(feature = "engine")]
fn tree(dir: &Path, at: &str, files: &mut BTreeMap<String, PathBuf>) {
    for e in fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let e = e.unwrap();
        let (p, at) = (e.path(), format!("{at}/{}", e.file_name().to_str().unwrap()));
        if e.file_type().unwrap().is_dir() {
            tree(&p, &at, files);
        } else {
            files.insert(at, p);
        }
    }
}

/// A wasm module without its custom sections .debug_*.
#[cfg(feature = "engine")]
fn strip_debug(wasm: &[u8]) -> Vec<u8> {
    fn leb(b: &[u8], i: &mut usize) -> usize {
        let (mut n, mut shift) = (0, 0);
        loop {
            let x = b[*i];
            *i += 1;
            n |= ((x & 0x7f) as usize) << shift;
            shift += 7;
            if x < 0x80 {
                return n;
            }
        }
    }
    let mut out = wasm[..8].to_vec();
    let mut i = 8;
    while i < wasm.len() {
        let start = i;
        let id = wasm[i];
        i += 1;
        let size = leb(wasm, &mut i);
        let end = i + size;
        let debug = id == 0 && {
            let n = leb(wasm, &mut i);
            wasm[i..i + n].starts_with(b".debug")
        };
        if !debug {
            out.extend_from_slice(&wasm[start..end]);
        }
        i = end;
    }
    out
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
