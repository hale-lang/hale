# The Aperio Standard Library — Roadmap

The stdlib's v1.x build-out unfolds in five phases. Phases 1
through 5 are sealed as of m93. Phase 6 (the substrate plan
forced by the Aperio IDE) is the next arc.

For "what can I actually use today," see
[What you can build today](./ready-today.md).

## Phase 1 — Foundations (sealed, m71 → m76)

- `std::io::tcp` — multi-accept Listener + Stream send/recv.
  Substrate: `lotus_tcp_*` over AF_INET with internal length-prefix
  framing.
- `std::io::fs` — `read_file`, `write_file`, `file_exists`,
  `file_size`, plus `read_bytes` (m89) and `list_dir` (m90).
- `std::env` — `args_count`, `arg`, `var`, `var_exists`.
- `std::str` — `parse_int`, `can_parse_int`, `index_of` (m84).
- `std::time` — `sleep`, `monotonic` (under the `std::*` namespace).
- `std::process` — `pid`, `exit`.

Magic `std::*` path resolver in codegen; no module system.

## Phase 2 — Test framework (v0.1 sealed, m87 → m88)

- `std::test::assert(cond, msg)`
- `std::test::assert_eq_int(actual, expected, msg)`
- `std::test::assert_eq_str(actual, expected, msg)`

Test-runner contract: pass = exit 0 silent; fail = non-zero +
`"ASSERTION FAILED: …"` on stdout. Sufficient for self-tests
written in Aperio that drive real Aperio behavior.

**v1.0 still on the table:** `aperio test` CLI runner,
`assert_rejects` (compile-error tests), `assert_closure` (closure
introspection), `assert_neq_*` siblings, benchmarks. Property-based
testing explicitly deferred per spec.

## Phase 3 — HTTP server (sealed, m83 → m86)

- `std::http::Request` and `std::http::Response` — record types.
- `std::http::parse_request(raw) -> Request`
- `std::http::write_response(stream, response)`
- Multi-accept TCP Listener composes via `on_connection: fn(Stream)`.

**v1.0 still on the table:** header-map type, `Connection: keep-alive`,
streaming bodies > 8 KB, listener bind-readiness primitive,
content-type-by-extension dispatch.

## Phase 4 — Text processing (v0.1 sealed, m91)

- `std::text::md_to_html(md) -> String` — block-level markdown:
  ATX headings, paragraphs joined by single space, fenced code
  blocks, HTML escape (`&` / `<` / `>`).

**v1.0 still on the table:** inline formatting (`**bold**`,
`*italic*`, `` `code` ``, `[text](url)`).

## Phase 5 — Synthesis (sealed, m92)

`examples/docs-server/main.ap` — ~200 lines of Aperio composing
seven stdlib namespaces into a real HTTP server that lists and
renders markdown docs from a configured directory. Demonstrates
the stdlib end-to-end.

## Phase 6 — Substrate for the IDE (m94 onwards, planned)

Forcing function: an Aperio IDE that renders the locus tree of
running programs. Each milestone exposes a `std::*` namespace
useful to every Aperio program, not just the IDE.

| #     | Name                          | Surface                                                              |
| ----- | ----------------------------- | -------------------------------------------------------------------- |
| m94   | Filesystem watch              | `std::fs::watch::{create, next, close}`                              |
| m95   | Runtime debug instrumentation | Runtime publishes `lotus.debug.*` when `APERIO_DEBUG` env-enabled    |
| m96   | 3D graphics                   | `std::graphics::{scene, camera, mesh, transform, animate}`           |
| m97   | UI panels                     | `std::ui::{panel, label, button, text_input, ...}`                   |
| m98   | Embedded shell                | `std::shell::{pty_spawn, pty_read, pty_write, terminal_widget}`      |
| m99   | MCP server                    | `std::mcp::{serve_http, tool, register_tool}`                        |
| m100  | Compiler self-introspection   | `std::aperio::{parse, typecheck, ast_walk}`                          |

Detailed plan: `notes/aperio-ide-design.md` (in the repo).

## Future arc — Lotus harness for n-dim growth (exploratory)

> **Status:** Design exploration; not committed. The libraries
> below are sketches driven by two concrete app shapes — a
> market-data gateway (mdgw) and a price triangulator — both
> hourglass-shaped (saturated input → modes-bearing waist →
> multi-channel fanout).

The driving observation: a class of applications takes the shape
of an hourglass / double-pyramid prism — collect multidimensional
state, project through a normalizing waist, fan back out via
multiple channels. The deeper primitive both apps want is the
**harness** that lets a lotus extend into any growth shape (stem,
branch, fan, mesh, hourglass) without each shape needing bespoke
wiring. The harness compiles down to existing substrate —
`projection` + `schedule` annotations + bus-subject naming +
F.9 closure-test wiring — making axis-shape the unit of decision
rather than per-locus-hand-annotated.

### Dimensional staging

Per The Design's dimension-composition rule, libraries fall out
per dimension:

- **0-dim — single locus.** Already shipped.
- **1-dim — a stem.** `std::lotus::Grow.along(axis)`; smallest
  demos: single-symbol mdgw, single-window triangulator.
- **2-dim — a branch.** Axis composition + junction closures
  (`sum(branches) ~~ trunk within ε` at every fork).
- **3-dim — a mesh.** Modes (bulk / harmonic / resolution) become
  meaningful; waist has enough internal structure to project
  three ways.
- **n-dim — arbitrary composition.** No new structure; algebra
  works at any depth.

Static-vs-dynamic growth dissolves at this framing: outer axes
are mostly configured-at-boot (Venue, Window-class, Model-class);
inner axes are continuously growing (Time/Tick). The harness
needs both regimes, at different depths.

### Library inventory

Marked **need** (load-bearing for demo apps), **probable** (real
use wants it), or **stub** (sketch surface, punt body).

| Namespace | Status | Surface |
| --- | --- | --- |
| `std::lotus` | need | Axis declaration + growth-tip + junction-closure + `Bridge.between` |
| `std::io::ws` | need | Saturated single-connection WebSocket client; `: schedule pinned` |
| `std::io::frame` | probable | Typed frame primitives — length-prefix, delimiter, JSON-blob, line |
| `std::bus::udp_multicast` | need | Line-rate fanout adapter; best-effort, fanout-max=many, ordering=none |
| `std::bus::tcp` | stub | Ordered, reliable, point-to-point |
| `std::bus::nats` | stub | Broker-mediated, reliable, request-response |
| `std::window::ring` | need | Monotonic-time-indexed ring buffer; Recognition-class allocator |
| `std::window::decay` | probable | Decay-weighted accumulator |
| `std::window::bucket` | stub | Time-bucketed aggregator; open-codable on `ring` |
| `std::geom::segment` | need | Linear regression over (t, v) pairs; slope + intercept + residual |
| `std::geom::leading_edge` | probable | Slope / curvature / acceleration at freshest point |
| `std::geom::triangulate` | stub | Multi-source position estimator |
| `market::book` (3p seed) | probable | Canonical L2 order book type |
| `market::tick` (3p seed) | probable | Top-of-book canonical type |
| `market::normalize` (3p) | stub | Per-venue → canonical layer |

Plus pattern catalog entries (not libraries):

- **`std::hourglass`** — waist-locus + ingest-locus + fanout-publish recipe.
- **`std::stem`** / **`std::branch`** / **`std::fan`** — growth-shape variants.

### Language extensions the harness needs

Two proposals; neither committed:

1. **Tier as coordinate.** Current `tier 4` (scalar) → `tier (4, 2, 1)` (n-tuple, one component per growth axis). Shape declared / inferred at compile time; concrete values assigned at birth from parent's tier + axis-step.
2. **Runtime-readable `self.tier`.** Today tier is declaration-only. Expose as runtime-readable tuple (as `self.k_max` is per F.16). Enables positional bus-subject construction, bridge addressing, closure logging. Immutable after birth — recoordination = dissolve + rebirth.

### Edge-loci and the bridge pattern

Edges can themselves be loci when there's flow / work on them (normalizers, throttlers, multiplexers). Rule: **edge-locus's dim-span = endpoint dim-distance + 1**. Parent↔child edge spans 2 dims; sibling↔sibling edge spans 1 dim — but vertical-only flow forces sibling-bridges to be vertical children of the shared parent (the "bridge pattern": one extra vertical child mediating laterally-shaped flow through vertical edges).

### Not in scope

- `std::lotus::garden` — opportunistic-GC projection class on a graph-cyclicity axis. F.2 leaves the door open; no demo needs it.
- `std::script::*` — embedded meta-language (Lua-inside-Aperio). Different question; locus-bounded guest evaluator.
- Exchange-specific protocols (`fix`, `ouch`) — WebSocket covers the demo case.
- NATS / TCP adapter bodies — stubs only; UDP multicast covers the demo.

### Sketches landed (2026-05-10)

Eight numbered examples exercise the surfaces above as pure-
Aperio sketches. Each compiles via `aperio build` and passes
its own `std::test::assert*` suite on run; exit 0 silent =
green. Lifted to bundled stdlib (`crates/aperio-codegen/runtime/stdlib/`)
when the surface settles and the path-rewrite list is updated.

| Sketch | Surface | Status |
| --- | --- | --- |
| `examples/51-geom-segment` | `Segment` online linear-regression accumulator (`push` / `slope` / `intercept` / `count` / `clear`) | green |
| `examples/52-window-decay` | `Ema` exponential moving average (`push` / `value` / `count` / `clear`) — discrete per-push `alpha`; time-weighted variant deferred until `std::math::exp` ships | green |
| `examples/53-window-ring` | `Ring` 8-cap (t, v) ring buffer (`push` / `len` / `time_at` / `value_at` / `clear`); fixed cap until generic `Ring<N>` lands | green |
| `examples/54-geom-leading-edge` | `LeadingEdge` windowed regression (ring + ephemeral Segment via `fit()`); `slope` / `intercept` / `extrapolate` | green |
| `examples/55-geom-triangulate` | `Triangulator` parent locus accepting `LeadingEdge` children; modes `bulk` (mean) / `harmonic` (spread) / `resolution` (closest-to-mean) — the canonical hourglass-waist shape | green |
| `examples/56-io-frame-line` | `LineFrame` line-oriented stream parser (`feed` / `next` / `lines_emitted` / `pending`); decoupled from TCP — same parser drives stdin, sockets, test buffers | green |
| `examples/57-geom-velocity` | `Velocity` finite-difference d/dt + d²/dt² with EMA smoothing (`push` / `velocity` / `acceleration` / `count` / `clear`) | green |
| `examples/58-stat-correlate` | `Correlator` online Pearson r² (`push` / `r_squared` / `covariance` / `count` / `clear`); `r` reported as `r²` until `std::math::sqrt` ships | green |

Friction surfaced during this round logged at
`notes/aperio-friction.md` (three entries dated 2026-05-10):
closure-keyword-shadows-helper-ident, if-needs-block-value,
float-surface-gaps. None blocked the sketches; each is a small
ergonomics gap that compounds.

## How priorities are set

The next milestone is chosen from friction signals — moments where
real Aperio programs hit something missing. Sources:

- `notes/aperio-friction.md` — global friction log.
- `apps/<name>/FRICTION.md` — per-app logs.
- The IDE design plan's own pre-emptive friction list (above).

If you are an agent session writing an Aperio program, your
friction log is the prioritization signal. See
`notes/agent-onboarding/app-dev-brief.md`.
