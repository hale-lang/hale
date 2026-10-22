# Strings & Text

`String` in Hale is an immutable UTF-8 byte sequence, while `Bytes` holds arbitrary binary data (including null bytes). For text processing, Hale provides string interpolation (`f"..."`), multi-line raw block strings (`"""`), `BytesBuilder` for linear-time buffer construction, and `ByteView` for zero-rescan parsing.

```hale
fn main() {
    let user = "alice";
    let score = 98.456;
    println(f"user={std::str::upper(user)} score={score:.1}");

    let v = std::str::bytes_view("id=42&role=admin");
    let amp = std::str::find(v, "&", 0);
    println("first pair: ", std::str::slice(v, 0, amp));
}
```

## `std::str`, `std::bytes`, and `std::regex` Reference

| Function / Type | Signature | Description |
|---|---|---|
| `len(s)` | `(String \| Bytes) -> Int` | Builtin: byte length of `s` |
| `to_string(x)` | `(T) -> String` | Builtin: formats scalar or printable composite `x` as `String` |
| `std::str::upper(s)` / `lower(s)` | `(String) -> String` | ASCII case conversion |
| `std::str::trim(s)` | `(String) -> String` | Strips leading and trailing ASCII whitespace |
| `std::str::replace(s, old, new)` | `(String, String, String) -> String` | Replaces all occurrences of `old` with `new` |
| `std::str::contains(s, sub)` | `(String, String) -> Bool` | True if `s` contains `sub` (also available as builtin `contains`) |
| `std::str::starts_with(s, pre)` / `ends_with(s, suf)` | `(String, String) -> Bool` | Prefix / suffix test (`starts_with` is also a builtin) |
| `std::str::substring(s, start, len)` | `(String, Int, Int) -> String` | Extracts `len` bytes starting at byte offset `start` |
| `std::str::index_of(s, sub)` / `index_of_from(s, sub, pos)` | `(String, String, ...) -> Int` | Byte offset of first match, or `-1` if absent |
| `std::str::split_into(s, sep, out)` | `(String, String, VecLocus) -> ()` | Splits `s` by `sep` into a caller-owned `@form(vec)` of `String` |
| `std::str::join(items, sep)` | `(VecLocus, String) -> String` | Joins elements of a `@form(vec)` of `String` with `sep` |
| `std::str::bytes_view(s)` | `(String) -> ByteView` | Measures `s` once (`v.n` bytes) for `O(1)` indexed scanning |
| `std::str::byte_at(v, i)` | `(ByteView, Int) -> Int` | Returns byte `0..255` at `i`, or `-1` if `i` is out of bounds |
| `std::str::slice(v, lo, hi)` | `(ByteView, Int, Int) -> String` | Copies `v[lo..hi]` as a `String` (clamped to `0..v.n`) |
| `std::str::find(v, sub, start)` | `(ByteView, String, Int) -> Int` | Finds `sub` in `v` starting at `start` without re-measuring `s` |
| `std::str::cp_count(s)` | `(String) -> Int` | UTF-8 code point count, or `-1` on invalid UTF-8 |
| `std::str::cp_at(s, i)` / `cp_size(s, i)` | `(String, Int) -> Int` | Code point value or byte width (`1..4`) at byte offset `i` (`-1` if invalid) |
| `std::str::view_cp_at(v, i)` / `view_cp_size(v, i)` | `(ByteView, Int) -> Int` | `O(1)` code point value or byte width at offset `i` on a `ByteView` |
| `std::str::parse_int(s)` / `parse_float(s)` / `parse_decimal(s)` | `(String) -> T fallible(ParseError)` | Parses text into `Int`, `Float`, or `Decimal<S>` |
| `std::str::can_parse_int(s)` | `(String) -> Bool` | Non-failing check for valid integer syntax |
| `std::str::from_bytes(b)` / `std::bytes::from_string(s)` | `(Bytes) -> String` / `(String) -> Bytes` | Converts between `Bytes` and `String` |
| `std::bytes::at(b, i)` | `(Bytes, Int) -> Int fallible(IndexError)` | Reads single byte (`0..255`) at index `i` |
| `std::bytes::BytesBuilder` | `locus { initial_cap: Int }` | Growable buffer (`append_str(s)`, `snapshot() -> Bytes fallible(...)`) |
| `std::regex::matches(pat, s)` / `find(pat, s)` / `valid(pat)` | `(String, ...) -> Bool \| Int` | Linear-time regular expression matching, byte search (`-1` on miss), and validation |

## Concatenation and `BytesBuilder`

Use `+` to join a fixed number of strings, and `to_string(x)` to convert numbers, booleans, or durations into strings:

```hale,fragment
let first = "alice";
let last  = "smith";
let full  = first + " " + last;
let label = "n=" + to_string(42);
```

### Building Strings in a Loop (`BytesBuilder`)

Do not accumulate strings with `out = out + chunk` inside a loop. Because `out + chunk` allocates a new string and copies the prefix on every iteration while old prefixes remain in the function's arena until return, loop concatenation uses `O(n²)` time and memory.

Instead, append into a single `std::bytes::BytesBuilder` (or use `std::str::join` when you already have a collection of strings):

```hale
fn page(n: Int) -> String {
    let b = std::bytes::BytesBuilder { initial_cap: 4096 };
    let mut i = 0;
    while i < n {
        if i > 0 { b.append_str(", "); }
        b.append_str("{\"id\": " + to_string(i) + "}");
        i = i + 1;
    }
    return "[" + std::str::from_bytes(b.snapshot() or b"") + "]";
}

fn main() {
    println(page(3));   // [{"id": 0}, {"id": 1}, {"id": 2}]
}
```

## F-Strings (`f"..."`)

Prefix a string literal with `f` to interpolate arbitrary Hale expressions inside `{...}`:

```hale,fragment
let name = "alice";
let n    = 3;

println(f"hello, {name}");
println(f"{n} items, {n * 2} halves");
println(f"upper: {std::str::upper(name)}");
println(f"literal {{braces}}, interpolated {n}");
```

Plain `"..."` strings do not interpolate; writing `"x={x}"` leaves the braces literal and emits a compiler warning if `x` is in scope. Escape literal braces in an f-string with `{{` and `}}`.

### Printing Whole Values

Both `println` and f-string `{...}` interpolation recursively format structs, tuples, and arrays of printable elements:

```hale
type Point   { x: Int; y: Int; }
type Reading { sensor: String; at: Point; }

fn main() {
    let r = Reading { sensor: "t-1", at: Point { x: 3, y: 4 } };
    println(f"{r}");
    // Reading { sensor: "t-1", at: Point { x: 3, y: 4 } }
}
```

Strings nested inside a composite value are printed with quotes (`"t-1"`), while a standalone string (`f"{name}"`) is printed unquoted.

Two types cannot be formatted directly:
1. **Loci:** A locus is a stateful component (which may be `@sealed` to confine secrets), not a value record. Format specific fields instead.
2. **`Bytes`:** Raw binary buffers have no single canonical text form; explicitly decode them with `std::str::from_bytes(b)`, format their `len(b)`, or encode them as hex.

### Format Specs

Add `:[[fill]align][width][.precision][kind]` after an interpolated expression to control alignment, padding, precision, or base:

| Spec | Example (`n = 42`, `name = "alice"`, `r = 3.14159`) | Output | Meaning |
|---|---|---|---|
| `:width` | `f"[{n:6}]"` | `[    42]` | Minimum byte width (numbers right-align; strings left-align) |
| `:<width` / `:^width` / `:>width` | `f"[{n:<6}]"` | `[42    ]` | Explicit left (`<`), center (`^`), or right (`>`) alignment |
| `:fill>width` | `f"[{n:0>6}]"` | `[000042]` | Custom single-byte fill character (`0`) with alignment |
| `:.precision` | `f"{r:.2}"` | `3.14` | Fractional digits for `Float` and `Decimal` |
| `:x` | `f"{n:x}"` | `2a` | Lowercase hexadecimal formatting for `Int` |

Width specifications count bytes and **never truncate**: a value wider than `width` prints all of its characters rather than silently dropping digits.

## Block Strings

Write multi-line raw text (SQL queries, JSON fixtures, embedded source code) as a **block string** opened by `"""` at the end of a line and closed by `"""` on its own line:

```hale
fn main() {
    let query = """
        SELECT id, name
          FROM people
         WHERE id = 1
        """;
    println(query);
}
```

Block strings obey four lexical rules (`spec/tokens.md` § Block strings):
1. **Closing-delimiter indentation stripping:** Leading whitespace matching the indentation of the closing `"""` is stripped from every content line. Lines indented further than the closing `"""` preserve their relative indentation; any line indented less than the closing `"""` is a compile error.
2. **Raw content (no escapes or interpolation):** Backslashes (`\n`, `\"`) and braces (`{`) are literal characters. Join with `+` if you need to splice runtime values.
3. **Boundary newlines excluded:** The newline immediately following the opening `"""` and the newline immediately preceding the closing `"""` are not included in the string. Add a blank line before the closing `"""` if you want a trailing newline.
4. **Extended fences (`""""`):** If a line inside the block string starts with `"""`, open and close the block string with four or more quotes (`""""`). `hale fmt` re-indents block strings alongside surrounding code without changing their contents.

## Slicing, Scanning, and `ByteView`

### Basic Slicing and Search

You can slice a `String` by byte range with `s[lo..hi]` and search short strings with `std::str::index_of`:

```hale,fragment
let s = "hello world";
let h = s[0..5];                                  // "hello"
let i = std::str::index_of(s, "world");           // 6
```

### Walking Large Strings with `ByteView`

A `String` in Hale is a null-terminated byte pointer without an inline length header. Calling `len(s)`, slicing `s[i..(i + 1)]`, or calling `std::str::index_of_from(s, "\n", start)` inside a loop scans for the null terminator on every iteration, turning a linear scan into an `O(n²)` bottleneck on megabyte-scale inputs.

To scan or tokenize a string in `O(n)` time, construct a `std::str::ByteView` **once** before the loop:

```hale
fn count_lines(all: String) -> Int {
    let v = std::str::bytes_view(all);
    let mut k = 0;
    let mut nl = std::str::find(v, "\n", 0);
    while nl >= 0 {
        k = k + 1;
        nl = std::str::find(v, "\n", nl + 1);
    }
    return k;
}

fn main() {
    println(count_lines("a\nb\n\nc\n"));   // 4
}
```

`std::str::bytes_view(s)` measures the string once and stores its byte length in `v.n`:
- **`std::str::byte_at(v, i)`** reads a single byte (`0..255`) in `O(1)` time and returns `-1` for any index outside `0..v.n`.
- **`std::str::slice(v, lo, hi)`** copies `v[lo..hi]` in `O(hi - lo)` time and clamps out-of-range bounds to `0..v.n`.
- **`std::str::find(v, needle, start)`** searches from byte offset `start` using `v.n` without re-measuring the string.

```hale
fn line_end(v: std::str::ByteView, start: Int) -> Int {
    let mut i = start;
    while i < v.n {
        if std::str::byte_at(v, i) == 10 { return i; }   // 10 is '\n'
        i = i + 1;
    }
    return v.n;
}

fn count_nonempty_rows(all: String) -> Int {
    let v = std::str::bytes_view(all);
    let mut rows = 0;
    let mut start = 0;
    while start < v.n {
        let nl = line_end(v, start);
        let line = std::str::slice(v, start, nl);
        start = nl + 1;
        if len(line) > 0 { rows = rows + 1; }
    }
    return rows;
}

fn main() {
    println(count_nonempty_rows("a\nb\n\nc"));   // 3
}
```

> [!WARNING]
> **View Lifetimes (`ByteView`, `StringView`, `BytesView`):** Views borrow their underlying buffer without copying. When reading `StringView` or `BytesView` spans from a socket, file, or parser buffer, the view is valid **only** until the next `recv` or buffer overwrite. Call `std::str::clone` (or `std::str::slice`) to copy data into an owned `String` if it must outlive the current activation. Accessing a stale view triggers a runtime panic guard.

## Splitting, Joining, and Parsing

### Splitting and Joining

Because collections in Hale are loci rather than garbage-collected values, `std::str::split_into` writes into a `@form(vec)` locus that you supply:

```hale
@form(vec)
locus Fields {
    capacity { heap items of String; }
}

fn main() {
    let f = Fields { };
    std::str::split_into("a,b,c", ",", f);
    println(std::str::join(f, " | "));      // a | b | c
}
```

- **Reusable storage:** A message handler can keep a single `Fields` locus in its `params` and call `f.clear()` before each `split_into` call for zero steady-state allocations.
- **Preserves empty fields:** `"a,,b,"` splits into 4 elements (`"a"`, `""`, `"b"`, `""`), preserving column alignment in CSV/TSV data.

### Parsing Numbers and Binary `Bytes`

Number parsers return `fallible(ParseError)` and must be addressed with `or` (covered in [When a Call Can Fail](./fallible.md)):

```hale,fragment
let n = std::str::parse_int("42") or 0;
let ok = std::str::can_parse_int("42");        // true (non-failing check)
```

Use `b"..."` literals and `std::bytes` when working with raw binary payloads:

```hale,fragment
let raw: Bytes = b"\x00\x01\xff";
let b = std::bytes::from_string("hello");      // String -> Bytes
let s = std::str::from_bytes(b);               // Bytes  -> String
let byte0 = std::bytes::at(b, 0) or 0;         // 104 ('h')
```

## Text That Isn't ASCII (UTF-8 Code Points)

`len(s)` and string slices operate on UTF-8 byte offsets. Because UTF-8 is self-synchronizing—no multi-byte code point ever contains a byte in the ASCII range `0..127`—byte scanning is always safe for ASCII delimiters, digits, and punctuation.

When you need Unicode code points, use `cp_count`, `cp_at`, `cp_size`, or their `O(1)` `ByteView` counterparts `view_cp_at` and `view_cp_size`:

```hale
fn count_code_points(s: String) -> Int {
    let v = std::str::bytes_view(s);
    let mut i = 0;
    let mut k = 0;
    while i < v.n {
        let w = std::str::view_cp_size(v, i);
        if w < 0 { return -1; }
        i = i + w;
        k = k + 1;
    }
    return k;
}

fn main() {
    println(count_code_points("héllo"));       // 5 (6 UTF-8 bytes)
    println(std::str::cp_at("日本語", 0));      // 26085
}
```

All code-point functions return `-1` if the input contains invalid UTF-8, if the byte offset lands in the middle of a multi-byte character, or if the offset is out of range (they never silently substitute `U+FFFD`). Unicode normalization, locale-dependent case folding, and grapheme cluster segmentation are not included in `std::str`.

## Regular Expressions (`std::regex`)

`std::regex` provides a non-backtracking, linear-time regular expression engine:

```hale,fragment
let full   = std::regex::matches("h.llo", "hello");   // true (anchored full match)
let offset = std::regex::find("l+", "hello");         // 2 (leftmost byte offset, or -1)
let is_ok  = std::regex::valid("a(");                 // false
```

Supported syntax includes literals, `.`, `*`, `+`, `?`, alternation `|`, grouping `(...)`, and character classes (`[a-z]`, `[^a-z]`). Backreferences and lookahead are omitted so that every match executes in guaranteed `O(n)` time and zero heap allocations on hot paths. Call `std::regex::valid(pat)` first whenever `pat` comes from external configuration or user input.

Next: [When a Call Can Fail](./fallible.md).
