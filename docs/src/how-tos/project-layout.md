# Project layout & build commands

Aperio doesn't have a `src/` directory, a build directory, or
package metadata beyond an optional `aperio.toml`. The directory
on disk *is* the project. This page covers the three project
shapes you'll meet in practice and the four commands that
operate on them.

## The three project shapes

### One file

The smallest legal project is one `.ap` file.

```sh
aperio run hello.ap
aperio build hello.ap          # produces ./hello
```

`aperio run` parses, typechecks, and interprets the file via the
tree-walking interpreter. `aperio build` produces a native
binary at `./<stem>` (here, `./hello`).

### One directory (a "seed")

Drop multiple `.ap` files in one directory; every top-level
declaration in every file becomes visible to every other file
in the directory. This is Aperio's substitute for "a package" —
the directory **is** the seed.

```
myapp/
  types.ap        // type Trade { ... }
  helpers.ap      // fn notional(t: Trade) -> Decimal { ... }
  main.ap         // fn main() { ... }
```

```sh
aperio build myapp/            # binary lands at myapp/myapp
./myapp/myapp
```

No `import` statement needed inside one seed. The compiler
flattens all top-level decls into one shared scope, in
alphabetical file order (file order is not observable — name
resolution is order-free within a seed).

### Multiple directories (cross-seed imports)

A second seed can be referenced from the first via
`import "<path>" as <alias>;`.

```
myapp/
  shared/
    topics.ap      // topic Beat { payload: Tick; }
  publisher/
    main.ap        // import "shared" as shared;
  subscriber/
    main.ap        // import "shared" as shared;
```

Import-path resolution looks (in order):

1. `<importer-dir>/<path>.ap` — a sibling `.ap` file.
2. `<importer-dir>/<path>/` — a sibling directory.
3. `<workspace-root>/<path>/` — the directory the build was
   invoked from, as a fallback.

So with the layout above and `cd myapp/`, `aperio build
publisher/` resolves `import "shared"` to `myapp/shared/`.
References cross the import boundary as `shared::Beat`.

(`std::*` paths are special-cased — never `import` them.)

## The four commands

| Command | What it does |
|---|---|
| `aperio check <file-or-dir>` | Parse + typecheck. No interpretation, no codegen — fastest path to "does this compile?" |
| `aperio run <file-or-dir>` | Parse + typecheck + interpret. Fast feedback; no binary produced. |
| `aperio build <file-or-dir>` | Parse + typecheck + emit a native binary via LLVM. |
| `aperio fetch [repo-root]` | Clone git dependencies declared in `aperio.toml` into `vendor/`. |

`check` is the type-only validator; `run` is the interpreter;
`build` is the native compiler. For "did I just break this
file?" use `check`. For single-file scripts and exploration, `run`
is faster. For anything that ships, `build`.

(The CLI also exposes `aperio lex <file.ap>` and
`aperio parse <file.ap>` for printing tokens / AST — useful when
debugging the compiler itself, but not part of the day-to-day
authoring loop.)

(`run` ignores `import` paths — if your program uses
cross-seed imports, use `build`.)

For programmatic tests against `std::test::assert*`, write
them as ordinary `.ap` programs whose `main` runs the
assertions, then invoke them via `aperio run` or `aperio
build` in your CI pipeline.

## Dependencies & vendoring

Declare git deps in `aperio.toml` at the seed root:

```toml
[deps]
pond-protocol = { git = "https://github.com/aperio-lang/pond-protocol", rev = "main" }
```

Run `aperio fetch` once; it clones each dep into
`vendor/<name>/` and writes `aperio.lock` pinning the resolved
commit. Commit both `aperio.toml` and `aperio.lock`. Reference
the dep in code via the directory path:

```aperio
import "vendor/pond-protocol" as proto;

fn main() {
    let msg = proto::Message { id: 1 };
}
```

`vendor/` is toolchain-managed — never edit files inside it.
Hand-maintained libraries that aren't fetched live in `lib/`,
which `aperio fetch` never touches.

## File naming

- `*.ap` — Aperio source.
- `aperio.toml` — manifest. Optional.
- `aperio.lock` — fetch output. Auto-generated.

No mandatory file. A seed with a single `main.ap` is a complete
project.

## See also

- [Install](../getting-started/install.md) — install the
  compiler.
- [Your first locus](../getting-started/first-locus.md) — the
  smallest possible program.
- [Multi-binary bus](./multi-binary-bus.md) — when two seeds
  need to share types AND coordinate over a network bus.
