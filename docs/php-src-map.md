# php-src → PHPCraft map

This document maps the parts of php-src (branch `PHP-8.5`) that PHPCraft
reimplements. Keep `vendor/php-src` read-only; it is a reference checkout and is
gitignored.

## Source files

| php-src | PHPCraft | Notes |
| --- | --- | --- |
| `Zend/zend_language_scanner.l` | `crates/phpcraft-lexer/src/lib.rs` | Tags, inline HTML, comments, numbers, strings, interpolation, heredoc/nowdoc. |
| `Zend/zend_language_scanner.h` | `crates/phpcraft-lexer/src/token.rs` | Token kinds and interpolation parts. |
| `Zend/zend_language_parser.y` | `crates/phpcraft-parser/src/lib.rs` | Statements are recursive descent; expressions use Pratt parsing with the PHP 8 precedence table. |
| `Zend/zend_ast.h`, `Zend/zend_ast.c` | `crates/phpcraft-ast/src/lib.rs` | Simplified AST; nodes only exist for implemented constructs. |
| `Zend/zend_compile.c`, `Zend/zend_compile.h` | `crates/phpcraft-compiler/src/lib.rs` | AST lowering, jump patching, function hoisting, namespace/import resolution, magic constants, constant folding for parameter defaults. |
| `Zend/zend_opcode.c`, `Zend/zend_vm_opcodes.h` | `crates/phpcraft-bytecode/src/lib.rs` | `Op`, `Chunk`, `Function`, `CompiledProgram`. Stack machine instead of Zend's register slots. |
| `Zend/zend_vm_def.h`, `Zend/zend_vm_execute.h` | `crates/phpcraft-vm/src/lib.rs` | Dispatch loop, frames, variable fetch/store, iterators. |
| `Zend/zend_execute.c` | `crates/phpcraft-vm/src/lib.rs` | Function calls, `global` binding, reference cells and by-reference parameter binding, return values, nesting guard, runtime includes. |
| `Zend/zend_closures.c` | `crates/phpcraft-runtime/src/value.rs`, `crates/phpcraft-vm/src/lib.rs` | `PhpClosure`, capture binding and invocation. |
| `Zend/zend_objects.c`, `Zend/zend_object_handlers.c` | `crates/phpcraft-runtime/src/value.rs`, `crates/phpcraft-vm/src/lib.rs` | `PhpObject`, class registry, property storage, method dispatch and inheritance. |
| `Zend/zend_exceptions.c` | `crates/phpcraft-runtime/src/error.rs`, `crates/phpcraft-vm/src/lib.rs` | `ExceptionData`, built-in exception hierarchy, `throw` and catch matching. |
| `Zend/zend_types.h` | `crates/phpcraft-runtime/src/value.rs` | `Value` corresponds to `zval`; `PhpStr` to `zend_string`. |
| `Zend/zend_string.c`, `Zend/zend_string.h` | `crates/phpcraft-runtime/src/value.rs` | Byte strings and PHP string conversions. |
| `Zend/zend_hash.c`, `Zend/zend_hash.h` | `crates/phpcraft-runtime/src/array.rs` | Ordered hash table with `Key::Int`/`Key::Str`, next-index tracking and COW. |
| `Zend/zend_operators.c`, `Zend/zend_operators.h` | `crates/phpcraft-runtime/src/value.rs` | Loose/strict comparison, numeric strings, arithmetic coercion. |
| `Zend/zend_arithmetic.c` | `crates/phpcraft-vm/src/lib.rs` | Overflow to float, division/modulo rules, power. |
| `Zend/zend_errors.h`, `Zend/zend_exceptions.c` | `crates/phpcraft-runtime/src/error.rs` | `PhpError` / `ErrorKind`. Fatal classes are reported by the CLI. |
| `Zend/zend_builtin_functions.c` | `crates/phpcraft-stdlib/src/lib.rs` | `gettype`, `var_dump`, `define`, `defined`, `function_exists`, ... |
| `ext/standard/string.c` | `crates/phpcraft-stdlib/src/lib.rs` | String builtins. |
| `ext/standard/array.c` | `crates/phpcraft-stdlib/src/lib.rs` | Array builtins. |
| `ext/standard/math.c` | `crates/phpcraft-stdlib/src/lib.rs` | Math builtins; `round()` follows `_php_math_round()`. |
| `ext/standard/formatted_print.c` | `crates/phpcraft-stdlib/src/lib.rs` | `sprintf`/`printf` formats. |
| `ext/standard/type.c` | `crates/phpcraft-stdlib/src/lib.rs` | `is_*`, `get_debug_type`. |
| `ext/standard/basic_functions.c` | `crates/phpcraft-stdlib/src/lib.rs`, `crates/phpcraft-vm/src/lib.rs` | Registry and startup constants. |
| `sapi/cli/php_cli.c` | `crates/phpcraft-cli/src/main.rs` | Options, `-r`, `-l`, stdin, `$argv`/`$argc`, exit codes, stderr. |
| `main/main.c`, `main/php_ini.c` | (not implemented) | No INI system yet. |

## Opcode correspondence

PHPCraft opcodes are stack based. A single opcode can correspond to several
Zend opcodes because Zend uses operands (`op1`, `op2`, `result`) and separate
fetch/assign instructions.

| `phpcraft_bytecode::Op` | Zend opcode(s) |
| --- | --- |
| `Null`, `True`, `False`, `Int`, `Float`, `StrConst` | `ZEND_QM_ASSIGN` with a constant operand |
| `ConstName` | `ZEND_FETCH_CONSTANT` |
| `Pop` | `ZEND_FREE` / `ZEND_POP` |
| `Dup` | `ZEND_COPY_TMP` / `ZEND_QM_ASSIGN` |
| `LoadVar` | `ZEND_FETCH_R` (CV or `$GLOBALS` fetch) |
| `LoadVarSafe` | `ZEND_FETCH_IS` / `ZEND_ISSET_ISEMPTY_CV` |
| `StoreVar` | `ZEND_ASSIGN` |
| `StoreVarKeep` | `ZEND_ASSIGN` + `ZEND_QM_ASSIGN` (assignment value) |
| `LoadVarRef`, `StoreVarRef` | `ZEND_ASSIGN_REF`, `ZEND_FETCH_R`/`ZEND_SEND_VAR_EX` on references |
| `LoadGlobal`, `StoreGlobal`, `LoadGlobalRef`, `StoreGlobalRef` | `ZEND_BIND_GLOBAL`, then CV access / `ZEND_ASSIGN_REF` |
| `LoadTmp`, `StoreTmp` | `ZEND_QM_ASSIGN` / temporary slots (`TMP_VAR`) |
| `Binary` | `ZEND_ADD`, `SUB`, `MUL`, `DIV`, `MOD`, `POW`, `CONCAT`, `SL`, `SR`, `BW_AND`, `BW_OR`, `BW_XOR`, `IS_EQUAL`, `IS_NOT_EQUAL`, `IS_IDENTICAL`, `IS_NOT_IDENTICAL`, `IS_SMALLER`, `IS_SMALLER_OR_EQUAL`, `IS_GREATER`, `IS_GREATER_OR_EQUAL`, `SPACESHIP`, `BOOL_XOR` |
| `Unary` | `ZEND_BOOL_NOT`, `ZEND_BW_NOT`, negation via `ZEND_MUL`/`ZEND_SUB` |
| `Cast` | `ZEND_CAST` (`IS_LONG`, `IS_DOUBLE`, `IS_STRING`, `IS_BOOL`, `IS_ARRAY`) |
| `ToBool` | `ZEND_BOOL` / implicit `JMPZ` conversion |
| `IsNotNull` | `ZEND_ISSET_ISEMPTY` / `ZEND_TYPE_CHECK` |
| `Jump` | `ZEND_JMP` |
| `JumpIfFalse`, `JumpIfFalseKeep` | `ZEND_JMPZ`, `ZEND_JMPZ_EX` |
| `JumpIfTrue`, `JumpIfTrueKeep` | `ZEND_JMPNZ`, `ZEND_JMPNZ_EX` |
| `JumpIfNotNull` | `ZEND_JMP_SET` (`??` lowering) |
| `Echo` | `ZEND_ECHO` |
| `ArrayNew` | `ZEND_INIT_ARRAY` |
| `ArraySet` | `ZEND_ADD_ARRAY_ELEMENT`, `ZEND_INIT_ARRAY` |
| `Index` | `ZEND_FETCH_DIM_R` |
| `IndexSafe` | `ZEND_FETCH_DIM_IS` |
| `AssignDim` | `ZEND_ASSIGN_DIM`, `ZEND_ASSIGN_DIM_OP` |
| `Call` | `ZEND_INIT_FCALL`, `ZEND_SEND_VAL`, `ZEND_DO_FCALL` |
| `CallValue` | `ZEND_INIT_DYNAMIC_CALL`, `ZEND_DO_FCALL` |
| `Include` | `ZEND_INCLUDE_OR_EVAL` |
| `MakeClosure` | `ZEND_DECLARE_LAMBDA_FUNCTION`, `ZEND_BIND_LEXICAL` |
| `NewObject` | `ZEND_NEW`, `ZEND_DO_FCALL` (constructor) |
| `LoadProperty`, `AssignProperty` | `ZEND_FETCH_OBJ_R`, `ZEND_ASSIGN_OBJ` |
| `CallMethod`, `CallStaticMethod` | `ZEND_INIT_METHOD_CALL`, `ZEND_INIT_STATIC_METHOD_CALL`, `ZEND_DO_FCALL` |
| `LoadStaticProperty`, `AssignStaticProperty` | `ZEND_FETCH_STATIC_PROP_*`, `ZEND_ASSIGN_STATIC_PROP_*` |
| `ClassConstant` | `ZEND_FETCH_CLASS_CONSTANT` |
| `InstanceOf` | `ZEND_INSTANCEOF` |
| `CloneObject` | `ZEND_CLONE` |
| `Throw` | `ZEND_THROW` |
| `TryStart`, `TryEnd`, `Rethrow`, `RethrowIfNotNull` | `ZEND_CATCH`, `ZEND_FAST_CALL`, `ZEND_FAST_RET` and handler-table unwinding |
| `ThrowUnhandledMatch` | `ZEND_MATCH_ERROR` |
| `IterInit` | `ZEND_FE_RESET_R` |
| `IterNext` | `ZEND_FE_FETCH_R`, `ZEND_FE_FREE` |
| `Return`, `ReturnNull` | `ZEND_RETURN`, `ZEND_RETURN` with `null` |

`switch` uses only the comparison and jump opcodes above; Zend additionally has
`ZEND_SWITCH_LONG`/`ZEND_SWITCH_STRING` fast paths, which are not needed yet.
`isset()` and `empty()` compile to `LoadVarSafe`/`IndexSafe` followed by
`ToBool`/`IsNotNull`; Zend uses `ZEND_ISSET_ISEMPTY_*`.

## Value and conversion map

| php-src | PHPCraft |
| --- | --- |
| `IS_NULL` | `Value::Null` |
| `IS_FALSE`, `IS_TRUE` | `Value::Bool(false/true)` |
| `IS_LONG` | `Value::Int(i64)` |
| `IS_DOUBLE` | `Value::Float(f64)` |
| `IS_STRING` (`zend_string`) | `Value::Str(Rc<PhpStr>)` |
| `IS_ARRAY` (`HashTable`) | `Value::Array(Rc<PhpArray>)` |
| `IS_OBJECT` (`Closure`) | `Value::Closure(Rc<PhpClosure>)` |
| `IS_OBJECT` (`zend_object`) | `Value::Object(Rc<RefCell<PhpObject>>)` |
| `ZVAL_DEREF`, reference zvals | `Value::Ref(Rc<RefCell<Value>>)`, `Value::deref` |
| `IS_RESOURCE` | not implemented |

| Behaviour | php-src location | PHPCraft |
| --- | --- | --- |
| Numeric-string detection and leading-numeric parsing | `Zend/zend_operators.c` (`is_numeric_string`) | `phpcraft_runtime::value::parse_numeric` |
| Loose comparison | `zend_compare`, `zend_operators.c` | `phpcraft_runtime::compare`, `loose_eq` |
| Strict comparison | `zend_is_identical` | `strict_eq` |
| Array key coercion | `zend_symtable_*`, `zend_hash.c` | `PhpArray`, `Key::from_value` |
| Float to string (`precision=14`) | `zend_gcvt`, `zend_strtod.c` | `float_to_string` |
| Float shortest repr for `var_dump` | `zend_gcvt` (`zero_fraction`, `precision=17`) | `float_to_repr` |
| `sprintf` formatting | `ext/standard/formatted_print.c` | `format_args` in `phpcraft-stdlib` |
| `round()` edge cases | `_php_math_round` in `ext/standard/math.c` | `php_math_round` in `phpcraft-stdlib` |

## Builtins

| php-src function | PHPCraft |
| --- | --- |
| `strlen`, `str_repeat`, `strtoupper`, `strtolower`, `ucfirst`, `lcfirst` | `crates/phpcraft-stdlib/src/lib.rs` |
| `trim`, `ltrim`, `rtrim`, `substr`, `strpos` | same |
| `str_contains`, `str_starts_with`, `str_ends_with`, `str_replace` | same |
| `implode`/`join`, `explode`, `str_pad`, `strrev`, `strcmp`, `chr`, `ord` | same |
| `sprintf`, `printf`, `number_format` | same |
| `count`/`sizeof`, `array_keys`, `array_values`, `array_key_exists` | same |
| `in_array`, `array_search`, `array_sum`, `array_product` | same |
| `array_reverse`, `array_slice`, `array_merge`, `array_fill` | same |
| `array_combine`, `array_flip`, `array_map`, `range` | same |
| `abs`, `ceil`, `floor`, `round`, `sqrt`, `pow`, `intdiv`, `fmod` | same |
| `max`, `min`, `pi` | same |
| `gettype`, `get_debug_type`, `get_class`, `intval`, `floatval`, `strval`, `boolval` | same |
| `is_null`, `is_int`, `is_float`, `is_string`, `is_bool`, `is_array`, `is_object` | same |
| `is_numeric`, `is_scalar`, `is_callable`, `function_exists` | same |
| `call_user_func`, `call_user_func_array` | same, via `RuntimeHooks::call_value` |
| `var_dump`, `print_r` | same |
| `define`, `defined`, `constant` | same, backed by `Vm::define_constant` |

## Tests

| php-src tests | PHPCraft corpus |
| --- | --- |
| `tests/basic/*.phpt` | `tests/php/01_basics.php`, `tests/php/09_tags.php` |
| `tests/lang/operators/*.phpt` | `tests/php/06_operators.php` |
| `tests/lang/arrays/*.phpt` | `tests/php/03_arrays.php` |
| `tests/lang/strings/*.phpt` | `tests/php/02_strings.php` |
| `Zend/tests/arithmetic/*.phpt` | `tests/php/01_basics.php`, `tests/php/08_floats.php` |
| `ext/standard/tests/strings/*.phpt` | `tests/php/02_strings.php`, `tests/php/07_stdlib.php`, `tests/php/14_stdlib_batch.php` |
| `ext/standard/tests/array/*.phpt` | `tests/php/03_arrays.php`, `tests/php/14_stdlib_batch.php` |
| `ext/standard/tests/math/*.phpt` | `tests/php/07_stdlib.php`, `tests/php/08_floats.php`, `tests/php/14_stdlib_batch.php` |
| `tests/lang/control_structures/*.phpt` | `tests/php/04_control.php` |
| `tests/lang/functions/*.phpt` | `tests/php/05_functions.php` |
| `Zend/tests/closures/*.phpt` | `tests/php/15_closures.php` |
| `Zend/tests/classes/*.phpt`, `Zend/tests/exceptions/*.phpt` | `tests/php/16_objects.php` |
| `Zend/tests/ns/*.phpt`, `Zend/tests/use/*.phpt` | `tests/php/17_namespaces_includes.php`, `tests/php/18_namespace_blocks.php` |
| `Zend/tests/ref/*.phpt`, `ext/standard/tests/array/sort*.phpt` | `tests/php/19_references.php` |

`scripts/phpt-subset.sh` extracts `--FILE--` sections from a php-src checkout
and skips tests that need extensions, INI settings, request data or special
headers. Extracted files live under `tests/phpt/` (gitignored) and can be run
with `scripts/diff-test.sh tests/phpt`. Most will initially fail on unimplemented
features; they form the backlog for the next milestones.
