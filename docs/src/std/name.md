# `std::name`

Orthography helpers — `snake_case` ↔ `CamelCase`, file-stem →
locus name. Both apps/onboard and apps/tower-join hand-rolled
identical `__propose_locus_name(file)` helpers before this
extraction lifted them into the std seed.

`std::name` is kept separate from `std::lang` because the
operations are purely orthographic: they don't depend on
which tree-sitter flavor the file came from, and they don't
need a node-kind vocabulary to do their job. `std::lang`
knows ASTs; `std::name` knows letters.

## Loci

### `std::name::Convention`

A namespace lotus with one configuration parameter
(`strip`, the source extension to peel off file names before
the orthographic transform).

#### Synopsis

```aperio
locus std::name::Convention {
    params {
        strip: String = ".go";
    }
    fn snake_to_camel(s: String) -> String;
    fn camel_to_snake(s: String) -> String;
    fn strip_extension(file: String) -> String;
    fn propose_locus_name(file: String) -> String;
}
```

#### Method semantics

- **`snake_to_camel(s)`** — `request_cache` → `RequestCache`.
  The very first character is also capitalized, so the result
  is PascalCase. Non-letter characters pass through unchanged.
- **`camel_to_snake(s)`** — `RequestCache` → `request_cache`.
  Splits before each uppercase letter (except at offset 0).
  No special acronym handling: `HTTP` → `h_t_t_p`. Refine
  later if needed.
- **`strip_extension(file)`** — strips `self.strip` from the
  end of `file`. Returns `file` unchanged when the suffix
  doesn't match.
- **`propose_locus_name(file)`** — `request_cache.go` →
  `RequestCacheL` when `strip = ".go"`. Returns `"?L"` for
  inputs that strip down to an empty stem.

#### Use

```aperio
let nc = std::name::Convention { strip: ".go" };
let proposed = nc.propose_locus_name("request_cache.go");
// proposed == "RequestCacheL"
```

## See Also

- [`std::lang`](./lang.md) — node-kind vocabulary for AST
  walks. Kept distinct from `std::name`: orthography ≠ syntax.
- `notes/aperio-refactor-proposal.md` — the duplication
  inventory that motivated this extraction.
