# Testing pipeline

The testing pipeline is part of the language toolchain, not an
add-on. `hale test` ships in the same binary as `hale build`.
Test infrastructure exists from day 1 because the language's
discipline (closure tests, k_max bounds, projection-class
invariants, multi-perspective stability commit-rules) needs
testing infrastructure to be enforced.

`hale test` (Layer 1 + Layer 2 discovery and execution) ships
today. The benchmark, formatter, and standalone-verify tooling
described below (`hale bench` / `hale fmt` / `hale verify`) is
specified here but **not yet implemented** — the shipping CLI is
`lex` / `parse` / `check` / `run` / `build` / `test` / `fetch`.
Those sections describe the intended design, not current behavior.

## Three layers of correctness

Hale testing distinguishes three layers, each with its own
tooling:

### Layer 1 — Language correctness

*Does this program parse, typecheck, and have the meaning the
language spec says it should?*

- **Parser tests.** Given source, the parser produces the
  expected AST (or rejects with the expected error). Stored as
  `.hl` files paired with `.expected.json` (or similar) AST
  dumps. Driven by the grammar in `spec/grammar.ebnf`.
- **Typechecker tests.** Given a program, the typechecker
  accepts or rejects with the expected diagnostic. Same shape
  as parser tests.
- **Operational-semantics tests.** Given a program and inputs,
  the program produces the expected output. Driven by the
  operational semantics document (yet to be written).

These run as part of `hale test` and as part of compiler CI.
A compiler regression should be caught here.

### Layer 2 — Mathematical / framework correctness

*Does the framework's discipline hold for this program?*

The language's job is not just "compile the source" — it's
"refuse to compile a program that violates framework discipline."
The framework's commitments need test infrastructure:

- **k_max bound verification.** For every locus, the compiler
  computes `k_max = B / [(1 − phi) * c + phi * sigma]` and
  checks that no `accept` call site can exceed it. Tests assert
  the compiler rejects over-budget call sites.
- **Closure-test existence.** For every `closure name { ... }`
  block, the compiler verifies the cycle exists (both sides of
  `~~` reference defined values within the same scope). Tests
  assert the compiler rejects cycles that don't close.
- **Projection-class invariants.** A locus declared `projection
  rich` cannot be instantiated with N > rich's bound; etc.
  Tests assert mismatches are rejected.
- **Multi-perspective stability commit-rules.** A `perspective`
  with `stable_when |perspectives| >= 3` cannot be serialized
  with fewer perspectives validated. Tests assert violations
  fail at runtime.
- **Substrate-derivation discipline.** When a value carries
  anchor metadata, anchor-self-consistent uses are flagged.
  Tests assert anchor-self-consistency triggers a warning or
  error per declared policy.
- **Vertical-only flow.** Lateral references between sibling
  loci are compile-time rejected. Tests assert the compiler
  rejects sibling-to-sibling access.

These tests are written in Hale itself. The standard library
provides `assert(...)`, `assert_rejects(...)`, `assert_closure(...)`
and similar primitives. Test programs are valid Hale programs;
the framework discipline applies to them too.

### Layer 3 — Performance

*Does this program meet its declared performance envelope?
And how does it compare to equivalent implementations in other
languages?*

#### Single-language benchmarks

A benchmark is a function annotated with `bench` (TBD: grammar
extension, or stdlib function with a magic name like Go's
`Benchmark*`). The runner invokes it for a measured number of
iterations; reports time-per-op, allocations-per-op, memory
high-water.

```
fn bench_hello() {
    Hello { };
}
```

Output is JSON-serializable for CI consumption. Baselines are
checkpointed in version control; regressions produce a diff
the developer must explicitly accept.

#### Comparative benchmarks (Hale vs. other languages)

These are **internal development tools**, not published results.
Their purpose is to give the team visibility into the language's
performance shape as it evolves — to catch regressions, validate
that framework-discipline overhead is in the expected range, and
spot when a design choice is costing us order-of-magnitude
throughput.

A benchmark file declares its equivalent in another language as
a sibling:

```
// bench_message_passing_test.hl
//
// @external_equivalents:
//   - lang: go
//     path: ./equivalents/message_passing.go
//   - lang: rust
//     path: ./equivalents/message_passing.rs
```

The runner builds and runs each; reports a comparison table.
The author writes the equivalent however they think is fair for
the comparison they want — there is no "fairness review,"
because nothing is being published. The numbers are useful to
us; they don't need to be defensible to outsiders.

Useful comparative-perf categories for internal use:

- **Coordination-overhead.** Many-message-passing scenarios
  vs. Erlang / Go.
- **Region-allocation throughput.** Allocation / deallocation
  rate vs. GC'd languages.
- **Closure-test overhead.** Same program with closure tests
  on / off — measures the cost of the framework discipline.
- **Mode-projection.** Same kernel computed three ways
  (bulk / harmonic / resolution) vs. a hand-written
  per-N-implementation in another language.

Comparative results are not gatekept; any branch can produce
them and stash them in `bench-results/` (gitignored). A regression
in hale-vs-X ratio is a developer signal, not a CI gate.

#### Performance regressions in CI

Every benchmark has a stored baseline (numerical envelope, not
a fixed value — a tolerance band). The runner asserts current
runtime is within band. Bands tighten over time as the compiler
improves; widening a band is an explicit, reviewed action.

## Test file layout

```
project/
├── src/
│   └── *.hl            // production source
└── tests/
    ├── unit/
    │   └── *_test.hl   // unit tests, by module
    ├── integration/
    │   └── *_test.hl   // multi-locus integration tests
    ├── bench/
    │   └── *_bench.hl  // benchmarks
    └── equivalents/    // external-language equivalents for
        ├── go/         // comparative benchmarks
        ├── rust/
        └── erlang/
```

Or, alternatively, Go-style: `*_test.hl` lives next to the
source it tests. Both layouts are supported; the runner finds
tests by suffix (`_test.hl`) regardless of location.

## Toolchain commands

| Command | Purpose |
|---|---|
| `hale build` | Compile source → executable / library |
| `hale check` | Static checks: parse, typecheck, framework discipline |
| `hale test` | Run all `*_test.hl` files in the project |
| `hale test -run pattern` | Run matching tests only |
| `hale bench` *(planned)* | Run all `*_bench.hl` files |
| `hale bench -compare` *(planned)* | Build and run external equivalents alongside |
| `hale verify` *(planned)* | Layer-2 discipline checks specifically (no execution) |
| `hale fmt` | Canonical formatter (Go-style: zero config; see below) |

`hale test` runs Layer 1 + Layer 2 today; `hale bench` (planned)
runs Layer 3.

## `hale fmt` — the canonical formatter

Zero config, Go-style: there are no options that change the output.
`hale fmt [paths]` formats `.hl` files in place (no path = the
current directory tree; `vendor/` and dot-directories are skipped);
`--check` lists files that would change and exits 1 (the CI gate);
`--diff` previews without writing; `--stdin` filters stdin→stdout
for editor integration.

What canonical form means (a token-stream formatter — the author's
line-break structure is PRESERVED, gofmt-style; there is no
max-line-length enforcement):

- **Indentation** — 4 spaces per bracket depth. A closing bracket
  returns to its opener's line indent; brackets opened together on
  one line indent their contents once. Bracket-less continuation
  lines (a leading `&&`/`.`, a trailing binary operator on the
  previous line) get one extra level.
- **Spacing** — canonical pair rules: binary operators spaced,
  unary `-`/`!` tight to their operand, `.`/`::`/`..` tight,
  nothing inside `(` `)` `[` `]`, literal braces spaced
  (`Rec { key: 1 }`, `{ }`), `:` tight-left (except the spaced
  `locus X : serves P` conformance colon, per this spec's own
  examples), generic angles tight (`Holder<Int>`), lifecycle
  parens tight (`run()`).
- **Blank lines** — collapsed to at most one; none at file start;
  exactly one trailing newline. Intra-line alignment padding
  (`let x   = 1;`) collapses to single spaces.
- **Comments** — preserved verbatim in position: own-line comments
  indent with the code, trailing comments sit one space after it.

Safety: the formatter re-lexes its own output and refuses to write
unless the semantic token stream is byte-identical to the input's —
a formatter bug can mangle whitespace, never what the compiler
sees. Files that don't lex are reported and left untouched.
Formatting is idempotent; the corpus test
(`hale-syntax/tests/fmt_corpus.rs`) holds every fixture example and
stdlib source to both properties.

## Test assertion library

Provided by `std::test`. Not a separate testing framework; the
language's stdlib includes test primitives.

### v0.1 (sealed m87, m88)

Three primitives, all written purely in Hale (composing
`std::process::exit`):

```hale
fn main() {
    std::test::assert(2 + 2 == 4, "trivial arithmetic");
    std::test::assert_eq_int(answer(), 42, "answer");
    std::test::assert_eq_str(greet("world"), "hello, world", "greeting");
}
```

The test-runner contract is exit-code based:

- **Pass** = exit 0 with **no stdout**. A test program that
  runs to completion silently has passed.
- **Fail** = non-zero exit code with `ASSERTION FAILED: <msg>`
  (and, for `assert_eq_*`, `expected: X / actual: Y`) on
  stdout. The first failure short-circuits — `std::process::exit`
  terminates immediately.

A `.hl` test program is just an ordinary Hale binary. The
current test runner is the Rust integration harness in
`crates/hale-codegen/tests/`; future `hale test` CLI runs
the same `.hl` programs unchanged.

### What landed vs what's still aspirational

| Surface | Status |
|---|---|
| `std::test::assert(cond, msg)` | sealed m87 |
| `std::test::assert_eq_int(actual, expected, msg)` | sealed m87 |
| `std::test::assert_eq_str(actual, expected, msg)` | sealed m87 |
| `assert_neq` / `assert_neq_int` / `assert_neq_str` | not shipped |
| `assert_rejects(...)` (compile-time errors) | not shipped — needs compiler-level surface |
| `assert_closure(name, tolerance)` | not shipped — needs closure-test introspection |
| `mock_locus<T>(...)` | not shipped |
| `bench_iter(n, f)` | not shipped |
| `hale test` CLI runner | shipped — discovery→compile→run→report driver over `*_test.hl` (`-run`, `--json`) |

## Property-based testing

Reserved as a future extension. The language's strong
type-and-discipline surface makes property-based testing
particularly natural — you can declare properties that should
hold for all inputs and let the runner generate counter-examples.
Not in the v0.1 stdlib.

## Continuous integration

The toolchain emits machine-readable output:

- `hale test --json` produces JSON test results (per-test pass/fail,
  per-test timing, error messages).
- `hale bench --json` produces JSON benchmark output (per-bench
  time, allocations, comparative table if `-compare` given).
- `hale check --json` produces JSON diagnostics.

CI consumes the JSON; standard reporters (JUnit XML, GitHub
Actions annotations, etc.) are downstream conversions.

## What writing this surfaces (for resolution)

1. **`bench` annotation: keyword, attribute, or naming convention?**
   Go uses `BenchmarkName`. Rust uses `#[bench]`. Hale has
   neither attributes nor magic-name conventions yet. Decision
   pending; probably an attribute (`@bench fn ...`) added to
   the grammar in v0.2.
2. **Determinism.** For benchmarks, the runtime should be
   isolatable (no GC pauses to confound; we don't have GC, so
   that's free). Does the runtime need deterministic
   scheduling for benchmark consistency? Probably yes for some
   benchmark classes; opt-in.
3. **External-language toolchain access.** `hale bench
   -compare` needs `go`, `rustc`, `erlc`, etc. on PATH.
   Documenting this clearly is dev-experience work.
