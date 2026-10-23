//! AST -> bytecode compiler.
//!
//! Zend compiles to a register machine (`zend_op_array` with `op1/op2/result`
//! slots); PHPCraft compiles to a simpler stack machine. The control-flow
//! lowering (short-circuit operators, ternary, `??`, loops, `foreach`,
//! `switch`) follows the same strategies as `zend_compile.c`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use phpcraft_ast::{
    ArrayItem, BinOp, CatchClause, ClassDecl, ClosureBody, ClosureDecl, Expr, FunctionDecl,
    InterpPart, MatchArm, Param, Program, Stmt, UnOp, UseKind,
};
use phpcraft_bytecode::{
    Chunk, ClassDef, ClassMethod, ClassProperty, CompiledProgram, DimMode, Function, Op, ParamSlot,
};
use phpcraft_runtime::{key_from_value, PhpArray, Value};

#[derive(Debug, Clone)]
pub struct CompileError {
    pub message: String,
    pub line: u32,
}

impl CompileError {
    fn new(message: impl Into<String>, line: u32) -> Self {
        Self {
            message: message.into(),
            line,
        }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line > 0 {
            write!(f, "{} on line {}", self.message, self.line)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for CompileError {}

static CLOSURE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_closure_name(line: u32) -> String {
    let id = CLOSURE_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
    format!("{{closure:{line}:{id}}}")
}

fn check_param_names(params: &[Param], line: u32) -> Result<(), CompileError> {
    let mut seen = HashSet::new();
    for param in params {
        if !seen.insert(param.name.as_str()) {
            return Err(CompileError::new(
                format!("Redefinition of parameter ${}", param.name),
                line,
            ));
        }
    }
    Ok(())
}

type RefSignature = (Vec<bool>, bool);

fn collect_ref_signatures(
    stmts: &[Stmt],
    namespace: &str,
    signatures: &mut HashMap<String, RefSignature>,
) {
    let qualify = |name: &str, namespace: &str| -> String {
        let name = name.trim_start_matches('\\');
        if namespace.is_empty() || name.contains('\\') {
            name.to_string()
        } else {
            format!("{namespace}\\{name}")
        }
    };
    let mut active_namespace = namespace.to_string();
    for stmt in stmts {
        match stmt {
            Stmt::NamespaceMark(name) => {
                active_namespace = name.clone().unwrap_or_default();
            }
            Stmt::NamespaceBlock { name, body } => {
                collect_ref_signatures(body, name.as_deref().unwrap_or_default(), signatures);
            }
            Stmt::Function(decl) => {
                let name = qualify(&decl.name, &active_namespace);
                signatures.insert(
                    name.to_ascii_lowercase(),
                    (
                        decl.params.iter().map(|param| param.by_ref).collect(),
                        decl.params
                            .last()
                            .map(|param| param.variadic)
                            .unwrap_or(false),
                    ),
                );
                collect_ref_signatures(&decl.body, &active_namespace, signatures);
            }
            Stmt::Class(decl) => {
                let class = qualify(&decl.name, &active_namespace);
                for method in &decl.methods {
                    signatures.insert(
                        format!("{}::{}", class, method.name).to_ascii_lowercase(),
                        (
                            method.params.iter().map(|param| param.by_ref).collect(),
                            method
                                .params
                                .last()
                                .map(|param| param.variadic)
                                .unwrap_or(false),
                        ),
                    );
                    collect_ref_signatures(&method.body, &active_namespace, signatures);
                }
            }
            _ => {}
        }
    }
}

fn argument_is_by_ref(signature: Option<&RefSignature>, index: usize) -> bool {
    let Some((params, variadic)) = signature else {
        return false;
    };
    match params.get(index) {
        Some(by_ref) => *by_ref,
        None => *variadic && params.last().copied().unwrap_or(false),
    }
}

/// Compile a parsed program. `file` is only used for diagnostics.
pub fn compile(program: &Program, file: &str) -> Result<CompiledProgram, CompileError> {
    let mut signatures = HashMap::new();
    collect_ref_signatures(&program.stmts, "", &mut signatures);
    let mut compiler = FnCompiler::new_main(file);
    compiler.signatures = signatures;
    compiler.compile_stmts(&program.stmts)?;
    let (main, functions, classes) = compiler.finish();

    let mut seen: HashMap<String, ()> = HashMap::new();
    for function in &functions {
        let key = function.name.to_ascii_lowercase();
        if seen.insert(key, ()).is_some() {
            return Err(CompileError::new(
                format!("Cannot redeclare function {}()", function.name),
                function.line,
            ));
        }
    }
    let mut seen_classes: HashMap<String, ()> = HashMap::new();
    for class in &classes {
        let key = class.name.to_ascii_lowercase();
        if seen_classes.insert(key, ()).is_some() {
            return Err(CompileError::new(
                format!("Cannot declare class {}", class.name),
                class.line,
            ));
        }
    }

    Ok(CompiledProgram {
        main,
        functions,
        classes,
        file: file.to_string(),
    })
}

#[derive(Default)]
struct LoopCtx {
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

/// A `finally` body that is active while compiling the statements it protects.
#[derive(Clone)]
struct ActiveFinally {
    body: Vec<Stmt>,
    /// Exception handlers active outside the `try` that owns this `finally`.
    /// A `return` pops handlers down to this depth before running the body so
    /// an exception thrown by `finally` is still seen by outer `try` blocks.
    outer_handler_depth: usize,
}

struct FnCompiler {
    name: String,
    display_name: Option<String>,
    line: u32,
    is_main: bool,
    locals: HashMap<String, u32>,
    globals: HashSet<String>,
    next_slot: u32,
    ops: Vec<Op>,
    consts: Vec<Value>,
    const_str_cache: HashMap<Vec<u8>, u32>,
    loops: Vec<LoopCtx>,
    params: Vec<ParamSlot>,
    capture_slots: Vec<u32>,
    capture_names: Vec<String>,
    capture_refs: Vec<bool>,
    returns_ref: bool,
    functions: Vec<Function>,
    classes: Vec<ClassDef>,
    namespace: String,
    class_uses: HashMap<String, String>,
    function_uses: HashMap<String, String>,
    const_uses: HashMap<String, String>,
    signatures: HashMap<String, RefSignature>,
    /// `finally` bodies enclosing the code currently being compiled. A
    /// `return` has to run these before leaving the function.
    active_finally: Vec<ActiveFinally>,
    /// Number of exception handlers active at the current compile point.
    handler_depth: usize,
    file: String,
}

impl FnCompiler {
    fn new_main(file: &str) -> Self {
        Self {
            name: "{main}".to_string(),
            display_name: None,
            line: 0,
            is_main: true,
            locals: HashMap::new(),
            globals: HashSet::new(),
            next_slot: 0,
            ops: Vec::new(),
            consts: Vec::new(),
            const_str_cache: HashMap::new(),
            loops: Vec::new(),
            params: Vec::new(),
            capture_slots: Vec::new(),
            capture_names: Vec::new(),
            capture_refs: Vec::new(),
            returns_ref: false,
            functions: Vec::new(),
            classes: Vec::new(),
            namespace: String::new(),
            class_uses: HashMap::new(),
            function_uses: HashMap::new(),
            const_uses: HashMap::new(),
            signatures: HashMap::new(),
            active_finally: Vec::new(),
            handler_depth: 0,
            file: file.to_string(),
        }
    }

    fn new_function(decl: &FunctionDecl, file: &str) -> Self {
        Self {
            name: decl.name.clone(),
            display_name: None,
            line: decl.line,
            is_main: false,
            locals: HashMap::new(),
            globals: HashSet::new(),
            next_slot: 0,
            ops: Vec::new(),
            consts: Vec::new(),
            const_str_cache: HashMap::new(),
            loops: Vec::new(),
            params: Vec::new(),
            capture_slots: Vec::new(),
            capture_names: Vec::new(),
            capture_refs: Vec::new(),
            returns_ref: decl.returns_ref,
            functions: Vec::new(),
            classes: Vec::new(),
            namespace: String::new(),
            class_uses: HashMap::new(),
            function_uses: HashMap::new(),
            const_uses: HashMap::new(),
            signatures: HashMap::new(),
            active_finally: Vec::new(),
            handler_depth: 0,
            file: file.to_string(),
        }
    }

    fn new_closure(name: &str, line: u32, file: &str) -> Self {
        Self {
            name: name.to_string(),
            display_name: Some(format!("{{closure:{file}:{line}}}")),
            line,
            is_main: false,
            locals: HashMap::new(),
            globals: HashSet::new(),
            next_slot: 0,
            ops: Vec::new(),
            consts: Vec::new(),
            const_str_cache: HashMap::new(),
            loops: Vec::new(),
            params: Vec::new(),
            capture_slots: Vec::new(),
            capture_names: Vec::new(),
            capture_refs: Vec::new(),
            returns_ref: false,
            functions: Vec::new(),
            classes: Vec::new(),
            namespace: String::new(),
            class_uses: HashMap::new(),
            function_uses: HashMap::new(),
            const_uses: HashMap::new(),
            signatures: HashMap::new(),
            active_finally: Vec::new(),
            handler_depth: 0,
            file: file.to_string(),
        }
    }

    fn new_method(name: &str, line: u32, file: &str) -> Self {
        let mut compiler = Self::new_closure(name, line, file);
        compiler.display_name = Some(name.to_string());
        // Slot 0 always holds `$this`; method parameters start at slot 1.
        compiler.locals.insert("this".to_string(), 0);
        compiler.next_slot = 1;
        compiler
    }

    fn finish(mut self) -> (Function, Vec<Function>, Vec<ClassDef>) {
        self.emit(Op::ReturnNull);
        let mut slot_names = vec![None; self.next_slot as usize];
        for (name, slot) in &self.locals {
            slot_names[*slot as usize] = Some(name.clone());
        }
        let function = Function {
            name: self.name,
            display_name: self.display_name,
            params: self.params,
            capture_slots: self.capture_slots,
            capture_names: self.capture_names,
            capture_refs: self.capture_refs,
            returns_ref: self.returns_ref,
            chunk: Chunk {
                ops: self.ops,
                consts: self.consts,
            },
            max_slots: self.next_slot,
            slot_names,
            line: self.line,
        };
        (function, self.functions, self.classes)
    }

    // ------------------------------------------------------------- utilities

    fn emit(&mut self, op: Op) -> usize {
        self.ops.push(op);
        self.ops.len() - 1
    }

    fn patch(&mut self, at: usize, target: usize) {
        match &mut self.ops[at] {
            Op::Jump(t)
            | Op::JumpIfFalse(t)
            | Op::JumpIfTrue(t)
            | Op::JumpIfFalseKeep(t)
            | Op::JumpIfTrueKeep(t)
            | Op::JumpIfNotNull(t) => *t = target,
            Op::TryStart { catch_ip, .. } => *catch_ip = target,
            Op::IterNext { end, .. } => *end = target,
            other => panic!("cannot patch op {other:?}"),
        }
    }

    fn const_str(&mut self, bytes: &[u8]) -> u32 {
        if let Some(index) = self.const_str_cache.get(bytes) {
            return *index;
        }
        let index = self.consts.len() as u32;
        self.consts.push(Value::str(bytes.to_vec()));
        self.const_str_cache.insert(bytes.to_vec(), index);
        index
    }

    fn qualify_with_uses(&self, name: &str, uses: &HashMap<String, String>) -> String {
        let raw = name.trim_start_matches('\\');
        if name.starts_with('\\') {
            return raw.to_string();
        }
        if raw.eq_ignore_ascii_case("self")
            || raw.eq_ignore_ascii_case("parent")
            || raw.eq_ignore_ascii_case("static")
        {
            return raw.to_string();
        }
        if raw.eq_ignore_ascii_case("namespace") {
            return self.namespace.clone();
        }
        if raw.len() > "namespace\\".len()
            && raw[.."namespace\\".len()].eq_ignore_ascii_case("namespace\\")
        {
            let rest = &raw["namespace\\".len()..];
            return if self.namespace.is_empty() {
                rest.to_string()
            } else {
                format!("{}\\{}", self.namespace, rest)
            };
        }
        let (first, rest) = match raw.split_once('\\') {
            Some((first, rest)) => (first, Some(rest)),
            None => (raw, None),
        };
        if let Some(target) = uses.get(&first.to_ascii_lowercase()) {
            return match rest {
                Some(rest) => format!("{target}\\{rest}"),
                None => target.clone(),
            };
        }
        if self.namespace.is_empty() {
            raw.to_string()
        } else {
            format!("{}\\{}", self.namespace, raw)
        }
    }

    fn qualify_class(&self, name: &str) -> String {
        self.qualify_with_uses(name, &self.class_uses)
    }

    fn resolve_function_call(&self, name: &str) -> (String, bool) {
        let raw = name.trim_start_matches('\\');
        if name.starts_with('\\') {
            return (raw.to_string(), false);
        }
        if raw.eq_ignore_ascii_case("namespace") {
            return (self.namespace.clone(), false);
        }
        if raw.len() > "namespace\\".len()
            && raw[.."namespace\\".len()].eq_ignore_ascii_case("namespace\\")
        {
            let rest = &raw["namespace\\".len()..];
            return (
                if self.namespace.is_empty() {
                    rest.to_string()
                } else {
                    format!("{}\\{}", self.namespace, rest)
                },
                false,
            );
        }
        let (first, rest) = match raw.split_once('\\') {
            Some((first, rest)) => (first, Some(rest)),
            None => (raw, None),
        };
        if let Some(target) = self.function_uses.get(&first.to_ascii_lowercase()) {
            return (
                match rest {
                    Some(rest) => format!("{target}\\{rest}"),
                    None => target.clone(),
                },
                false,
            );
        }
        if rest.is_some() {
            return (
                if self.namespace.is_empty() {
                    raw.to_string()
                } else {
                    format!("{}\\{}", self.namespace, raw)
                },
                false,
            );
        }
        if self.namespace.is_empty() {
            (raw.to_string(), false)
        } else {
            (format!("{}\\{}", self.namespace, raw), true)
        }
    }

    fn qualify_constant(&self, name: &str) -> String {
        self.qualify_with_uses(name, &self.const_uses)
    }

    fn qualify_decl_name(&self, name: &str, _function: bool) -> String {
        let raw = name.trim_start_matches('\\');
        if name.starts_with('\\') || self.namespace.is_empty() {
            raw.to_string()
        } else {
            format!("{}\\{}", self.namespace, raw)
        }
    }

    fn magic_file(&self) -> String {
        if matches!(
            self.file.as_str(),
            "Command line code" | "Standard input code"
        ) {
            return self.file.clone();
        }
        std::fs::canonicalize(&self.file)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| self.file.clone())
    }

    fn magic_dir(&self) -> String {
        if matches!(
            self.file.as_str(),
            "Command line code" | "Standard input code"
        ) {
            return std::env::current_dir()
                .map(|path| path.display().to_string())
                .unwrap_or_default();
        }
        let file = self.magic_file();
        Path::new(&file)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.display().to_string())
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|path| path.display().to_string())
            })
            .unwrap_or_default()
    }

    fn slot(&mut self, name: &str) -> u32 {
        if let Some(slot) = self.locals.get(name) {
            return *slot;
        }
        let slot = self.next_slot;
        self.next_slot += 1;
        self.locals.insert(name.to_string(), slot);
        slot
    }

    fn temp(&mut self) -> u32 {
        let slot = self.next_slot;
        self.next_slot += 1;
        slot
    }

    fn load_var(&mut self, name: &str) {
        if self.is_main || self.globals.contains(name) {
            let index = self.const_str(name.as_bytes());
            self.emit(Op::LoadGlobal(index));
        } else {
            let slot = self.slot(name);
            self.emit(Op::LoadVar(slot));
        }
    }

    fn load_var_ref(&mut self, name: &str) {
        if self.is_main || self.globals.contains(name) {
            let index = self.const_str(name.as_bytes());
            self.emit(Op::LoadGlobalRef(index));
        } else {
            let slot = self.slot(name);
            self.emit(Op::LoadVarRef(slot));
        }
    }

    fn store_var_ref(&mut self, name: &str) {
        if self.is_main || self.globals.contains(name) {
            let index = self.const_str(name.as_bytes());
            self.emit(Op::StoreGlobalRef(index));
        } else {
            let slot = self.slot(name);
            self.emit(Op::StoreVarRef(slot));
        }
    }

    /// Push a reference cell for a supported reference source.
    fn load_ref_source(&mut self, source: &Expr) -> Result<(), CompileError> {
        match source {
            Expr::Var(name) => {
                if name == "this" {
                    return Err(CompileError::new("Cannot re-assign $this", self.line));
                }
                self.load_var_ref(name);
            }
            Expr::Index { .. } => {
                let (root, dims) = Self::lvalue_chain(source).ok_or_else(|| {
                    CompileError::new(
                        "references to array elements currently require a variable root",
                        self.line,
                    )
                })?;
                self.load_var(&root);
                let mut modes = Vec::with_capacity(dims.len());
                for dim in dims {
                    match dim {
                        Some(index) => {
                            self.expr(index)?;
                            modes.push(DimMode::Key);
                        }
                        None => {
                            return Err(CompileError::new(
                                "cannot take a reference to an appended array element",
                                self.line,
                            ))
                        }
                    }
                }
                self.emit(Op::MakeDimRef { modes });
                self.store_var(&root, false);
            }
            Expr::Property { object, name } => {
                self.expr(object)?;
                let name = self.const_str(name.as_bytes());
                self.emit(Op::MakePropertyRef { name });
            }
            Expr::StaticProperty { class, name } => {
                let class = self.qualify_class(class);
                let class = self.const_str(class.as_bytes());
                let name = self.const_str(name.as_bytes());
                self.emit(Op::MakeStaticPropertyRef { class, name });
            }
            Expr::Call { .. }
            | Expr::DynamicCall { .. }
            | Expr::MethodCall { .. }
            | Expr::StaticCall { .. } => self.expr(source)?,
            _ => {
                return Err(CompileError::new(
                    "references can only be taken of variables, array elements, properties or reference-returning calls",
                    self.line,
                ))
            }
        }
        Ok(())
    }

    /// Store the reference on top of the stack into a reference target.
    fn store_ref_target(&mut self, target: &Expr) -> Result<(), CompileError> {
        match target {
            Expr::Var(name) => {
                if name == "this" {
                    return Err(CompileError::new("Cannot re-assign $this", self.line));
                }
                self.store_var_ref(name);
            }
            Expr::Index { .. } => {
                let (root, dims) = Self::lvalue_chain(target).ok_or_else(|| {
                    CompileError::new(
                        "reference assignment targets currently require a variable root",
                        self.line,
                    )
                })?;
                self.load_var(&root);
                let mut modes = Vec::with_capacity(dims.len());
                for dim in dims {
                    match dim {
                        Some(index) => {
                            self.expr(index)?;
                            modes.push(DimMode::Key);
                        }
                        None => modes.push(DimMode::Append),
                    }
                }
                self.emit(Op::SetDimRef { modes });
                self.store_var(&root, false);
            }
            Expr::Property { object, name } => {
                self.expr(object)?;
                let name = self.const_str(name.as_bytes());
                self.emit(Op::SetPropertyRef { name });
            }
            Expr::StaticProperty { class, name } => {
                let class = self.qualify_class(class);
                let class = self.const_str(class.as_bytes());
                let name = self.const_str(name.as_bytes());
                self.emit(Op::SetStaticPropertyRef { class, name });
            }
            _ => {
                return Err(CompileError::new(
                    "reference assignment target must be a variable, array element or property",
                    self.line,
                ))
            }
        }
        Ok(())
    }

    fn expr_arg(&mut self, expr: &Expr) -> Result<(), CompileError> {
        if let Expr::Var(name) = expr {
            if name != "this" {
                self.load_var_ref(name);
                return Ok(());
            }
        }
        self.expr(expr)
    }

    /// Evaluate an argument that the callee declares by reference. Array
    /// elements and properties must become shared cells before the call.
    fn expr_arg_ref(&mut self, expr: &Expr) -> Result<(), CompileError> {
        match expr {
            Expr::Var(_)
            | Expr::Index { .. }
            | Expr::Property { .. }
            | Expr::StaticProperty { .. } => self.load_ref_source(expr),
            _ => self.expr(expr),
        }
    }

    /// Method calls do not carry a compile-time receiver type. Promote an
    /// element/property argument when every known method with this name takes
    /// that position by reference.
    fn method_argument_by_ref(&self, method: &str, index: usize) -> bool {
        let suffix = format!("::{}", method.to_ascii_lowercase());
        let mut found = false;
        for (name, signature) in &self.signatures {
            if name.ends_with(&suffix) {
                found = true;
                if !argument_is_by_ref(Some(signature), index) {
                    return false;
                }
            }
        }
        found
    }

    fn load_var_safe(&mut self, name: &str) {
        if self.is_main || self.globals.contains(name) {
            let index = self.const_str(name.as_bytes());
            self.emit(Op::LoadGlobalSafe(index));
        } else {
            let slot = self.slot(name);
            self.emit(Op::LoadVarSafe(slot));
        }
    }

    fn store_var(&mut self, name: &str, keep: bool) {
        if self.is_main || self.globals.contains(name) {
            let index = self.const_str(name.as_bytes());
            self.emit(if keep {
                Op::StoreGlobalKeep(index)
            } else {
                Op::StoreGlobal(index)
            });
        } else {
            let slot = self.slot(name);
            self.emit(if keep {
                Op::StoreVarKeep(slot)
            } else {
                Op::StoreVar(slot)
            });
        }
    }

    fn lvalue_chain(expr: &Expr) -> Option<(String, Vec<Option<&Expr>>)> {
        match expr {
            Expr::Var(name) => Some((name.clone(), Vec::new())),
            Expr::Index { base, index } => {
                let (root, mut dims) = Self::lvalue_chain(base)?;
                dims.push(index.as_deref());
                Some((root, dims))
            }
            _ => None,
        }
    }

    /// Store the value on top of the stack into `target`.
    fn store_target(&mut self, target: &Expr, keep: bool) -> Result<(), CompileError> {
        match target {
            Expr::Var(name) => {
                self.store_var(name, keep);
                Ok(())
            }
            Expr::Property { object, name } => {
                let value_tmp = self.temp();
                self.emit(Op::StoreTmp(value_tmp));
                self.expr(object)?;
                self.emit(Op::LoadTmp(value_tmp));
                let index = self.const_str(name.as_bytes());
                self.emit(Op::AssignProperty { name: index, keep });
                Ok(())
            }
            Expr::StaticProperty { class, name } => {
                let value_tmp = self.temp();
                self.emit(Op::StoreTmp(value_tmp));
                let class = self.qualify_class(class);
                let class = self.const_str(class.as_bytes());
                let name = self.const_str(name.as_bytes());
                self.emit(Op::LoadTmp(value_tmp));
                self.emit(Op::AssignStaticProperty { class, name, keep });
                Ok(())
            }
            Expr::Index { .. } => {
                let (root, dims) = Self::lvalue_chain(target).ok_or_else(|| {
                    CompileError::new(
                        "nested property/dimension assignment is not supported yet",
                        self.line,
                    )
                })?;
                let value_tmp = self.temp();
                self.emit(Op::StoreTmp(value_tmp));
                self.load_var(&root);
                let mut modes = Vec::with_capacity(dims.len());
                for dim in dims {
                    match dim {
                        Some(index) => {
                            self.expr(index)?;
                            modes.push(DimMode::Key);
                        }
                        None => modes.push(DimMode::Append),
                    }
                }
                self.emit(Op::LoadTmp(value_tmp));
                self.emit(Op::AssignDim { modes });
                self.store_var(&root, false);
                if keep {
                    self.emit(Op::LoadTmp(value_tmp));
                }
                Ok(())
            }
            _ => Err(CompileError::new(
                "left-hand side of assignment is not writable",
                self.line,
            )),
        }
    }

    /// Read like `expr` but without warnings for undefined variables/keys,
    /// as needed by `isset`, `empty` and `??`.
    fn silent_read(&mut self, expr: &Expr) -> Result<(), CompileError> {
        match expr {
            Expr::Var(name) => {
                self.load_var_safe(name);
                Ok(())
            }
            Expr::Index { base, index } => {
                self.silent_read(base)?;
                match index {
                    Some(index) => {
                        self.expr(index)?;
                        self.emit(Op::IndexSafe);
                        Ok(())
                    }
                    None => Err(CompileError::new("Cannot use [] for reading", self.line)),
                }
            }
            Expr::Property { object, name } => {
                self.expr(object)?;
                let index = self.const_str(name.as_bytes());
                self.emit(Op::LoadProperty {
                    name: index,
                    safe: true,
                });
                Ok(())
            }
            Expr::StaticProperty { class, name } => {
                let class = self.qualify_class(class);
                let class = self.const_str(class.as_bytes());
                let name = self.const_str(name.as_bytes());
                self.emit(Op::LoadStaticProperty {
                    class,
                    name,
                    safe: true,
                });
                Ok(())
            }
            other => self.expr(other),
        }
    }

    // ------------------------------------------------------------ statements

    fn compile_stmts(&mut self, stmts: &[Stmt]) -> Result<(), CompileError> {
        for stmt in stmts {
            self.compile_stmt(stmt)?;
        }
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<(), CompileError> {
        match stmt {
            Stmt::Nop => Ok(()),
            Stmt::InlineHtml(bytes) => {
                let index = self.const_str(bytes);
                self.emit(Op::StrConst(index));
                self.emit(Op::Echo);
                Ok(())
            }
            Stmt::Echo(exprs) => {
                for expr in exprs {
                    self.expr(expr)?;
                    self.emit(Op::Echo);
                }
                Ok(())
            }
            Stmt::Expr(expr) => {
                self.expr(expr)?;
                self.emit(Op::Pop);
                Ok(())
            }
            Stmt::Block(stmts) => self.compile_stmts(stmts),
            Stmt::If { branches, else_ } => self.compile_if(branches, else_.as_deref()),
            Stmt::While { cond, body } => self.compile_while(cond, body),
            Stmt::DoWhile { body, cond } => self.compile_do_while(body, cond),
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => self.compile_for(init, cond, step, body),
            Stmt::Foreach {
                subject,
                key,
                value,
                by_ref,
                body,
            } => self.compile_foreach(subject, key.as_ref(), value, *by_ref, body),
            Stmt::Switch { subject, cases } => self.compile_switch(subject, cases),
            Stmt::Function(decl) => self.compile_function_decl(decl),
            Stmt::Class(decl) => self.compile_class(decl),
            Stmt::NamespaceMark(name) => {
                self.namespace = name.clone().unwrap_or_default();
                self.class_uses.clear();
                self.function_uses.clear();
                self.const_uses.clear();
                Ok(())
            }
            Stmt::NamespaceBlock { name, body } => {
                let saved_namespace =
                    std::mem::replace(&mut self.namespace, name.clone().unwrap_or_default());
                let saved_class_uses = std::mem::take(&mut self.class_uses);
                let saved_function_uses = std::mem::take(&mut self.function_uses);
                let saved_const_uses = std::mem::take(&mut self.const_uses);
                let result = self.compile_stmts(body);
                self.namespace = saved_namespace;
                self.class_uses = saved_class_uses;
                self.function_uses = saved_function_uses;
                self.const_uses = saved_const_uses;
                result
            }
            Stmt::Use(imports) => {
                for import in imports {
                    let map = match import.kind {
                        UseKind::Class => &mut self.class_uses,
                        UseKind::Function => &mut self.function_uses,
                        UseKind::Constant => &mut self.const_uses,
                    };
                    map.insert(import.alias.to_ascii_lowercase(), import.target.clone());
                }
                Ok(())
            }
            Stmt::Try {
                body,
                catches,
                finally,
            } => self.compile_try(body, catches, finally.as_deref()),
            Stmt::Global(names) => {
                if !self.is_main {
                    for name in names {
                        self.globals.insert(name.clone());
                    }
                }
                Ok(())
            }
            Stmt::Unset(targets) => self.compile_unset(targets),
            Stmt::Return(value) => {
                match value {
                    Some(expr) if self.returns_ref => {
                        self.load_ref_source(expr)?;
                        self.compile_finally_before_return()?;
                        self.emit(Op::Return);
                    }
                    Some(expr) => {
                        self.expr(expr)?;
                        self.compile_finally_before_return()?;
                        self.emit(Op::Return);
                    }
                    None => {
                        self.compile_finally_before_return()?;
                        self.emit(Op::ReturnNull);
                    }
                }
                Ok(())
            }
            Stmt::Break(level) => self.compile_break(*level, false),
            Stmt::Continue(level) => self.compile_break(*level, true),
        }
    }

    fn compile_if(
        &mut self,
        branches: &[(Expr, Vec<Stmt>)],
        else_: Option<&[Stmt]>,
    ) -> Result<(), CompileError> {
        let mut end_jumps = Vec::new();
        for (cond, body) in branches {
            self.expr(cond)?;
            let next = self.emit(Op::JumpIfFalse(0));
            self.compile_stmts(body)?;
            end_jumps.push(self.emit(Op::Jump(0)));
            let label = self.ops.len();
            self.patch(next, label);
        }
        if let Some(else_) = else_ {
            self.compile_stmts(else_)?;
        }
        let end = self.ops.len();
        for jump in end_jumps {
            self.patch(jump, end);
        }
        Ok(())
    }

    fn compile_while(&mut self, cond: &Expr, body: &[Stmt]) -> Result<(), CompileError> {
        let start = self.ops.len();
        self.expr(cond)?;
        let exit = self.emit(Op::JumpIfFalse(0));
        self.loops.push(LoopCtx::default());
        self.compile_stmts(body)?;
        let ctx = self.loops.pop().expect("loop context");
        for jump in ctx.continues {
            self.patch(jump, start);
        }
        self.emit(Op::Jump(start));
        let end = self.ops.len();
        self.patch(exit, end);
        for jump in ctx.breaks {
            self.patch(jump, end);
        }
        Ok(())
    }

    fn compile_do_while(&mut self, body: &[Stmt], cond: &Expr) -> Result<(), CompileError> {
        let start = self.ops.len();
        self.loops.push(LoopCtx::default());
        self.compile_stmts(body)?;
        let cond_label = self.ops.len();
        let ctx = self.loops.pop().expect("loop context");
        for jump in ctx.continues {
            self.patch(jump, cond_label);
        }
        self.expr(cond)?;
        self.emit(Op::JumpIfTrue(start));
        let end = self.ops.len();
        for jump in ctx.breaks {
            self.patch(jump, end);
        }
        Ok(())
    }

    fn compile_for(
        &mut self,
        init: &[Expr],
        cond: &[Expr],
        step: &[Expr],
        body: &[Stmt],
    ) -> Result<(), CompileError> {
        for expr in init {
            self.expr(expr)?;
            self.emit(Op::Pop);
        }
        let cond_label = self.ops.len();
        if cond.is_empty() {
            self.emit(Op::True);
        } else {
            for (i, expr) in cond.iter().enumerate() {
                self.expr(expr)?;
                if i + 1 < cond.len() {
                    self.emit(Op::Pop);
                }
            }
        }
        let exit = self.emit(Op::JumpIfFalse(0));

        self.loops.push(LoopCtx::default());
        self.compile_stmts(body)?;

        let step_label = self.ops.len();
        let ctx = self.loops.pop().expect("loop context");
        for jump in ctx.continues {
            self.patch(jump, step_label);
        }
        for expr in step {
            self.expr(expr)?;
            self.emit(Op::Pop);
        }
        self.emit(Op::Jump(cond_label));
        let end = self.ops.len();
        self.patch(exit, end);
        for jump in ctx.breaks {
            self.patch(jump, end);
        }
        Ok(())
    }

    fn compile_foreach(
        &mut self,
        subject: &Expr,
        key: Option<&Expr>,
        value: &Expr,
        by_ref: bool,
        body: &[Stmt],
    ) -> Result<(), CompileError> {
        self.expr(subject)?;
        if by_ref {
            self.emit(Op::PrepareRefs);
            self.emit(Op::Dup);
            self.store_target(subject, false)?;
        }
        let iter = self.temp();
        self.emit(Op::IterInit { iter });

        let value_tmp = self.temp();
        let key_tmp = key.map(|_| self.temp());
        let loop_start = self.ops.len();
        let next = self.emit(Op::IterNext {
            iter,
            key: key_tmp,
            value: value_tmp,
            by_ref,
            end: 0,
        });

        self.loops.push(LoopCtx::default());
        if let (Some(key_target), Some(key_slot)) = (key, key_tmp) {
            self.emit(Op::LoadTmp(key_slot));
            self.store_target(key_target, false)?;
        }
        if by_ref {
            let Expr::Var(name) = value else {
                return Err(CompileError::new(
                    "by-reference foreach value must be a variable",
                    self.line,
                ));
            };
            self.emit(Op::LoadTmp(value_tmp));
            self.store_var_ref(name);
        } else {
            self.emit(Op::LoadTmp(value_tmp));
            self.store_target(value, false)?;
        }
        self.compile_stmts(body)?;

        let ctx = self.loops.pop().expect("loop context");
        for jump in ctx.continues {
            self.patch(jump, loop_start);
        }
        self.emit(Op::Jump(loop_start));
        let end = self.ops.len();
        self.patch(next, end);
        for jump in ctx.breaks {
            self.patch(jump, end);
        }
        Ok(())
    }

    fn compile_unset(&mut self, targets: &[Expr]) -> Result<(), CompileError> {
        for target in targets {
            match target {
                Expr::Var(name) => {
                    if name == "this" {
                        return Err(CompileError::new("Cannot unset $this", self.line));
                    }
                    if self.is_main || self.globals.contains(name) {
                        let index = self.const_str(name.as_bytes());
                        self.emit(Op::UnsetGlobal(index));
                    } else {
                        let slot = self.slot(name);
                        self.emit(Op::UnsetVar(slot));
                    }
                }
                Expr::Index { .. } => {
                    let (root, dims) = Self::lvalue_chain(target).ok_or_else(|| {
                        CompileError::new(
                            "unset() targets currently require a variable root",
                            self.line,
                        )
                    })?;
                    self.load_var(&root);
                    let mut modes = Vec::with_capacity(dims.len());
                    for dim in dims {
                        match dim {
                            Some(index) => {
                                self.expr(index)?;
                                modes.push(DimMode::Key);
                            }
                            None => modes.push(DimMode::Append),
                        }
                    }
                    self.emit(Op::UnsetDim { modes });
                    self.store_var(&root, false);
                }
                Expr::Property { object, name } => {
                    self.expr(object)?;
                    let name = self.const_str(name.as_bytes());
                    self.emit(Op::UnsetProperty { name });
                }
                Expr::StaticProperty { class, name } => {
                    let class = self.qualify_class(class);
                    let class = self.const_str(class.as_bytes());
                    let name = self.const_str(name.as_bytes());
                    self.emit(Op::UnsetStaticProperty { class, name });
                }
                _ => {
                    return Err(CompileError::new(
                        "unset() expects a variable, array element or property",
                        self.line,
                    ))
                }
            }
        }
        Ok(())
    }

    fn compile_switch(
        &mut self,
        subject: &Expr,
        cases: &[phpcraft_ast::SwitchCase],
    ) -> Result<(), CompileError> {
        self.expr(subject)?;
        let tmp = self.temp();
        self.emit(Op::StoreTmp(tmp));

        let mut dispatch: Vec<(usize, usize)> = Vec::new();
        let mut default_index = None;
        for (i, case) in cases.iter().enumerate() {
            match &case.test {
                Some(test) => {
                    self.emit(Op::LoadTmp(tmp));
                    self.expr(test)?;
                    self.emit(Op::Binary(BinOp::Eq));
                    dispatch.push((self.emit(Op::JumpIfTrue(0)), i));
                }
                None => default_index = Some(i),
            }
        }
        let no_match = self.emit(Op::Jump(0));

        self.loops.push(LoopCtx::default());
        let mut starts = vec![0usize; cases.len()];
        for (i, case) in cases.iter().enumerate() {
            starts[i] = self.ops.len();
            self.compile_stmts(&case.body)?;
        }
        let ctx = self.loops.pop().expect("switch context");

        let end = self.ops.len();
        for (jump, index) in dispatch {
            self.patch(jump, starts[index]);
        }
        self.patch(no_match, default_index.map(|i| starts[i]).unwrap_or(end));
        for jump in ctx.breaks.into_iter().chain(ctx.continues) {
            self.patch(jump, end);
        }
        Ok(())
    }

    fn compile_class(&mut self, decl: &ClassDecl) -> Result<(), CompileError> {
        if !self.is_main {
            return Err(CompileError::new(
                "class declarations inside functions are not supported",
                decl.line,
            ));
        }
        let class_name = self.qualify_decl_name(&decl.name, false);
        let parent = decl
            .parent
            .as_ref()
            .map(|parent| self.qualify_class(parent));
        let mut properties = Vec::new();
        for property in &decl.properties {
            properties.push(ClassProperty {
                name: property.name.clone(),
                default: match &property.default {
                    Some(expr) => const_expr(expr, property.line)?,
                    None => Value::Null,
                },
                visibility: property.visibility,
                static_: property.static_,
                line: property.line,
            });
        }
        let mut constants = Vec::new();
        for constant in &decl.constants {
            constants.push((
                constant.name.clone(),
                const_expr(&constant.value, constant.line)?,
            ));
        }

        let mut methods = Vec::new();
        for method in &decl.methods {
            let function_name = format!("{}::{}", class_name, method.name);
            let mut nested = FnCompiler::new_method(&function_name, method.line, &self.file);
            nested.signatures = self.signatures.clone();
            nested.returns_ref = method.returns_ref;
            nested.namespace = self.namespace.clone();
            nested.class_uses = self.class_uses.clone();
            nested.function_uses = self.function_uses.clone();
            nested.const_uses = self.const_uses.clone();
            for param in &method.params {
                let slot = nested.slot(&param.name);
                let default = match &param.default {
                    Some(expr) => Some(const_expr(expr, method.line)?),
                    None => None,
                };
                nested.params.push(ParamSlot {
                    name: param.name.clone(),
                    slot,
                    default,
                    variadic: param.variadic,
                    by_ref: param.by_ref,
                });
            }
            check_param_names(&method.params, method.line)?;
            nested.compile_stmts(&method.body)?;
            let (function, inner, _) = nested.finish();
            self.functions.push(function);
            self.functions.extend(inner);
            methods.push(ClassMethod {
                name: method.name.clone(),
                function: function_name,
                visibility: method.visibility,
                static_: method.static_,
                line: method.line,
            });
        }

        self.classes.push(ClassDef {
            name: class_name,
            parent,
            properties,
            constants,
            methods,
            line: decl.line,
        });
        Ok(())
    }

    fn compile_try(
        &mut self,
        body: &[Stmt],
        catches: &[CatchClause],
        finally: Option<&[Stmt]>,
    ) -> Result<(), CompileError> {
        let catch_slot = self.temp();
        let start = self.emit(Op::TryStart {
            catch_ip: 0,
            catch_slot,
        });
        let outer_handler_depth = self.handler_depth;
        self.handler_depth += 1;
        if let Some(finally) = finally {
            self.active_finally.push(ActiveFinally {
                body: finally.to_vec(),
                outer_handler_depth,
            });
        }
        self.compile_stmts(body)?;
        if finally.is_some() {
            self.active_finally.pop();
        }
        self.emit(Op::TryEnd);
        self.handler_depth -= 1;
        let try_done = self.emit(Op::Jump(0));
        let catch_label = self.ops.len();
        if let Op::TryStart { catch_ip, .. } = &mut self.ops[start] {
            *catch_ip = catch_label;
        }

        let mut caught_jumps = Vec::new();
        // `finally` has to run when a catch body throws, so every matching
        // handler gets an exception handler of its own that targets the
        // finally block.
        let mut catch_handlers = Vec::new();
        if let Some(finally) = finally {
            self.active_finally.push(ActiveFinally {
                body: finally.to_vec(),
                outer_handler_depth,
            });
        }
        for catch in catches {
            let mut hit_jumps = Vec::new();
            for class in &catch.classes {
                self.emit(Op::LoadTmp(catch_slot));
                let class = self.qualify_class(class);
                let index = self.const_str(class.as_bytes());
                self.emit(Op::InstanceOf { class: index });
                hit_jumps.push(self.emit(Op::JumpIfTrue(0)));
            }
            let next_catch = self.emit(Op::Jump(0));
            let matched = self.ops.len();
            for hit in hit_jumps {
                self.patch(hit, matched);
            }
            self.emit(Op::LoadTmp(catch_slot));
            self.store_target(&Expr::Var(catch.variable.clone()), false)?;
            // Clear the pending exception before running a matching handler.
            self.emit(Op::Null);
            self.emit(Op::StoreTmp(catch_slot));
            if finally.is_some() {
                catch_handlers.push(self.emit(Op::TryStart {
                    catch_ip: 0,
                    catch_slot,
                }));
                self.handler_depth += 1;
            }
            self.compile_stmts(&catch.body)?;
            if finally.is_some() {
                self.emit(Op::TryEnd);
                self.handler_depth -= 1;
            }
            caught_jumps.push(self.emit(Op::Jump(0)));
            let next_label = self.ops.len();
            self.patch(next_catch, next_label);
        }
        if finally.is_some() {
            self.active_finally.pop();
        }
        let mut rethrow_jump = None;
        if !catches.is_empty() {
            if finally.is_some() {
                // The unmatched exception is still in `catch_slot`; let the
                // finally block run before rethrowing it.
                rethrow_jump = Some(self.emit(Op::Jump(0)));
            } else {
                self.emit(Op::Rethrow { slot: catch_slot });
            }
        }

        let finally_label = self.ops.len();
        if let Some(finally) = finally {
            self.compile_stmts(finally)?;
            self.emit(Op::RethrowIfNotNull { slot: catch_slot });
        }
        let end = self.ops.len();
        let after = if finally.is_some() {
            finally_label
        } else {
            end
        };
        self.patch(try_done, after);
        for jump in caught_jumps {
            self.patch(jump, after);
        }
        if let Some(jump) = rethrow_jump {
            self.patch(jump, finally_label);
        }
        for handler in catch_handlers {
            if let Op::TryStart { catch_ip, .. } = &mut self.ops[handler] {
                *catch_ip = finally_label;
            }
        }
        Ok(())
    }

    /// Emit the `finally` bodies surrounding a `return`. PHP evaluates the
    /// return value first, then runs every enclosing finally block (innermost
    /// first) before actually leaving the function.
    fn compile_finally_before_return(&mut self) -> Result<(), CompileError> {
        let blocks = self.active_finally.clone();
        if blocks.is_empty() {
            return Ok(());
        }
        let handler_depth = self.handler_depth;
        for index in (0..blocks.len()).rev() {
            let target = blocks[index].outer_handler_depth;
            while self.handler_depth > target {
                self.emit(Op::TryEnd);
                self.handler_depth -= 1;
            }
            let body = blocks[index].body.clone();
            let outer = blocks[..index].to_vec();
            let saved = std::mem::replace(&mut self.active_finally, outer);
            self.compile_stmts(&body)?;
            self.active_finally = saved;
        }
        self.handler_depth = handler_depth;
        Ok(())
    }

    fn compile_function_decl(&mut self, decl: &FunctionDecl) -> Result<(), CompileError> {
        check_param_names(&decl.params, decl.line)?;
        let mut nested = FnCompiler::new_function(decl, &self.file);
        nested.name = self.qualify_decl_name(&decl.name, true);
        nested.namespace = self.namespace.clone();
        nested.class_uses = self.class_uses.clone();
        nested.function_uses = self.function_uses.clone();
        nested.const_uses = self.const_uses.clone();
        for param in &decl.params {
            let slot = nested.slot(&param.name);
            let default = match &param.default {
                Some(expr) => Some(const_expr(expr, decl.line)?),
                None => None,
            };
            nested.params.push(ParamSlot {
                name: param.name.clone(),
                slot,
                default,
                variadic: param.variadic,
                by_ref: param.by_ref,
            });
        }
        nested.compile_stmts(&decl.body)?;
        let (function, inner, _) = nested.finish();
        self.functions.push(function);
        self.functions.extend(inner);
        Ok(())
    }

    /// Lower the `(...)` first-class callable forms to a Closure value. The
    /// inner expression is the matching call node with no arguments.
    fn compile_first_class_callable(&mut self, inner: &Expr) -> Result<(), CompileError> {
        match inner {
            Expr::Call { name, args } if args.is_empty() => {
                let (name, fallback) = self.resolve_function_call(name);
                let index = self.const_str(name.as_bytes());
                self.emit(Op::MakeFunctionCallable {
                    name: index,
                    fallback,
                });
            }
            Expr::MethodCall {
                object,
                method,
                args,
            } if args.is_empty() => {
                self.expr(object)?;
                let index = self.const_str(method.as_bytes());
                self.emit(Op::MakeMethodCallable { method: index });
            }
            Expr::StaticCall {
                class,
                method,
                args,
            } if args.is_empty() => {
                let class_name = self.qualify_class(class);
                let class = self.const_str(class_name.as_bytes());
                let method = self.const_str(method.as_bytes());
                self.emit(Op::MakeStaticCallable { class, method });
            }
            Expr::DynamicCall { callee, args } if args.is_empty() => {
                self.expr(callee)?;
                self.emit(Op::CallableFromValue);
            }
            _ => {
                return Err(CompileError::new(
                    "Cannot create Closure for this expression",
                    self.line,
                ))
            }
        }
        Ok(())
    }

    fn compile_closure(&mut self, decl: &ClosureDecl) -> Result<(), CompileError> {
        let captures: Vec<(String, bool)> = if decl.is_arrow() {
            let params: HashSet<String> = decl.params.iter().map(|p| p.name.clone()).collect();
            let mut names = Vec::new();
            let mut seen = HashSet::new();
            match &decl.body {
                ClosureBody::Expr(body) => {
                    collect_free_vars(body, &params, &mut names, &mut seen);
                }
                ClosureBody::Block(_) => {}
            }
            names.into_iter().map(|name| (name, false)).collect()
        } else {
            let captures: Vec<(String, bool)> = decl
                .captures
                .iter()
                .map(|capture| (capture.name.clone(), capture.by_ref))
                .collect();
            let mut seen: HashSet<&str> = HashSet::new();
            if let Some((capture, _)) = captures
                .iter()
                .find(|(name, _)| !seen.insert(name.as_str()))
            {
                return Err(CompileError::new(
                    format!("Cannot use variable ${capture} twice"),
                    decl.line,
                ));
            }
            if let Some((capture, _)) = captures
                .iter()
                .find(|(name, _)| decl.params.iter().any(|param| param.name == *name))
            {
                return Err(CompileError::new(
                    format!("Cannot use lexical variable ${capture} as a parameter name"),
                    decl.line,
                ));
            }
            captures
        };
        check_param_names(&decl.params, decl.line)?;
        if captures.iter().any(|(name, _)| name == "this") {
            return Err(CompileError::new(
                "Cannot use $this as lexical variable",
                decl.line,
            ));
        }

        let name = next_closure_name(decl.line);
        let mut nested = FnCompiler::new_closure(&name, decl.line, &self.file);
        nested.signatures = self.signatures.clone();
        nested.returns_ref = decl.returns_ref;
        nested.namespace = self.namespace.clone();
        nested.class_uses = self.class_uses.clone();
        nested.function_uses = self.function_uses.clone();
        nested.const_uses = self.const_uses.clone();
        for (capture, by_ref) in &captures {
            let slot = nested.slot(capture);
            nested.capture_slots.push(slot);
            nested.capture_names.push(capture.clone());
            nested.capture_refs.push(*by_ref);
        }
        for param in &decl.params {
            let slot = nested.slot(&param.name);
            let default = match &param.default {
                Some(expr) => Some(const_expr(expr, decl.line)?),
                None => None,
            };
            nested.params.push(ParamSlot {
                name: param.name.clone(),
                slot,
                default,
                variadic: param.variadic,
                by_ref: param.by_ref,
            });
        }
        match &decl.body {
            ClosureBody::Block(body) => nested.compile_stmts(body)?,
            ClosureBody::Expr(body) => {
                if nested.returns_ref {
                    nested.load_ref_source(body)?;
                } else {
                    nested.expr(body)?;
                }
                nested.emit(Op::Return);
            }
        }
        let (function, inner, _) = nested.finish();
        self.functions.push(function);
        self.functions.extend(inner);

        for (capture, by_ref) in &captures {
            if *by_ref {
                self.load_var_ref(capture);
            } else {
                self.load_var(capture);
            }
        }
        let index = self.const_str(name.as_bytes());
        self.emit(Op::MakeClosure {
            name: index,
            argc: captures.len() as u32,
        });
        Ok(())
    }

    fn compile_break(&mut self, level: Option<u32>, is_continue: bool) -> Result<(), CompileError> {
        let level = level.unwrap_or(1) as usize;
        if level == 0 || level > self.loops.len() {
            return Err(CompileError::new(
                format!(
                    "Cannot {} {} level(s)",
                    if is_continue { "continue" } else { "break" },
                    level
                ),
                self.line,
            ));
        }
        let index = self.loops.len() - level;
        let jump = self.emit(Op::Jump(0));
        if is_continue {
            self.loops[index].continues.push(jump);
        } else {
            self.loops[index].breaks.push(jump);
        }
        Ok(())
    }

    // ----------------------------------------------------------- expressions

    fn expr(&mut self, expr: &Expr) -> Result<(), CompileError> {
        match expr {
            Expr::Null => {
                self.emit(Op::Null);
            }
            Expr::Bool(true) => {
                self.emit(Op::True);
            }
            Expr::Bool(false) => {
                self.emit(Op::False);
            }
            Expr::Int(value) => {
                self.emit(Op::Int(*value));
            }
            Expr::Float(value) => {
                self.emit(Op::Float(*value));
            }
            Expr::Str(bytes) => {
                let index = self.const_str(bytes);
                self.emit(Op::StrConst(index));
            }
            Expr::Interp(parts) => {
                if parts.is_empty() {
                    let index = self.const_str(b"");
                    self.emit(Op::StrConst(index));
                } else {
                    for (i, part) in parts.iter().enumerate() {
                        match part {
                            InterpPart::Lit(bytes) => {
                                let index = self.const_str(bytes);
                                self.emit(Op::StrConst(index));
                            }
                            InterpPart::Expr(expr) => self.expr(expr)?,
                        }
                        if i > 0 {
                            self.emit(Op::Binary(BinOp::Concat));
                        }
                    }
                }
            }
            Expr::Var(name) => self.load_var(name),
            Expr::Name(name) => {
                let lowered = name.to_ascii_lowercase();
                let magic = match lowered.as_str() {
                    "__file__" => Some(self.magic_file()),
                    "__dir__" => Some(self.magic_dir()),
                    "__namespace__" => Some(self.namespace.clone()),
                    "__class__" => Some(match self.name.split_once("::") {
                        Some((class, _)) => class.to_string(),
                        None => String::new(),
                    }),
                    "__method__" => Some(match self.name.as_str() {
                        "{main}" => String::new(),
                        name => name.to_string(),
                    }),
                    "__function__" => Some(match self.name.rsplit_once("::") {
                        Some((_, function)) => function.to_string(),
                        None if self.name == "{main}" => String::new(),
                        None => self.name.clone(),
                    }),
                    "__line__" => Some("1".to_string()),
                    _ => None,
                };
                if let Some(value) = magic {
                    let index = self.const_str(value.as_bytes());
                    self.emit(Op::StrConst(index));
                } else {
                    let name = self.qualify_constant(name);
                    let index = self.const_str(name.as_bytes());
                    self.emit(Op::ConstName(index));
                }
            }
            Expr::Array(items) => self.compile_array(items)?,
            Expr::Assign { target, op, value } => self.compile_assign(target, op, value)?,
            Expr::Binary { op, lhs, rhs } => self.compile_binary(*op, lhs, rhs)?,
            Expr::Unary { op, expr } => {
                self.expr(expr)?;
                self.emit(Op::Unary(*op));
            }
            Expr::Cast { kind, expr } => {
                self.expr(expr)?;
                self.emit(Op::Cast(*kind));
            }
            Expr::IncDec {
                target,
                increment,
                prefix,
            } => self.compile_incdec(target, *increment, *prefix)?,
            Expr::Ternary { cond, then_, else_ } => {
                self.compile_ternary(cond, then_.as_deref(), else_)?
            }
            Expr::Index { base, index } => {
                self.expr(base)?;
                match index {
                    Some(index) => {
                        self.expr(index)?;
                        self.emit(Op::Index);
                    }
                    None => return Err(CompileError::new("Cannot use [] for reading", self.line)),
                }
            }
            Expr::Call { name, args } => {
                let (name, fallback) = self.resolve_function_call(name);
                let signature = self.signatures.get(&name.to_ascii_lowercase()).cloned();
                let short_name = name.rsplit('\\').next().unwrap_or(&name);
                let builtin_by_ref = phpcraft_stdlib::by_ref_params(short_name);
                for (arg_index, arg) in args.iter().enumerate() {
                    if argument_is_by_ref(signature.as_ref(), arg_index)
                        || (signature.is_none() && builtin_by_ref.contains(&arg_index))
                    {
                        self.expr_arg_ref(arg)?;
                    } else {
                        self.expr(arg)?;
                    }
                }
                let index = self.const_str(name.as_bytes());
                self.emit(Op::Call {
                    name: index,
                    argc: args.len() as u32,
                    fallback,
                });
            }
            Expr::DynamicCall { callee, args } => {
                self.expr(callee)?;
                for arg in args {
                    self.expr_arg(arg)?;
                }
                self.emit(Op::CallValue {
                    argc: args.len() as u32,
                });
            }
            Expr::FirstClassCallable(inner) => self.compile_first_class_callable(inner)?,
            Expr::New { class, args } => {
                let class_name = self.qualify_class(class);
                let signature = self
                    .signatures
                    .get(&format!("{}::__construct", class_name).to_ascii_lowercase())
                    .cloned();
                for (arg_index, arg) in args.iter().enumerate() {
                    if argument_is_by_ref(signature.as_ref(), arg_index) {
                        self.expr_arg_ref(arg)?;
                    } else {
                        self.expr_arg(arg)?;
                    }
                }
                let index = self.const_str(class_name.as_bytes());
                self.emit(Op::NewObject {
                    class: index,
                    argc: args.len() as u32,
                });
            }
            Expr::Property { object, name } => {
                self.expr(object)?;
                let index = self.const_str(name.as_bytes());
                self.emit(Op::LoadProperty {
                    name: index,
                    safe: false,
                });
            }
            Expr::MethodCall {
                object,
                method,
                args,
            } => {
                self.expr(object)?;
                for (arg_index, arg) in args.iter().enumerate() {
                    if self.method_argument_by_ref(method, arg_index) {
                        self.expr_arg_ref(arg)?;
                    } else {
                        self.expr_arg(arg)?;
                    }
                }
                let index = self.const_str(method.as_bytes());
                self.emit(Op::CallMethod {
                    method: index,
                    argc: args.len() as u32,
                });
            }
            Expr::StaticCall {
                class,
                method,
                args,
            } => {
                let class_name = self.qualify_class(class);
                let signature = self
                    .signatures
                    .get(&format!("{}::{}", class_name, method).to_ascii_lowercase())
                    .cloned();
                for (arg_index, arg) in args.iter().enumerate() {
                    if argument_is_by_ref(signature.as_ref(), arg_index) {
                        self.expr_arg_ref(arg)?;
                    } else {
                        self.expr_arg(arg)?;
                    }
                }
                let class = self.const_str(class_name.as_bytes());
                let method = self.const_str(method.as_bytes());
                self.emit(Op::CallStaticMethod {
                    class,
                    method,
                    argc: args.len() as u32,
                });
            }
            Expr::StaticProperty { class, name } => {
                let class = self.qualify_class(class);
                let class = self.const_str(class.as_bytes());
                let name = self.const_str(name.as_bytes());
                self.emit(Op::LoadStaticProperty {
                    class,
                    name,
                    safe: false,
                });
            }
            Expr::ClassConst { class, name } => {
                let class = self.qualify_class(class);
                let class = self.const_str(class.as_bytes());
                let name = self.const_str(name.as_bytes());
                self.emit(Op::ClassConstant { class, name });
            }
            Expr::InstanceOf { expr, class } => {
                self.expr(expr)?;
                let class = self.qualify_class(class);
                let index = self.const_str(class.as_bytes());
                self.emit(Op::InstanceOf { class: index });
            }
            Expr::Clone(expr) => {
                self.expr(expr)?;
                self.emit(Op::CloneObject);
            }
            Expr::Reference(_) => {
                return Err(CompileError::new(
                    "reference expressions are only supported on the right-hand side of `=`",
                    self.line,
                ))
            }
            Expr::Throw(expr) => {
                self.expr(expr)?;
                self.emit(Op::Throw);
            }
            Expr::Include {
                path,
                once,
                require,
            } => {
                self.expr(path)?;
                self.emit(Op::Include {
                    once: *once,
                    require: *require,
                });
            }
            Expr::Closure(decl) => self.compile_closure(decl)?,
            Expr::Match { subject, arms } => self.compile_match(subject, arms)?,
            Expr::Isset(args) => self.compile_isset(args)?,
            Expr::Empty(expr) => {
                self.silent_read(expr)?;
                self.emit(Op::ToBool);
                self.emit(Op::Unary(UnOp::Not));
            }
        }
        Ok(())
    }

    fn compile_array(&mut self, items: &[ArrayItem]) -> Result<(), CompileError> {
        self.emit(Op::ArrayNew);
        let tmp = self.temp();
        self.emit(Op::StoreTmp(tmp));
        for item in items {
            if let Some(key) = &item.key {
                self.expr(key)?;
            }
            self.expr(&item.value)?;
            self.emit(Op::ArraySet {
                tmp,
                has_key: item.key.is_some(),
            });
        }
        self.emit(Op::LoadTmp(tmp));
        Ok(())
    }

    fn compile_match(&mut self, subject: &Expr, arms: &[MatchArm]) -> Result<(), CompileError> {
        let mut default: Option<&Expr> = None;
        for arm in arms {
            if arm.conditions.is_empty() {
                if default.is_some() {
                    return Err(CompileError::new(
                        "Match expressions may only contain one default arm",
                        self.line,
                    ));
                }
                default = Some(&arm.body);
            }
        }

        self.expr(subject)?;
        let subject_tmp = self.temp();
        self.emit(Op::StoreTmp(subject_tmp));

        let mut end_jumps = Vec::new();
        for arm in arms.iter().filter(|arm| !arm.conditions.is_empty()) {
            let mut matched_jumps = Vec::new();
            for condition in &arm.conditions {
                self.emit(Op::LoadTmp(subject_tmp));
                self.expr(condition)?;
                self.emit(Op::Binary(BinOp::Identical));
                matched_jumps.push(self.emit(Op::JumpIfTrue(0)));
            }
            let miss = self.emit(Op::Jump(0));
            let matched = self.ops.len();
            for jump in matched_jumps {
                self.patch(jump, matched);
            }
            self.expr(&arm.body)?;
            end_jumps.push(self.emit(Op::Jump(0)));
            let next_arm = self.ops.len();
            self.patch(miss, next_arm);
        }

        if let Some(body) = default {
            self.expr(body)?;
        } else {
            self.emit(Op::ThrowUnhandledMatch {
                subject: subject_tmp,
            });
        }

        let end = self.ops.len();
        for jump in end_jumps {
            self.patch(jump, end);
        }
        Ok(())
    }

    fn compile_assign(
        &mut self,
        target: &Expr,
        op: &Option<BinOp>,
        value: &Expr,
    ) -> Result<(), CompileError> {
        match op {
            None => {
                if let Expr::Reference(source) = value {
                    self.load_ref_source(source)?;
                    self.emit(Op::Dup);
                    self.store_ref_target(target)?;
                    return Ok(());
                }
                self.expr(value)?;
                self.store_target(target, true)?;
            }
            Some(BinOp::Coalesce) => {
                // `$a ??= b`: keep the existing value when it is not null.
                self.silent_read(target)?;
                let jump = self.emit(Op::JumpIfNotNull(0));
                self.expr(value)?;
                let end = self.ops.len();
                self.patch(jump, end);
                self.store_target(target, true)?;
            }
            Some(op) => {
                self.expr(target)?;
                self.expr(value)?;
                self.emit(Op::Binary(*op));
                self.store_target(target, true)?;
            }
        }
        Ok(())
    }

    fn compile_incdec(
        &mut self,
        target: &Expr,
        increment: bool,
        prefix: bool,
    ) -> Result<(), CompileError> {
        if prefix {
            self.expr(target)?;
            self.emit(Op::IncDec { increment });
            self.store_target(target, true)?;
        } else {
            let old = self.temp();
            self.expr(target)?;
            self.emit(Op::StoreTmp(old));
            self.emit(Op::LoadTmp(old));
            self.emit(Op::IncDec { increment });
            self.store_target(target, false)?;
            self.emit(Op::LoadTmp(old));
        }
        Ok(())
    }

    fn compile_binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> Result<(), CompileError> {
        match op {
            BinOp::LogicalAnd => {
                self.expr(lhs)?;
                let lhs_false = self.emit(Op::JumpIfFalse(0));
                self.expr(rhs)?;
                let rhs_false = self.emit(Op::JumpIfFalse(0));
                self.emit(Op::True);
                let to_end = self.emit(Op::Jump(0));
                let false_label = self.ops.len();
                self.patch(lhs_false, false_label);
                self.patch(rhs_false, false_label);
                self.emit(Op::False);
                let end = self.ops.len();
                self.patch(to_end, end);
            }
            BinOp::LogicalOr => {
                self.expr(lhs)?;
                let lhs_true = self.emit(Op::JumpIfTrue(0));
                self.expr(rhs)?;
                let rhs_true = self.emit(Op::JumpIfTrue(0));
                self.emit(Op::False);
                let to_end = self.emit(Op::Jump(0));
                let true_label = self.ops.len();
                self.patch(lhs_true, true_label);
                self.patch(rhs_true, true_label);
                self.emit(Op::True);
                let end = self.ops.len();
                self.patch(to_end, end);
            }
            BinOp::Coalesce => {
                self.silent_read(lhs)?;
                let jump = self.emit(Op::JumpIfNotNull(0));
                self.expr(rhs)?;
                let end = self.ops.len();
                self.patch(jump, end);
            }
            _ => {
                self.expr(lhs)?;
                self.expr(rhs)?;
                self.emit(Op::Binary(op));
            }
        }
        Ok(())
    }

    fn compile_ternary(
        &mut self,
        cond: &Expr,
        then_: Option<&Expr>,
        else_: &Expr,
    ) -> Result<(), CompileError> {
        match then_ {
            Some(then_) => {
                self.expr(cond)?;
                let to_else = self.emit(Op::JumpIfFalse(0));
                self.expr(then_)?;
                let to_end = self.emit(Op::Jump(0));
                let else_label = self.ops.len();
                self.patch(to_else, else_label);
                self.expr(else_)?;
                let end = self.ops.len();
                self.patch(to_end, end);
            }
            None => {
                // `a ?: b` evaluates `a` exactly once.
                let tmp = self.temp();
                self.expr(cond)?;
                self.emit(Op::StoreTmp(tmp));
                self.emit(Op::LoadTmp(tmp));
                let to_else = self.emit(Op::JumpIfFalse(0));
                self.emit(Op::LoadTmp(tmp));
                let to_end = self.emit(Op::Jump(0));
                let else_label = self.ops.len();
                self.patch(to_else, else_label);
                self.expr(else_)?;
                let end = self.ops.len();
                self.patch(to_end, end);
            }
        }
        Ok(())
    }

    fn compile_isset(&mut self, args: &[Expr]) -> Result<(), CompileError> {
        let mut false_jumps = Vec::new();
        for arg in args {
            self.silent_read(arg)?;
            self.emit(Op::IsNotNull);
            false_jumps.push(self.emit(Op::JumpIfFalse(0)));
        }
        self.emit(Op::True);
        let to_end = self.emit(Op::Jump(0));
        let false_label = self.ops.len();
        for jump in false_jumps {
            self.patch(jump, false_label);
        }
        self.emit(Op::False);
        let end = self.ops.len();
        self.patch(to_end, end);
        Ok(())
    }
}

/// Evaluate a compile-time constant expression (parameter defaults only).
fn note_free_var(
    name: &str,
    params: &HashSet<String>,
    out: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    if params.contains(name) || !seen.insert(name.to_string()) {
        return;
    }
    out.push(name.to_string());
}

fn collect_free_vars(
    expr: &Expr,
    params: &HashSet<String>,
    out: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    match expr {
        Expr::Var(name) => note_free_var(name, params, out, seen),
        Expr::Cast { expr, .. } | Expr::Unary { expr, .. } | Expr::Empty(expr) => {
            collect_free_vars(expr, params, out, seen);
        }
        Expr::Interp(parts) => {
            for part in parts {
                if let InterpPart::Expr(expr) = part {
                    collect_free_vars(expr, params, out, seen);
                }
            }
        }
        Expr::Array(items) => {
            for item in items {
                if let Some(key) = &item.key {
                    collect_free_vars(key, params, out, seen);
                }
                collect_free_vars(&item.value, params, out, seen);
            }
        }
        Expr::Assign { target, value, .. } => {
            collect_free_vars(target, params, out, seen);
            collect_free_vars(value, params, out, seen);
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_free_vars(lhs, params, out, seen);
            collect_free_vars(rhs, params, out, seen);
        }
        Expr::IncDec { target, .. } => collect_free_vars(target, params, out, seen),
        Expr::Ternary { cond, then_, else_ } => {
            collect_free_vars(cond, params, out, seen);
            if let Some(then_) = then_ {
                collect_free_vars(then_, params, out, seen);
            }
            collect_free_vars(else_, params, out, seen);
        }
        Expr::Index { base, index } => {
            collect_free_vars(base, params, out, seen);
            if let Some(index) = index {
                collect_free_vars(index, params, out, seen);
            }
        }
        Expr::Call { args, .. } => {
            for arg in args {
                collect_free_vars(arg, params, out, seen);
            }
        }
        Expr::DynamicCall { callee, args } => {
            collect_free_vars(callee, params, out, seen);
            for arg in args {
                collect_free_vars(arg, params, out, seen);
            }
        }
        Expr::FirstClassCallable(inner) => collect_free_vars(inner, params, out, seen),
        Expr::New { args, .. } | Expr::StaticCall { args, .. } => {
            for arg in args {
                collect_free_vars(arg, params, out, seen);
            }
        }
        Expr::Property { object, .. } => collect_free_vars(object, params, out, seen),
        Expr::MethodCall { object, args, .. } => {
            collect_free_vars(object, params, out, seen);
            for arg in args {
                collect_free_vars(arg, params, out, seen);
            }
        }
        Expr::InstanceOf { expr, .. } => collect_free_vars(expr, params, out, seen),
        Expr::Clone(expr) | Expr::Reference(expr) | Expr::Throw(expr) => {
            collect_free_vars(expr, params, out, seen)
        }
        Expr::Include { path, .. } => collect_free_vars(path, params, out, seen),
        Expr::Closure(decl) => {
            if decl.is_arrow() {
                let mut nested_params = params.clone();
                nested_params.extend(decl.params.iter().map(|param| param.name.clone()));
                if let ClosureBody::Expr(body) = &decl.body {
                    collect_free_vars(body, &nested_params, out, seen);
                }
            } else {
                for capture in &decl.captures {
                    if !capture.by_ref {
                        note_free_var(&capture.name, params, out, seen);
                    }
                }
            }
        }
        Expr::Match { subject, arms } => {
            collect_free_vars(subject, params, out, seen);
            for arm in arms {
                for condition in &arm.conditions {
                    collect_free_vars(condition, params, out, seen);
                }
                collect_free_vars(&arm.body, params, out, seen);
            }
        }
        Expr::Isset(args) => {
            for arg in args {
                collect_free_vars(arg, params, out, seen);
            }
        }
        Expr::Null
        | Expr::Bool(_)
        | Expr::Int(_)
        | Expr::Float(_)
        | Expr::Str(_)
        | Expr::Name(_)
        | Expr::StaticProperty { .. }
        | Expr::ClassConst { .. } => {}
    }
}

fn const_expr(expr: &Expr, line: u32) -> Result<Value, CompileError> {
    match expr {
        Expr::Null => Ok(Value::Null),
        Expr::Bool(value) => Ok(Value::Bool(*value)),
        Expr::Int(value) => Ok(Value::Int(*value)),
        Expr::Float(value) => Ok(Value::Float(*value)),
        Expr::Str(bytes) => Ok(Value::str(bytes.clone())),
        Expr::Unary { op, expr } => {
            let value = const_expr(expr, line)?;
            match (op, value) {
                (UnOp::Neg, Value::Int(value)) => Ok(Value::Int(value.wrapping_neg())),
                (UnOp::Neg, Value::Float(value)) => Ok(Value::Float(-value)),
                (UnOp::Pos, value) => Ok(value),
                (UnOp::BitNot, Value::Int(value)) => Ok(Value::Int(!value)),
                (UnOp::Not, value) => Ok(Value::Bool(!value.to_bool())),
                (op, _) => Err(CompileError::new(
                    format!("unsupported constant expression operator {}", op.symbol()),
                    line,
                )),
            }
        }
        Expr::Array(items) => {
            let mut array = PhpArray::new();
            for item in items {
                let value = const_expr(&item.value, line)?;
                match &item.key {
                    Some(key) => {
                        let key_value = const_expr(key, line)?;
                        let key = key_from_value(&key_value).map_err(|e| {
                            CompileError::new(format!("invalid array key: {}", e.message), line)
                        })?;
                        array.insert(key, value);
                    }
                    None => {
                        array.push(value);
                    }
                }
            }
            Ok(Value::array(array))
        }
        _ => Err(CompileError::new(
            "default parameter values must be constant expressions",
            line,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use phpcraft_parser::parse_file;

    #[test]
    fn compiles_loops_and_functions() {
        let program = parse_file(
            br#"<?php
            function fib(int $n) {
                if ($n < 2) { return $n; }
                return fib($n - 1) + fib($n - 2);
            }
            for ($i = 0; $i < 5; $i++) { echo fib($i), " "; }
        "#,
        )
        .unwrap();
        let compiled = compile(&program, "test.php").unwrap();
        assert!(!compiled.main.chunk.ops.is_empty());
        assert_eq!(compiled.functions.len(), 1);
        assert_eq!(compiled.functions[0].name, "fib");
    }

    #[test]
    fn rejects_non_constant_defaults() {
        let program = parse_file(br#"<?php function f($a = strlen("x")) {}"#).unwrap();
        let err = compile(&program, "test.php").unwrap_err();
        assert!(err.message.contains("constant"));
    }
}
