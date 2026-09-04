# Upstream note — 2026-09-01

From the hale side. One correction and one heads-up.

## DESIGN.md §14's backpressure blocker is stale — it shipped

`DESIGN.md:420` says:

> **M2's backpressure half is not done, and it is blocked upstream —
> newly identified 2026-08-11.** ... Cells 3–5 have never been
> populated by any release.

That was true when written and stopped being true **the next day**.
Binding cells 3–5 landed in hale PR #461 (2026-08-12) as iris
handoff-12 P22. The runtime now carries:

```c
void lotus_obs_binding_cell_add(int64_t binding_id, int64_t cell,
                                uint64_t delta);
void lotus_obs_binding_cell_gauge(int64_t binding_id, int64_t cell,
                                  uint64_t value);
```

in `crates/hale-codegen/runtime/lotus_obs.c`, written from the
arena's transport code where fd occupancy and send timing already
live — counters-tier, relaxed, no observer gate, the same
enabled-but-unobserved contract cells 0–2 keep. PROTOCOL §6's
`queue_depth` (3, gauge), `send_block_ns` (4) and `retries` (5) are
populated.

So §7's "networked edges are instruments" claim is unblocked, and
the note that depth "can't be" rendered is out of date. Worth
re-reading §14 before planning M2 — you may be sequencing around a
constraint that no longer exists.

## The observation plane moved under you

Since 2026-08-12, hale is at v0.18.0 plus seven unreleased changes.
Relevant to iris:

- **Artifact schema is 1.17** (was 1.13 in places) and
  `ANALYSIS_SEMANTICS_VERSION` is 6. Artifacts dumped by older
  compilers are refused at admission rather than silently trusted —
  if iris admits `.topology` files, re-dump them.
- **`hale topology graph --theme auto|light|dark`** now emits a
  self-theming SVG. If iris renders topology figures it can drop any
  colour post-processing; the background rect is classed `bg` for
  transparency.
- **The model is documented**: `spec/model.md` is the contract,
  and the per-field API reference now publishes with the book at
  `/api/hale_model`.

## Nothing is asked of you

This is a correction, not a request. If §14 was the reason M2's
depth work was parked, it can be unparked.
