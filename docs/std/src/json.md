# `std::json`

Small JSON-shape helpers. Three apps had independent
`__append_entry(arr, entry)` implementations; one extra app had
a `__build_imports_array` that combined line iteration with
the same append pattern. `std::json::Builder` collects those
into one namespace lotus.

`Builder` is deliberately small. It is not a parser, a
pretty-printer, or a schema validator — just the concatenation
operations that show up when glueing JSON output together by
hand. A full JSON emitter / parser ships when the language
needs it; today's apps emit JSON as plain string
concatenation and consume it through the substrate's own
shape.

## Loci

### `std::json::Builder`

A namespace lotus with empty `params { }`.

#### Synopsis

```aperio
locus std::json::Builder {
    fn append_entry(arr: String, entry: String) -> String;
    fn quote(s: String) -> String;
    fn wrap_array(entries: String) -> String;
    fn wrap_object(fields: String) -> String;
    fn build_array(items: String) -> String;
    fn build_quoted_array(items: String) -> String;
}
```

#### Method semantics

- **`append_entry(arr, entry)`** — appends `entry` to `arr`,
  prefixing with `", "` when `arr` already has content.
  Returns the new accumulator.
- **`quote(s)`** — wraps `s` in double-quotes. v0: no
  escaping — embedded `"` or `\` characters pass through
  verbatim. App values (Go identifiers, paths, log strings)
  don't contain those, so the no-op is functionally correct
  today; the hook exists so a future escaping pass lands
  without touching every call site.
- **`wrap_array(entries)`** — `[ + entries + ]`.
- **`wrap_object(fields)`** — `{ + fields + }`.
- **`build_array(items)`** — takes a `\n`-separated list of
  pre-built entries and returns a JSON array. Blank lines
  are skipped. Use when entries are objects, numbers, or
  pre-quoted strings.
- **`build_quoted_array(items)`** — like `build_array`, but
  quotes every entry. The common case for arrays of
  identifier-like strings.

## Limitations (v0)

- **No escaping** — `quote` is a no-op wrapper. Embedded `"`
  or `\` characters will produce invalid JSON. Acceptable
  today because all current consumers pass simple
  identifier-like strings; revisit when a consumer needs
  arbitrary payloads.
- **No parsing** — there is no `Builder.parse` method. JSON
  produced by these helpers is consumed by external tools
  (apps/tower-join consumers, browser UIs to come), not by
  Aperio itself.

## See Also

- [`std::iter`](./iter.md) — `build_array` walks its input
  via `std::iter::Lines`.
- `notes/aperio-refactor-proposal.md` — the duplication
  inventory that motivated this extraction.
