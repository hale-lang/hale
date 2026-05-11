# Aperio Codebase Onboarding — design plan

> *The dev sees their existing codebase rendered as three lotus
> towers and recognizes the shape. Aperio then absorbs the
> codebase into Aperio source.* This is the load-bearing pitch.
> Every other decision falls out of it.

## Context

This plan supersedes `notes/aperio-ide-design.md` as the
**primary product target**. The IDE-as-runtime-visualizer
remains a coherent secondary product (you watch your *running*
Aperio program in three lotus views), but the codebase-
onboarding pitch reaches every dev with a codebase, not just
devs already writing Aperio. The substrate requirements
overlap substantially; the first-shipped product is the
codebase onboarder.

**The thesis:** there are four doorways into Aperio:

1. **Curious devs** — `docs/src/grimoire/`. Already shipped.
2. **Technical devs** — `docs/src/book/`. Already shipped.
3. **Devs with existing codebases** — *this product*. They
   never have to learn Aperio's primitives in the abstract;
   they see *their own code* already exhibiting them.
4. **The codebases themselves** — Aperio absorbs them. The
   spell of spellcasting completes the circle.

(3) is the marketing edge: every dev with a codebase has
something to feed in. (4) is the ambitious endgame: the
language eating other languages.

## The three lotus perspectives

Operational principle: **three perspectives are needed to
operationalize an idea in the dark.** This is a blind
migration — the dev is being shown that their code already
has lotus shape *before* they understand what lotus shape is.
Three views from different epistemic angles triangulate the
recognition.

The three perspectives map directly onto Aperio's projection
modes — same source projected three ways, exactly the primitive
the language already has for "view a thing through three
lenses."

| Perspective    | Mode       | Question it answers              | Primary tool      |
|----------------|------------|----------------------------------|-------------------|
| Operational    | resolution | "What runs? What moves?"         | LSP (semantic)    |
| Import-graph   | harmonic   | "What depends on what?"          | tree-sitter       |
| Domain         | bulk       | "What is this code *about*?"     | tree-sitter + rule-based morpheme rewriter |

Each tower is the same codebase — viewed differently, structured
differently, surfaced as different loci with different
contracts. **A dev rotating between the three towers sees the
same artifact under three modes; that experience IS Aperio's
mental model, internalized through their own code rather than
through a tutorial.**

### Cross-tower agreement = locus identity

The three towers are independent extractions, but they describe
*one* underlying codebase. When the same source entity surfaces
in multiple towers, that coincidence is the **signal** of
locus-ness. Per the foundational axiom *types are for shapes,
loci are for flow* (`notes/aperio-types-vs-loci.md`), the join
layer that produces absorbed Aperio source obeys this rule:

- **Node-name with ≥ 2 tower roles → emit as a locus.** It
  appears in the operational tower as an entrypoint / handler /
  spawn-target *and* in the import-graph tower as the originator
  of certain dependencies, *or* in the domain tower as a named
  entity with a motion-form. Cross-tower coincidence means the
  codebase itself treats this thing as having flow.
- **Node-name with exactly 1 tower role → emit as a type, free
  fn, or structural comment.** Single-tower presence is a leaf
  artifact, not a locus. A pure-data struct with no operational
  signature is a `type`. A free function with no concurrent /
  handler / lifecycle role is `fn`.
- **Three-tower agreement → emit as a locus AND surface its
  motion-form** (from the bulk tower) as a `// motion: ...`
  comment until Aperio has a first-class metadata slot. This
  is the strongest signal: the codebase explicitly names this
  thing.

The join layer **never invents loci**. Cross-tower coincidence
does the inventing. The absorber's job is to detect agreement,
not to impose structure.

**Naming convention.** When a locus is emitted, its name comes
from the operational tower's role (entrypoint / handler / spawn-
target name) with an `L` suffix per the apps-are-loci rule.
The motion-form goes in a comment, never in the name — names
are the codebase's, not the extractor's.

**Internal behavior is itself a tower.** When a locus is
identified, its body — methods, lifecycle steps, child loci,
sub-flows — gets extracted as a sub-tower one layer down (per
the recursive principle). The recursion bottoms out at primitive
operations.

### Operational lotus (resolution mode)

The runtime / process model. What entities have lifecycles?
What's an entrypoint? What spawns concurrent work? What holds
long-lived state? This is the closest perspective to a literal
Aperio program — services become loci, async tasks become
children, request handlers become bus subscribers, etc.

Extraction rules (per language):

- Top-level binary entry → root locus's `main`.
- Long-lived service object / class with init + shutdown →
  locus with `birth` + `dissolve`.
- Background worker / `tokio::spawn` / `go func` → child locus,
  schedule class inferred (cooperative for green threads,
  pinned for OS threads).
- HTTP route handler / message queue handler → bus subscriber
  on a derived subject.
- Long-running loop → `run()` body.
- Static singleton state → locus params.
- Inter-service network calls → bus publishes (subject
  derived from URL or queue topic).

Needs **semantic** information — type info, call hierarchies,
async-fn detection, who-spawns-whom across files. **Language
servers are the right substrate.** Tree-sitter alone would
miss "this is a tokio task" vs. "this is a sync helper."

### Import-graph lotus (harmonic mode)

The codebase's own dependency structure. What modules /
packages / files import what? What are the cycles? What's the
fan-out / fan-in shape? This perspective treats *the codebase*
as the lotus — modules are loci, imports are contracts, and
the harmonic projection (relationships across the whole)
shows the structural skeleton.

Extraction rules:

- Each module / package / file → a locus.
- `import` / `use` / `require` statements → contract
  relationships.
- Re-exports → expose ports.
- Cross-module function calls → bus-style references.
- Module init order → birth ordering.
- Cycles → flagged as F.4 violations (Aperio doesn't permit
  cyclic dependencies; rendering the cycle is a teaching moment).

**Tree-sitter is sufficient** for this perspective — per-file
import statements, parsed locally. LSP can refine (resolve
relative imports, follow re-exports), but the 80% from
tree-sitter is enough for the visualization.

### Domain lotus (bulk mode)

The semantic / business-domain layer. What concepts does this
codebase work with? **Built on the principle that "all
language describes motion."** Every named thing in the code is
a frozen motion-form: nouns are stilled verbs. Reverse-rendering
the symbology means taking each name and finding its motion-
equivalent — which is exactly what an Aperio locus IS (a
frozen motion: birth → run → dissolve).

The rewriting is **a deterministic transformation**, not an LLM
inference. Given the source language's grammar, identifier
names follow regular patterns:

| Pattern                       | Rule                                  | Example                     |
|-------------------------------|---------------------------------------|-----------------------------|
| `-er` / `-or` / `-ar` suffix  | strip suffix → add `-ing`             | Controller → controlling    |
| Compound CamelCase / snake    | split morphemes, apply rule per token | OrderProcessor → order-processing |
| Method-style verb-already     | keep as-is, mark as motion            | `process()` → processing    |
| Entity-noun (no -er suffix)   | lookup table                          | Repository → carrying       |
| Unknown morpheme              | mark explicit `<unknown:Foobaz>`      | Foobaz → ?                  |

A small lookup table covers the entity-noun cases that don't
fit the suffix rule. Initial seed:

| Codebase symbol      | Motion-form         | Locus shape                                |
|----------------------|---------------------|--------------------------------------------|
| `Controller`         | controlling         | locus that controls some state             |
| `Processor`          | processing          | locus with run() body                      |
| `Manager`            | managing            | parent locus accepting children            |
| `Handler`            | handling            | bus subscriber                             |
| `Listener`           | listening           | locus with run() that accepts              |
| `Validator`          | checking            | closure-test on incoming data              |
| `Builder`            | building            | locus that constructs and yields           |
| `Parser`             | parsing             | transformation locus                       |
| `Repository`         | carrying / holding  | locus that holds state, exposes via contract |
| `Cache`              | remembering         | locus with TTL-shaped lifecycle            |
| `Bus` / `Queue`      | routing / queuing   | the bus itself, made explicit              |
| `Pool`               | pooling             | parent locus with N replicas               |
| `Pipeline`           | piping / flowing    | locus chain                                |
| `Service`            | serving             | top-level locus with run() body            |

The table is per-target-language extensible — Go's idiom is
slightly different from Rust's slightly different from
Python's. Each language's extractor seeds its own lookup
table; unknown identifiers surface as explicit
`<unknown:Foobaz>` markers in the visualization rather than
fabricated motion-forms. **Honesty about uncertainty preserves
the recognition; over-claiming breaks it.**

Extraction needs:

- Identifier nodes (tree-sitter gives positions).
- Doc-strings / comments (tree-sitter again — for *display
  context* in the visualization, not for inferring motion).
- Class / type / fn definitions and their members.
- The morpheme-tokenizer + rule table + lookup. Pure
  deterministic transform. Per-language seed.

This perspective is the gateway drug. The dev sees their own
code's vocabulary translated into Aperio's vocabulary by a
mechanical, auditable rule and recognizes the mapping. The
other two perspectives prove the mapping structurally; this
one shows it is already in the names.

## The transpiler — downstream of extraction

Once the three lotus towers exist as data structures (not just
visualizations), generating Aperio source from them is
**structural code generation, not parsing**. The hard work
already happened in extraction.

Transpiler shape:

- Input: the three tower models + access to the original
  source.
- Output: a directory of `.ap` files representing the codebase
  as Aperio.
- Function bodies stay as comments / `TODO: port from
  <language>` markers — full body translation is a research
  problem; structural-skeleton translation is engineering.
- Domain perspective informs **naming** (loci named via
  motion-forms, not original noun-forms).
- Import-graph perspective informs **module structure** (one
  `.ap` file per module, contracts between them).
- Operational perspective informs **lifecycle wiring**
  (birth / accept / run / drain / dissolve, schedule classes,
  bus subjects).

The transpiler IS itself an Aperio program — it composes
`std::io::fs` to walk the codebase and write output, `std::ts`
(tree-sitter) to read foreign source, and pure data
manipulation between them. ~one milestone given the
extractors and the prerequisite stdlib surface.

The output is **scaffolding**, not running code. The human (or
another agent session) ports body logic afterward. That's the
right shape — and a pattern that fits the parallel-sessions
strategy already validated in this repo.

## Substrate stack required

Each follows the m71-m95 pattern: C or Rust shim under
`crates/aperio-codegen/runtime/`, `std::*` namespace exposed
via STDLIB_PATH_RENAMES, Aperio-side surface in a new
`runtime/stdlib/<name>.ap` file.

| Layer | Substrate | What it provides |
|-------|-----------|------------------|
| **Foreign-AST ingest** | `std::ts` (tree-sitter shim) | `parse(lang, src) -> AstNode`, `walk(node, visitor)`, `field(node, name)` |
| **Semantic enrichment** | `std::lsp` (LSP-client shim) | Types, call hierarchies, references; queries against rust-analyzer / gopls / pyright |
| **Tower data model** | (Aperio source) | Three record types: `OperationalTower`, `ImportTower`, `DomainTower` |
| **Visualization** | `std::graphics`, `std::ui` | Same as IDE plan's m99-m100. Renders towers, not running programs |
| **MCP for agent control** | `std::mcp` | Same as IDE plan's m102 |
| **Transpiler primitives** | (Aperio source) | Code generation as String concat + `write_file` |

The visualization substrate is **shared with the IDE plan** —
graphics + UI + MCP serve both products. The runtime-debug
substrate (`std::bus::expose` + `lotus.debug.*` instrumentation)
is **specific to the IDE-as-runtime-visualizer product** and
is not a prerequisite for codebase onboarding. That arm becomes
later / optional.

## What's blocking on Aperio's own surface

The transpiler and extractors are AST-walking, collection-heavy
work. Today's gaps:

| Gap | Impact | Workaround until shipped |
|-----|--------|--------------------------|
| Generics (`List<T>`, `Map<K,V>`) | High — AST collections everywhere | Hand-rolled fixed-shape arrays; pain |
| Variant-pattern matching with payloads | High — AST nodes are sum types | Tag-field if-chain; verbose |
| Multi-file Aperio modules | Medium — transpiler is multi-file | Single big `main.ap` until module support |
| `std::aperio::parse` (compiler self-introspection) | Low for transpilation; high for self-validation | Defer; emit then run `aperio check` externally |

Generics + variant-patterns are likely the next major
language-design arc independent of this plan. Multi-file
modules are tracked in the leading-edge dev session. None
strictly block first proof-of-concept work — they make it
painful, not impossible.

## Milestone sequencing

Numbered to slot into Phase 6 (post-m95). The IDE plan's
`std::fs::watch` (m96) and `std::bus::expose` (m97) become
secondary; they ship when the runtime-IDE arm is committed to,
not as prerequisites here.

| #     | Name                                | Effort       | What it unlocks                                                |
|-------|-------------------------------------|--------------|----------------------------------------------------------------|
| m96   | `std::ts` (tree-sitter substrate)   | ~1 week      | All three extractors have foreign AST input                    |
| m97   | Import-graph extractor + tower model | ~2 weeks    | First tower; cheapest to validate the rendering pipeline       |
| m98   | `std::graphics` (Bevy host)         | multi-month  | Visualization surface; shared with IDE plan                    |
| m99   | Tower visualization v0              | ~2 weeks     | One-tower static render; the demo                              |
| m100  | Operational extractor (LSP-driven)  | ~3-4 weeks   | Second tower; LSP integration via `std::lsp`                   |
| m101  | `std::lsp` (LSP-client substrate)   | ~1-2 weeks   | Bundled with m100; could split if needed                       |
| m102  | Domain extractor                    | ~2-3 weeks   | Third tower; needs LLM access (not yet a stdlib namespace)     |
| m103  | Three-tower rendering with mode switching | ~2 weeks | Composes m99 across all three perspectives; the *product*    |
| m104  | `std::ui` (egui host)               | multi-month  | Inspector panels, search, filtering                            |
| m105  | Transpiler v0 (one source language) | ~3 weeks     | Codebase absorption; depends on three towers + std::ts         |
| m106  | `std::mcp` (MCP server)             | multi-month  | Agent-driven view control; shared with IDE plan                |

**The "demo product" candidate** is m96+m97+m98+m99 — one tower
(import-graph, the cheapest) rendered with the graphics
substrate. That's the smallest end-to-end thing that proves the
pitch.

**The "real product" target** is m103 — three towers, mode
switching, dev sees their codebase under three Aperio
projections. The transpiler (m105) is a follow-up that the
demo audience asks for; you don't need it to *sell* the
recognition.

## The "blind migration" framing

The user's epistemological framing: this is a blind codebase
migration. The dev being onboarded does not yet have the
mental model to evaluate whether their code is "really"
lotus-shaped. They have to operationalize the recognition in
the dark.

Three perspectives is the minimum count for that
operationalization. Any single view risks looking like a
gimmick or a forced fit. Three views from orthogonal angles —
operational, structural, semantic — make the lotus-shape land
as a property of the code itself rather than as an
interpretation imposed on it.

This is exactly why Aperio has three projection modes
(bulk / harmonic / resolution) as a language primitive.
Codebase onboarding isn't borrowing the metaphor; it's
applying the language's own primitive to foreign data.

## How this changes the IDE plan

`notes/aperio-ide-design.md` (the runtime-visualizer pitch)
remains coherent but moves to *secondary product*:

- Substrate it shares with codebase onboarding (m98 graphics,
  m104 UI, m106 MCP) is built once and used by both.
- Substrate specific to runtime visualization (`std::bus::expose`,
  `lotus.debug.*` runtime instrumentation) ships when the
  runtime-IDE arm is committed to. No longer a Phase-6
  blocker.
- The dogfooding mechanism is the same: real Aperio programs
  feeding their friction back through the parallel-sessions
  infrastructure.

After codebase-onboarding ships and demonstrates value, the
runtime-IDE arm becomes the natural follow-up: *now that you've
seen your codebase as lotus, here's your running Aperio program
as one*.

## Open questions

1. **(Resolved.)** Domain-extractor was originally sketched as
   LLM-shaped. It is not — given the source language's grammar,
   the noun-to-motion rewriter is a deterministic transform
   (suffix rule + per-language lookup table + explicit
   `<unknown>` markers). No `std::llm` dependency.

2. **Visualization scope per tower.** Each tower could have
   thousands of nodes for a real codebase. UX questions: do
   we cluster? Pan/zoom? Search? Limit to one module at a
   time? Probably "fit-to-view at top level, drill down on
   click" but the answer wants a UX prototype.

3. **Cross-language transpiler scope.** **Go is the v0 target.**
   Reasons: mature tree-sitter grammar; gopls is a stable LSP;
   Go's concurrency primitives (goroutines, channels, init())
   map directly onto Aperio's locus / bus / birth idioms with
   minimal impedance mismatch; struct + method composition
   without inheritance is already Aperio-shaped. Other
   languages (Rust, Python, TypeScript) follow as their own
   per-language milestones once Go validates the per-language
   pipeline shape.

4. **Extraction confidence / accuracy.** What does the
   visualization show when the extractor isn't sure? E.g., a
   function that *might* be an entrypoint vs. *clearly is*.
   Honesty about uncertainty in the visualization preserves
   the recognition; over-claiming breaks it.

5. **Lossy semantics in the transpiler.** Some constructs
   have no Aperio equivalent (Rust borrow checker, Python
   metaclasses, JS prototypes). Does the transpiler emit
   stubs + flags? Refuse to translate? Ask the human?
   Probably emit stubs with explicit `// LOSSY:` markers.

## Risks and kill-switches

| Risk | Detect-point | Pivot |
|------|--------------|-------|
| Three perspectives don't visually triangulate — devs see them as three unrelated graphs, not three views of one thing | First user test post-m103 | Visualization design pass; possibly explicit cross-tower linking (click a node, see it across all three) |
| Domain extractor's per-language lookup table is too sparse — too many `<unknown>` markers | m102 | Iterate on per-language seed table; surface unknowns honestly rather than guessing |
| LSP integration is too brittle per-language | m101 | Fall back to tree-sitter-only operational extraction; lossy but works |
| Tree-sitter / gopls maturity for Go specifically | m96 onwards | Go is mature; risk is low. If Rust target later proves harder, that's downstream |
| Codebase onboarding works but transpilation is unconvincing | m105 | Ship the visualization product without the transpiler; transpilation becomes a v2 feature |
| Visualization substrate is too expensive to build | m98 | Start with 2D HTML/SVG instead of 3D Bevy; the recognition doesn't strictly require 3D |

## Critical files

**To create (m96 — start here):**

- `crates/aperio-codegen/runtime/lotus_treesitter.rs`
- `crates/aperio-codegen/runtime/stdlib/ts.ap`
- `examples/ts-walk-demo/main.ap` (or `apps/`?)
- `tests/stdlib_ts.rs`

**To create (later milestones):**

- `crates/aperio-codegen/runtime/lotus_lsp.rs` (m101)
- `crates/aperio-codegen/runtime/lotus_graphics.rs` (m98)
- `crates/aperio-codegen/runtime/lotus_ui.rs` (m104)
- `crates/aperio-codegen/runtime/lotus_mcp.rs` (m106)
- `crates/aperio-codegen/runtime/stdlib/{lsp,graphics,ui,mcp}.ap`
- `apps/codebase-onboarder/src/` (the actual product, multi-file)

**Sibling plan (substrate is shared, runtime-IDE arm goes
secondary):**

- `notes/aperio-ide-design.md`

## Sequence

1. **m96** — `std::ts`. Walkthrough above. Smallest substrate
   move; unlocks all extraction work.
2. **m97** — Import-graph extractor as a focused dogfood. Read
   a Go module, output a JSON-shaped tower model. Pure data,
   no rendering. Validates whether the lotus-shape mapping is
   tractable.
3. **(spike, 1 week)** — minimal `lotus_graphics_*` rendering
   the m97 tower. Go/no-go on the visualization product.
4. **m98** — `std::graphics`. Big; sub-milestones likely.
5. **m99** — Tower visualization v0 (one tower).
6. **m100 + m101** — Operational extractor + `std::lsp`. The
   second tower.
7. **m102** — Domain extractor. The third tower.
8. **m103** — Three-tower rendering with mode switching. The
   real product.
9. **m104** — `std::ui` (inspector panels, search).
10. **m105** — Transpiler v0 (Go → Aperio).
11. **m106** — `std::mcp` (agent-driven views).
12. **v0 tag** — three-tower demo on a real Go module works,
    plus the Go → Aperio transpiler emits scaffolding that
    `aperio check` accepts.

Each milestone is independently useful — every Aperio program
gets `std::ts`, `std::lsp`, `std::graphics`, `std::ui`,
`std::mcp` for free. The codebase onboarder is just the first
big consumer.
