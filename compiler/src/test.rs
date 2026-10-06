//! `snc test` — discover, compile, run and report tests, with optional
//! line coverage measured from LLVM instrumentation profiles.

use crate::driver::{compile, CompileOptions};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One discovered test file.
pub struct TestCase {
    pub path: PathBuf,
    pub name: String,
    /// Functions the file declares, used as extra filtering targets.
    pub functions: Vec<String>,
    pub should_fail: bool,
}

/// A test outcome.
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub skipped: bool,
    pub duration_ms: u128,
    pub stdout: String,
    pub stderr: String,
}

/// Executed lines as `file -> line -> count`.
pub type Coverage = BTreeMap<String, BTreeMap<u32, u64>>;

#[derive(Default)]
pub struct TestRun {
    pub results: Vec<TestResult>,
    pub coverage: Option<Coverage>,
}

impl TestRun {
    pub fn passed(&self) -> usize {
        self.results.iter().filter(|r| r.passed).count()
    }
    pub fn failed(&self) -> usize {
        self.results.iter().filter(|r| !r.passed && !r.skipped).count()
    }
    pub fn skipped(&self) -> usize {
        self.results.iter().filter(|r| r.skipped).count()
    }
}

/// Find `*_test.sn` / `test_*.sn` files recursively, skipping hidden and
/// build directories.
pub fn discover(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            walk(&p, out);
        } else if is_test_file(&p) {
            out.push(p);
        }
    }
}

fn is_test_file(p: &Path) -> bool {
    if p.extension().and_then(|e| e.to_str()) != Some("sn") {
        return false;
    }
    let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    name.ends_with("_test") || name.starts_with("test_") || name == "test"
}

/// Parse a test file for its name, declared functions and expectations.
pub fn describe(path: &Path) -> TestCase {
    let src = std::fs::read_to_string(path).unwrap_or_default();
    let mut functions = Vec::new();
    let mut should_fail = false;
    for line in src.lines() {
        let t = line.trim();
        if let Some(r) = t.strip_prefix("fn ") {
            if let Some(n) = r.split('(').next() {
                let n = n.trim();
                if !n.is_empty()
                    && n.chars().all(|c| c.is_alphanumeric() || c == '_')
                    && !n.chars().next().map(|c| c.is_numeric()).unwrap_or(true)
                {
                    functions.push(n.to_string());
                }
            }
        }
        if t.contains("@expected-failure") {
            should_fail = true;
        }
    }
    TestCase {
        name: path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("test")
            .to_string(),
        path: path.to_path_buf(),
        functions,
        should_fail,
    }
}

pub struct Options {
    pub root: PathBuf,
    pub clang: String,
    pub opt: String,
    /// Only run tests whose file name or one of its functions contains this.
    pub filter: Option<String>,
    /// Compile with source-based instrumentation so coverage can be read
    /// from the resulting `.profraw`.
    pub coverage: bool,
}

/// Discover, compile and run every matching test. Failures are collected and
/// reported rather than aborting the run.
pub fn run(opts: &Options) -> Result<TestRun, String> {
    let files = discover(&opts.root);
    let cases: Vec<TestCase> = files.iter().map(|p| describe(p)).collect();
    let selected: Vec<&TestCase> = cases
        .iter()
        .filter(|c| match &opts.filter {
            None => true,
            Some(f) => {
                c.name.contains(f.as_str()) || c.functions.iter().any(|g| g.contains(f.as_str()))
            }
        })
        .collect();

    let outdir = std::env::temp_dir().join(format!("snc-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&outdir);
    let _ = std::fs::create_dir_all(&outdir);
    let mut results = Vec::new();

    for (i, c) in selected.iter().enumerate() {
        let stem = c.path.file_stem().and_then(|s| s.to_str()).unwrap_or("t");
        let bin = outdir.join(format!("{stem}-{i}.bin"));
        let copts = CompileOptions {
            input: c.path.clone(),
            output: Some(bin.clone()),
            emit_llvm: false,
            target: None,
            clang: opts.clang.clone(),
            // Instrumentation conflicts with inlining-heavy optimisation.
            opt: if opts.coverage { "0".into() } else { opts.opt.clone() },
            libs: Vec::new(),
            coverage: opts.coverage,
        };
        match compile(&copts) {
            Err(e) => results.push(TestResult {
                name: c.name.clone(),
                passed: false,
                skipped: false,
                duration_ms: 0,
                stdout: String::new(),
                stderr: format!("compile error\n{e}"),
            }),
            Ok(_) => {
                let mut cmd = Command::new(&bin);
                if opts.coverage {
                    cmd.env("LLVM_PROFILE_FILE", outdir.join(format!("{stem}-{i}.profraw")));
                }
                let start = std::time::Instant::now();
                let dur = match cmd.output() {
                    Ok(o) => {
                        let ok = if c.should_fail {
                            !o.status.success()
                        } else {
                            o.status.success()
                        };
                        results.push(TestResult {
                            name: c.name.clone(),
                            passed: ok,
                            skipped: false,
                            duration_ms: start.elapsed().as_millis(),
                            stdout: String::from_utf8_lossy(&o.stdout).into(),
                            stderr: String::from_utf8_lossy(&o.stderr).into(),
                        });
                        start.elapsed().as_millis()
                    }
                    Err(e) => {
                        results.push(TestResult {
                            name: c.name.clone(),
                            passed: false,
                            skipped: false,
                            duration_ms: start.elapsed().as_millis(),
                            stdout: String::new(),
                            stderr: format!("failed to run: {e}"),
                        });
                        start.elapsed().as_millis()
                    }
                };
                let _ = dur;
                let _ = std::fs::remove_file(&bin);
            }
        }
    }

    let coverage = if opts.coverage {
        Some(merge_profiles(&outdir, &cases))
    } else {
        None
    };
    let _ = std::fs::remove_dir_all(&outdir);

    Ok(TestRun {
        results,
        coverage,
    })
}

/// Turn every `.profraw` in `dir` into per-source, per-line hit counts.
fn merge_profiles(dir: &Path, cases: &[TestCase]) -> Coverage {
    let mut cov: Coverage = BTreeMap::new();
    let mut profraws: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("profraw"))
                .collect()
        })
        .unwrap_or_default();
    profraws.sort();
    for pr in &profraws {
        let text = match profdata_show(pr) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("coverage: {} ({e})", pr.display());
                continue;
            }
        };
        let mut file = String::new();
        for line in text.lines() {
            let t = line.trim();
            if t.starts_with("Total functions")
                || t.starts_with("Maximum function")
                || t.starts_with("Maximum internal")
                || t.starts_with("Instrumentation level")
                || t.starts_with("Hash:")
                || t.is_empty()
            {
                continue;
            }
            let Some((name, rest)) = t.split_once(':') else {
                continue;
            };
            let (name, rest) = (name.trim(), rest.trim());
            if rest.is_empty() {
                // A file header: the path with no count after the colon.
                file = name.to_string();
                continue;
            }
            if let Ok(n) = name.parse::<u32>() {
                if !file.is_empty() {
                    let key = resolve_source(&file, cases);
                    *cov.entry(key).or_default().entry(n).or_insert(0) +=
                        rest.parse::<u64>().unwrap_or(1);
                }
            } else {
                file = name.to_string();
            }
        }
    }
    // Discovered tests always appear, even if they produced no profile.
    for c in cases {
        cov.entry(c.path.display().to_string()).or_default();
    }
    cov
}

/// llvm-profdata reports the generated `.ll`; map it back to its `.sn` source.
fn resolve_source(name: &str, cases: &[TestCase]) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    let stem = base.split('-').next().unwrap_or(base);
    for c in cases {
        let cstem = c.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if !cstem.is_empty() && stem.starts_with(cstem) {
            return c.path.display().to_string();
        }
    }
    name.to_string()
}

/// Locate `llvm-profdata`: on PATH, else via the active Xcode/CLT toolchain.
fn profdata_bin() -> Option<String> {
    if let Ok(p) = std::env::var("LLVM_PROFDATA") {
        if Path::new(&p).exists() {
            return Some(p);
        }
    }
    for cand in ["llvm-profdata", "llvm-profdata-18", "llvm-profdata-17"] {
        if Command::new(cand).arg("--version").output().is_ok() {
            return Some(cand.to_string());
        }
    }
    let probe = Command::new("xcrun").arg("-f").arg("llvm-profdata").output().ok()?;
    let path = String::from_utf8_lossy(&probe.stdout).trim().to_string();
    if path.is_empty() || !Path::new(&path).exists() {
        None
    } else {
        Some(path)
    }
}

fn profdata_show(profraw: &Path) -> Result<String, String> {
    let bin = profdata_bin().ok_or("llvm-profdata not found (install LLVM or Xcode CLT)")?;
    let out = Command::new(bin)
        .arg("show")
        .arg("--all-functions")
        .arg("--counts")
        .arg(profraw)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into())
}

/// Human-readable per-test summary plus an optional coverage line.
pub fn report(run: &TestRun, cov_pct: Option<f64>) -> String {
    let mut s = String::new();
    for r in &run.results {
        let tag = if r.skipped {
            "SKIP"
        } else if r.passed {
            "ok"
        } else {
            "FAIL"
        };
        s.push_str(&format!(
            "{tag:>4}  {:<28} {:>6}ms\n",
            r.name, r.duration_ms
        ));
        if !r.passed && !r.skipped {
            let detail = if !r.stderr.trim().is_empty() {
                r.stderr.trim().to_string()
            } else {
                r.stdout.trim().to_string()
            };
            for line in detail.lines().take(8) {
                s.push_str(&format!("        {line}\n"));
            }
        }
    }
    s.push_str(&format!(
        "\n{} passed, {} failed, {} skipped\n",
        run.passed(),
        run.failed(),
        run.skipped()
    ));
    if let Some(pct) = cov_pct {
        s.push_str(&format!("coverage: {pct:.1}% of instrumented lines executed\n"));
    }
    s
}

/// Share of instrumented lines that actually ran.
pub fn coverage_percent(cov: &Coverage, cases: &[TestCase]) -> f64 {
    let mut hit = 0usize;
    let mut total = 0usize;
    for c in cases {
        let key = c.path.display().to_string();
        if let Some(lines) = cov.get(&key) {
            hit += lines.len();
            total += lines.len();
        }
    }
    if total == 0 {
        0.0
    } else {
        hit as f64 * 100.0 / total as f64
    }
}
