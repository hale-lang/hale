# Brief: working on the Aperio compiler

You're an agent (or human) working on the compiler / runtime
itself. This brief tells you the architecture, the invariants
that constrain how the language evolves, and the discipline
that's expected of changes.

## The pipeline

```
.ap source
  → aperio-syntax::lexer    (tokens)
  → aperio-syntax::parser   (AST)
  → aperio-types::resolve   (top scope)
  → aperio-types::check     (typechecked Bundle)
  → either:
      aperio-runtime::eval        (tree-walk interpreter, for `aperio run`)
      aperio-codegen::codegen     (LLVM IR via inkwell → object → linked binary, for `aperio build`)
```

The C runtime (`crates/aperio-codegen/runtime/lotus_arena.c`)
and the stdlib seed (`crates/aperio-codegen/runtime/stdlib/`)
are bundled into the codegen crate via `include_str!` and
`include_bytes!`; the resulting binary needs no separate
runtime install.

## Crate map

- **`aperio-syntax/`** — lexer, parser, AST, diagnostic types,
  and the post-parse desugar passes (e.g.
  `desugar::desugar_topics`, `desugar::desugar_intra_locus_topics`).
  Hand-written recursive-descent parser; no parser-generator
  dependency.
- **`aperio-types/`** — symbol resolution (`resolve.rs`) and
  type checking (`check.rs`). `Ty` is the canonical type
  representation here.
- **`aperio-runtime/`** — tree-walking interpreter. Used by
  `aperio run`. Parity with codegen is a soft goal; some
  features ship in codegen first.
- **`aperio-codegen/`** — LLVM codegen via inkwell. Owns the C
  runtime + the bundled stdlib seed. The main file
  (`codegen.rs`) is intentionally large (~18kLOC); don't split
  it without a strong reason. `CodegenTy` is the codegen-side
  type rep (carries layout / repr info that `Ty` doesn't).
- **`aperio-cli/`** — the `aperio` binary. Owns the
  parse-with-imports flow for cross-seed imports.
- **`aperio-ts-shim/`** — tree-sitter staticlib bridge backing
  `std::ts::*`. The `[lib].path` redirect points to
  `runtime/lotus_treesitter.rs`.

## Two type representations

There are deliberately two: `aperio_types::Ty` and
`aperio_codegen::CodegenTy`. They mirror each other at the
primitive level (Int, String, Bool) but `CodegenTy` carries
layout-aware variants (`LocusRef(name)`, `TypeRef(name)`,
`Array(elem, n)`) the typechecker doesn't need. Don't try to
unify them; the split is load-bearing — typechecking happens
without LLVM context, codegen happens with it.

## Recent significant ships (orientation)

Read recent commits before starting (`git log --oneline | head
-30`). At time of writing, the major shipped pieces include:

- v1.x topic system (Phase 1 + Phase 2): typed `topic` decls,
  hierarchical wire subjects, `main` locus + `bindings { }`,
  closed-world intra-locus optimization.
- v1.x-FORM-{1..5}: `fallible(T)`, `@form(vec)`,
  `@form(hashmap)`, `@form(ring_buffer)`, locus arena elision,
  bus drain fast paths.
- v1.x-3 recognition projection class: `fixed_cell` +
  `shared_slab` end-to-end.
- v1.x-IMPORT: cross-seed imports via vendoring (`import
  "lib/x" as alias;`).
- F.20 structural interfaces (Phase A + B): `interface I { ... }`
  with vtable dispatch.

The full ledger is in `spec/stdlib.md` (per-phase tables) and
the `notes/v1.x-checkpoint.md` (current work tracker).

## Anti-patterns the compiler explicitly rejects

These are choices the language has made and won't unmake on a
whim. If your design pulls toward one, you're probably solving
the wrong problem.

- **No parametric tagged enums for errors.** `fallible(T)` is
  the value-level error protocol. The runtime observes one
  failure mechanism: closure violation.
- **No `trait` system.** `interface I { ... }` is structural
  (any locus whose methods cover the signature satisfies it).
  `trait` is reserved with no semantics.
- **No `lotus_*` → `aperio_*` rename.** The C-runtime prefix
  is `lotus_*` by design — Aperio is the language; lotus is
  the substrate. Keep the split.
- **No splitting `codegen.rs` for cleanliness.** It's
  intentionally one file; split only with a stronger
  justification than length.
- **No feature flags for staged rollout.** One branch, one
  binary, one set of behaviors.
- **No new `unsafe` Rust blocks** except where inkwell / LLVM
  demand them.
- **No removing `mNN` / `v1.x-*` milestone refs from
  comments.** They trace why code exists.

## Spec-vs-implementation discipline

This is locked workflow. For any user-visible change:

1. Land the implementation (parser / typechecker / codegen /
   runtime + tests).
2. Add an F-numbered commitment to
   `spec/design-rationale.md` (for new design choices) OR
   update the relevant `spec/<topic>.md` (for shipping under
   an existing commitment).
3. Update `docs/` if user-facing onboarding material changes.
4. Resolve `notes/open-questions.md` if you closed a deferred
   question.
5. On removal: both sides delete. Don't leave the spec
   asserting things the impl no longer does.

The spec is **not aspirational**. If `spec/design-rationale.md`
asserts an F-commitment, that behavior is shipped — the spec
is the contract, not a wishlist.

## Verification

After non-trivial changes:

```sh
cargo build --release
cargo test --release --workspace -- --test-threads=1
```

The serial flag avoids "text file busy" flakes from parallel
builds racing each other on the same temp binary path. Phase-2
topic tests (`crates/aperio-codegen/tests/topic_phase2.rs`) and
the examples-parse acceptance test
(`crates/aperio-syntax/tests/examples.rs`) are the broadest
canaries — neither should regress.

For codegen-only changes, the integration tests under
`crates/aperio-codegen/tests/` are the broadest exercise
surface.

## The recursive principle, applied to compiler internals

Aperio's "everything is a locus" axiom applies inside the
compiler too:

- A compiler **type** = shape, no flow. In Rust: `struct`.
  Examples: `Ty`, `CodegenTy`, `Span`, `Token`.
- A compiler **locus** = flow with invariants. In Rust:
  `struct` with methods that maintain invariants. Examples:
  `Cx` (codegen context), `Lexer`, `Parser`,
  `Resolver` (typechecker).

When adding a subsystem, ask: *does this have flow, or is it a
record?* Records simpler — prefer them. Loci for
genuinely-state-accumulating systems.

## Hard guardrails

- Don't break existing tests. If a test fails because the spec
  genuinely changed, update the test in the same commit and
  explain in the commit message. Never delete a test to make
  it pass.
- Don't invent grammar or stdlib paths. Spec first, implement
  second.
- Don't generate session-state markdown files in the repo (no
  "PROGRESS.md", "PLAN.md", "STATUS.md" — those belong in
  conversation, the friction log, or `notes/open-questions.md`).
- Don't commit changes the user didn't ask for. The user owns
  commit cadence.
- Don't skip pre-commit hooks (`--no-verify`) unless the user
  explicitly asks.

## Milestone idiom

Features land as `mNN` (early m-numbered milestones) or
`v1.x-FORM-N` / `v1.x-FOO` (the current naming family).
Conventions:

- Commit message starts with the milestone tag.
- Tests in `crates/aperio-codegen/tests/<name>.rs` (codegen)
  or the relevant crate's `tests/`.
- Often a corresponding small example fixture under
  `crates/aperio-codegen/tests/fixtures/examples/<NN>-<name>/`.

`git log --oneline | head -50` is the fastest orientation on
recent work.
