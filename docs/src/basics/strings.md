# Strings & Bytes

`String` in Hale is an immutable UTF-8 byte sequence, while `Bytes` holds arbitrary binary data (including null bytes). For text processing, Hale provides string interpolation (`f"..."`), multi-line raw block strings (`"""`), one-line raw strings (`r"..."`), `BytesBuilder` for linear-time buffer construction, and `ByteView` for zero-rescan parsing.

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
| `std::str::trim(s)` | `(String) -> String` | Strips leading and trailing ASCII whitespace (space, tab, CR, LF) |
| `std::str::trim_start(s)` / `trim_end(s)` | `(String) -> String` | Strips the same whitespace from one end only |
| `std::str::trim_prefix(s, pre)` / `trim_suffix(s, suf)` | `(String, String) -> String` | Removes one leading `pre` / trailing `suf`; `s` unchanged when it is absent |
| `std::str::cut(s, sep)` / `cut_last(s, sep)` | `(String, String) -> (String, String) fallible(ParseError)` | The text before and after the first / last `sep`; `kind: "not_found"` when `s` has none |
| `std::str::eq_ignore_case(a, b)` | `(String, String) -> Bool` | Equal with ASCII letters compared without case |
| `std::str::replace(s, old, new)` | `(String, String, String) -> String` | Replaces all occurrences of `old` with `new` |
| `std::str::contains(s, sub)` | `(String, String) -> Bool` | True if `s` contains `sub` (also available as builtin `contains`) |
| `std::str::starts_with(s, pre)` / `ends_with(s, suf)` | `(String, String) -> Bool` | Prefix / suffix test (both are also builtins) |
| `std::str::substring(s, lo, hi)` | `(String, Int, Int) -> String` | The bytes from offset `lo` up to, not including, `hi` (the same slice as `s[lo..hi]`; both ends are clamped into the string) |
| `std::str::index_of(s, sub)` / `index_of_from(s, sub, pos)` | `(String, String, ...) -> Int` | Byte offset of first match, or `-1` if absent |
| `std::str::last_index_of(s, sub)` | `(String, String) -> Int` | Byte offset of the last match, or `-1` if absent |
| `std::str::split_into(s, sep, out)` | `(String, String, VecLocus) -> ()` | Splits `s` by `sep` into a caller-owned `@form(vec)` of `String` |
| `for p in std::str::split(s, sep)` | `for` source | Visits the parts of `s` between `sep`s, empty ones kept (one pass; not a value) |
| `for l in std::str::lines(s)` | `for` source | Visits the lines of `s`, without `\r\n` / `\n` endings; no final empty line |
| `for w in std::str::words(s)` | `for` source | Visits the runs of `s` between ASCII whitespace; never an empty word |
| `std::str::join(items, sep)` | `(VecLocus, String) -> String` | Joins elements of a `@form(vec)` of `String` with `sep` |
| `std::str::bytes_view(s)` | `(String) -> ByteView` | Measures `s` once (`v.n` bytes) for `O(1)` indexed scanning |
| `std::str::byte_at(v, i)` | `(ByteView, Int) -> Int` | Returns byte `0..255` at `i`, or `-1` if `i` is out of bounds |
| `std::str::slice(v, lo, hi)` | `(ByteView, Int, Int) -> String` | Copies `v[lo..hi]` as a `String` (clamped to `0..v.n`) |
| `std::str::find(v, sub, start)` | `(ByteView, String, Int) -> Int` | Finds `sub` in `v` starting at `start` without re-measuring `s` |
| `std::str::cp_count(s)` | `(String) -> Int` | UTF-8 code point count, or `-1` on invalid UTF-8 |
| `std::str::cp_at(s, i)` / `cp_size(s, i)` | `(String, Int) -> Int` | Code point value or byte width (`1..4`) at byte offset `i` (`-1` if invalid) |
| `std::str::view_cp_at(v, i)` / `view_cp_size(v, i)` | `(ByteView, Int) -> Int` | `O(1)` code point value or byte width at offset `i` on a `ByteView` |
| `std::str::parse_int(s)` / `parse_float(s)` / `parse_decimal(s)` | `(String) -> T fallible(ParseError)` | Parses text into `Int`, `Float`, or `Decimal<S>` |
| `std::str::parse_bool(s)` | `(String) -> Bool fallible(ParseError)` | `"true"` or `"false"`, exactly; anything else fails with `kind: "parse_bool"` |
| `std::str::can_parse_int(s)` | `(String) -> Bool` | Non-failing check for valid integer syntax |
| `std::str::from_bytes(b)` / `std::bytes::from_string(s)` | `(Bytes) -> String` / `(String) -> Bytes` | Converts between `Bytes` and `String` |
| `std::bytes::at(b, i)` | `(Bytes, Int) -> Int fallible(IndexError)` | Reads single byte (`0..255`) at index `i` |
| `std::bytes::BytesBuilder` | `locus { initial_cap: Int }` | Growable buffer (`append_str(s)`, `snapshot() -> Bytes fallible(...)`) |
| `std::regex::matches(pat, s)` / `find(pat, s)` / `valid(pat)` | `(String, ...) -> Bool \| Int` | Linear-time regular expression matching, byte search (`-1` on miss), and validation |

## Concatenation and `BytesBuilder`

Use `+` to join strings, and `to_string(x)` to convert numbers, booleans, or durations into strings:

```hale,fragment
let first = "alice";
let last  = "smith";
let full  = first + " " + last;
let label = "n=" + to_string(42);
```

### Building Strings in a Loop

Appending to a local declared `let mut` is linear: `out = out + chunk`, `out += chunk` and `out = f"{out}…"` append into a buffer the local owns, which grows by half again whenever it fills, so building `n` bytes costs `O(n)` time and memory. A chain of `+` such as `a + b + c` is one allocation, not one per `+`. The same holds for a `String` field of `self` grown inside a method.

```hale
fn page(n: Int) -> String {
    let mut out = "[";
    let mut i = 0;
    while i < n {
        if i > 0 { out += ", "; }
        out = out + "{\"id\": " + to_string(i) + "}";
        i = i + 1;
    }
    return out + "]";
}

fn main() {
    println(page(3));   // [{"id": 0}, {"id": 1}, {"id": 2}]
}
```

The buffer stays the local's only while nothing else holds the text. Strings are values, so a read that may keep the local's value (passing it to a function, binding it to another name, storing it, returning it) gives the buffer up, and the next append copies the whole text first; a copy taken before an append never sees it. Reading the text as an operand of `+` or of a comparison, in a slice (`s[lo..hi]` copies), or as an argument of a `std::` reader that returns a number or a `Bool` (`len`, `std::str::contains`, `index_of`, `starts_with`) keeps the buffer; passing it to a function of your own does not, since the compiler does not know what that function keeps. So inside the loop, use the growing text only in those ways, and hand it on after the loop. A loop that appends and also passes the text to a call (`out += row; log(out);`) copies the text on every iteration, and so does appending to any other place (`rec.body = rec.body + row`); `hale verify` reports both (and `hale check --warn-hot-path` shows them).

For those cases, or for a buffer that outlives one call, append into a `std::bytes::BytesBuilder` (or use `std::str::join` when you already have a collection of strings):

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

`len(out)` reads the length the last append left, so testing it in the loop is linear too.

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

Plain `"..."` strings do not interpolate; writing `"x={x}"` leaves the braces literal and emits a compiler warning if `x` is in scope. Escape literal braces in an f-string with `{{` and `}}`. A plain string may also span lines: a line break inside the quotes is part of the value as written, escapes still apply, and nothing is stripped; for text with its own syntax, reach for a block string.

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

### Declaring How a Type Reads: `@display`

The structural form is good for debugging but rarely how a domain value should read. A record, or an enum's variant, can declare its own text with a template. Every rendering path uses it: `println`, `print`, `to_string`, `String + x` and f-strings.

```hale
@display("${dollars}.{cents:0>2} {currency}")
type Money { dollars: Int; cents: Int; currency: String; }

type Side = enum {
    @display("buy") Buy,
    @display("other: {0}") Other(String),
};

fn main() {
    let m = Money { dollars: 12, cents: 5, currency: "USD" };
    println(m);                    // $12.05 USD
    println(Side::Other("x"));     // other: x
    println(f"{m:?}");             // Money { dollars: 12, cents: 5, currency: "USD" }
    println(Money::display(m));    // $12.05 USD
}
```

- A template is literal text and holes. Each hole names a field (`{dollars}`) or a variant's payload position (`{0}`), with an optional format spec (`{cents:0>2}`). `{{` and `}}` are literal braces.
- A template holds no expressions or calls, so rendering can't fail. The compiler checks each hole like an f-string's, at the template: the field must exist and be printable, and the spec must fit it.
- `@display` is allowed only on a type that is already printable, so a record holding `Bytes` takes none.
- A nested value renders by its own template, or structurally if it has none. An alias or an imported type renders as its declaration does.
- `{x:?}` shows the structural form, bypassing every template at every depth. Use it in logs of internals and test messages.
- `T::display(v)` names the rendering and can be passed as a function value.
- Rendering through a template allocates in the caller's frame, just like the equivalent hand-written f-string, and the allocation checks count it the same way.

### Format Specs

Add `:[[fill]align][width][.precision][kind]` after an interpolated expression to control alignment, padding, precision, or base:

| Spec | Example (`n = 42`, `name = "alice"`, `r = 3.14159`) | Output | Meaning |
|---|---|---|---|
| `:width` | `f"[{n:6}]"` | `[    42]` | Minimum byte width (numbers right-align; strings left-align) |
| `:<width` / `:^width` / `:>width` | `f"[{n:<6}]"` | `[42    ]` | Explicit left (`<`), center (`^`), or right (`>`) alignment |
| `:fill>width` | `f"[{n:0>6}]"` | `[000042]` | Custom single-byte fill character (`0`) with alignment |
| `:.precision` | `f"{r:.2}"` | `3.14` | Fractional digits for `Float` and `Decimal` |
| `:x` | `f"{n:x}"` | `2a` | Lowercase hexadecimal formatting for `Int` |
| `:?` | `f"{m:?}"` | `Money { dollars: 12, … }` | The structural form, past every `@display` template |

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

## Raw Strings

A one-line text whose backslashes are its own, such as a regular expression or a Windows path, is a **raw string**: `r` before the opening quote. Nothing in it is an escape and `{` is plain text. When the text holds a `"`, put `#` between the `r` and the quote and close with `"#` (add more `#` if the text holds `"#`):

```hale
fn main() {
    println(r"C:\temp\new");        // C:\temp\new
    println(r"^\d+\.\d+$");        // ^\d+\.\d+$
    println(r#"say "hi" {x}"#);    // say "hi" {x}
}
```

A raw string is an ordinary `String` literal and goes wherever one does (`spec/tokens.md` § Raw strings).

## Slicing, Scanning, and `ByteView`

### Basic Slicing and Search

You can slice a `String` by byte range with `s[lo..hi]` and search short strings with `std::str::index_of`:

```hale,fragment
let s = "hello world";
let h = s[0..5];                                  // "hello"
let i = std::str::index_of(s, "world");           // 6
```

Leave out a bound to run from the start or to the end:

```hale,fragment
let line = "key=value";
let at = std::str::index_of(line, "=");           // 3
let key = line[..at];                             // "key", the same as line[0..at]
let value = line[(at + 1)..];                     // "value", the same as line[(at + 1)..len(line)]
let first = line[..=2];                           // "key", bytes 0 through 2
let copy = line[..];                              // "key=value"
```

| Form | Bytes |
|---|---|
| `s[lo..hi]` | `lo` up to, not including, `hi` |
| `s[lo..=hi]` | `lo` through `hi` |
| `s[lo..]` | `lo` to the end |
| `s[..hi]` / `s[..=hi]` | from the start |
| `s[..]` | all of them |

The receiver is evaluated once, so `next_line()[4..]` calls `next_line` once. Bounds are clamped, never an error: a start below 0 is 0, a bound past the end is the end, and an end before the start gives `""`. Offsets count bytes, not characters (`"né"` is three bytes long). A bound can only be left out inside the brackets: `3..` on its own is a parse error, and `s[lo..=]` is refused because `..=` needs an end to include.

A `String` has no single element to index: `s[i]` is refused, because one position could mean a byte or a character. Slice one byte (`s[i..(i + 1)]`), or read it as a number through a view (`std::str::byte_at`, below). For the same reason `for c in s` is refused: say what the parts are with one of the sources below.

### Splitting in a Loop: `split`, `lines` and `words`

The loop that takes a text apart by re-slicing the rest is quadratic. Every turn copies the rest of the text, and the copies stay in the function's scratch until it returns:

```hale,fragment
let mut rest = list + " ";
while len(rest) > 0 {
    let sp = std::str::index_of(rest, " ");
    let w = rest[..sp];
    rest = rest[(sp + 1)..];                // a copy of the rest
}
```

Write the loop over a text source instead. `std::str::split(s, sep)`, `std::str::lines(s)` and `std::str::words(s)` visit the parts of `s` one per iteration, in one pass over the text:

```hale
fn main() {
    for field in std::str::split("id,name,,note", ",") {
        println("[" + field + "]");          // [id] [name] [] [note]
    }
    for line in std::str::lines("first\r\nsecond\n") {
        println(line);                       // first, second
    }
    let mut n = 0;
    for w in std::str::words("  any   amount of  space ") {
        if w == "of" { continue; }
        n = n + 1;
    }
    println(n);                              // 3
}
```

| Source | The parts |
|---|---|
| `split(s, sep)` | The text between occurrences of `sep`, empty parts kept: `"a,,b,"` is `a`, `""`, `b`, `""`. `""` is one empty part; an empty `sep` gives none. |
| `lines(s)` | The text between `\n`s, each without the `\r` of a CRLF ending. A final newline starts no line. |
| `words(s)` | The runs of text between ASCII whitespace (space, tab, CR, LF, vertical tab, form feed); never an empty word. |

- Each part is a new `String` for its iteration. Keep it if you need it: assign it to an outer variable, return it or store it.
- `break` leaves the loop and `continue` goes to the next part. Loops over sources nest.
- The arguments are evaluated once, before the first part.
- Offsets are bytes. A UTF-8 separator never cuts a character, and a non-ASCII space is text to `words`.
- The loop does nothing but read the text, so it is legal in `@deterministic @no_syscall` code.

Each source also starts an [element chain](../everyday/collections.md#collection-query-chains), the same single pass with a stage or a terminal in place of the loop body. Every part is the chain's `it`:

```hale
fn main() {
    let text = "id,name,,note";
    println(std::str::split(text, ",").filter(len(it) > 0).count());       // 3
    println(std::str::words("a an the tree").filter(len(it) > 2).count()); // 2
    let longest = std::str::words("pear fig banana").max(len(it)) or "";
    println(longest);                                                     // banana
}
```

Every stage and terminal works over a text source, without a stage too (`std::str::words(s).count()`). `first`, `find`, `min` and `max` give back the part they picked, and an empty selection fails with an `IndexError`, so `or` supplies the fallback, as it does over a vec. The chain keeps no list of the parts: like the loop, it builds one `String` per part, for that iteration.

The sources are not values. Hale has no sequence value for them to return, so `let parts = std::str::split(s, ",");` is refused, and the message points at a chain's `into(v)` and at `std::str::split_into` (below), each of which writes the parts into storage you own. Over 1 MiB of 4 KiB words, `words` takes 0.7 ms and the re-slicing loop above takes 15 ms; four times the text costs `words` four times as long and the loop sixteen. A chain over a source scales the same way.

### Walking Large Strings with `ByteView`

A `String` in Hale is a null-terminated byte pointer without an inline length header. A local or parameter remembers the length of the value it holds, so `len(s)`, a slice `s[i..(i + 1)]`, `std::str::substring`, `std::str::index_of_from(s, "\n", start)` and `cp_at` / `cp_size` on it measure the text once, not on every iteration: `while i < len(s) { let c = s[i..(i + 1)]; … }` is linear. A String reached any other way (a field such as `self.text` or `rec.body`, an element, a call's result) carries no length and is measured by every one of those calls, so inside a loop bind it to a local first. Each one-byte slice is still a new String.

For a scan the three sources do not cover, construct a `std::str::ByteView` **once** before the loop. It scans or tokenizes a string in `O(n)` time:

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

A byte is an `Int`, and a **byte literal** writes one by its character: `b'/'` is `47`, `b'\n'` is `10`. It takes one printable ASCII character or one escape (`\n`, `\t`, `\r`, `\\`, `\'`, `\"`, `\0`, `\xNN`), and it is an integer literal everywhere one is legal, so it compares with `byte_at`, matches in an arm's pattern (`b'a' | b'A' -> …`), and initializes a constant or an array element. `b''`, `b'ab'` and `b'é'` are refused where they are written: a byte literal is exactly one byte, so a non-ASCII character takes `\xNN` or a `b"…"` `Bytes` literal.

```hale
fn line_end(v: std::str::ByteView, start: Int) -> Int {
    let mut i = start;
    while i < v.n {
        if std::str::byte_at(v, i) == b'\n' { return i; }
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

To visit the parts once, loop over `std::str::split` (above). To keep them, remember that collections in Hale are loci rather than garbage-collected values: `std::str::split_into` writes into a `@form(vec)` locus that you supply:

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

### Splitting Once and Trimming

To split a line once, at a separator, `std::str::cut` returns the text on each side of it as a tuple. A line without the separator fails with `ParseError { kind: "not_found", input: s }`, so the `or` says what to do then:

```hale
fn setting(line: String) -> String {
    let (key, value) = std::str::cut(line, "=") or { return "no setting in " + line; };
    return key + " -> " + value;
}

fn main() {
    println(setting("timeout=30s"));                      // timeout -> 30s
    println(setting("a=b=c"));                            // a -> b=c
    println(setting("verbose"));                          // no setting in verbose
    let (dir, file) = std::str::cut_last("a/b/c.txt", "/") or { return; };
    println(dir, " | ", file);                            // a/b | c.txt
    println(std::str::trim_prefix("sha256:ab12", "sha256:")); // ab12
    println(std::str::trim_suffix("report.csv", ".csv")); // report
    println(std::str::eq_ignore_case("Content-Type", "content-type")); // true
}
```

`cut` splits at the first separator and keeps the rest whole; `cut_last` splits at the last one. `trim_prefix` and `trim_suffix` remove the affix once if it is there and otherwise return the string unchanged, so they replace both the `starts_with` test and the magic offset (`g[7..]`). `std::str::last_index_of` is the offset `cut_last` splits at. Offsets and comparisons are bytes: a separator matches byte for byte, and `eq_ignore_case` folds only `A`–`Z`.

### Parsing Numbers and Binary `Bytes`

Number parsers return `fallible(ParseError)` and must be addressed with `or` (covered in [When a Call Can Fail](./fallible.md)):

```hale,fragment
let n = std::str::parse_int("42") or 0;
let ok = std::str::can_parse_int("42");        // true (non-failing check)
let on = std::str::parse_bool("true") or false; // only "true" and "false" parse
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
