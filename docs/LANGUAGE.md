# Language tour

SNlang looks like a teaching language and compiles to native code via LLVM + clang.

## Hello

```sn
fn main() {
    str name = "Ada"
    print("hello {name}")
}
```

## Types

`int`, `bool`, `byte`, `str`, `dec(X)`, `list<T>`, `map<K,V>`, `T?`, `none`, `error`,
`chan<T>`, `ref<T>`, `lock`, `json`, `any`, `record`, function types.

Null is opt-in: `str name` cannot be `none`; `str? nickname` can.

```sn
str? nick = none
str shown = nick otherwise "anon"
```

## Lexical & Formatting Rules

SNlang is designed with a **free-form, whitespace-tolerant** grammar:
- **No Semicolons**: Statements do not terminate with semicolons `;`. Delimitation is governed by structural tokens, closing braces `}`, keywords, or comma separators.
- **Mandatory Curly Braces `{}`**: All blocks (`if`, `while`, `for`, `nursery`, `wave`, `blueprint`) require curly braces `{}`. Single-line unbraced bodies (like C's `if (x) foo();`) are prohibited.
- **Whitespace Insensitivity**: Indentation is non-significant (unlike Python). Whitespace (spaces, tabs, newlines) is freely used for formatting or omitted around operators and punctuation.
- **Token Boundaries**: Whitespace is required only between adjacent alphanumeric identifiers or keywords (e.g. `int x`, not `intx`; `for x in list`, not `forxinlist`). Around punctuation and operators, whitespace is 100% optional (`a+b` is identical to `a + b`).
- **Compound Operators**: Multi-character operators (`+=`, `-=`, `*=`, `/=`, `==`, `!=`, `<=`, `>=`) must not have interior spaces (`+=`, not `+ =`).

## Control Flow & Loop Mechanics

SNlang provides three core iteration constructs: **Counted Loops**, **Iterator Loops**, and **While Loops**.

### 1. Counted `for` Loop (Uses Commas, Not Semicolons)

> [!IMPORTANT]
> Unlike C, Java, or C++ which use semicolons `for (init; cond; step)`, **SNlang's counted loop uses commas `,`**: `for (init, cond, step) { ... }`.

Counted loops support both standard and zero-space compact formatting:

```sn
// Standard spaced formatting:
for (int i = 0, i < 10, i += 1) {
    print(i)
}

// Zero-space compact formatting (100% valid):
for(int i=0,i<10,i+=1){
    print(i)
}
```

### 2. Iterator `for-in` Loop

Traverse arrays, slices, and collections:

```sn
list<str> fruits = ["Apple", "Mango", "Banana"]

for (fruit in fruits) {
    print(fruit)
}

// Parentheses are optional for collection iterators:
for fruit in fruits {
    print(fruit)
}
```

### 3. While Loop

Conditional iteration:

```sn
int count = 10
while (count > 0) {
    print("Countdown: {count}")
    count -= 1
}

// Compact variant:
while(count>0){count-=1}
```

### 4. Loop Control: `stop` and `skip`

SNlang uses `stop` (break) to immediately exit a loop and `skip` (continue) to jump to the next iteration:

```sn
for (int i = 0, i < 10, i += 1) {
    if (i == 3) {
        skip    // Skip iteration 3
    }
    if (i == 7) {
        stop    // Terminate loop at 7
    }
    print(i)
}
```

### 5. Conditionals: `if`, `else if`, `else`

Condition expressions require parentheses; blocks require curly braces:

```sn
if (x > 100) {
    print("Large")
} else if (x > 10) {
    print("Medium")
} else {
    print("Small")
}
```

## Pattern Matching & Enums

### Algebraic Enums & Pattern Matching

SNlang supports tagged unions (algebraic data types) with payload variants and strict compile-time exhaustiveness checking:

```sn
enum Token {
    Ident(str name),
    Number(int val),
    Eof
}

fn describe(Token tok) {
    match (tok) {
        Token.Ident(name) {
            print("Identifier: {name}")
        }
        Token.Number(val) {
            print("Number: {val}")
        }
        Token.Eof {
            print("End of File")
        }
    }
}
```

The compiler enforces exhaustive coverage:
- Omitting a variant from `match` without a `default` arm triggers a compile error.
- Matching on nullable `T?` requires covering `none` and value arms or providing `default`.

```sn
str? nickname = none
match (nickname) {
    none {
        print("Anonymous user")
    }
    str name {
        print("Welcome, {name}")
    }
}
```

## Errors & Failure Handling

SNlang avoids exceptions in favor of explicit, zero-cost error values and tuple returns `(T, error)`:

```sn
fn div(int a, int b) -> (int, error) {
    if (b == 0) {
        return 0, error("division by zero")
    }
    return a / b, none
}
```

### The `try` Operator
To avoid tedious `if (err != none)` boilerplate, the `try` operator unwraps the value or immediately returns the error from the enclosing function (Zig/Rust-style early return):

```sn
fn compute(int a, int b) -> (int, error) {
    int quotient = try div(a, b)
    return quotient * 2, none
}
```

## Function Contracts & Static Invariant Proving

SNlang features a built-in formal verification layer using Difference Bound Matrix (DBM) zone analysis. Functions declare mathematical preconditions (`requires`) and postconditions (`ensures`):

```sn
// Precondition proved at compile time: array bounds check is completely eliminated!
fn get_item(list<int> xs, int i) -> int requires(0 <= i < xs.length()) {
    return xs[i]
}

// Postcondition with contextual return-value binding (`result`):
fn clamp(int x) -> int requires(0 <= x <= 100) ensures(0 <= result <= 100) {
    return x
}

// Strict formal verification: Tier N non-linear expressions are rejected at compile time
verified fn add_positive(int a, int b) -> int 
    requires(a > 0 and b > 0) 
    ensures(result > 0) 
{
    return a + b
}
```

- **Compile-Time Discharge:** Proved linear contracts incur **zero CPU instructions at runtime**.
- **Safe Degradation:** In standard `fn`, unprovable invariants degrade safely to typed runtime assertions (`[E-0702]`).
- **Verified Mode (`verified fn`):** Unprovable or non-linear expressions produce a hard compilation error (`E0701`).

## Modules

```sn
use std.math
use mylib.calc
```

Search order: importer dir, `packages/`, `stdlib/`, path deps in `sn.toml`.

## Object-Oriented Architecture (OOP)

SNlang provides a robust, strongly-typed object model built on **Blueprints**, **Single Class Inheritance**, and **Multiple Contract Conformance**.

```sn
blueprint Point {
    int x
    int y

    fn distance_squared() -> int {
        return self.x * self.x + self.y * self.y
    }
}
```

### 1. Single Inheritance (`from Parent`)
A blueprint can inherit fields and concrete method implementations from at most **one** parent blueprint using the `from` keyword:

```sn
blueprint Animal {
    str species
    int legs

    fn describe() -> str {
        return self.species + " with " + cast(self.legs, str) + " legs"
    }
}

blueprint Dog from Animal {
    str breed

    // Override parent method and invoke super:
    fn describe() -> str {
        return super.describe() + " (Breed: " + self.breed + ")"
    }

    fn bark() {
        print("Woof!")
    }
}
```

### 2. Elimination of the Diamond Problem
Languages with multiple class inheritance (such as C++) suffer from the **Diamond Problem**: if class `D` inherits from both `B` and `C`, and both inherit from `A`, `D` faces ambiguous state duplication (`A::field` duplicated twice) and method resolution conflicts. Solving this requires virtual base tables (`vbtable`), runtime pointer adjustments (`this`-pointer thunks), and multiple `vptrs` inside every object.

**SNlang completely eliminates the Diamond Problem by design**:
* **Strict Single Inheritance**: Blueprints can only inherit state and concrete code from a single parent (`from`).
* **Flat C-Struct Memory Layout**: A child blueprint's fields are laid out contiguously immediately after the parent's fields in memory. Casting from child to parent is a zero-cost operation at byte offset 0.
* **Single Vtable (`vptr`)**: Every blueprint instance holds exactly one `vptr` at offset 0, matching raw C ABI performance with zero pointer thunks.

### 3. Multiple Contract Conformance (`follows Contract1, Contract2, ...`)
When a blueprint needs to satisfy multiple roles, it implements **Contracts** (behavioral interfaces) using `follows`:

```sn
contract Drawable {
    fn draw()
    fn resize(int width, int height)
}

contract Clickable {
    fn on_click(int x, int y)
}

// Inherits state from UIElement, implements behavior for Drawable and Clickable:
blueprint Button from UIElement follows Drawable, Clickable {
    str label

    fn draw() {
        print("Rendering button: {self.label}")
    }

    fn resize(int width, int height) {
        self.width = width
        self.height = height
    }

    fn on_click(int x, int y) {
        print("Button clicked at ({x}, {y})")
    }
}
```

* **No State in Contracts**: Contracts define function signatures only; they **cannot hold fields**.
* **Zero Diamond Ambiguity**: Because contracts carry no data, implementing multiple contracts can never duplicate memory or cause state collision.
* **Contract-Typed Polymorphism**: Variables can be typed as contracts (`Drawable d = button`), checked statically at compile time.

### 4. Abstract Classes vs. Contracts

| Feature | `contract Foo` (Interface) | `blueprint Bar { abstract fn ... }` (Abstract Class) |
| :--- | :---: | :---: |
| **Can hold fields/state?** | **No** (pure behavior) | **Yes** (holds member variables) |
| **Can hold concrete methods?** | **No** (signatures only) | **Yes** (mix of concrete and abstract methods) |
| **Multiple inheritance?** | **Yes** (`follows A, B, C`) | **No** (Single `from Parent`) |
| **Direct instantiation?** | **No** | **No** (Rejected at compile time) |

```sn
// Abstract blueprint: holds state and concrete methods, but cannot be instantiated directly:
blueprint Shape {
    int x
    int y

    fn move(int dx, int dy) {
        self.x += dx
        self.y += dy
    }

    abstract fn area() -> int // Subclasses MUST implement this
}

blueprint Rectangle from Shape {
    int width
    int height

    fn area() -> int {
        return self.width * self.height
    }
}
```
Attempting `new Shape()` triggers a compile error: `abstract blueprint cannot be instantiated`.

### 5. Access Control
SNlang provides three clean levels of visibility:
* **`open`** (default): Accessible from anywhere.
* **`closed`**: Strictly private to the declaring blueprint (cannot be accessed by external code or subclasses).
* **`guarded`**: Protected; accessible by the declaring blueprint and any child blueprints that inherit from it.

```sn
blueprint Account {
    open str owner
    closed dec(2) balance = 0.00
    guarded int security_pin

    closed fn recalculate_interest() {
        // internal helper only
    }
}
```

### 6. Devirtualization, Generics & Variance
* **Direct Devirtualization (1B calls/sec)**: When the concrete receiver type is known at compile time, the compiler bypasses the `vptr` and lowers calls directly to `@sn_m_{Blueprint}_{method}` with full LLVM inlining.
* **Heterogeneous Lists**: `list<Animal>` accepts any subclass instance (`zoo.push(dog)`, `zoo.push(cat)`).
* **Generic Monomorphization**: `blueprint Box<T>` monomorphizes into specialized zero-overhead types.
* **Type Bounds & Variance**:
  * `blueprint Container<T: Drawable>`: Enforces that type argument `T` must implement contract `Drawable`.
  * `blueprint Producer<out T>`: Covariant in `T`.
  * `blueprint Consumer<in T>`: Contravariant in `T`.

## Concurrency & Structured Concurrency

SNlang delivers high-throughput Go-style concurrency combined with Rust-grade compile-time data race immunity:

- `spawn { }` — Work-stealing green tasks (M:N cooperative scheduler scaling to 10,000+ tasks in milliseconds with minimal RAM)
- `nursery { }` — Structured Concurrency scope: automatically awaits all child `spawn` tasks before exiting the block, eliminating orphaned thread leaks
- `chan<T>` — Move-on-Send Channels: sending an object transfers ownership at compile time, eliminating shared-memory data races with zero locks
- `val, ok = ch.receive()` — Closed channel detection returning a `(T, bool)` tuple without crashing
- `lock` — Thread synchronization with guaranteed unwind safety

```sn
// Structured Concurrency with nursery
nursery {
    spawn worker(1)
    spawn worker(2)
} // The closing brace automatically joins all child tasks!

// Move-on-Send Channels (Zero-Lock Data-Race Immunity)
chan<str> ch
str msg = "secret"
ch.send(msg) // 'msg' is moved to the channel
// print(msg) // Compile Error: use of moved value 'msg'

// Safe Channel Read
str val, bool ok = ch.receive()
if (not ok) {
    print("Channel closed")
}
```

## Systems Memory & Performance Innovations

### Generational Pools (`std.pool` / `pool<T>` / `handle<T>`)

Generational slot vaults provide O(1) allocation and deallocation without garbage collection overhead, protected by 64-bit generation handles:

```sn
use std.pool

fn main() {
    let p = Pool.new()
    let h1 = p.insert(100)
    let h2 = p.insert(200)

    print(p.get(h1)) // 100
    p.remove(h1)

    // Stale handle detection protects against use-after-free and ABA bugs:
    if (!p.is_valid(h1)) {
        print("Handle h1 is dead and cannot be reused")
    }
}
```

### Elastic Lane Arenas (`region { ... }` / `std.arena`)

Scoped arenas use virtual-memory bump allocators with instant O(1) rollback on block exit:

```sn
region {
    // Fast O(1) bump allocations inside scope
    let buf = lane_alloc(0, 1024)
    // ...
} // Exiting the region instantly rewinds the lane watermark without GC traversal!
```

### Exact Fixed-Point Decimals (`dec(X)`)

SNlang provides fixed-point decimal arithmetic for financial precision with exact scale tracking across addition, subtraction, multiplication, and division:

```sn
let a: dec(2) = 1.50
let b: dec(2) = 2.00
let product = a * b   // 3.00 (automatically divided by 10^2)
let quotient = a / b  // 0.75 (automatically prescaled by 10^2)
```

### Async and the event loop

`await` never spins and futures are never thread-blocking. A single reactor
thread owns a timer heap and socket readiness (kqueue / epoll / poll):

- `sleep_async(ms)` registers a timer; the reactor blocks until the nearest
  deadline, so a thousand overlapping timers cost one thread and finish in the
  time of the longest one.
- `get_async(url)` performs a non-blocking connect / send / recv driven entirely
  by readiness events.
- `await future` parks the calling thread on a condition variable.

`select(chans, timeout_ms)` waits on a real condition variable (no polling
loop) and returns the index of the first channel with a message, or `-1`.

### Sockets

`use std.net` exposes non-blocking TCP over the same reactor. `select_read` is a
true multi-fd readiness wait:

```sn
use std.net

fn main() {
    int listener = listen(0)
    int port = port(listener)

    spawn {
        list<int> watch = [listener]
        if (select_read(watch, 5000) >= 0) {
            int conn = accept(listener)
            write(conn, "hello")
            close(conn)
        }
    }

    int c = connect("127.0.0.1", port, 5000)
    print(read(c))
    close(c)
}
```

## Strings & StringBuilder

Basic strings are immutable UTF-8 buffers. For lightweight tasks, standard concatenation (`+`) and built-in methods are supported:

```sn
str s = "hello" + " world"
int len = s.length()
str sub = s.slice(0, 5)
```

### High-Performance String Building (`StringBuilder`)

For high-throughput loops, file formatting, or JSON/HTML construction, repeated `str + str` incurs $O(N^2)$ memory copying. `StringBuilder` provides an amortized $O(1)$ exponential capacity buffer:

```sn
use std.string

new StringBuilder sb(64)
sb.append("Hello")
sb.append(" ")
sb.append("World")
sb.append_int(42)

print(sb.to_str())    // "Hello World42"
print(sb.len())       // 14
sb.clear()            // resets length to 0 without deallocating buffer
```

In benchmarks (`tests/benchmarks/bench_string.sn`), appending 50,000 items with `StringBuilder` finishes in **2 ms**, delivering an estimated **~150× speedup** over quadratic string concatenation.

## Method Devirtualization & C++ Execution Speed

When calling methods on blueprint instances, the compiler automatically devirtualizes calls whenever the concrete type is known at compile time:

```sn
new Counter c()
for (int i = 0, i < 1000000, i += 1) {
    c.increment()  // Devirtualized: direct LLVM call, inlined at -O2/-O3
}
```

- **Zero-Cost Dispatch:** If a blueprint has no overriding subclasses or the receiver is a direct `new` instance, `snc` lowers the call directly to `@sn_m_{Blueprint}_{method}` instead of querying the object's `vptr` table.
- **LLVM Inlining:** Eliminates indirect branch latency and allows LLVM to unroll and inline method bodies into machine registers.
- **1 Billion calls / sec:** In `tests/benchmarks/bench_dispatch.sn`, 1,000,000 method calls complete in **1 ms**.
- **ARC Borrow Elision:** When passing references (`ref<T>`) or stack-guaranteed variables, the compiler elides redundant `sn_retain`/`sn_release` pairs, preventing refcount counter thrashing in tight loops.

## Byte buffers

`ptr` from `std.bytes` is a growable mutable byte buffer:

```sn
use std.bytes

fn main() {
    ptr buf = buffer(0)
    append_str(buf, "hello")
    push(buf, 33)
    print(len(buf))          // 6
    print(get(buf, 0))        // 104  (byte value)
    print(find(buf, 108))    // 4
    print(hex(slice(buf, 0, 2)))  // 6865
    truncate(buf, 2)
    print(len(buf))          // 2
}
```

The buffer grows automatically: `push` past the initial capacity reallocates.
`raw(buf)` exposes the bytes for FFI calls; `write_at` and `append_bytes` write
into existing memory.

## Cryptography

```sn
use std.crypto
use std.bytes

fn main() {
    print(hex(sha256("abc")))
    // ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
    print(hex(md5("abc")))
    print(crc32("123456789"))   // 3421780262
    print(eq("secret", "secret"))  // constant-time compare
}
```

`std.crypto` provides SHA-256, SHA-512, MD5, HMAC-SHA256, CRC-32, a
constant-time string comparison, and `wipe` for zeroing secret buffers. Each
digest returns a `bytearray` holding the raw digest, so results can be compared
byte for byte. The implementations are verified against the published FIPS 180-4
and RFC 1321/2104 test vectors.

## Memory management

Reference counting handles ordinary lifetimes; a **cycle collector** reclaims
reference cycles that refcounting alone can never free.

`gc_collect()` runs a mark-sweep collection. The compiler hands it the value of
every GC-visible local that is live at that point, so reachability is exact:

```sn
blueprint Node {
    Node next
    int value = 0
}

fn build_cycle(int n) {
    new Node head()
    int i = 1
    while (i < n) {
        new Node cur()
        cur.next = head
        head = cur
        i = i + 1
    }
}

fn main() {
    gc_set_threshold(1024)
    build_cycle(500)
    gc_collect()
    print(gc_count())   // the unreachable cycle is gone
}
```

The collector traces list items, map keys and values, error messages, JSON
children, and the pointer-shaped fields of blueprint instances. Collection only
runs when no other thread is executing SNlang code, so a scan never races with
a mutator.

### Borrow lifetimes & Compile-Time Memory Safety

The compiler statically enforces Rust-grade memory safety and thread safety without requiring complex lifetime syntax:

1. **Borrow Exclusivity:** A location has at most one live mutable borrow (`mut<T>`), or multiple shared borrows (`ref<T>`), but never both simultaneously.
2. **Stack Escape Prevention:** A borrow of a local variable cannot escape its declaring scope, cannot be stored into an outer struct/blueprint field that outlives it, and cannot be returned from its function.
3. **Collection Invalidation Protection:** While an element of a `list` or `map` is borrowed (`ref<T>`), mutating methods (`.push()`, `.clear()`, `.pop()`, `.insert()`) are rejected at compile time to prevent dangling pointer reallocations.
4. **Affine Pointer Safety:** Calling `free(p)` invalidates `p`. Any subsequent read (`value(p)`), write (`set(p, ...)`), or second `free(p)` is caught at compile time as a use-after-free or double-free.
5. **Thread Concurrency Safety:** `spawn` statically rejects passing unsynchronized mutable references or unlocked blueprints, preventing data races across threads.

```sn
fn read_both(int a, int b) -> int {
    ref<int> ra = address(a)   // shared borrow of a
    mut<int> mb = address(b)   // exclusive borrow of b
    return value(ra) + value(mb)
}

fn bad_borrow() {
    int x = 10
    mut<int> m1 = address(x)
    mut<int> m2 = address(x)   // error: 'x' is already mutably borrowed by 'm1'
}

fn bad_collection() {
    list<int> xs = [1, 2, 3]
    ref<int> first = address(xs[0])
    xs.push(4)                 // error: cannot mutate 'xs' while borrowed by 'first'
}

fn bad_uaf() {
    ref<byte> p = alloc(16)
    free(p)
    set(p, 99)                 // error: use of freed pointer 'p'
    free(p)                    // error: double free of 'p'
}

fn escapes() -> ref<int> {
    int local = 5
    ref<int> r = address(local)
    return r   // error: cannot assign address of local 'local': target outlives declaring scope
}
```

Disjoint borrows coexist, and the owner stays usable through its own name.
Borrowing a borrow (`ref<T> b = a`) consumes `a` and transfers the loan, so
using `a` afterwards is a use-after-move. Leaving a scope ends the borrows taken
inside it, exactly like a real lifetime. See `tests/memory_safety/` for comparative
exploits and benchmarks against Rust.

### Guarded manual memory

`ptr_alloc` / `ptr_free` manage memory explicitly, and the runtime guards every
block with a poisoned header plus a quarantine, so the two mistakes that turn
into silent corruption in a long-running server are caught:

```sn
ptr a = ptr_alloc(16)
ptr_store(a, 0, 1234)
print(alloc_size(a))     // 16
print(check_live(a))     // 1
print(free_checked(a))   // 0  — first free succeeds
print(check_live(a))     // 0  — "runtime error: use after free"
print(free_checked(a))   // -1 — "runtime error: double free"
```

`check_live` reports whether a block is still valid, `alloc_size` reports its
length, and `alloc_live_count` / `alloc_freed_count` expose allocator counters.
Freed blocks are quarantined, so the header stays readable and a use-after-free
is a diagnostic rather than a wild write.

## C interoperability

`extern "libname" { fn ... }` declares C functions. The library is linked
automatically; an absolute path to a `.dylib` / `.so` / `.a` / `.o` works too,
and an rpath is added for it.

```sn
extern "sqlite3" {
    fn sqlite3_libversion() -> str
    fn sqlite3_open(str filename, ptr db) -> int
}

fn main() {
    print(sqlite3_libversion())
}
```

ABI mapping: `int` → `i64`, `float` → `double`, `str` ↔ NUL-terminated
`char *` (marshalled in both directions), `ptr` → raw pointer.

`ptr` is the opaque raw pointer used for C handles and out-parameters. It has
no automatic dereference, so the runtime provides explicit accessors:

| builtin | meaning |
| --- | --- |
| `ptr_alloc(n)` / `ptr_free(p)` | allocate / free raw memory |
| `ptr_load(p, off)` / `ptr_store(p, off, v)` | 64-bit load / store |
| `ptr_load_byte` / `ptr_store_byte` | byte load / store |
| `ptr_load_f64` / `ptr_store_f64` | double load / store |
| `ptr_load_str(p, off)` / `ptr_store_str(p, off, s)` | C string load / store |
| `ptr_int(p)` / `int_ptr(v)` | pointer ↔ integer |

Note that C APIs distinguish a *pointer to a handle* from *the handle itself*:
`sqlite3_open(path, &db)` needs the former, `sqlite3_exec(db, ...)` the latter.
`int_ptr(ptr_load(slot, 0))` performs that conversion. `std.db` wraps this for
SQLite:

```sn
use std.db

fn main() {
    ptr db = db_open(":memory:")
    db_exec(db, "CREATE TABLE t(a INTEGER)")
    db_exec(db, "INSERT INTO t VALUES (7)")
    ptr rows = db_prepare(db, "SELECT a FROM t")
    db_step(rows)
    print(db_column_int(rows, 0))
    db_finalize(rows)
}
```

## Inline assembly

```sn
asm "nop"
asm "dmb ish" : "memory"
```

An `asm` statement takes a raw template string that is handed to the LLVM
backend, plus an optional comma-separated clobber list after a colon
(rendered as `~{name}` in the constraint string). The feature is a real
escape hatch: the same lowering path runs for every supported target triple,
so macOS (arm64/x64), Linux, and Windows targets all lower the same syntax
once you supply target-valid assembly. Use `--emit-llvm` to inspect the
generated `call void asm sideeffect ...` line.

## Editor support

`snc lsp` speaks LSP over stdio and goes well past diagnostics:

- **Completion** — locals in scope, file-level functions, blueprints and
  contracts, and runtime builtins. After `d.` it offers the methods of `d`'s
  type, walking the inheritance chain and including contracts.
- **Hover** — the full signature of a function (with `method` / `static` /
  `extern C` markers), or a blueprint with its fields and methods.
- **Go to definition**, **find references**, and **rename** — rename rewrites
  every occurrence and refuses builtins, `extern` symbols, and invalid
  identifiers.
- **Document symbols** — a nested tree of blueprints (with their methods) and
  top-level functions.

## Tests

```sn
use std.test

fn main() {
    assert_eq_int(2 + 2, 4)
}
```

See `examples/std_test.sn`.

---

## Advanced Systems Programming & Kernel Development

For high-performance systems architecture and bare-metal programming:

* **[Memory Safety & Tiered Execution Architecture](MEMORY_SAFETY.md)**: Deep dive into the 4 memory tiers, pinned thread lanes, $O(1)$ lexical region scopes (`region { }`), affine escape analysis (`E-ESCAPE`), memory space isolation (`E-GC-IN-LANE`), poison safety (`T-FIBR`), and nursery lane carving.
* **[Advanced Systems Guide](SYSTEMS_GUIDE.md)**: Deterministic regional memory, cache-oblivious collections (`morton list`, `hilbert map`), confluent race-free concurrency (`wave { }`, `lcell`), contract-driven static verification (`requires`, `ensures`), and differential state chronology (`tape`, `zset`, `zfix`).
* **[Bare-Metal Kernel Architecture Spec](RFC_BAREMETAL_KERNEL_MASTER_SPEC.md)**: Freestanding mode (`--freestanding`), pluggable page allocators, hardware volatile registers (`mmio<T>`), and static interrupt safety (`irq fn`).
* **[Formal Language Specification](../SNLANG_SPEC.md)**: The authoritative formal specification for the SNlang grammar (complete EBNF), type system, operational semantics, and runtime ABI.
