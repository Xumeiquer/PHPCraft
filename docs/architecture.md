# PHPCraft architecture

PHPCraft is a PHP 8.5-compatible interpreter written in Rust. It targets the
subset of the language exercised by the tests in `tests/php`, and it grows by
differential testing against the reference `php` binary.

The implementation follows PHP's own pipeline: source bytes are lexed, parsed
into an AST, lowered to bytecode, and executed by a virtual machine. The main
deliberate difference is the instruction format: Zend uses a register machine,
while PHPCraft uses a stack machine, which keeps the compiler and VM small and
easy to audit.

```
source bytes
    │
    ▼
phpcraft-lexer      tokens (tags, strings, interpolation, heredoc)
    │
    ▼
phpcraft-parser     AST (recursive descent + Pratt expressions)
    │
    ▼
phpcraft-compiler   bytecode (stack machine, jumps for control flow)
    │
    ▼
phpcraft-vm         execution, frames, iterators, globals, builtins
    │
    ├── phpcraft-runtime   values, arrays, coercions, comparisons, errors
    └── phpcraft-stdlib    native functions written in Rust
```

## Crates

| Crate | Responsibility | Zend counterpart |
| --- | --- | --- |
| `phpcraft-lexer` | Byte-level scanner. Handles `<?php`/`?>`/`<?=`, inline HTML, comments, numeric literals, quoted strings, interpolation, heredoc/nowdoc indentation. | `Zend/zend_language_scanner.l` |
| `phpcraft-ast` | AST node definitions (`Program`, `Stmt`, `Expr`, operators). | `Zend/zend_ast.h`, `Zend/zend_compile.h` |
| `phpcraft-parser` | Recursive-descent statements and Pratt expressions using the PHP 8 precedence table. Produces explicit errors for unsupported constructs. | `Zend/zend_language_parser.y` |
| `phpcraft-bytecode` | Stack-machine `Op` set, `Chunk` constant pool, `Function` layout, `CompiledProgram`. | `Zend/zend_opcode.c`, `zend_vm_opcodes.h` |
| `phpcraft-compiler` | AST → bytecode lowering, including short-circuiting, ternary, `??`, loops, `switch`, `foreach`, `global`, function hoisting. | `Zend/zend_compile.c` |
| `phpcraft-vm` | Dispatch loop, call frames, variable scopes, class registry, method dispatch, exception handlers, iterators, operators, fatal/warning reporting. | `Zend/zend_vm_def.h`, `Zend/zend_execute.c` |
| `phpcraft-runtime` | `Value`, `PhpStr`, `PhpObject`, ordered copy-on-write `PhpArray`, `Key`, numeric parsing, PHP coercions and comparisons, float formatting, `PhpError`. | `Zend/zend_types.h`, `Zend/zend_hash.c`, `Zend/zend_operators.c`, `Zend/zend_exceptions.c` |
| `phpcraft-stdlib` | Builtin functions. Each entry corresponds to a `PHP_FUNCTION` in php-src. | `ext/standard/*.c`, `Zend/zend_builtin_functions.c` |
| `phpcraft-cli` | `phpcraft` binary: file execution, `-r`, `-l`, `-v`, stdin, `$argv`/`$argc`, stderr diagnostics. | `sapi/cli/php_cli.c` |

## Value model

`Value` mirrors a zval without reference counting exposed to users:

```rust
enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Rc<PhpStr>),
    Array(Rc<PhpArray>),
    Closure(Rc<PhpClosure>),
    Object(Rc<RefCell<PhpObject>>),
    Ref(Rc<RefCell<Value>>),
}
```

* PHP integers are `i64`, matching 64-bit builds. Overflowing arithmetic
  produces float, as in PHP.
* Strings are byte strings, not UTF-8, because PHP strings are binary safe.
* Arrays are ordered hash maps. Integer-like string keys are canonicalised
  (`"8"` becomes `8`), but only when the string is a canonical decimal integer;
  `"08"`, `"+8"`, `" 8"`, `"8 "` and `"-0"` remain strings.
* Arrays use `Rc` plus `Rc::make_mut` for copy-on-write, so assignment copies
  lazily while preserving PHP's value semantics.
* Objects use `Rc<RefCell<PhpObject>>`: assignment and argument passing share
  the same object, matching PHP's handle semantics. Property storage is an
  ordered `PhpArray`; declared visibility is kept beside it for `var_dump()`
  and `print_r()`.
* References are `Value::Ref(Rc<RefCell<Value>>)`. Reads dereference the cell,
  ordinary stores write through it, and `$a = &$b` rebinds both variables to
  the same cell. Array elements and object/static properties can also hold
  reference cells; assignment, nested writes and `unset()` update the shared
  cell rather than replacing it. Function and method calls evaluate simple
  variable arguments as reference candidates, and compile-time signatures
  promote array-element/property arguments for by-reference parameters;
  by-value parameters copy the dereferenced value and by-reference parameters
  bind the cell itself. Native calls preserve cells only for the argument
  positions listed in `phpcraft_stdlib::by_ref_params`. By-reference `foreach`
  converts the subject’s elements/properties with `PrepareRefs`, and returns
  preserve the cell only when the function carries `returns_ref`.

Classes are compiled into `ClassDef` metadata (properties, constants and
method entry points) next to the function table. The VM resolves inherited
properties and methods through the parent chain. User exceptions are ordinary
objects carrying an `ExceptionData` payload; `throw` transports the object in
`PhpError::thrown`, and `try` handlers installed as bytecode opcodes unwind the
current frame to the matching `catch` clause.

Namespace and `use` state is tracked by the compiler. Declarations are stored
under qualified names, class names are rewritten at compile time, and
unqualified function calls carry a fallback flag so the VM can try the global
function when the namespaced one does not exist. `__FILE__` and `__DIR__` are
folded from the source path; included files are parsed and compiled on demand
by the VM, executed in the shared global scope and merged into the function and
class registries. A canonical-path set implements the `*_once` forms.

The runtime also owns observable formatting rules: `echo` uses `precision=14`
with scientific notation from `1e14`, `var_dump` uses the shortest round-trip
representation, and `(int)` conversion clamps out-of-range floats.

## Variables and frames

Top-level variables live in the VM's `globals` map rather than a frame. This
makes `global $x` inside a function share the top-level storage. Userland
function calls create a frame with fixed slots; `slot_names` in `Function`
supports diagnostics. Temporary slots are used by the compiler for arrays,
`foreach` iterators and short-circuit evaluation.

Anonymous functions compile to ordinary `Function` bodies with synthetic
names. `MakeClosure` captures values into a `PhpClosure`; the body's
`capture_slots` bind those values when the closure is invoked. Arrow functions
collect free variables at compile time and use the same runtime path. Dynamic
calls use `CallValue`, which dispatches closures and callable strings.

## Control flow

The compiler lowers structured control flow to jumps:

* `if`/`elseif`/`else` → `JumpIfFalse`.
* `while`, `do/while`, `for` → conditional and unconditional jumps.
* `&&`, `||`, `??` → `JumpIfFalseKeep` / `JumpIfTrueKeep` / `JumpIfNotNull`,
  preserving the operand value like PHP.
* `foreach` → `IterInit` plus `IterNext`, which owns a snapshot iterator.
* `switch` → compare-and-jump chains with fallthrough and `break` targets.
* `break`/`continue` → patched jumps with optional levels.

## Errors and diagnostics

`PhpError` carries an `ErrorKind` (`Fatal`, `TypeError`, `ValueError`,
`ArithmeticError`, `ArgumentCountError`, `Thrown`). A thrown userland exception
keeps its object in `PhpError::thrown`; when an active handler matches it the
VM binds it to the catch variable, otherwise the CLI formats it like PHP's
uncaught-exception banner. The CLI writes warnings and fatals to stderr and
script output to stdout, matching the CLI SAPI. Fatals exit with status 255.
Parse errors are reported before any code runs.

Unsupported syntax fails at parse or compile time with a message naming the
feature. This is intentional: silent misbehaviour is worse than an explicit
boundary.

## Testing

* `cargo test --workspace` covers each crate's unit-level behaviour.
* `scripts/diff-test.sh` runs `tests/php/*.php` with both `php` and
  `target/debug/phpcraft`, comparing stdout and exit status.
* `scripts/phpt-subset.sh` extracts self-contained `--FILE--` sections from a
  read-only `vendor/php-src` checkout so real PHP tests can be used as a
  backlog. Tests with `--SKIPIF--`, `--EXTENSIONS--`, `--INI--`, request data or
  headers are skipped.

The reference interpreter is the installed `php` binary (8.5.4 at the time of
writing), and `vendor/php-src` tracks branch `PHP-8.5`.

## Current limits

Deliberately unimplemented (they produce explicit errors): attributes,
interfaces, traits, enums, references rooted inside object/static-property
dimensions, reference destructuring in `list()`, first-class callable syntax,
named arguments and alternative control syntax. The implemented
by-reference builtins are `sort`, `rsort`, `asort`, `arsort`, `ksort`,
`krsort`, `array_push`, `array_pop`, `array_shift` and `array_unshift`.
`sort()` flags cover `SORT_REGULAR`, `SORT_NUMERIC`, `SORT_STRING`,
`SORT_LOCALE_STRING` and `SORT_FLAG_CASE`; `SORT_NATURAL` and unknown flags
raise explicit errors. Namespace support covers ordinary and braced declarations,
per-kind `use` imports and aliases; group `use`, namespaced `const`
declarations and autoloading are not implemented. Includes search the calling
script's directory and then the current directory; `include_path` is ignored
and an include written inside a function still executes in the global scope.
Visibility is parsed and reported by `var_dump()`/`print_r()`, but
private/protected access is not enforced yet. `return` and `throw` that escape
a `catch`/`finally` block can still skip the `finally` body. Opcodes carry no
source line information yet, so exceptions and runtime diagnostics use line 1.
