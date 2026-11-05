//! The REPL's loop with its line editor: the lines are the library's
//! (`sc::machine::shell`), with `:theme`.
use crate::platform::Machine;
use sc::machine::shell::{self, Step};
use sc::machine::{Lang, Safety};
use std::sync::{Arc, Mutex};
use crate::reader::{Reader, Theme};
use crate::{config, Result};

pub fn welcome_message(lang: Lang, safety: Safety) -> String {
    let std = match lang {
        Lang::C => "C (gnu23)",
        Lang::Cxx => "C++ (gnu++26)",
    };
    let safety = if safety == Safety::On { String::new() } else { format!(", safety {}", safety.name()) };
    format!("Welcome to sc {} for {std}{safety}.\nType ctrl-d or \":quit\" to exit.\n", env!("CARGO_PKG_VERSION"))
}

/// The commands: the library's, then the theme.
pub fn commands() -> Vec<(&'static str, &'static str, &'static str)> {
    let mut c = shell::COMMANDS.to_vec();
    c.push((":theme", "[light|dark]", "Show or switch the theme"));
    c
}

/// The REPL until the end of the input, `:quit` or an `exit`: the status.
pub fn run(machine: Machine) -> Result<i32> {
    let mut theme = config::load().theme;
    let commands = commands();
    let completions_now = completions(&machine, &commands);
    let machine = Arc::new(Mutex::new(machine));
    let mut reader = Reader::new(Arc::clone(&machine), completions_now, theme)?;
    let mut status = 0;
    while let Some(input) = reader.next() {
        let mut machine = machine.lock().unwrap();
        let mut theme_arg = None;
        let step = shell::line(&mut machine, &input, &commands, |_, arg| {
            theme_arg = Some(arg.to_string());
            Step::Same
        })?;
        match step {
            Step::Status(s) => {
                status = s;
                reader.set_completions(completions(&machine, &commands));
            }
            Step::Names => reader.set_completions(completions(&machine, &commands)),
            Step::Same => {}
            Step::Quit => break,
            Step::Exit(code) => return Ok(code),
        }
        match theme_arg.as_deref() {
            None => {}
            Some("") => println!("{}", theme.name()),
            Some(arg) => match Theme::parse(arg) {
                Some(t) => {
                    theme = t;
                    reader.set_theme(t);
                    config::save(t);
                }
                None => println!("Unknown theme: {arg}. Use 'light' or 'dark'."),
            },
        }
    }
    Ok(status)
}

/// The names Tab offers.
fn completions(machine: &Machine, commands: &[(&str, &str, &str)]) -> Vec<String> {
    shell::candidates(machine.names(), commands)
}
