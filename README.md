# SNlang (`snc`)

<p align="center">
  <img src="logo.png" alt="SNlang Logo" width="160"/>
</p>

SNlang is a high-performance systems programming language that compiles to **native machine code** through **LLVM IR** + `clang`. It combines plain, readable syntax (`fn`, braces, `and` / `or` / `not`) with deterministic memory tiers, cache-oblivious spatial collections, race-free concurrency confluence, and compile-time contract verification.

**License:** [Apache-2.0](LICENSE) — Copyright (c) 2026 Asish Sharma / SafarNow.

---

## Installation

Requires [Rust](https://rustup.rs/) (`cargo` on `PATH`) and `clang 15+`.

```sh
# Ensure cargo is on PATH (rustup default):
export PATH="$HOME/.cargo/bin:$PATH"

make          # builds ./snc
./snc examples/hello_world.sn -o hello
./hello
```

Windows: `./build.ps1` then `./snc.exe …`.

```sh
make test     # cargo test + hello smoke run
```

---

## Download Prebuilt Binaries

Prebuilt archives for macOS (Apple Silicon arm64 & Intel x64), Linux (x64), and Windows (x64) are available on [GitHub Releases](../../releases):

```sh
./snc --version   # snc 0.3.1
./snc hello_world.sn -o hello && ./hello
```

Each archive contains `snc`, `stdlib/`, sample `packages/`, `runtime.c`, and sample code. You still need **clang 15+** on `PATH` for LLVM object linking.

---

## Quickstart

```sn
fn main() {
    str name = "Ada"
    print("hello {name}")
    list<int> xs = [1, 2, 3]
    for (x in xs) {
        print(x)
    }
}
```

```sh
./snc examples/hello_world.sn -o hello && ./hello
./snc examples/use_std_math.sn -o math && ./math
./snc examples/std_test.sn -o t && ./t
```

---

## Key Systems Capabilities

### 1. Deterministic Regional Memory & Elastic Arenas
Thread-local virtual-memory bump allocators provide $\approx 3\text{ ns}$ allocation with scoped $\mathcal{O}(1)$ bulk rewinds, eliminating stop-the-world garbage collection pauses:

```snlang
use std.arena

region {
    ptr scratch = lane_alloc(0, 4096)
    // High-performance scratch computations...
} // High-water mark automatically reset to baseline in 1 store instruction!
```

### 2. Cache-Oblivious Space-Filling Collections
Multidimensional arrays and spatial structures mapped via Morton $Z$-order and Hilbert fractal curves preserve cacheline locality across arbitrary hardware topologies:

```snlang
// 16x16 power-of-two 2D tensor in Morton curve order
morton list<int, dims=[16, 16]> grid = morton_new(16, 16)
morton_set(grid, 4, 8, 42)
int val = morton_get(grid, 4, 8)

// 2D spatial bounding-box query along the continuous Hilbert curve
hilbert map<int, int> spatial = hilbert_new(3) // Grid side 2^3 = 8
int cursor = hilbert_box_first(spatial, 1, 1, 5, 5)
```

### 3. Confluent Semilattices & Deterministic Wave Parallelism
Shared mutable state is constrained to join-semilattices (`lcell`). Wave execution blocks (`wave { }`) guarantee bit-identical output regardless of thread arrival order or OS scheduling:

```snlang
ptr peak = lcell_new(0) // Monotonic max register (⊔ = max)

wave {
    int base = read_W(peak)       // Read frozen capture κ
    contrib(peak, base * 2, 0)    // Staged payload
} // J(W) join point: staged offers fold atomically and deterministically!

int final_peak = read_J(peak)     // Observe folded result
```

### 4. Contract-Driven Static Verification
Preconditions (`requires`) and postconditions (`ensures`) are discharged at compile time via Difference Bound Matrix (DBM) zone analysis with zero runtime cost:

```snlang
fn safe_lookup(list<int> xs, int i) -> int 
    requires(0 <= i < xs.length()) 
{
    return xs[i] // Array bounds check proved and eliminated at compile time!
}

fn clamp(int x) -> int 
    requires(0 <= x <= 100) 
    ensures(0 <= result <= 100) 
{
    return x
}
```

### 5. Bare-Metal & Freestanding Mode (`--freestanding`)
Compile OS kernels and embedded firmware without libc, POSIX, or dynamic GC heaps, featuring hardware volatile registers (`mmio<T>`) and static interrupt safety (`irq fn`):

```snlang
// Volatile hardware register wrapper with escape analysis:
mmio<u32> uart_dr = mmio_at(0x3F8)
uart_dr.write(0x41) // Guaranteed volatile store; cannot escape to heap

// Interrupt handler statically checked against allocation and blocking:
irq fn keyboard_isr() {
    // Compile-time checks (E-IRQ-ALLOC, E-IRQ-BLOCK) guarantee no sleep or deadlocks
}
```

---

## Language Overview

| Area | Highlights |
|---|---|
| **Types** | `int`, `bool`, `str`, `byte`, `dec(X)`, `list<T>`, `map<K,V>`, `T?`, `error`, `chan<T>`, `ref<T>`, `morton list<T, dims=[N, M]>`, `hilbert map<K, V>`, `tape<T>` |
| **Memory** | Stack, Thread-Local Pinned Lanes (3 ns bump), Scoped `region { }` Arenas, Generational Pools (`std.pool`), Self-Relative Links (`link<T, u16>`), Freestanding Mode |
| **Safety** | Affine stack & region escape analysis (`E-ESCAPE`), isolated memory spaces (`E-GC-IN-LANE`), wave purity check (`E0812`), copy discipline (`E0801`), global pool discipline (`E0803`) |
| **Verification** | Difference Bound Matrix (DBM) zone solver for `requires` and `ensures`, typed runtime assertion boundaries (`[E-0702]`), `verified fn` strict mode (`E0701`) |
| **Speed** | LLVM IR backend + clang `-O2`, direct method devirtualization, native `StringBuilder`, branchless Morton bit-interleaving via BMI2 (`PDEP`/`PEXT`) |
| **Concurrency** | Work-stealing green tasks (`spawn`), task groups (`nursery { }`), move-on-send channels (`chan<T>`), confluent semilattices (`lcell`), deterministic waves (`wave`) |
| **Null Safety** | Nullable types `T?`, null coalescence `otherwise`, non-null assertion `x!!`, smart unwrapping |
| **Error Handling** | Go/Zig-style value tuples `(T, error)`, early-return `try expr`, panic fail-fasts |
| **OOP** | `blueprint`, inheritance, `closed` / `guarded` access modifiers, monomorphized generics |
| **Tooling** | Native compiler (`snc`), package manager (`snc pkg`), LSP (`snc lsp`), translator (`snc translate`) |

---

## Standard Library

| Package | Role |
|---|---|
| `std.arena` | Thread-local virtual memory bump allocators with $\mathcal{O}(1)$ bulk rollback |
| `std.lattice` | Join-semilattice cells (`LCell`) with Max, Grow-Only Set, and Last-Write-Wins carriers |
| `std.delta` | Differential $\mathbb{Z}$-set collections with exact retractions and multiset arithmetic |
| `std.zeta` | Bounded fixpoint engine (`ZFix`) with semi-naïve round evaluation and termination invariants |
| `std.pool` | Generational slot vaults with 64-bit handles (ABA and use-after-free immune) |
| `std.math` | Fast numeric routines, trigonometric functions, integer powers, and bit operations |
| `std.string` | String utilities and amortized native `StringBuilder` |
| `std.io` | Formatted output and file streaming |
| `std.json` | Zero-allocation streaming JSON parser and encoder |
| `std.time` | Monotonic timers, sleep, and wall-clock formatting |
| `std.http` / `std.net` | HTTP networking client, socket primitives, and verified TLS |
| `std.test` | Unit and integration test assertions (`assert_eq_int`, `assert_true`, etc.) |

---

## Package Manager

Local folders + `sn.toml`, lockfile `sn.lock.toml`, optional remote registry:

```sh
snc pkg init --name myapp
snc pkg add mylib --path packages/mylib
snc pkg add foo --registry https://example.com/snlang --version 1.0.0
snc pkg list
snc pkg build -o app
snc pkg publish                 # dist/<name>-<ver>.tar.gz
```

Commit `sn.lock.toml` in apps. Registry `index.json` format and recipes: [docs/PACKAGES.md](docs/PACKAGES.md).

---

## Production Feature Status Matrix

| Capability | Status | Description |
|---|:---:|---|
| **Regional Memory & Scoped Arenas** | **Stable** | Thread-local bump allocators, `region { }` rollbacks, and poison safety. |
| **Space-Filling Collections** | **Stable** | 2D Morton lists and Hilbert spatial maps with hardware BMI2 dispatch. |
| **Confluent Concurrency & Waves** | **Stable** | Monotonic semilattices (`lcell`), wave blocks (`wave`), and join confluence. |
| **Contract-Driven Verification** | **Stable** | DBM linear range discharge, remainder axioms, and typed runtime boundaries. |
| **Differential State & Tapes** | **Stable** | $\mathbb{Z}$-set deltas, bounded fixpoint loops (`zfix`), and append-only tapes. |
| **Relocatable Links & Pools** | **Stable** | Self-relative displacements (`link<T, u16>`) and generational handle vaults. |
| **Bare-Metal & Freestanding Mode** | **Preview (v0.3.1)** | `--freestanding` compilation, memory provider ABI, `mmio<T>`, and IRQ safety. |
| **Language Server Protocol (LSP)** | **Scaffold** | Diagnostics with line/col ranges and structured document symbols. |
| **Self-Hosting Compiler** | **Roadmap** | Self-hosted lexer complete; AST and code generation in development. |

---

## Documentation Sitemap

| Guide | Description |
|---|---|
| [**docs/LANGUAGE.md**](docs/LANGUAGE.md) | **Core Language Tour** — Basic syntax, types, control flow, functions, OOP, and errors |
| [**docs/MEMORY_SAFETY.md**](docs/MEMORY_SAFETY.md) | **Memory Safety & Tiered Execution** — 4 memory spaces, pinned lanes, $O(1)$ regions, escape analysis, and poison guards |
| [**docs/SYSTEMS_GUIDE.md**](docs/SYSTEMS_GUIDE.md) | **Advanced Systems Architecture** — Regional memory, SFC, waves, contracts, tapes, and production tutorials |
| [**docs/RFC_BAREMETAL_KERNEL_MASTER_SPEC.md**](docs/RFC_BAREMETAL_KERNEL_MASTER_SPEC.md) | **Bare-Metal Kernel RFC** — Freestanding runtime, MMIO registers, and IRQ static safety |
| [**docs/PACKAGES.md**](docs/PACKAGES.md) | **Package Management** — `sn.toml` manifest, dependencies, lockfiles, and publishing |
| [**docs/HTTP.md**](docs/HTTP.md) | **Networking & Services** — HTTP client, configuration, and TLS verification |
| [**SNLANG_SPEC.md**](SNLANG_SPEC.md) | **Formal Language Specification** — Authoritative formal grammar (EBNF), type system, operational semantics, and runtime ABI |
