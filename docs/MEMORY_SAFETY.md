# SNlang Memory Safety & Tiered Execution Architecture
## Formal Memory Spaces, Regional Arenas, Affine Escape Analysis, and Zero-GC Lifetimes

**Edition:** SNlang 0.3.1  
**Target Audience:** Systems Software Engineers, Performance Architects, and Runtime Developers  

---

## Table of Contents
1. [Executive Overview & The Multi-Tier Model](#1-executive-overview--the-multi-tier-model)
2. [Memory Tier Hierarchy & Physical Layout](#2-memory-tier-hierarchy--physical-layout)
3. [Lexical Region Scopes (`region { ... }`)](#3-lexical-region-scopes-region--)
4. [Affine Escape Analysis & The Outlives Relation](#4-affine-escape-analysis--the-outlives-relation)
5. [Memory Space Isolation (`E-GC-IN-LANE`)](#5-memory-space-isolation-e-gc-in-lane)
6. [Poison Safety & Quarantine Guards (`T-FIBR`)](#6-poison-safety--quarantine-guards-t-fibr)
7. [Structured Concurrency & Nursery Lane Carving](#7-structured-concurrency--nursery-lane-carving)
8. [Generational Slot Pools (`std.pool`)](#8-generational-slot-pools-stdpool)
9. [Production Systems Patterns & Recipes](#9-production-systems-patterns--recipes)
10. [Memory Safety Diagnostic Code Reference](#10-memory-safety-diagnostic-code-reference)

---

## 1. Executive Overview & The Multi-Tier Model

Modern systems programming languages typically force developers into an extreme architectural compromise:
* **Manual Pointer Management (C, C++, Zig)**: Delivers raw execution speed and predictable memory footprints, but burdens developers with manual memory tracking, use-after-free (UAF) vulnerabilities, double-free crashes, and memory leaks.
* **Global Garbage Collection (Go, Java, C#)**: Provides safety against memory corruption, but incurs unpredictable stop-the-world (STW) pauses, memory footprint bloat (often 2x–3x active working set), and cache-unfriendly pointer-heavy heap structures.
* **Complex Compile-Time Borrow Checking (Rust)**: Guarantees memory safety without a GC, but imposes strict lexical lifetime constraints, complex lifetime annotations (`'a`, `'b`), and severe friction when modelling cyclical graphs, arena-allocated ASTs, or concurrent task pipelines.

**SNlang introduces a Tiered Memory Architecture**. Instead of treating all allocated memory as a single homogeneous heap, SNlang segregates memory into four distinct mathematical and physical tiers:

```
+-----------------------------------------------------------------------------------+
|                                 SNlang Memory Model                               |
+-----------------------------------------------------------------------------------+
|  Tier 0: The Execution Stack                                                      |
|  - Automatic call-frame unwinding. Scalar types, primitives, and local registers. |
+-----------------------------------------------------------------------------------+
|  Tier 1: Pinned Thread Lanes (Virtual Memory Bump Arenas)                         |
|  - Dedicated 128 MB virtual arenas per thread core (Lanes L0 - L7).               |
|  - 3-instruction bump allocator (%ptr = %base + %size, ~3 ns).                    |
|  - Scoped rewinds via lexical `region { }` blocks in O(1) time (single store).    |
|  - Completely invisible to the Garbage Collector (Zero GC Scanning Overhead).    |
+-----------------------------------------------------------------------------------+
|  Tier 2: Generational Object Pools (`std.pool`)                                   |
|  - Dense slot vaults indexed by 64-bit generation-tagged handles (`handle<T>`).   |
|  - O(1) reuse with complete ABA and use-after-free immunity.                      |
|  - Ideal for graph topologies, entities, and long-lived dynamic structures.       |
+-----------------------------------------------------------------------------------+
|  Tier 3: Traced Reference-Counted / Managed Heap Space                            |
|  - Automatic Reference Counting (ARC) with precise cycle collection.              |
|  - Strict compile-time isolation barrier preventing cross-tier corruption.        |
+-----------------------------------------------------------------------------------+
```

By enforcing strict **compile-time memory space isolation** and **affine escape analysis**, SNlang guarantees memory safety, deterministic sub-microsecond latency, and zero GC pauses in high-throughput systems paths.

---

## 2. Memory Tier Hierarchy & Physical Layout

### 2.1 The Four Memory Tiers

| Tier | Name | Storage Mechanism | Allocation Cost | Deallocation Cost | Best For |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **0** | **Stack** | CPU Stack Pointer (`RSP`/`SP`) | $\mathcal{O}(1)$ (1 instruction) | $\mathcal{O}(1)$ (Frame pop) | Local scalars, parameters, return values |
| **1** | **Pinned Lanes** | Contiguous virtual memory arena | $\mathcal{O}(1)$ (3 instructions, ~3 ns) | $\mathcal{O}(1)$ (Instant watermark reset) | Scratchpads, network buffers, batch loops, ASTs |
| **2** | **Generational Pools** | Dense indexed slot vault | $\mathcal{O}(1)$ (Free-list pop) | $\mathcal{O}(1)$ (Slot generation increment) | Graphs, game entities, caches, persistent records |
| **3** | **Managed Heap** | Reference-counted heap with cycle GC | $\mathcal{O}(1)$ (Allocator malloc) | Amortized (ARC decrement / Cycle sweep) | High-level strings, polymorphic trees, user payloads |

### 2.2 Physical Anatomy of an Execution Lane

Each execution thread in SNlang possesses an array of dedicated execution lanes (Lanes `0` through `7`). Lane `0` is the default thread-local scratch arena; Lanes `1` through `7` are reserved for specialized subsystems, pinned data structures, and child task nursery carving.

A lane consists of a reserved virtual memory range with three physical boundaries:

```
Virtual Memory Window (e.g. 128 MB Reservation)
+-----------------------------------------------------------------------------------+
| [ Base ] ==================> [ Watermark ] ----------------------> [ Limit ]      |
+-----------------------------------------------------------------------------------+
  ^                             ^                                     ^
  |                             |                                     |
  Start of Lane Memory          Current Allocation Frontier           End of Reserved Space
  (Fixed virtual address)       (Bumped forward on every alloc)       (Guard page boundary)
```

1. **`Base`**: The base virtual address of the lane mmap reservation.
2. **`Watermark` (High-Water Mark)**: A 64-bit pointer pointing to the first unallocated byte. Allocation is simply:
   $$\text{Watermark}_{\text{new}} = \text{Watermark}_{\text{old}} + \text{AlignUp}(\text{RequestedSize}, 8)$$
3. **`Limit`**: The upper boundary of the allocated virtual address space. Exceeding `Limit` triggers an out-of-memory trap (`E0802`).

### 2.3 Cacheline Alignment Discipline
All lane allocations enforce an 8-byte minimum alignment boundary. For vector and SIMD structures (such as space-filling curve grids or cryptographic state), lane allocators align to 64-byte hardware cacheline boundaries to eliminate unaligned memory penalties and false sharing across cores.

---

## 3. Lexical Region Scopes (`region { ... }`)

### 3.1 Mechanics of $O(1)$ Loop Rewinds

In conventional heap allocators, allocating thousands of temporary objects in a tight loop incurs severe malloc metadata bookkeeping and memory fragmentation.

In SNlang, temporary computation is enclosed in a lexical `region { ... }` block:

```snlang
use std.arena

fn process_telemetry_batch(list<int> packets) -> int {
    int baseline = lane_used(0)
    int valid_count = 0

    // Lexical region block:
    region {
        // High-water mark of Lane 0 is captured at entry: Mark = Watermark
        ptr scratch_buffer = lane_alloc(0, 4096)
        
        // Multiple temporary allocations take 3 ns each:
        ptr header = lane_alloc(0, 128)
        ptr payload = lane_alloc(0, 2048)

        // Process packets using scratch memory...
        if (ptr_load(header, 0) == 0xAA) {
            valid_count += 1
        }
        
    } // SCOPE EXIT: Watermark is restored to baseline in ONE STORE instruction!
      // Total deallocation cost: 0 ns overhead, 0 GC cycles.

    assert(lane_used(0) == baseline)
    return valid_count
}
```

```
Region Execution Timeline:
1. Entry:           Capture baseline:  Watermark_0 = Watermark
2. Allocation:      Bump forward:      Watermark_1 = Watermark_0 + Size
3. Loop Execution:  Repeated bumps:    Watermark_N = Watermark_N-1 + Size
4. Exit:            Bulk Restore:      Watermark = Watermark_0 (1 store!)
```

### 3.2 High-Throughput Stream Processing Loops

A common trap in modern web and systems software is accumulating temporary buffers inside a loop, causing GC pressure or allocator heap fragmentation. In SNlang, placing the `region` block inside the loop body guarantees zero memory growth:

```snlang
fn process_infinite_stream(int socket_fd) {
    while (true) {
        // Each packet gets a clean, zero-cost scratch arena
        region {
            ptr raw_frame = lane_alloc(0, 65536) // 64 KB frame
            int bytes_read = socket_read(socket_fd, raw_frame, 65536)
            if (bytes_read <= 0) {
                stop // Exits loop cleanly, region is unwound automatically
            }
            parse_and_dispatch(raw_frame, bytes_read)
        } // The 64 KB is reclaimed instantaneously here; memory usage is CONSTANT!
    }
}
```

### 3.3 The Object-Oriented Arena API (`std.arena`)

For non-lexical lifetimes or fine-grained manual checkpoints, the `std.arena` module exposes the `Arena` blueprint:

```snlang
use std.arena

fn manual_arena_control() {
    let ar = Arena.new(0) // Bind to thread lane 0
    
    // Save current high-water mark
    ptr checkpoint = ar.save()
    
    ptr block_a = ar.alloc(1024)
    ptr block_b = ar.alloc(2048)
    int current_footprint = ar.used()
    
    // Roll back to checkpoint:
    ar.restore(checkpoint)
    
    // Memory is freed immediately; the next allocation reuses the exact same memory!
    ptr block_c = ar.alloc(512)
    assert(ptr_int(block_c) == ptr_int(block_a))
}
```

---

## 4. Affine Escape Analysis & The Outlives Relation

Because memory allocated within a `region` block is wiped instantly upon block exit, allowing any pointer or reference derived from that region to survive past the closing brace would create a catastrophic **use-after-free** (dangling pointer) vulnerability.

SNlang prevents this at compile time using **Affine Escape Analysis**.

### 4.1 The Outlives Scope Lattice

The compiler models scopes as an ordered mathematical lattice of lifetimes ($\prec$ denotes "is strictly contained within" or "outlived by"):

$$\text{Local Region}_{\text{inner}} \prec \text{Local Region}_{\text{outer}} \prec \text{Function Scope} \prec \text{Thread Lane} \prec \text{Heap} \prec \text{Global}$$

```
                [ Global / Static ]
                         ^
                         |
                   [ Managed Heap ]
                         ^
                         |
                [ Thread Lane L0-L7 ]
                         ^
                         |
               [ Function Call Frame ]
                         ^
                         |
           [ Outer Region { ... } ]
                         ^
                         |
           [ Inner Region { ... } ]
```

**Fundamental Safety Invariant**: A reference or pointer allocated at scope $S_{\text{alloc}}$ may **only** be stored into a destination variable at scope $S_{\text{dest}}$ if $S_{\text{alloc}}$ outlives or equals $S_{\text{dest}}$ ($S_{\text{dest}} \preceq S_{\text{alloc}}$).

### 4.2 Escape Diagnostics & Concrete Rules

#### Rule 1: Region-Allocated Pointers Cannot Be Returned (`E0802` / `E-ESCAPE`)
Returning a pointer allocated in a local region to an outer calling scope is strictly forbidden.

```snlang
// FORBIDDEN: Compiler refuses with E0802
fn illegal_escape() -> ptr {
    ptr escaped_ptr = none
    region {
        ptr temp = lane_alloc(0, 128)
        escaped_ptr = temp // ERROR E0802: target outlives declaring region scope
    }
    return escaped_ptr // Region is dead; returning this would cause a wild crash!
}

// ALLOWED: Compute the scalar result inside the region and return the value:
fn legal_regional_computation() -> int {
    int final_result = 0
    region {
        ptr temp = lane_alloc(0, 128)
        ptr_store(temp, 0, 42)
        final_result = ptr_load(temp, 0) // Copy scalar value
    } // Memory reclaimed, but integer scalar safely copied to caller!
    return final_result
}
```

#### Rule 2: Outliving Structures Cannot Hold Scoped Pointers (`E0802c`)
Assigning a region-scoped pointer into a struct or blueprint defined in an outer scope is statically rejected.

```snlang
blueprint PacketHeader {
    int packet_id
    ptr payload_ref
}

fn illegal_field_assignment() {
    new PacketHeader pkt() // Outer scope
    
    region {
        ptr scratch = lane_alloc(0, 256)
        // ERROR E0802c: cannot store pointer from inner region into outer struct field
        pkt.payload_ref = scratch 
    }
}
```

---

## 5. Memory Space Isolation (`E-GC-IN-LANE`)

A central pillar of the SNlang runtime is **Memory Space Isolation**. Pinned thread lanes and the managed garbage-collected heap operate under completely distinct physical invariants.

```
+------------------------------------+          +------------------------------------+
|       Thread Lane Space            |          |       Managed Heap Space           |
|------------------------------------|          |------------------------------------|
| - Bump allocated (%base + %size)   |          | - ARC + Cycle Garbage Collector   |
| - Bulk wiped on region exit        |          | - Individual object tracing        |
| - Zero GC tracking overhead        |          | - Dynamic graph allocations        |
+------------------------------------+          +------------------------------------+
                  \                                /
                   \      BARRIER: E-GC-IN-LANE   /
                    \                            /
                     x <======================> x
                      NO CROSS-TIER REFERENCES!
```

### 5.1 Why the Isolation Barrier is Absolute

1. **Lanes $\to$ Heap**: If an object inside a thread lane were allowed to hold a reference to a garbage-collected object, the GC tracer would have to scan every thread lane in virtual memory to find root references. This would destroy SNlang's zero-GC pause guarantee for lanes.
2. **Heap $\to$ Lanes**: If a managed heap object were allowed to hold a raw pointer into a thread lane, when that thread lane performs a `region` rewind, the heap object would hold a wild dangling pointer into dead virtual memory, causing memory corruption.

### 5.2 Compiler Enforcement (`E-GC-IN-LANE`)

The compiler strictly verifies at compile time that lane-allocated structures (such as pinned lists or lane buffers) never cross into the garbage-collected domain:

```snlang
fn illegal_cross_tier_capture() {
    list<str> managed_strings = ["apple", "banana"] // Managed heap list
    
    region {
        // Pinned list allocated on Lane 2:
        let pinned_acc = sn_list_new_lane(0, 2)
        
        // ERROR E-GC-IN-LANE: Attempted to store managed GC reference into pinned lane
        pinned_acc.push(managed_strings[0])
    }
}
```

**The Runtime Half of the Contract**:
In the SNlang runtime (`compiler/runtime.c`), pinned lane objects are never registered with `sn_gc_track`. Throughout the entire lifecycle of a pinned lane, `sn_gc_object_count()` remains unchanged, guaranteeing that the garbage collector incurs zero overhead from lane usage.

---

## 6. Poison Safety & Quarantine Guards (`T-FIBR`)

In high-performance systems programming, a subtle bug occurs when code inadvertently reads uninitialized memory or continues reading stale data from an unwound buffer after a scope exit.

SNlang incorporates **Compiler Poison Guards (`T-FIBR`)**.

### 6.1 The 0xAA Poison Invariant

When code is compiled in debug mode (`--poison` or `-O0`), the compiler and runtime enforce active memory poisoning:

1. **Inline Allocation Poisoning**: Every bump allocation on a thread lane immediately poisons the allocated range with the byte `0xAA` (`170` decimal).
2. **Region Rewind Re-poisoning**: When a `region { ... }` block unwinds, the restored virtual address range $[ \text{Watermark}_{\text{new}}, \text{Watermark}_{\text{old}} )$ is actively overwritten with `0xAA`.

```
Memory Before Alloc:   [ 00 00 00 00 00 00 00 00 ] (Fresh zeroed virtual page)
After lane_alloc:      [ AA AA AA AA AA AA AA AA ] (Poisoned with 0xAA)
User writes value:     [ 05 00 00 00 00 00 00 00 ] (Valid data: integer 5)
On Region Rewind:      [ AA AA AA AA AA AA AA AA ] (Instantly re-poisoned to 0xAA!)
```

### 6.2 AddressSanitizer (ASan) Integration

In CI and test builds, SNlang compiles directly with LLVM's `-fsanitize=address`. Because the compiler marks unwound lane ranges as poisoned in the ASan shadow memory:
* Any **Read-Before-Write** on fresh allocations immediately crashes with an AddressSanitizer report.
* Any **Use-After-Scope** access across a region boundary immediately triggers a deterministic SIGSEGV / ASan trap.
* Stale data cannot silently corrupt downstream calculations.

---

## 7. Structured Concurrency & Nursery Lane Carving

Concurrency in SNlang combines the simplicity of Go-style green tasks with strict compile-time race immunity and deterministic thread-local memory.

### 7.1 The Nursery Scope (`nursery { ... }`)

Orphaned background threads are a primary source of memory leaks and silent crashes in production. In SNlang, concurrent tasks are managed through structured **Task Nurseries**:

```snlang
fn parallel_batch_processor() {
    new AtomicCounter total()
    
    // Structured Concurrency Nursery Scope:
    nursery {
        spawn worker_task(1, total)
        spawn worker_task(2, total)
        spawn worker_task(3, total)
    } // CRITICAL: The nursery block blocks until ALL spawned children complete!
      // No thread can leak beyond this closing brace.
}
```

### 7.2 Child-Lane Carving (`SnLane.parent` & Loaned Lanes)

When a child task is spawned inside a nursery, assigning each child a global heap allocator would cause severe atomic contention on the OS allocator locks.

Instead, SNlang uses **Nursery Child-Lane Carving**:
1. When entering a nursery, the runtime activates a dedicated child-lane registry ($\nu$-lanes).
2. Each child task is loaned a dedicated execution lane carved from the parent thread's lane pool (Lanes 1 through 7).
3. The child runs completely lock-free, bump-allocating on its own isolated lane without touching the parent's memory.
4. **$O(1)$ Join Reclaim**: When `nursery` joins all child tasks, the parent executes a 1-store reset per loaned lane, instantaneously reclaiming all child scratch memory in parallel without traversing the allocations!

```
Parent Thread (Lane 0)
   |
   +--- nursery {
          |
          +---> spawn Child 1: Loaned Lane 1 (Isolated 128 MB Arena)
          +---> spawn Child 2: Loaned Lane 2 (Isolated 128 MB Arena)
          +---> spawn Child 3: Loaned Lane 3 (Isolated 128 MB Arena)
          |
        } Join Point: All loaned lanes (1, 2, 3) wiped simultaneously in O(1)!
```

---

## 8. Generational Slot Pools (`std.pool`)

While thread lanes excel at short-lived, scratchpad, and hierarchical workloads, dynamic systems software frequently requires complex graph networks, entity-component systems (ECS), or long-lived caches where objects are created and destroyed in arbitrary order.

If managed with raw pointers, these structures risk **Use-After-Free (UAF)** and **ABA problems**.

SNlang provides **Generational Slot Pools (`std.pool`)**.

### 8.1 64-Bit Generation-Tagged Handles (`handle<T>`)

Instead of handing raw virtual memory addresses to user code, `std.pool` stores elements in a dense array and returns opaque 64-bit integer handles:

```
+-------------------------------------------------------------------+
|               64-Bit Generational Handle Structure                |
+-----------------------------------+-------------------------------+
|  Upper 32 Bits: Generation Tag    |  Lower 32 Bits: Slot Index    |
|  (Incremented on every free)      |  (Direct offset into array)   |
+-----------------------------------+-------------------------------+
```

```snlang
use std.pool

fn generational_safety_demo() {
    // Initialize pool with initial capacity of 64 slots
    let p = Pool.new(64)
    
    // Allocate slot: returns 64-bit handle (Slot Index 0, Generation 1)
    int h1 = p.insert(100)
    print("Value: " + cast(p.get(h1), str)) // Outputs: 100
    
    // Deallocate the item:
    p.remove(h1) // Slot 0 is placed on the free list; Generation increments to 2!
    
    // 1. Stale Handle Check:
    if (!p.is_valid(h1)) {
        print("Handle h1 is dead; access prevented!")
    }
    
    // 2. ABA Prevention:
    // A new item reuses slot 0, but gets Generation 2:
    int h2 = p.insert(999) // Returns (Slot Index 0, Generation 2)
    
    // Accessing with the stale handle h1 is mathematically rejected:
    // p.get(h1) -> Trapped! (Generation 1 != Generation 2)
    assert(p.get(h2) == 999)
}
```

### 8.2 Concurrency Discipline for Pools (`E0803`)

The SNlang type checker strictly enforces pool access patterns:
* **`pool<T> thread_local`**: Bound exclusively to the declaring thread. Transferring a thread-local pool across a `spawn` boundary is rejected at compile time with `E0803.1`.
* **`pool<T> synchronized`**: Thread-safe pool wrapped in atomic synchronization primitives for multi-threaded work stealing.
* **Unfrozen Scopes (`unfrozen (P) { ... }`)**: Guarantees atomic batch updates across multiple pool items without intermediate state visibility.

---

## 9. Production Systems Patterns & Recipes

### Recipe 9.1: Zero-Allocation High-Throughput Packet Processor

This recipe processes incoming network frames at line rate ($10\text{ Gbps}+$) with zero memory allocations on the heap and zero GC pauses.

```snlang
use std.arena

blueprint PacketSummary {
    int packet_id
    int payload_size
    int checksum
}

fn process_network_stream(int socket_fd) {
    int baseline = lane_used(0)

    while (true) {
        // Enclose each packet frame in an O(1) region
        region {
            // Allocate 8 KB frame buffer directly on Lane 0 (~3 ns)
            ptr frame = lane_alloc(0, 8192)
            
            int n = socket_recv(socket_fd, frame, 8192)
            if (n <= 0) {
                stop
            }
            
            // Extract header and verify checksum using scratch buffers
            int pkt_id = ptr_load(frame, 0)
            int data_len = ptr_load(frame, 8)
            
            // Allocate scratch parsing buffer:
            ptr scratch = lane_alloc(0, 1024)
            int computed_crc = compute_fast_crc(frame + 16, data_len, scratch)
            
            if (computed_crc == ptr_load(frame, 12)) {
                // Forward packet payload...
                dispatch_packet(pkt_id, frame + 16, data_len)
            }
            
        } // Bulk O(1) rewind: frame + scratch discarded instantly!
    }

    assert(lane_used(0) == baseline)
}
```

### Recipe 9.2: Recursive AST / Graph Traversal with Regional Wipe

Building intermediate syntax trees during compiler passes typically causes severe heap churn. By executing each compilation pass inside a region, millions of AST nodes are wiped with a single store instruction.

```snlang
use std.arena

blueprint ASTNode {
    int kind
    int value
    ptr left_child
    ptr right_child
}

fn compile_expression(str source_code) -> int {
    int optimized_constant = 0

    // All AST construction happens inside the regional arena:
    region {
        let ar = Arena.new(0)
        
        ptr root = ar.alloc(32) // sizeof(ASTNode)
        ptr left = ar.alloc(32)
        ptr right = ar.alloc(32)
        
        // Initialize tree nodes...
        ptr_store(root, 0, 1) // OpAdd
        ptr_store(root, 16, ptr_int(left))
        ptr_store(root, 24, ptr_int(right))
        
        // Evaluate constant folding:
        optimized_constant = evaluate_ast_node(root)
    } // The entire 3-node tree (and all scratch parser memory) is wiped in 0 ns!

    return optimized_constant
}
```

---

## 10. Memory Safety Diagnostic Code Reference

| Error Code | Category | Root Cause | Remediation Strategy |
| :--- | :--- | :--- | :--- |
| **`E-GC-IN-LANE`** | Space Isolation | Attempted to store a managed garbage-collected reference into a pinned lane or vice versa. | Keep memory tiers isolated. Copy data by value or convert managed payloads into raw lane bytes. |
| **`E0802` / `E-ESCAPE`** | Lifetime Escape | A pointer or reference allocated in an inner `region` was returned to an outer scope. | Return calculated scalar values or deep-copy the payload to the caller's target buffer before region exit. |
| **`E0802c`** | Lifetime Escape | A region-scoped pointer was assigned to a field of a struct that outlives the region. | Ensure the target struct is declared inside the same region, or store a generation handle / scalar index. |
| **`E0803.1`** | Concurrency Violation | A `thread_local` pool or arena handle was captured across a `spawn` task boundary. | Use a `pool<T> synchronized` pool or channel message passing (`chan<T>`) to transfer ownership safely. |
| **`E0803.2`** | Concurrency Violation | An unfrozen pool was read concurrently without an active `unfrozen` guard block. | Enclose concurrent updates inside an `unfrozen (P) { ... }` block to ensure atomic batch visibility. |
| **`T-FIBR`** | Sanitizer Trap | Memory read from an unwritten or unwound region buffer in `--poison` mode. | Initialize all lane allocations before reading, and never access pointers past their declaring region scope. |
| **`E0801`** | Relocation Violation | A structure containing a self-relative link (`link<T, u16>`) was copied by value (`a = b`). | Relocate buffers containing self-relative links using the verified `relocate_range` runtime intrinsic. |
