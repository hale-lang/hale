# Codebase-onboarder — progress report

> Date: 2026-05-10. **Updated** after the polished-middle-step
> arc (apps/onboard, per-codebase overrides, handler route
> derivation, goroutine target resolution) AND the corpus
> extraction pass (std::iter::Lines, std::tagged::Accumulator,
> std::name::Convention, std::json::Builder, std::io::fs::extension
> primitive + all five app migrations).
> Plan: `notes/codebase-onboarding-design.md`.
> Validation: `notes/three-tower-validation.md`.
> Shape rules: `notes/onboarding-shape-rules.md`.
> Refactor: `notes/aperio-refactor-proposal.md` (now historical).

## The pitch (one paragraph)

A dev with an existing codebase points the tool at it. They see
their own code rendered as **three lotus towers** (operational,
import-graph, domain) and recognize the shape — *before* they
have learned what an Aperio lotus is. Aperio then absorbs the
codebase into Aperio source. The blind-migration framing: three
perspectives are the minimum count for the recognition to
operationalize without prior model.

Per the design doc, this is the **primary product target**,
ahead of the runtime-IDE arm.

## Where we are: data layer complete, rendering deferred

### ✅ Shipped — substrate

| What                              | Milestone | Status |
|-----------------------------------|-----------|--------|
| Tree-sitter substrate             | m96       | done   |
| Go grammar bundled                | m96       | done   |
| Language-agnostic facade (`std::lang::Lang`) | bonus | done   |

`std::lang::Lang` was not in the original plan — it landed in
response to the *"are we shaping it language agnostic"* check-
in. All Go-specific kind strings + idiom heuristics live behind
one type with a flavor switch. Adding Rust / Python / TS is a
flavor-arm extension, not an extractor rewrite.

### ✅ Shipped — three towers (data layer)

| Tower         | Mode       | Milestone | Status                |
|---------------|------------|-----------|-----------------------|
| Import-graph  | harmonic   | m97       | done (file-level)     |
| Operational   | resolution | m100 v0   | done (tree-sitter only; LSP deferred) |
| Domain        | bulk       | m102 v0   | done (15-entry Go lookup seed) |

All three emit JSON tower data. Per
`notes/three-tower-validation.md`, run on a shared Go fixture,
the three perspectives **triangulate** — the recognition lands
at the data level. File-level granularity has known awkwardness
(Go packages span files; rolled up downstream is the planned
fix), but the lotus shape is visible across the three views.

### ✅ Shipped — apps and tests

| App                       | What it does                              | Tests |
|---------------------------|-------------------------------------------|-------|
| `apps/ts-walk-demo`       | Substrate-validation walk of Go AST       | 4     |
| `apps/import-graph`       | Harmonic tower extractor                  | 5     |
| `apps/operational-graph`  | Resolution tower extractor                | 6     |
| `apps/domain-graph`       | Bulk tower extractor + morpheme rewriter  | 5     |

Each app is locus-shaped per the apps-are-loci ethos. The
`MorphemeRewriterL` inside domain-graph also validates the
**namespace-lotus pattern** — empty `params { }`, only methods,
self-method calls compose.

### ✅ Shipped — the polished middle-step product

| What                              | Status        |
|-----------------------------------|---------------|
| Tower aggregator / cross-tower join (m102.5) | done — `apps/tower-join` |
| Polished CLI recognition report   | done — `apps/onboard` |
| Per-codebase overrides loop       | done — `<dir>/.aperio-overrides` |
| HTTP handler → route derivation   | done — handlers show routes inline |
| Goroutine call-site → target file | done — cross-file FN_DEF aggregate |
| Shape-rules doc for agents        | done — `notes/onboarding-shape-rules.md` |
| `std::lang::Morpheme` (stdlib)    | done — flavor + overrides params |

The agentic feedback loop is closed end-to-end: agent runs
onboard → reads structured recognition report with cross-file
flow + agent-actionable unknowns → resolves unknowns by reading
flagged files → drops decisions in `<dir>/.aperio-overrides` →
re-runs onboard, recognition crystallizes. Universal seed
stays clean; per-codebase vocabulary lives next to the codebase.

### ✅ Shipped — corpus extraction (post-polished-middle-step)

Per `notes/aperio-refactor-proposal.md` (now historical), the
duplicated helper clusters across the `apps/` tree lifted into
the std seed:

| Extraction                | Surface                                  | Status |
|---------------------------|------------------------------------------|--------|
| `std::io::fs::extension`  | basename-aware extension primitive       | done   |
| `std::iter::Lines`        | cursor-shape newline walker              | done   |
| `std::tagged::Accumulator`| TAG:body line accumulator parsing        | done   |
| `std::name::Convention`   | snake/Camel + file-stem → LocusL         | done   |
| `std::json::Builder`      | JSON-shape glue helpers                  | done   |

All five extractions ship with stdlib doc pages
(`docs/src/std/{iter,tagged,name,json}.md`) and ~25 new tests
across `crates/aperio-codegen/tests/stdlib_{iter,tagged,name,json}.rs`
plus a 6-shape test in `stdlib_fs.rs`. Workspace test count
moved from 290 → 312 across this arc.

All five app migrations land byte-identical output against the
checked-in fixtures (`apps/operational-graph/fixture`,
`apps/import-graph/fixture`). Net: ~430 LOC of duplicated
glue removed from `apps/`, ~150 LOC added to the std seed.

One v0 compiler bug surfaced during the migration and was
logged: cross-locus method return that follows a different
sub-locus's method call loses its String at the free-fn return
boundary. Workaround is inline primitive concat; tracked in
`notes/aperio-friction.md` as `2026-05-10 cross-locus-return-
deep-copy`. Two functions (`__collect_types_with_motion` in
tower-join, `__collect_section` in operational-graph) carry
the workaround.

### ⏳ Not yet — rendering and follow-ons

| What                              | Milestone | Effort       | Status |
|-----------------------------------|-----------|--------------|--------|
| Tower aggregator (file → package) | m102.5    | small        | next critical-path |
| Cross-tower join layer            | m103a     | small-medium | needed for unified product |
| Graphics substrate (Bevy host)    | m98       | multi-month  | deferred until aggregator + join validated |
| Single-tower viz v0               | m99       | ~2 weeks     | blocked on m98 |
| Three-tower rendering + mode switch | m103   | ~2 weeks     | blocked on m99 |
| `std::lsp` (LSP-client)           | m101      | ~1-2 weeks   | deferred; tree-sitter heuristics suffice for v0 |
| Domain lookup expansion           | m102.5    | curation     | small but high-leverage |
| Transpiler v0 (Go → Aperio)       | m105      | ~3 weeks     | needs aggregator + join |
| `std::ui` (egui host)             | m104      | multi-month  | deferred |
| `std::mcp` (MCP server)           | m106      | multi-month  | deferred |
| Rust/Python/TS via Lang flavor arms | parallel | ~half-day each | deferred until Go validated |

## Now-open follow-ups

Items below the critical path; useful but not blocking the
demo product:

1. **Validation on a real-world Go project.** Micro-fixtures
   landed; a small open-source project would surface scaling
   issues (parsing edge cases, lookup-miss patterns at scale,
   recognition narrative quality). ~1 day to set up + iterate.
2. **Tower-join JSON parity with onboard.** Onboard surfaces
   handler routes + goroutine target files; tower-join's JSON
   doesn't yet. Quick mirror refactor (the unified walk
   shipped; just thread WIRE + FN_DEF aggregates through).
3. **Add a Rust flavor to `std::lang::Lang`.** Validates the
   single-locus + flavor-switch pattern by adding a second
   per-language implementation. Requires bundling
   `tree-sitter-rust` in `aperio-ts-shim`. ~half-day.
4. **Per-framework HTTP routing.** Currently detects
   `*.HandleFunc` (stdlib net/http). Chi/Echo/Gin/Gorilla
   variations (`r.Get`, `e.GET`, `app.Get`, etc.) extend by
   adding more field_identifier names to
   `is_handler_registration`'s match list.
5. **Method-based fn detection.** Currently
   `extract_spawn_target` only resolves bare-identifier callees
   (`go someFn()`). Selector callees (`go obj.Method()`) return
   "" — agent fills these in or they remain unresolved.
6. **Recognition narrative depth.** Current narrative is
   template-driven on summary counts; could grow cross-file
   relationship descriptions ("MainL spawns WorkerL via
   backgroundWorker; WorkerL drains via parent dissolve").

## Distance to the demo product

Per the design doc, the **demo-product candidate** is m96 + m97
+ m98 + m99 — single-tower (import-graph, the cheapest)
rendered with the graphics substrate. We're 2-of-4 done on
that bundle. **The data layer is fully shipped; the gap is
purely visualization.**

The **real product** is m103 — three towers with mode switching.
We're 3-of-N done on that, where N includes m98+m99+m103.

## Critical path before any visualization investment

Per `notes/three-tower-validation.md`, three items were
identified as needing to ship before m98. Status update:

1. **Tower aggregator** — ✅ shipped via `apps/tower-join`.
2. **Cross-tower join layer** — ✅ shipped. Cross-tower-
   agreement rule emits loci; HTTP handler → route derivation
   surfaces subjects; goroutine call-site → target file
   resolution does cross-file flow inference without LSP.
3. **Domain lookup table expansion** — ✅ superseded by the
   agent-driven reframe (`notes/onboarding-shape-rules.md`).
   Per-codebase overrides via `<dir>/.aperio-overrides` close
   the agent loop — agents resolve unknowns by reading source,
   not by extending a universal table. The seed stays at 15
   universally-clear entries.

All three items: done, with interpretation refined per the
agent-first reframe.

## Friction logged this session

Driving the next round of language follow-ups:

- **Lifecycle method bodies don't accept `return`** — `birth()`
  / `run()` / `dissolve()` reject `return`; short-circuit
  paths must factor into a free helper fn. m82 follow-up.
- **fn-pointer callbacks can't share state** — already in the
  brief, hit again here when the tagged-accumulator section
  formatter needs flavor context.
- **`aperio run` rejects qualified-name literals** — every
  app must be `aperio build`'d, not `run`. Pre-existing,
  consistent.
- **`list_dir` is newline-string, not `[String]`** — every
  extractor manually splits. Waits on `List<T>`.
- **`fs::mkdir` missing** — not hit yet but tracked for when
  a tower writer wants to mkdir its output dir.
- **Aperio doesn't have multi-file modules yet** — every app
  is one big main.ap. Means duplication where shared types
  would be natural. Tracked as Phase 1+ language work.

## Verdict

**Data layer is solid. The blind-migration framing holds on
real Go code.** Three towers, three perspectives, hand-validated
to triangulate. The architecture is now language-agnostic via
`std::lang`; adding a second source language is a half-day move
when justified.

The next bite-sized increment is the **tower aggregator**
(m102.5) — small, unblocks the cross-tower join, lets us run
the full pipeline against richer fixtures. Then graphics
(m98), then we have the demo product.

The runtime-IDE arm
(`notes/aperio-ide-design.md`) remains secondary; substrate
shared with this product (graphics, UI, MCP) gets built once
and used by both.
