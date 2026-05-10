# `std::io::tcp`

TCP networking for Aperio programs. Phase 1 (m73) ships
`Listener` — a stdlib locus that binds a TCP port and accepts
one incoming connection in its lifecycle, then closes cleanly.
The single-accept shape is the smallest testable unit; the
multi-accept loop and per-connection handler primitives
(`Stream`, `send`, `recv`) follow in a later milestone once
the shape of "how user code sees an accepted connection" has
been worked out.

## Loci

### `std::io::tcp::Listener`

#### Synopsis

```aperio
locus Listener {
    params {
        host: String = "127.0.0.1";
        port: Int = 0;
        listen_fd: Int = -1;     // overwritten in birth()
    }
    birth() { /* bind + listen */ }
    run()   { /* accept one connection, then return */ }
    dissolve() { /* close listen socket */ }
}
```

#### Grammar

A path-qualified locus instantiation:

```ebnf
listener_inst ::= "std" "::" "io" "::" "tcp" "::" "Listener"
                  "{" field_init ( "," field_init )* "}"
```

#### Semantics

- **birth()** binds an `AF_INET` `SOCK_STREAM` socket to
  `host:port`, sets `SO_REUSEADDR` to tolerate quick rebinds,
  and calls `listen(backlog=16)`. The listening file
  descriptor is stored on `self.listen_fd` for `run()` and
  `dissolve()` to read back.
- **run()** blocks on `accept()` until exactly one peer
  connects, prints a diagnostic line containing the accepted
  connection's fd, closes that connection, and returns. The
  Phase-1 Listener accepts a single connection by design;
  multi-accept loops become available when `Stream` and
  per-connection handler dispatch land.
- **dissolve()** closes the listening fd. Because the
  listening fd is not used between accepts, the OS port is
  released as soon as `dissolve()` runs.
- Errors at any step (bind failure, accept failure, close
  failure) print a `perror`-style diagnostic to stderr and
  return -1 from the underlying primitive; the lifecycle
  continues to the next stage rather than aborting.

#### Fields

- `host: String = "127.0.0.1"` — bind interface. Use
  `"0.0.0.0"` to bind on all interfaces.
- `port: Int = 0` — TCP port. `0` lets the OS pick a free
  ephemeral port.
- `listen_fd: Int = -1` — internal. The Listener locus
  overwrites this in `birth()`; user code does not set it.

#### Examples

Single-accept echo-style listener:

```aperio
fn main() {
    std::io::tcp::Listener {
        host: "127.0.0.1",
        port: 8080,
    };
}
```

Run that program; from another shell, `nc 127.0.0.1 8080`
connects, the Aperio program logs the accepted fd and
exits, freeing the port.

Default host (any host the OS resolves to localhost):

```aperio
fn main() {
    std::io::tcp::Listener { port: 8080 };
}
```

#### Limitations (Phase 1)

- **Single accept**: `run()` returns after one connection.
  Multi-connection servers wait on `Stream` + handler
  dispatch.
- **No send/recv on the accepted connection from user code**.
  m73 closes the accepted fd inside the Listener's `run()`
  body. Reading or writing on the accepted connection requires
  the future `Stream` locus and its `send` / `recv` methods.
- **AF_INET only**: IPv4. AF_INET6 is a follow-up.

## Internal primitives

The stdlib locus delegates to three internal `std::io::tcp::__*`
path-call primitives. These are not part of the user surface —
they exist only so the bundled stdlib source has a way to call
into the C runtime — but their shape is documented here for
implementers and curious readers:

| Primitive                                 | Type                                  | Backs                               |
|-------------------------------------------|---------------------------------------|-------------------------------------|
| `std::io::tcp::__listen_socket(host, port)` | `(String, Int) -> Int`               | `lotus_tcp_listen_socket` C runtime |
| `std::io::tcp::__accept_one(listen_fd)`     | `(Int) -> Int`                       | `lotus_tcp_accept_one` C runtime    |
| `std::io::tcp::__close_fd(fd)`              | `(Int) -> Int`                       | `lotus_tcp_close_fd` C runtime      |

Future stdlib loci (Phase 1 `Stream`, Phase 3 HTTP) extend
this internal-primitive set; the user surface stays at the
locus level.

## See Also

- [Roadmap](../roadmap.md) — Phase 1+ stdlib build-out plan.
- `spec/stdlib.md` (in the language repo) — path-resolution
  semantics, the m71 dispatcher, and design principles.
- `crates/aperio-codegen/runtime/stdlib.ap` (in the language
  repo) — bundled source for `__StdIoTcpListener` and any
  future stdlib loci.
