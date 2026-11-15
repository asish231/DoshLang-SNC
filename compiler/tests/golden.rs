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
fn use_std_web() {
    assert_out(
        "examples/use_std_web.sn",
        "true\n200\n{\"status\":\"operational\"}\n200\n{\"echo\":\"ping\"}\n204\ntrue\ntrue\n200\n{\"id\":\"42\"}\n302\n/home\n",
    );
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

/// SWARM RABBIT HOLE — Wave 6 / DIFF-1 Rule 1b (kills G1 §1 A5a): an
/// inout parameter GROWN across recursive SCC re-entries is recorded in
/// FnBorrowSummary.scc_accum_params by the Rule-1a fixpoint, and passing a
/// `lane_alloc`-backed raw local (frame-local scratch — the single-lane
/// model, any lane that isn't pinned) inout into it is the buried-top
/// hazard: intervening scope exits would truncate its tail while its
/// length still counts the truncated bytes. → E0802.
#[test]
fn scc_accum_lane_scratch_e0802() {
    let err = compile_err("examples/scc_accum_lane_fail.sn");
    assert!(err.contains("[E0802]"), "expected E0802, got: {err}");
    assert!(
        err.contains("SCC accumulator 'buf'"),
        "expected the buried-top diagnostic for 'buf', got: {err}"
    );
}

/// SWARM RABBIT HOLE — Wave 8 (Impl-J): W2A §1.5 T-PUSH/T-STORE — the
/// escape rule (E-ESCAPE). A ref whose TARGET lives in the shorter
/// (inner) region — it exits first — is stored into a holder declared in
/// the enclosing region: the safety law ∀ρᵢ ∈ refs(v): ρᵢ ⊑ₗ reg(dst)
/// (W2A §1.1; §4.1 P-I, the RI invariant reg(a) ⊑ₗ reg(cell)) is
/// violated — the target dies before the holder, the dangling-reference
/// class the region poset was built to kill. Both store sites must
/// reject it: the T-STORE binding assignment and the T-PUSH list push.
#[test]
fn escape_ref_into_shorter_region_fail() {
    let err = compile_err("examples/escape_ref_shorter_region_fail.sn");
    assert!(
        err.contains("[E-ESCAPE]"),
        "expected the E-ESCAPE diagnostic, got: {err}"
    );
    assert!(
        err.contains("the target dies before the holder"),
        "expected the dangling-reference explanation, got: {err}"
    );
    assert!(
        err.contains("escapes into region 0"),
        "expected the holder region named in the message, got: {err}"
    );
}

/// SWARM RABBIT HOLE — Wave 8 (Impl-J) discharge: refs created and
/// stored within the SAME region — reg(target) == reg(holder), ⊑ₗ by
/// reflexivity — the push site and the assignment site must both stay
/// silent, and the program runs.
#[test]
fn escape_ref_into_same_region_ok() {
    assert_out("examples/escape_ref_same_region_ok.sn", "7\n1\n");
}

/// SWARM RABBIT HOLE — Wave 8 (Impl-J) discharge: refs CREATED in an
/// inner region whose targets live in the OUTER region, stored into
/// outer-region holders (container + binding). reg(target) ⊑ₗ
/// reg(holder) — outer targets outlive every inner region (W2A §1.1:
/// ⊑ₗ = phase superset) — so the rule must NOT fire, and the holder
/// stays valid after the region exits.
#[test]
fn escape_ref_into_outer_ok() {
    assert_out("examples/escape_ref_outer_ok.sn", "5\n1\n");
}

/// SWARM RABBIT HOLE — Wave 12 (Impl-Q): the push_ptr pointee retention
/// fix — closing Impl-J's Wave-8 flag (escape_ref_outer_ok deferred the
/// runtime retain to this tier). A `ref<int>` of a local pushed into a
/// `list<ref<int>>` is a borrow of the slot: the runtime must not ARC the
/// slot address (the old blind sn_retain treated the slot as an Obj
/// header and clobbered the local 99 -> 100), and the list read must
/// give the right pointee. Both push spellings (named ref, inline
/// address()).
#[test]
fn list_ref_local_roundtrip_ok() {
    assert_out("examples/list_ref_local_roundtrip_ok.sn", "99\n99\n99\n99\n");
}

/// The discharge side of the same rule: a heap-backed list local passed
/// inout into the recursive accumulator has no lane assignment, so the
/// E0802 wiring must NOT over-fire off the lane model — the program
/// compiles and runs (4 pushes: 3, 2, 1, 0).
#[test]
fn scc_accum_heap_list_ok() {
    assert_out("examples/scc_accum_ok.sn", "4\n");
}

/// SWARM RABBIT HOLE — Wave 8 / Impl-I, DIFF-1 Rule 1b pre-coloring +
/// Rule 1d budget (W2A §1.3 S6, §1.7 K1 + the budget lemma): the
/// pinned-lane discharge, end-to-end. A recursive SCC accumulator with
/// a lane-free allocation site is PRE-COLORED onto a pinned lane L2..L6
/// at compile time; the IR constructs the growable ON the lane
/// (sn_list_new_lane → sn_lane_alloc(π_j)), so E0802 does not fire, the
/// list survives the recursion (K1: nothing else opens on π_j), and
/// the pinned lane actually holds the allocation (lane_used(2) > 0 —
/// the full check → color → lower → run loop).
#[test]
fn scc_accum_pinned_lane_ok() {
    assert_out("examples/scc_accum_pinned_lane_ok.sn", "4\ntrue\n");

    // The explicit discharge: `lane_alloc` on a pinned lane (L2..L6) is
    // the user-declared pin the coloring prefers — compiles clean, no
    // E0802 (the scratch twin is scc_accum_lane_scratch_e0802).
    let err = compile_err("examples/scc_accum_lane_pinned_ok.sn");
    assert!(err.is_empty(), "expected a clean compile, got: {err}");
}

/// SWARM RABBIT HOLE — Wave 8 (Impl-L): GC scoping — D0 gap #6 + W2A
/// §1.4's memory categories — the E-GC-IN-LANE rule. GC roots are
/// globals+stacks+compiler-pushed values only: a GC-managed pointer
/// stored into lane memory (untracked, never gc_track'd) is invisible
/// to the collector and can be swept while live. A ref to a GC-managed
/// binding ('p', the `new` → sn_obj_new → gc_track path) stored into a
/// lane-backed destination ('lane_box', the explicit `lane_alloc` pin)
/// must fire E-GC-IN-LANE at the T-STORE site. The single-region ⊑ₗ
/// poset cannot see the hazard — both bindings share region 0 — so only
/// the category rule rejects it.
#[test]
fn gc_ref_into_lane_fail() {
    let err = compile_err("examples/gc_ref_into_lane_fail.sn");
    assert!(
        err.contains("[E-GC-IN-LANE]"),
        "expected the E-GC-IN-LANE diagnostic, got: {err}"
    );
    assert!(
        err.contains("reference to 'p' (GC-managed) stored into 'lane_box' (lane-backed)"),
        "expected the category-confusion explanation naming both sides, got: {err}"
    );
    assert!(
        err.contains("the GC cannot trace into lanes"),
        "expected the sweep-hazard explanation, got: {err}"
    );
}

/// SWARM RABBIT HOLE — Wave 8 (Impl-L): the inverse direction — a
/// lane-backed binding's ref stored into a GC-managed destination. The
/// lane rewinds at its extent while the GC holder persists, so the
/// stored lane reference dangles: same category confusion, opposite
/// direction (D0 gap #6; W2A §1.4). Must fire at the T-STORE site with
/// the destination's provenance root ('h').
#[test]
fn lane_ref_into_gc_fail() {
    let err = compile_err("examples/lane_ref_into_gc_fail.sn");
    assert!(
        err.contains("[E-GC-IN-LANE]"),
        "expected the E-GC-IN-LANE diagnostic, got: {err}"
    );
    assert!(
        err.contains("reference to 'lane_cell' (lane-backed) stored into 'h' (GC-managed)"),
        "expected the category-confusion explanation naming both sides, got: {err}"
    );
}

/// SWARM RABBIT HOLE — Wave 8 (Impl-L) discharge: GC ref into a
/// GC-managed destination — the normal case. Both sides are the GC
/// heap's category ('p' via `new`, 'cells' via the list literal →
/// sn_list_new → gc_track), the collector owns the whole chain, the
/// rule must stay silent and the program runs (list holds the pushed
/// ref; lane 2 untouched).
#[test]
fn gc_ref_into_gc_ok() {
    assert_out("examples/gc_ref_into_gc_ok.sn", "1\nfalse\n");
}

/// SWARM RABBIT HOLE — Wave 8 (Impl-L) discharge: lane ref into a
/// lane-backed destination. Both sides are the lane-bytes category —
/// the lane owns its own references and rewinds them together, so the
/// rule must stay silent and the program runs (the lane-2 watermark
/// proves the allocations landed).
#[test]
fn lane_ref_into_lane_ok() {
    assert_out("examples/lane_ref_into_lane_ok.sn", "true\n");
}

/// DIFF-1 Rule 1d (the budget lemma, W2A §1.7): the sixth distinct
/// simultaneously-live accumulator in one recursion SCC exhausts the
/// L2..L6 pinned pool — never silent reuse, the coloring leaves it
/// lane-less and its call site fires the budget error. → E0802c.
#[test]
fn scc_accum_lane_budget_e0802c() {
    let err = compile_err("examples/scc_accum_lane_budget_fail.sn");
    assert!(err.contains("[E0802]"), "expected E0802, got: {err}");
    assert!(
        err.contains("needs a pinned lane but L2..L6 are all live"),
        "expected the E0802c budget diagnostic, got: {err}"
    );
    assert!(
        err.contains("'f'"),
        "expected the diagnostic for the sixth accumulator 'f', got: {err}"
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
    // Build the tiny C shim the example links against, directly at the
    // absolute path the example names. (No temp+copy: when TMPDIR is /tmp,
    // as on Linux CI, a copy would truncate the file onto itself.)
    let root = repo_root();
    let linked = PathBuf::from("/tmp/sn_ffi_shim.o");
    let cc = Command::new("clang")
        .arg("-c")
        .arg("-o")
        .arg(&linked)
        .arg(root.join("compiler/tests/shim.c"))
        .output()
        .expect("run clang");
    assert!(cc.status.success(), "shim compile failed");

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
fn subprocess_exec() {
    assert_out(
        "examples/subprocess.sn",
        "exec code: 0\nexec out: hello from os.exec\nsystem code: 0\nsystem out: hello from os.system\n",
    );
}

#[test]
fn escaped_braces() {
    assert_out(
        "examples/escaped_braces.sn",
        "literal: {abc}\ninterpolated: Ada and literal: {xyz}\n",
    );
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

#[test]
fn algebraic_enums() {
    assert_out(
        "examples/algebraic_enums.sn",
        "foo_var\n42\nEOF\nmatched foo_var\nmatched EOF\n",
    );
}

#[test]
fn stress_algebraic_enums() {
    assert_out(
        "examples/stress_algebraic_enums.sn",
        "100\n42\n42\n0\nn2 equals eq_test1\nn2 not equals eq_test2\nn4 is Empty\n",
    );
}

#[test]
fn generic_fn() {
    assert_out(
        "examples/generic_fn.sn",
        "42\nhello generics\n99\n10\nalpha\n777\n5\n5\n1234\nbeep boop\n",
    );
}

#[test]
fn lane_poison_flag_fills_inline_bump() {
    // SWARM RABBIT HOLE, Wave 6 (Impl-D, bench/RESULTS.md §5): SN_POISON=1
    // was a silent no-op on the inline fast path (the bump never entered
    // the runtime). `--poison` must make the 0xAA fill unconditional on
    // every codegen path. Default -O2 keeps the constant-size bump on the
    // inline path — exactly the path the finding is about — and the second
    // region proves the rewind re-poison (Impl-B's inline-restore residual).
    let root = repo_root();
    let src = root.join("tests/systems/lane_poison_fill.sn");
    let out = std::env::temp_dir().join(format!("snc-poison-{}", std::process::id()));
    let compile = Command::new(snc_bin())
        .arg(&src)
        .arg("--poison")
        .arg("-o")
        .arg(&out)
        .current_dir(&root)
        .output()
        .expect("run snc --poison");
    assert!(
        compile.status.success(),
        "snc --poison failed: {}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = Command::new(&out).output().expect("run poison binary");
    let _ = std::fs::remove_file(&out);
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "poison ok\n",
        "inline bump poison fill / rewind re-poison failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
}

// ---------------------------------------------------------------------
// SWARM RABBIT HOLE — Wave 9 (Impl-M, Cohort Delta): the Tier-L
// join-semilattice cell (W2C §1.1 Tier L; final report §8.1) — the
// compiler-proved semilattice tier's first executable slice. Carriers
// with checked lattice laws, the delta-mutator law
// m(X) = X ⊔ m^δ(X), NO ⊖ on the Tier-L surface, and the (L, exact)
// rejection made executable (E-LRET — the Grothendieck argument as a
// checked error).
// ---------------------------------------------------------------------

/// MAX carrier: the high-water mark — monotone inflation only (an
/// offer below the HWM is a no-op), ⊔ = max, the identity element is
/// ⊥ = INT64_MIN, and the mutator law's round-trip + quiescence are
/// observable end-to-end through the compiler.
#[test]
fn lcell_max_hwm() {
    assert_out(
        "examples/lcell_max_hwm.sn",
        "0\n-9223372036854775808\n10\n10\n42\n1\n42\n1\n42\n42\n42\ntrue\n0\n\
         -9223372036854775808\nPASS: lcell_max_hwm\n",
    );
}

/// GSET carrier: the grow-only int set — idempotent inserts, the
/// union merge, and commutativity observable end-to-end
/// (a⊔b == b⊔a via readouts), plus the state-relative fragment
/// (delta is NOT a copy) and the mutator-law round-trip.
#[test]
fn lcell_gset_union() {
    assert_out(
        "examples/lcell_gset_union.sn",
        "2\ntrue\ntrue\nfalse\n3\ntrue\ntrue\ntrue\n3\ntrue\ntrue\ntrue\n2\n\
         2\nfalse\ntrue\ntrue\n3\ntrue\ntrue\ntrue\n0\nPASS: lcell_gset_union\n",
    );
}

/// LWW carrier: the logical clock wins, ties resolve by the
/// DETERMINISTIC value tiebreak in either merge order (commutativity
/// depends on the tiebreak being order-independent), losing offers are
/// no-ops, and the mutator law round-trips.
#[test]
fn lcell_lww_tiebreak() {
    assert_out(
        "examples/lcell_lww_tiebreak.sn",
        "0\n0\n100\n200\n200\n1\n7\n7\ntrue\n42\n1\n7\n1\n7\n1\n0\n\
         PASS: lcell_lww_tiebreak\n",
    );
}

/// The mutator law m(X) = X ⊔ m^δ(X) across ALL THREE carriers
/// (Almeida–Shoker–Baquero JPDC 2018; Enes et al. 2018): the
/// round-trip prev ⊔ delta(prev,dst) == dst, the state-relative
/// fragment property (delta carries only what moved), quiescence
/// delta(x,x) = ⊥, and delta replay converging from a common base.
#[test]
fn lcell_mutator_law() {
    assert_out("examples/lcell_mutator_law.sn", "PASS: lcell_mutator_law\n");
}

/// The (L, exact) rejection made executable — the Grothendieck
/// argument as a checked compile error: x⊔x = x ⇒ [x] = 0, so the
/// idempotent merge operator does not group-embed and exact retraction
/// on the Tier-L carrier is impossible. `lcell_retract` is a RECOGNIZED
/// intrinsic (a named, checked rejection — not "unknown function")
/// that the checker ALWAYS rejects with E-LRET, pointing at Tier G.
#[test]
fn lcell_retract_rejected_e_lret() {
    let err = compile_err("examples/lcell_retract_fail.sn");
    assert!(err.contains("[E-LRET]"), "expected E-LRET, got: {err}");
    assert!(
        err.contains("exact retraction on a Tier-L join-semilattice is impossible"),
        "expected the E-LRET headline, got: {err}"
    );
    assert!(
        err.contains("x⊔x = x ⇒ [x] = 0"),
        "expected the Grothendieck triviality argument in the diagnostic, got: {err}"
    );
    assert!(
        err.contains("tombstoned or none, never exact"),
        "expected the legal Tier-L retraction labels, got: {err}"
    );
    assert!(
        err.contains("use a Tier-G Z-set"),
        "expected the Tier-G pointer in the diagnostic, got: {err}"
    );
}

/// The per-carrier readout surface (HWM value; set size + membership;
/// the LWW winner) and the carrier tag — the honest-label plumbing.
#[test]
fn lcell_carriers() {
    assert_out(
        "examples/lcell_carriers.sn",
        "0\n7\n1\ntrue\nfalse\n1\n2\ntrue\nfalse\n0\n2\n11\n1\nfalse\n\
         PASS: lcell_carriers\n",
    );
}

/// The std.lattice surface: the LCell blueprint over the runtime
/// engine (the std.delta/ZSet wrapper pattern) — named carrier
/// constructors, merge/delta/apply as methods, and the mutator law
/// through the wrapper. No retract method exists on the surface: the
/// (L, exact) capability is rejected at the intrinsic level.
#[test]
fn lcell_stdlib_surface() {
    assert_out(
        "examples/lcell_stdlib.sn",
        "10\n1\n3\ntrue\nfalse\n1\n2\ntrue\n200\n2\nPASS: lcell_stdlib\n",
    );
}

// ---------------------------------------------------------------------
// SWARM RABBIT HOLE — Wave 9 (Impl-N, Cohort Delta): Salt ζ — the
// fixpoint-bracket semantics and the admission checker made executable
// (W2C §3.3–§3.5). The convergence theorem's sufficient condition as
// a checked, runtime-executable bracket: monotone body (in the
// recursive variable) + finite-height per-key carrier ⇒ the δ₀/∫
// bracket terminates in ≤ height rounds at the LEAST fixpoint
// (Knaster–Tarski; Cousot–Cousot Pacific J. Math. 82(1) 1979). The
// two canonical divergence classes are recognized intrinsics, ALWAYS
// rejected with E-ZADM at compile time (refusals are compile-time
// errors, never silent divergence — W2C §3.5).
// ---------------------------------------------------------------------

/// The ADMITTED body end-to-end: transitive closure
/// reach(a) :- edge(a,b), reach(b) over the chain 0→1→…→9 seeded at
/// the far end — monotone in the recursive variable, carrier =
/// presence {0,1} over the finite node universe. Converges at the
/// least fixpoint in exactly the diameter many rounds (the ≤ height
/// bound observable, equality on a chain), with the semi-naïve probe
/// count exact (Thm 4b: each round derives only from the previous
/// round's delta).
#[test]
fn zfix_closure_chain() {
    assert_out(
        "examples/zfix_closure_chain.sn",
        "10\n9\n9\ntrue\ntrue\nfalse\n9\ntrue\nPASS: zfix_closure_chain\n",
    );
}

/// Divergence class 1 — counting-through-recursion (r(a) :- r(a)
/// with seed 1): the body increments a ℤ-valued count through the
/// recursive variable each round, so the per-key carrier has INFINITE
/// height and the convergence theorem's premise fails (monotone ≠
/// finite-height, BOTH required). `zfix_count` is a RECOGNIZED
/// intrinsic (a named, checked rejection — not "unknown function")
/// that the checker ALWAYS rejects with E-ZADM.
#[test]
fn zfix_count_rejected_e_zadm() {
    let err = compile_err("examples/zfix_count_fail.sn");
    assert!(err.contains("[E-ZADM]"), "expected E-ZADM, got: {err}");
    assert!(
        err.contains("fixpoint body diverges: counting-through-recursion"),
        "expected the E-ZADM headline, got: {err}"
    );
    assert!(
        err.contains("infinite height"),
        "expected the infinite-height carrier argument in the diagnostic, got: {err}"
    );
    assert!(
        err.contains("monotone ≠ finite-height, both required"),
        "expected the both-premises-required argument in the diagnostic, got: {err}"
    );
    assert!(
        err.contains("the bracket would never quiesce"),
        "expected the non-quiescence verdict in the diagnostic, got: {err}"
    );
}

/// Divergence class 2 — negation-through-recursion (μy. 1⊖y): the
/// body applies ⊖/negation through the recursive variable, so it is
/// NOT monotone in the recursive variable and Knaster–Tarski does not
/// apply — the failure is the non-monotone body, tier-orthogonal
/// (1⊖y is group arithmetic). `zfix_negate` is a RECOGNIZED intrinsic
/// that the checker ALWAYS rejects with E-ZADM.
#[test]
fn zfix_negate_rejected_e_zadm() {
    let err = compile_err("examples/zfix_negate_fail.sn");
    assert!(err.contains("[E-ZADM]"), "expected E-ZADM, got: {err}");
    assert!(
        err.contains("fixpoint body diverges: negation-through-recursion"),
        "expected the E-ZADM headline, got: {err}"
    );
    assert!(
        err.contains("NOT monotone in the recursive variable"),
        "expected the non-monotone-body argument in the diagnostic, got: {err}"
    );
    assert!(
        err.contains("Knaster–Tarski does not apply"),
        "expected the Knaster–Tarski refutation in the diagnostic, got: {err}"
    );
    assert!(
        err.contains("μy. 1⊖y oscillates"),
        "expected the oscillation example in the diagnostic, got: {err}"
    );
    assert!(
        err.contains("tier-orthogonal"),
        "expected the tier-orthogonality note in the diagnostic, got: {err}"
    );
}

/// The Tier-L variant — the same closure body over a GSET LCell
/// carrier (⊔ = union is the join): Tier-L carriers qualify BY
/// CONSTRUCTION (W2C §3.3's admission rule), so the bracket converges
/// at the same least fixpoint in the same number of rounds, and the
/// Tier-L stop condition F(s) ⊑ s (the mutator-law test — 𝒟
/// presupposes ⊖, absent on Tier L) is exercised end-to-end.
#[test]
fn zfix_lcell_closure() {
    assert_out(
        "examples/zfix_lcell_closure.sn",
        "10\n9\ntrue\nfalse\ntrue\n10\n9\ntrue\nPASS: zfix_lcell_closure\n",
    );
}

/// The quiescence observables: a seed-only graph (0 productive
/// rounds — quiescent at once, 𝒟(∫α) = 0) and the diamond (2 rounds,
/// 4 facts, exact probe count) — the per-tier stop conditions and the
/// rounds/len readouts of the bracket.
#[test]
fn zfix_quiescence() {
    assert_out(
        "examples/zfix_quiescence.sn",
        "0\n1\ntrue\ntrue\n2\n4\n4\ntrue\nPASS: zfix_quiescence\n",
    );
}

/// The std.zeta surface: the ZFix blueprint over the runtime SnZFix
/// engine (the std.delta/ZSet wrapper pattern). No divergent body
/// exists on the surface: zfix_count/zfix_negate have no methods —
/// they are recognized intrinsics always rejected with E-ZADM.
#[test]
fn zfix_stdlib_surface() {
    assert_out(
        "examples/zfix_stdlib.sn",
        "9\n10\ntrue\nfalse\n9\nPASS: zfix_stdlib\n",
    );
}

// =====================================================================
// SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the global-pool
// discipline (kills G1 §1 A6c). Three declared modes, frozen by
// default; the `unfrozen (P) { ... }` window; E0803.1/.2/.3 at the
// spawn/chan capture seam and the mutation seam.
// =====================================================================

/// E0803.3 — frozen by default: insert/set/remove on a bare (frozen)
/// global pool outside an `unfrozen` window are rejected, all three
/// ops, with the verbatim diagnostic.
#[test]
fn gpool_frozen_default_rejected_e0803_3() {
    let err = compile_err("examples/gpool_frozen_default_fail.sn");
    assert!(err.contains("[E0803]"), "expected E0803, got: {err}");
    assert!(
        err.contains("E0803.3: cannot mutate global pool 'gfrozen' (insert)"),
        "expected the verbatim E0803.3 insert headline, got: {err}"
    );
    assert!(
        err.contains("cannot mutate global pool 'gfrozen' (set)"),
        "expected the verbatim E0803.3 set headline, got: {err}"
    );
    assert!(
        err.contains("cannot mutate global pool 'gfrozen' (remove)"),
        "expected the verbatim E0803.3 remove headline, got: {err}"
    );
    assert!(
        err.contains("global pools are frozen by default"),
        "expected the frozen-by-default statement, got: {err}"
    );
    assert!(
        err.contains("Wrap the mutation in 'unfrozen (gfrozen) { ... }'"),
        "expected the window discharge, got: {err}"
    );
    assert!(
        err.contains("or declare the pool 'thread_local' (single-thread) or 'synchronized' (locked)"),
        "expected the two mode discharges, got: {err}"
    );
}

/// The `unfrozen (P) { ... }` window: mutation inside is legal on the
/// declaring thread; at window exit the pool re-freezes and reads
/// observe the mutated state. Runs end-to-end.
#[test]
fn gpool_unfrozen_window_ok() {
    assert_out("examples/gpool_unfrozen_ok.sn", "99\n1\n");
}

/// The Sendable frozen read: a frozen global pool's handle captured by
/// an unscoped spawn OUTSIDE any window is legal (no legal write path
// exists — the freelist cannot be raced) and reads correctly.
#[test]
fn gpool_frozen_spawn_ok() {
    assert_out("examples/gpool_frozen_spawn_ok.sn", "21\ntrue\n");
}

/// E0803.2 — the A6c kill: capturing a frozen pool's handle (or the
/// pool object) in an unscoped spawn INSIDE an `unfrozen` window is
/// rejected with the verbatim diagnostic — the receiving thread could
/// race the freelist with the mutator.
#[test]
fn gpool_unfrozen_spawn_rejected_e0803_2() {
    let err = compile_err("examples/gpool_unfrozen_spawn_fail.sn");
    assert!(err.contains("[E0803]"), "expected E0803, got: {err}");
    assert!(
        err.contains("E0803.2: cannot capture/send handle 'h' from pool 'gz' in unscoped spawn/chan"),
        "expected the verbatim E0803.2 handle headline, got: {err}"
    );
    assert!(
        err.contains("'gz' is unfrozen in this scope (mutable window)"),
        "expected the mutable-window verdict, got: {err}"
    );
    assert!(
        err.contains("the receiving thread could race its freelist"),
        "expected the freelist-race argument, got: {err}"
    );
    assert!(
        err.contains("Move the capture outside the 'unfrozen (gz)' block, or declare 'gz' 'synchronized'"),
        "expected both discharges, got: {err}"
    );
    assert!(
        err.contains("cannot access pool 'gz' in unscoped spawn/chan"),
        "expected the pool-object flavor of E0803.2, got: {err}"
    );
}

/// E0803.1 — `thread_local` pools: handles (and the pool object) are
/// confined to the declaring thread; any spawn/chan capture is
/// rejected.
#[test]
fn gpool_thread_local_spawn_rejected_e0803_1() {
    let err = compile_err("examples/gpool_thread_local_spawn_fail.sn");
    assert!(err.contains("[E0803]"), "expected E0803, got: {err}");
    assert!(
        err.contains("E0803.1: cannot capture/send handle 'h' from pool 'gtl' in unscoped spawn/chan"),
        "expected the E0803.1 handle headline, got: {err}"
    );
    assert!(
        err.contains("'gtl' is 'thread_local'"),
        "expected the thread_local verdict, got: {err}"
    );
    assert!(
        err.contains("cannot access pool 'gtl' in unscoped spawn/chan"),
        "expected the pool-object flavor of E0803.1, got: {err}"
    );
    assert!(
        err.contains("Declare 'gtl' 'synchronized' for cross-thread access"),
        "expected the synchronized discharge, got: {err}"
    );
}

/// The Synchronized mode: mutated from the main thread (legal — every
/// op locked) and its handle spawned (legal — Sendable everywhere).
/// Runs end-to-end.
#[test]
fn gpool_synchronized_ok() {
    assert_out("examples/gpool_synchronized_ok.sn", "42\n1\n");
}

/// Frozen-read correctness end-to-end: insert via the window, then
/// read both handles from a spawned task — generations stable, values
/// exact, across the thread boundary.
#[test]
fn gpool_frozen_read_spawn_ok() {
    assert_out("examples/gpool_frozen_read_spawn_ok.sn", "18\n2\n");
}

/// E0803.2's chan flavor: sending a frozen pool's handle over a
/// channel while the pool is unfrozen at the send site — the same
/// freelist race on the channel seam.
#[test]
fn gpool_chan_send_rejected_e0803_2() {
    let err = compile_err("examples/gpool_chan_send_fail.sn");
    assert!(err.contains("[E0803]"), "expected E0803, got: {err}");
    assert!(
        err.contains("E0803.2: cannot capture/send handle 'h' from pool 'gq' in unscoped spawn/chan"),
        "expected the verbatim E0803.2 chan headline, got: {err}"
    );
    assert!(
        err.contains("'gq' is unfrozen in this scope (mutable window)"),
        "expected the mutable-window verdict, got: {err}"
    );
}

/// The spawn-body discipline: a LOCAL pool object is not Sendable
/// (the owning frame can die while the task runs; two threads on one
/// freelist is exactly A6c), and a frozen global pool mutated from a
/// spawned task races the window's freelist — only 'synchronized'
/// pools accept cross-thread mutation.
#[test]
fn gpool_spawn_body_rejected_e0803() {
    let err = compile_err("examples/gpool_spawn_body_fail.sn");
    assert!(err.contains("[E0803]"), "expected E0803, got: {err}");
    assert!(
        err.contains("cannot access local pool 'lp' in unscoped spawn/chan"),
        "expected the local-pool Sendable rejection, got: {err}"
    );
    assert!(
        err.contains("a local pool's storage and freelist are not Sendable"),
        "expected the local-pool rationale, got: {err}"
    );
    assert!(
        err.contains("cannot mutate pool 'gw' (insert) inside a spawned task"),
        "expected the spawn-body mutation rejection, got: {err}"
    );
    assert!(
        err.contains("only 'synchronized' pools accept cross-thread mutation"),
        "expected the synchronized-only rule, got: {err}"
    );
}

/// The int-COPY provenance: handles are held in `int` variables on
/// this surface, so `int hcopy = h` must keep the minting pool's tag
/// — the copy is captured inside the window and E0803.2 fires on the
/// copy exactly as on the original (without the copy tracking, the
/// A6c hole reopens through a copy).
#[test]
fn gpool_handle_copy_rejected_e0803_2() {
    let err = compile_err("examples/gpool_handle_copy_fail.sn");
    assert!(err.contains("[E0803]"), "expected E0803, got: {err}");
    assert!(
        err.contains("E0803.2: cannot capture/send handle 'hcopy' from pool 'gz' in unscoped spawn/chan"),
        "expected the E0803.2 headline on the COPY binding, got: {err}"
    );
    assert!(
        err.contains("'gz' is unfrozen in this scope (mutable window)"),
        "expected the mutable-window verdict, got: {err}"
    );
}

/// The ALIAS closure — the second seam of A6c. `pool<int> q = gfr`
/// (or a parameter alias `take(gfr)`) would rebind a global pool
/// under a local name whose mutation the checker cannot mode-check:
/// the mode table is keyed by the declared name, and a local binding
/// accepts unguarded mutation. Pool objects are not copyable values —
/// all three alias forms are rejected (declaration, assignment,
/// parameter).
#[test]
fn gpool_alias_rejected_e0803() {
    let err = compile_err("examples/gpool_alias_fail.sn");
    assert!(err.contains("[E0803]"), "expected E0803, got: {err}");
    assert!(
        err.contains("cannot alias pool 'gall' into 'q': pool objects are not copyable values"),
        "expected the alias rejection, got: {err}"
    );
    assert!(
        err.contains("the declared mode cannot follow an alias"),
        "expected the mode-cannot-follow rationale, got: {err}"
    );
    assert!(
        err.contains("a local binding accepts unguarded mutation"),
        "expected the unguarded-mutation rationale, got: {err}"
    );
    assert!(
        err.contains("cannot pass pool 'gall' by value to 'take'"),
        "expected the parameter-alias rejection, got: {err}"
    );
    assert!(
        err.contains("Pass the pool's handles (ints)"),
        "expected the handles-only discharge, got: {err}"
    );
}

// =====================================================================
// SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): the E0801 seam
// closure — the self-relative `link<T, u16>` system (kills G1 §1 A2
// → S4). The spec-as-written NoRelLinkCopy discipline (STRATA §2.4.6)
// was silent on push/insert/match-binding, never gave relocate_node
// semantics, and claimed "100% legal bulk copy" without a sender-side
// self-containedness check. The closure: the link type + the HasSelfLink
// predicate + the four by-value-copy rejection sites with the verbatim
// diagnostics; relocate_node/relocate_range with decode/re-encode
// semantics and the constant-shift exemption; the to_bytes scan.
// =====================================================================

/// The by-value-copy rejection, assignment form — the spec's rule 2
/// (both the declaration-initialization `Node d = s` and the plain
/// `d = s`), on the HasSelfLink predicate: 'Node' contains a
/// self-relative link field, and the RHS reads stored bytes (an
/// lvalue-load). → E0801 (assign).
#[test]
fn link_assign_lvalue_load_rejected_e0801() {
    let err = compile_err("examples/link_assign_fail.sn");
    assert!(err.contains("[E0801]"), "expected E0801, got: {err}");
    assert!(
        err.contains("E0801 (assign): cannot assign 's' to 'd' by value"),
        "expected the E0801 (assign) headline on both sites, got: {err}"
    );
    assert!(
        err.contains("'Node' contains a self-relative 'link<Node, u16>'"),
        "expected the HasSelfLink verdict naming the type, got: {err}"
    );
    assert!(
        err.contains("(the copied delta would decode against 'd''s address, not the link's target)"),
        "expected the decode-against-the-wrong-address rationale, got: {err}"
    );
    assert!(
        err.contains("pass 'ref<Node>', or use 'relocate_node(src, dst)'"),
        "expected both discharges, got: {err}"
    );
}

/// The by-value-copy rejection, pass-by-value form — a HasSelfLink
/// lvalue-load into a by-value parameter copies the delta bytes to the
/// parameter's address. → E0801 (call).
#[test]
fn link_pass_by_value_rejected_e0801() {
    let err = compile_err("examples/link_passby_fail.sn");
    assert!(err.contains("[E0801]"), "expected E0801, got: {err}");
    assert!(
        err.contains("E0801 (call): cannot pass 's' by value to parameter 'n' of 'take'"),
        "expected the E0801 (call) headline naming value, parameter and function, got: {err}"
    );
    assert!(
        err.contains("'Node' contains a self-relative 'link<Node, u16>'"),
        "expected the HasSelfLink verdict, got: {err}"
    );
    assert!(
        err.contains("(the copied delta would decode against the parameter's address, not the link's target)"),
        "expected the parameter-address rationale, got: {err}"
    );
}

/// THE A2 KILL — the push form, verbatim: `dst.push(src[0])` copies a
/// HasSelfLink value by value (the pushed bytes carry deltas encoded
/// against the SOURCE field addresses). The spec's exact diagnostic:
/// E0801 (push), every clause asserted.
#[test]
fn link_push_lvalue_load_rejected_e0801_push_verbatim() {
    let err = compile_err("examples/link_push_fail.sn");
    assert!(
        err.contains(
            "E0801 (push): cannot push 'src[0]' into 'dst' by value because \
             'Node' contains a self-relative 'link<Node, u16>' (the copied \
             delta would decode against 'dst''s address, not the link's \
             target); push a freshly constructed value (built in place via \
             sret), pass 'ref<Node>', or use 'relocate_node(src, dst)'. [E0801]"
        ),
        "expected the verbatim E0801 (push) diagnostic, got: {err}"
    );
}

/// The push rejection's discharge (rule 1, unchanged): the RHS that
/// MATERIALIZES fresh (a call result — sret builds it at its final
/// address) is not a load of stored bytes, so `xs.push(make())` stays
/// legal, compiles, and runs; the pushed value carries only null links
/// (position-independent), so the copy is sound.
#[test]
fn link_push_fresh_constructor_ok() {
    assert_out("examples/link_push_fresh_ok.sn", "1\n7\n");
}

/// The by-value-copy rejection, match-binding form, verbatim: binding a
/// HasSelfLink payload to a by-value pattern variable copies the delta
/// bytes into the binding's address. E0801 (match), every clause
/// asserted.
#[test]
fn link_match_binding_rejected_e0801_match_verbatim() {
    let err = compile_err("examples/link_match_fail.sn");
    assert!(
        err.contains(
            "E0801 (match): cannot bind 'n' by value from the match arm on \
             'Tree': 'Tree' contains a self-relative 'link<Tree, u16>'; bind \
             'ref<Tree>' in the pattern, or 'relocate_node' into a fresh \
             destination. [E0801]"
        ),
        "expected the verbatim E0801 (match) diagnostic, got: {err}"
    );
}

/// relocate_node's decode/re-encode semantics (spec (c)), end-to-end: an
/// out-link keeps its absolute target (decode at the destination ==
/// addr(a[1])), an in-link is fixed to the new interior (decode ==
/// addr(b[0])), and the source is untouched — no delta is ever
/// interpreted against an address it was not encoded for.
#[test]
fn link_relocate_node_in_out_links_ok() {
    assert_out("examples/link_relocate_ok.sn", "true\ntrue\ntrue\ntrue\n");
}

/// The i16-range rejection: the re-encoded delta for the out-link's
/// target exceeds [−32767, +32767] — the 2-byte link type cannot
/// represent the moved geometry, and the relocation fail-fasts with the
/// verbatim E0801 (relocate) diagnostic instead of silently truncating
/// the delta (runtime check: the bound depends on the actual addresses).
#[test]
fn link_relocate_i16_range_fail_verbatim() {
    let (code, stdout, stderr) = compile_run("examples/link_relocate_overflow_fail.sn");
    assert_ne!(code, 0, "expected the relocation to fail fast, got: {stdout}");
    assert!(
        stderr.contains("E0801 (relocate): cannot relocate 'next' from 'a[0]' to 'b[0]'"),
        "expected the verbatim E0801 (relocate) headline naming field, source and destination, got: {stderr}"
    );
    assert!(
        stderr.contains("the re-encoded delta ("),
        "expected the re-encoded delta value in the diagnostic, got: {stderr}"
    );
    assert!(
        stderr.contains("exceeds the i16 range [−32767, +32767]"),
        "expected the exact i16 range with the unicode minus, got: {stderr}"
    );
    assert!(
        stderr.contains("move the value and its target closer together, or use 'link<Node in pool, u32>'. [E0801]"),
        "expected both discharges, got: {stderr}"
    );
}

/// relocate_range — the constant-shift exemption: a self-contained
/// block (every link intra-range) bulk-copies with every intra-range
/// delta provably unchanged (target and field both shift by k), and
/// all destination links decode to the correct destination neighbors.
#[test]
fn link_relocate_range_self_contained_ok() {
    assert_out("examples/link_relocate_range_ok.sn", "4\n");
}

/// The to_bytes sender-side scan, REJECT side, verbatim: serializing a
/// sub-slice whose link targets lie OUTSIDE the slice (cross-slice
/// links — legal to hold, illegal to serialize) fail-fasts at pack time
/// with E0801 (serialize) — the guaranteed load-time failure never
/// happens.
#[test]
fn link_to_bytes_cross_slice_fail_verbatim() {
    let (code, stdout, stderr) = compile_run("examples/link_to_bytes_fail.sn");
    assert_ne!(code, 0, "expected the serialization to fail fast, got: {stdout}");
    assert!(
        stderr.contains(
            "E0801 (serialize): cannot serialize 'xs[0..2]' to bytes: \
             'link<Node, u16>' at element 1 ('next') decodes to an address \
             outside the serialized slice (cross-slice links cannot survive \
             relocation). Serialize the enclosing whole list, drop the link, \
             or use 'link<Node in pool, u32>'. [E0801]"
        ),
        "expected the verbatim E0801 (serialize) diagnostic, got: {stderr}"
    );
}

/// The to_bytes scan, ACCEPT side: the whole-list serialization of a
/// self-contained block passes the scan (every target inside the
/// serialized slice), packs the slice bytes, and the round trip is
/// sound — the bulk-copy claim true by precondition instead of false by
/// omission. Buffer readback verifies the packed bytes.
#[test]
fn link_to_bytes_whole_list_ok() {
    assert_out("examples/link_to_bytes_ok.sn", "true\n4242\n");
}

// ---------------------------------------------------------------------
// SWARM RABBIT HOLE — Wave 13 (Impl-R, Cohort Delta): the wave region
// — Theorem WS made executable (W2B §4, the composed confluence
// theorem; the typed read-mode surface W2B §3). The wave body is a
// pure function of the frozen capture κ (Lemma WD — the E0812 purity
// gate is the kill switch), contributions stage as payloads and fold
// canonically at the J(W) join (Lemma NI(ii): contributions ARE
// atomic ⊔-merges on seam cells; Thm WS Part 2 — the J2 batched fold),
// and read_J after the join observes the folded value. R1 (quiescent
// capture) is the checked side condition that upgrades Part 2's
// confluence to Part 3's composed determinism.
// ---------------------------------------------------------------------

/// Thm WS Part 2 end-to-end: a pure wave body stages contributions
/// to GSET and MAX seam cells; at J(W) the staged multiset folds
/// canonically (one atomic ⊔ per cell); read_J observes
/// κ₀(c) ⊔ ⊔𝒰_W(c). The MAX carrier's losing offer is a no-op (⊔ =
/// max), the GSET support grows by exactly the staged elements.
#[test]
fn wave_fold_read_j() {
    assert_out(
        "examples/wave_fold.sn",
        "20\n3\ntrue\nPASS: wave_fold\n",
    );
}

/// The read-only capture: 𝒰_W(c) = ∅ ⇒ J(W) is the identity on the
/// cell (the fold of the empty multiset is ⊥), and read_J after the
/// wave equals the frozen κ readout. Wave observation leaves the
/// seam untouched.
#[test]
fn wave_readonly_capture() {
    assert_out(
        "examples/wave_readonly.sn",
        "5\n5\n5\nPASS: wave_readonly\n",
    );
}

/// The S5 dividend at the seam (Part 2(iii)): a duplicate
/// contribution folds once — ⊔-idempotence executes at the join, so
/// at-least-once delivery costs nothing (W2B §1 D6's division of
/// labor: inflations do not absorb duplicates, ⊔-idempotence does).
#[test]
fn wave_contrib_duplicate_folds_once() {
    assert_out(
        "examples/wave_contrib_idem.sn",
        "1\ntrue\nPASS: wave_contrib_idem\n",
    );
}

/// The two read modes and the sequential-composition case (§4.6 case
/// (c)): read_W inside the body is the FROZEN κ (contribs stage, they
/// never merge mid-body — the J1 exit join, not the J0 immediate
/// merge); read_J after the block is the folded value; the second
/// wave's capture is quiescent exactly because J(W₁) provably
/// completed (the static rule's accepted form), and its frozen κ is
/// wave 1's fold — the induction step of Part 3.
#[test]
fn wave_read_w_frozen_vs_read_j_folded() {
    assert_out(
        "examples/wave_read_w_vs_j.sn",
        "100\n100\n200\n200\n200\nPASS: wave_read_w_vs_j\n",
    );
}

/// E0812 — the kill switch, verbatim (§4.6 case (d)): a wall-clock
/// read in a wave body injects the schedule into 𝒰_W and destroys
/// the wave-determinism kernel (Lemma WD's load-bearing premise).
/// The gate is a compile-time error, never a silent downgrade.
#[test]
fn wave_e0812_time_rejected_verbatim() {
    let err = compile_err("examples/wave_e0812_time_fail.sn");
    assert!(
        err.contains(
            "E0812: wave body reads schedule-dependent state 'time_now_ms' — \
             wave bodies must be pure functions of the frozen capture \
             (Lemma WD's premise); this is the kill switch: the determinism \
             grade degrades without it [E0812]"
        ),
        "expected the verbatim E0812 kill-switch diagnostic, got: {err}"
    );
}

/// E0812's remaining rejection set, accumulated in one file: the env
/// read, the channel receive (the recv injects the delivery schedule
/// into the body), spawn inside the body (the wave IS the
/// parallelism), the nested wave (two waves on one cell with no
/// quiescence proof — case (b) by construction), and the early
/// return (the exit edge contrib→exit→J(W) is structural; termination
/// is the E0810·R runtime contract).
#[test]
fn wave_e0812_env_chan_spawn_nested_early_rejected() {
    let err = compile_err("examples/wave_e0812_env_chan_fail.sn");
    assert!(
        err.contains("E0812: wave body reads schedule-dependent state 'os_getenv'"),
        "expected the env-read E0812 arm, got: {err}"
    );
    assert!(
        err.contains("E0812: wave body reads schedule-dependent state 'receive'"),
        "expected the chan-recv E0812 arm, got: {err}"
    );
    assert!(
        err.contains("E0812: wave body spawns — the wave IS the parallelism"),
        "expected the spawn E0812 arm, got: {err}"
    );
    assert!(
        err.contains("E0812: a wave body may not nest a wave"),
        "expected the nested-wave E0812 arm, got: {err}"
    );
    assert!(
        err.contains("E0812: a wave body may not return early"),
        "expected the early-return E0812 arm, got: {err}"
    );
}

/// The in-wave mode rules (W2B §3 — the mode is in the type): the
/// fallback read (read_S — lcell_value on the captured cell) and the
/// fallback sync ops (lcell_merge — the J0 immediate merge;
/// lcell_inflate — the S1 application) are refused inside a wave
/// body: the seam's in-wave surface is read_W/contrib only.
#[test]
fn wave_e0812_read_s_and_syncops_rejected() {
    let err = compile_err("examples/wave_e0812_reads_fail.sn");
    assert!(
        err.contains(
            "E0812: wave body reads seam cell 'c' in fallback mode (read_S)"
        ),
        "expected the read_S mode E0812 arm, got: {err}"
    );
    assert!(
        err.contains(
            "E0812: wave body performs the fallback sync op 'lcell_merge'"
        ),
        "expected the J0 sync-op E0812 arm, got: {err}"
    );
}

/// Rule R1 — the non-quiescent capture, verbatim (§4.3): a fallback
/// merge into a seam cell that is neither program-order-before the
/// wave nor provably after J(W) (lexical order cannot prove the
/// after-arm when the capture is loop-shaped — the merge can sit
/// between two J(W)s) is a checked compile error: mixed use without
/// proof ⇒ conditional determinism, and the checker refuses it.
#[test]
fn wave_er1_merge_after_wave_rejected_verbatim() {
    let err = compile_err("examples/wave_er1_merge_after_fail.sn");
    assert!(
        err.contains(
            "E-R1: capture of 'c' is not R1-quiescent: a fallback merge \
             into 'c' is not HB-before this wave — determinism would be \
             conditional; make the merge program-order-before the wave, \
             or use contrib-only [E-R1]"
        ),
        "expected the verbatim E-R1 non-quiescent-capture diagnostic, got: {err}"
    );
}

/// Rule R1's two-waves arm, verbatim (§4.6 case (b) — a theorem, not
/// a defect): waves in different functions share the seam cell with
/// no provable J(W₁)→capture(W₂) edge, so neither capture can be
/// certified quiescent w.r.t. the other and the checker refuses
/// both, symmetrically. Sequential composition inside ONE function
/// is the accepted form (case (c)).
#[test]
fn wave_er1_two_waves_share_cell_rejected_verbatim() {
    let err = compile_err("examples/wave_er1_two_waves_fail.sn");
    assert!(
        err.contains(
            "E-R1: two waves share seam cell 'shared' — neither capture \
             can be R1-quiescent w.r.t. the other; compose them \
             sequentially [E-R1]"
        ),
        "expected the verbatim E-R1 two-waves diagnostic, got: {err}"
    );
}

/// The wave-body operations outside a wave (W2B §3's scope rule):
/// contrib stages into the ENCLOSING wave's J(W) and read_W reads the
/// ENCLOSING wave's frozen κ — with no enclosing wave, both are
/// checked refusals pointing at the fallback tier's own ops (the S1
/// path; read_S).
#[test]
fn wave_er1_contrib_read_w_outside_rejected() {
    let err = compile_err("examples/wave_er1_contrib_outside_fail.sn");
    assert!(
        err.contains(
            "E-R1: contrib stages a payload into the enclosing wave's join (J(W))"
        ),
        "expected the contrib-outside E-R1 arm, got: {err}"
    );
    assert!(
        err.contains(
            "E-R1: read_W reads the frozen capture of an enclosing wave"
        ),
        "expected the read_W-outside E-R1 arm, got: {err}"
    );
}

/// read_J's ordering discipline (W2B §3: "ordered after J(W)"): the
/// fold-valued claim is temporal — before the wave the cell holds κ₀
/// and the honest read mode is read_S; mislabeling it read_J is a
/// checked refusal.
#[test]
fn wave_er1_read_j_before_join_rejected() {
    let err = compile_err("examples/wave_er1_read_j_fail.sn");
    assert!(
        err.contains(
            "E-R1: read_J requires J(W) to have happened — 'c' has no wave \
             joined at this point"
        ),
        "expected the read_J-before-J(W) E-R1 arm, got: {err}"
    );
}

/// The seam-cell qualification rule (W2B §4.1): seam cells are
/// module-scope Tier-L cells (the R1/E0811 analysis and the J(W) fold
/// are defined over the module-scope seam); a function-local cell is
/// wave-internal by construction and read_W/contrib on it is a named
/// checked refusal, not an unknown-name error.
#[test]
fn wave_er1_not_a_seam_cell_rejected() {
    let err = compile_err("examples/wave_er1_not_seam_fail.sn");
    assert!(
        err.matches("E-R1: 'local' is not a seam cell").count() >= 2,
        "expected the not-a-seam E-R1 arm on both read_W and contrib, got: {err}"
    );
    assert!(
        err.contains(
            "a seam cell is a module-scope 'ptr' declared with an lcell_new initializer"
        ),
        "expected the seam-cell qualification rule in the diagnostic, got: {err}"
    );
}

// ---------------------------------------------------------------------
// SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the SFC layouts
// — A2's space-filling-geometry decision table made executable (A2
// §2.4: morton list = tensor iteration with per-element indexed
// access; hilbert map = spatial range queries, the key computed once
// and amortized over the box; soa = the lexicographic axis-committed
// field projection, already landed). The interleave/deinterleave is
// FEATURE-DISPATCHED once per process (C2.8: PDEP/PEXT on x86-64
// with BMI2, the software shift-or path elsewhere); the T5b warning
// binds iteration order to storage order (A2 §3).
// ---------------------------------------------------------------------

/// Compile with --emit-llvm and REQUIRE success, returning the
/// compile's stderr — the W-* warnings land there (warnings are
/// never errors; the build proceeds).
fn compile_ok_stderr(rel: &str) -> String {
    let root = repo_root();
    let src = root.join(rel);
    let compile = Command::new(snc_bin())
        .arg(&src)
        .arg("--emit-llvm")
        .current_dir(&root)
        .output()
        .expect("run snc");
    assert!(
        compile.status.success(),
        "{rel} must compile — warnings are not errors"
    );
    String::from_utf8_lossy(&compile.stderr).into()
}

/// The morton index arithmetic round trip: interleave↔deinterleave is
/// a bijection over the grid (every pair encodes to a distinct code,
/// every code decodes to an in-grid pair — the O(1)-carry iteration
/// visits every index exactly once; the code space covers a
/// power-of-two grid exactly).
#[test]
fn morton_index_roundtrip() {
    assert_out(
        "examples/morton_roundtrip.sn",
        "0\n1\n2\n3\n4\n0\n17031168\nPASS: morton_roundtrip\n",
    );
}

/// from_lex equivalence: the blocked reshuffle is a permutation —
/// every cell reads back its row-major value (the coordinate view)
/// and the curve-order scan sees the same multiset (2016 both ways).
#[test]
fn morton_from_lex_equivalence() {
    assert_out(
        "examples/morton_from_lex.sn",
        "0\n2016\n2016\n64\nPASS: morton_from_lex\n",
    );
}

/// Position independence, executed: a morton list holds values only
/// (no absolute pointers), so to_bytes/from_bytes round-trips with
/// ZERO fixups — the serialized bytes rebuild a fresh, bit-identical
/// collection (the link<T, u16> seam needs an O(N) scan for exactly
/// the self-relative bytes SFC storage does not have).
#[test]
fn morton_bytes_position_independence() {
    assert_out(
        "examples/morton_bytes.sn",
        "0\n32896\n256\nPASS: morton_bytes\n",
    );
}

/// Hilbert point queries, exact: the Butz/Skilling key computed once
/// per insert; the sorted lane's order IS the index (binary search
/// point lookups, no auxiliary hash); the entry readback exposes the
/// lane's sorted order.
#[test]
fn hilbert_point_queries_exact() {
    assert_out(
        "examples/hilbert_points.sn",
        "17\ntrue\nfalse\n9\n0\n16\ntrue\ntrue\nPASS: hilbert_points\n",
    );
}

/// The segment-advance box query == the linear-scan reference, EXACT:
/// every axis-aligned box of the 8×8 grid (1296 boxes), both sides
/// accumulating the same order-free (count, sum, xor) triple over
/// the result set — Tropf–Herzog/Lawder–King exactness (A2 §3).
#[test]
fn hilbert_box_matches_linear_scan() {
    assert_out(
        "examples/hilbert_box.sn",
        "1296\n1\nPASS: hilbert_box\n",
    );
}

/// The out-of-order insert — the honest slice-1 REFUSAL, verbatim:
/// the sorted lane IS the index and slice 1 does not silently sort;
/// the diagnostic names the rebuild-epoch future form (A2 §3's
/// P1/P3 flag). The process dies at the insert, exit 1.
#[test]
fn hilbert_insert_order_refused_verbatim() {
    let (code, out, err) = compile_run("examples/hilbert_insert_order_fail.sn");
    assert_eq!(code, 1, "the process must die at the refused insert");
    assert!(
        out.is_empty(),
        "nothing may print before the refusal, got: {out}"
    );
    assert!(
        err.contains(
            "runtime error: hilbert insert out of key order (new key 2 after \
             last 5) — the sorted lane IS the index and slice 1 does not \
             reorder; insert along the curve or await the rebuild-epoch \
             policy (P1/P3) [E-SFC-ORDER]"
        ),
        "expected the verbatim [E-SFC-ORDER] refusal, got: {err}"
    );
}

/// The morton bounds discipline, verbatim: an out-of-range coordinate
/// is a program bug (the code space covers the power-of-two grid
/// exactly — no padding cell to land in); the runtime fails fast with
/// [E-SFC], exit 1.
#[test]
fn morton_oob_refused_verbatim() {
    let (code, out, err) = compile_run("examples/morton_oob_fail.sn");
    assert_eq!(code, 1, "the process must die at the out-of-bounds read");
    assert!(
        out.is_empty(),
        "nothing may print before the refusal, got: {out}"
    );
    assert!(
        err.contains(
            "runtime error: morton index (9, 0) out of bounds for dims \
             [8, 8] — SFC addressing is bounds-checked, never wraps [E-SFC]"
        ),
        "expected the verbatim [E-SFC] bounds refusal, got: {err}"
    );
}

/// The T5b warning, verbatim, on the provable shape: a counted for
/// over i, a nested counted for over j, both indexing the morton list
/// directly (morton_get(m, i, j)) — the lexicographic pair over SFC
/// storage (A2 §3/T5(b): Θ(N) misses, a factor B/w over the stride-1
/// curve scan). A WARNING, never an error — the build succeeds.
#[test]
fn sfc_t5b_warning_fires_verbatim() {
    let warn = compile_ok_stderr("examples/sfc_t5b_warn.sn");
    assert!(
        warn.contains(
            "W-SFC-T5B: lexicographic iteration ('i' then 'j') over morton \
             list 'm' wastes the space-filling layout — iterate the curve \
             order (morton_scan_get over the code) or use soa for \
             axis-committed access [W-SFC-T5B]"
        ),
        "expected the verbatim W-SFC-T5B warning, got: {warn}"
    );
}

/// The T5b no-false-positive side: curve-ordered iteration (the code
/// IS the linear index; the scan strides through the buffer in
/// storage order) compiles SILENT and runs correct — the warning
/// fires on the lexicographic pair only.
#[test]
fn sfc_t5b_no_false_positive() {
    assert_out("examples/sfc_t5b_ok.sn", "2016\nPASS: sfc_t5b_ok\n");
    let warn = compile_ok_stderr("examples/sfc_t5b_ok.sn");
    assert!(
        !warn.contains("W-SFC-T5B"),
        "curve-ordered iteration must not warn, got: {warn}"
    );
}

/// The power-of-two discipline, verbatim (A2 §3): slice 1 maps the
/// SQUARE power-of-two grid onto the code space exactly (no padding,
/// every code an in-grid cell); unequal dims want the padded
/// reservation / the hybrid 2^t tile floor — the slice-2 form.
#[test]
fn morton_dims_rejected_verbatim() {
    let err = compile_err("examples/morton_dims_fail.sn");
    assert!(
        err.contains(
            "E-SFC: morton dims must be equal powers of two (slice 1 maps the \
             SQUARE grid onto the code space exactly — no padding; unequal dims \
             want the padded reservation / the hybrid 2^t tile floor, the \
             slice-2 form), got dims=[8, 12] [E-SFC]"
        ),
        "expected the verbatim E-SFC power-of-two diagnostic, got: {err}"
    );
}

/// The dims-are-in-the-type rule, verbatim: the constructor's literal
/// dims must agree with the declared morton list type — the type
/// annotation must not lie about the bounds the runtime checks.
#[test]
fn morton_decl_mismatch_rejected_verbatim() {
    let err = compile_err("examples/morton_mismatch_fail.sn");
    assert!(
        err.contains(
            "E-SFC: morton_new's dims [16, 16] do not match the declared \
             morton list type's dims=[8, 8] — the dims are in the type; the \
             constructor must agree [E-SFC]"
        ),
        "expected the verbatim E-SFC dims-mismatch diagnostic, got: {err}"
    );
}

/// The E-GC-IN-LANE store rule at the SFC boundary, verbatim: a
/// lane-backed SFC collection cannot enter a GC-managed holder — the
/// collector cannot trace lane memory and the holder's release would
/// free lane bytes as heap.
#[test]
fn morton_store_into_gc_holder_rejected_verbatim() {
    let err = compile_err("examples/morton_push_fail.sn");
    assert!(
        err.contains(
            "E-GC-IN-LANE: cannot store 'm' into 'xs' — a lane-backed SFC \
             collection cannot enter a GC-managed holder (the collector \
             cannot trace lane memory and the holder's release would free \
             it); SFC collections live in their region's lane and die at \
             its rewind [E-GC-IN-LANE]"
        ),
        "expected the verbatim E-GC-IN-LANE SFC-store diagnostic, got: {err}"
    );
}

/// The fixed-size element discipline, verbatim (A2 §3: "no directory
/// for fixed-size T"): slice 1 implements morton list<int, ...> — the
/// Morton-keyed directory lane for variable-size elements is the
/// documented slice-2 form.
#[test]
fn morton_elem_type_rejected_verbatim() {
    let err = compile_err("examples/morton_elem_fail.sn");
    assert!(
        err.contains(
            "E-SFC: slice 1 implements morton list<int, dims=[n1, n2]> — \
             fixed-size int elements only (no directory for fixed-size T; \
             the Morton-keyed directory lane is the slice-2 form), got \
             element type str [E-SFC]"
        ),
        "expected the verbatim E-SFC element-type diagnostic, got: {err}"
    );
}

/// The hilbert map's slice-1 element discipline, verbatim: the d=2
/// point key is the (x, y) int pair, the value is int; other
/// key/value types are the slice-2 boundary.
#[test]
fn hilbert_kv_rejected_verbatim() {
    let err = compile_err("examples/hilbert_kv_fail.sn");
    assert!(
        err.contains(
            "E-SFC: slice 1 implements hilbert map<int, int> — the d=2 \
             point key is the (x, y) int pair, the value is int; other \
             key/value types are the slice-2 boundary, got hilbert \
             map<str, int> [E-SFC]"
        ),
        "expected the verbatim E-SFC hilbert element-type diagnostic, got: {err}"
    );
}

// ---------------------------------------------------------------------
// SWARM RABBIT HOLE — Wave 15 (Impl-T, Cohort Delta): Salt ε — the
// numeric contract layer (requires/ensures clauses on fn
// declarations; A4's Presburger decidable ladder + the final report
// §7.3's boundary rule, executable). The engine is the facts table
// (Go-prove-style: sound, not complete) over the DBM zone layer
// (Floyd CACM 1962; Dill CAV'89; Bengtsson–Yi; Miné HOSC 2006); the
// tiers are L (linear), D (divisibility ledge: the remainder range
// axiom only — the EPAD/Lipshitz solver is a later rung), N
// (nonlinear: E0701 in a verified fn, a typed runtime assertion in a
// plain fn). An atom the engine proves is discharged at compile
// time; an atom it cannot prove degrades to a typed runtime
// assertion at the boundary (the call site / fn entry for requires,
// the fn exit for ensures) — never silently dropped, priced per
// atom.
// ---------------------------------------------------------------------

/// The DISCHARGE tier, end to end: every atom proved at compile
/// time, the emitted code carries NO contract assertion, and the
/// compile is warning-free (the discharge's whole point — the
/// loop-index-vs-len classic with a literal caller, the call-chain
/// propagation of the caller's own requires, the ensures discharge
/// over the return-value binding, the mod range axiom (Tier D), and
/// the mod-by-literal fold).
#[test]
fn contract_discharge() {
    let (code, stdout, stderr) = compile_run("examples/contract_discharge.sn");
    assert_eq!(code, 0, "contract_discharge failed ({code}): {stderr}\n{stdout}");
    assert_eq!(stdout, "10\n30\n40\n50\n3\n1\n");
    let w = compile_ok_stderr("examples/contract_discharge.sn");
    assert!(
        !w.contains("W-CONTRACT"),
        "a discharged contract must be silent, got warnings: {w}"
    );
}

/// The DEGRADE tier, warning verbatim (the boundary rule's honest
/// degradation, priced per atom): the mod-term caller fact
/// discharges 'i < xs.length()' (the axiom gives i ≤ 2, the literal
/// list's length is 3 — the DBM closure derives i − len ≤ −1) but
/// cannot discharge '0 <= i' (the axiom gives i ≥ −2, not i ≥ 0);
/// the Tier D mod-by-variable atom '0 <= x % m' degrades under the
/// remainder range axiom. Both runtime assertions pass on the valid
/// input.
#[test]
fn contract_degrade_warn_verbatim() {
    let (code, stdout, stderr) = compile_run("examples/contract_degrade.sn");
    assert_eq!(code, 0, "contract_degrade failed ({code}): {stderr}\n{stdout}");
    assert_eq!(stdout, "ok\n");
    let w = compile_ok_stderr("examples/contract_degrade.sn");
    assert!(
        w.contains(
            "W-CONTRACT: requires atom '0 <= i' of 'get' was not \
             discharged at the call in 'main' (tier L: linear) — a typed \
             runtime assertion now sits at the call site; the facts table + \
             DBM zones are sound, not complete. [W-CONTRACT]"
        ),
        "expected the verbatim W-CONTRACT tier-L degrade warning, got: {w}"
    );
    assert!(
        w.contains(
            "W-CONTRACT: requires atom '0 <= x % m' of 'rmod' was not \
             discharged at the call in 'main' (tier D: divisibility ledge — \
             the remainder-range axiom is a range fact, never a divisibility \
             solve; the EPAD solver tier is a later rung) — a typed runtime \
             assertion now sits at the call site; the facts table + DBM zones \
             are sound, not complete. [W-CONTRACT]"
        ),
        "expected the verbatim W-CONTRACT tier-D degrade warning, got: {w}"
    );
}

/// The REFUTED arm, verbatim: the literal 99 against the 3-element
/// list — the engine proves the negation (the conjoined atom closes
/// to a negative diagonal cycle), the warning says the assertion will
/// always fail, and the runtime assertion still fires: the process
/// dies with the [E-0702] fail-fast message (soundness never rests on
/// the refutation alone — the caller's facts are enforced elsewhere).
#[test]
fn contract_degrade_refuted_fail_verbatim() {
    let w = compile_ok_stderr("examples/contract_degrade_fail.sn");
    assert!(
        w.contains(
            "W-CONTRACT: requires atom 'i < xs.length()' of 'get' is \
             REFUTED at the call in 'main' — the engine proved the negation \
             from the caller's facts; the runtime assertion will always \
             fail here. [W-CONTRACT]"
        ),
        "expected the verbatim W-CONTRACT refutation warning, got: {w}"
    );
    let (code, _stdout, stderr) = compile_run("examples/contract_degrade_fail.sn");
    assert_ne!(code, 0, "the refuted call must die");
    assert!(
        stderr.contains(
            "runtime error: contract requires 'i < xs.length()' of 'get' \
             violated at the call in 'main' (a typed runtime assertion — \
             the compile-time engine proved the negation) [E-0702]"
        ),
        "expected the verbatim [E-0702] fail-fast message, got: {stderr}"
    );
}

/// The ensures DEGRADE at the fn exit, verbatim: the coefficient-2
/// return binding (result := 2x) is not a difference bound (the
/// octagon rung is a later rung), so 'result <= 100' has no exit-zone
/// bound; the assertion sits at every return of 'double' and fires on
/// the 120.
#[test]
fn contract_ensures_degrade_fail_verbatim() {
    let w = compile_ok_stderr("examples/contract_ensures_degrade_fail.sn");
    assert!(
        w.contains(
            "W-CONTRACT: ensures atom 'result <= 100' of 'double' was not \
             discharged at the fn exit (tier L: linear) — a typed runtime \
             assertion now sits at every return of 'double'. [W-CONTRACT]"
        ),
        "expected the verbatim W-CONTRACT ensures-degrade warning, got: {w}"
    );
    let (code, _stdout, stderr) = compile_run("examples/contract_ensures_degrade_fail.sn");
    assert_ne!(code, 0, "the violated ensures must die");
    assert!(
        stderr.contains(
            "runtime error: contract ensures 'result <= 100' of 'double' \
             violated at the exit of 'double' (a typed runtime assertion — \
             the compile-time engine could not discharge this atom) [E-0702]"
        ),
        "expected the verbatim [E-0702] ensures fail-fast message, got: {stderr}"
    );
}

/// E0701, the PRODUCT arm, verbatim (A4 §4 rule 1: the verified tier
/// is all-or-nothing — a variable×variable product inside a verified
/// fn's contract is Tier N, never dischargeable, and the diagnostic
/// must name the atom, the rule, and the bounded-degree redemption
/// clause: degrade is a total function of (atom shape × known
/// bounds), not a blanket rule).
#[test]
fn contract_e0701_product_verbatim() {
    let err = compile_err("examples/contract_e0701_fail.sn");
    assert!(
        err.contains(
            "E0701: the contract atom 'w * h > 0' in verified fn 'area' is \
             Tier N (nonlinear: a variable\u{d7}variable product) — the \
             verified tier is all-or-nothing: every atom must be \
             dischargeable by a terminating decision procedure, and a \
             product is off the decidable ladder (MRDP). Degrade is a \
             total function of (atom shape \u{d7} known bounds), not a \
             blanket rule: DBM-bounded variables redeem by enumeration and \
             univariate atoms by root bounds (A4 \u{a7}4 rule 3) — a \
             two-variable product has no bounded-degree redemption here. \
             Drop 'verified' from 'area' and the atom becomes a typed \
             runtime assertion. [E0701]"
        ),
        "expected the verbatim E0701 product diagnostic, got: {err}"
    );
}

/// E0701, the QUOTIENT-BY-VARIABLE arm, verbatim: q = x / y binds the
/// product q·y = x — division, not mod, is the undecidability vector
/// (the final report §7.2; mod-by-variable is the allowed Tier D
/// ledge, div-by-variable is not).
#[test]
fn contract_e0701_quotient_verbatim() {
    let err = compile_err("examples/contract_e0701_div_fail.sn");
    assert!(
        err.contains(
            "E0701: the contract atom 'x / y >= 0' in verified fn 'quot' is \
             Tier N (nonlinear: quotient by a variable — q = x / y binds the \
             product q\u{b7}y = x, and division, not mod, is the \
             undecidability vector) — the verified tier is all-or-nothing: \
             every atom must be dischargeable by a terminating decision \
             procedure. Degrade is a total function of (atom shape \u{d7} \
             known bounds), not a blanket rule: DBM-bounded variables \
             redeem by enumeration and univariate atoms by root bounds (A4 \
             \u{a7}4 rule 3). Drop 'verified' from 'quot' and the atom \
             becomes a typed runtime assertion. [E0701]"
        ),
        "expected the verbatim E0701 quotient diagnostic, got: {err}"
    );
}

/// The TYPED degradation of a Tier N atom in a PLAIN fn (A4 §4 rule
/// 2, the mirror of E0701): the warning names the tier (nonlinear
/// product — never dischargeable), the assertion carries the
/// classification, and the valid input runs clean.
#[test]
fn contract_nonlinear_degrade_verbatim() {
    let (code, stdout, stderr) = compile_run("examples/contract_nonlinear.sn");
    assert_eq!(code, 0, "contract_nonlinear failed ({code}): {stderr}\n{stdout}");
    assert_eq!(stdout, "ok\n");
    let w = compile_ok_stderr("examples/contract_nonlinear.sn");
    assert!(
        w.contains(
            "W-CONTRACT: requires atom 'x * y >= 0' of 'area3' was not \
             discharged at the call in 'main' (tier N: nonlinear product — \
             never dischargeable) — a typed runtime assertion now sits at \
             the call site. [W-CONTRACT]"
        ),
        "expected the verbatim typed-degradation warning, got: {w}"
    );
}

/// The typed Tier N runtime assertion FIRING, verbatim: the violating
/// input dies with the message naming the atom, the callee and the
/// typed classification.
#[test]
fn contract_nonlinear_fail_verbatim() {
    let (code, _stdout, stderr) = compile_run("examples/contract_nonlinear_fail.sn");
    assert_ne!(code, 0, "the violated nonlinear assertion must die");
    assert!(
        stderr.contains(
            "runtime error: contract requires 'x * y >= 0' of 'area3' \
             violated at the call in 'main' (a typed runtime assertion — \
             the atom is Tier N: nonlinear product, never dischargeable) \
             [E-0702]"
        ),
        "expected the verbatim [E-0702] typed-degradation message, got: {stderr}"
    );
}

// ---------------------------------------------------------------------
// SWARM RABBIT HOLE — Wave 16 (Impl-U, Cohort Delta): the TAPE —
// W2C's P5↔P2 elevation plan (§36–§38) made executable. The tape is
// the δ₀-chronicle SHAPE (discrete impulses against an integrated
// state, ordered by a clock, replay = re-integration from a
// checkpoint) with the algebra honestly TIER-PARAMETRIC: opaque
// records are the free-monoid baseline (§36 — no group, no law,
// differentiation vacuous); delta records carry the (prev → next)
// change against the landed carriers with the per-operation law
// CHECKED at the effect boundary (G: ILC's a⊕(b⊖a)=b — the final
// report §332's tier-parametric law; L: the mutator law X ⊔ m^δ(X)
// = m(X), NO ⊖ — the E-LRET precedent); checkpoint records snapshot
// the carrier state and reset the integration base. The E1 extern
// layer (`-> tape_delta(G|L)`) types the producer's results;
// unannotated externs stay opaque (P2's default-unpure form — the
// §4 kinship activates exactly when the annotation exists). The
// boundary rule: one tape, one ledger — no tape op crosses a wave
// body ([E-TAPE-WAVE]).
// ---------------------------------------------------------------------

/// The OPAQUE chronicle: append/readback on the default kind — raw
/// int payloads, no algebra (W2C §36's honest baseline, executable).
#[test]
fn tape_opaque_chronicle() {
    assert_out(
        "examples/tape_opaque.sn",
        "3\n0\n0\n11\n13\nPASS: tape_opaque\n",
    );
}

/// The Tier-G DELTA record: the (prev → next) change derived with
/// the Z-set's exact ⊖, ILC's a⊕(b⊖a)=b CHECKED at the effect
/// boundary, the impulse stored as raw (key, count) pairs (E2's
/// embeddability, literal), and the replay == re-integration
/// witness: the fold from ∅ equals the chain's endpoint exactly.
#[test]
fn tape_delta_g_law_and_replay() {
    assert_out(
        "examples/tape_delta_g.sn",
        "3\n1\n3\n1\n2\n6\n6\n1\nPASS: tape_delta_g\n",
    );
}

/// The Tier-L DELTA record: the mutator-law fragment
/// (lcell_delta — state-relative, ⊔-world, NO ⊖) with
/// X ⊔ m^δ(X) = m(X) checked at the boundary, and the ⊔-fold
/// replay witness into a fresh ⊥ cell (the final report §332's
/// v2.1 fix: L checks the mutator law and surfaces NO ⊖).
#[test]
fn tape_delta_l_mutator_law_and_replay() {
    assert_out(
        "examples/tape_delta_l.sn",
        "3\n2\n4\ntrue\ntrue\nfalse\n4\nPASS: tape_delta_l\n",
    );
}

/// The CHECKPOINT record: the snapshot resets the integration base;
/// replay folds ONLY the post-checkpoint window (the pre-checkpoint
/// record is never re-walked) and a RETRACTION crosses the
/// checkpoint — the replayed state equals the direct computation
/// exactly (W2C §36's "replay = re-integration from a checkpoint").
#[test]
fn tape_checkpoint_replay_witness() {
    assert_out(
        "examples/tape_checkpoint.sn",
        "4\n2\n0\n4\n1\n5\n5\nPASS: tape_checkpoint\n",
    );
}

/// E1 — the delta-typed extern: `-> tape_delta(G)` types the feed's
/// results as Tier-G delta-chain endpoints (the signature
/// discipline at compile time), the law is checked at the effect
/// boundary on every recorded triple, and the replay == the feed's
/// endpoint. The shim (compiler/tests/tape_shim.c) builds through
/// the runtime's own symbols, resolved at the final link — the
/// ffi_extern precedent.
#[test]
fn tape_extern_delta_e1() {
    let root = repo_root();
    let linked = PathBuf::from("/tmp/sn_tape_shim.o");
    let cc = Command::new("clang")
        .arg("-c")
        .arg("-o")
        .arg(&linked)
        .arg(root.join("compiler/tests/tape_shim.c"))
        .output()
        .expect("run clang");
    assert!(cc.status.success(), "tape shim compile failed");

    assert_out(
        "examples/tape_extern_delta.sn",
        "2\n2\n4\n8\n12\n12\nPASS: tape_extern_delta\n",
    );
}

/// The record kind is a compile-time fact — a variable kind is a
/// checked refusal (the tier classification and the arity rule fire
/// on the literal), verbatim.
#[test]
fn tape_kind_not_literal_rejected_verbatim() {
    let err = compile_err("examples/tape_kind_not_literal_fail.sn");
    assert!(
        err.contains(
            "E-TAPE: the record kind must be a literal — 0=opaque, 1=delta, \
             2=checkpoint — the kind is a compile-time fact (the tier \
             classification and the arity rule fire on it) [E-TAPE]"
        ),
        "expected the verbatim kind-literal E-TAPE diagnostic, got: {err}"
    );
}

/// The Tier-O classification — E2's honest fallback, verbatim: a
/// value with no tier-classified origin (an int here; an unannotated
/// extern's results identically) is classified O, and the delta kind
/// is refused — no delta-local law at any price.
#[test]
fn tape_delta_o_refused_verbatim() {
    let err = compile_err("examples/tape_delta_o_fail.sn");
    assert!(
        err.contains(
            "E-TAPE: a delta record needs tier-classified endpoints — 'x' is \
             classified O (no delta-local law at any price, W2C §1.1 Tier O; \
             the ⊕/⊖ laws do not exist there); record it opaque (kind 0), or \
             produce it from a tier origin: a zset_*/lcell_* constructor or a \
             tape_delta(G|L) extern (E1) [E-TAPE]"
        ),
        "expected the verbatim Tier-O E-TAPE refusal, got: {err}"
    );
}

/// The mixed-tier refusal, verbatim: the Tier-G ⊕ and the Tier-L ⊔
/// do not mix — a delta record's endpoints must share one tier
/// (E-LKIND's tier-confusion seam).
#[test]
fn tape_delta_mixed_refused_verbatim() {
    let err = compile_err("examples/tape_delta_mixed_fail.sn");
    assert!(
        err.contains(
            "E-TAPE: a delta record's endpoints must share one tier — 'z' is \
             classified Tier G and 'c' is classified Tier L; the Tier-G ⊕ and \
             the Tier-L ⊔ do not mix (E-LKIND's tier-confusion seam; W2C §1.1) \
             [E-TAPE]"
        ),
        "expected the verbatim mixed-tier E-TAPE refusal, got: {err}"
    );
}

/// The checkpoint-on-O refusal, verbatim: a checkpoint is a carrier
/// snapshot — Tier O has no carrier to snapshot.
#[test]
fn tape_checkpoint_o_refused_verbatim() {
    let err = compile_err("examples/tape_checkpoint_o_fail.sn");
    assert!(
        err.contains(
            "E-TAPE: a checkpoint is a carrier snapshot (a Tier-G Z-set or a \
             Tier-L cell) — '5' is classified O (no carrier to snapshot, W2C \
             §1.1 Tier O); checkpoint a classified state or record opaque \
             [E-TAPE]"
        ),
        "expected the verbatim checkpoint-on-O E-TAPE refusal, got: {err}"
    );
}

/// The per-kind ARITY rule, verbatim: the payload shape is part of
/// the kind's contract.
#[test]
fn tape_arity_rejected_verbatim() {
    let err = compile_err("examples/tape_arity_fail.sn");
    assert!(
        err.contains(
            "E-TAPE: tape_record(t, kind, ...) — the opaque kind takes 1 int \
             payload, the delta kind takes the (prev, next) carrier pair, the \
             checkpoint kind takes the carrier state; got 3 args [E-TAPE]"
        ),
        "expected the verbatim arity E-TAPE refusal, got: {err}"
    );
}

/// The BOUNDARY RULE, verbatim: one tape, one ledger — the P2↔wave
/// boundary rule extended to P5. A tape op cannot cross a wave body
/// (both the record and the replay arms fire in this file).
#[test]
fn tape_wave_boundary_rejected_verbatim() {
    let err = compile_err("examples/tape_wave_fail.sn");
    assert!(
        err.matches("E-TAPE-WAVE").count() >= 2,
        "expected the E-TAPE-WAVE refusal on both tape_record and tape_replay, got: {err}"
    );
    assert!(
        err.contains(
            "E-TAPE-WAVE: a tape op cannot cross a wave body — one tape, one \
             ledger: the P2↔wave boundary rule (E0812's bracket, NI(ii) 'ports \
             don't escape') extended to P5 (W2C §4); the wave stages its own \
             effects (contrib → J(W)) and the tape's chronicle must stay one \
             linear ledger of the fallback state; record and read \
             program-order-outside the wave [E-TAPE-WAVE]"
        ),
        "expected the verbatim E-TAPE-WAVE boundary-rule diagnostic, got: {err}"
    );
}

/// The E-LRET-ADJACENT refusal, verbatim: the tape's Tier-L path
/// records mutator-law FRAGMENTS (⊔-world) and adds NO ⊖ to the L
/// surface — lcell_retract stays always-rejected (the Grothendieck
/// argument).
#[test]
fn tape_lcell_retract_still_refused_verbatim() {
    let err = compile_err("examples/tape_retract_l_fail.sn");
    assert!(
        err.contains(
            "exact retraction on a Tier-L join-semilattice is impossible \
             (the merge operator is non-embeddable: x⊔x = x ⇒ [x] = 0 — \
             Grothendieck triviality; retraction on Tier L is tombstoned \
             or none, never exact); use a Tier-G Z-set (zset_*) for exact \
             retraction [E-LRET]"
        ),
        "expected the verbatim E-LRET refusal on the Tier-L tape path, got: {err}"
    );
}

/// The element discipline, verbatim: slice 1 implements tape<int>
/// (the payloads are int64 pairs throughout).
#[test]
fn tape_elem_refused_verbatim() {
    let err = compile_err("examples/tape_elem_fail.sn");
    assert!(
        err.contains(
            "E-TAPE: slice 1 implements tape<int> — the record payloads are \
             int64 pairs (the ℤ[X] impulses and the ⊔ offers), got element \
             type str [E-TAPE]"
        ),
        "expected the verbatim element E-TAPE refusal, got: {err}"
    );
}

/// The scope rule, verbatim: slice-1 tapes are function-local; the
/// module-scope chronicle is the named slice-2 form (the DIFF-4 slot
/// pattern), refused here rather than silently lowered to a dead
/// slot.
#[test]
fn tape_module_scope_refused_verbatim() {
    let err = compile_err("examples/tape_module_scope_fail.sn");
    assert!(
        err.contains(
            "E-TAPE: slice-1 tapes are function-local chronicles — a \
             module-scope tape wants the DIFF-4 slot pattern (the seam cells' \
             @sn_g_ construction at main entry), the named slice-2 form; \
             declare the tape inside a function [E-TAPE]"
        ),
        "expected the verbatim module-scope E-TAPE refusal, got: {err}"
    );
}

/// The E1 signature discipline, verbatim: tape_delta types annotate
/// extern RESULTS; a delta-typed parameter is not the slice-1
/// surface (the E3 chain-rule boundary is named, not claimed).
#[test]
fn tape_extern_param_refused_verbatim() {
    let err = compile_err("examples/tape_extern_param_fail.sn");
    assert!(
        err.contains(
            "E-TAPE: tape_delta types annotate extern RESULTS (E1: the delta \
             type of what the extern produces) — 'feed' declares a \
             delta-typed parameter, which the slice-1 surface does not \
             define; the E3 chain-rule boundary is named, not claimed [E-TAPE]"
        ),
        "expected the verbatim E-TAPE extern-parameter refusal, got: {err}"
    );
}
