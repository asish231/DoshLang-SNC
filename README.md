# SNlang (`snc`)

<p align="center">
  <img src="logo.png" alt="SNlang Logo" width="160"/>
</p>


SNlang is a small typed language with plain syntax (`fn`, braces, `and` / `or` / `not`)
that compiles to **native code** through **LLVM IR** + `clang`. The compiler is written
in Rust.

**License:** [Apache-2.0](LICENSE) — Copyright (c) 2026 Asish Sharma / SafarNow.

## Install

Needs [Rust](https://rustup.rs/) (`cargo` on `PATH`) and `clang`.

```sh
# Ensure cargo is on PATH (rustup default):
export PATH="$HOME/.cargo/bin:$PATH"

make          # builds ./snc
./snc examples/hello_world.sn -o hello
./hello
```

Windows: `./build.ps1` then `./snc.exe …`.

```sh
make test     # cargo test + hello smoke run
```

## Download (prebuilt binaries)

Grab `snlang-<os>-<arch>.tar.gz` / `.zip` from
[GitHub Releases](../../releases) — macOS (arm64 + x64), Linux (x64),
Windows (x64). Each archive ships `snc`, `stdlib/`, sample `packages/`,
`runtime.c`, and a hello world:

```sh
./snc --version   # snc 0.2.0
./snc hello_world.sn -o hello && ./hello
```

You still need **clang 15+ on `PATH`** (Xcode command-line tools,
`apt install clang-15`, or LLVM for Windows) because `snc` links through it
— stock Ubuntu 22.04 clang 14 cannot parse the IR. Keep the
extracted folder together — or add it to `PATH` — and compile from anywhere.

## Quickstart

```sn
fn main() {
    str name = "Ada"
    print("hello {name}")
    list<int> xs = [1, 2, 3]
    for (x in xs) {
        print(x)
    }
}
```

```sh
./snc examples/hello_world.sn -o hello && ./hello
./snc examples/use_std_math.sn -o math && ./math
./snc examples/std_test.sn -o t && ./t
```

More: [docs/LANGUAGE.md](docs/LANGUAGE.md) · full reference [SNLANG_SPEC.md](SNLANG_SPEC.md)

## Language tour (implemented)

| Area | Highlights |
|---|---|
| Types | `int`, `bool`, `str`, `list<T>`, `map<K,V>`, `T?`, `error`, `chan`, `ref`, `mut`, `lock`, `json`, `any`, `record`, fn types |
| Safety | Compile-time Rust-grade guarantees: stack escape checks, collection borrow locks, affine UAF/double-free prevention, thread-race checks; **self-relative link system** (`link<T, u16>` — a 2-byte δ = target − field_address delta field; the **E0801 copy-discipline closure**: every by-value copy of a HasSelfLink value that READS stored bytes is rejected at four sites — assignment, pass-by-value, push/insert, match-binding (fresh constructions via sret stay legal); `relocate_node`/`relocate_range` re-encode every delta against the new field address (in-links fixed to the moved interior, out-links re-based to their absolute targets; the constant-shift bulk exemption makes self-contained ranges memcpy-sound without per-link re-encode); `to_bytes` runs the O(N) sender-side self-containedness scan — cross-slice links are legal to hold but illegal to serialize, making the bulk-copy claim true by precondition instead of false by omission; the i16 range [−32767, +32767] is checked at every re-encode); **global-pool discipline** (three declared modes, frozen by default — `pool<T>` bare = **Frozen** (read-only everywhere outside an `unfrozen (P) { ... }` window, handles Sendable — zero-copy shared reads), `pool<T> thread_local` = **ThreadConfined** (mutate on the declaring thread only; capture = E0803.1), `pool<T> synchronized` = **Synchronized** (mutex-protected vault, handles Sendable everywhere); E0803.2 rejects spawn/chan capture of a Frozen pool's handles *inside* the window — the receiver could race the freelist; E0803.3 rejects mutation outside it; the runtime carries `frozen_depth`, the vault lock, and the home-thread tag as the enforcement backstop); **region poset** (RgnId provenance, `scc_accum_params` fixpoint — E0802 fires on recursive inout-accumulator hazards); **pinned-lane coloring** (L2–L6 accumulator assignment — E0802c on budget exhaustion, never silent reuse); **escape checker** (E-ESCAPE: refs stored into a holder must have the target outlive the holder); **GC scoping** (E-GC-IN-LANE: GC-managed and lane-backed memory cannot reference each other — the GC can't trace into lanes and lane rewinds don't respect the GC heap); **tiered differential calculus** (Tier-G Z-sets with exact retraction, **Tier-L semilattice cells** with carriers verified join-semilattices — NO ⊖ on the L surface, E-LRET rejects exact retraction with the Grothendieck argument at compile time, E-LKIND fail-fasts tier/carrier confusion at runtime; capability order O ⊑ L ⊑ G); **Salt ζ — the fixpoint admission checker** (the convergence theorem's sufficient condition made executable: monotone body + finite-height per-key carrier ⇒ the δ₀/∫ bracket terminates in ≤ height rounds at the least fixpoint, Knaster–Tarski; the two named divergence classes are recognized intrinsics ALWAYS rejected at compile time with E-ZADM — counting-through-recursion: infinite-height carrier; negation-through-recursion: μy. 1⊖y oscillates, non-monotone body — refusals are compile-time errors, never silent divergence); **wave regions — Theorem WS executable** (`wave { ... }` — the wave-seam composition theorem's deterministic observation bracket over module-scope Tier-L **seam cells** (`ptr s = lcell_new(k)`): the body is checked pure — **E0812**, the kill switch, rejects wall-clock/env/chan-recv/spawn/fallback-sync-op reads (interprocedurally, via the call-graph purity fixpoint) because a schedule read would inject non-determinism into the contribution multiset and destroy Lemma WD; **R1 quiescent capture** (E-R1) rejects fallback merges into a claimed seam cell unless program-order-before the wave — lexical order cannot prove the "after J(W)" arm when a capture is loop-shaped — and rejects unordered second waves on the same cell (W2B §4.6 case (b): a theorem, not a defect); the seam typing is `read_W` (the frozen capture κ) / `contrib(c,a,b)` (payloads staged into the J(W) join) / `read_J` (fold-valued, checked ordered-after J(W)) with the fallback `read_S` mode refused in-wave; the block exit is **J(W) — the J2 batched canonical fold**: the staged multiset folds through ONE atomic ⊔ per cell (the carrier's checked lattice laws are the fold's correctness — order/grouping/duplication-independent, the S5 retry dividend), so contributions ARE atomic ⊔-merges on seam cells, Lemma NI(ii)); **Elastic Lanes** (O(1) bump + scope-exit reset, inline at `-O1+`, T-FIBR poison-fill in debug; grown element arrays of lane-backed lists reclaimed at rewind — the lane-aware items pool) |
| Speed | LLVM method devirtualization (1B calls/sec), native `StringBuilder` (~150× faster strings), ARC borrow elision |
| Null | `otherwise`, `x!!`, `x?.m()`, smart unwrap after `if (x != none)` |
| Errors | `(T, error)`, `try`, `panic` — not Java exceptions |
| OOP | `blueprint`, inheritance, `closed`/`guarded`, MVP polymorphism + generics |
| Concurrency | Work-stealing green tasks (`spawn`), structured concurrency (`nursery`), move-on-send channels (`chan<T>`), locks |
| Modules | `use std.math`, `use mylib.calc` |
| Tooling | `snc fmt`, `snc lsp`, `snc debug`, `snc translate`, `snc pkg`, `snc --poison` |
| Memory | **Elastic Lanes** (virtual-memory bump allocators, O(1) scope-exit reset); `SnLane.parent` (task nurseries loan lanes, reclaimed O(1) at join); inline 3-instruction bump + 1-store reset at `-O1+`; compile-time poison-fill (`--poison` / `-O0` / debug builds — the T-FIBR read-before-write guard); linear `SnMsgEnvelope` cross-thread slabs (exactly-once adopt, panic-safe drain); global pools with mode-aware runtime objects (`frozen_depth`, the synchronized vault lock, the `thread_local` home-thread tag — `unfrozen` windows re-freeze on every exit) |

## Stdlib

| Package | Role |
|---|---|
| `std.math` | `abs`, `min`, `max`, `pow`, `clamp`, `sign` |
| `std.string` | `isEmpty`, `repeat`, `StringBuilder` (amortized O(1) buffer) |
| `std.io` | `println` |
| `std.json` | `parse` / `encode` |
| `std.time` | `now_ms`, `sleep_ms`, `format` |
| `std.http` / `std.net` | HTTP client + `serve_once`; **HTTPS via verified `curl`** |
| `std.file` / `std.path` / `std.os` | files, paths, env/args |
| `std.pool` | Generational slot vaults with 64-bit handles (ABA / UAF immune); global pools carry a mutability mode — frozen by default, `thread_local`, or `synchronized` (the E0803.1/.2/.3 discipline) |
| `std.delta` | **Tier-G Z-set delta collections** — `ZSet` with exact retraction (add/remove as ±ℤ coefficients), pure `delta`/`apply` operators (the DBSP 𝒟/⊕ primitives), `merge`, `count`, `total`; O(1)/key operations over the O(1) hash map |
| `std.lattice` | **Tier-L semilattice cells** — `LCell` with three join-semilattice carriers (`max` high-water mark, `gset` grow-only set, `lww` last-writer-wins + logical clock), monotone `inflate` (the only mutator), `merge` (⊔), pure `delta`/`apply` (the mutator law m(X) = X ⊔ m^δ(X)); NO ⊖ on this surface — exact retraction is rejected at compile time (E-LRET) |
| `std.zeta` | **Salt ζ — the fixpoint bracket + the admission checker** — `ZFix` over the admitted transitive-closure body (`reach(a) :- edge(a,b), reach(b)`; edges as packed pairs `a·2³² + b` in a `std.delta` Z-set): `new`/`new_l` (Tier-G presence / Tier-L GSET-closure carriers — Tier L qualifies by construction), `run` (the δ₀/∫ bracket to quiescence — semi-naïve rounds, ≤ height at the least fixpoint), `rounds`/`len`/`has`/`probes` (the work observables), `quiescent_g`/`quiescent_l` (the per-tier stop tests: 𝒟(∫α) = 0 / F(s) ⊑ s via the mutator law); the divergent modes `zfix_count`/`zfix_negate` are recognized intrinsics always rejected with E-ZADM (admission is undecidable in general — the check is the sufficient condition priced at expressiveness) |
| `std.arena` | Elastic lane bump allocators with instant O(1) bulk rollback |
| `std.test` | `assert_true`, `assert_eq_int`, `assert_eq_str`, … |

HTTP details: [docs/HTTP.md](docs/HTTP.md)

## Package manager

Local folders + `sn.toml`, lockfile `sn.lock.toml`, optional remote registry.

```sh
snc pkg init --name myapp
snc pkg add mylib --path packages/mylib
snc pkg add foo --registry https://example.com/snlang --version 1.0.0
snc pkg list
snc pkg build -o app
snc pkg publish                 # dist/<name>-<ver>.tar.gz
```

Commit `sn.lock.toml` in apps. Registry `index.json` format and a static-server recipe:
[docs/PACKAGES.md](docs/PACKAGES.md)

## MVP vs complete (honest)

| Feature | Status | Notes |
|---|---|---|
| HTTPS/TLS | **MVP** | `https://` via system `curl` with cert verify; `SN_HTTP_INSECURE=1` for `-k` |
| Remote registry | **MVP** | Fetch `index.json` + tarball; loud error on bad extract (no stub `lib.sn`) |
| Lockfile / publish | **Done** | `sn.lock.toml`; `pkg publish` → `.tar.gz` |
| async/await | **MVP** | Blocking futures |
| Concurrency & Nurseries | **Hardened** | Work-stealing pool, `nursery { }` task groups, move-on-send channels, `(val, ok)` close handling (see `tests/concurrency/`) |
| Speed / Devirtualization | **Done** | Direct call lowering in `ir.rs`, native `StringBuilder` in `runtime.c` (see `tests/benchmarks/`) |
| LSP | **Scaffold** | Diagnostics with line/col ranges; symbols |
| Self-hosting | **Scaffold** | Lexer demo only |
| Central registry hosting | **Not done** | Bring your own static host |
| In-process OpenSSL | **Not done** | Curl for HTTPS keeps builds simple |
| Full lifetimes / work-stealing | **Not done** | — |

## Translator

Subset map from Python / JS / Go / C → SNlang (`snc translate --from python …`).
Not a full frontend for those languages.

## Layout

- `compiler/` — Rust `snc`
- `compiler/runtime.c` — strings, lists, HTTP, threads, …
- `stdlib/std/` — standard library
- `packages/` — sample packages (`mylib`)
- `examples/` — runnable programs
- `docs/` — packages, HTTP, language tour
- `tests/` — extra samples
- `vscode-snlang/` — editor grammar
- `LICENSE` — Apache-2.0

## Docs

| Doc | Contents |
|---|---|
| [docs/LANGUAGE.md](docs/LANGUAGE.md) | Short language tour |
| [docs/PACKAGES.md](docs/PACKAGES.md) | `sn.toml`, lockfile, publish, registry |
| [docs/HTTP.md](docs/HTTP.md) | Client, env vars, TLS |
| [SNLANG_SPEC.md](SNLANG_SPEC.md) | Full language status |
