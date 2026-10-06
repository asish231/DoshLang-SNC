use super::{escape_sn_string, indent};

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Int(i64),
    Str(String),
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBrack,
    RBrack,
    Comma,
    Semi,
    Eq,
    EqEq,
    EqEqEq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    AmpAmp,
    PipePipe,
    Bang,
    Dot,
    Eof,
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
        }
    }
    fn tokenize(&mut self) -> Result<Vec<Tok>, String> {
        let mut t = Vec::new();
        loop {
            let x = self.next()?;
            let eof = x == Tok::Eof;
            t.push(x);
            if eof {
                break;
            }
        }
        Ok(t)
    }
    fn next(&mut self) -> Result<Tok, String> {
        self.skip();
        if self.pos >= self.bytes.len() {
            return Ok(Tok::Eof);
        }
        let b = self.bytes[self.pos];
        let two = |a: u8, b: u8| self.bytes.get(self.pos + 1) == Some(&b) && a == self.bytes[self.pos];
        match b {
            b'(' => {
                self.pos += 1;
                Ok(Tok::LParen)
            }
            b')' => {
                self.pos += 1;
                Ok(Tok::RParen)
            }
            b'{' => {
                self.pos += 1;
                Ok(Tok::LBrace)
            }
            b'}' => {
                self.pos += 1;
                Ok(Tok::RBrace)
            }
            b'[' => {
                self.pos += 1;
                Ok(Tok::LBrack)
            }
            b']' => {
                self.pos += 1;
                Ok(Tok::RBrack)
            }
            b',' => {
                self.pos += 1;
                Ok(Tok::Comma)
            }
            b';' => {
                self.pos += 1;
                Ok(Tok::Semi)
            }
            b'.' => {
                self.pos += 1;
                Ok(Tok::Dot)
            }
            b'+' => {
                self.pos += 1;
                Ok(Tok::Plus)
            }
            b'-' => {
                self.pos += 1;
                Ok(Tok::Minus)
            }
            b'*' => {
                self.pos += 1;
                Ok(Tok::Star)
            }
            b'/' => {
                self.pos += 1;
                Ok(Tok::Slash)
            }
            b'%' => {
                self.pos += 1;
                Ok(Tok::Percent)
            }
            b'=' => {
                if two(b'=', b'=') {
                    if self.bytes.get(self.pos + 2) == Some(&b'=') {
                        self.pos += 3;
                        return Ok(Tok::EqEqEq);
                    }
                    self.pos += 2;
                    Ok(Tok::EqEq)
                } else {
                    self.pos += 1;
                    Ok(Tok::Eq)
                }
            }
            b'!' => {
                if two(b'!', b'=') {
                    self.pos += 2;
                    Ok(Tok::NotEq)
                } else {
                    self.pos += 1;
                    Ok(Tok::Bang)
                }
            }
            b'<' => {
                if two(b'<', b'=') {
                    self.pos += 2;
                    Ok(Tok::LtEq)
                } else {
                    self.pos += 1;
                    Ok(Tok::Lt)
                }
            }
            b'>' => {
                if two(b'>', b'=') {
                    self.pos += 2;
                    Ok(Tok::GtEq)
                } else {
                    self.pos += 1;
                    Ok(Tok::Gt)
                }
            }
            b'&' if two(b'&', b'&') => {
                self.pos += 2;
                Ok(Tok::AmpAmp)
            }
            b'|' if two(b'|', b'|') => {
                self.pos += 2;
                Ok(Tok::PipePipe)
            }
            b'"' | b'\'' => self.lex_string(),
            b'0'..=b'9' => {
                let s = self.pos;
                while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
                Ok(Tok::Int(self.src[s..self.pos].parse().unwrap_or(0)))
            }
            b'A'..=b'Z' | b'a'..=b'z' | b'_' => {
                let s = self.pos;
                self.pos += 1;
                while self.pos < self.bytes.len() {
                    let c = self.bytes[self.pos];
                    if c.is_ascii_alphanumeric() || c == b'_' {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                Ok(Tok::Ident(self.src[s..self.pos].to_string()))
            }
            other => Err(format!("unsupported JS character {:?}", other as char)),
        }
    }
    fn lex_string(&mut self) -> Result<Tok, String> {
        let q = self.bytes[self.pos];
        self.pos += 1;
        let mut o = String::new();
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            if c == q {
                self.pos += 1;
                return Ok(Tok::Str(o));
            }
            if c == b'\\' {
                self.pos += 1;
                let e = self.bytes[self.pos];
                o.push(match e {
                    b'n' => '\n',
                    b't' => '\t',
                    other => other as char,
                });
                self.pos += 1;
                continue;
            }
            o.push(c as char);
            self.pos += 1;
        }
        Err("unterminated string".into())
    }
    fn skip(&mut self) {
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
            break;
        }
    }
}

struct P {
    toks: Vec<Tok>,
    pos: usize,
}

impl P {
    fn parse(src: &str) -> Result<String, String> {
        let toks = Lexer::new(src).tokenize()?;
        let mut p = P { toks, pos: 0 };
        let mut items = Vec::new();
        let mut saw_main = false;
        while !p.eof() {
            if p.is("function") {
                let f = p.parse_fn()?;
                if f.contains("fn main(") {
                    saw_main = true;
                }
                items.push(f);
            } else {
                items.push(p.parse_stmt()?);
            }
        }
        let mut body = items.join("\n");
        if !saw_main {
            let top: Vec<_> = items
                .iter()
                .filter(|s| !s.trim_start().starts_with("fn "))
                .cloned()
                .collect();
            let fns: Vec<_> = items
                .iter()
                .filter(|s| s.trim_start().starts_with("fn "))
                .cloned()
                .collect();
            body = fns.join("\n\n");
            if !body.is_empty() {
                body.push_str("\n\n");
            }
            body.push_str("fn main() {\n");
            body.push_str(&indent(&top.join("\n"), 1));
            body.push_str("\n}\n");
        }
        Ok(body)
    }

    fn parse_fn(&mut self) -> Result<String, String> {
        self.expect_ident("function")?;
        let name = self.name()?;
        self.eat(&Tok::LParen)?;
        let mut ps = Vec::new();
        if !self.at(&Tok::RParen) {
            loop {
                ps.push(format!("int {}", self.name()?));
                if !self.eat_if(&Tok::Comma) {
                    break;
                }
            }
        }
        self.eat(&Tok::RParen)?;
        let body = self.parse_block()?;
        let ret = if body.lines().any(|l| l.trim().starts_with("return ")) {
            " -> int"
        } else {
            ""
        };
        Ok(format!("fn {name}({}){ret} {{\n{}\n}}", ps.join(", "), indent(&body, 1)))
    }

    fn parse_block(&mut self) -> Result<String, String> {
        self.eat(&Tok::LBrace)?;
        let mut s = Vec::new();
        while !self.at(&Tok::RBrace) && !self.eof() {
            s.push(self.parse_stmt()?);
        }
        self.eat(&Tok::RBrace)?;
        Ok(s.join("\n"))
    }

    fn parse_stmt(&mut self) -> Result<String, String> {
        if self.is("if") {
            self.bump();
            self.eat(&Tok::LParen)?;
            let c = self.parse_expr()?;
            self.eat(&Tok::RParen)?;
            let t = if self.at(&Tok::LBrace) {
                self.parse_block()?
            } else {
                self.parse_stmt()?
            };
            let mut o = format!("if ({c}) {{\n{}\n}}", indent(&t, 1));
            if self.is("else") {
                self.bump();
                let e = if self.at(&Tok::LBrace) {
                    self.parse_block()?
                } else {
                    self.parse_stmt()?
                };
                o.push_str(&format!(" else {{\n{}\n}}", indent(&e, 1)));
            }
            return Ok(o);
        }
        if self.is("while") {
            self.bump();
            self.eat(&Tok::LParen)?;
            let c = self.parse_expr()?;
            self.eat(&Tok::RParen)?;
            let b = self.parse_block()?;
            return Ok(format!("while ({c}) {{\n{}\n}}", indent(&b, 1)));
        }
        if self.is("for") {
            return Err("JS C-style `for(;;)` is unsupported; use `while` or Python-style loops".into());
        }
        if self.is("return") {
            self.bump();
            if self.at(&Tok::Semi) || self.at(&Tok::RBrace) {
                self.eat_if(&Tok::Semi);
                return Ok("return".into());
            }
            let e = self.parse_expr()?;
            self.eat_if(&Tok::Semi);
            return Ok(format!("return {e}"));
        }
        if self.is("const") || self.is("let") || self.is("var") {
            self.bump();
            let n = self.name()?;
            self.eat(&Tok::Eq)?;
            let r = self.parse_expr()?;
            self.eat_if(&Tok::Semi);
            let ty = infer(&r);
            return Ok(format!("{ty} {n} = {r}"));
        }
        for kw in ["class", "import", "export", "async", "await", "try", "catch", "new", "function"] {
            if self.is(kw) && kw != "function" {
                return Err(format!("unsupported JS construct `{kw}` in the SNlang subset translator"));
            }
        }
        let e = self.parse_expr()?;
        if self.eat_if(&Tok::Eq) {
            let r = self.parse_expr()?;
            self.eat_if(&Tok::Semi);
            return Ok(format!("{e} = {r}"));
        }
        self.eat_if(&Tok::Semi);
        Ok(rewrite_call(&e))
    }

    fn parse_expr(&mut self) -> Result<String, String> {
        self.parse_or()
    }
    fn parse_or(&mut self) -> Result<String, String> {
        let mut e = self.parse_and()?;
        while self.eat_if(&Tok::PipePipe) {
            let r = self.parse_and()?;
            e = format!("({e} or {r})");
        }
        Ok(e)
    }
    fn parse_and(&mut self) -> Result<String, String> {
        let mut e = self.parse_eq()?;
        while self.eat_if(&Tok::AmpAmp) {
            let r = self.parse_eq()?;
            e = format!("({e} and {r})");
        }
        Ok(e)
    }
    fn parse_eq(&mut self) -> Result<String, String> {
        let mut e = self.parse_add()?;
        loop {
            let op = if self.eat_if(&Tok::EqEq) || self.eat_if(&Tok::EqEqEq) {
                "=="
            } else if self.eat_if(&Tok::NotEq) {
                "!="
            } else if self.eat_if(&Tok::LtEq) {
                "<="
            } else if self.eat_if(&Tok::GtEq) {
                ">="
            } else if self.eat_if(&Tok::Lt) {
                "<"
            } else if self.eat_if(&Tok::Gt) {
                ">"
            } else {
                break;
            };
            let r = self.parse_add()?;
            e = format!("({e} {op} {r})");
        }
        Ok(e)
    }
    fn parse_add(&mut self) -> Result<String, String> {
        let mut e = self.parse_mul()?;
        loop {
            let op = if self.eat_if(&Tok::Plus) {
                "+"
            } else if self.eat_if(&Tok::Minus) {
                "-"
            } else {
                break;
            };
            let r = self.parse_mul()?;
            e = format!("({e} {op} {r})");
        }
        Ok(e)
    }
    fn parse_mul(&mut self) -> Result<String, String> {
        let mut e = self.parse_unary()?;
        loop {
            let op = if self.eat_if(&Tok::Star) {
                "*"
            } else if self.eat_if(&Tok::Slash) {
                "/"
            } else if self.eat_if(&Tok::Percent) {
                "%"
            } else {
                break;
            };
            let r = self.parse_unary()?;
            e = format!("({e} {op} {r})");
        }
        Ok(e)
    }
    fn parse_unary(&mut self) -> Result<String, String> {
        if self.eat_if(&Tok::Bang) {
            let e = self.parse_unary()?;
            return Ok(format!("(not {e})"));
        }
        if self.eat_if(&Tok::Minus) {
            let e = self.parse_unary()?;
            return Ok(format!("(0 - {e})"));
        }
        self.parse_post()
    }
    fn parse_post(&mut self) -> Result<String, String> {
        let mut e = self.parse_prim()?;
        loop {
            if self.eat_if(&Tok::LParen) {
                let mut a = Vec::new();
                if !self.at(&Tok::RParen) {
                    loop {
                        a.push(self.parse_expr()?);
                        if !self.eat_if(&Tok::Comma) {
                            break;
                        }
                    }
                }
                self.eat(&Tok::RParen)?;
                e = format!("{e}({})", a.join(", "));
            } else if self.eat_if(&Tok::LBrack) {
                let i = self.parse_expr()?;
                self.eat(&Tok::RBrack)?;
                e = format!("{e}[{i}]");
            } else if self.eat_if(&Tok::Dot) {
                let n = self.name()?;
                e = format!("{e}.{n}");
            } else {
                break;
            }
        }
        Ok(e)
    }
    fn parse_prim(&mut self) -> Result<String, String> {
        match self.peek().cloned() {
            Some(Tok::Int(n)) => {
                self.bump();
                Ok(n.to_string())
            }
            Some(Tok::Str(s)) => {
                self.bump();
                Ok(escape_sn_string(&s))
            }
            Some(Tok::Ident(n)) => {
                self.bump();
                Ok(match n.as_str() {
                    "true" | "false" => n,
                    "null" | "undefined" => "none".into(),
                    other => other.to_string(),
                })
            }
            Some(Tok::LParen) => {
                self.bump();
                let e = self.parse_expr()?;
                self.eat(&Tok::RParen)?;
                Ok(format!("({e})"))
            }
            Some(Tok::LBrack) => {
                self.bump();
                let mut els = Vec::new();
                if !self.at(&Tok::RBrack) {
                    loop {
                        els.push(self.parse_expr()?);
                        if !self.eat_if(&Tok::Comma) {
                            break;
                        }
                    }
                }
                self.eat(&Tok::RBrack)?;
                Ok(format!("[{}]", els.join(", ")))
            }
            other => Err(format!("unexpected JS token {other:?}")),
        }
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn eof(&self) -> bool {
        matches!(self.peek(), Some(Tok::Eof) | None)
    }
    fn at(&self, t: &Tok) -> bool {
        self.peek() == Some(t)
    }
    fn is(&self, s: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(n)) if n == s)
    }
    fn bump(&mut self) -> Tok {
        let t = self.peek().cloned().unwrap_or(Tok::Eof);
        if self.pos < self.toks.len() {
            self.pos += 1;
        }
        t
    }
    fn eat(&mut self, t: &Tok) -> Result<(), String> {
        if self.at(t) {
            self.bump();
            Ok(())
        } else {
            Err(format!("expected {t:?}"))
        }
    }
    fn eat_if(&mut self, t: &Tok) -> bool {
        if self.at(t) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn expect_ident(&mut self, s: &str) -> Result<(), String> {
        if self.is(s) {
            self.bump();
            Ok(())
        } else {
            Err(format!("expected {s}"))
        }
    }
    fn name(&mut self) -> Result<String, String> {
        match self.bump() {
            Tok::Ident(n) => Ok(n),
            o => Err(format!("expected name, got {o:?}")),
        }
    }
}

fn infer(r: &str) -> &'static str {
    let r = r.trim();
    if r.starts_with('"') {
        "str"
    } else if r == "true" || r == "false" {
        "bool"
    } else if r.starts_with('[') {
        "list<int>"
    } else {
        "int"
    }
}

fn rewrite_call(e: &str) -> String {
    if let Some(rest) = e.strip_prefix("console.log(") {
        if let Some(args) = rest.strip_suffix(')') {
            return format!("print({args})");
        }
    }
    e.to_string()
}

pub fn translate(src: &str) -> Result<String, String> {
    P::parse(src)
}
