# `std::iter`

Iteration primitives. v0 ships a single namespace lotus —
`std::iter::Lines` — that walks a newline-separated `String`
using a cursor shape (advance an index, ask for the line at
that index). The cursor shape is honest about today's
language: `fn`-pointer callbacks cannot capture state, so a
callback-shaped iterator would push every caller into a
bus-route or rebuild-state-inside-the-closure dance. A
`Lines::each(s, fn)` method lands as a backward-compatible
addition once closures-with-state arrive.

This module replaces the 6-line newline-iteration boilerplate
that a half-dozen apps were hand-rolling — see
`notes/aperio-refactor-proposal.md` for the duplication
inventory that motivated the extraction.

## Loci

### `std::iter::Lines`

A namespace lotus with empty `params { }` and three methods.

#### Synopsis

```aperio
locus std::iter::Lines {
    fn next_idx(s: String, from: Int) -> Int;
    fn line_at(s: String, from: Int) -> String;
    fn is_skippable(line: String) -> Bool;
}
```

#### Use

```aperio
let it = std::iter::Lines { };
let mut from = 0;
while from >= 0 {
    let line = it.line_at(s, from);
    from = it.next_idx(s, from);
    if it.is_skippable(line) { continue; }
    // do something with line
}
```

#### Method semantics

- **`next_idx(s, from)`** returns the index where the next line
  begins. Returns `len(s)` when the cursor is on the last line
  and that line has no trailing newline (the follow-up call
  from there returns `-1`). Returns `-1` once the input is
  consumed.
- **`line_at(s, from)`** returns the line at `from`, with the
  trailing newline stripped. Returns `""` when `from` is past
  the end — the safe value for the one-extra trailing
  iteration `next_idx` produces when the last line lacks a
  newline.
- **`is_skippable(line)`** returns `true` for blank lines and
  lines whose first character is `#`. Covers the config-file
  case; callers with different skip rules write their own
  predicate.

#### Notes

- The loop guard is `while from >= 0` because `next_idx`
  returns `-1` when there's nothing left.
- The walk produces one extra trailing iteration when the input
  ends without a newline. `is_skippable` (or an explicit
  `if line == "" { continue; }`) keeps that iteration silent.
- Instantiation is free of state (`params { }` is empty), so
  the per-locus alloc is negligible. Reuse the same `let it = ...`
  across multiple walks in the same fn body if you want.

## See Also

- [`std::str`](./str.md) — `index_of` is the underlying primitive
  `next_idx` and `line_at` build on.
- [`std::io::fs::list_dir`](./io/fs.md) — returns a
  newline-separated `String` that callers walk via this module.
