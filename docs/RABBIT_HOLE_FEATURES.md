# The Advanced Systems & Non-von Neumann Features Guide

**Language:** SNlang (`snc 0.3.1+`)  
**Audience:** Systems programmers, kernel architects, performance engineers, and language researchers.  
**Scope:** The 5 Non-von Neumann foundations and advanced execution tiers implemented across the SNlang compiler and runtime.

---

## Overview: The 5 Non-von Neumann Systems Pillars

Traditional imperative languages (C, C++, Rust, Go) inherit the von Neumann architectural bottleneck: sequential memory access, pointer aliasing, non-deterministic thread interleavings, and stop-the-world garbage collection pauses.

SNlang addresses these physical and mathematical limits through five specialized language pillars:

```
                                  THE 5 SYSTEMS PILLARS
                                             │
      ┌──────────────┬───────────────┼───────────────┬──────────────┐
      ▼              ▼               ▼               ▼              ▼
1. Pinned Lanes   2. Space-Filling   3. Confluent    4. Numeric     5. Differential
   & Region          Collections        Lattices &      Contracts      Dataflow & Tape
   Arenas            (SFC)              Waves          (Salt ε)        (DBSP / Z-sets)
   `region { }`     `morton list`      `lcell`        `requires`      `zset`, `zfix`
   O(1) stack       `hilbert map`      `wave { }`     `ensures`       `tape`
```

| Pillar | Physical / Mathematical Challenge | SNlang Mechanism | Key Primitives |
| :--- | :--- | :--- | :--- |
| **1. Memory** | Rice–Turing barrier ($\tau^*(a)$ undecidable); GC pause spikes | **Pinned Lanes & Region Posets** | `region { }`, `lane_alloc`, `std.arena` |
| **2. Layout** | DRAM latency wall (400–550 cycle CPU stalls) | **Cache-Oblivious Geometry (SFC)** | `morton list`, `hilbert map` |
| **3. Concurrency** | Interleaving explosion $\Omega = \frac{(TK)!}{(K!)^T}$; data races | **Theorem WS & Join-Semilattices** | `lcell`, `wave { }`, `read_W`, `contrib`, `read_J` |
| **4. Verification** | Matiyasevich's theorem (Diophantine undecidability) | **Salt $\varepsilon$ DBM Zone Contracts** | `requires(...)`, `ensures(...)`, `verified fn` |
| **5. Reactivity** | Computational waste; $\mathcal{O}(N)$ recomputation on updates | **DBSP $Z$-sets & Execution Tape** | `zset`, `zfix`, `tape` |

---

## 1. Pinned Lanes & Elastic Region Arenas (`region { ... }`)

### The Concept
Rice's Theorem proves that finding the exact minimal lifetime $\tau^*(a)$ of a memory address is undecidable.
- **Tracing GC** over-approximates lifetimes, introducing unpredictable stop-the-world latency spikes and $2\times$ to $3\times$ memory bloat.
- **Manual malloc/free** under-approximates lifetimes, causing use-after-free and double-free security vulnerabilities.

SNlang introduces **Thread-Local Pinned Lanes** with a compile-time **Region Poset** $(\mathcal{R}, \sqsubseteq_l)$:
1. Memory is allocated via a single-cycle bump pointer inside a thread-local lane.
2. A `region { ... }` block records the lane's allocation watermark on entry and restores it on exit.
3. Deallocation is **$\mathcal{O}(1)$ wholesale rollback**—equivalent to resetting the hardware stack frame.
4. The compiler enforces that inner region pointers **cannot escape** into outer region variables (`E-LANE-ESCAPE`).

```
Lane Base ────────────────────────────────────────────────────────► Lane Limit
 [  Outer Region Memory  ] [      Inner `region { }` Block      ]
                         ▲                                      ▲
                    Base Watermark                         Bump Pointer
                         ▲
                         └──── On block exit, watermark instantly rewinds to here!
```

### Syntax & Usage

```snlang
use std.arena

fn process_batch(int n) {
    // Baseline allocation state before the region
    int baseline = lane_used(0)

    // Everything allocated inside this region is wholesale-reclaimed on exit!
    region {
        ptr scratch_a = lane_alloc(0, 4096)
        ptr scratch_b = lane_alloc(0, 8192)

        // Perform intensive scratchpad calculations...
        print("Lane used inside region: " + cast(lane_used(0), str))
    }

    // On exit, the bump pointer is instantly restored! Zero leaks, zero GC pauses.
    int restored = lane_used(0)
    print("Lane used after region exit: " + cast(restored, str)) // Equals baseline!
}
```

### High-Level Arena Blueprint (`std.arena`)
For dynamic manual control outside lexical blocks:

```snlang
use std.arena

fn main() {
    let ar = Arena.new(0)
    ptr mark = ar.save()           // Save current watermark

    ptr buf = ar.alloc(1048576)    // Allocate 1 MB scratch buffer
    // ... compute ...

    ar.restore(mark)               // Instant O(1) rewind
}
```

### Compile-Time Invariants
- **`[E-LANE-ESCAPE]`**: Attempting to store a reference to an inner region into a variable living in an outer scope is rejected at compile time.
- **`[E-GC-IN-LANE]`**: Storing a GC heap pointer into a pinned lane or vice versa without explicit crossing rules is rejected.

---

## 2. Space-Filling Geometry Collections (`morton list` & `hilbert map`)

### The Concept
On modern 5.0 GHz processors, the physical speed of light in dielectric insulators ($v \approx 150\text{ mm/ns}$) combined with RC gate delay limits unbuffered signal propagation to $< 2.5\text{ mm}$ per clock cycle. Main DRAM is off-chip ($> 30\text{ mm}$ away), incurring **400 to 550 clock cycles of latency** on an L3 cache miss. If $2\%$ of memory accesses miss cache, the CPU spends over **$90\%$ of its time stalled**.

SNlang bakes **Cache-Oblivious Space-Filling Curves** directly into its collection types to preserve spatial locality in 2D and 3D data:

```
        Standard Row-Major (Cache-Hostile)              Morton Z-Order (Cache-Oblivious)
        ┌───┬───┬───┬───┐                              ┌───┬───┬───┬───┐
      0 │ 0 │ 1 │ 2 │ 3 │                            0 │ 0 ──► 1   4 ──► 5 │
        ├───┼───┼───┼───┤                              ├───┼───┼───┼───┤
      1 │ 4 │ 5 │ 6 │ 7 │                              │   ┌───┘   │   ┌───┘
        ├───┼───┼───┼───┤                            1 │ 2 ──► 3   6 ──► 7 │
      2 │ 8 │ 9 │10 │11 │                              ├───┼───┼───┼───┤
        ├───┼───┼───┼───┤                            2 │ 8 ──► 9  12 ──►13 │
      3 │12 │13 │14 │15 │                              ├───┼───┼───┼───┤
        └───┴───┴───┴───┘                            3 │10 ──►11  14 ──►15 │
         Stepping down a column misses cache!          └───┴───┴───┴───┘
                                                        Spatial neighbors stay in same cacheline!
```

1. **`morton list<T, dims=[N, M]>`**:
   - Interleaves coordinate bits $(x, y) \to k$ via hardware **BMI2 (`PDEP`/`PEXT`)** intrinsics on x86_64, or optimized software shifts on ARM64.
   - Ideal for: Matrix stencil sweeps, tensor operations, image filters, and 2D/3D physics simulations.
2. **`hilbert map<K, V>`**:
   - Maps 2D keys along a continuous Hilbert curve.
   - Ideal for: GIS coordinates, spatial range queries, and bounding-box scans.

### Syntax & Usage

#### Morton 2D Grid
```snlang
fn main() {
    // Declare a 16x16 power-of-two Morton list
    morton list<int, dims=[16, 16]> m = morton_new(16, 16)

    // Indexed access via 2D coordinates (x, y)
    morton_set(m, 4, 8, 42)
    int val = morton_get(m, 4, 8)
    print("Value at (4,8): " + cast(val, str))

    // Pure Morton bitwise interleave & coordinate extraction
    int code = morton_index(4, 8)       // Interleaves bits of 4 and 8
    print("X: " + cast(morton_x(code), str)) // 4
    print("Y: " + cast(morton_y(code), str)) // 8

    // Stride-1 curve-order sequential traversal (Hardware Prefetch Friendly!)
    int k = 0
    int total = morton_len(m)
    while (k < total) {
        int item = morton_scan_get(m, k)
        k = k + 1
    }

    // Zero-copy serialization: Values only, no pointer fixups needed!
    ptr blob = morton_to_bytes(m)
    morton list<int, dims=[16, 16]> m2 = morton_from_bytes(blob, 16, 16)
}
```

#### Hilbert Spatial Map & Segment Advance
```snlang
fn main() {
    // Hilbert map of order 3 (8x8 grid: 2^3 = 8)
    hilbert map<int, int> h = hilbert_new(3)

    // Insert elements in Hilbert curve order
    int c = 0
    while (c < 64) {
        int x = hilbert_dx(c, 3)
        int y = hilbert_dy(c, 3)
        hilbert_insert(h, x, y, c * 10)
        c = c + 1
    }

    // Exact bounding-box range query: [x0, y0] to [x1, y1]
    // Tropf-Herzog / Lawder-King segment advance skips irrelevant memory blocks!
    int x0 = 2
    int y0 = 2
    int x1 = 5
    int y1 = 5

    int idx = hilbert_box_first(h, x0, y0, x1, y1)
    while (idx != -1) {
        int entry_val = hilbert_entry_val(h, idx)
        print("Found in box: " + cast(entry_val, str))
        idx = hilbert_box_next(h, x0, y0, x1, y1, idx)
    }
}
```

### Compiler Invariants & Warnings
- **`[E-SFC]`**: Morton dimensions must be equal powers of two (e.g., `dims=[16, 16]`, `dims=[64, 64]`).
- **`[W-SFC-T5B]`**: Emits a performance warning if the programmer writes nested row-major `for (x) for (y)` loops indexing a Morton list, advising sequential curve iteration (`morton_scan_get`) instead.
- **`[E-SFC-ORDER]`**: Hilbert map insertions must follow curve key order; out-of-order insertions are rejected at run time.

---

## 3. Mathematically Confluent Concurrency (`lcell` & `wave { ... }`)

### The Concept
For $T$ threads executing $K$ instructions, the potential execution interleavings grow factorially:
$$\Omega = \frac{(T \cdot K)!}{(K!)^T}$$
Testing all possibilities is impossible.

SNlang guarantees **Data-Race Freedom by Construction** through **Theorem WS** (Wave-Seam Composition Theorem):
1. **Join-Semilattices (`lcell`)**: Shared concurrent state can only evolve monotonically along a join-semilattice $\langle S, \sqcup, \sqsubseteq, \bot \rangle$. Operations commute: $A \sqcup B = B \sqcup A$.
2. **Deterministic Wave Regions (`wave { ... }`)**:
   - `read_W(cell)` reads a frozen, immutable capture $\kappa$ of the cell state.
   - The body executes deterministically.
   - `contrib(cell, val)` stages an atomic join offer.
   - On wave exit ($J(W)$), all staged contributions fold into the cell via an atomic lattice join ($\sqcup$).
   - `read_J(cell)` observes the final folded state.

```
                  WAVE CONCURRENCY TIMELINE
      Thread / Task A            Thread / Task B
            │                           │
            ├───────────────┬───────────┤
            ▼                           ▼
     [ read_W(cell) ]            [ read_W(cell) ]   <── Reads frozen immutable state κ
            │                           │
     [ contrib(...) ]            [ contrib(...) ]   <── Stages lattice offers
            │                           │
            └───────────────┬───────────┘
                            ▼
                    [ JOIN POINT J(W) ]             <── Staged offers fold atomically via ⊔
                            │
                     [ read_J(cell) ]               <── Provably deterministic observation!
```

### Supported Lattice Carriers (`lcell_new(carrier)`)
| Carrier ID | Semilattice Name | Join Operation ($\sqcup$) | Monotonicity Law |
| :---: | :--- | :--- | :--- |
| `0` | **Max HWM** | $\max(a, b)$ | Values can only increase (High Water Mark). |
| `1` | **Grow-Only Set (G-Set)** | $A \cup B$ | Elements can be added, never deleted. |
| `2` | **Last-Write-Wins (LWW)** | Lamport clock tiebreak | Monotonically advances logical clock timestamp. |

### Syntax & Usage

```snlang
// Seam cells MUST be declared at module scope
ptr max_peak = lcell_new(0)       // Carrier 0: Max High Water Mark
ptr active_users = lcell_new(1)   // Carrier 1: Grow-only Set

fn main() {
    // Initial setup merges (must precede the wave in program order)
    lcell_inflate(max_peak, 100, 0)

    wave {
        // 1. read_W reads the deterministic frozen capture κ
        int base = read_W(max_peak)

        // 2. contrib stages offers into the seam cells
        contrib(max_peak, base + 50, 0)   // Offers 150
        contrib(max_peak, base + 20, 0)   // Offers 120 (loses against 150 under max)

        contrib(active_users, 42, 0)      // Adds user ID 42 to set
        contrib(active_users, 84, 0)      // Adds user ID 84 to set
    }
    // Join J(W) happens here: All contributions fold canonically into cells!

    // 3. read_J reads the folded result
    print("Folded Peak: " + cast(read_J(max_peak), str))         // 150
    print("User Count: " + cast(lcell_len(active_users), str))   // 2
}
```

### Compiler Invariants (The Purity Kill Switch)
- **`[E0812]` (Wave Purity Violation)**: The body of a `wave { ... }` block must be a pure mathematical function of the captured state $\kappa$.
  - The compiler **rejects**:
    * Reading system clocks (`time_now_ms()`)
    * Receiving from channels (`chan.recv()`)
    * Thread spawning (`spawn`, `nursery`, `go`)
    * Mutex locking or sleeping
    * File, network, or console I/O
- **`[E-R1]` (Quiescent Capture Rule)**: Rejects concurrent waves sharing the same seam cell unless properly sequenced.

---

## 4. Salt $\varepsilon$ Formal Numeric Contracts (`requires`, `ensures`)

### The Concept
Matiyasevich's Theorem (1970) proved that general integer Diophantine satisfiability is undecidable.
If a language permits arbitrary non-linear contracts ($x \cdot y \le c$, modulo by variable), the compiler's solver either enters an infinite loop or times out.

SNlang establishes the **Presburger Decidable Ladder**:
1. **Linear Contracts (Tier L)**: Difference Bound Matrices (DBM) solve potential constraints ($x - y \le c$) in polynomial $\mathcal{O}(|V|^3)$ time via Floyd–Warshall.
2. **Divisibility Ledge (Tier D)**: Modular arithmetic ($x \% m$) is bounded by remainder-range axioms.
3. **Non-Linear Equations (Tier N)**: Products ($x \cdot y$) and division by variable cannot be verified statically.
   - Inside a **`verified fn`**: Emits compile error `[E0701]`.
   - Inside a **regular `fn`**: Emits compile warning `[W-CONTRACT]` and **safely degrades** into a typed runtime assertion `[E-0702]`.

```
                    THE DECIDABILITY LADDER
    ┌─────────────────────────────────────────────────────────┐
    │ Tier L (Linear): Intervals & Differences [x - y <= c]   │ ──► Discharged at Compile-Time (O(V³))
    ├─────────────────────────────────────────────────────────┤
    │ Tier D (Divisibility): Constants & Modular Ranges       │ ──► Discharged via Remainder Axioms
    ├─────────────────────────────────────────────────────────┤
    │ Tier N (Non-Linear): Products [x * y], Div-by-Variable  │ ──► Degrades to Typed Runtime Assert [E-0702]
    └─────────────────────────────────────────────────────────┘
```

### Syntax & Usage

```snlang
// Preconditions (requires) and Postconditions (ensures)
fn clamp(int x) -> int
    requires(0 <= x <= 100)
    ensures(0 <= result <= 100)
{
    return x
}

// Bounded list index access
fn safe_lookup(list<int> xs, int idx) -> int
    requires(0 <= idx < xs.length())
{
    return xs[idx]
}

// Strict formal verification: Rejects unprovable non-linear operations
verified fn safe_math(int a, int b) -> int
    requires(a > 0 and b > 0)
    ensures(result > 0)
{
    return a + b  // OK! Linear addition is decidable.
    // return a * b  // ERROR [E0701]: Non-linear variable product rejected in verified fn!
}

fn main() {
    list<int> items = [10, 20, 30]

    // Compile-time verified: 0 <= 1 < 3 is proved by compiler!
    int val = safe_lookup(items, 1)

    // Non-linear operation in standard fn gracefully degrades to runtime check:
    int res = clamp(50)
}
```

### Compiler Invariants & Diagnostic Codes
- **Zero-Cost Discharge**: When the compiler proves the contract statically, **no runtime checks or branches are emitted** in the native binary.
- **`[E0701]`**: Non-linear arithmetic inside a `verified fn` is rejected at compile time.
- **`[W-CONTRACT]`**: Warning notifying that an unprovable contract has been degraded into a typed runtime assertion `[E-0702]`.

---

## 5. Differential Dataflow & Deterministic Tape (`zset`, `zfix`, `tape`)

### The Concept
Recomputing an entire dataset of size $N$ when a single item changes is computationally wasteful.
SNlang incorporates **Discrete Database Stream Processing (DBSP)**:
$$\mathcal{F}(A + \Delta A) = \mathcal{F}(A) + \mathcal{D}\mathcal{F}(A, \Delta A)$$
Updates propagate in $\mathcal{O}(|\Delta A|)$ time, proportional only to the delta and independent of total dataset size.

```
       Standard Recomputation                           Differential Incremental Update
      ┌─────────────────────────┐                     ┌─────────────────────────┐
      │   Full Collection A     │                     │     Delta ΔA only       │
      │       (1,000,000 items) │                     │       (1 item)          │
      └────────────┬────────────┘                     └────────────┬────────────┘
                   ▼                                               ▼
            Re-run Pipeline                                 Apply Derivative 𝒟
                   ▼                                               ▼
         O(N) = 1,000,000 ops                            O(|Δ|) = 1 op
```

### 1. $\mathbb{Z}$-Sets (`zset`) — Collections with Exact Retraction
A $\mathbb{Z}$-set maps elements to integer weights (positive for additions, negative for retractions):

```snlang
fn main() {
    // Carrier 0 = Integer weights
    ptr s = zset_new(0)

    zset_add(s, 100, 2)       // Add element 100 with weight +2
    zset_add(s, 200, 1)       // Add element 200 with weight +1
    print(zset_count(s, 100)) // 2

    // Exact mathematical retraction (⊖)
    zset_remove(s, 100, 2)    // Weight becomes 0 (retracted)
    print(zset_count(s, 100)) // 0
}
```

### 2. Fixpoint Bracket (`zfix { ... }`) — Salt $\zeta$
Computes transitive closures over cyclic dependency graphs with guaranteed termination. Only newly discovered deltas are processed in each round (semi-naïve evaluation):

```snlang
fn main() {
    ptr edges = zset_new(0)
    // Encode graph edge 1 -> 2: key = u * 2^32 + v
    zset_add(edges, 1 * 4294967296 + 2, 1)
    zset_add(edges, 2 * 4294967296 + 3, 1)

    ptr seeds = zset_new(0)
    zset_add(seeds, 1, 1)     // Start search from node 1

    // Create fixpoint bracket
    ptr fix = zfix_new(edges, seeds)
    int rounds = zfix_run(fix)  // Iterates until delta = 0

    print("Rounds taken: " + cast(rounds, str))
    print("Nodes reached: " + cast(zfix_len(fix), str)) // Reachable: 1, 2, 3
}
```

### 3. Chronological Execution Tape (`tape`)
An append-only execution log for state snapshots, incremental delta recording, and exact time-travel replay:

```snlang
fn main() {
    tape<int> t = tape_new("audit_ledger")

    ptr state_0 = zset_new(0)
    ptr state_1 = zset_new(0)
    zset_add(state_1, 50, 1)

    // Record Delta (Kind 1): state_0 -> state_1
    tape_record(t, 1, state_0, state_1)

    // Record Checkpoint (Kind 2): Materialized state snapshot
    tape_record(t, 2, state_1)

    // Replay: Instant re-integration from the last checkpoint!
    ptr replayed = zset_new(0)
    tape_replay(t, replayed)
    print("Replayed count: " + cast(zset_count(replayed, 50), str)) // 1
}
```

---

## 6. Quick Reference Cheat Sheet

| Feature | Primary Types & Keywords | Key Intrinsics | Compile-Time Diagnostics |
| :--- | :--- | :--- | :--- |
| **Pinned Lanes** | `region { }`, `ptr` | `lane_alloc`, `lane_used`, `std.arena` | `[E-LANE-ESCAPE]`, `[E-GC-IN-LANE]` |
| **Space-Filling Geometry** | `morton list<T, dims=[N,M]>`, `hilbert map<K,V>` | `morton_new`, `morton_get`, `morton_set`, `morton_scan_get`, `hilbert_new`, `hilbert_insert`, `hilbert_box_first`/`next` | `[E-SFC]`, `[W-SFC-T5B]`, `[E-SFC-ORDER]` |
| **Wave Concurrency** | `wave { }`, `lcell` | `read_W`, `contrib`, `read_J`, `lcell_new`, `lcell_inflate` | `[E0812]` (purity), `[E-R1]` (quiescence) |
| **Numeric Contracts** | `requires(...)`, `ensures(...)`, `verified fn` | `result`, `param.length()` | `[E0701]` (strict), `[W-CONTRACT]` (degrade), `[E-0702]` (runtime) |
| **Differential Dataflow** | `zset`, `zfix`, `tape<T>` | `zset_new`, `zset_add`, `zset_remove`, `zfix_new`, `zfix_run`, `tape_new`, `tape_record`, `tape_replay` | `[E-ZADM]`, `[E-TAPE]` |
