//! `snc test` — discover, compile, run and report tests, with optional
//! statement coverage measured from `sn_cov_hit` counters.

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
    /// Lines the compiler instrumented, per file.
    pub covered_declared: Option<Coverage>,
    /// Lines that actually ran, per file.
    pub covered_lines: Option<Coverage>,
}

impl TestRun {
    pub fn passed(&self) -> usize {
        self.results.iter().filter(|r| r.passed).count()
    }
    pub fn failed(&self) -> usize {
        self.results
            .iter()
            .filter(|r| !r.passed && !r.skipped)
            .count()
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
    /// Compile with per-statement counters and collect `SN_COVERAGE_OUT`.
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
    let mut acc = CovAcc::default();

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
            opt: if opts.coverage {
                "0".into()
            } else {
                opts.opt.clone()
            },
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
            Ok(res) => {
                if opts.coverage {
                    // Record every instrumented line first, so lines that never
                    // execute still count against the total.
                    accumulate(&mut acc, &res.cov_slots, &res.cov_files, &[]);
                }
                let mut cmd = Command::new(&bin);
                if opts.coverage {
                    cmd.env("SN_COVERAGE_OUT", outdir.join(format!("{stem}-{i}.cov")));
                }
                let cov_file = outdir.join(format!("{stem}-{i}.cov"));
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
                if opts.coverage {
                    accumulate(
                        &mut acc,
                        &res.cov_slots,
                        &res.cov_files,
                        &read_counts(&cov_file),
                    );
                }
                let _ = dur;
                let _ = std::fs::remove_file(&bin);
                let _ = std::fs::remove_file(&cov_file);
            }
        }
    }

    let _ = std::fs::remove_dir_all(&outdir);

    Ok(TestRun {
        results,
        covered_declared: opts.coverage.then_some(acc.declared),
        covered_lines: opts.coverage.then_some(acc.lines),
    })
}

/// Accumulate the counters written by `SN_COVERAGE_OUT` into per-file,
/// per-line hit counts.
#[derive(Default)]
struct CovAcc {
    lines: Coverage,
    /// Slots seen per test binary, kept so unused lines stay reportable.
    declared: Coverage,
}

fn read_counts(path: &Path) -> Vec<(u64, u64)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let slot = it.next()?.parse::<u64>().ok()?;
            let n = it.next()?.parse::<u64>().ok()?;
            Some((slot, n))
        })
        .collect()
}

fn accumulate(acc: &mut CovAcc, slots: &[(u32, u32)], files: &[String], counts: &[(u64, u64)]) {
    for (slot, _) in slots.iter().enumerate() {
        let (file_id, line) = slots[slot];
        let Some(path) = files.get(file_id as usize) else {
            continue;
        };
        let entry = acc.declared.entry(path.clone()).or_default();
        entry.entry(line).or_insert(0);
    }
    for (slot, n) in counts {
        let Some((_, line)) = slots.get(*slot as usize) else {
            continue;
        };
        let (file_id, line) = (slots[*slot as usize].0, *line);
        let Some(path) = files.get(file_id as usize) else {
            continue;
        };
        *acc.lines
            .entry(path.clone())
            .or_default()
            .entry(line)
            .or_insert(0) += n;
    }
}

/// Per-file coverage summary with the lines that never ran.
pub fn coverage_report(declared: &Coverage, lines: &Coverage) -> String {
    let mut s = String::new();
    let mut files: Vec<&String> = declared.keys().collect();
    files.sort();
    for f in files {
        let decl = &declared[f];
        if decl.is_empty() {
            continue;
        }
        let hits = lines.get(f);
        let hit = hits.map(|h| h.len()).unwrap_or(0);
        let pct = hit as f64 * 100.0 / decl.len() as f64;
        let mut missing: Vec<u32> = decl
            .iter()
            .filter(|(l, _)| hits.map(|h| !h.contains_key(l)).unwrap_or(true))
            .map(|(l, _)| *l)
            .collect();
        missing.sort_unstable();
        s.push_str(&format!("{:>6.1}%  {}/{}  {}\n", pct, hit, decl.len(), f));
        if !missing.is_empty() {
            // Collapse runs so long uncovered blocks stay readable.
            let mut i = 0;
            let mut runs: Vec<String> = Vec::new();
            while i < missing.len() {
                let start = missing[i];
                let mut end = start;
                while i + 1 < missing.len() && missing[i + 1] == end + 1 {
                    i += 1;
                    end = missing[i];
                }
                if start == end {
                    runs.push(start.to_string());
                } else {
                    runs.push(format!("{start}-{end}"));
                }
                i += 1;
            }
            s.push_str(&format!("        uncovered: {}\n", runs.join(", ")));
        }
    }
    s
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
    if let (Some(pct), Some(declared), Some(lines)) = (
        cov_pct,
        run.covered_declared.as_ref(),
        run.covered_lines.as_ref(),
    ) {
        s.push_str(&format!(
            "coverage: {pct:.1}% of instrumented lines executed\n\n"
        ));
        s.push_str(&coverage_report(declared, lines));
    }
    s
}

/// Share of instrumented lines that actually ran.
///
/// `declared` holds every line the compiler instrumented (so an unexecuted
/// `if` body still counts against coverage); `lines` holds the hits.
pub fn coverage_percent(declared: &Coverage, lines: &Coverage) -> f64 {
    let mut hit = 0usize;
    let mut total = 0usize;
    for (file, lmap) in declared {
        total += lmap.len();
        if let Some(hits) = lines.get(file) {
            hit += hits.len();
        }
    }
    if total == 0 {
        0.0
    } else {
        hit as f64 * 100.0 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn describe_detects_functions_and_expected_failures() {
        let path = std::env::temp_dir().join(format!("snc-describe-{}.sn", std::process::id()));
        std::fs::write(&path, "fn alpha() {\n}\n// @expected-failure\n").unwrap();
        let case = describe(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(case.functions, vec!["alpha".to_string()]);
        assert!(case.should_fail);
    }

    #[test]
    fn coverage_helpers_count_lines_and_collapse_gaps() {
        let mut declared = Coverage::new();
        declared.insert(
            "a.sn".to_string(),
            BTreeMap::from([(1, 0), (2, 0), (3, 0), (4, 0), (5, 0)]),
        );
        let mut lines = Coverage::new();
        lines.insert("a.sn".to_string(), BTreeMap::from([(2, 1), (4, 2)]));
        assert!((coverage_percent(&declared, &lines) - 40.0).abs() < 1e-6);
        let report = coverage_report(&declared, &lines);
        assert!(report.contains("40.0%"), "{report}");
        assert!(report.contains("uncovered: 1, 3, 5"), "{report}");
    }

    #[test]
    fn accumulate_merges_counts_for_one_line() {
        let mut acc = CovAcc::default();
        accumulate(
            &mut acc,
            &[(0, 7), (0, 7)],
            &["a.sn".to_string()],
            &[(0, 2), (1, 3)],
        );
        assert_eq!(acc.declared["a.sn"].len(), 1);
        assert_eq!(acc.lines["a.sn"][&7], 5);
    }
}
