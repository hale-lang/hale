# apps/ferryman

> Codebase-onboarder. Static + agent-driven recognition of
> foreign codebases. The partner-facing CLI evolution of
> `apps/onboard/`.
>
> Audience for this README: a dev cloning lotus-lang who wants
> to understand the ferryman app from the inside — its file
> layout, its pipeline, its CLI, and how the pieces fit. For
> using ferryman against your own codebase, see
> [`notes/getting-started-ferryman.md`](../../notes/getting-started-ferryman.md).

## What it is

Ferryman recognizes a codebase by extracting three lotus
towers (operational / harmonic / domain) per discovered binary
and rendering them as a partner-readable report. The static
stages run as one binary; the recognition stage runs as an LLM
agent reading source and writing enrichment cells back to a
yaml that the renderer consumes.

The whole architecture is **filesystem-first**: the cheapest
stage (FS walk) runs on any-size codebase; the expensive
per-binary stages are opt-in per the agent's picks. See the
"Pipeline" section below.

## Build

`apps/ferryman/` is a per-directory seed (every `.ap` file in
this dir is one bundle). Build with the directory as the
target:

```
aperio build apps/ferryman/
```

This produces `apps/ferryman/ferryman` (the binary name follows
the directory name). Single-file invocations like
`aperio build apps/ferryman/main.ap` will fail — `main.ap`
references identifiers defined in sibling files.

## File layout

```
apps/ferryman/
├── main.ap     entry point, FerrymanL (legacy report driver),
│               DriveL (bare-repo orchestrator), SkeletonL,
│               RenderL, CLI dispatch, all the shared per-binary
│               helpers
├── tree.ap     stage 0 — FS lotus extraction + flavor
│               detection + .gitignore filtering
├── prompt.ap   PromptL — emits the agent enrichment protocol
│               text (embedded String literal; compact version
│               of notes/agent-onboarding/ferryman-enrichment-protocol.md)
├── README.md   this file
└── ferryman    build artifact (gitignored)
```

`main.ap` is large (~2400 lines) and would benefit from
further decomposition into `skeleton.ap` / `render.ap` /
`topology.ap`. Tracked as a future cleanup; not blocking
anything.

## Pipeline

```
ferryman <repo> [--lang=go]
   │
   ▼
 ┌─────────────────────────────────────────────────┐
 │ STAGE 0 — filesystem lotus                      │
 │   tree.ap : __build_tree_yaml                   │
 │   → <repo>/.ferryman/00-tree.yaml               │
 └─────────────────────────────────────────────────┘
   │
   │  the agent reads tree.yaml + source,
   │  picks 2-4 binaries to deep-dive
   ▼
 ┌─────────────────────────────────────────────────┐
 │ STAGE 1 — per-binary static towers              │
 │   main.ap : SkeletonL → __build_skeleton        │
 │   ferryman skeleton <bin-dir> <out.yaml>        │
 │   → .ferryman/01-<bin>-skel.yaml                │
 │   Emits operational (outward_tower) + harmonic  │
 │   (inward_tower) + domain skeleton (names) for  │
 │   one binary. Splitting into separate           │
 │   01-operational.yaml / 02-harmonic.yaml /      │
 │   03-domain-skel.yaml is a future refactor;     │
 │   today they share one yaml per binary.         │
 └─────────────────────────────────────────────────┘
   │
   ▼
 ┌─────────────────────────────────────────────────┐
 │ STAGE 4 — agent enrichment                      │
 │   no ferryman code — protocol-driven            │
 │   .ferryman/PROMPT.md (from prompt.ap) tells    │
 │   the agent what cells to add.                  │
 │   Agent writes 04-<bin>-enriched.yaml.          │
 └─────────────────────────────────────────────────┘
   │
   ▼
 ┌─────────────────────────────────────────────────┐
 │ STAGE 5 — render                                │
 │   main.ap : RenderL → __do_render               │
 │   ferryman render <enriched.yaml>               │
 │   Pure presentation: reads yaml via             │
 │   std::yaml::Reader, writes via                 │
 │   std::text::Sink (interface). No tree-sitter,  │
 │   no morpheme rewriting.                        │
 │   → recognition-<bin>.txt                       │
 └─────────────────────────────────────────────────┘
```

Stage 4 isn't a ferryman code path — it's a *protocol* the
agent follows. The protocol text is shipped via
`prompt.ap`'s `__prompt_text()` and lands as `PROMPT.md` in
the `.ferryman/` output dir during stage 0.

## CLI surface

```
ferryman <repo> [--lang=go]      bare-repo: stage 0 + write
                                 PROMPT.md. The default UX.

ferryman skeleton <bin-dir> <out.yaml>
                                 stage 1 for one binary.
                                 Agent invokes this per pick.

ferryman render <yaml>           stage 5. Reads the yaml,
                                 writes a text report to
                                 stdout.

ferryman prompt [write <path>]   emits the enrichment protocol
                                 text. Without args → stdout.
                                 With `write <path>` → file.

ferryman                          (no args) legacy fallback:
                                 the FerrymanL all-in-one
                                 report driver on the default
                                 fixture. Pre-pipeline shape;
                                 preserved for the
                                 apps/operational-graph/fixture
                                 regression test.
```

Dispatch lives in `main.ap`'s `fn main()`. The bare-repo form
constructs `DriveL`; the subcommand forms construct
`SkeletonL` / `RenderL` / `PromptL`. The fallback constructs
`FerrymanL`.

## Conventions the code follows

Per `spec/styleguide.md`:

- **Apps are loci.** Every entrypoint is `<Name>L` (DriveL,
  SkeletonL, RenderL, PromptL, FerrymanL).
- **Lifecycle bodies reject `return`.** Short-circuit logic
  factors into `__do_*` / `__drive_*` helpers called from the
  lifecycle method.
- **Free fns use `__` prefix** to mark seed-internal scope.
  Cross-file visibility within the per-directory seed makes
  this convention more important, not less — the prefix says
  "implementation detail, not user surface."
- **The three-tower frame** is the source axiom for what gets
  recognized. Per `notes/aperio-types-vs-loci.md`: a source-
  level node is a locus iff ≥2 of (operational / harmonic /
  domain) towers point at it with coherent roles. The
  renderer's "Loci (cross-tower agreement)" section surfaces
  this verdict (a future rename will use the three-tower
  names directly).
- **Honest unknowns.** The skeleton stage doesn't guess at
  ambiguous syntax (`kind: "method"` with lowercase receiver
  is the canonical case). Resolution is the agent's job, not
  a heuristic in the static layer. Same principle in the
  enrichment cells — agents log `unknowns` entries instead of
  fabricating.

## Smoke testing

The in-repo smoke target is `apps/operational-graph/fixture/`:

```
./apps/ferryman/ferryman apps/operational-graph/fixture --lang=go
# → apps/operational-graph/fixture/.ferryman/
#   00-tree.yaml (small; 4 .go files)
#   PROMPT.md

./apps/ferryman/ferryman skeleton \
    apps/operational-graph/fixture \
    apps/operational-graph/fixture/.ferryman/01-skel.yaml

./apps/ferryman/ferryman render \
    apps/operational-graph/fixture/.ferryman/01-skel.yaml
# → 61-line text report to stdout
```

Use `--lang=go` since the fixture has no `go.mod` to auto-detect
from. Output dir is gitignored.

For larger smoke-testing, point at any Go monorepo you have
locally. Stage 0 (filesystem lotus) scales to grease's 36-binary
monorepo cleanly; stage 1 is bounded by the Reader concat
ceiling — see "Known limits" below.

## Known limits

- **Stage 1 on a single very-large binary** can hit the O(N²)
  yaml-build ceiling inside `std::yaml::Reader.list_item` and
  `.nested`. Per
  [`notes/aperio-friction.md`](../../notes/aperio-friction.md)'s
  `2026-05-10 reader-list_item-quadratic-concat` entry, this
  is a stdlib runtime issue, not a ferryman issue — waits on
  a rope / string-view primitive at the language level.
  Mitigation: deep-dive specific binaries rather than running
  skeleton on the whole monorepo (which is the architectural
  shape anyway).
- **The renderer uses "Outward tower" / "Inward tower" /
  "Loci (cross-tower agreement)" as section headers**, not
  the operational/harmonic/domain names from the design notes.
  A rename pass is queued — the framing lives in PROMPT.md and
  the docs, not in the user-visible output.
- **YAML escape leak in summary rendering.** A summary cell
  containing `\"foo\"` shows the literal backslash in the
  rendered report. Cosmetic.
- **Inward tower can be verbose** on binaries with deep
  transitive import graphs — lists every reachable file. For
  partner-facing reports, a collapse-or-filter pass might be
  worth a follow-up.
- **Single-language v0.** Go only. Other flavors (Rust, Node,
  Python) detect from manifests in `tree.ap` but don't yet
  have flavor-specific skeleton-stage extraction. Adding a
  flavor is a `std::lang::Lang` extension plus a small
  ferryman pass over kind classification.

## Where to read next

- [`notes/getting-started-ferryman.md`](../../notes/getting-started-ferryman.md) —
  user-facing walkthrough; the dev-using-ferryman story.
- [`notes/agent-onboarding/ferryman-enrichment-protocol.md`](../../notes/agent-onboarding/ferryman-enrichment-protocol.md) —
  long-form version of what `PROMPT.md` ships; the protocol
  the agent follows for stage 4.
- [`notes/aperio-types-vs-loci.md`](../../notes/aperio-types-vs-loci.md) —
  the source axiom; where the three-tower-agreement rule
  comes from.
- [`notes/onboarding-shape-rules.md`](../../notes/onboarding-shape-rules.md) —
  the Agent / Entity / Shape noun categories used during
  enrichment.
- [`notes/codebase-onboarder-progress.md`](../../notes/codebase-onboarder-progress.md) —
  cross-session journal; project state.
- [`spec/styleguide.md`](../../spec/styleguide.md) — the
  six-pattern catalog and naming conventions ferryman's code
  follows.
- `apps/onboard/main.ap` — the predecessor; still alive. Will
  retire once ferryman covers its test surface.
