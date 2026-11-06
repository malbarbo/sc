//! What the machine reads, embedded (build.rs, PLANO-REPL.md §5.4): a
//! squashfs with the session's arguments at `repl/` and the libraries the
//! loader asks for, with their `.cwasm`, at `sysroot/lib/` ([`read`]). The
//! session's headers and libraries are in sc.wasm (engine/sc_pack.cpp). A
//! file is decompressed once, the first time it is read, and kept until the
//! process ends.
use backhand::{FilesystemReader, InnerNode};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::{Mutex, OnceLock};

// clippy (the CI's make check) builds without the engine (build.rs)
#[cfg(not(clippy))]
static PACK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sc.sqfs"));
#[cfg(clippy)]
static PACK: &[u8] = &[];

struct Pack {
    fs: FilesystemReader<'static>,
    /// The node and the size of each file, by path from the root.
    files: HashMap<String, (usize, u64)>,
    /// The files read.
    read: Mutex<HashMap<String, &'static [u8]>>,
}

fn pack() -> &'static Pack {
    static P: OnceLock<Pack> = OnceLock::new();
    P.get_or_init(|| {
        let fs = FilesystemReader::from_reader(Cursor::new(PACK)).expect("the pack");
        let mut files = HashMap::new();
        for (i, node) in fs.files().enumerate() {
            if let InnerNode::File(f) = &node.inner {
                let path = node.fullpath.to_str().expect("a path in UTF-8").trim_start_matches('/');
                files.insert(path.to_string(), (i, f.file_len() as u64));
            }
        }
        Pack { fs, files, read: Mutex::new(HashMap::new()) }
    })
}

/// The file at `path` from the root (`repl/c-on.args`), if the pack has it.
pub fn read(path: &str) -> Option<&'static [u8]> {
    let p = pack();
    let &(i, size) = p.files.get(path)?;
    let mut read = p.read.lock().unwrap();
    if let Some(b) = read.get(path) {
        return Some(b);
    }
    let InnerNode::File(f) = &p.fs.root.nodes[i].inner else { unreachable!() };
    let mut b = Vec::with_capacity(size as usize);
    p.fs.file(f).reader().read_to_end(&mut b).expect("a file of the pack");
    let b: &'static [u8] = b.leak();
    read.insert(path.to_string(), b);
    Some(b)
}
