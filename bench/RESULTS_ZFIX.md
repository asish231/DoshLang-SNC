# Wave-9 Salt-ζ Fixpoint-Bracket Benchmark — semi-naïve ζ rounds vs the naive fixpoint

**Agent:** Impl-N, Cohort Delta (implementers), Wave 9 · 2026-11-15
**Feature measured:** Salt ζ — the δ₀/∫ fixpoint bracket (`SnZFix`, on `rabbit-hole-impl`; W2C §3.2–§3.5): the convergence theorem's sufficient condition made executable. The LAWS and the exact per-round accounting are proven elsewhere (golden: `compiler/tests/golden.rs` `zfix_*`, 6 tests; systems: `tests/systems/test_zfix.c`, checks 17/17b — per-round deltas == BFS levels EXACTLY, per-round probes == the delta's indeg sum). This benchmark prices what the bracket COSTS: the incremental semi-naïve rounds (Thm 4b — the incrementalized fixpoint IS classical semi-naïve evaluation, Bancilhon–Ramakrishnan SIGMOD 1986) vs the NAIVE fixpoint (recompute from the full state every round — no delta tracking).
**Machine:** Apple M4, macOS 27.2.1 (darwin27, arm64), 16 GB · single-threaded
**Toolchain:** snc 0.2.0 (worktree at HEAD past a4eeef8) · Apple clang `-O2` (the snc default `.ll` link)
**Protocol:** the layered graph = 1000 chains × 100 edges = N=100,000 edges (101,000 nodes), seeds at each chain's far end; backward reach (`reach(a) :- edge(a,b), reach(b)`) covers all 101,000 nodes in exactly **100 rounds — the diameter** (the ≤ height bound with equality; the known-depth control). Every timed phase totals ≥ 100 ms (≥ 100× the .sn ms clock, per the W2F discipline); one warmup at rounds/10, **median of 5 timed runs**; each run prints its own clock-pair overhead (timer row). Raw runs: `results/sn_zfix_bench.txt` (committed).
**Re-run:** `ZFIX_ONLY=1 ./run_bench.sh` (env `ZFIX_ROUNDS`, `RUNS`).

## 1 · The shape, and what is held fixed

Both engines consume the SAME prebuilt relation (a Z-set of 100,000 packed pairs `a·2³² + b`) and the SAME 1000-node seed set, and must reach the SAME least fixpoint — the `agree=` cross-check asserts identical size and node-id sum **every run** (5/5 identical here). What the rows time:

- **zfix_run** — R × { `zfix_new` + `zfix_run` } per bracket: the one-time O(|E|) CSR materialization (the bracket's ℐ step) + the semi-naïve rounds. Per round the engine touches ONLY the previous round's delta: ~1000 delta nodes → ~1000 adjacency probes → never the 101K-node state. ops = 100,000 derivations per bracket (each non-seed node derived exactly once); `probes` = the total adjacency-probe count (100,000 — each reached node's adjacency visited exactly once).
- **zfix_l_run** — the Tier-L GSET-closure variant at the same shape: the same rounds over the semilattice carrier (union joins + the membership-guarded state-relative fragment — the mutator-law delta computed in O(|δ|), not the O(|state|) scan the ⊖-less carrier would otherwise need).
- **naive_run** — R/10 × { the naive fixpoint in-language: scan ALL 100,000 edges every round until no change }: the honest O(|state|)-per-round twin — no delta tracking, (100+1) × 100,000 = 10,100,000 edge visits per bracket. ops = edge scans. (Its `rounds=101` counts the terminating no-change scan; the bracket's `rounds=100` counts productive rounds — the same bracket, the counters count different things.)
- **zfix_build** — the relation's construction cost (100,000 packed-pair `zset_add`s + the seed set), the one-time materialization outside every per-round claim.

GC threshold pinned to 2×10⁹ (out of every timed phase); an explicit `gc_collect()` BETWEEN phases only. Single-threaded; median of 5. Per-phase round counts: R_ZFIX=20 brackets, R_NAIVE=2 brackets (the naive engine is ~100× the per-round work — 10× fewer brackets keeps its phase in the same wall-time class).

## 2 · Results (median of 5; per-bracket wall time = phase_ms / brackets)

| Row | ns/op | per-bracket | ops/bracket | checksum (5/5 identical) |
|---|---:|---:|---:|---:|
| timer (clock-pair overhead) | 10 | — (phase_ms=1, the clock's floor) | 100,000 | wall-clock sum (varies by construction) |
| zfix_build (the relation's construction) | 26 | 2.6 ms | 100,000 adds | 2,020,000 (20 brackets) |
| **zfix_run** (Tier-G bracket: CSR ℐ + semi-naïve rounds) | **119** | **12.0 ms** | 100,000 derivations | 2,020,000; rounds=100, probes=100,000 |
| **zfix_l_run** (Tier-L GSET-closure bracket) | **116** | **11.6 ms** | 100,000 derivations | 2,020,000; rounds=100, probes=100,000 |
| **naive_run** (the O(\|state\|)-per-round twin) | **17** | **180.5 ms** | 10,100,000 edge scans | 202,000 (2 brackets) |

Fixpoint agreement: `zfix_len=101000 = naive_len`, `zfix_sum=5100449500 = naive_sum`, `agree=true` — **5/5 runs identical** (the two engines meet at the same least fixpoint; the id-sum is the value checksum).
Raw per-run spreads: zfix_run ns/derivation {118, 119, 119, 123, 125} · zfix_l_run {110, 113, 116, 117, 120} · naive_run {17, 17, 17, 17, 17} · zfix_build {26, 26, 26, 26, 26} · timer {10, 10, 10, 10, 10}.

## 3 · What the numbers say about the bracket

1. **The semi-naïve bracket beats the naive fixpoint 15× on wall time at this shape — and the WORK shape is 100× apart.** Per bracket the ζ engine performs 100,000 adjacency probes (each reached node's adjacency visited exactly once — the Thm 4b invariant the systems test proves exactly); the naive engine performs 10,100,000 edge scans (every edge, every round). The wall-time gap (12.0 vs 180.5 ms) is smaller than the work gap because the per-op costs differ: a naive scan is a list read + two O(1) hash probes (17 ns), while the bracket's 119 ns/derivation carries the CSR materialization (~O(|E|) once per bracket) plus the per-derivation hash inserts. On the rounds alone (bracket minus the ℐ step, ~8 ms vs 180 ms) the gap is ~22×.
2. **Per-round work is O(|δ|), never O(|state|) — the Thm 4b claim, priced.** Each of the 100 rounds touches ~1000 delta nodes and probes ~1000 adjacencies (probes=100,000 total); the naive round touches all 100,000 edges (10.1M total). The systems test asserts the per-round probe counts EXACTLY against reference BFS levels; this benchmark prices the same shape at scale.
3. **Tier L matches Tier G at the same shape (116 vs 119 ns/derivation)** — the GSET-closure bracket rides the identical semi-naïve round mechanics (union joins + the membership-guarded fragment), confirming the tier system's pricing: the tiers differ in ALGEBRA (retraction: exact vs tombstoned/none; deltas: 𝒟 vs the mutator law), not in bracket cost. The by-construction admission (W2C §3.3) costs nothing here.
4. **The rounds ≤ height bound is the observable, not the cost model.** The bracket terminates in exactly 100 rounds = the graph's diameter (the longest derivation chain); a wide-but-shallow graph would converge in fewer rounds over MORE facts (the systems test's star: 1 round, 257 facts). Convergence bounds bound the ROUNDS; per-round work remains data-dependent (W2C §3.4).

## 4 · The honest caveats

- **W_μ is the ROUND-SUM, not |ΔDB|-bounded (W2C §3.4 — the §3.4 caveat, verbatim).** One seed batch of 1000 facts (|ΔDB| = 1000) here drives 100 rounds and 100,000 derivations — a 100× amplification, the fraud-propagation shape (differential dataflow's CIDR 2013 case). The bracket's TOTAL work is Σ_rounds c·(|δ₀^(r)| + Σ_v |Δv^(r)|): bounded by the lattice height (the convergence theorem), never by the update size. The O(|δ|) claim is PER ROUND.
- **The per-op columns compare different op units.** A zfix "op" is a derivation (with the CSR ℐ amortized into it); a naive "op" is one edge scan. The honest cross-engine comparison is the per-bracket wall time and the work counts; the ns/op columns price the ops within each engine.
- **The naive twin is a fair naive, not a strawman.** It scans a flat key list (no delta tracking, but O(1) hash membership — the best a naive Datalog evaluator does without semi-naïve; Bancilhon–Ramakrishnan's own baseline shape). A naive engine that re-derived from a sorted adjacency would still pay O(|E|) per round.
- **The layered chain is the worst-case depth shape** (diameter = |nodes|/1000 … per chain): it maximizes rounds per fact. A shallow graph converges in fewer rounds and the bracket's advantage narrows toward the CSR-ℐ floor — the gap is a function of depth × width, exactly as W2C §3.4 says (W_μ is per-circuit; no intermediate-size completeness is claimed).
- **The ms clock's floor.** SNlang's only clock is `time_now_ms`; every phase is sized ≥ 100 ms so quantization stays ≤ 1%. The timer row is inherently sub-ms (it IS the clock-pair measurement); its checksum sums wall-clock readings and varies by construction, while every op checksum is bit-identical across runs.
- **Honest scope: per-mode admission only.** The benchmark's body is the ADMITTED built-in closure; the surface has no user-defined fixpoint bodies yet, so the admission check is per-mode on the built-in bodies — a user-body certifier (the Propel-style checked-not-proven class, W2C §6.4) is a later slice. The two divergent modes (counting-/negation-through-recursion) never reach the runtime: E-ZADM rejects them at compile time.
