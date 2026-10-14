//! The value of a cell that ends in an expression without its `;`, which the
//! session turns into a call to `__sc_show_int` or `__sc_show_float`
//! (`enum sc_show` in engine/sc.h). It is written as clang-repl writes it:
//! the type, then the value, as in `(int) 3`.
use sc_proto::sys::sc_show as kind;

/// The longest string shown.
const MAX: usize = 1000;

/// The bytes of the C string at `addr`, at most `max`, and whether it goes
/// on; none if `addr` is outside the memory.
fn cstr(mem: &[u8], addr: u32, max: usize) -> Option<(&[u8], bool)> {
    let s = mem.get(addr as usize..)?;
    let end = s.iter().take(max).position(|&b| b == 0);
    Some(match end {
        Some(n) => (&s[..n], false),
        None => (&s[..max.min(s.len())], true),
    })
}

/// `c` escaped as in a C literal delimited by `quote`.
fn escape(c: char, quote: char, out: &mut String) {
    match c {
        '\n' => out.push_str("\\n"),
        '\t' => out.push_str("\\t"),
        '\r' => out.push_str("\\r"),
        '\0' => out.push_str("\\0"),
        '\\' => out.push_str("\\\\"),
        c if c == quote => {
            out.push('\\');
            out.push(c);
        }
        c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
        c => out.push(c),
    }
}

fn char_literal(b: u8) -> String {
    let mut s = String::from("'");
    if b.is_ascii() {
        escape(b as char, '\'', &mut s);
    } else {
        s.push_str(&format!("\\x{b:02x}"));
    }
    s.push('\'');
    s
}

fn string_literal(bytes: &[u8], cut: bool) -> String {
    let mut s = String::from("\"");
    for c in String::from_utf8_lossy(bytes).chars() {
        escape(c, '"', &mut s);
    }
    s.push('"');
    if cut {
        s.push_str("...");
    }
    s
}

fn type_name(mem: &[u8], ty: u32) -> String {
    cstr(mem, ty, 256).map(|(t, _)| String::from_utf8_lossy(t).into_owned()).unwrap_or_default()
}

/// `__sc_show_int(type, kind, value)`; `null` is how a null pointer is
/// written.
pub fn int(mem: &[u8], ty: u32, k: u32, v: i64, null: &str) -> String {
    let value = match k {
        kind::SC_SHOW_UNSIGNED => (v as u64).to_string(),
        kind::SC_SHOW_BOOL => (v != 0).to_string(),
        kind::SC_SHOW_CHAR => char_literal(v as u8),
        kind::SC_SHOW_NULLPTR => "nullptr".into(),
        kind::SC_SHOW_POINTER | kind::SC_SHOW_STRING if v == 0 => null.into(),
        kind::SC_SHOW_STRING => match cstr(mem, v as u32, MAX) {
            Some((s, cut)) => string_literal(s, cut),
            None => format!("0x{:x}", v as u32),
        },
        kind::SC_SHOW_POINTER => format!("0x{:x}", v as u32),
        _ => v.to_string(),
    };
    format!("({}) {value}", type_name(mem, ty))
}

/// `__sc_show_float(type, kind, value)`: the shortest digits that read back
/// as the same number.
pub fn float(mem: &[u8], ty: u32, k: u32, v: f64) -> String {
    let value = if k == kind::SC_SHOW_FLOAT { format!("{:?}", v as f32) } else { format!("{v:?}") };
    format!("({}) {}", type_name(mem, ty), if v.is_nan() { "nan" } else { &value })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values() {
        let mut mem = b"\0int\0char *\0ol\xc3\xa1\n\"\0".to_vec();
        mem.extend([b'x'; 3]);
        assert_eq!(int(&mem, 1, kind::SC_SHOW_SIGNED, -3, "NULL"), "(int) -3");
        assert_eq!(int(&mem, 1, kind::SC_SHOW_UNSIGNED, -1, "NULL"), "(int) 18446744073709551615");
        assert_eq!(int(&mem, 1, kind::SC_SHOW_CHAR, b'\'' as i64, "NULL"), "(int) '\\''");
        assert_eq!(int(&mem, 1, kind::SC_SHOW_CHAR, -61, "NULL"), "(int) '\\xc3'");
        assert_eq!(int(&mem, 5, kind::SC_SHOW_STRING, 12, "NULL"), "(char *) \"olá\\n\\\"\"");
        assert_eq!(int(&mem, 5, kind::SC_SHOW_STRING, 0, "NULL"), "(char *) NULL");
        assert_eq!(int(&mem, 5, kind::SC_SHOW_STRING, 19, "NULL"), "(char *) \"xxx\"...");
        assert_eq!(int(&mem, 5, kind::SC_SHOW_POINTER, 16, "nullptr"), "(char *) 0x10");
        assert_eq!(float(&mem, 1, kind::SC_SHOW_DOUBLE, 0.1 + 0.2), "(int) 0.30000000000000004");
        assert_eq!(float(&mem, 1, kind::SC_SHOW_FLOAT, 0.1f32 as f64), "(int) 0.1");
        assert_eq!(float(&mem, 1, kind::SC_SHOW_DOUBLE, 2.0), "(int) 2.0");
    }
}
