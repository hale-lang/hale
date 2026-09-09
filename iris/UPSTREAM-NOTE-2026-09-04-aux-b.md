# hale → iris: `aux_b` resolved at proto 0.4 (handoff-14 P31)

Answering `HANDOFF-iris-14-aux-b-collision.md`. hale picked option 2:
**retire v0's meaning.** Tracked as GH hale#525 item 3 (Track 0 of
the DNA epic, hale#521); the hale side is PR
`fix(obs): aux_b carries the canonical entity id; retire the v0
binding/scheduler meaning, proto 0.4`.

Why this way and not a new field: the entity id is the #476 join
between a running segment and the canonical model, and it is what
every consumer that arrives next (the DNA experience view, the
model-diff perspective) will read. v0's use was thin — one binding
row and the scheduler rows in `synth.c`, a pass-through in
`observe/` — and no consumer in this repo read it. Moving the
thing that matters to make room for the thing nobody read would
have been backwards.

## What changed on the hale side

- `lotus_obs.c` writes `proto_minor = 4`. No layout change; the
  bump records the meaning change, because a consumer depends on
  the meaning, not the offset.
- The rationale in `lotus_obs.c` that claimed the field had "been
  written as 0 by every path" now says what actually happened,
  crediting handoff-14.
- `obs_entity_ids_unstamped.rs` pins `proto_minor == 4` in the
  header, so a future bump is a protocol decision, not a test fix.
- `spec/runtime.md` § *Native observation emission* records 0.4.

## What needs to change here

Consumer rule, stated once for PROTOCOL §4: use the ids only when
`entity_id_digest != 0` matches the model you hold. At
`proto_minor >= 4` a nonzero `aux_b` was never anything else. A
0.3 segment from `synth`/`observe` carries the old meaning and a
zero digest, so the digest gate alone is correct for it too.

Emitters: `synth.c` and `observe/glue.c` write 0 into `aux_b`
(no model, so no ids), the scheduler cpu index moves to `aux_a`
(u16, unused on scheduler rows until now), and the binding →
topic pairing is dropped. `observe/glue.c` keeps the `topic`
parameter on `obs_binding` for source compatibility and no longer
stores it. `protocol.h` goes to `OBS_PROTO_MINOR 4` with the
history line and the `aux_b` comment rewritten. PROTOCOL §4's
CONTESTED bullet becomes the resolution, and the §13 freeze item
is struck.

**Applied in the commit that carries this note** (2026-09-04),
after the inspector work was committed and the observer renderer
lineage was merged, so the tree it lands on is one lineage.

Nothing in `consumer/`, `fuse-hl`, or `inspect/` read `aux_b`, so
no consumer code moves. With this in, §13 has no open items that
block the v0 freeze on the hale side.

## Not asked, noted

`observe/glue.c` still stamps `proto_minor = 1` in its own header
(`hdr_t` predates `model_hash`/`entity_id_digest`). That was true
before this note and is unaffected by it; a consumer at minor ≥ 2
already ignores that segment's tail. Bringing the observe library
to 0.4 is its own change if anyone wants its segments to carry ids.
