mod c;
mod go;
mod js;
mod python;

pub fn translate(from: &str, src: &str, filename: &str) -> Result<String, String> {
    let lang = normalize_lang(from, filename)?;
    let body = match lang.as_str() {
        "python" => python::translate(src)?,
        "js" => js::translate(src)?,
        "go" => go::translate(src)?,
        "c" => c::translate(src)?,
        other => {
            return Err(format!(
                "unknown language '{other}'; use python, js, go, or c"
            ))
        }
    };
    Ok(format!(
        "// translated from {filename} ({lang})\n// subset translator — not a full compiler of {lang}\n\n{body}"
    ))
}

fn normalize_lang(from: &str, filename: &str) -> Result<String, String> {
    let f = from.trim().to_ascii_lowercase();
    if f == "auto" || f.is_empty() {
        return match PathExt(filename).ext() {
            "py" => Ok("python".into()),
            "js" | "mjs" => Ok("js".into()),
            "go" => Ok("go".into()),
            "c" | "h" => Ok("c".into()),
            _ => Err("cannot infer language; pass --from python|js|go|c".into()),
        };
    }
    Ok(match f.as_str() {
        "python" | "py" => "python".into(),
        "js" | "javascript" => "js".into(),
        "go" | "golang" => "go".into(),
        "c" | "clike" => "c".into(),
        other => other.to_string(),
    })
}

struct PathExt<'a>(&'a str);
impl PathExt<'_> {
    fn ext(&self) -> &str {
        self.0.rsplit('.').next().unwrap_or("")
    }
}

pub(crate) fn indent(s: &str, n: usize) -> String {
    let pad = "    ".repeat(n);
    s.lines()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn escape_sn_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}
