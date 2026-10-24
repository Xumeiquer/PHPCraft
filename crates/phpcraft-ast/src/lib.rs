//! PHP abstract syntax tree.
//!
//! Mirrors the shape of `zend_ast.h` (`ZEND_AST_*` nodes) but in idiomatic
//! Rust. Only the constructs the engine currently executes are represented;
//! attributes are skipped by the parser and therefore have no node.

/// A parsed PHP file.
#[derive(Clone, Debug)]
pub struct Program {
    pub stmts: Vec<Stmt>,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    /// Text outside PHP tags, echoed verbatim.
    InlineHtml(Vec<u8>),
    /// `echo a, b;`
    Echo(Vec<Expr>),
    Expr(Expr),
    Block(Vec<Stmt>),
    If {
        /// `if` plus any `elseif` branches, in order.
        branches: Vec<(Expr, Vec<Stmt>)>,
        else_: Option<Vec<Stmt>>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    DoWhile {
        body: Vec<Stmt>,
        cond: Expr,
    },
    For {
        init: Vec<Expr>,
        cond: Vec<Expr>,
        step: Vec<Expr>,
        body: Vec<Stmt>,
    },
    Foreach {
        subject: Expr,
        key: Option<Expr>,
        value: Expr,
        by_ref: bool,
        body: Vec<Stmt>,
    },
    Switch {
        subject: Expr,
        cases: Vec<SwitchCase>,
    },
    Function(FunctionDecl),
    /// `class Name extends Parent { ... }`
    Class(ClassDecl),
    /// `namespace Name;` — applies to the following statements.
    NamespaceMark(Option<String>),
    /// `namespace Name { ... }` or `namespace { ... }`.
    NamespaceBlock {
        name: Option<String>,
        body: Vec<Stmt>,
    },
    /// One or more `use` imports.
    Use(Vec<UseDecl>),
    /// Namespace-level `const NAME = value, ...;`.
    Const(Vec<ConstDecl>),
    /// `declare(strict_types=1);` and block forms. Directive values are parsed
    /// but not enforced by this engine.
    Declare {
        directives: Vec<(String, Expr)>,
        body: Option<Vec<Stmt>>,
    },
    /// `try { ... } catch (...) { ... } finally { ... }`
    Try {
        body: Vec<Stmt>,
        catches: Vec<CatchClause>,
        finally: Option<Vec<Stmt>>,
    },
    /// `global $a, $b;`
    Global(Vec<String>),
    /// `static $a = 1, $b;`
    Static(Vec<(String, Option<Expr>)>),
    /// `unset($a, $b[0], $object->property);`
    Unset(Vec<Expr>),
    Return(Option<Expr>),
    Break(Option<u32>),
    Continue(Option<u32>),
    /// `goto label;`
    Goto(String),
    /// `label:`
    Label(String),
    Nop,
}

#[derive(Clone, Debug)]
pub struct SwitchCase {
    /// `None` for `default:`.
    pub test: Option<Expr>,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Debug)]
pub struct FunctionDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    /// `function &name()` returns by reference.
    pub returns_ref: bool,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub default: Option<Expr>,
    pub variadic: bool,
    pub by_ref: bool,
    /// Constructor property promotion (`public int $value`).
    pub promoted: Option<Visibility>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseKind {
    Class,
    Function,
    Constant,
}

#[derive(Clone, Debug)]
pub struct UseDecl {
    /// Fully-qualified target, without a leading backslash.
    pub target: String,
    /// Local alias; defaults to the final segment of `target`.
    pub alias: String,
    pub kind: UseKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Protected,
    Private,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Protected => "protected",
            Visibility::Private => "private",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PropertyDecl {
    pub name: String,
    pub default: Option<Expr>,
    pub visibility: Visibility,
    pub static_: bool,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub struct ConstDecl {
    pub name: String,
    pub value: Expr,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub struct ClassConstDecl {
    pub name: String,
    pub value: Expr,
    pub line: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassKind {
    Class,
    Interface,
    Trait,
    Enum,
}

#[derive(Clone, Debug)]
pub struct EnumCaseDecl {
    pub name: String,
    pub value: Option<Expr>,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub struct MethodDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    pub visibility: Visibility,
    pub static_: bool,
    /// `function &name()` returns by reference.
    pub returns_ref: bool,
    /// Interface/abstract declaration (`function f();`).
    pub abstract_: bool,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub struct TraitAdaptation {
    /// `Trait::` prefix, when present.
    pub trait_name: Option<String>,
    pub method: String,
    pub instead_of: Vec<String>,
    pub alias: Option<String>,
    pub visibility: Option<Visibility>,
}

#[derive(Clone, Debug)]
pub struct ClassDecl {
    pub kind: ClassKind,
    pub name: String,
    pub parent: Option<String>,
    pub interfaces: Vec<String>,
    pub traits: Vec<String>,
    pub trait_adaptations: Vec<TraitAdaptation>,
    pub properties: Vec<PropertyDecl>,
    pub constants: Vec<ClassConstDecl>,
    pub methods: Vec<MethodDecl>,
    pub enum_backing: Option<String>,
    pub enum_cases: Vec<EnumCaseDecl>,
    pub abstract_: bool,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub struct CatchClause {
    /// One or more class names (`catch (A|B $e)`).
    pub classes: Vec<String>,
    pub variable: String,
    pub body: Vec<Stmt>,
    pub line: u32,
}

/// A variable imported by `use (...)` on a closure.
#[derive(Clone, Debug)]
pub struct Capture {
    pub name: String,
    pub by_ref: bool,
}

/// The body of an anonymous function: a block for `function`, or a single
/// expression for `fn`.
#[derive(Clone, Debug)]
pub enum ClosureBody {
    Block(Vec<Stmt>),
    Expr(Expr),
}

#[derive(Clone, Debug)]
pub struct ClosureDecl {
    pub params: Vec<Param>,
    /// Explicit `use (...)` captures. Arrow functions leave this empty and
    /// capture free variables implicitly.
    pub captures: Vec<Capture>,
    pub body: ClosureBody,
    /// `function &()` / `fn&()` returns by reference.
    pub returns_ref: bool,
    pub line: u32,
}

impl ClosureDecl {
    pub fn is_arrow(&self) -> bool {
        matches!(self.body, ClosureBody::Expr(_))
    }
}

#[derive(Clone, Debug)]
pub struct ArrayItem {
    pub key: Option<Expr>,
    /// `None` is an empty slot in a destructuring target (`[, $b]`).
    pub value: Option<Expr>,
    /// `&$value` inside a destructuring target or array literal.
    pub by_ref: bool,
    /// `...$array` inside an array literal.
    pub spread: bool,
}

#[derive(Clone, Debug)]
pub struct MatchArm {
    /// Empty for `default`.
    pub conditions: Vec<Expr>,
    pub body: Expr,
}

#[derive(Clone, Debug)]
pub enum InterpPart {
    Lit(Vec<u8>),
    Expr(Expr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastKind {
    Int,
    Float,
    String,
    Bool,
    Array,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Vec<u8>),
    /// `(int) $x`, `(string) $x`, ...
    Cast {
        kind: CastKind,
        expr: Box<Expr>,
    },
    /// Double-quoted/heredoc string with `$vars` and `{$expr}`.
    Interp(Vec<InterpPart>),
    Var(String),
    /// `$$name` or `${expr}`.
    VariableVariable {
        name: Box<Expr>,
    },
    /// `yield`, `yield $value`, `yield $key => $value`, or `yield from $iterable`.
    Yield {
        key: Option<Box<Expr>>,
        value: Option<Box<Expr>>,
        from: bool,
    },
    /// A bare name used as a constant (`PHP_EOL`, `true` is parsed separately).
    Name(String),
    Array(Vec<ArrayItem>),
    /// `list(...)` or `[...]` used as an assignment/foreach target.
    Destructure(Vec<ArrayItem>),
    Assign {
        target: Box<Expr>,
        /// `None` for `=`, otherwise the compound operator (`+=`).
        op: Option<BinOp>,
        value: Box<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Unary {
        op: UnOp,
        expr: Box<Expr>,
    },
    /// `$a++`, `--$a`.
    IncDec {
        target: Box<Expr>,
        increment: bool,
        prefix: bool,
    },
    Ternary {
        cond: Box<Expr>,
        /// `None` for the short form `$a ?: $b`.
        then_: Option<Box<Expr>>,
        else_: Box<Expr>,
    },
    Index {
        base: Box<Expr>,
        /// `None` means append (`$a[] = ...`), only legal as an assignment target.
        index: Option<Box<Expr>>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
    /// `$fn(...)`, `(function () { ... })(...)`, or a callable expression.
    DynamicCall {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    /// First-class callable syntax: `strlen(...)`, `$object->method(...)`,
    /// `Class::method(...)` or `$callable(...)`. The inner expression is the
    /// corresponding call with an empty argument list.
    FirstClassCallable(Box<Expr>),
    /// `new Class(...)`.
    New {
        class: String,
        args: Vec<Expr>,
    },
    /// `new $class(...)`.
    NewDynamic {
        class: Box<Expr>,
        args: Vec<Expr>,
    },
    /// Anonymous class instantiation (`new class { ... }`).
    NewAnonymous {
        class: Box<ClassDecl>,
        args: Vec<Expr>,
    },
    /// `$object->name` (method calls are represented separately).
    Property {
        object: Box<Expr>,
        name: String,
    },
    /// `$object->$name` / `$object->{$expr}`.
    DynamicProperty {
        object: Box<Expr>,
        name: Box<Expr>,
    },
    /// `$object->method(...)`.
    MethodCall {
        object: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    /// `$object->$method(...)`.
    DynamicMethodCall {
        object: Box<Expr>,
        method: Box<Expr>,
        args: Vec<Expr>,
    },
    /// `Class::method(...)` (also `self::`, `parent::`, `static::`).
    StaticCall {
        class: String,
        method: String,
        args: Vec<Expr>,
    },
    /// `$class::method(...)` or `$class::$method(...)`.
    DynamicStaticCall {
        class: Box<Expr>,
        method: Box<Expr>,
        args: Vec<Expr>,
    },
    /// `Class::$property`.
    StaticProperty {
        class: String,
        name: String,
    },
    /// `$class::$property` / `$class::$$name`.
    DynamicStaticProperty {
        class: Box<Expr>,
        name: Box<Expr>,
    },
    /// `Class::CONSTANT`, including `Class::class`.
    ClassConst {
        class: String,
        name: String,
    },
    /// `$class::CONSTANT` or `$class::{$name}`.
    DynamicClassConst {
        class: Box<Expr>,
        name: Box<Expr>,
    },
    /// `$value instanceof Class`.
    InstanceOf {
        expr: Box<Expr>,
        class: String,
    },
    /// `clone $object`.
    Clone(Box<Expr>),
    /// `&$value`, only valid as an assignment value.
    Reference(Box<Expr>),
    /// `throw $value` (an expression in PHP 8).
    Throw(Box<Expr>),
    /// `include`, `include_once`, `require` or `require_once`.
    Include {
        path: Box<Expr>,
        once: bool,
        require: bool,
    },
    /// `function (...) use (...) { ... }` or `fn (...) => ...`.
    Closure(Box<ClosureDecl>),
    /// `match ($value) { 1, 2 => ..., default => ... }`.
    Match {
        subject: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    Isset(Vec<Expr>),
    Empty(Box<Expr>),
    /// `print $value` (an expression returning `1`).
    Print(Box<Expr>),
    /// `exit` / `die`, with an optional status message or code.
    Exit(Option<Box<Expr>>),
    /// `...$values` in a call argument list.
    Spread(Box<Expr>),
    /// `name: $value` in a call argument list.
    NamedArg {
        name: String,
        value: Box<Expr>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Concat,
    Shl,
    Shr,
    BitAnd,
    BitOr,
    BitXor,
    Eq,
    NotEq,
    Identical,
    NotIdentical,
    Lt,
    Le,
    Gt,
    Ge,
    Spaceship,
    LogicalAnd,
    LogicalOr,
    LogicalXor,
    Coalesce,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Pow => "**",
            BinOp::Concat => ".",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Eq => "==",
            BinOp::NotEq => "!=",
            BinOp::Identical => "===",
            BinOp::NotIdentical => "!==",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Spaceship => "<=>",
            BinOp::LogicalAnd => "&&",
            BinOp::LogicalOr => "||",
            BinOp::LogicalXor => "xor",
            BinOp::Coalesce => "??",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Pos,
    Not,
    BitNot,
}

impl UnOp {
    pub fn symbol(self) -> &'static str {
        match self {
            UnOp::Neg => "-",
            UnOp::Pos => "+",
            UnOp::Not => "!",
            UnOp::BitNot => "~",
        }
    }
}
