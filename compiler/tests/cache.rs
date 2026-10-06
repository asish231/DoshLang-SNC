//! End-to-end test for the incremental build cache: compiling the same
//! program twice must reuse the cached binary on the second build.

use snc::driver::{compile, CompileOptions};
use std::process::Command;

#[test]
fn incremental_cache_reuses_identical_builds() {
    let dir = std::env::temp_dir().join(format!("snc-cache-e2e-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("SNC_CACHE_DIR", dir.join("cache"));
    let src = dir.join("hello_cache.sn");
    std::fs::write(&src, "fn main() {\n    print(\"cached hello\")\n}\n").unwrap();
    let opts = |out: &str| CompileOptions {
        input: src.clone(),
        output: Some(dir.join(out)),
        emit_llvm: false,
        target: None,
        clang: "clang".into(),
        opt: "2".into(),
        libs: Vec::new(),
        coverage: false,
    };

    let first = compile(&opts("a.bin")).expect("cold build");
    assert!(!first.cached, "cold cache must do a real build");
    let second = compile(&opts("b.bin")).expect("warm build");
    assert!(second.cached, "identical rebuild must hit the cache");
    assert_eq!(first.llvm_ir, second.llvm_ir);

    for res in [&first, &second] {
        let out = Command::new(res.binary.as_ref().unwrap())
            .output()
            .expect("run cached binary");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), "cached hello\n");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
