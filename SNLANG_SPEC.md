# SNlang — The SafarNow Language

**Version:** 0.2  
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
4. **Thread Isolation in `spawn`:** Passing unsynchronized mutable references or unlocked blueprints across threads in `spawn` is blocked at compile time to prevent data races.

See `tests/memory_safety/` for executable exploit test suites and comparison with Rust.

### High-Performance Systems Allocators

SNlang includes zero-overhead native allocators for high-throughput systems programming:

1. **Generational Pools (`std.pool` / `pool<T>` / `handle<T>`):**
   - Implements 64-bit generation-indexed slotmaps: 32-bit slot index + 32-bit generation counter.
   - Provides O(1) allocation, retrieval, and deallocation.
   - Immune to the ABA problem and dangling pointers: accessing a deleted handle is detected immediately (`is_valid(h) == false`), preventing use-after-free bugs without garbage collector pauses.

2. **Elastic Lane Arenas (`region { ... }` / `std.arena`):**
   - High-speed bump allocation backed by 256 MB virtual memory reservations per lane.
   - Scope-bound `region { }` automatically captures the lane watermark on entry and rewinds it on exit, delivering instant O(1) bulk deallocation.
   - **Scope Unwind Guarantee:** Early returns (`return`) or errors (`try`) inside a `region { }` automatically restore the lane watermark, ensuring zero memory leaks on early exit paths.
   - `frozen { }` blocks lock lanes into read-only mode to prevent accidental allocation or mutation.
   - **Inline lowering** (`-O1+`): constant-size lane allocations compile to a 3-instruction bump (load top, bump, store) with a guard branch routing to the loud runtime path; region exit is a 1-store watermark restore.
   - **T-FIBR poison-fill** (`--poison` / `-O0` / debug builds): lane bytes are filled with `0xAA` on allocation and re-poisoned on rewind — the read-before-write guard (E0807), unconditional across all codegen paths. Release builds emit zero extra instructions.
   - **Pinned-lane accumulator coloring** (compiler-assigned): recursive `inout` list parameters that grow across a recursive call are automatically assigned to dedicated pinned lanes (L2–L6) — each pinned lane hosts exactly one growable for its entire SCC lifetime, so the accumulator is never buried under descendant scratch. The 6th simultaneously-live accumulator fires **E0802c** (the budget error — never silent reuse). An explicit `lane_alloc` on a pinned lane is the user-declared discharge; scratch lanes (L0/L1) for SCC accumulators fire **E0802**.
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
5. **Salt ζ — the Fixpoint Bracket + the Admission Checker (`std.zeta` / `ZFix`):**
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

