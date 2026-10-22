# Testing

A test in Hale is an ordinary standalone `.hl` file named `*_test.hl` whose `fn main()` runs assertions from `std::test`. `hale test` discovers all `*_test.hl` files, compiles each to a native binary in a temporary directory, and executes them in parallel.

```hale
// arith_test.hl
fn main() {
    std::test::assert(2 + 2 == 4, "trivial arithmetic");
    std::test::assert_eq_int(6 * 7, 42, "product");
    std::test::assert_eq_str("con" + "cat", "concat", "string concat");
}
```

---

## Assertions & CLI Reference

### `std::test` Assertions

| Assertion Function | Signature | Pass Condition & Failure Output |
|---|---|---|
| `std::test::assert` | `fn(Bool, String)` | Passes when `cond` is `true`; prints `ASSERTION FAILED: <msg>` on `false` |
| `std::test::assert_eq_int` | `fn(Int, Int, String)` | Passes when `actual == expected`; prints `expected` vs. `actual` diff on mismatch |
| `std::test::assert_eq_str` | `fn(String, String, String)` | Passes when `actual == expected`; prints quoted `expected` vs. `actual` diff on mismatch |
| `std::test::assert_near` | `fn(Float, Float, Float, String)` | Passes when `actual == expected` or `\|actual - expected\| <= tolerance` (`NaN` always fails) |

### Toolchain Commands & Environment Variables

| Command / Variable | Description |
|---|---|
| `hale test [dir]` | Discovers, compiles, and runs all `*_test.hl` files under the current directory or `dir` |
| `hale test -run <substr>` | Runs only test files whose path contains `<substr>` |
| `hale test --json` | Outputs one JSON record per file (`file`, `status`, `elapsed_ms`, and `message` on failure) |
| `hale test -j <N>` / `HALE_TEST_JOBS=N` | Runs at most `N` test files in parallel (default: one worker per CPU core) |
| `HALE_TEST_KEEP_VAULT=1` | Preserves each test's isolated `HALE_VAULT_DIR` and `XDG_DATA_HOME` after completion and prints its path |
| `hale check --workspace <dir>` | Typechecks every seed and every `*_test.hl` file as its own standalone program |
| `hale bench [dir]` | Runs `*_bench.hl` benchmarks (`bench_<name>` functions), reporting calibrated `ns/op` and `allocs/op` |
| `hale verify <dir>` | CI gate: runs `hale check` analysis and fails with a non-zero exit code on any finding |
| `hale fmt [--check] <path>` | Formats `.hl` files into canonical layout (or verifies formatting in CI with `--check`) |
| `hale doc` | Renders API documentation from `///` doc comments |

---

## Test Execution Model & Seed Isolation

### Pass / Fail Contract

- **Pass**: The test binary runs to completion and exits with status `0` while writing **nothing to `stdout`**. Output written to `stderr` (`eprintln`) is treated as diagnostic output and does not fail a quiet test.
- **Fail**: The first failing assertion prints `ASSERTION FAILED: <msg>` (plus `expected` and `actual` values for `assert_eq_*` / `assert_near`), stops further test assertions, unwinds active loci, and exits with status `1`.
- **Direct execution**: Because every test file is a standard Hale program, you can also run a test file directly with `hale run arith_test.hl`.

### Seed Isolation & Imports

Each `*_test.hl` file is compiled as its own independent program (the test file plus the modules reached by its `import` statements) and is **never** bundled into the seed of its enclosing directory:
- Placing `main_test.hl` alongside `main.hl` does not conflict with `main.hl`'s `fn main()`.
- Multiple `*_test.hl` files inside a `tests/` directory do not share top-level scope or conflict with one another.
- To test code in a parent directory from `tests/`, import the parent seed: `import ".." as app;`.
- `hale check <dir>` typechecks the seed excluding `*_test.hl` files (and reports the count of excluded test files); `hale check --workspace <dir>` typechecks every seed and every `*_test.hl` file.

---

## Teardown & Resource Cleanup on Failure

When an assertion fails, the process does **not** abort immediately. After printing the failure diagnostic, `main` unwinds through its normal shutdown sequence (`spec/testing.md` § "What runs after a failed assertion"):
1. Worker pools are joined and the message bus drains.
2. Every locus `let`-bound in `main` (as well as the `main locus`, its fields, and `pinned` children) executes its `dissolve()` hook in reverse declaration order.
3. The process exits with code `1`.

Place external resource cleanup (killing spawned child processes, removing temporary directories or lock files) inside a guard locus's `dissolve()` block so failed tests always clean up after themselves:

```hale
locus Sandbox {
    params {
        dir: String = "";
        pid: Int = -1;
    }
    dissolve() {
        // Runs on every exit from main, including after a failed assertion
        let _stop = std::process::run(f"sh\n-c\nkill {self.pid}; rm -rf {self.dir}")
            or std::process::ProcessOutput { code: -1, signal: 0, stdout: "", stderr: "" };
    }
}

fn fetch(url: String) -> String {
    let resp = std::http::get(url) or { return ""; };
    return std::str::from_bytes(resp.body);
}

fn main() {
    let dir = f"/tmp/my_suite_{std::process::pid()}";
    std::io::fs::mkdir(dir) or discard;
    let srv = std::process::spawn("./build/server") or raise;
    let sandbox = Sandbox { dir: dir, pid: srv.pid };

    std::test::assert_eq_str(fetch("http://127.0.0.1:8080/health"), "ok",
        "server is up");
}
```

### Teardown Rules & Edge Cases

- **Explicit cleanup**: `dissolve()` only cleans up what your code tells it to clean up; the runtime does not automatically terminate child processes spawned via `std::process::spawn`.
- **Failures outside `fn main()`**:
  - An assertion failing inside a helper `fn`, a locus method, or a synchronous `run()` unwinds and dissolves all loci owned by `main` before exiting `1`.
  - An assertion failing on a `pinned` thread or separate worker pool finishes executing the current handler body (with subsequent assertions in that body acting as no-ops) and halts the test at `main`'s next `sleep` or `yield`.
- **`SIGKILL` (`kill -9`)**: If the operating system or an external watchdog sends `SIGKILL`, the kernel terminates the process immediately without running `dissolve()` or `atexit` handlers.

---

## Running Suites, Isolation & Determinism

### Temporary Build Artifacts & `--json` Output

- **Clean working tree**: `hale test` compiles all test binaries and object files inside a private temporary directory under `$TMPDIR` (`hale-test-<pid>-…`, mode `0700`) and deletes it when the run completes (`hale run` and `hale replay` similarly use `hale-run-…` and `hale-replay-…`). To keep a compiled binary on disk, use `hale build ... -o <path>`.
- **Canonical `--json` paths**: In `hale test --json`, each record's `file` field is the canonical absolute path (symlinks resolved, no `..`), matching `hale check --json` so results from both commands can be joined by path. Interactive terminal output (`ok` / `FAIL`) preserves the relative path spelling you passed on the command line.

### Parallel Execution & Isolated Vaults / Habitats

- **Parallel execution**: `hale test` compiles and runs test files in parallel (`-j N` or `HALE_TEST_JOBS=N`), buffering output per test and printing results in deterministic sorted file order. Avoid hardcoded TCP ports or fixed `/tmp` filenames that could collide across concurrent test processes.
- **Per-test vault and habitat**: For each test file, `hale test` creates a fresh private directory for `HALE_VAULT_DIR` (used by `std::secret`) and sets `XDG_DATA_HOME` inside it. Any secrets created or organisms birthed/migrated during a test are isolated to that temporary habitat, and any compose projects started by the test are torn down when the vault is removed. Set `HALE_TEST_KEEP_VAULT=1` to retain the vault and habitat directories for inspection.

### Deterministic Single-Pool Scheduling

A test whose loci all run on the default `main` scheduler (with no `placement { }` block or additional pools) is **deterministic by construction**: a single pool runs on a single thread and executes bus handlers to completion, so every message publish and delivery occurs in the exact same order on every run.

If a single-pool test exhibits flakiness, the nondeterminism comes from an external input (wall-clock time, OS entropy, environment variables, or network I/O). Annotate functions under test with [`@deterministic`](../effects.md) to have `hale check` statically reject any dependency on nondeterministic effects.

---

## Runtime Invariants (`closure`) & Benchmarks (`hale bench`)

- **Continuous runtime invariants**: Unit tests assert point-in-time values. To assert stateful invariants that must hold continuously across a locus's lifetime (such as `"debits == credits"` or `"len <= capacity"`), declare [invariant closures](../services/failure.md#declaring-an-invariant-closure) on the locus (see [Verification](../verification.md)).
- **Microbenchmarks (`hale bench`)**: Name benchmark files `*_bench.hl` and define `bench_<name>` functions. `hale bench` calibrates iteration counts automatically and reports `ns/op` and `allocs/op`. If a benchmark function takes a fixture parameter (`fn bench_parse(f: Fixture)`), define a matching `fn setup_parse() -> Fixture`; the runner builds the fixture before each batch outside the timed region.
