# Handoff: two concurrent blocking TLS recvs — the second is starved

**Status:** **RE-AUDITED 2026-06-13 (hale side).** Two distinct symptoms have been reported under this title. (1) The *original* "blocks forever, zero notifications" is a half-open-connection hang in pond's `read_msg` — see the Corrected Verdict (2026-06-12) below; pond-side fix, primitive shipped. (2) a downstream app's *refined* repro (`a WS-concurrency repro`) shows a **different** symptom — the quiet connection gets a **valid** sub-ack but no pushes, payload-size-dependent — i.e. a *garbled subscribe frame*, not a hang. The substrate was re-audited for (2): a real latent shared-arena race was found and fixed (`lotus_tls_recv_bytes`), **but both of pond's actual WS paths were proven not to touch it**, so symptom (2) is still open and pond-side. See the 2026-06-13 section directly below.
**Area (corrected):** `pond/websocket/client.hl` `read_msg` (half-open detection / recv timeout). **NOT** `std::io::tls` or the pinned-locus scheduler.
**Severity:** real for any sparse/idle long-lived stream, but it's a connection-liveness bug, not a concurrency limit. Multi-connection ingest is *not* blocked by the substrate.

---

## UPDATE 2026-06-13 — garbled-subscribe-frame symptom: substrate re-audited, latent recv_bytes race fixed, pond WS paths exonerated

a downstream app's `a WS-concurrency repro` repro is a **different** failure mode
from the half-open hang: the quiet connection receives a **valid
subscribe-ack** (so the socket is alive and bidirectional — not half-open),
then gets **zero pushes** with **flat `bytes_received`**, and the failure is
**payload-size-dependent** (small `newHeads` survives; the larger
`eth_subscribe(logs, …)` filter fails; two `newHeads` both stream; a single
log-sub alone works). That fingerprint is a **corrupted/garbled subscribe
frame** — the envelope parses (valid ack) but the `params` filter is mangled,
so it matches nothing and never pushes.

### Hypothesis tested: per-byte `from_int` racing a shared arena

pond's `emit_frame` masks the payload with one `std::bytes::from_int` per
byte. `from_int` routes through `lotus_caller_arena_or_global()`. **If** the
per-thread `caller_arena` TLS is unset, that falls back to the single global
`g_bus_payload_arena`, and `from_int` then allocates via `lotus_arena_alloc`
whose bump (`c->used`) is **unlocked**. Two pinned WsClients building frames
concurrently into that one shared arena would race the bump → overlapping
1-byte temporaries → wrong bytes appended → garbled frame, worse for larger
payloads. Fits every symptom.

### Result: hypothesis REFUTED for pond's paths (but a real adjacent race found)

A direct repro (`/tmp/arena_race.hl`: two `pinned(core=6/7)` loci each
building 48-byte patterns via per-byte `from_int`+`append` and checksumming,
200k iters) reported **`corrupt=0` with *and* without the fix.** That means
**`caller_arena` is per-thread during a pinned `run()`** — so `from_int`
there allocates in the per-thread scratch, never the shared arena. The
`emit_frame` hypothesis is **wrong**.

Tracing the two pond WS paths to the metal confirms **neither touches the
shared arena**:

| pond WS path | runtime | shared arena? |
|---|---|---|
| recv — `recv_into` | `lotus_tls_recv_into` → `SSL_read(ssl, tail, n)` straight into the per-instance `rx_buf` | **no** — no arena alloc at all |
| send — `emit_frame` per-byte `from_int` | `lotus_caller_arena_or_global()` → per-thread `caller_arena` in `run()` (validated `corrupt=0`) | **no** — per-thread scratch |

So the substrate does **not** corrupt pond's frames. Symptom (2) originates
in pond's own `.hl` logic or a concurrency interaction not yet reproduced.

### The real race that *was* found and fixed (separate path, not pond's)

`lotus_tls_recv_bytes` allocates its blob from `lotus_bus_payload_arena_get()`
— the **shared global arena, always, ignoring `caller_arena`** — via a
lock-free `lotus_arena_alloc`. So **two pinned loci calling
`std::io::tls::recv_bytes` concurrently race the unlocked bump** and corrupt
each other's received blobs. (`lotus_tcp_recv_bytes` uses
`caller_arena_or_global`, so it's per-thread-safe when `caller_arena` is set,
i.e. in `run()`/handlers.) Fix (lotus_arena.c): a per-arena
`shared_concurrent` flag, set only on `g_bus_payload_arena`, makes
`lotus_arena_alloc` serialize its bump on the arena's `subregion_lock`.
Per-instance arenas keep the lock-free fast path (validated unchanged by the
`corrupt=0` repro). This is genuine hardening — but pond's WsClient uses
`recv_into`, **not** `recv_bytes`, so it does **not** explain a downstream app's bug.

### Next step for symptom (2) — instrument the actual bytes (pond-side)

Static analysis + the substrate audit have exonerated the runtime; the
divergence is now best found by capturing it directly. In
`a WS-concurrency repro`, hexdump the **exact bytes handed to `write_all`**
for the quiet connection's subscribe frame, and compare against the bytes a
**working single-connection** run writes for the same subscribe. Whatever
differs (a mangled length, a wrong mask, a truncated payload, interleaved
writes to the same fd) localizes it to the producing `.hl` code. Candidates
to scrutinize in pond, in order: (a) any **shared mutable state** in the
frame/subscribe builder reused across the two pinned clients (a module-level
or accidentally-shared buffer/seed); (b) `write_all` partial-write handling
under concurrent sends; (c) the `mask_seed` LCG if two clients can ever share
one instance. The substrate guarantees per-instance locus fields are not
shared — so the leak, if real, is a pond-level aliasing of a builder/seed.

The substrate-scheduler / TLS-recv-fairness hypothesis was tested layer by
layer with dependency-free repros (timed, not judged by cross-thread print
order — an early read of mine was misled by stdout buffering and corrected).
**Every layer handles two concurrent connections correctly:**

| Layer | Verdict | Evidence |
|---|---|---|
| Pinned-locus scheduler | true parallel OS threads | two pinned CPU spinners finish in 1× wall time (`pthread_create` per pinned locus, join deferred to scope exit — `instantiation.rs:2098–2444`) |
| Blocking TCP recv | both progress | two pinned readers, one hammered + one trickled, both reach their target counts (timed) |
| `lotus_tls` recv path | clean | `lotus_tls_recv_into` is **lock-free per-connection** — no lock held across `SSL_read` |
| `lotus_tls` connect | clean | `g_tls_mutex` is released **before** the blocking `SSL_connect` (handshakes don't serialize) |
| OpenSSL | clean | 3.0.13 — per-object thread-safe, no app locking callbacks needed |
| pond `WsClient` buffers | per-instance | `rx_buf` / `frag_buf` are per-locus child `BytesBuilder`s, not shared |
| pond `read_msg` loop | per-instance, no shared state | single-threaded cooperative peel loop, nothing two clients contend on |
| a downstream app instantiation | correct | `reader_eth` / `reader_base` = two independent `pinned(core=6/7)` loci |

**Root cause: `read_msg` has no liveness check.** The loop reacts only when
`recv_into_rx()` *returns* `≤ 0` (clean EOF/error). A **half-open** socket —
silently dropped by a NAT/firewall idle timeout or a server-side drop, with
no FIN and no error — leaves `SSL_read` blocked **forever**. The loop wedges:
no return, no reconnect, no error. Exactly the symptom ("blocks forever,
zero notifications").

**Why the *quiet* / "second" connection.** A quiet stream is far more prone
to silent half-open: sparse traffic lets NAT mappings expire and idle
timeouts fire, with nothing flowing to surface the break. A busy stream
stays warm (constant traffic) *and* surfaces any break instantly (next recv
returns `≤ 0` → reconnect). So it isn't "busy monopolizes" — it's "**quiet
dies silently and is never noticed**." The "second concurrent recv"
correlation is incidental: in the gateway the second-subscribed pool is the
quieter one. (Consistent with the control test: a standalone BASE client
that keeps getting messages simply didn't go half-open in that window — it's
intermittent and NAT/traffic-dependent.)

**The fix is pond-side, primitive already shipped.** Enforce liveness in
`read_msg`: wrap `recv_into_rx` with `std::io::tcp::set_recv_timeout`
(shipped, hale #15) so a stalled recv **times out → reconnect** instead of
blocking forever, and/or send proactive pings on `ping_interval` and treat a
missed pong (`pong_timeout`) as a dead connection. This is what the original
report listed as *mitigation #2* — but it's the **actual fix**, not a mask.
Mitigation #1 (make the substrate scheduler fair) is a dead end: there is
nothing to fix in the substrate.

**One confirming test** (distinguishes this from a hypothetical deeper
OpenSSL-read bug): `strace -f -e trace=network -p <gateway pid>` while it's
wedged and watch `reader_base`'s fd. (a) `recvfrom`/`SSL_read` blocked with
**no** incoming data → the half-open hang (this verdict). (b) data arriving
but the read never returns → reopen the substrate hunt. Strong prior after
clearing every layer: (a).

**Repros (dependency-free, in `/tmp` during the investigation; reconstructable
from the recipes here):** two pinned CPU spinners (parallel, 1× time); a
pinned spinner + sleeper (parallel); two pinned TCP readers, busy + quiet,
both reaching their counts. None reproduce starvation — because the substrate
doesn't have it.

---

## One-line

When two pinned loci each hold their own blocking TLS connection and call `std::io::tls::recv_into` in a loop, a **busy** connection monopolizes the runtime and the **second** connection's `recv_into` blocks forever — it never returns, even though data is arriving on its socket.

## Symptom (the concrete case)

The downstream multi-chain gateway is a demand-driven on-chain market-data gateway. To serve two chains it runs **one pinned `EvmReader` locus per chain**, each a pond `ws::WsClient` on its own TLS connection to a JSON-RPC websocket:

- `reader_eth` (pinned core 6) → `wss://ethereum-rpc.publicnode.com`, `eth_subscribe("logs", …)` on a Uniswap-V3 pool (a swap ≈ every 12s, ~1KB notifications, plus other log traffic on the connection).
- `reader_base` (pinned core 7) → `wss://base-rpc.publicnode.com`, `eth_subscribe("logs", …)` on an Aerodrome pool (a swap ≈ every 14s).

Both connect, send their subscribe frame, and receive the subscription-id ack. Then:

- `reader_eth.read_msg()` returns notifications continuously and decodes them correctly — forever.
- `reader_base.read_msg()` → `std::io::tls::recv_into` **blocks forever and returns zero notifications**, despite the BASE socket having data to deliver.

## Why it is the substrate, not the application

Every application-side cause was eliminated, live, on 2026-06-12:

1. **Not the endpoint / subscription / decode.** A **standalone single-client** pond `WsClient` (same library, same subscribe frame, same TLS endpoint) against the *same* BASE pool receives Sync+Swap notifications every ~14s with zero reconnects — proven repeatedly, **including while the full gateway is running concurrently from the same host/VPN exit IP**. So it is not the BASE endpoint, not the topics, not a decode bug, and not server-side IP rate-limiting.

2. **Not chain- or core-specific.** Swapping which chain each reader serves (ETH↔BASE) while keeping the cores fixed did **not** move the failure to ETH. ETH always streams; the *other* reader always starves — regardless of chain identity, endpoint, or which core/locus it is. **The starved party is whichever is the second concurrent blocking TLS recv.**

3. **The busy socket monopolizes.** The continuously-active connection (large, frequent messages) appears to hold the runtime such that the quieter connection's blocking `recv_into` is never serviced.

## Root-cause hypothesis

`std::io::tls::recv_into` is a *blocking* read that does not cooperatively yield while waiting, and/or the runtime does not fairly schedule blocking I/O across pinned loci. A connection with steady inbound traffic keeps its `recv_into` returning and re-entering, monopolizing whatever shared resource (event loop, poll set, lock) backs TLS reads, and starves a second pinned locus blocked in its own `recv_into`.

The maintainers know the I/O model; the above is a downstream app's outside-in inference. The actionable, verified fact is: **two simultaneous blocking TLS `recv_into`s across two pinned loci do not both make progress; the busier one wins and the other never does.**

## Compounding issue (pond, separate but related)

pond's `read_msg` does not enforce `pong_timeout` (see `vendor/pond/websocket/FRICTION.md`). So a starved/half-open recv is **never detected** — it hangs until the OS TCP timeout (minutes) rather than surfacing as a recoverable drop+reconnect. This turns the starvation from "degraded" into "silently dark." Independently worth fixing via `std::io::tcp::set_recv_timeout` in pond's read loop, but it only masks, not fixes, the underlying starvation.

## Minimal reproducer (recipe)

The shape, reducible to a standalone Hale binary with no a downstream app deps:

```
main:
  spawn two pinned loci, A (core N) and B (core N+1).
  A: open a TLS websocket to a BUSY public stream; loop { recv_into; count++ }.
  B: open a TLS websocket to a QUIET public stream; loop { recv_into; count++ }.
  every 1s, print A.count and B.count.
Expected (correct): both counts climb.
Observed (bug):     A climbs continuously; B stays at 0 (or only its handshake), forever.
```

A reliable busy/quiet pair: two public-RPC-endpoint chains, both `eth_subscribe("logs", {address: <a pool>})`, where chain A's pool sees more traffic than chain B's — or simply A subscribes `newHeads` on a fast chain and B subscribes `logs` on a single low-traffic contract. a downstream app can supply a packaged standalone repro on request; the live gateway (the multi-chain gateway, run with both `rpc_ws ETH …` and `rpc_ws BASE …` configured) reproduces it deterministically today.

## Candidate fixes (substrate)

> **SUPERSEDED — see the Corrected Verdict at the top.** Fix #1 below is a
> dead end (the substrate scheduler is not the cause; two pinned blocking
> recvs run in true parallel). Fix #2 (recv timeout / `pong_timeout`) is the
> *actual* fix, applied pond-side in `read_msg`. Left here as the original
> outside-in reasoning.

1. **Make `recv_into` cooperative / fairly scheduled across pinned loci** — the real fix. A blocking TLS read on one pinned locus must not prevent a blocking TLS read on another from being serviced (yield to the scheduler while waiting on the socket; or back blocking reads with a fair readiness poll). This unblocks N-connection ingest generally.
2. **(pond, mitigation)** enforce `pong_timeout` in `read_msg` via `std::io::tcp::set_recv_timeout` so a starved/half-open recv is detected and reconnected — converts a silent hang into a recoverable drop. Does not fix starvation.
3. **(documentation, if #1 is far off)** document the limitation and bless the app-side workaround: a single-threaded non-blocking/poll multiplex over both sockets in one locus (one `recv` with a timeout, round-robin the connections), or one OS process per connection. a downstream app can ship base/aero today via one gateway process per chain — but that is a workaround for a substrate limitation that every multi-connection ingest path will otherwise re-hit.

## Impact / scope

Any Hale binary needing two or more concurrent long-lived TLS read streams across pinned loci. For a downstream app specifically: the multi-chain DEX md gateway (Ethereum + Base + future Arbitrum/BSC/Optimism), and more broadly any ingest that fans in several authenticated streaming sources. Single-connection gateways (all current CeFi mdgws, the single-chain ETH evm gateway) are unaffected.

## Pointers

- Demonstrating case + full evidence log: the downstream gateway issue log (§ "two concurrent blocking TLS recvs").
- Gateway code (multi-chain structure is complete + correct; it is purely blocked here): the downstream gateway source (`EvmReader` per chain, `EvmRepublisher` shared demand).
- Verified Aerodrome/Solidly event topic0s and decode (unrelated to the bug, but in the same file) are also recorded in the downstream issue tracker.
- Related pond gap: `pond/vendor/pond/websocket/FRICTION.md` (`pong_timeout` not enforced).
