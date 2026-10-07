# SNlang (`snc`)

<p align="center">
  <img src="logo.png" alt="SNlang Logo" width="160"/>
</p>


SNlang is a small typed language with plain syntax (`fn`, braces, `and` / `or` / `not`)
that compiles to **native code** through **LLVM IR** + `clang`. The compiler is written
in Rust.

**License:** [MIT](LICENSE) — Copyright (c) 2026 Asish Sharma / SafarNow.

## Install

Needs [Rust](https://rustup.rs/) (`cargo` on `PATH`) and `clang`.

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

## Download (prebuilt binaries)

Grab `snlang-<os>-<arch>.tar.gz` / `.zip` from
[GitHub Releases](../../releases) — macOS (arm64 + x64), Linux (x64),
Windows (x64). Each archive ships `snc`, `stdlib/`, sample `packages/`,
`runtime.c`, and a hello world:

```sh
./snc --version   # snc 0.2.0
./snc hello_world.sn -o hello && ./hello
```

You still need **clang on `PATH`** (Xcode command-line tools, `apt install
clang`, or LLVM for Windows) because `snc` links through it. Keep the
extracted folder together — or add it to `PATH` — and compile from anywhere.

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

More: [docs/LANGUAGE.md](docs/LANGUAGE.md) · full reference [SNLANG_SPEC.md](SNLANG_SPEC.md)

## Language tour (implemented)

| Area | Highlights |
|---|---|
| Types | `int`, `bool`, `str`, `list<T>`, `map<K,V>`, `T?`, `error`, `chan`, `ref`, `lock`, `json`, `any`, `record`, fn types |
| Null | `otherwise`, `x!!`, `x?.m()`, smart unwrap after `if (x != none)` |
| Errors | `(T, error)`, `try`, `panic` — not Java exceptions |
| OOP | `blueprint`, inheritance, `closed`/`guarded`, MVP polymorphism + generics |
| Concurrency | `spawn`, `goroutine` pool, channels, locks |
| Modules | `use std.math`, `use mylib.calc` |
| Tooling | `snc fmt`, `snc lsp`, `snc debug`, `snc translate`, `snc pkg` |

## Stdlib

| Package | Role |
|---|---|
| `std.math` | `abs`, `min`, `max`, `pow`, `clamp`, `sign` |
| `std.string` | `isEmpty`, `repeat` (+ built-in string methods) |
| `std.io` | `println` |
| `std.json` | `parse` / `encode` |
| `std.time` | `now_ms`, `sleep_ms`, `format` |
| `std.http` / `std.net` | HTTP client + `serve_once`; **HTTPS via verified `curl`** |
| `std.file` / `std.path` / `std.os` | files, paths, env/args |
| `std.test` | `assert_true`, `assert_eq_int`, `assert_eq_str`, … |

HTTP details: [docs/HTTP.md](docs/HTTP.md)

## Package manager

Local folders + `sn.toml`, lockfile `sn.lock.toml`, optional remote registry.

```sh
snc pkg init --name myapp
snc pkg add mylib --path packages/mylib
snc pkg add foo --registry https://example.com/snlang --version 1.0.0
snc pkg list
snc pkg build -o app
snc pkg publish                 # dist/<name>-<ver>.tar.gz
```

Commit `sn.lock.toml` in apps. Registry `index.json` format and a static-server recipe:
[docs/PACKAGES.md](docs/PACKAGES.md)

## MVP vs complete (honest)

| Feature | Status | Notes |
|---|---|---|
| HTTPS/TLS | **MVP** | `https://` via system `curl` with cert verify; `SN_HTTP_INSECURE=1` for `-k` |
| Remote registry | **MVP** | Fetch `index.json` + tarball; loud error on bad extract (no stub `lib.sn`) |
| Lockfile / publish | **Done** | `sn.lock.toml`; `pkg publish` → `.tar.gz` |
| async/await | **MVP** | Blocking futures |
| Goroutines | **MVP** | Fixed 4-worker pool |
| OOP polymorphism | **MVP** | `type_id` dispatch |
| Generic blueprints | **MVP** | Monomorphize `Box<int>` |
| Borrow checker | **MVP** | Move on `ref` assign only |
| LSP | **Scaffold** | Diagnostics with line/col ranges; symbols |
| Self-hosting | **Scaffold** | Lexer demo only |
| Central registry hosting | **Not done** | Bring your own static host |
| In-process OpenSSL | **Not done** | Curl for HTTPS keeps builds simple |
| Full lifetimes / work-stealing | **Not done** | — |

## Translator

Subset map from Python / JS / Go / C → SNlang (`snc translate --from python …`).
Not a full frontend for those languages.

## Layout

- `compiler/` — Rust `snc`
- `compiler/runtime.c` — strings, lists, HTTP, threads, …
- `stdlib/std/` — standard library
- `packages/` — sample packages (`mylib`)
- `examples/` — runnable programs
- `docs/` — packages, HTTP, language tour
- `tests/` — extra samples
- `vscode-snlang/` — editor grammar
- `LICENSE` — MIT

## Docs

| Doc | Contents |
|---|---|
| [docs/LANGUAGE.md](docs/LANGUAGE.md) | Short language tour |
| [docs/PACKAGES.md](docs/PACKAGES.md) | `sn.toml`, lockfile, publish, registry |
| [docs/HTTP.md](docs/HTTP.md) | Client, env vars, TLS |
| [SNLANG_SPEC.md](SNLANG_SPEC.md) | Full language status |
