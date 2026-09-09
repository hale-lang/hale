# Upstream response to handoff 12 (2026-08-12)

All five, same day. In your table's order:

## P25 — supervision is in the model (schema 1.10)

You get all three tiers of the ask, not just the minimum. One
hashed `supervision` row per `on_failure` handler:

```json
"supervision": [
  {"locus": "App", "child": "Worker", "err": "ClosureViolation",
   "ops": ["restart"], "retry_bound": 3}
]
```

- `ops` is what the handler body actually invokes (restart /
  restart_in_place / quarantine / reorganize / bubble), walked
  through branches.
- `retry_bound` appears when a literal is written
  (`restart(c) for 3`); a dynamic bound is simply absent.
- Spans ride in `provenance.supervision` rows
  ({locus, child, source, span}) — unhashed like all provenance,
  so your acceptance triple (supervising locus, child type, source
  span) is exactly the row shape.
- HASHED, deliberately: a supervision policy change is a topology
  change and moves `shape_hash`. Pinned by test, including the
  hash movement.

"Declared retry cap 3, observed 3 in 40 s" is now drawable.

## P22 — cells 3–5 are written (your priority order, all three)

- `queue_depth` (3): last-write-wins gauge of the KERNEL send-queue
  occupancy (`SIOCOUTQ`), sampled at send time. This is the honest
  per-binding depth — sends are synchronous, so the kernel queue is
  where a slow consumer backs up first. 0 on platforms without the
  ioctl.
- `send_block_ns` (4): accumulated transport-send call duration,
  success and failure paths both. Healthy baseline is syscall cost;
  a stalled consumer makes it explode — the leading-signal shape
  you asked for.
- `retries` (5): reconnects, monotonic (the transport's
  connect-side re-establish — the same event the RESTART record
  marks).

Counters-tier as requested — but note the measurement itself
(two clock reads + one ioctl per remote send) is gated on
`LOTUS_OBS` being enabled, which is the counters contract's
boundary anyway (enabled-but-unobserved = counters maintained).
Acceptance pinned: `overloaded_consumer_shows_depth_and_block_time`
— a consumer that accepts and never reads shows depth climbing and
block time accruing with zero loss.

## P24 — every `sorts.topics` name has a `provenance.decls` row

Span = the topic's name ident. Pinned:
`topic_declarations_carry_provenance_spans`.

## P26 — `model_hash` u64 at header 0x80, proto 0.2

Stamped at segment creation from a value the CLI computes from the
SAME bundle it typechecks (extracted from `dump_topology`, so the
two cannot drift) and codegen bakes into the prelude. Your
acceptance is the test verbatim: comment-only rebuild keeps the
value, adding a locus moves it. Harness builds (direct
`build_executable` callers) read 0 = unstamped. PROTOCOL §3
amendment is yours as offered; header_len covers the new field.

## P20 — closed as stale, with your conjunction as a permanent pin

We built the exact shape you specified rather than asking you to
re-raise a fleet: an `accept()`-spawned publisher (two spawned
workers under an accepting coordinator), a remote-only PLAIN topic
(udp connect binding, no local subscriber), observer attached
mid-steady-state. CT_PUBLISHED = 40/40, five runs out of five, on
current HEAD (`accept_spawned_remote_only_plain_publishes_count`,
now permanent beside the four earlier flavors). Combined with your
own read — the evidence predates the v0.11.15 keyed-probe fix and
several plausibly-relevant shipments — we're taking your offered
answer: closed. If the fleet ever shows it again on a ≥ today
build, the pin gives us an A/B in minutes.

## Scoreboard math

That clears the board: P20 closed-stale, P22/P24/P25/P26 shipped.
Nothing carried.
