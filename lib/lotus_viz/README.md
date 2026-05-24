# lotus_viz

iris's default `VizScene` — renders any Hale locus tower as
a beautiful animated lotus flower. Substrate-invariant: works
for Hale source projects (the codebase IS the model) and
for non-Hale domains (the agent writes Hale modeling the
domain into `<workspace>/.iris/models/`, the flower renders
the agent's model).

This is iris's cinematic centerpiece — see
[`VISION.md §4`](../../VISION.md) for the strategic frame and
[`PLAN.md §5 Phase 3`](../../PLAN.md) for the build phase.

## Files

| File | What it covers |
|---|---|
| `DESIGN.md` | Pinned design decisions (palette, layout, animations, parser strategy) |
| `model.hl` | Locus tree types: `Petal` (with `child_type_names` for F.29 resolution), `LocusKind`, `ProjectionClass`, `LocusTree` |
| `aesthetics.hl` | Color palette + animation timing + layout constants |
| `layout.hl` | Radial geometry math; ring `r(d) = R0 + R1 * d^1.3`; child-arc spread; thin wrappers over `std::math::sin/cos` |
| `animator.hl` | Per-petal transient state (Born / Dying / Spotlit / Pulsing) + ambient sway; swap-remove on expire |
| `parse.hl` | `LocusTreeProvider` interface + `EmptyProvider` test-fallback |
| `heron_adapter.hl` | `TreeSitterHaleParser` implementing `LocusTreeProvider` via `../heron`; walks heron's AST extracting Petals; F.29 parent-edge resolution; per-file petal removal for re-parse idempotency |
| `scene.hl` | `LotusFlowerScene` — the `VizScene` impl that ties everything together |
| `examples/petal_extract.hl` | End-to-end pipeline demo: read .hl file → parse via heron → extract Petals → resolve F.29 edges → lay out → print |

## Consumer

iris's `viz_pane.hl` instantiates `LotusFlowerScene` as the
default `active_scene`. Switching to a different scene
(domain plugin) is via `viz.set_scene` proposal — see
`AGENT_PROTOCOL.md`.

## Status

Phase 3 structural work complete:

- All seven `.hl` files typecheck cleanly via `hale check`
- heron-backed parsing pipeline works end-to-end at the data-
  flow level (parse → walk → emit petals → resolve F.29 → lay
  out) — exercised in `examples/petal_extract.hl`
- Scene channels (`spotlight` / `pulse` / `draw_edge` /
  `narrate`) wired through to the Animator; cinematic
  surface ready for the agent

Pending for visible rendering:

- iris build needs F.10 closed (compiler-side; see
  `../../FRICTION.md`)
- Once iris compiles + libtree-sitter is installed on the
  build host, the flower's render() draws spheres-per-petal
  with the lifecycle animations
- v0.1 polish adds proper petal silhouettes (ellipses with
  pointed tips), richer edge rendering, multi-file workspace
  parsing
