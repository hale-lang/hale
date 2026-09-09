# Handoff 10: the reserved backpressure cells were never filled — plus P20, still open

Context: iris re-synced to v0.16.0+ this week after a gap
(iris had been sitting on the v0.11.24-era world). Three things
we confirmed on your side before asking for anything, because
two of them were ours to re-measure and one was a thank-you:

- **The `@form(vec).set` leak is gone.** This was iris's largest
  standing gap — fuse-hl needed periodic restarts on long
  deployments, and you flagged it independently in handoff-8's
  disposition ("fuse-hl at 85% CPU / 63% RSS on one host —
  worth a look at its memory behavior regardless"). Re-ran the
  same repro (`iris` `consumer/fuse-hl/upstream-repro/repro3.hl`)
  on current HEAD: **100M sets peak at 7.9 MB and run in 0.60 s**
  — flat, ~6 ns per get+set pair. Before: 2M sets → 70 MB at
  ~1 µs each. We assume this fell out of the #383/#402
  factory-locus reclaim work rather than a targeted fix; either
  way the restart requirement is retired and the repro stays
  in-tree as a regression probe.
- **P18's heartbeat replay and the unbounded-alloc analysis both
  hold.** `hale verify` on fuse-hl went from 26 advisories to 9
  (the const-bounded init loops are now proven bounded — thank
  you), and `@unbounded` gave us the acknowledgement that didn't
  exist when we last looked. fuse-hl now verifies clean at 0
  findings with exactly one annotated site.
- **The #399 topic-identity vectors re-verify.** Both pinned
  vectors in PROTOCOL §4 reproduce against v0.16.0+
  (`0xf7d174542aa33437`; parented `org.metrics`). The artifact's
  `topics` section is where you said it would be. We also took
  your schema-1.3 point: iris now owes an `artifact_digest`
  check before joining on those rows, since `shape_hash` doesn't
  cover them — recorded in PROTOCOL §4, ours to implement.

## P22 (new) — the per-binding backpressure counters have never been written

PROTOCOL §6 has reserved seven cells on a binding line since v0:

```
bindings:  sent, delivered, bytes, queue_depth (gauge),
           send_block_ns, retries, seq_high_water
```

The native emitter writes cells **0, 1, 2 only**
(`obs_count(MK_BINDING, id, 0|1|2, …)` in `lotus_obs.c`). Cells
3–5 — `queue_depth`, `send_block_ns`, `retries` — are written by
no release, on any path. Same on the topic line: 0/1/2 are live,
nothing else.

This isn't a regression; it's a gap that has been invisible
because **the demand side was missing in the same place**. iris
never read those cells either, so nine rounds of field testing
never surfaced it. We only found it costing out the next
milestone.

Why it matters more than a missing gauge: iris's M2 is "loss and
backpressure made visible," and those three cells are exactly
what separates a **lossy** edge from a **saturated** one. Loss we
can already render — the wire seq's gaps are the evidence, and
that half works beautifully. But a consumer falling behind shows
up first as depth climbing and send-block time accruing, *before*
anything drops. Without cells 3–5 the observer can only report
the drop after the fact, which is the failure mode M2 exists to
get ahead of. It is also the readout a supervisor's reaction is
legible against: "depth climbed, block time accrued, then the
supervisor absorbed it" is the story; right now we can only show
the last clause.

Ask, in rough priority order (all three are useful; the first
alone unblocks the milestone):

1. **`queue_depth` (cell 3)** — a gauge, last-write-wins, stored
   at enqueue/dequeue on the bus queue. The cheapest of the
   three and the one that carries the demo.
2. **`send_block_ns` (cell 4)** — accumulated nanoseconds
   blocked in the send path. Monotonic counter.
3. **`retries` (cell 5)** — monotonic count of transport-level
   retries.

`seq_high_water` (cell 6) we can derive consumer-side from the
NET seq stream, so no ask there unless it's free.

Cost note, since these sit on the send path: all three are
relaxed atomic stores/adds on a line already in cache from the
`sent`/`bytes` bumps two lines up — the same dormant-mode
contract as the existing counters (counters are the "enabled but
unobserved" tier, so they should count without `obs_gate()`,
exactly as cells 0–2 do today).

Acceptance: fleet up under the overlay, deliberately overload one
consumer → `curl :8787/snapshot` shows the binding's depth
climbing ahead of any loss. Ten-minute loop as always. Consumer
side is cheap once the cells are live — they're reads on a
counter line fuse-hl already fuses.

## P20 (carried, unanswered) — dynamically spawned publishers still count zero

Filed as handoff-9 on 2026-07-29 with the discriminating table;
no disposition came back, and we can't tell whether it was
triaged and rejected or simply lost in the v0.12→v0.16 run. No
judgment either way — that stretch shipped constitutions, fleet
composition, the secrets surface and a target model, which is a
lot of board. Restating it compactly so it's answerable:

| publisher locus | declaration | CT_PUBLISHED |
|---|---|---|
| gateway reader (keyed in-process topic) | static param default | counts (12.6k) |
| reader/coordinator loci publishing products | static param default | counts (~2k each) |
| per-symbol book loci → the md plane | **accept()-spawned child** | **0** (dlv 9.8k) |
| per-symbol feature loci → feature plane | **accept()-spawned child** | **0** (dlv 2.0k) |

Uniform across three unrelated apps: every statically-declared
publisher counts, every dynamically-spawned publisher counts
zero — while their messages demonstrably deliver (consumers' dlv
counters and cross-process edges are full). Not nested-handler
related: one static reader publishes from inside a delivery
handler and counts fine.

Your four-flavor repro in handoff-8's disposition didn't
reproduce it, and we think the missing discriminator is the
conjunction: the existing fleet-contract accept()-spawned case
asserts **attribution** on a keyed **local** topic; the shape
that fails in the field is a spawned child's publish being
**counted** on a **remote-only plain** topic, with the observer
attaching after steady state. If that combination passes on
current HEAD, we'll take the finding as stale and close it —
it predates several releases now and may have been fixed
incidentally, the way the `.set` leak was.

## Housekeeping (no action)

- iris-side fixes this round, for the record: consumer liveness
  now ANDs the header ALIVE flag with `kill(pid, 0)` — reading
  the flag alone rendered SIGKILLed emitters as live forever
  (we measured 14 phantom processes in one snapshot, all
  leftovers from *your* test suite's segments, which is also a
  small vote of confidence in the v0.11.24 sweep working as
  intended for anything that starts after it). And peek's
  `unknown:<origin>` cosmetic from handoff-5 is fixed: it was
  resolving the NET w1 origin against the local binding name
  table, which is a foreign id space. Both ours.
- Running scoreboard: 22 findings across 10 handoffs; 19
  resolved (13 upstream, 6 iris), 2 open above, 2 retired.
