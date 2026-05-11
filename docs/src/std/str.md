# `std::str`

Minimal string-processing primitives. m78 added the integer
parsers (`parse_int`, `can_parse_int`); m84 added `index_of`
for substring search. v0 scope is small by design — most
string work in Aperio uses bare-name builtins (`len`,
`starts_with`, `contains`, `to_string`) plus the `+` operator
for concatenation and `s[start..end]` for slicing. The path-
qualified surface here covers the cases that need a real
function call.

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
- **No whitespace trimming.** `parse_int` rejects any leading
  or trailing whitespace.
- **No floating-point parsing.** `parse_float` lands when
  there's a use case forcing it.
- **No `int_to_str` path-call.** Use the bare-name builtin
  `to_string(n)` for the Int → String direction.
- **`index_of` is byte-wise, not codepoint-wise.** Fine for
  ASCII; surprising for multi-byte UTF-8 if the search needle
  could split a codepoint.
- **No `split` / `replace` / `trim` / `to_lower` / `to_upper`.**
  Hand-roll using `index_of` + slicing for now.

## See Also

- [Roadmap](./roadmap.md) — Phase 1+ stdlib build-out plan.
- [`std::env`](./env.md) — argv access; the canonical
  pairing for `parse_int`-from-argv usage.
- [What you can build today](./ready-today.md) — capability
  matrix including the bare-name string surface.
