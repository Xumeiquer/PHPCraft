//! PHP parser.
//!
//! Statement parsing is recursive descent; expressions use a Pratt parser with
//! the PHP 8 operator precedence table (`zend_operators` / `zend_language_parser.y`).
//!
//! Unsupported constructs produce a parse error naming the feature, so the
//! boundary of the implementation is always explicit.

use phpcraft_ast::{
    ArrayItem, BinOp, Capture, CastKind, CatchClause, ClassConstDecl, ClassDecl, ClosureBody,
    ClosureDecl, Expr, FunctionDecl, InterpPart, MatchArm, MethodDecl, Param, Program,
    PropertyDecl, Stmt, SwitchCase, UnOp, UseDecl, UseKind, Visibility,
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

    fn peek_keyword_at(&self, offset: usize) -> Option<String> {
        match self.peek_at(offset) {
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
        if self.check_keyword("function")
            && (matches!(self.peek_at(1), Tok::Ident(_))
                || (matches!(self.peek_at(1), Tok::BitAnd)
                    && matches!(self.peek_at(2), Tok::Ident(_))))
        {
            return Ok(Stmt::Function(self.parse_function()?));
        }
        if self.check_keyword("class")
            || (self.check_keyword("final") && self.peek_keyword_at(1).as_deref() == Some("class"))
            || (self.check_keyword("abstract")
                && self.peek_keyword_at(1).as_deref() == Some("class"))
            || (self.check_keyword("readonly")
                && self.peek_keyword_at(1).as_deref() == Some("class"))
        {
            return Ok(Stmt::Class(self.parse_class()?));
        }
        if self.check_keyword("namespace") {
            return self.parse_namespace();
        }
        if self.check_keyword("use") {
            return self.parse_use();
        }
        if self.check_keyword("try") {
            return self.parse_try();
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
        if self.check_keyword("unset") {
            self.advance();
            self.expect(&Tok::LParen)?;
            let targets = self.parse_expr_list_until(&Tok::RParen)?;
            self.expect(&Tok::RParen)?;
            self.end_statement()?;
            return Ok(Stmt::Unset(targets));
        }

        for keyword in [
            "interface",
            "trait",
            "enum",
            "const",
            "static",
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
        let first_by_ref = self.eat(&Tok::BitAnd);
        let first = self.parse_expr()?;
        let (key, value, by_ref) = if self.eat(&Tok::DoubleArrow) {
            if first_by_ref {
                return Err(self.error("foreach key cannot be a reference"));
            }
            let by_ref = self.eat(&Tok::BitAnd);
            let value = Self::destructure_value(self.parse_expr()?);
            (Some(first), value, by_ref)
        } else {
            (None, Self::destructure_value(first), first_by_ref)
        };
        if let Some(key) = &key {
            if !is_valid_target(key) {
                return Err(self.error("foreach key must be a variable"));
            }
        }
        if !is_valid_target(&value) {
            return Err(self.error("foreach value must be a variable"));
        }
        if by_ref && !matches!(value, Expr::Var(_)) {
            return Err(self.unsupported("foreach by reference targets other than variables"));
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
            by_ref,
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
        let returns_ref = self.eat(&Tok::BitAnd);
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
            returns_ref,
            line,
        })
    }

    fn parse_namespace(&mut self) -> Result<Stmt, ParseError> {
        self.expect_keyword("namespace")?;
        if self.eat(&Tok::LBrace) {
            let body = self.parse_block_body()?;
            return Ok(Stmt::NamespaceBlock { name: None, body });
        }
        let name = self.parse_qualified_name()?;
        if self.eat(&Tok::LBrace) {
            let body = self.parse_block_body()?;
            return Ok(Stmt::NamespaceBlock {
                name: Some(name),
                body,
            });
        }
        self.end_statement()?;
        Ok(Stmt::NamespaceMark(Some(name)))
    }

    fn parse_use(&mut self) -> Result<Stmt, ParseError> {
        self.expect_keyword("use")?;
        let kind = if self.eat_keyword("function") {
            UseKind::Function
        } else if self.eat_keyword("const") {
            UseKind::Constant
        } else {
            UseKind::Class
        };

        // Parse the first name segment by segment so a trailing `\{` can be
        // recognised as group-use syntax.
        let mut prefix = String::new();
        if self.eat(&Tok::Backslash) {
            prefix.push('\\');
        }
        prefix.push_str(&self.expect_ident()?);
        while self.eat(&Tok::Backslash) {
            if self.eat(&Tok::LBrace) {
                let imports = self.parse_use_group(&prefix, kind)?;
                self.end_statement()?;
                return Ok(Stmt::Use(imports));
            }
            prefix.push('\\');
            prefix.push_str(&self.expect_ident()?);
        }

        let mut imports = Vec::new();
        let mut target = prefix;
        loop {
            target = target.trim_start_matches('\\').to_string();
            let alias = if self.eat_keyword("as") {
                self.expect_ident()?
            } else {
                target
                    .rsplit('\\')
                    .next()
                    .unwrap_or(target.as_str())
                    .to_string()
            };
            imports.push(UseDecl {
                target,
                alias,
                kind,
            });
            if !self.eat(&Tok::Comma) {
                break;
            }
            target = self.parse_qualified_name()?;
        }
        self.end_statement()?;
        Ok(Stmt::Use(imports))
    }

    /// Parse the `{...}` part of `use Prefix\{Name, function f, const C}`.
    /// Group items may override the statement's default import kind.
    fn parse_use_group(
        &mut self,
        prefix: &str,
        default_kind: UseKind,
    ) -> Result<Vec<UseDecl>, ParseError> {
        let prefix = prefix.trim_matches('\\');
        let mut imports = Vec::new();
        loop {
            if self.eat(&Tok::RBrace) {
                break;
            }
            let kind = if self.eat_keyword("function") {
                UseKind::Function
            } else if self.eat_keyword("const") {
                UseKind::Constant
            } else {
                default_kind
            };
            let item = self.parse_qualified_name()?;
            let item = item.trim_start_matches('\\');
            let target = if prefix.is_empty() {
                item.to_string()
            } else {
                format!("{prefix}\\{item}")
            };
            let alias = if self.eat_keyword("as") {
                self.expect_ident()?
            } else {
                item.rsplit('\\').next().unwrap_or(item).to_string()
            };
            imports.push(UseDecl {
                target,
                alias,
                kind,
            });
            if self.eat(&Tok::Comma) {
                continue;
            }
            self.expect(&Tok::RBrace)?;
            break;
        }
        Ok(imports)
    }

    fn parse_class(&mut self) -> Result<ClassDecl, ParseError> {
        let line = self.line();
        while matches!(
            self.peek_keyword().as_deref(),
            Some("final" | "abstract" | "readonly")
        ) {
            self.advance();
        }
        self.expect_keyword("class")?;
        let name = self.expect_ident()?;
        let parent = if self.eat_keyword("extends") {
            Some(self.parse_qualified_name()?)
        } else {
            None
        };
        if self.check_keyword("implements") {
            return Err(self.unsupported("interfaces (implements)"));
        }
        self.expect(&Tok::LBrace)?;

        let mut properties = Vec::new();
        let mut constants = Vec::new();
        let mut methods = Vec::new();
        while !matches!(self.peek(), Tok::RBrace | Tok::Eof) {
            if self.eat(&Tok::Semi) {
                continue;
            }
            let member_line = self.line();
            let (visibility, static_) = self.parse_member_modifiers()?;
            if self.check_keyword("function") {
                methods.push(self.parse_method(visibility, static_)?);
            } else if self.check_keyword("const") {
                self.advance();
                loop {
                    let name = self.expect_ident()?;
                    self.expect(&Tok::Assign)?;
                    let value = self.parse_expr()?;
                    constants.push(ClassConstDecl {
                        name,
                        value,
                        line: member_line,
                    });
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.end_statement()?;
            } else {
                // Typed properties are accepted; the type is currently only
                // metadata and is not enforced by the runtime.
                while !matches!(
                    self.peek(),
                    Tok::Var(_) | Tok::Semi | Tok::RBrace | Tok::Eof
                ) {
                    self.advance();
                }
                loop {
                    let name = self.expect_variable()?;
                    let default = if self.eat(&Tok::Assign) {
                        Some(self.parse_expr()?)
                    } else {
                        None
                    };
                    properties.push(PropertyDecl {
                        name,
                        default,
                        visibility,
                        static_,
                        line: member_line,
                    });
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.end_statement()?;
            }
        }
        self.expect(&Tok::RBrace)?;
        Ok(ClassDecl {
            name,
            parent,
            properties,
            constants,
            methods,
            line,
        })
    }

    fn parse_member_modifiers(&mut self) -> Result<(Visibility, bool), ParseError> {
        let mut visibility = Visibility::Public;
        let mut static_ = false;
        while let Some(keyword) = self.peek_keyword() {
            match keyword.as_str() {
                "public" => {
                    self.advance();
                    visibility = Visibility::Public;
                }
                "protected" => {
                    self.advance();
                    visibility = Visibility::Protected;
                }
                "private" => {
                    self.advance();
                    visibility = Visibility::Private;
                }
                "static" => {
                    self.advance();
                    static_ = true;
                }
                "var" => {
                    self.advance();
                }
                "final" | "abstract" | "readonly" => {
                    self.advance();
                }
                _ => break,
            }
        }
        Ok((visibility, static_))
    }

    fn parse_method(
        &mut self,
        visibility: Visibility,
        static_: bool,
    ) -> Result<MethodDecl, ParseError> {
        let line = self.line();
        self.expect_keyword("function")?;
        let returns_ref = self.eat(&Tok::BitAnd);
        let name = self.expect_ident()?;
        self.expect(&Tok::LParen)?;
        let params = self.parse_params()?;
        if self.eat(&Tok::Colon) {
            while !matches!(self.peek(), Tok::LBrace | Tok::Semi | Tok::Eof) {
                self.advance();
            }
        }
        if self.eat(&Tok::Semi) {
            return Err(self.unsupported("abstract/interface method declarations"));
        }
        self.expect(&Tok::LBrace)?;
        let body = self.parse_block_body()?;
        Ok(MethodDecl {
            name,
            params,
            body,
            visibility,
            static_,
            returns_ref,
            line,
        })
    }

    fn parse_qualified_name(&mut self) -> Result<String, ParseError> {
        let mut name = String::new();
        if self.eat(&Tok::Backslash) {
            name.push('\\');
        }
        name.push_str(&self.expect_ident()?);
        while self.eat(&Tok::Backslash) {
            name.push('\\');
            name.push_str(&self.expect_ident()?);
        }
        Ok(name)
    }

    fn parse_try(&mut self) -> Result<Stmt, ParseError> {
        self.expect_keyword("try")?;
        self.expect(&Tok::LBrace)?;
        let body = self.parse_block_body()?;
        let mut catches = Vec::new();
        while self.eat_keyword("catch") {
            let line = self.line();
            self.expect(&Tok::LParen)?;
            let mut classes = vec![self.parse_qualified_name()?];
            while self.eat(&Tok::BitOr) {
                classes.push(self.parse_qualified_name()?);
            }
            let variable = self.expect_variable()?;
            self.expect(&Tok::RParen)?;
            self.expect(&Tok::LBrace)?;
            let body = self.parse_block_body()?;
            catches.push(CatchClause {
                classes,
                variable,
                body,
                line,
            });
        }
        let finally = if self.eat_keyword("finally") {
            self.expect(&Tok::LBrace)?;
            Some(self.parse_block_body()?)
        } else {
            None
        };
        if catches.is_empty() && finally.is_none() {
            return Err(self.error(
                "syntax error, unexpected end of try block, expecting \"catch\" or \"finally\"",
            ));
        }
        Ok(Stmt::Try {
            body,
            catches,
            finally,
        })
    }

    fn parse_closure_expr(&mut self, arrow: bool) -> Result<Expr, ParseError> {
        let line = self.line();
        if arrow {
            self.expect_keyword("fn")?;
        } else {
            self.expect_keyword("function")?;
        }
        let returns_ref = self.eat(&Tok::BitAnd);
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
            returns_ref,
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
            let by_ref = self.eat(&Tok::BitAnd);
            let variadic = self.eat(&Tok::Ellipsis);
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
                by_ref,
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
                lhs = match self.parse_call_args()? {
                    CallArguments::List(args) => Expr::DynamicCall {
                        callee: Box::new(lhs),
                        args,
                    },
                    CallArguments::FirstClass => {
                        Expr::FirstClassCallable(Box::new(Expr::DynamicCall {
                            callee: Box::new(lhs),
                            args: Vec::new(),
                        }))
                    }
                };
                continue;
            }
            if self.eat(&Tok::Arrow) {
                let name = match self.peek().clone() {
                    Tok::Ident(name) => {
                        self.advance();
                        name
                    }
                    Tok::Var(_) => return Err(self.unsupported("dynamic property names")),
                    other => {
                        return Err(self.error(format!(
                            "syntax error, unexpected {}, expecting property name",
                            other.describe()
                        )))
                    }
                };
                lhs = if self.eat(&Tok::LParen) {
                    match self.parse_call_args()? {
                        CallArguments::List(args) => Expr::MethodCall {
                            object: Box::new(lhs),
                            method: name,
                            args,
                        },
                        CallArguments::FirstClass => {
                            Expr::FirstClassCallable(Box::new(Expr::MethodCall {
                                object: Box::new(lhs),
                                method: name,
                                args: Vec::new(),
                            }))
                        }
                    }
                } else {
                    Expr::Property {
                        object: Box::new(lhs),
                        name,
                    }
                };
                continue;
            }
            if self.eat(&Tok::DoubleColon) {
                let class = match &lhs {
                    Expr::Name(name) => name.clone(),
                    _ => return Err(self.unsupported("dynamic class names")),
                };
                if let Tok::Var(name) = self.peek().clone() {
                    self.advance();
                    lhs = Expr::StaticProperty { class, name };
                    continue;
                }
                let name = self.expect_ident()?;
                lhs = if self.eat(&Tok::LParen) {
                    match self.parse_call_args()? {
                        CallArguments::List(args) => Expr::StaticCall {
                            class,
                            method: name,
                            args,
                        },
                        CallArguments::FirstClass => {
                            Expr::FirstClassCallable(Box::new(Expr::StaticCall {
                                class,
                                method: name,
                                args: Vec::new(),
                            }))
                        }
                    }
                } else {
                    Expr::ClassConst { class, name }
                };
                continue;
            }
            if self.check_keyword("instanceof") {
                const INSTANCEOF_BP: u8 = 13;
                if INSTANCEOF_BP < min_bp {
                    break;
                }
                self.advance();
                let class = self.parse_qualified_name()?;
                lhs = Expr::InstanceOf {
                    expr: Box::new(lhs),
                    class,
                };
                continue;
            }

            // Assignment (right associative, lowest precedence after and/or/xor).
            if let Some(op) = assignment_op(self.peek()) {
                const ASSIGN_BP: u8 = 4;
                if ASSIGN_BP < min_bp {
                    break;
                }
                self.advance();
                lhs = match lhs {
                    Expr::Array(items) => {
                        Expr::Destructure(Self::array_items_into_destructure(items))
                    }
                    other => other,
                };
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
                        Ok(Expr::Array(self.parse_array_items(&Tok::RParen)?))
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
                    "list" if self.peek_at(1) == &Tok::LParen => {
                        self.advance();
                        self.advance();
                        let items = self.parse_array_items(&Tok::RParen)?;
                        Ok(Expr::Destructure(Self::array_items_into_destructure(items)))
                    }
                    "include" | "include_once" | "require" | "require_once" => {
                        let keyword = match self.peek().clone() {
                            Tok::Ident(name) => name.to_ascii_lowercase(),
                            _ => unreachable!(),
                        };
                        self.advance();
                        let path = self.parse_bp(0)?;
                        Ok(Expr::Include {
                            path: Box::new(path),
                            once: keyword.ends_with("_once"),
                            require: keyword.starts_with("require"),
                        })
                    }
                    "new" => {
                        self.advance();
                        if self.check_keyword("class") {
                            return Err(self.unsupported("anonymous classes"));
                        }
                        let class = self.parse_qualified_name()?;
                        let args = if self.eat(&Tok::LParen) {
                            match self.parse_call_args()? {
                                CallArguments::List(args) => args,
                                CallArguments::FirstClass => {
                                    return Err(
                                        self.error("Cannot create Closure for new expression")
                                    )
                                }
                            }
                        } else {
                            Vec::new()
                        };
                        Ok(Expr::New { class, args })
                    }
                    "clone" => {
                        self.advance();
                        Ok(Expr::Clone(Box::new(self.parse_bp(18)?)))
                    }
                    "throw" => {
                        self.advance();
                        Ok(Expr::Throw(Box::new(self.parse_bp(0)?)))
                    }
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
                    "static" if self.peek_at(1) == &Tok::DoubleColon => {
                        self.advance();
                        Ok(Expr::Name("static".to_string()))
                    }
                    "function" => self.parse_closure_expr(false),
                    "fn" => self.parse_closure_expr(true),
                    "match" if self.peek_at(1) == &Tok::LParen => self.parse_match(),
                    "yield" => Err(self.unsupported("generators")),
                    "print" => Err(self.unsupported("the print expression; use echo")),
                    "exit" | "die" => Err(self.unsupported("exit/die")),
                    "instanceof" => Err(self.unsupported("instanceof")),
                    "namespace" if self.peek_at(1) == &Tok::Backslash => {
                        self.advance();
                        let mut name = String::from("namespace");
                        while self.eat(&Tok::Backslash) {
                            name.push('\\');
                            name.push_str(&self.expect_ident()?);
                        }
                        if self.eat(&Tok::LParen) {
                            self.finish_function_call(name)
                        } else {
                            Ok(Expr::Name(name))
                        }
                    }
                    _ => {
                        self.advance();
                        let mut name = name;
                        while self.eat(&Tok::Backslash) {
                            name.push('\\');
                            name.push_str(&self.expect_ident()?);
                        }
                        if self.eat(&Tok::LParen) {
                            self.finish_function_call(name)
                        } else {
                            Ok(Expr::Name(name))
                        }
                    }
                }
            }
            Tok::Backslash => {
                let name = self.parse_qualified_name()?;
                if self.eat(&Tok::LParen) {
                    self.finish_function_call(name)
                } else {
                    Ok(Expr::Name(name))
                }
            }
            Tok::BitAnd => {
                self.advance();
                Ok(Expr::Reference(Box::new(self.parse_bp(18)?)))
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
        Ok(Expr::Array(self.parse_array_items(close)?))
    }

    /// Parse the comma-separated contents shared by array literals and
    /// destructuring targets. Empty entries and `&` are represented in the
    /// AST so a following `=` (or `foreach`) can turn the literal into a
    /// destructuring target; `compile_array` rejects them in value position.
    fn parse_array_items(&mut self, close: &Tok) -> Result<Vec<ArrayItem>, ParseError> {
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
            if self.eat(&Tok::Comma) {
                items.push(ArrayItem {
                    key: None,
                    value: None,
                    by_ref: false,
                });
                continue;
            }
            let by_ref = self.eat(&Tok::BitAnd);
            let first = self.parse_expr()?;
            if self.eat(&Tok::DoubleArrow) {
                if by_ref {
                    return Err(self.error("syntax error, unexpected token \"=>\""));
                }
                if matches!(self.peek(), Tok::Comma | Tok::RParen | Tok::RBracket) {
                    return Err(self.error("syntax error, unexpected token \",\""));
                }
                let by_ref = self.eat(&Tok::BitAnd);
                let value = self.parse_expr()?;
                items.push(ArrayItem {
                    key: Some(first),
                    value: Some(value),
                    by_ref,
                });
            } else {
                items.push(ArrayItem {
                    key: None,
                    value: Some(first),
                    by_ref,
                });
            }
            if self.eat(&Tok::Comma) {
                continue;
            }
            self.expect(close)?;
            break;
        }
        Ok(items)
    }

    /// Turn an array literal used as an assignment/foreach target into a
    /// destructuring node. An array literal nested inside another
    /// destructuring target is converted by `array_items_into_destructure`.
    fn destructure_value(expr: Expr) -> Expr {
        match expr {
            Expr::Array(items) => Expr::Destructure(Self::array_items_into_destructure(items)),
            other => other,
        }
    }

    /// Convert any array literals nested directly inside a destructuring
    /// target into destructuring nodes. Deeper array literals (for example an
    /// expression used as an index) are left untouched.
    fn array_items_into_destructure(items: Vec<ArrayItem>) -> Vec<ArrayItem> {
        items
            .into_iter()
            .map(|mut item| {
                if let Some(value) = item.value.take() {
                    item.value = Some(match value {
                        Expr::Array(inner) => {
                            Expr::Destructure(Self::array_items_into_destructure(inner))
                        }
                        other => other,
                    });
                }
                item
            })
            .collect()
    }

    /// Finish a named-function call after its `(` was consumed. `(...)` is
    /// first-class callable syntax rather than an empty argument list.
    fn finish_function_call(&mut self, name: String) -> Result<Expr, ParseError> {
        Ok(match self.parse_call_args()? {
            CallArguments::List(args) => Expr::Call { name, args },
            CallArguments::FirstClass => Expr::FirstClassCallable(Box::new(Expr::Call {
                name,
                args: Vec::new(),
            })),
        })
    }

    fn parse_call_args(&mut self) -> Result<CallArguments, ParseError> {
        let mut args = Vec::new();
        if self.eat(&Tok::RParen) {
            return Ok(CallArguments::List(args));
        }
        // `(...)` is the PHP 8.1 first-class callable syntax.
        if self.peek() == &Tok::Ellipsis && self.peek_at(1) == &Tok::RParen {
            self.advance();
            self.advance();
            return Ok(CallArguments::FirstClass);
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
        Ok(CallArguments::List(args))
    }
}

/// Arguments of a call: either an ordinary list or the `(...)` marker that
/// creates a first-class callable.
enum CallArguments {
    List(Vec<Expr>),
    FirstClass,
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
        Expr::Property { .. } | Expr::StaticProperty { .. } => true,
        Expr::Index { base, .. } => is_valid_target(base),
        Expr::Destructure(items) => items.iter().all(|item| match &item.value {
            None => true,
            Some(Expr::Destructure(_)) => !item.by_ref,
            Some(target) => is_valid_target(target),
        }),
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
    fn parses_first_class_callables() {
        let program = parse_file(
            br#"<?php
                $a = strlen(...);
                $b = $object->method(...);
                $c = Foo::bar(...);
                $d = $callable(...);
            "#,
        )
        .unwrap();
        let values: Vec<&Expr> = program
            .stmts
            .iter()
            .map(|stmt| match stmt {
                Stmt::Expr(Expr::Assign { value, .. }) => value.as_ref(),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        for value in &values {
            assert!(matches!(value, Expr::FirstClassCallable(_)));
        }
        match values[1] {
            Expr::FirstClassCallable(inner) => {
                assert!(matches!(inner.as_ref(), Expr::MethodCall { args, .. } if args.is_empty()))
            }
            other => panic!("unexpected {other:?}"),
        }
        match values[2] {
            Expr::FirstClassCallable(inner) => {
                assert!(matches!(inner.as_ref(), Expr::StaticCall { args, .. } if args.is_empty()))
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(parse_file(br#"<?php $f = new Foo(...);"#).is_err());
    }

    #[test]
    fn parses_destructuring_targets() {
        let program = parse_file(
            br#"<?php
                [$a, [, $b], [$d, [$e]]] = $source;
                list('k' => &$c, 'j' => list($f, $g)) = $source;
                foreach ($rows as $key => [&$h, list($i, $j)]) {}
            "#,
        )
        .unwrap();
        match &program.stmts[0] {
            Stmt::Expr(Expr::Assign { target, .. }) => match target.as_ref() {
                Expr::Destructure(items) => {
                    assert_eq!(items.len(), 3);
                    assert!(matches!(
                        items[0].value,
                        Some(Expr::Var(ref name)) if name == "a"
                    ));
                    assert!(matches!(
                        items[1].value,
                        Some(Expr::Destructure(ref inner))
                            if inner.len() == 2
                                && inner[0].value.is_none()
                                && matches!(inner[1].value, Some(Expr::Var(_)))
                    ));
                    assert!(matches!(items[2].value, Some(Expr::Destructure(_))));
                }
                other => panic!("unexpected {other:?}"),
            },
            other => panic!("unexpected {other:?}"),
        }
        match &program.stmts[1] {
            Stmt::Expr(Expr::Assign { target, .. }) => match target.as_ref() {
                Expr::Destructure(items) => {
                    assert!(items[0].by_ref);
                    assert!(matches!(items[0].key, Some(Expr::Str(_))));
                    assert!(matches!(items[1].value, Some(Expr::Destructure(_))));
                }
                other => panic!("unexpected {other:?}"),
            },
            other => panic!("unexpected {other:?}"),
        }
        match &program.stmts[2] {
            Stmt::Foreach { key, value, .. } => {
                assert!(matches!(key, Some(Expr::Var(_))));
                assert!(matches!(value, Expr::Destructure(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_namespaces_imports_and_includes() {
        let program = parse_file(
            br#"<?php
                namespace App\Util;
                use Foo\Bar as Baz;
                use function Foo\helper;
                use const Foo\VERSION;
                include __DIR__ . "/lib.php";
                require_once "other.php";
                namespace Other {
                    use App\Util\Baz;
                    echo Baz::class;
                }
            "#,
        )
        .unwrap();
        assert!(matches!(
            &program.stmts[0],
            Stmt::NamespaceMark(Some(name)) if name == "App\\Util"
        ));
        match &program.stmts[1] {
            Stmt::Use(imports) => {
                assert_eq!(imports.len(), 1);
                assert_eq!(imports[0].alias, "Baz");
                assert_eq!(imports[0].target, "Foo\\Bar");
            }
            other => panic!("unexpected {other:?}"),
        }
        match &program.stmts[4] {
            Stmt::Expr(Expr::Include { once, require, .. }) => {
                assert!(!once);
                assert!(!require);
            }
            other => panic!("unexpected {other:?}"),
        }
        match &program.stmts[5] {
            Stmt::Expr(Expr::Include { once, require, .. }) => {
                assert!(once);
                assert!(require);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            &program.stmts[6],
            Stmt::NamespaceBlock { name: Some(name), .. } if name == "Other"
        ));
    }

    #[test]
    fn parses_group_use_declarations() {
        let program = parse_file(
            br#"<?php
                use Foo\Bar\{Baz, Qux as Q, function helper as h, const VERSION};
                use function Foo\Funcs\{first, second as two};
                use const Foo\Values\{FIRST, SECOND as TWO};
            "#,
        )
        .unwrap();
        match &program.stmts[0] {
            Stmt::Use(imports) => {
                assert_eq!(imports.len(), 4);
                assert_eq!(imports[0].target, "Foo\\Bar\\Baz");
                assert_eq!(imports[0].alias, "Baz");
                assert_eq!(imports[0].kind, UseKind::Class);
                assert_eq!(imports[1].target, "Foo\\Bar\\Qux");
                assert_eq!(imports[1].alias, "Q");
                assert_eq!(imports[2].target, "Foo\\Bar\\helper");
                assert_eq!(imports[2].alias, "h");
                assert_eq!(imports[2].kind, UseKind::Function);
                assert_eq!(imports[3].target, "Foo\\Bar\\VERSION");
                assert_eq!(imports[3].alias, "VERSION");
                assert_eq!(imports[3].kind, UseKind::Constant);
            }
            other => panic!("unexpected {other:?}"),
        }
        match &program.stmts[1] {
            Stmt::Use(imports) => {
                assert_eq!(imports.len(), 2);
                assert_eq!(imports[0].target, "Foo\\Funcs\\first");
                assert_eq!(imports[0].kind, UseKind::Function);
                assert_eq!(imports[1].target, "Foo\\Funcs\\second");
                assert_eq!(imports[1].alias, "two");
                assert_eq!(imports[1].kind, UseKind::Function);
            }
            other => panic!("unexpected {other:?}"),
        }
        match &program.stmts[2] {
            Stmt::Use(imports) => {
                assert_eq!(imports.len(), 2);
                assert_eq!(imports[0].target, "Foo\\Values\\FIRST");
                assert_eq!(imports[0].kind, UseKind::Constant);
                assert_eq!(imports[1].target, "Foo\\Values\\SECOND");
                assert_eq!(imports[1].alias, "TWO");
                assert_eq!(imports[1].kind, UseKind::Constant);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_references_and_by_reference_parameters() {
        let program = parse_file(
            br#"<?php
                function f(&$x, $y) {
                    $x = &$y;
                }
                $fn = function () use (&$x) {
                    return $x;
                };
            "#,
        )
        .unwrap();
        match &program.stmts[0] {
            Stmt::Function(decl) => {
                assert!(decl.params[0].by_ref);
                assert!(!decl.params[1].by_ref);
                match &decl.body[0] {
                    Stmt::Expr(Expr::Assign { value, .. }) => {
                        assert!(matches!(value.as_ref(), Expr::Reference(_)));
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
            other => panic!("unexpected {other:?}"),
        }
        match &program.stmts[1] {
            Stmt::Expr(Expr::Assign { value, .. }) => match value.as_ref() {
                Expr::Closure(decl) => {
                    assert!(decl.captures[0].by_ref);
                    assert_eq!(decl.captures[0].name, "x");
                }
                other => panic!("unexpected {other:?}"),
            },
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_advanced_reference_forms() {
        let program = parse_file(
            br#"<?php
                function &collect(&...$values) {
                    unset($values[0]);
                    foreach ($values as $key => &$value) {
                        $value = $key;
                    }
                    return $values;
                }
                unset($a[0], $b);
            "#,
        )
        .unwrap();
        match &program.stmts[0] {
            Stmt::Function(decl) => {
                assert!(decl.returns_ref);
                assert!(decl.params[0].by_ref);
                assert!(decl.params[0].variadic);
                assert!(matches!(decl.body[0], Stmt::Unset(_)));
                match &decl.body[1] {
                    Stmt::Foreach { by_ref, value, .. } => {
                        assert!(by_ref);
                        assert!(matches!(value, Expr::Var(name) if name == "value"));
                    }
                    other => panic!("unexpected {other:?}"),
                }
                assert!(matches!(decl.body[2], Stmt::Return(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(&program.stmts[1], Stmt::Unset(targets) if targets.len() == 2));
    }

    #[test]
    fn reports_unsupported_constructs() {
        let err = parse_file(b"<?php interface Foo {}").unwrap_err();
        assert!(err.message.contains("interface"));
    }

    #[test]
    fn parses_classes_methods_and_exceptions() {
        let program = parse_file(
            br#"<?php
                class Foo extends Bar {
                    public int $x = 1;
                    private static $count = 0;
                    const KIND = "foo";
                    public function __construct($x) { $this->x = $x; }
                    public function get(): int { return $this->x; }
                }
                try { throw new Foo(2); }
                catch (Foo | Exception $e) { echo $e->getMessage(); }
                finally { echo "done"; }
            "#,
        )
        .unwrap();
        match &program.stmts[0] {
            Stmt::Class(class) => {
                assert_eq!(class.name, "Foo");
                assert_eq!(class.parent.as_deref(), Some("Bar"));
                assert_eq!(class.methods.len(), 2);
                assert_eq!(class.properties.len(), 2);
                assert_eq!(class.constants.len(), 1);
            }
            other => panic!("unexpected {other:?}"),
        }
        match &program.stmts[1] {
            Stmt::Try {
                catches, finally, ..
            } => {
                assert_eq!(catches.len(), 1);
                assert_eq!(catches[0].classes.len(), 2);
                assert!(finally.is_some());
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
