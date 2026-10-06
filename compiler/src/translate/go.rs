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
    Eq,
    ColonEq,
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
    AndAnd,
    OrOr,
    Bang,
    Dot,
    Colon,
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
            b':' if p1 == Some(b'=') => {
                self.pos += 2;
                Ok(Tok::ColonEq)
            }
            b':' => {
                self.pos += 1;
                Ok(Tok::Colon)
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
                Ok(Tok::AndAnd)
            }
            b'|' if p1 == Some(b'|') => {
                self.pos += 2;
                Ok(Tok::OrOr)
            }
            b'"' | b'`' => self.str_lit(),
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
            other => Err(format!("unsupported Go character {:?}", other as char)),
        }
    }
    fn str_lit(&mut self) -> Result<Tok, String> {
        let q = self.b[self.pos];
        self.pos += 1;
        let mut o = String::new();
        while self.pos < self.b.len() {
            let c = self.b[self.pos];
            if c == q {
                self.pos += 1;
                return Ok(Tok::Str(o));
            }
            if c == b'\\' {
                self.pos += 1;
                o.push(self.b[self.pos] as char);
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
            if p.is("package") {
                p.bump();
                p.name()?;
                continue;
            }
            if p.is("import") {
                return Err("Go `import` is unsupported in the subset; drop imports and use SNlang `use`".into());
            }
            if p.is("func") {
                items.push(p.parse_fn()?);
            } else if p.is("var") || p.is("const") || p.is("type") {
                return Err(format!(
                    "unsupported top-level Go construct `{}`",
                    p.name_peek()
                ));
            } else {
                items.push(p.parse_stmt()?);
            }
        }
        Ok(items.join("\n\n"))
    }
    fn parse_fn(&mut self) -> Result<String, String> {
        self.expect("func")?;
        let name = self.name()?;
        self.eat(&Tok::LParen)?;
        let mut ps = Vec::new();
        if !self.at(&Tok::RParen) {
            loop {
                let n = self.name()?;
                let ty = self.go_type()?;
                ps.push(format!("{ty} {n}"));
                if !self.eat_if(&Tok::Comma) {
                    break;
                }
            }
        }
        self.eat(&Tok::RParen)?;
        let ret = if self.is("int") || self.is("string") || self.is("bool") {
            let t = self.go_type()?;
            format!(" -> {t}")
        } else {
            String::new()
        };
        let body = self.parse_block()?;
        Ok(format!(
            "fn {name}({}){ret} {{\n{}\n}}",
            ps.join(", "),
            indent(&body, 1)
        ))
    }
    fn go_type(&mut self) -> Result<String, String> {
        if self.is("int") || self.is("int64") {
            self.bump();
            return Ok("int".into());
        }
        if self.is("string") {
            self.bump();
            return Ok("str".into());
        }
        if self.is("bool") {
            self.bump();
            return Ok("bool".into());
        }
        if self.at(&Tok::LBrack) {
            self.bump();
            self.eat(&Tok::RBrack)?;
            let inner = self.go_type()?;
            return Ok(format!("list<{inner}>"));
        }
        Err("unsupported Go type in subset".into())
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
            let c = self.parse_expr()?;
            let t = self.parse_block()?;
            let mut o = format!("if ({c}) {{\n{}\n}}", indent(&t, 1));
            if self.is("else") {
                self.bump();
                let e = if self.is("if") {
                    self.parse_stmt()?
                } else {
                    self.parse_block()?
                };
                o.push_str(&format!(" else {{\n{}\n}}", indent(&e, 1)));
            }
            return Ok(o);
        }
        if self.is("for") {
            self.bump();
            if self.at(&Tok::LBrace) {
                let b = self.parse_block()?;
                return Ok(format!("while (true) {{\n{}\n}}", indent(&b, 1)));
            }
            let a = self.parse_expr()?;
            if self.at(&Tok::LBrace) {
                let b = self.parse_block()?;
                return Ok(format!("while ({a}) {{\n{}\n}}", indent(&b, 1)));
            }
            return Err("Go subset supports `for { }` and `for cond { }` only".into());
        }
        if self.is("return") {
            self.bump();
            if self.at(&Tok::RBrace) {
                return Ok("return".into());
            }
            let e = self.parse_expr()?;
            return Ok(format!("return {e}"));
        }
        if self.is("var") {
            self.bump();
            let n = self.name()?;
            let ty = if self.is("int") || self.is("string") || self.is("bool") {
                self.go_type()?
            } else {
                "int".into()
            };
            if self.eat_if(&Tok::Eq) {
                let r = self.parse_expr()?;
                return Ok(format!("{ty} {n} = {r}"));
            }
            return Ok(format!("{ty} {n} = 0"));
        }
        for kw in ["go", "select", "defer", "chan", "struct", "interface", "range"] {
            if self.is(kw) {
                return Err(format!("unsupported Go construct `{kw}` in the subset translator"));
            }
        }
        let e = self.parse_expr()?;
        if self.eat_if(&Tok::ColonEq) {
            let r = self.parse_expr()?;
            let n = e;
            let ty = infer(&r);
            return Ok(format!("{ty} {n} = {r}"));
        }
        if self.eat_if(&Tok::Eq) {
            let r = self.parse_expr()?;
            return Ok(format!("{e} = {r}"));
        }
        Ok(rewrite(&e))
    }
    fn parse_expr(&mut self) -> Result<String, String> {
        self.parse_or()
    }
    fn parse_or(&mut self) -> Result<String, String> {
        let mut e = self.parse_and()?;
        while self.eat_if(&Tok::OrOr) {
            let r = self.parse_and()?;
            e = format!("({e} or {r})");
        }
        Ok(e)
    }
    fn parse_and(&mut self) -> Result<String, String> {
        let mut e = self.parse_eq()?;
        while self.eat_if(&Tok::AndAnd) {
            let r = self.parse_eq()?;
            e = format!("({e} and {r})");
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
            let r = self.parse_un()?;
            e = format!("({e} {op} {r})");
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
            } else if self.eat_if(&Tok::Dot) {
                e = format!("{e}.{}", self.name()?);
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
                    "nil" => "none".into(),
                    other => other.to_string(),
                })
            }
            Some(Tok::LParen) => {
                self.bump();
                let e = self.parse_expr()?;
                self.eat(&Tok::RParen)?;
                Ok(format!("({e})"))
            }
            Some(Tok::LBrack) => Err("Go composite literals are not in the subset".into()),
            o => Err(format!("unexpected Go token {o:?}")),
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
    fn name_peek(&self) -> String {
        match self.peek() {
            Some(Tok::Ident(n)) => n.clone(),
            _ => "?".into(),
        }
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
    fn expect(&mut self, s: &str) -> Result<(), String> {
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
            o => Err(format!("expected name {o:?}")),
        }
    }
}

fn infer(r: &str) -> &'static str {
    if r.trim().starts_with('"') {
        "str"
    } else if r == "true" || r == "false" {
        "bool"
    } else {
        "int"
    }
}

fn rewrite(e: &str) -> String {
    if let Some(rest) = e.strip_prefix("fmt.Println(") {
        if let Some(a) = rest.strip_suffix(')') {
            return format!("print({a})");
        }
    }
    if let Some(rest) = e.strip_prefix("fmt.Print(") {
        if let Some(a) = rest.strip_suffix(')') {
            return format!("printn({a})");
        }
    }
    e.to_string()
}

pub fn translate(src: &str) -> Result<String, String> {
    P::parse(src)
}
