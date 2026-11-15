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

## Control

`if` / `else if` / `else`, `while`, counted `for`, `for in`, `match`, `stop`, `skip`, `defer`.

## Errors

```sn
fn div(int a, int b) -> (int, error) {
    if (b == 0) {
        return 0, error("div0")
    }
    return a / b, none
}
```

`try expr` early-returns on error (Zig-style).

## Modules

```sn
use std.math
use mylib.calc
```

Search order: importer dir, `packages/`, `stdlib/`, path deps in `sn.toml`.

## OOP

`blueprint`, `from`, `super.method()`, `closed` / `guarded` fields and methods,
and `blueprint Box<T>` monomorphization.

- **Vtable dispatch** — every class gets a per-class vtable emitted as an LLVM
  global. Calls go through the object's vptr slot, so overriding a method in a
  subclass works without any `type_id` comparison chain.
- **Heterogeneous lists** — `list<Animal>` accepts any subclass instance:
  `zoo.push(dog)`, `zoo.push(cat)`.
- **Contract-typed variables** — `Drawable d = circle` with full checking.
- **Static members** — `static fn create() -> int` on a blueprint, called as
  `Counter.create()`. Static methods have no `self` and may not use it; static
  methods never occupy a vtable slot.
- **Abstract methods** — `abstract fn area() -> int` declares a signature with no
  body. A blueprint with unimplemented abstract methods cannot be instantiated,
  and subclasses must supply implementations.
- **Method-level access** — `closed fn audit() -> int` is private to the
  blueprint that declares it, while a subclass may still call it on itself.
- **Generic constraints and variance**
  - `blueprint Holder<T: Printable>` requires the argument to satisfy a contract.
  - `blueprint Producer<out T>` is covariant, `blueprint Consumer<in T>` is
    contravariant, and a plain `<T>` is invariant.

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

## Advanced Systems & Non-von Neumann Features

SNlang provides five advanced systems pillars for high-performance, deterministic, and bare-metal systems programming. For a complete guide with working examples and formal rules, see **[RABBIT_HOLE_FEATURES.md](RABBIT_HOLE_FEATURES.md)** and the **[Bare-Metal Architecture Suite](RFC_BAREMETAL_KERNEL_MASTER_SPEC.md)**.

1. **Pinned Lanes & Region Blocks (`region { ... }`)**:
   - Single-cycle bump allocation inside thread-local lanes.
   - $\mathcal{O}(1)$ wholesale rollback on block exit; zero GC pauses and zero memory leaks.
   - Checked region poset preventing inner pointers from escaping into outer scopes (`E-LANE-ESCAPE`).

2. **Space-Filling Geometry Collections (`morton list` & `hilbert map`)**:
   - Cache-oblivious 2D/3D Morton $Z$-curve layouts (`morton list<int, dims=[16, 16]>`) via hardware BMI2 (`PDEP`/`PEXT`) for stencil sweeps and matrices.
   - Hilbert spatial maps (`hilbert map<int, int>`) with segment-advance bounding-box queries (`hilbert_box_first`/`next`).

3. **Mathematically Confluent Concurrency (`lcell` & `wave { ... }`)**:
   - Join-semilattices (Max HWM, Grow-Only Set, Last-Write-Wins) guaranteeing data-race freedom by construction (Theorem WS).
   - Deterministic wave regions: `read_W(cell)` reads frozen capture $\kappa$, `contrib(cell, val)` stages offers, and `read_J(cell)` observes the atomically folded join on exit. Purity enforced via compiler rule `[E0812]`.

4. **Salt $\varepsilon$ Formal Numeric Contracts (`requires`, `ensures`, `verified fn`)**:
   - Static verification of integer intervals and difference bounds ($x - y \le c$) using Difference Bound Matrices (DBM) in $\mathcal{O}(|V|^3)$ time.
   - Unprovable non-linear operations safely degrade to typed runtime assertions (`[E-0702]`) in standard functions or trigger compile error `[E0701]` in `verified fn`.

5. **Differential Dataflow & Deterministic Tape (`zset`, `zfix`, `tape`)**:
   - $\mathbb{Z}$-sets (`zset`) with exact retractions ($\oplus, \ominus$) for incremental $\mathcal{O}(|\Delta|)$ computation.
   - Fixpoint brackets (`zfix`) evaluating recursive cyclic graphs with semi-naïve round evaluation and guaranteed termination.
   - Append-only chronological execution ledgers (`tape`) with delta and checkpoint replay.

6. **Bare-Metal & Freestanding Mode (`--freestanding`)**:
   - Pluggable kernel memory provider ABI (`sn_set_memory_provider`) and panic handler (`sn_set_panic_handler`).
   - Volatile MMIO device access (`mmio<T>`) with escape prohibition (`E-MMIO-ESCAPE`).
   - Static interrupt safety (`irq fn`) with compile-time prohibition of allocation (`E-IRQ-ALLOC`) and blocking synchronization (`E-IRQ-BLOCK`).

Full language reference: [SNLANG_SPEC.md](../SNLANG_SPEC.md).
