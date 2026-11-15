# SNlang Advanced Systems Architecture Guide
## High-Performance Memory Tiers, Cache-Oblivious Geometry, Deterministic Concurrency, and Formal Verification

**Edition:** SNlang 0.3.1  
**Target Audience:** Systems Engineers, Kernel Developers, and High-Performance Software Architects  

---

## Table of Contents
1. [Architecture Overview & Design Philosophy](#1-architecture-overview--design-philosophy)
2. [Chapter 1: Deterministic Regional Memory & Elastic Arenas](#chapter-1-deterministic-regional-memory--elastic-arenas)
3. [Chapter 2: Cache-Oblivious Space-Filling Collections](#chapter-2-cache-oblivious-space-filling-collections)
4. [Chapter 3: Deterministic Concurrency & Monotonic Semilattices](#chapter-3-deterministic-concurrency--monotonic-semilattices)
5. [Chapter 4: Contract-Based Static Verification](#chapter-4-contract-based-static-verification)
6. [Chapter 5: State Chronology & Differential Dataflow](#chapter-5-state-chronology--differential-dataflow)
7. [Chapter 6: Relocatable Links & Generational Slot Pools](#chapter-6-relocatable-links--generational-slot-pools)
8. [Compiler Diagnostic Code Reference](#compiler-diagnostic-code-reference)

---

## 1. Architecture Overview & Design Philosophy

SNlang is designed for systems where latency predictability, physical cache efficiency, and formal safety guarantees are non-negotiable. Modern systems software faces three fundamental physical and mathematical boundaries:

1. **The Allocation Pause Barrier**: Dynamic heap collectors introduce unpredictable latency spikes (stop-the-world pauses), while manual pointer memory management introduces use-after-free and double-free vulnerabilities.
2. **The Physical Memory Wall**: High CPU clock rates ($5.0\text{ GHz}$) outpace off-chip DRAM latency ($>400\text{ cycles}$). Pointer chasing and non-contiguous matrix traversals cause CPUs to stall for up to $90\%$ of execution cycles waiting for memory buses.
3. **The Concurrency State Explosion**: Traditional lock-based and thread-interleaved shared state leads to an intractable state-space explosion ($\Omega = \frac{(TK)!}{(K!)^T}$), making multithreaded bug reproduction non-deterministic.

SNlang addresses these challenges directly in its type system and runtime via:
* **Deterministic Regional Memory**: Thread-local pinned lanes and lexical `region { }` blocks providing $\mathcal{O}(1)$ bulk rewinds with zero garbage collection pauses.
* **Cache-Oblivious Space-Filling Curves**: Morton $Z$-order and Hilbert geometry layouts providing optimal $\mathcal{O}(1 + N/B)$ spatial locality across arbitrary hardware cache hierarchies without manual tuning.
* **Join-Semilattices & Wave Concurrency**: Mathematically confluent shared memory (`lcell`) and deterministic observation windows (`wave { }`), guaranteeing identical execution outcomes regardless of thread scheduling.
* **Static Numeric Contracts**: Difference Bound Matrix (DBM) zone solving to eliminate array bounds checks and invariant violations at compile time.
* **Differential Dataflow**: Multiset arithmetic ($\mathbb{Z}$-sets) providing incremental computations in $\mathcal{O}(\|\Delta\|)$ time independent of collection volume.

---

## Chapter 1: Deterministic Regional Memory & Elastic Arenas

### 1.1 Architectural Model
Instead of forcing all dynamic allocations through a global heap allocator or garbage collector, SNlang partitions memory into three explicit tiers:
1. **Thread-Local Execution Lanes**: Contiguous, virtual-memory arenas dedicated to each execution core. Allocation is a 3-instruction bump pointer (`%ptr = %base + %size`), executing in $\approx 3\text{ ns}$.
2. **Lexical Region Scopes (`region { ... }`)**: Scoped execution boundaries that record the lane watermark on entry and automatically reset the bump pointer to the baseline on exit in $\mathcal{O}(1)$ time.
3. **Generational Global Pools & Heap**: Long-lived objects explicitly managed via generational handles or reference counting.

```
+-------------------------------------------------------------------------+
| Thread Lane 0 (Virtual Memory Bump Arena)                               |
| [ Base ] --------------> [ Watermark (Mark 1) ] -------> [ Limit ]      |
|                            |                                            |
|                            +--- region { ... } Allocations              |
|                            |                                            |
| [ Base ] <-------------- (Scope Exit: O(1) Instantaneous Rewind)        |
+-------------------------------------------------------------------------+
```

### 1.2 Syntax and Programming Interface
SNlang provides both a lexical scope keyword (`region`) and an object-oriented arena interface via `std.arena`:

```snlang
use std.arena

fn process_batch(list<int> input) -> int {
    int baseline = lane_used(0)

    // Lexical region block: all allocations inside are automatically
    // reclaimed when the block exits without calling free() or GC.
    region {
        ptr scratch_buffer = lane_alloc(0, 4096)
        // Perform heavy temporary calculations...
    } // High-water mark restored instantly to baseline in 1 store instruction!

    // Object-oriented arena interface:
    let ar = Arena.new(0)
    ptr mark = ar.save()
    ptr temp_block = ar.alloc(2048)
    
    // Explicit bulk restore:
    ar.restore(mark)
    return input.length()
}
```

### 1.3 Compile-Time Type Safety & Invariants
* **`E-GC-IN-LANE` (Memory Space Isolation)**: The compiler strictly forbids storing references to lane-allocated objects into GC-managed structures, and vice versa. Because lanes are reclaimed wholesale, cross-tier pointers would create dangling references.
* **`E0802` / `E0802c` (Affine Escape Analysis)**: The type checker performs interprocedural escape analysis. Returning a pointer allocated in a child region or passing a local lane reference to an outer scope is rejected at compile time.
* **Poison Invariant (`T-FIBR`)**: In debug and `-O0` builds (`--poison`), reclaimed lane segments are immediately overwritten with the poison byte `0xAA`. Any read-before-write or use-after-scope access immediately triggers an AddressSanitizer panic.

---

## Chapter 2: Cache-Oblivious Space-Filling Collections

### 2.1 The Memory Wall & Spatial Locality
Conventional 2D arrays and matrices are stored in row-major or column-major order. When performing stencils, transpositions, or multi-dimensional range queries, accessing neighboring elements along the non-contiguous axis incurs severe cache line thrashing.

SNlang introduces **Cache-Oblivious Space-Filling Collections**:
* **`morton list<T, dims=[N, M]>`**: 2D tensor storage indexed via Morton $Z$-order curves.
* **`hilbert map<K, V>`**: 2D spatial maps sorted along the continuous Hilbert fractal curve.

Because space-filling curves preserve spatial multi-dimensional proximity in 1D linear memory, memory access maintains high cache locality across L1, L2, L3, and TLB caches without requiring hardware-specific tile tuning.

```
Row-Major Storage:                 Morton Z-Order Storage:
(0,0) -> (0,1) -> (0,2)            (0,0) -> (0,1)
  |                                  |     /
  v                                  v    v
(1,0) -> (1,1) -> (1,2)            (1,0) -> (1,1)
(Jumping between rows traverses    (Neighbors stay together in
 entire memory cachelines!)         adjacent cacheline bytes!)
```

### 2.2 Syntax and Programming Interface

#### Morton Tensor Lists
```snlang
fn compute_stencils() {
    // Declares a 16x16 power-of-two Morton grid
    morton list<int, dims=[16, 16]> grid = morton_new(16, 16)

    // Element access by 2D coordinates:
    morton_set(grid, 4, 8, 100)
    int val = morton_get(grid, 4, 8)

    // Direct Morton curve arithmetic:
    int code = morton_index(4, 8) // Bit-interleaved 1D curve index
    int x = morton_x(code)        // Extract original X coordinate
    int y = morton_y(code)        // Extract original Y coordinate

    // Fast stride-1 curve scan:
    int direct_val = morton_scan_get(grid, code)
}
```

#### Hilbert Spatial Maps
```snlang
fn spatial_lookup() {
    // Declares a Hilbert map of order k=3 (grid side 2^3 = 8)
    hilbert map<int, int> spatial_index = hilbert_new(3)

    // Elements are inserted in Hilbert curve order:
    hilbert_insert(spatial_index, 2, 3, 500)
    hilbert_insert(spatial_index, 4, 4, 750)

    // Bounding Box Segment Advance:
    // Efficiently queries all entries within the rectangle (x0, y0) to (x1, y1)
    int cursor = hilbert_box_first(spatial_index, 1, 1, 5, 5)
    while (cursor != -1) {
        int payload = hilbert_entry_val(spatial_index, cursor)
        cursor = hilbert_box_next(spatial_index, 1, 1, 5, 5, cursor)
    }
}
```

### 2.3 Compile-Time Type Safety & Hardware Acceleration
* **`E-SFC` (Power-of-Two Dimension Invariant)**: Morton list dimensions must be equal powers of two (e.g., `dims=[8, 8]`, `dims=[64, 64]`). Non-square or non-power-of-two dimensions are rejected at compile time to ensure bi-directional bitwise interleave bijections.
* **`W-SFC-T5B` (Access Pattern Warning)**: The compiler detects naive row-major nested loops iterating over a `morton list` and emits a warning that row-major traversal wastes the space-filling cacheline guarantees.
* **Hardware Intrinsics Dispatch**: On x86_64 CPUs supporting BMI2, coordinate bit-interleaving automatically lowers to single-cycle hardware instructions (`PDEP` / `PEXT`). On ARM64 (Apple Silicon), it lowers to an optimized software shift-OR sequence.

---

## Chapter 3: Deterministic Concurrency & Monotonic Semilattices

### 3.1 The Concurrency Confluence Model
Multithreaded programming typically suffers from non-deterministic interleavings: thread execution speeds dictate whether a race condition occurs.

SNlang solves this by implementing **Theorem WS (Wave-Seam Composition)**:
1. **Lattice Cells (`lcell`)**: Shared mutable state must be constrained to a **Join-Semilattice** $\langle S, \sqcup, \sqsubseteq \rangle$. Updates are strictly monotonic: values can only inflate ($\sqcup$), never overwrite or retract.
2. **Deterministic Wave Blocks (`wave { ... }`)**: Code blocks that operate over a frozen snapshot $\kappa$ of shared seam cells. Staged contributions (`contrib`) are folded deterministically into shared state at the wave boundary, producing bit-identical output regardless of thread arrival order.

```
Thread A: contrib(Cell, 20) --\
                               +--> Wave Join J(W): Cell = (OldVal ⊔ 20 ⊔ 15)
Thread B: contrib(Cell, 15) --/    (Result is IDENTICAL under any thread schedule!)
```

### 3.2 Supported Lattice Carriers
Lattice cells are created with `lcell_new(carrier_kind)`:
* **Carrier 0 (High-Water Mark / Max)**: Join operation is $\max(a, b)$.
* **Carrier 1 (Grow-Only Set / G-Set)**: Join operation is set union $A \cup B$.
* **Carrier 2 (Last-Write-Wins / LWW)**: Join operation is timestamped version tie-breaking.

### 3.3 Syntax and Programming Interface
```snlang
// Seam cells defined at module scope
ptr peak_latency = lcell_new(0)  // Max register (⊔ = max)
ptr active_nodes = lcell_new(1)  // Grow-only set (⊔ = union)

fn main() {
    // Initial sequential setup
    lcell_inflate(peak_latency, 50, 0)

    // Wave region: guaranteed race-free observation and contribution
    wave {
        // Read frozen snapshot value:
        int base = read_W(peak_latency)

        // Stage contributions to seam cells:
        contrib(active_nodes, 101, 0)
        contrib(active_nodes, 102, 0)
        contrib(peak_latency, base + 25, 0)
    } // Wave join point J(W): all staged contributions fold atomically!

    // Post-join observation:
    int final_peak = read_J(peak_latency) // Evaluates to 75
    int total_nodes = lcell_len(active_nodes)
    bool has_node = lcell_has(active_nodes, 101)
}
```

### 3.4 Compile-Time Invariants & Diagnostics
* **`E0812` (Purity Kill-Switch)**: Code executed inside `wave { }` must be a pure mathematical function of the captured state. The compiler rejects wall-clock reads (`time_now_ms`), environment lookups, channel receives, and locks inside wave blocks via static interprocedural analysis.
* **`E-R1` (Quiescent Capture Violation)**: Modifying a seam cell concurrently during an active wave is rejected. Calling `read_W` outside a wave or `read_J` before a join point triggers a compile-time refusal.
* **`E-LRET` (Monotonicity Violation)**: Join-semilattices do not admit negation or decrement. Attempting to retract or clear an `lcell` is forbidden.

---

## Chapter 4: Contract-Based Static Verification

### 4.1 Compile-Time Invariant Proving
SNlang incorporates a formal contract verification layer built on **Difference Bound Matrix (DBM)** zone analysis. Functions can specify formal preconditions (`requires`) and postconditions (`ensures`).

The compiler classifies contract expressions into three mathematical tiers:
1. **Tier L (Linear Invariants)**: Integer relations of the form $x - y \le c$. These are fully proved and eliminated at compile time via $\mathcal{O}(\|V\|^3)$ Floyd-Warshall closure. Proved contracts incur **zero CPU cycles at runtime**.
2. **Tier D (Divisibility Invariants)**: Modulo operations with known positive bounds ($x \pmod m < m$), discharged via remainder-range axioms.
3. **Tier N (Non-Linear Terms)**: Variable-by-variable multiplications ($x \cdot y$) and variable divisions.

```
+-------------------------------------------------------------------------+
| Compile-Time Contract Engine                                            |
|                                                                         |
| requires(0 <= index < list.length())                                    |
|              |                                                          |
|              +---> Tier L (DBM Zone Solver)                             |
|                    Proves index is within bounds statically             |
|                    ==> ZERO RUNTIME OVERHEAD (Check Eliminated!)        |
+-------------------------------------------------------------------------+
```

### 4.2 Syntax and Programming Interface
```snlang
// Array bounds check discharged entirely at compile time
fn safe_lookup(list<int> xs, int index) -> int requires(0 <= index < xs.length()) {
    return xs[index]
}

// Precondition and postcondition with contextual return-value binding (`result`)
fn clamp_percentage(int val) -> int 
    requires(val >= 0) 
    ensures(0 <= result <= 100) 
{
    if (val > 100) {
        return 100
    }
    return val
}

// Verified functions: Tier N non-linear expressions are hard compile errors
verified fn strictly_positive_add(int a, int b) -> int 
    requires(a > 0 and b > 0) 
    ensures(result > 0) 
{
    return a + b
}
```

### 4.3 Safe Degradation & Diagnostics
* **Discharged Invariant**: When the DBM solver proves the condition holds under all execution paths, compilation is completely silent and no runtime instruction is emitted.
* **`W-CONTRACT` & `[E-0702]` (Typed Runtime Assertion)**: If an invariant cannot be proved at compile time in a standard `fn`, the compiler emits a warning and automatically degrades the condition to an optimized runtime assertion (`sn_contract_assert`).
* **`E0701` (Strict Non-Linear Refusal)**: In a `verified fn`, non-linear expressions that cannot be solved at compile time produce a hard compiler error.
* **Refuted Contracts**: If the solver proves an invariant is mathematically impossible (e.g., `requires(x > 10 and x < 5)`), the compiler reports `REFUTED` at compile time.

---

## Chapter 5: State Chronology & Differential Dataflow

### 5.1 Incremental Computing with $\mathbb{Z}$-Sets
Traditional systems recalculate results from scratch when an input changes. SNlang incorporates the principles of Differential Dataflow via **$\mathbb{Z}$-sets** (multisets mapping keys to positive or negative integer multiplicities $\mathbb{Z}[X]$):
$$\mathcal{Q}(A + \Delta A) = \mathcal{Q}(A) + \mathcal{D}\mathcal{Q}(A, \Delta A)$$
By applying discrete derivatives, updates propagate in $\mathcal{O}(\|\Delta A\|)$ time rather than re-evaluating the entire dataset.

### 5.2 Deterministic Execution Tapes
The `tape<T>` type provides an append-only event-sourcing ledger that records state changes as:
* **Delta Records**: Incremental differences between previous and next states ($\Delta = B \ominus A$).
* **Checkpoint Records**: Periodic full snapshots from which the state can be deterministically re-integrated.
* **Opaque Records**: Unstructured audit records.

### 5.3 Syntax and Programming Interface
```snlang
fn state_ledger_demo() {
    // 1. Create differential Z-set
    ptr state_v0 = zset_new(0)
    ptr state_v1 = zset_new(0)

    zset_add(state_v1, 1001, 5) // Key 1001 added with multiplicity +5
    zset_add(state_v1, 1002, 2) // Key 1002 added with multiplicity +2
    zset_remove(state_v1, 1001, 1) // Key 1001 reduced: multiplicity is now 4

    // 2. Initialize append-only execution tape
    tape<int> audit_tape = tape_new("transactions")

    // Record delta and checkpoint
    tape_record(audit_tape, 1, state_v0, state_v1) // DELTA record
    tape_record(audit_tape, 2, state_v1)           // CHECKPOINT record

    // 3. Deterministic replay: reconstructs state by folding deltas from checkpoint
    ptr reconstructed = zset_new(0)
    tape_replay(audit_tape, reconstructed)
    int count = zset_count(reconstructed, 1001) // Evaluates to 4
}

// Fixed-point calculation with guaranteed termination
fn calculate_reachability(ptr graph_edges, ptr initial_seeds) {
    ptr fixpoint = zfix_new(graph_edges, initial_seeds)
    int rounds = zfix_run(fixpoint) // Semi-naïve fixpoint evaluation
    bool reachable = zfix_has(fixpoint, 42)
}
```

### 5.4 Type Safety & Invariants
* **`[E-ZADM]` (Monotonicity Invariant)**: Rejects fixpoint definitions whose state updates are non-monotonic or oscillatory, guaranteeing Knaster-Tarski termination.
* **`[E-TAPE]`**:
  - Rejects mixing different carrier algebras on a single tape (*one tape, one ledger*).
  - Rejects replaying a delta-chain without a base checkpoint snapshot.
* **`E-TAPE-WAVE`**: Tapes cannot be recorded inside `wave { }` blocks to preserve wave purity.

---

## Chapter 6: Relocatable Links & Generational Slot Pools

### 6.1 Self-Relative Relocatable Links (`link<T, u16>`)
When transferring complex pointer-linked graph structures across networks, between processes, or into shared memory, absolute pointers require expensive pointer-swizzling and serialization fixups.

SNlang provides **Self-Relative Links (`link<T, u16>`)**:
* Instead of storing absolute 64-bit virtual memory addresses, the link stores a 16-bit relative byte displacement $\delta = \text{target} - \text{field\_address}$.
* A memory buffer containing self-relative links can be copied or relocated in memory using raw `memcpy` without updating any pointer fields.

```snlang
blueprint GraphNode {
    int value
    link<GraphNode, u16> next // 2-byte relative displacement
}
```

* **`E0801` (Copy Discipline)**: By-value assignment of structures containing self-relative links (`a = b`) is rejected by the compiler. Relocation must use the verified `relocate_range` primitive to maintain link integrity.

### 6.2 Generational Slot Pools (`std.pool`)
To prevent use-after-free in dynamic object pools without a garbage collector, `std.pool` pairs slot indices with generational counters:

```snlang
use std.pool

fn pool_example() {
    let p = Pool.new(16)
    int handle = p.insert(99)

    // Free the slot:
    p.remove(handle)

    // Stale handle check: returns false because the slot generation incremented!
    bool is_alive = p.is_valid(handle)
}
```

* **`E0803` (Global Pool Discipline)**:
  - `pool<T>`: Read-only outside an `unfrozen (P) { ... }` block.
  - `pool<T> thread_local`: Confined to the declaring thread; thread transfer is rejected with `E0803.1`.
  - `pool<T> synchronized`: Thread-safe mutex-protected pool.

---

## Compiler Diagnostic Code Reference

| Error Code | Category | Cause and Remedy |
| :--- | :--- | :--- |
| **`E-GC-IN-LANE`** | Memory Safety | Attempted to store a lane reference into a GC-managed object or vice versa. Keep memory tiers isolated. |
| **`E0802` / `E0802c`** | Lifetime Escape | A local lane reference escaped its declaring region scope or exceeded lane capacity. |
| **`E-SFC`** | Geometry Invariant | Morton list dimensions are not equal powers of two (must be square $2^k \times 2^k$). |
| **`E-SFC-ORDER`** | Order Refusal | Attempted out-of-order insertion into a Hilbert map. Inserts must follow Hilbert order. |
| **`W-SFC-T5B`** | Performance Warning | Row-major nested loop detected over Morton storage. Traverse in curve order to maintain cache locality. |
| **`E0812`** | Concurrency Purity | Impure operation (IO, clock read, channel receive, lock) detected inside a `wave { }` block. |
| **`E-R1`** | Quiescent Seam | Seam cell was modified while a wave was active, or `read_J` was called before the join. |
| **`E-LRET`** | Lattice Monotonicity | Attempted to retract or delete from a join-semilattice cell. Lattices are grow-only. |
| **`E0701`** | Static Contract Error | Non-linear arithmetic term ($x \cdot y$) found in a `verified fn` that cannot be proved at compile time. |
| **`[E-0702]`** | Runtime Assertion | An unprovable contract in a standard `fn` failed its runtime assertion check. |
| **`E-ZADM`** | Fixpoint Termination | Non-monotonic or oscillatory recursion rejected in a `zfix` loop. |
| **`E-TAPE`** | State Ledger Refusal | Attempted to mix carrier tiers or replay a delta stream without a base checkpoint. |
| **`E0801`** | Relocation Violation | Direct by-value copy of a `link<T, u16>` structure rejected. Use `relocate_range`. |
| **`E0803`** | Pool Concurrency | Un-synchronized cross-thread capture of an unfrozen pool object rejected. |
