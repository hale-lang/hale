# Handoff 4: v0.11.14 field test — two emission regressions + a wire-compat landmine

Clean-slate acceptance on the uniform fleet (fresh registrations,
fresh segments, no generation mixing — yesterday's stale-reg
pollution eliminated first). The transport-branch NET_SEND now
fires on the fleet's path (449 `net>` in the gateway segment),
but three new findings, two of them regressions introduced by
the v0.11.14 refactors. Still: edges 0, attribution 0.

Evidence: `/tmp/pk-mk4.txt`, snapshot via `curl :8787/snapshot`,
`/tmp/hale-obs/` live while the fleet runs.

## P14 (regression) — NET records lost their topic id

`lotus_obs_net_send`/`_deliver` emit `obs_emit(EK_NET_SEND, 0,
...)` — the record's **id field is hardcoded 0**. peek renders
`unknown:0`; P4-era records carried the topic id and rendered
the subject. PROTOCOL §8: for ekinds 3/4 the id field is the
TOPIC id — it's the join key the consumer uses to place the
event in a fused topic row. With id 0, iris cannot associate any
NET event with any topic, so seq matching has nothing to join
on: **edges are structurally impossible regardless of
(origin, seq) correctness.** The call sites have the binding id
for the counter line; they need to also resolve + pass the
topic id (the subject is in hand at both sites).

Also: the sender-side record's w1 origin bits still read 0 in
the field (`net> unknown:0 ... seq=N` — first column is w1's
low 16). The transport-branch commit stamps `obs_origin` at the
fanout; something between that stamp and the emitted word still
drops it. Same acceptance check will catch both.

## P15 (regression) — the publish side vanished

Fleet-wide: **every topic's CT_PUBLISHED counter is 0** and no
BUS_PUBLISH records exist (peek: 0 `pub` events in a gateway
segment that logged thousands of publishes; consumers' dlv
counters flow normally — md.book.signal read pub=0 / dlv=2660).
v0.11.13-era runs had pub counters correct (265k+).

Suspect: the P13 consume-once publisher-TLS split — "BUS_PUBLISH
only for a genuine local publish" — now gates BOTH the record
AND the published counter behind a TLS that the fleet's publish
lowering never sets (`lotus_obs_note_publisher` is in the
binaries but evidently not called on this dispatch flavor's
codegen path, or the dispatch consumes it before the emit).
Attribution is unfixable in the field until the flavor
inventory includes whichever lowering fathom's `<-` sites
compile to. Note: the counter must not be TLS-gated at all —
counters are the dormant-mode contract (P4: "enabled-but-
unobserved = counters only"); a publish should count even when
unattributable.

## P16 (landmine, decision needed) — the obs wire header breaks old receivers

Reproduced deliberately: a pre-0.11.14 binary subscribing a
subject receives ZERO messages from an observed new-runtime
sender — the 16-byte `[magic][origin|seq]` prefix makes its
deserialize fail on every datagram, silently (rebuilding the
same binary on HEAD immediately received 538 msgs). Old→new is
fine (headerless is peeled-through); **new observed →old is
total silent loss.**

Consequence: enabling LOTUS_OBS in any fleet with one stale
binary partitions it invisibly. Options, in rough order of
preference: (a) treat the header as a negotiated capability
(hard over multicast), (b) gate header emission behind an
explicit env (`LOTUS_OBS_WIRE=1`) so observation alone never
changes the wire, with edges documented as requiring it,
(c) accept + release-note loudly: LOTUS_OBS=1 requires the
whole fleet ≥ the header version. iris has no preference on
mechanism, but silent loss can't be the failure mode.

## Ask

The recurring cost here is that each fix is verified against a
unit harness that doesn't look like a fleet. Strong recommend
before the next round: an upstream integration test that is
shaped like the field — 3+ processes over a real UDP multicast
group, LOTUS_OBS=1, asserting the full consumer-visible
contract in one pass: nonzero pub AND dlv counters, NET records
with nonzero topic id + origin, cross-process (origin, seq)
pairs, BUS_PUBLISH w1 locus == publisher's birth instance.
That's exactly iris's acceptance check; encoding it upstream
ends the ping-pong.

---

# Dispositions (hale, 2026-07-28 — shipped in v0.11.15)

All three findings fixed and released as **v0.11.15** (PR #277 fixes,
PR #278 release; assets + image green). Fix commit: `61b40bd`.

## P14 — FIXED. NET records now carry the topic id.

`lotus_obs_net_send`/`_deliver` gained a leading `subject`
parameter; the probe resolves the topic slot and emits its id as
the record's w0 id field (PROTOCOL §8 join key). All four emit
sites (raw-udp + transport send; udp-reader + transport-reader
deliver) pass the binding's subject. The binding id still keys
the per-binding counter line only — record id and counter line
are now decoupled as the protocol intends.

On the origin-still-0 column: origin plumbing was verified
correct end-to-end (the loopback and the new multicast tests
both assert nonzero origin on every NET record). The field's
`unknown:0` rendering was the id=0 join failure, and note the
P16 disposition below — with v0.11.15, wire origin/seq also
requires the `LOTUS_OBS_WIRE=1` opt-in, so an un-opted fleet
will legitimately read origin 0 (headerless wire) while
counters and topic ids remain correct.

## P15 — FIXED, and the flavor inventory found the real culprit.

Two defects, one of which reframes the diagnosis:

1. **The keyed dispatch flavors had NO publish probe and NO
   deliver probe at all** (`lotus_bus_local_dispatch_keyed`,
   `lotus_bus_dispatch_wire_keyed` — all four delivery
   branches). A `keyed_by` topic — your routed feed shape —
   recorded zero publishes AND zero keyed-path deliveries in
   every version to date. Since genuine Hale publishes always
   carry `self` (free fns can't `<-`), this probe gap — not the
   TLS itself — is the primary cause of the fleet's pub=0. The
   v0.11.13-era "correct" 265k pub was inbound re-dispatch being
   wrongly counted as publishes; P13 correctly stopped that and
   thereby exposed the keyed gap. Both probes added.

2. **The counter is no longer attribution-gated, per your P4
   contract note.** The positive publisher-TLS gate is replaced
   with negative marking: the reader thread brackets its inbound
   re-dispatch (`lotus_obs_begin/end_redispatch`, consume-once)
   and the publish probe skips marked calls. Genuine publishes
   are the unmarked default — always counted (dormant-mode
   contract), emitted with best-effort locus attribution (0 only
   when genuinely unknown). Subscribers' published counters stay
   0 (asserted in the new test).

## P16 — FIXED with your option (b): `LOTUS_OBS_WIRE=1`.

`LOTUS_OBS=1` alone no longer touches the wire in any way — the
UDP magic header AND the framed-transport origin word are both
gated behind an explicit `LOTUS_OBS_WIRE=1`. An observed sender
with only `LOTUS_OBS=1` is byte-for-byte identical to an
unobserved one (regression-tested: a pristine pre-header
receiver receives everything from an observed sender).

**Action for iris: cross-process edges now require
`LOTUS_OBS_WIRE=1` set fleet-wide (all nodes ≥ v0.11.15) in
addition to `LOTUS_OBS=1`.** Without it you get counters, topic
ids, and local records, but NET records carry (0, local-seq) —
document it as the edge prerequisite, per your suggestion.
Spec (`runtime.md`) and the operations chapter carry the same
statement.

## Ask — DONE. The field-shaped test is in-tree and gating.

`crates/hale-codegen/tests/obs_fleet_contract.rs`: 3 processes
over a real UDP multicast group (`239.255.77.12`), LOTUS_OBS=1 +
LOTUS_OBS_WIRE=1, asserting in one pass: nonzero pub AND dlv
counters, NET records with nonzero topic id + origin,
cross-process (origin, seq) pairs, BUS_PUBLISH locus == a real
LOCUS_BIRTH instance, and subscriber published == 0. Plus a
keyed-probe test and the pristine-wire test. It reproduces P14
and P15 exactly and runs in CI on every PR — this is your
acceptance check, encoded upstream.

## Still open (unchanged from handoff-3)

- The ADAPTER branch (user transport loci) has no NET probe —
  its wire is Hale-owned, so the header belongs in the std
  transport layer, not C. Needs your fleet's binding config to
  confirm whether any binding routes through it.
- Non-framed transports carry no wire seq (falls back to
  (0, local)).

## Bench sanity

Full micro+app bench sweep on v0.11.15: zero regression
attributable to this release (two benches over the stale
v0.11.3 baseline band are byte-identical on a v0.11.13 binary —
pre-existing/env). The new probes are branch-only when dormant,
as before.
