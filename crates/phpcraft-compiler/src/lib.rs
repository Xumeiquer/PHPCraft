//! AST -> bytecode compiler.
//!
//! Zend compiles to a register machine (`zend_op_array` with `op1/op2/result`
//! slots); PHPCraft compiles to a simpler stack machine. The control-flow
//! lowering (short-circuit operators, ternary, `??`, loops, `foreach`,
//! `switch`) follows the same strategies as `zend_compile.c`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use phpcraft_ast::{
    ArrayItem, BinOp, CatchClause, ClassDecl, ClosureBody, ClosureDecl, Expr, FunctionDecl,
    InterpPart, MatchArm, Param, Program, Stmt, UnOp,
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

/// Compile a parsed program. `file` is only used for diagnostics.
pub fn compile(program: &Program, file: &str) -> Result<CompiledProgram, CompileError> {
    let mut compiler = FnCompiler::new_main(file);
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
    functions: Vec<Function>,
    classes: Vec<ClassDef>,
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
            functions: Vec::new(),
            classes: Vec::new(),
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
            functions: Vec::new(),
            classes: Vec::new(),
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
            functions: Vec::new(),
            classes: Vec::new(),
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
                body,
            } => self.compile_foreach(subject, key.as_ref(), value, body),
            Stmt::Switch { subject, cases } => self.compile_switch(subject, cases),
            Stmt::Function(decl) => self.compile_function_decl(decl),
            Stmt::Class(decl) => self.compile_class(decl),
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
            Stmt::Return(value) => {
                match value {
                    Some(expr) => {
                        self.expr(expr)?;
                        self.emit(Op::Return);
                    }
                    None => {
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
        body: &[Stmt],
    ) -> Result<(), CompileError> {
        self.expr(subject)?;
        let iter = self.temp();
        self.emit(Op::IterInit { iter });

        let value_tmp = self.temp();
        let key_tmp = key.map(|_| self.temp());
        let loop_start = self.ops.len();
        let next = self.emit(Op::IterNext {
            iter,
            key: key_tmp,
            value: value_tmp,
            end: 0,
        });

        self.loops.push(LoopCtx::default());
        if let (Some(key_target), Some(key_slot)) = (key, key_tmp) {
            self.emit(Op::LoadTmp(key_slot));
            self.store_target(key_target, false)?;
        }
        self.emit(Op::LoadTmp(value_tmp));
        self.store_target(value, false)?;
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
            let function_name = format!("{}::{}", decl.name, method.name);
            let mut nested = FnCompiler::new_method(&function_name, method.line, &self.file);
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
            name: decl.name.clone(),
            parent: decl.parent.clone(),
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
        self.compile_stmts(body)?;
        self.emit(Op::TryEnd);
        let try_done = self.emit(Op::Jump(0));
        let catch_label = self.ops.len();
        if let Op::TryStart { catch_ip, .. } = &mut self.ops[start] {
            *catch_ip = catch_label;
        }

        let mut caught_jumps = Vec::new();
        for catch in catches {
            let mut hit_jumps = Vec::new();
            for class in &catch.classes {
                self.emit(Op::LoadTmp(catch_slot));
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
            self.compile_stmts(&catch.body)?;
            caught_jumps.push(self.emit(Op::Jump(0)));
            let next_label = self.ops.len();
            self.patch(next_catch, next_label);
        }
        if !catches.is_empty() {
            self.emit(Op::Rethrow { slot: catch_slot });
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
        Ok(())
    }

    fn compile_function_decl(&mut self, decl: &FunctionDecl) -> Result<(), CompileError> {
        check_param_names(&decl.params, decl.line)?;
        let mut nested = FnCompiler::new_function(decl, &self.file);
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
            });
        }
        nested.compile_stmts(&decl.body)?;
        let (function, inner, _) = nested.finish();
        self.functions.push(function);
        self.functions.extend(inner);
        Ok(())
    }

    fn compile_closure(&mut self, decl: &ClosureDecl) -> Result<(), CompileError> {
        let captures = if decl.is_arrow() {
            let params: HashSet<String> = decl.params.iter().map(|p| p.name.clone()).collect();
            let mut captures = Vec::new();
            let mut seen = HashSet::new();
            match &decl.body {
                ClosureBody::Expr(body) => {
                    collect_free_vars(body, &params, &mut captures, &mut seen);
                }
                ClosureBody::Block(_) => {}
            }
            captures
        } else {
            if let Some(capture) = decl.captures.iter().find(|capture| capture.by_ref) {
                return Err(CompileError::new(
                    format!(
                        "by-reference closure captures are not supported (${})",
                        capture.name
                    ),
                    decl.line,
                ));
            }
            let captures: Vec<String> = decl
                .captures
                .iter()
                .map(|capture| capture.name.clone())
                .collect();
            let mut seen: HashSet<&str> = HashSet::new();
            if let Some(capture) = captures.iter().find(|name| !seen.insert(name.as_str())) {
                return Err(CompileError::new(
                    format!("Cannot use variable ${capture} twice"),
                    decl.line,
                ));
            }
            if let Some(capture) = captures
                .iter()
                .find(|name| decl.params.iter().any(|param| &param.name == *name))
            {
                return Err(CompileError::new(
                    format!("Cannot use lexical variable ${capture} as a parameter name"),
                    decl.line,
                ));
            }
            captures
        };
        check_param_names(&decl.params, decl.line)?;
        if captures.iter().any(|name| name == "this") {
            return Err(CompileError::new(
                "Cannot use $this as lexical variable",
                decl.line,
            ));
        }

        let name = next_closure_name(decl.line);
        let mut nested = FnCompiler::new_closure(&name, decl.line, &self.file);
        for capture in &captures {
            let slot = nested.slot(capture);
            nested.capture_slots.push(slot);
            nested.capture_names.push(capture.clone());
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
            });
        }
        match &decl.body {
            ClosureBody::Block(body) => nested.compile_stmts(body)?,
            ClosureBody::Expr(body) => {
                nested.expr(body)?;
                nested.emit(Op::Return);
            }
        }
        let (function, inner, _) = nested.finish();
        self.functions.push(function);
        self.functions.extend(inner);

        for capture in &captures {
            self.load_var(capture);
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
                let index = self.const_str(name.as_bytes());
                self.emit(Op::ConstName(index));
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
                for arg in args {
                    self.expr(arg)?;
                }
                let index = self.const_str(name.as_bytes());
                self.emit(Op::Call {
                    name: index,
                    argc: args.len() as u32,
                });
            }
            Expr::DynamicCall { callee, args } => {
                self.expr(callee)?;
                for arg in args {
                    self.expr(arg)?;
                }
                self.emit(Op::CallValue {
                    argc: args.len() as u32,
                });
            }
            Expr::New { class, args } => {
                for arg in args {
                    self.expr(arg)?;
                }
                let index = self.const_str(class.as_bytes());
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
                for arg in args {
                    self.expr(arg)?;
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
                for arg in args {
                    self.expr(arg)?;
                }
                let class = self.const_str(class.as_bytes());
                let method = self.const_str(method.as_bytes());
                self.emit(Op::CallStaticMethod {
                    class,
                    method,
                    argc: args.len() as u32,
                });
            }
            Expr::StaticProperty { class, name } => {
                let class = self.const_str(class.as_bytes());
                let name = self.const_str(name.as_bytes());
                self.emit(Op::LoadStaticProperty {
                    class,
                    name,
                    safe: false,
                });
            }
            Expr::ClassConst { class, name } => {
                let class = self.const_str(class.as_bytes());
                let name = self.const_str(name.as_bytes());
                self.emit(Op::ClassConstant { class, name });
            }
            Expr::InstanceOf { expr, class } => {
                self.expr(expr)?;
                let index = self.const_str(class.as_bytes());
                self.emit(Op::InstanceOf { class: index });
            }
            Expr::Clone(expr) => {
                self.expr(expr)?;
                self.emit(Op::CloneObject);
            }
            Expr::Throw(expr) => {
                self.expr(expr)?;
                self.emit(Op::Throw);
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
        Expr::Clone(expr) | Expr::Throw(expr) => collect_free_vars(expr, params, out, seen),
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
