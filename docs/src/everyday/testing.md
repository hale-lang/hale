# Testing

A test in Hale is just an ordinary program. There's no framework to
learn, no annotations, no test classes — a test is a `.hl` file whose
`fn main()` runs some assertions, and `hale test` finds and runs them.

## Writing a test

Name a file `*_test.hl` and write assertions in `main`:

```hale
// arith_test.hl
fn main() {
    std::test::assert(2 + 2 == 4, "trivial arithmetic");
    std::test::assert_eq_int(6 * 7, 42, "product");
    std::test::assert_eq_str("con" + "cat", "concat", "string concat");
}
```

`std::test` ships three primitives, all written in Hale itself:

| Assertion | Passes when… |
|---|---|
| `assert(cond, msg)` | `cond` is `true` |
| `assert_eq_int(a, b, msg)` | `a == b` (with an `expected / actual` diff on failure) |
| `assert_eq_str(a, b, msg)` | `a == b` |

The contract is exit-code based, and it's the whole model:

- **Pass** — the program runs to completion and exits `0` with no
  output. A silent test has passed.
- **Fail** — the first failing assertion prints
  `ASSERTION FAILED: <msg>` (and, for `assert_eq_*`, the expected and
  actual values) and exits non-zero immediately. There's no "collect
  all failures" — the first one stops the run.

Because a test is an ordinary binary, you can also just run it
directly: `hale run arith_test.hl` passes silently or prints the
failure.

## Running the suite

```sh
hale test               # discover + run every *_test.hl under the cwd
hale test tests/        # ...under a directory
hale test -run concat   # only files whose name matches a substring
hale test --json        # machine-readable results (one record per file)
```

`hale test` compiles each discovered file to a native binary and runs
it, reporting which passed and which failed. It's the same binary that
`hale build` produces — there's no separate test runtime.

## Testing what runs over time

The assertions above check *values*. For a long-running locus, the
property you want to hold isn't a single value but an **invariant** —
"debits always equal credits," "the buffer never exceeds capacity."
Those are [closures](../services/failure.md#declaring-an-invariant-closure):
you declare the invariant on the locus, and the runtime audits it as
the program runs. Closures are part of Hale's
[verification](../verification.md) story, not its testing library —
but they're how you assert the things a unit test can't reach.

The rest of the toolchain rides alongside: `hale bench` runs
`*_bench.hl` benchmarks (zero-param `bench_*` fns, self-calibrated
ns/op + allocs/op), `hale verify` is the CI gate (identical
analysis to `check`, but *any* finding fails), `hale fmt` keeps
everything canonical (`--check` in CI), and `hale doc` renders API
references from `///` comments. Everything ships in the one
binary.
