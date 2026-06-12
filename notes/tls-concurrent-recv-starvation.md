# Handoff: two concurrent blocking TLS recvs — the second is starved

**Status:** **REFRAMED 2026-06-12 (hale side).** The substrate is **not** the cause — see the Corrected Verdict below. The original fathom report (preserved in full beneath it) is the symptom record; its lead hypothesis (substrate scheduler / TLS-recv fairness) is **refuted**. The actual root cause is a missing connection-liveness check in pond's `WsClient.read_msg`, and the fix is pond-side with an already-shipped primitive.
**Area (corrected):** `pond/websocket/client.hl` `read_msg` (half-open detection / recv timeout). **NOT** `std::io::tls` or the pinned-locus scheduler.
**Severity:** real for any sparse/idle long-lived stream, but it's a connection-liveness bug, not a concurrency limit. Multi-connection ingest is *not* blocked by the substrate.

---

## CORRECTED VERDICT (2026-06-12) — substrate exonerated; it's a half-open-connection hang in pond

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
| fathom instantiation | correct | `reader_eth` / `reader_base` = two independent `pinned(core=6/7)` loci |

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

fathom's `apps/mdgw/evm` is a demand-driven on-chain market-data gateway. To serve two chains it runs **one pinned `EvmReader` locus per chain**, each a pond `ws::WsClient` on its own TLS connection to a JSON-RPC websocket:

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

The maintainers know the I/O model; the above is fathom's outside-in inference. The actionable, verified fact is: **two simultaneous blocking TLS `recv_into`s across two pinned loci do not both make progress; the busier one wins and the other never does.**

## Compounding issue (pond, separate but related)

pond's `read_msg` does not enforce `pong_timeout` (see `vendor/pond/websocket/FRICTION.md`). So a starved/half-open recv is **never detected** — it hangs until the OS TCP timeout (minutes) rather than surfacing as a recoverable drop+reconnect. This turns the starvation from "degraded" into "silently dark." Independently worth fixing via `std::io::tcp::set_recv_timeout` in pond's read loop, but it only masks, not fixes, the underlying starvation.

## Minimal reproducer (recipe)

The shape, reducible to a standalone Hale binary with no fathom deps:

```
main:
  spawn two pinned loci, A (core N) and B (core N+1).
  A: open a TLS websocket to a BUSY public stream; loop { recv_into; count++ }.
  B: open a TLS websocket to a QUIET public stream; loop { recv_into; count++ }.
  every 1s, print A.count and B.count.
Expected (correct): both counts climb.
Observed (bug):     A climbs continuously; B stays at 0 (or only its handshake), forever.
```

A reliable busy/quiet pair: two `*-rpc.publicnode.com` chains, both `eth_subscribe("logs", {address: <a pool>})`, where chain A's pool/连 sees more traffic than chain B's — or simply A subscribes `newHeads` on a fast chain and B subscribes `logs` on a single low-traffic contract. fathom can supply a packaged standalone repro on request; the live gateway (`apps/mdgw/evm`, run with both `rpc_ws ETH …` and `rpc_ws BASE …` configured) reproduces it deterministically today.

## Candidate fixes (substrate)

> **SUPERSEDED — see the Corrected Verdict at the top.** Fix #1 below is a
> dead end (the substrate scheduler is not the cause; two pinned blocking
> recvs run in true parallel). Fix #2 (recv timeout / `pong_timeout`) is the
> *actual* fix, applied pond-side in `read_msg`. Left here as the original
> outside-in reasoning.

1. **Make `recv_into` cooperative / fairly scheduled across pinned loci** — the real fix. A blocking TLS read on one pinned locus must not prevent a blocking TLS read on another from being serviced (yield to the scheduler while waiting on the socket; or back blocking reads with a fair readiness poll). This unblocks N-connection ingest generally.
2. **(pond, mitigation)** enforce `pong_timeout` in `read_msg` via `std::io::tcp::set_recv_timeout` so a starved/half-open recv is detected and reconnected — converts a silent hang into a recoverable drop. Does not fix starvation.
3. **(documentation, if #1 is far off)** document the limitation and bless the app-side workaround: a single-threaded non-blocking/poll multiplex over both sockets in one locus (one `recv` with a timeout, round-robin the connections), or one OS process per connection. fathom can ship base/aero today via one gateway process per chain — but that is a workaround for a substrate limitation that every multi-connection ingest path will otherwise re-hit.

## Impact / scope

Any Hale binary needing two or more concurrent long-lived TLS read streams across pinned loci. For fathom specifically: the multi-chain DEX md gateway (Ethereum + Base + future Arbitrum/BSC/Optimism), and more broadly any ingest that fans in several authenticated streaming sources. Single-connection gateways (all current CeFi mdgws, the single-chain ETH evm gateway) are unaffected.

## Pointers

- Demonstrating case + full evidence log: `fathom/apps/mdgw/evm/FRICTION.md` (§ "two concurrent blocking TLS recvs").
- Gateway code (multi-chain structure is complete + correct; it is purely blocked here): `fathom/apps/mdgw/evm/main.hl` (`EvmReader` per chain, `EvmRepublisher` shared demand).
- Verified Aerodrome/Solidly event topic0s and decode (unrelated to the bug, but in the same file) are also recorded in the fathom FRICTION.
- Related pond gap: `pond/vendor/pond/websocket/FRICTION.md` (`pong_timeout` not enforced).
