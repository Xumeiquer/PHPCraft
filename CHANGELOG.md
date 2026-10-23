# Changelog

All notable changes to PHPCraft are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project aims to follow [Semantic Versioning](https://semver.org/).
The reference behaviour is PHP 8.5.4.

## [Unreleased]

### Added

- Classes and objects: constructors, properties (including typed and static
  declarations), methods, inheritance, `$this`, `parent::`, `self::`,
  `static::`, class constants, `new`, `clone`, `instanceof`, object identity
  and PHP-style `var_dump()`/`print_r()` output with visibility metadata.
- Exceptions: `throw` expressions, `try`/`catch`/`finally` with union catch
  types, user exception subclasses, the built-in `Exception`/`Error` hierarchy
  (including `TypeError`, `ValueError`, `RuntimeException` and
  `LogicException`), exception accessors and PHP-like uncaught diagnostics.
- Runtime errors raised by builtins are catchable as their corresponding
  `Error` subclasses.
- Visibility is enforced for instance and static properties and for methods:
  `private` members are only visible to the declaring class, `protected`
  members work across the inheritance chain, and access failures raise the
  same `Error` messages as PHP.
- `__toString()` is honoured by string-typed builtins and by untyped string
  contexts such as `strval()`, `implode()`, `str_replace()` and `sprintf("%s")`.
- `finally` now runs on every exit path, including `return` from a `try` or
  `catch`, exceptions thrown inside a `catch`, and uncaught exceptions leaving
  a `try` block.
- Namespaces: `namespace Name;`, braced namespace blocks (including the global
  `namespace { ... }` form), `use` imports for classes, functions and
  constants, aliases, group `use` declarations, qualified and fully-qualified
  names, and the `namespace\Name` relative syntax. Unqualified function and
  constant lookups fall back to the global namespace as in PHP.
- Magic constants `__FILE__`, `__DIR__`, `__NAMESPACE__`, `__CLASS__`,
  `__METHOD__`, `__FUNCTION__` and `__LINE__`.
- `include`, `include_once`, `require` and `require_once`, including return
  values, shared globals, included definitions, once-per-file tracking and
  PHP's `true` result for an already-included file. Includes honour
  `include_path`; `get_include_path()`, `set_include_path()` and
  `PATH_SEPARATOR` are available.
- References: `$a = &$b`, by-reference parameters for functions, methods and
  closures, by-reference variadics, by-reference closure captures,
  write-through assignment, references to array elements and object/static
  properties, by-reference `foreach`, by-reference returns, `unset()`, and the
  by-reference array builtins `sort`, `rsort`, `asort`, `arsort`, `ksort`,
  `krsort`, `usort`, `uasort`, `uksort`, `shuffle`, `array_multisort`,
  `array_push`, `array_pop`, `array_shift` and `array_unshift`.
  `sort()` and `array_multisort()` flags include regular, numeric, string,
  locale-string and case-insensitive string comparison. Comparator-based
  sorts are stable, including ties in `array_multisort()`.
- Destructuring: `list(...)` and `[...]` assignment targets, skipped entries,
  nested and keyed targets, by-reference slots, and destructuring in
  `foreach` (including keyed and nested targets).
- References rooted in object and static-property dimensions, such as
  `&$object->items[0]` and `&Class::$items[0]`, including dimension
  assignment, `unset()` and destructuring targets.
- First-class callables: `strlen(...)`, `$object->method(...)`,
  `Class::method(...)` and `$callable(...)`, including bound `$this`, static
  scope, namespaced function resolution and PHP-compatible `var_dump()`/
  `print_r()` metadata (function name, bound object and reflected parameters).
- Callable arrays (`[$object, "method"]`, `["Class", "method"]`), callable
  strings (including `Class::method`) and objects with `__invoke()` work with
  direct invocation, `is_callable()`, `call_user_func()`,
  `call_user_func_array()` and `array_map()`.
- Differential coverage for classes, inheritance, static members, object
  identity, cloning, exception handling, namespaces, includes and references.
- Closures and arrow functions: `function (...) use (...)` captures, implicit
  arrow-function captures, default/variadic parameters, nested closures and
  by-value capture semantics.
- Dynamic calls (`$fn(...)`, `(fn() => ...)()`) and callable strings, including
  closure callbacks for `array_map()`, `call_user_func()` and
  `call_user_func_array()`.
- `is_object()` and `get_class()` recognize closures, and `var_dump()`/`print_r()`
  expose their source location, captures and parameter metadata.
- `match` expressions: multiple conditions per arm, strict (`===`)
  comparison, a `default` arm in any position, trailing commas, nested
  expressions and `UnhandledMatchError` when no arm matches.
- Differential coverage for `match` and integer-boundary builtin calls.
- 37 standard builtins:
  - strings: `strstr`, `strrchr`, `strpbrk`, `substr_count`, `ucwords`,
    `nl2br`
  - arrays: `array_key_first`, `array_key_last`, `array_is_list`,
    `array_unique`, `array_diff`, `array_intersect`, `array_diff_key`,
    `array_intersect_key`, `array_chunk`, `array_pad`
  - math: `log`, `log10`, `exp`, `sin`, `cos`, `tan`, `asin`, `acos`,
    `atan`, `atan2`, `hypot`, `deg2rad`, `rad2deg`, `fdiv`, `is_nan`,
    `is_finite`, `is_infinite`
  - URL: `urlencode`, `urldecode`, `rawurlencode`, `rawurldecode`
- Differential coverage for the new stdlib batch.
- Differential coverage for closures, arrow functions, dynamic calls and
  closure callbacks.
- Differential coverage for comparator sorts, callable forms, `shuffle()` and
  `array_multisort()`.

### Fixed

- Integer-boundary arithmetic in `explode()`, `substr()`, `array_slice()`,
  `array_fill()` and `range()` no longer wraps or overflows.
- `range()` uses exact `i128` arithmetic for integer inputs. Ranges ending at
  `PHP_INT_MAX` previously added `1.0` to an `f64` that could not change and
  looped forever.
- `range()` now rejects steps outside the spanned interval and oversized
  results instead of allocating until failure.
- `number_format()` uses `_php_math_round()` semantics (`number_format(1.005,
  2)` is now `1.01`) and can emit exact fractional expansions beyond Rust's
  formatter precision.
- `sprintf()` clamps floating-point precision to PHP's 53-digit maximum, as
  `formatted_print.c` does.
- Assignments to static properties now resolve namespaced class names the same
  way reads and method calls do, so `Counter::$value = ...` inside a namespace
  no longer writes to an unrelated class entry.

### Security

- Hardened builtins against panics and allocator aborts caused by extreme
  integer arguments: `str_repeat()`, `str_pad()`, `explode()`,
  `array_fill()`, `number_format()` and `sprintf()`.
- `sprintf()` validates width/precision against PHP's `INT_MAX` limit and all
  padding goes through fallible allocation.
- Rust integer overflow and capacity-overflow paths found by a boundary audit
  now produce PHP fatal errors (status 255) instead of process aborts. A
  single user-controlled allocation is capped at 64 MiB as a stop-gap until
  an explicit `memory_limit` implementation exists.

### Tested

- 59 Rust unit/integration tests.
- 26 differential scripts, all byte-identical to PHP 8.5.4 stdout and exit
  status.

### Planned

- Class autoloading and namespaced constant declarations.
- `break`/`continue` leaving a `try` block currently bypasses its `finally`
  body (return and throw paths are covered).

## [0.1.0] - 2026-10-22

The first end-to-end vertical slice: source text is lexed, parsed, compiled to
stack-machine bytecode and executed with PHP-compatible observable behaviour.

### Featured

- **A real PHP pipeline in Rust.** Every stage is a separate crate:
  `phpcraft-lexer`, `phpcraft-parser`, `phpcraft-compiler`, `phpcraft-bytecode`,
  `phpcraft-vm`, `phpcraft-runtime`, `phpcraft-stdlib` and `phpcraft-cli`.
- **Differential correctness.** Ten scenario scripts are executed with both the
  installed PHP 8.5.4 binary and PHPCraft; stdout and exit status must match
  byte for byte. `scripts/phpt-subset.sh` turns php-src tests into a runnable
  backlog.
- **PHP value semantics, not approximations.** Ordered arrays with PHP key
  coercion, copy-on-write values, integer overflow to float, loose vs. strict
  comparison, binary-safe strings, float formatting with PHP's precision
  thresholds and `_php_math_round()` edge-case handling.
- **CLI parity.** `phpcraft file.php`, `-r`, `-l`, `-v`, stdin `-`,
  `$argv`/`$argc`, stdout/stderr separation and exit status 255 on fatal
  errors.

### Added

#### Language

- `<?php`, `<?=`, `?>` tags, inline HTML and close-tag newline handling.
- Numeric literals: decimal, binary, octal, hexadecimal, underscores, floats
  and exponents.
- Single-quoted, double-quoted and heredoc/nowdoc strings, including `$var`,
  `$var[key]`, `{$expr}` interpolation and flexible indentation stripping.
- Scalars, ordered arrays, array literals and nested array access.
- Arithmetic, string concatenation, bitwise, comparison, logical and spaceship
  operators with the PHP 8 precedence table (including `.` below `+`/`-`).
- Assignments, compound assignments (`+=`, `??=`, ...), pre/post increment and
  decrement, including Perl-style string increment.
- Casts: `(int)`, `(integer)`, `(float)`, `(double)`, `(real)`, `(string)`,
  `(binary)`, `(bool)`, `(boolean)` and `(array)`.
- `if`/`elseif`/`else`, `while`, `do/while`, `for`, `foreach` (key and value),
  `switch` with fallthrough, `break` and `continue`.
- Null coalescing (`??`), ternary and short ternary, `isset()` and `empty()`.
- Functions with parameters, constant defaults, recursion and `global`.
- Runtime constants: `define()`, `defined()`, `constant()` and the common
  `PHP_*`, `M_*`, `E_*`, `INF`, `NAN` and `STR_PAD_*` constants.

#### Standard library

- Strings: `strlen`, `str_repeat`, `strtoupper`, `strtolower`, `ucfirst`,
  `lcfirst`, `trim`/`ltrim`/`rtrim`, `substr`, `strpos`/`strrpos`,
  `str_contains`, `str_starts_with`, `str_ends_with`, `str_replace`,
  `implode`/`join`, `explode`, `sprintf`, `printf`, `number_format`,
  `str_pad`, `strrev`, `strcmp`, `chr`, `ord`.
- Arrays: `count`/`sizeof`, `array_keys`, `array_values`, `array_key_exists`,
  `in_array`, `array_search`, `array_sum`, `array_product`, `array_reverse`,
  `array_slice`, `array_merge`, `array_fill`, `array_combine`, `array_flip`,
  `array_map`, `range`.
- Math: `abs`, `ceil`, `floor`, `round`, `sqrt`, `pow`, `intdiv`, `fmod`,
  `max`, `min`, `pi`.
- Types and output: `gettype`, `get_debug_type`, `intval`, `floatval`,
  `strval`, `boolval`, `is_*`, `is_numeric`, `is_scalar`, `is_callable`,
  `function_exists`, `var_dump`, `print_r`.

#### Tooling

- Cargo workspace with per-subsystem crates and shared dependencies.
- `Makefile` with `build`, `release`, `test`, `diff`, `phpt`, `fmt`,
  `fmt-check`, `clippy`, `check`, `run`, `repl`, `clean` and `help` targets.
- `scripts/diff-test.sh`: recursive differential runner for PHP scripts.
- `scripts/phpt-subset.sh`: extracts self-contained `--FILE--` sections from a
  read-only php-src checkout and skips tests requiring extensions, INI,
  request data or headers.
- Architecture and php-src correspondence documentation in `docs/`.

### Fixed

- `phpcraft -r` parses code without an opening tag and exposes script
  arguments correctly (`$argc`, `$argv[0] == "Standard input code"`).
- Heredoc/nowdoc closing labels no longer swallow the following `;`.
- A newline immediately after `?>` is consumed, as PHP does.
- `explode()` with a positive limit joins the remainder with the separator.
- `array_sum()` accounts for integers accumulated before the first float.
- `array_slice()`, `array_fill()` and append-after-negative-index semantics.
- `round()` reproduces PHP's floating-point edge cases.
- String offsets (`$s[0]`, `$s[-1]`) and `strrpos()` offsets.
- Increment/decrement evaluate their target once and handle strings, `null`,
  booleans and numeric strings like PHP.

### Security

- No known security vulnerabilities in this release.
- PHPCraft executes PHP code by design and does not provide a sandbox, memory
  limit or permission boundary. Do not run untrusted scripts with it.
- Diagnostics go to stderr and fatal errors return status 255, matching the PHP
  CLI, so script output cannot silently mask interpreter failures.

### Tested

- 34 Rust unit/integration tests.
- 10 differential scenario scripts, all byte-identical to PHP 8.5.4 stdout and
  exit status.

[Unreleased]: https://github.com/Xumeiquer/PHPCraft/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Xumeiquer/PHPCraft/releases/tag/v0.1.0
