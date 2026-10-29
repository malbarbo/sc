//! What a session reads, embedded (build.rs, PLANO-REPL.md §5.4): a squashfs
//! with the sysroot at `sysroot/`, clang's headers at `res/` and the
//! session's arguments at `repl/`. The machine reads the arguments and the
//! libraries the loader asks for ([`read`]), and the child's clang and
//! wasm-ld see it all under [`ROOT`] ([`install`]). A file is decompressed
//! once, the first time it is read, and kept until the process ends: the
//! engine keeps pointers to it.
use backhand::{FilesystemReader, InnerNode};
use sc::engine::sys::sc_files;
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_ulonglong, CStr, CString};
use std::io::{Cursor, Read};
use std::sync::{Mutex, OnceLock};

static PACK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sc.sqfs"));

use sc::machine::ROOT;

enum Entry {
    /// The node and the size.
    File(usize, u64),
    /// The names in it, each followed by a NUL.
    Dir(Vec<u8>),
}

struct Pack {
    fs: FilesystemReader<'static>,
    /// By path from the root, "" for the root.
    entries: HashMap<String, Entry>,
    /// The files read, with a NUL after them.
    read: Mutex<HashMap<String, &'static [u8]>>,
}

fn pack() -> &'static Pack {
    static P: OnceLock<Pack> = OnceLock::new();
    P.get_or_init(|| {
        let fs = FilesystemReader::from_reader(Cursor::new(PACK)).expect("the pack");
        let mut entries = HashMap::new();
        for (i, node) in fs.files().enumerate() {
            let path = node.fullpath.to_str().expect("a path in UTF-8").trim_start_matches('/');
            let e = match &node.inner {
                InnerNode::File(f) => Entry::File(i, f.file_len() as u64),
                _ => Entry::Dir(Vec::new()),
            };
            entries.insert(path.to_string(), e);
        }
        // each name in its directory
        let paths: Vec<String> = entries.keys().filter(|p| !p.is_empty()).cloned().collect();
        for p in paths {
            let (dir, name) = p.rsplit_once('/').unwrap_or(("", &p));
            if let Some(Entry::Dir(names)) = entries.get_mut(dir) {
                names.extend_from_slice(name.as_bytes());
                names.push(0);
            }
        }
        Pack { fs, entries, read: Mutex::new(HashMap::new()) }
    })
}

/// The file at `path` from the root (`repl/c-on.args`), if the pack has it.
pub fn read(path: &str) -> Option<&'static [u8]> {
    with_nul(path).map(|b| &b[..b.len() - 1])
}

fn with_nul(path: &str) -> Option<&'static [u8]> {
    let p = pack();
    let Some(&Entry::File(i, size)) = p.entries.get(path) else {
        return None;
    };
    let mut read = p.read.lock().unwrap();
    if let Some(b) = read.get(path) {
        return Some(b);
    }
    let InnerNode::File(f) = &p.fs.root.nodes[i].inner else { unreachable!() };
    let mut b = Vec::with_capacity(size as usize + 1);
    p.fs.file(f).reader().read_to_end(&mut b).expect("a file of the pack");
    b.push(0);
    let b: &'static [u8] = b.leak();
    read.insert(path.to_string(), b);
    Some(b)
}

/// The engine's path as one from the root: under [`ROOT`], with either
/// separator.
fn local(path: *const c_char) -> Option<String> {
    // SAFETY: the engine passes a C string
    let p = unsafe { CStr::from_ptr(path) }.to_str().ok()?.replace('\\', "/");
    let p = p.strip_prefix(ROOT)?;
    (p.is_empty() || p.starts_with('/')).then(|| p.trim_matches('/').to_string())
}

extern "C" fn status(path: *const c_char, size: *mut c_ulonglong) -> c_int {
    let Some(p) = local(path) else { return 0 };
    let (kind, n) = match pack().entries.get(&p) {
        None => return 0,
        Some(Entry::File(_, n)) => (1, *n),
        Some(Entry::Dir(_)) => (2, 0),
    };
    // SAFETY: the engine passes where the size goes
    unsafe { *size = n };
    kind
}

extern "C" fn read_c(path: *const c_char, size: *mut c_ulonglong) -> *const c_char {
    let Some(b) = local(path).and_then(|p| with_nul(&p)) else {
        return std::ptr::null();
    };
    unsafe { *size = b.len() as u64 - 1 };
    b.as_ptr().cast()
}

extern "C" fn list(path: *const c_char, size: *mut c_ulonglong) -> *const c_char {
    let Some(Entry::Dir(names)) = local(path).and_then(|p| pack().entries.get(&p)) else {
        return std::ptr::null();
    };
    unsafe { *size = names.len() as u64 };
    names.as_ptr().cast()
}

/// The pack for the sessions of this process (`sc_set_files`).
pub fn install() {
    let root = CString::new(ROOT).unwrap().into_raw();
    let files = sc_files { root, status: Some(status), read: Some(read_c), list: Some(list) };
    sc::engine::set_files(Box::leak(Box::new(files)));
}
