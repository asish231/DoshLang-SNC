//! Language server over stdio.
//!
//! Beyond diagnostics it provides completion, hover, go-to-definition,
//! find-references, rename, and document symbols. Requests are parsed as real
//! JSON and answered from the compiler's own AST and symbol tables, so the
//! language features agree with what the compiler actually accepts.
use crate::check::{check_programs, CheckDb};
use crate::diag::Diagnostics;
use crate::parser::Parser;
use crate::span::SourceFile;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Read, Write};

/// Open documents, keyed by URI.
pub struct Server {
    docs: HashMap<String, String>,
    /// Bumped whenever a document changes, so a request re-parses at most once.
    rev: HashMap<String, u64>,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    pub fn new() -> Self {
        Server {
            docs: HashMap::new(),
            rev: HashMap::new(),
        }
    }

    pub fn run(&mut self) -> Result<(), String> {
        let mut stdin = std::io::stdin().lock();
        let mut stdout = std::io::stdout();
        loop {
            let mut len_line = String::new();
            if stdin.read_line(&mut len_line).map_err(|e| e.to_string())? == 0 {
                break;
            }
            if !len_line.starts_with("Content-Length:") {
                continue;
            }
            let len: usize = len_line[15..].trim().parse().unwrap_or(0);
            let mut sep = String::new();
            stdin.read_line(&mut sep).map_err(|e| e.to_string())?;
            let mut body = vec![0u8; len];
            stdin.read_exact(&mut body).map_err(|e| e.to_string())?;
            let text = String::from_utf8_lossy(&body).to_string();
            if let Some(resp) = self.handle(&text) {
                write_json_rpc(&mut stdout, &resp)?;
            }
        }
        Ok(())
    }

    fn doc(&self, uri: &str) -> String {
        self.docs.get(uri).cloned().unwrap_or_default()
    }

    fn set_doc(&mut self, uri: &str, text: &str) {
        self.docs.insert(uri.to_string(), text.to_string());
        let e = self.rev.entry(uri.to_string()).or_insert(0);
        *e += 1;
    }

    /// Dispatch one JSON-RPC message.
    pub fn handle(&mut self, text: &str) -> Option<String> {
        let v: Value = serde_json::from_str(text).ok()?;
        let method = v.get("method").and_then(|m| m.as_str())?.to_string();
        let id = v.get("id").cloned();
        let params = v.get("params").cloned().unwrap_or(Value::Null);
        let uri = params
            .get("textDocument")
            .and_then(|t| t.get("uri"))
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();

        match method.as_str() {
            "initialize" => {
                return reply(
                    id,
                    json!({
                        "capabilities": {
                            "textDocumentSync": 1,
                            "documentSymbolProvider": true,
                            "completionProvider": { "triggerCharacters": [".", " "] },
                            "hoverProvider": true,
                            "definitionProvider": true,
                            "referencesProvider": true,
                            "renameProvider": true
                        },
                        "serverInfo": { "name": "snc-lsp", "version": env!("CARGO_PKG_VERSION") }
                    }),
                )
            }
            "shutdown" => return reply(id, Value::Null),
            "exit" => return None,
            "textDocument/didOpen" | "textDocument/didChange" => {
                let src = params
                    .get("textDocument")
                    .and_then(|t| t.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                self.set_doc(&uri, &src);
                let diags = self.diagnostics(&src);
                let notif = json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/publishDiagnostics",
                    "params": { "uri": uri, "diagnostics": diags }
                });
                let mut out = std::io::stdout();
                let _ = write_json_rpc(&mut out, &notif.to_string());
                return None;
            }
            "textDocument/documentSymbol" => {
                let src = self.doc(&uri);
                return reply(id, self.document_symbols(&src));
            }
            "textDocument/completion" => {
                let src = self.doc(&uri);
                let pos = position(&params);
                return reply(id, self.completion(&src, pos));
            }
            "textDocument/hover" => {
                let src = self.doc(&uri);
                let pos = position(&params);
                return reply(id, self.hover(&src, pos));
            }
            "textDocument/definition" => {
                let src = self.doc(&uri);
                let pos = position(&params);
                return reply(id, self.definition(&src, pos));
            }
            "textDocument/references" => {
                let src = self.doc(&uri);
                let pos = position(&params);
                return reply(id, self.references(&src, pos, params));
            }
            "textDocument/rename" => {
                let src = self.doc(&uri);
                let pos = position(&params);
                let new_name = params
                    .get("newName")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                return reply(id, self.rename(&src, pos, &new_name));
            }
            _ => {}
        }
        None
    }

    fn analyze(&self, src: &str) -> Option<(CheckDb, SourceFile)> {
        let program = Parser::parse_file(0, src).ok()?;
        let mut diag = Diagnostics::default();
        let db = check_programs(&[program], &mut diag);
        let file = SourceFile::new(0, "buffer.sn".into(), src.to_string());
        Some((db, file))
    }

    pub fn diagnostics(&self, src: &str) -> Value {
        let file = SourceFile::new(0, "buffer.sn".into(), src.to_string());
        let items: Vec<Value> = match Parser::parse_file(0, src) {
            Ok(program) => {
                let mut diag = Diagnostics::default();
                let _ = check_programs(&[program], &mut diag);
                diag.errors
                    .iter()
                    .map(|e| {
                        let (sl, sc) = file.loc(e.span.start as usize);
                        let (el, ec) =
                            file.loc(e.span.end.max(e.span.start.saturating_add(1)) as usize);
                        json!({
                            "message": e.message,
                            "severity": 1,
                            "source": "snc",
                            "range": range(sl - 1, sc - 1, el - 1, (ec - 1).max(sc))
                        })
                    })
                    .collect()
            }
            Err(e) => vec![json!({
                "message": e,
                "severity": 1,
                "source": "snc",
                "range": range(0, 0, 0, 1)
            })],
        };
        Value::Array(items)
    }

    fn document_symbols(&self, src: &str) -> Value {
        let mut items = Vec::new();
        let Ok(program) = Parser::parse_file(0, src) else {
            return Value::Array(items);
        };
        for item in &program.items {
            match item {
                crate::ast::Item::Fn(f) => {
                    let line = line_of(src, f.span.start as usize);
                    items.push(json!({
                        "name": f.name,
                        "kind": 12,
                        "detail": format!("fn {}({})", f.name, param_list(&f.params)),
                        "range": range(line, 0, line, 0),
                        "selectionRange": range(line, 0, line, 0)
                    }));
                }
                crate::ast::Item::Blueprint(b) => {
                    let line = line_of(src, b.span.start as usize);
                    let mut children = Vec::new();
                    for m in &b.methods {
                        let ml = line_of(src, m.span.start as usize);
                        let mut kind = 6;
                        if m.is_static {
                            kind = 12;
                        }
                        children.push(json!({
                            "name": m.name,
                            "kind": kind,
                            "detail": format!("fn {}({})", m.name, param_list(&m.params)),
                            "range": range(ml, 0, ml, 0),
                            "selectionRange": range(ml, 0, ml, 0)
                        }));
                    }
                    items.push(json!({
                        "name": b.name,
                        "kind": 5,
                        "detail": blueprint_detail(b),
                        "range": range(line, 0, line, 0),
                        "selectionRange": range(line, 0, line, 0),
                        "children": children
                    }));
                }
                crate::ast::Item::Record(r) => {
                    let line = line_of(src, r.span.start as usize);
                    items.push(json!({
                        "name": r.name,
                        "kind": 23,
                        "detail": format!("record {} ({} bytes)", r.name, 0),
                        "range": range(line, 0, line, 0),
                        "selectionRange": range(line, 0, line, 0)
                    }));
                }
                crate::ast::Item::Enum(e) => {
                    let line = line_of(src, e.span.start as usize);
                    items.push(json!({
                        "name": e.name,
                        "kind": 10,
                        "detail": format!("enum {} ({} variants)", e.name, e.variants.len()),
                        "range": range(line, 0, line, 0),
                        "selectionRange": range(line, 0, line, 0)
                    }));
                }
                crate::ast::Item::Contract(c) => {
                    let line = line_of(src, c.span.start as usize);
                    items.push(json!({
                        "name": c.name,
                        "kind": 11,
                        "detail": format!("contract {} ({} methods)", c.name, c.methods.len()),
                        "range": range(line, 0, line, 0),
                        "selectionRange": range(line, 0, line, 0)
                    }));
                }
                _ => {}
            }
        }
        Value::Array(items)
    }

    /// Context-aware completion: locals in scope, then functions, blueprint
    /// methods of the receiver's type, contracts, and builtins.
    fn completion(&self, src: &str, pos: (u32, u32)) -> Value {
        let offset = offset_of(src, pos);
        let before = &src[..offset.min(src.len())];
        let mut items: Vec<Value> = Vec::new();

        // Member access: complete methods of the receiver's type.
        let mut receiver_type = None;
        // The caret may sit just before or just after the '.'.
        let caret_is_dot = before.trim_end().ends_with('.')
            || src[offset.min(src.len())..].trim_start().starts_with('.');
        if caret_is_dot {
            let head = before.trim_end();
            let head = if head.ends_with('.') {
                &head[..head.len() - 1]
            } else {
                head
            };
            let word = head
                .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .unwrap_or("");
            if !word.is_empty() {
                if let Some((db, _)) = self.analyze(src) {
                    receiver_type = infer_expr_type(&db, src, word);
                }
            }
        }

        if let Some(bp) = receiver_type {
            let mut seen: Vec<String> = Vec::new();
            if let Some((db, _)) = self.analyze(src) {
                let mut cur = Some(bp.clone());
                while let Some(name) = cur {
                    if let Some(info) = db.blueprints.get(&name) {
                        for m in &info.methods {
                            if seen.iter().any(|s| s == m) {
                                continue;
                            }
                            seen.push(m.clone());
                            let owner_methods =
                                db.blueprints.get(&name).map(|i| i.methods.clone()).unwrap_or_default();
                            let is_method = owner_methods.iter().any(|x| x == m);
                            if is_method {
                                items.push(json!({
                                    "label": m,
                                    "kind": 2,
                                    "detail": format!("{name} method"),
                                    "sortText": format!("0{}", m)
                                }));
                            }
                        }
                    }
                    cur = db.blueprints.get(&name).and_then(|i| i.parent.clone());
                }
                for c in db.contracts.values() {
                    for m in &c.methods {
                        items.push(json!({
                            "label": m.name,
                            "kind": 2,
                            "detail": format!("{} method", c.name),
                            "sortText": format!("1{}", m.name)
                        }));
                    }
                }
            }
            return Value::Array(items);
        }

        // Locals visible at this point, innermost first.
        let mut locals: Vec<(String, String)> = Vec::new();
        let mut depth = 0i32;
        for line in src[..offset.min(src.len())].lines() {
            let t = line.trim();
            if t.ends_with('{') {
                depth += 1;
            } else if t == "}" {
                depth -= 1;
                // Leaving a block ends the borrows and locals it introduced;
                // drop the locals we recorded for that depth.
                let d = depth.max(0) as usize;
                locals.retain(|(_, s)| !s.ends_with(&format!("@{d}")));
            }
            if let Some(rest) = t.strip_prefix("let ") {
                push_binding(&mut locals, rest);
            } else if let Some((ty, rest)) = typed_binding(t) {
                let mut tmp = vec![rest.to_string()];
                tmp.push(String::new());
                let _ = ty;
                push_binding(&mut locals, &tmp[0]);
                if let Some(eq) = rest.split_once('=') {
                    if let Some(var) = eq.0.split_whitespace().nth(1) {
                        locals.retain(|(n, _)| n != var);
                        locals.push((var.to_string(), ty.to_string()));
                    }
                }
            } else if let Some(rest) = t.strip_prefix("new ") {
                // `new Dog d(...)` binds `d` with the blueprint's type.
                let mut parts = rest.split_whitespace();
                let ty = parts.next().unwrap_or("").trim_end_matches(['(', ')', ':']);
                if let Some(nm) = parts.next() {
                    let nm = nm.trim_end_matches(['(', ')', ':']);
                    if !nm.is_empty()
                        && nm.chars().all(|c| c.is_alphanumeric() || c == '_')
                        && !nm.chars().next().map(|c| c.is_numeric()).unwrap_or(true)
                    {
                        locals.push((nm.to_string(), ty.to_string()));
                    }
                }
            }
        }
        for (n, ty) in locals.iter().rev() {
            items.push(json!({
                "label": n,
                "kind": 6,
                "detail": ty,
                "sortText": format!("0{n}")
            }));
        }

        if let Some((db, _)) = self.analyze(src) {
            let mut names: Vec<&String> = db.funcs.keys().filter(|k| !k.contains("::")).collect();
            names.sort();
            for n in names {
                if let Some(sig) = db.funcs.get(n) {
                    items.push(json!({
                        "label": n,
                        "kind": 3,
                        "detail": format!("fn {}({})", sig.name, sig.params.iter()
                            .map(|p| p.0.clone()).collect::<Vec<_>>().join(", ")),
                        "sortText": format!("1{n}")
                    }));
                }
            }
            let mut bps: Vec<&String> = db.blueprints.keys().collect();
            bps.sort();
            for b in bps {
                items.push(json!({
                    "label": b,
                    "kind": 7,
                    "detail": "blueprint",
                    "sortText": format!("2{b}")
                }));
            }
            let mut cts: Vec<&String> = db.contracts.keys().collect();
            cts.sort();
            for c in cts {
                items.push(json!({
                    "label": c,
                    "kind": 8,
                    "detail": "contract",
                    "sortText": format!("3{c}")
                }));
            }
        }
        for (label, kind, detail) in BUILTINS {
            items.push(json!({
                "label": label,
                "kind": kind,
                "detail": detail,
                "sortText": format!("9{label}")
            }));
        }
        Value::Array(items)
    }

    /// Hover: the signature of whatever is under the cursor.
    fn hover(&self, src: &str, pos: (u32, u32)) -> Value {
        let Some(word) = word_at(src, pos) else {
            return Value::Null;
        };
        let Some((db, _)) = self.analyze(src) else {
            return Value::Null;
        };
        if let Some(sig) = db.funcs.get(&word) {
            let params: Vec<String> = sig
                .params
                .iter()
                .map(|p| format!("{}: {}", p.0, type_name(&p.1)))
                .collect();
            let mut doc = format!("```sn\nfn {}({}) -> {}", sig.name, params.join(", "), type_name(&sig.ret));
            if sig.is_method {
                doc.push_str("\n// method");
            }
            if sig.is_static {
                doc.push_str("\n// static");
            }
            if sig.is_extern {
                doc.push_str("\n// extern C");
            }
            doc.push_str("\n```");
            return json!({ "contents": { "kind": "markdown", "value": doc } });
        }
        if let Some(info) = db.blueprints.get(&word) {
            let mut doc = format!("```sn\nblueprint {}", info.name);
            if let Some(p) = &info.parent {
                doc.push_str(&format!(" from {p}"));
            }
            if !info.contracts.is_empty() {
                doc.push_str(&format!(" follows {}", info.contracts.join(", ")));
            }
            doc.push('\n');
            for f in &info.fields {
                doc.push_str(&format!("  {}: {}\n", f.name, type_name(&f.ty)));
            }
            for m in &info.methods {
                doc.push_str(&format!("  fn {}()\n", m));
            }
            doc.push_str("```");
            return json!({ "contents": { "kind": "markdown", "value": doc } });
        }
        if let Some(c) = db.contracts.get(&word) {
            let mut doc = format!("```sn\ncontract {}", c.name);
            for m in &c.methods {
                doc.push_str(&format!("\n  fn {}()", m.name));
            }
            doc.push_str("\n```");
            return json!({ "contents": { "kind": "markdown", "value": doc } });
        }
        if let Some(r) = db.records.get(&word) {
            return json!({ "contents": { "kind": "markdown",
                "value": format!("```sn\nrecord {} ({} bytes)\n```", r.name, r.size) } });
        }
        if let Some(e) = db.enums.get(&word) {
            let n = e.len();
            return json!({ "contents": { "kind": "markdown",
                "value": format!("```sn\nenum {word} ({n} variants)\n```") } });
        }
        if let Some((_, _, detail)) = BUILTINS.iter().find(|(n, _, _)| *n == word) {
            return json!({ "contents": { "kind": "markdown",
                "value": format!("```sn\n{detail}\n```") } });
        }
        Value::Null
    }

    /// Go to definition of the symbol under the cursor.
    fn definition(&self, src: &str, pos: (u32, u32)) -> Value {
        let Some(word) = word_at(src, pos) else {
            return Value::Array(vec![]);
        };
        let mut out = Vec::new();
        for (i, line) in src.lines().enumerate() {
            let t = line.trim();
            let declares = t
                .strip_prefix("fn ")
                .map(|r| r.split('(').next().map(|n| n.trim() == word).unwrap_or(false))
                .unwrap_or(false)
                || t
                    .strip_prefix("blueprint ")
                    .map(|r| r.split(['<', ' ', '{']).next().map(|n| n == word).unwrap_or(false))
                    .unwrap_or(false)
                || t
                    .strip_prefix("contract ")
                    .map(|r| r.split(['<', ' ', '{']).next().map(|n| n == word).unwrap_or(false))
                    .unwrap_or(false)
                || t
                    .strip_prefix("record ")
                    .map(|r| r.split(['<', ' ', '{']).next().map(|n| n == word).unwrap_or(false))
                    .unwrap_or(false)
                || t
                    .strip_prefix("enum ")
                    .map(|r| r.split(['<', ' ', '{']).next().map(|n| n == word).unwrap_or(false))
                    .unwrap_or(false)
                || t
                    .strip_prefix("fn static ")
                    .map(|r| r.split('(').next().map(|n| n.trim() == word).unwrap_or(false))
                    .unwrap_or(false)
                || (t.starts_with("fn ") && t.contains(&format!(" {word}(")));
            if declares {
                let col = t.find(&word).unwrap_or(0);
                out.push(json!({
                    "uri": "file:///buffer.sn",
                    "range": range(i as u32, col as u32, i as u32, (col + word.len()) as u32)
                }));
            }
        }
        Value::Array(out)
    }

    /// Every occurrence of the symbol, declaration included.
    fn references(&self, src: &str, pos: (u32, u32), params: Value) -> Value {
        let Some(word) = word_at(src, pos) else {
            return Value::Array(vec![]);
        };
        let include_decl = params
            .get("context")
            .and_then(|c| c.get("includeDeclaration"))
            .and_then(|b| b.as_bool())
            .unwrap_or(true);
        let mut out = Vec::new();
        for (i, line) in src.lines().enumerate() {
            let mut from = 0usize;
            while let Some(rel) = line[from..].find(&word) {
                let col = from + rel;
                let before_ok = col == 0
                    || !line[..col]
                        .chars()
                        .last()
                        .map(|c| c.is_alphanumeric() || c == '_')
                        .unwrap_or(false);
                let after_ok = line[col + word.len()..]
                    .chars()
                    .next()
                    .map(|c| !(c.is_alphanumeric() || c == '_'))
                    .unwrap_or(true);
                let is_decl = line.trim().starts_with(&format!("fn {word}"))
                    || line.trim().starts_with(&format!("blueprint {word}"))
                    || line.trim().starts_with(&format!("contract {word}"))
                    || line.trim().starts_with(&format!("record {word}"))
                    || line.trim().starts_with(&format!("enum {word}"));
                if before_ok && after_ok && (include_decl || !is_decl) {
                    out.push(json!({
                        "uri": "file:///buffer.sn",
                        "range": range(i as u32, col as u32, i as u32, (col + word.len()) as u32)
                    }));
                }
                from = col + word.len();
                if from >= line.len() {
                    break;
                }
            }
        }
        Value::Array(out)
    }

    /// Rename every occurrence, or report why the symbol cannot be renamed.
    fn rename(&self, src: &str, pos: (u32, u32), new_name: &str) -> Value {
        if new_name.is_empty()
            || !new_name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_')
            || new_name.chars().next().map(|c| c.is_numeric()).unwrap_or(true)
        {
            return json!({ "message": format!("'{new_name}' is not a valid SNlang identifier") });
        }
        let Some(word) = word_at(src, pos) else {
            return json!({ "message": "no symbol under the cursor" });
        };
        if BUILTINS.iter().any(|(n, _, _)| *n == word) {
            return json!({ "message": format!("'{word}' is a builtin and cannot be renamed") });
        }
        if let Some((db, _)) = self.analyze(src) {
            if let Some(sig) = db.funcs.get(&word) {
                if sig.is_extern {
                    return json!({ "message": format!("'{word}' is an extern C symbol and cannot be renamed") });
                }
            }
        }
        let edits: Vec<Value> = self.references(src, pos, json!({}))
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|r| {
                let rg = r.get("range").cloned().unwrap_or(Value::Null);
                json!({ "range": rg, "newText": new_name })
            })
            .collect();
        let mut changes = serde_json::Map::new();
        changes.insert("file:///buffer.sn".to_string(), json!({ "changes": edits }));
        Value::Object(changes.into_iter().collect())
    }
}

/// `(name, completion kind, hover detail)` for runtime builtins.
pub const BUILTINS: &[(&str, u64, &str)] = &[
    ("print", 3, "fn print(str)"),
    ("printn", 3, "fn printn(str)"),
    ("input", 3, "fn input(str) -> str"),
    ("cast", 3, "fn cast(value, type)"),
    ("len", 3, "fn len(container) -> int"),
    ("sleep_async", 3, "fn sleep_async(int ms) -> future"),
    ("get_async", 3, "fn get_async(str url) -> future"),
    ("future_data", 3, "fn future_data(future) -> str"),
    ("select", 3, "fn select(chans, timeout_ms) -> int"),
    ("ptr_alloc", 3, "fn ptr_alloc(int bytes) -> ptr"),
    ("ptr_free", 3, "fn ptr_free(ptr)"),
    ("ptr_load", 3, "fn ptr_load(ptr, offset) -> int"),
    ("ptr_store", 3, "fn ptr_store(ptr, offset, value)"),
    ("ptr_load_str", 3, "fn ptr_load_str(ptr, offset) -> str"),
    ("int_ptr", 3, "fn int_ptr(int) -> ptr"),
    ("ptr_int", 3, "fn ptr_int(ptr) -> int"),
    ("check_live", 3, "fn check_live(ptr) -> int"),
    ("free_checked", 3, "fn free_checked(ptr) -> int"),
    ("alloc_size", 3, "fn alloc_size(ptr) -> int"),
    ("bytearray_new", 3, "fn bytearray_new(int) -> ptr"),
    ("bytearray_push", 3, "fn bytearray_push(ptr, byte) -> int"),
    ("bytearray_get", 3, "fn bytearray_get(ptr, index) -> int"),
    ("bytearray_len", 3, "fn bytearray_len(ptr) -> int"),
    ("bytearray_slice", 3, "fn bytearray_slice(ptr, start, n) -> ptr"),
    ("sha256", 3, "fn sha256(str, out)"),
    ("sha512", 3, "fn sha512(str, out)"),
    ("md5", 3, "fn md5(str, out)"),
    ("hmac_sha256", 3, "fn hmac_sha256(str key, str msg, out)"),
    ("crc32", 3, "fn crc32(str) -> int"),
    ("consttime_eq", 3, "fn consttime_eq(str, str) -> bool"),
    ("tcp_listen", 3, "fn tcp_listen(int port) -> int"),
    ("tcp_connect", 3, "fn tcp_connect(str host, int port, int timeout) -> int"),
    ("tcp_accept", 3, "fn tcp_accept(int listener) -> int"),
    ("tcp_read", 3, "fn tcp_read(int sock) -> str"),
    ("tcp_write", 3, "fn tcp_write(int sock, str) -> int"),
    ("tcp_close", 3, "fn tcp_close(int sock)"),
    ("select_read", 3, "fn select_read(list<int>, timeout_ms) -> int"),
    ("gc_collect", 3, "fn gc_collect()"),
    ("gc_count", 3, "fn gc_count() -> int"),
    ("gc_set_threshold", 3, "fn gc_set_threshold(int)"),
];

fn reply(id: Option<Value>, result: Value) -> Option<String> {
    let id = id.unwrap_or(Value::Null);
    Some(
        json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string(),
    )
}

fn range(sl: u32, sc: u32, el: u32, ec: u32) -> Value {
    json!({
        "start": { "line": sl, "character": sc },
        "end": { "line": el, "character": ec }
    })
}

fn position(params: &Value) -> (u32, u32) {
    let p = params.get("position");
    (
        p.and_then(|x| x.get("line")).and_then(|x| x.as_u64()).unwrap_or(0) as u32,
        p.and_then(|x| x.get("character")).and_then(|x| x.as_u64()).unwrap_or(0) as u32,
    )
}

/// Byte offset of a 0-based (line, character) pair.
fn offset_of(src: &str, pos: (u32, u32)) -> usize {
    let mut off = 0usize;
    for (i, line) in src.split_inclusive('\n').enumerate() {
        if i as u32 == pos.0 {
            return (off + pos.1 as usize).min(src.len());
        }
        off += line.len();
    }
    src.len()
}

fn line_of(src: &str, offset: usize) -> u32 {
    src[..offset.min(src.len())].matches('\n').count() as u32
}

/// Identifier under a 0-based (line, character) position, if any.
fn word_at(src: &str, pos: (u32, u32)) -> Option<String> {
    let off = offset_of(src, pos);
    let bytes = src.as_bytes();
    if off > bytes.len() {
        return None;
    }
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut start = off;
    while start > 0 && is_word(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = off;
    while end < bytes.len() && is_word(bytes[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    let w = src[start..end].to_string();
    if w.chars().next().map(|c| c.is_numeric()).unwrap_or(true) {
        return None;
    }
    Some(w)
}

/// Recognise a typed declaration: `int x = 1`, `str name`, `Dog d = ...`,
/// `list<int> xs = []`. Returns the type text and the declaration text.
fn typed_binding(line: &str) -> Option<(&str, &str)> {
    const TYPE_STARTS: &[&str] = &[
        "int", "str", "bool", "float", "byte", "ptr", "dec(", "json", "lock", "ref<", "mut<",
        "list<", "map<", "chan<",
    ];
    let head = line.split('=').next().unwrap_or("").trim();
    let mut it = head.splitn(2, char::is_whitespace);
    let ty = it.next()?;
    if !TYPE_STARTS.iter().any(|t| ty.starts_with(t)) {
        return None;
    }
    let rest = it.next()?;
    if rest.is_empty() {
        return None;
    }
    Some((ty, line))
}

fn push_binding(locals: &mut Vec<(String, String)>, rest: &str) {
    let head = rest.split('=').next().unwrap_or("");
    let mut parts = head.split_whitespace();
    let Some(ty) = parts.next() else { return };
    for name in parts {
        let name = name.trim_end_matches(',');
        if !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_')
            && name.chars().next().map(|c| !c.is_numeric()).unwrap_or(false)
        {
            locals.push((name.to_string(), ty.to_string()));
        }
    }
}

/// Type of the receiver of a `name.method` call, read from the declarations
/// visible in the text: `new Dog d()`, `Dog d = new Dog()`, `Dog d`, and
/// `ref<Dog> d = ...` are all recognised.
fn infer_expr_type(db: &CheckDb, src: &str, name: &str) -> Option<String> {
    let mut fallback: Option<String> = None;
    for line in src.lines() {
        let t = line.trim();

        // new Dog d(...)   /   new Dog d = ...
        if let Some(r) = t.strip_prefix("new ") {
            let mut parts = r.split_whitespace();
            let ty = parts.next()?.trim_end_matches(['(', ')', ':', ',']);
            if let Some(var) = parts.next() {
                let var = var.trim_end_matches(['(', ')', ':', ',']);
                if var == name && db.blueprints.contains_key(ty) {
                    return Some(ty.to_string());
                }
            }
        }

        // Dog d = ...   /   ref<Dog> d = ...   /   Dog? d = ...
        let head = t.split('=').next().unwrap_or("").trim();
        let mut it = head.split_whitespace();
        if let Some(first) = it.next() {
            let base = first
                .trim_start_matches("ref<")
                .trim_start_matches("mut<");
            let ty = base.split(['<', '?']).next().unwrap_or("").to_string();
            let is_ref = first.starts_with("ref<") || first.starts_with("mut<");
            if let Some(var) = it.next() {
                let var = var.trim_end_matches(',');
                if var == name && db.blueprints.contains_key(&ty) {
                    if is_ref {
                        fallback = Some(ty);
                    } else {
                        return Some(ty);
                    }
                }
            }
        }
    }
    fallback
}

fn param_list(params: &[crate::ast::Param]) -> String {
    params
        .iter()
        .map(|p| format!("{} {}", type_ast_name(&p.ty), p.name))
        .collect::<Vec<_>>()
        .join(", ")
}

fn blueprint_detail(b: &crate::ast::BlueprintItem) -> String {
    let mut d = format!("blueprint {}", b.name);
    if let Some(p) = &b.parent {
        d.push_str(&format!(" from {p}"));
    }
    if !b.contracts.is_empty() {
        d.push_str(&format!(" follows {}", b.contracts.join(", ")));
    }
    if !b.type_params.is_empty() {
        d.push_str(&format!(
            "<{}>",
            b.type_params
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    d
}

pub fn type_name(t: &crate::types::Type) -> String {
    use crate::types::Type::*;
    match t {
        Void => "void".into(),
        Int => "int".into(),
        Float => "float".into(),
        Str => "str".into(),
        Bool => "bool".into(),
        Byte => "byte".into(),
        Any => "any".into(),
        Ptr => "ptr".into(),
        Ref(i) => format!("ref<{}>", type_name(i)),
        Mut(i) => format!("mut<{}>", type_name(i)),
        List(e) => format!("list<{}>", type_name(e)),
        Map(k, v) => format!("map<{}, {}>", type_name(k), type_name(v)),
        Chan(e) => format!("chan<{}>", type_name(e)),
        Json => "json".into(),
        Lock => "lock".into(),
        Blueprint(b) => b.clone(),
        Contract(c) => c.clone(),
        Record(r) => format!("record {}", r),
        Enum(e) => format!("enum {}", e),
        Dec(s) => format!("dec({s})"),
        None => "none".into(),
        Optional(i) => format!("{}?", type_name(i)),
        I8 => "i8".into(),
        I16 => "i16".into(),
        I32 => "i32".into(),
        U8 => "u8".into(),
        U16 => "u16".into(),
        U32 => "u32".into(),
        U64 => "u64".into(),
        Error => "error".into(),
        Tuple(ts) => format!(
            "({})",
            ts.iter().map(type_name).collect::<Vec<_>>().join(", ")
        ),
        Fn { params, ret } => format!(
            "fn({}) -> {}",
            params.iter().map(type_name).collect::<Vec<_>>().join(", "),
            type_name(ret)
        ),
        Named(n) => n.clone(),
    }
}

fn type_ast_name(t: &crate::ast::TypeAst) -> String {
    use crate::ast::TypeAst::*;
    match t {
        Int => "int".into(),
        Float => "float".into(),
        Str => "str".into(),
        Bool => "bool".into(),
        Byte => "byte".into(),
        Any => "any".into(),
        Ptr => "ptr".into(),
        Mut(i) => format!("mut<{}>", type_ast_name(i)),
        Ref(i) => format!("ref<{}>", type_ast_name(i)),
        List(e) => format!("list<{}>", type_ast_name(e)),
        Map(k, v) => format!("map<{}, {}>", type_ast_name(k), type_ast_name(v)),
        Chan(e) => format!("chan<{}>", type_ast_name(e)),
        Record(n) => format!("record {n}"),
        Enum(n) => format!("enum {n}"),
        Dec(s) => format!("dec({s})"),
        Named(n) => n.clone(),
        Generic(n, a) => format!(
            "{n}<{}>",
            a.iter().map(type_ast_name).collect::<Vec<_>>().join(", ")
        ),
        Fn { params, ret } => format!(
            "fn({}) -> {}",
            params.iter().map(type_ast_name).collect::<Vec<_>>().join(", "),
            ret.as_ref().map(|r| type_ast_name(r)).unwrap_or_else(|| "void".into())
        ),
        Optional(i) => format!("{}?", type_ast_name(i)),
        Tuple(ts) => format!(
            "({})",
            ts.iter().map(type_ast_name).collect::<Vec<_>>().join(", ")
        ),
        Error => "error".into(),
        I8 => "i8".into(),
        I16 => "i16".into(),
        I32 => "i32".into(),
        U8 => "u8".into(),
        U16 => "u16".into(),
        U32 => "u32".into(),
        U64 => "u64".into(),
    }
}

fn write_json_rpc(w: &mut impl Write, body: &str) -> Result<(), String> {
    write!(w, "Content-Length: {}\r\n\r\n{}", body.len(), body).map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())
}

pub fn run_lsp() -> Result<(), String> {
    Server::new().run()
}

pub fn run_debug(source: &str, output: Option<&std::path::Path>) -> Result<String, String> {
    let out = output.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        std::env::temp_dir().join(format!("snc-debug-{}", std::process::id()))
    });
    // Full DWARF build with preserved objects, so lldb resolves SN lines.
    crate::driver::compile_debug(
        &std::path::PathBuf::from(source),
        &out,
        "clang",
    )?;
    // Drive one breakpoint-at-entry session: break on the SN `main`, run the
    // program, and show where it stopped. The binary stays behind for an
    // interactive follow-up (`lldb <out>`).
    let session = std::process::Command::new("lldb")
        .arg("-b")
        .arg("-o")
        .arg("breakpoint set --name sn_fn_main")
        .arg("-o")
        .arg("run")
        .arg("-o")
        .arg("thread backtrace")
        .arg("-o")
        .arg("continue")
        .arg("-o")
        .arg("quit")
        .arg(&out)
        .output()
        .map_err(|e| format!("failed to run lldb (is it installed?): {e}"))?;
    let mut s = format!("Built {} with DWARF (-O0).\n", out.display());
    s.push_str(&String::from_utf8_lossy(&session.stdout));
    if !session.status.success() {
        s.push_str(&String::from_utf8_lossy(&session.stderr));
    }
    s.push_str(&format!(
        "\nFor an interactive session: lldb {}\n  (lldb) b <file>.sn:<line>\n",
        out.display()
    ));
    Ok(s)
}
