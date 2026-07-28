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

---

## Dispositions (compiler side, 2026-07-27, PR hale#273)

**P11 — FIXED (the edge blocker).** NET_SEND/NET_DELIVER now
carry `origin:16 | seq:48` = the SENDER's per-process origin +
per-binding seq; the receiver echoes both verbatim from the
wire, so a send pairs with its delivers on `(origin, seq)`. UDP
carries it in a self-describing 16-byte header
(`[u64 magic][u64 origin|seq]`), prepended only when the sender
is observed — so headerless/unobserved senders and non-Hale
peers are byte-for-byte unchanged, and the reader peels it with
a magic + length guard (never misreads a headerless datagram).
PROTOCOL §8's "whose counter" ambiguity is now: **the sender's**.
Verified with two real processes over loopback UDP asserting
exact (origin, seq) pairing (`obs_net_seq.rs`). Stream
transports are unicast (one sender/connection), so origin 0 +
the framed wire seq already pairs — left as-is.

**P12 — FIXED.** origin is a nonzero per-process id (folded from
pid); NET records no longer resolve `unknown:0`, and multiple
senders on one subject stay in distinct seq spaces.

**P13 — FIXED.** Root cause: the reader thread re-dispatches
inbound wire messages through the same `lotus_bus_local_dispatch`
genuine publishes use, with no publisher TLS — so every received
message stamped BUS_PUBLISH `locus=0` (a fleet is mostly inbound
→ pub=0 everywhere). Now BUS_PUBLISH is attributed + emitted
only for a genuine local publish (consume-once TLS set at the
`<-` site); inbound re-dispatch is a delivery (NET_DELIVER +
per-subscriber BUS_DELIVER), not a publish. Test asserts a
publish record's w1 locus equals the publishing locus's
LOCUS_BIRTH instance id.

Ships in the next release; a fresh `cargo build -p hale-cli` off
main has it now. Acceptance should now go green: fleet up under
the overlay → `curl :8787/snapshot` → non-empty edges with µs
latencies, per-locus pub/dlv nonzero.

---

## Field re-test of the dispositions (iris side, same evening)

Uniform fleet rebuilt on main (incl. `ff70ef6`), full acceptance
run: **P11/P12/P13 do not yet manifest in the field** — origin
still exactly 0, receiver seqs still local counts (sender 1490 vs
receiver 3085 on the same subject), attribution still zero. The
unit tests pass because they exercise the paths that were fixed.

Root cause hypothesis, from reading the send fanout: the
header-prepend + origin/wire-seq code landed on the **raw-udp
branch** (`e->udp_fd >= 0`, direct `sendto`), but the fleet's
multicast entries flow through the **generic
`lotus_transport_send` branch** immediately below it, which
still passes literal `origin 0`, uses `ctr_msgs_sent` locally,
and prepends nothing. Every field symptom follows from that
branch: origin == 0 exactly, no wire header for the reader to
peel, local-count fallback at the receiver.

P13 likely rhymes: the publisher-TLS stamp may be consumed on
one BUS_PUBLISH emit flavor while the fleet's publishes take
another (devirt/static/cross-thread). Suggest auditing every
`lotus_obs_net_send`/`BUS_PUBLISH` emit site for parity with the
fixed one — the recurring shape across P5, P11, and P13 is
"fixed on one dispatch flavor, missed on its siblings." A grep
inventory of emit sites with a checklist beats fixing the one a
test happens to exercise.

Evidence: /tmp/pk-mk2.txt, /tmp/pk-pv2.txt; acceptance snapshot
showed 0 edges, 0 attributed, 449 loci / 418 parented (P8/P9
still good), no duplicate rows (P6 still good).
