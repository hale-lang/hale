# Ferryman enrichment protocol (for agents)

> Captured 2026-05-10. Scope: the operational protocol an agent
> follows during the **enrichment** stage of ferryman's
> codebase-onboarding pipeline. This is the load-bearing
> middle step: ferryman's `skeleton` mode emits a yaml that is
> honest about syntax-layer uncertainty; the agent reads that
> yaml and the source it points to; the agent writes back the
> enrichment cells the renderer expects; `render` mode produces
> the partner-facing report.
>
> Designed for an LLM agent running inside a Claude Code (or
> equivalent) session with file-read access to the target
> codebase. The agent is the primary reader. A human can run
> the protocol by hand but it's not the design target.

## The pipeline

```
  ferryman skeleton <repo> <skel.yaml>     ← syntactic extraction
                ↓
  AGENT reads <skel.yaml> + source
  AGENT writes enrichment cells back into the yaml
                ↓
  ferryman render <enriched.yaml>          ← prose recognition report
```

Three honest claims about the shape:

- **The skeleton stage knows only syntax.** It can tell that
  `fmt.Errorf` is a selector call with receiver `fmt` and name
  `Errorf`. It cannot tell whether `fmt` is the Go standard
  library, a local variable, or a renamed import. That
  ambiguity is left as agent work, not papered over with a
  heuristic.
- **The renderer is pure presentation.** It reads the yaml and
  emits text. It never re-touches the source. Every quality
  improvement past skeleton's output happens in the yaml the
  agent writes.
- **All enrichment cells are optional.** If the agent fills
  nothing, the renderer still produces a usable (less polished)
  report. Enrichment is additive; partial enrichment is fine.

## What the agent receives

The skeleton yaml is a structured document with this top-level
shape (paths quoted; v0 yaml is block-style, no flow):

```yaml
schema_version: "1"
codebase:
  root: "<absolute repo path>"
  flavor: "go"             # only Go in v0
  module_name: "<go.mod name>"
  module_root: "<dir containing go.mod>"
binaries:
  - name: "cmd/foo"
    rel_path: "cmd/foo"
    entrypoint_file: "main.go"
    package: "main"
    files:
      - file: "main.go"
        package: "main"
        has_main: true
        has_init: false
        n_handlers: 0
        n_spawns: 0
        n_loops: 0
      # ... one entry per file in the binary's package
    outward_tower:
      root: "main"
      root_file: "main.go"
      nodes:
        - call: "Errorf"
          receiver: "fmt"
          site: "{method}"
          kind: "method"
          children: []
        - call: "panic"
          receiver: ""
          site: "{builtin}"
          kind: "builtin"
          children: []
        # ...
    inward_tower:
      files:
        - file: "main.go"
          imports:
            - path: "fmt"
              kind: "stdlib"
            - path: "github.com/spf13/cobra"
              kind: "external"
          # ...
  # ... one entry per discovered main()
```

### Reading the cells

| Path | What it means | Notes |
|---|---|---|
| `codebase.root` | Absolute path of the repo being onboarded | Use this for source-file reads; everything else is relative |
| `codebase.flavor` | Source language | Only `"go"` at v0 |
| `binaries[i].rel_path` | Binary's directory relative to root | Read source files at `<root>/<rel_path>/<file>` |
| `binaries[i].package` | Go package name of the binary | Usually `"main"` |
| `binaries[i].files[j].file` | File name relative to the binary's dir | The `package` field is the Go package, often same across files |
| `binaries[i].files[j].has_main` | Has a `func main()` | One file per binary will be true; others false |
| `binaries[i].files[j].has_init` | Has a `func init()` | Go-style init hook |
| `binaries[i].files[j].n_handlers` | Count of recognized HTTP handler registrations | E.g. `*.HandleFunc` calls |
| `binaries[i].files[j].n_spawns` | Count of `go ...` goroutine spawns | |
| `binaries[i].files[j].n_loops` | Count of `for { }`-style unbounded loops | |
| `binaries[i].outward_tower.nodes[k].call` | Callee name (bare) | `Errorf`, `panic`, `NewClient` |
| `binaries[i].outward_tower.nodes[k].receiver` | Selector receiver if a selector call | `fmt`, `client`, `secman.Instance` |
| `binaries[i].outward_tower.nodes[k].kind` | Syntactic classification | `internal`, `external`, `method`, `builtin` |
| `binaries[i].inward_tower.files[j].imports[k].path` | Import path | `fmt`, `github.com/foo/bar` |
| `binaries[i].inward_tower.files[j].imports[k].kind` | Import provenance | `stdlib`, `local`, `external` |

### The kind values, decoded

The skeleton stage emits these `kind` values for outward-tower
nodes; their meaning is precise:

- **`builtin`** — Go language builtin (`len`, `make`, `panic`,
  `append`, ...). Eighteen names in `std::lang::Lang.is_builtin`.
  Receiver is `""`. Render as a leaf; do not enrich.
- **`internal`** — bare callee with a matching `FN_DEF` in the
  same package. Cross-file fn-to-fn flow already resolved. The
  outward tower walks into the children.
- **`external`** — bare callee with no local `FN_DEF`, or a
  selector call with an uppercase-receiver (`secman.Instance`).
  Treat as an opaque leaf at v0; the agent can override if it
  recognizes the symbol.
- **`method`** — selector call with a lowercase receiver
  (`fmt.Errorf`). Ambiguous between *package call* (`fmt` =
  imported stdlib package) and *method call on a local var*
  (`client.Send`). **This is the primary agent decision point**
  — see "Resolving method ambiguity" below.

## What the agent writes

Every enrichment cell is optional. Each one improves the
rendered report; the renderer falls back when absent. Add
cells in-place in the yaml, preserving the existing structure.

### Per-file (under `binaries[i].files[j]`)

| Cell | Type | Purpose | Example |
|---|---|---|---|
| `classification` | scalar String | What the file *does* in the binary's operation | `"http transport"`, `"persistence"`, `"command dispatch"`, `"domain core"` |
| `contributes_to` | scalar String | Which locus / subject the file participates in | `"HTTP request handling"`, `"trade lifecycle"`, `"config loading"` |

Both fields appear inline in the file box of the rendered
report. Keep each to a short phrase (3-6 words); the renderer
doesn't reflow.

### Per-binary (under `binaries[i]`)

| Cell | Shape | Purpose |
|---|---|---|
| `summary` | scalar String | One-paragraph prose recognition of what this binary *does at runtime*. Reads as a paragraph in the report header. Aim for 2-4 sentences |
| `loci` | list | Cross-tower-agreement loci you've identified in the binary's source. One entry per locus |
| `unknowns` | list | Things you tried to resolve and couldn't, with why. Surfaces in the report as the next-stage work queue |

#### `loci` list items

Apply the **three-tower-agreement rule** from
`notes/onboarding-shape-rules.md` and `notes/aperio-types-vs-loci.md`:
a source-level node is a locus iff ≥2 of the three towers
(operational, harmonic, domain) point at it with coherent
roles.

| Field | Required | Purpose | Example |
|---|---|---|---|
| `name` | yes | Source-name of the entity | `"Server"`, `"OrderProcessor"`, `"sessionManager"` |
| `verdict` | yes | One of: `"locus"`, `"type"`, `"unknown"`. `"locus"` means flow; `"type"` means pure data; `"unknown"` means the agent looked but couldn't decide | `"locus"` |
| `agreement` | optional | How many of the three towers agreed (Int, 1-3). Lets the renderer show confidence | `"3"` |
| `shape` | optional | Which existing Aperio pattern this maps to (see `notes/agent-onboarding/aperio-styleguide.md`'s six-pattern catalog) | `"service"`, `"namespace"`, `"shape-type"`, `"spawned-child"` |
| `motion` | optional | Present-participle motion form per the shape rules. Use only for Agent / Entity nouns — leave absent for Shape nouns | `"serving"`, `"remembering"`, `"validating"` |
| `source_name` | optional | The original name in the source if different from `name` (e.g. when `name` is a renamed Aperio-shaped identifier) | `"HTTPServer"` |
| `prose` | optional | One-line domain reading of what the locus *means* | `"Holds the long-lived TCP listener and accepts connections"` |

#### `unknowns` list items

When you tried to resolve something and couldn't, log the
unknown rather than guessing. Honest unknowns are the agent's
output for downstream consumers (human reviewer, next agent
pass, the reader of the report).

| Field | Required | Purpose |
|---|---|---|
| `kind` | yes | What kind of thing you couldn't resolve. Suggested: `"package"` (couldn't tell if a lowercase-receiver is a package or local var), `"locus"` (couldn't classify), `"motion"` (couldn't derive a motion form), `"contributes_to"` (couldn't decide what a file does) |
| `target` | yes | The specific name / symbol / file involved |
| `file` | optional | File where the unresolved item appears |
| `why` | optional | One-sentence reason you couldn't resolve |
| `action` | optional | What a subsequent reader could do to resolve it (read another file, query LSP, ask the human) |

## The agent's workflow

A bounded loop, applied per binary:

1. **Read the binary block.** Note `rel_path`, the file list,
   the outward-tower roots, the import set.
2. **Pre-scan the source.** Open each file in `<root>/<rel_path>`
   (or wherever `package` lives). Skim — you're looking for
   types, interfaces, top-level fns, package boundaries. Don't
   exhaustively read leaf code yet.
3. **Resolve method ambiguity in the outward tower** (see next
   section). This eliminates the largest source of unknowns
   from the skeleton output.
4. **Identify candidate loci.** Per the three-tower rule:
   - Operational signals: things that hold long-running state
     (`for { select { ... } }`), spawn goroutines, register
     handlers, own listeners or pools.
   - Harmonic signals: things imported by many files in the
     same package; things that import many other files.
   - Domain signals: Agent-noun / Entity-noun names per the
     shape rules (`Server`, `Manager`, `Cache`, `Bus`, ...).
   - **≥2 of the three** = `verdict: "locus"`. Otherwise =
     `verdict: "type"` (shape noun) or `verdict: "unknown"`
     (couldn't decide).
5. **Write the cells.** Add the file `classification` /
   `contributes_to` for each file, the `loci` list, optionally
   the `summary` paragraph. Log honest `unknowns` for anything
   you couldn't justify.
6. **Verify with render.** Run `ferryman render <enriched.yaml>`
   on the binary subset. Look at the report. Iterate if
   anything looks wrong.

### Resolving method ambiguity

The single most common skeleton unknown is `kind: "method"` for
a selector call with a lowercase receiver — `fmt.Errorf`,
`client.Send`, `r.Body`. The skeleton stage deliberately
doesn't guess between *package call* (lowercase = imported pkg)
and *method call on a local var*.

Decision procedure for a single `kind: "method"` node:

1. **Check the file's `imports`** in `inward_tower.files`.
   If `receiver` appears as the leaf of any import path, it's a
   package call. (`fmt.Errorf` → imports contain `"fmt"` →
   package call.)
2. **If not in imports, scan the file for a local var
   declaration** with that name. (`client.Send` → grep for
   `client :=` or `client *http.Client` in the file. If
   found, it's a method on a local var.)
3. **If still ambiguous** (e.g. the var is declared in a
   different file in the same package): log an `unknowns`
   entry with `kind: "package"` and move on.

Once resolved, the agent doesn't currently mutate the node's
`kind` field (the skeleton schema is append-only for v0). The
distinction lands instead as part of the binary's `summary`
prose and the file's `classification`.

## Honest unknowns

Per `notes/onboarding-shape-rules.md`:

> The static tools optimize for **honest classification with
> rich context**, not for low unknown rates. A lower unknown
> rate at the cost of fabrication is *worse* than an honest
> unknown — the agent wastes time second-guessing a wrong
> "confident" answer.

The same applies to the agent. Three rules:

- **Don't invent motion forms for Shape nouns.** A `Request` /
  `User` / `Config` is data, not flow. No `motion:` field.
- **Don't classify a locus you didn't read.** If you only saw
  the type name in a tower row but not its methods or
  usage, mark `verdict: "unknown"` and add an `unknowns`
  entry explaining what you'd need to read.
- **Don't fill `summary` from the file list alone.** The
  summary is a domain reading of what the binary does at
  runtime. If the source doesn't make that legible in a
  pre-scan, leave it blank and log an `unknowns` entry.

The `unknowns` block surfaces prominently in the rendered
report under "Unknowns (agent-actionable)". A report with five
honest unknowns is more useful than the same report with five
plausible-sounding fabrications.

## A worked example

Skeleton input for one tiny binary:

```yaml
  - name: "cmd/healthcheck"
    rel_path: "cmd/healthcheck"
    entrypoint_file: "main.go"
    package: "main"
    files:
      - file: "main.go"
        package: "main"
        has_main: true
        has_init: false
        n_handlers: 1
        n_spawns: 0
        n_loops: 0
    outward_tower:
      root: "main"
      root_file: "main.go"
      nodes:
        - call: "HandleFunc"
          receiver: "http"
          site: "{method}"
          kind: "method"
          children: []
        - call: "ListenAndServe"
          receiver: "http"
          site: "{method}"
          kind: "method"
          children: []
        - call: "Fatal"
          receiver: "log"
          site: "{method}"
          kind: "method"
          children: []
    inward_tower:
      files:
        - file: "main.go"
          imports:
            - path: "log"
              kind: "stdlib"
            - path: "net/http"
              kind: "stdlib"
```

Agent reads `cmd/healthcheck/main.go`, sees a 15-line program
that registers a handler at `/health` and starts an HTTP server.
Method ambiguity resolves cleanly: `http`, `log` are imports →
package calls. No local-var receivers anywhere.

Enriched yaml the agent writes back:

```yaml
  - name: "cmd/healthcheck"
    rel_path: "cmd/healthcheck"
    entrypoint_file: "main.go"
    package: "main"
    summary: "A minimal HTTP health-check server. Registers a single handler at /health that returns 200 OK, then blocks in ListenAndServe on :8080. No goroutines, no shared state — the entire program is the listener and one handler."
    files:
      - file: "main.go"
        package: "main"
        has_main: true
        has_init: false
        n_handlers: 1
        n_spawns: 0
        n_loops: 0
        classification: "http server"
        contributes_to: "health-check endpoint"
    loci:
      - name: "healthHandler"
        verdict: "locus"
        agreement: "2"
        shape: "subscriber"
        motion: "handling"
        prose: "Single-route HTTP handler; subscribes to GET /health"
    outward_tower:
      # ... unchanged from skeleton
    inward_tower:
      # ... unchanged from skeleton
```

What rendered output gains over the skeleton-only run:

- The binary block leads with the `summary` paragraph.
- The one file gets `classification: http server` and
  `contributes_to: health-check endpoint` inline.
- A "Loci (cross-tower agreement)" section shows
  `healthHandler [locus]  agreement=2/3` with `shape: subscriber`
  and `motion: handling`.

What's deliberately absent:

- No locus for "main" itself — main is the lifecycle, not a
  separately-recognized locus.
- No `motion:` on the file boxes — files don't have motion forms.
- No `unknowns:` block — everything was resolvable. (If
  `http.HandleFunc` had been ambiguous between net/http and a
  user-defined http variable, an entry would have logged that.)

## Failure modes

Three patterns to recognize and back away from:

1. **The agent enriches one binary out of N and the rest stay
   skeleton-only.** Acceptable for a partial run. The
   rendered report shows enriched binaries followed by
   bare-skeleton binaries; both are usable. Don't pad the
   un-enriched ones with low-confidence guesses just to make
   the report look uniform.

2. **The agent invents a locus that doesn't pass the
   three-tower rule.** Recognize when only one tower is
   pointing at something — usually a name in the domain
   tower (because every PascalCase identifier has a name).
   One-tower presence is structural artifact, not its own
   locus. `verdict: "type"` or no entry at all.

3. **The agent over-fills `unknowns` with non-actionable
   entries.** An `unknowns` entry should describe something a
   downstream reader could resolve — "needs LSP", "needs
   reading file X", "depends on runtime config". An entry
   that says "couldn't tell what this does, sorry" without an
   `action:` field is noise.

## Cross-references

- `notes/agent-onboarding/aperio-styleguide.md` — the six-pattern
  catalog the `shape` field maps to.
- `notes/onboarding-shape-rules.md` — the Agent / Entity / Shape
  noun categories and the `motion:` derivation rules.
- `notes/aperio-types-vs-loci.md` — the source axiom and the
  three-tower-agreement rule that defines locus identity.
- `notes/codebase-onboarder-progress.md` — overall project state
  and where ferryman sits in the arc.
- `apps/ferryman/main.ap` — the renderer. The set of optional
  cells the agent can write is exactly the set the renderer's
  `__render_*_yaml` fns check via `has(...)`.

## Scope and known limits

The protocol is honest about what it can and can't yet do:

- **v0 covers Go.** Other languages will need their own
  flavor's `kind` semantics resolved before this protocol
  applies.
- **Schema is append-only at v0.** Agents add cells; they
  don't mutate existing skeleton cells. If a `kind: "method"`
  node is actually a package call, the agent records the
  reading in `summary` and `loci`, not by rewriting the node's
  kind.
- **Large codebases hit a Reader scaling limit.** Per
  `notes/aperio-friction.md` (the 2026-05-10
  `reader-list_item-quadratic-concat` entry), ferryman renders
  ~1-3 binaries reliably; full multi-binary monorepos wait on
  a language-level runtime fix. The protocol applies the same
  way at all scales; only the `render` step has the ceiling.
- **No LSP yet.** Semantic enrichment via gopls / rust-analyzer
  is a future milestone. The agent fills the gap by reading
  source directly; this is slower but produces honest results.
