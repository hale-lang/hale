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
