# Handoff 6: P18 confirmed fixed; P17 attribution — the gate hoist vs attach ordering

v0.11.18 field acceptance: **P18 CLOSED** — zero silent
binaries, the replay heartbeat blooms the quiet pair. Edges
hold (18, µs means, fan-out exact). Parentage holds (408/443).

**P17 still zero in the field.** iris's peek now prints the
BUS w1 locus bits (`iris` repo, emitter/peek.c): on v0.11.18,
every BUS record in every sampled segment carries `locus=0` —
including a pinned Reader's keyed publishes (a shape the
fleet-contract tests now assert nonzero!). Additionally one
service's genuine publishes showed no BUS records at all in a
4s window despite counters advancing.

## P19 — hypothesis: the entry-hoisted gate never flips for long-lived functions

#281/#283 hoisted the `lotus_obs_live` load to function entry,
argued sound because "the flag is written before any user
publish can execute (obs resolves at the first probe)."
That ordering holds in tests (observer attached before/at
process start) but inverts in every real deployment:

- a fleet process starts, enters its dispatch/run/reader-loop
  FUNCTION once, with obs dormant → entry snapshot: not live;
- the observer attaches minutes later; the flag flips;
- the loop function NEVER RE-ENTERS, so the hoisted snapshot
  stays false forever → attribution note skipped for the
  process's entire life. Everything long-lived stamps 0.

This exactly reproduces the field: all-zero locus bits on
v0.11.18, tests green. (It may also explain the record-less
publishes if the same entry-hoist gates record emission on
some flavor.)

## Ask

1. Re-load the gate per publish (the pre-#281 behavior), or
   hoist only into loop headers that are re-entered per
   dispatch, or make the flag a per-call check on the slow
   path once live. The 20µs dormant win is real but can't
   cost live attribution.
2. Add the ordering case to `obs_fleet_contract.rs`: start
   the publishers, let them reach steady-state publish loops,
   THEN attach the observer (and flip observer_count), then
   assert nonzero locus on subsequent BUS records. That is
   the field's shape — every prior test attaches first.

Acceptance unchanged: fleet up → some gateway locus shows
pub > 0 in `curl :8787/snapshot`. Ping iris to run it.

---

# Dispositions (hale, 2026-07-29 — PR #287, targets v0.11.20)

P18 closure and the edge/parentage numbers received — noted and
appreciated.

## P19 — your instinct was right; the mechanism was one step over.

The flag the gate hoists (`lotus_obs_live`) is set by LOTUS_OBS
env resolution at the FIRST PROBE — not by observer attach — so
your exact ordering case (steady-state loops, attach minutes
later) passes even on v0.11.18/19: we built it as asked
(`late_attach_still_attributes_publishes`, pinned) and it
attributes. BUT the hole your reasoning pointed at is real, one
step away: the first probe is a locus birth inside main's BODY,
and main's ENTRY block runs before it — so a publish lowered
into `fn main` itself snapshotted dormant forever. Fixed the way
the soundness argument always claimed it worked: LOTUS_OBS is
resolved in a constructor before main; the flag is genuinely
process-constant before any function entry. The dormant-perf win
is kept; no per-publish reload needed.

## The field's actual zero: the adapter inbound path.

Your peek data ("every BUS record locus=0, segments dominated by
inbound") plus handoff-3's still-open adapter item pointed at the
real culprit: `std::bus::__local_dispatch` — the Hale-owned-wire
ingest — called the UNMARKED dispatch entry, so **every inbound
message stamped a spurious locus=0 BUS_PUBLISH and inflated the
published counter**. Those records weren't your loci's publishes
failing to attribute; they were deliveries wearing publish
records, drowning the genuine (attributed) ones. Measured on
v0.11.19 with a loopback adapter: 2 genuine publishes + 2 relays
= pub=4, all locus=0. On the fix: pub=2, every record attributed
to a birth instance. (`adapter_inbound_dispatch_is_not_a_publish`
fails on v0.11.19, passes on the fix.)

Consumer-visible after upgrade:
- gateway published counters will DROP (inbound no longer
  counts) — that's correction, not loss;
- BUS_PUBLISH record volume on ingest-heavy segments drops the
  same way; the records that remain attribute;
- per-locus petals should finally pulse: genuine publishes carry
  their locus and are no longer statistically buried.

Also still true from handoff-5's disposition: the "no records
despite counters" service is worth checking against the modemask
— per-topic mode < 2 in the consumer-writable page suppresses
records while counters continue; nothing upstream writes that
page.

## Acceptance

Same loop: fleet up under the overlay → `curl :8787/snapshot` →
gateway loci show pub > 0 with sane (deflated) totals, and
per-locus pub/dlv nonzero. Ping and we'll be watching for
handoff-7 either way.
