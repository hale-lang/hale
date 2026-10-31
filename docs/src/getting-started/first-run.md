# Quickstart & Toolchain

## 1. Hello World

Create `hello.hl`:

```hale
fn main() {
    let name = "Hale";
    println("Hello from ", name, ".");
}
```

Compile and execute it in one step with `hale run`:

```sh
hale run hello.hl
```

```text
Hello from Hale.
```

To produce a standalone native binary on disk, use `hale build`:

```sh
hale build hello.hl
./hello
```

`hale run` and `hale build` use the exact same LLVM compiler pipeline—there is no separate interpreter. `hale run` compiles to a temporary binary, executes it, and cleans it up.

### Scaffold a New Project

To start a multi-file project with a `hale.toml` manifest, an entry seed, and a test file:

```sh
hale init my-app
cd my-app
hale run .
hale test .
```

Passing a directory (`hale run .`, `hale check .`, `hale build .`) compiles all `.hl` files in that directory as a single **seed** (package/module unit) with shared top-level scope.

---

## 2. Basic Syntax Rules

- **Entry point:** Execution starts at `fn main()`.
- **Output:** `println(a, b, c)` concatenates any number of arguments and appends a newline. String interpolation uses `f"Hello {name}"`.
- **Semicolons:** Every statement ends with `;`. Newlines are whitespace.
- **Encoding:** Source files are ASCII outside of string literals and comments.
- **Comments:** `//` for single-line comments, `/* ... */` for block comments, and `///` for doc comments rendered by `hale doc`.

---

## 3. Essential Toolchain Commands

| Command | Description |
|---|---|
| `hale check <file\|dir>` | Parse, typecheck, and run static topology/effect/memory analyses without codegen |
| `hale verify <file\|dir>` | Strict CI gate: runs `hale check` and fails on any warning or advisory |
| `hale run <file\|dir> [args...]` | Compile and immediately execute |
| `hale build <file\|dir> [-o <out>]` | Compile to a native binary (or `.wasm` + `.mjs` with `--target wasm32`) |
| `hale test <file\|dir>` | Compile and run `*_test.hl` test suites |
| `hale bench <file\|dir>` | Compile and run `*_bench.hl` benchmarks (`ns/op`, `allocs/op`) |
| `hale fmt <file\|dir>` | Format source code in place (zero-config, 100-column structural line breaking) |
| `hale doc [<file\|dir> \| --stdlib]` | Render documentation from `///` comments or inspect the standard library (`--stdlib`) |
| `hale lsp` | Start the built-in Language Server Protocol (LSP) server over stdio |
| `hale mcp [--app <endpoint>]` | Start the Model Context Protocol (MCP) server for AI agents |

---

## 4. Build Modes, Tracing & Debugging

### Fast Iteration (`--dev`)

By default, `hale build` and `hale run` compile with `-O3` tuned for the host CPU. During rapid edit-compile-run loops, enable the lighter optimization pipeline:

```sh
hale build --dev app.hl
# or via environment variable:
HALE_DEV=1 hale run app.hl
```

### Build Timing & Lifecycle Tracing

| Environment Variable | Effect |
|---|---|
| `HALE_TIME=1` | Prints per-phase compiler wall times during `hale build` / `hale run`. |
| `HALE_LIFECYCLE_TRACE=1` | Instruments the binary to log every locus `birth`, `run`, `drain`, `dissolve`, `reclaim`, and pool join to `stderr` with instance IDs, restart incarnations, and thread IDs. |
| `LOTUS_NO_DEBUGINFO=1` | Omits DWARF debug symbols from the emitted binary. |

### Native Debugging with `gdb` / `lldb`

Compiled binaries include full DWARF debug info by default at zero runtime cost:

- Set breakpoints by `.hl` file and line (`break app.hl:42`).
- Inspect backtraces with original Hale source locations.
- Print local variables and typed `String` values with `info locals` / `print x`.
- AddressSanitizer (`LOTUS_ASAN=1`) reports exact `.hl` source lines.

---

## 5. Editor Integration (`hale lsp` & `--json`)

### Language Server (`hale lsp`)

Point any LSP-compatible editor (VS Code, Neovim, Helix, Zed, Emacs) at `hale lsp`. It requires zero configuration and provides:

- **Live incremental diagnostics:** Type errors, effect violations, placement errors, and memory-bound advisories update as you type. On programs with `claims`, type errors report first, followed by claim verdicts.
- **Hover & completions:** Displays function signatures, `fallible(E)` contracts, `@hot`/`@budget` annotations, topic routing keys, `self.` members, and `std::` symbols.
- **Go-to-definition & references:** Resolves across your seed, imported libraries, and directly into the compiler's embedded `std::` standard library source.
- **Custom topology queries:** Exposes `hale/busGraph`, `hale/placement`, and `hale/allocSummary` LSP requests for inspecting the pub/sub graph, thread pool mapping, and arena allocation sites.

### Machine-Readable Diagnostics (`--json`)

For scripts, save hooks, or custom tooling, pass `--json` to `hale check`:

```sh
hale check app.hl --json
```

This emits one NDJSON object per diagnostic (`file`, `line`, `col`, `severity`, `kind`, `message`) on `stdout`. An empty output stream guarantees a clean check.

Next: **[Tutorial: Build a Job Queue](../tutorial/job-queue.md)** or jump straight to **[Values, Types & Variables](../basics/values.md)**.
