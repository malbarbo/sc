//! The REPL's loop with its line editor, over the library's shell
//! (`sc::machine::shell`), to which it adds `:theme`.
use crate::platform::{Repl, Shell};
use crate::reader::{Reader, Theme};
use crate::{config, Result};
use sc::machine::shell::Status;
use sc::machine::{Lang, Safety};
use std::sync::{Arc, Mutex};

pub fn welcome_message(lang: Lang, safety: Safety) -> String {
    let std = match lang {
        Lang::C => "C (gnu23)",
        Lang::Cxx => "C++ (gnu++26)",
    };
    let safety = if safety == Safety::On { String::new() } else { format!(", safety {}", safety.name()) };
    format!("Welcome to sc {} for {std}{safety}.\nType ctrl-d or \":quit\" to exit.\n", env!("CARGO_PKG_VERSION"))
}

/// The REPL until the end of the input, `:quit` or an `exit`. Returns the
/// code of the `exit`, else 1 if the last input failed, else 0.
pub fn run(repl: Repl) -> Result<i32> {
    let theme = Arc::new(Mutex::new(config::load().theme));
    let mut shell = Shell::new(repl);
    shell.add(":theme", Some("[light|dark]"), "Show or switch the theme", theme_action(Arc::clone(&theme)));
    let completions = shell.completions();
    let shell = Arc::new(Mutex::new(shell));
    let mut reader = Reader::new(Arc::clone(&shell), completions, *theme.lock().unwrap())?;
    let mut status = 0;
    while let Some(input) = reader.next() {
        let mut s = shell.lock().unwrap();
        match s.run(&input)? {
            Status::Quit => break,
            // a blank line runs nothing
            _ if input.trim().is_empty() => {}
            st => status = st as i32,
        }
        reader.set_completions(s.completions());
        reader.set_theme(*theme.lock().unwrap());
    }
    let exit_status = shell.lock().unwrap().exit_status();
    Ok(exit_status.unwrap_or(status))
}

/// `:theme` alone says which theme is on; with a name it switches to it.
fn theme_action(theme: Arc<Mutex<Theme>>) -> impl FnMut(&str) -> std::result::Result<(), String> + Send {
    move |arg| {
        if arg.is_empty() {
            println!("{}", theme.lock().unwrap().name());
            return Ok(());
        }
        let t = Theme::parse(arg).ok_or_else(|| format!("Unknown theme: {arg}. Use 'light' or 'dark'."))?;
        *theme.lock().unwrap() = t;
        config::save(t);
        Ok(())
    }
}
