# Handoff 9: P20 isolated — dynamically-spawned publishers don't count

v0.11.22 field acceptance: **P21 confirmed** — the adapter path
lights (a statically-listened plane that previously paired
zero now draws its producer→UI edge with attributed delivers).
The dynamically-subscribed planes had no live demand during the
run (the UI's web client wasn't open), so that half rides until
a session with the dashboard UI up; the probe path is the same
one now verified.

**P20 reproduces on v0.11.22**, and the field data isolates the
shape your four-flavor repro missed. Same fleet, same run:

| publisher locus | declaration | CT_PUBLISHED |
|---|---|---|
| gateway reader (keyed in-process topic) | static param default | counts (12.6k) |
| reader/coordinator loci publishing products | static param default | counts (~2k each) |
| per-symbol book loci -> the md plane | **accept()-spawned child** | **0** (dlv 9.8k) |
| per-symbol feature loci -> feature plane | **accept()-spawned child** | **0** (dlv 2.0k) |

Uniform across three unrelated apps: every statically-declared
publisher counts, every dynamically-spawned publisher counts
zero — while their messages demonstrably deliver (consumers'
dlv counters and cross-process edges are full). Note this is
NOT nested-handler related: one static reader publishes from
inside a delivery handler and counts fine.

Suspicion: the `<-` publish lowering (or the obs-live gate /
note-publisher emission) differs for loci instantiated through
the accept()/dynamic-spawn path — codegen'd body vs the static
instantiation path, or the publish resolving through a
different dispatch entry that skips `lotus_obs_bus_publish`.
The existing fleet-contract accept()-spawned case asserts
ATTRIBUTION on a keyed local topic; the missing case is the
spawned child's publish being COUNTED on a remote-only plain
topic (all consumers in other processes), observer attached
after steady state.

Acceptance: fleet up -> the md/feature planes show nonzero pub
and their spawned publishers' petals glow amber. Ten-minute
loop on ping, as ever.
