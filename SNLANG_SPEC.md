# SNlang — The SafarNow Language

**Version:** 0.3.1  
**Created by:** SafarNow  
**Paradigm:** Compiled, Strongly-Typed, Object-Oriented, Concurrent  
**Philosophy:** Easy syntax, native LLVM, opt-in null — not “as fast as C” by slogan, and not a Go/Rust clone.

SNlang compiles to native code through LLVM (`snc` emits IR, `clang` builds the binary). No semicolons. Curly braces `{}` for blocks. English-readable logic (`and` / `or` / `not`). The compiler lives in `compiler/` (Rust).

---

## 1. Comments

```text
// This is a single-line comment

/*
   This is a multi-line comment.
   Use it for documentation.
*/
```

---

## 2. Data Types

SNlang enforces strict typing. Every variable must declare its type.

### Primitive Types

| Type       | Description                          | Example                  |
|------------|--------------------------------------|--------------------------|
| `int`      | Whole numbers (64-bit signed)        | `int age = 25`           |
| `dec(X)`   | Decimal with X digits precision      | `dec(2) price = 99.50`   |
| `str`      | Text / string of characters          | `str name = "Alice"`     |
| `bool`     | True or false                        | `bool active = true`     |
| `byte`     | Single byte (0–255)                  | `byte flag = 0xFF`       |

### Collection Types

| Type          | Description                             | Example                              |
|---------------|-----------------------------------------|--------------------------------------|
| `list<T>`     | Ordered collection of type T            | `list<int> scores = [90, 85, 70]`    |
| `map<K, V>`   | Key-value pairs                         | `map<str, int> ages = {"Bob": 30}`   |

### Special Types

| Type       | Description                          | Example                  |
|------------|--------------------------------------|--------------------------|
| `none`     | Represents no value / null           | `return none`            |

> **`any`** — MVP in v0.2: `any_box(value)`, `.as_int()`, `.as_str()`, `.type_name()`. Boxed int/str/bool tags only.

> **`record`** — copy-by-value stack struct: `record Point { int x; int y }`.

---

## 3. Constants

Constants are immutable. Once set, they can never change.

```text
const int MAX_USERS = 1000
const str APP_NAME = "SafarNow"
const dec(2) TAX_RATE = 18.00
```

---

## 4. Variables & Assignment

```text
int count = 0
str greeting = "Hello"
bool isReady = false

// Reassignment (same type only — strict!)
count = 10        // OK
count = "hello"   // ERROR: type mismatch
```

---

## 5. Operators

### Arithmetic

| Operator | Description    | Example         |
|----------|----------------|-----------------|
| `+`      | Addition       | `x + y`         |
| `-`      | Subtraction    | `x - y`         |
| `*`      | Multiplication | `x * y`         |
| `/`      | Division       | `x / y`         |
| `%`      | Modulo         | `x % 2`         |
| `**`     | Power          | `x ** 3`        |

> **Fixed-Point Scaling (`dec(X)`):** Fixed-point multiplication and division automatically maintain exact precision by rescaling with $10^X$ (e.g. `1.50 * 2.00` yields `3.00`, and `1.50 / 2.00` yields `0.75`), preventing IEEE floating-point imprecision.

### Comparison

| Operator | Description          | Example       |
|----------|----------------------|---------------|
| `==`     | Equal to             | `x == 5`      |
| `!=`     | Not equal to         | `x != 5`      |
| `>`      | Greater than         | `x > 5`       |
| `<`      | Less than            | `x < 5`       |
| `>=`     | Greater or equal     | `x >= 5`      |
| `<=`     | Less or equal        | `x <= 5`      |

### Logical (English words, not symbols!)

| Operator | Description          | Example                    |
|----------|----------------------|----------------------------|
| `and`    | Both must be true    | `x > 0 and x < 100`       |
| `or`     | Either can be true   | `x == 0 or x == 1`        |
| `not`    | Negation             | `not isReady`              |

### Assignment Shortcuts

| Operator | Description           | Example      |
|----------|-----------------------|--------------|
| `+=`     | Add and assign        | `x += 5`     |
| `-=`     | Subtract and assign   | `x -= 5`     |
| `*=`     | Multiply and assign   | `x *= 2`     |
| `/=`     | Divide and assign     | `x /= 2`     |

---

## 6. Blocks — Curly Braces `{}`

SNlang uses curly braces `{}` for all block scoping. This keeps the language familiar to anyone coming from or going to Java, C++, or Go. No semicolons are needed — each statement is one line.

```text
fn main() {
    print("Hello")
}
```

---

## 7. Control Flow

### If / Else If / Else

Parentheses are **required** around conditions. Curly braces define blocks.

```text
if (age >= 18 and age < 60) {
    print("Adult")
} else if (age >= 60) {
    print("Senior")
} else {
    print("Child")
}
```

### Match (Switch-Case Alternative)

Clean pattern matching. No `break` needed — only the matched arm executes.

```text
match (status) {
    "active" {
        print("User is active")
    }
    "banned" {
        print("User is banned")
    }
    default {
        print("Unknown status")
    }
}
```

---

## 8. Loops

### For Loop (Counted)

Uses the standard `for` keyword — easy to transfer skills to Java/C++.

```text
for (int i = 0, i < 10, i += 1) {
    print(i)
}
```

### For-In Loop (Iterate Collections)

Same `for` keyword with `in` to iterate over collections.

```text
list<str> fruits = ["Apple", "Mango", "Banana"]

for (fruit in fruits) {
    print(fruit)
}
```

### While Loop

```text
int attempts = 0

while (attempts < 3) {
    print("Trying...")
    attempts += 1
}
```

### Loop Control

| Keyword   | Description                         |
|-----------|-------------------------------------|
| `stop`    | Exits the loop immediately (break)  |
| `skip`    | Skips to next iteration (continue)  |

```text
for (num in [1, 2, 3, 4, 5]) {
    if (num == 3) {
        skip
    }
    if (num == 5) {
        stop
    }
    print(num)
}
// Output: 1, 2, 4
```

---

## 9. Functions

Use `fn` to declare functions. Return types are declared after `->`.

```text
fn greet(str name) -> str {
    return "Hello, " + name
}

fn add(int a, int b) -> int {
    return a + b
}

// Functions with no return value
fn logMessage(str msg) {
    print("[LOG] " + msg)
}
```

### Default Parameters

```text
fn connect(str host, int port = 8080) {
    print("Connecting to " + host)
}

connect("localhost")         // Uses port 8080
connect("localhost", 3000)   // Uses port 3000
```

### Multiple Return Values (like Go!)

```text
fn divide(int a, int b) -> (int, bool) {
    if (b == 0) {
        return (0, false)
    }
    return (a / b, true)
}

int result, bool ok = divide(10, 3)

if (not ok) {
    print("Division failed!")
}
```

---

## 10. Pointers & Addresses (Simplified)

SNlang gives you direct memory access like C, but makes it **much simpler and safer**. No confusing `*`, `&`, `->` juggling.

### Declaring a Pointer

Use `ref<T>` (reference) to declare a pointer to a type. It reads like English: "a reference to an int."

```text
int x = 42
ref<int> p = address(x)    // p holds the memory address of x
```

### Reading the Value at an Address

Use `value(p)` to dereference — get the data the pointer points to.

```text
int y = value(p)    // y = 42
```

A `ref<T>` is a **non-owning borrow**. Storing one in a list (`list<ref<int>>`) keeps the
target's address, never a reference count: the runtime does no ARC on a borrow element —
not on the slot (which is not an object; the old blind retain corrupted the target
local's value) and not on the pointee (the target's owner holds it). The target's value is
unchanged by the push and the list read gives the right pointee
(`examples/list_ref_local_roundtrip_ok.sn`); the E-ESCAPE region ordering guarantees the
target outlives the list.

### Writing to an Address

```text
set(p, 100)         // The memory at p now holds 100
print(value(p))     // 100
print(x)            // 100 — x changed because p points to x
```

### Pointer Arithmetic

Move through memory manually. Essential for building your own data structures.

```text
ref<byte> start = address(buffer)
ref<byte> next = start + 1      // Move 1 byte forward
ref<byte> tenth = start + 10    // Move 10 bytes forward

byte val = value(next)          // Read the byte at that address
```

### Allocating Raw Memory

Use `alloc(bytes)` to request raw memory from the system. Use `free(p)` to release it.

```text
ref<byte> block = alloc(1024)   // Allocate 1024 bytes

// Write to it
set(block, 0xFF)
set(block + 1, 0x00)

// Read from it
byte first = value(block)       // 0xFF

// Release when done
free(block)
```

### Compile-Time Memory Safety (Rust-Grade Guarantees)

SNlang enforces strict compile-time safety checks on references and pointers without requiring complex lifetime syntax:
1. **Stack Escape Prevention:** An address of a local variable (`address(x)`) cannot outlive the variable's declaring scope. Assigning a stack address to a blueprint field, returning it from a function, or assigning it to an outer variable triggers: `cannot assign address of local '{name}': target outlives declaring scope`.
2. **Collection Invalidation Protection:** While an element of a `list` or `map` is borrowed (`ref<T>`), mutating operations (`push`, `insert`, `clear`) are rejected at compile time: `cannot mutate '{name}' while borrowed by '{borrower}'`.
3. **Affine Memory Deallocation:** Calling `free(p)` consumes the pointer. Subsequent reads, writes, or deallocations trigger `use of freed pointer '{name}'` or `double free of '{name}'`.
4. **Thread Isolation in `spawn`:** Passing unsynchronized mutable references or unlocked blueprints across threads in `spawn` is blocked at compile time to prevent data races. Global pools carry their own mode discipline (frozen by default; E0803.1/.2/.3 — see the pool section above): a handle into a frozen pool is Sendable (no legal write path), a `thread_local` pool's handle never leaves its thread, and a `synchronized` pool's every op takes its lock.
5. **Self-Relative Links (the E0801 copy discipline):** A `link<T, u16>` field stores δ = target − field_address (decoded: target = field_address + δ; |δ| ≤ 32767). A value of such a type is position-dependent BY CONSTRUCTION — a byte-copy preserves deltas encoded for the SOURCE addresses, which then decode against the copy's address. See **§10.5 Self-Relative Links** below for the full discipline.

See `tests/memory_safety/` for executable exploit test suites and comparison with Rust.

### High-Performance Systems Allocators

SNlang includes zero-overhead native allocators for high-throughput systems programming:

1. **Generational Pools (`std.pool` / `pool<T>` / `handle<T>`):**
   - Implements 64-bit generation-indexed slotmaps: 32-bit slot index + 32-bit generation counter.
   - Provides O(1) allocation, retrieval, and deallocation.
   - Immune to the ABA problem and dangling pointers: accessing a deleted handle is detected immediately (`is_valid(h) == false`), preventing use-after-free bugs without garbage collector pauses.
   - **The global-pool discipline — three declared modes, frozen by default** (the E0803 family; a global pool's handle passing freely into an unscoped `spawn`/`chan` would put two threads on one freelist — freelist corruption, memory unsafety):
     - `pool<T> p = pool_new<T>(...)` at module scope (bare) — **Frozen** (the default): read-only from every thread. `insert`/`set`/`remove` outside an `unfrozen (p) { ... }` window are rejected (**E0803.3**); handles are **Sendable** — zero-copy shared reads, spawn them freely: a frozen pool has no legal write path, so no thread can corrupt or race the freelist.
     - `pool<T> thread_local` — **ThreadConfined**: mutations legal on the declaring thread only; handles (and the pool) captured by spawn/chan = **E0803.1**. The runtime carries a home-thread tag as the backstop (a foreign-thread mutation fail-fasts).
     - `pool<T> synchronized` — **Synchronized**: a mutex-protected vault — every `insert`/`set`/`remove` takes the pool's lock; handles Sendable everywhere. No two threads can ever interleave insert/remove on one pool — the data-race-freedom condition for the vault.
     - **The `unfrozen (p) { ... }` window** — the structured single-thread mutation window on a Frozen pool (the `frozen { }` block's syntactic mirror): inside, mutations are legal on the declaring thread; at exit the pool re-freezes (early returns re-freeze; a panic never resumes). **E0803.2** rejects capturing/sending `p`'s handles (or `p` itself) in an unscoped spawn/chan *inside* the window — the receiving thread could race the freelist with the mutator; move the capture outside the window or declare the pool `synchronized`. Mode conflicts are checked too: an `unfrozen` window refuses a `thread_local`/`synchronized` pool (each mode has its own discipline) and refuses to nest inside a `frozen (p)` block; `frozen (p)` refuses a `synchronized` pool.
     - **Construction:** module-scope pools are real runtime objects — each declared pool is constructed at `main`'s entry with its declared mode (`sn_pool_new_mode`) and registered as a permanent GC root (`sn_gc_register_global`): the pool is module-scoped and immortal, and without registration an unlucky collection would sweep it (a bug caught during pool stress testing).
     - **Soundness:** a Frozen pool has no legal write path, so any number of threads holding handles cannot corrupt or race the freelist; ThreadConfined handles never leave the owning thread (capture rejected; the home-thread tag refuses the rest); Synchronized pools serialize every freelist mutation behind a lock; the `unfrozen` window restores single-thread, structured, capture-blocked mutation. **Pool objects are not copyable values** — `pool<T> q = p`, `q = p`, and passing `p` by value into a `pool<T>`-typed parameter are all rejected (the alias would rebind the runtime object under a local name whose mutation the checker cannot mode-check; only handles cross, and the pool is used by its declared name). The checker is the gate; the runtime's `frozen_depth`, vault lock, and home-thread tag are the enforcement backstop (the E-LKIND two-layer precedent).
     - **The honest capture-site caveat:** E0803.2 keys on the capture SITE's window depth (the spec's rule) — a task spawned *outside* a window may still be running when a *later* window opens. The A6c kill (two threads **mutating** one freelist → corruption → memory unsafety) is absolutely prevented: the window's mutations are confined to the declaring thread and every other mode serializes its mutators. A concurrent *reader* can never corrupt the freelist (reads do not write it); the residual is the reader's narrow generation-check-vs-`grow` window (a stale read aborts fail-fast per the generational design). A task-lifetime capture rule (blocking captures across windows) is the later-slice work item.
     - **The diagnostics:** `E0803.1: cannot capture/send handle '{h}' from pool '{P}' in unscoped spawn/chan: '{P}' is 'thread_local' — handles carry its home-thread tag and never leave the declaring thread. Declare '{P}' 'synchronized' for cross-thread access. [E0803]` · `E0803.2: cannot capture/send handle '{h}' from pool '{P}' in unscoped spawn/chan: '{P}' is unfrozen in this scope (mutable window) and the receiving thread could race its freelist. Move the capture outside the 'unfrozen ({P})' block, or declare '{P}' 'synchronized'. [E0803]` · `E0803.3: cannot mutate global pool '{P}' ({op}): global pools are frozen by default. Wrap the mutation in 'unfrozen ({P}) { ... }', or declare the pool 'thread_local' (single-thread) or 'synchronized' (locked). [E0803]`
     - See `examples/gpool_*.sn` (golden — including `gpool_alias_fail.sn`, the alias closure) and `tests/systems/test_gpool.c` (systems — the concurrent frozen read, the locked mutation, the window depth, the E0803.1 backstop).

2. **Elastic Lane Arenas (`region { ... }` / `std.arena`):**
   - High-speed bump allocation backed by 256 MB virtual memory reservations per lane.
   - Scope-bound `region { }` automatically captures the lane watermark on entry and rewinds it on exit, delivering instant O(1) bulk deallocation.
   - **Scope Unwind Guarantee:** Early returns (`return`) or errors (`try`) inside a `region { }` automatically restore the lane watermark, ensuring zero memory leaks on early exit paths.
   - `frozen { }` blocks lock lanes into read-only mode to prevent accidental allocation or mutation.
   - **Inline lowering** (`-O1+`): constant-size lane allocations compile to a 3-instruction bump (load top, bump, store) with a guard branch routing to the loud runtime path; region exit is a 1-store watermark restore.
   - **T-FIBR poison-fill** (`--poison` / `-O0` / debug builds): lane bytes are filled with `0xAA` on allocation and re-poisoned on rewind — the read-before-write guard (E0807), unconditional across all codegen paths. Release builds emit zero extra instructions.
   - **Pinned-lane accumulator coloring** (compiler-assigned): recursive `inout` list parameters that grow across a recursive call are automatically assigned to dedicated pinned lanes (L2–L6) — each pinned lane hosts exactly one growable for its entire SCC lifetime, so the accumulator is never buried under descendant scratch. The 6th simultaneously-live accumulator fires **E0802c** (the budget error — never silent reuse). An explicit `lane_alloc` on a pinned lane is the user-declared discharge; scratch lanes (L0/L1) for SCC accumulators fire **E0802**.
   - **The lane-aware items pool**: a lane-backed list's header dies at the rewind that covers it, and the *same rewind frees its grown items array* (a per-lane registry of lane-backed list headers; the rewind pops the covered suffix — O(#freed), never O(#live); no ref to lane memory can escape per E-GC-IN-LANE, so header and array are unreachable at the rewind). The E-NESTLED grown-array lifecycle: element storage is slab-backed under 65,536 elements (a 512 KB window carved from the shared 256 MB list slab — reused, never freed), malloc'd above the window, freed at the rewind when lane-backed. Heap-backed lists never enter the registry — `sn_release`/`gc_free` own their arrays.
   - **Escape checker** (E-ESCAPE): storing a reference into a destination requires the target to *outlive* the holder (the region-poset `⊑ₗ` law). A reference declared in a region that exits before the destination's region fires the error: "the target dies before the holder."
   - **GC scoping** (E-GC-IN-LANE): GC-managed memory (created via `new`/list literals — tracked by the garbage collector) and lane-backed memory (created via `lane_alloc` — untracked bump bytes) cannot reference each other. The GC cannot trace into lanes (its roots are globals+stacks+pushed values only), and lane rewinds don't respect the GC heap — a cross-category reference is a checked error in both directions.
   - **Task nursery lanes:** spawned tasks loan lanes from their thread's pool (`SnLane.parent`); at `nursery` join, each loaned lane resets to base in one store — the child's entire allocation reclaimed O(1).
3. **Tier-G Z-Set Delta Collections (`std.delta` / `ZSet`):**
   - `ZSet` = a group-valued multiset: each key carries a ℤ coefficient (the direct sum ℤ[X]).
   - **Exact retraction:** `remove(k, n)` = `add(k, -n)` — O(1)/key, no scans.
   - **Pure delta operators:** `delta(a, b)` computes a−b as a Z-set (change-size support); `apply(a, b)` computes a+b. These are the DBSP 𝒟/⊕ primitives.
   - `merge`, `count`, `total`, `len`, `keys`, `copy` — all O(1) amortized via the O(1) hash map.
   - Zero coefficients leave the support in the canonical form (configurable).
   - See `tests/systems/` for executable test suites.
4. **Tier-L Semilattice Cells (`std.lattice` / `LCell`):**
   - The join-semilattice tier of the differential calculus (the capability order **O ⊑ L ⊑ G** — the honest label of a pipeline is the meet). Values only **inflate**; ⊔ (join) is the only combinator; there is **NO ⊖** on this surface.
   - **Three carriers, each a join-semilattice by construction** (the admission rule: carrier verified idempotent + commutative + associative, checked structurally):
     - `max` — an int high-water mark; ⊔ = max. idempotent (`max(x,x)=x`), commutative, associative; ⊥ = the least int64. A losing offer is a no-op — the mark never drops.
     - `gset` — a grow-only int set; ⊔ = union. `A∪A=A`, `A∪B=B∪A`, `(A∪B)∪C=A∪(B∪C)`; membership/insert share the Tier-G O(1) hash machinery.
     - `lww` — last-writer-wins with a logical clock; ⊔ = the **lexicographic max of (clock, value)**: the higher clock wins, and clock ties break *deterministically* by the higher value — the winner of any bag of offers is order-independent (a total order's max is idempotent, commutative, associative).
   - **The delta-mutator law:** `m(X) = X ⊔ m^δ(X)` (Almeida–Shoker–Baquero JPDC 2018; Enes et al. 2018). `delta(dst, prev)` returns the **join-decomposition-optimal state-relative fragment** — only what moved, never a copy — such that `prev ⊔ delta(dst, prev) == dst` under the monotone regime `prev ⊑ dst` (Tier-L states only inflate, so this is the only regime the law is claimed for). `apply(base, d)` = base ⊔ d (pure). Quiescence: `delta(x, x)` is the identity cell.
   - **The (L, exact) rejection — the Grothendieck argument as a checked compile error.** The merge operator is idempotent (`x⊔x = x`), and the Grothendieck completion of an idempotent operator is trivial: in the group of fractions `[x] = [x⊔x] − [x] = 0`. The idempotent merge operator therefore does not group-embed (the sharp embeddability boundary: a commutative monoid embeds into a group iff cancellative — max/union/lww are not), and **exact retraction on the Tier-L carrier is impossible**. Retraction on Tier L is `tombstoned` or `none`, never `exact`. The `lcell_retract` intrinsic is *recognized* and **always rejected at compile time with E-LRET**:
     > `exact retraction on a Tier-L join-semilattice is impossible (the merge operator is non-embeddable: x⊔x = x ⇒ [x] = 0 — Grothendieck triviality; retraction on Tier L is tombstoned or none, never exact); use a Tier-G Z-set (zset_*) for exact retraction [E-LRET]`
   - **E-LKIND (tier confusion, runtime-enforced):** the surface type of a cell is an untyped `ptr`, so the checker cannot see the carrier — the runtime fail-fasts on carrier mismatch (`cannot merge a max cell with a gset cell — the ⊔ of different carriers is undefined`), on kind mismatch (an L op on a non-LCell, or an LCell handed to `zset_merge` — the Tier-G twin of the same rule). `SN_LATTICE_CHECK=1` turns on per-merge in-engine law self-checks (idempotence + commutativity); the exhaustive randomized law harness is `tests/systems/test_lcell.c`.
   - See `examples/lcell_*.sn` (golden) and `tests/systems/test_lcell.c` (systems) for the executable proofs.
5. **The Bounded Fixpoint Engine + Admission Checker (`std.zeta` / `ZFix`):**
   - The fixpoint semantics of the differential calculus (W2C §3, the δ₀/∫ bracket): the ζ engine runs the ADMITTED body — transitive closure `reach(a) :- edge(a,b), reach(b)` — as a bracket of **semi-naïve rounds** (Thm 4b: the incrementalized fixpoint IS classical semi-naïve evaluation — Bancilhon–Ramakrishnan SIGMOD 1986, derived from the calculus): each round derives ONLY from the previous round's delta δ₀^(r), folds the new facts into the integral ∫, then tests quiescence.
   - **The convergence theorem, executable (W2C §3.3):** a MONOTONE body (in the recursive variable) over a FINITE-HEIGHT per-key carrier ⇒ the bracket terminates in **≤ height rounds at the LEAST fixpoint** (Knaster–Tarski; Cousot–Cousot Pacific J. Math. 82(1) 1979). Both premises hold for the admitted closure body: monotone by construction; the carrier is presence {0,1} over the keys of the FINITE edge universe (sets over a finite universe qualify). The bound is observable: on a chain the bracket takes exactly the diameter many rounds (`zfix_rounds`); on a seed-only graph it is quiescent at once (0 rounds).
   - **The two named divergence classes — recognized intrinsics, ALWAYS rejected at compile time with E-ZADM** (admission is UNDECIDABLE in general; the checker is the sufficient condition priced at expressiveness — "arbitrary SQL/Datalog" shrinks to the stratified-monotone fragment — mandatory per the no-handwaving rule; refusals are compile-time errors, never silent divergence, W2C §3.5):
     - `zfix_count` — **counting-through-recursion** (`r(a) :- r(a)` with seed 1, generalized to `r(a) += incr` every round): monotone, but the per-key carrier is ℤ — INFINITE height — so the convergence theorem's premise fails:
       > `fixpoint body diverges: counting-through-recursion — the body increments a ℤ-valued count through the recursive variable each round, so the per-key carrier has infinite height and the convergence theorem's premise fails (monotone ≠ finite-height, both required); the bracket would never quiesce [E-ZADM]`
     - `zfix_negate` — **negation-through-recursion** (`μy. 1⊖y`): ⊖/negation through the recursive variable makes the body NON-MONOTONE, so Knaster–Tarski does not apply and the iterates oscillate; the failure is the non-monotone body, **tier-orthogonal** (1⊖y is group arithmetic — no carrier choice repairs it):
       > `fixpoint body diverges: negation-through-recursion — the body applies ⊖/negation through the recursive variable, so it is NOT monotone in the recursive variable and Knaster–Tarski does not apply (μy. 1⊖y oscillates; the failure is the non-monotone body, tier-orthogonal) [E-ZADM]`
   - **The per-tier quiescence tests, both executable** (the stop conditions differ because the algebras differ — 𝒟 presupposes ⊖, which does not and cannot exist on Tier L, the Grothendieck/E-LRET argument; hence a different test per tier):
     - **Tier G** (the Z-set carrier): `𝒟(∫α) = 0` — the round's delta Z-set is empty (len 0). The standalone predicate `zfix_quiescent_g` runs the direct whole-state form (one naive body step derives nothing).
     - **Tier L** (the GSET-closure carrier, `zfix_new_l` — Tier-L carriers qualify BY CONSTRUCTION, W2C §3.3's admission rule): `F(s) ⊑ s` via the **mutator law** — the round's state-relative fragment (`sn_lcell_delta`; `delta(x,x) = ⊥`, no inflation) is empty. The standalone predicate `zfix_quiescent_l` runs the direct form: one naive body step joined into a candidate, then `lcell_delta(s ⊔ F(s), s)` must be ⊥.
   - **Per-update cost accounting (W2C §3.4 — the honest caveat):** the bracket's total work is the ROUND-SUM `W_μ = Σ_rounds c·(|δ₀^(r)| + Σ_v |Δv^(r)|)` — **NOT bounded by |ΔDB| nor |ΔV|**: one seed fact can trigger Θ(n) re-derivation rounds (the fraud-propagation shape). Convergence bounds bound the ROUNDS (the lattice height); per-round work remains data-dependent (O(|δ|) per round — the `zfix_probes` observable).
   - **The surface:** edges as packed pairs `a·2³² + b` (a, b ∈ [0, 2³²)) in a `std.delta` Z-set; seeds as a Z-set of nodes. `zfix_new(edges, seed)` / `zfix_new_l(...)` construct the bracket; `zfix_run` runs to quiescence; `zfix_rounds` / `zfix_len` / `zfix_has` / `zfix_probes` read out the fixpoint and the work; `zfix_state` returns a copy of the reach set.
   - **Honest scope:** the surface has no user-defined fixpoint bodies yet, so the admission check is per-mode on the built-in body; a user-body certifier (the Propel-style checked-not-proven class, W2C §6.4) is a later slice.
   - See `examples/zfix_*.sn` (golden), `tests/systems/test_zfix.c` (systems — closure vs a reference bitset BFS, exact per-round probe accounting), and `bench/RESULTS_ZFIX.md` (the semi-naïve bracket vs the naive fixpoint at N=100K edges).

### Why This Is Easier Than C

| Task               | C Syntax                | SNlang Syntax            |
|--------------------|-------------------------|--------------------------|
| Declare pointer    | `int *p;`               | `ref<int> p`             |
| Get address        | `p = &x;`               | `p = address(x)`         |
| Read value         | `y = *p;`               | `y = value(p)`           |
| Write value        | `*p = 100;`             | `set(p, 100)`            |
| Pointer arithmetic | `*(p + 3) = 10;`        | `set(p + 3, 10)`         |
| Allocate memory    | `p = malloc(1024);`     | `p = alloc(1024)`        |
| Free memory        | `free(p);`              | `free(p)`                |

With this system, a programmer can build **any** data structure (linked lists, stacks, queues, trees, hash maps) from scratch — without fighting cryptic syntax.

---

## 10.5 Self-Relative Links — `link<T, u16>` (the E0801 copy discipline)

**Relocatable Graph Storage — Safe Relative Pointers and Relocation Boundaries (E0801).**

### The type

`link<T, u16>` is a 2-byte **self-relative delta field**. A value of the type at address `A` stores `δ = target − A` — decoded: `target = A + δ` (the i16 sign extension *is* the decode). The u16 width bounds the reach: `|δ| ≤ 32767` (at 16 bytes/record, ±2046 elements). A raw `0` is the **null link** (position-free — copies of null-linked values are sound anywhere).

```sn
record Node {
    int val            // offset 0
    link<Node> next    // offset 8 — a 2-byte delta to another Node
}
```

The value is position-dependent **by construction**: a byte-copy preserves deltas encoded for the *source* field addresses, and those copied deltas then decode against the *copy's* address — every link silently retargets. The discipline below makes that a checked error instead of a silent corruption.

### The HasSelfLink predicate

A type is **HasSelfLink** when a by-value copy of it moves link-bearing bytes: `link<T, u16>` itself; a record/blueprint/tuple with any by-value link-bearing field (nested records recurse; container-header fields — `list`/`map`/`str` — do **not** propagate: copying a header does not move the elements); and `list<E>` propagates `HasSelfLink(E)` — the *element* is what a push/pop/copy moves.

### The rejection set (E0801, compile-time)

**E0801 fires iff `has_self_link(type_of(rhs)) ∧ rhs_is_lvalue_load(rhs)`** — the RHS *reads* a HasSelfLink value from storage that survives the copy (Ident/Index/Member reads), vs. *constructs* one fresh (constructor/literal/call result — the DPS `sret` destination-projection semantics: the value is built in place at its final address, so its deltas encode correctly). Four sites:

| Site | Rejected | Legal discharge |
|---|---|---|
| Assignment / decl-init | `d = s`, `Node d = s` | `d = make()` (fresh, sret) |
| Pass-by-value | `take(s)` | `take_ref(ref<Node> r)` / `inout` |
| push/insert | `dst.push(src[0])` | `xs.push(make())` (fresh), `push_ref`, `relocate_node` |
| match-binding | `Tree.node(n)` (by-value pattern) | `ref<Tree>` pattern binding |

```
E0801 (push): cannot push '{src[i]}' into '{dst}' by value because '{T}' contains a
       self-relative 'link<{T}, u16>' (the copied delta would decode against '{dst}''s
       address, not the link's target); push a freshly constructed value (built in place
       via sret), pass 'ref<{T}>', or use 'relocate_node(src, dst)'. [E0801]

E0801 (match): cannot bind '{b}' by value from the match arm on '{T}': '{T}' contains a
       self-relative 'link<{T}, u16>'; bind 'ref<{T}>' in the pattern, or 'relocate_node'
       into a fresh destination. [E0801]
```

(The assignment flavor is `E0801 (assign): cannot assign '{src}' to '{dst}' by value …`; the pass-by-value flavor is `E0801 (call): cannot pass '{src}' by value to parameter '{p}' of '{f}' …`.)

### The legal movers (runtime)

- **`relocate_node<T>(src, si, dst, di)`** — decode every link at the source (`t = addr(f_src) + δ`), byte-copy, re-encode at the destination (`δ' = t − addr(f_dst)`). Links targeting **inside** the moved value are fixed to the new interior; links targeting **outside** keep their absolute target (re-based: `δ' = δ − (addr(f_dst) − addr(f_src))`). Requires `|δ'| ≤ 32767`:
  ```
  E0801 (relocate): cannot relocate '{f}' from '{src}' to '{dst}': the re-encoded delta
         ({d'}) for its link target exceeds the i16 range [−32767, +32767]; move the value
         and its target closer together, or use 'link<{T} in pool, u32>'. [E0801]
  ```
- **`relocate_range<T>(src, dst, n)`** — the **constant-shift exemption**: with uniform `k = dst_base − src_base`, every *intra-range* link is delta-preserving (target and field both shift by `k` ⇒ δ unchanged), so the per-link re-encode is provably the identity for them and the range bulk-copies **iff self-contained** (the same scan as to_bytes); range-boundary-crossing links re-encode once with `δ' = δ − k`. The only bulk path where memcpy is sound without per-link re-encode.
- **`link_to_bytes<T>(base, a, b)`** — the O(N) **sender-side self-containedness scan** before packing: for every non-null link inside the serialized range, decode `t = field_addr + δ` and require `slice_base ≤ t < slice_end`. Cross-slice links (targets in other lists on the same lane — legal to *hold*, illegal to *serialize*: the bytes cannot survive relocation) fail fast:
  ```
  E0801 (serialize): cannot serialize '{xs}[{a}..{b}]' to bytes: 'link<{U}, u16>' at
         element {i} ('{field}') decodes to an address outside the serialized slice
         (cross-slice links cannot survive relocation). Serialize the enclosing whole
         list, drop the link, or use 'link<{U} in pool, u32>'. [E0801]
  ```
  This makes "100% legal bulk copy" **true by precondition** instead of false by omission — §4.5's load-time verifier would otherwise receive a guaranteed failure.

Supporting carriers: `link_block_new<T>(n)` (a zeroed contiguous block — every field starts null), `link_elem<T>(block, i)` (slot address), `link_set<T>(block, i, field, tblock, ti)` (the only checked writer — it encodes against the field's actual address) and `link_get<T>(block, i, field)` (the decode readout).

### The soundness argument

Every copy of a HasSelfLink value now either **constructs it fresh at its final address** (sret — deltas encode correctly by construction), **re-encodes every delta against the new field address** (`relocate_node`/`relocate_range`, with the i16 range checked at every re-encode), or **is rejected** (E0801 at the four by-value sites). No 2-byte delta is ever interpreted against an address it was not encoded for; the `to_bytes` scan extends the same invariant across the serialization boundary.

**Out of scope for this slice (documented):** the pool-relative form `link<T in pool, u32>` (a 4-byte pool-base-relative delta that survives arbitrary copies — named in every diagnostic as the unbounded-range discharge); automatic codegen encode/decode on user-struct field access (the `link_*` intrinsics are the explicit surface — `link_set` encodes against the actual field address, `link_get` decodes — while the checker's rejections hold for ALL copies of link-bearing values regardless of how they were built); and `from_bytes` re-hydration (the pack side and the scan are this slice's kill; the load side remains §4.5's two-pass verifier as spec'd).

---

## 11. Building Your Own Data Structures (Example)

SNlang does **not** provide built-in complex data structures. The programmer builds them using blueprints, pointers, and raw memory. Here is an example of a simple linked list node:

```text
blueprint Node {
    int data
    ref<Node>? next = none

    fn create(int data) {
        self.data = data
    }
}

fn main() {
    object a = Node(10)
    object b = Node(20)
    object c = Node(30)

    a.next = address(b)
    b.next = address(c)

    // Walk the list
    ref<Node>? current = address(a)
    while (current != none) {
        print(value(current).data)
        current = value(current).next
    }
    // Output: 10, 20, 30
}
```

---

## 12. Error Handling

Explicit error handling inspired by Go. No hidden exceptions. You always deal with errors.

```text
fn readConfig(str path) -> (str, error) {
    str data = file_read(path)
    if (data == "") {
        return ("", error("File missing or empty: " + path))
    }
    return (data, none)
}

str data, error err = readConfig("config.txt")

if (err != none) {
    print("Error: " + err.message())
} else {
    print(data)
}
```

> Prefer **`use std.file`** — `read`/`write` return `(str, error)` / `error`; `exists(path) -> bool`.

### Panic (Unrecoverable Errors)

```text
fn mustConnect(str db) -> connection {
    connection conn, error err = openDB(db)
    if (err != none) {
        panic("Cannot start without database!")
    }
    return conn
}
```

---

## 13. Blueprints (OOP — Classes)

`blueprint` replaces the word `class`. Curly braces `{}` for the body.

```text
blueprint User {
    str name
    str email
    int age

    fn create(str name, str email, int age) {
        self.name = name
        self.email = email
        self.age = age
    }

    fn greet() -> str {
        return "Hi, I'm " + self.name
    }

    fn isAdult() -> bool {
        return (self.age >= 18)
    }
}
```

### Object Creation

Use `new Type name(field: value, …)`. Field values are assigned, then optional `fn create()` runs if defined.

```text
new User alice(name: "Alice", email: "alice@mail.com", age: 25)
print(alice.greet())

if (alice.isAdult()) {
    print("Access granted")
}
```

### OOP — what is implemented vs not

SNlang has a **teaching OOP subset**, not full Java/C++ OOP:

| Concept | Status |
|---------|--------|
| Blueprints (classes), `self`, fields, methods | ✅ |
| Single inheritance (`from`) + `super.method()` | ✅ |
| Contracts + `follows` (compile-time interface check) | ✅ |
| Field access: `open` / `closed` / `guarded` | ✅ enforced at compile time |
| Optional `fn create()` after field init | ✅ |
| Subtype polymorphism (`Child` where `Parent` expected) | ✅ Vtable dispatch |
| Contract-typed variables (`Drawable x`) | ✅ |
| Virtual / dynamic dispatch | ✅ Full Vtable dispatch |
| Method devirtualization | ✅ LLVM direct calls for known types (1B calls/sec) |
| Generic blueprints (`blueprint Box<T>`) | ✅ Monomorphize at `new` |
| Static members, abstract classes | ✅ |
| Method-level `closed` / `guarded` | ✅ |
| Heterogeneous collections (`list<Animal>` mixed subtypes) | ✅ |

---

## 14. Inheritance

Use `from` to inherit from another blueprint.

```text
blueprint Animal {
    str species
    int legs

    fn describe() -> str {
        return self.species + " with " + cast(self.legs, str) + " legs"
    }
}

blueprint Dog from Animal {
    str breed

    fn bark() {
        print("Woof!")
    }
}
```

```text
new Dog rex(species: "Canine", legs: 4, breed: "Labrador")
print(rex.describe())
rex.bark()
```

---

## 15. Contracts (Interfaces)

`contract` defines a set of functions that a blueprint **must** implement.

```text
contract Drawable {
    fn draw()
    fn resize(int width, int height)
}

blueprint Circle follows Drawable {
    int radius

    fn draw() {
        print("Drawing circle with radius " + cast(self.radius, str))
    }

    fn resize(int width, int height) {
        self.radius = width / 2
    }
}
```

---

## 16. Access Control

| Keyword   | Description                                  |
|-----------|----------------------------------------------|
| `open`    | Accessible from anywhere (default)           |
| `closed`  | Only accessible within the blueprint itself   |
| `guarded` | Accessible by the blueprint and its children  |

```text
blueprint BankAccount {
    open str ownerName
    closed dec(2) balance = 0.00

    fn deposit(dec(2) amount) {
        self.balance += amount
    }

    fn getBalance() -> dec(2) {
        return self.balance
    }
}
```

---

## 17. Modules & Imports

Every `.sn` file is a module. Import using `use`.

```text
// file: math_utils.sn
fn square(int x) -> int {
    return x ** 2
}
```

```text
// file: main.sn
use math_utils

int result = math_utils.square(5)
print(result)   // 25
```

### Multiple Module Imports

You can import multiple modules in a single file:

```text
use std.math
use utils.string
use io.file
use net.http

fn main() {
    print("All modules loaded!")
}
```

### Dotted Module Paths

Modules can be organized in hierarchical paths:

```text
use std.collections.list
use std.collections.map
use company.auth.jwt
use company.database.postgres
```

**Current Implementation Status:**
- ✅ `use module.path` syntax parsing
- ✅ Single and multiple module imports
- ✅ Dotted module paths (`use std.math`)
- ✅ File loading and symbol resolution (`driver.rs` walks imports)
- ✅ Search paths: importer directory, `packages/`, `stdlib/`, path deps in `sn.toml`
- ✅ `snc pkg init|add|list|build|publish` for local packages + lockfile (`sn.lock.toml`)
- ✅ Remote package registry MVP (`--registry` fetches `index.json` + tarball; see `docs/PACKAGES.md`)

### Standard Library Imports

```text
use std.io          // print, file_read, file_write (legacy globals)
use std.file        // read, write, append, exists, delete, copy, move, mkdir, rmdir, list_dir
use std.path        // join, base, dir, ext
use std.os          // getenv, exit, args
use std.math        // abs, min, max, pow, …
use std.net         // HTTP client (plain HTTP sockets; HTTPS via curl)
use std.http        // get, post, serve_once (TLS via verified curl by default)
use std.time        // now_ms, sleep_ms, format
use std.json        // parse, encode
use std.string      // isEmpty, repeat
use std.test        // assert_true, assert_eq_int, assert_eq_str, …
```

---

## 18. Concurrency / Threads

### Spawning Green Tasks (Work-Stealing Scheduler)

`spawn` executes tasks cooperatively on a lightweight M:N work-stealing pool across available CPU cores:

```text
fn downloadFile(str url) {
    print("Downloading: " + url)
}

// Spawns lightweight green task
spawn downloadFile("https://example.com/data.zip")
```

In benchmarks (`tests/concurrency/bench_spawn_scale.sn`), SNlang executes **10,000 concurrent green tasks in ~16–20 ms**.

### Structured Concurrency (`nursery`)

The `nursery` scope guarantees that all child `spawn` tasks are completed before execution leaves the block, eliminating orphaned background tasks:

```text
nursery {
    spawn worker(1)
    spawn worker(2)
} // The closing brace automatically joins all child tasks!

print("All workers finished cleanly!")
```

> **Guaranteed Scope Unwind:** Even if a function executes an early `return` or propagates an error (`try`) from within a `nursery { }`, the compiler emits unwind joins to guarantee all spawned tasks finish before the calling stack frame exits.

### Move-on-Send Channels (Zero-Lock Data-Race Immunity)

Channels enforce affine ownership transfer at compile time. Sending an object over a channel moves it, preventing the sender from accessing or mutating it:

```text
chan<str> messages

str payload = "Order #101"
messages.send(payload) // 'payload' is moved to the channel

// print(payload) // COMPILE ERROR: use of moved value 'payload'
```

> **Deep Ownership Transfer:** Sending an object member (`messages.send(p.data)`) or collection element (`messages.send(xs[0])`) automatically transfers ownership of the root container variable (`p` or `xs`), preventing concurrent read/write data races across threads.

### Safe Channel Receive & Status Check

Receiving from a channel supports tuple destructuring to detect closed channels without crashing:

```text
str msg, bool ok = messages.receive()
if (not ok) {
    print("Channel was closed by sender!")
}
```

### Channels (Safe Communication)

```text
chan<str> messages

spawn {
    messages.send("Hello from thread!")
}

spawn {
    str msg = messages.receive()
    print(msg)
}
```

### Locks (Synchronization)

```text
lock counterLock
int counter = 0

fn increment() {
    lock(counterLock) {
        counter += 1
    }
}

spawn increment()
spawn increment()
print(counter)  // Output: 2
```

### Async/Await

```text
async fn slowTask() -> int {
    return 42
}

int result = await slowTask()
print(result)
```

---

## 19. Type Casting

Explicit only. No silent coercion — ever.

```text
int x = 42
dec(2) y = cast(x, dec(2))   // y = 42.00
str z = cast(x, str)         // z = "42"

str numStr = "100"
int num = cast(numStr, int)  // num = 100
```

---

## 20. Null Safety

A variable can only be `none` if you explicitly allow it with `?`.

```text
str name = "Alice"       // Can NEVER be none
str? nickname = none     // Allowed to be none

if (nickname != none) {
    print(nickname)
}

// Or use the 'otherwise' operator
print(nickname otherwise "No nickname")
```

---

## 21. String Operations

```text
str greeting = "Hello, World!"

int len = greeting.length           // 13
str sub = greeting.slice(0, 5)      // "Hello"
bool has = greeting.contains("World")  // true
str fixed = greeting.replace("World", "SNlang")
list<str> parts = greeting.split(", ")
str upper = greeting.upper()        // "HELLO, WORLD!"
str lower = greeting.lower()        // "hello, world!"

// String interpolation
str name = "Alice"
int age = 25
str msg = "My name is {name} and I am {age} years old"
```

### High-Performance String Building (`StringBuilder`)

For tight loops, large payload serialization, or continuous concatenation, standard `str + str` incurs $O(N^2)$ reallocation overhead. `StringBuilder` provides amortized $O(1)$ exponential capacity growth:

```text
use std.string

new StringBuilder sb(64)
sb.append("Hello")
sb.append(" ")
sb.append("World")
sb.append_int(42)

print(sb.to_str())     // "Hello World42"
print(sb.len())        // 14
sb.clear()             // resets length to 0 without reallocating buffer
```

In benchmarks (`tests/benchmarks/bench_string.sn`), `StringBuilder` processes 50,000 appends in **2 ms**, delivering an estimated **~150× speedup** over naive concatenation.

---

## 22. Input / Output

```text
use std.io

print("Hello, SNlang!")

str userInput = input("Enter your name: ")
print("Welcome, " + userInput)
```

### File I/O

**Legacy globals** (no import): `file_read(path) -> str`, `file_write(path, data) -> bool`.

**`std.file`** (recommended):

```text
use std.file

fn main() {
    str data, error err = read("config.txt")
    if (err != none) {
        print(err.message())
        return
    }
    error e = write("out.txt", data)
    print(exists("out.txt"))
    list<str> names, error le = list_dir(".")
}
```

| Function | Returns |
|---|---|
| `read(path)` | `(str, error)` |
| `write(path, data)` | `error` |
| `append(path, data)` | `error` |
| `exists(path)` | `bool` |
| `delete(path)` | `error` |
| `copy(src, dst)` | `error` |
| `move(src, dst)` | `error` |
| `mkdir(path)` | `error` |
| `rmdir(path)` | `error` |
| `list_dir(path)` | `(list<str>, error)` |

### Path and OS

```text
use std.path
use std.os

str p = join("dir", "file.txt")
print(base(p))
str? v = getenv("HOME")
list<str> argv = args()
```

### Membership (`in`)

```text
list<int> xs = [1, 2, 3]
if (2 in xs) { print("yes") }
if ("lo" in "hello") { print("substring") }
```

Distinct from `for (x in xs)` — `in` is also a binary operator.

### Formatter

```sh
snc fmt examples/*.sn
```

Trims trailing whitespace, normalizes brace-based indentation (4 spaces), ensures a final newline.

### File I/O (legacy detail)

Whole-file text only — global builtins, no import:

```text
fn main() {
    bool ok = file_write("output.txt", "Hello from SNlang!")
    str contents = file_read("output.txt")
    print(contents)
}
```

- `file_read(path) -> str` — returns `""` if the file is missing or empty (legacy; prefer `std.file.read`)
- `file_write(path, data) -> bool` — overwrites the whole file

**MVP (see README):** HTTPS via system `curl` (cert verify by default; `SN_HTTP_INSECURE=1` for `-k`), remote registry fetch + `sn.lock.toml` + `pkg publish`, async/await (blocking), goroutine pool, OOP polymorphism, generic blueprints.

**Still not production-grade:** in-process OpenSSL/LibreSSL, hosted central registry, true async I/O, work-stealing scheduler, full borrow lifetimes, complete LSP/debugger, self-hosted compiler.

---

## 23. Program Entry Point

Every SNlang program starts from a `main` function.

```text
fn main() {
    print("Welcome to SNlang!")
    int x = 10
    int y = 20
    print("Sum: " + cast(x + y, str))
}
```

---

## 24. Complete Example Program

```text
use std.io

const str APP_NAME = "SNlang Demo"
const dec(2) TAX_RATE = 18.00

contract Printable {
    fn display() -> str
}

blueprint Product follows Printable {
    str name
    dec(2) price
    int quantity

    fn create(str name, dec(2) price, int quantity) {
        self.name = name
        self.price = price
        self.quantity = quantity
    }

    fn totalPrice() -> dec(2) {
        dec(2) subtotal = self.price * cast(self.quantity, dec(2))
        dec(2) tax = subtotal * TAX_RATE / 100.00
        return subtotal + tax
    }

    fn display() -> str {
        return self.name + " — $" + cast(self.totalPrice(), str)
    }
}

blueprint DigitalProduct from Product {
    str downloadURL

    fn display() -> str {
        return self.name + " [Digital] — $" + cast(self.totalPrice(), str)
    }
}

fn main() {
    print("=== " + APP_NAME + " ===")

    new Product laptop(name: "Laptop", price: 999.99, quantity: 1)
    new DigitalProduct ebook(
        name: "SNlang Guide",
        price: 29.99,
        quantity: 1,
        downloadURL: "https://safarnow.com/snlang-guide"
    )

    // MVP subtype polymorphism: Child may be used where Parent is expected (type_id dispatch).
    // Homogeneous list<Product> holding mixed subclasses is still limited — call methods directly:
    print(laptop.display())
    print(ebook.display())

    dec(2) total = laptop.totalPrice() + ebook.totalPrice()

    print("Total: $" + cast(total, str))

    str? coupon = none
    str discount = coupon otherwise "No coupon applied"
    print(discount)

    if (total > 500.00) {
        print("Free shipping!")
    } else {
        print("Shipping: $9.99")
    }
}
```

---

## Quick Reference Card

| Feature          | SNlang Syntax                    | Java / C++ / Go           |
|------------------|----------------------------------|----------------------------|
| Variable         | `int x = 5`                      | `int x = 5;`              |
| Constant         | `const int X = 5`                | `final int X = 5;`        |
| Function         | `fn add(int a, int b) -> int {`  | `int add(int a, int b) {` |
| Class            | `blueprint User {`               | `class User {`             |
| Interface        | `contract Drawable {`            | `interface Drawable {`     |
| Inheritance      | `blueprint Dog from Animal {`    | `class Dog extends Animal` |
| Implements       | `blueprint X follows Y {`        | `class X implements Y {`   |
| If               | `if (x > 5) {`                   | `if (x > 5) {`            |
| For Loop         | `for (int i=0, i<10, i+=1) {`   | `for(int i=0;i<10;i++) {` |
| For Each         | `for (x in list) {`             | `for (auto x : list) {`   |
| Switch           | `match (x) {`                   | `switch(x) {`              |
| Pointer          | `ref<int> p`                     | `int *p`                   |
| Get address      | `address(x)`                     | `&x`                       |
| Dereference      | `value(p)`                       | `*p`                       |
| Write to ptr     | `set(p, 100)`                    | `*p = 100`                 |
| Allocate         | `alloc(1024)`                    | `malloc(1024)`             |
| Free             | `free(p)`                        | `free(p)`                  |
| Thread           | `thread fn work() {`            | `new Thread(...)`          |
| Null check       | `str? x = none`                  | `String x = null;`        |
| Null fallback    | `x otherwise "default"`          | `x ?? "default"`           |
| Cast             | `cast(x, str)`                   | `(String) x`               |
| Import           | `use std.io`                     | `import java.io.*`         |
| Print            | `print("hi")`                    | `System.out.println("hi")` |
| Logical AND      | `and`                            | `&&`                       |
| Logical OR       | `or`                             | `\|\|`                     |
| Logical NOT      | `not`                            | `!`                        |
| Break            | `stop`                           | `break`                    |
| Continue         | `skip`                           | `continue`                 |
| Public           | `open`                           | `public`                   |
| Private          | `closed`                         | `private`                  |
| Protected        | `guarded`                        | `protected`                |

---

## File Extension

All SNlang source files use the **`.sn`** extension.

```
main.sn
utils.sn
models.sn
```

---

*SNlang — Built by SafarNow. Code should be simple, strict, and fast.*

---

## 25. Defer, try, null finish, closures (implemented)

```text
defer { print("on function exit") }

fn divide(int a, int b) -> (int, error) { ... }
int q = try divide(10, 2)   // early-return error; not Java exceptions

str? name = none
str shown = name otherwise "n/a"
if (name != none) { print(name) }   // name is str in this branch
str must = name!!                   // panic if none
print(name?.length())               // optional chaining

fn apply(fn(int) -> int f, int x) -> int { return f(x) }
```

`defer` is **function-scoped** (like Go): it runs on return / function exit, not at the end of an `if` block.

`match` on `T?` / `error` must be exhaustive (`none` + another arm, or `default`).

Nested `fn` and `fn(int x) -> int { ... }` lambdas are values (limited captures, copied like `spawn`). There is no goroutine scheduler and no borrow checker.

## 26. Packages

`sn.toml` + `snc pkg init|add|build|list`. `use foo.bar` resolves through `packages/` and `stdlib/`. See README.

## 27. Wave Regions — Theorem WS, executable (implemented)

A **wave region** is a deterministic observation bracket over module-scope Tier-L seam cells — the executable form of the wave-seam composition theorem (**Theorem WS**: the typed read-mode surface + verified confluent join semantics).

```text
ptr results = lcell_new(1)          // module scope: a SEAM CELL (carrier 0=max, 1=gset, 2=lww)
ptr peak    = lcell_new(0)

fn main() {
    lcell_inflate(peak, 10, 0)       // fallback merge, program-order-BEFORE the wave (R1's provable arm)

    wave {
        int base = read_W(peak)      // the frozen capture κ (deterministic read mode)
        contrib(results, base - 8, 0)  // stage a payload; ⊔-applied at J(W)
        contrib(peak, base * 2, 0)
    }
    // J(W) happened here: the staged multiset folded into the cells
    print(read_J(peak))              // 20 — κ₀ ⊔ ⊔𝒰_W (fold-valued; schedule-independent)
    print(lcell_len(results))        // 2 — the fold observed via the fallback reads
}
```

**The theorem, in brief.** Under the C-conditions the checker enforces, every observation in and after a wave is fully deterministic: in-wave (Part 1 — the body is a pure function of the frozen capture κ, Lemma WD), at the join (Part 2 — the fold's result is independent of staging order, grouping, duplication, and thread count, by the Tier-L carrier's checked lattice laws; contributions are ONE atomic ⊔-merge per cell, Lemma NI(ii)), and composed (Part 3 — sequential waves on one cell compose, J(Wᵢ) → capture(W_{i+1})). Wave owns observation; the semilattice owns the seams.

**The typed read-mode surface (the mode is in the type).**

| Mode | Op | Meaning |
|---|---|---|
| `read_W(c)` | in-wave | the frozen capture κ's readout — deterministic (W2B §3) |
| `contrib(c, a, b)` | in-wave | stage the payload pair (exactly an `lcell_inflate` offer); ⊔-applied at J(W) |
| `read_J(c)` | after J(W) | the folded readout — fold-valued, schedule-independent; the checker verifies the wave provably closed (a branch-local wave does not certify the fall-through path) |
| `lcell_value`/`len`/`has` | fallback (`read_S`) | defined + session-monotone, **schedule-dependent — never deterministic**; refused on captured cells in-wave (E0812) |
| `lcell_merge`/`lcell_inflate` | fallback (S1) | the fallback merge path; into a *claimed* seam cell, legal only program-order-before the wave |

**The checked side conditions.**

- **E0812 — the purity kill switch.** The wave body must be a pure function of κ (Lemma WD's premise). Rejected, as compile errors: wall-clock (`time_now_ms`), env/argv/IO reads, chan receives, `spawn`/`nursery`/`go`/`lock` (the wave IS the parallelism), nested waves, early `return`/`stop` (the exit edge contrib→exit→J(W) is structural; termination is the runtime contract E0810·R), `defer`, `unfrozen` windows, asm, calls to impure functions (an **interprocedural** call-graph fixpoint — a wall-clock read one call away does not slip through), method calls (outside the wave-purity fragment), and the fallback sync ops (`lcell_inflate`/`lcell_merge` — the J0 immediate merge is not the default). Without the gate, the determinism grade would degrade *undetected* (W2B §4.6 case (d)) — so it fires.
- **E-R1 — quiescent capture (Rule R1).** A seam cell's capture is legal only if every fallback merge into it is HB-before the capture or after J(W). Lexical order can prove only the **before** arm (a textually-later merge can sit *between* two J(W)s when the wave is loop-shaped), so: fallback merges into a claimed seam cell are legal only program-order-before the wave; a second wave on the same cell is legal only as sequential composition inside one function body (`two waves share seam cell 'c' — neither capture can be R1-quiescent w.r.t. the other` is case (b), *a theorem, not a defect*: concurrent waves give confluence, never value determinism); `read_W`/`contrib` outside a wave, on a non-seam cell, or `read_J` before the join are named refusals.

**The runtime.** `wave { }` lowers to `sn_wave_new` + one `sn_wave_capture` per captured cell (sorted; pointer identity is the slot key, so aliases resolve) + the body + `sn_wave_join` — the **J2 batched canonical fold**: per cell, the staged multiset folds into one offer cell (any internal order — the laws make it order-independent) and joins through ONE `sn_lcell_merge` (atomic, and running the `SN_LATTICE_CHECK=1` law self-checks on exactly the event the theorem quantifies over). Runtime backstops (`[E-R1]`, fail-fast): read_W/contrib on an uncaptured cell, a cell captured twice, an out-of-range slot.

**The honest boundary (slice-1).** The body runs ONCE, single-tile — the wave IS the parallelism in the model, and the *parallel* form (spawn-per-tile + live-arity barrier, DBM emit ranges, the canonical (sender,seq) k-way merge) is future surface this API is shaped for (the staging buffer is the tile payload interface; the fold already ignores arrival order by construction). The frozen κ is realized by the static discipline (nothing can merge into the cell between capture and J(W)), not a runtime copy — the copy becomes necessary exactly when tiles run concurrently. Wave-internal termination is not proved (E0810·R, the runtime contract — Rice, honestly relocated). The mode analysis is name-based on the module-scope binding: an aliased read (`ptr p = s; lcell_value(p)`) slips past the read_S refusal — in slice-1 that alias reads the same frozen value (single tile, no concurrent merges), so the residual is a discipline hole, not a soundness hole; it becomes load-bearing only under the parallel form (where the snapshot closes it by construction). Benchmarked in `bench/RESULTS_WAVE.md`; runtime mechanics in `tests/systems/test_wave.c` (checks 21/21b); the compile-time arms in `examples/wave_*.sn` + `compiler/tests/golden.rs`.

## 28. SFC Layouts — A2's space-filling-geometry decision table, executable (implemented)

**`morton list<T, dims=[n1, n2]>`** and **`hilbert map<K, V>`** bake the curve into the collection, not the algorithm (optimal space-filling cacheline locality across multi-dimensional traversals):

| Layout | The decision-table role |
|---|---|
| `morton list<T, dims=[n1, n2]>` | **tensor iteration** with per-element indexed access — stencil sweeps, recursive kernels (contiguous dyadic operands ⟹ no packing copies — Chatterjee ICS'99) |
| `hilbert map<K, V>` | **spatial range queries** — the key computed once per box and amortized over V elements (segment-advance iterators); the query direction is where hilbert's measured advantage lives (Moon et al. 2001 cluster counts; Kamel–Faloutsos VLDB'94) |
| `soa` (already landed) | **lexicographic axis-committed field-projected SIMD streaming** — the third leg of the table, retained unchanged |

```text
morton list<int, dims=[16, 16]> m = morton_new(16, 16)        // or morton_from_lex(src, 16, 16)
morton_set(m, i, j, v)            // addr = lane_base + interleave(i, j)·8, bounds-checked [E-SFC]
int v = morton_get(m, i, j)
int k = morton_index(i, j)        // the pure interleave; morton_x(k)/morton_y(k) invert it
int w = morton_scan_get(m, k)     // curve-order read: data[k] — the stride-1 scan
ptr b = morton_to_bytes(m)        // values only — zero fixups by construction
hilbert map<int, int> h = hilbert_new(3)          // the order k: the grid side is 2^k
hilbert_insert(h, x, y, v)        // append-in-key-order O(1) — the only insert discipline
int v = hilbert_get(h, x, y)      // binary search over the sorted lane; hilbert_has probes
int i = hilbert_box_first(h, x0, y0, x1, y1)       // the segment advance
while (i != -1) { use(hilbert_entry_val(h, i)); i = hilbert_box_next(h, x0, y0, x1, y1, i) }
```

**The runtime design.** Both layouts are lane-backed (E-GC-IN-LANE: never gc_track'd, never ARC'd — `sn_release` on either kind is a no-op backstop — and the checker rejects storing them into GC-managed holders). A morton is ONE contiguous lane reservation (header + the values in curve order); the code space covers a SQUARE power-of-two grid EXACTLY (every code in `[0, d1·d2)` is an in-grid cell — unequal power-of-two dims spread the code space past d1·d2 and are refused), so the **O(1)-carry iteration is the integer increment** — consecutive cells have consecutive codes; the code IS the linear index — and in-curve-order traversal is a stride-1 buffer scan (A2 §2.3: the curve costs nothing at L1 during traversal). A hilbert map's entries sit on a lane **sorted by key — the order IS the index**: exact point lookup is a binary search (no auxiliary hash), and the box query is the Tropf–Herzog/Lawder–King **segment advance**: the box's key range is computed exactly (the curve enters and exits any box on its boundary — connectedness — so min/max over the boundary cells IS the range; Θ(perimeter·k), the Lawder–King Θ(k) hix descent is the slice-2 refinement), **once per query and cached** (A2 §2.4: amortized over the yielded entries), and the advance touches only the in-range entries.

**The feature-dispatch obligation (C2.8).** The interleave/deinterleave is dispatched ONCE PER PROCESS: BMI2 PDEP/PEXT on x86-64 (`__builtin_cpu_supports("bmi2")`, `#if defined(__x86_64__)`-gated — the builtins do not compile on arm64), the ~10-op software shift-or path everywhere else (AArch64 has no BMI2 — the software path is what runs on Apple Silicon). `SN_SFC_FORCE_PATH=soft` forces the software path (the systems suite's equivalence probe runs the grid under both settings and diffs); forcing `bmi2` without the feature is the honest `[E-SFC-PATH]` fail-fast, never an illegal instruction. The per-access index price is C2.9's honest cost — measured in `bench/RESULTS_SFC.md` (~2 ns/op software-path).

**The checked rules.**

- **E-SFC (dims)** — the dims must be equal powers of two (a square grid: slice 1 maps it onto the code space exactly — no padding; unequal dims want the padded reservation / the hybrid 2^t tile floor of Chatterjee TPDS 13(11) 2002, the slice-2 form), and the constructor's literal dims must agree with the declared type (the dims are in the type). Slice 1 implements `morton list<int, ...>` — fixed-size int elements only, **no directory for fixed-size T** (the Morton-keyed directory lane for packed/variable-size elements is the slice-2 form) — and `hilbert map<int, int>` (the d=2 point key is the (x, y) int pair).
- **E-SFC (runtime)** — bounds-checked addressing, never wraps (the power-of-two grid has no padding cell to land in); `from_lex` requires the exact length; `from_bytes` checks the blob's dims header agrees with the type.
- **E-SFC-ORDER / E-SFC-DUP (runtime)** — out-of-order and duplicate inserts are honest slice-1 REFUSALS, fail-fast: the sorted lane IS the index and slice 1 does not silently sort; the diagnostics name the **rebuild-epoch policy** (A2 §3's P1/P3 flag) as the future form.
- **W-SFC-T5B (compile-time warning)** — the provable lexicographic shape (a counted `for` over i, a nested counted `for` over j, both indexing the morton list directly as `morton_get(m, i, j)`): *wrong-order iteration over SFC storage is Θ(N) misses, a factor B/w over the stride-1 curve scan* (A2 §3/T5(b)) — a WARNING, never an error (the program is legal, the layout is wasted; the guidance names the two cures: iterate the curve order, or use `soa` for axis-committed access). Curve-ordered iteration is shapeless here and stays silent.
- **E-GC-IN-LANE (stores)** — a lane-backed SFC collection cannot be pushed into a list or written into a list literal: the collector cannot trace lane memory and the holder's release would free lane bytes as heap.

**Position independence.** A morton holds VALUES ONLY — no absolute pointers anywhere in the layout — so `to_bytes`/`from_bytes` round-trips with ZERO fixups by construction (the `link<T, u16>` seam needs its O(N) self-containedness scan for exactly the self-relative bytes this layout does not have).

**The honest boundary (slice-1).** d=2 first — d=3 (the `0x9249…`/`0x2492…`/`0x4924…` magic-mask interleave) is the slice-2 boundary; pure Morton (the hybrid 2^t tile floor — Morton above the tile, row-major inside, Chatterjee TPDS 13(11) 2002 — is the documented slice-2 form for arbitrary dims and the padding discipline); the segment advance scans the full key range (the Lawder–King in-box-key jump is the slice-2 refinement for off-aligned boxes); out-of-order inserts refuse rather than rebuild (the rebuild-epoch policy is the future form); the `foreach` lowering shape is the explicit `hilbert_box_first`/`next` iterator intrinsic (the FUR-Hilbert compiler-generated curve loop — non-recursive, O(1) worst per iteration, arbitrary bounds — is the documented future lowering). Benchmarked in `bench/RESULTS_SFC.md` (the honest direction report: row-major wins the flat 5-point stencil — Abel–Mark's mean-neighbor-gap; the segment advance wins the query direction 26–122×; the from_lex reshape ≈ one build pass, the ICS'99 2–5% window); runtime mechanics in `tests/systems/test_sfc.c` (checks 22/22b); the compile-time arms in `examples/morton_*.sn` + `examples/hilbert_*.sn` + `examples/sfc_*.sn` + `compiler/tests/golden.rs`.

---

## 29. The Numeric Contract Layer: requires/ensures, the Decidable Ladder, and the Typed Boundary (implemented)

**`fn f(params) requires(cond) ensures(cond) { ... }`** — the numeric contract layer (Presburger decidable ladder, compile-time DBM zone discharge, and typed runtime boundary rules). NOTE: the `contract` keyword is the trait/interface system (§15) — the numeric layer's surface is the two clause heads. Multiple clauses are allowed and conjoin; the requires condition is checked at every call site (the caller proves it or the call degrades), the ensures condition is the function's own proof obligation over its parameters and `result` (the contextual return-value name, allowed in ensures only, not a keyword).

```text
fn get(list<int> xs, int i) -> int requires(0 <= i < xs.length()) {
    return xs[i]
}
fn clamp(int x) -> int requires(0 <= x <= 100) ensures(0 <= result <= 100) {
    return x
}
fn rem(int x, int m) -> int requires(m > 0) ensures(result < m) {
    return x % m          // discharged by the remainder-range axiom
}
```

Conditions are int-valued boolean expressions over the int parameters, `param.length()` on list parameters (the opaque len symbol), and — in ensures — `result`: literals, `+ -`, literal-scaled `*`, `%` and `/`, comparisons, `and/or/not`. **Comparison chains desugar**: the parser is left-associative (`0 <= i < n` would parse as `(0 <= i) < n`, a bool against an int), so inside a clause the standard mathematical reading is restored — `0 <= i and i < n` (the runtime lowering sees the chain semantics; that is what keeps a degraded assertion sound).

### The atom classifier (A4 §4 — a total function of atom shape × known bounds, stamped at extraction)

| Tier | Shape | Discharge |
|---|---|---|
| **L — linear** | ℤ-linear terms, `len()`/`capacity`-style opaque symbols, mod/div by literals | the facts table + DBM zones (the ladder's rungs (i)/(ii)); termination guaranteed |
| **D — divisibility ledge** | mod by a VARIABLE with a known interval | the remainder-range axiom ONLY in slice 1 (see below); the EPAD/Bel'tyukov–Lipshitz solver tier is a later rung — named, not claimed |
| **N — nonlinear** | variable×variable products (MRDP); div-by-variable quotient binding (`q = x / y` binds the product `q·y = x` — division, not mod, is the undecidability vector) | never dischargeable |

**E0701** — a Tier N atom inside a **`verified fn`** is a hard error, all-or-nothing by design: the diagnostic names the offending atom, the rule, and the bounded-degree redemption clause ("degrade is a total function of (atom shape × known bounds), not a blanket rule": DBM-bounded variables redeem by enumeration, univariate atoms by root bounds — a two-variable product has no bounded-degree redemption here). In a plain `fn` the same atom degrades to a typed runtime assertion (below). A site whose operands ALL fold to constants discharges by plain constant evaluation — sound for every tier.

### The engine (slice 1: the facts table over the DBM zone layer)

- **The facts table** (Go-prove-style — sound, but not complete): a per-function table of known facts: declared bounds (a local's literal/copy/mod initializer, valid until its first assignment — loop-entry facts are a later rung), the requires clauses propagated from call chains (the caller's own entry zone is the proof context the callee's atoms are tested against), literal-folded values, and list-literal lengths.
- **The DBM zone layer** (A4's rung (i): Floyd CACM 5(6):345 1962; Dill CAV'89 LNCS 407; Bengtsson–Yi LNCS 3098): difference-bound matrices over the function's int variables — `x − y ≤ c` edges, with vertex 0 the zero constant, so interval facts (`x ≤ c`) are zero-var difference edges; Floyd–Warshall all-pairs closure (O(V³) — the pipeline floor, §7.3: "closure is the floor, not the budget"); emptiness ⟺ a negative diagonal cycle (Bellman 1958); **∩ exact but requiring re-closure for emptiness detection, and ∪ NOT closed — zones are convex** (Miné HOSC 19(1) Thms 4.1/4.2, A4 C4.1's correction): the branch/return-site merge is the OVER-APPROXIMATING lattice join only (the pointwise max of closed matrices) — exact unions need DBM federations (named, not claimed). The closure is the classic index-vs-len discharge: `i ≤ 2` + `len = 3` closes to `i − len ≤ −1`.
- **The memoized fast path** (§7.3): the entry zone is closed once per function and every call site inside reads the closed matrix; two sites with identical fact signatures share one closure (the signature-keyed memo). Benchmarked in `bench/RESULTS_CONTRACT.md`: the memo hit is ~4 ns against ~21,200 ns for the naive re-closure (~5,300×); the per-atom discharge test is an O(1) edge lookup (~2 ns).

### The remainder-range axiom (Tier D) — stated for the semantics that actually run

For `r = x % m` with m's interval known positive, the engine injects **two DBM edges given m's interval**: `r − m ≤ −1` (r < m, against the modulus VERTEX) plus the interval edges from m's upper bound — **a RANGE FACT, never a divisibility solve** (Strata's rule). NOTE the honest deviation: Strata's Euclidean-% form is `0 ≤ r < m`; this language's runtime `%` is C-**truncating** (`sn_mod_i64` — r has the dividend's sign), so the axiom here is stated as `−(m−1) ≤ r ≤ m−1` for m > 0 — discharging `0 <= x % m` under the Euclidean constants would be UNSOUND against the semantics that run, and it correctly does NOT follow (it degrades unless the dividend's non-negativity is itself a fact).

### The boundary rule (executable)

- **Discharged** — the engine proves the atom from the caller's facts (or the fn's own entry facts, at the ensures exit): compile-time OK, nothing lowered, no warning.
- **DEGRADED — W-CONTRACT** — the engine cannot prove it: a WARNING (never a gate) names the atom, its tier, and where the typed runtime assertion now sits (the call site for requires — or the fn entry, for atoms over defaulted-out params — and every return, for ensures), and the assertion is lowered through the fail-fast convention (`sn_contract_assert`: fprintf + exit(1)) with the [E-0702] message carrying the atom text and the typed classification. **Never silently dropped; priced per atom.** A REFUTED atom (the engine proved the negation — the conjoined atom closed to a negative diagonal) says so in both the warning and the runtime message: the assertion will always fail there.
- At the fn exit, the ensures test runs against the JOINED exit zone (per-return-site result bindings — `result := x`, `result := c`, `result := x % m` via the axiom — joined per Miné Thm 4.2, with the fall-through exit modeled as `result := 0`); a coefficient-2 binding (`result := x * 2`) is NOT a difference bound and honestly degrades (the octagon rung is later).

**The honest slice-1 boundary.** Contracts live on top-level non-generic functions (clauses on generic/abstract/nested fns and methods are refused, never silently dropped). The discharge test covers the zone shapes — the interval bound (x ≤ c), THE DIFFERENCE BOUND (x − y ≤ c), equalities, constant folds, and the mod range axiom; multi-variable sums, |coeff| ≠ 1 bindings, `!=` and `or` (non-convex — parity is not a zone), div-quotient terms, and loop-carried values honestly degrade. The **QF-PA solver (rung iii — NP-complete, poly at fixed #vars)** and the **EPAD/Z3 oracle tiers** are LATER RUNGS — named, not claimed; so are the octagon rung (ii), loop-entry facts, and bounded-degree redemption by enumeration. Pinned by the golden suite (`examples/contract_*.sn` + `compiler/tests/golden.rs` — the discharge, the degrade, the refutation, the E0701 product/quotient arms, the typed nonlinear degrade, all verbatim); the engine's math properties by the `dbm` unit tests + `tests/systems/test_contract.c` (checks 23/23b — the line-matched C twin: the closure vs the brute-force relax-to-fixpoint reference over randomized zones, emptiness, ∩ exact by enumeration, the join over-approximates sound-not-exact, the facts-table hit/miss, the fork-probed [E-0702] fail-fast); the price by `bench/RESULTS_CONTRACT.md` (the O(V³) floor at V=25/50/100, the memoized fast path's payoff, the ~0.96 ns/call runtime price of the degrade against zero for the discharge).

---

## 30. Tape Record Kinds — W2C's P5↔P2 elevation plan, executable (implemented)

**`tape<int> t = tape_new("name")`** — the append-only **$\delta_0$-chronicle**: discrete impulses against an integrated state, ordered by an execution clock, where replay equals re-integration from a base checkpoint.

**The kinship, ruled honestly (W2C §36):** the tape's SHAPE is the δ₀-chronicle's — true as stated. The ALGEBRA is not: no group on opaque entries (a free monoid at best — differentiation vacuous), no ℐ/𝒟 pair, no chain rule. The elevation is **tier-parametric**: what the tape's records can DO depends on what the recorded values ARE, and the compiler classifies before the first record is written.

```text
fn main() {
    tape<int> t = tape_new("gz")

    ptr x0 = zset_new(0)                 // the Tier-G carrier (ℤ[X], exact ⊖)
    ptr x1 = zset_new(0)
    zset_add(x1, 10, 3)

    tape_record(t, 1, x0, x1)            // DELTA: the (prev → next) pair — the law CHECKED here
    tape_record(t, 2, x1)                // CHECKPOINT: the state snapshot (the re-integration base)

    ptr r = zset_new(0)
    tape_replay(t, r)                    // replay == re-integration: base ⊕ the window
    print(zset_count(r, 10))             // 3 — the direct computation, identically

    tape_record(t, 0, 42)                // OPAQUE: the raw chronicle (no algebra, no law)
    print(tape_len(t))                   // 3
    print(tape_rec_payload(t, 2))        // 42 — the opaque readback
}
```

**The three record kinds** (`tape_record(t, kind, ...)` — the kind is a compile-time literal):

| Kind | Record | The law, checked at the effect boundary (the record IS the boundary) | Replay |
|---|---|---|---|
| **0 — opaque** (the default) | a raw int payload | none — W2C §36's free-monoid baseline; differentiation vacuous | **refused** in the window ([E-TAPE]: an opaque record has no integral to contribute) |
| **1 — delta** | the `(prev → next)` carrier pair | **Tier G**: d = b ⊖ a derived (`sn_zset_delta`), ILC's **a ⊕ (b ⊖ a) = b** verified by reconstruction (apply + compare) before the impulse is accepted; stored as raw (key, count) pairs — E2's embeddability LITERAL (the delta records embed in the ℤ[X] multiplicity carrier). **Tier L**: m^δ = `sn_lcell_delta` (the join-decomposition-optimal fragment), the mutator law **X ⊔ m^δ(X) = m(X)** verified (apply + `sn_lcell_eq`); NO ⊖ exists on this path (the E-LRET precedent). **Tier O**: REFUSED — no delta-local law at any price; the tape falls back to opaque (E2's honest fallback) | Tier G: ⊕-fold of the pairs. Tier L: ⊔-fold of the offers (inflate only) |
| **2 — checkpoint** | the carrier state snapshot | the snapshot IS the base (no law to check; a Tier-O value is refused — no carrier to snapshot) | resets the integration base; `tape_replay` folds ONLY the records after the LAST checkpoint |

**`tape_replay(t, target)` — replay = re-integration, tier-parametrically.** **Tier G**: the target is reset to the zero state (the group inverse — Tier-G's exact capability), the last checkpoint's snapshot pairs apply, then every delta impulse in the window; the fold is idempotent by exactness, and the no-checkpoint fold from ∅ is the accumulated change (exact when the chain is rooted at ∅). **Tier L**: the base's offers and the window's fragments ⊔-join into the target (inflate only — the LVar discipline; idempotent by ⊔-idempotence); **without a checkpoint the replay is REFUSED** ([E-TAPE]) — the fragments are state-relative (the mutator law joins m^δ onto ITS base), and a ⊥-based fold would silently under-approximate. The honest asymmetry: **Tier-G deltas are absolute (a group difference); Tier-L fragments are state-relative (the semilattice's price — no ⊖, E-LRET).**

**The tier classification (per record kind, at compile time).** The checker classifies a delta record's endpoints as a pure function of their origin (W2C §1.1/§38 E2): **Tier G** — the `zset_*` constructors (`zset_new/copy/delta/apply` initializers) or a `tape_delta(G)` extern's results; **Tier L** — the `lcell_*` constructors or a `tape_delta(L)` extern's results; **Tier O** — everything else (an int, an unannotated extern's results, an unclassified binding): the delta and checkpoint kinds are REFUSED ([E-TAPE], verbatim), the tape falls back to opaque. The classification is the DECLARED ORIGIN (the initializer); the runtime's kind guard is the backstop (a raw-ptr tier mix dies [E-TAPE] at the boundary — the E-LKIND pattern).

**E1 — typed extern deltas (the signature discipline).** `extern "lib" { fn feed(int step) -> tape_delta(G) }` declares the delta type of the extern's RESULTS: the feed's results are delta-chain endpoints of the tier's carrier (the CDC-feed shape — the change between consecutive results is a ℤ[X] element / a Tier-L fragment). Unannotated externs keep **P2's default-unpure OPAQUE form** — the §4 kinship activates exactly when the annotation exists. The compile-time check is the SIGNATURE discipline (the classification above); the ⊕/⊖ laws are checked at the effect boundary on every recorded triple (a lying producer dies there — the tripwire). `tape_delta` in a PARAMETER position is the honest slice-1 refusal (a delta-consuming callee's assumptions are the E3 boundary, below).

**The boundary rules (executable).**
- **ONE TAPE, ONE LEDGER** — the P2↔wave boundary rule extended to P5 (W2C §4's E4): the first delta/checkpoint record fixes the tape's (tier, carrier); a record against a different carrier, or a replay into a mismatched tier's target, dies [E-TAPE] at the effect boundary (the runtime backstop; the chronicle must stay one linear ledger of one carrier's evolution).
- **E-TAPE-WAVE** — the whole tape surface (records, replay, reads) is refused inside a wave body: the wave stages its own effects (contrib → J(W), NI(ii) "ports don't escape") and the tape's chronicle must stay one linear ledger of the fallback state; record and read program-order-outside the wave.
- **E-GC-IN-LANE** — the tape is lane-backed (the hilbert precedent: the header, the record directory, and the flat payload buffer live on lane 0, never gc_track'd, `sn_release` a no-op backstop, refused in GC-managed holders); the records hold RAW int64 pairs only — no GC pointer ever crosses into the chronicle (the derived d/m^δ objects are built, law-checked, flattened onto the lane, and released inside the record call).

**The honest slice-1 boundary.** Slice 1 implements `tape<int>` (the payloads are int64 pairs throughout — other element types are the named boundary) and **function-local** chronicles (a module-scope tape wants the DIFF-4 slot pattern — the named slice-2 form; refused, never silently lowered). **E3 — the chain rule** (transporting typed deltas through derived views, making the tape an impulse stream *in the calculus* — W2C §38's ingredient (3)) is the **named slice-2 boundary**: slice 1 records and replays typed deltas against the LANDED carriers; it does not compose them through user views. **No user-body delta certifier** (W2C §6.4's Propel-style checked-not-proven class) and **no new incrementalization mathematics** (the final report §395 item 11): the compiler's job is classification — which records carry a law, which tier's law it is, and what the replay may honestly claim. Benchmarked in `bench/RESULTS_TAPE.md` (the typed record's ~190× premium over the opaque append — the premium IS the law check; the checkpoint's window economics: O(window) replay vs the O(K) re-scan); runtime mechanics in `tests/systems/test_tape.c` (checks 24/24b — the ILC law on randomized Tier-G triples, the mutator law on all three Tier-L carriers, the replay == re-integration witness on randomized streams with mid-chain checkpoints, the opaque refusal, the one-tape-one-ledger/no-base/tier backstops, fork-probed verbatim); the compile-time arms in `examples/tape_*.sn` + `compiler/tests/golden.rs` (the kind-literal rule, the Tier-O/mixed refusals, the E1 signature discipline, the E-TAPE-WAVE boundary rule, the E-LRET-adjacent ⊖ refusal — all verbatim).


---

## 31. Bare-Metal Kernel Expansion: MMIO Type Semantics, Freestanding Mode, and Extended Inline Assembly

### 1. `volatile<T>` / `mmio<T>` Type Semantics
- **Type Syntax**: `mmio<u8>`, `mmio<u16>`, `mmio<u32>`, `mmio<u64>` (restricting `T` to primitive scalar widths). `volatile<T>` aliases `mmio<T>`. Non-scalar arguments (aggregates, records, strings, lists) are rejected with `[E-MMIO-INVALID-TYPE]`.
- **Operations**:
  - `reg.read() -> T`: volatile load of width $\mathrm{sizeof}(T)$ from the bound address.
  - `reg.write(val: T) -> void`: volatile store of width $\mathrm{sizeof}(T)$ to the bound address.
  - `reg.address() -> u64`: underlying memory address introspection.
  - Implicit dereference (`*reg`), direct assignment (`reg = val`), and compound arithmetic (`reg += val`) are rejected at compile time.
- **Escape Analysis (`E-MMIO-ESCAPE`)**:
  - Taking references (`&reg`, `ref<mmio<T>>`, `mut<mmio<T>>`, `ptr`) is strictly rejected at compile-time (`E-MMIO-ESCAPE`).
  - Device registers cannot be declared as fields of GC-managed blueprints (`new Blueprint()`), nor captured across escaping heap closures.
  - Cannot be captured or transferred across threads in `spawn`, `go`, or `chan<T>` without explicit mutual exclusion.
- **LLVM Lowering**:
  - Lowers to LLVM `ptr`.
  - Emits `load volatile <iN>, ptr %addr, align <N/8>` and `store volatile <iN> %val, ptr %addr, align <N/8>`.
  - Sequential consistency among volatile operations within each thread is preserved; LLVM optimizer passes (`DSE`, `GVN`, `LICM`) are forbidden from eliding, coalescing, or hoisting volatile accesses.

### 2. GC Link-Out Rule in `--freestanding` Mode
- **Compilation Flag**: `--freestanding` compiles kernel binaries without standard C runtime, POSIX pthreads, or the garbage collector runtime.
- **Compile-Time Diagnostic**: Any call to `new T()` is rejected at compile-time with:
  `E-NO-GC-IN-FREESTANDING: dynamic GC heap allocation 'new T()' is forbidden in --freestanding mode. Use stack allocation, 'region { }' elastic arenas, pinned lanes, or 'std.pool'.`
- **Zero GC Symbols**: LLVM IR emission conditionally suppresses all `sn_gc_*` declarations and calls. Binaries link with `runtime_freestanding.c`, guaranteeing zero unresolved GC symbols.

### 3. Extended Inline Assembly `asm { ... }`
- **Syntax**:
  ```snlang
  asm {
      "<template>"
      in("<constraint>") <expr>,
      out("<constraint>") <lvalue>,
      inout("<constraint>") <ident>,
      clobber("<reg>" | "memory" | "cc")
  }
  ```
- **Register Constraints**: Supports `"rax"`, `"rbx"`, `"rcx"`, `"rdx"`, `"rsi"`, `"rdi"`, general register `"r"`, memory `"m"`, immediate `"i"`, and subregisters (`"al"`, `"ax"`, `"eax"`, `"dl"`, etc.).
- **Operand Bitwidth Validation**: Mismatch between operand bitwidth and constraint register width produces compile-time diagnostic `[E-ASM-OPERAND-WIDTH]`.
- **Volatile Emission**: Lowers to LLVM `call void asm sideeffect "<template>", "<constraints>"(...)`. The `sideeffect` attribute guarantees immunity from dead-code elimination and speculative hoisting; `clobber("memory")` acts as an LLVM compiler memory fence.


---

## 32. Bare-Metal Kernel Expansion: The Novel `irq fn` Static Interrupt Safety Discipline

### 1. Mathematical Typing Invariants & Interrupt Execution Context
- **Interrupt Context**: Hardware interrupt service routines (`irq fn`) execute asynchronously at elevated hardware Interrupt Priority Levels ($\mathrm{ipl} \ge 2$), interrupting arbitrary instructions on a core. The context has no schedulable Task Control Block ($\mathrm{tid} = \bot$) and operates on a dedicated, bounded Interrupt Stack Table (IST) frame.
- **Prohibition of Re-entrancy & State Inconsistency**: Hardware interrupts are non-reentrant for their vector until End-of-Interrupt (EOI) signaling. Access to shared mutable kernel state must be protected by wait-free atomic primitives.
- **Prohibition of Allocation Deadlock (Theorem 1)**: Preempting a thread while it holds an internal allocator spinlock or metadata, followed by dynamic allocation inside the ISR on the same core, produces an unrecoverable uniprocessor self-deadlock ($c \to c$). Dynamic allocation is strictly prohibited.
- **Prohibition of Sleep-in-Atomic (Theorem 2)**: Schedulers require an active TCB ($\mathrm{tid} \neq \bot$) to suspend and yield. Blocking or sleeping inside an ISR results in undefined behavior, hardware interrupt mask freeze, or catastrophic IST stack corruption.

### 2. The Three Static Invariants Checked at Compile-Time
1. **`E-IRQ-ALLOC` — Static Prohibition of Dynamic Memory Allocation**:
   - Rejects all heap allocations (`new T()`), dynamic string concatenation, array resizing/growing, and unbounded closures.
   - Rejects elastic lane expansion that can invoke the external page memory provider (`sn_set_memory_provider`).
   - Permitted: Lexical stack variables (subject to static stack consumption bound $S(f) \le \mathrm{LIMIT}_{\mathrm{IST}}$) and pre-allocated, bounded pinned-lane scratchpad memory ($\pi_{\mathrm{irq}}$) restored via `region { }` without dynamic page requests.
2. **`E-IRQ-BLOCK` — Static Prohibition of Blocking Synchronization**:
   - Rejects acquiring sleeping mutexes (`std.sync.Mutex`, `pthread_mutex_t`), channel operations (`chan.send()`, `chan.recv()`), coroutine yielding (`await`), semaphores, condition variables, and `thread.sleep()`.
   - Rejects unbounded spinlocks over non-atomic variables.
3. **`E-IRQ-INTERPROC` — Interprocedural Purity Propagation**:
   - Constructs the call graph $\mathcal{G} = (V, E)$ from all `irq fn` entrypoints ($\mathcal{V}_{\mathrm{irq\_roots}}$).
   - Computes transitive effect closure $\mathcal{E}^*(u) = \mathcal{E}_{\mathrm{local}}(u) \cup \bigcup_{(u, v)} \mathcal{E}^*(v)$ via Tarjan's SCC and Knaster-Tarski fixpoint iteration.
   - Any function transitively reachable from an `irq fn` that performs an allocating or blocking operation is rejected with `[E-IRQ-INTERPROC]`, detailing the complete call trace $f_{\mathrm{irq}} \to g_1 \to \dots \to v$.
   - Function pointers invoked in `irq fn` must explicitly carry the `irq fn(...) -> T` capability; passing unannotated `fn` pointers is rejected with `[E-IRQ-CAPABILITY]`. Unbounded recursion cycles are rejected with `[E-IRQ-RECURSION]`.

### 3. Concurrency Integration & Permitted Primitives
- **Lock-Free Atomics**:
  - `AtomicBool`, `AtomicU8`, `AtomicU16`, `AtomicU32`, `AtomicU64`, `AtomicInt`, `AtomicPtr`.
  - Operations: `.load(order)`, `.store(val, order)`, `.fetch_add`, `.fetch_sub`, `.fetch_or`, `.fetch_and`, `.fetch_xor`, `.compare_exchange_weak`, `.compare_exchange_strong`.
  - Supported orderings: `Relaxed`, `Acquire`, `Release`, `AcqRel`, `SeqCst`.
  - Lowering: Zero-lock hardware atomic instructions (`lock cmpxchg`, `lock xadd`, `lock or`, etc.), guaranteed wait-free $\mathcal{O}(1)$.
- **Wait-Free SPSC Ring Buffer (`SpscRing<T, const N: u64>`)**:
  - Statically pre-allocated buffer of power-of-two capacity $N = 2^k$.
  - Atomic `head` written exclusively by `irq fn` (Producer); atomic `tail` written exclusively by thread/worker (Consumer).
  - Wait-free `.push(item: T) -> bool` executes in bounded step count without loops or locks; returns `false` on buffer overrun.
  - Provably free of torn reads and overwrites via Acquire-Release memory order semantics.
- **Deferred Bottom-Half Signaling (SoftIRQ)**:
  - Top-half `irq fn` acknowledges hardware (e.g., LAPIC EOI), pushes events to SPSC ring, and sets an atomic bit in a global pending mask (`softirq_pending.fetch_or(1 << IRQ_ID, MemoryOrder.Release)`).
  - The kernel interrupt exit trampoline inspects `softirq_pending` and dispatches deferred bottom-half workers (`softirq fn`) at $\mathrm{ipl} = 1$ with interrupts enabled, where heavier processing safely occurs.
- **LLVM Lowering**:
  - `irq fn` lowers with LLVM calling convention `x86_intr` (`x86_intrcc`) and attribute `"no-red-zone"="true"`.
  - Automatically preserves all touched CPU scratch registers and terminates with `iretq` (x86_64) or `eret` (AArch64).

---

## 33. Formal Grammar Specification (EBNF)

This section provides the complete, authoritative Context-Free Grammar for SNlang 0.3.1 specified in Extended Backus–Naur Form (EBNF).

### 33.1 Notation & Conventions
* `Symbol ::= Definition` defines a non-terminal symbol.
* `"token"` denotes a literal terminal keyword, operator, or punctuation mark.
* `[ Item ]` denotes an optional item (0 or 1 occurrence).
* `{ Item }` denotes 0 or more occurrences.
* `Item*` denotes 0 or more occurrences; `Item+` denotes 1 or more occurrences.
* `A | B` denotes an alternative choice.
* `( ... )` groups elements.

---

### 33.2 Lexical Grammar

```ebnf
Letter              ::= [a-zA-Z_]
Digit               ::= [0-9]
HexDigit            ::= [0-9a-fA-F]

Ident               ::= Letter ( Letter | Digit )*
IntegerLiteral      ::= Digit+ | "0x" HexDigit+ | "0b" [01]+
FloatLiteral        ::= Digit+ "." Digit+
DecimalLiteral      ::= Digit+ "." Digit+
StringLiteral       ::= '"' ( [^"\\] | "\\" . )* '"'
ByteLiteral         ::= "0x" HexDigit HexDigit

SingleLineComment   ::= "//" [^\n]*
MultiLineComment    ::= "/*" ( [^*] | "*"+ [^*/] )* "*"+ "/"
```

#### Reserved Keywords
```text
abstract, and, any, as, asm, async, await, blueprint, bool, byte, chan,
closed, const, contract, dec, default, defer, else, ensures, enum, errdefer,
error, extern, float, fn, for, from, frozen, guard, guarded, handle, hilbert,
if, in, int, i8, i16, i32, irq, let, link, list, lock, map, match, mmio,
morton, mut, new, none, not, nursery, or, otherwise, packed, pool, ptr,
record, ref, region, requires, return, skip, spawn, static, stop, str,
tape, tape_delta, try, unfrozen, u8, u16, u32, u64, use, verified, wave, while
```

---

### 33.3 Syntactic Grammar

#### Program & Items
```ebnf
Program             ::= Item*

Item                ::= UseItem
                      | FnItem
                      | BlueprintItem
                      | RecordItem
                      | EnumItem
                      | ContractItem
                      | ExternItem
                      | ExtensionItem
                      | Stmt

UseItem             ::= "use" Ident ( "." Ident )*
```

#### Function Declarations & Contracts
```ebnf
FnItem              ::= FnModifier? "fn" Ident [ "<" TypeParams ">" ] 
                        "(" ParamList? ")" [ "->" Type ] 
                        ContractClause* Block

FnModifier          ::= "verified" | "async" | "irq"

ParamList           ::= Param ( "," Param )*
Param               ::= Type Ident [ "=" Expr ]

ContractClause      ::= "requires" "(" Expr ")"
                      | "ensures" "(" Expr ")"
```

#### Object-Oriented Definitions
```ebnf
BlueprintItem       ::= "blueprint" Ident [ "<" TypeParams ">" ] [ "from" Ident ] 
                        "{" BlueprintMember* "}"

BlueprintMember     ::= FieldDecl
                      | MethodDecl

FieldDecl           ::= [ "closed" | "guarded" ] Type Ident [ "=" Expr ]

MethodDecl          ::= [ "closed" | "guarded" | "static" | "abstract" ]
                        "fn" Ident [ "<" TypeParams ">" ] 
                        "(" ParamList? ")" [ "->" Type ] 
                        ContractClause* ( Block | ";" )

RecordItem          ::= [ "packed" ] "record" Ident "{" FieldDecl* "}"

EnumItem            ::= "enum" Ident "{" EnumVariant ( "," EnumVariant )* ","? "}"
EnumVariant         ::= Ident [ "(" Type Ident? ")" ]

ContractItem        ::= "contract" Ident "{" ContractMethod* "}"
ContractMethod      ::= "fn" Ident "(" ParamList? ")" [ "->" Type ]

ExtensionItem       ::= "extension" Ident "{" MethodDecl* "}"

ExternItem          ::= "extern" StringLiteral "{" ExternFnDecl* "}"
ExternFnDecl        ::= "fn" Ident "(" ParamList? ")" [ "->" Type ]

TypeParams          ::= TypeParam ( "," TypeParam )*
TypeParam           ::= [ "out" | "in" ] Ident [ ":" Ident ]
```

#### Types
```ebnf
Type                ::= TypeBase [ "?" ]

TypeBase            ::= PrimitiveType
                      | FixedPointType
                      | CollectionType
                      | SpatialType
                      | ChronologyType
                      | PointerType
                      | ConcurrencyType
                      | HardwareType
                      | FunctionType
                      | TupleType
                      | NamedType

PrimitiveType       ::= "int" | "i8" | "i16" | "i32" 
                      | "u8" | "u16" | "u32" | "u64"
                      | "float" | "str" | "bool" | "byte" | "error" | "any"

FixedPointType      ::= "dec" "(" IntegerLiteral ")"

CollectionType      ::= "list" "<" Type ">"
                      | "map" "<" Type "," Type ">"

SpatialType         ::= "morton" "list" "<" Type "," "dims" "=" "[" IntegerLiteral ( "," IntegerLiteral )* "]" ">"
                      | "hilbert" "map" "<" Type "," Type ">"

ChronologyType      ::= "tape" "<" Type ">"
                      | "tape_delta" "(" ( "G" | "L" ) ")"

PointerType         ::= "ptr"
                      | "ref" "<" Type ">"
                      | "mut" "<" Type ">"
                      | "pool" "<" Type ">"
                      | "handle" "<" Type ">"
                      | "link" "<" Type [ "," "u16" ] ">"

ConcurrencyType     ::= "chan" "<" Type ">"

HardwareType        ::= "mmio" "<" Type ">"

FunctionType        ::= "fn" "(" [ Type ( "," Type )* ] ")" [ "->" Type ]

TupleType           ::= "(" Type ( "," Type )+ ")"

NamedType           ::= Ident [ "<" Type ( "," Type )* ">" ]
```

#### Statements & Regional Scopes
```ebnf
Block               ::= "{" Stmt* "}"

Stmt                ::= StmtIf
                      | StmtWhile
                      | StmtFor
                      | StmtMatch
                      | StmtReturn
                      | StmtGuard
                      | StmtRegion
                      | StmtNursery
                      | StmtWave
                      | StmtFrozen
                      | StmtUnfrozen
                      | StmtStop
                      | StmtSkip
                      | StmtSpawn
                      | StmtLock
                      | StmtDefer
                      | StmtErrdefer
                      | StmtAsm
                      | StmtNew
                      | StmtDecl
                      | StmtAssign
                      | StmtExpr

StmtIf              ::= "if" "(" Expr ")" Block ( "else" "if" "(" Expr ")" Block )* [ "else" Block ]
StmtWhile           ::= "while" "(" Expr ")" Block
StmtFor             ::= "for" "(" StmtDecl "," Expr "," StmtAssign ")" Block
                      | "for" Ident "in" Expr Block

StmtMatch           ::= "match" "(" Expr ")" "{" MatchArm* "}"
MatchArm            ::= Pattern Block
Pattern             ::= Ident [ "." Ident [ "(" Ident ")" ] ]
                      | "none"
                      | "default"

StmtReturn          ::= "return" [ Expr ( "," Expr )* ] [ "from" Ident ]
StmtGuard           ::= "guard" [ "(" ] Expr [ ")" ] ( "otherwise" | "else" ) Block

StmtRegion          ::= "region" Block
StmtNursery         ::= "nursery" Block
StmtWave            ::= "wave" Block
StmtFrozen          ::= "frozen" "(" Ident ( "," Ident )* ")" Block
StmtUnfrozen        ::= "unfrozen" "(" Ident ")" Block

StmtStop            ::= "stop"
StmtSkip            ::= "skip"

StmtSpawn           ::= "spawn" ( Block | CallExpr )
StmtLock            ::= "lock" [ "(" Ident ")" ] Block

StmtDefer           ::= "defer" Block
StmtErrdefer        ::= "errdefer" Block
StmtAsm             ::= "asm" StringLiteral [ ":" StringLiteral ]

StmtNew             ::= [ "new" ] Ident Ident "(" ArgList? ")"
StmtDecl            ::= ( "let" | "const" ) Ident [ ":" Type ] "=" Expr
                      | Type Ident ( "," Ident )* [ "=" Expr ( "," Expr )* ]
StmtAssign          ::= AssignLValue AssignOp Expr
AssignOp            ::= "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^="

AssignLValue        ::= Ident
                      | Expr "." Ident
                      | Expr "[" Expr "]"
                      | "*" Expr

StmtExpr            ::= Expr
```

#### Expressions & Precedence Ladder
Expressions follow strict precedence order (lowest to highest):

| Precedence | Associativity | Category | Operators / Syntax |
| :---: | :---: | :--- | :--- |
| **1** | Left | Null Coalescing | `otherwise` |
| **2** | Left | Logical Disjunction | `or` |
| **3** | Left | Logical Conjunction | `and` |
| **4** | Left | Bitwise OR | `\|` |
| **5** | Left | Bitwise XOR | `^` |
| **6** | Left | Bitwise AND | `&` |
| **7** | None | Comparison Chains | `==`, `!=`, `<`, `<=`, `>`, `>=` |
| **8** | Left | Additive | `+`, `-` |
| **9** | Left | Multiplicative | `*`, `/`, `%` |
| **10** | Right | Exponentiation | `**` |
| **11** | Right | Unary Prefix | `-`, `not`, `~`, `try`, `*` (deref) |
| **12** | Left | Postfix | `(args)` (call), `[index]` (index), `.field` (access), `!!` (non-null assertion) |
| **13** | — | Primary | Ident, Literals, `none`, `result`, Parenthesized `(Expr)` |

```ebnf
Expr                ::= OtherwiseExpr
OtherwiseExpr       ::= OrExpr [ "otherwise" OrExpr ]
OrExpr              ::= AndExpr ( "or" AndExpr )*
AndExpr             ::= BitOrExpr ( "and" BitOrExpr )*
BitOrExpr           ::= BitXorExpr ( "|" BitXorExpr )*
BitXorExpr          ::= BitAndExpr ( "^" BitAndExpr )*
BitAndExpr          ::= CmpExpr ( "&" CmpExpr )*

CmpExpr             ::= AddExpr [ CmpOp AddExpr ( CmpOp AddExpr )* ]
CmpOp               ::= "==" | "!=" | "<" | "<=" | ">" | ">="

AddExpr             ::= MulExpr ( ( "+" | "-" ) MulExpr )*
MulExpr             ::= PowExpr ( ( "*" | "/" | "%" ) PowExpr )*
PowExpr             ::= UnaryExpr [ "**" PowExpr ]

UnaryExpr           ::= ( "-" | "not" | "~" | "try" | "*" ) UnaryExpr
                      | PostfixExpr

PostfixExpr         ::= PrimaryExpr ( CallSuffix | IndexSuffix | MemberSuffix | "!!" )*
CallSuffix          ::= "(" ArgList? ")"
IndexSuffix         ::= "[" Expr "]"
MemberSuffix        ::= "." Ident

PrimaryExpr         ::= Ident
                      | IntegerLiteral
                      | FloatLiteral
                      | DecimalLiteral
                      | StringLiteral
                      | ByteLiteral
                      | "true"
                      | "false"
                      | "none"
                      | "result"
                      | ListLiteral
                      | MapLiteral
                      | TupleLiteral
                      | "(" Expr ")"

ListLiteral         ::= "[" [ Expr ( "," Expr )* ] "]"
MapLiteral          ::= "{" [ Expr ":" Expr ( "," Expr ":" Expr )* ] "}"
TupleLiteral        ::= "(" Expr ( "," Expr )+ ")"
ArgList             ::= Expr ( "," Expr )*
```
