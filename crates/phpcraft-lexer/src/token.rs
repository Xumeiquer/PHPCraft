//! Token definitions for PHP 8.5.
//!
//! Keywords are not distinct token kinds: like PHP's own lexer they are
//! identifiers, and the parser decides based on context. That keeps `match`,
//! `fn` and future reserved words usable as method names later.

use std::fmt;

/// A piece of an interpolated string (`"a $b c"`).
#[derive(Clone, Debug, PartialEq)]
pub enum StrPart {
    /// Literal bytes.
    Lit(Vec<u8>),
    /// `$name`.
    Var(String),
    /// `$name[key]`, only the "simple" syntax PHP allows inside strings.
    Dim(String, DimKey),
    /// `{$...}`: raw PHP source to be parsed as an expression by the parser.
    Code(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum DimKey {
    Int(i64),
    Str(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: Tok,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Eof,
    /// Text outside `<?php ... ?>`; emitted to output verbatim.
    InlineHtml(Vec<u8>),
    Ident(String),
    Var(String),
    /// A bare `$` starting a variable variable (`$$name`, `${expr}`).
    Dollar,
    Int(i64),
    Float(f64),
    Str(Vec<u8>),
    Interp(Vec<StrPart>),

    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Semi,
    Comma,
    Colon,
    Question,
    At,
    Backslash,
    Dot,
    Arrow,
    DoubleArrow,
    DoubleColon,
    Ellipsis,

    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Pow,
    Inc,
    Dec,
    BitNot,

    Assign,
    PlusEq,
    MinusEq,
    MulEq,
    DivEq,
    ModEq,
    PowEq,
    ConcatEq,
    AndEq,
    OrEq,
    XorEq,
    ShlEq,
    ShrEq,
    CoalesceEq,

    EqEq,
    NotEq,
    Identical,
    NotIdentical,
    Lt,
    Gt,
    Le,
    Ge,
    Spaceship,
    Not,
    AndAnd,
    OrOr,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Coalesce,
}

impl Tok {
    /// Human-readable token name for parse errors, in the style of PHP's own
    /// `unexpected token "..."` messages.
    pub fn describe(&self) -> String {
        match self {
            Tok::Eof => "end of file".to_string(),
            Tok::InlineHtml(_) => "inline HTML".to_string(),
            Tok::Ident(name) => format!("\"{name}\""),
            Tok::Var(name) => format!("\"${name}\""),
            Tok::Dollar => "\"$\"".to_string(),
            Tok::Int(v) => format!("\"{v}\""),
            Tok::Float(v) => format!("\"{v}\""),
            Tok::Str(_) | Tok::Interp(_) => "string literal".to_string(),
            other => format!("\"{}\"", other.symbol()),
        }
    }

    fn symbol(&self) -> &'static str {
        match self {
            Tok::LParen => "(",
            Tok::RParen => ")",
            Tok::LBrace => "{",
            Tok::RBrace => "}",
            Tok::LBracket => "[",
            Tok::RBracket => "]",
            Tok::Semi => ";",
            Tok::Comma => ",",
            Tok::Colon => ":",
            Tok::Question => "?",
            Tok::At => "@",
            Tok::Backslash => "\\",
            Tok::Dot => ".",
            Tok::Arrow => "->",
            Tok::DoubleArrow => "=>",
            Tok::DoubleColon => "::",
            Tok::Ellipsis => "...",
            Tok::Plus => "+",
            Tok::Minus => "-",
            Tok::Star => "*",
            Tok::Slash => "/",
            Tok::Percent => "%",
            Tok::Pow => "**",
            Tok::Inc => "++",
            Tok::Dec => "--",
            Tok::BitNot => "~",
            Tok::Assign => "=",
            Tok::PlusEq => "+=",
            Tok::MinusEq => "-=",
            Tok::MulEq => "*=",
            Tok::DivEq => "/=",
            Tok::ModEq => "%=",
            Tok::PowEq => "**=",
            Tok::ConcatEq => ".=",
            Tok::AndEq => "&=",
            Tok::OrEq => "|=",
            Tok::XorEq => "^=",
            Tok::ShlEq => "<<=",
            Tok::ShrEq => ">>=",
            Tok::CoalesceEq => "??=",
            Tok::EqEq => "==",
            Tok::NotEq => "!=",
            Tok::Identical => "===",
            Tok::NotIdentical => "!==",
            Tok::Lt => "<",
            Tok::Gt => ">",
            Tok::Le => "<=",
            Tok::Ge => ">=",
            Tok::Spaceship => "<=>",
            Tok::Not => "!",
            Tok::AndAnd => "&&",
            Tok::OrOr => "||",
            Tok::BitAnd => "&",
            Tok::BitOr => "|",
            Tok::BitXor => "^",
            Tok::Shl => "<<",
            Tok::Shr => ">>",
            Tok::Coalesce => "??",
            Tok::Eof | Tok::InlineHtml(_) | Tok::Ident(_) | Tok::Var(_) | Tok::Dollar => "",
            Tok::Int(_) | Tok::Float(_) | Tok::Str(_) | Tok::Interp(_) => "",
        }
    }
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.describe())
    }
}
