//! `phpcraft-runtime`: the PHP value model shared by the compiler, VM and
//! standard library.
//!
//! This crate corresponds to the parts of Zend that describe data rather than
//! execute code: `zend_types.h` (zval), `zend_string.h`, `zend_hash.h` and the
//! conversion helpers in `zend_operators.c`.

pub mod array;
pub mod error;
pub mod value;

pub use array::{Key, PhpArray};
pub use error::{ErrorKind, PhpError, PhpResult};
pub use value::{
    compare, float_to_fixed, float_to_general, float_to_repr, float_to_string, key_from_value,
    loose_eq, strict_eq, Numeric, PhpStr, ToNumber, Value,
};

/// Services the standard library needs from the engine while a script runs.
///
/// Implemented by the VM. Keeping this as a trait lets `phpcraft-stdlib` stay
/// independent of `phpcraft-vm` (no dependency cycle).
pub trait RuntimeHooks {
    /// Append bytes to the script's output stream (`echo`, `printf`, ...).
    fn write(&mut self, bytes: &[u8]);

    /// Emit an `E_WARNING`. Warnings do not abort execution.
    fn warn(&mut self, message: &str);

    /// Call a userland function by name (for the few callbacks we support).
    fn call(&mut self, name: &str, args: Vec<Value>) -> PhpResult<Value>;

    /// Whether a function (builtin or userland) exists.
    fn function_exists(&mut self, name: &str) -> bool;

    /// Look up a runtime constant (`PHP_EOL`, user `define()`d values, ...).
    fn constant(&mut self, name: &str) -> Option<Value>;

    /// Define a runtime constant (`define()`).
    fn define_constant(&mut self, name: &str, value: Value);
}
