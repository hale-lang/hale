# `std::str`

Minimal string parsing primitives. m78 ships two functions:
`parse_int` (atoi-ish) and `can_parse_int` (the boolean
sibling for callers that need to distinguish "0" from "bad
input"). v0 scope: signed 64-bit integers in base 10. Hex /
octal / underscore separators wait on a richer parsing
library.

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

## Limitations

- **Base 10 only.** No `0x`, `0o`, `0b` prefixes.
- **No whitespace trimming.** Leading or trailing whitespace
  rejects the input. Callers that want lenient input strip
  manually (which itself wants string ops Aperio doesn't
  yet ship — a future arc).
- **No floating-point parsing.** `parse_float` lands when
  there's a use case forcing it.
- **No reverse direction (int → String) here.** `println`
  already formats Ints; explicit `int_to_str` waits on the
  string-builder library.

## See Also

- [Roadmap](./roadmap.md) — Phase 1+ stdlib build-out plan.
- [`std::env`](./env.md) — argv access; the canonical
  pairing for `parse_int`-from-argv usage.
