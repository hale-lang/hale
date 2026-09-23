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

- **Pass** — the program runs to completion and exits `0` with nothing
  on stdout. A silent test has passed. Only stdout is inspected: what
  the program writes to stderr (`eprintln`) is diagnostic output and
  doesn't count, so print progress there if you want it.
- **Fail** — the first failing assertion prints
  `ASSERTION FAILED: <msg>` (and, for `assert_eq_*`, the expected and
  actual values) and the program exits non-zero. There's no "collect
  all failures" — the first one stops the run.

Because a test is an ordinary binary, you can also just run it
directly: `hale run arith_test.hl` passes silently or prints the
failure.

## A failing test still cleans up after itself

"The first failure stops the run" doesn't mean the program is shot
where it stands. A failing assertion prints its line, records the
failure, and then `main` **exits the way it always does** — pools
joined, bus drained, and every locus you `let`-bound in `main`
dissolved in reverse order — before the process returns `1`.

That matters the moment a test owns something real. Suppose a test
starts a server under test and writes into a scratch directory:

```hale
locus Sandbox {
    params {
        dir: String = "";
        pid: Int = -1;
    }
    dissolve() {
        // Runs on every exit from main — including a failed
        // assertion. Cleans exactly what this test created.
        let _stop = std::process::run(f"sh\n-c\nkill {self.pid}; rm -rf {self.dir}")
            or std::process::ProcessOutput { code: -1, signal: 0, stdout: "", stderr: "" };
    }
}

fn main() {
    let dir = "/tmp/my_suite_scratch";
    std::io::fs::mkdir(dir) or discard;
    let srv = std::process::spawn("./build/server") or raise;
    let sandbox = Sandbox { dir: dir, pid: srv.pid };

    std::test::assert_eq_str(fetch("/health"), "ok", "server is up");
}
```

If `assert_eq_str` fails, `Sandbox.dissolve()` still runs: the server
is stopped and the scratch directory is gone. The next run starts
clean. Put cleanup in a `dissolve()` and you never need a hand-rolled
exit watcher.

Three edges are worth knowing:

- **Only what you own.** `dissolve()` is your code; it cleans your
  child, your directory, your lock file. Nothing cleans up on your
  behalf.
- **Assertions belong in `main`.** An assertion that fails inside a
  helper `fn` or a locus method has no `main` frame to return
  through, and ends the process on the spot without the dissolve
  cascade. Keep the assertions that guard owned resources at the top
  level.
- **`kill -9` is different.** If something SIGKILLs the test process,
  nothing runs — not `dissolve()`, not an atexit hook. That's the
  platform, not Hale; an external reaper is the only answer.

`spec/testing.md` § "What runs after a failed assertion" is the
normative version of this.

## Running the suite

```sh
hale test               # discover + run every *_test.hl under the cwd
hale test tests/        # ...under a directory
hale test -run concat   # only files whose name matches a substring
hale test --json        # machine-readable results (one record per file)
hale test -j 1          # one file at a time (default: one per core)
```

Each `--json` record carries `file`, `status`, `elapsed_ms` and — on
a failure — `message`. The `file` is that test's **canonical** path
(absolute, symlinks resolved, no `..`), whatever spelling you typed,
so records from `hale test --json` and `hale check --json` join on
it; the `ok` / `FAIL` lines you read on a terminal keep the spelling
you typed.

`hale test` compiles each discovered file to a native binary and runs
it, reporting which passed and which failed. It's the same binary that
`hale build` produces — there's no separate test runtime.

Files compile and run **in parallel**, one per available core unless
you say otherwise with `-j N` (or `HALE_TEST_JOBS=N`). The report does
not depend on it: the lines come out in sorted file order once every
file is done, and whatever a test prints stays with that test. What
parallelism does ask of a test is that it own what it touches — a
test that listens on a fixed port or writes a fixed path under `/tmp`
can collide with its neighbour. Take a free port and a per-run
scratch directory instead.

One property comes free with that binary: **a test whose loci all
run on the main scheduler is deterministic.** No `placement`, no
extra pools — then every publish and every delivery happens in the
same order on every run, by construction (one pool is one consumer
thread; handlers run to completion). A flaky single-pool test is
never the scheduler's fault: look for a real input — time, entropy,
environment, the network — feeding the assertion instead. If you
want the compiler to prove there isn't one, mark the code under
test [`@deterministic`](../effects.md).

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
