# Values & variables

> The vocabulary every Hale program is built from.

A variable is introduced with `let`:

```hale,fragment
let greeting = "hello";
let count    = 3;
let ratio    = 0.5;
let ready    = true;
```

Hale infers the type from the value. You can write it explicitly
when you want to be sure, or when there's no value to infer
from:

```hale,fragment
let count: Int = 3;
```

## Immutable by default

A plain `let` binding can't be reassigned. To make a variable
you can change, add `mut`:

```hale,fragment
let total = 0;
total = total + 1;        // ERROR: total is immutable

let mut total = 0;
total = total + 1;        // fine
```

Immutable-by-default is a per-binding property, not a property
of the type. There's no separate "constant" concept for locals —
`let` *is* the constant, `let mut` is the variable. (Top-level
program constants use `const NAME: T = ...;` and are written
`SCREAMING_SNAKE_CASE`.)

Shadowing — declaring a second `let x` in the same scope — is
not allowed. Pick a new name. The language would rather you say
what you mean than quietly reuse a name for a different value.

## The primitive types

These are the scalar types built into the language:

| Type | What it holds | Literal examples |
|---|---|---|
| `Int` | 64-bit signed integer | `0`, `42`, `1_000_000`, `0xFF`, `0b1010` |
| `Float` | 64-bit IEEE float | `3.14`, `1.0e-3`, `2.5` |
| `Bool` | true / false | `true`, `false` |
| `String` | UTF-8 text | `"hello"`, `"line\n"` |
| `Decimal` | exact fixed-point number | `1.50d`, `0.00d` |
| `Duration` | a span of time | `100ms`, `5s`, `1h30m` |
| `Time` | a wall-clock instant | `` `2026-05-08T12:00:00Z` `` |
| `Bytes` | a binary blob | `b"\x00\x01\xff"` |

`Decimal`, `Duration`, and `Time` are first-class — not strings
you parse, not integers you remember the units of. They get
their own chapter ([Math, money & time](./math.md)) because
they're a real ergonomic upgrade over what most languages give
you.

Underscores in number literals are just for readability
(`1_000_000`). Integers default to `Int`, decimals-with-a-point
default to `Float`; the `d` suffix makes a `Decimal`.

## Strings

Double-quoted, with the usual escapes (`\n`, `\t`, `\"`, `\\`,
`\xNN`). One extra form — the f-string:

```hale,fragment
let name  = "world";
let hi    = f"hello {name}";           // f-string interpolation
```

An f-string evaluates the expressions inside `{...}` and renders
them into the text. Use `{{` and `}}` for literal braces.

## Printing values

`println` and `print` take any number of arguments and
concatenate them. `to_string` turns a value into text when you
need it as a `String`:

```hale
fn main() {
    let n = 41;
    println("n + 1 = ", n + 1);        // n + 1 = 42
    let s = to_string(n + 1);          // "42"
    println(s);
}
```

`println`, `print`, `to_string`, and `len` are *builtins* —
called as plain functions, not methods. You write `len(s)`, not
`s.len()`. (Methods with `.` come later, on loci and your own
types.)

Next: [Math, money & time](./math.md).
