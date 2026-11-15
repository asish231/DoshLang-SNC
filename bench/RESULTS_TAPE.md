# Wave-16 Tape Benchmark — the δ₀-chronicle's record kinds, priced

**Agent:** Impl-U, Cohort Delta (implementers), Wave 16 · 2026-11-15
**Feature measured:** the tape record kinds — W2C's P5↔P2 elevation plan made executable (§36–§38 + §48 item 4; the final report §332's tier-parametric per-operation laws). The tape is the δ₀-chronicle SHAPE (discrete impulses against an integrated state, replay = re-integration from a checkpoint) with three record kinds: **opaque** (§36's free-monoid baseline — raw payloads, no algebra), **delta** (the (prev → next) carrier pair; the impulse derived with the carrier's own ⊖/fragment ops and the LAW CHECKED at the effect boundary: ILC's a⊕(b⊖a)=b on Tier-G triples, the mutator law X ⊔ m^δ(X) = m(X) on Tier-L pairs), and **checkpoint** (the state snapshot; the base that makes replay start in O(1)). This benchmark prices the typed record's PREMIUM over the opaque baseline, the checkpoint snapshot, and the §36 replay-vs-reintegration economics (the window fold vs the full re-scan).
**Machine:** Apple M4, macOS 27.2.1 (darwin27, arm64), 16 GB · single-threaded
**Toolchain:** snc 0.2.0 (worktree at HEAD past 47965af) · Apple clang `-O2` (the snc default `.ll` link)
**Protocol:** K = 100,000 records per tape; TAPE_ROUNDS = 1000 (the delta row × 1/40, the checkpoint row × 1/2 — the law-checked record path is the heavyweight row, ~750 ns/record; the other rows sized to the same window) so every timed op phase totals ≥ 100 ms (≥ 100× the .sn ms clock, per the W2F discipline); one warmup at rounds/10, **median of 5 timed runs**; every row prints its own deterministic checksum. Raw runs: `results/sn_tape_bench.txt` (committed).
**Re-run:** `TAPE_ONLY=1 ./run_bench.sh` (env `TAPE_ROUNDS`, `RUNS`).

## 1 · The shape, and what is held fixed

Every row moves the SAME record count K per tape over `region { }`-recycled chronicles (the tape header, the record directory, and the flat payload buffer are LANE bytes — E-GC-IN-LANE; the region rewind recycles each round's tape, and the growth's abandoned blocks die with it). The delta rows record the fixed-shape pair (pa → pb) whose difference is exactly the impulse d = {1000: +2, 2000: −1} (support 2 — the record cost is a function of the SUPPORT, not the content); the checkpoint state is EXACTLY (K/2)·d, so the two replay tapes — the full tape (K copies of d) and the checkpointed tape (K/2 copies + checkpoint((K/2)·d) + K/2 copies) — chronicle the SAME impulse multiset and must replay to the IDENTICAL integral: the cross-row checksum is the replay == re-integration witness, self-checked on every run. GC pinned to 2×10⁹ (out of every timed phase).

## 2 · Results (median of 5; ns per record)

| Row | ns/op | ops/run | phase_ms (median) | checksum (5/5 identical) |
|---|---:|---:|---:|---:|
| timer (clock-pair overhead) | 10 | 100,000 | 1 | wall-clock sum (varies by construction) |
| **tape_opaque** (the free-monoid append) | **4** | 100,000,000 | 419 | 4,999,950,000,000 |
| **tape_delta_g** (derive + the ILC law check + flatten + append) | **768** | 2,500,000 | 1,922 | 2,500,000 |
| **tape_ckpt** (the state snapshot record) | **19** | 50,000,000 | 965 | 50,000,000 |
| **tape_replay_ck** (the window fold — second half only) | **6** | 50,000,000 | 329 | 100,000,000 |
| **tape_replay_full** (the full re-scan from the zero state) | **6** | 100,000,000 | 625 | 100,000,000 |

Raw per-run spreads: timer {10,10,20,10,10} · tape_opaque {4,4,4,4,4} · tape_delta_g {757,762,768,768,776} · tape_ckpt {19,19,19,19,19} · tape_replay_ck {6,6,6,6,6} · tape_replay_full {6,6,6,6,6}.

## 3 · What the numbers say about the elevation design

1. **The typed record's premium is ~190× the opaque append (768 vs 4 ns) — and the premium is the LAW, not the storage.** The delta record pays per record: the ⊖ derive (`sn_zset_delta` — a fresh Z-set), the ILC reconstruction (`sn_zset_apply` — another fresh Z-set — compared against `next`), the raw-pair flatten, and the append; the opaque record pays the append alone. The storage delta is visible in isolation: the checkpoint row (a support-2 snapshot append, no derive, no law) costs 19 ns — ~5× the opaque append, which is the honest price of the typed record's BOOKKEEPING; the remaining ~40× of the delta premium is the derive-and-check discipline E1 demands at the effect boundary. That is the honest cost of "the ⊕/⊖ laws are CHECKED at the effect boundary" — the tripwire is not free, and the benchmark says so.
2. **The replay economics: 6 ns/record folded, and the checkpoint halves the scan.** `tape_replay_ck` walks the second half only (the last checkpoint reset the base) at 329 ms per 50M records; `tape_replay_full` re-scans all 100M impulses at 625 ms — the same 6 ns/record either way, ~2× the total work without the checkpoint (the phase_ms pair is the §36 economics: replay is O(window), re-integration-from-scratch is O(K), and the checkpoint is what shrinks the window). The per-record replay cost (6 ns — two `sn_zset_add` pairs and the payload walk) is ~128× cheaper than the law-checked record: reading the chronicle back is dramatically cheaper than certifying an impulse into it.
3. **The replay == re-integration witness is the checksum pair**: `tape_replay_ck` and `tape_replay_full` fold DIFFERENT window shapes (half + base vs the whole impulse stream from the zero state) and land the IDENTICAL integral (100,000,000 = 1000 reps × the endpoint total 100,000), five runs out of five — the §36 "replay = re-integration from a checkpoint" claim, executed and self-checked on every run.
4. **The opaque baseline is the cheapest legal record (4 ns)** — the free-monoid chronicle is an append and nothing else (§36's honest baseline: no algebra, no law, no premium). The tape's zero-cost-when-unused story: a program that records opaque pays the chronicle's floor; the typed surface is strictly opt-in.

## 4 · The honest caveats

- **The delta row's premium is allocation-dominated at support 2** (the derive and the reconstruction each allocate a fresh Z-set, and the flatten buffer mallocs per record — ~12 heap ops per record against the opaque row's zero). At larger supports the law check's O(support) walks grow with the state while the allocations stay flat — the 768 ns figure is the SMALL-state worst-premium case, not a per-key rate; per-key marginal costs live in the z-set's own rows (RESULTS_LCELL.md's zset_build/zset_merge).
- **The replay rows fold support-2 impulses** (the fixed-shape pair's difference); a wide-impulse replay pays the carrier's own per-key add rate (the zset_add path), not the flat 6 ns.
- **The checkpoint row prices the SNAPSHOT, not a marker**: slice 1's checkpoint is a materialized state (O(support) at record — 19 ns at support 2), the design that makes replay start in O(1); an O(1) positional marker would move the cost to the replay (the re-scan this benchmark prices as `tape_replay_full`). The trade is stated, and both sides are measured.
- **The timer row is the clock-pair measurement itself** (inherently sub-ms; its checksum sums wall-clock readings and varies by construction — the pinned-lane precedent). Every *op* phase is ≥ 100 ms and every *op* checksum is bit-identical across runs.
- **Single-threaded, single-lane — by design.** Slice 1's tape is one linear ledger on lane 0 (the one-tape-one-ledger rule); no concurrent-append rows exist because no concurrent append exists (the wave boundary rule refuses the tape surface in-wave; the parallel form is the documented future slice).
