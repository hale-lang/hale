# Your first run

> Put something on screen.

Create a file `hello.hl`:

```hale
fn main() {
    println("Hello from Hale.");
}
```

Run it:

```sh
hale run hello.hl
```

```
Hello from Hale.
```

`hale run` compiles your program and runs it in one step — it's
the same native code `hale build` produces, just executed
immediately and not left on disk. When you want the artifact to
keep and ship, build it:

```sh
hale build hello.hl
./hello
```

Same compiler, same output: `run` is the fast inner-loop shape,
`build` is for the binary you deploy. There's no separate
interpreter, so anything that runs under `build` runs identically
under `run`.

## What's here

- **`fn main()`** is the entry point, the same as it is in C, Go,
  or Rust. A Hale program starts by calling it.
- **`println(...)`** prints its arguments followed by a newline.
  It takes *any number* of arguments and concatenates them —
  there's no format string:

  ```hale
  fn main() {
      let name = "Hale";
      println("Hello from ", name, ".");
  }
  ```

- **Statements end with `;`.** Newlines are just whitespace —
  they don't end statements. Source is ASCII outside of string
  literals and comments.

Comments are C-style:

```hale
// a line comment
/* a block comment */
```

That's the whole surface you need to start. The next chapter
introduces variables and the value types — the vocabulary every
Hale program is built from.

> **`hale run` and imports.** A single file's `import "..." as
> ...;` directives are resolved by `hale run` just as `hale build`
> resolves them. The one gap is the ad-hoc *directory* form (`hale
> run ./dir`), which bundles the directory's files without
> cross-seed import resolution — use `hale build ./dir` for a
> multi-file project that imports libraries.

## Build modes, diagnostics, and debugging

A few switches worth knowing from day one:

- **Faster iteration:** `hale build --dev` (or `HALE_DEV=1`) uses a
  lighter optimization pipeline — noticeably quicker builds while
  you're in an edit-run loop. Release builds default to `-O3`
  tuned for your CPU.
- **Where did the build time go?** `HALE_TIME=1 hale build app.hl`
  prints per-phase wall times.
- **Editor & agent integration:** `hale lsp` is a stdio Language
  Server — point any LSP-speaking editor (or agent harness) at it
  and you get live diagnostics: type errors as errors, the
  advisory analyses (unbounded-alloc survey, hot-path lint,
  placement warnings) as warnings, re-checked whole-program on
  every keystroke because the check runs in ~10 ms. Hover shows
  signatures with their contracts (fallibility, `@hot`/`@budget`
  status, a topic's routing key), completion covers `self.`
  members, the `std::` surface, and your seed's symbols,
  go-to-definition and references work across the seed, and the custom requests `hale/busGraph`,
  `hale/placement`, and `hale/allocSummary` return the pub/sub
  topology, the thread/pool map, and the allocation survey's leak
  sites. No configuration. Prefer plain JSON?
  `hale check app.hl --json` emits one object per diagnostic
  (file, line, col, severity, message) on stdout — a save-hook is
  all a minimal integration needs.
- **Real debugging:** binaries carry DWARF line tables by default —
  `gdb ./app`, `break app.hl:42`, backtraces with real file:line,
  and ASAN reports that point at the exact source line. Zero
  runtime cost; opt out with `LOTUS_NO_DEBUGINFO=1`.
