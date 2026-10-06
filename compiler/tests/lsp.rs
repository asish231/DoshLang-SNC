//! End-to-end LSP tests: drive `Server::handle` with real JSON-RPC messages
//! and assert on the responses.

use serde_json::{json, Value};

use snc::lsp::Server;

const SRC: &str = r#"blueprint Dog {
    int age = 3

    fn speak() -> str { return "woof" }
}

fn helper(int n) -> int {
    return n * 2
}

fn main() {
    int x = 7
    print(helper(x))
    new Dog d()
    print(d.speak())
}
"#;

fn open(s: &mut Server) {
    s.handle(
        &json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": "file:///buffer.sn", "text": SRC } }
        })
        .to_string(),
    );
}

fn request(s: &mut Server, id: u64, method: &str, params: Value) -> Value {
    let msg = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    });
    let resp = s
        .handle(&msg.to_string())
        .unwrap_or_else(|| panic!("{method} produced no response"));
    let v: Value = serde_json::from_str(&resp).expect("valid JSON-RPC response");
    assert_eq!(v["id"], id, "{method} echoed the wrong id");
    v["result"].clone()
}

fn at(line: u32, character: u32) -> Value {
    json!({ "line": line, "character": character })
}

fn doc() -> Value {
    json!({ "uri": "file:///buffer.sn" })
}

#[test]
fn initialize_advertises_language_features() {
    let mut s = Server::new();
    let caps = request(&mut s, 1, "initialize", json!({}));
    for key in [
        "completionProvider",
        "hoverProvider",
        "definitionProvider",
        "referencesProvider",
        "renameProvider",
        "documentSymbolProvider",
    ] {
        assert!(
            !caps["capabilities"][key].is_null(),
            "{key} not advertised"
        );
    }
    assert_eq!(caps["capabilities"]["definitionProvider"], json!(true));
}

#[test]
fn completion_offers_locals_and_globals() {
    let mut s = Server::new();
    open(&mut s);
    // Inside `main`, after `x` is bound.
    let items = request(
        &mut s,
        2,
        "textDocument/completion",
        json!({ "textDocument": doc(), "position": at(12, 6) }),
    );
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["label"].as_str().unwrap())
        .collect();
    assert!(labels.contains(&"helper"), "global fn missing: {labels:?}");
    assert!(labels.contains(&"x"), "local not offered: {labels:?}");
    assert!(labels.contains(&"print"), "builtin missing: {labels:?}");
    assert!(labels.contains(&"Dog"), "blueprint missing: {labels:?}");

    // After `new Dog d()`, `d` is in scope too.
    let items = request(
        &mut s,
        21,
        "textDocument/completion",
        json!({ "textDocument": doc(), "position": at(14, 6) }),
    );
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["label"].as_str().unwrap())
        .collect();
    assert!(labels.contains(&"d"), "later local not offered: {labels:?}");
}

#[test]
fn completion_after_dot_offers_receiver_methods() {
    let mut s = Server::new();
    open(&mut s);
    // `print(d.speak())` — caret sits on the '.' at column 11.
    let items = request(
        &mut s,
        3,
        "textDocument/completion",
        json!({ "textDocument": doc(), "position": at(14, 11) }),
    );
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec!["speak"], "expected only Dog's methods");
}

#[test]
fn hover_shows_signature() {
    let mut s = Server::new();
    open(&mut s);
    let hov = request(
        &mut s,
        4,
        "textDocument/hover",
        json!({ "textDocument": doc(), "position": at(12, 11) }),
    );
    let text = hov["contents"]["value"].as_str().unwrap();
    assert!(text.contains("fn helper(n: int) -> int"), "hover: {text}");

    // Blueprint hover lists its fields and methods.
    let hov = request(
        &mut s,
        5,
        "textDocument/hover",
        json!({ "textDocument": doc(), "position": at(0, 11) }),
    );
    let text = hov["contents"]["value"].as_str().unwrap();
    assert!(text.contains("blueprint Dog"), "hover: {text}");
    assert!(text.contains("speak"), "hover should list methods: {text}");

    // Builtin hover.
    let hov = request(
        &mut s,
        6,
        "textDocument/hover",
        json!({ "textDocument": doc(), "position": at(12, 6) }),
    );
    assert!(hov["contents"]["value"]
        .as_str()
        .unwrap()
        .contains("print"));
}

#[test]
fn definition_jumps_to_declaration() {
    let mut s = Server::new();
    open(&mut s);
    for (id, method) in [(7u64, "helper"), (8, "Dog")] {
        let (line, ch) = if method == "helper" { (12, 11) } else { (13, 9) };
        let defs = request(
            &mut s,
            id,
            "textDocument/definition",
            json!({ "textDocument": doc(), "position": at(line, ch) }),
        );
        let arr = defs.as_array().unwrap();
        assert!(!arr.is_empty(), "no definition for {method}");
        assert_eq!(arr[0]["range"]["start"]["line"], if method == "helper" { 6 } else { 0 }, "{method} decl line");
    }
}

#[test]
fn references_finds_declaration_and_uses() {
    let mut s = Server::new();
    open(&mut s);
    let refs = request(
        &mut s,
        9,
        "textDocument/references",
        json!({
            "textDocument": doc(),
            "position": at(6, 4),
            "context": { "includeDeclaration": true }
        }),
    );
    let lines: Vec<u64> = refs
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["range"]["start"]["line"].as_u64().unwrap())
        .collect();
    assert_eq!(lines.len(), 2, "expected decl + one use: {lines:?}");
    assert!(lines.contains(&6) && lines.contains(&12), "{lines:?}");

    // Excluding the declaration leaves only the use.
    let refs = request(
        &mut s,
        10,
        "textDocument/references",
        json!({
            "textDocument": doc(),
            "position": at(6, 4),
            "context": { "includeDeclaration": false }
        }),
    );
    assert_eq!(refs.as_array().unwrap().len(), 1);
}

#[test]
fn rename_rewrites_every_occurrence() {
    let mut s = Server::new();
    open(&mut s);
    let edit = request(
        &mut s,
        11,
        "textDocument/rename",
        json!({
            "textDocument": doc(),
            "position": at(6, 4),
            "newName": "doubled"
        }),
    );
    let changes = edit["file:///buffer.sn"]["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 2, "expected two edits");
    for c in changes {
        assert_eq!(c["newText"], json!("doubled"));
    }

    // Invalid identifiers are rejected.
    let err = request(
        &mut s,
        12,
        "textDocument/rename",
        json!({
            "textDocument": doc(),
            "position": at(6, 4),
            "newName": "9bad name"
        }),
    );
    assert!(
        err["message"].as_str().unwrap().contains("not a valid"),
        "expected an invalid-identifier error"
    );

    // Builtins are protected.
    let err = request(
        &mut s,
        13,
        "textDocument/rename",
        json!({
            "textDocument": doc(),
            "position": at(12, 6),
            "newName": "output"
        }),
    );
    assert!(
        err["message"].as_str().unwrap().contains("builtin"),
        "expected a builtin-rename error: {err}"
    );
}

#[test]
fn document_symbols_are_structured() {
    let mut s = Server::new();
    open(&mut s);
    let syms = request(&mut s, 14, "textDocument/documentSymbol", json!({ "textDocument": doc() }));
    let arr = syms.as_array().unwrap();
    let names: Vec<&str> = arr.iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["Dog", "helper", "main"]);

    let dog = &arr[0];
    assert_eq!(dog["kind"], json!(5), "blueprint symbol kind");
    assert!(dog["detail"].as_str().unwrap().contains("blueprint Dog"));
    let methods = dog["children"].as_array().unwrap();
    assert_eq!(methods[0]["name"], json!("speak"));
}

#[test]
fn diagnostics_report_type_errors() {
    let bad = "fn main() {\n    int x = \"not an int\"\n    print(y)\n}\n";
    let mut s = Server::new();
    s.handle(
        &json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": "file:///buffer.sn", "text": bad } }
        })
        .to_string(),
    );
    let diags = s.diagnostics(bad);
    let arr = diags.as_array().unwrap();
    assert!(!arr.is_empty(), "expected diagnostics for a broken file");
    assert!(arr.iter().any(|d| d["message"]
        .as_str()
        .unwrap()
        .contains("unknown name 'y'")));

    // A clean file produces none.
    assert!(s.diagnostics(SRC).as_array().unwrap().is_empty());
}
