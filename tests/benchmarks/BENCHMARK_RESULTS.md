# SNlang Performance Benchmark Results

Generated on: Sat Nov 14 18:48:16 UTC 2026
Environment: Darwin arm64, LLVM / Clang -O2

## Summary of Results

| Benchmark | Workload | Metric / Result | Status |
| :--- | :--- | :--- | :--- |
| **String Operations** | Baseline `str + str` vs `StringBuilder` | Amortized $O(1)$ exponential buffer growth | **PASSED** |
| **Method Dispatch** | 1,000,000 Blueprint method calls | Devirtualized direct dispatch + LLVM inlining | **PASSED** |
| **Object Allocation** | 500,000 Blueprint objects creation & traversal | Rapid allocation + GC tracking | **PASSED** |

---

## Detailed Benchmark Logs

### 1. String Concatenation vs StringBuilder (`bench_string.sn`)
```text
==================================================
  SNlang Benchmark: String Concatenation vs StringBuilder
==================================================
Running Baseline (str + str, 5000 iterations)...
Baseline (5,000 iters): 3 ms (length=5000)
Running StringBuilder (5,000 iterations)...
StringBuilder (5,000 iters): 1 ms (length=5000)
Speedup multiplier (5,000 iters): 3x
Running StringBuilder (50,000 iterations)...
StringBuilder (50,000 iters): 1 ms (length=50000)
Extrapolated baseline for 50,000 iters (O(N^2)): ~300 ms
Estimated speedup multiplier (50,000 iters): ~300x
==================================================
```

### 2. Method Dispatch (`bench_dispatch.sn`)
```text
==================================================
  SNlang Benchmark: Method Dispatch (1,000,000 calls)
==================================================
Iterations: 1000000
Elapsed time: 1 ms
Throughput: 1000000000 calls/sec
Verification: x=1000000, y=2000000
Verification distance_sq=5000000000000
==================================================
```

### 3. Rapid Object Allocation & Iteration (`bench_alloc.sn`)
```text
==================================================
  SNlang Benchmark: Rapid Object Allocation & Iteration
==================================================
Initial GC tracked objects: 3
Allocated objects: 500000
Allocation elapsed time: 9 ms
Allocation throughput: 55555555 objects/sec
GC tracked objects after allocation: 500005
Traversed objects: 500000
Traversal elapsed time: 3 ms
Traversal throughput: 166666666 objects/sec
Checksum sum: 24750000
==================================================
```
