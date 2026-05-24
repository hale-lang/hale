# lotus_viz — pinned design

This document pins the design decisions for the lotus.flower
scene. Implementations track this doc; deviations need a
reason captured here.

Strategic frame: VISION.md §1 + §4. Implementation roadmap:
PLAN.md §3.5 + §5 Phase 3.

---

## What we render

**Hale locus trees, as petals on a lotus flower.** The
source is `.hl` files in the workspace plus `.hl` files the
agent writes into `<workspace>/.iris/models/`. We don't
distinguish the two sources at render time — both become
loci in one tree.

Per file, we extract:

- **Locus declarations** (`locus Name { ... }`) — become
  petals.
- **Type declarations** (`type Name { ... }`) — become
  petals (different color family; types are loci-in-proto-form
  per the foundational axiom).
- **Interface declarations** (`interface Name { ... }`) —
  become petals (silver-blue, distinct visual identity).
- **Locus parameter fields whose type is another declared
  locus** (per F.29) — become parent → child edges in the
  tree. This is how we reconstruct nesting from flat source.
- **Projection class** (`: projection rich|chunked|recognition`)
  — drives the petal's color family.
- **Capacity slots** (`capacity { pool ... ; heap ... ; }`) —
  not rendered at v0; reserved for richer per-petal glyphs in
  polish phase.
- **Bus subscribes/publishes** — not rendered at v0; reserved
  for agent-driven `draw_edge` proposals in cinematic walks.

The tree is **flat-source-flattened**: a workspace's many
`.hl` files are merged into one logical tree per F.19 (seed
model). Top-level declarations are root petals; F.29
locus-typed fields produce child petals nested under their
parent.

---

## Color palette

Projection class drives the hue family. Each petal carries a
radial gradient from a **pale inner color** (toward the
petal's center) to a **saturated outer color** (toward the
petal's rim).

| Kind / projection | Pale inner | Saturated outer | Rationale |
|---|---|---|---|
| Plain locus (no projection) | `#F8F1E0` cream | `#F5C56E` warm gold | Default "this is a locus" hue; warm, neutral, not category-coded |
| Rich projection | `#FBE0DA` blush | `#E8806E` warm rose | Rich = "few children, observed in detail." Warm tones = intimacy. |
| Chunked projection | `#D7EBEB` pale teal | `#5BAEAE` cool teal | Chunked = "moderate population, chunk-level observation." Cool = breadth. |
| Recognition projection | `#DDEDDB` sage mist | `#8DBC8B` sage | Recognition = "population, aggregate observation." Green = mass, growth. |
| `type` (shape) | `#EDE5F5` pale lavender | `#C8B6E2` lavender | Types are inert (no flow). Cool / unsaturated. |
| `interface` (F.20) | `#DCE3EB` pale silver | `#A8B9CC` silver-blue | Interfaces are abstractions. Cool, metallic. |

**Background.** Window background is `#1a1a1a` (matches
iris's renderer.hl `BG`). Flower stage uses a subtle radial
dim from `#1a1a1a` at edges to `#222222` at center, giving
the petals depth without competing with their saturation.

**Selection / spotlight.** A `Spotlit` petal gets a +20%
saturation boost on its outer ring + an additive `#FFFFFF` at
8% glow extending ~0.3 petal-radii outward. Other petals
during a spotlight drop to 30% saturation (still visible,
clearly de-emphasized).

**Dim mode (resolution).** The focused petal renders at full
saturation; all others drop to 30% saturation + 50% opacity.

---

## Geometry — radial layout

Petals are placed on **concentric rings** centered at the
origin, one ring per depth in the locus tree.

- **Root petals** (top-level decls — no F.29 parent in scope)
  sit on ring 0. There may be many roots; ring 0 has positive
  radius too, just smaller than ring 1.
- **Children** of a petal at depth `d` sit on ring `d+1`,
  distributed angularly around their parent's angular
  position. If parent is at angle θ and has N children, they
  spread across an arc proportional to N (max ~120° per
  parent's child-arc to prevent overlap with siblings'
  children).
- **Ring radius** grows superlinearly: `r(d) = R₀ + R₁ * d^1.3`
  where `R₀ = 1.5` (ring 0 baseline) and `R₁ = 1.8`. The 1.3
  exponent gives outer rings more room without making the
  flower huge.
- **Petal size** scales inversely with ring index: outer
  petals are slightly smaller (more of them fit). Base size
  `0.5 * (1 / (1 + 0.15 * d))`.

The layout is **deterministic** given the locus tree —
same input produces same petal positions. This matters for
animation continuity (when a new locus is added, only its
petal animates in; existing petals stay put).

**3D positioning.** Petals lie on a slightly curved surface
— a shallow dome with the root at the top, children
radiating outward AND downward by a small amount. This gives
the camera a natural angle to orbit. Exact dome curvature:
`z = -0.15 * r` where `r` is the petal's horizontal distance
from origin.

---

## Animations

Four transient per-petal states, plus one ambient global
behavior:

| State | Trigger | Duration | Visual |
|---|---|---|---|
| `Born` | New petal added to tree | 600ms | Scale 0 → 1.0 with ease-out-cubic; alpha 0 → 1 |
| `Dying` | Petal removed from tree | 800ms | Scale 1.0 → 0.7 with ease-in; alpha 1 → 0; slight downward droop (z += 0.5) |
| `Spotlit` | Scene channel `viz.scene.spotlight(ids)` | Held until cleared | Glow + saturation boost (see palette) |
| `Pulsing` | Scene channel `viz.scene.pulse(id, color)` | 1.2s loop, configurable repeats | Color tint cycles between base and pulse color; subtle scale 1.0 → 1.05 → 1.0 |

**Ambient sway.** All petals continuously rotate ±2°
(rotation around their stem axis) with a 4s period, phase
offset per petal (using petal index as seed). This keeps the
flower feeling alive even in steady state. Sway is suppressed
when the user is actively orbiting the camera — otherwise the
two motions confuse the eye.

**Easing.** All eased animations use cubic curves. Specifically:
- ease-out-cubic: `t³ → 1 - (1-t)³`
- ease-in-cubic: `t³`
- ease-in-out-cubic: piecewise

---

## Interaction

### Camera (universal substrate per VizScene)

- WASD fly (front/left/back/right relative to camera)
- Mouse drag → orbit around scene origin
- Scroll wheel → zoom (camera moves along its view axis,
  clamped to `[1.5, 50.0]` from origin)

### Click (delegated to scene)

Pane-relative `(x, y)` → unproject to a 3D ray from camera
through cursor → hit-test against petal bounding spheres
(petal center, radius = `petal_size * 1.3`). On hit:

1. Camera focuses on the petal (smooth interpolation, 400ms
   ease-out)
2. Source pane navigates to the petal's source location via
   `source.goto` proposal

### Modes (universal substrate per VizScene)

- **bulk** (`1` key) — every petal at full detail with edges
  visible
- **harmonic** (`2` key) — petals dimmed except current
  projection-class group; pressing `2` again cycles through
  rich → chunked → recognition → plain → type → interface
  → back to all
- **resolution** (`3` key) — camera-framed petal at full
  opacity, others 30% saturation + 50% opacity

### Scene channels (agent-facing cinematic surface)

| Channel | Payload | Behavior |
|---|---|---|
| `ide.cmd.viz.scene.spotlight` | `{ids: [String]}` | Set the spotlight set; petals not in the set drop to 30% saturation. Empty list clears spotlight. |
| `ide.cmd.viz.scene.pulse` | `{id: String, color: String?, repeats: Int = 3}` | Pulse a petal N times. Color defaults to white. |
| `ide.cmd.viz.scene.draw_edge` | `{a: String, b: String, color: String?, fade_ms: Int = 3000}` | Draw an arced curve between petals a and b; fade after fade_ms. |
| `ide.cmd.viz.scene.narrate` | `{id: String, text: String, duration_ms: Int = 4000}` | Caption overlay anchored to the petal. |

---

## Parser strategy — shipped

**Status (2026-05-24):** the heron-backed pipeline is live.
`pond/heron` ships a tree-sitter grammar for Hale plus a
thin Hale-side `@ffi` wrapper exposing `Parser` / `Tree` /
`Node` / `Query`. lotus_viz vendors it at `../heron` and
instantiates a `TreeSitterHaleParser` (see
`heron_adapter.hl`) as the default `LocusTreeProvider`.

What `parse.hl` carries today:

```hale
interface LocusTreeProvider {
    fn parse_file(path: String, content: String);
    fn clear();
    fn tree() -> LocusTree;
}

locus EmptyProvider { ... }   // test-only fallback for
                              // headless tests that don't
                              // link libtree-sitter
```

The concrete impl `TreeSitterHaleParser` lives in
`heron_adapter.hl` (same seed). It walks heron's AST and
emits `Petal`s, resolves F.29 parent edges across the full
tree, and tracks per-file petal sets for re-parse
idempotency. Uses the M90-avoiding `parse_into_self`
pattern on heron's Parser.

**Coverage:** heron handles every construct iris exercises
today — `locus_decl`, `type_decl`, `interface_decl`,
`params_block` (with locus-typed field detection for F.29),
`capacity_block`, `closure_decl`, `bus_block`,
`form_annotation`. Polish-phase per-petal glyphs (capacity
slots, closures, bus topology) can land as scene-side
rendering whenever they're prioritized.

The historical pivot rationale (why heron over a hand-rolled
parser): a tree-sitter grammar unblocks lotus_viz cleanly AND
lights up editor syntax highlighting universally AND seeds
the future LSP path. Same effort budget, much higher leverage.

---

## Intersection with dev-tools work

Multiple sessions had flagged that Hale was missing key
dev tooling — LSP and a tree-sitter grammar. **The
tree-sitter grammar shipped as `pond/heron`** during this
work; lotus_viz uses it (see "Parser strategy" above). LSP
still deferred.

- **Tree-sitter grammar (shipped).** `pond/heron`. lotus_viz
  consumes via vendored `iris/lib/heron/`. Editors that
  support tree-sitter (Helix, Neovim, Zed, Emacs, VSCode via
  extension) get Hale syntax highlighting + symbol
  navigation by pointing at the heron repo. Helix
  integration ships in `pond/heron/integrations/helix/`.
  Materially improves the "iris is summoned, your external
  editor stays" workflow (VISION §5).

- **LSP server (deferred).** iris's MCP server is the
  plausible host when this revisits — `pane.read`,
  `viz.snapshot`, `viz.find` already expose locus-aware
  read tools; an LSP server could be an adapter that
  translates LSP protocol → iris MCP calls. One source of
  structural truth, two protocols on top. The tree-sitter
  grammar built here unlocks the path without committing
  to LSP-shaped infrastructure now.

---

## Open questions deferred

- **Petal silhouette.** v0 ships disks with radial gradient.
  Polish phase: actual elongated-ellipse petal shapes with
  pointed tips, possibly with subtle alpha-feathered edges.
  Costlier to render; defer until the rest works.
- **Edge rendering for bus topology.** Subscribe/publish
  relationships could become rendered edges between petals,
  but the density would be visually noisy. v0: agent-driven
  edges only (via `draw_edge` scene channel). v0.1+:
  consider a "bus mode" toggle that shows subscribe/publish
  edges, possibly only between currently-spotlit petals to
  control density.
- **Per-petal complexity glyphs.** Capacity slots, closure
  declarations, k_max could become small icons on the petal
  rim. Defer until the parser handles those constructs.
- **Mobile / very-small windows.** Layout assumes >800px in
  each dimension. Smaller window degraded behavior is
  unspecified.
- **Many-root workspaces.** A workspace with 50+ top-level
  loci has 50+ root-ring petals. Visual density problem.
  Possible solutions: automatic harmonic-mode grouping by
  file, or a "thin out unfocused branches" mode. Defer until
  a real workload surfaces the issue.
- **Source-of-truth conflicts.** If `.hl` file in workspace
  and an `.hl` file in `.iris/models/` both declare a locus
  named `Foo`, which wins? v0: last-parsed wins (alphabetical
  filename order, models/ parsed after workspace). v0.1:
  diagnostic surface.
