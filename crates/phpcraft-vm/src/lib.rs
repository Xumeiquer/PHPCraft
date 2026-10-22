//! PHPCraft virtual machine.
//!
//! Executes the stack-machine bytecode produced by `phpcraft-compiler`. The
//! structure mirrors `Zend/zend_vm_def.h`: a dispatch loop over an op array,
//! a call frame with slots, and a value stack. Function calls recurse in Rust
//! (Zend also uses the C stack), with an explicit nesting guard.

use std::collections::HashMap;
use std::rc::Rc;

use phpcraft_ast::{BinOp, CastKind, UnOp};
use phpcraft_bytecode::{Chunk, CompiledProgram, DimMode, Function, Op};
use phpcraft_runtime::{
    compare, key_from_value, loose_eq, strict_eq, ErrorKind, Key, Numeric, PhpArray, PhpError,
    PhpResult, RuntimeHooks, ToNumber, Value,
};

/// Maximum userland call depth, mirroring the usual `xdebug.max_nesting_level`
/// style guard rather than PHP's unbounded C recursion.
const MAX_CALL_DEPTH: u32 = 512;

struct IterState {
    array: PhpArray,
    pos: usize,
}

struct Frame {
    slots: Vec<Value>,
    defined: Vec<bool>,
    iters: HashMap<u32, IterState>,
}

impl Frame {
    fn new(size: usize) -> Self {
        Self {
            slots: vec![Value::Null; size],
            defined: vec![false; size],
            iters: HashMap::new(),
        }
    }

    fn set(&mut self, slot: u32, value: Value) {
        let index = slot as usize;
        if index < self.slots.len() {
            self.slots[index] = value;
            self.defined[index] = true;
        }
    }

    fn get(&self, slot: u32) -> &Value {
        &self.slots[slot as usize]
    }
}

pub struct Vm {
    functions: HashMap<String, Rc<Function>>,
    globals: PhpArray,
    constants: HashMap<String, Value>,
    output: Vec<u8>,
    warnings: Vec<String>,
    depth: u32,
    file: String,
    next_object_id: u64,
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

impl Vm {
    pub fn new() -> Self {
        let mut constants = HashMap::new();
        constants.insert("PHP_EOL".to_string(), Value::str(b"\n".to_vec()));
        constants.insert("PHP_VERSION".to_string(), Value::from_str_lit("8.5.4"));
        constants.insert("PHP_MAJOR_VERSION".to_string(), Value::Int(8));
        constants.insert("PHP_MINOR_VERSION".to_string(), Value::Int(5));
        constants.insert("PHP_INT_MAX".to_string(), Value::Int(i64::MAX));
        constants.insert("PHP_INT_MIN".to_string(), Value::Int(i64::MIN));
        constants.insert("PHP_INT_SIZE".to_string(), Value::Int(8));
        constants.insert("PHP_FLOAT_EPSILON".to_string(), Value::Float(f64::EPSILON));
        constants.insert("PHP_FLOAT_MAX".to_string(), Value::Float(f64::MAX));
        constants.insert("PHP_FLOAT_MIN".to_string(), Value::Float(f64::MIN_POSITIVE));
        constants.insert("PHP_FLOAT_DIG".to_string(), Value::Int(15));
        constants.insert("M_PI".to_string(), Value::Float(std::f64::consts::PI));
        constants.insert("M_E".to_string(), Value::Float(std::f64::consts::E));
        constants.insert("INF".to_string(), Value::Float(f64::INFINITY));
        constants.insert("NAN".to_string(), Value::Float(f64::NAN));
        constants.insert("E_ERROR".to_string(), Value::Int(1));
        constants.insert("E_WARNING".to_string(), Value::Int(2));
        constants.insert("E_PARSE".to_string(), Value::Int(4));
        constants.insert("E_NOTICE".to_string(), Value::Int(8));
        constants.insert("E_ALL".to_string(), Value::Int(32767));
        constants.insert("E_DEPRECATED".to_string(), Value::Int(8192));
        constants.insert("STR_PAD_LEFT".to_string(), Value::Int(0));
        constants.insert("STR_PAD_RIGHT".to_string(), Value::Int(1));
        constants.insert("STR_PAD_BOTH".to_string(), Value::Int(2));
        constants.insert("SORT_REGULAR".to_string(), Value::Int(0));
        constants.insert("SORT_NUMERIC".to_string(), Value::Int(1));
        constants.insert("SORT_STRING".to_string(), Value::Int(2));
        constants.insert("COUNT_NORMAL".to_string(), Value::Int(0));
        constants.insert("COUNT_RECURSIVE".to_string(), Value::Int(1));

        Self {
            functions: HashMap::new(),
            globals: PhpArray::new(),
            constants,
            output: Vec::new(),
            warnings: Vec::new(),
            depth: 0,
            file: String::new(),
            next_object_id: 1,
        }
    }

    /// Register every compiled userland function.
    pub fn load(&mut self, program: &CompiledProgram) {
        self.file = program.file.clone();
        for function in &program.functions {
            self.functions.insert(
                function.name.to_ascii_lowercase(),
                Rc::new(function.clone()),
            );
        }
    }

    /// Execute the top-level `{main}` chunk.
    pub fn run_main(&mut self, program: &CompiledProgram) -> PhpResult<Value> {
        self.load(program);
        let main = Rc::new(program.main.clone());
        self.call_function(main, Vec::new())
    }

    pub fn output(&self) -> &[u8] {
        &self.output
    }

    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn globals(&self) -> &PhpArray {
        &self.globals
    }

    /// Define a global variable before the script runs (used for `$argv`).
    pub fn set_global(&mut self, name: &str, value: Value) {
        self.globals.insert(Key::from_str(name), value);
    }

    /// Register a runtime constant (`define()` semantics).
    pub fn define_constant(&mut self, name: &str, value: Value) {
        self.constants.insert(name.to_string(), value);
    }

    /// Call a userland function or builtin by name, case-insensitively.
    pub fn call_named(&mut self, name: &str, args: Vec<Value>) -> PhpResult<Value> {
        let lower = name.to_ascii_lowercase();
        if let Some(function) = self.functions.get(&lower).cloned() {
            return self.call_function(function, args);
        }
        if let Some(native) = phpcraft_stdlib::find(&lower) {
            return native(self, args);
        }
        Err(PhpError::fatal(format!(
            "Call to undefined function {name}()"
        )))
    }

    fn warn(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.warnings.push(message);
    }

    fn output_bytes(&mut self, value: &Value) -> PhpResult<Vec<u8>> {
        if matches!(value, Value::Closure(_)) {
            return Err(PhpError::fatal(
                "Object of class Closure could not be converted to string",
            ));
        }
        Ok(value.to_bytes())
    }

    fn call_function(&mut self, function: Rc<Function>, args: Vec<Value>) -> PhpResult<Value> {
        self.call_function_with_captures(function, &[], args)
    }

    fn call_function_with_captures(
        &mut self,
        function: Rc<Function>,
        captures: &[Value],
        args: Vec<Value>,
    ) -> PhpResult<Value> {
        if self.depth >= MAX_CALL_DEPTH {
            return Err(PhpError::fatal(
                "Maximum function nesting level reached (infinite recursion?)",
            ));
        }

        let mut frame = Frame::new(function.max_slots as usize);
        for (slot, value) in function.capture_slots.iter().zip(captures.iter()) {
            frame.set(*slot, value.clone());
        }
        let mut arg_index = 0usize;
        for param in &function.params {
            if param.variadic {
                let rest: Vec<Value> = args.iter().skip(arg_index).cloned().collect();
                frame.set(param.slot, Value::array(PhpArray::from(rest)));
                break;
            }
            if arg_index < args.len() {
                frame.set(param.slot, args[arg_index].clone());
                arg_index += 1;
            } else if let Some(default) = &param.default {
                frame.set(param.slot, default.clone());
            } else {
                return Err(PhpError::argument_count(format!(
                    "Too few arguments to function {}(); {} passed and at least {} expected",
                    function.display_name(),
                    args.len(),
                    function.params.len()
                )));
            }
        }

        self.depth += 1;
        let result = self.run_chunk(&function.chunk, &function.slot_names, &mut frame);
        self.depth -= 1;
        result
    }

    /// Invoke a callable value: a closure or a function name.
    pub fn invoke(&mut self, callable: Value, args: Vec<Value>) -> PhpResult<Value> {
        match callable {
            Value::Closure(closure) => {
                let lower = closure.function.to_ascii_lowercase();
                let function = self.functions.get(&lower).cloned().ok_or_else(|| {
                    PhpError::fatal(format!("Closure function {} not found", closure.function))
                })?;
                self.call_function_with_captures(function, &closure.captures, args)
            }
            Value::Str(name) => {
                let name = String::from_utf8_lossy(name.as_bytes()).into_owned();
                self.call_named(&name, args)
            }
            other => Err(PhpError::fatal(format!(
                "Value of type {} is not callable",
                other.debug_type_name()
            ))),
        }
    }

    fn run_chunk(
        &mut self,
        chunk: &Chunk,
        slot_names: &[Option<String>],
        frame: &mut Frame,
    ) -> PhpResult<Value> {
        let mut stack: Vec<Value> = Vec::with_capacity(16);
        let mut ip = 0usize;

        while ip < chunk.ops.len() {
            let op = &chunk.ops[ip];
            ip += 1;

            match op {
                Op::Null => stack.push(Value::Null),
                Op::True => stack.push(Value::Bool(true)),
                Op::False => stack.push(Value::Bool(false)),
                Op::Int(value) => stack.push(Value::Int(*value)),
                Op::Float(value) => stack.push(Value::Float(*value)),
                Op::StrConst(index) => stack.push(chunk.consts[*index as usize].clone()),
                Op::ConstName(index) => {
                    let name = String::from_utf8_lossy(chunk.const_str(*index)).into_owned();
                    match self.constants.get(&name) {
                        Some(value) => stack.push(value.clone()),
                        None => {
                            return Err(PhpError::fatal(format!("Undefined constant \"{name}\"")))
                        }
                    }
                }
                Op::Pop => {
                    stack.pop();
                }
                Op::Dup => {
                    if let Some(value) = stack.last() {
                        stack.push(value.clone());
                    }
                }
                Op::LoadVar(slot) => {
                    let index = *slot as usize;
                    if !frame.defined.get(index).copied().unwrap_or(false) {
                        let name = slot_names
                            .get(index)
                            .and_then(|name| name.clone())
                            .unwrap_or_else(|| "this".to_string());
                        self.warn(format!("Undefined variable ${name}"));
                    }
                    stack.push(frame.get(*slot).clone());
                }
                Op::LoadVarSafe(slot) => {
                    let index = *slot as usize;
                    let value = if frame.defined.get(index).copied().unwrap_or(false) {
                        frame.get(*slot).clone()
                    } else {
                        Value::Null
                    };
                    stack.push(value);
                }
                Op::StoreVar(slot) => {
                    let value = stack.pop().unwrap_or(Value::Null);
                    frame.set(*slot, value);
                }
                Op::StoreVarKeep(slot) => {
                    let value = stack.last().cloned().unwrap_or(Value::Null);
                    frame.set(*slot, value);
                }
                Op::LoadGlobal(index) => {
                    let name = String::from_utf8_lossy(chunk.const_str(*index)).into_owned();
                    let key = Key::from_str(&name);
                    match self.globals.get(&key) {
                        Some(value) => stack.push(value.clone()),
                        None => {
                            self.warn(format!("Undefined variable ${name}"));
                            stack.push(Value::Null);
                        }
                    }
                }
                Op::LoadGlobalSafe(index) => {
                    let name = String::from_utf8_lossy(chunk.const_str(*index)).into_owned();
                    let key = Key::from_str(&name);
                    stack.push(self.globals.get(&key).cloned().unwrap_or(Value::Null));
                }
                Op::StoreGlobal(index) => {
                    let name = String::from_utf8_lossy(chunk.const_str(*index)).into_owned();
                    let value = stack.pop().unwrap_or(Value::Null);
                    self.globals.insert(Key::from_str(&name), value);
                }
                Op::StoreGlobalKeep(index) => {
                    let name = String::from_utf8_lossy(chunk.const_str(*index)).into_owned();
                    let value = stack.last().cloned().unwrap_or(Value::Null);
                    self.globals.insert(Key::from_str(&name), value);
                }
                Op::LoadTmp(slot) => stack.push(frame.get(*slot).clone()),
                Op::StoreTmp(slot) => {
                    let value = stack.pop().unwrap_or(Value::Null);
                    frame.set(*slot, value);
                }
                Op::Binary(binary) => {
                    let rhs = stack.pop().unwrap_or(Value::Null);
                    let lhs = stack.pop().unwrap_or(Value::Null);
                    let result = self.binary_op(*binary, lhs, rhs)?;
                    stack.push(result);
                }
                Op::Unary(unary) => {
                    let value = stack.pop().unwrap_or(Value::Null);
                    let result = self.unary_op(*unary, value)?;
                    stack.push(result);
                }
                Op::Cast(kind) => {
                    let value = stack.pop().unwrap_or(Value::Null);
                    let result = self.cast_value(*kind, value);
                    stack.push(result);
                }
                Op::IncDec { increment } => {
                    let value = stack.pop().unwrap_or(Value::Null);
                    let result = if *increment {
                        increment_value(value)
                    } else {
                        decrement_value(value)
                    };
                    stack.push(result);
                }
                Op::ToBool => {
                    let value = stack.pop().unwrap_or(Value::Null);
                    stack.push(Value::Bool(value.to_bool()));
                }
                Op::IsNotNull => {
                    let value = stack.pop().unwrap_or(Value::Null);
                    stack.push(Value::Bool(!value.is_null()));
                }
                Op::Jump(target) => ip = *target,
                Op::JumpIfFalse(target) => {
                    let value = stack.pop().unwrap_or(Value::Null);
                    if !value.to_bool() {
                        ip = *target;
                    }
                }
                Op::JumpIfTrue(target) => {
                    let value = stack.pop().unwrap_or(Value::Null);
                    if value.to_bool() {
                        ip = *target;
                    }
                }
                Op::JumpIfFalseKeep(target) => {
                    if !stack.last().map(Value::to_bool).unwrap_or(false) {
                        stack.pop();
                        ip = *target;
                    }
                }
                Op::JumpIfTrueKeep(target) => {
                    if stack.last().map(Value::to_bool).unwrap_or(false) {
                        stack.pop();
                        ip = *target;
                    }
                }
                Op::JumpIfNotNull(target) => {
                    let value = stack.pop().unwrap_or(Value::Null);
                    if !value.is_null() {
                        stack.push(value);
                        ip = *target;
                    }
                }
                Op::Echo => {
                    let value = stack.pop().unwrap_or(Value::Null);
                    let bytes = self.output_bytes(&value)?;
                    self.output.extend_from_slice(&bytes);
                }
                Op::ArrayNew => stack.push(Value::array(PhpArray::new())),
                Op::ArraySet { tmp, has_key } => {
                    let value = stack.pop().unwrap_or(Value::Null);
                    let key = if *has_key {
                        let key_value = stack.pop().unwrap_or(Value::Null);
                        Some(key_from_value(&key_value)?)
                    } else {
                        None
                    };
                    let slot = *tmp as usize;
                    if let Some(Value::Array(array)) = frame.slots.get_mut(slot) {
                        let array = Rc::make_mut(array);
                        match key {
                            Some(key) => array.insert(key, value),
                            None => {
                                array.push(value);
                            }
                        }
                    } else {
                        let mut array = PhpArray::new();
                        match key {
                            Some(key) => array.insert(key, value),
                            None => {
                                array.push(value);
                            }
                        }
                        frame.set(*tmp, Value::array(array));
                    }
                }
                Op::Index => {
                    let key_value = stack.pop().unwrap_or(Value::Null);
                    let base = stack.pop().unwrap_or(Value::Null);
                    let value = self.index_value(base, &key_value, true)?;
                    stack.push(value);
                }
                Op::IndexSafe => {
                    let key_value = stack.pop().unwrap_or(Value::Null);
                    let base = stack.pop().unwrap_or(Value::Null);
                    let value = self.index_value(base, &key_value, false)?;
                    stack.push(value);
                }
                Op::AssignDim { modes } => {
                    let value = stack.pop().unwrap_or(Value::Null);
                    let mut keys: Vec<Option<Value>> = Vec::with_capacity(modes.len());
                    for mode in modes.iter().rev() {
                        match mode {
                            DimMode::Key => keys.push(Some(stack.pop().unwrap_or(Value::Null))),
                            DimMode::Append => keys.push(None),
                        }
                    }
                    keys.reverse();
                    let base = stack.pop().unwrap_or(Value::Null);
                    let mut array = match base {
                        Value::Array(array) => (*array).clone(),
                        Value::Null => PhpArray::new(),
                        other => {
                            self.warn(format!(
                                "Cannot use a value of type {} as an array",
                                other.debug_type_name()
                            ));
                            PhpArray::new()
                        }
                    };
                    self.insert_path(&mut array, &keys, value)?;
                    stack.push(Value::array(array));
                }
                Op::Call { name, argc } => {
                    let name = String::from_utf8_lossy(chunk.const_str(*name)).into_owned();
                    let argc = *argc as usize;
                    let args = if argc == 0 {
                        Vec::new()
                    } else {
                        stack.split_off(stack.len() - argc)
                    };
                    let result = self.call_named(&name, args)?;
                    stack.push(result);
                }
                Op::CallValue { argc } => {
                    let argc = *argc as usize;
                    let args = if argc == 0 {
                        Vec::new()
                    } else {
                        stack.split_off(stack.len() - argc)
                    };
                    let callee = stack.pop().unwrap_or(Value::Null);
                    let result = self.invoke(callee, args)?;
                    stack.push(result);
                }
                Op::MakeClosure { name, argc } => {
                    let name = String::from_utf8_lossy(chunk.const_str(*name)).into_owned();
                    let argc = *argc as usize;
                    let captures = if argc == 0 {
                        Vec::new()
                    } else {
                        stack.split_off(stack.len() - argc)
                    };
                    let (line, capture_names, parameters) = self
                        .functions
                        .get(&name.to_ascii_lowercase())
                        .map(|function| {
                            (
                                function.line,
                                function.capture_names.clone(),
                                function
                                    .params
                                    .iter()
                                    .map(|param| {
                                        (
                                            param.name.clone(),
                                            param.default.is_some() || param.variadic,
                                        )
                                    })
                                    .collect(),
                            )
                        })
                        .unwrap_or((0, Vec::new(), Vec::new()));
                    let id = self.next_object_id;
                    self.next_object_id += 1;
                    stack.push(Value::closure(
                        name,
                        self.file.clone(),
                        line,
                        id,
                        capture_names,
                        parameters,
                        captures,
                    ));
                }
                Op::ThrowUnhandledMatch { subject } => {
                    let value = frame.get(*subject);
                    return Err(PhpError::new(
                        ErrorKind::UnhandledMatch,
                        format!("Unhandled match case of type {}", value.type_name()),
                    ));
                }
                Op::IterInit { iter } => {
                    let subject = stack.pop().unwrap_or(Value::Null);
                    let array = match subject {
                        Value::Array(array) => (*array).clone(),
                        other => {
                            self.warn(format!(
                                "foreach() argument must be of type array|object, {} given",
                                other.debug_type_name()
                            ));
                            PhpArray::new()
                        }
                    };
                    frame.iters.insert(*iter, IterState { array, pos: 0 });
                }
                Op::IterNext {
                    iter,
                    key,
                    value,
                    end,
                } => {
                    let next = match frame.iters.get_mut(iter) {
                        Some(state) if state.pos < state.array.len() => {
                            let entry = state
                                .array
                                .iter()
                                .nth(state.pos)
                                .map(|(key, value)| (key.clone(), value.clone()));
                            state.pos += 1;
                            entry
                        }
                        _ => None,
                    };
                    match next {
                        Some((entry_key, entry_value)) => {
                            frame.set(*value, entry_value);
                            if let Some(key_slot) = key {
                                let key_value = match entry_key {
                                    Key::Int(number) => Value::Int(number),
                                    Key::Str(string) => Value::Str(string),
                                };
                                frame.set(*key_slot, key_value);
                            }
                        }
                        None => {
                            frame.iters.remove(iter);
                            ip = *end;
                        }
                    }
                }
                Op::Return => {
                    return Ok(stack.pop().unwrap_or(Value::Null));
                }
                Op::ReturnNull => return Ok(Value::Null),
            }
        }

        Ok(Value::Null)
    }

    fn index_value(&mut self, base: Value, key_value: &Value, warn: bool) -> PhpResult<Value> {
        match base {
            Value::Array(array) => {
                let key = key_from_value(key_value)?;
                match array.get(&key) {
                    Some(value) => Ok(value.clone()),
                    None => {
                        if warn {
                            self.warn(format!("Undefined array key {}", display_key(&key)));
                        }
                        Ok(Value::Null)
                    }
                }
            }
            Value::Null => {
                if warn {
                    self.warn("Trying to access array offset on value of type null");
                }
                Ok(Value::Null)
            }
            Value::Str(string) => {
                let offset = match key_value {
                    Value::Int(index) => *index,
                    Value::Str(text) => match phpcraft_runtime::value::parse_numeric(&text.0) {
                        Some((number, true)) => match number {
                            Numeric::Int(index) => index,
                            Numeric::Float(index) => phpcraft_runtime::value::float_to_int(index),
                        },
                        _ => {
                            if warn {
                                self.warn(format!(
                                    "Illegal string offset \"{}\"",
                                    text.to_string_lossy()
                                ));
                            }
                            return Ok(Value::Null);
                        }
                    },
                    other => other.to_int_cast(),
                };
                let bytes = string.as_bytes();
                let mut offset = offset;
                if offset < 0 {
                    offset += bytes.len() as i64;
                }
                if offset < 0 || offset >= bytes.len() as i64 {
                    if warn {
                        self.warn(format!("Uninitialized string offset {offset}"));
                    }
                    Ok(Value::Null)
                } else {
                    Ok(Value::str(vec![bytes[offset as usize]]))
                }
            }
            other => {
                if warn {
                    self.warn(format!(
                        "Trying to access array offset on value of type {}",
                        other.debug_type_name()
                    ));
                }
                Ok(Value::Null)
            }
        }
    }

    fn insert_path(
        &mut self,
        array: &mut PhpArray,
        keys: &[Option<Value>],
        value: Value,
    ) -> PhpResult<()> {
        let (first, rest) = keys.split_first().expect("at least one key");
        let key = match first {
            Some(key_value) => key_from_value(key_value)?,
            None => Key::Int(array.next_index()),
        };
        if rest.is_empty() {
            array.insert(key, value);
            return Ok(());
        }
        let mut child = match array.get(&key) {
            Some(Value::Array(child)) => (**child).clone(),
            Some(_) => {
                self.warn("Cannot use a scalar value as an array");
                PhpArray::new()
            }
            None => PhpArray::new(),
        };
        self.insert_path(&mut child, rest, value)?;
        array.insert(key, Value::array(child));
        Ok(())
    }

    // ------------------------------------------------------------- operators

    fn binary_op(&mut self, op: BinOp, lhs: Value, rhs: Value) -> PhpResult<Value> {
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow => {
                self.arithmetic(op, lhs, rhs)
            }
            BinOp::Concat => {
                let mut out = self.output_bytes(&lhs)?;
                out.extend_from_slice(&self.output_bytes(&rhs)?);
                Ok(Value::str(out))
            }
            BinOp::Shl | BinOp::Shr | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                let a = self.integer_operand(&lhs, op)?;
                let b = self.integer_operand(&rhs, op)?;
                Ok(Value::Int(match op {
                    BinOp::Shl => {
                        if b < 0 {
                            return Err(PhpError::arithmetic("Bit shift by negative number"));
                        }
                        if b >= 64 {
                            0
                        } else {
                            a.wrapping_shl(b as u32)
                        }
                    }
                    BinOp::Shr => {
                        if b < 0 {
                            return Err(PhpError::arithmetic("Bit shift by negative number"));
                        }
                        if b >= 64 {
                            if a < 0 {
                                -1
                            } else {
                                0
                            }
                        } else {
                            a >> b
                        }
                    }
                    BinOp::BitAnd => a & b,
                    BinOp::BitOr => a | b,
                    BinOp::BitXor => a ^ b,
                    _ => unreachable!(),
                }))
            }
            BinOp::Eq => Ok(Value::Bool(loose_eq(&lhs, &rhs))),
            BinOp::NotEq => Ok(Value::Bool(!loose_eq(&lhs, &rhs))),
            BinOp::Identical => Ok(Value::Bool(strict_eq(&lhs, &rhs))),
            BinOp::NotIdentical => Ok(Value::Bool(!strict_eq(&lhs, &rhs))),
            BinOp::Lt => Ok(Value::Bool(matches!(
                compare(&lhs, &rhs),
                Some(std::cmp::Ordering::Less)
            ))),
            BinOp::Le => Ok(Value::Bool(matches!(
                compare(&lhs, &rhs),
                Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
            ))),
            BinOp::Gt => Ok(Value::Bool(matches!(
                compare(&lhs, &rhs),
                Some(std::cmp::Ordering::Greater)
            ))),
            BinOp::Ge => Ok(Value::Bool(matches!(
                compare(&lhs, &rhs),
                Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
            ))),
            BinOp::Spaceship => Ok(Value::Int(match compare(&lhs, &rhs) {
                Some(std::cmp::Ordering::Less) => -1,
                Some(std::cmp::Ordering::Greater) => 1,
                _ => 0,
            })),
            BinOp::LogicalAnd => Ok(Value::Bool(lhs.to_bool() && rhs.to_bool())),
            BinOp::LogicalOr => Ok(Value::Bool(lhs.to_bool() || rhs.to_bool())),
            BinOp::LogicalXor => Ok(Value::Bool(lhs.to_bool() ^ rhs.to_bool())),
            BinOp::Coalesce => {
                if lhs.is_null() {
                    Ok(rhs)
                } else {
                    Ok(lhs)
                }
            }
        }
    }

    fn numeric_operand(&mut self, value: &Value, op: BinOp, other: &Value) -> PhpResult<Numeric> {
        match value.to_number() {
            ToNumber::Num(number) => Ok(number),
            ToNumber::Leading(number) => {
                self.warn("A non-numeric value encountered");
                Ok(number)
            }
            ToNumber::Invalid => Err(PhpError::type_error(format!(
                "Unsupported operand types: {} {} {}",
                value.debug_type_name(),
                op.symbol(),
                other.debug_type_name()
            ))),
        }
    }

    fn integer_operand(&mut self, value: &Value, op: BinOp) -> PhpResult<i64> {
        match value.to_number() {
            ToNumber::Num(Numeric::Int(number)) => Ok(number),
            ToNumber::Num(Numeric::Float(number)) => {
                Ok(phpcraft_runtime::value::float_to_int(number))
            }
            ToNumber::Leading(Numeric::Int(number)) => {
                self.warn("A non-numeric value encountered");
                Ok(number)
            }
            ToNumber::Leading(Numeric::Float(number)) => {
                self.warn("A non-numeric value encountered");
                Ok(phpcraft_runtime::value::float_to_int(number))
            }
            ToNumber::Invalid => Err(PhpError::type_error(format!(
                "Unsupported operand types: {} {} int",
                value.debug_type_name(),
                op.symbol()
            ))),
        }
    }

    fn arithmetic(&mut self, op: BinOp, lhs: Value, rhs: Value) -> PhpResult<Value> {
        let a = self.numeric_operand(&lhs, op, &rhs)?;
        let b = self.numeric_operand(&rhs, op, &lhs)?;

        match op {
            BinOp::Add => Ok(add_numbers(a, b)),
            BinOp::Sub => Ok(sub_numbers(a, b)),
            BinOp::Mul => Ok(mul_numbers(a, b)),
            BinOp::Div => {
                if b.to_float() == 0.0 {
                    return Err(PhpError::arithmetic("Division by zero"));
                }
                match (a, b) {
                    (Numeric::Int(x), Numeric::Int(y)) => match x.checked_rem(y) {
                        Some(0) => Ok(match x.checked_div(y) {
                            Some(value) => Value::Int(value),
                            None => Value::Float(x as f64 / y as f64),
                        }),
                        _ => Ok(Value::Float(x as f64 / y as f64)),
                    },
                    _ => Ok(Value::Float(a.to_float() / b.to_float())),
                }
            }
            BinOp::Mod => {
                let x = match a {
                    Numeric::Int(value) => value,
                    Numeric::Float(value) => phpcraft_runtime::value::float_to_int(value),
                };
                let y = match b {
                    Numeric::Int(value) => value,
                    Numeric::Float(value) => phpcraft_runtime::value::float_to_int(value),
                };
                if y == 0 {
                    return Err(PhpError::arithmetic("Modulo by zero"));
                }
                Ok(Value::Int(x.wrapping_rem(y)))
            }
            BinOp::Pow => {
                if let (Numeric::Int(base), Numeric::Int(exponent)) = (a, b) {
                    if exponent >= 0 && exponent <= u32::MAX as i64 {
                        if let Some(value) = base.checked_pow(exponent as u32) {
                            return Ok(Value::Int(value));
                        }
                    }
                }
                Ok(Value::Float(a.to_float().powf(b.to_float())))
            }
            _ => unreachable!(),
        }
    }

    fn cast_value(&mut self, kind: CastKind, value: Value) -> Value {
        match kind {
            CastKind::Int => Value::Int(value.to_int_cast()),
            CastKind::Float => Value::Float(value.to_float_cast()),
            CastKind::Bool => Value::Bool(value.to_bool()),
            CastKind::String => {
                if matches!(value, Value::Array(_)) {
                    self.warn("Array to string conversion");
                }
                Value::str(value.to_bytes())
            }
            CastKind::Array => match value {
                Value::Array(_) => value,
                Value::Null => Value::array(PhpArray::new()),
                other => {
                    let mut array = PhpArray::new();
                    array.push(other);
                    Value::array(array)
                }
            },
        }
    }

    fn unary_op(&mut self, op: UnOp, value: Value) -> PhpResult<Value> {
        match op {
            UnOp::Not => Ok(Value::Bool(!value.to_bool())),
            UnOp::Neg => match value.to_number() {
                ToNumber::Num(Numeric::Int(number)) => Ok(match number.checked_neg() {
                    Some(value) => Value::Int(value),
                    None => Value::Float(-(number as f64)),
                }),
                ToNumber::Num(Numeric::Float(number)) => Ok(Value::Float(-number)),
                ToNumber::Leading(Numeric::Int(number)) => {
                    self.warn("A non-numeric value encountered");
                    Ok(Value::Int(number.wrapping_neg()))
                }
                ToNumber::Leading(Numeric::Float(number)) => {
                    self.warn("A non-numeric value encountered");
                    Ok(Value::Float(-number))
                }
                ToNumber::Invalid => Err(PhpError::type_error(format!(
                    "Unsupported operand types: {} * int",
                    value.debug_type_name()
                ))),
            },
            UnOp::Pos => match value.to_number() {
                ToNumber::Num(number) | ToNumber::Leading(number) => Ok(match number {
                    Numeric::Int(number) => Value::Int(number),
                    Numeric::Float(number) => Value::Float(number),
                }),
                ToNumber::Invalid => Err(PhpError::type_error(format!(
                    "Unsupported operand types: {} * int",
                    value.debug_type_name()
                ))),
            },
            UnOp::BitNot => {
                let integer = self.integer_operand(&value, BinOp::BitXor)?;
                Ok(Value::Int(!integer))
            }
        }
    }
}

impl RuntimeHooks for Vm {
    fn write(&mut self, bytes: &[u8]) {
        self.output.extend_from_slice(bytes);
    }

    fn warn(&mut self, message: &str) {
        self.warn(message.to_string());
    }

    fn call(&mut self, name: &str, args: Vec<Value>) -> PhpResult<Value> {
        self.call_named(name, args)
    }

    fn call_value(&mut self, callable: &Value, args: Vec<Value>) -> PhpResult<Value> {
        self.invoke(callable.clone(), args)
    }

    fn function_exists(&mut self, name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        self.functions.contains_key(&lower) || phpcraft_stdlib::find(&lower).is_some()
    }

    fn constant(&mut self, name: &str) -> Option<Value> {
        self.constants.get(name).cloned()
    }

    fn define_constant(&mut self, name: &str, value: Value) {
        self.define_constant(name, value);
    }
}

/// `increment_function()` from `Zend/zend_operators.c`.
fn increment_value(value: Value) -> Value {
    match value {
        Value::Int(number) => match number.checked_add(1) {
            Some(result) => Value::Int(result),
            None => Value::Float(number as f64 + 1.0),
        },
        Value::Float(number) => Value::Float(number + 1.0),
        Value::Null => Value::Int(1),
        Value::Str(string) => match phpcraft_runtime::value::parse_numeric(&string.0) {
            Some((Numeric::Int(number), _)) => match number.checked_add(1) {
                Some(result) => Value::Int(result),
                None => Value::Float(number as f64 + 1.0),
            },
            Some((Numeric::Float(number), _)) => Value::Float(number + 1.0),
            None => Value::str(increment_string(&string.0)),
        },
        other => other,
    }
}

/// `decrement_function()`: strings are only incremented in PHP.
fn decrement_value(value: Value) -> Value {
    match value {
        Value::Int(number) => match number.checked_sub(1) {
            Some(result) => Value::Int(result),
            None => Value::Float(number as f64 - 1.0),
        },
        Value::Float(number) => Value::Float(number - 1.0),
        Value::Str(string) if string.0.is_empty() => Value::Int(-1),
        Value::Str(string) => match phpcraft_runtime::value::parse_numeric(&string.0) {
            Some((Numeric::Int(number), _)) => match number.checked_sub(1) {
                Some(result) => Value::Int(result),
                None => Value::Float(number as f64 - 1.0),
            },
            Some((Numeric::Float(number), _)) => Value::Float(number - 1.0),
            None => Value::Str(string),
        },
        other => other,
    }
}

/// Perl-style string increment (`increment_string` in `zend_operators.c`).
fn increment_string(bytes: &[u8]) -> Vec<u8> {
    if bytes.is_empty() {
        return b"1".to_vec();
    }
    let mut result = bytes.to_vec();
    let mut carry = false;
    let mut last = b'a';
    let mut position = result.len() - 1;
    loop {
        let ch = result[position];
        if ch.is_ascii_lowercase() {
            last = b'a';
            if ch == b'z' {
                result[position] = b'a';
                carry = true;
            } else {
                result[position] = ch + 1;
                carry = false;
            }
        } else if ch.is_ascii_uppercase() {
            last = b'A';
            if ch == b'Z' {
                result[position] = b'A';
                carry = true;
            } else {
                result[position] = ch + 1;
                carry = false;
            }
        } else if ch.is_ascii_digit() {
            last = b'1';
            if ch == b'9' {
                result[position] = b'0';
                carry = true;
            } else {
                result[position] = ch + 1;
                carry = false;
            }
        } else {
            break;
        }
        if !carry || position == 0 {
            break;
        }
        position -= 1;
    }
    if carry {
        result.insert(0, last);
    }
    result
}

fn display_key(key: &Key) -> String {
    match key {
        Key::Int(number) => number.to_string(),
        Key::Str(string) => format!("\"{}\"", string.to_string_lossy()),
    }
}

fn add_numbers(a: Numeric, b: Numeric) -> Value {
    match (a, b) {
        (Numeric::Int(x), Numeric::Int(y)) => match x.checked_add(y) {
            Some(value) => Value::Int(value),
            None => Value::Float(x as f64 + y as f64),
        },
        _ => Value::Float(a.to_float() + b.to_float()),
    }
}

fn sub_numbers(a: Numeric, b: Numeric) -> Value {
    match (a, b) {
        (Numeric::Int(x), Numeric::Int(y)) => match x.checked_sub(y) {
            Some(value) => Value::Int(value),
            None => Value::Float(x as f64 - y as f64),
        },
        _ => Value::Float(a.to_float() - b.to_float()),
    }
}

fn mul_numbers(a: Numeric, b: Numeric) -> Value {
    match (a, b) {
        (Numeric::Int(x), Numeric::Int(y)) => match x.checked_mul(y) {
            Some(value) => Value::Int(value),
            None => Value::Float(x as f64 * y as f64),
        },
        _ => Value::Float(a.to_float() * b.to_float()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use phpcraft_compiler::compile;
    use phpcraft_parser::parse_file;

    fn run(source: &str) -> (String, Vec<String>) {
        let program = parse_file(source.as_bytes()).expect("parses");
        let compiled = compile(&program, "test.php").expect("compiles");
        let mut vm = Vm::new();
        vm.run_main(&compiled).expect("runs");
        (
            String::from_utf8_lossy(vm.output()).into_owned(),
            vm.warnings().to_vec(),
        )
    }

    #[test]
    fn arithmetic_and_echo() {
        let (out, _) = run("<?php echo 1 + 2 * 3, \"|\", 7 / 2, \"|\", 6 / 2, \"|\", 2 ** 10;");
        assert_eq!(out, "7|3.5|3|1024");
    }

    #[test]
    fn arrays_are_values() {
        let (out, _) = run(r#"<?php
            $a = [1, 2, 3];
            $b = $a;
            $b[] = 4;
            $b["x"] = 5;
            echo count($a), "|", count($b), "|", implode(",", $b);
        "#);
        assert_eq!(out, "3|5|1,2,3,4,5");
    }

    #[test]
    fn loops_and_functions() {
        let (out, _) = run(r#"<?php
            function fib($n) {
                if ($n < 2) { return $n; }
                return fib($n - 1) + fib($n - 2);
            }
            $out = [];
            for ($i = 0; $i < 10; $i++) { $out[] = fib($i); }
            foreach ($out as $k => $v) { echo $k, ":", $v, " "; }
        "#);
        assert_eq!(out, "0:0 1:1 2:1 3:2 4:3 5:5 6:8 7:13 8:21 9:34 ");
    }

    #[test]
    fn strings_and_interpolation() {
        let (out, _) = run(r#"<?php
            $name = "world";
            $a = ["k" => "v"];
            echo "hello $name {$a["k"]}", "|", strtoupper("abc"), "|", substr("abcdef", 1, 3);
        "#);
        assert_eq!(out, "hello world v|ABC|bcd");
    }

    #[test]
    fn switch_and_ternary() {
        let (out, _) = run(r#"<?php
            foreach ([1, 2, 3, 4] as $x) {
                switch ($x) {
                    case 1: echo "one"; break;
                    case 2:
                    case 3: echo "two-or-three"; break;
                    default: echo "many";
                }
                echo $x % 2 == 0 ? "-even " : "-odd ";
            }
        "#);
        assert_eq!(out, "one-odd two-or-three-even two-or-three-odd many-even ");
    }

    #[test]
    fn globals_are_shared() {
        let (out, _) = run(r#"<?php
            $counter = 0;
            function bump() { global $counter; $counter++; }
            bump(); bump();
            echo $counter;
        "#);
        assert_eq!(out, "2");
    }

    #[test]
    fn increment_and_decrement_follow_php() {
        let (out, _) = run(r#"<?php
            $s = "a"; $s++; echo $s, "|";
            $s = "Az"; $s++; echo $s, "|";
            $s = "z"; $s++; echo $s, "|";
            $s = "a9"; $s++; echo $s, "|";
            $s = "9"; $s++; echo $s, "|";
            $n = null; $n++; echo $n, "|";
            $n = null; $n--; var_dump($n);
            $s = "abc"; $s--; echo $s;
        "#);
        assert_eq!(out, "b|Ba|aa|b0|10|1|NULL\nabc");
    }

    #[test]
    fn closures_capture_by_value_and_arrow_functions() {
        let (out, _) = run(r#"<?php
            $n = 2;
            $closure = function ($x) use ($n) { return $x + $n; };
            $arrow = fn($x) => $x * $n;
            $n = 10;
            echo $closure(3), "|", $arrow(3), "|";
            $make = fn($a) => fn($b) => $a + $b;
            echo $make(4)(5), "|";
            echo array_map(fn($v) => $v + 1, [1, 2, 3])[2], "|";
            $name = "strlen";
            echo $name("abcd"), "|";
            echo $closure === $closure ? "same" : "different";
        "#);
        assert_eq!(out, "5|6|9|4|4|same");
    }

    #[test]
    fn append_uses_next_index_after_negative_key() {
        let (out, _) = run(r#"<?php
            $a = [-5 => "a"];
            $a[] = "b";
            echo implode(",", array_keys($a));
        "#);
        assert_eq!(out, "-5,-4");
    }
}
