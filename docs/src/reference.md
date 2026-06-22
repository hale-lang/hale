# Reference

This guide is the tour. The **canonical contract** — what the
compiler actually enforces — lives in the `spec/` directory at
the repository root. When the guide and the spec disagree, the
spec wins; when you need the exact rule, an edge case, or a
diagnostic's meaning, go there.

## The spec, by topic

| You want | Read |
|---|---|
| The formal grammar | [`spec/grammar.ebnf`](../../spec/grammar.ebnf) |
| Lexical structure, literals, operators | [`spec/tokens.md`](../../spec/tokens.md) |
| Operator precedence & associativity | [`spec/precedence.md`](../../spec/precedence.md) |
| Operational semantics (lifecycle, bus, recovery, fallible) | [`spec/semantics.md`](../../spec/semantics.md) |
| The type system | [`spec/types.md`](../../spec/types.md) |
| Memory: regions, capacity slots, projection classes | [`spec/memory.md`](../../spec/memory.md) |
| The form library (`vec` / `hashmap` / `ring_buffer`) | [`spec/forms.md`](../../spec/forms.md) |
| The always-loaded runtime | [`spec/runtime.md`](../../spec/runtime.md) |
| The standard library surface | [`spec/stdlib.md`](../../spec/stdlib.md) |
| Idiomatic patterns & the six shapes | [`spec/styleguide.md`](../../spec/styleguide.md) |
| The FFI contract — C (`@ffi("c")`) and the WASM host interface (`@ffi("js")` / `@export`) | [`spec/ffi.md`](../../spec/ffi.md) |
| Dependencies & vendoring | [`spec/packages.md`](../../spec/packages.md) |
| Project layout & imports | [`spec/projects.md`](../../spec/projects.md) |
| How tests are written and run | [`spec/testing.md`](../../spec/testing.md) |
| Why every design choice was made | [`spec/design-rationale.md`](../../spec/design-rationale.md) |

## Two more anchors

- **[`AGENTS.md`](../../AGENTS.md)** — the load-bearing prompt for
  agents writing `.hl`. It condenses the six idiomatic patterns,
  the "what's not in the language" reflexes, and the formal
  design model into one file. Excellent for a human, too.
- **Working programs** — `crates/hale-codegen/tests/fixtures/examples/`
  holds ~70 small per-feature programs, numbered. Reading a few
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
