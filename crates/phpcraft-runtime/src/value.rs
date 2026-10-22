//! PHP scalar values (`zval`) and the conversion rules PHP applies to them.
//!
//! Strings are byte strings, like PHP's `zend_string`. Arrays are handled by
//! [`crate::PhpArray`] and share with `Rc` (copy-on-write, like Zend).

use std::cmp::Ordering;
use std::fmt;
use std::rc::Rc;

use crate::array::{Key, PhpArray};
use crate::error::{PhpError, PhpResult};

/// An immutable PHP string. Byte-oriented (`zend_string` is not UTF-8 aware).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PhpStr(pub Vec<u8>);

impl PhpStr {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        Self(s.as_bytes().to_vec())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn to_string_lossy(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }
}

impl fmt::Debug for PhpStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.to_string_lossy())
    }
}

/// A userland closure value. The VM resolves `function` to the compiled body;
/// `captures` holds the by-value `use (...)` or arrow-function imports.
#[derive(Clone)]
pub struct PhpClosure {
    /// Synthetic function name used to resolve the compiled body.
    pub function: String,
    /// Closure display name (`{closure:<file>:<line>}`).
    pub name: String,
    pub file: String,
    pub line: u32,
    /// Object handle shown by `var_dump`.
    pub id: u64,
    pub capture_names: Vec<String>,
    /// `(name, optional)` for the closure's declared parameters.
    pub parameters: Vec<(String, bool)>,
    pub captures: Vec<Value>,
}

impl fmt::Debug for PhpClosure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "closure({}, {} captures)",
            self.function,
            self.captures.len()
        )
    }
}

/// A numeric value produced by PHP's numeric conversion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Numeric {
    Int(i64),
    Float(f64),
}

impl Numeric {
    pub fn to_float(self) -> f64 {
        match self {
            Numeric::Int(i) => i as f64,
            Numeric::Float(f) => f,
        }
    }

    pub fn is_int(self) -> bool {
        matches!(self, Numeric::Int(_))
    }
}

/// Outcome of converting a value for use in arithmetic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToNumber {
    /// The value is a number.
    Num(Numeric),
    /// The value began with a number but had trailing garbage (`"12abc"`);
    /// PHP emits a warning and uses the numeric prefix.
    Leading(Numeric),
    /// The value is not numeric at all.
    Invalid,
}

impl ToNumber {
    pub fn is_int(self) -> bool {
        matches!(self, ToNumber::Num(Numeric::Int(_)))
    }
}

/// The PHP value type. Objects and resources are not implemented yet.
#[derive(Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Rc<PhpStr>),
    Array(Rc<PhpArray>),
    Closure(Rc<PhpClosure>),
}

impl Value {
    pub fn str(bytes: impl Into<Vec<u8>>) -> Self {
        Value::Str(Rc::new(PhpStr::new(bytes.into())))
    }

    pub fn from_str_lit(s: &str) -> Self {
        Value::Str(Rc::new(PhpStr::from_str(s)))
    }

    pub fn array(items: PhpArray) -> Self {
        Value::Array(Rc::new(items))
    }

    pub fn closure(
        function: String,
        file: String,
        line: u32,
        id: u64,
        capture_names: Vec<String>,
        parameters: Vec<(String, bool)>,
        captures: Vec<Value>,
    ) -> Self {
        let name = format!("{{closure:{file}:{line}}}");
        Value::Closure(Rc::new(PhpClosure {
            function,
            name,
            file,
            line,
            id,
            capture_names,
            parameters,
            captures,
        }))
    }

    /// PHP's `gettype()` name.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Closure(_) => "object",
        }
    }

    /// PHP's `get_debug_type()` name, used in error messages.
    pub fn debug_type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Closure(_) => "Closure",
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn is_scalar(&self) -> bool {
        matches!(
            self,
            Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Str(_)
        )
    }

    /// PHP truthiness: `false`, `0`, `0.0`, `""`, `"0"`, `[]` and `null` are falsy.
    pub fn to_bool(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !(s.0.is_empty() || s.0 == b"0"),
            Value::Array(a) => !a.is_empty(),
            Value::Closure(_) => true,
        }
    }

    /// String conversion as performed by `echo`, string casts and concatenation.
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Value::Null => Vec::new(),
            Value::Bool(true) => b"1".to_vec(),
            Value::Bool(false) => Vec::new(),
            Value::Int(i) => i.to_string().into_bytes(),
            Value::Float(f) => float_to_string(*f).into_bytes(),
            Value::Str(s) => s.0.clone(),
            Value::Array(_) => b"Array".to_vec(),
            Value::Closure(_) => b"Closure".to_vec(),
        }
    }

    pub fn to_string_lossy(&self) -> String {
        String::from_utf8_lossy(&self.to_bytes()).into_owned()
    }

    /// Integer cast, following PHP's `(int)` rules where they do not warn.
    pub fn to_int_cast(&self) -> i64 {
        match self {
            Value::Null => 0,
            Value::Bool(b) => i64::from(*b),
            Value::Int(i) => *i,
            Value::Float(f) => float_to_int(*f),
            Value::Str(s) => match parse_numeric(&s.0) {
                Some((n, _)) => match n {
                    Numeric::Int(i) => i,
                    Numeric::Float(f) => float_to_int(f),
                },
                None => 0,
            },
            Value::Array(a) => i64::from(!a.is_empty()),
            Value::Closure(_) => 1,
        }
    }

    /// Float cast.
    pub fn to_float_cast(&self) -> f64 {
        match self {
            Value::Null => 0.0,
            Value::Bool(b) => f64::from(u8::from(*b)),
            Value::Int(i) => *i as f64,
            Value::Float(f) => *f,
            Value::Str(s) => match parse_numeric(&s.0) {
                Some((n, _)) => n.to_float(),
                None => 0.0,
            },
            Value::Array(a) => f64::from(u8::from(!a.is_empty())),
            Value::Closure(_) => 1.0,
        }
    }

    /// Convert for arithmetic. Arrays are not usable as numbers.
    pub fn to_number(&self) -> ToNumber {
        match self {
            Value::Null => ToNumber::Num(Numeric::Int(0)),
            Value::Bool(b) => ToNumber::Num(Numeric::Int(i64::from(*b))),
            Value::Int(i) => ToNumber::Num(Numeric::Int(*i)),
            Value::Float(f) => ToNumber::Num(Numeric::Float(*f)),
            Value::Str(s) => match parse_numeric(&s.0) {
                Some((n, true)) => ToNumber::Num(n),
                Some((n, false)) => ToNumber::Leading(n),
                None => ToNumber::Invalid,
            },
            Value::Array(_) | Value::Closure(_) => ToNumber::Invalid,
        }
    }

    /// `(string)` / `(int)`-style coercion used by builtin arguments.
    pub fn coerce_string(&self) -> PhpResult<Rc<PhpStr>> {
        match self {
            Value::Str(s) => Ok(s.clone()),
            Value::Array(_) => Err(PhpError::type_error(
                "Array to string conversion is not supported",
            )),
            Value::Closure(_) => Err(PhpError::type_error(
                "Object of class Closure could not be converted to string",
            )),
            other => Ok(Rc::new(PhpStr::new(other.to_bytes()))),
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Bool(b) => write!(f, "bool({b})"),
            Value::Int(i) => write!(f, "int({i})"),
            Value::Float(v) => write!(f, "float({})", float_to_repr(*v)),
            Value::Str(s) => write!(f, "string({:?})", s.to_string_lossy()),
            Value::Array(a) => write!(f, "array({})", a.len()),
            Value::Closure(c) => write!(f, "closure({})", c.function),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        strict_eq(self, other)
    }
}

/// PHP `(int)` cast of a float. Out-of-range values are undefined behaviour in
/// C; PHP 8.1+ clamps them, which we reproduce.
pub fn float_to_int(f: f64) -> i64 {
    if f.is_nan() || f.is_infinite() {
        return 0;
    }
    let t = f.trunc();
    if t >= 9223372036854775808.0 {
        i64::MAX
    } else if t <= -9223372036854775808.0 {
        i64::MIN
    } else {
        t as i64
    }
}

/// Parse a PHP numeric string.
///
/// Returns the numeric value and whether the whole string was consumed.
/// Leading whitespace is permitted (PHP 8); trailing whitespace is ignored.
/// `None` means the string does not start with a number.
pub fn parse_numeric(bytes: &[u8]) -> Option<(Numeric, bool)> {
    let len = bytes.len();
    let mut i = 0;
    while i < len && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let start = i;
    if i < len && (bytes[i] == b'+' || bytes[i] == b'-') {
        i += 1;
    }
    let int_start = i;
    while i < len && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let int_end = i;
    let mut is_float = false;
    if i < len && bytes[i] == b'.' {
        is_float = true;
        i += 1;
        while i < len && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < len && (bytes[i] == b'e' || bytes[i] == b'E') {
        let save = i;
        i += 1;
        if i < len && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        if i < len && bytes[i].is_ascii_digit() {
            is_float = true;
            while i < len && bytes[i].is_ascii_digit() {
                i += 1;
            }
        } else {
            i = save;
        }
    }
    let has_digits = int_end > int_start || (is_float && i > int_end + 1);
    if !has_digits {
        return None;
    }

    let mut j = i;
    while j < len && bytes[j].is_ascii_whitespace() {
        j += 1;
    }
    let complete = j == len;

    let text = std::str::from_utf8(&bytes[start..i]).ok()?;
    let value = if is_float {
        Numeric::Float(parse_float_text(text))
    } else {
        match text.parse::<i64>() {
            Ok(v) => Numeric::Int(v),
            Err(_) => {
                // Integer overflow promotes to float, exactly like Zend.
                match text.parse::<i128>() {
                    Ok(v) if v >= i64::MIN as i128 && v <= i64::MAX as i128 => {
                        Numeric::Int(v as i64)
                    }
                    _ => Numeric::Float(parse_float_text(text)),
                }
            }
        }
    };
    Some((value, complete))
}

fn parse_float_text(text: &str) -> f64 {
    let mut owned;
    let mut t = text;
    if t.starts_with('.') {
        owned = format!("0{t}");
        t = &owned;
    } else if t.starts_with("-.") || t.starts_with("+.") {
        owned = format!("{}0{}", &t[..1], &t[1..]);
        t = &owned;
    }
    if t.ends_with('.') {
        owned = format!("{t}0");
        t = &owned;
    }
    t.parse::<f64>().unwrap_or(0.0)
}

/// Is this string a "canonical" decimal integer, i.e. one PHP converts to an
/// integer array key? `"8"` yes, `"08"`, `"+8"`, `"-0"` and `" 8"` no.
pub fn is_canonical_int(bytes: &[u8]) -> bool {
    match parse_numeric(bytes) {
        Some((Numeric::Int(v), true)) => {
            let canon = v.to_string();
            canon.as_bytes() == bytes
        }
        _ => false,
    }
}

/// Formatting used by `echo`/string casts: C `printf("%.14G")`, PHP's default
/// `precision=14`.
pub fn float_to_string(f: f64) -> String {
    format_float(f, 14, 14, false)
}

/// Formatting used by `var_dump`/`debug_zval_dump`: `serialize_precision=-1`,
/// i.e. the shortest representation that round-trips.
pub fn float_to_repr(f: f64) -> String {
    format_float(f, 17, 17, true)
}

/// C `printf("%.*g")`: `precision` significant digits, trailing zeros trimmed.
pub fn float_to_general(f: f64, precision: usize) -> String {
    format_float(f, precision.max(1), precision.max(1) as i32, false)
}

/// C `printf("%.*f")`.
pub fn float_to_fixed(f: f64, decimals: usize) -> String {
    if f.is_nan() {
        return "NAN".to_string();
    }
    if f.is_infinite() {
        return if f.is_sign_negative() { "-INF" } else { "INF" }.to_string();
    }
    format!("{f:.decimals$}")
}

fn format_float(f: f64, precision: usize, threshold: i32, shortest: bool) -> String {
    if f.is_nan() {
        return "NAN".to_string();
    }
    if f.is_infinite() {
        return if f.is_sign_negative() { "-INF" } else { "INF" }.to_string();
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0" } else { "0" }.to_string();
    }

    let negative = f.is_sign_negative();
    let (digits, exponent) = if shortest {
        shortest_digits(f.abs())
    } else {
        rounded_digits(f.abs(), precision)
    };
    let mut digits = digits;
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }

    let mut out = String::new();
    if negative {
        out.push('-');
    }

    if exponent < -4 || exponent >= threshold {
        out.push_str(&digits[..1]);
        out.push('.');
        if digits.len() > 1 {
            out.push_str(&digits[1..]);
        } else {
            out.push('0');
        }
        out.push('E');
        if exponent < 0 {
            out.push('-');
        } else {
            out.push('+');
        }
        out.push_str(&exponent.unsigned_abs().to_string());
        out
    } else if exponent >= 0 {
        let int_len = exponent as usize + 1;
        if digits.len() <= int_len {
            out.push_str(&digits);
            for _ in digits.len()..int_len {
                out.push('0');
            }
        } else {
            out.push_str(&digits[..int_len]);
            out.push('.');
            out.push_str(&digits[int_len..]);
        }
        out
    } else {
        out.push_str("0.");
        for _ in 0..(-exponent - 1) {
            out.push('0');
        }
        out.push_str(&digits);
        out
    }
}

/// Shortest round-tripping decimal digits plus the power-of-ten exponent of the
/// first digit (`1.5` -> `("15", 0)`).
fn shortest_digits(f: f64) -> (String, i32) {
    let s = format!("{f:e}");
    let (mantissa, exp) = s.split_once('e').expect("exponential format");
    let exponent: i32 = exp.parse().expect("exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    (digits, exponent)
}

/// `precision` significant decimal digits, rounded like `printf`.
fn rounded_digits(f: f64, precision: usize) -> (String, i32) {
    let s = format!("{:.*e}", precision.saturating_sub(1), f);
    let (mantissa, exp) = s.split_once('e').expect("exponential format");
    let exponent: i32 = exp.parse().expect("exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    (digits, exponent)
}

/// PHP `==`.
pub fn loose_eq(a: &Value, b: &Value) -> bool {
    matches!(compare(a, b), Some(Ordering::Equal))
}

/// PHP `===`.
pub fn strict_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x.0 == y.0,
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return false;
            }
            x.iter()
                .zip(y.iter())
                .all(|((ka, va), (kb, vb))| ka == kb && strict_eq(va, vb))
        }
        (Value::Closure(x), Value::Closure(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

/// PHP 8 comparison rules, as used by `<=>`, `<`, `>` and loose equality.
/// Returns `None` when the two values are not comparable (e.g. mismatched
/// array keys), which callers treat as "not equal".
pub fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    use Value::*;

    // bool/null comparisons are done as booleans.
    let boolish = |v: &Value| matches!(v, Null | Bool(_));
    if boolish(a) || boolish(b) {
        return Some(a.to_bool().cmp(&b.to_bool()));
    }

    match (a, b) {
        (Int(x), Int(y)) => Some(x.cmp(y)),
        (Int(_) | Float(_), Int(_) | Float(_)) => {
            let (x, y) = (a.to_float_cast(), b.to_float_cast());
            x.partial_cmp(&y)
        }
        (Str(x), Str(y)) => Some(compare_strings(&x.0, &y.0)),
        (Str(s), Int(_) | Float(_)) => {
            if let Some((n, true)) = parse_numeric(&s.0) {
                Some(
                    n.to_float()
                        .partial_cmp(&b.to_float_cast())
                        .unwrap_or(Ordering::Equal),
                )
            } else {
                Some(compare_bytes(&s.0, &b.to_bytes()))
            }
        }
        (Int(_) | Float(_), Str(s)) => {
            if let Some((n, true)) = parse_numeric(&s.0) {
                Some(
                    a.to_float_cast()
                        .partial_cmp(&n.to_float())
                        .unwrap_or(Ordering::Equal),
                )
            } else {
                Some(compare_bytes(&a.to_bytes(), &s.0))
            }
        }
        (Array(x), Array(y)) => compare_arrays(x, y),
        (Array(_), _) => Some(Ordering::Greater),
        (_, Array(_)) => Some(Ordering::Less),
        (Closure(x), Closure(y)) => Some(if Rc::ptr_eq(x, y) {
            Ordering::Equal
        } else {
            Ordering::Greater
        }),
        (Closure(_), _) => Some(Ordering::Greater),
        (_, Closure(_)) => Some(Ordering::Less),
        (Null, _) | (_, Null) | (Bool(_), _) | (_, Bool(_)) => Some(a.to_bool().cmp(&b.to_bool())),
    }
}

fn compare_strings(a: &[u8], b: &[u8]) -> Ordering {
    let na = parse_numeric(a).filter(|(_, complete)| *complete);
    let nb = parse_numeric(b).filter(|(_, complete)| *complete);
    if let (Some((x, _)), Some((y, _))) = (na, nb) {
        return x
            .to_float()
            .partial_cmp(&y.to_float())
            .unwrap_or(Ordering::Equal);
    }
    compare_bytes(a, b)
}

fn compare_bytes(a: &[u8], b: &[u8]) -> Ordering {
    a.cmp(b)
}

fn compare_arrays(x: &PhpArray, y: &PhpArray) -> Option<Ordering> {
    if x.len() != y.len() {
        return Some(x.len().cmp(&y.len()));
    }
    let mut result = Ordering::Equal;
    for (key, xv) in x.iter() {
        let yv = y.get(key)?;
        match compare(xv, yv) {
            Some(Ordering::Equal) => {}
            Some(other) => result = other,
            None => return None,
        }
    }
    Some(result)
}

/// Array-key coercion for `$a[$value]`. Returns the key, or an error for
/// illegal offset types (arrays), matching PHP's `TypeError`.
pub fn key_from_value(value: &Value) -> PhpResult<Key> {
    Ok(match value {
        Value::Int(i) => Key::Int(*i),
        Value::Str(s) => Key::from_bytes(&s.0),
        Value::Bool(b) => Key::Int(i64::from(*b)),
        Value::Float(f) => Key::Int(float_to_int(*f)),
        Value::Null => Key::Str(Rc::new(PhpStr::from_str(""))),
        Value::Array(_) | Value::Closure(_) => {
            return Err(PhpError::type_error(
                "Illegal offset type: array keys must be int or string",
            ))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_formatting_matches_php() {
        assert_eq!(float_to_string(1.0), "1");
        assert_eq!(float_to_string(0.1 + 0.2), "0.3");
        assert_eq!(float_to_string(1e14), "1.0E+14");
        assert_eq!(float_to_string(1e20), "1.0E+20");
        assert_eq!(float_to_string(1.5e-5), "1.5E-5");
        assert_eq!(float_to_string(1e-4), "0.0001");
        assert_eq!(float_to_string(-2.5), "-2.5");
    }

    #[test]
    fn repr_formatting_matches_php() {
        assert_eq!(float_to_repr(1.0), "1");
        assert_eq!(float_to_repr(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(float_to_repr(1e16), "10000000000000000");
        assert_eq!(float_to_repr(1e17), "1.0E+17");
        assert_eq!(float_to_repr(1e-4), "0.0001");
        assert_eq!(float_to_repr(1.5e-5), "1.5E-5");
    }

    #[test]
    fn canonical_int_keys() {
        assert!(is_canonical_int(b"8"));
        assert!(is_canonical_int(b"-8"));
        assert!(is_canonical_int(b"0"));
        assert!(!is_canonical_int(b"08"));
        assert!(!is_canonical_int(b"-0"));
        assert!(!is_canonical_int(b"+8"));
        assert!(!is_canonical_int(b" 8"));
        assert!(!is_canonical_int(b"8 "));
    }
}
