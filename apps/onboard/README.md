# onboard — Aperio codebase recognition

**The polished middle-step product.** Points an LLM agent (or
a curious developer) at a foreign codebase and produces a
structured recognition report — per-file lotus-shape
classifications with motion-forms, agent-actionable unknowns,
and a recognition narrative.

This is **not the transpiler**. The transpiler (m105) emits
Aperio source. Onboard emits *understanding* — the dev sees
their own code as three lotus towers and recognizes the shape.
The transpiler comes after the recognition lands.

## Run

```
aperio build apps/onboard/main.ap
apps/onboard/main /path/to/go-codebase
```

`argv[1]` is the directory to scan; defaults to the operational
fixture for self-demo. `argv[2]` is the language flavor (`go`
today; `rust` / `python` / `typescript` slot in via
`std::lang::Lang` extension).

## Output shape

Three sections, in order:

1. **Per-file boxes.** One box per source file, each with:
   - Verdict marker — `[locus]` / `[type_or_fn]` / `[structural]`
   - Cross-tower agreement count (0–3)
   - Operational role summary (main/init/handlers/spawns/loops)
   - Harmonic role (imports)
   - Domain role (named types with motion-forms inline)
   - Aperio-shape interpretation (what kind of locus this would
     be in absorbed source)

2. **Unknowns flagged for agent review.** Each `<unknown:X>`
   marker the morpheme rewriter produced, with the file path
   and a one-line action prompt. The shape-rules reference
   (`notes/onboarding-shape-rules.md`) points the agent at the
   three categorical shapes (Agent / Entity / Shape) for
   resolution.

3. **Recognition narrative.** A short paragraph summarizing
   what the towers reveal: how many files exhibit lotus shape,
   what the recognition means, and how the agent should treat
   any unknowns.

## Sample (operational fixture)

```
============================================================
  Aperio recognition: apps/operational-graph/fixture
  flavor: go
============================================================

+-- MainL  (main.go)  [locus]  agreement=2/3 ----
  package       main
  operational   main() entrypoint; init() lifecycle hook; 1 concurrent spawn(s): named
  harmonic      log, net/http
  domain        (none)
  aperio shape  root locus — birth() = init code, run() = main body, child loci = spawned goroutines / handler subscribers

+-- HandlersL  (handlers.go)  [locus]  agreement=2/3 ----
  package       main
  operational   2 HTTP handler(s): helloHandler, statusHandler
  harmonic      fmt, net/http
  domain        (none)
  aperio shape  bus-subscriber locus — each handler is a subscriber on a path-derived subject (HTTP route → bus subject)

[... worker.go, store.go ...]

Unknowns flagged for agent review
----------------------------------

  * store.go : RequestCache
      static rewriter: <unknown:Request>-remembering
      action: open store.go and apply shape rules.
  ...

Recognition
-----------

4 file(s) exhibit lotus shape (at least 2 of 3 tower roles populated).

What this means: your codebase already projects through three lenses
cleanly. ...
```

## How an agent uses this

Per `notes/onboarding-shape-rules.md`, the codebase-onboarder
is designed to run inside an LLM-driven session:

1. Run `apps/onboard/main <project-dir>`.
2. For each unknown in the "flagged for agent review" section:
   - Open the file with the unknown.
   - Read the type definition + nearby usage.
   - Apply the shape rules (Agent / Entity / Shape) to classify.
   - Propose a motion-form (or mark as type, not locus).
3. (Future) Emit per-codebase overrides JSON; re-run; confirm.
4. (Future, m105) Emit absorbed Aperio source from the resolved
   tower model.

## What this is companion to

- **`apps/tower-join`** — emits the same data as raw JSON for
  machine consumption (downstream pipelines, the future
  transpiler, automated CI checks).
- **`apps/import-graph` / `operational-graph` / `domain-graph`**
  — single-tower data extractors. Onboard composes all three
  views; the standalone extractors are for inspecting one tower
  in isolation.

## v0 scope cuts

- File-level granularity. Package-level rollup (multiple `.go`
  files in one `package main` aggregating into one root locus)
  is a downstream pass.
- No cross-file flow inference. The "child loci = spawned
  goroutines" line in the Aperio-shape interpretation is
  template-level; we don't yet identify which goroutine target
  fn lives in which file (m100.5 / LSP work).
- No HTTP route → subject mapping yet (HandleFunc wiring
  detection is an extractor enrichment, not yet shipped).
- Recognition narrative is template-driven. A future round may
  add cross-file relationship descriptions.
