//! The REPL's loop and its `:` commands, as in sgleam and spython.
use crate::machine::{Lang, Machine, Outcome};
use std::sync::{Arc, Mutex};
use crate::reader::{Reader, Theme};
use crate::{config, Result};

pub fn welcome_message(lang: Lang) -> String {
    let std = match lang {
        Lang::C => "C (gnu23)",
        Lang::Cxx => "C++ (gnu++26)",
    };
    format!("Welcome to sc {} for {std}.\nType ctrl-d or \":quit\" to exit.\n", env!("CARGO_PKG_VERSION"))
}

/// The commands: the name, what it takes, and its help.
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (":help", "", "Show this help"),
    (":quit", "", "Exit the REPL"),
    (":type", "EXPR", "Show the type of an expression, without running it"),
    (":undo", "", "Forget the last cell's declarations (what it ran stays done)"),
    (":theme", "[light|dark]", "Show or switch the theme"),
];

/// The name of the command at the head of the input and what follows it, or
/// none for code, which starts with `:` only as C++'s `::x`.
pub fn split(input: &str) -> Option<(&str, &str)> {
    let trimmed = input.trim();
    if !trimmed.starts_with(':') || trimmed.starts_with("::") {
        return None;
    }
    let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let (name, arg) = trimmed.split_at(end);
    Some((name, arg.trim_start()))
}

/// The REPL until the end of the input, `:quit` or an `exit`: the status.
pub fn run(machine: Machine) -> Result<i32> {
    let mut theme = config::load().theme;
    let completions_now = completions(&machine);
    let machine = Arc::new(Mutex::new(machine));
    let mut reader = Reader::new(Arc::clone(&machine), completions_now, theme)?;
    let mut status = 0;
    while let Some(input) = reader.next() {
        let mut machine = machine.lock().unwrap();
        if input.trim().is_empty() {
            continue;
        }
        let Some((name, arg)) = split(&input) else {
            status = match machine.cell(&input)? {
                Outcome::Ok => 0,
                Outcome::Error => 1,
                Outcome::Exit(code) => return Ok(code),
            };
            reader.set_completions(completions(&machine));
            continue;
        };
        let Some(&(_, takes, _)) = COMMANDS.iter().find(|c| c.0 == name) else {
            println!("Unknown command {name}. Type :help to see the commands.");
            continue;
        };
        if takes.is_empty() && !arg.is_empty() {
            println!("The {name} command takes nothing after it.");
            continue;
        }
        match name {
            ":help" => {
                let usage = |(n, t, _): &(&str, &str, &str)| if t.is_empty() { n.to_string() } else { format!("{n} {t}") };
                let width = COMMANDS.iter().map(|c| usage(c).len()).max().unwrap_or(0);
                println!("Commands:");
                for c in COMMANDS {
                    println!("  {:<width$}  {}", usage(c), c.2);
                }
            }
            ":quit" => break,
            ":type" if arg.is_empty() => println!("The :type command takes an expression."),
            ":type" => status = if machine.type_of(arg)? { 0 } else { 1 },
            ":undo" => {
                if machine.undo()? {
                    reader.set_completions(completions(&machine));
                }
            }
            ":theme" if arg.is_empty() => println!("{}", theme.name()),
            ":theme" => match Theme::parse(arg) {
                Some(t) => {
                    theme = t;
                    reader.set_theme(t);
                    config::save(t);
                }
                None => println!("Unknown theme: {arg}. Use 'light' or 'dark'."),
            },
            _ => unreachable!("a command in COMMANDS"),
        }
    }
    Ok(status)
}

/// The names Tab offers: the cells', the keywords and the commands.
fn completions(machine: &Machine) -> Vec<String> {
    // a name that starts with `_` is the implementation's, or mangled
    let mut names: Vec<String> = machine.names().filter(|n| !n.starts_with('_')).map(String::from).collect();
    names.extend(crate::reader::KEYWORDS.iter().map(|k| k.to_string()));
    names.extend(COMMANDS.iter().map(|c| if c.1.is_empty() { c.0.to_string() } else { format!("{} ", c.0) }));
    names.sort();
    names.dedup();
    names
}
