//! What the loader asks of the wasm runtime it runs on: the primitives of
//! the host API for wasm (NOTES.md §63). wasmtime gives them in `sc`
//! ([`super::native`]); the host gives them to sc.wasm as `env.wasm_*`
//! imports. Everything else the loader needs (the memory, the table, the
//! stack pointer, setting the table and the globals) it makes from them,
//! with modules it generates ([`super::gen`]).

use std::fmt;

/// A handle: a module instance, or something one exports. 0 is none.
pub type H = u32;

/// A value of an import of [`Runtime::instantiate`].
#[derive(Clone, Copy, Debug)]
pub enum Import {
    /// What the runtime gives by the import's name: WASI, which works on
    /// the memory the module imports, and `env.wasm_stack`, which keeps
    /// the stack of its caller under a number ([`Runtime::stack`]).
    Default,
    Handle(H),
    /// A global the runtime makes, i32, with this value; the instantiation
    /// puts its handle here.
    I32 { value: i32, mutable: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValType {
    I32,
    I64,
    F32,
    F64,
    FuncRef,
}

impl ValType {
    /// The letter of the type in a signature: `i` i32, `j` i64, `f` f32,
    /// `d` f64, `r` a reference (a handle).
    pub fn letter(self) -> char {
        match self {
            ValType::I32 => 'i',
            ValType::I64 => 'j',
            ValType::F32 => 'f',
            ValType::F64 => 'd',
            ValType::FuncRef => 'r',
        }
    }

    pub fn from_letter(c: char) -> Option<ValType> {
        Some(match c {
            'i' => ValType::I32,
            'j' => ValType::I64,
            'f' => ValType::F32,
            'd' => ValType::F64,
            'r' => ValType::FuncRef,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FuncType {
    pub params: Vec<ValType>,
    pub results: Vec<ValType>,
}

impl FuncType {
    pub fn new(params: impl IntoIterator<Item = ValType>, results: impl IntoIterator<Item = ValType>) -> FuncType {
        FuncType { params: params.into_iter().collect(), results: results.into_iter().collect() }
    }

    /// The signature: the parameters, `:`, the results (`ij:i`).
    pub fn sig(&self) -> String {
        let mut s: String = self.params.iter().map(|t| t.letter()).collect();
        s.push(':');
        s.extend(self.results.iter().map(|t| t.letter()));
        s
    }
}

/// Why code stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrapKind {
    /// Ctrl-C.
    Interrupt,
    StackOverflow,
    MemoryOutOfBounds,
    IntegerDivisionByZero,
    IntegerOverflow,
    BadSignature,
    IndirectCallToNull,
    TableOutOfBounds,
    Unreachable,
    /// Another: the message says it.
    Other,
}

/// A frame of a stack, innermost first.
#[derive(Clone, Debug)]
pub struct Frame {
    /// The instance whose code it is.
    pub instance: Option<H>,
    /// From the name section.
    pub func: Option<String>,
    /// The offset of the code in the module's bytes.
    pub offset: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Trap {
    pub kind: TrapKind,
    /// The runtime's description.
    pub message: String,
    /// The address of an access out of the memory, if the runtime says it.
    pub fault: Option<u64>,
    pub frames: Vec<Frame>,
}

#[derive(Clone, Debug)]
pub enum Error {
    /// The code called `exit` (WASI's `proc_exit`).
    Exit(i32),
    /// The code stopped.
    Trap(Trap),
    /// A module that does not compile or link, or another failure.
    Fail(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Exit(c) => write!(f, "exit with code {c}"),
            Error::Trap(t) => write!(f, "{}", t.message),
            Error::Fail(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

/// A failure, as [`Error::Fail`].
macro_rules! fail {
    ($($t:tt)*) => { return Err($crate::loader::runtime::Error::Fail(format!($($t)*))) };
}
pub(crate) use fail;

/// The primitives. A runtime keeps what it makes for as long as it lives:
/// a handle stays valid.
pub trait Runtime {
    /// Compiles and instantiates `bytes` (or `compiled`, the same module
    /// compiled ahead for this runtime, if it takes it) with `imports`, one
    /// for each import of the module, in its order. An [`Import::I32`]
    /// becomes the handle of the global made for it.
    fn instantiate(&mut self, bytes: &[u8], compiled: Option<&[u8]>, imports: &mut [Import]) -> Result<H>;

    /// What `inst` exports as `name`; 0 if nothing.
    fn export(&mut self, inst: H, name: &str) -> H;

    /// Calls `func`, of signature `sig` ([`FuncType::sig`]): its arguments
    /// in `vals`, each in 8 bytes (an f32 in the low 4, a reference as its
    /// handle); the results come back in them. `vals` has room for both.
    fn call(&mut self, func: H, sig: &str, vals: &mut [u64]) -> Result<()>;

    /// Reads the memory `mem` from `addr` into `buf`: false if out of it.
    fn read(&mut self, mem: H, addr: u32, buf: &mut [u8]) -> bool;

    /// Writes `data` to the memory `mem` at `addr`: false if out of it.
    fn write(&mut self, mem: H, addr: u32, data: &[u8]) -> bool;

    /// The stack that `env.wasm_stack` kept under `id`.
    fn stack(&self, id: u32) -> Vec<Frame>;
}
