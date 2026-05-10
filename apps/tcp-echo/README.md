# tcp-echo

Bare TCP echo server with structured logging via `std::log`.

Composes three stdlib pieces:

- `std::io::tcp::Listener` (m83 multi-accept)
- `std::io::tcp::Stream` (m81/m82 — `recv`/`send` on let-bound handle)
- `std::log::{Logger, StdoutSink}` (m95 cascading-namespace logging)

## What it does

Listens on `127.0.0.1:<port>`. For each accepted connection:

1. Logs `info` on `net` — "connection accepted".
2. Reads up to 4096 bytes via `recv`.
3. Logs `info` on `net.echo` — "echoing N bytes" (the child sub-tree).
4. Sends the received bytes back unchanged.
5. Logs `info` on `net` — "connection closed".

Two Loggers in different sub-trees demonstrate the cascading
namespace: `net` is the root for connection-level events,
`net.echo` (child) is for per-echo data events. A single
`StdoutSink` subscribed to `log.**` sees both.

## Run

```sh
aperio build apps/tcp-echo/main.ap
./apps/tcp-echo/main                # default port 7777, 5 accepts
./apps/tcp-echo/main 9000           # port 9000, 5 accepts
./apps/tcp-echo/main 9000 -1        # port 9000, run until killed
```

`aperio run` does not work for this program because the
interpreter does not yet support qualified-name struct/locus
literals like `std::log::Logger { ... }`. Use `aperio build`.

## Smoke test

```sh
# terminal 1
./apps/tcp-echo/main 7777 3

# terminal 2
printf "hello\n" | nc -q1 127.0.0.1 7777
printf "world\n" | nc -q1 127.0.0.1 7777
printf "bye\n"   | nc -q1 127.0.0.1 7777
```

Expected server output:

```
[INFO net] listening on 127.0.0.1:7777 (max_accepts=3)
[INFO net] connection accepted
[INFO net.echo] echoing 6 bytes
[INFO net] connection closed
[INFO net] connection accepted
[INFO net.echo] echoing 6 bytes
[INFO net] connection closed
[INFO net] connection accepted
[INFO net.echo] echoing 4 bytes
[INFO net] connection closed
[INFO net] listener exited
```

(The Listener locus also prints `__StdIoTcpListener.birth` /
`.dissolve` lines around the accept loop; those come from the
stdlib locus, not from app code.)

## What is not done

- **No bytes-truncation handling.** A client sending more than
  4096 bytes in one write will have the tail dropped — the
  Stream API exposes only a single `recv` per call; there is no
  framing or "until EOF" loop yet.
- **No NUL-safe echo.** `Stream.send` truncates at embedded
  NULs (Aperio Strings are NUL-terminated). For binary-safe
  echo, `send_bytes` would be the right primitive but `recv`
  returns `String`, not `Bytes`, so a true byte-pipe would need
  a `recv_bytes` that the stdlib does not yet ship.
- **Synchronous accept loop.** One connection is fully handled
  before the next is accepted. No concurrency.
- **No graceful shutdown signal.** SIGINT terminates the
  process; `dissolve()` runs only when the bounded
  `max_accepts` is reached.

See `FRICTION.md` for friction notes that landed on the
language session.
