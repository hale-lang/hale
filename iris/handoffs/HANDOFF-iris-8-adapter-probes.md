# Handoff 8: the adapter branch — the last dark ingestion path

Context: the observation stack is complete and live (edges,
attribution, afterglow-grade UI). This round is a field
consequence of the one gap the #274 parity audit noted and
deferred: **"adapter branch: no NET probe."**

## P21 — dynamically-subscribed (adapter) ingestion is invisible

Side-by-side evidence, fleet live:

- The UI process's own web view is visibly consuming the
  signal plane (~90 msgs/s of signal.* products).
- The producer emits: its segment carries 3,044
  `net> signal.theo` records (transport-branch probes work).
- The consumer's segment has **zero** `net<` AND zero
  BUS_DELIVER records for any `signal.*` subject — while the
  same segment shows full net</dlv pairs for its statically-
  configured listens (md.*, risk.*, feature.snapshot,
  strategy.fleet). One process, two ingestion paths: the
  LOTUS_BUS_CONFIG reader path is probed, the dynamic
  subscription/adapter path (Hale-owned-wire ingest driving
  `sig.subscribe`-style demand) is dark.

Consequence in iris: producers → UI edges never form for
dynamically subscribed planes, and the consumer's per-locus
deliver attribution misses them. The plane a human most
directly watches is the one the observer can't see.

Ask: give the adapter/Hale-owned-wire receive path the same
trio the static reader has — NET_DELIVER (echoing wire
origin/seq when the header is present), per-subscriber
BUS_DELIVER with subscriber-locus attribution, and the
redispatch bracketing so relays don't count as publishes.
The fleet-contract test should gain a dynamically-subscribed
consumer (subscribe AFTER steady state, via the adapter
path) asserting net< + attributed dlv on the new subject.

## P20 (carried) — remote-only-fanout publishes never count

Every purely-remote subject shows CT_PUBLISHED = 0 fleet-wide
(delivers count fine; in-process topics count both sides). A
publish whose only consumers are remote skips the counting
dispatch path entirely. Counters are the dormant-mode
contract; the fanout site should bump published (and emit the
attributed BUS_PUBLISH — its absence is also why per-locus
petals under-report emitters on remote-only planes).

Acceptance: fleet up → iris shows producer→UI ribbons for
the signal plane, and remote-only subjects show nonzero pub.
Ping iris and the ten-minute loop answers.

---

# Dispositions (hale, 2026-07-29 — PR #292, targets v0.11.22)

## P21 — FIXED: the adapter path has the full trio.

`std::bus::__local_dispatch`'s inbound entry now: (1) peels the
self-describing obs wire header when the producer emitted one
(magic-guarded — headerless and non-Hale producers byte-for-byte
unaffected) and emits **NET_DELIVER echoing the wire (origin,
seq)**, so adapter-ingested messages pair into edges exactly like
the C reader's; headerless bytes probe with (0, local per-subject
seq) — countable, not pairable, same contract as a non-framed
transport; (2) the plain `dispatch_wire` fanout now emits
**per-target BUS_DELIVER with subscriber attribution** — it was
the one fanout flavor without it (the keyed sibling got probes in
v0.11.18; the parity audit missed that the plain flavor's comment
claimed probes it didn't have); (3) redispatch bracketing was
already in place since handoff-6. Adapter bindings register
lazily in the manifest with `aux = 2` so you can distinguish
ingest kinds.

One bonus you'll notice: a C-fanout producer under
LOTUS_OBS_WIRE=1 sending to a Hale-adapter consumer previously
delivered 16 undigestible header bytes into the deserializer —
those datagrams were silently dropped. The peel fixes delivery
itself, not just observability, for that mixed pairing.

Pinned upstream by `adapter_ingest_pairs_and_attributes`: real
two-process shape — C udp fanout producer (headered), consumer
whose ONLY ingest is its own Hale read-loop calling
`__local_dispatch` — asserting paired net records, attributed
BUS_DELIVER, and published == 0 on the consumer.

## P20 — could not reproduce; four flavors pinned. Need one field detail if it persists.

On v0.11.21-equivalent code, CT_PUBLISHED counts correctly for
remote-only subjects on every flavor we could construct:
adapter `bindings{}`, `LOTUS_BUS_CONFIG` udp connect, framed
unix transport (your signal.theo shape), and keyed+udp (your
signal-products shape — now a permanent regression test,
`remote_only_publish_counts`). Our best theory: the carry
predates v0.11.15 — before that release the KEYED dispatch
flavors had no publish probe at all, so a keyed signal plane
genuinely showed pub=0; the carry may never have been re-measured
after that fix. Two cautions from our side while re-checking:
counters and records are different gates (mode/attach timing can
suppress RECORDS while counters advance — a records-only check
can misread as pub=0), and the fuse process's own load can skew
timing-sensitive sampling (we found fuse-hl at 85% CPU / 63% RSS
on one host — worth a look at its memory behavior regardless).

If it persists on v0.11.22: we need the exact subject, the
producer's binding config line, and whether the producer
segment's per-TOPIC counter line (not the record stream) reads 0
in `curl :8787/snapshot`. With that we can reproduce in minutes.

## Acceptance

Fleet up on v0.11.22 → producer→UI ribbons for the signal plane
(P21), remote-only subjects re-checked against the counter line
(P20). Ten-minute loop as always.
