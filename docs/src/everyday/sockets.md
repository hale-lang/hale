# UDP & TLS

Below [`std::http`](./http.md), Hale provides low-level socket primitives in `std::io::udp`, `std::io::tls`, `std::io::tcp`, and `std::io::sockopt`. These modules wrap platform file descriptors (`Int`) and integrate with `where async_io` coroutine pools for non-blocking I/O.

```hale
locus Ingest {
    params {
        r: std::io::udp::Reader =
            std::io::udp::Reader { addr: "127.0.0.1", port: 9000, cap: 2048 };
    }
    run() {
        while true {
            let dg = self.r.next() or raise;   // parks until a datagram arrives, zero-alloc
            println(len(dg));
        }
    }
}

fn main() { }
```

> [!NOTE]
> For TCP servers (`std::io::tcp::Listener`, `Acceptor`, `Connections`, `Stream`) and Unix domain socket listeners (`std::io::tcp::Acceptor { path: "/run/app.sock" }`), see [HTTP § Concurrent Serving](./http.md#2-concurrent-serving).

---

## Socket & Transport Summary

| Module / Type | Primary Functions & Loci | Purpose |
|---|---|---|
| `std::io::udp` | `bind`, `send`, `recv_with_source`, `last_source_host`, `last_source_port`, `close`, `set_recv_timeout`, `join_group`, `set_multicast_ttl`, `set_multicast_loop` | Connectionless UDP datagram sockets and multicast |
| `std::io::udp::Reader` | `Reader { addr, port, cap }`, `next()` | Event-driven, zero-allocation UDP datagram reader |
| `std::io::tls` | `connect`, `connect_within`, `send_bytes`, `recv_bytes`, `recv_into`, `set_recv_timeout`, `close` | Client-side TLS 1.2+ connections (OpenSSL with SNI and system trust store) |
| `std::io::tcp` | `connect`, `connect_within`, `connect_wait`, `set_nodelay`, `set_recv_timeout`, `set_send_timeout`, `set_rx_timestamps`, `stop`, `Listener`, `Acceptor`, `Connections`, `Stream` | Outbound TCP dialing with deadlines/retries, socket tuning, and TCP/Unix listeners |
| `std::io::sockopt` | `SOL_SOCKET()`, `SO_REUSEADDR()`, etc. | Platform-independent socket option constants for `set_option_int` / `set_option_bool` / `get_option_int` |

---

## UDP Datagrams (`std::io::udp`)

### Binding, Sending, and Receiving

`bind` and UDP I/O calls are `fallible(IoError)` and operate on an integer file descriptor (`fd`):

```hale,fragment
let fd = std::io::udp::bind("0.0.0.0", 9000) or raise;
std::io::udp::send(fd, "127.0.0.1", 9001, "ping") or raise;
```

To receive a datagram and inspect the sender's address, call `recv_with_source` followed immediately by the thread-local accessors `last_source_host()` and `last_source_port()`:

```hale,fragment
let msg  = std::io::udp::recv_with_source(fd, 1500) or raise;  // Bytes
let host = std::io::udp::last_source_host();
let port = std::io::udp::last_source_port();
println(host, ":", to_string(port), " sent ",
        to_string(len(msg)), " bytes");
std::io::udp::close(fd);
```

- **Datagram boundaries**: Every `send` corresponds to a single `recv`. Delivery is best-effort.
- **Receive timeouts**: Call `std::io::udp::set_recv_timeout(fd, 100ms) or raise` to bound blocking receives.
- **Multicast**: Join a multicast group with `join_group` and configure multicast behavior with `set_multicast_ttl` and `set_multicast_loop`.

### Zero-Allocation Ingest (`std::io::udp::Reader`)

For high-rate UDP ingestion without per-message heap allocation, use `std::io::udp::Reader`:

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

- **Buffer reuse & view lifetime**: `Reader` owns the UDP socket and a single buffer of capacity `cap`. `next()` binds lazily on the first call, overwrites the internal buffer in place, and returns a zero-copy view (`BytesView`). Because the view aliases the internal buffer, it remains valid only until the next `next()` call—parse or copy out any data you need to keep before the next iteration.
- **`async_io` integration**: On a `where async_io` pool, `next()` parks on `EPOLLIN` (woken directly by the kernel with zero idle CPU and no polling interval).
- **Error handling**: Because `next()` parks until a datagram arrives, it only fails on an initial `bind` error or a broken socket. Always handle it with `or raise` rather than `or discard`.

> [!TIP]
> **UDP as a Bus Transport**: Raw `std::io::udp` sockets are distinct from Hale's typed message bus. To route bus topics over UDP or Unix domain sockets across processes, use the `udp://host:port` or `unix://` substrate transport bindings (see [The bus](../services/bus.md)).

---

## TLS Client & TCP Dialing (`std::io::tls` and `std::io::tcp`)

### Establishing a TLS Connection

`std::io::tls::connect` performs a TCP connection and a TLS 1.2+ handshake (including SNI and certificate verification against the system trust store via platform OpenSSL) in a single call:

```hale,fragment
let h = std::io::tls::connect("example.com", 443) or raise;
std::io::tls::send_bytes(h, std::bytes::from_string(
    "GET / HTTP/1.0\r\nHost: example.com\r\n\r\n"));
let resp = std::io::tls::recv_bytes(h, 4096);   // Bytes
println(std::str::from_bytes(resp));
std::io::tls::close(h);
```

`std::io::tls` is **client-side only** (there is no TLS server in the standard library).

### Connection Deadlines and Retries

Plain `connect` (`std::io::tcp::connect` and `std::io::tls::connect`) makes a single connection attempt:
- If no server is listening on the target port, `connect` fails immediately with `err.kind == "connection_refused"`.
- If the remote host is blackholed or a firewall silently drops the `SYN` packet, plain `connect` waits for the OS kernel timeout (~2 minutes).

Use `connect_within` and `connect_wait` to control dial behavior:

| Function | Behavior |
|---|---|
| `std::io::tcp::connect(host, port)` | Single attempt; fails immediately on `connection_refused`, waits for OS timeout on silent drop |
| `std::io::tcp::connect_within(host, port, deadline)` | Single attempt bounded by `deadline`; fails with `err.kind == "timeout"` on expiry or immediately on `connection_refused` |
| `std::io::tcp::connect_wait(host, port, wait_duration)` | Retries `connection_refused` until `wait_duration` expires (useful when spawning a local peer process that is still starting up) |
| `std::io::tls::connect_within(host, port, deadline)` | Bounds both the TCP connection and the TLS handshake by `deadline` (`err.kind == "timeout"` on expiry) |

```hale,fragment
let fd = std::io::tcp::connect_within("10.0.0.7", 5432, 2s) or raise;
let h  = std::io::tls::connect_within("api.example.com", 443, 3s) or raise;
```

### IPv4, IPv6, and `async_io` Behavior

- **IPv4 and IPv6**: All TCP, TLS, and UDP functions accept both IPv4 and IPv6 addresses. IPv6 literals may be written bare (`"::1"`) or bracketed (`"[::1]"`). Connecting to a hostname attempts every resolved address in order, so `"localhost"` connects to either IPv4 or IPv6 loopback. When stopping an IPv6 listener via `std::io::tcp::stop`, its default name uses brackets (`"[::1]:8080"`).
- **Non-blocking `async_io` coroutines**: When executed on a `where async_io` pool, TCP dials (`connect_within`, `connect_wait`), TLS handshakes, `send_bytes`, and reads park only the calling coroutine while other coroutines on the pool continue running.
- **TLS receive timeouts**: `std::io::tls::set_recv_timeout(h, 5s) or raise` bounds read operations. When a timeout is configured, `recv_into` returns the `-2` ("timed out, retryable") sentinel on timeout so long-lived clients can perform periodic keep-alive work.

---

## Socket Options (`std::io::sockopt`)

All socket option setters (`set_recv_timeout`, `set_send_timeout`, `set_nodelay`, `set_rx_timestamps`, `set_option_int`, `set_option_bool`) are `fallible(IoError)` and require an `or` handler at the call site.

`std::io::sockopt` exposes platform-specific socket constants as zero-argument functions for use with `std::io::udp::set_option_int`, `set_option_bool`, and `get_option_int`:

```hale,fragment
std::io::udp::set_option_bool(
    fd, std::io::sockopt::SOL_SOCKET(),
    std::io::sockopt::SO_REUSEADDR(), true) or raise;
```

For TCP sockets, disable Nagle's algorithm with `std::io::tcp::set_nodelay(fd, true) or raise`.

Next: [Hashing, encoding & randomness](./crypto.md).
