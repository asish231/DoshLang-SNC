# Packages

SNlang packages are folders with `sn.toml` plus `.sn` sources.

## Commands

```sh
snc pkg init --name myapp          # sn.toml + main.sn + sn.lock.toml
snc pkg add mylib --path packages/mylib
snc pkg add foo --registry https://example.com/snlang --version 1.2.3
snc pkg list
snc pkg build -o app               # refreshes sn.lock.toml
snc pkg publish                    # writes dist/<name>-<version>.tar.gz
```

## sn.toml

```toml
name = "myapp"
version = "0.1.0"

[deps]
mylib = { path = "packages/mylib" }
remote = { registry = "https://example.com/snlang", version = "1.0.0" }
std = "0.2.0"
```

## sn.lock.toml

Written on `pkg add`, `pkg build`, `pkg init`, and `pkg publish`. Commit it to pin
path/registry/version deps. Example:

```toml
[[package]]
name = "myapp"
version = "0.1.0"
source = "root"
path = "."

[[package]]
name = "mylib"
version = "0.1.0"
source = "path"
path = "packages/mylib"
```

## Publishing

`snc pkg publish` packs `sn.toml` and `.sn` files into `dist/<name>-<version>.tar.gz`
(or `packages/dist/` when publishing from under `packages/`).

## Simple registry

Serve a directory over HTTPS/HTTP with:

```text
index.json
packages/
  mylib-0.1.0.tar.gz
```

### index.json format

```json
{
  "mylib": {
    "version": "0.1.0",
    "url": "https://example.com/snlang/packages/mylib-0.1.0.tar.gz"
  },
  "other": {
    "version": "2.0.0",
    "tarball": "https://example.com/snlang/packages/other-2.0.0.tar.gz"
  }
}
```

- Top-level keys are package names.
- Each value needs `"version"` and either `"url"` or `"tarball"`.
- If a name is missing from the index, `snc` falls back to
  `{registry}/packages/{name}-{version}.tar.gz`.

Example static server:

```sh
python3 -m http.server 8080 --directory /path/to/registry
snc pkg add mylib --registry http://127.0.0.1:8080 --version 0.1.0
```

If tarball extract fails, `snc` **errors loudly** and does **not** write a stub `lib.sn`.

Registry fetches use `curl` with TLS verification by default. Set `SN_HTTP_INSECURE=1`
only when you must skip certificate checks.
