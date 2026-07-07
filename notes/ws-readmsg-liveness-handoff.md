# Handoff: connection-liveness for pond WsClient.read_msg (the real fix for the "TLS recv starvation")

**Context.** The a downstream app-reported "two concurrent blocking TLS recvs — the
second starves" is **not** a substrate bug. Full corrected verdict in
`notes/tls-concurrent-recv-starvation.md`. Short version: every substrate /
TLS / OpenSSL / pond-buffer layer handles two concurrent connections
correctly; the symptom is a **half-open socket** (silent NAT/idle drop, no
FIN/error) that `pond/websocket/client.hl` `read_msg` never detects, so
`SSL_read` blocks forever. The quiet/"second" connection is the victim
because sparse traffic invites silent half-open. The fix is **connection
liveness in `read_msg`** — and it splits into a small hale-side prerequisite
and the pond-side change.

The current wedge, precisely (`client.hl`):

```hale
fn read_msg() -> Bool {
    while !got_message && !bail {
        ...
        let r = self.try_peel_one();
        if r == 0 {
            let got = self.recv_into_rx();   // blocks in SSL_read FOREVER on half-open
            if got <= 0 { ...reconnect/bail... }   // only ever fires on EOF/error
        }
        ...
    }
}
fn recv_into_rx() -> Int {
    let got = std::io::tls::recv_into(self.tls, self.rx_buf, self.recv_chunk);
    ...                                       // returns SSL_read's >0/0/<0
}
```

`self.tls` is a TLS **handle** (index into the runtime's entry table), not a
raw fd. `self.ping_interval` (30s) and `self.pong_timeout` (10s) params
already exist but are documented as "FRICTION items" — i.e. **not enforced**.

---

## Part A — hale-side prerequisite (small, well-scoped)

Two gaps block any liveness scheme, both in the language repo:

### A1. `std::io::tls::set_recv_timeout(handle, dur)` (+ `set_send_timeout`)

`std::io::tcp::set_recv_timeout` exists (hale #15) but takes a raw TCP fd;
TLS connections are addressed by handle. Add the TLS-handle variant:

- **Runtime** (`lotus_tls.c`): `int lotus_tls_set_recv_timeout_ns(int handle,
  int64_t ns)` — look up `g_tls_entries[handle].raw_fd`, call the existing
  `sock_set_timeout_ns(raw_fd, SO_RCVTIMEO, ns)` (already used by
  `lotus_tcp_set_recv_timeout_ns` in `lotus_arena.c:7929`). One-liner over
  the existing helper.
- **Codegen + stdlib surface**: dispatch `["std","io","tls","set_recv_timeout"]`
  the same way the tcp/udp ones are wired; add the `std::io::tls`
  declaration. Mirror the tcp shape exactly.

### A2. `tls::recv_into` must distinguish TIMEOUT from FATAL

Today `lotus_tls_recv_into` returns `-1` for *every* `SSL_read` error,
including the `SSL_ERROR_WANT_READ` that `SO_RCVTIMEO` produces on a timeout
(underlying `recv` → `EAGAIN`/`EWOULDBLOCK`). pond can't tell "timed out,
run liveness" from "connection is dead." Make the timeout a distinct,
**non-fatal** signal:

- On `SSL_read` returning `< 0`, inspect `SSL_get_error`. If
  `SSL_ERROR_WANT_READ`/`WANT_WRITE` (the SO_RCVTIMEO case on a blocking fd),
  return a distinguished sentinel — e.g. **`0` is taken (EOF), so return a
  dedicated value like `-2` ("would-block / timed out, retryable")** and keep
  `-1` for genuinely fatal. Document the three-way contract on the fn.
- Same treatment for `std::io::tcp::recv_into` for symmetry (so a TCP
  WsClient gets the same liveness path).

(Both A1 and A2 are independent of pond and gate any liveness fix; do them
first. Each is a few lines + a dispatch arm + a test.)

---

## Part B — pond-side fix (the actual liveness state machine)

A naive `SO_RCVTIMEO = pong_timeout` is **wrong** for sparse streams: a
healthy BASE connection with ~14s between swaps would time out at 10s and
reconnect on every quiet gap. The recv timeout must drive a **ping/pong
liveness** state machine, not an immediate reconnect.

Shape (in `WsClient`):

1. After `connect_once` sets `self.tls`, call
   `std::io::tls::set_recv_timeout(self.tls, self.ping_interval)` so a stalled
   `recv` returns periodically instead of blocking forever.
2. Track `last_recv_at`, `last_ping_at`, `awaiting_pong: Bool` (use the
   existing `std::time` monotonic source).
3. In `read_msg`, when `recv_into_rx()` returns the **timeout sentinel**
   (`-2` from A2), do **not** treat it as a lost connection — run the
   liveness check instead:
   - if `awaiting_pong` and `now - last_ping_at > pong_timeout` → connection
     is **dead** → `close_sock` + reconnect (or bail per `auto_reconnect`).
   - else if `now - last_recv_at > ping_interval` and not `awaiting_pong` →
     send a ping (the lib already replies to peer pings; add the proactive
     send), set `awaiting_pong = true`, `last_ping_at = now`.
   - else → loop and recv again.
4. On any data/pong received, `last_recv_at = now`, `awaiting_pong = false`.
   (`try_peel_one` already routes pongs — clear the flag there.)
5. Keep `got <= 0` (the real `-1`/EOF) as the existing "connection lost →
   reconnect" path.

Net behavior: a half-open quiet connection sends a ping at `ping_interval`,
gets no pong, and is declared dead + reconnected within
`ping_interval + pong_timeout` — instead of hanging forever. A healthy quiet
connection answers the ping and keeps streaming. No per-gap reconnect churn.

**Interim (if proactive ping is too much for v1):** set the recv timeout to a
value safely above the max expected inter-message gap (e.g.
`ping_interval + pong_timeout`) and treat the timeout sentinel as
"dead → reconnect." Catches half-open within that window; the risk is a false
reconnect on a stream genuinely idle longer than the window. The ping/pong
machine (B above) is the robust version and the one to ship.

## Hand-off split

- **language repo (hale):** A1 + A2. Small, gated, do first. (I can take
  these — they mirror the shipped tcp/udp timeout primitives.)
- **pond:** B, on top of A. Enforces the `ping_interval` / `pong_timeout`
  params that already exist but are inert. Drop the "FRICTION item" note on
  them once shipped.

## Confirming the diagnosis first

Before building, one `strace -f -e trace=network -p <wedged gateway pid>`
settles it: `reader_base`'s fd blocked in `recvfrom`/`SSL_read` with **no**
incoming data ⇒ this handoff is correct. Data arriving but the read never
returning ⇒ a deeper TLS-read bug — reopen the substrate hunt instead.
