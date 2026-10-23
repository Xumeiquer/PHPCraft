//! PHP builtin functions.
//!
//! Each entry of [`REGISTRY`] corresponds to a `PHP_FUNCTION(...)` in php-src
//! (`ext/standard/*.c`, `Zend/zend_builtin_functions.c`). Names are matched
//! case-insensitively, like PHP function names.
//!
//! Functions receive `&mut dyn RuntimeHooks`, so they can write output, warn,
//! call back into userland and read constants without depending on the VM.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::rc::Rc;

mod signatures;

pub use signatures::parameters;

use phpcraft_runtime::value::parse_numeric;
use phpcraft_runtime::{
    compare, float_to_fixed, float_to_general, float_to_repr, key_from_value, loose_eq, strict_eq,
    Key, Numeric, PhpArray, PhpClosure, PhpError, PhpObject, PhpResult, RuntimeHooks, ToNumber,
    Value,
};

pub type NativeFn = fn(&mut dyn RuntimeHooks, Vec<Value>) -> PhpResult<Value>;

static REGISTRY: &[(&str, NativeFn)] = &[
    // strings
    ("strlen", strlen),
    ("str_repeat", str_repeat),
    ("strtoupper", strtoupper),
    ("strtolower", strtolower),
    ("ucfirst", ucfirst),
    ("lcfirst", lcfirst),
    ("trim", trim),
    ("ltrim", ltrim),
    ("rtrim", rtrim),
    ("substr", substr),
    ("strstr", strstr),
    ("strrchr", strrchr),
    ("strpbrk", strpbrk),
    ("substr_count", substr_count),
    ("ucwords", ucwords),
    ("nl2br", nl2br),
    ("strpos", strpos),
    ("strrpos", strrpos),
    ("str_contains", str_contains),
    ("str_starts_with", str_starts_with),
    ("str_ends_with", str_ends_with),
    ("str_replace", str_replace),
    ("implode", implode),
    ("join", implode),
    ("explode", explode),
    ("sprintf", sprintf),
    ("printf", printf),
    ("number_format", number_format),
    ("str_pad", str_pad),
    ("strrev", strrev),
    ("strcmp", strcmp),
    ("chr", chr),
    ("ord", ord),
    ("urlencode", urlencode),
    ("urldecode", urldecode),
    ("rawurlencode", rawurlencode),
    ("rawurldecode", rawurldecode),
    // arrays
    ("count", count),
    ("sizeof", count),
    ("array_keys", array_keys),
    ("array_values", array_values),
    ("array_key_exists", array_key_exists),
    ("in_array", in_array),
    ("array_search", array_search),
    ("array_sum", array_sum),
    ("array_product", array_product),
    ("array_reverse", array_reverse),
    ("array_key_first", array_key_first),
    ("array_key_last", array_key_last),
    ("array_is_list", array_is_list),
    ("array_unique", array_unique),
    ("array_diff", array_diff),
    ("array_intersect", array_intersect),
    ("array_diff_key", array_diff_key),
    ("array_intersect_key", array_intersect_key),
    ("array_chunk", array_chunk),
    ("array_pad", array_pad),
    ("array_slice", array_slice),
    ("array_merge", array_merge),
    ("array_fill", array_fill),
    ("array_combine", array_combine),
    ("array_flip", array_flip),
    ("array_map", array_map),
    ("sort", sort),
    ("rsort", rsort),
    ("asort", asort),
    ("arsort", arsort),
    ("ksort", ksort),
    ("krsort", krsort),
    ("array_push", array_push),
    ("array_pop", array_pop),
    ("array_shift", array_shift),
    ("array_unshift", array_unshift),
    ("range", range),
    // math
    ("abs", abs),
    ("ceil", ceil),
    ("floor", floor),
    ("round", round),
    ("sqrt", sqrt),
    ("pow", pow),
    ("intdiv", intdiv),
    ("fmod", fmod),
    ("fdiv", fdiv),
    ("log", log),
    ("log10", log10),
    ("exp", exp),
    ("sin", sin),
    ("cos", cos),
    ("tan", tan),
    ("asin", asin),
    ("acos", acos),
    ("atan", atan),
    ("atan2", atan2),
    ("hypot", hypot),
    ("deg2rad", deg2rad),
    ("rad2deg", rad2deg),
    ("is_nan", is_nan),
    ("is_finite", is_finite),
    ("is_infinite", is_infinite),
    ("max", max),
    ("min", min),
    ("pi", pi),
    // types
    ("gettype", gettype),
    ("get_debug_type", get_debug_type),
    ("get_class", get_class),
    ("intval", intval),
    ("floatval", floatval),
    ("doubleval", floatval),
    ("strval", strval),
    ("boolval", boolval),
    ("is_null", is_null),
    ("is_int", is_int),
    ("is_integer", is_int),
    ("is_long", is_int),
    ("is_float", is_float),
    ("is_double", is_float),
    ("is_string", is_string),
    ("is_bool", is_bool),
    ("is_array", is_array),
    ("is_object", is_object),
    ("is_numeric", is_numeric),
    ("is_scalar", is_scalar),
    ("is_callable", is_callable),
    ("function_exists", function_exists),
    ("call_user_func", call_user_func),
    ("call_user_func_array", call_user_func_array),
    // output
    ("var_dump", var_dump),
    ("print_r", print_r),
    // constants
    ("define", define),
    ("defined", defined),
    ("constant", constant_value),
];

pub fn find(name: &str) -> Option<NativeFn> {
    let lower = name.to_ascii_lowercase();
    REGISTRY
        .iter()
        .find(|(candidate, _)| *candidate == lower)
        .map(|(_, function)| *function)
}

pub fn names() -> impl Iterator<Item = &'static str> {
    REGISTRY.iter().map(|(name, _)| *name)
}

/// Argument indexes that a builtin declares by reference. The VM preserves
/// reference cells for these positions and dereferences every other native
/// argument before the call.
pub fn by_ref_params(name: &str) -> &'static [usize] {
    match name.to_ascii_lowercase().as_str() {
        "sort" | "rsort" | "asort" | "arsort" | "ksort" | "krsort" | "array_push" | "array_pop"
        | "array_shift" | "array_unshift" => &[0],
        _ => &[],
    }
}

// ---------------------------------------------------------------- arguments

fn arity(fname: &str, args: &[Value], min: usize, max: Option<usize>) -> PhpResult<()> {
    if args.len() < min {
        return Err(PhpError::argument_count(format!(
            "{fname}() expects at least {min} argument{}, {} given",
            plural(min),
            args.len()
        )));
    }
    if let Some(max) = max {
        if args.len() > max {
            return Err(PhpError::argument_count(format!(
                "{fname}() expects at most {max} argument{}, {} given",
                plural(max),
                args.len()
            )));
        }
    }
    Ok(())
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn type_error_arg(fname: &str, index: usize, param: &str, expected: &str, got: &Value) -> PhpError {
    PhpError::type_error(format!(
        "{fname}(): Argument #{} (${param}) must be of type {expected}, {} given",
        index + 1,
        got.debug_type_name()
    ))
}

fn as_string(args: &[Value], index: usize, fname: &str, param: &str) -> PhpResult<Vec<u8>> {
    match args.get(index) {
        None => Err(PhpError::argument_count(format!(
            "{fname}(): Argument #{} (${param}) not passed",
            index + 1
        ))),
        Some(Value::Str(s)) => Ok(s.0.clone()),
        Some(Value::Array(_)) => Err(type_error_arg(fname, index, param, "string", &args[index])),
        Some(other) => Ok(other.to_bytes()),
    }
}

fn as_int(args: &[Value], index: usize, fname: &str, param: &str) -> PhpResult<i64> {
    match args.get(index) {
        None => Err(PhpError::argument_count(format!(
            "{fname}(): Argument #{} (${param}) not passed",
            index + 1
        ))),
        Some(value) => Ok(value.to_int_cast()),
    }
}

fn as_float(args: &[Value], index: usize, fname: &str, param: &str) -> PhpResult<f64> {
    match args.get(index) {
        None => Err(PhpError::argument_count(format!(
            "{fname}(): Argument #{} (${param}) not passed",
            index + 1
        ))),
        Some(value) => Ok(value.to_float_cast()),
    }
}

fn as_bool(args: &[Value], index: usize) -> bool {
    args.get(index).map(Value::to_bool).unwrap_or(false)
}

fn as_array(args: &[Value], index: usize, fname: &str, param: &str) -> PhpResult<Rc<PhpArray>> {
    match args.get(index) {
        None => Err(PhpError::argument_count(format!(
            "{fname}(): Argument #{} (${param}) not passed",
            index + 1
        ))),
        Some(Value::Array(array)) => Ok(array.clone()),
        Some(other) => Err(type_error_arg(fname, index, param, "array", other)),
    }
}

fn string_value(bytes: Vec<u8>) -> Value {
    Value::str(bytes)
}

/// Maximum bytes a single user-controlled allocation may request.
///
/// PHP relies on `memory_limit` to turn absurd lengths into a fatal error.
/// PHPCraft has no allocator limiter yet, so without this cap a call such as
/// `str_repeat("a", PHP_INT_MAX)` would abort the Rust process instead of
/// producing a catchable fatal error. A single allocation is capped rather
/// than total memory; scripts can still allocate many bounded values.
const MAX_USER_ALLOCATION: usize = 64 * 1024 * 1024;

/// Mirrors Zend's `HT_MAX_SIZE` on 64-bit platforms. `range()` and
/// `array_fill()` reject results at or above this size in php-src.
const HT_MAX_SIZE: u128 = 0x4000_0000;

/// Rust's dynamic formatting precision is limited to this value. Every f64
/// has a finite decimal expansion of at most 1074 fractional digits, so larger
/// precisions can be produced exactly by appending zeroes.
const MAX_RUST_FORMAT_PRECISION: usize = 65_535;
const EXACT_DECIMAL_FRACTION_DIGITS: usize = 1_074;
/// `MAX_FLOAT_PRECISION` in `ext/standard/formatted_print.c`.
const PHP_MAX_FLOAT_PRECISION: usize = 53;

fn allocation_error() -> PhpError {
    PhpError::fatal("Allowed memory size exhausted")
}

fn ensure_allocation(len: usize) -> PhpResult<()> {
    if len > MAX_USER_ALLOCATION || len > isize::MAX as usize {
        return Err(allocation_error());
    }
    Ok(())
}

fn reserve_exact<T>(values: &mut Vec<T>, additional: usize) -> PhpResult<()> {
    ensure_allocation(additional)?;
    values
        .try_reserve_exact(additional)
        .map_err(|_| allocation_error())
}

fn fixed_decimal(value: f64, decimals: usize) -> PhpResult<String> {
    if !value.is_finite() || decimals <= MAX_RUST_FORMAT_PRECISION {
        return Ok(float_to_fixed(value, decimals));
    }
    ensure_allocation(decimals)?;
    let mut text = float_to_fixed(value, EXACT_DECIMAL_FRACTION_DIGITS);
    let extra = decimals - EXACT_DECIMAL_FRACTION_DIGITS;
    text.try_reserve(extra).map_err(|_| allocation_error())?;
    text.extend(std::iter::repeat_n('0', extra));
    Ok(text)
}

fn integer_operand(value: &Value) -> Option<i64> {
    match value.to_number() {
        ToNumber::Num(Numeric::Int(value)) | ToNumber::Leading(Numeric::Int(value)) => Some(value),
        _ => None,
    }
}

fn range_boundary_error() -> PhpError {
    PhpError::value_error(
        "range(): Argument #3 ($step) must be less than the range spanned by argument #1 ($start) and argument #2 ($end)",
    )
}

fn range_negative_step_error() -> PhpError {
    PhpError::value_error(
        "range(): Argument #3 ($step) must be greater than 0 for increasing ranges",
    )
}

fn range_too_large_error() -> PhpError {
    PhpError::value_error("The supplied range exceeds the maximum array size")
}

fn key_to_value(key: &Key) -> Value {
    match key {
        Key::Int(i) => Value::Int(*i),
        Key::Str(s) => Value::Str(s.clone()),
    }
}

// ------------------------------------------------------------------ strings

fn strlen(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("strlen", &args, 1, Some(1))?;
    Ok(Value::Int(
        as_string(&args, 0, "strlen", "string")?.len() as i64
    ))
}

fn str_repeat(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("str_repeat", &args, 2, Some(2))?;
    let string = as_string(&args, 0, "str_repeat", "string")?;
    let times = as_int(&args, 1, "str_repeat", "times")?;
    if times < 0 {
        return Err(PhpError::value_error(
            "str_repeat(): Argument #2 ($times) must be greater than or equal to 0",
        ));
    }
    if string.is_empty() || times == 0 {
        return Ok(string_value(Vec::new()));
    }
    let times = usize::try_from(times).map_err(|_| allocation_error())?;
    let total = string
        .len()
        .checked_mul(times)
        .ok_or_else(allocation_error)?;
    ensure_allocation(total)?;
    let mut out = Vec::new();
    reserve_exact(&mut out, total)?;
    for _ in 0..times {
        out.extend_from_slice(&string);
    }
    Ok(string_value(out))
}

fn strtoupper(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("strtoupper", &args, 1, Some(1))?;
    Ok(string_value(
        as_string(&args, 0, "strtoupper", "string")?.to_ascii_uppercase(),
    ))
}

fn strtolower(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("strtolower", &args, 1, Some(1))?;
    Ok(string_value(
        as_string(&args, 0, "strtolower", "string")?.to_ascii_lowercase(),
    ))
}

fn ucfirst(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("ucfirst", &args, 1, Some(1))?;
    let mut bytes = as_string(&args, 0, "ucfirst", "string")?;
    if let Some(first) = bytes.first_mut() {
        first.make_ascii_uppercase();
    }
    Ok(string_value(bytes))
}

fn lcfirst(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("lcfirst", &args, 1, Some(1))?;
    let mut bytes = as_string(&args, 0, "lcfirst", "string")?;
    if let Some(first) = bytes.first_mut() {
        first.make_ascii_lowercase();
    }
    Ok(string_value(bytes))
}

fn trim_impl(args: &[Value], fname: &str, left: bool, right: bool) -> PhpResult<Value> {
    arity(fname, args, 1, Some(2))?;
    let bytes = as_string(args, 0, fname, "string")?;
    let chars = if args.len() > 1 {
        as_string(args, 1, fname, "characters")?
    } else {
        b" \t\n\r\0\x0b".to_vec()
    };
    let mut start = 0;
    let mut end = bytes.len();
    if left {
        while start < end && chars.contains(&bytes[start]) {
            start += 1;
        }
    }
    if right {
        while end > start && chars.contains(&bytes[end - 1]) {
            end -= 1;
        }
    }
    Ok(string_value(bytes[start..end].to_vec()))
}

fn trim(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    trim_impl(&args, "trim", true, true)
}

fn ltrim(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    trim_impl(&args, "ltrim", true, false)
}

fn rtrim(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    trim_impl(&args, "rtrim", false, true)
}

fn substr(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("substr", &args, 2, Some(3))?;
    let bytes = as_string(&args, 0, "substr", "string")?;
    let offset = as_int(&args, 1, "substr", "offset")?;
    let length = if args.len() > 2 {
        Some(as_int(&args, 2, "substr", "length")?)
    } else {
        None
    };
    let len = bytes.len() as i64;
    let mut start = if offset < 0 { len + offset } else { offset };
    if start < 0 {
        start = 0;
    }
    if start > len {
        return Ok(string_value(Vec::new()));
    }
    let mut end = len;
    if let Some(length) = length {
        if length < 0 {
            end = len.saturating_add(length).max(start);
        } else {
            end = start.saturating_add(length).min(len);
        }
    }
    if end <= start {
        return Ok(string_value(Vec::new()));
    }
    Ok(string_value(bytes[start as usize..end as usize].to_vec()))
}

fn find_sub(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from.min(haystack.len()));
    }
    if from > haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|position| position + from)
}

fn strpos(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("strpos", &args, 2, Some(3))?;
    let haystack = as_string(&args, 0, "strpos", "haystack")?;
    let needle = as_string(&args, 1, "strpos", "needle")?;
    let offset = if args.len() > 2 {
        as_int(&args, 2, "strpos", "offset")?
    } else {
        0
    };
    let len = haystack.len() as i64;
    let mut start = if offset < 0 { len + offset } else { offset };
    if start < 0 {
        start = 0;
    }
    match find_sub(&haystack, &needle, start as usize) {
        Some(position) => Ok(Value::Int(position as i64)),
        None => Ok(Value::Bool(false)),
    }
}

fn strrpos(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("strrpos", &args, 2, Some(3))?;
    let haystack = as_string(&args, 0, "strrpos", "haystack")?;
    let needle = as_string(&args, 1, "strrpos", "needle")?;
    let offset = if args.len() > 2 {
        as_int(&args, 2, "strrpos", "offset")?
    } else {
        0
    };
    let len = haystack.len() as i64;
    let start = if offset < 0 { 0 } else { offset };
    let end = if offset < 0 {
        (len + offset).max(0) as usize
    } else {
        haystack.len()
    };
    if needle.is_empty() {
        return Ok(Value::Int(len));
    }
    let mut position = start.max(0) as usize;
    let mut found = None;
    while position + needle.len() <= end {
        if haystack[position..].starts_with(&needle) {
            found = Some(position);
            position += 1;
        } else {
            position += 1;
        }
    }
    match found {
        Some(position) => Ok(Value::Int(position as i64)),
        None => Ok(Value::Bool(false)),
    }
}

fn str_contains(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("str_contains", &args, 2, Some(2))?;
    let haystack = as_string(&args, 0, "str_contains", "haystack")?;
    let needle = as_string(&args, 1, "str_contains", "needle")?;
    Ok(Value::Bool(find_sub(&haystack, &needle, 0).is_some()))
}

fn str_starts_with(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("str_starts_with", &args, 2, Some(2))?;
    let haystack = as_string(&args, 0, "str_starts_with", "haystack")?;
    let needle = as_string(&args, 1, "str_starts_with", "needle")?;
    Ok(Value::Bool(haystack.starts_with(&needle)))
}

fn str_ends_with(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("str_ends_with", &args, 2, Some(2))?;
    let haystack = as_string(&args, 0, "str_ends_with", "haystack")?;
    let needle = as_string(&args, 1, "str_ends_with", "needle")?;
    Ok(Value::Bool(haystack.ends_with(&needle)))
}

fn replace_all(haystack: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
    if needle.is_empty() {
        return haystack.to_vec();
    }
    let mut out = Vec::with_capacity(haystack.len());
    let mut i = 0;
    while i < haystack.len() {
        if haystack[i..].starts_with(needle) {
            out.extend_from_slice(replacement);
            i += needle.len();
        } else {
            out.push(haystack[i]);
            i += 1;
        }
    }
    out
}

fn str_replace(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("str_replace", &args, 3, Some(3))?;
    let searches: Vec<Vec<u8>> = match &args[0] {
        Value::Array(array) => array
            .values()
            .iter()
            .map(|value| value.to_bytes())
            .collect(),
        other => vec![other.to_bytes()],
    };
    let replacements: Vec<Vec<u8>> = match &args[1] {
        Value::Array(array) => array
            .values()
            .iter()
            .map(|value| value.to_bytes())
            .collect(),
        other => vec![other.to_bytes()],
    };

    let apply = |input: &[u8]| -> Vec<u8> {
        let mut current = input.to_vec();
        for (i, search) in searches.iter().enumerate() {
            let replacement = replacements.get(i).cloned().unwrap_or_default();
            current = replace_all(&current, search, &replacement);
        }
        current
    };

    match &args[2] {
        Value::Array(array) => {
            let mut result = PhpArray::new();
            for (key, value) in array.iter() {
                result.insert(key.clone(), string_value(apply(&value.to_bytes())));
            }
            Ok(Value::array(result))
        }
        other => Ok(string_value(apply(&other.to_bytes()))),
    }
}

fn implode(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("implode", &args, 1, Some(2))?;
    let (separator, array) = if args.len() == 1 {
        (Vec::new(), as_array(&args, 0, "implode", "array")?)
    } else {
        (
            as_string(&args, 0, "implode", "separator")?,
            as_array(&args, 1, "implode", "array")?,
        )
    };
    let mut out = Vec::new();
    for (i, value) in array.values().iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(&separator);
        }
        out.extend_from_slice(&value.to_bytes());
    }
    Ok(string_value(out))
}

fn explode(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("explode", &args, 2, Some(3))?;
    let separator = as_string(&args, 0, "explode", "separator")?;
    let string = as_string(&args, 1, "explode", "string")?;
    if separator.is_empty() {
        return Err(PhpError::value_error(
            "explode(): Argument #1 ($separator) cannot be empty",
        ));
    }
    let limit = if args.len() > 2 {
        Some(as_int(&args, 2, "explode", "limit")?)
    } else {
        None
    };

    let mut parts: Vec<Vec<u8>> = Vec::new();
    let mut start = 0;
    let mut position = 0;
    while let Some(found) = find_sub(&string, &separator, position) {
        parts.push(string[start..found].to_vec());
        start = found + separator.len();
        position = start;
    }
    parts.push(string[start..].to_vec());

    if let Some(limit) = limit {
        if limit > 0 {
            let limit = limit as usize;
            if parts.len() > limit {
                let tail = parts.split_off(limit - 1);
                let mut rest = Vec::new();
                for (index, part) in tail.iter().enumerate() {
                    if index > 0 {
                        rest.extend_from_slice(&separator);
                    }
                    rest.extend_from_slice(part);
                }
                parts.push(rest);
            }
        } else if limit < 0 {
            // `-i64::MIN` overflows in debug builds; `unsigned_abs()` also
            // handles the "drop everything" case correctly.
            let drop = usize::try_from(limit.unsigned_abs()).unwrap_or(usize::MAX);
            if drop < parts.len() {
                parts.truncate(parts.len() - drop);
            } else {
                parts.clear();
            }
        }
    }

    let mut array = PhpArray::with_capacity(parts.len());
    for part in parts {
        array.push(string_value(part));
    }
    Ok(Value::array(array))
}

fn sprintf(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("sprintf", &args, 1, None)?;
    let format = as_string(&args, 0, "sprintf", "format")?;
    Ok(string_value(format_args(&format, &args[1..], "sprintf")?))
}

fn printf(hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("printf", &args, 1, None)?;
    let format = as_string(&args, 0, "printf", "format")?;
    let out = format_args(&format, &args[1..], "printf")?;
    hooks.write(&out);
    Ok(Value::Int(out.len() as i64))
}

struct FormatSpec {
    left: bool,
    plus: bool,
    zero: bool,
    space: bool,
    width: usize,
    precision: Option<usize>,
}

fn format_args(format: &[u8], args: &[Value], fname: &str) -> PhpResult<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut arg_index = 0;
    while i < format.len() {
        if format[i] != b'%' {
            out.push(format[i]);
            i += 1;
            continue;
        }
        i += 1;
        if i < format.len() && format[i] == b'%' {
            out.push(b'%');
            i += 1;
            continue;
        }
        let mut spec = FormatSpec {
            left: false,
            plus: false,
            zero: false,
            space: false,
            width: 0,
            precision: None,
        };
        loop {
            match format.get(i).copied() {
                Some(b'-') => spec.left = true,
                Some(b'+') => spec.plus = true,
                Some(b'0') => spec.zero = true,
                Some(b' ') => spec.space = true,
                _ => break,
            }
            i += 1;
        }
        while let Some(digit) = format.get(i).filter(|c| c.is_ascii_digit()) {
            spec.width = spec.width.saturating_mul(10) + (digit - b'0') as usize;
            i += 1;
        }
        // php_sprintf_getnumber() rejects values outside [0, INT_MAX].
        if spec.width > i32::MAX as usize {
            return Err(PhpError::value_error(format!(
                "{fname}(): Width must be between 0 and {}",
                i32::MAX
            )));
        }
        if format.get(i) == Some(&b'.') {
            i += 1;
            let mut precision = 0usize;
            while let Some(digit) = format.get(i).filter(|c| c.is_ascii_digit()) {
                precision = precision.saturating_mul(10) + (digit - b'0') as usize;
                i += 1;
            }
            if precision > i32::MAX as usize {
                return Err(PhpError::value_error(format!(
                    "{fname}(): Precision must be between 0 and {}",
                    i32::MAX
                )));
            }
            spec.precision = Some(precision);
        }
        let conversion = match format.get(i).copied() {
            Some(c) => c,
            None => break,
        };
        i += 1;
        let value = args.get(arg_index).ok_or_else(|| {
            PhpError::argument_count(format!("{fname}(): Argument #{} not found", arg_index + 2))
        })?;
        arg_index += 1;

        // php_sprintf() truncates floating-point precision to 53 digits.
        if matches!(conversion, b'f' | b'F' | b'e' | b'E' | b'g' | b'G') {
            if let Some(precision) = spec.precision.as_mut() {
                *precision = (*precision).min(PHP_MAX_FLOAT_PRECISION);
            }
        }

        match conversion {
            b's' => {
                let mut bytes = value.to_bytes();
                if let Some(precision) = spec.precision {
                    bytes.truncate(precision);
                }
                out.extend(pad(&[], &bytes, spec.width, spec.left, false)?);
            }
            b'd' | b'i' => {
                let number = value.to_int_cast();
                let negative = number < 0;
                let mut digits = number.unsigned_abs().to_string().into_bytes();
                if let Some(precision) = spec.precision {
                    if digits.len() < precision {
                        ensure_allocation(precision)?;
                        let mut padded = Vec::new();
                        reserve_exact(&mut padded, precision)?;
                        padded.resize(precision - digits.len(), b'0');
                        padded.extend_from_slice(&digits);
                        digits = padded;
                    }
                }
                let sign = sign_prefix(negative, spec.plus, spec.space);
                out.extend(pad(&sign, &digits, spec.width, spec.left, spec.zero)?);
            }
            b'u' => {
                let number = value.to_int_cast() as u64;
                let digits = number.to_string().into_bytes();
                out.extend(pad(&[], &digits, spec.width, spec.left, spec.zero)?);
            }
            b'f' | b'F' => {
                let number = value.to_float_cast();
                let decimals = spec.precision.unwrap_or(6);
                let text = fixed_decimal(number.abs(), decimals)?;
                let sign = sign_prefix(number < 0.0, spec.plus, spec.space);
                out.extend(pad(
                    &sign,
                    text.as_bytes(),
                    spec.width,
                    spec.left,
                    spec.zero,
                )?);
            }
            b'e' | b'E' => {
                let number = value.to_float_cast();
                let decimals = spec.precision.unwrap_or(6);
                ensure_allocation(decimals)?;
                let text = format!("{:.*e}", decimals, number.abs());
                let (mantissa, exponent) = text.split_once('e').expect("exponential");
                let exponent: i32 = exponent.parse().expect("exponent");
                let mut body = mantissa.as_bytes().to_vec();
                body.push(if conversion == b'E' { b'E' } else { b'e' });
                body.push(if exponent < 0 { b'-' } else { b'+' });
                body.extend_from_slice(exponent.unsigned_abs().to_string().as_bytes());
                let sign = sign_prefix(number < 0.0, spec.plus, spec.space);
                out.extend(pad(&sign, &body, spec.width, spec.left, spec.zero)?);
            }
            b'g' | b'G' => {
                let number = value.to_float_cast();
                let precision = spec.precision.unwrap_or(6).max(1);
                ensure_allocation(precision)?;
                let mut text = float_to_general(number.abs(), precision);
                if conversion == b'G' {
                    text = text.to_uppercase();
                } else {
                    text = text.replace('E', "e");
                }
                let sign = sign_prefix(number < 0.0, spec.plus, spec.space);
                out.extend(pad(
                    &sign,
                    text.as_bytes(),
                    spec.width,
                    spec.left,
                    spec.zero,
                )?);
            }
            b'x' | b'X' | b'o' | b'b' => {
                let number = value.to_int_cast() as u64;
                let mut digits = match conversion {
                    b'x' | b'X' => format!("{number:x}"),
                    b'o' => format!("{number:o}"),
                    _ => format!("{number:b}"),
                }
                .into_bytes();
                if conversion == b'X' {
                    digits.make_ascii_uppercase();
                }
                out.extend(pad(&[], &digits, spec.width, spec.left, spec.zero)?);
            }
            b'c' => {
                let code = value.to_int_cast() as u32;
                let ch = char::from_u32(code).unwrap_or('\u{fffd}');
                let mut buf = [0u8; 4];
                out.extend(pad(
                    &[],
                    ch.encode_utf8(&mut buf).as_bytes(),
                    spec.width,
                    spec.left,
                    false,
                )?);
            }
            other => {
                return Err(PhpError::value_error(format!(
                    "{fname}(): Unknown format specifier \"{}\"",
                    char::from(other)
                )))
            }
        }
    }
    Ok(out)
}

fn sign_prefix(negative: bool, plus: bool, space: bool) -> Vec<u8> {
    if negative {
        b"-".to_vec()
    } else if plus {
        b"+".to_vec()
    } else if space {
        b" ".to_vec()
    } else {
        Vec::new()
    }
}

fn pad(sign: &[u8], body: &[u8], width: usize, left: bool, zero: bool) -> PhpResult<Vec<u8>> {
    let total = sign.len() + body.len();
    if total >= width {
        let mut out = sign.to_vec();
        out.extend_from_slice(body);
        return Ok(out);
    }
    let fill = width - total;
    ensure_allocation(width)?;
    let mut out = Vec::new();
    reserve_exact(&mut out, width)?;
    if left {
        out.extend_from_slice(sign);
        out.extend_from_slice(body);
        out.resize(width, b' ');
    } else if zero {
        out.extend_from_slice(sign);
        out.resize(sign.len() + fill, b'0');
        out.extend_from_slice(body);
    } else {
        out.resize(fill, b' ');
        out.extend_from_slice(sign);
        out.extend_from_slice(body);
    }
    Ok(out)
}

fn number_format(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("number_format", &args, 1, Some(4))?;
    let number = as_float(&args, 0, "number_format", "num")?;
    let decimals = if args.len() > 1 {
        as_int(&args, 1, "number_format", "decimals")?.max(0) as usize
    } else {
        0
    };
    // Rust's formatting machinery panics when the precision cannot be
    // represented by the runtime; reject absurd values before that point.
    ensure_allocation(decimals)?;
    let decimal_point = if args.len() > 2 {
        as_string(&args, 2, "number_format", "decimal_separator")?
    } else {
        b".".to_vec()
    };
    let thousands_sep = if args.len() > 3 {
        as_string(&args, 3, "number_format", "thousands_separator")?
    } else {
        b",".to_vec()
    };

    let rounded = php_math_round(number.abs(), decimals as i64);
    let text = fixed_decimal(rounded, decimals)?;
    let (integer, fraction) = match text.split_once('.') {
        Some((integer, fraction)) => (integer.to_string(), fraction.to_string()),
        None => (text, String::new()),
    };

    let mut grouped = String::new();
    for (i, ch) in integer.chars().enumerate() {
        if i > 0 && (integer.len() - i) % 3 == 0 {
            grouped.push_str(&String::from_utf8_lossy(&thousands_sep));
        }
        grouped.push(ch);
    }
    let mut out = Vec::new();
    if number < 0.0 && rounded != 0.0 {
        out.push(b'-');
    }
    out.extend_from_slice(grouped.as_bytes());
    if decimals > 0 {
        out.extend_from_slice(&decimal_point);
        out.extend_from_slice(fraction.as_bytes());
    }
    Ok(string_value(out))
}

fn str_pad(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("str_pad", &args, 2, Some(4))?;
    let mut input = as_string(&args, 0, "str_pad", "string")?;
    let length = as_int(&args, 1, "str_pad", "length")?;
    let pad_string = if args.len() > 2 {
        as_string(&args, 2, "str_pad", "pad_string")?
    } else {
        b" ".to_vec()
    };
    let pad_type = if args.len() > 3 {
        as_int(&args, 3, "str_pad", "pad_type")?
    } else {
        1
    };
    if length <= input.len() as i64 {
        return Ok(string_value(input));
    }
    if pad_string.is_empty() {
        return Err(PhpError::value_error(
            "str_pad(): Argument #3 ($pad_string) must be a non-empty string",
        ));
    }
    let total = usize::try_from(length).map_err(|_| allocation_error())?;
    ensure_allocation(total)?;
    let needed = total - input.len();
    let mut padding = Vec::new();
    reserve_exact(&mut padding, needed)?;
    while padding.len() < needed {
        padding.push(pad_string[padding.len() % pad_string.len()]);
    }
    match pad_type {
        0 => {
            let mut out = Vec::new();
            reserve_exact(&mut out, total)?;
            out.extend_from_slice(&padding[..needed]);
            out.extend_from_slice(&input);
            input = out;
        }
        2 => {
            let left = needed / 2;
            let right = needed - left;
            let mut out = Vec::new();
            reserve_exact(&mut out, total)?;
            out.extend_from_slice(&padding[..left]);
            out.extend_from_slice(&input);
            out.extend_from_slice(&padding[..right]);
            input = out;
        }
        _ => {
            input.try_reserve(needed).map_err(|_| allocation_error())?;
            input.extend_from_slice(&padding[..needed]);
        }
    }
    Ok(string_value(input))
}

fn strrev(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("strrev", &args, 1, Some(1))?;
    let mut bytes = as_string(&args, 0, "strrev", "string")?;
    bytes.reverse();
    Ok(string_value(bytes))
}

fn strcmp(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("strcmp", &args, 2, Some(2))?;
    let a = as_string(&args, 0, "strcmp", "string1")?;
    let b = as_string(&args, 1, "strcmp", "string2")?;
    Ok(Value::Int(match a.cmp(&b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }))
}

fn strstr(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("strstr", &args, 2, Some(3))?;
    let haystack = as_string(&args, 0, "strstr", "haystack")?;
    let needle = as_string(&args, 1, "strstr", "needle")?;
    if needle.is_empty() {
        return Ok(string_value(haystack));
    }
    let before = as_bool(&args, 2);
    match find_sub(&haystack, &needle, 0) {
        Some(position) if before => Ok(string_value(haystack[..position].to_vec())),
        Some(position) => Ok(string_value(haystack[position..].to_vec())),
        None => Ok(Value::Bool(false)),
    }
}

fn strrchr(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("strrchr", &args, 2, Some(2))?;
    let haystack = as_string(&args, 0, "strrchr", "haystack")?;
    let needle = as_string(&args, 1, "strrchr", "needle")?;
    if needle.is_empty() {
        return Ok(Value::Bool(false));
    }
    let mut found = None;
    let mut from = 0;
    while let Some(position) = find_sub(&haystack, &needle, from) {
        found = Some(position);
        from = position + 1;
    }
    match found {
        Some(position) => Ok(string_value(haystack[position..].to_vec())),
        None => Ok(Value::Bool(false)),
    }
}

fn strpbrk(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("strpbrk", &args, 2, Some(2))?;
    let haystack = as_string(&args, 0, "strpbrk", "string")?;
    let characters = as_string(&args, 1, "strpbrk", "characters")?;
    if characters.is_empty() {
        return Err(PhpError::value_error(
            "strpbrk(): Argument #2 ($characters) must be a non-empty string",
        ));
    }
    match haystack.iter().position(|byte| characters.contains(byte)) {
        Some(position) => Ok(string_value(haystack[position..].to_vec())),
        None => Ok(Value::Bool(false)),
    }
}

fn substr_count(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("substr_count", &args, 2, Some(4))?;
    let haystack = as_string(&args, 0, "substr_count", "haystack")?;
    let needle = as_string(&args, 1, "substr_count", "needle")?;
    if needle.is_empty() {
        return Err(PhpError::value_error(
            "substr_count(): Argument #2 ($needle) must not be empty",
        ));
    }
    let total = haystack.len() as i64;
    let offset = if args.len() > 2 {
        as_int(&args, 2, "substr_count", "offset")?
    } else {
        0
    };
    let start = if offset < 0 {
        total.saturating_add(offset)
    } else {
        offset
    };
    if start < 0 || start > total {
        return Err(PhpError::value_error(
            "substr_count(): Argument #3 ($offset) must be contained in argument #1 ($haystack)",
        ));
    }
    let end = if args.len() > 3 && !args[3].is_null() {
        let length = as_int(&args, 3, "substr_count", "length")?;
        let end = if length < 0 {
            total.saturating_add(length)
        } else {
            start.saturating_add(length)
        };
        if end < start || end > total {
            return Err(PhpError::value_error(
                "substr_count(): Argument #4 ($length) must be contained in argument #1 ($haystack)",
            ));
        }
        end
    } else {
        total
    };

    let slice = &haystack[start as usize..end as usize];
    let mut count = 0i64;
    let mut from = 0;
    while let Some(position) = find_sub(slice, &needle, from) {
        count += 1;
        from = position + needle.len();
    }
    Ok(Value::Int(count))
}

fn ucwords(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("ucwords", &args, 1, Some(2))?;
    let mut bytes = as_string(&args, 0, "ucwords", "string")?;
    let separators = if args.len() > 1 {
        as_string(&args, 1, "ucwords", "separators")?
    } else {
        b" \t\n\r\x0c\x0b".to_vec()
    };
    let mut at_word_start = true;
    for byte in &mut bytes {
        if at_word_start {
            byte.make_ascii_uppercase();
        }
        at_word_start = separators.contains(byte);
    }
    Ok(string_value(bytes))
}

fn nl2br(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("nl2br", &args, 1, Some(2))?;
    let bytes = as_string(&args, 0, "nl2br", "string")?;
    let xhtml = if args.len() > 1 {
        args[1].to_bool()
    } else {
        true
    };
    let break_tag: &[u8] = if xhtml { b"<br />" } else { b"<br>" };
    let mut out = Vec::new();
    reserve_exact(&mut out, bytes.len())?;
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'\r' || byte == b'\n' {
            out.extend_from_slice(break_tag);
            out.push(byte);
            i += 1;
            if byte == b'\r' && bytes.get(i) == Some(&b'\n') {
                out.push(b'\n');
                i += 1;
            }
        } else {
            out.push(byte);
            i += 1;
        }
    }
    Ok(string_value(out))
}

fn chr(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("chr", &args, 1, Some(1))?;
    let value = as_int(&args, 0, "chr", "codepoint")?;
    Ok(string_value(vec![(value & 0xff) as u8]))
}

fn ord(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("ord", &args, 1, Some(1))?;
    let bytes = as_string(&args, 0, "ord", "character")?;
    Ok(Value::Int(bytes.first().copied().unwrap_or(0) as i64))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn url_encode_bytes(bytes: &[u8], raw: bool) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = Vec::new();
    for &byte in bytes {
        let unreserved = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.')
            || (raw && byte == b'~');
        if unreserved {
            out.push(byte);
        } else if !raw && byte == b' ' {
            out.push(b'+');
        } else {
            out.push(b'%');
            out.push(HEX[(byte >> 4) as usize]);
            out.push(HEX[(byte & 0x0f) as usize]);
        }
    }
    out
}

fn urlencode(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("urlencode", &args, 1, Some(1))?;
    let bytes = as_string(&args, 0, "urlencode", "string")?;
    Ok(string_value(url_encode_bytes(&bytes, false)))
}

fn rawurlencode(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("rawurlencode", &args, 1, Some(1))?;
    let bytes = as_string(&args, 0, "rawurlencode", "string")?;
    Ok(string_value(url_encode_bytes(&bytes, true)))
}

fn url_decode_bytes(bytes: &[u8], raw: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if !raw && byte == b'+' {
            out.push(b' ');
            i += 1;
        } else if byte == b'%' && i + 2 < bytes.len() {
            match (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                (Some(high), Some(low)) => {
                    out.push(high * 16 + low);
                    i += 3;
                }
                _ => {
                    out.push(byte);
                    i += 1;
                }
            }
        } else {
            out.push(byte);
            i += 1;
        }
    }
    out
}

fn urldecode(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("urldecode", &args, 1, Some(1))?;
    let bytes = as_string(&args, 0, "urldecode", "string")?;
    Ok(string_value(url_decode_bytes(&bytes, false)))
}

fn rawurldecode(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("rawurldecode", &args, 1, Some(1))?;
    let bytes = as_string(&args, 0, "rawurldecode", "string")?;
    Ok(string_value(url_decode_bytes(&bytes, true)))
}

// ------------------------------------------------------------------- arrays

fn count(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("count", &args, 1, Some(2))?;
    let array = as_array(&args, 0, "count", "value")?;
    Ok(Value::Int(array.len() as i64))
}

fn array_keys(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_keys", &args, 1, Some(3))?;
    let array = as_array(&args, 0, "array_keys", "array")?;
    let filter = args.get(1);
    let strict = as_bool(&args, 2);
    let mut out = PhpArray::new();
    for (key, value) in array.iter() {
        let include = match filter {
            None => true,
            Some(needle) if strict => strict_eq(value, needle),
            Some(needle) => loose_eq(value, needle),
        };
        if include {
            out.push(key_to_value(key));
        }
    }
    Ok(Value::array(out))
}

fn array_values(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_values", &args, 1, Some(1))?;
    let array = as_array(&args, 0, "array_values", "array")?;
    let mut out = PhpArray::new();
    for value in array.values() {
        out.push(value);
    }
    Ok(Value::array(out))
}

fn array_key_exists(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_key_exists", &args, 2, Some(2))?;
    let array = as_array(&args, 1, "array_key_exists", "array")?;
    let key = key_from_value(&args[0])?;
    Ok(Value::Bool(array.contains_key(&key)))
}

fn in_array(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("in_array", &args, 2, Some(3))?;
    let haystack = as_array(&args, 1, "in_array", "haystack")?;
    let strict = as_bool(&args, 2);
    for value in haystack.values() {
        if (strict && strict_eq(&value, &args[0])) || (!strict && loose_eq(&value, &args[0])) {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn array_search(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_search", &args, 2, Some(3))?;
    let haystack = as_array(&args, 1, "array_search", "haystack")?;
    let strict = as_bool(&args, 2);
    for (key, value) in haystack.iter() {
        if (strict && strict_eq(value, &args[0])) || (!strict && loose_eq(value, &args[0])) {
            return Ok(key_to_value(key));
        }
    }
    Ok(Value::Bool(false))
}

fn array_sum(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_sum", &args, 1, Some(1))?;
    let array = as_array(&args, 0, "array_sum", "array")?;
    let mut int_total: i64 = 0;
    let mut float_total = 0.0f64;
    let mut is_float = false;
    for value in array.values() {
        match value.to_number() {
            phpcraft_runtime::ToNumber::Num(phpcraft_runtime::Numeric::Int(i)) => {
                if is_float {
                    float_total += i as f64;
                } else {
                    match int_total.checked_add(i) {
                        Some(total) => int_total = total,
                        None => {
                            is_float = true;
                            float_total = int_total as f64 + i as f64;
                        }
                    }
                }
            }
            phpcraft_runtime::ToNumber::Num(phpcraft_runtime::Numeric::Float(f))
            | phpcraft_runtime::ToNumber::Leading(phpcraft_runtime::Numeric::Float(f)) => {
                if !is_float {
                    is_float = true;
                    float_total = int_total as f64;
                }
                float_total += f;
            }
            phpcraft_runtime::ToNumber::Leading(phpcraft_runtime::Numeric::Int(i)) => {
                if is_float {
                    float_total += i as f64;
                } else {
                    int_total += i;
                }
            }
            phpcraft_runtime::ToNumber::Invalid => {}
        }
    }
    Ok(if is_float {
        Value::Float(float_total)
    } else {
        Value::Int(int_total)
    })
}

fn array_product(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_product", &args, 1, Some(1))?;
    let array = as_array(&args, 0, "array_product", "array")?;
    let mut int_total: i64 = 1;
    let mut float_total = 1.0f64;
    let mut is_float = false;
    for value in array.values() {
        match value.to_number() {
            phpcraft_runtime::ToNumber::Num(phpcraft_runtime::Numeric::Int(i))
            | phpcraft_runtime::ToNumber::Leading(phpcraft_runtime::Numeric::Int(i)) => {
                if is_float {
                    float_total *= i as f64;
                } else {
                    match int_total.checked_mul(i) {
                        Some(total) => int_total = total,
                        None => {
                            is_float = true;
                            float_total = int_total as f64 * i as f64;
                        }
                    }
                }
            }
            phpcraft_runtime::ToNumber::Num(phpcraft_runtime::Numeric::Float(f))
            | phpcraft_runtime::ToNumber::Leading(phpcraft_runtime::Numeric::Float(f)) => {
                is_float = true;
                float_total *= f;
            }
            phpcraft_runtime::ToNumber::Invalid => {}
        }
    }
    Ok(if is_float {
        Value::Float(float_total)
    } else {
        Value::Int(int_total)
    })
}

fn array_reverse(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_reverse", &args, 1, Some(2))?;
    let array = as_array(&args, 0, "array_reverse", "array")?;
    let preserve_keys = as_bool(&args, 1);
    let mut out = PhpArray::with_capacity(array.len());
    let entries: Vec<(Key, Value)> = array
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for (key, value) in entries.into_iter().rev() {
        match key {
            Key::Str(_) => {
                out.insert(key, value);
            }
            Key::Int(i) if preserve_keys => {
                out.insert(Key::Int(i), value);
            }
            Key::Int(_) => {
                out.push(value);
            }
        }
    }
    Ok(Value::array(out))
}

fn array_key_first(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_key_first", &args, 1, Some(1))?;
    let array = as_array(&args, 0, "array_key_first", "array")?;
    Ok(array.first_key().map(key_to_value).unwrap_or(Value::Null))
}

fn array_key_last(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_key_last", &args, 1, Some(1))?;
    let array = as_array(&args, 0, "array_key_last", "array")?;
    Ok(array
        .iter()
        .last()
        .map(|(key, _)| key_to_value(key))
        .unwrap_or(Value::Null))
}

fn array_is_list(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_is_list", &args, 1, Some(1))?;
    let array = as_array(&args, 0, "array_is_list", "array")?;
    for (index, (key, _)) in array.iter().enumerate() {
        if key != &Key::Int(index as i64) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn numeric_for_array_unique(value: &Value) -> f64 {
    match value.to_number() {
        ToNumber::Num(number) | ToNumber::Leading(number) => number.to_float(),
        ToNumber::Invalid => 0.0,
    }
}

fn array_unique(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_unique", &args, 1, Some(2))?;
    let array = as_array(&args, 0, "array_unique", "array")?;
    let flags = if args.len() > 1 {
        as_int(&args, 1, "array_unique", "flags")?
    } else {
        2 // SORT_STRING
    };
    let mut out = PhpArray::try_with_capacity(array.len()).ok_or_else(allocation_error)?;
    let mut seen_strings: HashSet<Vec<u8>> = HashSet::new();
    let mut seen_numeric: Vec<f64> = Vec::new();
    let mut seen_values: Vec<Value> = Vec::new();
    for (key, value) in array.iter() {
        let duplicate = match flags {
            0 => seen_values.iter().any(|seen| loose_eq(seen, value)),
            1 => {
                let number = numeric_for_array_unique(value);
                seen_numeric.contains(&number)
            }
            _ => {
                let text = value.to_bytes();
                if seen_strings.contains(&text) {
                    true
                } else {
                    seen_strings.insert(text);
                    false
                }
            }
        };
        if duplicate {
            continue;
        }
        match flags {
            0 => seen_values.push(value.clone()),
            1 => seen_numeric.push(numeric_for_array_unique(value)),
            _ => {}
        }
        out.insert(key.clone(), value.clone());
    }
    Ok(Value::array(out))
}

fn comparison_text(value: &Value) -> Vec<u8> {
    value.to_bytes()
}

fn array_diff(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_diff", &args, 1, None)?;
    let first = as_array(&args, 0, "array_diff", "array")?;
    let mut others: Vec<HashSet<Vec<u8>>> = Vec::new();
    for index in 1..args.len() {
        let array = as_array(&args, index, "array_diff", "arrays")?;
        let mut set = HashSet::with_capacity(array.len());
        for value in array.values() {
            set.insert(comparison_text(&value));
        }
        others.push(set);
    }
    let mut out = PhpArray::try_with_capacity(first.len()).ok_or_else(allocation_error)?;
    for (key, value) in first.iter() {
        let text = comparison_text(value);
        if !others.iter().any(|set| set.contains(&text)) {
            out.insert(key.clone(), value.clone());
        }
    }
    Ok(Value::array(out))
}

fn array_intersect(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_intersect", &args, 1, None)?;
    let first = as_array(&args, 0, "array_intersect", "array")?;
    let mut others: Vec<HashSet<Vec<u8>>> = Vec::new();
    for index in 1..args.len() {
        let array = as_array(&args, index, "array_intersect", "arrays")?;
        let mut set = HashSet::with_capacity(array.len());
        for value in array.values() {
            set.insert(comparison_text(&value));
        }
        others.push(set);
    }
    let mut out = PhpArray::try_with_capacity(first.len()).ok_or_else(allocation_error)?;
    for (key, value) in first.iter() {
        let text = comparison_text(value);
        if others.iter().all(|set| set.contains(&text)) {
            out.insert(key.clone(), value.clone());
        }
    }
    Ok(Value::array(out))
}

fn array_diff_key(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_diff_key", &args, 1, None)?;
    let first = as_array(&args, 0, "array_diff_key", "array")?;
    let mut others: Vec<HashSet<Key>> = Vec::new();
    for index in 1..args.len() {
        let array = as_array(&args, index, "array_diff_key", "arrays")?;
        others.push(array.keys().into_iter().collect());
    }
    let mut out = PhpArray::try_with_capacity(first.len()).ok_or_else(allocation_error)?;
    for (key, value) in first.iter() {
        if !others.iter().any(|set| set.contains(key)) {
            out.insert(key.clone(), value.clone());
        }
    }
    Ok(Value::array(out))
}

fn array_intersect_key(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_intersect_key", &args, 1, None)?;
    let first = as_array(&args, 0, "array_intersect_key", "array")?;
    let mut others: Vec<HashSet<Key>> = Vec::new();
    for index in 1..args.len() {
        let array = as_array(&args, index, "array_intersect_key", "arrays")?;
        others.push(array.keys().into_iter().collect());
    }
    let mut out = PhpArray::try_with_capacity(first.len()).ok_or_else(allocation_error)?;
    for (key, value) in first.iter() {
        if others.iter().all(|set| set.contains(key)) {
            out.insert(key.clone(), value.clone());
        }
    }
    Ok(Value::array(out))
}

fn array_chunk(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_chunk", &args, 2, Some(3))?;
    let array = as_array(&args, 0, "array_chunk", "array")?;
    let length = as_int(&args, 1, "array_chunk", "length")?;
    if length <= 0 {
        return Err(PhpError::value_error(
            "array_chunk(): Argument #2 ($length) must be greater than 0",
        ));
    }
    let preserve_keys = as_bool(&args, 2);
    let entries: Vec<(Key, Value)> = array
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if entries.is_empty() {
        return Ok(Value::array(PhpArray::new()));
    }
    let length = usize::try_from(length).unwrap_or(usize::MAX);
    let chunk_count = (entries.len() as u128).div_ceil(length as u128);
    let mut out = PhpArray::try_with_capacity(chunk_count as usize).ok_or_else(allocation_error)?;
    for chunk in entries.chunks(length) {
        let mut part = PhpArray::try_with_capacity(chunk.len()).ok_or_else(allocation_error)?;
        for (key, value) in chunk {
            if preserve_keys {
                part.insert(key.clone(), value.clone());
            } else {
                part.push(value.clone());
            }
        }
        out.push(Value::array(part));
    }
    Ok(Value::array(out))
}

fn array_pad(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_pad", &args, 3, Some(3))?;
    let array = as_array(&args, 0, "array_pad", "array")?;
    let length = as_int(&args, 1, "array_pad", "length")?;
    let target = length.unsigned_abs() as u128;
    if target >= HT_MAX_SIZE {
        return Err(PhpError::value_error(
            "array_pad(): Argument #2 ($length) must not exceed the maximum allowed array size",
        ));
    }
    if target <= array.len() as u128 {
        return Ok(Value::Array(array));
    }
    let target = target as usize;
    let pad_count = target - array.len();
    let mut out = PhpArray::try_with_capacity(target).ok_or_else(allocation_error)?;
    if length < 0 {
        for _ in 0..pad_count {
            out.push(args[2].clone());
        }
        for (key, value) in array.iter() {
            match key {
                Key::Str(_) => {
                    out.insert(key.clone(), value.clone());
                }
                Key::Int(_) => {
                    out.push(value.clone());
                }
            }
        }
    } else {
        for (key, value) in array.iter() {
            match key {
                Key::Str(_) => {
                    out.insert(key.clone(), value.clone());
                }
                Key::Int(_) => {
                    out.push(value.clone());
                }
            }
        }
        for _ in 0..pad_count {
            out.push(args[2].clone());
        }
    }
    Ok(Value::array(out))
}

fn array_slice(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_slice", &args, 2, Some(4))?;
    let array = as_array(&args, 0, "array_slice", "array")?;
    let offset = as_int(&args, 1, "array_slice", "offset")?;
    let length = if args.len() > 2 && !args[2].is_null() {
        Some(as_int(&args, 2, "array_slice", "length")?)
    } else {
        None
    };
    let preserve_keys = as_bool(&args, 3);

    let entries: Vec<(Key, Value)> = array
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let total = entries.len() as i64;
    let mut start = if offset < 0 { total + offset } else { offset };
    start = start.clamp(0, total);
    let mut end = total;
    if let Some(length) = length {
        if length < 0 {
            end = total.saturating_add(length).max(start);
        } else {
            end = start.saturating_add(length).min(total);
        }
    }

    let mut out = PhpArray::new();
    for (key, value) in &entries[start as usize..end as usize] {
        match key {
            Key::Str(_) => out.insert(key.clone(), value.clone()),
            Key::Int(_) => {
                if preserve_keys {
                    out.insert(key.clone(), value.clone());
                } else {
                    out.push(value.clone());
                }
            }
        }
    }
    Ok(Value::array(out))
}

fn array_merge(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_merge", &args, 1, None)?;
    let mut out = PhpArray::new();
    for (index, value) in args.iter().enumerate() {
        let array = match value {
            Value::Array(array) => array.clone(),
            other => {
                return Err(type_error_arg(
                    "array_merge",
                    index,
                    "arrays",
                    "array",
                    other,
                ))
            }
        };
        for (key, value) in array.iter() {
            match key {
                Key::Int(_) => {
                    out.push(value.clone());
                }
                Key::Str(_) => {
                    out.insert(key.clone(), value.clone());
                }
            }
        }
    }
    Ok(Value::array(out))
}

fn array_fill(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_fill", &args, 3, Some(3))?;
    let start = as_int(&args, 0, "array_fill", "start_index")?;
    let count = as_int(&args, 1, "array_fill", "count")?;
    if count < 0 {
        return Err(PhpError::value_error(
            "array_fill(): Argument #2 ($count) must be greater than or equal to 0",
        ));
    }
    if count > 0 {
        // php-src rejects counts that do not fit in a 32-bit array size and
        // detects an overflowing next index before allocating.
        if count > i32::MAX as i64 {
            return Err(PhpError::value_error(
                "array_fill(): Argument #2 ($count) is too large",
            ));
        }
        if start > i64::MAX - count + 1 {
            return Err(PhpError::fatal(
                "Cannot add element to the array as the next element is already occupied",
            ));
        }
    }
    let mut out = PhpArray::try_with_capacity(count as usize).ok_or_else(allocation_error)?;
    for i in 0..count {
        out.insert_int(start + i, args[2].clone());
    }
    Ok(Value::array(out))
}

fn array_combine(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_combine", &args, 2, Some(2))?;
    let keys = as_array(&args, 0, "array_combine", "keys")?;
    let values = as_array(&args, 1, "array_combine", "values")?;
    if keys.len() != values.len() {
        return Err(PhpError::value_error(
            "array_combine(): Argument #1 ($keys) and argument #2 ($values) must have the same number of elements",
        ));
    }
    let mut out = PhpArray::new();
    for ((_, key), (_, value)) in keys.iter().zip(values.iter()) {
        out.insert(key_from_value(key)?, value.clone());
    }
    Ok(Value::array(out))
}

fn array_flip(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_flip", &args, 1, Some(1))?;
    let array = as_array(&args, 0, "array_flip", "array")?;
    let mut out = PhpArray::new();
    for (key, value) in array.iter() {
        match value {
            Value::Int(_) | Value::Str(_) => {
                out.insert(key_from_value(value)?, key_to_value(key));
            }
            other => {
                return Err(PhpError::type_error(format!(
                    "array_flip(): Can only flip string and integer values, {} given",
                    other.debug_type_name()
                )))
            }
        }
    }
    Ok(Value::array(out))
}

fn array_map(hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_map", &args, 2, Some(2))?;
    let array = as_array(&args, 1, "array_map", "array")?;
    let callback = match &args[0] {
        Value::Null => None,
        value if hooks.is_callable(value) => Some(args[0].clone()),
        other => {
            return Err(type_error_arg(
                "array_map",
                0,
                "callback",
                "callable",
                other,
            ))
        }
    };
    let mut out = PhpArray::new();
    for (key, value) in array.iter() {
        let mapped = match &callback {
            None => value.clone(),
            Some(callable) => hooks.call_value(callable, vec![value.clone()])?,
        };
        match key {
            Key::Str(_) => {
                out.insert(key.clone(), mapped);
            }
            Key::Int(_) => {
                out.push(mapped);
            }
        }
    }
    Ok(Value::array(out))
}

// ------------------------------------------------------- by-ref array builtins

fn ref_array(
    args: &[Value],
    index: usize,
    fname: &str,
    param: &str,
) -> PhpResult<(Rc<std::cell::RefCell<Value>>, PhpArray)> {
    let cell = match args.get(index) {
        None => {
            return Err(PhpError::argument_count(format!(
                "{fname}(): Argument #{} (${param}) not passed",
                index + 1
            )))
        }
        Some(Value::Ref(cell)) => cell.clone(),
        Some(_) => {
            return Err(PhpError::fatal(format!(
                "{fname}(): Argument #{} (${param}) cannot be passed by reference",
                index + 1
            )))
        }
    };
    let inner = cell.borrow().deref();
    match inner {
        Value::Array(array) => Ok((cell, array.as_ref().clone())),
        other => Err(type_error_arg(fname, index, param, "array", &other)),
    }
}

fn sort_flags(fname: &str, args: &[Value]) -> PhpResult<i64> {
    let flags = if args.len() > 1 {
        as_int(args, 1, fname, "flags")?
    } else {
        0
    };
    let base = flags & !8;
    if !matches!(base, 0 | 1 | 2 | 5) {
        return Err(PhpError::fatal(format!(
            "{fname}(): sort flags {flags} are not supported"
        )));
    }
    Ok(flags)
}

fn numeric_sort_key(value: &Value) -> Numeric {
    match value.deref().to_number() {
        ToNumber::Num(number) | ToNumber::Leading(number) => number,
        ToNumber::Invalid => Numeric::Int(0),
    }
}

fn lower_ascii(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(u8::to_ascii_lowercase).collect()
}

fn compare_sort_values(a: &Value, b: &Value, flags: i64) -> Ordering {
    let base = flags & !8;
    let case_insensitive = flags & 8 != 0;
    match base {
        1 => {
            let (left, right) = (numeric_sort_key(a), numeric_sort_key(b));
            match (left, right) {
                (Numeric::Int(x), Numeric::Int(y)) => x.cmp(&y),
                _ => left
                    .to_float()
                    .partial_cmp(&right.to_float())
                    .unwrap_or(Ordering::Equal),
            }
        }
        2 | 5 => {
            let (left, right) = (a.deref().to_bytes(), b.deref().to_bytes());
            if case_insensitive {
                lower_ascii(&left).cmp(&lower_ascii(&right))
            } else {
                left.cmp(&right)
            }
        }
        _ => compare(a, b).unwrap_or(Ordering::Equal),
    }
}

fn key_as_value(key: &Key) -> Value {
    match key {
        Key::Int(value) => Value::Int(*value),
        Key::Str(value) => Value::Str(value.clone()),
    }
}

fn sort(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("sort", &args, 1, Some(2))?;
    let flags = sort_flags("sort", &args)?;
    let (cell, array) = ref_array(&args, 0, "sort", "array")?;
    let mut values = array.values();
    values.sort_by(|a, b| compare_sort_values(a, b, flags));
    *cell.borrow_mut() = Value::array(PhpArray::from(values));
    Ok(Value::Bool(true))
}

fn rsort(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("rsort", &args, 1, Some(2))?;
    let flags = sort_flags("rsort", &args)?;
    let (cell, array) = ref_array(&args, 0, "rsort", "array")?;
    let mut values = array.values();
    values.sort_by(|a, b| compare_sort_values(b, a, flags));
    *cell.borrow_mut() = Value::array(PhpArray::from(values));
    Ok(Value::Bool(true))
}

fn asort(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("asort", &args, 1, Some(2))?;
    let flags = sort_flags("asort", &args)?;
    let (cell, array) = ref_array(&args, 0, "asort", "array")?;
    let mut pairs: Vec<(Key, Value)> = array
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    pairs.sort_by(|(_, a), (_, b)| compare_sort_values(a, b, flags));
    *cell.borrow_mut() = Value::array(PhpArray::from_pairs(pairs));
    Ok(Value::Bool(true))
}

fn arsort(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("arsort", &args, 1, Some(2))?;
    let flags = sort_flags("arsort", &args)?;
    let (cell, array) = ref_array(&args, 0, "arsort", "array")?;
    let mut pairs: Vec<(Key, Value)> = array
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    pairs.sort_by(|(_, a), (_, b)| compare_sort_values(b, a, flags));
    *cell.borrow_mut() = Value::array(PhpArray::from_pairs(pairs));
    Ok(Value::Bool(true))
}

fn ksort(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("ksort", &args, 1, Some(2))?;
    let flags = sort_flags("ksort", &args)?;
    let (cell, array) = ref_array(&args, 0, "ksort", "array")?;
    let mut pairs: Vec<(Key, Value)> = array
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    pairs.sort_by(|(a, _), (b, _)| compare_sort_values(&key_as_value(a), &key_as_value(b), flags));
    *cell.borrow_mut() = Value::array(PhpArray::from_pairs(pairs));
    Ok(Value::Bool(true))
}

fn krsort(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("krsort", &args, 1, Some(2))?;
    let flags = sort_flags("krsort", &args)?;
    let (cell, array) = ref_array(&args, 0, "krsort", "array")?;
    let mut pairs: Vec<(Key, Value)> = array
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    pairs.sort_by(|(a, _), (b, _)| compare_sort_values(&key_as_value(b), &key_as_value(a), flags));
    *cell.borrow_mut() = Value::array(PhpArray::from_pairs(pairs));
    Ok(Value::Bool(true))
}

fn array_push(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_push", &args, 1, None)?;
    let (cell, mut array) = ref_array(&args, 0, "array_push", "array")?;
    for value in args.iter().skip(1) {
        array.push(value.deref());
    }
    let len = array.len() as i64;
    *cell.borrow_mut() = Value::array(array);
    Ok(Value::Int(len))
}

fn array_pop(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_pop", &args, 1, Some(1))?;
    let (cell, mut array) = ref_array(&args, 0, "array_pop", "array")?;
    let value = array
        .first_key()
        .and_then(|_| array.keys().last().cloned())
        .and_then(|key| array.remove(&key))
        .unwrap_or(Value::Null);
    *cell.borrow_mut() = Value::array(array);
    Ok(value)
}

fn array_shift(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_shift", &args, 1, Some(1))?;
    let (cell, mut array) = ref_array(&args, 0, "array_shift", "array")?;
    let first = array.first_key().cloned();
    let value = first
        .as_ref()
        .and_then(|key| array.remove(key))
        .unwrap_or(Value::Null);
    let mut out = PhpArray::with_capacity(array.len());
    for (key, value) in array.iter() {
        match key {
            Key::Int(_) => {
                out.push(value.deref());
            }
            Key::Str(_) => {
                out.insert(key.clone(), value.deref());
            }
        }
    }
    *cell.borrow_mut() = Value::array(out);
    Ok(value)
}

fn array_unshift(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("array_unshift", &args, 1, None)?;
    let (cell, array) = ref_array(&args, 0, "array_unshift", "array")?;
    let mut out = PhpArray::with_capacity(array.len() + args.len() - 1);
    for value in args.iter().skip(1) {
        out.push(value.deref());
    }
    for (key, value) in array.iter() {
        match key {
            Key::Int(_) => {
                out.push(value.deref());
            }
            Key::Str(_) => {
                out.insert(key.clone(), value.deref());
            }
        }
    }
    let len = out.len() as i64;
    *cell.borrow_mut() = Value::array(out);
    Ok(Value::Int(len))
}

fn range(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("range", &args, 2, Some(3))?;

    let is_char = |value: &Value| -> Option<u8> {
        match value {
            Value::Str(s) if s.as_bytes().len() == 1 => Some(s.as_bytes()[0]),
            _ => None,
        }
    };

    let step_arg = args.get(2);
    let step_int = step_arg.map(Value::to_int_cast).unwrap_or(1);
    let step_is_float = step_arg.is_some_and(|value| {
        matches!(
            value.to_number(),
            ToNumber::Num(Numeric::Float(_)) | ToNumber::Leading(Numeric::Float(_))
        )
    });
    let step_negative = step_arg.is_some_and(|value| match value.to_number() {
        ToNumber::Num(Numeric::Int(step)) | ToNumber::Leading(Numeric::Int(step)) => step < 0,
        ToNumber::Num(Numeric::Float(step)) | ToNumber::Leading(Numeric::Float(step)) => step < 0.0,
        ToNumber::Invalid => false,
    });
    let step_float = step_arg.map(Value::to_float_cast).unwrap_or(1.0);
    if !step_float.is_finite() {
        return Err(PhpError::value_error(
            "range(): Argument #3 ($step) must be a finite number",
        ));
    }
    if step_float == 0.0 {
        return Err(PhpError::value_error(
            "range(): Argument #3 ($step) cannot be 0",
        ));
    }

    // Single-byte string ranges use byte arithmetic, independently of the
    // numeric branch. Keep the step positive and compute by index so that a
    // huge step cannot wrap or loop forever.
    if let (Some(start), Some(end)) = (is_char(&args[0]), is_char(&args[1])) {
        if step_int == i64::MIN {
            return Err(PhpError::value_error(
                "range(): Argument #3 ($step) must be greater than the minimum integer",
            ));
        }
        let step = (step_int as i128).abs();
        let (low, high) = (start as i128, end as i128);
        let (first, last, direction) = if low > high {
            (low, high, -1i128)
        } else if high > low {
            if step_negative {
                return Err(range_negative_step_error());
            }
            (low, high, 1i128)
        } else {
            return Ok(Value::array(PhpArray::from_pairs(vec![(
                Key::Int(0),
                string_value(vec![start]),
            )])));
        };
        let distance = (last - first).unsigned_abs();
        if distance < step as u128 {
            return Err(range_boundary_error());
        }
        let count = distance / step as u128 + 1;
        if count >= HT_MAX_SIZE {
            return Err(range_too_large_error());
        }
        let mut out = PhpArray::try_with_capacity(count as usize).ok_or_else(allocation_error)?;
        for i in 0..count {
            let current = first + direction * (i as i128 * step);
            out.push(string_value(vec![current as u8]));
        }
        return Ok(Value::array(out));
    }

    let start_int = integer_operand(&args[0]);
    let end_int = integer_operand(&args[1]);
    if let (Some(start), Some(end)) = (start_int, end_int) {
        if !step_is_float {
            // Zend's integer range path never converts through f64. Doing so
            // loses the final increment at PHP_INT_MAX and can loop forever.
            if step_int == i64::MIN {
                return Err(PhpError::value_error(
                    "range(): Argument #3 ($step) must be greater than the minimum integer",
                ));
            }
            let step = (step_int as i128).abs();
            if start == end {
                let mut out = PhpArray::new();
                out.push(Value::Int(start));
                return Ok(Value::array(out));
            }
            let (first, last, direction) = if start > end {
                if (start as i128 - end as i128) < step {
                    return Err(range_boundary_error());
                }
                (start as i128, end as i128, -1i128)
            } else {
                if step_negative {
                    return Err(range_negative_step_error());
                }
                if (end as i128 - start as i128) < step {
                    return Err(range_boundary_error());
                }
                (start as i128, end as i128, 1i128)
            };
            let distance = (last - first).unsigned_abs();
            let count = distance / step as u128 + 1;
            if count >= HT_MAX_SIZE {
                return Err(range_too_large_error());
            }
            let mut out =
                PhpArray::try_with_capacity(count as usize).ok_or_else(allocation_error)?;
            for i in 0..count {
                out.push(Value::Int((first + direction * (i as i128 * step)) as i64));
            }
            return Ok(Value::array(out));
        }
    }

    // Float (or mixed numeric/string) ranges. Compute an upper bound instead
    // of repeatedly adding `step` to a float that may no longer change.
    let start = args[0].to_float_cast();
    let end = args[1].to_float_cast();
    let step = step_float.abs();
    if start == end {
        let mut out = PhpArray::new();
        out.push(Value::Float(start));
        return Ok(Value::array(out));
    }
    let (first, last, direction) = if start > end {
        (start, end, -1.0)
    } else {
        if step_negative {
            return Err(range_negative_step_error());
        }
        (start, end, 1.0)
    };
    let distance = (last - first).abs();
    if distance < step {
        return Err(range_boundary_error());
    }
    let upper = (distance / step).floor();
    if !upper.is_finite() || upper >= (HT_MAX_SIZE - 2) as f64 {
        return Err(range_too_large_error());
    }
    let upper = upper as u128 + 2;
    let mut out = PhpArray::try_with_capacity(upper as usize).ok_or_else(allocation_error)?;
    let epsilon = f64::EPSILON * distance.abs().max(1.0);
    for i in 0..upper {
        let element = first + direction * (i as f64 * step);
        if direction > 0.0 {
            if element > last + epsilon {
                break;
            }
        } else if element < last - epsilon {
            break;
        }
        out.push(Value::Float(element));
    }
    Ok(Value::array(out))
}

// --------------------------------------------------------------------- math

fn abs(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("abs", &args, 1, Some(1))?;
    match &args[0] {
        Value::Int(value) => Ok(match value.checked_abs() {
            Some(value) => Value::Int(value),
            None => Value::Float((*value as f64).abs()),
        }),
        Value::Float(value) => Ok(Value::Float(value.abs())),
        other => Ok(Value::Float(other.to_float_cast().abs())),
    }
}

fn ceil(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("ceil", &args, 1, Some(1))?;
    Ok(Value::Float(as_float(&args, 0, "ceil", "num")?.ceil()))
}

fn floor(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("floor", &args, 1, Some(1))?;
    Ok(Value::Float(as_float(&args, 0, "floor", "num")?.floor()))
}

fn round(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("round", &args, 1, Some(2))?;
    let value = as_float(&args, 0, "round", "num")?;
    let precision = if args.len() > 1 {
        as_int(&args, 1, "round", "precision")?
    } else {
        0
    };
    Ok(Value::Float(php_math_round(value, precision)))
}

/// `_php_math_round()` from `ext/standard/math.c`, default `PHP_ROUND_HALF_UP`.
///
/// The naive `(value * 10^places).round() / 10^places` is wrong for values such
/// as `1.005` because the multiplication loses precision. PHP compensates by
/// testing whether the adjacent integral value maps back to the same input.
fn php_math_round(value: f64, places: i64) -> f64 {
    if !value.is_finite() || value == 0.0 {
        return value;
    }
    if places == 0 && value == value.trunc() {
        return value;
    }

    let places = places.clamp(i32::MIN as i64 + 1, i32::MAX as i64);
    let exponent = 10f64.powi(places.unsigned_abs().min(i32::MAX as u64) as i32);

    let scaled = if places > 0 {
        value * exponent
    } else {
        value / exponent
    };
    let (mut integral, neighbour) = if value >= 0.0 {
        let integral = scaled.floor();
        (integral, integral + 1.0)
    } else {
        let integral = scaled.ceil();
        (integral, integral - 1.0)
    };

    let restored = if places > 0 {
        neighbour / exponent
    } else {
        neighbour * exponent
    };
    if restored == value {
        integral = neighbour;
    }

    if integral.abs() >= 1e16 {
        return value;
    }

    // PHP_ROUND_HALF_UP.
    let edge_case = if places > 0 {
        ((integral + 0.5f64.copysign(integral)) / exponent).abs()
    } else {
        ((integral + 0.5f64.copysign(integral)) * exponent).abs()
    };
    if value.abs() >= edge_case {
        integral += 1.0f64.copysign(integral);
    }

    if places.unsigned_abs() < 23 {
        if places > 0 {
            integral / exponent
        } else {
            integral * exponent
        }
    } else {
        let text = format!("{integral:e}{}", -places);
        text.parse().unwrap_or(value)
    }
}

fn sqrt(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("sqrt", &args, 1, Some(1))?;
    let value = as_float(&args, 0, "sqrt", "num")?;
    if value < 0.0 {
        return Ok(Value::Float(f64::NAN));
    }
    Ok(Value::Float(value.sqrt()))
}

fn pow(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("pow", &args, 2, Some(2))?;
    if let (Value::Int(base), Value::Int(exponent)) = (&args[0], &args[1]) {
        if *exponent >= 0 {
            if let Ok(exponent) = u32::try_from(*exponent) {
                if let Some(result) = base.checked_pow(exponent) {
                    return Ok(Value::Int(result));
                }
            }
        }
    }
    let base = args[0].to_float_cast();
    let exponent = args[1].to_float_cast();
    Ok(Value::Float(base.powf(exponent)))
}

fn intdiv(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("intdiv", &args, 2, Some(2))?;
    let a = as_int(&args, 0, "intdiv", "num1")?;
    let b = as_int(&args, 1, "intdiv", "num2")?;
    if b == 0 {
        return Err(PhpError::arithmetic("Division by zero"));
    }
    a.checked_div(b)
        .map(Value::Int)
        .ok_or_else(|| PhpError::arithmetic("Division of PHP_INT_MIN by -1 is not an integer"))
}

fn fmod(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("fmod", &args, 2, Some(2))?;
    let a = as_float(&args, 0, "fmod", "num1")?;
    let b = as_float(&args, 1, "fmod", "num2")?;
    Ok(Value::Float(a % b))
}

fn extremum(args: &[Value], want_max: bool, name: &str) -> PhpResult<Value> {
    arity(name, args, 1, None)?;
    let values: Vec<Value> = if args.len() == 1 {
        match &args[0] {
            Value::Array(array) => {
                if array.is_empty() {
                    return Err(PhpError::value_error(format!(
                        "{name}(): Argument #1 ($value) must contain at least one element"
                    )));
                }
                array.values()
            }
            other => {
                return Err(type_error_arg(name, 0, "value", "array", other));
            }
        }
    } else {
        args.to_vec()
    };
    let mut best = values[0].clone();
    for value in &values[1..] {
        let ordering = compare(value, &best).unwrap_or(std::cmp::Ordering::Equal);
        let better = if want_max {
            ordering == std::cmp::Ordering::Greater
        } else {
            ordering == std::cmp::Ordering::Less
        };
        if better {
            best = value.clone();
        }
    }
    Ok(best)
}

fn max(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    extremum(&args, true, "max")
}

fn min(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    extremum(&args, false, "min")
}

fn log(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("log", &args, 1, Some(2))?;
    let number = as_float(&args, 0, "log", "num")?;
    let result = if args.len() > 1 {
        number.log(as_float(&args, 1, "log", "base")?)
    } else {
        number.ln()
    };
    Ok(Value::Float(result))
}

fn log10(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("log10", &args, 1, Some(1))?;
    Ok(Value::Float(as_float(&args, 0, "log10", "num")?.log10()))
}

fn exp(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("exp", &args, 1, Some(1))?;
    Ok(Value::Float(as_float(&args, 0, "exp", "num")?.exp()))
}

fn sin(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("sin", &args, 1, Some(1))?;
    Ok(Value::Float(as_float(&args, 0, "sin", "num")?.sin()))
}

fn cos(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("cos", &args, 1, Some(1))?;
    Ok(Value::Float(as_float(&args, 0, "cos", "num")?.cos()))
}

fn tan(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("tan", &args, 1, Some(1))?;
    Ok(Value::Float(as_float(&args, 0, "tan", "num")?.tan()))
}

fn asin(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("asin", &args, 1, Some(1))?;
    Ok(Value::Float(as_float(&args, 0, "asin", "num")?.asin()))
}

fn acos(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("acos", &args, 1, Some(1))?;
    Ok(Value::Float(as_float(&args, 0, "acos", "num")?.acos()))
}

fn atan(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("atan", &args, 1, Some(1))?;
    Ok(Value::Float(as_float(&args, 0, "atan", "num")?.atan()))
}

fn atan2(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("atan2", &args, 2, Some(2))?;
    Ok(Value::Float(
        as_float(&args, 0, "atan2", "y")?.atan2(as_float(&args, 1, "atan2", "x")?),
    ))
}

fn hypot(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("hypot", &args, 2, Some(2))?;
    Ok(Value::Float(
        as_float(&args, 0, "hypot", "x")?.hypot(as_float(&args, 1, "hypot", "y")?),
    ))
}

fn deg2rad(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("deg2rad", &args, 1, Some(1))?;
    Ok(Value::Float(
        as_float(&args, 0, "deg2rad", "num")?.to_radians(),
    ))
}

fn rad2deg(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("rad2deg", &args, 1, Some(1))?;
    Ok(Value::Float(
        as_float(&args, 0, "rad2deg", "num")?.to_degrees(),
    ))
}

fn fdiv(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("fdiv", &args, 2, Some(2))?;
    let numerator = as_float(&args, 0, "fdiv", "num1")?;
    let denominator = as_float(&args, 1, "fdiv", "num2")?;
    Ok(Value::Float(numerator / denominator))
}

fn is_nan(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("is_nan", &args, 1, Some(1))?;
    Ok(Value::Bool(as_float(&args, 0, "is_nan", "num")?.is_nan()))
}

fn is_finite(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("is_finite", &args, 1, Some(1))?;
    Ok(Value::Bool(
        as_float(&args, 0, "is_finite", "num")?.is_finite(),
    ))
}

fn is_infinite(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("is_infinite", &args, 1, Some(1))?;
    Ok(Value::Bool(
        as_float(&args, 0, "is_infinite", "num")?.is_infinite(),
    ))
}

fn pi(_hooks: &mut dyn RuntimeHooks, _args: Vec<Value>) -> PhpResult<Value> {
    Ok(Value::Float(std::f64::consts::PI))
}

// -------------------------------------------------------------------- types

fn gettype(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("gettype", &args, 1, Some(1))?;
    let value = args[0].deref();
    Ok(Value::from_str_lit(match &value {
        Value::Null => "NULL",
        Value::Bool(_) => "boolean",
        Value::Int(_) => "integer",
        Value::Float(_) => "double",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
        Value::Closure(_) | Value::Object(_) => "object",
        Value::Ref(_) => unreachable!("deref removes reference cells"),
    }))
}

fn get_debug_type(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("get_debug_type", &args, 1, Some(1))?;
    let name = args[0].debug_type_name();
    Ok(Value::from_str_lit(&name))
}

fn get_class(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("get_class", &args, 1, Some(1))?;
    match &args[0] {
        Value::Closure(_) => Ok(Value::from_str_lit("Closure")),
        Value::Object(object) => Ok(Value::str(object.borrow().class_name.as_bytes().to_vec())),
        other => Err(PhpError::type_error(format!(
            "get_class(): Argument #1 ($object) must be of type object, {} given",
            other.debug_type_name()
        ))),
    }
}

fn intval(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("intval", &args, 1, Some(2))?;
    let base = if args.len() > 1 {
        as_int(&args, 1, "intval", "base")?
    } else {
        10
    };
    match (&args[0], base) {
        (Value::Str(s), base) if base != 10 => {
            let text = String::from_utf8_lossy(s.as_bytes());
            let trimmed = text.trim();
            let parsed = match base {
                2 => i64::from_str_radix(trimmed.trim_start_matches("0b"), 2),
                8 => i64::from_str_radix(trimmed.trim_start_matches("0o"), 8),
                16 => i64::from_str_radix(trimmed.trim_start_matches("0x"), 16),
                _ => Ok(0),
            };
            Ok(Value::Int(parsed.unwrap_or(0)))
        }
        _ => Ok(Value::Int(args[0].to_int_cast())),
    }
}

fn floatval(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("floatval", &args, 1, Some(1))?;
    Ok(Value::Float(args[0].to_float_cast()))
}

fn strval(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("strval", &args, 1, Some(1))?;
    if matches!(args[0], Value::Closure(_)) {
        return Err(PhpError::fatal(
            "Object of class Closure could not be converted to string",
        ));
    }
    Ok(string_value(args[0].to_bytes()))
}

fn boolval(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("boolval", &args, 1, Some(1))?;
    Ok(Value::Bool(args[0].to_bool()))
}

macro_rules! type_predicate {
    ($name:ident, $label:expr, $pattern:pat) => {
        fn $name(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
            arity($label, &args, 1, Some(1))?;
            Ok(Value::Bool(matches!(&args[0], $pattern)))
        }
    };
}

type_predicate!(is_null, "is_null", Value::Null);
type_predicate!(is_int, "is_int", Value::Int(_));
type_predicate!(is_float, "is_float", Value::Float(_));
type_predicate!(is_string, "is_string", Value::Str(_));
type_predicate!(is_bool, "is_bool", Value::Bool(_));
type_predicate!(is_array, "is_array", Value::Array(_));
type_predicate!(is_object, "is_object", Value::Closure(_) | Value::Object(_));

fn is_numeric(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("is_numeric", &args, 1, Some(1))?;
    let result = match &args[0] {
        Value::Int(_) | Value::Float(_) => true,
        Value::Str(s) => matches!(parse_numeric(&s.0), Some((_, true))),
        _ => false,
    };
    Ok(Value::Bool(result))
}

fn is_scalar(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("is_scalar", &args, 1, Some(1))?;
    Ok(Value::Bool(args[0].is_scalar()))
}

fn is_callable(hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("is_callable", &args, 1, Some(3))?;
    Ok(Value::Bool(hooks.is_callable(&args[0])))
}

fn call_user_func(hooks: &mut dyn RuntimeHooks, mut args: Vec<Value>) -> PhpResult<Value> {
    arity("call_user_func", &args, 1, None)?;
    let callable = args.remove(0);
    // Dynamic calls cannot know the callee signature. Give each value a
    // temporary reference cell so by-reference parameters can still bind,
    // matching PHP's "value given" warning path without changing the caller.
    let args = args
        .into_iter()
        .map(|value| {
            if value.as_ref_cell().is_some() {
                value
            } else {
                Value::reference(value.deref())
            }
        })
        .collect();
    hooks.call_value(&callable, args)
}

fn call_user_func_array(hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("call_user_func_array", &args, 2, Some(2))?;
    let array = as_array(&args, 1, "call_user_func_array", "args")?;
    let values = array
        .iter()
        .map(|(_, value)| {
            if value.as_ref_cell().is_some() {
                value.clone()
            } else {
                Value::reference(value.deref())
            }
        })
        .collect();
    hooks.call_value(&args[0], values)
}

fn function_exists(hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("function_exists", &args, 1, Some(1))?;
    let name = as_string(&args, 0, "function_exists", "function")?;
    Ok(Value::Bool(
        hooks.function_exists(&String::from_utf8_lossy(&name)),
    ))
}

// ------------------------------------------------------------------- output

fn dump_container_id(value: &Value) -> Option<usize> {
    match value {
        Value::Array(array) => Some(Rc::as_ptr(array) as usize),
        Value::Object(object) => Some(Rc::as_ptr(object) as usize),
        _ => None,
    }
}

fn dump_value(
    value: &Value,
    depth: usize,
    mark_ref: bool,
    active: &mut HashSet<usize>,
    out: &mut Vec<u8>,
) {
    let is_ref = mark_ref && matches!(value, Value::Ref(_));
    let deref = value.deref();
    let value = &deref;
    let container = dump_container_id(value);
    if let Some(id) = container {
        if !active.insert(id) {
            out.extend(std::iter::repeat_n(b' ', depth * 2));
            out.extend_from_slice(b"*RECURSION*\n");
            return;
        }
    }
    out.extend(std::iter::repeat_n(b' ', depth * 2));
    if is_ref {
        out.push(b'&');
    }
    match value {
        Value::Null => out.extend_from_slice(b"NULL\n"),
        Value::Bool(true) => out.extend_from_slice(b"bool(true)\n"),
        Value::Bool(false) => out.extend_from_slice(b"bool(false)\n"),
        Value::Int(number) => out.extend_from_slice(format!("int({number})\n").as_bytes()),
        Value::Float(number) => {
            out.extend_from_slice(format!("float({})\n", float_to_repr(*number)).as_bytes())
        }
        Value::Str(string) => {
            out.extend_from_slice(format!("string({}) \"", string.as_bytes().len()).as_bytes());
            out.extend_from_slice(string.as_bytes());
            out.extend_from_slice(b"\"\n");
        }
        Value::Array(array) => {
            out.extend_from_slice(format!("array({}) {{\n", array.len()).as_bytes());
            for (key, item) in array.iter() {
                out.extend(std::iter::repeat_n(b' ', (depth + 1) * 2));
                match key {
                    Key::Int(number) => {
                        out.extend_from_slice(format!("[{number}]=>\n").as_bytes());
                    }
                    Key::Str(string) => {
                        out.extend_from_slice(b"[\"");
                        out.extend_from_slice(string.as_bytes());
                        out.extend_from_slice(b"\"]=>\n");
                    }
                }
                dump_value(item, depth + 1, true, active, out);
            }
            out.extend(std::iter::repeat_n(b' ', depth * 2));
            out.extend_from_slice(b"}\n");
        }
        Value::Object(object) => {
            let object = object.borrow();
            out.extend_from_slice(
                format!(
                    "object({})#{} ({}) {{\n",
                    object.class_name,
                    object.id,
                    object.properties.len()
                )
                .as_bytes(),
            );
            for (key, item) in object.properties.iter() {
                out.extend(std::iter::repeat_n(b' ', (depth + 1) * 2));
                match key {
                    Key::Int(number) => {
                        out.extend_from_slice(format!("[{number}]=>\n").as_bytes());
                    }
                    Key::Str(string) => {
                        out.extend_from_slice(b"[\"");
                        out.extend_from_slice(string.as_bytes());
                        let name = string.to_string_lossy();
                        match object.property_visibility.get(&name).map(String::as_str) {
                            Some("protected") => out.extend_from_slice(b"\":protected"),
                            Some("private") => {
                                out.extend_from_slice(b"\":\"");
                                out.extend_from_slice(object.class_name.as_bytes());
                                out.extend_from_slice(b"\":private");
                            }
                            _ => out.push(b'"'),
                        }
                        out.extend_from_slice(b"]=>\n");
                    }
                }
                dump_value(item, depth + 1, true, active, out);
            }
            out.extend(std::iter::repeat_n(b' ', depth * 2));
            out.extend_from_slice(b"}\n");
        }
        Value::Closure(closure) if closure.callable.is_some() => {
            let info = closure.callable.as_ref().expect("checked above");
            let properties =
                1 + usize::from(info.this.is_some()) + usize::from(!closure.parameters.is_empty());
            out.extend_from_slice(
                format!("object(Closure)#{} ({properties}) {{\n", closure.id).as_bytes(),
            );
            let property_indent = (depth + 1) * 2;
            out.extend(std::iter::repeat_n(b' ', property_indent));
            out.extend_from_slice(b"[\"function\"]=>\n");
            dump_value(
                &Value::from_str_lit(&info.function),
                depth + 1,
                false,
                active,
                out,
            );
            if let Some(this) = &info.this {
                out.extend(std::iter::repeat_n(b' ', property_indent));
                out.extend_from_slice(b"[\"this\"]=>\n");
                dump_value(this, depth + 1, false, active, out);
            }
            if !closure.parameters.is_empty() {
                out.extend(std::iter::repeat_n(b' ', property_indent));
                out.extend_from_slice(b"[\"parameter\"]=>\n");
                let mut parameters = PhpArray::new();
                for (name, optional) in &closure.parameters {
                    parameters.insert(
                        Key::from_str(&format!("${name}")),
                        Value::from_str_lit(if *optional {
                            "<optional>"
                        } else {
                            "<required>"
                        }),
                    );
                }
                dump_value(&Value::array(parameters), depth + 1, false, active, out);
            }
            out.extend(std::iter::repeat_n(b' ', depth * 2));
            out.extend_from_slice(b"}\n");
        }
        Value::Closure(closure) => {
            let properties = 3
                + usize::from(!closure.captures.is_empty())
                + usize::from(!closure.parameters.is_empty());
            out.extend_from_slice(
                format!("object(Closure)#{} ({properties}) {{\n", closure.id).as_bytes(),
            );
            let property_indent = (depth + 1) * 2;
            out.extend(std::iter::repeat_n(b' ', property_indent));
            out.extend_from_slice(b"[\"name\"]=>\n");
            dump_value(
                &Value::from_str_lit(&closure.name),
                depth + 1,
                false,
                active,
                out,
            );
            out.extend(std::iter::repeat_n(b' ', property_indent));
            out.extend_from_slice(b"[\"file\"]=>\n");
            dump_value(
                &Value::from_str_lit(&closure.file),
                depth + 1,
                false,
                active,
                out,
            );
            out.extend(std::iter::repeat_n(b' ', property_indent));
            out.extend_from_slice(b"[\"line\"]=>\n");
            dump_value(
                &Value::Int(i64::from(closure.line)),
                depth + 1,
                false,
                active,
                out,
            );
            if !closure.captures.is_empty() {
                out.extend(std::iter::repeat_n(b' ', property_indent));
                out.extend_from_slice(b"[\"static\"]=>\n");
                let mut captured = PhpArray::new();
                for (name, value) in closure.capture_names.iter().zip(closure.captures.iter()) {
                    captured.insert(Key::from_str(name), value.clone());
                }
                dump_value(&Value::array(captured), depth + 1, false, active, out);
            }
            if !closure.parameters.is_empty() {
                out.extend(std::iter::repeat_n(b' ', property_indent));
                out.extend_from_slice(b"[\"parameter\"]=>\n");
                let mut parameters = PhpArray::new();
                for (name, optional) in &closure.parameters {
                    parameters.insert(
                        Key::from_str(&format!("${name}")),
                        Value::from_str_lit(if *optional {
                            "<optional>"
                        } else {
                            "<required>"
                        }),
                    );
                }
                dump_value(&Value::array(parameters), depth + 1, false, active, out);
            }
            out.extend(std::iter::repeat_n(b' ', depth * 2));
            out.extend_from_slice(b"}\n");
        }
        Value::Ref(_) => unreachable!("deref removes reference cells"),
    }
    if let Some(id) = container {
        active.remove(&id);
    }
}

fn var_dump(hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    let mut out = Vec::new();
    for value in &args {
        let mut active = HashSet::new();
        dump_value(value, 0, false, &mut active, &mut out);
    }
    hooks.write(&out);
    Ok(Value::Null)
}

fn print_array_body(
    array: &PhpArray,
    level: usize,
    active: &mut HashSet<usize>,
    out: &mut Vec<u8>,
) {
    out.extend(std::iter::repeat_n(b' ', level * 4));
    out.extend_from_slice(b"(\n");
    for (key, value) in array.iter() {
        out.extend(std::iter::repeat_n(b' ', (level + 1) * 4));
        out.push(b'[');
        out.extend_from_slice(&key.as_display());
        out.extend_from_slice(b"] => ");
        let deref = value.deref();
        match &deref {
            Value::Array(child) => {
                print_array(child, level + 2, active, out);
                // Nested arrays already end with a newline; PHP leaves a blank
                // line before the next element.
                out.push(b'\n');
            }
            Value::Closure(child) => {
                print_closure(child, level + 2, active, out);
                out.push(b'\n');
            }
            Value::Object(child) => {
                print_object(child, level + 2, active, out);
                out.push(b'\n');
            }
            Value::Null => out.push(b'\n'),
            Value::Bool(true) => {
                out.push(b'1');
                out.push(b'\n');
            }
            Value::Bool(false) => out.push(b'\n'),
            other => {
                out.extend_from_slice(&other.to_bytes());
                out.push(b'\n');
            }
        }
    }
    out.extend(std::iter::repeat_n(b' ', level * 4));
    out.extend_from_slice(b")\n");
}

fn print_array(array: &PhpArray, level: usize, active: &mut HashSet<usize>, out: &mut Vec<u8>) {
    let id = array as *const PhpArray as usize;
    if !active.insert(id) {
        out.extend_from_slice(b"*RECURSION*\n");
        return;
    }
    out.extend_from_slice(b"Array\n");
    print_array_body(array, level, active, out);
    active.remove(&id);
}

fn print_object(
    object: &std::cell::RefCell<PhpObject>,
    level: usize,
    active: &mut HashSet<usize>,
    out: &mut Vec<u8>,
) {
    let id = object as *const std::cell::RefCell<PhpObject> as usize;
    if !active.insert(id) {
        out.extend_from_slice(b"*RECURSION*\n");
        return;
    }
    let object = object.borrow();
    out.extend_from_slice(object.class_name.as_bytes());
    out.extend_from_slice(b" Object\n");
    out.extend(std::iter::repeat_n(b' ', level * 4));
    out.extend_from_slice(b"(\n");
    for (key, value) in object.properties.iter() {
        out.extend(std::iter::repeat_n(b' ', (level + 1) * 4));
        out.push(b'[');
        out.extend_from_slice(&key.as_display());
        if let Key::Str(string) = key {
            let name = string.to_string_lossy();
            match object.property_visibility.get(&name).map(String::as_str) {
                Some("protected") => out.extend_from_slice(b":protected"),
                Some("private") => {
                    out.push(b':');
                    out.extend_from_slice(object.class_name.as_bytes());
                    out.extend_from_slice(b":private");
                }
                _ => {}
            }
        }
        out.extend_from_slice(b"] => ");
        let deref = value.deref();
        match &deref {
            Value::Array(child) => {
                print_array(child, level + 2, active, out);
                out.push(b'\n');
            }
            Value::Closure(child) => {
                print_closure(child, level + 2, active, out);
                out.push(b'\n');
            }
            Value::Object(child) => {
                print_object(child, level + 2, active, out);
                out.push(b'\n');
            }
            Value::Null => out.push(b'\n'),
            Value::Bool(true) => {
                out.push(b'1');
                out.push(b'\n');
            }
            Value::Bool(false) => out.push(b'\n'),
            other => {
                out.extend_from_slice(&other.to_bytes());
                out.push(b'\n');
            }
        }
    }
    out.extend(std::iter::repeat_n(b' ', level * 4));
    out.extend_from_slice(b")\n");
    drop(object);
    active.remove(&id);
}

fn print_closure(
    closure: &PhpClosure,
    level: usize,
    active: &mut HashSet<usize>,
    out: &mut Vec<u8>,
) {
    out.extend_from_slice(b"Closure Object\n");
    let mut properties = PhpArray::new();
    if let Some(info) = &closure.callable {
        properties.insert_str("function", Value::from_str_lit(&info.function));
        if let Some(this) = &info.this {
            properties.insert_str("this", this.clone());
        }
    } else {
        properties.insert_str("name", Value::from_str_lit(&closure.name));
        properties.insert_str("file", Value::from_str_lit(&closure.file));
        properties.insert_str("line", Value::Int(i64::from(closure.line)));
        if !closure.captures.is_empty() {
            let mut captures = PhpArray::new();
            for (name, value) in closure.capture_names.iter().zip(closure.captures.iter()) {
                captures.insert(Key::from_str(name), value.clone());
            }
            properties.insert_str("static", Value::array(captures));
        }
    }
    if !closure.parameters.is_empty() {
        let mut parameters = PhpArray::new();
        for (name, optional) in &closure.parameters {
            parameters.insert(
                Key::from_str(&format!("${name}")),
                Value::from_str_lit(if *optional {
                    "<optional>"
                } else {
                    "<required>"
                }),
            );
        }
        properties.insert_str("parameter", Value::array(parameters));
    }
    print_array_body(&properties, level, active, out);
}

fn print_r(hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("print_r", &args, 1, Some(2))?;
    let mut out = Vec::new();
    let mut active = HashSet::new();
    match &args[0] {
        Value::Array(array) => print_array(array, 0, &mut active, &mut out),
        Value::Closure(closure) => print_closure(closure, 0, &mut active, &mut out),
        Value::Object(object) => print_object(object, 0, &mut active, &mut out),
        Value::Null => {}
        Value::Bool(true) => out.push(b'1'),
        Value::Bool(false) => {}
        other => out.extend_from_slice(&other.to_bytes()),
    }
    if args.get(1).is_some_and(Value::to_bool) {
        Ok(string_value(out))
    } else {
        hooks.write(&out);
        Ok(Value::Bool(true))
    }
}

fn define(hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("define", &args, 2, Some(3))?;
    let name = as_string(&args, 0, "define", "constant_name")?;
    let name = String::from_utf8_lossy(&name).into_owned();
    hooks.define_constant(&name, args[1].clone());
    Ok(Value::Bool(true))
}

fn defined(hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("defined", &args, 1, Some(1))?;
    let name = as_string(&args, 0, "defined", "constant_name")?;
    let name = String::from_utf8_lossy(&name).into_owned();
    Ok(Value::Bool(hooks.constant(&name).is_some()))
}

fn constant_value(hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("constant", &args, 1, Some(1))?;
    let name = as_string(&args, 0, "constant", "name")?;
    let name = String::from_utf8_lossy(&name).into_owned();
    hooks
        .constant(&name)
        .ok_or_else(|| PhpError::fatal(format!("Undefined constant \"{name}\"")))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NullHooks {
        output: Vec<u8>,
    }

    impl RuntimeHooks for NullHooks {
        fn write(&mut self, bytes: &[u8]) {
            self.output.extend_from_slice(bytes);
        }
        fn warn(&mut self, _message: &str) {}
        fn call(&mut self, _name: &str, _args: Vec<Value>) -> PhpResult<Value> {
            Ok(Value::Null)
        }
        fn function_exists(&mut self, _name: &str) -> bool {
            false
        }
        fn constant(&mut self, _name: &str) -> Option<Value> {
            None
        }
        fn define_constant(&mut self, _name: &str, _value: Value) {}
    }

    #[test]
    #[allow(clippy::approx_constant)] // 3.14159 is test input, not an approximation of PI.
    fn sprintf_matches_php() {
        let mut hooks = NullHooks { output: vec![] };
        let out = sprintf(
            &mut hooks,
            vec![
                Value::str(b"%05.2f|%-6s|%+d|%x|%X|%o|%b|%c|%e|%E|%g|%u|%%".to_vec()),
                Value::Float(3.14159),
                Value::str(b"ab".to_vec()),
                Value::Int(42),
                Value::Int(255),
                Value::Int(255),
                Value::Int(8),
                Value::Int(5),
                Value::Int(65),
                Value::Float(1234.5678),
                Value::Float(1234.5678),
                Value::Float(1234.5678),
                Value::Int(-1),
            ],
        )
        .unwrap();
        assert_eq!(
            out.to_bytes(),
            b"03.14|ab    |+42|ff|FF|10|101|A|1.234568e+3|1.234568E+3|1234.57|18446744073709551615|%"
        );
        assert!(hooks.output.is_empty());
    }

    #[test]
    fn print_r_nested_matches_php() {
        let mut inner = PhpArray::new();
        inner.push(Value::Int(1));
        inner.push(Value::Int(2));
        let mut outer = PhpArray::new();
        outer.insert_str("a", Value::array(inner));
        outer.insert_str("b", Value::Int(2));

        let mut hooks = NullHooks { output: vec![] };
        let out = print_r(&mut hooks, vec![Value::array(outer), Value::Bool(true)]).unwrap();
        assert_eq!(
            String::from_utf8(out.to_bytes()).unwrap(),
            "Array\n(\n    [a] => Array\n        (\n            [0] => 1\n            [1] => 2\n        )\n\n    [b] => 2\n)\n"
        );
    }

    #[test]
    fn number_format_rounding_and_precision_limits() {
        let mut hooks = NullHooks { output: vec![] };
        let formatted =
            number_format(&mut hooks, vec![Value::Float(1.005), Value::Int(2)]).unwrap();
        assert_eq!(formatted.to_bytes(), b"1.01");

        let formatted = number_format(&mut hooks, vec![Value::Float(1.5), Value::Int(30)]).unwrap();
        assert_eq!(formatted.to_bytes(), b"1.500000000000000000000000000000");

        assert!(
            number_format(&mut hooks, vec![Value::Float(1.5), Value::Int(200_000_000)]).is_err()
        );
        let formatted = sprintf(
            &mut hooks,
            vec![Value::str(b"%.100000f".to_vec()), Value::Float(1.5)],
        )
        .unwrap();
        // php_sprintf() truncates floating-point precision to 53 digits.
        assert_eq!(formatted.to_bytes().len(), 55);
        let formatted = sprintf(
            &mut hooks,
            vec![Value::str(b"%.100000e".to_vec()), Value::Float(1.5)],
        )
        .unwrap();
        // PHP truncates floating-point precision to 53 digits.
        assert_eq!(formatted.to_bytes().len(), 58);

        // number_format() does not truncate; it emits the exact expansion.
        let formatted =
            number_format(&mut hooks, vec![Value::Float(1.5), Value::Int(100_000)]).unwrap();
        assert_eq!(formatted.to_bytes().len(), 100_002);
    }

    #[test]
    fn stdlib_batch_boundaries_fail_cleanly() {
        let mut hooks = NullHooks { output: vec![] };
        assert!(array_pad(
            &mut hooks,
            vec![
                Value::array(PhpArray::new()),
                Value::Int(i64::MAX),
                Value::Int(0)
            ]
        )
        .is_err());
        assert!(array_pad(
            &mut hooks,
            vec![
                Value::array(PhpArray::new()),
                Value::Int(i64::MIN),
                Value::Int(0)
            ]
        )
        .is_err());
        assert!(array_chunk(
            &mut hooks,
            vec![Value::array(PhpArray::new()), Value::Int(0)]
        )
        .is_err());
        assert!(substr_count(
            &mut hooks,
            vec![Value::str(b"abc".to_vec()), Value::str(Vec::new())]
        )
        .is_err());
        assert!(strpbrk(
            &mut hooks,
            vec![Value::str(b"abc".to_vec()), Value::str(Vec::new())]
        )
        .is_err());

        let chunked = array_chunk(
            &mut hooks,
            vec![
                Value::array(PhpArray::from_pairs(vec![
                    (Key::Int(0), Value::Int(1)),
                    (Key::Int(1), Value::Int(2)),
                ])),
                Value::Int(i64::MAX),
            ],
        )
        .unwrap();
        match chunked {
            Value::Array(chunks) => {
                assert_eq!(chunks.len(), 1);
                match chunks.get_int(0) {
                    Some(Value::Array(chunk)) => assert_eq!(chunk.len(), 2),
                    other => panic!("expected chunk, got {other:?}"),
                }
            }
            other => panic!("expected array, got {other:?}"),
        }
    }

    #[test]
    fn extreme_integer_arguments_fail_cleanly() {
        let mut hooks = NullHooks { output: vec![] };

        // These used to panic or abort the Rust process.
        assert!(str_repeat(
            &mut hooks,
            vec![Value::str(b"a".to_vec()), Value::Int(i64::MAX)]
        )
        .is_err());
        assert!(str_pad(
            &mut hooks,
            vec![Value::str(b"a".to_vec()), Value::Int(i64::MAX)]
        )
        .is_err());
        assert!(array_fill(
            &mut hooks,
            vec![Value::Int(0), Value::Int(i64::MAX), Value::Int(1)]
        )
        .is_err());
        assert!(array_fill(
            &mut hooks,
            vec![Value::Int(i64::MAX), Value::Int(2), Value::Int(1)]
        )
        .is_err());
        assert!(number_format(&mut hooks, vec![Value::Float(1.5), Value::Int(i64::MAX)]).is_err());
        assert!(sprintf(
            &mut hooks,
            vec![
                Value::str(b"%999999999999999999s".to_vec()),
                Value::str(b"x".to_vec())
            ]
        )
        .is_err());

        // Negative limits and large positive lengths must saturate, not wrap.
        let parts = explode(
            &mut hooks,
            vec![
                Value::str(b",".to_vec()),
                Value::str(b"a,b".to_vec()),
                Value::Int(i64::MIN),
            ],
        )
        .unwrap();
        match parts {
            Value::Array(parts) => assert!(parts.is_empty()),
            other => panic!("expected array, got {other:?}"),
        }
        assert_eq!(
            substr(
                &mut hooks,
                vec![
                    Value::str(b"abc".to_vec()),
                    Value::Int(1),
                    Value::Int(i64::MAX)
                ]
            )
            .unwrap()
            .to_bytes(),
            b"bc"
        );
        let slice = array_slice(
            &mut hooks,
            vec![
                Value::array(PhpArray::from_pairs(vec![
                    (Key::Int(0), Value::Int(1)),
                    (Key::Int(1), Value::Int(2)),
                    (Key::Int(2), Value::Int(3)),
                ])),
                Value::Int(1),
                Value::Int(i64::MAX),
            ],
        )
        .unwrap();
        match slice {
            Value::Array(slice) => assert_eq!(slice.len(), 2),
            other => panic!("expected array, got {other:?}"),
        }
    }

    #[test]
    fn range_handles_integer_boundaries() {
        let mut hooks = NullHooks { output: vec![] };
        let ranged = range(
            &mut hooks,
            vec![Value::Int(i64::MAX - 2), Value::Int(i64::MAX)],
        )
        .unwrap();
        match ranged {
            Value::Array(array) => {
                assert_eq!(array.len(), 3);
                assert!(matches!(array.get_int(0), Some(Value::Int(v)) if *v == i64::MAX - 2));
                assert!(matches!(array.get_int(1), Some(Value::Int(v)) if *v == i64::MAX - 1));
                assert!(matches!(array.get_int(2), Some(Value::Int(v)) if *v == i64::MAX));
            }
            other => panic!("expected array, got {other:?}"),
        }
        assert!(range(&mut hooks, vec![Value::Int(0), Value::Int(i64::MAX)]).is_err());
    }

    #[test]
    fn var_dump_detects_reference_cycles() {
        let cell = Rc::new(std::cell::RefCell::new(Value::Null));
        let mut array = PhpArray::new();
        array.push(Value::Ref(cell.clone()));
        *cell.borrow_mut() = Value::array(array.clone());
        let mut hooks = NullHooks { output: vec![] };
        var_dump(&mut hooks, vec![Value::Ref(cell)]).unwrap();
        assert_eq!(
            String::from_utf8(hooks.output).unwrap(),
            "array(1) {\n  [0]=>\n  *RECURSION*\n}\n"
        );
    }

    #[test]
    fn var_dump_format() {
        let mut array = PhpArray::new();
        array.push(Value::Int(1));
        array.insert_str("a", Value::str(b"foo".to_vec()));
        let mut hooks = NullHooks { output: vec![] };
        var_dump(&mut hooks, vec![Value::array(array)]).unwrap();
        assert_eq!(
            String::from_utf8(hooks.output).unwrap(),
            "array(2) {\n  [0]=>\n  int(1)\n  [\"a\"]=>\n  string(3) \"foo\"\n}\n"
        );
    }
}
