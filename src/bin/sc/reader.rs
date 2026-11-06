//! The line the terminal reads, and the colours it draws it in (from
//! sgleam's reader, for C).
use rustyline::{
    completion::Completer,
    error::ReadlineError,
    highlight::{CmdKind, Highlighter},
    history::FileHistory,
    validate::{ValidationContext, ValidationResult, Validator},
    Cmd, CompletionType, ConditionalEventHandler, Config, Context, Editor, Event, EventContext, EventHandler, Helper,
    Hinter, KeyCode, KeyEvent, Modifiers, Movement, Prompt, RepeatCount,
};
use std::borrow::Cow;
use std::cell::RefCell;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::platform::Machine;
use sc::machine::{Enter, INDENT};


pub struct Reader {
    // `next` takes the editor to read and gives it back, except on the read
    // that ends the session, which is how the iteration stops.
    editor: Option<Editor<InputHelper, FileHistory>>,
}

/// Whether `stream` takes colour: <https://no-color.org>, and `FORCE_COLOR`
/// where no terminal is watching.
pub fn color(terminal: bool) -> bool {
    let set = |v| std::env::var_os(v).is_some_and(|s| !s.is_empty());
    !set("NO_COLOR") && (terminal || set("FORCE_COLOR"))
}

impl Reader {
    pub fn new(machine: Arc<Mutex<Machine>>, completions: Vec<String>, theme: Theme) -> rustyline::Result<Reader> {
        let config = Config::builder().completion_type(CompletionType::List).build();
        let mut editor = Editor::with_config(config)?;
        editor.set_helper(Some(InputHelper {
            machine: Arc::clone(&machine),
            pending: RefCell::new(String::new()),
            completions,
            color: color(std::io::stdout().is_terminal()),
            theme,
        }));
        let bind = |e: &mut Editor<_, _>, key, h: Box<dyn ConditionalEventHandler>| {
            e.bind_sequence(KeyEvent(key, Modifiers::NONE), EventHandler::Conditional(h));
        };
        bind(&mut editor, KeyCode::Enter, Box::new(EnterHandler(machine)));
        bind(&mut editor, KeyCode::Tab, Box::new(TabHandler));
        bind(&mut editor, KeyCode::Backspace, Box::new(SmartBackspace));
        if let Some(history) = history_path() {
            let _ = editor.load_history(&history);
        }
        Ok(Reader { editor: Some(editor) })
    }

    /// The names Tab offers, which grow with every cell.
    pub fn set_completions(&mut self, completions: Vec<String>) {
        if let Some(helper) = self.editor.as_mut().and_then(Editor::helper_mut) {
            helper.completions = completions;
        }
    }

    pub fn set_theme(&mut self, theme: Theme) {
        if let Some(helper) = self.editor.as_mut().and_then(Editor::helper_mut) {
            helper.theme = theme;
        }
    }
}

struct ReplPrompt {
    /// The prompt in the theme's colours, or none when the output takes none.
    styled: Option<String>,
}

impl Prompt for ReplPrompt {
    fn raw(&self) -> &str {
        "> "
    }

    fn styled(&self) -> &str {
        self.styled.as_deref().unwrap_or(self.raw())
    }
}

impl Iterator for Reader {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        let mut editor = self.editor.take()?;
        let styled = editor.helper().filter(|h| h.color).map(|h| format!("{}>{RESET} ", h.theme.palette().prompt));
        match editor.readline(&ReplPrompt { styled }) {
            Ok(input) => {
                if !input.trim().is_empty() {
                    let _ = editor.add_history_entry(&input);
                }
                self.editor = Some(editor);
                Some(input)
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl-C throws the input away, so nothing stays pending
                take_pending(&editor);
                self.editor = Some(editor);
                Some(String::new())
            }
            Err(err) => {
                if !matches!(err, ReadlineError::Eof) {
                    eprintln!("Error: {err}");
                }
                if let Some(history) = history_path() {
                    let _ = editor.save_history(&history);
                }
                // an input the end of the file cut off still runs: the user
                // wants to read what is wrong with it
                let pending = take_pending(&editor);
                (!pending.trim().is_empty()).then_some(pending)
            }
        }
    }
}

fn take_pending(editor: &Editor<InputHelper, FileHistory>) -> String {
    editor.helper().map_or_else(String::new, |h| h.pending.take())
}

fn history_path() -> Option<PathBuf> {
    let dir = dirs::data_dir()?.join(crate::NAME);
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("history"))
}

#[derive(Helper, Hinter)]
struct InputHelper {
    /// The compiler, which says whether the input is whole.
    machine: Arc<Mutex<Machine>>,
    /// The unfinished input. The editor throws it away when the file ends
    /// before the rest of it, and says nothing about it.
    pending: RefCell<String>,
    completions: Vec<String>,
    color: bool,
    theme: Theme,
}

impl Completer for InputHelper {
    type Candidate = String;

    fn complete(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> rustyline::Result<(usize, Vec<String>)> {
        Ok(sc::machine::shell::complete(&self.completions, line, pos))
    }
}

impl Validator for InputHelper {
    /// Without a terminal, Enter is not a key: the input goes on while the
    /// compiler says it stops short.
    fn validate(&self, ctx: &mut ValidationContext) -> rustyline::Result<ValidationResult> {
        let input = ctx.input();
        let enter = self.machine.lock().unwrap().enter(input, input.len());
        *self.pending.borrow_mut() = if enter == Enter::Run { String::new() } else { input.into() };
        Ok(if enter == Enter::Run { ValidationResult::Valid(None) } else { ValidationResult::Incomplete })
    }
}

/// Enter: the input runs, or a line opens as deep as the brackets above the
/// caret ([`Machine::enter`]).
struct EnterHandler(Arc<Mutex<Machine>>);

impl ConditionalEventHandler for EnterHandler {
    fn handle(&self, _: &Event, _: RepeatCount, _: bool, ctx: &EventContext) -> Option<Cmd> {
        // the compile (Machine::enter) stops at Ctrl-C, which rustyline's
        // raw mode would read after it
        let enter = {
            let _ctrl_c = sc::loader::native::InRawMode::new();
            self.0.lock().unwrap().enter(ctx.line(), ctx.pos())
        };
        match enter {
            Enter::Run => None,
            Enter::Open(depth) => Some(Cmd::Insert(1, format!("\n{:1$}", "", depth * INDENT))),
        }
    }
}

/// Tab indents when only whitespace comes before the caret, and completes
/// otherwise.
struct TabHandler;

impl ConditionalEventHandler for TabHandler {
    fn handle(&self, _: &Event, _: RepeatCount, _: bool, ctx: &EventContext) -> Option<Cmd> {
        let before = &ctx.line()[..ctx.pos()];
        let line = &before[before.rfind('\n').map_or(0, |i| i + 1)..];
        if line.chars().all(char::is_whitespace) {
            Some(Cmd::Insert(1, " ".repeat(INDENT - line.len() % INDENT)))
        } else {
            Some(Cmd::Complete)
        }
    }
}

/// Backspace takes back a whole level of indentation on a continuation line
/// with only spaces before the caret. (A `}` does not: rustyline 18 leaves
/// the caret before what a `Cmd::Replace` inserts.)
struct SmartBackspace;

impl ConditionalEventHandler for SmartBackspace {
    fn handle(&self, _: &Event, _: RepeatCount, _: bool, ctx: &EventContext) -> Option<Cmd> {
        let before = &ctx.line()[..ctx.pos()];
        let line = &before[before.rfind('\n')? + 1..];
        let spaces = line.len();
        if spaces < 2 || !line.bytes().all(|b| b == b' ') {
            return None;
        }
        let remove = if spaces.is_multiple_of(INDENT) { INDENT } else { spaces % INDENT };
        Some(Cmd::Kill(Movement::BackwardChar(remove as u16)))
    }
}

// --- colours

const RESET: &str = "\x1b[0m";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    Dark,
    Light,
}

impl Theme {
    pub fn parse(name: &str) -> Option<Theme> {
        match name {
            "dark" => Some(Theme::Dark),
            "light" => Some(Theme::Light),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }

    fn palette(self) -> &'static Palette {
        match self {
            Theme::Dark => &ONE_DARK,
            Theme::Light => &ONE_LIGHT,
        }
    }
}

struct Palette {
    comment: &'static str,
    string: &'static str,
    number: &'static str,
    keyword: &'static str,
    function: &'static str,
    directive: &'static str,
    command: &'static str,
    prompt: &'static str,
}

// Zed One Dark, as sgleam's
const ONE_DARK: Palette = Palette {
    comment: "\x1b[38;2;93;99;111m",
    string: "\x1b[38;2;161;193;129m",
    number: "\x1b[38;2;191;149;106m",
    keyword: "\x1b[38;2;180;119;207m",
    function: "\x1b[38;2;115;173;233m",
    directive: "\x1b[38;2;110;180;191m",
    command: "\x1b[38;2;130;137;151m",
    prompt: "\x1b[38;2;115;173;233m",
};

// Zed One Light
const ONE_LIGHT: Palette = Palette {
    comment: "\x1b[38;2;162;163;167m",
    string: "\x1b[38;2;100;159;87m",
    number: "\x1b[38;2;173;110;37m",
    keyword: "\x1b[38;2;164;73;171m",
    function: "\x1b[38;2;91;121;227m",
    directive: "\x1b[38;2;56;130;183m",
    command: "\x1b[38;2;105;108;119m",
    prompt: "\x1b[38;2;91;121;227m",
};

/// C23's keywords and C++26's, which Tab also offers.
impl Highlighter for InputHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        if self.color {
            Cow::Owned(highlight(line, self.theme.palette()))
        } else {
            Cow::Borrowed(line)
        }
    }

    fn highlight_char(&self, _line: &str, _pos: usize, _kind: CmdKind) -> bool {
        self.color
    }
}

fn highlight(input: &str, t: &Palette) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let paint = |out: &mut String, colour: &str, s: &[char]| {
        out.push_str(colour);
        out.extend(s);
        out.push_str(RESET);
    };
    // a command is the first word of the input
    let start = chars.iter().take_while(|c| c.is_whitespace()).count();
    if chars.get(start) == Some(&':') {
        let end = start + chars[start..].iter().take_while(|c| !c.is_whitespace()).count();
        out.extend(&chars[..start]);
        paint(&mut out, t.command, &chars[start..end]);
        // what :type takes is code
        if chars[start..end].iter().collect::<String>() != ":type" {
            out.extend(&chars[end..]);
            return out;
        }
        i = end;
    }
    let mut line_start = i == 0;
    while let Some(&c) = chars.get(i) {
        let begin = i;
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while chars.get(i).is_some_and(|&c| c != '\n') {
                i += 1;
            }
            paint(&mut out, t.comment, &chars[begin..i]);
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i = (i + 2).min(chars.len());
            paint(&mut out, t.comment, &chars[begin..i]);
        } else if c == '"' || c == '\'' {
            i += 1;
            while let Some(&n) = chars.get(i) {
                i += 1;
                if n == '\\' {
                    i += 1;
                } else if n == c || n == '\n' {
                    break;
                }
            }
            i = i.min(chars.len());
            paint(&mut out, t.string, &chars[begin..i]);
        } else if c == '#' && line_start {
            i += 1;
            while chars.get(i).is_some_and(|c| c.is_whitespace() && *c != '\n') {
                i += 1;
            }
            while chars.get(i).is_some_and(|c| c.is_alphanumeric()) {
                i += 1;
            }
            paint(&mut out, t.directive, &chars[begin..i]);
        } else if c.is_ascii_digit() {
            while chars.get(i).is_some_and(|&c| c.is_ascii_alphanumeric() || c == '.' || c == '\'') {
                i += 1;
            }
            paint(&mut out, t.number, &chars[begin..i]);
        } else if c.is_alphabetic() || c == '_' {
            while chars.get(i).is_some_and(|&c| c.is_alphanumeric() || c == '_') {
                i += 1;
            }
            let word: String = chars[begin..i].iter().collect();
            if sc::machine::shell::KEYWORDS.contains(&word.as_str()) {
                paint(&mut out, t.keyword, &chars[begin..i]);
            } else if chars.get(i) == Some(&'(') {
                paint(&mut out, t.function, &chars[begin..i]);
            } else {
                out.push_str(&word);
            }
        } else {
            out.push(c);
            i += 1;
        }
        line_start = match c {
            '\n' => true,
            c if c.is_whitespace() => line_start,
            _ => false,
        };
    }
    out
}
