# `std::source`

Source-code corpus iteration. v0 ships a single namespace
lotus — `std::source::Walk` — that drives the standard
"open a dir, parse every file matching an extension, hand
each to a per-file callback" pattern. The seed owns the
iteration mechanics; the consumer supplies the per-file fn
that says *what* to extract.

The Walk seed is the dirwalking-shape companion to
`std::cli::Resolver`. Together they let an app's `main()`
collapse to its actual specificity: configure, iterate,
report. The boilerplate (argv parsing, file iteration,
parse-with-Lang) moves into the seeds.

## Loci

### `std::source::Walk`

A namespace lotus parameterized by the source-language flavor,
the file extension to match, and the per-file callback fn.

#### Synopsis

```aperio
locus std::source::Walk {
    params {
        flavor: String = "go";
        ext:    String = ".go";
        on_file: fn(std::lang::Lang, String, Int) -> String
                = __std_source_walk_noop;
    }
    fn each_file(dir: String) -> String;
}
```

The `on_file` signature is `(lang, name, root) -> String`:

- `lang` — the configured `std::lang::Lang` locus, ready to
  classify nodes (`lang.is_fn_decl(...)`, etc.).
- `name` — the bare file name (no directory prefix).
- `root` — the root node of the parsed tree-sitter tree.

The callback's returned String is concatenated into Walk's
output verbatim. Empty returns are fine.

#### Use

```aperio
fn __my_per_file(lang: std::lang::Lang, name: String, root: Int) -> String {
    // Extract whatever this app cares about. Examples:
    //   - per-file JSON entry
    //   - tagged tower rows
    //   - newline-separated type names
    return ...;
}

fn __drive(cfg: MyConfig) {
    let w = std::source::Walk {
        flavor: cfg.flavor,
        on_file: __my_per_file,
    };
    let body = w.each_file(cfg.dir);
    // wrap body in JSON / accumulator / report
}
```

#### Iteration contract

`each_file(dir)`:

1. Lists `dir` via `std::io::fs::list_dir`. Empty / unreadable
   → returns `""` immediately.
2. For each entry whose extension equals `self.ext`:
   - Reads via `std::io::fs::read_file`. Empty file → skip.
   - Parses via `lang.parse(src)`. Parse failure → skip.
   - Resolves the root node and calls `self.on_file(lang, name, root)`.
3. Concatenates every callback's return value in iteration
   order and returns the result.

The per-file guards (empty file → skip, parse failure → skip)
mirror the hand-rolled idiom this seed replaces, so migrations
preserve behavior byte-for-byte.

#### State stays in the caller

Fn-pointer callbacks can't capture state at v0. Any caller
state that needs to flow across files — running counters,
first-vs-not-first separator flags, file-prefix stamps for
cross-directory walks — must be embedded in the returned
String fragment and post-processed by the caller after
`each_file` returns. The walker is the iteration mechanism;
the caller assembles the result.

The common shapes for state-in-fragment:

- **Tagged rows** — emit `TAG:value\n` rows; post-process with
  `std::tagged::Accumulator`.
- **Always-prepend separator** — every callback emission
  starts with the array separator; the drive loop strips
  exactly one leading separator at the boundary.
- **Sentinel prefix** — emit `ENTRY:<single-line-payload>\n`
  rows; iterate via `std::iter::Lines` after the walk.

#### Notes

- The Walk owns its own `std::lang::Lang` locus — instantiated
  per `each_file` call from `self.flavor`. Don't pass a Lang
  in; pass the flavor.
- Multiple `each_file` calls on one Walk instance are
  supported. Each call constructs and dissolves its own Lang.
- The walker has no birth/run/dissolve. `each_file` is the
  single public method; the iteration runs synchronously.

## See Also

- [`std::lang`](./lang.md) — the `Lang` locus the walker
  instantiates for parsing.
- [`std::iter`](./iter.md) — line-cursor iteration that
  callers use to post-process the walker's tagged output.
- [`std::io::fs`](./io/fs.md) — `list_dir`, `read_file`,
  `extension` are the underlying primitives.
