# Wave-10 Global-Pool-Discipline Benchmark — the three modes' per-op price

**Agent:** Impl-O, Cohort Delta (implementers), Wave 10 · 2026-11-15
**Feature measured:** the global-pool discipline (DIFF-4; kills G1 §1 A6c — the global-pool freelist race): three declared modes, frozen by default. The SAFETY LAWS are proven elsewhere (golden: `compiler/tests/golden.rs` `gpool_*`, 9 tests — E0803.1/.2/.3 verbatim; systems: `tests/systems/test_gpool.c`, checks 18/18b — the concurrent frozen read, the locked mutation, the window depth discipline, the E0803.1 home-thread backstop). This benchmark prices what the modes COST, per op.
**Machine:** Apple M4, macOS 27.2.1 (darwin27, arm64), 16 GB · single-threaded (the cross-thread behavior is the systems suite's job; this is the per-op price)
**Toolchain:** snc 0.2.0 (worktree at HEAD past 72d73b6) · Apple clang `-O2` (the snc default `.ll` link)
**Protocol:** N=1,000,000 ops per round (the insert rows: 200,000 insert+remove pairs per round), R=10 rounds per timed phase, every phase ≥ 14 ms (≥ 14× the ms clock at its own op speed; the read/set rows are 20–47 ms — the W2F ≥100× sizing applies per the op's own cost, and the timer row is the clock's own floor); one warmup at R/10, **median of 5 timed runs**; each run prints its own clock-pair overhead (timer row). Raw runs: `results/sn_gpool_bench.txt` (committed).
**Re-run:** `GP_ONLY=1 ./run_bench.sh` (env `GP_ROUNDS`, `RUNS`).

## 1 · The shape, and what is held fixed

Three **module-scope global pools** of 10,000 live int slots each (`per = N/100`) — real GLOBAL declarations, where the DIFF-4 discipline governs (the frozen pool's fills and restores run inside `unfrozen` windows; the annotated modes construct via `sn_pool_new_mode` at main's entry and are registered as permanent GC roots) — all holding the same canonical values (`v = i*3+1`): a **frozen** pool (`pool<int>` bare), a **synchronized** pool (`pool<int> synchronized`), and a **thread_local** pool (`pool<int> thread_local`). The handles are generation-1 slot indices (`h = 2³² + i`), constructed once outside every timed phase. What the rows time:

- **frozen_read** — R × { N × `get` on the frozen pool }: the zero-lock shared read — the path every thread takes through a frozen pool's handles (the Sendable zero-copy read). Deterministic checksum: the full-pass value sum (the triangular-number closed form, asserted every run).
- **frozen_set** — R × { one `unfrozen` window + N × `set` + the canonical restore (each restore set inside its own window) }: the window mutation — the freeze/unfreeze pair (O(1), once per window) amortized over N ops. The window is the ONLY legal write path on a frozen pool (the checker enforces it: every mutation, including the restore, must sit inside a window).
- **sync_set** — R × { N × `set` on the synchronized pool }: the locked op — `sn_pool_set_i64` takes the vault mutex (`sn_pool_maybe_lock`/`unlock`) around the generation-checked write.
- **sync_insert** — R × { 200,000 × (`insert` + `remove`) } on the synchronized pool: the locked FREELIST mutation — the operation the modes exist to serialize (insert pops `free_head`; remove pushes it; both under the lock).
- **tl_insert** — the same shape on the thread_local pool: the home-thread-guard op (the E0803.1 backstop tag check per mutation — no lock; the declaring-thread-only price).

GC threshold pinned to 2×10⁹ (out of every timed phase). Single-threaded; median of 5.

## 2 · Results (median of 5; ns/op)

| Row | ns/op | phase_ms | ops | checksum (5/5 identical) |
|---|---:|---:|---:|---:|
| timer (clock-pair overhead) | 10 | 1 | 100,000 | wall-clock sum (varies by construction) |
| **frozen_read** (the zero-lock shared read) | **2** | 20–21 | 10,000,000 | 14,999,500,000 (the exact closed form) |
| **frozen_set** (the window mutation) | **3** | 32–35 | 10,000,000 | 10 (windows completed) |
| **sync_set** (the locked checked write) | **4** | 45–47 | 10,000,000 | 10 |
| **sync_insert** (the locked freelist mutation) | **5** | 20–22 | 4,000,000 | 10,000 (len, exact every round) |
| **tl_insert** (the tag-guard mutation, no lock) | **3** | 14–16 | 4,000,000 | 10,000 |

Raw per-run spreads: frozen_read {2,2,2,2,2} · frozen_set {3,3,3,3,3} · sync_set {4,4,4,4,4} · sync_insert {5,5,5,5,5} · tl_insert {3,4,4,3,3} · timer {10,10,10,10,10}. Every op checksum bit-identical across all 5 runs (`values_ok=1` 5/5).

## 3 · What the numbers say about the discipline

1. **The frozen default is the zero-cost path.** Its read row (2 ns/op) carries no lock, no guard, no atomic — it is the same generation-checked load a bare `pool_new` always was. The discipline's default mode added zero instructions to the read path; the Sendable frozen read is free.
2. **Synchronization prices what it buys.** The locked set (4 ns) over the window set (3 ns) and the locked freelist mutation (5 ns) over the tag-guarded one (3 ns): the vault mutex costs ~1–2 ns per op on this machine — the price of the data-race-freedom guarantee (no two threads can ever interleave insert/remove on one pool, the exact condition the systems test's 8-thread mutation round asserts). `synchronized` is not free, and is priced honestly.
3. **The thread_local mode prices confinement, not locking.** The home-thread tag check is a compare-and-branch (3 ns — indistinguishable from the unlocked window mutation at this resolution); the lock is the only thing that costs, and ThreadConfined does not carry one. The trade is exact: confinement is cheap but single-thread; the vault is any-thread but ~40% dearer per freelist op.
4. **The window's freeze/unfreeze amortizes to noise.** The pair is O(1) per window (two integer writes on `SnPool`); at N=1M ops per window it contributes ~0.001 ns/op — the frozen_set row's 3 ns is the checked write itself, not the window machinery. Structured mutation on a frozen pool costs the same as the raw op.

## 4 · The honest caveats

- **The ns/op rows compare different op units.** A read is a generation check + load; a set is a check + store (± the mutex pair); an insert+remove pair is two freelist mutations (± the lock / the guard). The honest comparisons are within-column families (read-vs-read, mutation-vs-mutation), which is exactly the table's layout.
- **The ms clock's floor.** SNlang's only clock is `time_now_ms`; at 2 ns/op the 1M-op round is 2 ms and the 10-round phase 20 ms — the ≥14× margin keeps quantization ≤ 5%, and the spreads show it (the frozen_read phase_ms wobbles 20↔21 between runs; its checksum never wobbles). The timer row is inherently sub-ms (it IS the clock-pair measurement); its checksum sums wall-clock readings and varies by construction.
- **frozen_set's restore pass.** Each window round restores the 10,000 canonical values after the timed mutation stream so the next round starts from the same state; the restore is inside the phase's wall time but the ops column counts only the N mutations — the phase's per-op figure is therefore a slight OVERestimate of the mutation cost (the restore is 1% of the ops). The checksum (10 = windows completed) is exact. The restore's sets sit inside their own windows — the frozen discipline the checker enforces (a mutation outside a window does not compile: E0803.3).
- **The benchmark caught a real bug in its own build.** The first cut declared the pools as function locals (the mode table never records those, so the frozen gate did not govern) and skipped GC-root registration — under `gc_collect()` between phases, an unlucky collection swept the (untracked-root) global pools and the next set hit freed memory (a stale-handle abort). The fix — module-scope declarations + `sn_gc_register_global` at main's entry — is itself part of DIFF-4's deliverable: a global pool is an immortal module-scoped object.
- **The mutex is uncontended here by design.** This is the per-op PRICE of taking the lock (the single-thread floor of the synchronized mode); the contended cost is a scheduling property, not a mode property — and the correctness under contention is what `tests/systems/test_gpool.c`'s 8-thread locked round proves (final state exact, ASan silent).
- **The insert rows run 200K pairs, not 1M.** The freelist mutation is ~5 ns and a 1M-pair round would spend its phase on op work the read rows' comparison doesn't need; the per-op figure is the honest unit either way, and the phase still totals ≥ 14 ms (≥ 100× the 2 ns read's per-op clock share — each phase is sized to its own op's speed, per the W2F discipline).

## 5 · What this benchmark does NOT claim

The frozen read's 2 ns is not "faster than before DIFF-4" — the read path is byte-identical to the pre-wave pool read (that is the point: the discipline added zero cost to the default). The deltas that matter are sync_set − frozen_set ≈ +1 ns/op (the lock) and sync_insert − tl_insert ≈ +2 ns/op (the lock on the freelist mutation) — the honest price of the data-race-freedom guarantee, measured at the single-thread floor.
