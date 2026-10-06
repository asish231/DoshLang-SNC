use std::fs;
use std::path::Path;

pub fn format_file(path: &Path) -> Result<String, String> {
    let src = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(format_source(&src))
}

pub fn format_source(src: &str) -> String {
    let mut out = String::new();
    let mut depth: i32 = 0;
    for raw in src.lines() {
        let trimmed = raw.trim_end();
        if trimmed.is_empty() {
            out.push('\n');
            continue;
        }
        let line = trimmed.trim_start().to_string();
        if line.starts_with('}') {
            depth = (depth - 1).max(0);
        }
        let indent = "    ".repeat(depth as usize);
        out.push_str(&indent);
        out.push_str(&line);
        out.push('\n');
        for ch in trimmed.chars() {
            if ch == '{' {
                depth += 1;
            } else if ch == '}' {
                depth = (depth - 1).max(0);
            }
        }
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}
