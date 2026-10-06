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
    Eof,
}

struct Lx<'a> {
    src: &'a str,
    b: &'a [u8],
    pos: usize,
}
impl<'a> Lx<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            b: src.as_bytes(),
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
        if self.pos >= self.b.len() {
            return Ok(Tok::Eof);
        }
        let c = self.b[self.pos];
        let p1 = self.b.get(self.pos + 1).copied();
        match c {
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
            b'=' if p1 == Some(b'=') => {
                self.pos += 2;
                Ok(Tok::EqEq)
            }
            b'=' => {
                self.pos += 1;
                Ok(Tok::Eq)
            }
            b'!' if p1 == Some(b'=') => {
                self.pos += 2;
                Ok(Tok::NotEq)
            }
            b'!' => {
                self.pos += 1;
                Ok(Tok::Bang)
            }
            b'<' if p1 == Some(b'=') => {
                self.pos += 2;
                Ok(Tok::LtEq)
            }
            b'<' => {
                self.pos += 1;
                Ok(Tok::Lt)
            }
            b'>' if p1 == Some(b'=') => {
                self.pos += 2;
                Ok(Tok::GtEq)
            }
            b'>' => {
                self.pos += 1;
                Ok(Tok::Gt)
            }
            b'&' if p1 == Some(b'&') => {
                self.pos += 2;
                Ok(Tok::AmpAmp)
            }
            b'|' if p1 == Some(b'|') => {
                self.pos += 2;
                Ok(Tok::PipePipe)
            }
            b'"' => self.str_lit(),
            b'0'..=b'9' => {
                let s = self.pos;
                while self.pos < self.b.len() && self.b[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
                Ok(Tok::Int(self.src[s..self.pos].parse().unwrap_or(0)))
            }
            b'A'..=b'Z' | b'a'..=b'z' | b'_' => {
                let s = self.pos;
                self.pos += 1;
                while self.pos < self.b.len() {
                    let x = self.b[self.pos];
                    if x.is_ascii_alphanumeric() || x == b'_' {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                Ok(Tok::Ident(self.src[s..self.pos].to_string()))
            }
            other => Err(format!("unsupported C character {:?}", other as char)),
        }
    }
    fn str_lit(&mut self) -> Result<Tok, String> {
        self.pos += 1;
        let mut o = String::new();
        while self.pos < self.b.len() {
            let c = self.b[self.pos];
            if c == b'"' {
                self.pos += 1;
                return Ok(Tok::Str(o));
            }
            if c == b'\\' {
                self.pos += 1;
                o.push(match self.b[self.pos] {
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
            while self.pos < self.b.len() && self.b[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.pos + 1 < self.b.len() && self.b[self.pos] == b'/' && self.b[self.pos + 1] == b'/' {
                self.pos += 2;
                while self.pos < self.b.len() && self.b[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }
            if self.pos + 1 < self.b.len() && self.b[self.pos] == b'/' && self.b[self.pos + 1] == b'*' {
                self.pos += 2;
                while self.pos + 1 < self.b.len()
                    && !(self.b[self.pos] == b'*' && self.b[self.pos + 1] == b'/')
                {
                    self.pos += 1;
                }
                if self.pos + 1 < self.b.len() {
                    self.pos += 2;
                }
                continue;
            }
            if self.pos < self.b.len() && self.b[self.pos] == b'#' {
                while self.pos < self.b.len() && self.b[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }
            break;
        }
    }
}

struct P {
    t: Vec<Tok>,
    i: usize,
}
impl P {
    fn parse(src: &str) -> Result<String, String> {
        let mut p = P {
            t: Lx::new(src).tokenize()?,
            i: 0,
        };
        let mut items = Vec::new();
        while !p.eof() {
            items.push(p.parse_item()?);
        }
        Ok(items.join("\n\n"))
    }
    fn parse_item(&mut self) -> Result<String, String> {
        let ty = self.ty()?;
        let name = self.name()?;
        if self.at(&Tok::LParen) {
            self.bump();
            let mut ps = Vec::new();
            if !self.at(&Tok::RParen) {
                loop {
                    let pt = self.ty()?;
                    let pn = self.name()?;
                    ps.push(format!("{pt} {pn}"));
                    if !self.eat_if(&Tok::Comma) {
                        break;
                    }
                }
            }
            self.eat(&Tok::RParen)?;
            let ret = if ty == "void" {
                String::new()
            } else {
                format!(" -> {ty}")
            };
            let body = self.parse_block()?;
            return Ok(format!(
                "fn {name}({}){ret} {{\n{}\n}}",
                ps.join(", "),
                indent(&body, 1)
            ));
        }
        Err("C subset only translates functions, not global variables".into())
    }
    fn ty(&mut self) -> Result<String, String> {
        if self.is("int") || self.is("long") {
            self.bump();
            return Ok("int".into());
        }
        if self.is("char") {
            self.bump();
            if self.eat_if(&Tok::Star) {
                return Ok("str".into());
            }
            return Err("bare `char` is unsupported; use char*".into());
        }
        if self.is("void") {
            self.bump();
            return Ok("void".into());
        }
        if self.is("bool") {
            self.bump();
            return Ok("bool".into());
        }
        Err("unsupported C type in subset (int, char*, void, bool)".into())
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
            return Err("C `for` is unsupported in the subset; use `while`".into());
        }
        if self.is("return") {
            self.bump();
            if self.at(&Tok::Semi) {
                self.bump();
                return Ok("return".into());
            }
            let e = self.parse_expr()?;
            self.eat_if(&Tok::Semi);
            return Ok(format!("return {e}"));
        }
        if self.is("int") || self.is("char") || self.is("bool") || self.is("long") {
            let ty = self.ty()?;
            let n = self.name()?;
            self.eat(&Tok::Eq)?;
            let r = self.parse_expr()?;
            self.eat_if(&Tok::Semi);
            return Ok(format!("{ty} {n} = {r}"));
        }
        for kw in ["struct", "typedef", "switch", "goto", "union"] {
            if self.is(kw) {
                return Err(format!("unsupported C construct `{kw}` in the subset translator"));
            }
        }
        let e = self.parse_expr()?;
        if self.eat_if(&Tok::Eq) {
            let r = self.parse_expr()?;
            self.eat_if(&Tok::Semi);
            return Ok(format!("{e} = {r}"));
        }
        self.eat_if(&Tok::Semi);
        Ok(rewrite(&e))
    }
    fn parse_expr(&mut self) -> Result<String, String> {
        self.parse_or()
    }
    fn parse_or(&mut self) -> Result<String, String> {
        let mut e = self.parse_and()?;
        while self.eat_if(&Tok::PipePipe) {
            e = format!("({e} or {})", self.parse_and()?);
        }
        Ok(e)
    }
    fn parse_and(&mut self) -> Result<String, String> {
        let mut e = self.parse_eq()?;
        while self.eat_if(&Tok::AmpAmp) {
            e = format!("({e} and {})", self.parse_eq()?);
        }
        Ok(e)
    }
    fn parse_eq(&mut self) -> Result<String, String> {
        let mut e = self.parse_add()?;
        loop {
            let op = if self.eat_if(&Tok::EqEq) {
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
            e = format!("({e} {op} {})", self.parse_add()?);
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
            e = format!("({e} {op} {})", self.parse_mul()?);
        }
        Ok(e)
    }
    fn parse_mul(&mut self) -> Result<String, String> {
        let mut e = self.parse_un()?;
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
            e = format!("({e} {op} {})", self.parse_un()?);
        }
        Ok(e)
    }
    fn parse_un(&mut self) -> Result<String, String> {
        if self.eat_if(&Tok::Bang) {
            return Ok(format!("(not {})", self.parse_un()?));
        }
        if self.eat_if(&Tok::Minus) {
            return Ok(format!("(0 - {})", self.parse_un()?));
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
                Ok(n)
            }
            Some(Tok::LParen) => {
                self.bump();
                let e = self.parse_expr()?;
                self.eat(&Tok::RParen)?;
                Ok(format!("({e})"))
            }
            o => Err(format!("unexpected C token {o:?}")),
        }
    }
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
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
        if self.i < self.t.len() {
            self.i += 1;
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
    fn name(&mut self) -> Result<String, String> {
        match self.bump() {
            Tok::Ident(n) => Ok(n),
            o => Err(format!("expected name {o:?}")),
        }
    }
}

fn rewrite(e: &str) -> String {
    if let Some(rest) = e.strip_prefix("printf(") {
        if let Some(args) = rest.strip_suffix(')') {
            if args.starts_with('"') && !args.contains(',') {
                return format!("print({args})");
            }
            if let Some((_, rest)) = args.split_once(',') {
                return format!("print({})", rest.trim());
            }
            return format!("print({args})");
        }
    }
    if let Some(rest) = e.strip_prefix("puts(") {
        if let Some(a) = rest.strip_suffix(')') {
            return format!("print({a})");
        }
    }
    e.to_string()
}

pub fn translate(src: &str) -> Result<String, String> {
    P::parse(src)
}
