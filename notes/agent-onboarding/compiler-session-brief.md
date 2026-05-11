# The Wandwork

> Orientation brief for a mainline compiler session in Aperio.
>
> *Aperio is the spell of spellcasting. The compiler is the wand.
> An app-dev session casts spells; this session shapes the wand.
> You are picking up tools the previous session set down. Read
> first; the substrate has commitments you can't see by reading
> any one file in isolation.*

## Read this first

You have **zero training data** on Aperio or its compiler. The
language was written recently; no model has seen it during
pretraining. Your priors about how a compiler "should" be
structured are useful only as starting bias — every concrete
choice in this codebase follows from the framework, not from
generic compiler-construction lore. Read first, then write.

The framework has its own design literature you must read before
you can make a load-bearing change:

- **The Design** — `~/notes/alpha-conjecture/theory/the_design.md`.
  The substrate-invariant recursion operator. Aperio is The
  Design operationalized at the software substrate; lotus is
  the substrate name; every commitment in `spec/` traces to
  it. **Required.**
- **`spec/design-rationale.md`** — for each grammar construct,
  what the framework commits to, what the syntax commits to,
  what was rejected and why. The F.1–F.18 numbered commitments
  are load-bearing across the codebase. **Required.**
- **`notes/aperio-types-vs-loci.md`** — the foundational axiom
  (types are for shapes; loci are for flow). The codebase has
  no third category. **Required.**
- **`spec/{semantics,types,memory,runtime,tokens,stdlib,testing,precedence}.md`**
  — the rest of the spec. Read on demand.
- **`notes/agent-onboarding/aperio-styleguide.md`** — the
  styleguide app-dev sessions follow. The compiler should
  emit code (errors, generated stdlib mangled names) that
  *agrees* with the styleguide.

## What you are doing here

You are the **mainline compiler session.** You own the
substrate that app-dev sessions cast against. Your territory:

- `crates/aperio-{cli,syntax,types,codegen,runtime,ts-shim}/`
  — the Rust compiler, typechecker, codegen, interpreter, and
  CLI.
- `crates/aperio-codegen/runtime/` — the C runtime
  (`lotus_arena.c`, `lotus_treesitter.rs`) and the bundled
  stdlib seed (`runtime/stdlib/*.ap`).
- `spec/` — the language specification.
- `docs/src/{reference,std,book,grimoire,quickstart}/` — the
  user-facing documentation.
- `examples/` — the canonical small programs that exercise
  each language primitive.
- `notes/` — design notes, friction logs, open questions.

You **do not modify** `apps/`. That is app-dev session
territory. If an app's `.ap` source has a bug, *that's the
app's problem* unless it surfaces a compiler defect — in
which case the fix lands in `crates/`, not in the app.

## The Aperio / lotus naming split

This is the discipline rule everyone trips over once:

- **Aperio** = the language proper. The thing users write `.ap`
  files in. Type representations in `aperio-types::Ty` and
  `aperio-codegen::CodegenTy`. CLI tool name. Crate prefix
  (`aperio-*`). Doc framing.
- **lotus** = the runtime / substrate concept. The thing that
  hosts a running Aperio program. C-runtime symbols stay
  `lotus_*` — `lotus_arena_create`, `lotus_bus_dispatch`,
  `lotus_mailbox_post`. The repo directory is `lotus-lang/`
  for historical reasons; do not "fix" it.

The styleguide also uses **"namespace lotus"** as the *pattern*
name for `locus { params { } fn ... }` — empty/config-only params,
methods only, used as a vocabulary container. This is correct
substrate-pattern usage; not a typo for "namespace Aperio."

When in doubt: if it's user-visible language surface, it's
Aperio. If it's runtime substrate or substrate-pattern, it's
lotus.

## The minimum mental model

**One sentence.** Aperio is a recursion-tower of loci; the
compiler maps `.ap` source through parse → typecheck → IR
→ LLVM → object → linked executable, with the lotus C runtime
linked into every binary.

**Everything is a locus.** Apps, namespaces of helpers,
long-running services, bus subscribers, per-request handlers,
streams, listeners — all loci. The recursion bottoms at
primitive operations. Inside any locus, behavior is itself
a locus tower one layer down. This is the framework's
*recursive principle* and it shapes the language: there is no
"module," "class," "package," or "namespace" keyword — there
is only `locus`.

**Form is invariant; parameters are substrate-local.** The
Design's central commitment, applied to compiler design: the
recursion-tower form (locus → coordinatees → ...) is invariant;
the per-substrate parameters (rich/chunked/recognition projection
class; cooperative/pinned schedule class; (B, c, σ, φ) capacity
tuple) populate the form. When adding a feature, ask: *which
side am I touching, form or parameters?* New form is rare;
new parameters are common.

**Pipeline shape.** Source → tokens (`aperio-syntax::lexer`)
→ AST (`::parser`) → resolved types (`aperio-types::resolve` →
`::check`) → either:

- Interpreter path (`aperio-runtime::eval`): tree-walking
  evaluator over the typed AST. Used for `aperio run`.
- Codegen path (`aperio-codegen::codegen`): AST → LLVM IR
  via inkwell, → object via LLVM, → linked binary via clang.
  Used for `aperio build`. The C runtime and stdlib seed are
  bundled at codegen-time (`include_str!`) and dropped next
  to the object file at compile time.

**Two type representations.** `aperio-types::Ty` is the
typechecker's view; `aperio-codegen::CodegenTy` is the codegen
IR-level view (smaller, includes layout/repr info). They
mirror each other at the primitive level; codegen has its own
because the codegen layer needs `LocusRef(name)` /
`TypeRef(name)` / `Array(elem, n)` shapes that the typechecker
doesn't.

## The friction-log contract

This is your primary input. Two layers:

- `notes/aperio-friction.md` — global friction (cross-app,
  session-level surprises).
- `apps/<name>/FRICTION.md` — per-app friction. Each entry
  describes a real moment where the language got in the way
  of writing what should be a correct program.

**Read both before picking a milestone.** A friction entry
that recurs across multiple apps is the loudest signal; a
single isolated entry waits until the pattern repeats.

The format is dated, append-only, four-line: Tried / Hit /
Workaround / Why-it-matters. Don't reformat earlier entries.

What is **not** a milestone driver:
- A general feature wish disconnected from concrete program
  resistance ("Aperio should have generics" — yes, we know;
  log when generics' absence blocked a *specific* program).
- A stylistic preference.
- A bug in app-dev code that the compiler caught (that's the
  compiler doing its job).

**Resolve, don't grow.** When a friction entry's underlying
gap is filled, mark it `[FIXED]` in-place (see the m49
`cross-locus-return-deep-copy` entry for shape) — but only
when the friction is genuinely gone, not when a workaround
was added.

## Spec-vs-implementation discipline

The spec is forward content; the implementation fills in
incrementally. **Drift in either direction is bad.**

When you add a feature:
1. Land the implementation (parser / typechecker / codegen /
   runtime, plus tests).
2. Add the F-numbered commitment to `spec/design-rationale.md`
   if it's a new design commitment, OR update the relevant
   `spec/<topic>.md` doc if it's filling in an existing
   commitment.
3. Update `docs/src/reference/<topic>.md` if it changes the
   user-visible language surface.
4. Resolve the corresponding `notes/open-questions.md` entry
   if you closed one.

When you remove a feature:
1. Both sides delete. Don't leave the spec asserting things
   the impl doesn't do (or vice versa).

When you find drift:
1. Decide which side is correct (usually the spec; sometimes
   the implementation has gone past the spec and the spec
   needs catch-up).
2. Update the wrong side. Note it in the commit message.

The spec is **not** an aspirational document. If something is
in `spec/design-rationale.md` as a numbered F-commitment, it is
a load-bearing claim about the language; downstream code (incl.
the styleguide and the app-dev brief) cites it.

## The milestone idiom

Features land as numbered milestones (m20, m49, m95, ...). Each
gets:

- A commit message starting with `mNN: <one-line summary>`.
- An entry in `spec/stdlib.md` (if it's a stdlib milestone) or
  `spec/runtime.md` / `spec/design-rationale.md` (if it's
  language / runtime).
- Tests in `crates/aperio-codegen/tests/<name>.rs` (codegen-
  level) or the relevant crate's `tests/` for typechecker /
  parser / runtime work.
- Often a corresponding `examples/<NN>-<name>/` showing the
  feature in isolation.

`git log --oneline | head -50` is the fastest way to orient on
recent milestone work. Look for which mNNs were the most recent
and what they touched (`git show <hash> --stat`).

## Crate map

| Crate | Owns |
|---|---|
| `aperio-syntax` | Lexer, parser, AST. Hand-written recursive descent (no parser generator). |
| `aperio-types` | Symbol resolution, type checking. `Ty` is the canonical type rep. |
| `aperio-runtime` | Tree-walking interpreter (`eval`), built-in fns, bus, env. v0 interpreter path. |
| `aperio-codegen` | AST → LLVM IR via inkwell. `CodegenTy` is the codegen-internal type rep. Owns `runtime/lotus_arena.c` (C runtime) and `runtime/stdlib/*.ap` (stdlib seed). |
| `aperio-cli` | Top-level `aperio run` / `aperio build` command. |
| `aperio-ts-shim` | tree-sitter staticlib bridge (m96+). Backs `std::ts::*`. |

`aperio-codegen` is the largest by ~1 OOM. It is also the
most-edited per milestone. `codegen.rs` is one ~18kloc file by
design — the codegen pipeline lives in one place; splitting it
would fragment locality across compilation phases. Don't split
it without a stronger reason than file-size.

## Stdlib seed shape conventions

The `std::*` surface ships in two shapes. **The shape is
determined by what the surface bridges to**, not by author
preference:

### Path-call dispatch (bare-fn surface)

Used when the surface bridges to **extern code** (libc, the C
runtime, the tree-sitter Rust shim). The codegen routes the
`std::path::name(...)` call directly to a libcall or a runtime-
provided extern fn. No Aperio source backs the function body.

Examples:

- `std::env::*` → bridges to argc/argv stash via `lotus_env_*`
- `std::time::*` → bridges to `clock_nanosleep` /
  `clock_gettime` (CLOCK_MONOTONIC)
- `std::process::{exit, pid}` → bridges to libc `exit` /
  `getpid`
- `std::str::{parse_int, can_parse_int, index_of}` → bridges
  to `strtoll` and inline pointer arithmetic
- `std::ts::*` → bridges to extern Rust functions in the
  `aperio-ts-shim` staticlib
- `std::io::fs::*` → bridges to `fopen` / `fread` / `stat` /
  `opendir`
- `std::text::md_to_html` → bridges to a free fn in `text.ap`
  whose body is pure Aperio but whose call shape is
  path-style for historical reasons (Phase 5 capstone
  predates the namespace-lotus pattern)

### Namespace lotus (`std::namespace::Name { }.method(...)`)

Used when the surface is **authored in Aperio**, in the stdlib
seed (`crates/aperio-codegen/runtime/stdlib/*.ap`). Lives as
a `locus __Std<Domain><Name>` with empty/config-only `params`
and self-composing methods. The path-rename table in
`codegen.rs` (`STDLIB_PATH_RENAMES`) maps the user-facing
path to the mangled internal name.

Examples (each is a namespace lotus per `std::lang::Morpheme`'s
pattern):

- `std::cli::Resolver`, `std::iter::Lines`, `std::json::Builder`,
  `std::lang::{Lang,Morpheme}`, `std::name::Convention`,
  `std::source::Walk`, `std::tagged::Accumulator`,
  `std::yaml::{Builder,Reader}`, `std::log::*`, `std::http::*`,
  `std::io::tcp::*`, `std::text::Sink`

### When adding a new stdlib surface

Ask: *does this surface need to call extern C/Rust to function,
or can it be implemented in Aperio?*

- **Extern needed** → path-call dispatch. Add a route in the
  appropriate `lower_std_*` block in `codegen.rs`. No `.ap`
  source needed (or just a thin extern signature in `core.ap`
  if the surface composes other stdlib helpers).
- **Pure Aperio** → namespace lotus. Add a new `*.ap` file in
  `runtime/stdlib/`, declare the locus following the styleguide
  (header rationale, namespace-lotus pattern, snake_case
  methods). Add an entry to `STDLIB_PATH_RENAMES` in
  `codegen.rs`. Add the file to `STDLIB_AP_SOURCE` concat.
  Add an entry to `regen-std-source.py`'s PAGE_MAP /
  TITLE_MAP. Run the regen script. Add the page to
  `docs/src/SUMMARY.md`.

The path-call form is **not** a deprecated form to migrate
away from; it's the right shape for substrate bridges. The
namespace-lotus form is the right shape for composable Aperio.
The two coexist permanently.

**Special case: hand-written reference docs.** A handful of
`docs/src/std/*.md` pages are hand-written prose because their
surface is path-call dispatch and has no `.ap` source to
include. Listed in `regen-std-source.py`'s docstring; left
alone by the regen script. When you migrate a hand-written
doc to a `.ap`-backed page, update the regen script's PAGE_MAP.

## Known design debt

Real shape-debt the v0 surface carries that future work will
relieve. Listed here so a fresh compiler session doesn't
re-rediscover the antipatterns or attempt premature fixes.

### Sink-as-tagged-locus (`std::text::Sink`)

`__StdTextSink` in `crates/aperio-codegen/runtime/stdlib/text.ap`
uses `if self.dest == "string" { ... } else { ... }` to branch
between an in-memory buffer and stdout streaming. Adding a
third destination (e.g. file) edits every method; the type
system can't see which destinations exist; unused params
(`buf` in stdout mode) sit in every instance.

- **Why it persists.** No interface / multi-impl-per-contract
  mechanism in v0. The friction-log entry
  (`notes/aperio-friction.md` 2026-05-10 sink-as-tagged-locus)
  documents the workaround.
- **What unblocks the fix.** F.14 (three-way interface; per-
  projection-class translation impls) lifted from typing rule
  to surface syntax. With multi-impl-per-contract,
  `StdoutSink` / `StringSink` / `FileSink` become separate
  loci with one shared `Sink` contract; the inner dispatch
  disappears.
- **Don't preempt.** A migration ahead of the interface
  mechanism would just rewrite the antipattern in a different
  shape. Wait for F.14 to land.

### Single-file-app-monolith

App-dev sessions can't decompose an app into multiple `.ap`
files in the same directory; each `.ap` is its own
translation unit. The friction log
(`apps/ferryman/FRICTION.md` 2026-05-10 single-file-app-monolith)
captured this.

- **Why it persists.** No per-directory package model in v0.
  The grammar reserves `module` as a keyword but doesn't
  resolve module-loading semantics
  (`notes/open-questions.md` Q18).
- **What unblocks the fix.** Module loading semantics +
  `aperio build apps/<name>/` treating the directory as one
  seed (the stdlib seed already cheats this way via
  `concat!(include_str!(...))`).
- **Don't preempt.** The compiler-internal cheat for stdlib
  is fine; don't surface it to user code without a real
  module system design.

### Two-form `std::*` surface

Documented in "Stdlib seed shape conventions" above as a
permanent feature, but worth reiterating in the debt list:
the user-visible split between path-call and namespace-lotus
surfaces *can* be confusing. Mitigation lives in docs (the
generated reference pages document the actual surface) and in
the styleguide's pattern-catalog grounding. Not debt to be
paid down; debt to be communicated clearly.

## Counter-hallucination list (compiler-author flavor)

Things you will reach for that **do not apply** here.

| You will reach for | It doesn't apply because |
|---|---|
| Adding a stdlib helper "for completeness" | The stdlib relieves real friction. Speculative additions create dead surface area. Wait for an entry in the friction log. |
| Splitting a long file "for cleanliness" | `codegen.rs` is intentionally one file. Other crates are already small. If you genuinely need a new module, justify it in the commit. |
| A trait system because "Rust does it that way" | Aperio doesn't have traits in v0 (reserved keyword, no semantics). Don't infer the language from compiler-internal Rust patterns. |
| Adding `Option<T>` / `Result<T, E>` because they're missing | Same — the language uses sentinel values + sibling booleans. v0 doesn't have generics; sum-typed payloads are deferred. |
| Renaming `lotus_*` symbols to `aperio_*` | The C-runtime symbol prefix is `lotus_*` by design. Don't "fix" it. |
| Generalizing a feature "for future flexibility" | Don't. Aperio's substrate is small on purpose. New form is rare. |
| Adding a feature flag for staged rollout | We have one branch and one binary; staged rollouts are deferred. Land the change or don't. |
| Adding error-recovery cases the compiler can't naturally hit | Don't. Trust the framework's invariants. |
| Reaching for `unsafe` Rust | The codegen is `unsafe`-free except where inkwell / LLVM bindings demand. Don't add new `unsafe` blocks without a load-bearing reason. |
| "Modernizing" comments by removing milestone refs | The `mNN` commit refs in comments are how you trace why a chunk of code exists. Keep them. |

## Verification protocol

After any compiler change:

```
cargo build                       # whole workspace
cargo test                        # all 343+ tests
```

For codegen-level changes, run a representative app to confirm
end-to-end:

```
target/debug/aperio build apps/onboard/main.ap
./main apps/operational-graph/fixture go
```

Or one of the small `examples/*/main.ap` binaries.

For changes to the typechecker or AST: run the
`crates/aperio-types/tests/` suite specifically. For lexer/
parser changes: `crates/aperio-syntax/tests/`.

The doc-tests fail to link with `libLLVM.so.22.1-rust-1.95.0-stable`
on this box (env issue, not a regression). Ignore unless you
have changed something that should affect doc-test linkage.

## The recursive principle, applied to compiler internals

The framework's "everything is a locus" applies to compiler
design too, with a translation:

- A language **type** has shape (fields, layout) — no flow.
  In Rust this is a `struct`. `Ty`, `CodegenTy`, `Span`, `Token`
  are shapes.
- A language **locus** has flow (lifecycle, dispatch, contracts).
  In Rust this is a `struct` with methods that maintain
  invariants across calls. `Cx` (the codegen context),
  `Lexer`, `Parser`, the typechecker `Resolver` are loci.

When adding a new compiler subsystem, ask: *does this thing
have flow, or is it a record?* Records are simpler — prefer
them. Loci are for things that genuinely accumulate state
across calls.

Same for the language layer: when growing the stdlib seed, the
styleguide's pattern catalog is the recipe. A new namespace
seed should mirror an existing namespace lotus exactly (same
shape, different domain) — if you find yourself inventing a
new shape, stop and reconsider.

## Hard guardrails

- **Do not edit `apps/`.** That is app-dev session territory.
  If you need to verify a compiler change against an app, run
  the app's binary; don't rewrite its `.ap` source to dodge
  a compiler limitation.
- **Do not skip pre-commit hooks** (`--no-verify`) unless the
  user explicitly asks.
- **Do not break existing tests.** If a test starts failing
  because the spec genuinely changed, update the test in the
  same commit and explain in the message. Never delete a test
  to make it pass.
- **Do not invent grammar or stdlib paths.** Spec the change
  first; implement second.
- **Do not generate `.md` files for tracking session state.**
  Use the conversation, the friction log, and `notes/open-
  questions.md`. The repo is not a personal scratch space.
- **Do not commit changes the user didn't ask for** — the
  user owns the commit cadence; you propose, they approve.

## Sister documents

- `notes/agent-onboarding/app-dev-brief.md` — the brief for
  the *other* kind of session. If a friction log entry
  doesn't make sense, read this brief to understand what the
  app-dev was trying to do.
- `notes/agent-onboarding/aperio-styleguide.md` — what
  idiomatic Aperio looks like in user code. The compiler's
  generated code (default lifecycle bodies, stdlib seed,
  error messages referencing user constructs) should *agree*
  with this guide.

## When you are stuck

1. Re-read the relevant `spec/<topic>.md` and the F-numbered
   commitments it cross-refs. Most stuck-points come from
   missing a load-bearing constraint.
2. `git log --oneline -- crates/<area>/` to see what recent
   work touched the area. The commit messages explain *why*
   in ways the code can't.
3. Read the closest neighbor in `examples/` and the closest
   user in `apps/` to see what behavior the change is
   accountable to.
4. If the question is "should I do X or Y," log it in
   `notes/open-questions.md` and ask the user. Don't guess
   on load-bearing direction.
