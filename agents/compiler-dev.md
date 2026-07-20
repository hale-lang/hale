# Brief: working on the Hale compiler

You're an agent (or human) working on the compiler / runtime
itself. This brief tells you the architecture, the invariants
that constrain how the language evolves, and the discipline
that's expected of changes.

## The pipeline

```
.hl source
  → hale-syntax::lexer    (tokens)
  → hale-syntax::parser   (AST)
  → hale-types::resolve   (top scope)
  → hale-types::check     (typechecked Bundle)
  → hale-codegen::codegen   (LLVM IR via inkwell → object → linked binary)
```

Both `hale run` and `hale build` go through codegen — `run`
compiles to a temporary binary and execs it, `build` leaves the
binary on disk. There is no separate interpreter.

The C runtime (`crates/hale-codegen/runtime/lotus_arena.c`)
and the stdlib seed (`crates/hale-codegen/runtime/stdlib/`)
are bundled into the codegen crate via `include_str!` and
`include_bytes!`; the resulting binary needs no separate
runtime install.

## Crate map

- **`hale-syntax/`** — lexer, parser, AST, diagnostic types,
  and the post-parse desugar passes (e.g.
  `desugar::desugar_topics`, `desugar::desugar_intra_locus_topics`).
  Hand-written recursive-descent parser; no parser-generator
  dependency.
- **`hale-types/`** — symbol resolution (`resolve.rs`) and
  type checking (`check.rs`). `Ty` is the canonical type
  representation here.
- **`hale-codegen/`** — LLVM codegen via inkwell. Owns the C
  runtime + the bundled stdlib seed. `CodegenTy` is the
  codegen-side type rep (carries layout / repr info that `Ty`
  doesn't). The crate is **model-organized** along the lotus /
  hypergraph language model (refactor delivery plan:
  `notes/refactor-codegen-model-org.md`):
  - `src/codegen.rs` holds `Cx<'ctx, 'p>` (the shared-state
    struct), the top-level entry points (`build_executable` /
    `build_program`), `lower_program` Pass-A/B orchestration,
    and the central `lower_stmt` / `lower_expr` /
    `lower_stdlib_path_call*` dispatchers.
  - `src/stdlib/<ns>.rs` — per-`std::*`-namespace path-call
    lowerings (`bus`, `bytes`, `crypto`, `decimal`, `env`,
    `io_fs`, `io_file`, `io_stdin`, `io_tcp`, `io_tls`, `io_udp`,
    `math`, `process`, `rand`, `sockopt`, `str`, `text`, `time`).
    Each is a `pub(crate) trait <Ns>Stdlib<'ctx>` extension on
    `Cx`; the trait is imported at the top of `codegen.rs` so
    call sites keep the `self.lower_std_*(...)` shape.
  - `src/bus/{wire,dispatch,runtime}.rs` — bus codegen
    (serialize/deserialize synthesis, publish + key-filter
    lowering, register/drain/destroy runtime hooks).
  - `src/locus/{decl,instantiation,method,dissolve,closure,return_path}.rs`
    — locus codegen (Phase-A struct + method decls,
    instantiation constructor, Phase-B method-body emit,
    drain/dissolve cascade, closure-eval emission, m49/m90
    return-path deep-copy).
  - `src/form/mod.rs` — `@form(vec)` / `@form(hashmap)` /
    `@form(ring_buffer)` synthesized-method dispatchers.
  - `src/types/mod.rs` — type-expression lowering, user type +
    enum + interface declarations, generic monomorphization,
    F.30 view coercions.
  - `src/channels/mod.rs` — `fallible(E)` value-error protocol
    (or-disposition lowering, sret epilogues, IoError/ParseError
    shapes) + structural-channel routing
    (`resolve_failure_route`).
  - `src/shared/builtins.rs` — libc + lotus extern declarations
    (`declare_builtins`).
  - `src/mangle.rs` — name mangling.

  Trait extensions are the dominant pattern for namespace-style
  call-site shapes; bare inherent `impl<'ctx, 'p> Cx<'ctx, 'p>`
  blocks in subdirectory files work too (Rust merges them) and
  are the pattern for cross-cutting helpers that don't benefit
  from a namespace trait. Cx fields and helper struct fields are
  mostly `pub(crate)` so cross-module code can read/write
  directly — encapsulation isn't load-bearing for an internal
  state container.
- **`hale-cli/`** — the `hale` binary. Owns the
  parse-with-imports flow for cross-seed imports.
- **`hale-ts-shim/`** — tree-sitter staticlib bridge backing
  `std::ts::*`. The `[lib].path` redirect points to
  `runtime/lotus_treesitter.rs`.

## Two type representations

There are deliberately two: `hale_types::Ty` and
`hale_codegen::CodegenTy`. They mirror each other at the
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
- F.27 (v1.x-VIOLATE): `epoch inline` closure + `violate NAME;`
  for inline structural failure from inside locus methods —
  the bridge between the value channel and the structural
  channel.
- 2026-05-16 library-shape sweep (token-efficiency parity):
  IoError fallible flip across `std::io::*`; `@form(vec)` sort
  family (`sort` / `sort_by` / `sort_desc_by`) + `set`;
  `@form(hashmap)` iteration (`key_at` / `entry_at`) + `bump`;
  `std::text` byte-class predicates + `tokenize_words_into`;
  `std::http::Server` route-dispatch locus; `std::json` escape
  + flat-shape parse helpers; `or discard` disposition;
  `std::env::arg_or`; `\xNN` ASCII-byte escape; readable
  callee-unresolved diagnostic with substring-aware close-match
  suggestion. Defensive alloca audit hoisted 5 escape-prone
  sites to entry-block to mirror the cliff-lift fix.

The full ledger is in `spec/stdlib.md` (per-phase tables).

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
- **No `lotus_*` → `hale_*` rename.** The C-runtime prefix
  is `lotus_*` by design — Hale is the language; lotus is
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
topic tests (`crates/hale-codegen/tests/topic_phase2.rs`) and
the examples-parse acceptance test
(`crates/hale-syntax/tests/examples.rs`) are the broadest
canaries — neither should regress.

For codegen-only changes, the integration tests under
`crates/hale-codegen/tests/` are the broadest exercise
surface.

## Performance

The bench harness lives in the sibling
[`hale-lang/bench`](https://github.com/hale-lang/bench)
repo. Clone it next to `hale/` so the harness finds the
compiled binary, or install `hale` on PATH:

```
~/code/hale-lang/
├── hale/   (this repo)
└── bench/    (clone of hale-lang/bench)
```

The harness resolves the binary as: `$HALE_BIN` →
`hale` on PATH → `../hale/target/release/hale`.

Perf-sensitive changes — anything touching codegen lowering,
the C runtime, or the bus dispatch path — should run the
relevant benches before and after, and either match or beat
the baseline.

```
cd ../bench
./run.sh --bench=<name>             # one bench
./run.sh                            # full suite; exits 1 on Hale regression
./run.sh --update-baselines         # after a verified gain
```

Baselines live in `hale-lang/bench/baselines.json` and update
deliberately (commit the new medians in the bench repo with a
commit message explaining what changed and where the gain came
from). Comparative numbers vs Go / Node / Python are emitted
in the report but never gate exit code — only Hale
regressions do.

## The recursive principle, applied to compiler internals

Hale's "everything is a locus" axiom applies inside the
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
- Tests in `crates/hale-codegen/tests/<name>.rs` (codegen)
  or the relevant crate's `tests/`.
- Often a corresponding small example fixture under
  `crates/hale-codegen/tests/fixtures/examples/<NN>-<name>/`.

`git log --oneline | head -50` is the fastest orientation on
recent work.
