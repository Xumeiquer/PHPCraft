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
| `phpcraft-vm` | Dispatch loop, call frames, variable scopes, iterators, operators, fatal/warning reporting. | `Zend/zend_vm_def.h`, `Zend/zend_execute.c` |
| `phpcraft-runtime` | `Value`, `PhpStr`, ordered copy-on-write `PhpArray`, `Key`, numeric parsing, PHP coercions and comparisons, float formatting, `PhpError`. | `Zend/zend_types.h`, `Zend/zend_hash.c`, `Zend/zend_operators.c` |
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

The runtime also owns observable formatting rules: `echo` uses `precision=14`
with scientific notation from `1e14`, `var_dump` uses the shortest round-trip
representation, and `(int)` conversion clamps out-of-range floats.

## Variables and frames

Top-level variables live in the VM's `globals` map rather than a frame. This
makes `global $x` inside a function share the top-level storage. Userland
function calls create a frame with fixed slots; `slot_names` in `Function`
supports diagnostics. Temporary slots are used by the compiler for arrays,
`foreach` iterators and short-circuit evaluation.

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
`ArithmeticError`, `ArgumentCountError`). The CLI writes warnings and fatals to
stderr and script output to stdout, matching the CLI SAPI. Fatals exit with
status 255. Parse errors are reported before any code runs.

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

Deliberately unimplemented (they produce explicit errors): classes, objects,
closures/arrow functions, namespaces, attributes, exceptions, references,
`include`/`require`, `new`, property/method access, static access, dynamic
calls, named arguments and alternative control syntax. By-reference
builtins such as `sort()` and `array_push()` are also absent until references
exist. Opcodes carry no source line information yet, so runtime diagnostics
omit the line number.
