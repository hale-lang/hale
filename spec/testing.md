# Testing pipeline

The testing pipeline is part of the language toolchain, not an
add-on. `hale test` ships in the same binary as `hale build`.
Test infrastructure exists from day 1 because the language's
discipline (closure tests, k_max bounds, projection-class
invariants, multi-perspective stability commit-rules) needs
testing infrastructure to be enforced.

`hale test` (Layer 1 + Layer 2), `hale bench` (Layer 3
single-language), `hale fmt`, and `hale doc` all ship in the CLI
today (`lex` / `parse` / `check` / `run` / `build` / `test` /
`bench` / `verify` / `fmt` / `doc` / `fetch` / `lsp` /
`mcp`). Only
`hale bench -compare` remains design-only below. `hale verify`
runs `check`'s exact analysis surface but GATES: any finding —
advisory or error — exits 1, making it the CI discipline gate
(where `check` stays the fast advisory oracle: warnings print,
only errors fail).

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
| | (`hale test` applies the same `hale.toml [ffi]` csrc/link pickup as `hale build`, so tests importing FFI-bearing libs link — 2026-07-18) |
| `hale bench` | Run all `*_bench.hl` files (see below) |
| `hale bench -compare` *(planned)* | Build and run external equivalents alongside |
| `hale verify` | Layer-2 discipline gate: `check`'s full analysis, ANY finding fails (no execution) |
| `hale fmt` | Canonical formatter (Go-style: zero config; see below) |
| `hale doc` | API reference from `///` doc comments (Markdown / `--json`; see below) |

`hale test` runs Layer 1 + Layer 2 today; `hale bench` runs
Layer 3's single-language half.

`check` and `verify` resolve the whole import graph, and a parse
failure **anywhere** in it — the target's own files, a library it
imports, a library that library imports — fails both, at the
offending file's own line and column, with the same exit status and
`--json` row any other error gets. That is the contract a gate rests
on: `check` never reports success on a tree `build` would refuse
(2026-09-19, GH #765; before it, an imported seed that failed to parse
left its declarations silently absent and both commands answered
clean). The `--json` half of that contract reached the target's OWN
files on the same day (GH #777): the parse path predated the JSON
reporting path and printed text to stderr, so a syntactic failure in
the seed being checked exited non-zero with an empty stream — a gate
saw a failure with nothing explaining it. Every parse diagnostic is
now a record like any other.

An input that could not be READ answers the same way. A target that
is not there, a `.hl` file of the seed that will not open, a file of
the import graph that will not open: each is one record,
`"kind":"io error"`, naming the path with the OS error as its
message and no position — `"line":0,"col":0`, because a file that
never opened has no text to be positioned in (2026-09-20, GH #806;
before it these printed a sentence on stderr and left `--json`
empty, so an environment failure and a crash were the same thing to
a gate). The text rendering is unchanged, and so is every command
without a machine-readable channel.

The ENTRY file of a build is in that set too. A `*_test.hl` that
will not open is a `hale test --json` row with `"status":"fail"`
whose `message` is the `could not read <path>: <os error>` sentence,
not an empty string (2026-09-20, GH #903; that one site still
printed at the failure and handed its caller nothing, so the row
reporting it carried no reason and the explanation went to stderr).

The **position** is not command-scoped. Every command that resolves
imports — `build`, `run`, `test`, `bench`, `replay`, as well as
`check` and `verify` — reports a diagnostic from an imported file as
`path:line:col: kind: message` with the offending source line and a
caret, at that file's OWN line and column. Each file of an import
graph is parsed at its own virtual base so the merged spans stay
globally unique; un-shifting by that base is part of reporting, not a
property of one command's reporting path (2026-09-20, GH #775; before
it, the commands with no `--json` channel rendered the bundle offset
against the file's own text, so the file name and the message were
right and the line and column were not).

Nor is the **path**. A diagnostic names its file by the file's
canonical path — absolute, symlinks resolved, with no `.` or `..`
component — in the text rendering, in a `note:` secondary location,
and in the `file` field of a `--json` record (the positionless
`io error` record above included), from every command, for the
target's own files and for every file reached through an `import`
alike. That is one string per file rather than
one per channel, which is what a consumer joining the two needs:
`check` used to print the canonical path an imported file was
recorded under while `build`, `run` and `test` printed it as the
resolver reached it (`/abs/app/../lib/second.hl`), and the target's
own files were named exactly as the command line spelled them, `..`
and all (2026-09-20, GH #822).

`hale test --json`'s rows follow the same rule for the same reason.
A row says which test RAN rather than where an error is, but a tool
joining those rows to `check --json` records on `file` needs one
string per file, and the row used to echo the command line — so
`hale test ../app/x_test.hl` and a `check` of the same seed named
that file differently (2026-09-20, GH #867). The human-readable
`ok <path>` / `FAIL <path>` lines keep the spelling the command
line used: they are read beside the command that produced them.

Nor is the **kind** of failure. A refusal raised by CODEGEN rather
than by the front end — a construct the checker accepts and the
backend does not support, a missing toolchain component the program
needs — is a located diagnostic like any other whenever it carries a
span: `path:line:col: codegen error: message`, with the offending
source line and a caret, from `build`, `run`, `test`, `bench` and
`replay` alike, byte for byte the same line from each. A codegen
refusal with no span to point at prints `codegen error: message` and
nothing else, from all of them. These commands have no
machine-readable channel, so this is a text contract only; `check`
never reaches codegen and is unaffected (2026-09-20, GH #848; before
it only `build` used the span, and the rest printed the error's Rust
debug form — `UnsupportedAt("…", Span { start: Pos(55), end:
Pos(60) })` — so `hale run` could refuse a program without naming a
line to open). `bench` names the bench file itself, not the temporary
copy with the synthesized driver appended that it actually compiles
and then deletes.

Nor, finally, is the **rule set**. The checks that need the whole
program — the F.18 bare-callee rule and the bare-identifier and
bare-type-name rules that follow it (`spec/types.md` § *Calls to bare
names*) — are on for every command that HAS the whole program:
`check` and `verify` on a seed, `build`, `run`, `test` and `replay`,
which compile exactly what they bundle, and `hale lsp`, which
typechecks only once the whole seed has parsed. A call to a name
nothing declares is therefore `path:line:col: type error: call to X:
no free fn, generic fn or fn-pointer binding with that name is in
scope` from all of them (2026-09-20, GH #911 B1 / #846; the callee
half was off on the build path, so the front end said that and the
backend answered `codegen error: unsupported in codegen v0: call to
X: …` — no file, no line, no caret, and a did-you-mean over
compiler-internal symbols. Two answers to one question, and the
useful one was the one the build did not give). `hale check <file>`
keeps the permissive reading, because the sibling it was not handed
may declare the name: the line is drawn by what the command was
given, not by which command it is. `hale bench` typechecks nothing
today, so its only answers still come from codegen.

## `hale bench` — the Layer-3 runner

`hale bench [file | dir]` discovers `*_bench.hl` files (dir walk,
`vendor/` and dot-dirs skipped); every **zero-param free fn named
`bench_*`** is a benchmark. The runner appends a synthesized
driver `main` (a bench file must not define its own), compiles at
the release profile with the same `hale.toml [ffi]` pickup as
build/test, and runs it. The driver self-calibrates Go-style:
batch sizes grow ×10 until one batch takes ≥100 ms, then the
final batch reports **ns/op** and **allocs/op**
(`std::diag::heap_alloc_count` deltas; shown as `-` in sanitizer
builds where the counting shim is absent). `-run <substr>`
filters by bench name; `--json` emits one record per bench
(`file`/`name`/`iters`/`ns_per_op`/`allocs_per_op`) for CI.
Benchmarks may print their own output — non-report lines pass
through.

Still planned from the original design: stored baselines with
tolerance bands as a CI gate, and `-compare` external-language
equivalents. The `fn bench_*` magic-name convention (Go's
`Benchmark*` shape) is the resolved answer to the "grammar
extension or magic name?" question above — no grammar change.

## `hale doc` — the API-reference generator

Zero config. `hale doc [file | dir]` renders a seed's API
reference from `///` doc comments (the convention in
spec/tokens.md): every public top-level declaration — fns, loci
(with params and their documented methods), types, topics,
interfaces, consts — with its signature and doc text. Markdown to
stdout by default; `-o <path>` writes it; `--json` emits one
record per declaration (`file`/`kind`/`name`/`signature`/`doc`/
`members`) for tooling and agents. `__`-prefixed names and `main`
are internal and skipped; a file that doesn't parse is reported
and skipped with exit 1. Doc text is recovered positionally (the
lines directly above the declaration, stepping over decorator
lines), so the lexer and AST are untouched.

`hale doc --stdlib` renders the `std::` surface instead: the
rename table supplies public paths, the bundled stdlib source
supplies decl shapes + `///` docs (mangled param types demangled;
internal-typed params hidden), and the typecheck signature table
supplies the C-primitive-backed free fns that have no `.hl` decl.
The spec/stdlib.md tables remain the canonical CONTRACT; the
generated reference is the browsable companion, and stdlib
declarations grow `///` docs namespace-by-namespace (metrics,
log, and BytesBuilder are done).

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

Three primitives, all written purely in Hale:

```hale
fn main() {
    std::test::assert(2 + 2 == 4, "trivial arithmetic");
    std::test::assert_eq_int(answer(), 42, "answer");
    std::test::assert_eq_str(greet("world"), "hello, world", "greeting");
}
```

The test-runner contract is exit-code based:

- **Pass** = exit 0 with **no stdout**. A test program that
  runs to completion silently has passed. Stderr is not inspected:
  a test may write progress or diagnostics with `eprintln` and
  still pass.
- **Fail** = non-zero exit code with `ASSERTION FAILED: <msg>`
  (and, for `assert_eq_*`, `expected: X / actual: Y`) on
  stdout. The first failure short-circuits.

A `.hl` test program is just an ordinary Hale binary.

#### What runs after a failed assertion (GH #717)

A failing assertion prints its diagnostic, **records** the failure
and returns. It does not terminate the process from inside the
assertion. What happens next is fixed:

1. **Nothing else in the test body runs.** Control leaves `fn main`
   at the failing assertion's call site. Every later
   `std::test::assert*` is also a no-op against the recorded
   failure, so there is never a second `ASSERTION FAILED` line and
   never a second diagnostic.
2. **`fn main`'s ordinary teardown runs**, exactly as it does on a
   normal return: cooperative-pool workers are joined, the bus
   queue is drained, and every locus `let`-bound in `main` before
   the failing assertion dissolves — reverse declaration order,
   child cascade, `dissolve()` bodies and all (`spec/memory.md`
   § Lifetime rules, § Drain cascade). A locus born *after* the
   failing assertion never existed and is skipped.
3. **The process then exits non-zero** (code 1), so the exit-code
   contract above is unchanged.

That is the whole mechanism. It is **not** exception unwinding: a
failed assertion is not a value, not catchable, and does not
propagate through arbitrary frames. Two consequences follow, and
they are the contract, not accidents:

- A test that must release something — a subprocess it spawned,
  scratch state it created, a socket, a lock file — releases it in
  the `dissolve()` of a locus the test `let`-binds in `main`. That
  is the supported cleanup route, and it cleans only what the test
  owns. There is no runner-owned cleanup hook, and none is needed.
- An assertion that fails *outside* `main`'s own frame — inside a
  free `fn`, a locus method, an `on_failure` body — has no main
  teardown to return through and still terminates the process
  immediately with code 1. Put the assertions that guard owned
  resources in `main`.

**Process kill is a separate case.** `SIGKILL` (and a hard crash)
cannot be intercepted by anything: no `dissolve()` runs, no atexit
handler runs, and a child or scratch directory the program owned
outlives it. That is a property of the platform, not of the
assertion path — a test whose resources must survive a `kill -9`
of the runner needs an external reaper (a process group, a cgroup,
a supervising harness), and Hale does not provide one. A failed
assertion and a `return` from `main` run the dissolve cascade; a
runtime panic runs atexit-registered cleanup but not the cascade;
`SIGKILL` runs nothing at all.

### The compiler's own Hale-language suite

`tests/hale/` holds `*_test.hl` programs that test the language in
the language, run by `hale test` and wired into the workspace suite
(`crates/hale-cli/tests/hale_native_suite.rs`). Run them directly
with `hale test tests/hale`.

A behaviour test belongs there rather than in a Rust integration
test when it is "run a program, check what it computed". Two things
change in the move:

- **The expectation stops being transcribed.** The Rust form
  compiles a program that *prints*, then substring-matches the
  output from another language. The Hale form asserts next to the
  code — no second copy to drift. It is also stricter:
  `assert_eq_int(n, 42)` rejects what `stdout.contains("a=42")`
  accepts, because the latter also passes on `a=421`.
- **The program gets typechecked.** `build_executable`, which the
  Rust codegen tests call, parses and lowers but never runs the
  checker, so those programs are compiled and executed without ever
  being checked.

What stays in Rust is anything asserting on *compiler output* rather
than program behaviour: diagnostics, IR shape, leak counts,
observation records.

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

## Determinism

**Single-pool execution is deterministic.** A program whose loci
all run on the main cooperative scheduler — no `placement`
pinning, no additional pools — produces the same publishes and
the same deliveries in the same order on every run, given the
same inputs. This is a guarantee, not an observation: one pool
is one consumer thread by construction (`spec/runtime.md`
§ Scheduler — the same invariant dispatch devirtualization rests
on), handlers run to completion, and the main-thread bus queue
drains FIFO, so there is no scheduling freedom for an order to
vary within.

"Given the same inputs" is load-bearing: time, entropy,
environment, FFI returns, and socket reads are inputs, and a
program that consumes them may compute differently even though
its scheduling cannot reorder. Whether a body reaches those
inputs is a compile-time question — `@deterministic`
(`spec/verification.md` § Effect assertions) asserts it, and
`frontier::infer_effects` answers it without any annotation.

Pinned: `crates/hale-codegen/tests/replay_determinism.rs`
compares the complete ordered `BUS_PUBLISH`/`BUS_DELIVER` record
sequence of a multi-locus cascade across repeated runs. A
divergence there is a runtime bug, never a flaky test.

Multi-pool programs get no ordering guarantee across pools
today: each pool's single consumer thread has a well-defined
per-consumer delivery order, but the interleaving between pools
is OS scheduling. Recording that per-consumer order (and
replaying it) is the record/replay track — GH #296.

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
2. **Determinism.** ~~Does the runtime need deterministic
   scheduling for benchmark consistency?~~ Resolved for the
   single-pool case: it already has it, by construction — see
   § Determinism above. Deterministic *re-execution* of
   multi-pool programs is the record/replay track (GH #296);
   forcing deterministic scheduling in production is an explicit
   non-goal there.
3. **External-language toolchain access.** `hale bench
   -compare` needs `go`, `rustc`, `erlc`, etc. on PATH.
   Documenting this clearly is dev-experience work.
