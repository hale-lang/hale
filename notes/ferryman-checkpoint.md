# Ferryman checkpoint — 2026-05-10

> Snapshot for resuming the partner-ready codebase-onboarder
> arc in a fresh session. The goal is an internal tool that
> works on the user's existing all-Go codebase (`~/code/grease`,
> a multi-binary Cobra-CLI monorepo). This doc captures where
> the work stands, what shipped this session, and — more
> importantly — the architectural realization at the end that
> should inform the next session's direction.

## The target

A binary the user can hand to their partners that runs against
`~/code/grease` (or any Go repo) and produces a polished
recognition report — one section per binary entrypoint, with
operational/harmonic/domain towers, outward + inward call
trees, and agent-actionable unknowns.

**Not** a public release. **Not** multi-language at v0. **Not**
a transpiler. The middle-step perception product.

## What shipped this session

Three commits, in order:

1. **`11ed600` — `apps/ferryman/main.ap`.** New app, 1411
   lines, copy of `apps/onboard/main.ap` plus recursive
   entrypoint discovery. Walks repo root via `__find_main_dirs`,
   finds every `main.go` (bounded depth 6, vendor / `.git` /
   build / dist / node_modules / dot-prefixed dirs skipped),
   runs the full recognition pipeline per discovered binary.
   Repo-root `.aperio-overrides` shared across all binaries.
   Validated against `~/code/grease`: 36 binaries discovered,
   11,989 lines of report, exits clean. Single-binary fallback
   preserved (point at a dir that has `main.go` directly).
   **Onboard kept alive** — tests stay green; partners migrate
   when ready.

2. **`4848dac` — Skip Go builtins.** Added
   `Morpheme.is_builtin(name)` recognizing Go's 18 language
   builtins (`len`, `make`, `panic`, `append`, ...). Ferryman's
   `__walk_outward` short-circuits motion rewriting for builtin
   callees, rendering them as `{builtin}` leaves with no
   `<unknown:X>` motion suffix. Cleared 372 noise rows in the
   grease report (`<unknown:len>` 110→0, `<unknown:panic>` 29→0,
   `<unknown:append>` 79→0, `<unknown:make>` 154→0).

3. **`f0b339d` — Acronym fix in `split_camel`.** Changed the
   morpheme splitter so consecutive uppercase letters stay
   together (`HTTPClient` → `HTTP\nClient` instead of
   `H\nT\nT\nP\nClient`). Cleared 1100+ single-letter
   morpheme noise rows. Acronyms now surface as recognizable
   morphemes (HTTP, JSON, UTC, API, DSN) that an agent can
   resolve via `.aperio-overrides` instead of fighting
   alphabet-soup output.

All 334 workspace tests stay green across the three commits.

## The realization (this is the important part)

Late in the session the user asked: **"why are we doing this?
isn't this what tree-sitter / AST / LSP is for?"** That question
landed.

The honest read: most of session 2 and 3's work was
*information-loss recovery*. The codebase-onboarder's
`extract_call_name` flattens `selector_expression(operand=fmt,
field=Errorf)` to the bare String `Errorf`, throwing away the
receiver context. Then downstream we hand-roll heuristics
(builtin tables, acronym splitters, morpheme rewriters) to
guess back what the AST already knew.

The three layers we're actually working across:

| Layer | What it answers | Right tool |
|-------|----------------|------------|
| **Syntactic structure** | "What kind of node? What's the receiver? What's the name?" | tree-sitter (we have this) |
| **Symbol semantics** | "Which file defines this fn? Is `fmt.Errorf` stdlib?" | LSP / gopls (deferred per design) |
| **Domain naming** | "What does `Cache` mean? Is `OrderProcessor` agent-noun-shaped?" | morpheme rewriter (correctly placed) |

**The morpheme rewriter is being applied too widely.** Motion
forms (`Cache → remembering`) are useful for noun-shaped
*type* names where the action is implied. They're noise for
verb-prefixed *call* names (`New*`, `Get*`, `Set*`, `Create*`,
`Build*`, `Make*`) where the action is already in the name.

Look at this row from the current grease output:

```
├─ NewHTTPClient  {external}  · <unknown:New>-HTTP-<unknown:Client>
```

Why does it have a motion form at all? `NewHTTPClient` is a
verb already — "construct a new HTTP client". The motion suffix
adds nothing. The same row would read cleaner as:

```
├─ NewHTTPClient  {external}
```

## What the next session should do (instead of more heuristics)

In priority order:

### 1. Preserve receiver context in `extract_call_name`

Currently `std::lang::Lang.extract_call_name(node)` returns the
bare RHS for `selector_expression` callees, losing the package
prefix. Change it to return `<receiver>.<name>` for selector
calls and bare `<name>` for direct calls.

Then the ferryman renderer can branch:

- **Bare name** → user fn → FN_DEF lookup (current path).
- **Lowercase-receiver** (`fmt.Errorf`) → likely package call →
  render `{stdlib}` or `{pkg-name}` leaf, no motion.
- **Uppercase-receiver** (`logger.Info`) → method on local var →
  `{method}` leaf, no motion (LSP would resolve; not at v0).

This alone eliminates most remaining `<unknown:>` noise without
hand-rolled tables. Pure AST → renderer.

Likely touch:
- `crates/aperio-codegen/runtime/stdlib/lang.ap` —
  `extract_call_name` definition.
- `apps/ferryman/main.ap` — `__walk_outward` branching on
  receiver shape.
- Onboard's `__unified_walk` emits CALL rows — same site.
  Tests in `crates/aperio-codegen/tests/onboard_entrypoint.rs`
  may assert on bare-name CALL rows; update or migrate.

### 2. Stop motion-rewriting verb-prefixed call names

If the callee matches a verb-prefix pattern (`^(New|Get|Set|
Build|Create|Make|Open|Close|Start|Stop|Init|Run|Do|Try)[A-Z]`),
render without motion suffix. The verb is already in the name.

Likely touch:
- `crates/aperio-codegen/runtime/stdlib/lang.ap` — add
  `Morpheme.has_verb_prefix(name) -> Bool`.
- `apps/ferryman/main.ap` — `__walk_outward` skips
  `name_to_motion` when `r.has_verb_prefix(callee)`.

### 3. Reserve motion forms for noun-shaped type names

Per the original design — motion forms surface domain
vocabulary on TYPE names, not on call sites. The domain tower
is the right place; the outward tower mostly isn't.

This is mostly the consequence of (1) + (2). No new code, just
fewer call sites for `name_to_motion`.

### 4. Then assess what's left

After (1) + (2), re-run ferryman against grease and count the
remaining `<unknown:>` rows. The ones that remain are either:

- Real domain vocabulary the seed should resolve via
  `.aperio-overrides` (write the file for grease).
- Or genuinely unresolvable without LSP — note them and move on.

## What's still on the partner-ready checklist

| # | Item | Notes |
|---|------|-------|
| 1 | Multi-binary discovery | ✅ shipped (`11ed600`) |
| 2 | Skip Go builtins | ✅ shipped (`4848dac`) |
| 3 | Acronym keep-together | ✅ shipped (`f0b339d`) |
| 4 | Preserve receiver context in calls | **next session priority** |
| 5 | Stop motion on verb-prefixed calls | **next session priority** |
| 6 | Cobra `cobra.Command` extraction | open — biggest remaining quality cap; tree-sitter recognizer in Lang |
| 7 | Selector-call resolution to file (`pkg.Func` → file path) | open — wants LSP or cross-file symbol index; deferred per design |
| 8 | Seed `.aperio-overrides` for grease vocab | open — zero-coding, ~30 min after (4) lands |
| 9 | Getting-started one-pager | open — zero-coding |
| 10 | Roll graph apps + tower-join into ferryman | open — structural cleanup, no user-visible delta |

## Repo structure pointers

For the fresh session — these are the load-bearing files:

- **`apps/ferryman/main.ap`** — the partner-facing app.
  `__drive` orchestrates per-binary; `__drive_one_binary`
  renders one. The outward-tower renderer
  (`__walk_outward`, `__render_call_row`) is where the next
  session's work concentrates.
- **`apps/onboard/main.ap`** — predecessor, still alive,
  tests still attached. Will retire after ferryman absorbs
  its test surface.
- **`crates/aperio-codegen/runtime/stdlib/lang.ap`** — `Lang`
  locus has the AST classifiers; `Morpheme` has the domain
  rewriter. Both will see edits in (1)+(2).
- **`crates/aperio-codegen/src/codegen.rs`** —
  `STDLIB_AP_SOURCE` and `STDLIB_PATH_RENAMES` are the
  stdlib registration sites.
- **`notes/codebase-onboarder-progress.md`** — cross-session
  journal. Worth updating with ferryman milestone before
  starting the next.
- **`notes/codebase-onboarding-design.md`** — primary plan.
- **`notes/onboarding-shape-rules.md`** — the agent-driven
  model that motivates everything.
- **`spec/styleguide.md`** — the consolidated normative
  styleguide. The "rolling the design" discipline lives there.

## How to run ferryman

```
cargo build --release -p aperio-cli                                # builds the toolchain
target/release/aperio build apps/ferryman/main.ap                  # builds ferryman binary
apps/ferryman/main /path/to/repo                                   # runs against a repo root
apps/ferryman/main /path/to/repo/cmd/api                           # single-binary fallback
```

For grease specifically:

```
apps/ferryman/main ~/code/grease > /tmp/ferry_grease.out 2>&1
wc -l /tmp/ferry_grease.out          # ~12k lines, 36 binaries
grep -c '<unknown:' /tmp/ferry_grease.out  # count remaining noise
```

## Open design questions parked

These came up during the session and weren't resolved:

1. **Should call rows ever have motion forms?** Stronger
   version of priority (2): drop them entirely, not just for
   verb-prefixed names. The outward tower's job is to show
   what the program does at runtime; the name itself
   suffices.

2. **Should `Morpheme.is_builtin` live in `Morpheme` or `Lang`?**
   It's flavor-specific language metadata, not domain
   vocabulary. The fit is closer to Lang. Consider moving in
   a follow-up cleanup.

3. **What's the cleanest API for receiver-preserving call
   extraction?** Options:
   - `extract_call_name` returns "fmt.Errorf" (one String,
     callers split on dot).
   - Add `extract_call_receiver` separately, callers compose.
   - Return a small tagged-row format like `RECV:fmt|NAME:Errorf`.
   Pick before implementing (1).

4. **When does onboard retire?** After ferryman's tests cover
   the same surface, or after one partner-validated run? The
   tests give regression confidence; the partner gives
   product validation. Probably both.

## Skim list for the fresh session

To pick this up cold, in this order:

1. Read this checkpoint (`notes/ferryman-checkpoint.md`).
2. Skim `spec/styleguide.md` for patterns (the six-pattern
   catalog + "Rolling the design").
3. `git log -10` — see the three session commits in context.
4. `apps/ferryman/main.ap` — the file the next work touches.
5. `crates/aperio-codegen/runtime/stdlib/lang.ap` — find
   `extract_call_name` (the function priority (1) modifies).
6. Run ferryman against grease for current-state observation
   before making changes.

Then start on priority (1).
