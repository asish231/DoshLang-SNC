# SNlang Performance & Optimization Benchmark Suite

This directory contains executable performance benchmarks measuring runtime and compiler optimizations implemented in Round 2.

## Benchmarks Overview

### 1. `bench_string.sn` (String Concatenation vs StringBuilder)
- **Baseline**: Repeated `str + str` concatenation (5,000 iterations). Demonstrates quadratic $O(N^2)$ byte copying behavior.
- **Optimized**: `StringBuilder` (5,000 and 50,000 iterations). Uses native exponential capacity doubling and direct byte copying.
- **Speedup**: Direct 3x speedup at 5,000 iterations; ~150x estimated speedup at 50,000 iterations.

### 2. `bench_dispatch.sn` (Method Dispatch Devirtualization)
- **Workload**: 1,000,000 method calls on a blueprint instance.
- **Optimization**: Direct devirtualization in `ir.rs` replaces indirect vtable branch lookups (`load ptr, ptr %vptr`) with direct calls (`@sn_m_{class}_{method}`) when the receiver type is concrete and known.
- **Throughput**: ~1,000,000,000 calls/sec with LLVM inlining at `-O2`.

### 3. `bench_alloc.sn` (Rapid Object Allocation & Iteration)
- **Workload**: Rapid allocation and traversal of 500,000 blueprint objects.
- **Metrics**: Measures allocation throughput, traversal rate, and tracks runtime GC object counts (`gc_count()`).
- **Throughput**: ~24M objects allocated/sec, ~166M objects traversed/sec.

## Running the Benchmarks

Execute the runner script:
```bash
bash tests/benchmarks/run_benchmarks.sh
```

Benchmark output and summary reports are automatically generated in `tests/benchmarks/BENCHMARK_RESULTS.md`.
