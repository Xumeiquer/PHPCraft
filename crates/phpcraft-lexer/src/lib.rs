//! PHP tokenizer.
//!
//! Entry points:
//! * [`tokenize`] lexes a whole file, including `<?php`/`?>` tags and inline
//!   HTML. It is what the CLI uses.
//! * [`tokenize_fragment`] lexes PHP code without tags; used for the
//!   `{$expr}` parts of interpolated strings.

pub mod token;

pub use token::{DimKey, StrPart, Tok, Token};

use std::fmt;

#[derive(Debug, Clone)]
pub struct LexError {
    pub message: String,
    pub line: u32,
}

impl LexError {
    fn new(message: impl Into<String>, line: u32) -> Self {
        Self {
            message: message.into(),
            line,
        }
    }
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} on line {}", self.message, self.line)
    }
}

impl std::error::Error for LexError {}

/// Lex a complete PHP file (code plus inline HTML).
pub fn tokenize(source: &[u8]) -> Result<Vec<Token>, LexError> {
    Lexer::new(source, Mode::Html).run()
}

/// Lex a bare PHP fragment, as if it appeared inside `<?php ... ?>`.
pub fn tokenize_fragment(source: &[u8]) -> Result<Vec<Token>, LexError> {
    Lexer::new(source, Mode::Php).run()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Html,
    Php,
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    mode: Mode,
    tokens: Vec<Token>,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a [u8], mode: Mode) -> Self {
        Self {
            src,
            pos: 0,
            line: 1,
            mode,
            tokens: Vec::new(),
        }
    }

    fn run(mut self) -> Result<Vec<Token>, LexError> {
        loop {
            match self.mode {
                Mode::Html => {
                    if !self.scan_html()? {
                        break;
                    }
                }
                Mode::Php => {
                    if self.pos >= self.src.len() {
                        break;
                    }
                    self.scan_php_token()?;
                }
            }
        }
        self.tokens.push(Token {
            kind: Tok::Eof,
            line: self.line,
        });
        Ok(self.tokens)
    }

    // ---------------------------------------------------------------- helpers

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos + offset).copied()
    }

    fn starts_with(&self, needle: &[u8]) -> bool {
        self.src[self.pos..].starts_with(needle)
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
        }
        Some(b)
    }

    /// Advance over a slice, keeping line numbers correct.
    fn advance(&mut self, mut n: usize) {
        while n > 0 {
            if self.bump().is_none() {
                break;
            }
            n -= 1;
        }
    }

    fn advance_to(&mut self, target: usize) {
        let target = target.min(self.src.len());
        if target > self.pos {
            self.advance(target - self.pos);
        }
    }

    fn push(&mut self, kind: Tok, line: u32) {
        self.tokens.push(Token { kind, line });
    }

    // ------------------------------------------------------------- HTML mode

    /// Emit inline HTML until an open tag. Returns `false` at end of input.
    fn scan_html(&mut self) -> Result<bool, LexError> {
        if self.pos >= self.src.len() {
            return Ok(false);
        }
        let rest = &self.src[self.pos..];
        let mut idx = 0;
        while idx + 2 < rest.len() {
            if rest[idx] == b'<' && rest[idx + 1] == b'?' {
                let after = idx + 2;
                if rest[after..].starts_with(b"php")
                    && rest
                        .get(after + 3)
                        .map(|c| c.is_ascii_whitespace())
                        .unwrap_or(true)
                {
                    self.emit_html(&rest[..idx]);
                    self.advance(idx + 5);
                    self.mode = Mode::Php;
                    return Ok(true);
                }
                if rest[after] == b'=' {
                    self.emit_html(&rest[..idx]);
                    self.advance(idx + 3);
                    let line = self.line;
                    self.push(Tok::Ident("echo".to_string()), line);
                    self.mode = Mode::Php;
                    return Ok(true);
                }
            }
            idx += 1;
        }
        self.emit_html(rest);
        self.advance(rest.len());
        Ok(false)
    }

    fn emit_html(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let line = self.line;
        self.push(Tok::InlineHtml(bytes.to_vec()), line);
    }

    // -------------------------------------------------------------- PHP mode

    fn scan_php_token(&mut self) -> Result<(), LexError> {
        self.skip_trivia()?;
        if self.pos >= self.src.len() {
            return Ok(());
        }
        let line = self.line;
        if self.peek() == Some(b'?') && self.peek_at(1) == Some(b'>') {
            self.advance(2);
            self.mode = Mode::Html;
            // A close tag terminates the statement, but `echo 1; ?>` must not
            // produce two semicolons.
            if !matches!(self.tokens.last().map(|t| &t.kind), Some(Tok::Semi)) {
                self.push(Tok::Semi, line);
            }
            // PHP swallows a single newline immediately after `?>`.
            if self.peek() == Some(b'\r') && self.peek_at(1) == Some(b'\n') {
                self.advance(2);
            } else if self.peek() == Some(b'\n') {
                self.advance(1);
            }
            return Ok(());
        }
        let kind = match self.peek().expect("checked") {
            b'$' => self.lex_variable()?,
            b'\'' => self.lex_single_quoted()?,
            b'"' => self.lex_double_quoted()?,
            b'<' if self.starts_with(b"<<<") => self.lex_heredoc()?,
            b'0'..=b'9' => self.lex_number()?,
            b'.' if self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) => self.lex_number()?,
            b'A'..=b'Z' | b'a'..=b'z' | b'_' | 0x80..=0xff => self.lex_ident(),
            b'`' => {
                return Err(LexError::new(
                    "shell execution (backticks) is not supported",
                    line,
                ))
            }
            _ => self.lex_operator(line)?,
        };
        self.push(kind, line);
        Ok(())
    }

    fn skip_trivia(&mut self) -> Result<(), LexError> {
        loop {
            match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.bump();
                }
                Some(b'#') => {
                    if self.peek_at(1) == Some(b'[') {
                        return Err(LexError::new(
                            "attributes (#[...]) are not supported yet",
                            self.line,
                        ));
                    }
                    self.skip_line_comment();
                }
                Some(b'/') if self.peek_at(1) == Some(b'/') => {
                    self.skip_line_comment();
                }
                Some(b'/') if self.peek_at(1) == Some(b'*') => {
                    let start_line = self.line;
                    self.advance(2);
                    let mut closed = false;
                    while self.pos < self.src.len() {
                        if self.peek() == Some(b'*') && self.peek_at(1) == Some(b'/') {
                            self.advance(2);
                            closed = true;
                            break;
                        }
                        self.bump();
                    }
                    if !closed {
                        return Err(LexError::new("unterminated comment", start_line));
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn skip_line_comment(&mut self) {
        while let Some(c) = self.peek() {
            if c == b'\n' {
                return;
            }
            if c == b'?' && self.peek_at(1) == Some(b'>') {
                return;
            }
            self.bump();
        }
    }

    fn lex_ident(&mut self) -> Tok {
        let start = self.pos;
        while self.peek().is_some_and(is_ident_char) {
            self.bump();
        }
        let name = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
        Tok::Ident(name)
    }

    fn lex_variable(&mut self) -> Result<Tok, LexError> {
        let line = self.line;
        self.bump(); // '$'
        match self.peek() {
            Some(c) if is_ident_start(c) => {
                let start = self.pos;
                while self.peek().is_some_and(is_ident_char) {
                    self.bump();
                }
                let name = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                Ok(Tok::Var(name))
            }
            Some(b'{') => Err(LexError::new(
                "variable variables (${...}) are not supported",
                line,
            )),
            _ => Err(LexError::new("unexpected character \"$\"", line)),
        }
    }

    fn lex_number(&mut self) -> Result<Tok, LexError> {
        let line = self.line;
        let start = self.pos;

        // Radix prefixes.
        if self.peek() == Some(b'0') {
            if let Some(radix_byte) = self.peek_at(1) {
                let radix = match radix_byte {
                    b'x' | b'X' => Some(16),
                    b'b' | b'B' => Some(2),
                    b'o' | b'O' => Some(8),
                    _ => None,
                };
                if let Some(radix) = radix {
                    self.advance(2);
                    let digits_start = self.pos;
                    while self
                        .peek()
                        .is_some_and(|c| c == b'_' || (c as char).is_digit(radix))
                    {
                        self.bump();
                    }
                    if self.pos == digits_start {
                        return Err(LexError::new("invalid numeric literal", line));
                    }
                    let text: String = self.src[digits_start..self.pos]
                        .iter()
                        .filter(|c| **c != b'_')
                        .map(|c| *c as char)
                        .collect();
                    return Ok(match i128::from_str_radix(&text, radix) {
                        Ok(v) if v <= i64::MAX as i128 => Tok::Int(v as i64),
                        Ok(v) => Tok::Float(v as f64),
                        Err(_) => Tok::Float(f64::INFINITY),
                    });
                }
            }
        }

        let mut is_float = false;
        let mut saw_digit = false;
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == b'_') {
            if self.peek() != Some(b'_') {
                saw_digit = true;
            }
            self.bump();
        }
        if self.peek() == Some(b'.') {
            // `1.` is a float; `1..2` is not valid PHP anyway.
            is_float = true;
            self.bump();
            let mut frac_digits = 0;
            while self.peek().is_some_and(|c| c.is_ascii_digit() || c == b'_') {
                if self.peek() != Some(b'_') {
                    frac_digits += 1;
                }
                self.bump();
            }
            if frac_digits > 0 {
                saw_digit = true;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            let save = self.pos;
            let save_line = self.line;
            self.bump();
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.bump();
            }
            if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                is_float = true;
                while self.peek().is_some_and(|c| c.is_ascii_digit() || c == b'_') {
                    self.bump();
                }
            } else {
                self.pos = save;
                self.line = save_line;
            }
        }
        if !saw_digit {
            return Err(LexError::new("invalid numeric literal", line));
        }

        let text: String = self.src[start..self.pos]
            .iter()
            .filter(|c| **c != b'_')
            .map(|c| *c as char)
            .collect();
        if is_float {
            let normalized = normalize_float_text(&text);
            normalized
                .parse::<f64>()
                .map(Tok::Float)
                .map_err(|_| LexError::new(format!("invalid float literal \"{text}\""), line))
        } else {
            match text.parse::<i64>() {
                Ok(v) => Ok(Tok::Int(v)),
                Err(_) => text
                    .parse::<f64>()
                    .map(Tok::Float)
                    .map_err(|_| LexError::new("integer literal out of range", line)),
            }
        }
    }

    fn lex_single_quoted(&mut self) -> Result<Tok, LexError> {
        let line = self.line;
        self.bump(); // opening quote
        let mut out = Vec::new();
        loop {
            match self.bump() {
                None => return Err(LexError::new("unterminated string", line)),
                Some(b'\'') => break,
                Some(b'\\') => match self.bump() {
                    Some(b'\\') => out.push(b'\\'),
                    Some(b'\'') => out.push(b'\''),
                    Some(other) => {
                        out.push(b'\\');
                        out.push(other);
                    }
                    None => return Err(LexError::new("unterminated string", line)),
                },
                Some(other) => out.push(other),
            }
        }
        Ok(Tok::Str(out))
    }

    fn lex_double_quoted(&mut self) -> Result<Tok, LexError> {
        let line = self.line;
        self.bump(); // opening quote
        let content_start = self.pos;
        let close = find_interpolated_end(self.src, content_start)
            .ok_or_else(|| LexError::new("unterminated string", line))?;
        let content = &self.src[content_start..close];
        let parts = interpolate_bytes(content, line)?;
        self.advance_to(close + 1);
        Ok(finish_interpolation(parts))
    }

    fn lex_heredoc(&mut self) -> Result<Tok, LexError> {
        let start_line = self.line;
        self.advance(3); // '<<<'
        while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
            self.bump();
        }
        let (nowdoc, quote) = match self.peek() {
            Some(b'\'') => {
                self.bump();
                (true, Some(b'\''))
            }
            Some(b'"') => {
                self.bump();
                (false, Some(b'"'))
            }
            _ => (false, None),
        };
        let label_start = self.pos;
        while self.peek().is_some_and(is_ident_char) {
            self.bump();
        }
        if self.pos == label_start {
            return Err(LexError::new("invalid heredoc label", start_line));
        }
        let label = self.src[label_start..self.pos].to_vec();
        if let Some(quote) = quote {
            if self.peek() != Some(quote) {
                return Err(LexError::new("invalid heredoc label", start_line));
            }
            self.bump();
        }
        while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
            self.bump();
        }
        if self.peek() == Some(b'\r') {
            self.bump();
        }
        if self.peek() != Some(b'\n') {
            return Err(LexError::new(
                "heredoc label must be followed by a newline",
                start_line,
            ));
        }
        self.bump();

        let body_start = self.pos;
        let mut cursor = body_start;
        let mut terminator_start = None;
        let mut indent = 0usize;
        while cursor <= self.src.len() {
            let line_start = cursor;
            let mut line_end = cursor;
            while line_end < self.src.len() && self.src[line_end] != b'\n' {
                line_end += 1;
            }
            let mut raw_end = line_end;
            if raw_end > line_start && self.src[raw_end - 1] == b'\r' {
                raw_end -= 1;
            }
            let raw = &self.src[line_start..raw_end];
            let mut ind = 0;
            while ind < raw.len() && (raw[ind] == b' ' || raw[ind] == b'\t') {
                ind += 1;
            }
            let trimmed = &raw[ind..];
            if trimmed.starts_with(&label) {
                let rest = &trimmed[label.len()..];
                if rest.is_empty() || rest == b";" {
                    terminator_start = Some(line_start);
                    indent = ind;
                    break;
                }
            }
            if line_end >= self.src.len() {
                break;
            }
            cursor = line_end + 1;
        }

        let content_end =
            terminator_start.ok_or_else(|| LexError::new("unterminated heredoc", start_line))?;
        let mut content = self.src[body_start..content_end].to_vec();
        if content.last() == Some(&b'\n') {
            content.pop();
            if content.last() == Some(&b'\r') {
                content.pop();
            }
        }
        let content = strip_indent(content, indent);

        let result = if nowdoc {
            Tok::Str(content)
        } else {
            finish_interpolation(interpolate_bytes(&content, start_line)?)
        };
        // The `;` after the closing label is a real statement terminator, so
        // leave it for the token loop instead of consuming it here.
        let stop = terminator_start.expect("set with terminator") + indent + label.len();
        self.advance_to(stop);
        Ok(result)
    }

    fn lex_operator(&mut self, line: u32) -> Result<Tok, LexError> {
        let rest = &self.src[self.pos..];
        // Longest match first; `<<<` was handled before this point.
        let (len, kind) = if rest.starts_with(b"<<=") {
            (3, Tok::ShlEq)
        } else if rest.starts_with(b">>=") {
            (3, Tok::ShrEq)
        } else if rest.starts_with(b"**=") {
            (3, Tok::PowEq)
        } else if rest.starts_with(b"??=") {
            (3, Tok::CoalesceEq)
        } else if rest.starts_with(b"===") {
            (3, Tok::Identical)
        } else if rest.starts_with(b"!==") {
            (3, Tok::NotIdentical)
        } else if rest.starts_with(b"<=>") {
            (3, Tok::Spaceship)
        } else if rest.starts_with(b"...") {
            (3, Tok::Ellipsis)
        } else if rest.starts_with(b"==") {
            (2, Tok::EqEq)
        } else if rest.starts_with(b"!=") || rest.starts_with(b"<>") {
            (2, Tok::NotEq)
        } else if rest.starts_with(b"<=") {
            (2, Tok::Le)
        } else if rest.starts_with(b">=") {
            (2, Tok::Ge)
        } else if rest.starts_with(b"&&") {
            (2, Tok::AndAnd)
        } else if rest.starts_with(b"||") {
            (2, Tok::OrOr)
        } else if rest.starts_with(b"<<") {
            (2, Tok::Shl)
        } else if rest.starts_with(b">>") {
            (2, Tok::Shr)
        } else if rest.starts_with(b"**") {
            (2, Tok::Pow)
        } else if rest.starts_with(b"??") {
            (2, Tok::Coalesce)
        } else if rest.starts_with(b"->") {
            (2, Tok::Arrow)
        } else if rest.starts_with(b"=>") {
            (2, Tok::DoubleArrow)
        } else if rest.starts_with(b"::") {
            (2, Tok::DoubleColon)
        } else if rest.starts_with(b"++") {
            (2, Tok::Inc)
        } else if rest.starts_with(b"--") {
            (2, Tok::Dec)
        } else if rest.starts_with(b"+=") {
            (2, Tok::PlusEq)
        } else if rest.starts_with(b"-=") {
            (2, Tok::MinusEq)
        } else if rest.starts_with(b"*=") {
            (2, Tok::MulEq)
        } else if rest.starts_with(b"/=") {
            (2, Tok::DivEq)
        } else if rest.starts_with(b"%=") {
            (2, Tok::ModEq)
        } else if rest.starts_with(b".=") {
            (2, Tok::ConcatEq)
        } else if rest.starts_with(b"&=") {
            (2, Tok::AndEq)
        } else if rest.starts_with(b"|=") {
            (2, Tok::OrEq)
        } else if rest.starts_with(b"^=") {
            (2, Tok::XorEq)
        } else {
            let single = match rest.first().copied() {
                Some(b'(') => Tok::LParen,
                Some(b')') => Tok::RParen,
                Some(b'{') => Tok::LBrace,
                Some(b'}') => Tok::RBrace,
                Some(b'[') => Tok::LBracket,
                Some(b']') => Tok::RBracket,
                Some(b';') => Tok::Semi,
                Some(b',') => Tok::Comma,
                Some(b':') => Tok::Colon,
                Some(b'?') => Tok::Question,
                Some(b'@') => Tok::At,
                Some(b'\\') => Tok::Backslash,
                Some(b'.') => Tok::Dot,
                Some(b'+') => Tok::Plus,
                Some(b'-') => Tok::Minus,
                Some(b'*') => Tok::Star,
                Some(b'/') => Tok::Slash,
                Some(b'%') => Tok::Percent,
                Some(b'=') => Tok::Assign,
                Some(b'<') => Tok::Lt,
                Some(b'>') => Tok::Gt,
                Some(b'!') => Tok::Not,
                Some(b'&') => Tok::BitAnd,
                Some(b'|') => Tok::BitOr,
                Some(b'^') => Tok::BitXor,
                Some(b'~') => Tok::BitNot,
                Some(c) => {
                    return Err(LexError::new(
                        format!(
                            "unexpected character \"{}\"",
                            char::from(c).escape_default()
                        ),
                        line,
                    ))
                }
                None => return Err(LexError::new("unexpected end of input", line)),
            };
            (1, single)
        };
        self.advance(len);
        Ok(kind)
    }
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

fn normalize_float_text(text: &str) -> String {
    let mut out = text.to_string();
    if out.starts_with('.') {
        out.insert(0, '0');
    } else if out.starts_with("-.") || out.starts_with("+.") {
        out.insert(1, '0');
    }
    if out.ends_with('.') {
        out.push('0');
    }
    out
}

/// Find the closing quote of an interpolated string, skipping escapes and
/// `{$...}` blocks (which may contain quotes, e.g. `"{$a["k"]}"`).
fn find_interpolated_end(src: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i < src.len() {
        match src[i] {
            b'\\' => i += 2,
            b'"' => return Some(i),
            b'{' if src.get(i + 1) == Some(&b'$') => {
                i = find_braced_end(src, i)?;
            }
            _ => i += 1,
        }
    }
    None
}

/// Index just past the `}` matching the `{` at `i`.
fn find_braced_end(src: &[u8], i: usize) -> Option<usize> {
    let mut cursor = i + 1;
    let mut depth = 0usize;
    while cursor < src.len() {
        match src[cursor] {
            b'{' => {
                depth += 1;
                cursor += 1;
            }
            b'}' => {
                if depth == 0 {
                    return Some(cursor + 1);
                }
                depth -= 1;
                cursor += 1;
            }
            quote @ (b'\'' | b'"') => {
                let close = find_string_end(src, cursor + 1, quote)?;
                cursor = close + 1;
            }
            _ => cursor += 1,
        }
    }
    None
}

/// Find the closing quote, skipping backslash escapes.
fn find_string_end(src: &[u8], start: usize, quote: u8) -> Option<usize> {
    let mut i = start;
    while i < src.len() {
        match src[i] {
            b'\\' => i += 2,
            c if c == quote => return Some(i),
            _ => i += 1,
        }
    }
    None
}

fn finish_interpolation(parts: Vec<StrPart>) -> Tok {
    if parts.is_empty() {
        return Tok::Str(Vec::new());
    }
    if parts.len() == 1 {
        if let StrPart::Lit(bytes) = &parts[0] {
            return Tok::Str(bytes.clone());
        }
    }
    // Merge adjacent literals produced by escapes.
    let mut merged: Vec<StrPart> = Vec::with_capacity(parts.len());
    for part in parts {
        match (merged.last_mut(), part) {
            (Some(StrPart::Lit(prev)), StrPart::Lit(next)) => prev.extend_from_slice(&next),
            (_, part) => merged.push(part),
        }
    }
    Tok::Interp(merged)
}

/// Convert the inside of an interpolated string into parts.
fn interpolate_bytes(src: &[u8], line: u32) -> Result<Vec<StrPart>, LexError> {
    let mut parts = Vec::new();
    let mut lit: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'\\' => {
                i += 1;
                if i >= src.len() {
                    lit.push(b'\\');
                    break;
                }
                i = read_escape_at(src, i, &mut lit, line)?;
            }
            b'$' => {
                if let Some((part, next)) = interp_var_at(src, i) {
                    flush_lit(&mut parts, &mut lit);
                    parts.push(part);
                    i = next;
                } else {
                    lit.push(b'$');
                    i += 1;
                }
            }
            b'{' if src.get(i + 1) == Some(&b'$') => {
                let (code, next) = braced_code_at(src, i, line)?;
                flush_lit(&mut parts, &mut lit);
                parts.push(StrPart::Code(code));
                i = next;
            }
            c => {
                lit.push(c);
                i += 1;
            }
        }
    }
    flush_lit(&mut parts, &mut lit);
    Ok(parts)
}

fn flush_lit(parts: &mut Vec<StrPart>, lit: &mut Vec<u8>) {
    if !lit.is_empty() {
        parts.push(StrPart::Lit(std::mem::take(lit)));
    }
}

/// Handle one escape sequence; `i` points at the character after the backslash.
/// Returns the index of the next character to read.
fn read_escape_at(src: &[u8], i: usize, out: &mut Vec<u8>, line: u32) -> Result<usize, LexError> {
    let c = src[i];
    let mut i = i + 1;
    match c {
        b'n' => out.push(b'\n'),
        b'r' => out.push(b'\r'),
        b't' => out.push(b'\t'),
        b'v' => out.push(0x0b),
        b'e' => out.push(0x1b),
        b'f' => out.push(0x0c),
        b'\\' => out.push(b'\\'),
        b'$' => out.push(b'$'),
        b'"' => out.push(b'"'),
        b'`' => out.push(b'`'),
        b'0'..=b'7' => {
            let mut value = c - b'0';
            let mut count = 1;
            while count < 3 && i < src.len() && (b'0'..=b'7').contains(&src[i]) {
                value = value.wrapping_mul(8).wrapping_add(src[i] - b'0');
                i += 1;
                count += 1;
            }
            out.push(value);
        }
        b'x' => {
            let mut value = 0u8;
            let mut count = 0;
            while count < 2 && i < src.len() && src[i].is_ascii_hexdigit() {
                value = value * 16 + hex_value(src[i]);
                i += 1;
                count += 1;
            }
            if count == 0 {
                out.push(b'\\');
                out.push(b'x');
            } else {
                out.push(value);
            }
        }
        b'u' if src.get(i) == Some(&b'{') => {
            i += 1;
            let start = i;
            let mut value: u32 = 0;
            while i < src.len() && src[i] != b'}' {
                if !src[i].is_ascii_hexdigit() {
                    return Err(LexError::new("invalid UTF-8 escape sequence", line));
                }
                value = value
                    .checked_mul(16)
                    .and_then(|v| v.checked_add(hex_value(src[i]) as u32))
                    .ok_or_else(|| LexError::new("invalid UTF-8 escape sequence", line))?;
                i += 1;
            }
            if i == start || i >= src.len() {
                return Err(LexError::new("invalid UTF-8 escape sequence", line));
            }
            i += 1; // '}'
            let ch = char::from_u32(value)
                .ok_or_else(|| LexError::new("invalid UTF-8 code point", line))?;
            let mut buf = [0u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        }
        other => {
            out.push(b'\\');
            out.push(other);
        }
    }
    Ok(i)
}

fn hex_value(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

/// Parse `$name` or `$name[key]` inside an interpolated string.
fn interp_var_at(src: &[u8], i: usize) -> Option<(StrPart, usize)> {
    let mut j = i + 1;
    if j >= src.len() || !is_ident_start(src[j]) {
        return None;
    }
    let name_start = j;
    while j < src.len() && is_ident_char(src[j]) {
        j += 1;
    }
    let name = String::from_utf8_lossy(&src[name_start..j]).into_owned();
    if j < src.len() && src[j] == b'[' {
        if let Some(close) = src[j + 1..].iter().position(|c| *c == b']') {
            let close = j + 1 + close;
            let key = &src[j + 1..close];
            if let Some(dim) = simple_dim_key(key) {
                return Some((StrPart::Dim(name, dim), close + 1));
            }
        }
    }
    Some((StrPart::Var(name), j))
}

fn simple_dim_key(key: &[u8]) -> Option<DimKey> {
    if key.is_empty() {
        return None;
    }
    let negative = key[0] == b'-';
    let digits = if negative { &key[1..] } else { key };
    if !digits.is_empty() && digits.iter().all(|c| c.is_ascii_digit()) {
        let text = std::str::from_utf8(digits).ok()?;
        let value: i64 = text.parse().ok()?;
        return Some(DimKey::Int(if negative { -value } else { value }));
    }
    if is_ident_start(key[0]) && key.iter().all(|c| is_ident_char(*c)) {
        return Some(DimKey::Str(key.to_vec()));
    }
    if key.len() >= 2
        && ((key[0] == b'\'' && key[key.len() - 1] == b'\'')
            || (key[0] == b'"' && key[key.len() - 1] == b'"'))
    {
        let inner = &key[1..key.len() - 1];
        let decoded = if key[0] == b'\'' {
            let mut out = Vec::new();
            let mut i = 0;
            while i < inner.len() {
                if inner[i] == b'\\'
                    && i + 1 < inner.len()
                    && (inner[i + 1] == b'\\' || inner[i + 1] == b'\'')
                {
                    out.push(inner[i + 1]);
                    i += 2;
                } else {
                    out.push(inner[i]);
                    i += 1;
                }
            }
            out
        } else {
            let mut out = Vec::new();
            let mut i = 0;
            while i < inner.len() {
                if inner[i] == b'\\' && i + 1 < inner.len() {
                    i = read_escape_at(inner, i + 1, &mut out, 0).ok()?;
                } else {
                    out.push(inner[i]);
                    i += 1;
                }
            }
            out
        };
        return Some(DimKey::Str(decoded));
    }
    None
}

/// Extract the source of a `{$...}` interpolation, returning the inner bytes
/// and the index just past the closing brace.
fn braced_code_at(src: &[u8], i: usize, line: u32) -> Result<(Vec<u8>, usize), LexError> {
    let start = i + 1;
    let end = find_braced_end(src, i)
        .ok_or_else(|| LexError::new("unterminated {$...} interpolation", line))?;
    Ok((src[start..end - 1].to_vec(), end))
}

/// Remove up to `indent` leading whitespace characters from every line
/// (PHP 7.3 flexible heredoc/nowdoc indentation).
fn strip_indent(content: Vec<u8>, indent: usize) -> Vec<u8> {
    if indent == 0 {
        return content;
    }
    let mut out = Vec::with_capacity(content.len());
    let mut i = 0;
    while i < content.len() {
        let mut removed = 0;
        while removed < indent && i < content.len() && (content[i] == b' ' || content[i] == b'\t') {
            i += 1;
            removed += 1;
        }
        while i < content.len() {
            let b = content[i];
            out.push(b);
            i += 1;
            if b == b'\n' {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Tok> {
        tokenize(src.as_bytes())
            .expect("lexes")
            .into_iter()
            .map(|t| t.kind)
            .filter(|t| !matches!(t, Tok::Eof))
            .collect()
    }

    #[test]
    fn tags_and_inline_html() {
        let toks = kinds("hello <?php echo 1; ?> world");
        assert_eq!(toks[0], Tok::InlineHtml(b"hello ".to_vec()));
        assert_eq!(toks[1], Tok::Ident("echo".into()));
        assert_eq!(toks[2], Tok::Int(1));
        assert_eq!(toks[3], Tok::Semi);
        assert_eq!(toks[4], Tok::InlineHtml(b" world".to_vec()));
    }

    #[test]
    fn short_echo_tag() {
        let toks = kinds("<?= 1 ?>");
        assert_eq!(toks[0], Tok::Ident("echo".into()));
        assert_eq!(toks[1], Tok::Int(1));
    }

    #[test]
    fn interpolation_parts() {
        let toks = kinds(r#"<?php "a $b {$c['d']} \$e";"#);
        match &toks[0] {
            Tok::Interp(parts) => {
                assert!(parts
                    .iter()
                    .any(|p| matches!(p, StrPart::Var(v) if v == "b")));
                assert!(parts.iter().any(|p| matches!(p, StrPart::Code(_))));
            }
            other => panic!("expected interpolation, got {other:?}"),
        }
    }

    #[test]
    fn heredoc_and_nowdoc() {
        let src = "<?php $a = <<<EOT\n  hello $name\n  EOT;\n";
        let toks = kinds(src);
        match &toks[2] {
            Tok::Interp(parts) => {
                assert_eq!(parts[0], StrPart::Lit(b"hello ".to_vec()));
                assert_eq!(parts[1], StrPart::Var("name".into()));
            }
            other => panic!("expected heredoc interpolation, got {other:?}"),
        }

        let src = "<?php $a = <<<'EOT'\nraw $x\nEOT;\n";
        let toks = kinds(src);
        assert_eq!(toks[2], Tok::Str(b"raw $x".to_vec()));
    }

    #[test]
    fn numbers() {
        let toks = kinds("<?php 1_000 0xff 0b101 0o17 1.5 .5 1e3 0xFFFFFFFFFFFFFFFF;");
        assert_eq!(toks[0], Tok::Int(1000));
        assert_eq!(toks[1], Tok::Int(255));
        assert_eq!(toks[2], Tok::Int(5));
        assert_eq!(toks[3], Tok::Int(15));
        assert_eq!(toks[4], Tok::Float(1.5));
        assert_eq!(toks[5], Tok::Float(0.5));
        assert_eq!(toks[6], Tok::Float(1000.0));
        assert_eq!(toks[7], Tok::Float(u64::MAX as f64));
    }
}
