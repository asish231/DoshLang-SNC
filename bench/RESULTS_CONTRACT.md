# Wave-15 Contract-Layer Benchmark — Salt ε's boundary, priced

**Agent:** Impl-T, Cohort Delta (implementer), Wave 15 · 2026-11-15
**Feature measured:** Salt ε — the numeric contract layer (`requires(cond) ensures(cond)` clauses on fn declarations; A4's Presburger decidable ladder + the final report §7.3's boundary rule, slice 1): the facts table (Go-prove-style, sound-not-complete) over the DBM zone layer (Floyd–Warshall closure, Miné HOSC 2006 Thms 4.1/4.2), the atom classifier (tiers L/D/N), and the typed runtime assertions the undischarged atoms degrade to.
**Machine:** Apple M4, macOS 27.2.1 (darwin27, arm64), 16 GB · single-threaded
**Toolchain:** snc 0.2.0 (worktree at HEAD past 20ffeb6) · Apple clang `-O2` (the snc default `.ll` link)
**Protocol:** CONTRACT_BASE = 5 (the base) so every timed op phase totals ≥ 100 ms (≥ 100× the .sn ms clock, per the W2F discipline); one warmup at base/10, **median of 5 timed runs**; every row prints a deterministic checksum. Raw runs: `results/sn_contract_bench.txt` (committed).
**Re-run:** `CONTRACT_ONLY=1 ./run_bench.sh` (env `CONTRACT_BASE`, `RUNS`).

## 1 · The shape, and what is held fixed

- **THE HONEST TWIN DISCIPLINE (stated up front).** The DBM engine lives in the checker (`compiler/src/dbm.rs`, compile-time only — the runtime never sees a zone); an in-language .sn program cannot call into the Rust compiler. Rows (a)–(c) therefore price THE ALGORITHM'S SHAPE on the runtime's list/map primitives — the same closure loop (INF-leg skip and all), the same signature-keyed memo, the same per-atom edge lookup — the same caveat the pinned-lane benchmark's C twin carries (bench/RESULTS_PINNED_LANE.md). The compile-time engine's own cost rides `snc`'s build and is NOT separately measured here. Row (d) is fully REAL: the same contracted fn compiled by this compiler, the degraded site paying the emitted assertion, the discharged site paying nothing.
- **(a) closure25/50/100** — the Floyd–Warshall all-pairs closure at V = 25/50/100 over DENSE seeded-LCG zones (V²/8 + V edges, constants in [−20, 20], INF = 2⁵⁰): per closure, a fresh seed + the full closure + the finite-cell fold. ns/op is per CLOSURE; ops is the cell-triple count (V³ per closure), so the per-cell figure divides out below. DENSE is the honest choice: on sparse zones the outer `d[i][k] = INF` skip collapses the inner loop and the cost tracks ~V², not the worst case — the floor §7.3 names is the V³ budget.
- **(b) memo_hit vs reclub** — the memoized facts-table hit: the zone's signature (a deterministic edge fold) keys a `map<int,int>`; a hit is ONE map probe. The naive baseline pays the full miss path per query (the re-seed + the closure). The CHECKSUMS MUST MATCH: both rows fold the same closed zone.
- **(c) atom_test vs reclub** — the per-atom discharge test: one resolved-edge lookup against the ALREADY-CLOSED zone (what a discharged atom costs at a call site, after the entry-zone memoization) vs the naive re-check-everything baseline (a full re-closure per atom — the reclub row priced per re-closure).
- **(d) degrade_call vs discharged_call** — the boundary rule's runtime price, end to end: `get(xs, k % 3)` (the engine could not discharge `0 <= k % 3` — the remainder-range axiom bounds it in [−2, 2], which does not prove non-negativity — so ONE typed runtime assertion fires per call; the second atom `k % 3 < xs.length()` IS discharged by the axiom + the literal length and lowers nothing) vs `get(xs, 0)` (both atoms fold; zero contract code emitted — the compile is warning-free at that site). The per-atom pricing is live in the emitted code, exactly as the W-CONTRACT warnings say.

GC threshold pinned to 2×10⁹ (out of every timed phase). Single-threaded.

## 2 · Results (median of 5)

| Row | ns/op | ops/run | phase_ms (median) | checksum (5/5 identical) |
|---|---:|---:|---:|---:|
| timer (clock-pair overhead) | 10 | 100,000 | 1–2 | wall-clock sum (varies by construction) |
| **closure25** (the V³ floor, V=25) | **27,000** per closure | 78,125,000 cell-triples | 135 | −8207760089458948732 |
| **closure50** (V=50) | **235,000** per closure | 75,000,000 | 141 | −6430673205530566869 |
| **closure100** (V=100) | **1,680,000** per closure | 75,000,000 | 126 | 7313645875543522403 |
| **memo_hit** (the fast path, 30M queries) | **4** | 30,000,000 | 127 | −863607337778311680 |
| **reclub** (the naive baseline, 5,000 re-closures) | **21,200** | 5,000 | 106 | −863607337778311680 |
| **atom_test** (the discharge lookup, 50M atoms) | **2** | 50,000,000 | 111 | −863745282384311680 |
| **degrade_call** (150M calls, one assertion each) | **1.72** (the integer print floors it to 1) | 150,000,000 | 258 | 2999998500 |
| **discharged_call** (150M calls, nothing lowered) | **0.74** (the integer print floors it to 0) | 150,000,000 | 112 | 1500000000 |

Raw per-run spreads (phase_ms): closure25 {134..136} · closure50 {140..143} · closure100 {125..130} · memo_hit {126..128} · reclub {106..108} · atom_test {111..114} · degrade_call {257..262} · discharged_call {111..113}. All checksums bit-identical across the 5 runs; **memo_hit == reclub** (the same zone, closed either way — the determinism claim, self-checked). The two `ns_per_op` values printed as 1 and 0 are the integer floor of 1.70 and 0.74 (the raw phase_ms/ops columns carry the precision; the table above derives from them).

## 3 · What the numbers say about the boundary

1. **The closure floor is real and cubic: ~1.7–1.9 ns per cell-triple, flat across V — the totals grow 27.0 µs → 235 µs → 1.68 ms, the 1 : 8.7 : 62.2 ratio against the 1 : 8 : 64 of V³.** That is §7.3's "closure is the floor, not the budget" made concrete: V = live SSA values, so a 100-value function pays ~1.7 ms per closure here (in-language list indexing; the Rust engine's flat arrays are faster but the SHAPE is the claim) — and the budget discipline is iterations × branch-factor × V³, of which this row prices exactly one V³. The dense seeding is what makes the row honest: on the sparse zones the outer INF-leg skip collapses the inner loop (measured ~66 µs at V=100 — the ~V² regime), which is the memo's payoff in miniature, not the floor.
2. **The memoized fast path pays for itself ~5,300×: 4 ns per query against 21,200 ns for the naive re-closure.** The memo row's one miss (the initial closure) is amortized over 30M hits; the naive row pays the miss path on every query. This is §7.3's "DBM/zone closure as a memoized fast path" — the entry zone is closed once per function (the caller-side memo) and every call site inside reads the closed matrix instead of re-closing.
3. **The per-atom discharge test is an O(1) lookup: ~2 ns against the naive 21,200 ns — the discharged atom costs four orders of magnitude less than a re-check.** After the zone is closed, testing an atom is one resolved-edge read (`d[i][j] ≤ c`); the naive baseline re-closes per atom. The classifier + the memo together are what make per-atom, per-callsite discharge affordable — the engine's whole design in one ratio.
4. **The boundary rule's runtime price is ~0.96 ns per call (1.70 vs 0.74) — one typed runtime assertion (a `%`, an `icmp`, and the `sn_contract_assert` call) against zero for the discharged twin.** The degraded atom is priced per call and per atom, exactly as the compile warning says; the discharged atom lowers nothing. Sub-nanosecond per call means degrading a hot loop's worth of atoms is cheap in time — the real cost of the degrade is the CHECK, not the clock: a violated assertion kills the process (the fail-fast), and the compile-time warning is the paper trail.
5. **The timer row is the clock-pair measurement itself** (inherently sub-ms; its checksum sums wall-clock readings and varies by construction — the wave-bench precedent). Every op phase is ≥ 105 ms and every op checksum is bit-identical across runs.

## 4 · The honest caveats

- **Rows (a)–(c) price the algorithm's shape in-language, not the Rust engine's absolute time** — the .sn twin discipline (the list-get/map-probe costs ride the runtime's primitives; the engine's flat Vec closure is faster by a constant factor). The O(V³) shape, the memo payoff, and the per-atom O(1) are the claims these rows support; the engine's constant is not.
- **Row (d) is the real thing end to end** — the same compiler, the same contracted fn, the emitted assertion vs the folded call — but it prices ONE degraded atom (the second atom discharged by the axiom): a call site with N degraded atoms pays N assertions, and an atom referencing `xs.length()` pays the length call inside the check.
- **The dense seeded zones include empty ones (planted negative cycles)** — the closure runs identically on them (the emptiness is the negative diagonal), and the checksums stay deterministic. The re-closure of an already-closed EMPTY zone is not a fixpoint (the negative cycle keeps tightening until saturation) — the naive baseline row therefore re-seeds per query, which is the honest miss path anyway.
- **`atom_test`'s checksum carries the walk's bound-sum on top of the zone fold** (it differs from memo/reclub by that sum by design); the memo/reclub pair is the agreement check.
- **The fall-through/default-arg/entry-degrade paths are not separately priced** — they are the same emitted assertion shape as degrade_call, at a different boundary.
