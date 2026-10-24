# PHPCraft

PHPCraft is a PHP 8.5-compatible interpreter written in Rust. It is a
from-scratch reimplementation of the PHP execution pipeline — lexer, parser,
compiler and bytecode VM — validated differentially against the reference `php`
binary and the `php-src` test suite.

This repository implements an end-to-end vertical slice of PHP 8.5: real
scripts using scalar values, strings, ordered arrays, control flow, functions,
closures, namespaces, classes, interfaces, traits, enums, references,
exceptions, generators (eagerly evaluated), `global`, `isset`/`empty`, and a
growing standard library execute correctly.

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
nowdoc, ordered arrays with PHP key coercion and copy-on-write (including
by-reference array elements such as `[&$value]` and array spread), arithmetic
and comparison semantics, `if`/`while`/`do`/`for`/`foreach`/`switch` and
`match` expressions, alternative control syntax (`if: ... endif;`, `for:
... endfor;`, `foreach: ... endforeach;`, `switch: ... endswitch;`), named and
unpacked arguments, `print`, `exit`/`die`, `goto` and labels, `declare`,
top-level and namespaced `const`, attributes (accepted and ignored), functions,
closures and arrow functions, first-class callables (including dynamic
`$object->$method(...)` and `$class::$method(...)` forms), dynamic calls,
classes and objects (inheritance, interfaces, traits with `insteadof`/`as`
adaptations, enums with backed-case validation and `cases()`/`from()`/
`tryFrom()`, abstract classes, constructor property promotion, anonymous
classes, static members, static locals, cloning and object identity),
dynamic class, property, method and class-constant access, variable variables
(`$$name`, `${expr}`), exceptions with `try`/`catch`/`finally` (including
finally on `return`, rethrow and `break`/`continue` paths), private/protected
visibility enforcement, `__toString()` coercion, namespaces and `use` imports
(including group imports), magic constants, `include`/`require` forms with
`include_path` support, autoloading, references (variables, array elements,
object properties, static properties, object/static-property dimensions,
by-reference `foreach` and returns, by-reference variadics), by-reference
parameters and closure captures, destructuring with `list()` and `[...]`
(including nested, keyed, skipped and by-reference targets, and `foreach`
targets), `unset`, closure captures and array builtins (including by-reference
`usort`/`uasort`/`uksort`, `shuffle` and `array_multisort`), callable arrays
and callable strings (including `Class::method`), `is_callable()` and
callbacks through `call_user_func()`/`array_map()`, `global`, `isset`/`empty`,
casts, string/array/math/type builtins, and CLI behaviour matching `php`
(stdout/stderr separation, exit status 255 on fatal errors).

Generators support the common `yield`, keyed `yield`, and `yield from` forms
by evaluating eagerly into an iterable array. A full resumable `Generator`
object (`current()`/`send()`/`getReturn()` and lazy execution) is not
implemented. Other deliberate limits: attributes are skipped rather than
exposed through reflection, `final`/`readonly` modifiers are parsed but their
constraints are not enforced, property hooks and asymmetric visibility are not
implemented, and `Closure::fromCallable()`/`Closure::bind()` are unavailable.
`sort()` and `array_multisort()` support `SORT_REGULAR`, `SORT_NUMERIC`,
`SORT_STRING`, `SORT_LOCALE_STRING` and `SORT_FLAG_CASE`; `SORT_NATURAL` and
unknown flags raise explicit errors. Unsupported constructs produce explicit
parse or compile errors rather than silently misbehaving.
