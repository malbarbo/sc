//! The messages on the pipes between the REPL and its child (`engine/sc.h`).
use super::MsgHeader;
use std::io::{self, Read, Write};

/// A message: its kind, its status and its three parts.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Msg {
    pub kind: u32,
    pub status: u32,
    pub parts: [Vec<u8>; 3],
}

impl Msg {
    pub fn new(kind: u32, status: u32, parts: [Vec<u8>; 3]) -> Msg {
        Msg { kind, status, parts }
    }
}

/// Writes a message and flushes.
pub fn write(w: &mut impl Write, kind: u32, status: u32, parts: [&[u8]; 3]) -> io::Result<()> {
    let mut len = [0u32; 3];
    for (l, p) in len.iter_mut().zip(parts) {
        *l = u32::try_from(p.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a part of 4 GiB or more"))?;
    }
    let h = MsgHeader { kind, status, len };
    let mut buf = Vec::with_capacity(20);
    for x in [h.kind, h.status, h.len[0], h.len[1], h.len[2]] {
        buf.extend(x.to_ne_bytes());
    }
    w.write_all(&buf)?;
    for p in parts {
        w.write_all(p)?;
    }
    w.flush()
}

/// Reads a message; `None` at the end of the input.
pub fn read(r: &mut impl Read) -> io::Result<Option<Msg>> {
    let mut buf = [0u8; 20];
    // the end of the input only before a message
    let mut got = 0;
    while got < buf.len() {
        match r.read(&mut buf[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    let word = |i: usize| u32::from_ne_bytes(buf[i * 4..i * 4 + 4].try_into().unwrap());
    let mut msg = Msg { kind: word(0), status: word(1), ..Msg::default() };
    for (i, p) in msg.parts.iter_mut().enumerate() {
        *p = vec![0; word(2 + i) as usize];
        r.read_exact(p)?;
    }
    Ok(Some(msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut buf = vec![];
        write(&mut buf, 2, 1, [b"int x;", b"", b"a\nb\n"]).unwrap();
        write(&mut buf, 3, 0, [b"", b"", b""]).unwrap();
        let mut r = &buf[..];
        assert_eq!(read(&mut r).unwrap(), Some(Msg::new(2, 1, [b"int x;".to_vec(), vec![], b"a\nb\n".to_vec()])));
        assert_eq!(read(&mut r).unwrap(), Some(Msg::new(3, 0, Default::default())));
        assert_eq!(read(&mut r).unwrap(), None);
        let mut cut = &buf[..10];
        assert_eq!(read(&mut cut).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }
}
