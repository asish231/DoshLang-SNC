use crate::span::Span;
use crate::token::{Token, TokenKind};

pub struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    file: u32,
}

impl<'a> Lexer<'a> {
    pub fn new(file: u32, src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            file,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            let eof = tok.kind == TokenKind::Eof;
            tokens.push(tok);
            if eof {
                break;
            }
        }
        Ok(tokens)
    }

    fn next_token(&mut self) -> Result<Token, String> {
        self.skip_ws_and_comments();
        let start = self.pos;
        if self.pos >= self.bytes.len() {
            return Ok(self.mk(TokenKind::Eof, start, start, ""));
        }
        let b = self.bytes[self.pos];
        match b {
            b'(' => {
                self.pos += 1;
                Ok(self.mk(TokenKind::LParen, start, self.pos, "("))
            }
            b')' => {
                self.pos += 1;
                Ok(self.mk(TokenKind::RParen, start, self.pos, ")"))
            }
            b'{' => {
                self.pos += 1;
                Ok(self.mk(TokenKind::LBrace, start, self.pos, "{"))
            }
            b'}' => {
                self.pos += 1;
                Ok(self.mk(TokenKind::RBrace, start, self.pos, "}"))
            }
            b'[' => {
                self.pos += 1;
                Ok(self.mk(TokenKind::LBracket, start, self.pos, "["))
            }
            b']' => {
                self.pos += 1;
                Ok(self.mk(TokenKind::RBracket, start, self.pos, "]"))
            }
            b',' => {
                self.pos += 1;
                Ok(self.mk(TokenKind::Comma, start, self.pos, ","))
            }
            b'.' => {
                self.pos += 1;
                Ok(self.mk(TokenKind::Dot, start, self.pos, "."))
            }
            b':' => {
                self.pos += 1;
                Ok(self.mk(TokenKind::Colon, start, self.pos, ":"))
            }
            b'?' => {
                self.pos += 1;
                Ok(self.mk(TokenKind::Question, start, self.pos, "?"))
            }
            b'+' => Ok(self.op2(start, b'+', TokenKind::Plus, TokenKind::PlusEq)),
            b'-' => {
                if self.peek_at(1) == Some(b'>') {
                    self.pos += 2;
                    return Ok(self.mk(TokenKind::Arrow, start, self.pos, "->"));
                }
                Ok(self.op2(start, b'-', TokenKind::Minus, TokenKind::MinusEq))
            }
            b'*' => {
                if self.peek_at(1) == Some(b'*') {
                    self.pos += 2;
                    return Ok(self.mk(TokenKind::StarStar, start, self.pos, "**"));
                }
                Ok(self.op2(start, b'*', TokenKind::Star, TokenKind::StarEq))
            }
            b'/' => Ok(self.op2(start, b'/', TokenKind::Slash, TokenKind::SlashEq)),
            b'%' => Ok(self.op2(start, b'%', TokenKind::Percent, TokenKind::PercentEq)),
            b'=' => {
                if self.peek_at(1) == Some(b'=') {
                    self.pos += 2;
                    Ok(self.mk(TokenKind::EqEq, start, self.pos, "=="))
                } else {
                    self.pos += 1;
                    Ok(self.mk(TokenKind::Eq, start, self.pos, "="))
                }
            }
            b'!' => {
                if self.peek_at(1) == Some(b'=') {
                    self.pos += 2;
                    Ok(self.mk(TokenKind::BangEq, start, self.pos, "!="))
                } else if self.peek_at(1) == Some(b'!') {
                    self.pos += 2;
                    Ok(self.mk(TokenKind::BangBang, start, self.pos, "!!"))
                } else {
                    Err(self.err(start, "unexpected '!'"))
                }
            }
            b'<' => {
                if self.peek_at(1) == Some(b'<') {
                    if self.peek_at(2) == Some(b'=') {
                        self.pos += 3;
                        return Ok(self.mk(TokenKind::ShlEq, start, self.pos, "<<="));
                    }
                    self.pos += 2;
                    return Ok(self.mk(TokenKind::Shl, start, self.pos, "<<"));
                }
                if self.peek_at(1) == Some(b'=') {
                    self.pos += 2;
                    Ok(self.mk(TokenKind::LtEq, start, self.pos, "<="))
                } else {
                    self.pos += 1;
                    Ok(self.mk(TokenKind::Lt, start, self.pos, "<"))
                }
            }
            b'>' => {
                if self.peek_at(1) == Some(b'>') {
                    if self.peek_at(2) == Some(b'=') {
                        self.pos += 3;
                        return Ok(self.mk(TokenKind::ShrEq, start, self.pos, ">>="));
                    }
                    self.pos += 2;
                    return Ok(self.mk(TokenKind::Shr, start, self.pos, ">>"));
                }
                if self.peek_at(1) == Some(b'=') {
                    self.pos += 2;
                    Ok(self.mk(TokenKind::GtEq, start, self.pos, ">="))
                } else {
                    self.pos += 1;
                    Ok(self.mk(TokenKind::Gt, start, self.pos, ">"))
                }
            }
            b'&' => {
                if self.peek_at(1) == Some(b'=') {
                    self.pos += 2;
                    Ok(self.mk(TokenKind::AmpEq, start, self.pos, "&="))
                } else {
                    self.pos += 1;
                    Ok(self.mk(TokenKind::Amp, start, self.pos, "&"))
                }
            }
            b'|' => {
                if self.peek_at(1) == Some(b'=') {
                    self.pos += 2;
                    Ok(self.mk(TokenKind::PipeEq, start, self.pos, "|="))
                } else {
                    self.pos += 1;
                    Ok(self.mk(TokenKind::Pipe, start, self.pos, "|"))
                }
            }
            b'^' => {
                if self.peek_at(1) == Some(b'=') {
                    self.pos += 2;
                    Ok(self.mk(TokenKind::CaretEq, start, self.pos, "^="))
                } else {
                    self.pos += 1;
                    Ok(self.mk(TokenKind::Caret, start, self.pos, "^"))
                }
            }
            b'~' => {
                self.pos += 1;
                Ok(self.mk(TokenKind::Tilde, start, self.pos, "~"))
            }
            b'"' => self.lex_string(start),
            b'0'..=b'9' => self.lex_number(start),
            b'A'..=b'Z' | b'a'..=b'z' | b'_' => Ok(self.lex_ident(start)),
            _ => Err(self.err(start, format!("unexpected character '{}'", b as char))),
        }
    }

    fn op2(&mut self, start: usize, _ch: u8, base: TokenKind, eq: TokenKind) -> Token {
        if self.peek_at(1) == Some(b'=') {
            self.pos += 2;
            let text = &self.src[start..self.pos];
            self.mk(eq, start, self.pos, text)
        } else {
            self.pos += 1;
            let text = &self.src[start..self.pos];
            self.mk(base, start, self.pos, text)
        }
    }

    fn lex_ident(&mut self, start: usize) -> Token {
        self.pos += 1;
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            if c.is_ascii_alphanumeric() || c == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = &self.src[start..self.pos];
        let kind = keyword(text);
        self.mk(kind, start, self.pos, text)
    }

    fn lex_number(&mut self, start: usize) -> Result<Token, String> {
        if self.bytes[self.pos] == b'0'
            && self.peek_at(1).map(|c| c == b'x' || c == b'X').unwrap_or(false)
        {
            self.pos += 2;
            let hex_start = self.pos;
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_hexdigit() {
                self.pos += 1;
            }
            if self.pos == hex_start {
                return Err(self.err(start, "invalid hex literal"));
            }
            let text = &self.src[start..self.pos];
            let val = i64::from_str_radix(&self.src[hex_start..self.pos], 16)
                .map_err(|_| self.err(start, "hex literal out of range"))?;
            let mut tok = self.mk(TokenKind::Int, start, self.pos, text);
            tok.int = val;
            return Ok(tok);
        }
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.pos < self.bytes.len()
            && self.bytes[self.pos] == b'.'
            && self.peek_at(1).map(|c| c.is_ascii_digit()).unwrap_or(false)
        {
            self.pos += 1;
            let frac_start = self.pos;
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            let scale = (self.pos - frac_start) as u32;
            // Float literal: trailing 'f' / 'F' => e.g. 3.14f . Otherwise dec.
            if self.pos < self.bytes.len()
                && (self.bytes[self.pos] == b'f' || self.bytes[self.pos] == b'F')
            {
                let text = &self.src[start..self.pos];
                let val: f64 = text
                    .parse()
                    .map_err(|_| self.err(start, "float literal out of range"))?;
                self.pos += 1;
                let mut tok = self.mk(TokenKind::FloatLit, start, self.pos, text);
                tok.float_val = val;
                return Ok(tok);
            }
            let text = &self.src[start..self.pos];
            let digits: String = text.chars().filter(|c| *c != '.').collect();
            let scaled: i64 = digits
                .parse()
                .map_err(|_| self.err(start, "decimal literal out of range"))?;
            let mut tok = self.mk(TokenKind::Dec, start, self.pos, text);
            tok.int = scaled;
            tok.scale = scale;
            return Ok(tok);
        }
        let text = &self.src[start..self.pos];
        let val: i64 = text
            .parse()
            .map_err(|_| self.err(start, "integer literal out of range"))?;
        let mut tok = self.mk(TokenKind::Int, start, self.pos, text);
        tok.int = val;
        Ok(tok)
    }

    fn lex_string(&mut self, start: usize) -> Result<Token, String> {
        self.pos += 1;
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            if c == b'"' {
                self.pos += 1;
                let mut tok = self.mk(TokenKind::String, start, self.pos, &out);
                tok.text = out;
                return Ok(tok);
            }
            if c == b'\\' {
                self.pos += 1;
                if self.pos >= self.bytes.len() {
                    return Err(self.err(start, "unterminated string"));
                }
                let e = self.bytes[self.pos];
                out.push(match e {
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    b'\\' => '\\',
                    b'"' => '"',
                    b'{' => '{',
                    b'}' => '}',
                    other => other as char,
                });
                self.pos += 1;
                continue;
            }
            if c == b'\n' {
                return Err(self.err(start, "unterminated string"));
            }
            out.push(c as char);
            self.pos += 1;
        }
        Err(self.err(start, "unterminated string"))
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.pos + 1 < self.bytes.len()
                && self.bytes[self.pos] == b'/'
                && self.bytes[self.pos + 1] == b'/'
            {
                self.pos += 2;
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }
            if self.pos + 1 < self.bytes.len()
                && self.bytes[self.pos] == b'/'
                && self.bytes[self.pos + 1] == b'*'
            {
                self.pos += 2;
                while self.pos + 1 < self.bytes.len()
                    && !(self.bytes[self.pos] == b'*' && self.bytes[self.pos + 1] == b'/')
                {
                    self.pos += 1;
                }
                if self.pos + 1 < self.bytes.len() {
                    self.pos += 2;
                }
                continue;
            }
            break;
        }
    }

    fn peek_at(&self, off: usize) -> Option<u8> {
        self.bytes.get(self.pos + off).copied()
    }

    fn mk(&self, kind: TokenKind, start: usize, end: usize, text: &str) -> Token {
        Token {
            kind,
            span: Span::new(self.file, start, end),
            text: text.to_string(),
            int: 0,
            scale: 0,
            float_val: 0.0,
        }
    }

    fn err(&self, start: usize, msg: impl Into<String>) -> String {
        format!("{} at offset {}", msg.into(), start)
    }
}

fn keyword(text: &str) -> TokenKind {
    match text {
        "fn" => TokenKind::Fn,
        "int" => TokenKind::IntKw,
        "i8" => TokenKind::I8Kw,
        "i16" => TokenKind::I16Kw,
        "i32" => TokenKind::I32Kw,
        "u8" => TokenKind::U8Kw,
        "u16" => TokenKind::U16Kw,
        "u32" => TokenKind::U32Kw,
        "u64" => TokenKind::U64Kw,
        "float" => TokenKind::FloatKw,
        "str" => TokenKind::StrKw,
        "bool" => TokenKind::BoolKw,
        "byte" => TokenKind::ByteKw,
        "enum" => TokenKind::EnumKw,
        "packed" => TokenKind::PackedKw,
        "dec" => TokenKind::DecKw,
        "list" => TokenKind::ListKw,
        "map" => TokenKind::MapKw,
        "const" => TokenKind::Const,
        "let" => TokenKind::Let,
        "if" => TokenKind::If,
        "else" => TokenKind::Else,
        "while" => TokenKind::While,
        "for" => TokenKind::For,
        "in" => TokenKind::In,
        "match" => TokenKind::Match,
        "default" => TokenKind::Default,
        "stop" => TokenKind::Stop,
        "skip" => TokenKind::Skip,
        "return" => TokenKind::Return,
        "use" => TokenKind::Use,
        "and" => TokenKind::And,
        "or" => TokenKind::Or,
        "not" => TokenKind::Not,
        "true" => TokenKind::True,
        "false" => TokenKind::False,
        "none" => TokenKind::None,
        "otherwise" => TokenKind::Otherwise,
        "blueprint" => TokenKind::Blueprint,
        "contract" => TokenKind::Contract,
        "follows" => TokenKind::Follows,
        "from" => TokenKind::From,
        "new" => TokenKind::New,
        "self" => TokenKind::SelfKw,
        "open" => TokenKind::Open,
        "closed" => TokenKind::Closed,
        "guarded" => TokenKind::Guarded,
        "spawn" => TokenKind::Spawn,
        "chan" => TokenKind::Chan,
        "async" => TokenKind::Async,
        "await" => TokenKind::Await,
        "lock" => TokenKind::Lock,
        "error" => TokenKind::ErrorKw,
        "ref" => TokenKind::RefKw,
        "defer" => TokenKind::Defer,
        "try" => TokenKind::TryKw,
        "record" => TokenKind::Record,
        "extension" => TokenKind::Extension,
        "any" => TokenKind::AnyKw,
        "super" => TokenKind::Super,
        "goroutine" => TokenKind::Goroutine,
        "asm" => TokenKind::Asm,
        "static" => TokenKind::Static,
        "abstract" => TokenKind::Abstract,
        "extern" => TokenKind::Extern,
        "ptr" => TokenKind::Ptr,
        "mut" => TokenKind::Mut,
        _ => TokenKind::Ident,
    }
}
