//! The `:` commands, over a REPL that knows none of them, as in sgleam
//! and spython. The native `sc` reads the inputs with its line editor and
//! adds `:theme` (src/bin/sc/shell.rs); sc.wasm reads them from its stdin,
//! or gets them from its host.
use super::{split, Enter, Outcome, Platform, Repl, Result, Safety};

/// What became of an input. `repl_run` answers with these numbers.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Ok = 0,
    Error = 1,
    Quit = 2,
}

pub struct Shell<P: Platform> {
    repl: Repl<P>,
    commands: Vec<Command>,
    /// The code of the `exit` that ended the session, if one ran.
    exit_status: Option<i32>,
}

struct Command {
    name: &'static str,
    arg: Arg,
    help: &'static str,
    action: Action,
}

/// What a command takes after its name.
#[derive(Clone, Copy)]
enum Arg {
    None,
    /// C or C++, required.
    Expr,
    /// An optional word, which the help shows exactly as it stands here.
    Word(&'static str),
}

enum Action {
    Builtin(Builtin),
    Host(HostAction),
}

/// A command of the host's. `Send`, as the native `sc` shares the shell
/// with its line editor.
type HostAction = Box<dyn FnMut(&str) -> std::result::Result<(), String> + Send>;

#[derive(Clone, Copy)]
enum Builtin {
    Help,
    Quit,
    Type,
    Undo,
    Leaks,
    Safety,
}

const BUILTINS: &[(&str, Arg, &str, Builtin)] = &[
    (":help", Arg::None, "Show this help", Builtin::Help),
    (":quit", Arg::None, "Exit the REPL", Builtin::Quit),
    (":type", Arg::Expr, "Show the type of an expression, without running it", Builtin::Type),
    (":undo", Arg::None, "Forget the last cell's declarations (what it ran stays done)", Builtin::Undo),
    (":leaks", Arg::None, "Show the memory the cells allocated and nothing points to any more", Builtin::Leaks),
    (
        ":safety",
        Arg::Word("[off|on|strict]"),
        "Show or switch how much the compiler checks (starts over, empty)",
        Builtin::Safety,
    ),
];

impl<P: Platform> Shell<P> {
    pub fn new(repl: Repl<P>) -> Shell<P> {
        let commands = BUILTINS
            .iter()
            .map(|&(name, arg, help, builtin)| Command { name, arg, help, action: Action::Builtin(builtin) })
            .collect();
        Shell { repl, commands, exit_status: None }
    }

    /// A command of the host's. `word` is what the help shows after the name,
    /// for one that takes a word; `run` gets what followed the name, and
    /// answers with what to tell the user when it refuses.
    pub fn add(
        &mut self,
        name: &'static str,
        word: Option<&'static str>,
        help: &'static str,
        run: impl FnMut(&str) -> std::result::Result<(), String> + Send + 'static,
    ) {
        debug_assert!(self.commands.iter().all(|c| c.name != name));
        self.commands.push(Command { name, arg: word.map_or(Arg::None, Arg::Word), help, action: Action::Host(Box::new(run)) });
    }

    /// The code of the `exit` that ended the session, if one ran.
    pub fn exit_status(&self) -> Option<i32> {
        self.exit_status
    }

    /// What Enter does with `input`: see [`Repl::enter`].
    pub fn enter(&mut self, input: &str, cursor: usize) -> Enter {
        self.repl.enter(input, cursor)
    }

    /// Runs an input, a cell or a command. An `Err` is a session that cannot
    /// go on.
    pub fn run(&mut self, input: &str) -> Result<Status> {
        if input.trim().is_empty() {
            return Ok(Status::Ok);
        }
        let Some((name, arg)) = split(input) else {
            return Ok(match self.repl.cell(input)? {
                Outcome::Ok => Status::Ok,
                Outcome::Error => Status::Error,
                Outcome::Exit(code) => {
                    self.exit_status = Some(code);
                    Status::Quit
                }
            });
        };
        let Some(command) = self.commands.iter_mut().find(|c| c.name == name) else {
            println!("Unknown command {name}. Type :help to see the commands.");
            return Ok(Status::Error);
        };
        match (command.arg, arg.is_empty()) {
            (Arg::None, false) => {
                println!("The {name} command takes nothing after it.");
                return Ok(Status::Error);
            }
            (Arg::Expr, true) => {
                println!("The {name} command takes an expression.");
                return Ok(Status::Error);
            }
            _ => {}
        }
        let builtin = match &mut command.action {
            Action::Host(run) => {
                return Ok(match run(arg) {
                    Ok(()) => Status::Ok,
                    Err(message) => {
                        println!("{message}");
                        Status::Error
                    }
                });
            }
            Action::Builtin(builtin) => *builtin,
        };
        self.builtin(builtin, arg)
    }

    fn builtin(&mut self, builtin: Builtin, arg: &str) -> Result<Status> {
        let ok = |done: bool| if done { Status::Ok } else { Status::Error };
        let m = &mut self.repl;
        Ok(match builtin {
            Builtin::Help => {
                let usages: Vec<String> = self.commands.iter().map(Command::usage).collect();
                let width = usages.iter().map(String::len).max().unwrap_or(0);
                println!("Commands:");
                for (usage, c) in usages.iter().zip(&self.commands) {
                    println!("  {usage:<width$}  {}", c.help);
                }
                Status::Ok
            }
            Builtin::Quit => Status::Quit,
            Builtin::Type => ok(m.type_of(arg)?),
            Builtin::Undo => ok(m.undo()),
            Builtin::Leaks if m.safety() == Safety::Off => {
                println!("The :leaks command needs safety on or strict.");
                Status::Error
            }
            Builtin::Leaks => {
                if m.leaks() == 0 {
                    println!("No leaks.");
                }
                Status::Ok
            }
            Builtin::Safety if arg.is_empty() => {
                println!("{}", m.safety().name());
                Status::Ok
            }
            Builtin::Safety => match Safety::parse(arg) {
                Some(s) => {
                    m.restart(s)?;
                    println!("Safety {}: the session starts over, empty.", s.name());
                    Status::Ok
                }
                None => {
                    println!("Unknown safety level: {arg}. Use 'off', 'on' or 'strict'.");
                    Status::Error
                }
            },
        })
    }

    /// The names Tab offers: the cells' but the implementation's (`_`), the
    /// keywords and the commands, sorted. A command that takes something
    /// comes with the space that goes after it.
    pub fn completions(&self) -> Vec<String> {
        let mut c: Vec<String> = self.repl.names().filter(|n| !n.starts_with('_')).map(String::from).collect();
        c.extend(KEYWORDS.iter().map(|k| k.to_string()));
        c.extend(self.commands.iter().map(|c| match c.arg {
            Arg::None => c.name.to_string(),
            Arg::Expr | Arg::Word(_) => format!("{} ", c.name),
        }));
        c.sort();
        c.dedup();
        c
    }
}

impl Command {
    fn usage(&self) -> String {
        match self.arg {
            Arg::None => self.name.to_string(),
            Arg::Expr => format!("{} EXPR", self.name),
            Arg::Word(word) => format!("{} {word}", self.name),
        }
    }
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
