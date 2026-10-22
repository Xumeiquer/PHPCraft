# Changelog

All notable changes to PHPCraft are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project aims to follow [Semantic Versioning](https://semver.org/).
The reference behaviour is PHP 8.5.4.

## [Unreleased]

### Planned

- References and by-reference builtins (`sort`, `array_push`, `unset`,
  `foreach &$value`).
- Closures, arrow functions and first-class callables.
- Classes, objects, property access and exceptions.
- `include`/`require` and namespaces.

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
