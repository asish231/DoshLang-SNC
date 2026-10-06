//! Incremental build cache: content-keyed storage for compiled outputs.
//!
//! `driver::compile` fingerprints every input that can change the result —
//! all loaded `.sn` sources, the target triple, optimisation level, link
//! libraries, coverage mode, the clang binary identity and `runtime.c` bytes.
//! On a hit the cached LLVM IR and linked binary are reused instead of
//! recompiling and relinking.
//!
//! The cache lives in `$SNC_CACHE_DIR`, defaulting to `snc-cache` inside the
//! platform temp dir. Set `SNC_CACHE=0` to disable it (codegen debugging).

use crate::span::SourceFile;
use std::path::{Path, PathBuf};

/// FNV-1a 64. Small and, unlike `DefaultHasher`, stable across processes.
fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn mix(mut h: u64, s: &str) -> u64 {
    h = fnv(h, s.as_bytes());
    fnv(h, &[0xff])
}

/// Minimal `PATH` lookup for a bare binary name.
fn which(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let p = dir.join(name);
        #[cfg(windows)]
        let p = if p.extension().is_none() {
            p.with_extension("exe")
        } else {
            p
        };
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Identity of the clang binary: the name plus the resolved path's
/// size/mtime when available, so toolchain upgrades invalidate the cache.
pub fn clang_id(clang: &str) -> String {
    let mut id = String::from("clang=");
    id.push_str(clang);
    let meta = if Path::new(clang).components().count() > 1 {
        std::fs::metadata(clang).ok()
    } else {
        which(clang).and_then(|p| std::fs::metadata(p).ok())
    };
    if let Some(m) = meta {
        id.push_str(&format!(";len={};mtime={:?}", m.len(), m.modified().ok()));
    }
    id
}

/// Identity of the running compiler binary, so codegen changes invalidate
/// the cache even when the package version has not been bumped.
pub fn compiler_id() -> String {
    let mut id = String::from("snc=");
    if let Ok(exe) = std::env::current_exe() {
        id.push_str(&exe.display().to_string());
        if let Ok(m) = std::fs::metadata(&exe) {
            id.push_str(&format!(";len={};mtime={:?}", m.len(), m.modified().ok()));
        }
    }
    id
}

/// Stable fingerprint over everything that can change the build output.
pub fn fingerprint(
    files: &[SourceFile],
    triple: &str,
    opt: &str,
    libs: &[String],
    coverage: bool,
    clang: &str,
    runtime_bytes: &[u8],
) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    h = mix(h, env!("CARGO_PKG_VERSION"));
    h = mix(h, &compiler_id());
    h = mix(h, triple);
    h = mix(h, opt);
    h = mix(h, if coverage { "cov=1" } else { "cov=0" });
    h = mix(h, &clang_id(clang));
    h = fnv(h, runtime_bytes);
    h = fnv(h, &[0xff]);
    for f in files {
        h = mix(h, &f.id.to_string());
        h = mix(h, &f.path);
        h = fnv(h, f.src.as_bytes());
        h = fnv(h, &[0xff]);
    }
    for lib in libs {
        h = mix(h, lib);
    }
    format!("{h:016x}")
}

/// Cache directory: `$SNC_CACHE_DIR`, else the platform temp dir.
pub fn cache_dir() -> PathBuf {
    if let Ok(d) = std::env::var("SNC_CACHE_DIR") {
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    std::env::temp_dir().join("snc-cache")
}

/// `SNC_CACHE=0|off|no|false` disables the cache.
pub fn cache_enabled() -> bool {
    !matches!(
        std::env::var("SNC_CACHE")
            .map(|v| v.to_ascii_lowercase())
            .as_deref(),
        Ok("0" | "off" | "no" | "false")
    )
}

pub struct Cached {
    pub llvm_ir: String,
    pub binary: Option<Vec<u8>>,
}

/// Look up a fingerprint. For `--emit-llvm` builds only the IR is needed.
pub fn get(fp: &str, need_binary: bool) -> Option<Cached> {
    let dir = cache_dir();
    let llvm_ir = std::fs::read_to_string(dir.join(format!("{fp}.ll"))).ok()?;
    let binary = if need_binary {
        Some(std::fs::read(dir.join(format!("{fp}.bin"))).ok()?)
    } else {
        None
    };
    Some(Cached { llvm_ir, binary })
}

/// Store an entry atomically, then prune the oldest entries past the cap.
pub fn put(fp: &str, llvm_ir: &str, binary: Option<&[u8]>) {
    let dir = cache_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    write_atomic(&dir.join(format!("{fp}.ll")), llvm_ir.as_bytes());
    if let Some(b) = binary {
        write_atomic(&dir.join(format!("{fp}.bin")), b);
    }
    prune(&dir);
}

fn write_atomic(path: &Path, bytes: &[u8]) {
    let tmp = path.with_extension(format!(
        "{}.tmp-{}",
        path.extension().and_then(|e| e.to_str()).unwrap_or("dat"),
        std::process::id()
    ));
    if std::fs::write(&tmp, bytes).is_err() {
        return;
    }
    let _ = std::fs::rename(&tmp, path);
}

/// Keep the newest 256 entries (by mtime); drop older `.bin`/`.ll` pairs.
fn prune(dir: &Path) {
    const KEEP: usize = 256;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut bins: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    let mut lone_ll: Vec<PathBuf> = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        match p.extension().and_then(|x| x.to_str()) {
            Some("bin") => {
                let mtime = e.metadata().and_then(|m| m.modified()).ok();
                if let Some(t) = mtime {
                    bins.push((t, p));
                }
            }
            Some("ll") => {
                let stem = p.file_stem().unwrap_or_default().to_owned();
                if !dir.join(&stem).with_extension("bin").exists() {
                    lone_ll.push(p);
                }
            }
            _ => {}
        }
    }
    bins.sort_by_key(|(t, _)| *t);
    bins.reverse();
    for (_, p) in bins.into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(p.with_extension("ll"));
        let _ = std::fs::remove_file(&p);
    }
    // Lone IR entries (from `--emit-llvm` builds) share the same budget.
    if lone_ll.len() > KEEP {
        let drop = lone_ll.len() - KEEP;
        for p in lone_ll.into_iter().take(drop) {
            let _ = std::fs::remove_file(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<SourceFile> {
        vec![SourceFile::new(
            0,
            "a.sn".to_string(),
            "fn main() {\n}\n".to_string(),
        )]
    }

    fn fp(
        files: &[SourceFile],
        opt: &str,
        libs: &[String],
        coverage: bool,
        runtime: &[u8],
    ) -> String {
        fingerprint(files, "triple", opt, libs, coverage, "clang", runtime)
    }

    #[test]
    fn fingerprint_is_stable() {
        let a = fp(&files(), "2", &[], false, b"rt");
        let b = fp(&files(), "2", &[], false, b"rt");
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn fingerprint_covers_sources_options_and_runtime() {
        let base = fp(&files(), "2", &[], false, b"rt");
        let mut changed = files();
        changed[0] = SourceFile::new(
            0,
            "a.sn".to_string(),
            "fn main() {\n    print(1)\n}\n".to_string(),
        );
        assert_ne!(base, fp(&changed, "2", &[], false, b"rt"));
        assert_ne!(base, fp(&files(), "0", &[], false, b"rt"));
        assert_ne!(base, fp(&files(), "2", &["m".to_string()], false, b"rt"));
        assert_ne!(base, fp(&files(), "2", &[], true, b"rt"));
        assert_ne!(base, fp(&files(), "2", &[], false, b"rt2"));
    }
}
