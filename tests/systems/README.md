# SNlang Systems & Security Verification Test Suite

This directory contains end-to-end verification tests covering Phase 4 systems innovations and security patches in SNlang.

## Test Matrix

| Test Script | Target Feature | Validation Description | Status |
|-------------|----------------|------------------------|--------|
| `test_dec.sn` | Fixed-Point Decimals | Verifies `dec(2)` and `dec(4)` multiplication and division rescaling (e.g. `1.50 * 2.00 = 3.00`, `1.50 / 2.00 = 0.75`). | PASSED |
| `test_const_assign.sn` | Const Immutability | Verifies compile-time rejection of compound assignment to const variables (`const x = 10; x += 1`). | PASSED |
| `test_await_nonfuture.sn` | Async Soundness | Verifies compile-time rejection of awaiting non-future expressions (`let x = await 42`). | PASSED |
| `test_pool.sn` | Generational Slot Pool (`std.pool`) | Verifies `Pool` generational handles, insertion, lookup, modification, deletion, and stale handle detection across slot reuse. | PASSED |
| `test_region.sn` | Region Blocks & Elastic Lanes | Verifies `region { }` automatic lane watermark rollback on block exit and `std.arena` allocation tracking. | PASSED |
| `test_web_traversal.sn` | Web Security (`std.web`) | Verifies path traversal defense in static file serving blocking `..`, `\\`, and leading `/` with 404 responses. | PASSED |
| `test_nursery_early_return.sn` | Structured Concurrency Unwind | Verifies early `return` inside `nursery { }` waits for all child tasks before returning. | PASSED |
| `test_region_early_return.sn` | Elastic Lane Unwind | Verifies early `return` inside `region { }` rewinds lane watermark back to 0 without leaking memory. | PASSED |
| `test_chan_member_move.sn` | Move-On-Send Deep Tracking | Verifies sending a blueprint field (`ch.send(p.x)`) or list element marks the container moved, preventing data races. | PASSED |
| `test_nursery_carve.sn` | Nursery Child-Lane Carve (language level) | Verifies nursery spawn/join round-trips with the ν loan registry in place: children's region-scoped lane-0 allocations keep working (the loaned-lane backstop never bites lane 0), and every child's result survives the join across two rounds. | PASSED |
| `test_nursery_carve.c` | Nursery Child-Lane Carve (runtime O(1) join, D0 gap #5) | Drives the runtime carve API the compiler wiring will emit: `sn_nursery_loan_lane` reserves a ν lane per child, children allocate into it, `sn_nursery_wait` reclaims every child allocation with one store per lane. Verifies exact join-reset/byte stats, the free-set return, nested nurseries, parent-lane-0 untouched, the loaned-lane fail-fast — plain, under ASan, and leak-checked with macOS `leaks(1)`. | PASSED |
| `test_view_engine.c` | Incremental View Engine (P5 slice 2, runtime) | Drives the DBSP-style delta-propagation circuit over Z-set views (`SnView`: FILTER / MAP / AGG-SUM / AGG-COUNT) end to end against an independent C reference model: the chain source→filter→map→aggregate with exact hash-map probe counts (3 ops/changed key per linear view — the O(\|dA\|) work bound, not wall time), committed deltas equal to scratch-recompute differences, retraction (keys and whole aggregate groups zeroed out and pruned), \|A\|-independence (100-key vs 100,000-key sources, identical probe counts), AGG-COUNT presence flips (negative weights present — Tier-G honesty), and tier-G quiescence (empty batch ⇒ 0 probes, empty commits: 𝒟(∫α)=0, W4C repair 3). Includes the fix to `zset_remove_at` (zombie-entry bug: the freed entry's bucket was left pointing at the dead slot; now cleared + backward-shift repair). Runs plain and under ASan. | PASSED |
| `test_zset_delta.sn` | Tier-G Z-set Delta Collections (P5 slice 1, stdlib surface) | Verifies the stdlib Z-set surface over the runtime `SnZSet` engine: exact retraction, 𝒟/⊕ primitives, negative coefficients, and the group law (W2C §1.1). | PASSED |
| `test_zset_delta.c` | Tier-G Z-set Delta (P5 slice 1, runtime mechanics) | Drives the runtime Z-set engine directly: the ⊕/𝒟/apply primitives, pruning, `total` bookkeeping, quiescence, and the group law — plain, under ASan, and leak-checked with macOS `leaks(1)`. | PASSED |
| `test_gpool.c` | Global-Pool Discipline (DIFF-4, Wave 10; kills G1 §1 A6c) | Drives the three declared modes' runtime mechanics: the concurrent FROZEN read (8 threads × 4096 handles — checksums exact, generations stable, len constant), the SYNCHRONIZED vault under 8-thread mutation (the lock serializes; final state exact), the `unfrozen` window's freeze/unfreeze depth discipline (nested depth 3, LIFO unwind, no underflow), and the E0803.1 home-thread backstop (a `thread_local` pool mutated from a foreign thread fail-fasts with `[E0803]`, probed in a fork'd child — the E-LKIND pattern). Plain and under ASan. | PASSED |

## Running the Suite

Run the master test runner script:

```bash
bash tests/systems/run_all.sh
```
