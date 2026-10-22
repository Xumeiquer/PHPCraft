//! Error types shared by the whole engine.
//!
//! Mirrors the Zend error classes that abort execution (`zend_error` with
//! `E_ERROR`/`E_TYPE_ERROR`/...) rather than the recoverable warning family,
//! which is reported through [`RuntimeHooks::warn`](crate::RuntimeHooks::warn).

use std::fmt;

use crate::Value;

/// Category of a fatal PHP error. The name is used verbatim in the CLI banner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Fatal,
    TypeError,
    ValueError,
    ArgumentCountError,
    ArithmeticError,
    ParseError,
    UnhandledMatch,
    /// A userland `throw` (the value is carried in `PhpError::thrown`).
    Thrown,
}

impl ErrorKind {
    pub fn label(self) -> &'static str {
        match self {
            ErrorKind::Fatal => "Fatal error",
            ErrorKind::TypeError => "TypeError",
            ErrorKind::ValueError => "ValueError",
            ErrorKind::ArgumentCountError => "ArgumentCountError",
            ErrorKind::ArithmeticError => "ArithmeticError",
            ErrorKind::ParseError => "Parse error",
            ErrorKind::UnhandledMatch => "UnhandledMatchError",
            ErrorKind::Thrown => "Exception",
        }
    }
}

/// A fatal PHP error: message plus the class it is reported under.
#[derive(Debug, Clone, PartialEq)]
pub struct PhpError {
    pub kind: ErrorKind,
    pub message: String,
    /// Present for `ErrorKind::Thrown`: the thrown object.
    pub thrown: Option<Value>,
}

impl PhpError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            thrown: None,
        }
    }

    pub fn thrown(value: Value, message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Thrown,
            message: message.into(),
            thrown: Some(value),
        }
    }

    pub fn fatal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Fatal, message)
    }

    pub fn type_error(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::TypeError, message)
    }

    pub fn value_error(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::ValueError, message)
    }

    pub fn argument_count(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::ArgumentCountError, message)
    }

    pub fn arithmetic(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::ArithmeticError, message)
    }

    pub fn parse(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::ParseError, message)
    }
}

impl fmt::Display for PhpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.label(), self.message)
    }
}

impl std::error::Error for PhpError {}

pub type PhpResult<T> = Result<T, PhpError>;
