use crate::ast::Program;
use crate::cache;
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
    /// Emit per-statement `sn_cov_hit` counters. The test runner collects the
    /// resulting `SN_COVERAGE_OUT` dump and maps slots back to source lines.
    pub coverage: bool,
}

pub struct CompileResult {
    pub llvm_ir: String,
    pub binary: Option<PathBuf>,
    /// Coverage counter slots as `(file_id, line)`, when `coverage` was on.
    pub cov_slots: Vec<(u32, u32)>,
    /// `file_id -> source path`, so slot file ids can be resolved.
    pub cov_files: Vec<String>,
    /// True when the IR/binary came from the incremental build cache.
    pub cached: bool,
}

/// Resolve raw `(file, byte_offset)` slots into `(file, line)` pairs.
fn resolve_cov_slots(slots: &[(u32, u32)], files: &[SourceFile]) -> Vec<(u32, u32)> {
    slots
        .iter()
        .map(|(f, off)| {
            let line = files
                .get(*f as usize)
                .map(|sf| sf.loc(*off as usize).0)
                .unwrap_or(0);
            (*f, line)
        })
        .collect()
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

/// Resolve the final binary path: an explicit `--output`, with `.exe` added
/// for Windows targets, else a stem-based name in the working directory.
fn output_binary(output: &Option<PathBuf>, triple: &str, input: &Path) -> PathBuf {
    if let Some(o) = output_path(output, triple) {
        return o;
    }
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("a");
    if triple.contains("windows") {
        PathBuf::from(format!("{stem}.exe"))
    } else {
        PathBuf::from(stem)
    }
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
    let cov_files: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
    let _ = crate::ir::cov_collect(opts.coverage);
    let ir = lower(&programs, &db, &files);
    let triple = match &opts.target {
        Some(t) => resolve_target(t)?,
        None => default_triple(),
    };
    let llvm_ir = emit_llvm(&ir, &triple);
    // Unique link libraries in declaration order; part of the cache key.
    let link_libs = link_libraries(&db, &opts.libs);
    let runtime = runtime_c_path();
    let runtime_bytes = fs::read(&runtime).unwrap_or_default();
    let fp = cache::fingerprint(
        &files,
        &triple,
        &opts.opt,
        &link_libs,
        opts.coverage,
        &opts.clang,
        &runtime_bytes,
    );
    if cache::cache_enabled() {
        if let Some(hit) = cache::get(&fp, !opts.emit_llvm) {
            if opts.emit_llvm {
                if let Some(out) = &opts.output {
                    fs::write(out, &hit.llvm_ir).map_err(|e| e.to_string())?;
                }
                return Ok(CompileResult {
                    llvm_ir: hit.llvm_ir,
                    binary: None,
                    cov_slots: resolve_cov_slots(&ir.cov, &files),
                    cov_files: cov_files.clone(),
                    cached: true,
                });
            }
            if !ir.has_main {
                return Err("no fn main() found; use --emit-llvm to dump IR\n".into());
            }
            let out_bin = output_binary(&opts.output, &triple, &opts.input);
            let bytes = hit
                .binary
                .ok_or_else(|| "build cache entry is missing its binary".to_string())?;
            fs::write(&out_bin, &bytes).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&out_bin, fs::Permissions::from_mode(0o755));
            }
            return Ok(CompileResult {
                llvm_ir: hit.llvm_ir,
                binary: Some(out_bin),
                cov_slots: resolve_cov_slots(&ir.cov, &files),
                cov_files: cov_files.clone(),
                cached: true,
            });
        }
    }
    if opts.emit_llvm {
        if let Some(out) = &opts.output {
            fs::write(out, &llvm_ir).map_err(|e| e.to_string())?;
        }
        if cache::cache_enabled() {
            cache::put(&fp, &llvm_ir, None);
        }
        return Ok(CompileResult {
            llvm_ir,
            binary: None,
            cov_slots: resolve_cov_slots(&ir.cov, &files),
            cov_files: cov_files.clone(),
            cached: false,
        });
    }
    if !ir.has_main {
        return Err("no fn main() found; use --emit-llvm to dump IR\n".into());
    }
    let out_bin = output_binary(&opts.output, &triple, &opts.input);
    let tmp = tempfile_ll(&opts.input);
    fs::write(&tmp, &llvm_ir).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(&opts.clang);
    cmd.arg(&tmp)
        .arg(&runtime)
        .arg(format!("-O{}", opts.opt))
        .arg("-o")
        .arg(&out_bin)
        .arg("-Wno-override-module");
    // Old clang (e.g. Ubuntu 22.04's clang 14) defaults to typed pointers and
    // rejects the opaque `ptr` IR snc emits. The probe below passes the
    // transition flag only when the toolchain honors it (new clangs that
    // dropped the flag misparse it as `-o paque-pointers`, so a `--version`
    // check is not sufficient).
    push_opaque_pointer_flag(&mut cmd, &opts.clang);
    if !triple.contains("windows") {
        cmd.arg("-pthread");
    } else {
        cmd.arg("-lws2_32");
    }
    // Link the libraries named by `extern` blocks (`link_libs` is already
    // deduplicated in declaration order).
    push_link_args(&mut cmd, &link_libs);
    if opts.opt == "0" {
        cmd.arg("-g");
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
    if cache::cache_enabled() {
        if let Ok(bytes) = fs::read(&out_bin) {
            cache::put(&fp, &llvm_ir, Some(&bytes));
        }
    }
    Ok(CompileResult {
        llvm_ir,
        binary: Some(out_bin),
        cov_slots: resolve_cov_slots(&ir.cov, &files),
        cov_files: cov_files.clone(),
        cached: false,
    })
}

fn tempfile_ll(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("snc");
    std::env::temp_dir().join(format!("{stem}-{}.ll", std::process::id()))
}

/// Unique link libraries in declaration order, collapsing the `extern`
/// blocks and the caller's extra `-L` flags.
fn link_libraries(
    db: &crate::check::CheckDb,
    extra: &[String],
) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for lib in db.extern_libs.iter().chain(extra.iter()) {
        if lib.is_empty() || !seen.insert(lib.clone()) {
            continue;
        }
        out.push(lib.clone());
    }
    out
}

fn runtime_c_path() -> PathBuf {
    // Release archives keep runtime.c next to the snc binary; dev checkouts
    // resolve it the same way (…/target/release/snc walks up to compiler/).
    let mut dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    for _ in 0..6 {
        let d = match dir {
            Some(d) => d,
            None => break,
        };
        let cand = d.join("runtime.c");
        if cand.exists() {
            return cand;
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime.c");
    if here.exists() {
        here
    } else {
        PathBuf::from("compiler/runtime.c")
    }
}

/// How to enable opaque-pointer IR parsing on this toolchain, if needed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OpaquePtrFlag {
    /// Driver-level `-opaque-pointers` (transition-era clang that knows it).
    Driver,
    /// `-Xclang -opaque-pointers` passthrough (driver never misparses this
    /// as `-o`, so it is safe to probe on toolchains that dropped the flag).
    Xclang,
}

/// Old clang (e.g. Ubuntu 22.04's clang 14) defaults to typed pointers and
/// rejects the opaque `ptr` IR snc emits. New clangs are opaque-by-default
/// and some (e.g. Apple clang 21) removed the flag entirely -- worse, an
/// unknown `-opaque-pointers` is misparsed as `-o paque-pointers`, silently
/// redirecting output. A `--version` probe cannot catch that, so compile a
/// tiny opaque-pointer `.ll` for real (flag positioned after `-o`, exactly
/// like the build command) and use whichever spelling yields the object.
fn opaque_pointer_flag(clang: &str) -> Option<OpaquePtrFlag> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Option<OpaquePtrFlag>>>> =
        std::sync::OnceLock::new();
    // Fast path: the probe result is cached per clang executable.
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Ok(map) = cache.lock() {
        if let Some(hit) = map.get(clang) {
            return *hit;
        }
    }
    // Try the driver flag first (single arg), then the cc1 passthrough.
    let found = if probe_opaque_flag(clang, &["-opaque-pointers"]) {
        Some(OpaquePtrFlag::Driver)
    } else if probe_opaque_flag(clang, &["-Xclang", "-opaque-pointers"]) {
        Some(OpaquePtrFlag::Xclang)
    } else {
        None
    };
    if let Ok(mut map) = cache.lock() {
        map.insert(clang.to_string(), found);
    }
    found
}

/// Append the working opaque-pointer flag for `clang` to `cmd`, if any.
fn push_opaque_pointer_flag(cmd: &mut Command, clang: &str) {
    match opaque_pointer_flag(clang) {
        Some(OpaquePtrFlag::Driver) => {
            cmd.arg("-opaque-pointers");
        }
        Some(OpaquePtrFlag::Xclang) => {
            cmd.arg("-Xclang").arg("-opaque-pointers");
        }
        None => {}
    }
}

fn probe_opaque_flag(clang: &str, flag_args: &[&str]) -> bool {
    let dir = std::env::temp_dir().join(format!("snc-opaque-probe-{}", std::process::id()));
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let ll = dir.join("probe.ll");
    let obj = dir.join("probe.o");
    // Opaque `ptr` types: rejected by typed-pointer-era clang without the flag.
    const PROBE_LL: &str = "declare ptr @sn_probe_fn(ptr, i64)\n\
         define i32 @main() {\n  ret i32 0\n}\n";
    if std::fs::write(&ll, PROBE_LL).is_err() {
        return false;
    }
    let status = std::process::Command::new(clang)
        // NB: the flag goes AFTER `-o`, mirroring the real build command.
        // Drivers that dropped the flag misparse it as `-o paque-pointers`,
        // which would clobber the output; running with the temp dir as CWD
        // keeps any such stray file inside the probe dir we delete below.
        .current_dir(&dir)
        .arg(&ll)
        .arg("-c")
        .arg("-o")
        .arg(&obj)
        .arg("-Wno-override-module")
        .args(flag_args)
        .output();
    let ok = status.map(|o| o.status.success()).unwrap_or(false) && obj.exists();
    let _ = std::fs::remove_dir_all(&dir);
    // Belt and braces: never leave a misdirected output behind in the
    // caller's directory if the driver ignored `current_dir` somehow.
    let _ = std::fs::remove_file(dir.join("paque-pointers"));
    ok
}

/// Translate a deduplicated library list into clang arguments, recording an
/// rpath for explicit files so the loader finds them at run time.
fn push_link_args(cmd: &mut Command, libs: &[String]) {
    let mut seen = std::collections::HashSet::new();
    for lib in libs {
        if lib.starts_with('-')
            || lib.ends_with(".a")
            || lib.ends_with(".so")
            || lib.ends_with(".dll.a")
        {
            cmd.arg(lib);
        } else if lib.ends_with(".dylib") || lib.ends_with(".so.1") || lib.contains('/')
            || lib.contains(".dylib.")
        {
            cmd.arg(lib);
            if let Some(dir) = Path::new(&lib).parent() {
                let dir = dir.to_string_lossy().to_string();
                if !dir.is_empty() && seen.insert(format!("rpath:{dir}")) {
                    cmd.arg(format!("-Wl,-rpath,{dir}"));
                }
            }
        } else {
            cmd.arg(format!("-l{lib}"));
        }
    }
}

/// Build with full DWARF and preserved object files for `snc debug`.
///
/// Unlike `compile`, the intermediates are kept (in a per-pid directory next
/// to the output) so `dsymutil` can assemble a dSYM on macOS and lldb can
/// resolve SN source lines.
pub fn compile_debug(input: &Path, output: &Path, clang: &str) -> Result<(), String> {
    let mut files = Vec::new();
    let mut programs = Vec::new();
    let mut loaded = std::collections::HashSet::new();
    load_recursive(
        input,
        &mut files,
        &mut programs,
        &mut loaded,
        input.parent().unwrap_or(Path::new(".")),
    )?;
    let mut diag = Diagnostics::default();
    let db = check_programs(&programs, &mut diag);
    if !diag.is_empty() {
        return Err(diag.render(&files));
    }
    let _ = crate::ir::cov_collect(false);
    let ir = lower(&programs, &db, &files);
    if !ir.has_main {
        return Err("no fn main() found\n".into());
    }
    let llvm_ir = emit_llvm(&ir, &default_triple());
    let workdir = output
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let stem = output
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("snc-debug");
    let workdir = workdir.join(format!(".{stem}-dbg-{}", std::process::id()));
    fs::create_dir_all(&workdir).map_err(|e| e.to_string())?;
    let ll = workdir.join("sn.ll");
    let sn_o = workdir.join("sn.o");
    let rt_o = workdir.join("rt.o");
    fs::write(&ll, &llvm_ir).map_err(|e| e.to_string())?;
    let runtime = runtime_c_path();

    let run = |mut cmd: Command, what: &str| -> Result<(), String> {
        let status = cmd.output().map_err(|e| format!("failed to run {what}: {e}"))?;
        if !status.status.success() {
            return Err(format!(
                "{what} failed:\n{}\n{}",
                String::from_utf8_lossy(&status.stdout),
                String::from_utf8_lossy(&status.stderr)
            ));
        }
        Ok(())
    };

    let mut cc = Command::new(clang);
    cc.arg("-c").arg(&ll).arg("-O0").arg("-g").arg("-o").arg(&sn_o).arg("-Wno-override-module");
    push_opaque_pointer_flag(&mut cc, clang);
    run(cc, "clang -c sn.ll")?;

    let mut cc = Command::new(clang);
    cc.arg("-c")
        .arg(&runtime)
        .arg("-O0")
        .arg("-g")
        .arg("-o")
        .arg(&rt_o);
    run(cc, "clang -c runtime.c")?;

    let mut link = Command::new(clang);
    link.arg(&sn_o).arg(&rt_o).arg("-O0").arg("-g").arg("-o").arg(output);
    link.arg("-pthread");
    push_link_args(&mut link, &link_libraries(&db, &[]));
    run(link, "clang link")?;

    #[cfg(target_os = "macos")]
    {
        let mut dsym = Command::new("dsymutil");
        dsym.arg(output);
        run(dsym, "dsymutil")?;
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::SourceFile;

    #[test]
    fn coverage_slots_resolve_byte_offsets_to_lines() {
        let files = vec![SourceFile::new(0, "a.sn".to_string(), "aa\nbbb\n".to_string())];
        assert_eq!(
            resolve_cov_slots(&[(0, 0), (0, 3)], &files),
            vec![(0, 1), (0, 2)]
        );
    }
}
