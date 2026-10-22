# PHPCraft

PHPCraft is a PHP 8.5-compatible interpreter written in Rust. It is a
from-scratch reimplementation of the PHP execution pipeline — lexer, parser,
compiler and bytecode VM — validated differentially against the reference `php`
binary and the `php-src` test suite.

This repository currently implements an end-to-end vertical slice: real PHP
scripts using scalar values, strings, ordered arrays, control flow, functions,
`global`, `isset`/`empty`, and a growing standard library execute correctly.

## Building and testing

```sh
make            # debug build
make check      # fmt + clippy + unit tests + differential tests
make run FILE=tests/php/01_basics.php
make repl CODE='echo "hello ", 1 + 2, PHP_EOL;'
make help       # every target
```

Or use Cargo directly:

```sh
cargo build
./target/debug/phpcraft tests/php/01_basics.php
./target/debug/phpcraft -r 'echo "hello ", 1 + 2, PHP_EOL;'
```

Options:

```
-r <code>     Run PHP code without a file
-l            Syntax-check only
-v            Print version information
-             Read the script from stdin
```

## Testing

Unit tests:

```sh
cargo test --workspace
```

Differential test against the installed `php` binary:

```sh
scripts/diff-test.sh
```

The corpus lives in `tests/php/`. Each script is run with both interpreters and
stdout plus exit status must match exactly.

To use php-src tests as a backlog, clone the reference source read-only:

```sh
git clone --depth 1 --branch PHP-8.5 https://github.com/php/php-src vendor/php-src
scripts/phpt-subset.sh --limit 200
scripts/diff-test.sh tests/phpt
```

`phpt-subset.sh` extracts self-contained `--FILE--` sections and skips tests
that require extensions, INI settings, request data or special headers.

## Workspace layout

| Crate | Role |
| --- | --- |
| `phpcraft-lexer` | Tokens, tags, inline HTML, strings, heredoc |
| `phpcraft-ast` | AST definitions |
| `phpcraft-parser` | Recursive descent + Pratt expressions |
| `phpcraft-bytecode` | Stack-machine instruction set |
| `phpcraft-compiler` | AST → bytecode |
| `phpcraft-vm` | Execution engine, frames, operators |
| `phpcraft-runtime` | Values, arrays, coercions, comparisons |
| `phpcraft-stdlib` | Builtin functions |
| `phpcraft-cli` | `phpcraft` executable |

See [`docs/architecture.md`](docs/architecture.md) for the design and
[`docs/php-src-map.md`](docs/php-src-map.md) for the php-src correspondence.
Release highlights live in [`CHANGELOG.md`](CHANGELOG.md).

## Status and limits

Implemented: scalar types, binary-safe strings and interpolation, heredoc and
nowdoc, ordered arrays with PHP key coercion and copy-on-write, arithmetic and
comparison semantics, `if`/`while`/`do`/`for`/`foreach`/`switch`, `match`
expressions, functions, closures and arrow functions, dynamic calls, `global`,
`isset`/`empty`, casts, string/array/math/type builtins, and CLI behaviour
matching `php` (stdout/stderr separation, exit status 255 on fatal errors).

Not yet implemented: classes, objects, namespaces, exceptions, references,
by-reference closure captures, first-class callable syntax, `include`/`require`,
named arguments and alternative control syntax. Unsupported constructs produce
explicit parse or compile errors rather than silently misbehaving.
