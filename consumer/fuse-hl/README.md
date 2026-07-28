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
./fuse-hl [port=8787] [webroot=../../render/web]
```

- `GET /`          the flower (render/web, static)
- `GET /snapshot`  fused state as JSON (monotonic totals;
                   clients derive rates from consecutive
                   frames). This is the surface the MCP
                   co-debugger will read (DESIGN §11).
- `GET /events`    SSE stream of the same JSON, server-capped
                   (10 Hz default) via std::http connection
                   takeover + a borrowed tcp Stream.

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

## Verified

Against the live synth pair, the snapshot's tables match the
C reference: same topics/shapes, supervision trees
(Main→Supervisor→Producer / Router+Workers) with churn
visible, and the long-running market-data pair's edge read
73.3µs mean / 151.6µs max — the exact numbers the 2026-07-22
verification recorded via the C path.

## Known gaps (2026-07-27, updated after batch drain)

- ~~Event-drain throughput~~ **FIXED**: `fz_next_batch` packs
  up to 512 events per FFI call into a reusable Bytes buffer
  (32B records), decoded with `std::bytes` reads and w1
  unpacked with native shifts. 60s soak at ~500k records/s
  with an SSE client attached: zero overruns, zero zombie
  loci, all restarts tracked. A stalled SSE client is bounded
  by SO_SNDTIMEO (200ms) instead of stalling the pump.
- **RSS growth root-caused to `@form(vec).set`** (upstream
  runtime bug, repro in hand): each fallible `.set or discard`
  leaks ~33B into a region that survives child dissolve, and
  costs ~1µs (~1000x the inlined `.get`). Evidence:
  100M parent-form `.get`s from per-iteration children peak
  at 4.9MB flat; 2M `.set`s peak at 70MB. fuse-hl does ~1M
  sets/s on the event path → ~MB/s growth; idle fuse is flat.
  Repro: upstream-repro/repro3.hl (repro.hl/repro2.hl are the flat controls). Until the runtime
  fix lands, fuse-hl needs a periodic restart on long
  deployments.
- **`hale verify` advisory gate**: `hale check` is clean
  (0 errors) but `verify` trips 26 unbounded-allocation
  advisories that are false positives for this shape — the
  analysis can't see (a) const-bounded init loops
  (`while i < NET_SLOTS * WINDOW`), (b) that Cycle is a
  per-iteration child whose region reclaims at dissolve, or
  (c) that form growth (segs/topics/edges) is domain-bounded.
  Three concrete cases for sharpening the analysis; no
  suppression annotation exists yet.
- **Late attach loses structure.** An observer attaching after
  segment start never sees the original LOCUS_BIRTH events
  (rings overwrote them): the tree rebuilds only from new
  births. Protocol-level fix candidates: emitters re-emit
  structural state on observer_count 0->1, or the manifest
  carries a live-locus table. PROTOCOL §13 material.
