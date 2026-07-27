# Handoff 3: NET seq semantics — the last blocker for edges

v0.11.13 (`aefe8cd`) verified against the uniform-rebuilt
fleet: **P6 canonicalization holds** (zero duplicate topic
rows), **P8/P9 hold** (425 loci, 392 parented, silent binaries
5 → 2), **P5 fires** (`net>` now emitted on the multicast
fanout). Three items remain; the first is the only edge
blocker.

## P11 — NET_DELIVER must echo the WIRE seq, not a local count

Field evidence (`/tmp/pk-mk.txt`, `/tmp/pk-pv.txt`): a gateway's
`net>` on a subject reads `seq≈341` (its own per-binding send
counter) while the consumer's `net<` for the same subject reads
`seq≈740` — the receiver stamps its **local receive counter**,
which sums across ALL senders on the subject (two gateways:
341 + ~400 ≈ 741, exact). Sender seq never equals receiver seq,
so the iris seq matcher pairs nothing: edges stay zero even
with both probes firing.

Local counters only coincide for loss-free unicast. The fix
needs the transport wire to carry the sender's identity + seq,
echoed by the reader:

- Wire: sender stamps `(origin_id:16, seq:48)` per message
  (origin = sender's binding/stream id; a per-subject counter
  at the SENDER).
- NET_SEND w1 = that pair. NET_DELIVER w1 = the pair READ FROM
  THE WIRE, verbatim.
- PROTOCOL.md §8 is being amended on the iris side to state
  this explicitly (it previously said only
  `binding_id:16 | seq:48` without saying whose counter).

This also fixes loss visibility: a receiver-local count can't
show gaps; the wire seq's gaps ARE the loss.

## P12 — binding ids still 0

`aefe8cd` says binding ids start at 1, but every NET record in
the fleet still resolves `unknown:0`. With multiple senders per
subject, binding 0 also collapses all senders into one seq
space at the consumer even after P11 — origin_id must be real.

## P13 — BUS locus attribution still zero in the field

`obs_emission.rs`'s ring-walk asserts attribution, but across
the uniform v0.11.13 fleet every locus shows pub=0/dlv=0 in
iris (w1 locus bits zero, or stamped in an id space that
doesn't match LOCUS_BIRTH instance ids — iris bumps by exact
(segment, instance) match). Worth asserting in the test that a
published record's w1 locus equals the PUBLISHING locus's
birth instance id specifically, in a program with several
loci, publishing from a non-first locus.

---

Acceptance stays the same one-liner: fleet up under the
overlay, two ctl-drives, `curl :8787/snapshot` → edges
non-empty with µs latencies, per-locus pub/dlv nonzero.
