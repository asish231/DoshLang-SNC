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

## Concurrency

- `spawn { }` — OS thread
- `goroutine { }` — work-stealing scheduler; workers are CPU-scaled and steal
  from each other when their own deque runs dry
- `chan<T>`, `lock`

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

### Borrow lifetimes

A location has at most one live borrow, `mut<T>` is exclusive, and a borrow of a
local cannot outlive its function.

```sn
fn read_both(int a, int b) -> int {
    ref<int> ra = address(a)   // shared borrow of a
    mut<int> mb = address(b)   // exclusive borrow of b
    return value(ra) + value(mb)
}

fn bad() {
    mut<int> m1 = address(x)
    mut<int> m2 = address(x)   // error: 'x' is already mutably borrowed by 'm1'
}

fn escapes() -> ref<int> {
    int local = 5
    ref<int> r = address(local)
    return r   // error: cannot return 'r': it borrows 'local'
}
```

Disjoint borrows coexist, and the owner stays usable through its own name.
Borrowing a borrow (`ref<T> b = a`) consumes `a` and transfers the loan, so
using `a` afterwards is a use-after-move. Leaving a scope ends the borrows taken
inside it, exactly like a real lifetime.

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

See `examples/std_test.sn`. Full language reference: [SNLANG_SPEC.md](../SNLANG_SPEC.md).
