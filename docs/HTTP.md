# HTTP

`std.http` / `std.net` provide a small HTTP client and a one-shot local server.

```sn
use std.http

fn main() {
    str body, error err = get("https://example.com/")
    if (err != none) {
        print(err)
        return
    }
    print(body.length())
}
```

## Transports

| Scheme | How it works |
|---|---|
| `http://` | Direct TCP sockets (HTTP/1.1), still supported |
| `https://` | System `curl` with **certificate verification** by default |

There is no in-process OpenSSL/LibreSSL yet; HTTPS stays curl-based so builds stay simple.

## Environment

| Variable | Default | Meaning |
|---|---|---|
| `SN_HTTP_TIMEOUT` | `30` | Seconds for curl `--max-time` and socket send/recv timeouts |
| `SN_HTTP_INSECURE` | unset | Set to `1` to pass curl `-k` (skip TLS verify) |
| `SN_HTTP_USER_AGENT` | `snlang/0.2` | `User-Agent` on HTTPS and plain HTTP |

## Headers

Requests send `Host`, `User-Agent`, `Connection: close`, and `Content-Length` for POST.
Custom header maps are not in the MVP API yet.

## Server

`serve_once(port, handler)` binds `127.0.0.1`, accepts one connection, calls `handler(path)`, responds, and exits.
