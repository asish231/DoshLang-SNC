use crate::ast::Program;
use crate::check::check_programs;
use crate::diag::Diagnostics;
use crate::ir::lower;
use crate::llvm::emit_llvm;
use crate::parser::Parser;
use crate::span::SourceFile;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct CompileOptions {
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub emit_llvm: bool,
    pub target: Option<String>,
    pub clang: String,
    pub opt: String,
    /// Extra link flags from `extern "lib"` blocks (`-l<name>`) plus any
    /// user-supplied libraries.
    pub libs: Vec<String>,
    /// Build with source-based LLVM instrumentation so line coverage can be
    /// measured from the resulting `.profraw`.
    pub coverage: bool,
}

pub struct CompileResult {
    pub llvm_ir: String,
    pub binary: Option<PathBuf>,
}

/// Expand a friendly target name into a full LLVM triple.
///
/// Accepts either a preset (`windows`, `linux`, `macos`, optionally with
/// `-arm64` / `-x64`) or a full triple such as
/// `aarch64-unknown-linux-gnu`.
pub fn resolve_target(spec: &str) -> Result<String, String> {
    let s = spec.trim().to_ascii_lowercase();
    // Split an optional arch suffix: `linux-arm64`, `windows-x64`, `macos`.
    let (base, suffix) = match s.rsplit_once('-') {
        Some((b, a)) if a == "arm64" || a == "aarch64" || a == "x64" || a == "x86" || a == "amd64" => {
            (b.to_string(), a)
        }
        _ => (s.clone(), ""),
    };
    let arm = suffix == "arm64" || suffix == "aarch64";
    let os = match base.as_str() {
        "windows" | "win" => "windows",
        "linux" => "linux",
        "macos" | "mac" | "darwin" => "macos",
        _ => {
            // Not a preset: accept a full LLVM triple verbatim.
            if spec.contains('-') && spec.matches('-').count() >= 2 {
                return Ok(spec.to_string());
            }
            return Err(format!(
                "unknown target '{spec}'; use a preset (windows, linux, macos) \
                 optionally suffixed with -arm64/-x64, or a full triple"
            ));
        }
    };
    let a = if arm { "aarch64" } else { "x86_64" };
    Ok(match os {
        "windows" => format!("{a}-pc-windows-msvc"),
        "linux" => format!("{a}-unknown-linux-gnu"),
        _ => format!("{a}-apple-macosx14.0.0"),
    })
}

/// Windows binaries get an `.exe` suffix unless the caller named the file.
pub fn output_path(output: &Option<std::path::PathBuf>, triple: &str) -> Option<std::path::PathBuf> {
    let out = output.as_ref()?;
    if !triple.contains("windows") {
        return Some(out.clone());
    }
    if out.extension().is_some() {
        return Some(out.clone());
    }
    let mut name = out.clone();
    name.set_extension("exe");
    Some(name)
}

pub fn default_triple() -> String {
    if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "arm64-apple-macosx14.0.0".into()
        } else {
            "x86_64-apple-macosx14.0.0".into()
        }
    } else if cfg!(target_os = "windows") {
        if cfg!(target_arch = "aarch64") {
            "aarch64-pc-windows-msvc".into()
        } else {
            "x86_64-pc-windows-msvc".into()
        }
    } else if cfg!(target_arch = "aarch64") {
        "aarch64-unknown-linux-gnu".into()
    } else {
        "x86_64-unknown-linux-gnu".into()
    }
}

pub fn compile(opts: &CompileOptions) -> Result<CompileResult, String> {
    let mut files = Vec::new();
    let mut programs = Vec::new();
    let mut loaded = std::collections::HashSet::new();
    load_recursive(
        &opts.input,
        &mut files,
        &mut programs,
        &mut loaded,
        opts.input.parent().unwrap_or(Path::new(".")),
    )?;
    let mut diag = Diagnostics::default();
    let db = check_programs(&programs, &mut diag);
    if !diag.is_empty() {
        return Err(diag.render(&files));
    }
    let ir = lower(&programs, &db);
    let triple = match &opts.target {
        Some(t) => resolve_target(t)?,
        None => default_triple(),
    };
    let llvm_ir = emit_llvm(&ir, &triple);
    if opts.emit_llvm {
        if let Some(out) = &opts.output {
            fs::write(out, &llvm_ir).map_err(|e| e.to_string())?;
        }
        return Ok(CompileResult {
            llvm_ir,
            binary: None,
        });
    }
    if !ir.has_main {
        return Err("no fn main() found; use --emit-llvm to dump IR\n".into());
    }
    let out_bin = match opts.output.as_ref() {
        Some(o) => output_path(&opts.output, &triple).unwrap_or_else(|| o.clone()),
        None => {
            let stem = opts
                .input
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("a");
            if triple.contains("windows") {
                PathBuf::from(format!("{stem}.exe"))
            } else {
                PathBuf::from(stem)
            }
        }
    };
    let tmp = tempfile_ll(&opts.input);
    fs::write(&tmp, &llvm_ir).map_err(|e| e.to_string())?;
    let runtime = runtime_c_path();
    let mut cmd = Command::new(&opts.clang);
    cmd.arg(&tmp)
        .arg(&runtime)
        .arg(format!("-O{}", opts.opt))
        .arg("-o")
        .arg(&out_bin)
        .arg("-Wno-override-module");
    if !triple.contains("windows") {
        cmd.arg("-pthread");
    } else {
        cmd.arg("-lws2_32");
    }
    // Link the libraries named by `extern` blocks. Duplicates are collapsed so
    // repeated declarations do not produce repeated flags.
    let mut seen = std::collections::HashSet::new();
    for lib in db.extern_libs.iter().chain(opts.libs.iter()) {
        if lib.is_empty() || !seen.insert(lib.clone()) {
            continue;
        }
        if lib.starts_with('-') {
            cmd.arg(lib);
        } else if lib.ends_with(".a") || lib.ends_with(".so") || lib.ends_with(".dll.a") {
            cmd.arg(lib);
        } else if lib.ends_with(".dylib") || lib.ends_with(".so.1") || lib.contains('/')
            || lib.contains(".dylib.")
        {
            // Explicit file: record an rpath so the loader finds its
            // dependencies at run time.
            cmd.arg(lib);
            if let Some(dir) = Path::new(&lib).parent() {
                let dir = dir.to_string_lossy().to_string();
                if !dir.is_empty() && seen.insert(format!("rpath:{dir}")) {
                    if triple.contains("darwin") {
                        cmd.arg(format!("-Wl,-rpath,{dir}"));
                    } else {
                        cmd.arg(format!("-Wl,-rpath,{dir}"));
                    }
                }
            }
        } else {
            cmd.arg(format!("-l{lib}"));
        }
    }
    if opts.opt == "0" {
        cmd.arg("-g");
    }
    if opts.coverage {
        // Source-based coverage: counters are emitted per line and written
        // to $LLVM_PROFILE_FILE when the program exits.
        cmd.arg("-fprofile-instr-generate")
            .arg("-fcoverage-mapping");
    }
    if opts.target.is_some() {
        // Pass the resolved triple, not the raw preset the user typed.
        cmd.arg(format!("--target={triple}"));
    }
    let status = cmd.output().map_err(|e| format!("failed to run clang: {e}"))?;
    let _ = fs::remove_file(&tmp);
    if !status.status.success() {
        return Err(format!(
            "clang failed:\n{}\n{}",
            String::from_utf8_lossy(&status.stdout),
            String::from_utf8_lossy(&status.stderr)
        ));
    }
    Ok(CompileResult {
        llvm_ir,
        binary: Some(out_bin),
    })
}

fn tempfile_ll(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("snc");
    std::env::temp_dir().join(format!("{stem}-{}.ll", std::process::id()))
}

fn runtime_c_path() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime.c");
    if here.exists() {
        here
    } else {
        PathBuf::from("compiler/runtime.c")
    }
}

fn load_recursive(
    path: &Path,
    files: &mut Vec<SourceFile>,
    programs: &mut Vec<Program>,
    loaded: &mut std::collections::HashSet<PathBuf>,
    from_dir: &Path,
) -> Result<(), String> {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !loaded.insert(canon.clone()) {
        return Ok(());
    }
    let src = fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let id = files.len() as u32;
    let file = SourceFile::new(id, path.display().to_string(), src.clone());
    let program = Parser::parse_file(id, &src).map_err(|e| format!("{}: {e}", path.display()))?;
    let uses: Vec<Vec<String>> = program
        .items
        .iter()
        .filter_map(|it| match it {
            crate::ast::Item::Use(u) => Some(u.path.clone()),
            _ => None,
        })
        .collect();
    files.push(file);
    programs.push(program);
    for p in uses {
        if let Some(found) = crate::pkg::resolve_use_path(&p, from_dir, path) {
            let parent = found.parent().unwrap_or(from_dir).to_path_buf();
            load_recursive(&found, files, programs, loaded, &parent)?;
        } else {
            return Err(format!(
                "cannot find module '{}' imported from {}",
                p.join("."),
                path.display()
            ));
        }
    }
    Ok(())
}
