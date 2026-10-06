use super::{escape_sn_string, indent};

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Int(i64),
    Str(String),
    Newline,
    Indent,
    Dedent,
    LParen,
    RParen,
    LBrack,
    RBrack,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Dot,
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
    Eof,
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    at_line_start: bool,
    indents: Vec<usize>,
    pending: Vec<Tok>,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            at_line_start: true,
            indents: vec![0],
            pending: vec![],
        }
    }

    fn tokenize(&mut self) -> Result<Vec<Tok>, String> {
        let mut out = Vec::new();
        loop {
            let t = self.next()?;
            let eof = t == Tok::Eof;
            out.push(t);
            if eof {
                break;
            }
        }
        Ok(out)
    }

    fn next(&mut self) -> Result<Tok, String> {
        if let Some(t) = self.pending.pop() {
            return Ok(t);
        }
        if self.at_line_start {
            self.at_line_start = false;
            let mut col = 0usize;
            while self.pos < self.bytes.len() {
                match self.bytes[self.pos] {
                    b' ' => {
                        col += 1;
                        self.pos += 1;
                    }
                    b'\t' => {
                        col += 4;
                        self.pos += 1;
                    }
                    b'\n' | b'\r' => {
                        self.pos += 1;
                        self.at_line_start = true;
                        return Ok(Tok::Newline);
                    }
                    b'#' => {
                        while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                            self.pos += 1;
                        }
                        continue;
                    }
                    _ => break,
                }
            }
            if self.pos >= self.bytes.len() {
                if self.indents.len() > 1 {
                    self.indents.pop();
                    return Ok(Tok::Dedent);
                }
                return Ok(Tok::Eof);
            }
            let cur = *self.indents.last().unwrap();
            if col > cur {
                self.indents.push(col);
                return Ok(Tok::Indent);
            }
            if col < cur {
                while self.indents.last().copied().unwrap_or(0) > col {
                    self.indents.pop();
                    self.pending.push(Tok::Dedent);
                }
                if self.indents.last().copied() != Some(col) {
                    return Err("inconsistent indentation".into());
                }
                if let Some(t) = self.pending.pop() {
                    return Ok(t);
                }
            }
        }
        self.skip_space_comment();
        if self.pos >= self.bytes.len() {
            if self.indents.len() > 1 {
                self.indents.pop();
                return Ok(Tok::Dedent);
            }
            return Ok(Tok::Eof);
        }
        let b = self.bytes[self.pos];
        match b {
            b'\n' => {
                self.pos += 1;
                self.at_line_start = true;
                Ok(Tok::Newline)
            }
            b'\r' => {
                self.pos += 1;
                if self.peek() == Some(b'\n') {
                    self.pos += 1;
                }
                self.at_line_start = true;
                Ok(Tok::Newline)
            }
            b'(' => {
                self.pos += 1;
                Ok(Tok::LParen)
            }
            b')' => {
                self.pos += 1;
                Ok(Tok::RParen)
            }
            b'[' => {
                self.pos += 1;
                Ok(Tok::LBrack)
            }
            b']' => {
                self.pos += 1;
                Ok(Tok::RBrack)
            }
            b'{' => {
                self.pos += 1;
                Ok(Tok::LBrace)
            }
            b'}' => {
                self.pos += 1;
                Ok(Tok::RBrace)
            }
            b',' => {
                self.pos += 1;
                Ok(Tok::Comma)
            }
            b':' => {
                self.pos += 1;
                Ok(Tok::Colon)
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
                self.pos += 1;
                if self.peek() == Some(b'=') {
                    self.pos += 1;
                    Ok(Tok::EqEq)
                } else {
                    Ok(Tok::Eq)
                }
            }
            b'!' => {
                self.pos += 1;
                if self.peek() == Some(b'=') {
                    self.pos += 1;
                    Ok(Tok::NotEq)
                } else {
                    Err("unsupported '!' in Python subset (use not)".into())
                }
            }
            b'<' => {
                self.pos += 1;
                if self.peek() == Some(b'=') {
                    self.pos += 1;
                    Ok(Tok::LtEq)
                } else {
                    Ok(Tok::Lt)
                }
            }
            b'>' => {
                self.pos += 1;
                if self.peek() == Some(b'=') {
                    self.pos += 1;
                    Ok(Tok::GtEq)
                } else {
                    Ok(Tok::Gt)
                }
            }
            b'"' | b'\'' => self.lex_string(),
            b'0'..=b'9' => {
                let start = self.pos;
                while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
                let n: i64 = self.src[start..self.pos]
                    .parse()
                    .map_err(|_| "integer out of range".to_string())?;
                Ok(Tok::Int(n))
            }
            b'A'..=b'Z' | b'a'..=b'z' | b'_' => {
                let start = self.pos;
                self.pos += 1;
                while self.pos < self.bytes.len() {
                    let c = self.bytes[self.pos];
                    if c.is_ascii_alphanumeric() || c == b'_' {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                Ok(Tok::Ident(self.src[start..self.pos].to_string()))
            }
            other => Err(format!(
                "unsupported character {:?} in Python subset",
                other as char
            )),
        }
    }

    fn lex_string(&mut self) -> Result<Tok, String> {
        let quote = self.bytes[self.pos];
        self.pos += 1;
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            if c == quote {
                self.pos += 1;
                return Ok(Tok::Str(out));
            }
            if c == b'\\' {
                self.pos += 1;
                if self.pos >= self.bytes.len() {
                    break;
                }
                let e = self.bytes[self.pos];
                out.push(match e {
                    b'n' => '\n',
                    b't' => '\t',
                    b'\\' => '\\',
                    b'"' => '"',
                    b'\'' => '\'',
                    other => other as char,
                });
                self.pos += 1;
                continue;
            }
            if c == b'\n' {
                return Err("unterminated string".into());
            }
            out.push(c as char);
            self.pos += 1;
        }
        Err("unterminated string".into())
    }

    fn skip_space_comment(&mut self) {
        loop {
            while self.pos < self.bytes.len()
                && (self.bytes[self.pos] == b' ' || self.bytes[self.pos] == b'\t')
            {
                self.pos += 1;
            }
            if self.pos < self.bytes.len() && self.bytes[self.pos] == b'#' {
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }
            break;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    declared: Vec<std::collections::HashSet<String>>,
}

impl Parser {
    fn parse(src: &str) -> Result<String, String> {
        let mut lx = Lexer::new(src);
        let toks = lx.tokenize()?;
        let mut p = Parser {
            toks,
            pos: 0,
            declared: vec![std::collections::HashSet::new()],
        };
        p.skip_nls();
        let mut items = Vec::new();
        let mut saw_main = false;
        while !p.at_eof() {
            p.skip_nls();
            if p.at_eof() {
                break;
            }
            if p.is_ident("def") {
                let f = p.parse_def()?;
                if f.contains("fn main(") {
                    saw_main = true;
                }
                items.push(f);
            } else {
                let st = p.parse_stmt(0)?;
                items.push(st);
            }
            p.skip_nls();
        }
        let mut body = items.join("\n\n");
        if !saw_main {
            let top: Vec<&str> = items
                .iter()
                .filter(|s| !s.trim_start().starts_with("fn "))
                .map(|s| s.as_str())
                .collect();
            let fns: Vec<&str> = items
                .iter()
                .filter(|s| s.trim_start().starts_with("fn "))
                .map(|s| s.as_str())
                .collect();
            if !top.is_empty() {
                body = fns.join("\n\n");
                if !body.is_empty() {
                    body.push_str("\n\n");
                }
                body.push_str("fn main() {\n");
                body.push_str(&indent(&top.join("\n"), 1));
                body.push_str("\n}\n");
            } else if !body.contains("fn main(") {
                body.push_str("\n\nfn main() {\n}\n");
            }
        }
        Ok(body)
    }

    fn parse_def(&mut self) -> Result<String, String> {
        self.expect_ident("def")?;
        let name = self.expect_name()?;
        self.eat(&Tok::LParen)?;
        let mut params = Vec::new();
        let mut pnames = Vec::new();
        if !self.at(&Tok::RParen) {
            loop {
                let n = self.expect_name()?;
                pnames.push(n.clone());
                params.push(format!("int {n}"));
                if !self.eat_if(&Tok::Comma) {
                    break;
                }
            }
        }
        self.eat(&Tok::RParen)?;
        self.eat(&Tok::Colon)?;
        self.eat_nls();
        self.declared.push(pnames.into_iter().collect());
        let body = self.parse_block()?;
        self.declared.pop();
        let ret = if body.lines().any(|l| l.trim().starts_with("return ")) {
            " -> int"
        } else {
            ""
        };
        let pname = if name == "main" { "main" } else { &name };
        Ok(format!(
            "fn {pname}({}){ret} {{\n{}\n}}",
            params.join(", "),
            indent(&body, 1)
        ))
    }

    fn parse_block(&mut self) -> Result<String, String> {
        self.eat(&Tok::Indent)?;
        let mut stmts = Vec::new();
        while !self.at(&Tok::Dedent) && !self.at_eof() {
            self.skip_nls();
            if self.at(&Tok::Dedent) || self.at_eof() {
                break;
            }
            stmts.push(self.parse_stmt(0)?);
            self.skip_nls();
        }
        self.eat(&Tok::Dedent)?;
        Ok(stmts.join("\n"))
    }

    fn parse_stmt(&mut self, _d: usize) -> Result<String, String> {
        if self.is_ident("if") {
            return self.parse_if();
        }
        if self.is_ident("while") {
            self.bump();
            let cond = self.parse_expr()?;
            self.eat(&Tok::Colon)?;
            self.eat_nls();
            let body = self.parse_block()?;
            return Ok(format!("while ({cond}) {{\n{}\n}}", indent(&body, 1)));
        }
        if self.is_ident("for") {
            self.bump();
            let name = self.expect_name()?;
            if !self.is_ident("in") {
                return Err("Python subset only supports `for x in xs:`".into());
            }
            self.bump();
            let iter = self.parse_expr()?;
            self.eat(&Tok::Colon)?;
            self.eat_nls();
            let body = self.parse_block()?;
            return Ok(format!("for ({name} in {iter}) {{\n{}\n}}", indent(&body, 1)));
        }
        if self.is_ident("return") {
            self.bump();
            if self.at(&Tok::Newline) || self.at(&Tok::Dedent) || self.at_eof() {
                return Ok("return".into());
            }
            let e = self.parse_expr()?;
            return Ok(format!("return {e}"));
        }
        if self.is_ident("pass") {
            self.bump();
            return Ok("// pass".into());
        }
        if self.is_ident("break") {
            self.bump();
            return Ok("stop".into());
        }
        if self.is_ident("continue") {
            self.bump();
            return Ok("skip".into());
        }
        for kw in [
            "class", "import", "from", "try", "except", "with", "async", "await", "yield",
            "lambda", "raise", "global", "nonlocal", "assert", "del",
        ] {
            if self.is_ident(kw) {
                return Err(format!(
                    "unsupported Python construct `{kw}` in the SNlang subset translator"
                ));
            }
        }
        let e = self.parse_expr()?;
        if self.eat_if(&Tok::Eq) {
            let rhs = self.parse_expr()?;
            if let Some(name) = ident_of(&e) {
                if self.is_declared(&name) {
                    return Ok(format!("{name} = {rhs}"));
                }
                self.declare(&name);
                let ty = infer_ty(&rhs);
                return Ok(format!("{ty} {name} = {rhs}"));
            }
            return Ok(format!("{e} = {rhs}"));
        }
        Ok(e)
    }

    fn parse_if(&mut self) -> Result<String, String> {
        self.expect_ident("if")?;
        let cond = self.parse_expr()?;
        self.eat(&Tok::Colon)?;
        self.eat_nls();
        let then_b = self.parse_block()?;
        let mut s = format!("if ({cond}) {{\n{}\n}}", indent(&then_b, 1));
        while self.is_ident("elif") {
            self.bump();
            let c = self.parse_expr()?;
            self.eat(&Tok::Colon)?;
            self.eat_nls();
            let b = self.parse_block()?;
            s.push_str(&format!(" else if ({c}) {{\n{}\n}}", indent(&b, 1)));
        }
        if self.is_ident("else") {
            self.bump();
            self.eat(&Tok::Colon)?;
            self.eat_nls();
            let b = self.parse_block()?;
            s.push_str(&format!(" else {{\n{}\n}}", indent(&b, 1)));
        }
        Ok(s)
    }

    fn parse_expr(&mut self) -> Result<String, String> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<String, String> {
        let mut e = self.parse_and()?;
        while self.is_ident("or") {
            self.bump();
            let r = self.parse_and()?;
            e = format!("({e} or {r})");
        }
        Ok(e)
    }

    fn parse_and(&mut self) -> Result<String, String> {
        let mut e = self.parse_not()?;
        while self.is_ident("and") {
            self.bump();
            let r = self.parse_not()?;
            e = format!("({e} and {r})");
        }
        Ok(e)
    }

    fn parse_not(&mut self) -> Result<String, String> {
        if self.is_ident("not") {
            self.bump();
            let e = self.parse_not()?;
            return Ok(format!("(not {e})"));
        }
        self.parse_cmp()
    }

    fn parse_cmp(&mut self) -> Result<String, String> {
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
        if self.eat_if(&Tok::Minus) {
            let e = self.parse_unary()?;
            return Ok(format!("(0 - {e})"));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<String, String> {
        let mut e = self.parse_primary()?;
        loop {
            if self.eat_if(&Tok::LParen) {
                let mut args = Vec::new();
                if !self.at(&Tok::RParen) {
                    loop {
                        args.push(self.parse_expr()?);
                        if !self.eat_if(&Tok::Comma) {
                            break;
                        }
                    }
                }
                self.eat(&Tok::RParen)?;
                if e == "print" {
                    e = format!("print({})", args.join(", "));
                } else if e == "len" && args.len() == 1 {
                    e = format!("{}.length()", args[0]);
                } else if e == "str" && args.len() == 1 {
                    e = format!("cast({}, str)", args[0]);
                } else if e == "int" && args.len() == 1 {
                    e = format!("cast({}, int)", args[0]);
                } else {
                    e = format!("{e}({})", args.join(", "));
                }
            } else if self.eat_if(&Tok::LBrack) {
                let i = self.parse_expr()?;
                self.eat(&Tok::RBrack)?;
                e = format!("{e}[{i}]");
            } else if self.eat_if(&Tok::Dot) {
                let n = self.expect_name()?;
                e = format!("{e}.{n}");
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> Result<String, String> {
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
                    "True" => "true".into(),
                    "False" => "false".into(),
                    "None" => "none".into(),
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
            Some(Tok::LBrace) => Err(
                "Python dicts `{k: v}` are not in the subset; use lists or skip this construct"
                    .into(),
            ),
            other => Err(format!("unexpected token in Python subset: {other:?}")),
        }
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn at(&self, t: &Tok) -> bool {
        self.peek() == Some(t)
    }
    fn at_eof(&self) -> bool {
        matches!(self.peek(), Some(Tok::Eof) | None)
    }
    fn is_ident(&self, s: &str) -> bool {
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
            Err(format!("expected {t:?}, found {:?}", self.peek()))
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
        if self.is_ident(s) {
            self.bump();
            Ok(())
        } else {
            Err(format!("expected {s}"))
        }
    }
    fn expect_name(&mut self) -> Result<String, String> {
        match self.bump() {
            Tok::Ident(n) => Ok(n),
            other => Err(format!("expected name, found {other:?}")),
        }
    }
    fn skip_nls(&mut self) {
        while self.at(&Tok::Newline) {
            self.bump();
        }
    }
    fn eat_nls(&mut self) {
        while self.at(&Tok::Newline) {
            self.bump();
        }
    }

    fn is_declared(&self, name: &str) -> bool {
        self.declared.iter().rev().any(|s| s.contains(name))
    }

    fn declare(&mut self, name: &str) {
        if let Some(s) = self.declared.last_mut() {
            s.insert(name.to_string());
        }
    }
}

fn ident_of(e: &str) -> Option<String> {
    if e.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !e.is_empty() {
        Some(e.to_string())
    } else {
        None
    }
}

fn infer_ty(rhs: &str) -> &'static str {
    let r = rhs.trim();
    if r.starts_with('"') {
        "str"
    } else if r == "true" || r == "false" {
        "bool"
    } else if r.starts_with('[') {
        "list<int>"
    } else if r == "none" {
        "int?"
    } else {
        "int"
    }
}

pub fn translate(src: &str) -> Result<String, String> {
    Parser::parse(src)
}
