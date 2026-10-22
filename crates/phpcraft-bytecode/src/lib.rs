//! Bytecode produced by the compiler and executed by the VM.
//!
//! This is PHPCraft's counterpart of Zend's `zend_op`/`zend_op_array`. The
//! instruction set is stack based (simpler to verify than Zend's register
//! slots) and every opcode maps to one or two Zend opcodes; see
//! `docs/php-src-map.md` for the correspondence.

use phpcraft_ast::{BinOp, CastKind, UnOp};
use phpcraft_runtime::Value;

/// How one `AssignDim` level addresses the array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DimMode {
    /// `[$expr]`
    Key,
    /// `[]`
    Append,
}

#[derive(Clone, Debug)]
pub enum Op {
    Null,
    True,
    False,
    Int(i64),
    Float(f64),
    /// Push the string stored at `consts[index]`.
    StrConst(u32),
    /// Push the runtime constant with the name stored at `consts[index]`.
    ConstName(u32),

    Pop,
    Dup,

    LoadVar(u32),
    /// Like `LoadVar`, but yields `null` without a warning when undefined.
    LoadVarSafe(u32),
    StoreVar(u32),
    /// Store and keep the value on the stack (assignment expressions).
    StoreVarKeep(u32),

    LoadGlobal(u32),
    LoadGlobalSafe(u32),
    StoreGlobal(u32),
    StoreGlobalKeep(u32),

    LoadTmp(u32),
    StoreTmp(u32),

    Binary(BinOp),
    Unary(UnOp),
    /// `++`/`--` on the top of the stack, including PHP's string increment.
    IncDec {
        increment: bool,
    },
    /// Convert the top of the stack to another type (`(int)`, `(string)`, ...).
    Cast(CastKind),
    /// Pop a value and push its truthiness.
    ToBool,
    /// Pop a value and push whether it is not `null`.
    IsNotNull,

    Jump(usize),
    JumpIfFalse(usize),
    JumpIfTrue(usize),
    JumpIfFalseKeep(usize),
    JumpIfTrueKeep(usize),
    /// Pop a value; if it is not null, push it back and jump.
    JumpIfNotNull(usize),

    Echo,

    /// Push a new empty array.
    ArrayNew,
    /// `tmp[has_key ? key : next] = value`; both operands are popped.
    ArraySet {
        tmp: u32,
        has_key: bool,
    },

    /// Pop `key` and `array`, push `array[key]` (warning when missing).
    Index,
    /// Same, but yields `null` silently.
    IndexSafe,

    /// Pop `array`, the keys and `value`; push the modified array.
    AssignDim {
        modes: Vec<DimMode>,
    },

    Call {
        name: u32,
        argc: u32,
    },

    /// Abort with `UnhandledMatchError` after no match arm was selected.
    ThrowUnhandledMatch {
        subject: u32,
    },

    /// Pop the subject into a foreach iterator slot.
    IterInit {
        iter: u32,
    },
    /// Advance the iterator; when exhausted jump to `end`.
    IterNext {
        iter: u32,
        key: Option<u32>,
        value: u32,
        end: usize,
    },

    Return,
    ReturnNull,
}

/// A compilation unit: the ops plus the constant pool they reference.
#[derive(Clone, Debug, Default)]
pub struct Chunk {
    pub ops: Vec<Op>,
    pub consts: Vec<Value>,
}

impl Chunk {
    pub fn const_str(&self, index: u32) -> &[u8] {
        match &self.consts[index as usize] {
            Value::Str(s) => s.as_bytes(),
            other => panic!("constant {index} is not a string: {other:?}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ParamSlot {
    pub name: String,
    pub slot: u32,
    /// Constant default value; `None` means the parameter is required.
    pub default: Option<Value>,
    pub variadic: bool,
}

/// A compiled function (or the top-level script, named `{main}`).
#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub params: Vec<ParamSlot>,
    pub chunk: Chunk,
    pub max_slots: u32,
    /// Variable name per slot (temps have `None`); used in diagnostics.
    pub slot_names: Vec<Option<String>>,
    pub line: u32,
}

impl Function {
    pub fn display_name(&self) -> String {
        if self.name == "{main}" {
            "main".to_string()
        } else {
            self.name.clone()
        }
    }
}

#[derive(Clone, Debug)]
pub struct CompiledProgram {
    pub main: Function,
    pub functions: Vec<Function>,
    pub file: String,
}

impl CompiledProgram {
    pub fn all_functions(
        &self,
    ) -> std::iter::Chain<std::iter::Once<&Function>, std::slice::Iter<'_, Function>> {
        std::iter::once(&self.main).chain(self.functions.iter())
    }
}
