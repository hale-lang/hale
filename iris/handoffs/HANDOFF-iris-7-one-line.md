# Handoff 7: attribution works — it's packed into the wrong bits

One-line fix. `lotus_obs_bus_publish` (and `_deliver`) emit:

    ((uint64_t)locus & 0xFFFFFu) | ((seq & 0xFFFFFFFFFFFULL) << 20)

i.e. **locus in bits 0..19, seq shifted to 20+**. PROTOCOL §8
and its executable reference (`iris` `emitter/protocol.h`,
`obs_bus_w1`) define the amendment as **locus:20 in the HIGH
bits**:

    ((locus & 0xFFFFF) << 44) | (seq & 0xFFFFFFFFFFF)

Every consumer (fuse, peek, the synth reference emitter)
decodes `w1 >> 44` → reads the top of a small seq → 0. Proven
with a 25-line CLI-built repro + gdb + a bit-dump: the note
fires, the flag is live (the handoff-6 constructor fix works),
the instance table resolves the publisher (`w1 & 0xFFFFF == 3`
== the publisher's birth instance id, exactly). Everything
upstream since handoff-3 has been correct except the shift.

Fix: pack per protocol.h in both probes. And the reason the
contract test stayed green: it decodes with the emitter's own
layout — have `obs_fleet_contract.rs` decode via protocol.h's
`obs_bus_locus`/`obs_bus_seq` (vendor the two shift lines with
a comment pointing at PROTOCOL §8) so emitter and consumers
can never disagree silently again. PROTOCOL.md's own rule:
"protocol.h is the executable form of this document; where
they disagree, that is a bug — fix both in one commit."

Acceptance unchanged; this is the last item on the board.

---

# Dispositions (hale, 2026-07-29 — PR #289, targets v0.11.21)

Fixed exactly as specified: both probes pack
`(locus & 0xFFFFF) << 44 | (seq & 0xFFFFFFFFFFF)` per PROTOCOL §8
/ protocol.h's `obs_bus_w1`. Your bit-dump diagnosis was complete
— nothing to add to the mechanism, and yes: everything since
handoff-3 computed the right locus and shipped it in the wrong
bits.

The loophole is closed the way you prescribed: the contract tests
now vendor protocol.h's decode (`obs_bus_locus(w1) = (w1 >> 44) &
0xFFFFF`, with the comment pointing at PROTOCOL §8 and the
fix-both-in-one-commit rule) instead of decoding with the
emitter's own layout. All 13 obs tests pass against the corrected
packing — meaning they now genuinely test the consumer contract,
not the emitter's self-consistency. This class of bug (emitter and
its tests sharing a wrong assumption) can't recur silently for
this record kind; if you amend protocol.h's BUS layout again,
grep for `obs_bus_locus` upstream and change both in one commit.

Acceptance: same loop — petals should pulse on v0.11.21 with no
iris-side change. Last item on the board cleared; we're watching
for the milestone snapshot.
