# Handoff 5: the last two — per-locus attribution + the silent pair

Context: v0.11.15 field-verified against the production fleet
and it's the milestone round — **19 seq-matched cross-process
edges, single-digit-µs means, lost=0, a one-send five-listener
multicast fan-out matched exactly per listener.** This round's
two blockers were iris-side (origin-blind matching, a silent
topic cap) and are fixed and pushed; `obs_fleet_contract.rs`
did its job — no upstream regressions. iris `observer` is now
merged to `main` (`105f2e8`).

Two items remain, both carried over rather than new:

## P17 — per-locus attribution never arrives in the field

With `LOTUS_OBS=1` + `LOTUS_OBS_WIRE=1`, fleet-wide, every
locus's per-locus pub/dlv reads 0 in iris — petals don't pulse.
This has been zero through every release since the attribution
amendment.

What iris does (consumer contract, PROTOCOL §8): reads
BUS_PUBLISH / BUS_DELIVER `w1 locus:20` (bits 44..63) and bumps
a per-(segment, locus) counter where `locus` must equal the
**LOCUS_BIRTH instance id** of the publishing/consuming locus in
that same segment. So the field zero means one of: (a) those
records still carry locus 0 on the fleet's flavors ("best-effort"
bottoming out), (b) the stamped id is from a different id space
than birth instance ids, or (c) BUS records for these flavors
don't fire at all (counters do — those are separate).

Ask: extend `obs_fleet_contract.rs` with the two publisher
shapes the fleet actually has, asserting nonzero w1 locus ==
the publisher's birth instance id:
1. a **keyed-topic** publish from a **dynamically-spawned child
   locus** (spawned via accept(), not a param default);
2. a **plain topic publish with zero local subscribers** (all
   consumers remote) — the pure-fanout path.
And on the deliver side: BUS_DELIVER stamped with the
**subscriber** locus for a keyed `where key ==` subscription.

## P18 — the silent pair

Two binaries (down from five after v0.11.13) register a segment
but emit zero records: no births, and the observer-attach
birth replay also produces nothing. Their common shape: a main
locus whose run() is a single long read loop with pinned
reader children and few/no cooperative children — the quiet
end of the lifecycle spectrum. Everything chatty births fine,
so this is the last uncovered lifecycle path (or these
processes genuinely park before any probe — worth
distinguishing: even so, the replay on observer_count 0→1
should re-emit their existing loci and doesn't).

## Housekeeping (no action)

- peek renders NET origin as `unknown:<origin>` (it decodes the
  w1 low bits as a binding id) — iris-side cosmetic, ours.
- Running scoreboard: 21 findings filed across 5 handoffs;
  17 resolved (12 upstream, 5 iris), 2 open above, 2 retired.

Acceptance for both: fleet up under the overlay →
`curl :8787/snapshot` → some locus in a gateway shows
pub > 0 (P17), and the two quiet binaries show their loci
(P18). Same ten-minute loop as always; ping and I'll run it.

---

# Dispositions (hale, 2026-07-28 — PR #283)

Milestone acknowledged — 19 edges / lost=0 / exact per-listener
multicast is the round we've been building toward since handoff 1.

## P17 — your hypothesis (c) was right, plus the asked-for tests.

The three publisher shapes you asked for are now in
`obs_fleet_contract.rs` (`attribution_on_fleet_publisher_shapes`):
keyed publish from an accept()-spawned child, plain publish with
zero local subscribers (remote-bound), and keyed `where key ==`
BUS_DELIVER — each asserting nonzero `w1 locus` == a LOCUS_BIRTH
instance id in the same segment, deliver distinct from publish.
All three PASS on the released code — those flavors attribute.

The field zero was (c): the **fully-devirtualized direct
dispatch** (single quiet same-thread subscriber — codegen bakes
the handler into the publish loop) emitted NO probes at all: its
subjects never registered a topic, never counted, never produced
BUS records. Any gateway path on that flavor was structurally
invisible — not just petals; its per-topic counters too. Both
direct flavors (baked IR loop + the C multi-handler helper) now
publish once + deliver per matched target with full attribution,
pinned by `direct_devirt_flavor_emits_attributed_probes`.
Expectation for your acceptance loop: loci publishing on
previously-invisible subjects will now show pub > 0 AND those
subjects will appear in manifests where they were absent — if a
gateway's topic list grows after the upgrade, that's this fix
working, not a new leak.

(Perf note, since the flavor exists for speed: the probe gate is
checked once per function entry and hoisted by LLVM — the
dormant publish loop is instruction-identical to the probe-free
lowering, and the bus_dispatch microbench came out FASTER than
the pre-observation baseline.)

## P18 — found: the replay was probe-driven.

The 0→1 birth replay ran inside `obs_gate()`, which only executes
FROM probes. Your two quiet binaries (long read-loop main + pinned
raw-fd readers) never fired a probe after your observer attached —
so the transition was never noticed and the replay never ran.
"These processes genuinely park before any probe" and "the replay
should re-emit and doesn't" were the same fact.

Fix: under `LOTUS_OBS=1` a detached heartbeat thread drives the
gate check every 250ms under the obs lock (teardown takes the same
lock around the unmap — no shutdown race). Replay latency after
attach is bounded at ~250ms with zero probe traffic. Pinned by
`quiet_process_replays_births_via_heartbeat` (attach AFTER birth,
no subsequent probes, births must appear).

One consumer-visible detail: the heartbeat claims one SPSC ring
slot for its replay emissions (TLS ring assignment), so a
`rings=N` segment has at most N-1 rings for app threads — the
default 8 accommodates it; a fleet pinning `LOTUS_OBS_RINGS` to
exactly its thread count should add one.

## Acceptance

Ping when the overlay is up — same ten-minute loop. Expected:
gateway loci show pub > 0 (P17), the two quiet binaries show
their loci within ~250ms of attach (P18), and previously-missing
subjects appear in manifests (the direct-flavor registration side
effect).
