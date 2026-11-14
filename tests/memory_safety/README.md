# Memory Safety Exploit Test Suite: Rust vs SNlang

This directory contains a comparative exploit and memory safety benchmark test suite evaluating **Rust** against **SNlang** across five fundamental memory safety dimensions:

1. **Stack Reference Escape** (`stack_escape.rs` / `stack_escape.sn`)
2. **Collection Invalidation During Borrow** (`list_invalidation.rs` / `list_invalidation.sn`)
3. **Heap Use-After-Free & Double-Free** (`heap_uaf.rs` / `heap_uaf.sn`)
4. **Concurrency Data Races in Thread Spawn** (`spawn_race.rs` / `spawn_race.sn`)
5. **Reference Cycle Leak vs Cycle Collection** (`cycle_leak.rs` / `cycle_collect.sn`)

---

## Summary of Results

| # | Test Scenario | Rust Behavior (`rustc`) | SNlang Baseline Behavior | SNlang Hardened Verification Status |
|---|---------------|-------------------------|--------------------------|-------------------------------------|
| 1 | **Stack Reference Escape** | **Compile Error** (`E0515`, `E0597`) | Compiles & runs unsafely (reads dangling stack) | **VERIFIED BLOCKED** (`cannot assign address of local 'local': target outlives declaring scope`) |
| 2 | **List Invalidation During Borrow** | **Compile Error** (`E0502`) | Compiles & runs unsafely (buffer realloc / dangling ptr) | **VERIFIED BLOCKED** (`cannot mutate 'numbers' while borrowed by 'first'`) |
| 3 | **Heap Use-After-Free & Double-Free** | **Compile Error** (`E0382` moved value) | Compiles & runs unsafely (UAF read/write, double-free abort) | **VERIFIED BLOCKED** (`use of freed pointer 'p'`, `double free of 'p'`) |
| 4 | **Concurrency Data Races in Spawn** | **Compile Error** (`E0277` `!Send`, `E0597`) | Compiles & runs unsafely (data race across threads) | **VERIFIED BLOCKED** (`cannot pass blueprint 'Counter' to spawn without locking`, `data race hazard`) |
| 5 | **Reference Cycle Reclamation** | **Compiles & Leaks Permanently** (RC strong count $\ge 1$) | Compiles & Reclaims Fully (mark-sweep GC frees cycle) | **VERIFIED FUNCTIONAL** (4 objects allocated in cycle -> 0 retained after `gc_collect()`) |

---

## Test Descriptions & Outputs

### 1. Stack Reference Escape (`stack_escape.rs` vs `stack_escape.sn`)
- **Vulnerability**: Returning the address of a local stack variable from a function, or storing the stack address into an outer struct/blueprint that outlives the declaring stack frame.
- **Rust (`stack_escape.rs`)**:
  ```text
  error[E0515]: cannot return reference to local variable `local`
    --> tests/memory_safety/stack_escape.rs:15:5
     |
  15 |     &local
     |     ^^^^^^ returns a reference to data owned by the current function

  error[E0597]: `local` does not live long enough
    --> tests/memory_safety/stack_escape.rs:23:27
     |
  22 |         let local = 99;
  23 |         holder.ptr = Some(&local);
     |                           ^^^^^^ borrowed value does not live long enough
  24 |     }
     |     - `local` dropped here while still borrowed
  ```
- **SNlang Baseline (`stack_escape.sn`)**:
  Compiles without errors. Calling `escape_return()` returns a pointer to popped stack space; reading it yields corrupted data (`10` instead of `42`).

---

### 2. Collection Invalidation During Borrow (`list_invalidation.rs` vs `list_invalidation.sn`)
- **Vulnerability**: Holding a borrow/reference (`ref<T>`) to an element of a list while calling mutating methods (`.push()`). Growing the vector triggers `realloc`, moving the storage buffer to a new heap location and turning active element borrows into dangling pointers.
- **Rust (`list_invalidation.rs`)**:
  ```text
  error[E0502]: cannot borrow `numbers` as mutable because it is also borrowed as immutable
    --> tests/memory_safety/list_invalidation.rs:16:5
     |
  12 |     let first = &numbers[0];
     |                  ------- immutable borrow occurs here
  16 |     numbers.push(40);
     |     ^^^^^^^^^^^^^^^^ mutable borrow occurs here
  18 |     println!("First element: {}", *first);
     |                                   ------ immutable borrow later used here
  ```
- **SNlang Baseline (`list_invalidation.sn`)**:
  Compiles without errors. Pushing multiple elements reallocates the backing storage, leaving `first` pointing to the stale buffer.

---

### 3. Heap Use-After-Free & Double-Free (`heap_uaf.rs` vs `heap_uaf.sn`)
- **Vulnerability**: Deallocating heap memory with `free(p)` and subsequent reading (`value(p)`), writing (`set(p, ...)`), or deallocating again (`free(p)`).
- **Rust (`heap_uaf.rs`)**:
  Rust uses affine types. Calling `drop(ptr)` moves and consumes ownership.
  ```text
  error[E0382]: borrow of moved value: `ptr` (UAF read)
  error[E0382]: use of moved value: `ptr`    (UAF write)
  error[E0382]: use of moved value: `ptr`    (Double-free)
  ```
- **SNlang Baseline (`heap_uaf.sn`)**:
  Compiles without errors. At runtime, reading and writing to freed memory succeed silently, while the second `free(p)` aborts with `runtime error: double free of 0x...`.

---

### 4. Concurrency Data Races in Thread Spawn (`spawn_race.rs` vs `spawn_race.sn`)
- **Vulnerability**: Passing unsynchronized mutable data (raw references or unlocked reference objects) into concurrent threads.
- **Rust (`spawn_race.rs`)**:
  Rust statically enforces the `Send` trait and `'static` lifetime bounds on `thread::spawn`:
  ```text
  error[E0277]: `Rc<RefCell<Counter>>` cannot be sent between threads safely
     = help: the trait `Send` is not implemented for `Rc<RefCell<Counter>>`

  error[E0597]: `val` does not live long enough
     = note: argument requires that `val` is borrowed for `'static`
  ```
- **SNlang Baseline (`spawn_race.sn`)**:
  Compiles and launches threads modifying the same blueprint instance and stack pointer concurrently without locks, exposing race conditions.

---

### 5. Reference Cycle Reclamation (`cycle_leak.rs` vs `cycle_collect.sn`)
- **Vulnerability & Comparison**: Pure reference counting (`Rc` / `Arc`) cannot detect or reclaim self-referencing or mutually referencing data cycles (`A -> B -> A`). Unless programmers manually intervene with `Weak` pointers, cycle graphs leak permanently.
- **Rust (`cycle_leak.rs`)**:
  ```text
  === Rust Reference Cycle Leak Demonstration ===
  Inside scope: a.strong_count = 2, b.strong_count = 2
  Exited cycle creation scope.
  Nodes dropped: 0 (expected: 0 because cycle leaks)
  RESULT: LEAK CONFIRMED. Reference cycle permanently leaked in Rust Rc.
  ```
- **SNlang (`cycle_collect.sn`)**:
  SNlang includes a mark-sweep cycle collector in `compiler/runtime.c`:
  ```text
  === SNlang Cycle Collector Demonstration ===
  Initial GC tracked object count: 1
  GC object count after allocating cycle: 4
  GC object count after gc_collect(): 0
  Cycle successfully reclaimed: true
  ```
  SNlang successfully traces heap connectivity from stack roots, identifies the unreferenced circular subgraph, and frees all cyclic objects.

---

## How to Run

To execute the entire comparison suite:

```bash
chmod +x tests/memory_safety/run_all.sh
./tests/memory_safety/run_all.sh
```
