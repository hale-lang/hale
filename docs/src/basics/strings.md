# Strings & text

> Building and inspecting text.

## Joining

The `+` operator concatenates strings, and `println` /
f-strings join for you:

```hale,fragment
let first = "Ada";
let last  = "Lovelace";

let full  = first + " " + last;
let hi    = f"hello, {first}";
println("full name: ", full);
```

`to_string(x)` converts a number, bool, duration, etc. into its
text form when you need a `String` specifically:

```hale,fragment
let n = 42;
let label = "n=" + to_string(n);
```

## f-strings

An `f` before the quote turns `{...}` into interpolation. What
goes inside is a whole expression, not just a name:

```hale,fragment
let name = "Ada";
let n    = 3;

println(f"hello, {name}");
println(f"{n} items, {n * 2} halves");
println(f"upper: {std::str::upper(name)}");
```

A plain `"..."` string leaves braces alone, so `"x={x}"` prints
the braces. That is the one mistake worth knowing about, and the
compiler warns when the braces name something in scope. Write
`{{` and `}}` for literal braces inside an f-string:

```hale,fragment
let x = 1;
println(f"literal {{x}}, interpolated {x}");   // literal {x}, interpolated 1
```

### Printing whole values

Interpolation renders structs, tuples and arrays, recursively —
which is usually what you want mid-debugging:

```hale
type Point   { x: Int; y: Int; }
type Reading { sensor: String; at: Point; }

fn main() {
    let r = Reading { sensor: "t-1", at: Point { x: 3, y: 4 } };
    println(f"{r}");
    // Reading { sensor: "t-1", at: Point { x: 3, y: 4 } }
}
```

Strings *inside* a rendered value are quoted, so a value
containing a comma still reads as one value. A string on its own
is not quoted — `f"{name}"` is just `Ada`.

Two things deliberately do not render: a **locus** (it is flow,
not shape — and rendering one would leak the state a `@sealed`
locus exists to confine) and **`Bytes`** (pick a rendering:
hex, length, or a text decode). Reach for a field instead.

### Format specs

An interpolation can say *how* to render, after a `:`:

```hale,fragment
let n     = 42;
let name  = "ada";
let ratio = 3.14159;

println(f"[{n:6}]");        // [    42]   numbers pad left
println(f"[{name:6}]");     // [ada   ]   text pads right
println(f"[{n:<6}]");       // [42    ]   `<` `^` `>` override
println(f"[{n:0>6}]");      // [000042]   a fill character
println(f"{ratio:.2}");     // 3.14       precision (Float/Decimal)
println(f"{n:x}");          // 2a         hexadecimal
```

The full form is `[[fill]align][width][.precision][kind]`. Width
counts bytes and never truncates — a value too wide for its
column keeps all of its digits and pushes the column, because a
silently shortened number in a log is worse than a ragged table.

## Length and inspection

`len(s)` is a builtin — the byte length of the string:

```hale,fragment
let s = "hello";
println(len(s));          // 5
```

Most text operations live in `std::str`, called as plain
functions:

```hale,fragment
let i   = std::str::index_of("hello world", "world");   // 6
let sub = std::str::substring("hello world", 0, 5);     // "hello"
let up  = std::str::upper("hi");                          // "HI"
let t   = std::str::trim("  spaced  ");                   // "spaced"
let r   = std::str::replace("a-b-c", "-", "+");          // "a+b+c"
```

Hale has no per-character method syntax (`s.charAt(i)`); you
slice with a range or use the `std::str` helpers. Slicing a
string by byte range:

```hale,fragment
let s = "hello";
let h = s[0..1];          // "h"
```

## Parsing numbers

Turning text into a number can fail — the text might not be a
number. So the parse functions are *fallible*, and the next
chapter ([When a call can fail](./fallible.md)) is exactly about
how you handle that. The shape, previewed:

```hale,fragment
let n = std::str::parse_int("42") or 0;     // 42, or 0 if it wasn't
```

There are also non-failing predicates to check first
(`std::str::can_parse_int`) when you'd rather branch than
recover.

## Bytes

Text is `String`; raw binary is `Bytes`. They're different types
because they have different rules — a `String` is valid UTF-8, a
`Bytes` is any sequence of octets, including embedded zeros.

```hale,fragment
let b = std::bytes::from_string("hello");   // String  -> Bytes
let s = std::str::from_bytes(b);            // Bytes   -> String
let byte0 = std::bytes::at(b, 0) or 0;       // a single byte (fallible)
```

You'll work with `Bytes` directly when you read from a socket or
a file and need to frame messages yourself — that's a topic for
[wire formats](../everyday/files.md) and the systems tier. At
this level, just know the two types are distinct and you convert
explicitly between them.

Next: the failure model — [When a call can fail](./fallible.md).

## Splitting, joining, searching

```hale,fragment
std::str::contains(line, "=")       // Bool
std::str::starts_with(line, "#")
std::str::ends_with(path, ".hl")
```

Splitting writes into a collection you supply rather than returning a
new one:

```hale,fragment
@form(vec)
locus Fields { capacity { heap items of String; } }

let f = Fields { };
std::str::split_into("a,b,c", ",", f);
println(std::str::join(f, " | "));      // a | b | c
```

The asymmetry is deliberate, and it's worth understanding rather than
memorising. `join` returns a `String` because a String is already a
value. `split` cannot return a list, because Hale has no list *value*
to return — collections are loci you own. So you hand it the storage.

That turns out to be the better shape anyway: you own the allocation,
so it's counted against your budget instead of hidden inside a return
value, and a handler that splits a line per message can reuse one
collection instead of allocating a fresh one every time.

Empty fields survive: `"a,,b,"` splits into four, not two. Losing them
silently would drop a column from a CSV row and you would never see it
happen.

## Text that isn't ASCII

`String` is bytes. When you need characters, ask for code points
explicitly:

```hale,fragment
std::str::cp_count("héllo")     // 5, though the string is 6 bytes
std::str::cp_at("日本語", 0)     // 26085
std::str::cp_size("héllo", 1)   // 2 — 'é' is two bytes
```

Invalid UTF-8 gives you `-1`, as does a byte offset that lands in the
middle of a character. It does not quietly hand back a replacement
character, because then you could not tell a corrupted string from one
that genuinely contains that character.

Normalization, case folding beyond ASCII, and grapheme clusters are
not provided. Each is a large commitment with its own tables, and a
`to_upper` that works for English and mangles Turkish is worse than
one that isn't there.

## Patterns

```hale,fragment
std::regex::matches("h.llo", "hello")   // true — a full match
std::regex::find("l+", "hello")         // 2 — leftmost byte offset
std::regex::valid("a(")                 // false
```

Literals, `.`, `*`, `+`, `?`, `|`, grouping, and character classes
(`[a-z]`, `[^a-z]`). No backreferences and no lookahead — those need a
backtracking engine, and a backtracking match has no upper bound on
how long it can take. In a language where you can write
`@budget(alloc_per_call = 0)` on a handler, a pattern that might take
exponential time is not something you can be allowed to put there.

Check `valid` on any pattern you didn't write yourself. Without it, a
typo looks exactly like "no matches".
