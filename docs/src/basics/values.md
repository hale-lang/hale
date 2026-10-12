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

`const` is **top-level only**: it goes beside your `locus` and
`fn` declarations, not inside a locus body. A top-level `const`
is already in scope inside every locus of the program, so that
is where a shared constant belongs; if each instance should
carry its own value, that's a `params` field with a default
(`params { limit: Int = 7; }`, read as `self.limit`). A `const`
written inside a locus body is an error at the `const` keyword,
saying both.

A `let` is visible from its statement to the end of its block. A
second `let x` later in the same block rebinds the name to the new
value from there on; a `let x` inside a nested block shadows an
outer `x` — a parameter's or an outer `let`'s — there and there only,
and the outer binding is what the rest of the function sees. The
slot a `let` binds lives until the function exits (that is what a
loop's reclamation is about), but the name is the block's.

## A name that isn't declared is an error

Reading a name nothing declares fails `hale check`, at the name:

```text
main.hl:3:18: type error: unknown identifier `totl`: no binding,
param, const or declaration with that name is in scope — did you
mean `total`?
```

`hale check app.hl` checks that file the way `hale build app.hl`
builds it: the file and what it imports, nothing else. So if your
program is several files in one directory and `main.hl` reads a
`const` that `limits.hl` declares, check the directory — `hale check
.` — not the one file; checked alone, the file is refused exactly as
its build would be, and the check adds a note saying the other files
beside it were not loaded.

A name a block bound and released (`if take { let x = 1; } return
x;`) is the block's, so reading it after the block is refused too,
and the message says the block has ended.

## A type name that isn't declared is an error too

The same rule holds where a *type* is written. The one that catches
everybody is capitalization — Hale's primitives are `Int`, `Float`,
`String`, not `int`, `float`, `string`:

```text
main.hl:1:16: type error: unknown type `int`: no type, enum, locus,
interface or alias with that name is declared — did you mean `Int`?
fn helper() -> int {
               ^^^
```

It is reported wherever the name is written: a parameter, a return
type, a struct field, a `params` field, a `let` ascription. Same
whole-program rule as above, and the same permissive reading for one
file of a multi-file seed — a sibling may declare the type.

A name with a `::` in it (`lib::Thing`, `std::text::Sink`) is not
held to this: it names something in another seed, which the checker
can only see once the whole seed is in front of it.

## A field declared twice is an error

A `type` names each field once, and an enum names each variant once.
Declaring `a` a second time in the same type is an error at the
second declaration, which names the first: `field `a` is already
declared in type `T``, with the first declaration's line beside it.
The same holds for a locus's or a perspective's `params`, a
`contract`'s members and an interface's methods — a name is one
slot.

## Framework words are names too

Hale's vocabulary is ordinary English: `run`, `epoch`, `restart`,
`capacity`, `tier`, `where`, `of`, `publish`. Each word means
something in one place — `run() { … }` at the head of a locus member,
`epoch tick;` inside a closure, `restart(c) for 3;` at the head of a
statement — and anywhere else it is a plain name:

```hale
type Job { run: Int; epoch: Int; restart: Bool; }

fn run(capacity: Int) -> Int {
    let where = capacity + 1;
    return where;
}
```

Two declarations stay refused, because the name would collide with
the syntax at its use. A locus method cannot be named like a
lifecycle hook or a mode — `fn run()` inside a locus is the `run`
hook spelled wrong, and the compiler says so:

```text
work.hl:2:8: parse error: a locus method cannot be named `run`: that
name is the `run` lifecycle hook — declare the hook as `run() { … }`
(no `fn`), or rename the method. …
```

And a free fn cannot be named like a recovery statement: `restart(x);`
at the head of a statement IS the recovery, so `fn restart(…)` could
never be called there.

A handful of words are reserved outright — the ones that start a
declaration or a statement anywhere (`fn`, `type`, `let`, `if`,
`match`, `return`, `self`, …) and the ones held for future syntax
(`async`, `await`, `trait`, `impl`, `macro`):

```hale,fragment
let match = 3;                    // no
```

The compiler says so at the word, once, naming what you were
declaring:

```text
work.hl:8:9: parse error: `match` is a reserved word and cannot
name a variable; rename it (Hale has no escaped-identifier form —
spec/tokens.md lists every reserved word)
        let match = 3;
            ^^^^^
```

There is no way to escape or quote a reserved word into a name.
Rename it and nothing else about the program changes. Every use of
the same word further down is part of that one mistake, so you get
one error, not one per line, and the rest of the file still parses
normally. The full list, with the one position each framework word
is a keyword in, lives in `spec/tokens.md`.

## The primitive types

These are the scalar types built into the language:

| Type | What it holds | Literal examples |
|---|---|---|
| `Int` | 64-bit signed integer | `0`, `42`, `1_000_000`, `0xFF`, `0b1010` |
| `Float` | 64-bit IEEE float | `3.14`, `1.0e-3`, `2.5` |
| `Bool` | true / false | `true`, `false` |
| `String` | UTF-8 text | `"hello"`, `"line\n"` |
| `Decimal` | exact fixed-point number | `1.50d`, `0.00d` |
| `Duration` | a span of time | `100ms`, `5s`, `90min` |
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
them into the text. Use `{{` and `}}` for literal braces. A plain
`"..."` string is *not* interpolated — `"x={x}"` prints the
braces — and the compiler warns if you write that by accident.

f-strings also take a format spec (`f"{n:>8}"`, `f"{r:.2}"`) and
render whole structs. Both are in
[Strings & text](./strings.md).

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
types.) If you reach for the spelling another language uses —
`s.len()`, `std::str::len(s)`, `std::math::abs(x)`,
`std::io::println(x)` — the checker names the builtin for you
rather than guessing at a near-miss stdlib name.

Printing isn't limited to single values: a struct, tuple or array
of printable things prints as a whole, recursively — which is
what you want when you're mid-debug and don't yet know which
field is wrong. Two things deliberately don't print: a **locus**
and **`Bytes`**. See
[Strings & text](./strings.md#printing-whole-values).

Next: [Math, money & time](./math.md).
