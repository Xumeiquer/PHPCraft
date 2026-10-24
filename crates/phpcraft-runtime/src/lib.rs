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
    loose_eq, strict_eq, CallableInfo, ExceptionData, Numeric, PhpClosure, PhpObject, PhpStr,
    ToNumber, Value,
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

    /// Call a callable value. The default only supports callable strings so
    /// lightweight test hooks do not need to understand closures.
    fn call_value(&mut self, callable: &Value, args: Vec<Value>) -> PhpResult<Value> {
        match callable {
            Value::Str(name) => self.call(&String::from_utf8_lossy(name.as_bytes()), args),
            other => Err(PhpError::type_error(format!(
                "Value of type {} is not callable",
                other.debug_type_name()
            ))),
        }
    }

    /// Whether a function (builtin or userland) exists.
    fn function_exists(&mut self, name: &str) -> bool;

    /// Register an `spl_autoload_register()` callback. Returns `false` when
    /// the callback was already registered.
    fn register_autoloader(&mut self, _callable: Value, _prepend: bool) -> PhpResult<bool> {
        Ok(false)
    }

    /// Remove a previously registered autoloader.
    fn unregister_autoloader(&mut self, _callable: &Value) -> bool {
        false
    }

    /// The currently registered autoload callbacks, in call order.
    fn autoload_functions(&mut self) -> Vec<Value> {
        Vec::new()
    }

    /// Invoke the autoload stack for `class`.
    fn autoload_call(&mut self, _class: &str) -> PhpResult<()> {
        Ok(())
    }

    /// `class_exists()`/`interface_exists()`/`trait_exists()`/`enum_exists()`.
    fn class_exists(&mut self, _class: &str, _autoload: bool) -> PhpResult<bool> {
        Ok(false)
    }

    /// Like [`RuntimeHooks::class_exists`], but only returns true when the
    /// declaration is of the requested kind (`class`, `interface`, `trait` or
    /// `enum`).
    fn type_exists(&mut self, class: &str, autoload: bool, _kind: &str) -> PhpResult<bool> {
        self.class_exists(class, autoload)
    }

    /// Whether a value can be invoked. The default implementation covers the
    /// subset lightweight test hooks understand; the VM also recognizes
    /// callable arrays, `Class::method` strings and objects with `__invoke`.
    fn is_callable(&mut self, value: &Value) -> bool {
        match value {
            Value::Closure(_) => true,
            Value::Str(name) => self.function_exists(&String::from_utf8_lossy(name.as_bytes())),
            _ => false,
        }
    }

    /// Look up a runtime constant (`PHP_EOL`, user `define()`d values, ...).
    fn constant(&mut self, name: &str) -> Option<Value>;

    /// Define a runtime constant (`define()`).
    fn define_constant(&mut self, name: &str, value: Value);

    /// Current `include_path`, using the platform path separator.
    fn get_include_path(&mut self) -> String {
        String::new()
    }

    /// Replace `include_path` and return the previous value.
    fn set_include_path(&mut self, path: String) -> String {
        path
    }

    /// Convert a value for a `string` parameter or other string context.
    /// Returns `None` when the value cannot be converted (for example an
    /// object without `__toString`, or an array); callers then report the
    /// usual type error for their parameter.
    fn object_to_string(&mut self, _value: &Value) -> Option<PhpResult<Vec<u8>>> {
        None
    }
}
