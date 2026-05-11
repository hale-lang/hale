# `std::io::tcp`

TCP networking for Aperio programs. Phase 1 introduced the
single-accept Listener (m73). Subsequent milestones generalized
it: m81 added the `Stream` type with `send`/`recv` methods, m82
made the Stream's let-binding lifecycle work cleanly, m83 added
the multi-accept Listener with an `on_connection` callback, and
m89 added `Stream.send_bytes` for binary-safe payloads.

The shipped surface is two stdlib types — `Listener` (a locus)
and `Stream` (a locus you receive but don't construct directly)
— plus their methods.

## Loci

### `std::io::tcp::Listener`

A multi-accept TCP listener. Binds a port, calls
`on_connection` once per accepted connection, terminates after
`max_accepts` accepts (or runs forever if `max_accepts == -1`).

#### Synopsis

```aperio
locus Listener {
    params {
        host:           String = "127.0.0.1";
        port:           Int    = 0;
        max_accepts:    Int    = 1;        // -1 for unbounded
        on_connection:  fn(Stream);
    }
    // birth: bind, listen
    // run:   accept loop, calling on_connection for each
    // dissolve: close listening fd
}
```

#### Fields

- `host: String` — bind interface. `"127.0.0.1"` for
  loopback only; `"0.0.0.0"` for all interfaces.
- `port: Int` — TCP port. `0` lets the OS pick a free
  ephemeral port.
- `max_accepts: Int` — accept limit. `-1` means unbounded
  (server runs until killed). Bounded values are useful for
  tests and one-shot tools.
- `on_connection: fn(Stream)` — callback invoked once per
  accepted connection. The Stream's lifecycle is owned by the
  callback's stack frame; when the callback returns, the
  Stream's `dissolve()` closes the connection's fd.

#### Semantics

- **birth()** binds an `AF_INET SOCK_STREAM` socket to
  `host:port` with `SO_REUSEADDR`, then calls `listen(backlog=16)`.
- **run()** loops calling `accept()`. Each accepted
  connection's fd is wrapped in a `Stream` locus, passed to
  `on_connection`, and dissolved at the callback's scope-exit.
  After `max_accepts` accepts (or never, if -1), `run()`
  returns.
- **dissolve()** closes the listening fd, releasing the port.
- Errors during accept print a `perror`-style line to stderr
  and continue the loop. A bind failure in `birth()` aborts
  the locus.

#### Examples

A multi-accept listener that handles requests until killed:

```aperio
fn handle(s: std::io::tcp::Stream) {
    let req = s.recv(8192);
    println("got ", len(req), " bytes");
    s.send("ok\n");
}

fn main() {
    std::io::tcp::Listener {
        host: "127.0.0.1",
        port: 8080,
        max_accepts: -1,
        on_connection: handle
    };
}
```

A bounded listener for a test (handle 3 connections, then
exit):

```aperio
fn main() {
    std::io::tcp::Listener {
        port: 8080,
        max_accepts: 3,
        on_connection: handle
    };
}
```

### `std::io::tcp::Stream`

A handle to one accepted TCP connection. You don't construct
`Stream` directly — instead, you receive one as the parameter
of an `on_connection` callback.

#### Synopsis

```aperio
locus Stream {
    params { conn_fd: Int = -1; }
    fn send(msg: String);
    fn send_bytes(payload: Bytes);
    fn recv(max: Int) -> String;
    // dissolve: close(conn_fd)
}
```

#### Methods

- **`send(msg: String)`** — writes the String's bytes to the
  connection. Aperio Strings are NUL-terminated in memory, so
  embedded NULs in `msg` truncate the write. Use `send_bytes`
  for binary-safe sends.
- **`send_bytes(payload: Bytes)`** — writes the full byte
  blob, length-preserved. Embedded NULs survive. (m89)
- **`recv(max: Int) -> String`** — reads up to `max` bytes
  from the connection. Returns the bytes received as a String
  in the lazy global payload arena. Returns the empty String
  on EOF or error. **Single recv per call** — the result is
  whatever one OS-level `read()` produces, not a guaranteed
  full message. For typical HTTP request payloads (under
  8 KB), one `recv(8192)` covers the whole request.

#### Semantics

- A `Stream` instantiated from outside an `on_connection`
  callback (e.g. `let s = std::io::tcp::Stream { conn_fd: ... }`)
  works the same way: methods operate on `conn_fd`; `dissolve`
  closes the fd at scope-exit. This is the m82 let-bound-locus
  lifecycle in action.
- The connection is closed exactly once, when the binding's
  scope ends. Don't close `conn_fd` directly via the
  `__close_fd` primitive while a Stream binding still holds
  it.

#### Examples

Echo server using the multi-accept Listener:

```aperio
fn echo(s: std::io::tcp::Stream) {
    let buf = s.recv(4096);
    s.send(buf);
}

fn main() {
    std::io::tcp::Listener {
        port: 9000, max_accepts: -1, on_connection: echo
    };
}
```

Outbound TCP client (compose with the lower-level `__connect`
primitive — there is no high-level `Stream::connect` yet):

```aperio
fn main() {
    let fd = std::io::tcp::__connect("127.0.0.1", 8080);
    let s  = std::io::tcp::Stream { conn_fd: fd };
    s.send("GET / HTTP/1.0\r\n\r\n");
    let body = s.recv(8192);
    println(body);
}
```

The let-binding `let s = ...` is the form that makes Stream
usable as a multi-statement handle (m82). A statement-position
Stream literal would dissolve immediately, closing `fd` before
`send`/`recv` ran.

## Limitations

- **Single recv per call.** No streaming-recv loop, no buffered
  reader. Bodies > 8 KB are not handled by Phase 3 v0; this is
  a Phase 3 v1.0 follow-up.
- **No `Stream::connect` constructor.** Outbound connections
  use the lower-level `__connect` primitive. A high-level
  `connect` form is a follow-up.
- **AF_INET only.** IPv4 — IPv6 (AF_INET6) is a follow-up.
- **No TLS.** No HTTPS substrate ships.
- **No bind-readiness signal.** Tests connecting to a listener
  immediately after instantiation may race the bind. The
  workaround is retry-connect from the client side.

## Internal primitives

The stdlib loci delegate to internal `std::io::tcp::__*`
path-calls. These are not part of the user surface but their
shape is documented for implementers:

| Primitive                                       | Type                              | Backs                            |
|-------------------------------------------------|-----------------------------------|----------------------------------|
| `std::io::tcp::__listen_socket(host, port)`     | `(String, Int) -> Int`            | `lotus_tcp_listen_socket`         |
| `std::io::tcp::__accept_one(listen_fd)`         | `(Int) -> Int`                    | `lotus_tcp_accept_one`            |
| `std::io::tcp::__close_fd(fd)`                  | `(Int) -> Int`                    | `lotus_tcp_close_fd`              |
| `std::io::tcp::__connect(host, port)`           | `(String, Int) -> Int`            | `lotus_tcp_connect`               |
| `std::io::tcp::__send(fd, msg)`                 | `(Int, String) -> Int`            | `lotus_tcp_send`                  |
| `std::io::tcp::__send_bytes(fd, payload)`       | `(Int, Bytes) -> Int`             | `lotus_tcp_send_bytes`            |
| `std::io::tcp::__recv(fd, max)`                 | `(Int, Int) -> String`            | `lotus_tcp_recv`                  |

User code reaches for the high-level `Listener` / `Stream`
surface; reach for `__*` primitives only when composing
something the high-level surface can't yet express (e.g.
outbound connection via `__connect`).

## See Also

- [Roadmap](../roadmap.md) — Phase 3 v1.0 follow-ups (keep-alive,
  streaming bodies, listener bind-readiness).
- [`std::http`](../http.md) — composes on `Listener` and
  `Stream` to build an HTTP server.
- [`Bytes`](../bytes.md) — type used by `send_bytes`.
- `examples/http-hello/main.ap` (in the language repo) —
  Listener + on_connection + handler pattern.
- `crates/aperio-codegen/runtime/stdlib/io_tcp.ap` (in the
  language repo) — bundled source for the stdlib loci.
