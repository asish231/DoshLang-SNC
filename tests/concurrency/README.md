# Phase 3 Concurrency Test Suite

This directory contains the verification tests and scale benchmarks for SNlang Phase 3: "Concurrent like Go & Concurrency Innovations".

## Tests Overview

| Test File | Target Feature | Expected Behavior |
|-----------|----------------|-------------------|
| `test_nursery.sn` | Structured Concurrency Nursery | Verifies that a `nursery { spawn ... }` block does not exit until all child tasks spawned within it have completed execution. |
| `test_move_on_send.sn` | Move-on-Send Channel Safety | Verifies compile-time rejection when a value is sent across a channel and accessed afterwards. |
| `test_chan_close.sn` | Non-crashing Channel Close Recv | Verifies reading from a closed channel returns `(val, false)` without crashing with `exit(1)`. |
| `bench_spawn_scale.sn` | Work-Stealing Pool Green Tasks Scale | Spawns 10,000+ green tasks on the work-stealing pool and verifies clean, high-performance execution without thread pool exhaustion. |

## Running the Suite

Execute:
```bash
./tests/concurrency/run_all.sh
```
Or test individually with the compiler `./snc`:
```bash
./snc tests/concurrency/test_nursery.sn -o /tmp/snc_nursery && /tmp/snc_nursery
./snc tests/concurrency/test_move_on_send.sn -o /tmp/snc_move # Expected compile error
./snc tests/concurrency/test_chan_close.sn -o /tmp/snc_close && /tmp/snc_close
./snc tests/concurrency/bench_spawn_scale.sn -o /tmp/snc_bench && /tmp/snc_bench
```
