# Introduction

Aperio is an experimental programming language for systems built
out of **loci** — typed, lifecycled units that publish and
subscribe to each other through a typed bus. Apps, services,
handlers, caches, schedulers, libraries: everything is a locus.
Composition is recursive — loci nest inside loci all the way down.

This book is a work in progress. Until the narrative chapters
catch up, the **canonical reference** is the spec corpus:

- [`spec/grammar.ebnf`](https://github.com/aperio-lang/aperio/blob/main/spec/grammar.ebnf) —
  formal grammar.
- [`spec/semantics.md`](https://github.com/aperio-lang/aperio/blob/main/spec/semantics.md) —
  language semantics.
- [`spec/styleguide.md`](https://github.com/aperio-lang/aperio/blob/main/spec/styleguide.md) —
  idiomatic Aperio.
- [`spec/design-rationale.md`](https://github.com/aperio-lang/aperio/blob/main/spec/design-rationale.md) —
  why the language is shaped the way it is.

For a tour by example, read the small programs under
[`crates/aperio-codegen/tests/fixtures/examples/`](https://github.com/aperio-lang/aperio/tree/main/crates/aperio-codegen/tests/fixtures/examples) —
they double as the language's acceptance test suite, so each one
is guaranteed to compile against the current `main`.
