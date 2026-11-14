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

## Running the Suite

Run the master test runner script:

```bash
bash tests/systems/run_all.sh
```
