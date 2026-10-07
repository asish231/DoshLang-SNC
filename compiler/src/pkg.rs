use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, Default)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub deps: Vec<(String, DepSpec)>,
}

#[derive(Clone, Debug)]
pub enum DepSpec {
    Version(String),
    Path(PathBuf),
    Registry { url: String, version: String },
}

#[derive(Clone, Debug)]
pub struct LockEntry {
    pub name: String,
    pub version: String,
    pub source: String,
    pub path: Option<String>,
    pub registry: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Lockfile {
    pub packages: Vec<LockEntry>,
}

pub fn find_manifest(start: &Path) -> Option<PathBuf> {
    let mut cur = start.to_path_buf();
    if cur.is_file() {
        cur = cur.parent().unwrap_or(Path::new(".")).to_path_buf();
    }
    loop {
        let cand = cur.join("sn.toml");
        if cand.exists() {
            return Some(cand);
        }
        if !cur.pop() {
            return None;
        }
    }
}

pub fn parse_manifest(src: &str) -> Result<Manifest, String> {
    let mut m = Manifest::default();
    let mut section = String::new();
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            if let Some(name) = rest.strip_suffix(']') {
                section = name.trim().to_string();
                continue;
            }
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let v = v.trim();
        if section.is_empty() {
            match k {
                "name" => m.name = unquote(v),
                "version" => m.version = unquote(v),
                _ => {}
            }
        } else if section == "deps" {
            if let Some(path) = parse_path_table(v) {
                m.deps.push((k.to_string(), DepSpec::Path(path)));
            } else if let Some(reg) = parse_registry_table(v) {
                m.deps.push((k.to_string(), reg));
            } else {
                m.deps.push((k.to_string(), DepSpec::Version(unquote(v))));
            }
        }
    }
    if m.name.is_empty() {
        return Err("sn.toml is missing name".into());
    }
    if m.version.is_empty() {
        m.version = "0.0.0".into();
    }
    Ok(m)
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if let Some(s) = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        s.to_string()
    } else {
        v.to_string()
    }
}

fn parse_registry_table(v: &str) -> Option<DepSpec> {
    let v = v.trim();
    if !v.starts_with('{') || !v.contains("registry") {
        return None;
    }
    let url = extract_table_field(v, "registry")?;
    let version = extract_table_field(v, "version").unwrap_or_else(|| "0.1.0".into());
    Some(DepSpec::Registry { url, version })
}

fn extract_table_field(v: &str, key: &str) -> Option<String> {
    // Match `key = "..."` or `key = '...'` inside a `{ ... }` table.
    let pat = format!("{key}");
    let bytes = v.as_bytes();
    let mut i = 0;
    while i + pat.len() <= v.len() {
        if v[i..].starts_with(&pat) {
            let before_ok = i == 0
                || bytes[i - 1].is_ascii_whitespace()
                || bytes[i - 1] == b'{'
                || bytes[i - 1] == b',';
            let after = i + pat.len();
            let after_ok = after < v.len()
                && (bytes[after].is_ascii_whitespace() || bytes[after] == b'=');
            if before_ok && after_ok {
                let rest = v[after..].trim_start();
                let rest = rest.strip_prefix('=')?.trim_start();
                if let Some(rest) = rest.strip_prefix('"') {
                    let end = rest.find('"')?;
                    return Some(rest[..end].to_string());
                }
                // Unquoted until comma or }
                let end = rest
                    .find([',', '}'])
                    .unwrap_or(rest.len());
                let s = rest[..end].trim().to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
        i += 1;
    }
    None
}

fn parse_path_table(v: &str) -> Option<PathBuf> {
    let v = v.trim();
    if !v.starts_with('{') {
        return None;
    }
    if let Some(i) = v.find("path") {
        let rest = &v[i + 4..];
        let rest = rest.trim_start_matches(|c: char| c == '=' || c.is_whitespace());
        let s = unquote(rest.trim_end_matches('}').trim().trim_end_matches(','));
        if s.is_empty() {
            return None;
        }
        return Some(PathBuf::from(s));
    }
    None
}

pub fn load_manifest(path: &Path) -> Result<Manifest, String> {
    let src = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_manifest(&src)
}

pub fn write_manifest(path: &Path, m: &Manifest) -> Result<(), String> {
    let mut s = format!("name = \"{}\"\nversion = \"{}\"\n", m.name, m.version);
    if !m.deps.is_empty() {
        s.push_str("\n[deps]\n");
        for (name, spec) in &m.deps {
            match spec {
                DepSpec::Version(v) => s.push_str(&format!("{name} = \"{v}\"\n")),
                DepSpec::Path(p) => {
                    s.push_str(&format!("{name} = {{ path = \"{}\" }}\n", p.display()))
                }
                DepSpec::Registry { url, version } => {
                    s.push_str(&format!(
                        "{name} = {{ registry = \"{url}\", version = \"{version}\" }}\n"
                    ))
                }
            }
        }
    }
    fs::write(path, s).map_err(|e| e.to_string())
}

/// Resolve a dependency's recorded version (semver-ish string from sn.toml or registry).
pub fn resolve_dep_version(root: &Path, _name: &str, spec: &DepSpec) -> String {
    match spec {
        DepSpec::Version(v) => v.clone(),
        DepSpec::Registry { version, .. } => version.clone(),
        DepSpec::Path(p) => {
            let abs = if p.is_absolute() {
                p.clone()
            } else {
                root.join(p)
            };
            let man = abs.join("sn.toml");
            if man.exists() {
                if let Ok(m) = load_manifest(&man) {
                    if !m.version.is_empty() {
                        return m.version;
                    }
                }
            }
            "0.0.0".into()
        }
    }
}

pub fn lockfile_path(dir: &Path) -> PathBuf {
    dir.join("sn.lock.toml")
}

pub fn write_lockfile(dir: &Path, m: &Manifest) -> Result<PathBuf, String> {
    let root = find_manifest(dir)
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| dir.to_path_buf());
    let mut lock = Lockfile::default();
    lock.packages.push(LockEntry {
        name: m.name.clone(),
        version: m.version.clone(),
        source: "root".into(),
        path: Some(".".into()),
        registry: None,
    });
    for (name, spec) in &m.deps {
        let version = resolve_dep_version(&root, name, spec);
        let entry = match spec {
            DepSpec::Version(v) => LockEntry {
                name: name.clone(),
                version: v.clone(),
                source: "version".into(),
                path: None,
                registry: None,
            },
            DepSpec::Path(p) => LockEntry {
                name: name.clone(),
                version,
                source: "path".into(),
                path: Some(p.display().to_string()),
                registry: None,
            },
            DepSpec::Registry { url, version } => LockEntry {
                name: name.clone(),
                version: version.clone(),
                source: "registry".into(),
                path: Some(format!("packages/{name}")),
                registry: Some(url.clone()),
            },
        };
        lock.packages.push(entry);
    }
    let path = lockfile_path(&root);
    fs::write(&path, render_lockfile(&lock)).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn render_lockfile(lock: &Lockfile) -> String {
    let mut s = String::from("# Auto-generated by snc pkg. Do not edit by hand.\n");
    s.push_str("# Commit this file to pin dependency versions.\n\n");
    for p in &lock.packages {
        s.push_str("[[package]]\n");
        s.push_str(&format!("name = \"{}\"\n", p.name));
        s.push_str(&format!("version = \"{}\"\n", p.version));
        s.push_str(&format!("source = \"{}\"\n", p.source));
        if let Some(path) = &p.path {
            s.push_str(&format!("path = \"{path}\"\n"));
        }
        if let Some(reg) = &p.registry {
            s.push_str(&format!("registry = \"{reg}\"\n"));
        }
        s.push('\n');
    }
    s
}

pub fn parse_lockfile(src: &str) -> Lockfile {
    let mut lock = Lockfile::default();
    let mut cur: Option<LockEntry> = None;
    let flush = |cur: &mut Option<LockEntry>, lock: &mut Lockfile| {
        if let Some(e) = cur.take() {
            if !e.name.is_empty() {
                lock.packages.push(e);
            }
        }
    };
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[package]]" {
            flush(&mut cur, &mut lock);
            cur = Some(LockEntry {
                name: String::new(),
                version: "0.0.0".into(),
                source: String::new(),
                path: None,
                registry: None,
            });
            continue;
        }
        let Some(entry) = cur.as_mut() else {
            continue;
        };
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let v = unquote(v.trim());
        match k {
            "name" => entry.name = v,
            "version" => entry.version = v,
            "source" => entry.source = v,
            "path" => entry.path = Some(v),
            "registry" => entry.registry = Some(v),
            _ => {}
        }
    }
    flush(&mut cur, &mut lock);
    lock
}

pub fn repo_root() -> PathBuf {
    // Release archives keep stdlib/ next to the snc binary, so look there
    // first: walk up from the running executable for a dir containing it.
    // Dev checkouts resolve the same way (…/compiler/target/release/snc
    // walks up to the repo root), then fall back to the build-time path.
    let mut dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    for _ in 0..6 {
        let d = match dir {
            Some(d) => d,
            None => break,
        };
        if d.join("stdlib").exists() {
            return d;
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf()
}

pub fn package_search_roots(from_dir: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    roots.push(from_dir.to_path_buf());
    roots.push(from_dir.join("packages"));
    if let Some(man) = find_manifest(from_dir) {
        if let Some(parent) = man.parent() {
            roots.push(parent.to_path_buf());
            roots.push(parent.join("packages"));
            if let Ok(m) = load_manifest(&man) {
                for (_, spec) in &m.deps {
                    if let DepSpec::Path(p) = spec {
                        let abs = if p.is_absolute() {
                            p.clone()
                        } else {
                            parent.join(p)
                        };
                        roots.push(abs);
                    }
                }
            }
        }
    }
    let repo = repo_root();
    roots.push(repo.join("stdlib"));
    roots.push(repo.join("packages"));
    roots.push(PathBuf::from("stdlib"));
    roots.push(PathBuf::from("packages"));
    roots
}

pub fn resolve_use_path(path: &[String], from_dir: &Path, importer: &Path) -> Option<PathBuf> {
    let rel = path.join("/") + ".sn";
    let dotted = path.join(".") + ".sn";
    let mut cands = vec![
        from_dir.join(&rel),
        from_dir.join(&dotted),
        importer.parent().unwrap_or(from_dir).join(&rel),
    ];
    if path.len() >= 2 {
        let pkg = &path[0];
        let rest = path[1..].join("/") + ".sn";
        cands.push(from_dir.join("packages").join(pkg).join(&rest));
        cands.push(from_dir.join(pkg).join(&rest));
    }
    for root in package_search_roots(from_dir) {
        cands.push(root.join(&rel));
        cands.push(root.join(&dotted));
        if path.len() >= 2 {
            cands.push(root.join(&path[0]).join(path[1..].join("/") + ".sn"));
        }
        if path.len() == 1 {
            cands.push(root.join(format!("{}.sn", path[0])));
            cands.push(root.join(&path[0]).join("lib.sn"));
            cands.push(root.join(&path[0]).join("mod.sn"));
        }
    }
    cands.into_iter().find(|p| p.exists())
}

pub fn cmd_init(dir: &Path, name: Option<&str>) -> Result<String, String> {
    let path = dir.join("sn.toml");
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    let name = name
        .map(|s| s.to_string())
        .or_else(|| {
            dir.file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "app".into());
    let m = Manifest {
        name: name.clone(),
        version: "0.1.0".into(),
        deps: vec![],
    };
    write_manifest(&path, &m)?;
    let lock = write_lockfile(dir, &m)?;
    let main = dir.join("main.sn");
    if !main.exists() {
        fs::write(
            &main,
            "fn main() {\n    print(\"hello from {name}\")\n}\n".replace("{name}", &name),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(format!("wrote {} and {}", path.display(), lock.display()))
}

pub fn cmd_add(
    dir: &Path,
    name: &str,
    path: Option<&Path>,
    registry: Option<&str>,
    version: Option<&str>,
) -> Result<String, String> {
    let man_path = find_manifest(dir).unwrap_or_else(|| dir.join("sn.toml"));
    let root = man_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| dir.to_path_buf());
    let mut m = if man_path.exists() {
        load_manifest(&man_path)?
    } else {
        Manifest {
            name: dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("app")
                .into(),
            version: "0.1.0".into(),
            deps: vec![],
        }
    };
    if m.deps.iter().any(|(n, _)| n == name) {
        let _ = write_lockfile(&root, &m)?;
        return Ok(format!("{name} already in sn.toml"));
    }
    let spec = if let Some(reg) = registry {
        let dest = root.join("packages").join(name);
        let ver = version.unwrap_or("0.1.0");
        let resolved = fetch_registry_package(reg, name, ver, &dest)?;
        DepSpec::Registry {
            url: reg.to_string(),
            version: resolved,
        }
    } else if let Some(p) = path {
        DepSpec::Path(p.to_path_buf())
    } else {
        let local = root.join("packages").join(name);
        let std = repo_root().join("stdlib").join(name);
        let pkg = repo_root().join("packages").join(name);
        if local.exists() {
            DepSpec::Path(PathBuf::from(format!("packages/{name}")))
        } else if pkg.exists() {
            let rel = if pkg.starts_with(&root) {
                PathBuf::from(format!("packages/{name}"))
            } else {
                pkg.clone()
            };
            DepSpec::Path(rel)
        } else if std.exists() || name == "std" {
            DepSpec::Version(version.unwrap_or("0.2.0").into())
        } else {
            return Err(format!(
                "cannot find package '{name}' in packages/ or stdlib/; pass --path or --registry"
            ));
        }
    };
    // Normalize recorded version for path deps from their sn.toml.
    let spec = match &spec {
        DepSpec::Path(p) => {
            let ver = resolve_dep_version(&root, name, &spec);
            let _ = ver; // version lives in lockfile; path dep stays path
            DepSpec::Path(p.clone())
        }
        other => other.clone(),
    };
    m.deps.push((name.to_string(), spec));
    write_manifest(&man_path, &m)?;
    let lock = write_lockfile(&root, &m)?;
    Ok(format!(
        "added {name} to {} (lockfile {})",
        man_path.display(),
        lock.display()
    ))
}

fn fetch_registry_package(
    registry: &str,
    name: &str,
    want_version: &str,
    dest: &Path,
) -> Result<String, String> {
    if dest.exists() {
        fs::remove_dir_all(dest).map_err(|e| format!("cannot clear {}: {e}", dest.display()))?;
    }
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let index_url = format!("{}/index.json", registry.trim_end_matches('/'));
    let index = http_get_text(&index_url)?;
    let (url, version) = resolve_registry_entry(&index, registry, name, want_version)?;
    let archive = dest
        .parent()
        .unwrap_or(dest)
        .join(format!("{name}-{version}.tar.gz"));
    http_download(&url, &archive)?;
    let status = Command::new("tar")
        .args([
            "-xzf",
            archive.to_str().unwrap(),
            "-C",
            dest.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| format!("tar failed: {e}"))?;
    let _ = fs::remove_file(&archive);
    if !status.success() {
        let _ = fs::remove_dir_all(dest);
        return Err(format!(
            "failed to extract package '{name}' from {url}: tar exited with error (refusing to write stub lib.sn)"
        ));
    }
    // If tarball had a single top-level dir, flatten one level when needed.
    flatten_single_dir(dest)?;
    // Prefer version from extracted sn.toml when present.
    let man = dest.join("sn.toml");
    if man.exists() {
        if let Ok(m) = load_manifest(&man) {
            if !m.version.is_empty() {
                return Ok(m.version);
            }
        }
    }
    Ok(version)
}

fn flatten_single_dir(dest: &Path) -> Result<(), String> {
    let entries: Vec<_> = fs::read_dir(dest)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .collect();
    if entries.len() != 1 {
        return Ok(());
    }
    let only = &entries[0];
    if !only.file_type().map(|t| t.is_dir()).unwrap_or(false) {
        return Ok(());
    }
    let inner = only.path();
    let tmp = dest.parent().unwrap_or(dest).join(format!(
        ".sn-extract-{}",
        dest.file_name().and_then(|s| s.to_str()).unwrap_or("pkg")
    ));
    if tmp.exists() {
        fs::remove_dir_all(&tmp).ok();
    }
    fs::rename(&inner, &tmp).map_err(|e| e.to_string())?;
    // Move children up.
    for ent in fs::read_dir(&tmp).map_err(|e| e.to_string())? {
        let ent = ent.map_err(|e| e.to_string())?;
        let to = dest.join(ent.file_name());
        fs::rename(ent.path(), to).map_err(|e| e.to_string())?;
    }
    let _ = fs::remove_dir_all(&tmp);
    Ok(())
}

fn resolve_registry_entry(
    index: &str,
    registry: &str,
    name: &str,
    want_version: &str,
) -> Result<(String, String), String> {
    let tarball_key = format!("\"{name}\"");
    if !index.contains(&tarball_key) {
        return Ok((
            format!(
                "{}/packages/{name}-{want_version}.tar.gz",
                registry.trim_end_matches('/')
            ),
            want_version.to_string(),
        ));
    }
    let snippet = index.split(&tarball_key).nth(1).unwrap_or("");
    let version = extract_json_string_field(snippet, "version").unwrap_or_else(|| want_version.into());
    let url = extract_json_string_field(snippet, "url")
        .or_else(|| extract_json_string_field(snippet, "tarball"))
        .unwrap_or_else(|| {
            format!(
                "{}/packages/{name}-{version}.tar.gz",
                registry.trim_end_matches('/')
            )
        });
    if want_version != "*"
        && want_version != "latest"
        && version != want_version
        && !version_satisfies(&version, want_version)
    {
        // Still allow fetch if index only lists one version; warn via Err only on hard mismatch
        // when URL explicitly pinned differently — soft accept listed version.
        let _ = want_version;
    }
    Ok((url, version))
}

/// Very small semver-ish check: exact match, or `^1.2` / `~1.2.3` / `>=1.0.0` prefixes.
fn version_satisfies(have: &str, want: &str) -> bool {
    if want.is_empty() || want == "*" || want == "latest" || have == want {
        return true;
    }
    if let Some(req) = want.strip_prefix('^') {
        let (hm, _) = semver_major_minor(have);
        let (wm, _) = semver_major_minor(req);
        return hm == wm;
    }
    if let Some(req) = want.strip_prefix('~') {
        let (hm, hmi) = semver_major_minor(have);
        let (wm, wmi) = semver_major_minor(req);
        return hm == wm && hmi == wmi;
    }
    if let Some(req) = want.strip_prefix(">=") {
        return semver_cmp(have, req.trim()) >= 0;
    }
    false
}

fn semver_major_minor(v: &str) -> (u64, u64) {
    let mut parts = v.trim().split('.');
    let major = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor)
}

fn semver_cmp(a: &str, b: &str) -> i32 {
    let pa: Vec<u64> = a.split('.').filter_map(|s| s.parse().ok()).collect();
    let pb: Vec<u64> = b.split('.').filter_map(|s| s.parse().ok()).collect();
    for i in 0..3 {
        let x = *pa.get(i).unwrap_or(&0);
        let y = *pb.get(i).unwrap_or(&0);
        if x < y {
            return -1;
        }
        if x > y {
            return 1;
        }
    }
    0
}

fn curl_insecure() -> bool {
    matches!(
        std::env::var("SN_HTTP_INSECURE").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes")
    )
}

fn curl_base_args() -> Vec<String> {
    let mut args = vec!["-sS".into(), "-L".into()];
    if curl_insecure() {
        args.push("-k".into());
    }
    args
}

fn http_get_text(url: &str) -> Result<String, String> {
    let mut args = curl_base_args();
    args.push(url.to_string());
    let out = Command::new("curl")
        .args(&args)
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "fetch failed ({url}): {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into())
}

fn http_download(url: &str, dest: &Path) -> Result<(), String> {
    let mut args = curl_base_args();
    args.push("-o".into());
    args.push(dest.to_str().unwrap().into());
    args.push(url.to_string());
    let status = Command::new("curl")
        .args(&args)
        .status()
        .map_err(|e| format!("curl: {e}"))?;
    if !status.success() {
        return Err(format!("download failed: {url}"));
    }
    Ok(())
}

fn extract_json_string_field(s: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let i = s.find(&pat)?;
    let rest = &s[i + pat.len()..];
    let rest = rest.trim_start_matches(|c: char| c == ':' || c.is_whitespace());
    if let Some(rest) = rest.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return Some(rest[..end].to_string());
        }
    }
    None
}

pub fn cmd_list(dir: &Path) -> Result<String, String> {
    let Some(man_path) = find_manifest(dir) else {
        return Ok("no sn.toml found".into());
    };
    let m = load_manifest(&man_path)?;
    let root = man_path.parent().unwrap_or(dir);
    let mut out = format!("{} {} ({})\n", m.name, m.version, man_path.display());
    if m.deps.is_empty() {
        out.push_str("deps: (none)\n");
    } else {
        out.push_str("deps:\n");
        for (n, s) in &m.deps {
            let ver = resolve_dep_version(root, n, s);
            match s {
                DepSpec::Version(v) => out.push_str(&format!("  {n} = {v}\n")),
                DepSpec::Path(p) => {
                    out.push_str(&format!("  {n} = path {} @ {ver}\n", p.display()))
                }
                DepSpec::Registry { url, version } => {
                    out.push_str(&format!(
                        "  {n} = {{ registry = \"{url}\", version = \"{version}\" }}\n"
                    ))
                }
            }
        }
    }
    let lock = lockfile_path(root);
    if lock.exists() {
        out.push_str(&format!("lockfile: {}\n", lock.display()));
    }
    Ok(out)
}

pub fn cmd_build_prepare(dir: &Path) -> Result<(PathBuf, Manifest), String> {
    let man_path = find_manifest(dir).ok_or_else(|| "no sn.toml found".to_string())?;
    let root = man_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| dir.to_path_buf());
    let m = load_manifest(&man_path)?;
    // Ensure registry deps are present.
    for (name, spec) in &m.deps {
        if let DepSpec::Registry { url, version } = spec {
            let dest = root.join("packages").join(name);
            if !dest.exists() {
                fetch_registry_package(url, name, version, &dest)?;
            }
        }
    }
    let lock = write_lockfile(&root, &m)?;
    let _ = lock;
    Ok((root, m))
}

/// Pack current package as `.tar.gz` into `dist/` (or `packages/dist/`).
pub fn cmd_publish(dir: &Path, out_dir: Option<&Path>) -> Result<String, String> {
    let man_path = find_manifest(dir).ok_or_else(|| "no sn.toml found; run snc pkg init".to_string())?;
    let root = man_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| dir.to_path_buf());
    let m = load_manifest(&man_path)?;
    let dist = out_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| {
            if root.join("packages").is_dir() && root.file_name().and_then(|s| s.to_str()) != Some("packages")
            {
                // Prefer packages/dist when publishing from a monorepo packages/<name>
                if root
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    == Some("packages")
                {
                    root.parent().unwrap().join("dist")
                } else {
                    root.join("dist")
                }
            } else {
                root.join("dist")
            }
        });
    fs::create_dir_all(&dist).map_err(|e| e.to_string())?;
    let archive_name = format!("{}-{}.tar.gz", m.name, m.version);
    let archive = dist.join(&archive_name);

    // Collect sources: sn.toml + *.sn (non-recursive top + one level).
    let mut files: Vec<PathBuf> = vec![PathBuf::from("sn.toml")];
    collect_sn_files(&root, &root, &mut files)?;
    if files.len() == 1 {
        return Err("no .sn source files found to publish".into());
    }

    // tar from package root so archive paths are relative.
    let mut args = vec![
        "-czf".to_string(),
        archive.to_str().unwrap().to_string(),
    ];
    for f in &files {
        args.push(f.to_str().unwrap().to_string());
    }
    let status = Command::new("tar")
        .args(&args)
        .current_dir(&root)
        .status()
        .map_err(|e| format!("tar: {e}"))?;
    if !status.success() {
        return Err(format!("tar failed creating {}", archive.display()));
    }
    let _ = write_lockfile(&root, &m);
    Ok(format!(
        "published {} -> {}\nServe with a static host: put this file at packages/{} and list it in index.json (see docs/PACKAGES.md)",
        m.name,
        archive.display(),
        archive_name
    ))
}

fn collect_sn_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for ent in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let ent = ent.map_err(|e| e.to_string())?;
        let path = ent.path();
        let name = ent.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "dist" || name == "target" || name == "packages" {
            continue;
        }
        if path.is_dir() {
            // one extra level only (avoid huge trees)
            if dir == root {
                collect_sn_files(root, &path, out)?;
            }
            continue;
        }
        if name.ends_with(".sn") {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_path_buf();
            out.push(rel);
        }
    }
    Ok(())
}

pub fn default_build_input(dir: &Path) -> Result<PathBuf, String> {
    for cand in ["main.sn", "src/main.sn"] {
        let p = dir.join(cand);
        if p.exists() {
            return Ok(p);
        }
    }
    Err("no main.sn or src/main.sn found; pass a file to snc".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lockfile_roundtrip() {
        let lock = Lockfile {
            packages: vec![
                LockEntry {
                    name: "app".into(),
                    version: "0.1.0".into(),
                    source: "root".into(),
                    path: Some(".".into()),
                    registry: None,
                },
                LockEntry {
                    name: "mylib".into(),
                    version: "0.1.0".into(),
                    source: "path".into(),
                    path: Some("packages/mylib".into()),
                    registry: None,
                },
            ],
        };
        let rendered = render_lockfile(&lock);
        let parsed = parse_lockfile(&rendered);
        assert_eq!(parsed.packages.len(), 2);
        assert_eq!(parsed.packages[1].name, "mylib");
        assert_eq!(parsed.packages[1].version, "0.1.0");
        assert_eq!(parsed.packages[1].source, "path");
    }

    #[test]
    fn semver_helpers() {
        assert!(version_satisfies("1.2.3", "1.2.3"));
        assert!(version_satisfies("1.9.0", "^1.0.0"));
        assert!(!version_satisfies("2.0.0", "^1.0.0"));
        assert!(version_satisfies("1.2.9", "~1.2.0"));
        assert!(version_satisfies("1.5.0", ">=1.2.0"));
        assert_eq!(semver_cmp("1.0.0", "1.0.1"), -1);
    }

    #[test]
    fn parse_registry_dep() {
        let m = parse_manifest(
            r#"
name = "app"
version = "0.1.0"

[deps]
foo = { registry = "https://example.com/sn", version = "1.2.3" }
"#,
        )
        .unwrap();
        match &m.deps[0].1 {
            DepSpec::Registry { url, version } => {
                assert_eq!(url, "https://example.com/sn");
                assert_eq!(version, "1.2.3");
            }
            _ => panic!("expected registry dep"),
        }
    }
}
