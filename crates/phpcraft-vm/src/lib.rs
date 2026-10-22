//! PHPCraft virtual machine.
//!
//! Executes the stack-machine bytecode produced by `phpcraft-compiler`. The
//! structure mirrors `Zend/zend_vm_def.h`: a dispatch loop over an op array,
//! a call frame with slots, and a value stack. Function calls recurse in Rust
//! (Zend also uses the C stack), with an explicit nesting guard.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use phpcraft_ast::{BinOp, CastKind, UnOp};
use phpcraft_bytecode::{Chunk, ClassDef, CompiledProgram, DimMode, Function, Op};
use phpcraft_runtime::{
    compare, key_from_value, loose_eq, strict_eq, ErrorKind, ExceptionData, Key, Numeric, PhpArray,
    PhpError, PhpObject, PhpResult, RuntimeHooks, ToNumber, Value,
};

/// Maximum userland call depth, mirroring the usual `xdebug.max_nesting_level`
/// style guard rather than PHP's unbounded C recursion.
const MAX_CALL_DEPTH: u32 = 512;

struct IterState {
    array: PhpArray,
    pos: usize,
}

enum Control {
    Continue,
    Return(Value),
}

struct Handler {
    catch_ip: usize,
    catch_slot: u32,
    /// Stack depth at `TryStart`, restored when unwinding.
    stack_depth: usize,
}

struct Frame {
    slots: Vec<Value>,
    defined: Vec<bool>,
    iters: HashMap<u32, IterState>,
    handlers: Vec<Handler>,
    /// Defining class of the currently executing method (`self`).
    class_name: Option<String>,
    /// Runtime class of the receiver (`static`), when different.
    called_class: Option<String>,
}

impl Frame {
    fn new(size: usize) -> Self {
        Self {
            slots: vec![Value::Null; size],
            defined: vec![false; size],
            iters: HashMap::new(),
            handlers: Vec::new(),
            class_name: None,
            called_class: None,
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
    classes: HashMap<String, Rc<ClassDef>>,
    static_properties: HashMap<String, PhpArray>,
    globals: PhpArray,
    constants: HashMap<String, Value>,
    output: Vec<u8>,
    warnings: Vec<String>,
    depth: u32,
    file: String,
    next_object_id: u64,
    included_files: HashSet<String>,
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
            classes: HashMap::new(),
            static_properties: HashMap::new(),
            globals: PhpArray::new(),
            constants,
            output: Vec::new(),
            warnings: Vec::new(),
            depth: 0,
            file: String::new(),
            next_object_id: 1,
            included_files: HashSet::new(),
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
        for class in &program.classes {
            let name = class.name.to_ascii_lowercase();
            self.classes.insert(name.clone(), Rc::new(class.clone()));
            let mut statics = PhpArray::new();
            for property in class.properties.iter().filter(|property| property.static_) {
                statics.insert_str(&property.name, property.default.clone());
            }
            self.static_properties.insert(name, statics);
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

    fn include_file(&mut self, raw: &str, once: bool, require: bool) -> PhpResult<Value> {
        let path = self.resolve_include_path(raw);
        let key = std::fs::canonicalize(&path)
            .unwrap_or_else(|_| path.clone())
            .display()
            .to_string();
        if once && self.included_files.contains(&key) {
            return Ok(Value::Int(1));
        }
        let source = match std::fs::read(&path) {
            Ok(source) => source,
            Err(error) => {
                let message = format!(
                    "{}: Failed to open stream: {}",
                    if require { "require" } else { "include" },
                    error
                );
                if require {
                    return Err(PhpError::fatal(message));
                }
                self.warn(message);
                return Ok(Value::Bool(false));
            }
        };
        let program = phpcraft_parser::parse_file(&source).map_err(|error| {
            PhpError::fatal(format!(
                "Parse error: {} in {} on line {}",
                error.message, key, error.line
            ))
        })?;
        let compiled = phpcraft_compiler::compile(&program, &key).map_err(|error| {
            PhpError::fatal(format!(
                "{} in {} on line {}",
                error.message, key, error.line
            ))
        })?;
        self.included_files.insert(key.clone());
        let previous_file = std::mem::replace(&mut self.file, key.clone());
        self.load(&compiled);
        let main = Rc::new(compiled.main.clone());
        let result = self.call_function(main, Vec::new());
        self.file = previous_file;
        let value = result?;
        Ok(if matches!(value, Value::Null) {
            Value::Int(1)
        } else {
            value
        })
    }

    fn resolve_include_path(&self, raw: &str) -> PathBuf {
        let path = PathBuf::from(raw);
        if path.is_absolute()
            || matches!(
                self.file.as_str(),
                "Command line code" | "Standard input code"
            )
        {
            return path;
        }
        if let Some(parent) = Path::new(&self.file).parent() {
            let candidate = parent.join(&path);
            if candidate.exists() {
                return candidate;
            }
        }
        path
    }

    fn warn(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.warnings.push(message);
    }

    fn output_bytes(&mut self, value: &Value) -> PhpResult<Vec<u8>> {
        match value {
            Value::Closure(_) => Err(PhpError::fatal(
                "Object of class Closure could not be converted to string",
            )),
            Value::Object(_) => {
                let string = self.call_method_value(value.clone(), "__toString", Vec::new())?;
                if matches!(string, Value::Object(_) | Value::Closure(_)) {
                    return Err(PhpError::fatal("__toString() must return a string value"));
                }
                Ok(string.to_bytes())
            }
            other => Ok(other.to_bytes()),
        }
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
        self.call_function_with_context(function, captures, None, None, None, args)
    }

    fn call_function_with_context(
        &mut self,
        function: Rc<Function>,
        captures: &[Value],
        this: Option<Value>,
        class_name: Option<String>,
        called_class: Option<String>,
        args: Vec<Value>,
    ) -> PhpResult<Value> {
        if self.depth >= MAX_CALL_DEPTH {
            return Err(PhpError::fatal(
                "Maximum function nesting level reached (infinite recursion?)",
            ));
        }

        let mut frame = Frame::new(function.max_slots as usize);
        frame.class_name = class_name;
        frame.called_class = called_class;
        if let Some(this) = this {
            frame.set(0, this);
        }
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

    fn normalize_class(name: &str) -> String {
        name.trim_start_matches('\\').to_string()
    }

    fn builtin_parent(name: &str) -> Option<&'static str> {
        match name.to_ascii_lowercase().as_str() {
            "exception" | "error" => Some("Throwable"),
            "errorexception" | "logicexception" | "runtimeexception" => Some("Exception"),
            "invalidargumentexception"
            | "domainexception"
            | "lengthexception"
            | "outofrangeexception"
            | "badfunctioncallexception"
            | "badmethodcallexception" => Some("LogicException"),
            "outofboundsexception"
            | "rangeexception"
            | "overflowexception"
            | "underflowexception"
            | "unexpectedvalueexception" => Some("RuntimeException"),
            "typeerror"
            | "valueerror"
            | "argumentcounterror"
            | "arithmeticerror"
            | "divisionbyzeroerror"
            | "unhandledmatcherror"
            | "parseerror" => Some("Error"),
            _ => None,
        }
    }

    fn is_exception_class(&self, name: &str) -> bool {
        let lower = Self::normalize_class(name).to_ascii_lowercase();
        matches!(
            lower.as_str(),
            "throwable"
                | "exception"
                | "error"
                | "typeerror"
                | "valueerror"
                | "argumentcounterror"
                | "arithmeticerror"
                | "divisionbyzeroerror"
                | "unhandledmatcherror"
                | "parseerror"
        ) || self.class_chain(name).iter().any(|class| {
            class.eq_ignore_ascii_case("Exception") || class.eq_ignore_ascii_case("Error")
        })
    }

    fn canonical_class_name(&self, name: &str) -> Option<String> {
        let normalized = Self::normalize_class(name);
        if let Some(class) = self.classes.get(&normalized.to_ascii_lowercase()) {
            return Some(class.name.clone());
        }
        for builtin in [
            "Throwable",
            "Exception",
            "Error",
            "TypeError",
            "ValueError",
            "ArgumentCountError",
            "ArithmeticError",
            "DivisionByZeroError",
            "UnhandledMatchError",
            "ParseError",
            "ErrorException",
            "LogicException",
            "InvalidArgumentException",
            "DomainException",
            "LengthException",
            "OutOfRangeException",
            "BadFunctionCallException",
            "BadMethodCallException",
            "RuntimeException",
            "OutOfBoundsException",
            "RangeException",
            "OverflowException",
            "UnderflowException",
            "UnexpectedValueException",
            "Closure",
        ] {
            if builtin.eq_ignore_ascii_case(&normalized) {
                return Some(builtin.to_string());
            }
        }
        None
    }

    fn class_chain(&self, name: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut current = Some(Self::normalize_class(name));
        let mut guard = 0;
        while let Some(class) = current {
            if guard > 64 {
                break;
            }
            guard += 1;
            chain.push(class.clone());
            if let Some(def) = self.classes.get(&class.to_ascii_lowercase()) {
                current = def.parent.clone();
            } else {
                current = Self::builtin_parent(&class).map(str::to_string);
            }
        }
        chain
    }

    fn resolve_class_marker(&self, marker: &str, frame: &Frame) -> PhpResult<String> {
        let normalized = Self::normalize_class(marker);
        match normalized.to_ascii_lowercase().as_str() {
            "self" => frame.class_name.clone().ok_or_else(|| {
                PhpError::fatal("Cannot use \"self\" when no class scope is active")
            }),
            "static" => frame
                .called_class
                .clone()
                .or_else(|| frame.class_name.clone())
                .ok_or_else(|| {
                    PhpError::fatal("Cannot use \"static\" when no class scope is active")
                }),
            "parent" => {
                let class = frame.class_name.clone().ok_or_else(|| {
                    PhpError::fatal("Cannot use \"parent\" when no class scope is active")
                })?;
                self.class_chain(&class).get(1).cloned().ok_or_else(|| {
                    PhpError::fatal("Cannot use \"parent\" when no parent class exists")
                })
            }
            _ => Ok(normalized),
        }
    }

    fn value_instanceof(&self, value: &Value, target: &str) -> bool {
        let Some(class) = value.object_class() else {
            return false;
        };
        let target = Self::normalize_class(target);
        if target.eq_ignore_ascii_case("Closure") {
            return matches!(value, Value::Closure(_));
        }
        let chain = self.class_chain(&class);
        if target.eq_ignore_ascii_case("Throwable") {
            return chain.iter().any(|name| {
                name.eq_ignore_ascii_case("Exception") || name.eq_ignore_ascii_case("Error")
            });
        }
        chain
            .iter()
            .any(|name| name.eq_ignore_ascii_case(target.as_str()))
    }

    fn find_method(&self, class: &str, method: &str) -> Option<(Rc<Function>, String, bool)> {
        for owner in self.class_chain(class) {
            let Some(def) = self.classes.get(&owner.to_ascii_lowercase()) else {
                continue;
            };
            if let Some(method) = def.method(method) {
                if let Some(function) = self.functions.get(&method.function.to_ascii_lowercase()) {
                    return Some((function.clone(), def.name.clone(), method.static_));
                }
            }
        }
        None
    }

    fn new_object(&mut self, class: &str, args: Vec<Value>) -> PhpResult<Value> {
        let canonical = self.canonical_class_name(class).ok_or_else(|| {
            PhpError::fatal(format!(
                "Class \"{}\" not found",
                Self::normalize_class(class)
            ))
        })?;
        let mut properties = PhpArray::new();
        let mut chain = self.class_chain(&canonical);
        chain.reverse();
        for owner in chain {
            if let Some(def) = self.classes.get(&owner.to_ascii_lowercase()) {
                for property in def.properties.iter().filter(|property| !property.static_) {
                    properties.insert_str(&property.name, property.default.clone());
                }
            }
        }
        let id = self.next_object_id;
        self.next_object_id += 1;
        let mut object = PhpObject::new(canonical.clone(), id);
        object.properties = properties;
        for owner in self.class_chain(&canonical) {
            if let Some(def) = self.classes.get(&owner.to_ascii_lowercase()) {
                for property in def.properties.iter().filter(|property| !property.static_) {
                    object.property_visibility.insert(
                        property.name.clone(),
                        property.visibility.as_str().to_string(),
                    );
                }
            }
        }
        let is_exception = self.is_exception_class(&canonical);
        if is_exception {
            object.exception = Some(ExceptionData {
                message: String::new(),
                code: 0,
                file: self.file.clone(),
                line: 1,
                trace: Vec::new(),
            });
        }
        let value = Value::object(object);
        if let Some((function, owner, _)) = self.find_method(&canonical, "__construct") {
            self.call_function_with_context(
                function,
                &[],
                Some(value.clone()),
                Some(owner),
                Some(canonical),
                args,
            )?;
            return Ok(value);
        }
        if is_exception {
            self.init_exception(&value, args)?;
        }
        Ok(value)
    }

    fn init_exception(&mut self, object: &Value, args: Vec<Value>) -> PhpResult<()> {
        let message = args.first().map(Value::to_string_lossy).unwrap_or_default();
        let code = args.get(1).map(Value::to_int_cast).unwrap_or(0);
        if let Value::Object(object) = object {
            let mut object = object.borrow_mut();
            let line = object.exception.as_ref().map(|data| data.line).unwrap_or(0);
            object.exception = Some(ExceptionData {
                message: message.clone(),
                code,
                file: self.file.clone(),
                line,
                trace: Vec::new(),
            });
            object
                .properties
                .insert_str("message", Value::str(message.into_bytes()));
            object.properties.insert_str("code", Value::Int(code));
            object
                .properties
                .insert_str("file", Value::str(self.file.as_bytes().to_vec()));
            object
                .properties
                .insert_str("line", Value::Int(line as i64));
            for name in ["message", "code", "file", "line"] {
                object
                    .property_visibility
                    .insert(name.to_string(), "protected".to_string());
            }
        }
        Ok(())
    }

    fn builtin_exception_method(
        &mut self,
        object: &Value,
        method: &str,
        args: Option<Vec<Value>>,
    ) -> Option<PhpResult<Value>> {
        let class = object.object_class()?;
        if !self.is_exception_class(&class) {
            return None;
        }
        let lower = method.to_ascii_lowercase();
        if lower == "__construct" {
            let args = args.unwrap_or_default();
            return Some(self.init_exception(object, args).map(|()| Value::Null));
        }
        let data = match object {
            Value::Object(object) => object.borrow().exception.clone(),
            _ => None,
        }?;
        let result = match lower.as_str() {
            "getmessage" => Value::str(data.message.into_bytes()),
            "getcode" => Value::Int(data.code),
            "getfile" => Value::str(data.file.into_bytes()),
            "getline" => Value::Int(i64::from(data.line)),
            "gettrace" => Value::array(PhpArray::new()),
            "getprevious" => Value::Null,
            "gettraceasstring" => Value::str(b"#0 {main}\n".to_vec()),
            "__tostring" => {
                let text = format!(
                    "{}: {} in {}:{}\nStack trace:\n#0 {{main}}\n",
                    class, data.message, data.file, data.line
                );
                Value::str(text.into_bytes())
            }
            _ => return None,
        };
        Some(Ok(result))
    }

    fn call_method_value(
        &mut self,
        object: Value,
        method: &str,
        args: Vec<Value>,
    ) -> PhpResult<Value> {
        let class = object.object_class().ok_or_else(|| {
            PhpError::fatal(format!(
                "Call to a member function {}() on {}",
                method,
                object.debug_type_name()
            ))
        })?;
        if let Some((function, owner, _)) = self.find_method(&class, method) {
            return self.call_function_with_context(
                function,
                &[],
                Some(object),
                Some(owner),
                Some(class),
                args,
            );
        }
        if let Some(result) = self.builtin_exception_method(&object, method, Some(args)) {
            return result;
        }
        Err(PhpError::fatal(format!(
            "Call to undefined method {}::{}()",
            class, method
        )))
    }

    fn call_static(
        &mut self,
        class: &str,
        method: &str,
        args: Vec<Value>,
        frame: &Frame,
    ) -> PhpResult<Value> {
        let resolved = self.resolve_class_marker(class, frame)?;
        let receiver = frame
            .defined
            .first()
            .copied()
            .unwrap_or(false)
            .then(|| frame.get(0).clone());
        if let Some((function, owner, is_static)) = self.find_method(&resolved, method) {
            return self.call_function_with_context(
                function,
                &[],
                if is_static { None } else { receiver },
                Some(owner),
                Some(resolved),
                args,
            );
        }
        if self.is_exception_class(&resolved) {
            if let Some(this) = frame
                .defined
                .first()
                .copied()
                .unwrap_or(false)
                .then(|| frame.get(0).clone())
            {
                if let Some(result) = self.builtin_exception_method(&this, method, Some(args)) {
                    return result;
                }
            }
        }
        Err(PhpError::fatal(format!(
            "Call to undefined method {}::{}()",
            resolved, method
        )))
    }

    fn load_property(&mut self, object: Value, name: &str, warn: bool) -> PhpResult<Value> {
        if let Value::Object(object) = object {
            let value = object.borrow().properties.get_str(name).cloned();
            if value.is_none() && warn {
                let class = object.borrow().class_name.clone();
                self.warn(format!("Undefined property: {}::${}", class, name));
            }
            return Ok(value.unwrap_or(Value::Null));
        }
        if warn {
            self.warn(format!(
                "Attempt to read property \"{}\" on {}",
                name,
                object.debug_type_name()
            ));
        }
        Ok(Value::Null)
    }

    fn assign_property(&mut self, object: Value, name: &str, value: Value) -> PhpResult<()> {
        match object {
            Value::Object(object) => {
                object.borrow_mut().properties.insert_str(name, value);
                Ok(())
            }
            other => Err(PhpError::type_error(format!(
                "Attempt to assign property \"{}\" on {}",
                name,
                other.debug_type_name()
            ))),
        }
    }

    fn static_property_owner(&self, class: &str, name: &str) -> Option<String> {
        self.class_chain(class).into_iter().find(|owner| {
            self.static_properties
                .get(&owner.to_ascii_lowercase())
                .map(|properties| properties.contains_key(&Key::from_str(name)))
                .unwrap_or(false)
        })
    }

    fn load_static_property(&mut self, class: &str, name: &str, warn: bool) -> PhpResult<Value> {
        if let Some(owner) = self.static_property_owner(class, name) {
            if let Some(properties) = self.static_properties.get(&owner.to_ascii_lowercase()) {
                return Ok(properties.get_str(name).cloned().unwrap_or(Value::Null));
            }
        }
        if warn {
            self.warn(format!("Undefined static property {}::${}", class, name));
        }
        Ok(Value::Null)
    }

    fn assign_static_property(&mut self, class: &str, name: &str, value: Value) -> PhpResult<()> {
        let owner = self
            .static_property_owner(class, name)
            .unwrap_or_else(|| Self::normalize_class(class));
        self.static_properties
            .entry(owner.to_ascii_lowercase())
            .or_default()
            .insert_str(name, value);
        Ok(())
    }

    fn class_constant(&self, class: &str, name: &str) -> PhpResult<Value> {
        let canonical = self.canonical_class_name(class);
        if name.eq_ignore_ascii_case("class") {
            return Ok(Value::str(
                canonical
                    .unwrap_or_else(|| Self::normalize_class(class))
                    .into_bytes(),
            ));
        }
        for owner in self.class_chain(class) {
            if let Some(def) = self.classes.get(&owner.to_ascii_lowercase()) {
                if let Some((_, value)) = def
                    .constants
                    .iter()
                    .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
                {
                    return Ok(value.clone());
                }
            }
        }
        Err(PhpError::fatal(format!(
            "Undefined constant {}::{}",
            class, name
        )))
    }

    fn clone_object(&mut self, value: Value) -> PhpResult<Value> {
        match value {
            Value::Object(object) => {
                let mut clone = object.borrow().clone();
                clone.id = self.next_object_id;
                self.next_object_id += 1;
                Ok(Value::object(clone))
            }
            other => Err(PhpError::type_error(format!(
                "__clone method called on non-object ({})",
                other.debug_type_name()
            ))),
        }
    }

    fn throw_value(&self, value: Value) -> PhpError {
        match &value {
            Value::Object(object) if self.is_exception_class(&object.borrow().class_name) => {
                let message = object
                    .borrow()
                    .exception
                    .as_ref()
                    .map(|data| data.message.clone())
                    .unwrap_or_default();
                PhpError::thrown(value, message)
            }
            _ => PhpError::type_error("Can only throw objects that implement Throwable"),
        }
    }

    fn error_object(&mut self, error: &PhpError) -> Value {
        let class = match error.kind {
            ErrorKind::TypeError => "TypeError",
            ErrorKind::ValueError => "ValueError",
            ErrorKind::ArgumentCountError => "ArgumentCountError",
            ErrorKind::ArithmeticError => "ArithmeticError",
            ErrorKind::ParseError => "ParseError",
            ErrorKind::UnhandledMatch => "UnhandledMatchError",
            _ => "Error",
        };
        let id = self.next_object_id;
        self.next_object_id += 1;
        let mut object = PhpObject::new(class, id);
        object.exception = Some(ExceptionData {
            message: error.message.clone(),
            code: 0,
            file: self.file.clone(),
            line: 1,
            trace: Vec::new(),
        });
        object
            .properties
            .insert_str("message", Value::str(error.message.as_bytes().to_vec()));
        object.properties.insert_str("code", Value::Int(0));
        for name in ["message", "code", "file", "line"] {
            object
                .property_visibility
                .insert(name.to_string(), "protected".to_string());
        }
        Value::object(object)
    }

    fn run_chunk(
        &mut self,
        chunk: &Chunk,
        slot_names: &[Option<String>],
        frame: &mut Frame,
    ) -> PhpResult<Value> {
        let mut stack: Vec<Value> = Vec::with_capacity(16);
        let mut ip = 0usize;

        loop {
            if ip >= chunk.ops.len() {
                return Ok(Value::Null);
            }
            let op = &chunk.ops[ip];
            ip += 1;

            let outcome: PhpResult<Control> = (|| {
                match op {
                    Op::Null => stack.push(Value::Null),
                    Op::True => stack.push(Value::Bool(true)),
                    Op::False => stack.push(Value::Bool(false)),
                    Op::Int(value) => stack.push(Value::Int(*value)),
                    Op::Float(value) => stack.push(Value::Float(*value)),
                    Op::StrConst(index) => stack.push(chunk.consts[*index as usize].clone()),
                    Op::ConstName(index) => {
                        let name = String::from_utf8_lossy(chunk.const_str(*index)).into_owned();
                        let value = self.constants.get(&name).cloned().or_else(|| {
                            name.rsplit_once('\\')
                                .and_then(|(_, short)| self.constants.get(short).cloned())
                        });
                        match value {
                            Some(value) => stack.push(value),
                            None => {
                                return Err(PhpError::fatal(format!(
                                    "Undefined constant \"{name}\""
                                )))
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
                    Op::Call {
                        name,
                        argc,
                        fallback,
                    } => {
                        let mut name = String::from_utf8_lossy(chunk.const_str(*name)).into_owned();
                        let argc = *argc as usize;
                        let args = if argc == 0 {
                            Vec::new()
                        } else {
                            stack.split_off(stack.len() - argc)
                        };
                        let lower = name.to_ascii_lowercase();
                        let exists = self.functions.contains_key(&lower)
                            || phpcraft_stdlib::find(&lower).is_some();
                        if !exists && *fallback {
                            if let Some((_, short)) = name.rsplit_once('\\') {
                                name = short.to_string();
                            }
                        }
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
                    Op::Include { once, require } => {
                        let path = stack.pop().unwrap_or(Value::Null);
                        let Value::Str(path) = path else {
                            return Err(PhpError::type_error(format!(
                                "include(): Argument #1 ($path) must be of type string, {} given",
                                path.debug_type_name()
                            )));
                        };
                        let path = String::from_utf8_lossy(path.as_bytes()).into_owned();
                        let result = self.include_file(&path, *once, *require)?;
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
                    Op::NewObject { class, argc } => {
                        let argc = *argc as usize;
                        let args = if argc == 0 {
                            Vec::new()
                        } else {
                            stack.split_off(stack.len() - argc)
                        };
                        let class = String::from_utf8_lossy(chunk.const_str(*class)).into_owned();
                        let class = self.resolve_class_marker(&class, frame)?;
                        let object = self.new_object(&class, args)?;
                        stack.push(object);
                    }
                    Op::LoadProperty { name, safe } => {
                        let object = stack.pop().unwrap_or(Value::Null);
                        let name = String::from_utf8_lossy(chunk.const_str(*name)).into_owned();
                        stack.push(self.load_property(object, &name, !*safe)?);
                    }
                    Op::AssignProperty { name, keep } => {
                        let value = stack.pop().unwrap_or(Value::Null);
                        let object = stack.pop().unwrap_or(Value::Null);
                        let name = String::from_utf8_lossy(chunk.const_str(*name)).into_owned();
                        self.assign_property(object, &name, value.clone())?;
                        if *keep {
                            stack.push(value);
                        }
                    }
                    Op::CallMethod { method, argc } => {
                        let argc = *argc as usize;
                        let args = if argc == 0 {
                            Vec::new()
                        } else {
                            stack.split_off(stack.len() - argc)
                        };
                        let object = stack.pop().unwrap_or(Value::Null);
                        let method = String::from_utf8_lossy(chunk.const_str(*method)).into_owned();
                        stack.push(self.call_method_value(object, &method, args)?);
                    }
                    Op::CallStaticMethod {
                        class,
                        method,
                        argc,
                    } => {
                        let argc = *argc as usize;
                        let args = if argc == 0 {
                            Vec::new()
                        } else {
                            stack.split_off(stack.len() - argc)
                        };
                        let class = String::from_utf8_lossy(chunk.const_str(*class)).into_owned();
                        let method = String::from_utf8_lossy(chunk.const_str(*method)).into_owned();
                        stack.push(self.call_static(&class, &method, args, frame)?);
                    }
                    Op::LoadStaticProperty { class, name, safe } => {
                        let class = String::from_utf8_lossy(chunk.const_str(*class)).into_owned();
                        let name = String::from_utf8_lossy(chunk.const_str(*name)).into_owned();
                        let class = self.resolve_class_marker(&class, frame)?;
                        stack.push(self.load_static_property(&class, &name, !*safe)?);
                    }
                    Op::AssignStaticProperty { class, name, keep } => {
                        let value = stack.pop().unwrap_or(Value::Null);
                        let class = String::from_utf8_lossy(chunk.const_str(*class)).into_owned();
                        let name = String::from_utf8_lossy(chunk.const_str(*name)).into_owned();
                        let class = self.resolve_class_marker(&class, frame)?;
                        self.assign_static_property(&class, &name, value.clone())?;
                        if *keep {
                            stack.push(value);
                        }
                    }
                    Op::ClassConstant { class, name } => {
                        let class = String::from_utf8_lossy(chunk.const_str(*class)).into_owned();
                        let name = String::from_utf8_lossy(chunk.const_str(*name)).into_owned();
                        let class = self.resolve_class_marker(&class, frame)?;
                        stack.push(self.class_constant(&class, &name)?);
                    }
                    Op::InstanceOf { class } => {
                        let value = stack.pop().unwrap_or(Value::Null);
                        let class = String::from_utf8_lossy(chunk.const_str(*class)).into_owned();
                        let class = self.resolve_class_marker(&class, frame)?;
                        stack.push(Value::Bool(self.value_instanceof(&value, &class)));
                    }
                    Op::CloneObject => {
                        let value = stack.pop().unwrap_or(Value::Null);
                        stack.push(self.clone_object(value)?);
                    }
                    Op::Throw => {
                        let value = stack.pop().unwrap_or(Value::Null);
                        return Err(self.throw_value(value));
                    }
                    Op::TryStart {
                        catch_ip,
                        catch_slot,
                    } => {
                        frame.handlers.push(Handler {
                            catch_ip: *catch_ip,
                            catch_slot: *catch_slot,
                            stack_depth: stack.len(),
                        });
                    }
                    Op::TryEnd => {
                        frame.handlers.pop();
                    }
                    Op::Rethrow { slot } => {
                        let value = frame.get(*slot).clone();
                        return Err(self.throw_value(value));
                    }
                    Op::RethrowIfNotNull { slot } => {
                        let value = frame.get(*slot).clone();
                        if !value.is_null() {
                            return Err(self.throw_value(value));
                        }
                    }
                    Op::IterInit { iter } => {
                        let subject = stack.pop().unwrap_or(Value::Null);
                        let array = match subject {
                            Value::Array(array) => (*array).clone(),
                            Value::Object(object) => object.borrow().properties.clone(),
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
                        return Ok(Control::Return(stack.pop().unwrap_or(Value::Null)));
                    }
                    Op::ReturnNull => return Ok(Control::Return(Value::Null)),
                }
                Ok(Control::Continue)
            })();

            match outcome {
                Ok(Control::Continue) => {}
                Ok(Control::Return(value)) => return Ok(value),
                Err(error) => {
                    if let Some(handler) = frame.handlers.pop() {
                        let thrown = match error.thrown.clone() {
                            Some(value) => value,
                            None => self.error_object(&error),
                        };
                        stack.truncate(handler.stack_depth);
                        frame.set(handler.catch_slot, thrown);
                        ip = handler.catch_ip;
                        continue;
                    }
                    return Err(error);
                }
            }
        }
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

    #[test]
    fn objects_inheritance_and_static_members() {
        let (out, warnings) = run(r#"<?php
            class Base {
                public $value = 1;
                public static $count = 0;
                const KIND = "base";
                public function __construct($value) {
                    $this->value = $value;
                    self::$count++;
                }
                public function describe() { return "Base(" . $this->value . ")"; }
                public static function make($value) { return new static($value); }
            }
            class Child extends Base {
                public function describe() { return "Child[" . parent::describe() . "]"; }
            }
            $child = Child::make(7);
            echo $child->describe(), "|", $child instanceof Base ? "yes" : "no", "|";
            echo Child::KIND, "|", Child::$count, "|";
            $same = $child;
            $same->value = 9;
            echo $child->value, "|", $child === $same ? "same" : "different", "|";
            $clone = clone $child;
            $clone->value = 11;
            echo $child->value, "|", $clone->value;
        "#);
        assert_eq!(out, "Child[Base(7)]|yes|base|1|9|same|9|11");
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn namespaces_resolve_declarations_and_imports() {
        let (out, warnings) = run(r#"<?php
            namespace Demo\Util;
            function name() { return __NAMESPACE__; }
            class Box {
                public $value;
                function __construct($value) { $this->value = $value; }
                function get() { return $this->value; }
            }
            echo name(), "|", strlen("abc"), "|";
            namespace App;
            use Demo\Util\Box as Crate;
            use function Demo\Util\name as util_name;
            $box = new Crate(7);
            echo util_name(), "|", $box->get(), "|", Crate::class;
        "#);
        assert_eq!(out, "Demo\\Util|3|Demo\\Util|7|Demo\\Util\\Box");
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn exceptions_are_caught_and_finally_runs() {
        let (out, warnings) = run(r#"<?php
            class MyError extends RuntimeException {}
            function fail() { throw new MyError("boom", 7); }
            try {
                fail();
            } catch (LogicException $e) {
                echo "wrong";
            } catch (Exception $e) {
                echo get_class($e), "|", $e->getMessage(), "|", $e->getCode(), "|";
            } finally {
                echo "finally|";
            }
            try {
                try { throw new Exception("inner"); }
                catch (TypeError $e) { echo "wrong"; }
            } catch (Throwable $e) {
                echo $e->getMessage(), "|";
            }
            try { strlen([]); }
            catch (TypeError $e) { echo "type:", get_class($e); }
        "#);
        assert_eq!(out, "MyError|boom|7|finally|inner|type:TypeError");
        assert!(warnings.is_empty(), "{warnings:?}");
    }
}
