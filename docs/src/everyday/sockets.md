# UDP & TLS

[HTTP](./http.md) covers the request/response server. Below it sit
two more transports: **UDP** for connectionless datagrams, and
**TLS** for an encrypted client connection. Both are thin wrappers
over the platform sockets — each call returns or takes a file
descriptor (an `Int`).

> For ordinary TCP request/response, prefer `std::http` or the
> `std::io::tcp` `Listener` / `Stream` loci. This chapter is the
> raw-datagram and TLS-client surface.

## UDP datagrams — `std::io::udp`

Bind a socket, then send and receive datagrams. `bind` and the I/O
calls are `fallible(IoError)`:

```hale
let fd = std::io::udp::bind("0.0.0.0", 9000) or raise;
std::io::udp::send(fd, "127.0.0.1", 9001, "ping") or raise;
```

To receive *and* learn who sent it, use `recv_with_source` and read
the thread-local source cache immediately after:

```hale
let msg  = std::io::udp::recv_with_source(fd, 1500) or raise;  // Bytes
let host = std::io::udp::last_source_host();
let port = std::io::udp::last_source_port();
println(host, ":", to_string(port), " sent ",
        to_string(len(msg)), " bytes");
std::io::udp::close(fd);
```

Datagram boundaries are preserved — one `send` is one `recv`.
Delivery is best-effort; layer acknowledgement or retry on top if
you need it. Multicast is a `join_group` away (`set_multicast_ttl`
/ `set_multicast_loop` tune it), and `set_recv_timeout(fd, 100ms)`
bounds a quiet `recv`.

### `Reader` — the ingest handle

For a signal reader that just wants each datagram with no per-message
allocation, reach for `std::io::udp::Reader` instead of hand-managing
a socket and a buffer:

```hale
locus Ingest {
    params {
        r: std::io::udp::Reader =
            std::io::udp::Reader { addr: "127.0.0.1", port: 9000, cap: 2048 };
    }
    run() {
        while true {
            let dg = self.r.next() or raise;   // parks until a datagram, zero-alloc
            // parse `dg` — a zero-copy view of this datagram's bytes
        }
    }
}
// placement { ing_owner: cooperative(pool = ing) where async_io; }
```

`Reader` bundles the bound socket and a single reused buffer. `next()`
binds lazily on the first call, refills the buffer in place (no
per-datagram allocation), and returns a **zero-copy view** aliasing it
— valid until the next `next()`, so read or copy out before looping.
On a `where async_io` pool `next()` parks on `EPOLLIN` (kernel-woken,
no busy-poll, no timeout quantum): an idle signal costs zero CPU and a
datagram wakes the worker in microseconds. It parks until data arrives,
so its only failures are a bind failure (first call) or a broken
socket — both exceptional, hence `or raise` (not `or discard`, which is
for Unit-returning fallibles). This is the event-driven, allocation-free
ingest shape as a default you reach for rather than one you assemble.

> **UDP as a bus transport.** The raw socket above is *not* the
> typed bus. To carry bus messages over UDP, use the
> `udp://host:port` substrate transport instead (see
> [the bus](../services/bus.md)) — same dispatch contract as
> `unix://`.

## TLS client — `std::io::tls`

`connect` does the TCP connection *and* the TLS 1.2+ handshake (SNI
+ system trust store) in one call, via the platform OpenSSL:

```hale
let h = std::io::tls::connect("example.com", 443) or raise;
std::io::tls::send_bytes(h, std::bytes::from_string(
    "GET / HTTP/1.0\r\nHost: example.com\r\n\r\n"));
let resp = std::io::tls::recv_bytes(h, 4096);   // Bytes
println(std::str::from_bytes(resp));
std::io::tls::close(h);
```

This is **client-side only** — there is no TLS *server* in the
stdlib. `set_recv_timeout(h, d)` bounds a read; with one set,
`recv_into` returns the `-2` "timed out, retryable" sentinel so a
long-lived client can run keep-alive work instead of hanging.

## Tuning sockets — `std::io::sockopt`

The UDP `set_option_int` / `set_option_bool` / `get_option_int`
calls take a `level` and `name` from `std::io::sockopt`'s named
constants, so you never hardcode a platform number:

```hale
std::io::udp::set_option_bool(
    fd, std::io::sockopt::SOL_SOCKET(),
    std::io::sockopt::SO_REUSEADDR(), true) or raise;
```

For TCP, `std::io::tcp::set_nodelay(fd, true)` is the common one
(disable Nagle for latency).

Next: hashing & encoding — [Hashing, encoding &
randomness](./crypto.md).
