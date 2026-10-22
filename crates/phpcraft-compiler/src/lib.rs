//! AST -> bytecode compiler.
//!
//! Zend compiles to a register machine (`zend_op_array` with `op1/op2/result`
//! slots); PHPCraft compiles to a simpler stack machine. The control-flow
//! lowering (short-circuit operators, ternary, `??`, loops, `foreach`,
//! `switch`) follows the same strategies as `zend_compile.c`.

use std::collections::{HashMap, HashSet};
use std::fmt;

use phpcraft_ast::{ArrayItem, BinOp, Expr, FunctionDecl, InterpPart, Program, Stmt, UnOp};
use phpcraft_bytecode::{Chunk, CompiledProgram, DimMode, Function, Op, ParamSlot};
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

/// Compile a parsed program. `file` is only used for diagnostics.
pub fn compile(program: &Program, file: &str) -> Result<CompiledProgram, CompileError> {
    let mut compiler = FnCompiler::new_main(file);
    compiler.compile_stmts(&program.stmts)?;
    let (main, functions) = compiler.finish();

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

    Ok(CompiledProgram {
        main,
        functions,
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
    functions: Vec<Function>,
    file: String,
}

impl FnCompiler {
    fn new_main(file: &str) -> Self {
        Self {
            name: "{main}".to_string(),
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
            functions: Vec::new(),
            file: file.to_string(),
        }
    }

    fn new_function(decl: &FunctionDecl, file: &str) -> Self {
        Self {
            name: decl.name.clone(),
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
            functions: Vec::new(),
            file: file.to_string(),
        }
    }

    fn finish(mut self) -> (Function, Vec<Function>) {
        self.emit(Op::ReturnNull);
        let mut slot_names = vec![None; self.next_slot as usize];
        for (name, slot) in &self.locals {
            slot_names[*slot as usize] = Some(name.clone());
        }
        let function = Function {
            name: self.name,
            params: self.params,
            chunk: Chunk {
                ops: self.ops,
                consts: self.consts,
            },
            max_slots: self.next_slot,
            slot_names,
            line: self.line,
        };
        (function, self.functions)
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
            Expr::Index { .. } => {
                let (root, dims) = Self::lvalue_chain(target).ok_or_else(|| {
                    CompileError::new("cannot assign to this expression", self.line)
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

    fn compile_function_decl(&mut self, decl: &FunctionDecl) -> Result<(), CompileError> {
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
        let (function, inner) = nested.finish();
        self.functions.push(function);
        self.functions.extend(inner);
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
