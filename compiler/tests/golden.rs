use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn snc_bin() -> PathBuf {
    env!("CARGO_BIN_EXE_snc").into()
}

fn compile_run(rel: &str) -> (i32, String, String) {
    let root = repo_root();
    let src = root.join(rel);
    let out = std::env::temp_dir().join(format!(
        "snc-test-{}-{}",
        src.file_stem().unwrap().to_string_lossy(),
        std::process::id()
    ));
    let compile = Command::new(snc_bin())
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .current_dir(&root)
        .output()
        .expect("run snc");
    if !compile.status.success() {
        return (
            compile.status.code().unwrap_or(1),
            String::new(),
            String::from_utf8_lossy(&compile.stderr).into(),
        );
    }
    let run = Command::new(&out).output().expect("run binary");
    let _ = std::fs::remove_file(&out);
    (
        run.status.code().unwrap_or(1),
        String::from_utf8_lossy(&run.stdout).into(),
        String::from_utf8_lossy(&run.stderr).into(),
    )
}

fn compile_err(rel: &str) -> String {
    let root = repo_root();
    let src = root.join(rel);
    let compile = Command::new(snc_bin())
        .arg(&src)
        .arg("--emit-llvm")
        .current_dir(&root)
        .output()
        .expect("run snc");
    String::from_utf8_lossy(&compile.stderr).into()
}

fn assert_out(rel: &str, expected: &str) {
    let (code, stdout, stderr) = compile_run(rel);
    assert_eq!(code, 0, "{rel} failed ({code}): {stderr}\n{stdout}");
    assert_eq!(stdout, expected, "{rel} output mismatch");
}

#[test]
fn hello_world() {
    assert_out("examples/hello_world.sn", "Hello, World!\n");
}

#[test]
fn grouping() {
    assert_out("examples/grouping.sn", "42\n22\n");
}

#[test]
fn functions() {
    assert_out(
        "examples/functions.sn",
        "42\n42\n10\n10\n7\n7\n36\nhello\nhi\nSNlang\n16\n",
    );
}

#[test]
fn for_loop() {
    assert_out(
        "examples/for_loop.sn",
        "0\n1\n2\n3\n4\n10\n11\n12\n0\n1\n3\n4\n100\n101\n102\n7\n8\n10\nApple\nBanana\n",
    );
}

#[test]
fn otherwise() {
    assert_out("examples/otherwise.sn", "No nickname\nAce\n");
}

#[test]
fn default_params() {
    assert_out("examples/default_params.sn", "Guest\nGuest\nAce\nAce\nNeo\n");
}

#[test]
fn multi_return() {
    assert_out("examples/multi_return.sn", "5\ntrue\n7\n3\n");
}

#[test]
fn string_methods() {
    assert_out(
        "examples/string_methods.sn",
        "11\nHello\ntrue\nHELLO WORLD\nhello world\nHello SNlang\nHello\nWorld\nhi Ada\n",
    );
}

#[test]
fn map_basic() {
    assert_out("examples/map_basic.sn", "30\n26\n2\n1\n");
}

#[test]
fn blueprint_point() {
    assert_out("examples/blueprint_point.sn", "30\n10\n");
}

#[test]
fn blueprint_inherit() {
    assert_out("examples/blueprint_inherit.sn", "Canine\nWoof\nLab\n");
}

#[test]
fn contract_draw() {
    assert_out("examples/contract_draw.sn", "5\n");
}

#[test]
fn spawn_chan() {
    assert_out("examples/spawn_chan.sn", "1\n2\n");
}

#[test]
fn error_demo() {
    assert_out("examples/error_demo.sn", "5\nok\n");
}

#[test]
fn use_std_math() {
    assert_out("examples/use_std_math.sn", "5\n9\n3\n10\n256\n");
}

#[test]
fn lock_counter() {
    assert_out("examples/lock_counter.sn", "2\n");
}

#[test]
fn closed_field_rejected() {
    let err = compile_err("examples/closed_field_fail.sn");
    assert!(
        err.contains("closed"),
        "expected closed-field error, got: {err}"
    );
}

#[test]
fn defer_try() {
    assert_out("examples/defer_try.sn", "5\n3\ndone\n");
}

#[test]
fn closures() {
    assert_out("examples/closures.sn", "42\n42\n7\n");
}

#[test]
fn null_finish() {
    assert_out("examples/null_finish.sn", "missing\nAda\nfallback\nAda\n");
}

#[test]
fn json_time() {
    assert_out(
        "examples/json_time.sn",
        "7\nhi\n[7,\"hi\"]\ntime\n1970-01-01T00:00:00Z\n",
    );
}

#[test]
fn match_incomplete() {
    let err = compile_err("examples/match_incomplete.sn");
    assert!(
        err.contains("exhaustive"),
        "expected exhaustive match error, got: {err}"
    );
}

#[test]
fn in_operator() {
    assert_out("examples/in_operator.sn", "true\nfalse\ntrue\nfalse\ntrue\nfalse\n");
}

#[test]
fn file_full() {
    assert_out(
        "examples/file_full.sn",
        "hello\nhello world\ntrue\nfalse\nhello world\n",
    );
}

#[test]
fn path_os() {
    let (code, stdout, stderr) = compile_run("examples/path_os.sn");
    assert_eq!(code, 0, "path_os failed: {stderr}\n{stdout}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "hello_world.sn");
    assert_eq!(lines[1], "examples");
    assert_eq!(lines[2], ".sn");
    assert!(lines[3] == "HOME set" || lines[3] == "USER set" || lines[3] == "no env");
    assert_eq!(lines[4], "1");
}

#[test]
fn translate_python_compiles() {
    let root = repo_root();
    let py = root.join("examples/translate/hello.py");
    let out_sn = std::env::temp_dir().join(format!("snc-tr-{}.sn", std::process::id()));
    let tr = Command::new(snc_bin())
        .args([
            "translate",
            "--from",
            "python",
            py.to_str().unwrap(),
            "-o",
            out_sn.to_str().unwrap(),
        ])
        .current_dir(&root)
        .output()
        .expect("translate");
    assert!(
        tr.status.success(),
        "translate failed: {}",
        String::from_utf8_lossy(&tr.stderr)
    );
    let bin = std::env::temp_dir().join(format!("snc-tr-bin-{}", std::process::id()));
    let compile = Command::new(snc_bin())
        .arg(&out_sn)
        .arg("-o")
        .arg(&bin)
        .current_dir(&root)
        .output()
        .expect("compile translated");
    assert!(
        compile.status.success(),
        "compile translated failed: {}\n{}",
        String::from_utf8_lossy(&compile.stderr),
        std::fs::read_to_string(&out_sn).unwrap_or_default()
    );
    let run = Command::new(&bin).output().expect("run translated");
    let _ = std::fs::remove_file(&out_sn);
    let _ = std::fs::remove_file(&bin);
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "5\n6\nok\n",
        "translated python output mismatch: {}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn https_get() {
    let (code, stdout, stderr) = compile_run("examples/https_get.sn");
    assert_eq!(code, 0, "https_get failed: {stderr}\n{stdout}");
    let n: i64 = stdout.trim().parse().unwrap_or(0);
    assert!(n > 100, "expected HTML body length > 100, got {stdout}");
}

#[test]
fn async_http() {
    assert_out("examples/async_http.sn", "5\n");
}

#[test]
fn chan_select() {
    assert_out("examples/chan_select.sn", "1\n77\n");
}

#[test]
fn oop_poly() {
    assert_out("examples/oop_poly.sn", "woof\n");
}

#[test]
fn abstract_method() {
    assert_out("examples/abstract_method.sn", "woof\n4\ntweet\n2\n");
}

#[test]
fn closed_method() {
    assert_out("examples/closed_method.sn", "200\n10\n");
}

/// The reactor multiplexes timers on a single thread: three overlapping
/// 300 ms sleeps must complete in ~300 ms, not ~900 ms, and issuing the
/// futures must be effectively free.
#[test]
fn reactor_concurrency() {
    assert_out("examples/reactor_concurrency.sn", "true\ntrue\ntrue\n");
}

/// Borrow lifetimes: disjoint borrows coexist, `mut<T>` is exclusive, and a
/// borrow of a local cannot escape its function.
#[test]
fn borrow_lifetimes() {
    assert_out("examples/borrow_lifetimes.sn", "3\n1\n2\n1\n2\n");

    let err = compile_err("examples/borrow_conflict_fail.sn");
    assert!(
        err.contains("already mutably borrowed"),
        "expected an exclusivity error, got: {err}"
    );

    let err = compile_err("examples/borrow_escape_fail.sn");
    assert!(
        err.contains("cannot return"),
        "expected a lifetime-escape error, got: {err}"
    );
}

/// `alloc`/`free` guards: double frees and use-after-free are reported
/// instead of corrupting the allocator.
#[test]
fn alloc_guards() {
    let (code, stdout, stderr) = compile_run("examples/alloc_guards.sn");
    assert_eq!(code, 0, "alloc guards failed: {stderr}");
    assert_eq!(
        stdout,
        "size=\n16\nlive after writes=\n1\nvalues=\n1234\n5678\n\
         free ok=\ntrue\nuse after free detected=\ntrue\n\
         double free rejected=\ntrue\nreported=\ntrue\n\
         live count grew=\ntrue\nnew block is live=\n1\n"
    );
    assert!(stderr.contains("use after free"), "expected a UAF report: {stderr}");
    assert!(stderr.contains("double free"), "expected a double-free report: {stderr}");
}

/// The cycle collector reclaims reference cycles that refcounting alone can
/// never free, while leaving reachable data intact.
#[test]
fn gc_cycles() {
    assert_out(
        "examples/gc_cycles.sn",
        "cycle reclaimed: \ntrue\nlive data kept: \ntrue\nlive head value: \n49\n",
    );
}

/// SHA-256 / SHA-512 / MD5 / HMAC-SHA256 / CRC-32, checked against the
/// published FIPS and RFC test vectors.
#[test]
fn hash_crypto() {
    assert_out(
        "examples/hash_crypto.sn",
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n\
         e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n\
         900150983cd24fb0d6963f7d28e17f72\n\
         ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
         2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f\n\
         f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8\n\
         3421780262\ntrue\nfalse\n",
    );
}

/// bytearray: growth past capacity, slicing, search and truncation.
#[test]
fn bytearray() {
    assert_out(
        "examples/bytearray.sn",
        "6\n104\n33\n1\n-1\n3\n3\n69\n4\n65\n2\n0\n300\n43\n",
    );
}

/// `std.db`: a real SQLite driver built on `extern` FFI, running against an
/// in-memory database.
#[test]
fn sqlite_driver() {
    assert_out(
        "examples/sqlite_driver.sn",
        "true\n0\n0\nada\n36\nalan\n41\ngrace\n45\n3\n",
    );
}

/// `extern "lib"` blocks: libc calls, float ABI, and C out-parameters
/// read back through `ptr_load`.
#[test]
fn ffi_extern() {
    // Build the tiny C shim the example links against.
    let root = repo_root();
    let obj = std::env::temp_dir().join("sn_ffi_shim.o");
    let cc = Command::new("clang")
        .arg("-c")
        .arg("-o")
        .arg(&obj)
        .arg(root.join("compiler/tests/shim.c"))
        .output()
        .expect("run clang");
    assert!(cc.status.success(), "shim compile failed");

    // The example names the object by absolute path, so link it there too.
    let linked = PathBuf::from("/tmp/sn_ffi_shim.o");
    let _ = std::fs::copy(&obj, &linked);

    assert_out(
        "examples/ffi_extern.sn",
        "42\n7\n1234\n255\n5\n7\n4242\n4243\n65\n",
    );
}

/// TCP sockets + select_read, all driven by the runtime reactor.
#[test]
fn tcp_echo() {
    assert_out(
        "examples/tcp_echo.sn",
        "true\necho:ping\nserved:ping\ntrue\n",
    );
}

#[test]
fn static_method() {
    assert_out(
        "examples/static_method.sn",
        "origin\norigin\n42\nsquare 4\nshape with 3 sides\n",
    );
}

#[test]
fn generic_bound() {
    assert_out("examples/generic_bound.sn", "dog\ncat\n");
}

#[test]
fn generic_variance() {
    assert_out("examples/generic_variance.sn", "dog\ncat\ndog\n");
}

#[test]
fn generic_box() {
    assert_out("examples/generic_box.sn", "42\n");
}

#[test]
fn record_point() {
    assert_out("examples/record_point.sn", "10\n5\n");
}

#[test]
fn extension_str() {
    assert_out("examples/extension_str.sn", "2\n");
}

#[test]
fn goroutine_spawn() {
    assert_out("examples/goroutine_spawn.sn", "42\n");
}

#[test]
fn borrow_ref() {
    assert_out("examples/borrow_ref.sn", "99\n");
}

#[test]
fn borrow_move_rejected() {
    let err = compile_err("examples/borrow_move_fail.sn");
    assert!(
        err.contains("moved") || err.contains("assign"),
        "expected borrow/move error, got: {err}"
    );
}

#[test]
fn selfhost_lexer() {
    assert_out("examples/selfhost_lexer.sn", "0\n1\n2\n3\n4\n5\n6\n7\n8\n9\ntokens done\n");
}

#[test]
fn std_test_helpers() {
    assert_out("examples/std_test.sn", "all tests passed\n");
}

#[test]
fn pkg_lockfile_and_publish() {
    let root = repo_root();
    let tmp = std::env::temp_dir().join(format!("snc-pkg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let init = Command::new(snc_bin())
        .args(["pkg", "init", "--name", "demo"])
        .current_dir(&tmp)
        .output()
        .expect("pkg init");
    assert!(
        init.status.success(),
        "pkg init: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    assert!(tmp.join("sn.toml").exists());
    assert!(tmp.join("sn.lock.toml").exists());

    // Copy mylib as a path dep fixture.
    let mylib_src = root.join("packages/mylib");
    let mylib_dst = tmp.join("packages/mylib");
    std::fs::create_dir_all(&mylib_dst).unwrap();
    for name in ["sn.toml", "calc.sn", "utils.sn"] {
        std::fs::copy(mylib_src.join(name), mylib_dst.join(name)).unwrap();
    }
    let add = Command::new(snc_bin())
        .args(["pkg", "add", "mylib", "--path", "packages/mylib"])
        .current_dir(&tmp)
        .output()
        .expect("pkg add");
    assert!(
        add.status.success(),
        "pkg add: {}",
        String::from_utf8_lossy(&add.stderr)
    );
    let lock = std::fs::read_to_string(tmp.join("sn.lock.toml")).unwrap();
    assert!(lock.contains("mylib"), "lock missing mylib: {lock}");
    assert!(lock.contains("0.1.0"), "lock missing version: {lock}");

    let pub_out = Command::new(snc_bin())
        .args(["pkg", "publish"])
        .current_dir(&tmp)
        .output()
        .expect("pkg publish");
    assert!(
        pub_out.status.success(),
        "pkg publish: {}\n{}",
        String::from_utf8_lossy(&pub_out.stderr),
        String::from_utf8_lossy(&pub_out.stdout)
    );
    let dist = tmp.join("dist/demo-0.1.0.tar.gz");
    assert!(dist.exists(), "missing {}", dist.display());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn static_on_instance_fail() {
    let err = compile_err("examples/static_on_instance_fail.sn");
    assert!(err.contains("is static"), "expected static error, got: {err}");
}

#[test]
fn static_self_fail() {
    let err = compile_err("examples/static_self_fail.sn");
    assert!(
        err.contains("'self' not allowed in a static method"),
        "expected static self error, got: {err}"
    );
}

#[test]
fn instance_as_static_fail() {
    let err = compile_err("examples/instance_as_static_fail.sn");
    assert!(
        err.contains("is not static"),
        "expected not-static error, got: {err}"
    );
}

#[test]
fn abstract_instantiate_fail() {
    let err = compile_err("examples/abstract_instantiate_fail.sn");
    assert!(
        err.contains("cannot instantiate abstract blueprint"),
        "expected abstract instantiation error, got: {err}"
    );
}

#[test]
fn abstract_missing_fail() {
    let err = compile_err("examples/abstract_missing_fail.sn");
    assert!(
        err.contains("not implemented 'area'"),
        "expected missing-impl error, got: {err}"
    );
}

#[test]
fn closed_method_fail() {
    let err = compile_err("examples/closed_method_fail.sn");
    assert!(
        err.contains("is closed on Account"),
        "expected closed-method error, got: {err}"
    );
}

#[test]
fn generic_bound_fail() {
    let err = compile_err("examples/generic_bound_fail.sn");
    assert!(
        err.contains("does not satisfy bound 'Printable'"),
        "expected bound error, got: {err}"
    );
}

#[test]
fn generic_variance_fail() {
    let err = compile_err("examples/generic_variance_fail.sn");
    assert!(
        err.contains("cannot assign Box_Dog to Box_Animal"),
        "expected invariant-variance error, got: {err}"
    );
}

/// `--target` accepts friendly presets and expands them to real LLVM triples,
/// and cross-compiles to a genuine foreign-architecture binary.
#[test]
fn cross_compile_targets() {
    let root = repo_root();
    let snc = snc_bin();
    let src = root.join("examples/hello_world.sn");

    let cases: &[(&str, &str)] = &[
        ("macos", "x86_64-apple-macosx"),
        ("macos-arm64", "aarch64-apple-macosx"),
        ("macos-x64", "x86_64-apple-macosx"),
        ("linux", "x86_64-unknown-linux-gnu"),
        ("linux-arm64", "aarch64-unknown-linux-gnu"),
        ("windows", "x86_64-pc-windows-msvc"),
        ("windows-arm64", "aarch64-pc-windows-msvc"),
    ];
    for (preset, expect) in cases {
        let ll = std::env::temp_dir().join(format!("snc-xt-{}.ll", preset));
        let out = Command::new(&snc)
            .arg(&src)
            .arg("--emit-llvm")
            .arg("-o")
            .arg(&ll)
            .arg("--target")
            .arg(*preset)
            .current_dir(&root)
            .output()
            .expect("run snc");
        assert!(
            out.status.success(),
            "target {preset} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let ir = std::fs::read_to_string(&ll).expect("read ir");
        assert!(
            ir.contains(expect),
            "target {preset} produced the wrong triple: {expect}"
        );
    }

    // A full triple passes through untouched.
    let ll = std::env::temp_dir().join("snc-xt-explicit.ll");
    let out = Command::new(&snc)
        .arg(&src)
        .arg("--emit-llvm")
        .arg("-o")
        .arg(&ll)
        .arg("--target")
        .arg("riscv64-unknown-linux-gnu")
        .current_dir(&root)
        .output()
        .expect("run snc");
    assert!(out.status.success(), "explicit triple failed");
    assert!(std::fs::read_to_string(&ll)
        .unwrap()
        .contains("riscv64-unknown-linux-gnu"));

    // An unknown name is a clear error, not a silently wrong triple.
    let out = Command::new(&snc)
        .arg(&src)
        .arg("--emit-llvm")
        .arg("-o")
        .arg(std::env::temp_dir().join("snc-xt-bad.ll"))
        .arg("--target")
        .arg("commodore64")
        .current_dir(&root)
        .output()
        .expect("run snc");
    assert!(!out.status.success(), "bogus target should be rejected");
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown target"));

    // Cross-compile for real: an x86_64 binary on an arm64 host.
    let bin = std::env::temp_dir().join("snc-xt-x64");
    let _ = std::fs::remove_file(&bin);
    let out = Command::new(&snc)
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .arg("--target")
        .arg("macos-x64")
        .current_dir(&root)
        .output()
        .expect("run snc");
    if out.status.success() {
        let bytes = std::fs::read(&bin).expect("read binary");
        assert!(
            bytes.windows(4).any(|w| w == [0xCF, 0xFA, 0xED, 0xFE]),
            "expected a 64-bit little-endian Mach-O"
        );
        let _ = std::fs::remove_file(&bin);
    }
}

#[test]
fn inline_asm_emits_and_runs() {
    assert_out("examples/inline_asm.sn", "asm ok\n");

    // The IR must contain a real LLVM inline-asm call.
    let root = repo_root();
    let ll = std::env::temp_dir().join("snc-inlineasm.ll");
    let out = Command::new(snc_bin())
        .arg(root.join("examples/inline_asm.sn"))
        .arg("--emit-llvm")
        .arg("-o")
        .arg(&ll)
        .current_dir(&root)
        .output()
        .expect("run snc");
    assert!(out.status.success(), "emit-llvm failed");
    let ir = std::fs::read_to_string(&ll).expect("read ir");
    assert!(
        ir.contains("asm sideeffect"),
        "expected inline asm in IR, got:\n{ir}"
    );
    let _ = std::fs::remove_file(&ll);

    // The same syntax must lower correctly for every target triple we
    // support; compiling is IR-only here, so no host toolchain needed.
    for preset in ["linux-x64", "windows-x64", "macos-arm64"] {
        let out = Command::new(snc_bin())
            .arg(root.join("examples/inline_asm.sn"))
            .arg("--emit-llvm")
            .arg("-o")
            .arg(&ll)
            .arg("--target")
            .arg(preset)
            .current_dir(&root)
            .output()
            .expect("run snc");
        assert!(
            out.status.success(),
            "target {preset} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let ir = std::fs::read_to_string(&ll).expect("read ir");
        assert!(
            ir.contains("asm sideeffect"),
            "target {preset} missing inline asm:\n{ir}"
        );
    }
    let _ = std::fs::remove_file(&ll);
}

#[test]
fn test_runner_reports_statement_coverage() {
    let root = repo_root();
    let out = Command::new(snc_bin())
        .arg("test")
        .arg("--root")
        .arg("examples")
        .arg("std_test")
        .arg("--coverage")
        .current_dir(&root)
        .output()
        .expect("run snc test");
    assert!(
        out.status.success(),
        "snc test failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("1 passed, 0 failed"), "{stdout}");
    assert!(stdout.contains("coverage:"), "{stdout}");
    assert!(stdout.contains("std_test.sn"), "{stdout}");
}

#[test]
fn debug_drives_lldb_with_sn_source_lines() {
    // lldb only ships with Xcode/CLT; elsewhere there is nothing to drive.
    let has_lldb = Command::new("lldb")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_lldb {
        return;
    }
    let root = repo_root();
    let bin = std::env::temp_dir().join(format!("snc-dbg-{}", std::process::id()));
    let out = Command::new(snc_bin())
        .arg("debug")
        .arg("examples/hello_world.sn")
        .arg("-o")
        .arg(&bin)
        .current_dir(&root)
        .output()
        .expect("run snc debug");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "snc debug failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("sn_fn_main"), "{stdout}");
    assert!(stdout.contains("hello_world.sn:2"), "{stdout}");
}
