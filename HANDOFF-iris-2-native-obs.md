# Handoff 2: native observation emission (P4) — field report from a production fleet

P4 (`455563b`, v0.11.12) was deployed against a real ~16-binary
multicast-bus fleet under `LOTUS_OBS=1` with the iris fuse
attached. **It fundamentally works** — every rebuilt binary
registered, birthed typed loci, and bus counters flowed with
correct multicast fan-out accounting (one subject showed pub=8
dlv=16 across two listeners — exactly right). The findings
below are ordered by impact on iris's marquee feature
(seq-matched cross-process edges), which currently renders
zero edges.

Live evidence on this machine: segments under `/tmp/hale-obs/`
(fleet still running), raw dumps `/tmp/peek-dash.txt`,
`/tmp/peek-rg.txt`, `/tmp/peek-sp.txt` (from
`iris-observer/emitter/peek`, run with `env -u XDG_RUNTIME_DIR`).
Keep fleet-specific subject/binary names out of any committed
test cases — synthesize neutral fixtures.

## P5 — NET_SEND never fires (the zero-edges cause)

Every segment dumped shows `net<` (NET_DELIVER, with monotonic
seqs, at the transport reader) and **zero `net>`** (NET_SEND).
A binary that publishes heavily over the UDP transport emits
pub records for each message but no send-side NET record. With
no sends, iris's seq matcher has nothing to pair — delivers
park forever, 0 edges.

Suspect: the fanout probe is on a transport path this fleet
doesn't take (or is ordered after an early-continue in the
fanout loop). The deliver-side probe placement is correct —
mirror it.

## P6 — shape_hash splits topics that share a subject

Two binaries declaring the same subject via *different local
topic type names* (producer declares its own `topic X {
subject: "s" }`, consumer declares `topic Y { subject: "s" }`)
get different shape_hashes. iris joins topics on
(shape_hash, subject) per PROTOCOL §5, so one subject splits
into two rows — observed live: a high-rate subject showed its
~266k publishes on one row (dlv 0) and its consumers' ~20k
delivers on another. Subjects declared once in a shared lib
fuse correctly.

This is PROTOCOL §13's open canonicalization item, now with a
concrete rule proposal: **hash the subject string + canonical
payload structure (field names/types in declaration order),
never the declaring type's name.** iris will adopt whatever
rule lands; today's split also breaks edge matching for those
subjects independently of P5.

## P7 — timestamp reconstruction ≈ 2^64 on some segments

One segment's reconstructed event timestamps read
`18446625687.x` seconds (u64 wrap territory: 2^64 ns ≈
18446744073 s) while a sibling segment reconstructs sane
values (`390.x` s) under the same consumer. Suspect the EPOCH
base or ts_delta encoding on one emission path (cross-thread
wire flavor?) — a negative delta stored into the u31 ts_delta
field would do this. Latency math survives only because both
sides of a pair usually share the broken base.

## P8 — five binaries register but emit nothing

Segments exist (registration + manifest), but zero records —
no births, and the observer-attach birth replay also produces
nothing (fuse attached long after start; replay contract says
tree reconstruction). The silent five in this fleet are the
ones whose work is a single pinned/main read loop. Possibly
the main-locus / pinned-reader lifecycle path skips both the
birth probe and the replay registry. The chatty binaries (many
cooperative children) all birth fine.

## P9 — births carry no parentage

Every birth record says `parent=(root)` — 22 loci in one
binary, all flat. LOCUS_BIRTH w1 packs parent:32|type:20;
the instantiation-site probe apparently doesn't thread the
spawning locus. Without it iris renders a flat fan instead of
the supervision tree (the flower's whole layout is the tree).

## P10 — BUS records: confirm locus attribution

PROTOCOL §8 (2026-07-27 amendment): BUS_PUBLISH/BUS_DELIVER
`w1 = locus:20 | seq:44`, 0 = unattributed. Petals in the live
fleet don't pulse, which suggests w1's locus bits are 0 —
stamp the current locus at dispatch (the runtime knows it;
that's the point of native emission). iris already renders it
(per-locus pub/dlv -> perimeter pulses) the moment it's
nonzero.

---

Verification loop once fixed: rebuild fleet binaries, restart
under the observe overlay, then `curl :8787/snapshot` — edges
non-empty with plausible µs latencies, one row per subject,
loci parented, petals pulsing. The iris side needs no changes
for P5/P7/P9/P10; P6 may need the fuse join updated to the
final canonicalization rule.
