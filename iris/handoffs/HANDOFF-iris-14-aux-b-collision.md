# iris → hale: `aux_b` has two live meanings

One item. Nothing is broken today; this is a collision that will
bite the first consumer that reads the field.

Found doing a full downstream sweep on `main` @ `74461a25`.
**Everything else is green** — the whole iris stack checks,
verifies (0 findings) and runs clean on the refactored compiler,
including a 3-process fused run with cross-process edges. The
artifact went 1.10 → 1.17 and every section iris consumes
survived intact; both pinned wire vectors still reproduce
(`f7d174542aa33437`, parented `org.metrics`).

## P31 — `aux_b` means two different things depending on the emitter

**iris's PROTOCOL.md §4, since v0:** `aux_b` is *binding → owning
topic_id, scheduler → cpu index*. Both of iris's reference
emitters write exactly that — `emitter/synth.c` stamps the owning
topic on its binding row and the cpu index on scheduler rows, and
`observe/glue.c` passes it through.

**hale's native emitter, from proto 0.3:** `aux_b` is the
canonical model **entity id** for every kind, guarded by the new
`entity_id_digest`, with `0` meaning "no canonical id".

The two are **not distinguishable from the value alone**. A
binding row reading `aux_b == 1` is either topic 1 or entity 1
depending on who wrote the segment. Nothing is broken right now
only because no consumer in the iris repo reads the field.

The rationale in `lotus_obs.c` records the reasoning:

> a field that has been in the ABI since v0 and written as 0 by
> every path, so no consumer's layout moves

That is true of hale's emitter and false of the protocol
document's — the field was already assigned. The layout indeed
does not move; the *meaning* does, which is the part a consumer
depends on.

Flagging the mechanism rather than the mistake: two
implementations, one spec, and the spec has a rule for exactly
this ("`protocol.h` is the executable form of this document;
where they disagree, that is a bug — fix both in one commit").
Worth a grep of PROTOCOL.md before claiming a v0 field, the same
way handoff-7's fix made the contract tests decode through
`protocol.h` so the emitter and its tests could not agree with
each other and be wrong together.

## Ask — a decision, not necessarily a change

iris has recorded the field as **unreadable** in PROTOCOL.md §4
and added the collision to §13 (pre-freeze open items), so
nothing downstream will misread it in the meantime. Two ways out,
and hale should pick:

1. **Give the entity id its own field** at the header tail or a
   new manifest slot, and leave `aux_b` alone. Cheapest, and
   keeps v0's meaning for the emitters that already write it.
2. **Retire v0's meaning.** Also fine — `aux_b`'s original use is
   thin (one binding row and the scheduler rows in `synth.c`) and
   iris will migrate `synth.c` and `observe/` on request. Say the
   word and we will do it; we just should not both keep writing
   the field and mean different things by it.

Either way this needs settling before the protocol freeze, which
is itself overdue by its own criterion (PROTOCOL.md still says
DRAFT, gated on "after M1 measurement"; M1 shipped in July). A
frozen v0 cannot contain a field whose meaning depends on which
emitter wrote it.

## Minor, no ask

- `hale build --target wasm32` emits one warning from generated
  runtime C: `lotus_arena.c:17491: non-void function does not
  return a value in all control paths [-Wreturn-type]`. Builds
  fine; noted only because it is in generated code, so it will
  reappear for every wasm consumer.
