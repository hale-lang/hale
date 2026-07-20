# Reference

This guide is the tour. The **canonical contract** — what the
compiler actually enforces — lives in the `spec/` directory at
the repository root. When the guide and the spec disagree, the
spec wins; when you need the exact rule, an edge case, or a
diagnostic's meaning, go there.

## The spec, by topic

| You want | Read |
|---|---|
| The formal grammar | [`spec/grammar.ebnf`](https://github.com/hale-lang/hale/blob/main/spec/grammar.ebnf) |
| Lexical structure, literals, operators | [`spec/tokens.md`](https://github.com/hale-lang/hale/blob/main/spec/tokens.md) |
| Operator precedence & associativity | [`spec/precedence.md`](https://github.com/hale-lang/hale/blob/main/spec/precedence.md) |
| Operational semantics (lifecycle, bus, recovery, fallible) | [`spec/semantics.md`](https://github.com/hale-lang/hale/blob/main/spec/semantics.md) |
| The type system | [`spec/types.md`](https://github.com/hale-lang/hale/blob/main/spec/types.md) |
| Memory: regions, capacity slots, projection classes | [`spec/memory.md`](https://github.com/hale-lang/hale/blob/main/spec/memory.md) |
| The form library (`vec` / `hashmap` / `ring_buffer`) | [`spec/forms.md`](https://github.com/hale-lang/hale/blob/main/spec/forms.md) |
| The always-loaded runtime | [`spec/runtime.md`](https://github.com/hale-lang/hale/blob/main/spec/runtime.md) |
| The standard library surface | [`spec/stdlib.md`](https://github.com/hale-lang/hale/blob/main/spec/stdlib.md) |
| Idiomatic patterns, the seven shapes, correctness + speed rules | [`spec/styleguide.md`](https://github.com/hale-lang/hale/blob/main/spec/styleguide.md) |
| The FFI contract — C (`@ffi("c")`) and the WASM host interface (`@ffi("js")` / `@export`) | [`spec/ffi.md`](https://github.com/hale-lang/hale/blob/main/spec/ffi.md) |
| Dependencies & vendoring | [`spec/packages.md`](https://github.com/hale-lang/hale/blob/main/spec/packages.md) |
| Project layout & imports | [`spec/projects.md`](https://github.com/hale-lang/hale/blob/main/spec/projects.md) |
| How tests are written and run | [`spec/testing.md`](https://github.com/hale-lang/hale/blob/main/spec/testing.md) |
| Why the current surface is shaped this way | [`spec/design-rationale.md`](https://github.com/hale-lang/hale/blob/main/spec/design-rationale.md) |
| The design-decision log (the F-series: commitments, rejects, sketches) | [`spec/decisions.md`](https://github.com/hale-lang/hale/blob/main/spec/decisions.md) |
| Internal codename legend (milestone / workstream tags) | [`spec/glossary.md`](https://github.com/hale-lang/hale/blob/main/spec/glossary.md) |

## Two more anchors

- **[`AGENTS.md`](https://github.com/hale-lang/hale/blob/main/AGENTS.md)** — the load-bearing prompt for
  agents writing `.hl`. It condenses the six idiomatic patterns,
  the "what's not in the language" reflexes, and the formal
  design model into one file. Excellent for a human, too.
- **Working programs** — `crates/hale-codegen/tests/fixtures/examples/`
  holds 83 small per-feature programs, numbered. Reading a few
  near your target shape is the fastest way to see real,
  compiling Hale.

## Toolchain commands

| Command | Does |
|---|---|
| `hale run <file/dir>` | compile + run (fast feedback) |
| `hale build <file/dir>` | compile to a native binary |
| `hale check` | parse + typecheck only |
| `hale test` | run `*_test.hl` |
| `hale fetch` | clone & pin git dependencies |
| `hale fmt` | canonical formatter |
