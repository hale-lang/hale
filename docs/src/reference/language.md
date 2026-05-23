# Language reference

This page is a reference index — high-level pointers to the
canonical formal definitions in the `spec/` corpus. Where
[Concepts](../concepts/the-locus.md) is pedagogical (how to
think in Aperio), this page is for *looking up* what the
compiler actually accepts.

The spec is the source of truth. If something here disagrees
with the spec, the spec wins.

## Grammar and syntax

- [`spec/grammar.ebnf`](https://github.com/aperio-lang/aperio/blob/main/spec/grammar.ebnf) —
  formal grammar in EBNF. Every syntactic construct the
  parser accepts.
- [`spec/tokens.md`](https://github.com/aperio-lang/aperio/blob/main/spec/tokens.md) —
  lexical structure: identifier rules, reserved words, literal
  forms (integer / float / decimal / string / bytes / time /
  duration / f-string), operators, contextual keywords.
- [`spec/precedence.md`](https://github.com/aperio-lang/aperio/blob/main/spec/precedence.md) —
  expression precedence and associativity table.

## Semantics

- [`spec/semantics.md`](https://github.com/aperio-lang/aperio/blob/main/spec/semantics.md) —
  operational semantics. Program startup, locus
  instantiation, lifecycle method dispatch, bus dispatch,
  closure-test evaluation, recovery primitives, dissolve
  timing rules, fallible call semantics, topic declarations.
- [`spec/runtime.md`](https://github.com/aperio-lang/aperio/blob/main/spec/runtime.md) —
  what the runtime ships with: region allocator, scheduler,
  bus router, time primitives, schedule classes, perspective
  hot-load machinery.

## Types

- [`spec/types.md`](https://github.com/aperio-lang/aperio/blob/main/spec/types.md) —
  the type system: primitive types, compound types,
  projection-class types, locus types, perspective types,
  structural interfaces, fallible typing.
- Numeric coercion: Int → Float widening at let-binding type
  ascriptions, fn-arg sites, mixed-type binary ops (`0.5 + n`,
  `i < 0.5`), and user-type field-init positions. Strictly
  one-way; Decimal never participates. See `types.md` §
  "Numeric coercion".
- Locus → Interface coercion (F.20 Phase B): at fn args,
  returns, struct/locus field initializers, `@form(vec)` cell
  push, `or`-fallback substitutes, and — as of G20 2026-05-23
  — at composite-typed let-binding ascriptions
  (`let arr: [Greeter; 2] = [Hi {}, Hey {}];`,
  `let pair: (Greeter, Greeter) = (Hi {}, Hey {});`,
  `let arr: [Greeter; 3] = [Hi {}; 3];`). The codegen
  propagates the ascription's element type through
  `lower_expr_into(expr, hint)` so per-position
  `coerce_to_interface` fires at construction. Composite
  *return* positions (and the nested locus-escape they
  imply) remain deferred — see `types.md` § "Composite-
  construction coercion".

## Storage and memory

- [`spec/memory.md`](https://github.com/aperio-lang/aperio/blob/main/spec/memory.md) —
  the memory model. Hierarchical regions, per-projection-
  class allocators, capacity slots (`pool` / `heap`), bookkeeping
  reclamation, drain cascade, region-escape rules. Includes
  the codegen ABI summary.
- [`spec/forms.md`](https://github.com/aperio-lang/aperio/blob/main/spec/forms.md) —
  the `@form(...)` annotation system: `@form(vec)`,
  `@form(hashmap)`, `@form(ring_buffer)`. Contract, lowering,
  performance bands, anti-patterns.

## Projects and packaging

- [`spec/projects.md`](https://github.com/aperio-lang/aperio/blob/main/spec/projects.md) —
  project layout, per-directory seed model (F.19), cross-seed
  imports (F.25), workspace fallback, resolution order,
  mangling scheme.
- [`spec/packages.md`](https://github.com/aperio-lang/aperio/blob/main/spec/packages.md) —
  the v1 package surface. `aperio.toml` manifest, `aperio.lock`,
  `aperio fetch` git-based dependency fetcher.

## Style and conventions

- [`spec/styleguide.md`](https://github.com/aperio-lang/aperio/blob/main/spec/styleguide.md) —
  idiomatic Aperio. The full version of the patterns
  introduced in
  [Modeling — how to think in Aperio](../concepts/modeling.md);
  full naming conventions; expanded anti-patterns.

## Testing

- [`spec/testing.md`](https://github.com/aperio-lang/aperio/blob/main/spec/testing.md) —
  the testing pipeline. Three layers of correctness, the
  `std::test` assertion library, benchmark surface.

## Design rationale

- [`spec/design-rationale.md`](https://github.com/aperio-lang/aperio/blob/main/spec/design-rationale.md) —
  *why* the language is shaped the way it is. Numbered
  commitments F.0 through F.27 cover every design decision
  the compiler currently makes — from projection-class
  semantics to capacity slots to structural interfaces to
  the package model — with a "considered and rejected"
  section for each.

This is the longest single document in the corpus and the
most useful for understanding the rationale behind a
particular surface choice. Worth reading once, end-to-end,
once you've internalized Concepts.

## Standard library

- [Standard library overview](./stdlib.md) — companion
  reference page on this site.
- [`spec/stdlib.md`](https://github.com/aperio-lang/aperio/blob/main/spec/stdlib.md) —
  full surface, phase by phase. Authoritative list of what
  ships in the bundled stdlib.

## Foreign-function interface

For binding to C-ABI libraries outside stdlib's curated set,
Aperio exposes user-extensible `@ffi("c")` declarations. Library
authors ship bindings (`pond/raylib`, `pond/sqlite`, ...)
without compiler changes.

- [Bind a C library](../how-tos/ffi-bindings.md) — minimum
  end-to-end how-to.
- [`spec/ffi.md`](https://github.com/aperio-lang/aperio/blob/main/spec/ffi.md) —
  authoritative contract: syntax, ABI, lifetime rules,
  diagnostic surface.
- [`agents/binding-packages.md`](https://github.com/aperio-lang/aperio/blob/main/agents/binding-packages.md) —
  authoring brief for binding-library packages.
