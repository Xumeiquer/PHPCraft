//! PHP builtin functions.
//!
//! Each entry of [`REGISTRY`] corresponds to a `PHP_FUNCTION(...)` in php-src
//! (`ext/standard/*.c`, `Zend/zend_builtin_functions.c`). Names are matched
//! case-insensitively, like PHP function names.
//!
//! Functions receive `&mut dyn RuntimeHooks`, so they can write output, warn,
//! call back into userland and read constants without depending on the VM.

use std::rc::Rc;

use phpcraft_runtime::value::parse_numeric;
use phpcraft_runtime::{
    compare, float_to_fixed, float_to_general, float_to_repr, key_from_value, loose_eq, strict_eq,
    Key, PhpArray, PhpError, PhpResult, RuntimeHooks, Value,
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
    ("array_slice", array_slice),
    ("array_merge", array_merge),
    ("array_fill", array_fill),
    ("array_combine", array_combine),
    ("array_flip", array_flip),
    ("array_map", array_map),
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
    ("max", max),
    ("min", min),
    ("pi", pi),
    // types
    ("gettype", gettype),
    ("get_debug_type", get_debug_type),
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
    ("is_numeric", is_numeric),
    ("is_scalar", is_scalar),
    ("is_callable", is_callable),
    ("function_exists", function_exists),
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
    Ok(string_value(string.repeat(times as usize)))
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
            end = (len + length).max(start);
        } else {
            end = (start + length).min(len);
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
            let drop = (-limit) as usize;
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
        if format.get(i) == Some(&b'.') {
            i += 1;
            let mut precision = 0usize;
            while let Some(digit) = format.get(i).filter(|c| c.is_ascii_digit()) {
                precision = precision.saturating_mul(10) + (digit - b'0') as usize;
                i += 1;
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

        match conversion {
            b's' => {
                let mut bytes = value.to_bytes();
                if let Some(precision) = spec.precision {
                    bytes.truncate(precision);
                }
                out.extend(pad(&[], &bytes, spec.width, spec.left, false));
            }
            b'd' | b'i' => {
                let number = value.to_int_cast();
                let negative = number < 0;
                let mut digits = number.unsigned_abs().to_string().into_bytes();
                if let Some(precision) = spec.precision {
                    while digits.len() < precision {
                        digits.insert(0, b'0');
                    }
                }
                let sign = sign_prefix(negative, spec.plus, spec.space);
                out.extend(pad(&sign, &digits, spec.width, spec.left, spec.zero));
            }
            b'u' => {
                let number = value.to_int_cast() as u64;
                let digits = number.to_string().into_bytes();
                out.extend(pad(&[], &digits, spec.width, spec.left, spec.zero));
            }
            b'f' | b'F' => {
                let number = value.to_float_cast();
                let decimals = spec.precision.unwrap_or(6);
                let text = float_to_fixed(number.abs(), decimals);
                let sign = sign_prefix(number < 0.0, spec.plus, spec.space);
                out.extend(pad(
                    &sign,
                    text.as_bytes(),
                    spec.width,
                    spec.left,
                    spec.zero,
                ));
            }
            b'e' | b'E' => {
                let number = value.to_float_cast();
                let decimals = spec.precision.unwrap_or(6);
                let text = format!("{:.*e}", decimals, number.abs());
                let (mantissa, exponent) = text.split_once('e').expect("exponential");
                let exponent: i32 = exponent.parse().expect("exponent");
                let mut body = mantissa.as_bytes().to_vec();
                body.push(if conversion == b'E' { b'E' } else { b'e' });
                body.push(if exponent < 0 { b'-' } else { b'+' });
                body.extend_from_slice(exponent.unsigned_abs().to_string().as_bytes());
                let sign = sign_prefix(number < 0.0, spec.plus, spec.space);
                out.extend(pad(&sign, &body, spec.width, spec.left, spec.zero));
            }
            b'g' | b'G' => {
                let number = value.to_float_cast();
                let precision = spec.precision.unwrap_or(6).max(1);
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
                ));
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
                out.extend(pad(&[], &digits, spec.width, spec.left, spec.zero));
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
                ));
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

fn pad(sign: &[u8], body: &[u8], width: usize, left: bool, zero: bool) -> Vec<u8> {
    let total = sign.len() + body.len();
    if total >= width {
        let mut out = sign.to_vec();
        out.extend_from_slice(body);
        return out;
    }
    let fill = width - total;
    let mut out = Vec::with_capacity(width);
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
    out
}

fn number_format(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("number_format", &args, 1, Some(4))?;
    let number = as_float(&args, 0, "number_format", "num")?;
    let decimals = if args.len() > 1 {
        as_int(&args, 1, "number_format", "decimals")?.max(0) as usize
    } else {
        0
    };
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

    let factor = 10f64.powi(decimals as i32);
    let rounded = (number.abs() * factor).round() / factor;
    let text = format!("{rounded:.decimals$}");
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
    let needed = length as usize - input.len();
    let mut padding = Vec::with_capacity(needed);
    while padding.len() < needed {
        padding.push(pad_string[padding.len() % pad_string.len()]);
    }
    match pad_type {
        0 => {
            let mut out = padding[..needed].to_vec();
            out.extend_from_slice(&input);
            input = out;
        }
        2 => {
            let left = needed / 2;
            let right = needed - left;
            let mut out = padding[..left].to_vec();
            out.extend_from_slice(&input);
            out.extend_from_slice(&padding[..right]);
            input = out;
        }
        _ => input.extend_from_slice(&padding[..needed]),
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
            end = (total + length).max(start);
        } else {
            end = (start + length).min(total);
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
    let mut out = PhpArray::with_capacity(count as usize);
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
        Value::Str(name) => Some(String::from_utf8_lossy(name.as_bytes()).into_owned()),
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
            Some(name) => hooks.call(name, vec![value.clone()])?,
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

fn range(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("range", &args, 2, Some(3))?;
    let mut out = PhpArray::new();

    let is_char = |value: &Value| -> Option<u8> {
        match value {
            Value::Str(s) if s.as_bytes().len() == 1 => Some(s.as_bytes()[0]),
            _ => None,
        }
    };

    if let (Some(start), Some(end)) = (is_char(&args[0]), is_char(&args[1])) {
        let step = if args.len() > 2 {
            as_int(&args, 2, "range", "step")?
        } else {
            1
        };
        if step == 0 {
            return Err(PhpError::value_error(
                "range(): Argument #3 ($step) cannot be 0",
            ));
        }
        let step = step.unsigned_abs() as usize;
        if start <= end {
            let mut current = start;
            loop {
                out.push(string_value(vec![current]));
                if current as usize + step > end as usize {
                    break;
                }
                current = (current as usize + step) as u8;
            }
        } else {
            let mut current = start as i32;
            loop {
                out.push(string_value(vec![current as u8]));
                if (current - step as i32) < (end as i32) {
                    break;
                }
                current -= step as i32;
            }
        }
        return Ok(Value::array(out));
    }

    let start = args[0].to_float_cast();
    let end = args[1].to_float_cast();
    let step = if args.len() > 2 {
        as_float(&args, 2, "range", "step")?
    } else {
        1.0
    };
    if step == 0.0 {
        return Err(PhpError::value_error(
            "range(): Argument #3 ($step) cannot be 0",
        ));
    }
    let step = step.abs();
    let integer_range =
        args[0].to_number().is_int() && args[1].to_number().is_int() && step.fract() == 0.0;
    if start <= end {
        let mut current = start;
        while current <= end + f64::EPSILON {
            out.push(if integer_range {
                Value::Int(current as i64)
            } else {
                Value::Float(current)
            });
            current += step;
        }
    } else {
        let mut current = start;
        while current >= end - f64::EPSILON {
            out.push(if integer_range {
                Value::Int(current as i64)
            } else {
                Value::Float(current)
            });
            current -= step;
        }
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

fn pi(_hooks: &mut dyn RuntimeHooks, _args: Vec<Value>) -> PhpResult<Value> {
    Ok(Value::Float(std::f64::consts::PI))
}

// -------------------------------------------------------------------- types

fn gettype(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("gettype", &args, 1, Some(1))?;
    Ok(Value::from_str_lit(match &args[0] {
        Value::Null => "NULL",
        Value::Bool(_) => "boolean",
        Value::Int(_) => "integer",
        Value::Float(_) => "double",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
    }))
}

fn get_debug_type(_hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("get_debug_type", &args, 1, Some(1))?;
    Ok(Value::from_str_lit(args[0].debug_type_name()))
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
    let result = match &args[0] {
        Value::Str(name) => hooks.function_exists(&String::from_utf8_lossy(name.as_bytes())),
        _ => false,
    };
    Ok(Value::Bool(result))
}

fn function_exists(hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("function_exists", &args, 1, Some(1))?;
    let name = as_string(&args, 0, "function_exists", "function")?;
    Ok(Value::Bool(
        hooks.function_exists(&String::from_utf8_lossy(&name)),
    ))
}

// ------------------------------------------------------------------- output

fn dump_value(value: &Value, depth: usize, out: &mut Vec<u8>) {
    out.extend(std::iter::repeat_n(b' ', depth * 2));
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
                dump_value(item, depth + 1, out);
            }
            out.extend(std::iter::repeat_n(b' ', depth * 2));
            out.extend_from_slice(b"}\n");
        }
    }
}

fn var_dump(hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    let mut out = Vec::new();
    for value in &args {
        dump_value(value, 0, &mut out);
    }
    hooks.write(&out);
    Ok(Value::Null)
}

fn print_array(array: &PhpArray, level: usize, out: &mut Vec<u8>) {
    out.extend_from_slice(b"Array\n");
    out.extend(std::iter::repeat_n(b' ', level * 4));
    out.extend_from_slice(b"(\n");
    for (key, value) in array.iter() {
        out.extend(std::iter::repeat_n(b' ', (level + 1) * 4));
        out.push(b'[');
        out.extend_from_slice(&key.as_display());
        out.extend_from_slice(b"] => ");
        match value {
            Value::Array(child) => {
                print_array(child, level + 2, out);
                // Nested arrays already end with a newline; PHP leaves a blank
                // line before the next element.
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

fn print_r(hooks: &mut dyn RuntimeHooks, args: Vec<Value>) -> PhpResult<Value> {
    arity("print_r", &args, 1, Some(2))?;
    let mut out = Vec::new();
    match &args[0] {
        Value::Array(array) => print_array(array, 0, &mut out),
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
