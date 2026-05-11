# `std::tagged`

Parsing helpers for the **tagged accumulator** shape — multi-
line strings where each line is `TAG:body`. Extractors in the
codebase-onboarder family produce this representation
pervasively (`IMPORT:log`, `TYPE:Server`, `WIRE:handler|/path`,
etc.), and several apps were hand-rolling identical scan loops
before this extraction lifted them into the std seed.

The accumulator shape is the v0 stand-in for `List<TaggedRow>`:
once generics ship and a proper list type lands, this module
either dissolves or evolves into the typed equivalent.

## Loci

### `std::tagged::Accumulator`

A namespace lotus with empty `params { }`. Each method walks
the accumulator once using `std::iter::Lines` and returns a
domain-shaped result.

#### Synopsis

```aperio
locus std::tagged::Accumulator {
    fn count(acc: String, tag: String) -> Int;
    fn first_body(acc: String, tag: String) -> String;
    fn each_body(acc: String, tag: String) -> String;
    fn collect_csv(acc: String, tag: String) -> String;
    fn collect_array(acc: String, tag: String) -> String;
}
```

#### Method semantics

- **`count(acc, tag)`** — number of lines whose tag matches.
- **`first_body(acc, tag)`** — body of the first matching
  line, or `""` when no line matches. Useful for tags that
  appear at most once (e.g. a `PKG:` row per file).
- **`each_body(acc, tag)`** — every matching body joined with
  `\n`. Re-iterate via `std::iter::Lines` when the caller
  needs per-body processing.
- **`collect_csv(acc, tag)`** — bare bodies joined with
  `, ` (comma + space). For human-readable report prose
  (`log, net/http, os`).
- **`collect_array(acc, tag)`** — JSON array of double-quoted
  bodies. Bodies are inserted verbatim — embedded `"` or `\`
  characters need escaping by the caller (or by a future
  `std::json::Builder.quote` once that surfaces).

#### Split rules

- The tag is the prefix up to the **first** `:`. Everything
  after that colon, including any further `:` characters, is
  the body (`WIRE:handler|/api:v1` → tag `WIRE`, body
  `handler|/api:v1`).
- Blank lines never match (their length is less than
  `len(tag) + 1`).
- Comment lines are **not** filtered. Callers that want to
  skip them should preprocess via `std::iter::Lines.is_skippable`.

## See Also

- [`std::iter`](./iter.md) — the underlying line walker. Use
  it to re-iterate the result of `each_body`.
- `notes/aperio-refactor-proposal.md` — the duplication
  inventory that motivated this extraction.
