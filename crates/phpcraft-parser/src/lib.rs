//! PHP parser.
//!
//! Statement parsing is recursive descent; expressions use a Pratt parser with
//! the PHP 8 operator precedence table (`zend_operators` / `zend_language_parser.y`).
//!
//! Unsupported constructs produce a parse error naming the feature, so the
//! boundary of the implementation is always explicit.

use phpcraft_ast::{
    ArrayItem, BinOp, Capture, CastKind, ClosureBody, ClosureDecl, Expr, FunctionDecl, InterpPart,
    MatchArm, Param, Program, Stmt, SwitchCase, UnOp,
};
use phpcraft_lexer::{self, DimKey, StrPart, Tok, Token};

#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: u32,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} on line {}", self.message, self.line)
    }
}

impl std::error::Error for ParseError {}

impl From<phpcraft_lexer::LexError> for ParseError {
    fn from(value: phpcraft_lexer::LexError) -> Self {
        Self {
            message: value.message,
            line: value.line,
        }
    }
}

/// Parse a complete PHP file.
pub fn parse_file(source: &[u8]) -> Result<Program, ParseError> {
    let tokens = phpcraft_lexer::tokenize(source)?;
    Parser::new(tokens).parse_program()
}

/// Parse a PHP fragment (no opening tag), as passed to `php -r`.
pub fn parse_fragment(source: &[u8]) -> Result<Program, ParseError> {
    let tokens = phpcraft_lexer::tokenize_fragment(source)?;
    Parser::new(tokens).parse_program()
}

/// Parse a bare PHP fragment (no tags) into a single expression.
pub fn parse_expression_fragment(source: &[u8]) -> Result<Expr, ParseError> {
    let tokens = phpcraft_lexer::tokenize_fragment(source)?;
    let mut parser = Parser::new(tokens);
    let expr = parser.parse_expr()?;
    parser.expect(&Tok::Eof)?;
    Ok(expr)
}

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// Guards against pathological nesting blowing the Rust stack.
    depth: u32,
}

const MAX_DEPTH: u32 = 512;

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            pos: 0,
            depth: 0,
        }
    }

    // ------------------------------------------------------------- utilities

    fn peek(&self) -> &Tok {
        &self.tokens[self.pos.min(self.tokens.len() - 1)].kind
    }

    fn peek_at(&self, offset: usize) -> &Tok {
        let index = (self.pos + offset).min(self.tokens.len() - 1);
        &self.tokens[index].kind
    }

    fn line(&self) -> u32 {
        self.tokens[self.pos.min(self.tokens.len() - 1)].line
    }

    fn advance(&mut self) -> Tok {
        let tok = self.tokens[self.pos.min(self.tokens.len() - 1)]
            .kind
            .clone();
        if self.pos < self.tokens.len().saturating_sub(1) {
            self.pos += 1;
        }
        tok
    }

    fn eat(&mut self, kind: &Tok) -> bool {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &Tok) -> Result<(), ParseError> {
        if self.eat(kind) {
            Ok(())
        } else {
            Err(self.error(format!(
                "syntax error, unexpected {}, expecting {}",
                self.peek().describe(),
                kind.describe()
            )))
        }
    }

    fn peek_keyword(&self) -> Option<String> {
        match self.peek() {
            Tok::Ident(name) => Some(name.to_ascii_lowercase()),
            _ => None,
        }
    }

    fn check_keyword(&self, keyword: &str) -> bool {
        self.peek_keyword().as_deref() == Some(keyword)
    }

    fn eat_keyword(&mut self, keyword: &str) -> bool {
        if self.check_keyword(keyword) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_keyword(&mut self, keyword: &str) -> Result<(), ParseError> {
        if self.eat_keyword(keyword) {
            Ok(())
        } else {
            Err(self.error(format!(
                "syntax error, unexpected {}, expecting \"{keyword}\"",
                self.peek().describe()
            )))
        }
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.peek().clone() {
            Tok::Ident(name) => {
                self.advance();
                Ok(name)
            }
            other => Err(self.error(format!(
                "syntax error, unexpected {}, expecting identifier",
                other.describe()
            ))),
        }
    }

    fn expect_variable(&mut self) -> Result<String, ParseError> {
        match self.peek().clone() {
            Tok::Var(name) => {
                self.advance();
                Ok(name)
            }
            other => Err(self.error(format!(
                "syntax error, unexpected {}, expecting variable",
                other.describe()
            ))),
        }
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            message: message.into(),
            line: self.line(),
        }
    }

    fn unsupported(&self, feature: &str) -> ParseError {
        self.error(format!("{feature} is not supported by PHPCraft yet"))
    }

    fn enter(&mut self) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error("nesting level too deep"));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    // ------------------------------------------------------------ statements

    pub fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut stmts = Vec::new();
        while !matches!(self.peek(), Tok::Eof) {
            if self.eat(&Tok::Semi) {
                continue;
            }
            stmts.push(self.parse_stmt()?);
        }
        Ok(Program { stmts })
    }

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        self.enter()?;
        let result = self.parse_stmt_inner();
        self.leave();
        result
    }

    fn parse_stmt_inner(&mut self) -> Result<Stmt, ParseError> {
        if let Tok::InlineHtml(bytes) = self.peek().clone() {
            self.advance();
            return Ok(Stmt::InlineHtml(bytes));
        }

        if self.check_keyword("echo") {
            self.advance();
            let exprs = self.parse_expr_list()?;
            self.end_statement()?;
            return Ok(Stmt::Echo(exprs));
        }

        if self.check_keyword("if") {
            return self.parse_if();
        }
        if self.check_keyword("while") {
            return self.parse_while();
        }
        if self.check_keyword("do") {
            return self.parse_do_while();
        }
        if self.check_keyword("for") {
            return self.parse_for();
        }
        if self.check_keyword("foreach") {
            return self.parse_foreach();
        }
        if self.check_keyword("switch") {
            return self.parse_switch();
        }
        // A statement starting with `function` is a declaration only when a
        // name follows; otherwise it is a closure expression.
        if self.check_keyword("function") && matches!(self.peek_at(1), Tok::Ident(_)) {
            return Ok(Stmt::Function(self.parse_function()?));
        }
        if self.check_keyword("return") {
            self.advance();
            let value = if matches!(self.peek(), Tok::Semi | Tok::Eof) {
                None
            } else {
                Some(self.parse_expr()?)
            };
            self.end_statement()?;
            return Ok(Stmt::Return(value));
        }
        if self.check_keyword("break") {
            self.advance();
            let level = self.parse_break_level()?;
            self.end_statement()?;
            return Ok(Stmt::Break(level));
        }
        if self.check_keyword("continue") {
            self.advance();
            let level = self.parse_break_level()?;
            self.end_statement()?;
            return Ok(Stmt::Continue(level));
        }
        if self.check_keyword("global") {
            self.advance();
            let mut names = Vec::new();
            loop {
                names.push(self.expect_variable()?);
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
            self.end_statement()?;
            return Ok(Stmt::Global(names));
        }

        for keyword in [
            "class",
            "interface",
            "trait",
            "enum",
            "namespace",
            "use",
            "const",
            "static",
            "unset",
            "try",
            "throw",
            "declare",
            "abstract",
            "final",
            "readonly",
            "typedef",
        ] {
            if self.check_keyword(keyword) {
                return Err(self.unsupported(&format!("\"{keyword}\"")));
            }
        }

        if self.eat(&Tok::LBrace) {
            let stmts = self.parse_block_body()?;
            return Ok(Stmt::Block(stmts));
        }

        if self.eat(&Tok::Semi) {
            return Ok(Stmt::Nop);
        }

        let expr = self.parse_expr()?;
        self.end_statement()?;
        Ok(Stmt::Expr(expr))
    }

    /// `;`, or the implicit semicolon produced by `?>`.
    fn end_statement(&mut self) -> Result<(), ParseError> {
        if self.eat(&Tok::Semi) {
            return Ok(());
        }
        Err(self.error(format!(
            "syntax error, unexpected {}, expecting \";\"",
            self.peek().describe()
        )))
    }

    fn parse_break_level(&mut self) -> Result<Option<u32>, ParseError> {
        if let Tok::Int(value) = self.peek().clone() {
            if value <= 0 {
                return Err(
                    self.error("break/continue operator must be followed by a positive integer")
                );
            }
            self.advance();
            return Ok(Some(value as u32));
        }
        Ok(None)
    }

    fn parse_block_body(&mut self) -> Result<Vec<Stmt>, ParseError> {
        let mut stmts = Vec::new();
        while !matches!(self.peek(), Tok::RBrace | Tok::Eof) {
            if self.eat(&Tok::Semi) {
                continue;
            }
            stmts.push(self.parse_stmt()?);
        }
        self.expect(&Tok::RBrace)?;
        Ok(stmts)
    }

    fn parse_stmt_or_block(&mut self) -> Result<Vec<Stmt>, ParseError> {
        if self.eat(&Tok::LBrace) {
            self.parse_block_body()
        } else {
            if matches!(self.peek(), Tok::Eof) {
                return Err(self.error("syntax error, unexpected end of file"));
            }
            Ok(vec![self.parse_stmt()?])
        }
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        self.expect_keyword("if")?;
        self.expect(&Tok::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(&Tok::RParen)?;
        if matches!(self.peek(), Tok::Colon) {
            return Err(self.unsupported("alternative if syntax (: ... endif)"));
        }
        let body = self.parse_stmt_or_block()?;

        let mut branches = vec![(cond, body)];
        let mut else_ = None;
        loop {
            if self.check_keyword("elseif") {
                self.advance();
                self.expect(&Tok::LParen)?;
                let cond = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                let body = self.parse_stmt_or_block()?;
                branches.push((cond, body));
            } else if self.check_keyword("else") {
                self.advance();
                // `else if` is a nested if inside the else branch.
                else_ = if self.check_keyword("if") {
                    Some(vec![self.parse_if()?])
                } else {
                    Some(self.parse_stmt_or_block()?)
                };
                break;
            } else {
                break;
            }
        }
        Ok(Stmt::If { branches, else_ })
    }

    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        self.expect_keyword("while")?;
        self.expect(&Tok::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(&Tok::RParen)?;
        if matches!(self.peek(), Tok::Colon) {
            return Err(self.unsupported("alternative while syntax (: ... endwhile)"));
        }
        let body = self.parse_stmt_or_block()?;
        Ok(Stmt::While { cond, body })
    }

    fn parse_do_while(&mut self) -> Result<Stmt, ParseError> {
        self.expect_keyword("do")?;
        let body = self.parse_stmt_or_block()?;
        self.expect_keyword("while")?;
        self.expect(&Tok::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(&Tok::RParen)?;
        self.end_statement()?;
        Ok(Stmt::DoWhile { body, cond })
    }

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        self.expect_keyword("for")?;
        self.expect(&Tok::LParen)?;
        let init = self.parse_expr_list_until(&Tok::Semi)?;
        self.expect(&Tok::Semi)?;
        let cond = self.parse_expr_list_until(&Tok::Semi)?;
        self.expect(&Tok::Semi)?;
        let step = self.parse_expr_list_until(&Tok::RParen)?;
        self.expect(&Tok::RParen)?;
        if matches!(self.peek(), Tok::Colon) {
            return Err(self.unsupported("alternative for syntax (: ... endfor)"));
        }
        let body = self.parse_stmt_or_block()?;
        Ok(Stmt::For {
            init,
            cond,
            step,
            body,
        })
    }

    fn parse_expr_list(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut exprs = vec![self.parse_expr()?];
        while self.eat(&Tok::Comma) {
            exprs.push(self.parse_expr()?);
        }
        Ok(exprs)
    }

    fn parse_expr_list_until(&mut self, end: &Tok) -> Result<Vec<Expr>, ParseError> {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(end) {
            return Ok(Vec::new());
        }
        let mut exprs = vec![self.parse_expr()?];
        while self.eat(&Tok::Comma) {
            exprs.push(self.parse_expr()?);
        }
        Ok(exprs)
    }

    fn parse_foreach(&mut self) -> Result<Stmt, ParseError> {
        self.expect_keyword("foreach")?;
        self.expect(&Tok::LParen)?;
        let subject = self.parse_expr()?;
        self.expect_keyword("as")?;
        if self.check_keyword("list") {
            return Err(self.unsupported("list() destructuring in foreach"));
        }
        if matches!(self.peek(), Tok::BitAnd) {
            return Err(self.unsupported("foreach by reference (&$value)"));
        }
        let first = self.parse_expr()?;
        let (key, value) = if self.eat(&Tok::DoubleArrow) {
            if matches!(self.peek(), Tok::BitAnd) {
                return Err(self.unsupported("foreach by reference (&$value)"));
            }
            let value = self.parse_expr()?;
            (Some(first), value)
        } else {
            (None, first)
        };
        if let Some(key) = &key {
            if !is_valid_target(key) {
                return Err(self.error("foreach key must be a variable"));
            }
        }
        if !is_valid_target(&value) {
            return Err(self.error("foreach value must be a variable"));
        }
        self.expect(&Tok::RParen)?;
        if matches!(self.peek(), Tok::Colon) {
            return Err(self.unsupported("alternative foreach syntax (: ... endforeach)"));
        }
        let body = self.parse_stmt_or_block()?;
        Ok(Stmt::Foreach {
            subject,
            key,
            value,
            body,
        })
    }

    fn parse_switch(&mut self) -> Result<Stmt, ParseError> {
        self.expect_keyword("switch")?;
        self.expect(&Tok::LParen)?;
        let subject = self.parse_expr()?;
        self.expect(&Tok::RParen)?;
        if matches!(self.peek(), Tok::Colon) {
            return Err(self.unsupported("alternative switch syntax (: ... endswitch)"));
        }
        self.expect(&Tok::LBrace)?;

        let mut cases: Vec<SwitchCase> = Vec::new();
        loop {
            if matches!(self.peek(), Tok::RBrace) {
                self.advance();
                break;
            }
            if matches!(self.peek(), Tok::Eof) {
                return Err(self.error("syntax error, unexpected end of file"));
            }
            if self.eat(&Tok::Semi) {
                continue;
            }
            if self.check_keyword("case") {
                self.advance();
                let test = self.parse_expr()?;
                self.expect(&Tok::Colon)?;
                cases.push(SwitchCase {
                    test: Some(test),
                    body: Vec::new(),
                });
                continue;
            }
            if self.check_keyword("default") {
                self.advance();
                self.expect(&Tok::Colon)?;
                cases.push(SwitchCase {
                    test: None,
                    body: Vec::new(),
                });
                continue;
            }
            let stmt = self.parse_stmt()?;
            match cases.last_mut() {
                Some(case) => case.body.push(stmt),
                None => return Err(self.error("statement before the first case label")),
            }
        }
        Ok(Stmt::Switch { subject, cases })
    }

    fn parse_function(&mut self) -> Result<FunctionDecl, ParseError> {
        let line = self.line();
        self.expect_keyword("function")?;
        if matches!(self.peek(), Tok::LParen) {
            return Err(self.unsupported("anonymous functions (closures)"));
        }
        let name = self.expect_ident()?;
        self.expect(&Tok::LParen)?;
        let params = self.parse_params()?;
        if self.eat(&Tok::Colon) {
            // Return type: consume until the body.
            while !matches!(self.peek(), Tok::LBrace | Tok::Semi | Tok::Eof) {
                self.advance();
            }
        }
        if self.eat(&Tok::Semi) {
            return Err(self.unsupported("abstract/interface function declarations"));
        }
        self.expect(&Tok::LBrace)?;
        let body = self.parse_block_body()?;
        Ok(FunctionDecl {
            name,
            params,
            body,
            line,
        })
    }

    fn parse_closure_expr(&mut self, arrow: bool) -> Result<Expr, ParseError> {
        let line = self.line();
        if arrow {
            self.expect_keyword("fn")?;
        } else {
            self.expect_keyword("function")?;
        }
        if matches!(self.peek(), Tok::BitAnd) {
            return Err(self.unsupported("by-reference closure returns"));
        }
        self.expect(&Tok::LParen)?;
        let params = self.parse_params()?;

        let mut captures = Vec::new();
        if !arrow && self.eat_keyword("use") {
            self.expect(&Tok::LParen)?;
            if self.eat(&Tok::RParen) {
                return Err(
                    self.error("syntax error, unexpected \")\", expecting variable in use (...)")
                );
            }
            loop {
                let by_ref = self.eat(&Tok::BitAnd);
                let name = self.expect_variable()?;
                captures.push(Capture { name, by_ref });
                if self.eat(&Tok::Comma) {
                    if self.eat(&Tok::RParen) {
                        break;
                    }
                    continue;
                }
                self.expect(&Tok::RParen)?;
                break;
            }
        } else if arrow && self.check_keyword("use") {
            return Err(self.unsupported("use (...) on arrow functions"));
        }

        if self.eat(&Tok::Colon) {
            while !matches!(
                self.peek(),
                Tok::LBrace | Tok::DoubleArrow | Tok::Semi | Tok::Eof
            ) {
                self.advance();
            }
        }

        let body = if arrow {
            self.expect(&Tok::DoubleArrow)?;
            ClosureBody::Expr(self.parse_expr()?)
        } else {
            self.expect(&Tok::LBrace)?;
            ClosureBody::Block(self.parse_block_body()?)
        };

        Ok(Expr::Closure(Box::new(ClosureDecl {
            params,
            captures,
            body,
            line,
        })))
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, ParseError> {
        let mut params = Vec::new();
        if self.eat(&Tok::RParen) {
            return Ok(params);
        }
        loop {
            if self.eat(&Tok::RParen) {
                break;
            }
            // Optional type declaration: consume tokens until the variable.
            while !matches!(
                self.peek(),
                Tok::Var(_) | Tok::Ellipsis | Tok::BitAnd | Tok::Comma | Tok::RParen
            ) {
                self.advance();
            }
            let variadic = self.eat(&Tok::Ellipsis);
            if matches!(self.peek(), Tok::BitAnd) {
                return Err(self.unsupported("by-reference parameters (&$param)"));
            }
            let name = self.expect_variable()?;
            let default = if self.eat(&Tok::Assign) {
                Some(self.parse_expr()?)
            } else {
                None
            };
            params.push(Param {
                name,
                default,
                variadic,
            });
            if self.eat(&Tok::Comma) {
                continue;
            }
            self.expect(&Tok::RParen)?;
            break;
        }
        Ok(params)
    }

    // ----------------------------------------------------------- expressions

    fn parse_match(&mut self) -> Result<Expr, ParseError> {
        self.expect_keyword("match")?;
        self.expect(&Tok::LParen)?;
        let subject = self.parse_expr()?;
        self.expect(&Tok::RParen)?;
        self.expect(&Tok::LBrace)?;

        let mut arms = Vec::new();
        loop {
            if self.eat(&Tok::RBrace) {
                break;
            }
            let mut conditions = Vec::new();
            if self.check_keyword("default") && self.peek_at(1) == &Tok::DoubleArrow {
                self.advance();
            } else {
                loop {
                    conditions.push(self.parse_expr()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
            }
            self.expect(&Tok::DoubleArrow)?;
            let body = self.parse_expr()?;
            arms.push(MatchArm { conditions, body });
            if self.eat(&Tok::Comma) {
                continue;
            }
            self.expect(&Tok::RBrace)?;
            break;
        }
        Ok(Expr::Match {
            subject: Box::new(subject),
            arms,
        })
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_bp(0)
    }

    fn parse_bp(&mut self, min_bp: u8) -> Result<Expr, ParseError> {
        self.enter()?;
        let result = self.parse_bp_inner(min_bp);
        self.leave();
        result
    }

    fn parse_bp_inner(&mut self, min_bp: u8) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_prefix()?;

        loop {
            // Postfix: array access.
            if self.eat(&Tok::LBracket) {
                if self.eat(&Tok::RBracket) {
                    lhs = Expr::Index {
                        base: Box::new(lhs),
                        index: None,
                    };
                } else {
                    let index = self.parse_expr()?;
                    self.expect(&Tok::RBracket)?;
                    lhs = Expr::Index {
                        base: Box::new(lhs),
                        index: Some(Box::new(index)),
                    };
                }
                continue;
            }
            if matches!(self.peek(), Tok::Inc | Tok::Dec) {
                let increment = matches!(self.peek(), Tok::Inc);
                self.advance();
                if !is_valid_target(&lhs) {
                    return Err(self.error("cannot increment/decrement a non-variable"));
                }
                lhs = Expr::IncDec {
                    target: Box::new(lhs),
                    increment,
                    prefix: false,
                };
                continue;
            }
            if self.eat(&Tok::LParen) {
                let args = self.parse_call_args()?;
                lhs = Expr::DynamicCall {
                    callee: Box::new(lhs),
                    args,
                };
                continue;
            }
            if matches!(self.peek(), Tok::Arrow) {
                return Err(self.unsupported("object property access (->)"));
            }
            if matches!(self.peek(), Tok::DoubleColon) {
                return Err(self.unsupported("class constants and static access (::)"));
            }

            // Assignment (right associative, lowest precedence after and/or/xor).
            if let Some(op) = assignment_op(self.peek()) {
                const ASSIGN_BP: u8 = 4;
                if ASSIGN_BP < min_bp {
                    break;
                }
                self.advance();
                if !is_valid_target(&lhs) {
                    return Err(self.error("left-hand side of assignment is not writable"));
                }
                let value = self.parse_bp(ASSIGN_BP)?;
                lhs = Expr::Assign {
                    target: Box::new(lhs),
                    op,
                    value: Box::new(value),
                };
                continue;
            }

            // Ternary (non-associative in PHP 8).
            if matches!(self.peek(), Tok::Question) {
                const TERNARY_BP: u8 = 5;
                if TERNARY_BP < min_bp {
                    break;
                }
                self.advance();
                if self.eat(&Tok::Colon) {
                    let else_ = self.parse_bp(TERNARY_BP)?;
                    lhs = Expr::Ternary {
                        cond: Box::new(lhs),
                        then_: None,
                        else_: Box::new(else_),
                    };
                } else {
                    let then_ = self.parse_expr()?;
                    self.expect(&Tok::Colon)?;
                    let else_ = self.parse_bp(TERNARY_BP)?;
                    lhs = Expr::Ternary {
                        cond: Box::new(lhs),
                        then_: Some(Box::new(then_)),
                        else_: Box::new(else_),
                    };
                }
                continue;
            }

            let Some((lbp, rbp, op)) = self.infix_op() else {
                break;
            };
            if lbp < min_bp {
                break;
            }
            self.advance();
            let rhs = self.parse_bp(rbp)?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }

        Ok(lhs)
    }

    fn infix_op(&self) -> Option<(u8, u8, BinOp)> {
        let (lbp, rbp, op) = match self.peek() {
            Tok::Pow => (19, 19, BinOp::Pow), // right associative
            Tok::Star => (17, 18, BinOp::Mul),
            Tok::Slash => (17, 18, BinOp::Div),
            Tok::Percent => (17, 18, BinOp::Mod),
            Tok::Plus => (16, 17, BinOp::Add),
            Tok::Minus => (16, 17, BinOp::Sub),
            // PHP 8 moved `.` below the shift operators.
            Tok::Shl => (15, 16, BinOp::Shl),
            Tok::Shr => (15, 16, BinOp::Shr),
            Tok::Dot => (14, 15, BinOp::Concat),
            Tok::Lt => (13, 14, BinOp::Lt),
            Tok::Le => (13, 14, BinOp::Le),
            Tok::Gt => (13, 14, BinOp::Gt),
            Tok::Ge => (13, 14, BinOp::Ge),
            Tok::Spaceship => (13, 14, BinOp::Spaceship),
            Tok::EqEq => (12, 13, BinOp::Eq),
            Tok::NotEq => (12, 13, BinOp::NotEq),
            Tok::Identical => (12, 13, BinOp::Identical),
            Tok::NotIdentical => (12, 13, BinOp::NotIdentical),
            Tok::BitAnd => (11, 12, BinOp::BitAnd),
            Tok::BitXor => (10, 11, BinOp::BitXor),
            Tok::BitOr => (9, 10, BinOp::BitOr),
            Tok::AndAnd => (8, 9, BinOp::LogicalAnd),
            Tok::OrOr => (7, 8, BinOp::LogicalOr),
            Tok::Coalesce => (6, 6, BinOp::Coalesce), // right associative
            Tok::Ident(name) => match name.to_ascii_lowercase().as_str() {
                "and" => (3, 4, BinOp::LogicalAnd),
                "xor" => (2, 3, BinOp::LogicalXor),
                "or" => (1, 2, BinOp::LogicalOr),
                _ => return None,
            },
            _ => return None,
        };
        Some((lbp, rbp, op))
    }

    fn parse_prefix(&mut self) -> Result<Expr, ParseError> {
        let line = self.line();
        match self.peek().clone() {
            Tok::Int(value) => {
                self.advance();
                Ok(Expr::Int(value))
            }
            Tok::Float(value) => {
                self.advance();
                Ok(Expr::Float(value))
            }
            Tok::Str(bytes) => {
                self.advance();
                Ok(Expr::Str(bytes))
            }
            Tok::Interp(parts) => {
                self.advance();
                Ok(self.interpolation(parts, line)?)
            }
            Tok::Var(name) => {
                self.advance();
                Ok(Expr::Var(name))
            }
            Tok::LParen => {
                self.advance();
                if let Tok::Ident(name) = self.peek().clone() {
                    if self.peek_at(1) == &Tok::RParen {
                        if let Some(kind) = cast_kind(&name) {
                            self.advance();
                            self.advance();
                            let expr = self.parse_bp(18)?;
                            return Ok(Expr::Cast {
                                kind,
                                expr: Box::new(expr),
                            });
                        }
                        if matches!(name.to_ascii_lowercase().as_str(), "object" | "unset") {
                            return Err(self.unsupported(&format!("the ({name}) cast")));
                        }
                    }
                }
                let expr = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                Ok(expr)
            }
            Tok::LBracket => self.parse_array_literal(&Tok::RBracket),
            Tok::Not => {
                self.advance();
                Ok(Expr::Unary {
                    op: UnOp::Not,
                    expr: Box::new(self.parse_bp(18)?),
                })
            }
            Tok::Minus => {
                self.advance();
                Ok(Expr::Unary {
                    op: UnOp::Neg,
                    expr: Box::new(self.parse_bp(18)?),
                })
            }
            Tok::Plus => {
                self.advance();
                Ok(Expr::Unary {
                    op: UnOp::Pos,
                    expr: Box::new(self.parse_bp(18)?),
                })
            }
            Tok::BitNot => {
                self.advance();
                Ok(Expr::Unary {
                    op: UnOp::BitNot,
                    expr: Box::new(self.parse_bp(18)?),
                })
            }
            Tok::Inc => {
                self.advance();
                let target = self.parse_bp(18)?;
                if !is_valid_target(&target) {
                    return Err(self.error("cannot increment a non-variable"));
                }
                Ok(Expr::IncDec {
                    target: Box::new(target),
                    increment: true,
                    prefix: true,
                })
            }
            Tok::Dec => {
                self.advance();
                let target = self.parse_bp(18)?;
                if !is_valid_target(&target) {
                    return Err(self.error("cannot decrement a non-variable"));
                }
                Ok(Expr::IncDec {
                    target: Box::new(target),
                    increment: false,
                    prefix: true,
                })
            }
            Tok::At => {
                // Error suppression has no effect in PHPCraft; parse the operand.
                self.advance();
                self.parse_bp(18)
            }
            Tok::Ident(name) => {
                let lower = name.to_ascii_lowercase();
                match lower.as_str() {
                    "true" => {
                        self.advance();
                        Ok(Expr::Bool(true))
                    }
                    "false" => {
                        self.advance();
                        Ok(Expr::Bool(false))
                    }
                    "null" => {
                        self.advance();
                        Ok(Expr::Null)
                    }
                    "array" if self.peek_at(1) == &Tok::LParen => {
                        self.advance();
                        self.advance();
                        self.parse_array_items(&Tok::RParen)
                    }
                    "isset" if self.peek_at(1) == &Tok::LParen => {
                        self.advance();
                        self.advance();
                        let mut args = Vec::new();
                        if !self.eat(&Tok::RParen) {
                            loop {
                                args.push(self.parse_expr()?);
                                if self.eat(&Tok::Comma) {
                                    continue;
                                }
                                self.expect(&Tok::RParen)?;
                                break;
                            }
                        }
                        Ok(Expr::Isset(args))
                    }
                    "empty" if self.peek_at(1) == &Tok::LParen => {
                        self.advance();
                        self.advance();
                        let expr = self.parse_expr()?;
                        self.expect(&Tok::RParen)?;
                        Ok(Expr::Empty(Box::new(expr)))
                    }
                    "list" => Err(self.unsupported("list() destructuring")),
                    "new" | "clone" => Err(self.unsupported("objects")),
                    "static"
                        if matches!(
                            self.peek_at(1),
                            Tok::Ident(next)
                                if next.eq_ignore_ascii_case("function")
                                    || next.eq_ignore_ascii_case("fn")
                        ) =>
                    {
                        self.advance();
                        let arrow = matches!(
                            self.peek(),
                            Tok::Ident(next) if next.eq_ignore_ascii_case("fn")
                        );
                        self.parse_closure_expr(arrow)
                    }
                    "function" => self.parse_closure_expr(false),
                    "fn" => self.parse_closure_expr(true),
                    "match" if self.peek_at(1) == &Tok::LParen => self.parse_match(),
                    "yield" => Err(self.unsupported("generators")),
                    "throw" => Err(self.unsupported("exceptions")),
                    "print" => Err(self.unsupported("the print expression; use echo")),
                    "exit" | "die" => Err(self.unsupported("exit/die")),
                    "include" | "include_once" | "require" | "require_once" => {
                        Err(self.unsupported("include/require"))
                    }
                    "instanceof" => Err(self.unsupported("instanceof")),
                    _ => {
                        self.advance();
                        if self.eat(&Tok::LParen) {
                            let args = self.parse_call_args()?;
                            Ok(Expr::Call { name, args })
                        } else {
                            Ok(Expr::Name(name))
                        }
                    }
                }
            }
            other => Err(self.error(format!("syntax error, unexpected {}", other.describe()))),
        }
    }

    fn interpolation(&mut self, parts: Vec<StrPart>, line: u32) -> Result<Expr, ParseError> {
        let mut out = Vec::with_capacity(parts.len());
        for part in parts {
            match part {
                StrPart::Lit(bytes) => out.push(InterpPart::Lit(bytes)),
                StrPart::Var(name) => out.push(InterpPart::Expr(Expr::Var(name))),
                StrPart::Dim(name, key) => {
                    let key_expr = match key {
                        DimKey::Int(value) => Expr::Int(value),
                        DimKey::Str(bytes) => Expr::Str(bytes),
                    };
                    out.push(InterpPart::Expr(Expr::Index {
                        base: Box::new(Expr::Var(name)),
                        index: Some(Box::new(key_expr)),
                    }));
                }
                StrPart::Code(code) => {
                    let expr = self.parse_embedded(&code, line)?;
                    out.push(InterpPart::Expr(expr));
                }
            }
        }
        Ok(Expr::Interp(out))
    }

    fn parse_embedded(&mut self, code: &[u8], line: u32) -> Result<Expr, ParseError> {
        let tokens = phpcraft_lexer::tokenize_fragment(code).map_err(|e| ParseError {
            message: e.message,
            line,
        })?;
        let mut parser = Parser::new(tokens);
        let expr = parser.parse_expr().map_err(|e| ParseError {
            message: e.message,
            line,
        })?;
        if !matches!(parser.peek(), Tok::Eof) {
            return Err(ParseError {
                message: format!(
                    "syntax error in string interpolation, unexpected {}",
                    parser.peek().describe()
                ),
                line,
            });
        }
        Ok(expr)
    }

    fn parse_array_literal(&mut self, close: &Tok) -> Result<Expr, ParseError> {
        self.advance(); // '[' or '('
        self.parse_array_items(close)
    }

    fn parse_array_items(&mut self, close: &Tok) -> Result<Expr, ParseError> {
        let mut items = Vec::new();
        loop {
            if std::mem::discriminant(self.peek()) == std::mem::discriminant(close) {
                self.advance();
                break;
            }
            if matches!(self.peek(), Tok::Eof) {
                return Err(self.error("syntax error, unexpected end of file"));
            }
            if matches!(self.peek(), Tok::Ellipsis) {
                return Err(self.unsupported("array spread (...$array)"));
            }
            if matches!(self.peek(), Tok::BitAnd) {
                return Err(self.unsupported("array items by reference (&$value)"));
            }
            let first = self.parse_expr()?;
            if self.eat(&Tok::DoubleArrow) {
                let value = self.parse_expr()?;
                items.push(ArrayItem {
                    key: Some(first),
                    value,
                });
            } else {
                items.push(ArrayItem {
                    key: None,
                    value: first,
                });
            }
            if self.eat(&Tok::Comma) {
                continue;
            }
            self.expect(close)?;
            break;
        }
        Ok(Expr::Array(items))
    }

    fn parse_call_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut args = Vec::new();
        if self.eat(&Tok::RParen) {
            return Ok(args);
        }
        loop {
            if self.eat(&Tok::RParen) {
                break;
            }
            if matches!(self.peek(), Tok::Ellipsis) {
                return Err(self.unsupported("argument unpacking (...$args)"));
            }
            if matches!(self.peek(), Tok::BitAnd) {
                return Err(self.unsupported("arguments by reference (&$arg)"));
            }
            // Named arguments (`name: value`) are recognised and rejected.
            if matches!(self.peek(), Tok::Ident(_)) && self.peek_at(1) == &Tok::Colon {
                return Err(self.unsupported("named arguments"));
            }
            args.push(self.parse_expr()?);
            if self.eat(&Tok::Comma) {
                continue;
            }
            self.expect(&Tok::RParen)?;
            break;
        }
        Ok(args)
    }
}

fn assignment_op(tok: &Tok) -> Option<Option<BinOp>> {
    Some(match tok {
        Tok::Assign => None,
        Tok::PlusEq => Some(BinOp::Add),
        Tok::MinusEq => Some(BinOp::Sub),
        Tok::MulEq => Some(BinOp::Mul),
        Tok::DivEq => Some(BinOp::Div),
        Tok::ModEq => Some(BinOp::Mod),
        Tok::PowEq => Some(BinOp::Pow),
        Tok::ConcatEq => Some(BinOp::Concat),
        Tok::AndEq => Some(BinOp::BitAnd),
        Tok::OrEq => Some(BinOp::BitOr),
        Tok::XorEq => Some(BinOp::BitXor),
        Tok::ShlEq => Some(BinOp::Shl),
        Tok::ShrEq => Some(BinOp::Shr),
        Tok::CoalesceEq => Some(BinOp::Coalesce),
        _ => return None,
    })
}

/// Can this expression appear on the left of an assignment?
fn is_valid_target(expr: &Expr) -> bool {
    match expr {
        Expr::Var(_) => true,
        Expr::Index { base, .. } => matches!(**base, Expr::Var(_) | Expr::Index { .. }),
        _ => false,
    }
}

/// Map `(int)`, `(integer)`, `(bool)`, ... to a cast kind.
fn cast_kind(name: &str) -> Option<CastKind> {
    match name.to_ascii_lowercase().as_str() {
        "int" | "integer" => Some(CastKind::Int),
        "float" | "double" | "real" => Some(CastKind::Float),
        "string" | "binary" => Some(CastKind::String),
        "bool" | "boolean" => Some(CastKind::Bool),
        "array" => Some(CastKind::Array),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_matches_php() {
        let expr = parse_expression_fragment(b"1 + 2 * 3").unwrap();
        match expr {
            Expr::Binary {
                op: BinOp::Add,
                rhs,
                ..
            } => {
                assert!(matches!(*rhs, Expr::Binary { op: BinOp::Mul, .. }))
            }
            other => panic!("unexpected {other:?}"),
        }

        // `-2 ** 2` is `-(2 ** 2)`.
        let expr = parse_expression_fragment(b"-2 ** 2").unwrap();
        match expr {
            Expr::Unary {
                op: UnOp::Neg,
                expr,
            } => {
                assert!(matches!(*expr, Expr::Binary { op: BinOp::Pow, .. }))
            }
            other => panic!("unexpected {other:?}"),
        }

        // PHP 8 moved `.` below `+`, so this is `1 . (2 + 3)`.
        let expr = parse_expression_fragment(b"1 . 2 + 3").unwrap();
        match expr {
            Expr::Binary {
                op: BinOp::Concat,
                rhs,
                ..
            } => {
                assert!(matches!(*rhs, Expr::Binary { op: BinOp::Add, .. }))
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_statements() {
        let src = br#"<?php
            function add($a, $b = 1) { return $a + $b; }
            $total = 0;
            foreach ([1, 2, 3] as $k => $v) {
                if ($v > 1) { $total += $v; } else { continue; }
            }
            while ($total < 100) { $total++; }
            echo add($total), PHP_EOL;
        "#;
        let program = parse_file(src).unwrap();
        assert!(program.stmts.iter().any(|s| matches!(s, Stmt::Function(_))));
        assert!(program
            .stmts
            .iter()
            .any(|s| matches!(s, Stmt::Foreach { .. })));
    }

    #[test]
    fn parses_interpolation_with_code() {
        let program = parse_file(br#"<?php echo "x={$a['k']}";"#).unwrap();
        match &program.stmts[0] {
            Stmt::Echo(exprs) => match &exprs[0] {
                Expr::Interp(parts) => {
                    assert!(matches!(parts[1], InterpPart::Expr(Expr::Index { .. })))
                }
                other => panic!("unexpected {other:?}"),
            },
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_closures_arrow_functions_and_dynamic_calls() {
        let program = parse_file(
            br#"<?php
                $f = function ($x) use ($y) { return $x + $y; };
                $g = fn($z) => $z * 2;
                $f(1);
            "#,
        )
        .unwrap();
        match &program.stmts[0] {
            Stmt::Expr(Expr::Assign { value, .. }) => {
                assert!(matches!(value.as_ref(), Expr::Closure(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
        match &program.stmts[1] {
            Stmt::Expr(Expr::Assign { value, .. }) => {
                assert!(matches!(value.as_ref(), Expr::Closure(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            &program.stmts[2],
            Stmt::Expr(Expr::DynamicCall { .. })
        ));
    }

    #[test]
    fn reports_unsupported_constructs() {
        let err = parse_file(b"<?php class Foo {}").unwrap_err();
        assert!(err.message.contains("class"));
    }
}
