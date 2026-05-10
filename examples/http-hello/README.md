# http-hello

Phase 3 capstone — a minimal HTTP server written in Aperio,
composing the four moving parts the phase ships:

- **m82** — locus-all-the-way-down lifecycle. `let s = Stream
  { conn_fd: fd }` defers Stream's dissolve to its enclosing
  scope, which is what makes per-connection method calls
  (`s.recv` / `s.send`) work without manually managing fds.
- **m83** — `__StdIoTcpListener` with `on_connection: fn(Stream)`
  callback. The accept loop dispatches each connection through
  the callback; per-connection scope owns the Stream's
  lifecycle.
- **m84** — `std::http::parse_request(raw)` extracts method,
  path, version, and body from raw request bytes. Headers
  between the request line and body are skipped (no header
  map type yet).
- **m85** — `std::http::write_response(s, resp)` ships the
  HTTP/1.1 wire format (status line + Content-Type +
  Content-Length + Connection: close + body) over a Stream.

## Run

```
cargo run --bin aperio -- run examples/http-hello/main.ap
# or with a custom port + accept cap:
cargo run --bin aperio -- run examples/http-hello/main.ap 9000 5
```

Then in another terminal:

```
curl http://127.0.0.1:8080/
curl 'http://127.0.0.1:8080/some/path?q=hi'
```

The server prints each request to stderr and responds with a
small HTML page echoing the requested path.

## Arguments

| arg     | meaning                                | default |
|---------|----------------------------------------|---------|
| `argv[1]` | listen port                          | 8080    |
| `argv[2]` | max_accepts (negative = forever)     | -1      |

## v0 limits

- Single-threaded — each request fully handled before the
  next accept.
- One recv per request; the parser assumes the entire request
  fits in 8KB. Real-world browser GETs are well within that.
- No persistent connections — `Connection: close` is
  hardcoded in the response.
- No header surface in v0 (waiting on a generic map type).
