//! PHP's ordered hash table (`HashTable`/`Bucket` in Zend).
//!
//! PHP arrays preserve insertion order, coerce numeric string keys to integers,
//! and track an internal "next free index" for `$a[] = ...`. Cloning is cheap
//! because [`Value::Array`](crate::Value) shares via `Rc`; mutation goes
//! through `Rc::make_mut`, which gives PHP's copy-on-write semantics.

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::value::{float_to_int, is_canonical_int, PhpStr, Value};

/// An array key. PHP arrays only have integer and string keys.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Key {
    Int(i64),
    Str(Rc<PhpStr>),
}

impl Key {
    /// Coerce a string to an integer key when it is a canonical decimal.
    pub fn from_bytes(bytes: &[u8]) -> Key {
        if is_canonical_int(bytes) {
            // `is_canonical_int` already proved this parses.
            let s = std::str::from_utf8(bytes).expect("ascii digits");
            Key::Int(s.parse().expect("validated integer"))
        } else {
            Key::Str(Rc::new(PhpStr::new(bytes.to_vec())))
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Key {
        Key::from_bytes(s.as_bytes())
    }

    pub fn as_display(&self) -> Vec<u8> {
        match self {
            Key::Int(i) => i.to_string().into_bytes(),
            Key::Str(s) => s.0.clone(),
        }
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Int(i) => write!(f, "{i}"),
            Key::Str(s) => write!(f, "{:?}", s.to_string_lossy()),
        }
    }
}

/// PHP array. `index` maps keys to positions in `entries` to keep `get` O(1)
/// while preserving insertion order.
#[derive(Clone, Default)]
pub struct PhpArray {
    entries: Vec<(Key, Value)>,
    index: HashMap<Key, usize>,
    next_index: i64,
}

impl PhpArray {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            entries: Vec::with_capacity(cap),
            index: HashMap::with_capacity(cap),
            next_index: 0,
        }
    }

    pub fn from_pairs(pairs: Vec<(Key, Value)>) -> Self {
        let mut arr = Self::with_capacity(pairs.len());
        for (k, v) in pairs {
            arr.insert(k, v);
        }
        arr
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The key that `$a[] = ...` will use next.
    pub fn next_index(&self) -> i64 {
        self.next_index
    }

    pub fn get(&self, key: &Key) -> Option<&Value> {
        self.index.get(key).map(|&i| &self.entries[i].1)
    }

    pub fn get_int(&self, key: i64) -> Option<&Value> {
        self.get(&Key::Int(key))
    }

    pub fn get_str(&self, key: &str) -> Option<&Value> {
        self.get(&Key::from_str(key))
    }

    pub fn contains_key(&self, key: &Key) -> bool {
        self.index.contains_key(key)
    }

    /// Insert or replace. Existing entries keep their position, like Zend.
    pub fn insert(&mut self, key: Key, value: Value) {
        if let Key::Int(i) = key {
            if i >= self.next_index {
                self.next_index = i.saturating_add(1);
            }
        }
        if let Some(&pos) = self.index.get(&key) {
            self.entries[pos].1 = value;
            return;
        }
        self.index.insert(key.clone(), self.entries.len());
        self.entries.push((key, value));
    }

    /// `$a[] = $value`.
    pub fn push(&mut self, value: Value) -> Key {
        let key = Key::Int(self.next_index);
        self.insert(key.clone(), value);
        key
    }

    pub fn insert_str(&mut self, key: &str, value: Value) {
        self.insert(Key::from_str(key), value);
    }

    pub fn insert_int(&mut self, key: i64, value: Value) {
        self.insert(Key::Int(key), value);
    }

    /// Remove a key, shifting later entries (Zend keeps a hole in the bucket
    /// list, but observable order is the same).
    pub fn remove(&mut self, key: &Key) -> Option<Value> {
        let pos = self.index.remove(key)?;
        let (_, value) = self.entries.remove(pos);
        for (_, index) in self.index.iter_mut() {
            if *index > pos {
                *index -= 1;
            }
        }
        Some(value)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Key, &Value)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    pub fn keys(&self) -> Vec<Key> {
        self.entries.iter().map(|(k, _)| k.clone()).collect()
    }

    pub fn values(&self) -> Vec<Value> {
        self.entries.iter().map(|(_, v)| v.clone()).collect()
    }

    /// Truncate to the given length (used by `array_slice`).
    pub fn truncate(&mut self, len: usize) {
        while self.entries.len() > len {
            let (key, _) = self.entries.pop().expect("non-empty");
            self.index.remove(&key);
        }
    }

    pub fn first_key(&self) -> Option<&Key> {
        self.entries.first().map(|(k, _)| k)
    }
}

impl fmt::Debug for PhpArray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.entries.iter().map(|(k, v)| (k, v)))
            .finish()
    }
}

impl FromIterator<Value> for PhpArray {
    fn from_iter<T: IntoIterator<Item = Value>>(iter: T) -> Self {
        let mut array = PhpArray::new();
        for value in iter {
            array.push(value);
        }
        array
    }
}

impl From<Vec<Value>> for PhpArray {
    fn from(values: Vec<Value>) -> Self {
        let mut arr = Self::with_capacity(values.len());
        for v in values {
            arr.push(v);
        }
        arr
    }
}

/// PHP float-to-key coercion truncates toward zero; keep the helper public so
/// the VM and builtins agree.
pub fn float_key(f: f64) -> i64 {
    float_to_int(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_string_keys_become_int_keys() {
        let mut a = PhpArray::new();
        a.insert(Key::from_str("8"), Value::Int(1));
        a.insert(Key::from_str("08"), Value::Int(2));
        assert!(matches!(a.first_key(), Some(Key::Int(8))));
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn append_after_explicit_int_key() {
        let mut a = PhpArray::new();
        a.insert_int(5, Value::Int(1));
        assert_eq!(a.push(Value::Int(2)), Key::Int(6));
    }

    #[test]
    fn replacement_keeps_position() {
        let mut a = PhpArray::new();
        a.push(Value::Int(1));
        a.push(Value::Int(2));
        a.insert_int(0, Value::Int(9));
        let vals: Vec<i64> = a.values().iter().map(|v| v.to_int_cast()).collect();
        assert_eq!(vals, vec![9, 2]);
    }
}
