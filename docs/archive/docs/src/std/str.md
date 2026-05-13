# `std::str`

String-processing primitives. m78 added the integer parsers
(`parse_int`, `can_parse_int`); m84 added `index_of` for
substring search. v1.x landed the float parsers, case folding,
trim / replace, repeat / padding, and the string-builder
primitive — these resolve the long-running
`reader-list_item-quadratic-concat` friction and the
case-insensitive HTTP-header lookup gap.

Most string work in Aperio uses bare-name builtins (`len`,
`starts_with`, `contains`, `to_string`) plus the `+` operator
for concatenation, `s[start..end]` for slicing, and `f"..."`
f-strings for interpolation. The path-qualified surface here
covers the cases that need a real function call: parsing,
transformation, formatting, and amortized-O(N) accumulation.

## Quick reference

| Family | Functions | When to reach for it |
|---|---|---|
| Parsing | `parse_int` / `can_parse_int` / `parse_float` / `can_parse_float` | Strings → numbers, paired "soft" predicate. |
| Search | `index_of` | Byte offset of first occurrence; `-1` for absent. |
| Case folding | `lower` / `upper` | ASCII-only; non-ASCII passes through. |
| Trimming + substitution | `trim` / `replace` | Whitespace strip; greedy substring replace. |
| Formatting | `repeat` / `pad_left` / `pad_right` | Separator lines + column alignment. |
| Accumulation | `builder_new` / `builder_append` / `builder_len` / `builder_finish` | Amortized O(N) build-by-pieces; replaces the O(N²) `buf = buf + chunk` shape. |
| Conversion | `from_bytes` | Bytes → String. |

## Functions

### `std::str::parse_int`

#### Synopsis

```aperio
fn parse_int(s: String) -> Int
```

Parses `s` as a signed base-10 integer. Returns the parsed
value on success, `0` on parse failure or empty input. Use
`std::str::can_parse_int` to disambiguate "the string was 0"
from "the string didn't parse."

#### Semantics

- Accepts an optional leading `-` sign.
- **Strict trailing-char check:** `"42"` parses; `"42abc"`,
  `"  42  "`, and `"42\n"` all reject and return `0`. v0
  doesn't trim whitespace; callers that need lenient parsing
  trim before calling.
- Range overflow (numbers > i64 max or < i64 min) sets errno
  via strtoll; this surface treats the overflow case the
  same as parse failure and returns `0`.

#### Examples

Reading a port from argv with a default fallback:

```aperio
fn main() {
    let mut port: Int = 9876;
    if std::env::args_count() > 1 {
        let p = std::str::parse_int(std::env::arg(1));
        if p > 0 {
            port = p;
        }
    }
    println("listening on port ", port);
}
```

### `std::str::can_parse_int`

#### Synopsis

```aperio
fn can_parse_int(s: String) -> Bool
```

Returns `true` if `s` parses cleanly as a signed base-10
integer (same rules as `parse_int`), `false` otherwise. Use
this when "did the string represent a number" is the question
and the actual value is secondary.

#### Examples

```aperio
fn main() {
    let candidate: String = "42";
    if std::str::can_parse_int(candidate) {
        println("parsed=", std::str::parse_int(candidate));
    } else {
        println("not a number");
    }
}
```

### `std::str::index_of`

#### Synopsis

```aperio
fn index_of(s: String, sub: String) -> Int
```

Returns the byte offset of the first occurrence of `sub`
within `s`, or `-1` if `sub` is not present. m84.

#### Semantics

- Byte-wise search, not codepoint-aware. Multi-byte UTF-8
  sequences in either argument are matched by their byte
  pattern. For ASCII content (the common case) the
  distinction doesn't matter.
- Empty `sub` returns 0 by convention (the empty string is
  trivially present at the start).
- Returns the index of the **first** match; there is no
  `find_all` or `last_index_of` in v0.
- `sub` longer than `s` returns -1.

#### Examples

Split-on-first-occurrence pattern:

```aperio
fn main() {
    let raw = "method=GET path=/index";
    let eq = std::str::index_of(raw, "=");
    if eq >= 0 {
        let key = raw[0..eq];
        let val = raw[(eq + 1)..len(raw)];
        println("key=", key, " val=", val);
    }
}
```

Test for substring presence — but prefer the bare-name
builtin `contains`:

```aperio
fn main() {
    let url = "https://example.com/path";

    // Idiomatic — bare-name builtin:
    if contains(url, "example.com") {
        println("matches");
    }

    // index_of works too, but is more verbose for a yes/no:
    if std::str::index_of(url, "example.com") >= 0 {
        println("matches");
    }
}
```

`index_of` shines when you need the **position**, not just
presence — e.g., for splitting on a delimiter or extracting
a header before / after a separator.

### `std::str::parse_float` / `can_parse_float`

#### Synopsis

```aperio
fn parse_float(s: String) -> Float
fn can_parse_float(s: String) -> Bool
```

Same shape as `parse_int` / `can_parse_int`, but for IEEE 754
doubles. `parse_float` returns `0.0` on parse failure or empty
input; `can_parse_float` returns the bool predicate. Strict
trailing-NUL — `"3.14abc"` rejects.

#### Examples

```aperio
fn main() {
    let raw = "3.14159";
    if std::str::can_parse_float(raw) {
        let pi = std::str::parse_float(raw);
        println(f"pi ~= {pi}");
    }
}
```

### `std::str::lower` / `std::str::upper`

#### Synopsis

```aperio
fn lower(s: String) -> String
fn upper(s: String) -> String
```

ASCII case folding. Non-ASCII bytes pass through unchanged
(no Unicode case tables in the runtime at v1).

#### Examples

Case-insensitive comparison without library help:

```aperio
fn main() {
    let user_input = "Yes";
    if std::str::lower(user_input) == "yes" {
        println("confirmed");
    }
}
```

### `std::str::trim` / `std::str::replace`

#### Synopsis

```aperio
fn trim(s: String) -> String
fn replace(s: String, needle: String, replacement: String) -> String
```

`trim(s)` strips ASCII whitespace (space, tab, `\r`, `\n`) from
both ends. `replace(s, needle, replacement)` does greedy
non-overlapping substring replacement.

#### Semantics

- `replace` advances by `len(needle)` after each match (no
  overlap), and the empty-needle case is a no-op to avoid the
  infinite-replace footgun.
- Both anchor results in the bus payload arena.

#### Examples

```aperio
fn main() {
    let v = std::str::trim("   hello world   \r\n");
    println(f"[{v}]");                              // [hello world]

    let r = std::str::replace("foo bar foo", "foo", "FOO");
    println(r);                                     // FOO bar FOO
}
```

### `std::str::repeat` / `pad_left` / `pad_right`

#### Synopsis

```aperio
fn repeat(s: String, n: Int) -> String
fn pad_left(s: String, width: Int, pad: String) -> String
fn pad_right(s: String, width: Int, pad: String) -> String
```

`repeat` returns N copies of `s` concatenated (N ≤ 0 → empty).
`pad_left` / `pad_right` align `s` to `width` using the first
byte of `pad` as the fill character. **No truncation** —
strings already at or over `width` come back unchanged.

#### Examples

Drawing a separator line + right-aligned numeric columns:

```aperio
fn main() {
    println(std::str::repeat("─", 40));
    println(std::str::pad_left("42",   8, " ") + " widgets");
    println(std::str::pad_left("1024", 8, " ") + " sprockets");
    println(std::str::repeat("─", 40));
}
```

### `std::str::builder_*`

#### Synopsis

```aperio
fn builder_new() -> Bytes              // opaque handle
fn builder_append(b: Bytes, s: String) // statement-position only
fn builder_len(b: Bytes) -> Int
fn builder_finish(b: Bytes) -> String  // materializes + frees
```

Amortized O(N) string accumulation, replacing the O(N²) shape
that `buf = buf + chunk` collapsed to under Aperio's
arena-anchored immutable Strings. The handle is `Bytes`-shaped
(opaque — users only pass it between the `builder_*` fns).

#### Semantics

- The buffer doubles on overflow (initial cap 64 bytes).
- `builder_finish` copies the accumulated bytes into the bus
  payload arena, frees the builder, and returns the
  NUL-terminated String. The handle must NOT be reused after
  `builder_finish`.
- Forgetting to call `builder_finish` leaks the builder. The
  shape fences this off in practice — every `builder_new` is
  dominated by a `builder_finish` in the same scope.

#### Examples

Streaming a CSV row out of a loop without quadratic concat:

```aperio
fn join_csv_row(values: [String; 8]) -> String {
    let b = std::str::builder_new();
    let mut i = 0;
    while i < 8 {
        if i > 0 {
            std::str::builder_append(b, ",");
        }
        std::str::builder_append(b, values[i]);
        i = i + 1;
    }
    return std::str::builder_finish(b);
}
```

### `std::str::from_bytes`

#### Synopsis

```aperio
fn from_bytes(b: Bytes) -> String
```

Phase 2g — copies the body of a `Bytes` value into a fresh
NUL-terminated String allocated in the global payload arena.
The inverse of `std::bytes::from_string`.

#### Semantics

- The Bytes body is memcpy'd verbatim into a `(len + 1)`-byte
  buffer; the trailing byte is set to `\0` so the result is a
  well-formed String for downstream `len(s)` / slicing.
- Embedded NUL bytes in the source persist in the buffer but
  the resulting String's strlen-based view will truncate at
  the first one. Callers who need NUL-safe handling should
  stay in Bytes.
- An empty or null Bytes value yields the stable empty-string
  sentinel.

#### Examples

Round-tripping a known-text payload through the binary-safe
surface — useful when shipping text through `Stream.send_bytes`
or storing it as a `Bytes` field for length-explicit reasons:

```aperio
fn main() {
    let original = "hello world";
    let b = std::bytes::from_string(original);
    println("len in bytes = ", len(b));
    let restored = std::str::from_bytes(b);
    println("restored = ", restored);
}
```

Reading a known-UTF-8 file as `Bytes`, then promoting to
`String` once length is known:

```aperio
fn main() {
    let raw = std::io::fs::read_bytes("config.txt");
    if len(raw) > 0 {
        let text = std::str::from_bytes(raw);
        println("config = ", text);
    }
}
```

See also [`std::bytes`](./bytes.md) for the Bytes-side
operations (`at`, `slice`, `from_string`) and the
binary-safe TCP surface (`Stream.recv_bytes`).

## Bare-name builtins

The most common string operations are bare-name builtins (no
`std::*` path), summarised here for completeness. Each is
documented in the language reference, not under `std::*`:

| Name           | Type                                  | Notes                          |
|----------------|---------------------------------------|--------------------------------|
| `len(s)`       | `(String) -> Int`                     | Byte length.                   |
| `starts_with`  | `(String, String) -> Bool`            | Prefix test.                   |
| `contains`     | `(String, String) -> Bool`            | Substring test (yes/no).       |
| `to_string`    | `(Int / Float / Decimal) -> String`   | Numeric → String.              |

Slicing uses range syntax: `s[start..end]`. Concatenation
uses `+`.

## Limitations

- **Base 10 only.** `parse_int` rejects `0x`, `0o`, `0b`
  prefixes.
- **No whitespace trimming in the parsers.** `parse_int` /
  `parse_float` reject any leading or trailing whitespace.
  Call `std::str::trim` first if your inputs are whitespace-
  noisy.
- **No `int_to_str` path-call.** Use the bare-name builtin
  `to_string(n)` for the Int → String direction.
- **`index_of` is byte-wise, not codepoint-wise.** Fine for
  ASCII; surprising for multi-byte UTF-8 if the search needle
  could split a codepoint. Same applies to `replace`.
- **`lower` / `upper` are ASCII-only.** Non-ASCII bytes pass
  through unchanged — no Unicode case folding in v1.
- **No `split`.** Hand-roll using `index_of` + slicing for now;
  splitting needs growable-vec-of-strings which gates on the
  generics design call.
- **No `last_index_of`, `replace_first`, `lines`.** All
  hand-roll until a workload forces the issue.

## See Also

- [Roadmap](./roadmap.md) — Phase 1+ stdlib build-out plan.
- [`std::env`](./env.md) — argv access; the canonical
  pairing for `parse_int`-from-argv usage.
- [What you can build today](./ready-today.md) — capability
  matrix including the bare-name string surface.
