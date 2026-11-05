//! The REPL's lines, as in sgleam and spython: a cell, or a `:` command.
//! The native `sc` reads them with its line editor (src/bin/sc/shell.rs),
//! which adds `:theme`; sc.wasm from its stdin, or from its host.
use super::{split, Machine, Outcome, Platform, Result, Safety};

/// The commands: the name, what it takes, and its help.
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (":help", "", "Show this help"),
    (":quit", "", "Exit the REPL"),
    (":type", "EXPR", "Show the type of an expression, without running it"),
    (":undo", "", "Forget the last cell's declarations (what it ran stays done)"),
    (":leaks", "", "Show the memory the cells allocated and nothing points to any more"),
    (":safety", "[off|on|strict]", "Show or switch how much the compiler checks (starts over, empty)"),
];

/// What a line did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The status of the REPL is now this: 0, or 1 after an error.
    Status(i32),
    /// The status stays.
    Same,
    /// The status stays, and the names of the cells changed.
    Names,
    /// `:quit`.
    Quit,
    /// The code called `exit`.
    Exit(i32),
}

/// Runs a line, as a cell or as one of `commands` (a list that starts with
/// [`COMMANDS`]): those that are not in [`COMMANDS`] are `other`'s.
pub fn line<P: Platform>(
    m: &mut Machine<P>,
    input: &str,
    commands: &[(&str, &str, &str)],
    other: impl FnOnce(&str, &str) -> Step,
) -> Result<Step> {
    if input.trim().is_empty() {
        return Ok(Step::Same);
    }
    let Some((name, arg)) = split(input) else {
        return Ok(match m.cell(input)? {
            Outcome::Ok => Step::Status(0),
            Outcome::Error => Step::Status(1),
            Outcome::Exit(code) => Step::Exit(code),
        });
    };
    let Some(&(_, takes, _)) = commands.iter().find(|c| c.0 == name) else {
        println!("Unknown command {name}. Type :help to see the commands.");
        return Ok(Step::Same);
    };
    if takes.is_empty() && !arg.is_empty() {
        println!("The {name} command takes nothing after it.");
        return Ok(Step::Same);
    }
    Ok(match name {
        ":help" => {
            let usage = |(n, t, _): &(&str, &str, &str)| if t.is_empty() { n.to_string() } else { format!("{n} {t}") };
            let width = commands.iter().map(|c| usage(c).len()).max().unwrap_or(0);
            println!("Commands:");
            for c in commands {
                println!("  {:<width$}  {}", usage(c), c.2);
            }
            Step::Same
        }
        ":quit" => Step::Quit,
        ":type" if arg.is_empty() => {
            println!("The :type command takes an expression.");
            Step::Same
        }
        ":type" => Step::Status(if m.type_of(arg)? { 0 } else { 1 }),
        ":undo" if m.undo()? => Step::Names,
        ":undo" => Step::Same,
        ":leaks" if m.safety() == Safety::Off => {
            println!("The :leaks command needs safety on or strict.");
            Step::Same
        }
        ":leaks" => {
            if m.leaks() == 0 {
                println!("No leaks.");
            }
            Step::Same
        }
        ":safety" if arg.is_empty() => {
            println!("{}", m.safety().name());
            Step::Same
        }
        ":safety" => match Safety::parse(arg) {
            Some(s) => {
                m.restart(s)?;
                println!("Safety {}: the session starts over, empty.", s.name());
                Step::Names
            }
            None => {
                println!("Unknown safety level: {arg}. Use 'off', 'on' or 'strict'.");
                Step::Same
            }
        },
        _ => other(name, arg),
    })
}

/// The keywords Tab offers, of C and C++.
pub const KEYWORDS: &[&str] = &[
    "alignas", "alignof", "auto", "bool", "break", "case", "catch", "char", "class", "const", "constexpr", "continue",
    "default", "delete", "do", "double", "else", "enum", "explicit", "extern", "false", "float", "for", "friend",
    "goto", "if", "inline", "int", "long", "namespace", "new", "noexcept", "nullptr", "operator", "private",
    "protected", "public", "register", "restrict", "return", "short", "signed", "sizeof", "static", "static_assert",
    "struct", "switch", "template", "this", "throw", "true", "try", "typedef", "typename", "typeof", "union",
    "unsigned", "using", "virtual", "void", "volatile", "while",
];

/// The names Tab offers: the cells' but the implementation's (`_`), the
/// keywords and the commands, sorted.
pub fn candidates<'a>(names: impl Iterator<Item = &'a str>, commands: &[(&str, &str, &str)]) -> Vec<String> {
    let mut c: Vec<String> = names.filter(|n| !n.starts_with('_')).map(String::from).collect();
    c.extend(KEYWORDS.iter().map(|k| k.to_string()));
    c.extend(commands.iter().map(|c| if c.1.is_empty() { c.0.to_string() } else { format!("{} ", c.0) }));
    c.sort();
    c.dedup();
    c
}

/// Tab at byte `pos` of `line`: where the word under completion starts, and
/// the `candidates` it can become; none without a word.
pub fn complete(candidates: &[String], line: &str, pos: usize) -> (usize, Vec<String>) {
    let before = &line[..pos];
    let start = before
        .char_indices()
        .rev()
        .find(|&(_, c)| !(c.is_alphanumeric() || c == '_' || c == ':'))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let prefix = &before[start..];
    if prefix.is_empty() {
        return (pos, vec![]);
    }
    (start, candidates.iter().filter(|n| n.starts_with(prefix)).cloned().collect())
}
