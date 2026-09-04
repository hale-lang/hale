# fuse-hl — the Hale fuse

The fusion + serve layer of iris, in Hale. `consumer/fuse.c`
stays as the C reference implementation (differential
testing: same segments in, same tables out); this is the one
iris actually ships. The split follows `observe/` on the
emitter side: C glue only where mmap'd shm can't be Hale yet
(`attach/glue.c` over `obs_attach.c`), everything above it —
topic join, seq matching, the live-locus table, snapshot
JSON, HTTP + SSE — pure Hale.

```
hale build .
./fuse-hl [port=8787] [webroot=../../render/web] [topology.json]
```

- `GET /`          the flower (render/web, static)
- `GET /snapshot`  fused state as JSON (monotonic totals;
                   clients derive rates from consecutive
                   frames). This is the surface the MCP
                   co-debugger will read (DESIGN §11).
- `GET /events`    SSE stream of the same JSON, server-capped
                   (10 Hz default) via std::http connection
                   takeover + a borrowed tcp Stream.

The optional third argument is a topology artifact (`hale check
. --dump-topology=...`). With it, fuse-hl verifies the
`artifact_digest`, ingests the model's claims/groups/adequacy,
watches the file for digest changes (1 Hz — a ride-along re-cut
lands within a second), and adds a `law` section to `/snapshot`:
each claim with its STATIC verdict and its WITNESSED state
against the observed fleet (INSPECTOR.md § *Witnessed law*).
The frontend's law view (key `[3]`) renders it. `examples/
claims-demo` drives every witnessed state.

Architecture notes:

- **Per-cycle child locus.** All per-iteration work (event
  drain, FFI name strings, snapshot building) runs inside a
  `Cycle` child born and dissolved every loop — its region
  reclaims at dissolve. This was the fix for `hale check`'s
  unbounded-allocation advisories on the first draft (which
  leaked ~2 MB/s exactly as warned).
- **Cross-pool cells.** The http handler (io pool) reads the
  latest snapshot from a `sync = serialized` hashmap the
  fusion loop writes; SSE fds claimed by the handler are
  pumped by the fusion loop the same way (metrics-Registry
  discipline).
- **Witness tables.** BUS_PUBLISH/BUS_DELIVER records carry
  the acting locus instance (PROTOCOL §8 attribution); the
  fusion loop maps those to locus TYPES per fused topic, which
  is the evidence the law evaluator quantifies over. An
  unattributed record (`"?"`) makes the evaluator abstain
  (`unwitnessed`), never guess.
- **SSE framing.** `/snapshot` may carry pretty-printed JSON
  fragments spliced verbatim from the artifact (adequacy,
  capabilities, groups). Those are collapsed to one physical
  line before the SSE frame — a literal newline in `data:`
  splits the frame and the client parses only its first line.
  (Found the hard way; the fix is `oneline()`.)

## Verified

Against the live synth pair, the snapshot's tables match the
C reference: same topics/shapes, supervision trees
(Main→Supervisor→Producer / Router+Workers) with churn
visible, and the long-running market-data pair's edge read
73.3µs mean / 151.6µs max — the exact numbers the 2026-07-22
verification recorded via the C path.

Toolchain: builds, checks and **verifies clean** on hale
v0.16.0+ (re-confirmed 2026-08-11 — `hale check` 0 errors,
`hale verify` 0 findings, live synth→snapshot loop green).

## Known gaps (2026-08-11, re-measured against hale v0.16.0+)

- ~~Event-drain throughput~~ **FIXED**: `fz_next_batch` packs
  up to 512 events per FFI call into a reusable Bytes buffer
  (32B records), decoded with `std::bytes` reads and w1
  unpacked with native shifts. 60s soak at ~500k records/s
  with an SSE client attached: zero overruns, zero zombie
  loci, all restarts tracked. A stalled SSE client is bounded
  by SO_SNDTIMEO (200ms) instead of stalling the pump.
- ~~**RSS growth root-caused to `@form(vec).set`**~~ **FIXED
  upstream — no periodic restart needed.** The bug was real:
  each fallible `.set or discard` leaked ~33B into a region
  that survived child dissolve and cost ~1µs (~1000x the
  inlined `.get`), so fuse-hl's ~1M sets/s on the event path
  grew ~MB/s. Re-measured 2026-08-11 on hale v0.16.0+ with the
  same repro: **100M sets peak at 7.9 MB and run in 0.60 s** —
  flat, and ~6 ns per get+set pair. (Before: 2M sets → 70 MB.)
  `upstream-repro/repro3.hl` stays in-tree as the regression
  probe; `repro.hl`/`repro2.hl` are the flat controls. Most
  likely closed by the GH #383/#402 factory-locus reclaim work
  rather than a targeted fix, so it is worth re-running rather
  than assuming.
- ~~**`hale verify` advisory gate**~~ **CLOSED — `verify` is
  green (0 findings).** Upstream sharpened the analysis (26
  advisories → 9: const-bounded init loops like
  `while i < NET_SLOTS * WINDOW` are now proven bounded) and
  shipped the suppression annotation that did not exist when
  this was written. The residual 9 were all one true shape the
  analysis cannot see — `Cycle::snapshot`'s string building —
  and that fn now carries `@unbounded` with the reasoning
  written at the site: `Cycle` *is* the per-iteration child, so
  its region reclaims at dissolve. The annotation is greppable
  by design; it is an acknowledgement, not a silencer.
- ~~**Late attach loses structure**~~ **FIXED upstream** (hale
  v0.11.18). Both candidate fixes were listed here; upstream
  took the first. A detached heartbeat thread drives the gate
  check every 250 ms under `LOTUS_OBS=1`, so the
  `observer_count` 0→1 birth replay fires even for a process
  that emits no probes at all (the failing shape was a long
  read loop with pinned readers). Post-attach replay latency is
  bounded at ~250 ms. Cost: the heartbeat claims one ring slot
  — a fleet pinning `LOTUS_OBS_RINGS` to its exact thread count
  should add one (PROTOCOL §14).

Everything above is closed, which makes the one item below the
whole list:

- **No backpressure readout — blocked upstream** (iris
  handoff-10, P22). fuse-hl fuses the counter lines it can, but
  `queue_depth`, `send_block_ns` and `retries` (binding cells
  3–5, PROTOCOL §6) are written by no hale release — the native
  emitter populates cells 0/1/2 only. So the snapshot can show
  an edge losing messages but not an edge *filling up*, which
  is the half of DESIGN §7 that distinguishes lossy from
  saturated. Consumer side is cheap once the cells are live
  (they are reads on a line already fused); nothing to build
  here until then.
