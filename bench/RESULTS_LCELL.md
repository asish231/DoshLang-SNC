# Wave-9 Tier-L Semilattice Cell Benchmark — ⊔ (join) costs per carrier

**Agent:** Impl-M, Cohort Delta (implementers), Wave 9 · 2026-11-15
**Feature measured:** the Tier-L join-semilattice cell (`SnLCell`, commit on `rabbit-hole-impl`; W2C §1.1 Tier L; final report §8.1) — the semilattice tier's first executable slice. The lattice LAWS are proven elsewhere (golden: `compiler/tests/golden.rs` `lcell_*`, 7 tests; systems: `tests/systems/test_lcell.c`, checks 16/16b — the three laws asserted per merge over randomized seeded-LCG op sequences). This benchmark prices what the laws cost.
**Machine:** Apple M4, macOS 27.2.1 (darwin27, arm64), 16 GB · single-threaded
**Toolchain:** snc 0.2.0 (worktree at HEAD past df347a5) · Apple clang `-O2` (the snc default `.ll` link)
**Protocol:** N = 100,000 elements; per-class round counts so every timed phase totals ≥ 90 ms (≥ 100× the .sn ms clock, per the W2F discipline); one warmup at rounds/10, **median of 5 timed runs**; each run prints its own clock-pair overhead (timer row). Raw runs: `results/sn_lcell_bench.txt` (committed).
**Re-run:** `LCELL_ONLY=1 ./run_bench.sh` (env `LCELL_ROUNDS`, `RUNS`).

## 1 · The shape, and what is held fixed

Each row builds or joins the SAME N-element state (values 0..N−1, the exact set — the deterministic checksums) through one primitive per phase:

- **gset rows** — the `LCELL_GSET` carrier (grow-only int set, ⊔ = union, the SnZSet O(1) open-addressing machinery): `gset_inflate` (monotone insert), `gset_merge_fresh` (a fresh destination joined with the prebuilt 100K source — the REAL union: every element inserts), `gset_merge_same` (one destination re-joined with the same source — the idempotent steady state: after round 1 every element is present, each merge is all probes), `gset_delta_apply` (the mutator-law pair: `delta(src, prev_half)` = the N/2-element state-relative fragment, `apply(prev_half, d)` = the round-trip back to the full state).
- **O(1) carrier rows** — `max_inflate` (LCELL_MAX: one compare per offer), `lww_inflate` (LCELL_LWW: one lexicographic (clock, value) compare per offer) — 100M-op sweeps, the same ≥100 ms discipline.
- **Tier-G comparison rows** — `zset_build` (`zset_add(k, 1)` — the same grow-only set as a Z-set, the ⊕ write path), `zset_merge` (a fresh Z-set joined with the prebuilt 100K source — the Tier-G ⊕ of the same shape).
- **plain-list baseline** — `list_push` (the list construction row).

GC threshold pinned to 2×10⁹ (out of every timed phase); an explicit `gc_collect()` runs BETWEEN phases only. Single-threaded; median of 5.

## 2 · Results (median of 5; ns per element of the N joined/built per round)

| Row | ns/elem | ops/run | checksum (5/5 identical) |
|---|---:|---:|---:|
| timer (clock-pair overhead) | 10 | 100,000 | wall-clock sum (varies by construction) |
| **gset_inflate** (monotone insert) | **24** | 4,000,000 | 4,000,000 |
| **gset_merge_fresh** (the ⊔ union join) | **23** | 4,000,000 | 4,000,000 |
| **gset_merge_same** (idempotent steady-state ⊔) | **7** | 12,000,000 | 12,100,000 |
| **gset_delta_apply** (the mutator-law pair, per element scanned) | **43** | 4,000,000 | 6,000,000 (fragment N/2 + round-trip N, per round) |
| **max_inflate** (the O(1) HWM carrier) | **1** | 100,000,000 | 99,999,000 |
| **lww_inflate** (the O(1) LWW carrier) | **1** | 100,000,000 | 99,999,000 |
| **zset_build** (Tier-G ⊕ write path) | **24** | 4,000,000 | 4,000,000 |
| **zset_merge** (Tier-G ⊕ join) | **25** | 4,000,000 | 4,000,000 |
| **list_push** (plain-list construction baseline) | **2** | 40,000,000 | 40,000,000 |

Raw per-run spreads: gset_inflate {23,23,23,24,24} · gset_merge_fresh {23,23,23,24,24} · gset_merge_same {7,7,7,8,8} · gset_delta_apply {42,43,43,43,45} · max/lww {0,1,1,1,1} · zset_build {23,23,24,24,25} · zset_merge {24,24,25,25,25} · list_push {2,2,2,2,2}.

## 3 · What the numbers say about the tier design

1. **The L ⊔ and the G ⊕ of the same set-shaped state cost the same: 23 vs 25 ns/element.** This is expected and honest: at the same carrier shape (an int-keyed hash map), union and pointwise-add ride the identical find-or-insert machinery — **the tiers differ in ALGEBRA (retraction: exact vs tombstoned/none; deltas: 𝒟/⊕ vs the mutator law), not in join cost.** The capability order O ⊑ L ⊑ G prices retraction, not merge speed.
2. **"Mergeable" is not free — and the semilattice's idempotence is the discount.** A fresh union join costs 23 ns/element (every element hashes+inserts); re-joining an already-⊑ source costs **7 ns/element — 3.3× cheaper** (the probe-only steady state). CRDT anti-entropy re-merges hit the cheap path; first contact pays the full insert.
3. **The carrier IS the cost knob — the spec's per-carrier ⊔ prices, measured.** The O(1) carriers (`max`, `lww`) run at **1 ns/op vs gset's O(n) union at 23 ns/element** — ~25× cheaper per operation, and independent of state size. Choosing max/lww over gset trades readout precision (one HWM / one winner vs a full set) for a constant-time join: exactly the HLL/Bloom-vs-materialized-set trade the final report §8.1 prices (Θ(ε⁻²+log U) bits vs Ω(n) space), now with runtime constants.
4. **The mutator-law pair costs ~2× a merge (43 vs 23 ns/element) — as predicted.** `delta` scans N against prev (N/2 hits, N/2 misses → the N/2-element fragment) and `apply` re-joins N/2 elements: the pair is one scan + one half-union. Tier L's state-relative deltas are in the same cost class as Tier G's 𝒟/⊕ round-trip at the same carrier shape — the mutator law is not a premium feature, it is the same hash machinery wearing different algebra.
5. **The plain-list baseline: construction 2 ns/element — but the union baseline is EXCLUDED, honestly.** `list<int>` has no O(1) membership primitive, so an in-language list "union" (push + membership scan per element) is Θ(n²) — 10¹⁰ comparisons at N=100K, minutes per round. That is a different algorithm class, not an honest merge-cost comparison; measuring it would compare scan-loops, not carriers. The list construction row (2 ns vs gset's 24 ns) shows what the O(1) hash machinery costs over a raw append — the price of membership, paid once at insert.

## 4 · The honest caveats

- **The ms clock's floor.** SNlang's only clock is `time_now_ms`; per-class round counts (R_HASH=40, R_PROBE=120, R_LIST=400, R_O1=1000 at N=100K) size every op phase to ≥ 90 ms so quantization stays ≤ ~1.1%. The timer row itself is inherently sub-ms (it IS the clock-pair measurement — the pinned-lane precedent, phase_ms=1); its checksum sums wall-clock readings and varies by construction, while every op checksum is bit-identical across runs.
- **The `max`/`lww` rows at 1 ns/op sit at the clock's arithmetic floor** (phase_ms × 10⁶ / ops with phase_ms ≈ 100, ops = 10⁸) — the true cost is sub-ns-to-ns; treat "1" as "≤1.5" honestly. The 100M-op sweep exists precisely to keep that floor honest.
- **Single carrier shape per tier.** The gset/zset comparison holds N, the int key universe, and the hash scheme fixed; a Str-keyed or sketch-shaped carrier (HLL registers) would price differently — this benchmark prices the L-vs-G join at the SAME shape, which is the comparison the tier question actually asks.
- **The idempotent-merge row measures the gset steady state** (identical sources re-joined). LWW's steady state is O(1) regardless (one compare per merge — see row 6), and max's likewise; gset is the only carrier whose idempotent re-join still scans the whole source.
- **No concurrent-merge contention rows** — the ⊔ is defined for single-threaded merge here; replicated/concurrent CRDT state machines are future surface (the envelope/lane machinery of earlier waves would carry them).
