use clap::{Parser, Subcommand};
use snc::driver::{compile, CompileOptions};
use snc::pkg;
use snc::translate;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(name = "snc", about = "SNlang compiler (LLVM)")]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
    /// Input .sn file
    input: Option<PathBuf>,
    /// Output binary (or .ll with --emit-llvm)
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Write LLVM IR instead of linking an executable
    #[arg(long)]
    emit_llvm: bool,
    /// LLVM/clang target triple (macOS and Windows supported)
    #[arg(long)]
    target: Option<String>,
    /// clang executable
    #[arg(long, default_value = "clang")]
    clang: String,
    /// Optimization level passed to clang (0-3, s, z)
    #[arg(short = 'O', default_value = "2")]
    opt: String,
    /// Extra library to link, repeatable (`-L sqlite3`). `extern "lib"` blocks
    /// add these automatically.
    #[arg(short = 'L', long = "link-lib")]
    libs: Vec<String>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Translate a Python / JS / Go / C subset into SNlang
    Translate {
        /// Source language: python, js, go, c
        #[arg(long)]
        from: String,
        /// Input source file
        input: PathBuf,
        /// Output .sn file
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Format .sn files (trim trailing space, normalize indent, ensure final newline)
    Fmt {
        /// One or more .sn files (in-place)
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Local package manager (sn.toml)
    Pkg {
        #[command(subcommand)]
        action: PkgCmd,
    },
    /// Discover and run tests, with optional line coverage
    Test {
        /// Directory to search for *_test.sn (default: the package root)
        #[arg(long)]
        root: Option<PathBuf>,
        /// Only run tests whose name or function contains this substring
        filter: Option<String>,
        /// Measure statement coverage with `sn_cov_hit` counters
        #[arg(long)]
        coverage: bool,
        #[arg(short = 'O', default_value = "2")]
        opt: String,
        #[arg(long, default_value = "clang")]
        clang: String,
    },
    /// Language server (JSON-RPC over stdio)
    Lsp,
    /// Build with debug symbols and print lldb workflow
    Debug {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum PkgCmd {
    /// Create sn.toml (and main.sn if missing)
    Init {
        #[arg(long)]
        name: Option<String>,
    },
    /// Record a local/std/registry dependency in sn.toml (+ sn.lock.toml)
    Add {
        name: String,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        registry: Option<String>,
        /// Semver-ish version for registry/std deps (default 0.1.0 / 0.2.0)
        #[arg(long)]
        version: Option<String>,
    },
    /// Compile main.sn / src/main.sn using package resolution (refreshes lockfile)
    Build {
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        emit_llvm: bool,
        #[arg(short = 'O', default_value = "2")]
        opt: String,
        #[arg(long, default_value = "clang")]
        clang: String,
    },
    /// Show sn.toml name, version, and deps
    List,
    /// Pack current package as .tar.gz into dist/ for a simple registry
    Publish {
        /// Output directory (default: dist/ or packages/dist/)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Some(Cmd::Translate {
            from,
            input,
            output,
        }) => match run_translate(&from, &input, output.as_ref()) {
            Ok(msg) => {
                eprintln!("{msg}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprint!("{e}");
                if !e.ends_with('\n') {
                    eprintln!();
                }
                ExitCode::FAILURE
            }
        },
        Some(Cmd::Fmt { files }) => match run_fmt(&files) {
            Ok(msg) => {
                if !msg.is_empty() {
                    eprintln!("{msg}");
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        Some(Cmd::Pkg { action }) => match run_pkg(action) {
            Ok(msg) => {
                print!("{msg}");
                if !msg.ends_with('\n') {
                    println!();
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        Some(Cmd::Test {
            root,
            filter,
            coverage,
            opt,
            clang,
        }) => {
            let root = root.unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
            });
            let opts = snc::test::Options {
                root,
                clang,
                opt,
                filter,
                coverage,
            };
            match snc::test::run(&opts) {
                Ok(run) => {
                    let pct = match (run.covered_declared.as_ref(), run.covered_lines.as_ref()) {
                        (Some(d), Some(l)) => Some(snc::test::coverage_percent(d, l)),
                        _ => None,
                    };
                    print!("{}", snc::test::report(&run, pct));
                    if run.failed() == 0 {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::FAILURE
                    }
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some(Cmd::Lsp) => match snc::lsp::run_lsp() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        Some(Cmd::Debug { input, output }) => match snc::lsp::run_debug(
            input.to_str().unwrap_or("main.sn"),
            output.as_deref(),
        ) {
            Ok(msg) => {
                print!("{msg}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        None => {
            let Some(input) = cli.input else {
                eprintln!("usage: snc file.sn -o app\n       snc translate --from python file.py -o out.sn\n       snc fmt file.sn ...\n       snc pkg init|add|build|list|publish");
                return ExitCode::FAILURE;
            };
            run_compile(CompileOptions {
                input,
                output: cli.output,
                emit_llvm: cli.emit_llvm,
                target: cli.target,
                clang: cli.clang,
                opt: cli.opt,
                libs: cli.libs,
                coverage: false,
            })
        }
    }
}

fn run_compile(opts: CompileOptions) -> ExitCode {
    let emit_stdout = opts.emit_llvm && opts.output.is_none();
    match compile(&opts) {
        Ok(res) => {
            if res.cached && std::env::var("SNC_CACHE_VERBOSE").is_ok() {
                eprintln!("(reused cached build)");
            }
            if emit_stdout {
                print!("{}", res.llvm_ir);
            } else if let Some(bin) = res.binary {
                eprintln!("wrote {}", bin.display());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprint!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run_translate(from: &str, input: &PathBuf, output: Option<&PathBuf>) -> Result<String, String> {
    let src = std::fs::read_to_string(input).map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    let name = input
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("input");
    let sn = translate::translate(from, &src, name)?;
    if let Some(out) = output {
        std::fs::write(out, &sn).map_err(|e| e.to_string())?;
        Ok(format!("wrote {}", out.display()))
    } else {
        print!("{sn}");
        Ok(String::new())
    }
}

fn run_fmt(files: &[PathBuf]) -> Result<String, String> {
    let mut out = String::new();
    for path in files {
        let formatted = snc::fmt::format_file(path)?;
        std::fs::write(path, &formatted).map_err(|e| format!("{}: {e}", path.display()))?;
        out.push_str(&format!("formatted {}\n", path.display()));
    }
    Ok(out)
}

fn run_pkg(action: PkgCmd) -> Result<String, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    match action {
        PkgCmd::Init { name } => pkg::cmd_init(&cwd, name.as_deref()),
        PkgCmd::Add {
            name,
            path,
            registry,
            version,
        } => pkg::cmd_add(
            &cwd,
            &name,
            path.as_deref(),
            registry.as_deref(),
            version.as_deref(),
        ),
        PkgCmd::List => pkg::cmd_list(&cwd),
        PkgCmd::Publish { output } => pkg::cmd_publish(&cwd, output.as_deref()),
        PkgCmd::Build {
            output,
            emit_llvm,
            opt,
            clang,
        } => {
            let (root, _m) = pkg::cmd_build_prepare(&cwd).unwrap_or_else(|_| {
                let m = pkg::find_manifest(&cwd)
                    .and_then(|p| pkg::load_manifest(&p).ok())
                    .unwrap_or_default();
                let _ = pkg::write_lockfile(&cwd, &m);
                (cwd.clone(), m)
            });
            let input = pkg::default_build_input(&root)?;
            let opts = CompileOptions {
                input,
                output,
                emit_llvm,
                target: None,
                clang,
                opt,
                libs: Vec::new(),
                coverage: false,
            };
            match compile(&opts) {
                Ok(res) => {
                    if let Some(bin) = res.binary {
                        Ok(format!("wrote {}", bin.display()))
                    } else {
                        Ok("ok".into())
                    }
                }
                Err(e) => Err(e),
            }
        }
    }
}
