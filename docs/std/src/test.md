# `std::test`

Assertion primitives for tests written in Aperio. Phase 2 v0.1
(m87) ships three: a general boolean assertion plus two
shape-specialized equality assertions for Int and String.

The implementations are written purely in Aperio — they compose
on `std::process::exit` and `println`. The test-runner contract
is:

- **Pass:** the program exits 0 with no assertion-related
  output.
- **Fail:** the program prints `ASSERTION FAILED: <msg>` (plus
  expected/actual detail for `assert_eq_*`) and exits non-zero.

A test program is therefore an ordinary Aperio binary; any
process-level test runner (the Rust integration harness today,
a future `aperio test` CLI tomorrow) consumes the exit code.

## Functions

### `std::test::assert`

#### Synopsis

```aperio
fn assert(cond: Bool, msg: String)
```

Asserts that `cond` is `true`. If `false`, prints
`ASSERTION FAILED: <msg>` to stdout and exits 1.

#### Semantics

- Successful assertions are silent. A test program that runs
  to completion with no assertion output and exit 0 has
  passed.
- The exit happens via `std::process::exit(1)`, which short-
  circuits the lifecycle dispatcher — no `dissolve()` blocks
  run after a failing assertion.
- `msg` is printed verbatim. For multi-piece messages,
  pre-format with String concat: `assert(ok, "step " + label + " failed")`.

#### Examples

```aperio
fn main() {
    let n = std::str::parse_int("42");
    std::test::assert(n == 42, "parse_int round-trips");
    std::test::assert(n > 0, "parsed value should be positive");
}
```

### `std::test::assert_eq_int`

#### Synopsis

```aperio
fn assert_eq_int(actual: Int, expected: Int, msg: String)
```

Asserts that `actual == expected` as Ints. On failure, prints
the message plus both values, then exits 1.

#### Semantics

- On failure the diagnostic format is:
  ```
  ASSERTION FAILED: <msg>
    expected: <expected>
    actual:   <actual>
  ```
- Argument order is `(actual, expected, msg)`. Reversing the
  first two compiles but produces backwards diagnostics.
- For strict-not-equal, no `assert_neq_int` ships in v0.1;
  use `std::test::assert(a != b, "...")`.

#### Examples

```aperio
fn main() {
    let r = std::str::parse_int("10");
    std::test::assert_eq_int(r, 10, "parse_int(\"10\")");
    std::test::assert_eq_int(len("hello"), 5, "len of hello");
}
```

### `std::test::assert_eq_str`

#### Synopsis

```aperio
fn assert_eq_str(actual: String, expected: String, msg: String)
```

Asserts that `actual == expected` as Strings. On failure prints
the message plus both values quoted, then exits 1.

#### Semantics

- On failure the diagnostic format is:
  ```
  ASSERTION FAILED: <msg>
    expected: "<expected>"
    actual:   "<actual>"
  ```
- String equality is byte-exact; trailing whitespace or
  embedded NULs matter.
- Argument order is `(actual, expected, msg)`.

#### Examples

```aperio
fn main() {
    let s = "hello, " + "world";
    std::test::assert_eq_str(s, "hello, world", "string concat");
    std::test::assert_eq_str(to_string(42), "42", "to_string(Int)");
}
```

## Writing tests in Aperio

A typical Aperio test program is one `main()` that runs a series
of assertions and exits 0 on success:

```aperio
fn main() {
    // Setup
    let n = compute_something();

    // Assertions (each exits 1 on failure)
    std::test::assert(n > 0, "n is positive");
    std::test::assert_eq_int(n, 42, "exact answer");

    // Reaching here = pass; default exit is 0
}
```

The Rust integration harness (`crates/aperio-codegen/tests/`)
spawns these binaries and asserts on exit code + stdout.
`tests/aperio_self_test.rs` is the canonical pattern: a Rust
test runs a `.ap` program and expects exit 0 plus no
"ASSERTION FAILED" line in stdout.

## Limitations (v0.1)

- **No `aperio test` CLI runner.** Tests run via the workspace
  Rust integration harness (`cargo test`). A native CLI is
  Phase 2 v1.0.
- **No `assert_rejects` (compile-error tests).** Asserting
  that a snippet fails to compile needs a compiler-level
  surface; deferred to v1.0.
- **No `assert_closure`** for closure-test introspection.
- **No `assert_neq_*`** siblings. Use the general `assert`
  with `!=` for now.
- **No fake time / fake bus / fake fs.** Tests run against
  real time and real I/O. Concurrent tests must pick unique
  ports and tmpfile paths to be parallel-safe.
- **No property-based testing.** Explicitly deferred per
  spec.

## See Also

- [Roadmap](./roadmap.md) — Phase 2 v1.0 plan.
- [`std::process`](./process/index.md) — `exit` is the
  primitive these compose on.
- `crates/aperio-codegen/tests/aperio_self_test.rs` (in the
  language repo) — the runner pattern that consumes this
  surface.
