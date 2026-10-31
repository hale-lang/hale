# Values & Variables

Hale is a statically typed systems language where local bindings are immutable by default and primitive types cover integers, floats, exact decimals, timestamps, durations, strings, and byte buffers. The compiler infers types from initializers while allowing explicit type annotations on any binding.

```hale
const MAX_RETRIES = 3;

fn main() {
    let service = "auth";
    let timeout: Duration = 250ms;
    let mut attempts = 0;

    attempts = attempts + 1;
    println(f"service={service} attempts={attempts}/{MAX_RETRIES} timeout={timeout}");
}
```

## Primitive and Value Types

Hale provides eight built-in scalar types and two inline sequence types. `Decimal`, `Duration`, and `Time` are first-class language primitives with dedicated literal syntax rather than library wrappers.

| Type | Description | Literal / Syntax Examples | Notes & Defaults |
|---|---|---|---|
| `Int` | 64-bit signed integer | `0`, `42`, `1_000_000`, `0xFF`, `0b1010` | Default type for integer literals |
| `Float` | 64-bit IEEE-754 float | `3.14`, `1.0e-3`, `2.5` | Default type for fractional literals |
| `Decimal` | Exact fixed-point number (`Decimal<9>`) | `1.50d`, `0.00d` | `d` suffix; parameterized as `Decimal<S>` (`S` in `0..=18`) |
| `Bool` | Boolean truth value | `true`, `false` | Supports `==`, `!=`, `&&`, `\|\|`, `!` |
| `String` | UTF-8 text | `"hello"`, `f"val={x}"`, `"""..."""` | Immutable UTF-8 byte sequence |
| `Bytes` | Raw binary data | `b"\x00\x01\xff"`, `b"raw"` | Arbitrary octets, including null bytes |
| `Duration` | Nanosecond time span | `100ns`, `50us`, `16ms`, `5s`, `90min`, `8h`, `7day` | `quantity Int in ns` |
| `Time` | UTC wall-clock instant | `` `2026-05-08T12:00:00Z` `` | `point Duration` (nanoseconds since Unix epoch) |
| `[T; N]` | Fixed-length array | `[1, 2, 3]`, `[0; 8]` | Inline fixed capacity `N` |
| `bounded[T; N]` | Fixed-capacity dynamic list | `tags: bounded[String; 32];` | Inline in `type` structs; starts empty (`count = 0`) |

Numeric literals accept underscores (`1_000_000`) as visual separators. Integer literals default to `Int`, fractional literals default to `Float`, and the `d` suffix produces an exact `Decimal` (covered in [Math, Money & Time](./math.md)).

## Variables (`let` and `let mut`)

Introduce local variables with `let`. Type annotations are optional when the compiler can infer the type from the initializer:

```hale,fragment
let greeting = "hello";
let count: Int = 3;
let ratio = 0.5;
let ready = true;
```

Bindings declared with `let` are immutable and cannot be reassigned. Add `mut` to allow reassignment:

```hale,fragment
let total = 0;
total = total + 1;        // compile error: `total` is immutable

let mut total = 0;
total = total + 1;        // valid
```

### Scope and Shadowing

A `let` binding is visible from its declaration to the end of the enclosing `{ ... }` block:

- **Same-block rebinding:** Declaring a second `let x = ...;` later in the same block rebinds `x` to the new value for the remainder of that block.
- **Nested-block shadowing:** Declaring `let x = ...;` inside an inner block shadows an outer `let x` or function parameter for the duration of the inner block. Once the inner block exits, the outer binding is visible again.
- **Block lifetime:** Reading a variable outside the block where it was declared (`if take { let x = 1; } return x;`) is a compile error. While the underlying stack/frame slot lives until the function exits, the identifier itself is scoped strictly to its block.

## Top-Level Constants (`const`)

Program-wide constants are declared at the top level using `const` and `SCREAMING_SNAKE_CASE`:

```hale
const LIMIT = 100;
const SCALED_LIMIT: Int = LIMIT * 2;

fn main() {
    println(SCALED_LIMIT);
}
```

When the initializer is a literal (`const LIMIT = 100;`), the compiler infers the type directly from the literal. Any compound or computed initializer must state its type explicitly (`const SCALED_LIMIT: Int = ...;`).

> [!IMPORTANT]
> `const` declarations are valid **only at the top level** of a file alongside `locus`, `type`, and `fn` declarations, and are automatically in scope inside every locus in the seed. Writing `const` inside a `locus` body is a compile error; if each locus instance needs a configurable field with a default value, declare it in `params { limit: Int = 7; }` and access it via `self.limit`.

## Strings and Interpolation

Double-quoted string literals support standard escape sequences (`\n`, `\t`, `\"`, `\\`, `\xNN`). Prefixing a string literal with `f` enables expression interpolation inside `{...}`:

```hale,fragment
let name = "world";
let hi   = f"hello {name}";           // "hello world"
let spec = f"{42:>8} | {3.14159:.2}"; // "      42 | 3.14"
```

Use `{{` and `}}` to write literal braces inside an f-string. Plain `"..."` strings do not interpolate—writing `"x={x}"` leaves the literal braces in the string and triggers a compiler warning if `x` is in scope.

Multi-line raw text (such as embedded SQL queries, JSON payloads, or source code) uses triple-quoted **block strings** (`"""`), covered in [Strings & Text](./strings.md#block-strings).

## Printing and Builtin Functions

`println`, `print`, `eprintln`, `eprint`, `to_string`, and `len` are compiler builtins called as plain functions rather than methods:

```hale
type Point { x: Int; y: Int; }

fn main() {
    let n = 41;
    println("n + 1 = ", n + 1);        // n + 1 = 42

    let s = to_string(n + 1);          // "42"
    println("len = ", len(s));         // len = 2

    let p = Point { x: 3, y: 4 };
    println(p);                        // Point { x: 3, y: 4 }
}
```

- `println` and `print` accept any number of arguments and concatenate their output without format strings.
- Structs, tuples, and arrays of printable elements format recursively. Two types deliberately do not print directly: **loci** (which represent stateful components rather than values) and **`Bytes`** (which must be explicitly formatted as hex, length, or decoded text; see [Strings & Text](./strings.md#printing-whole-values)).
- Builtins use function call syntax (`len(s)`, `abs(x)`), not method call syntax (`s.len()`) or stdlib paths (`std::str::len(s)`, `std::math::abs(x)`, `std::io::println(x)`). If you write one of those forms, `hale check` points directly to the builtin.

## Compiler Rules and Identifier Resolution

### Undeclared Value and Type Names

Referencing an undeclared variable, constant, or type fails `hale check` at the identifier site:

| Mistake | Example | Compiler Diagnostic / Fix |
|---|---|---|
| Misspelled variable or constant | `return totl;` | Reports `unknown identifier` and suggests `total` if in scope |
| Lowercase primitive type name | `fn f() -> int` | Reports `unknown type int — did you mean Int?` (`Int`, `Float`, `String`, `Bool` are PascalCase) |
| Duplicate field or variant | `type T { a: Int; a: Int; }` | Reports duplicate declaration and cites the line of the first declaration (applies to `type`, `enum`, `params`, `contract`, and `interface`) |
| Single-file check in multi-file seed | `hale check main.hl` | Checks only `main.hl` and its explicit `import`s; run `hale check .` to check all sibling `.hl` files in the directory together |

Cross-seed qualified type names containing `::` (such as `lib::Thing` or `std::text::Sink`) are resolved when checking the seed as a whole.

### Contextual Keywords vs. Reserved Words

Most Hale framework terms (`run`, `epoch`, `restart`, `capacity`, `tier`, `where`, `of`, `publish`) are **contextual keywords**: they act as keywords only in their designated grammatical positions and can be used as ordinary variable, parameter, or field names everywhere else:

```hale
type Job { run: Int; epoch: Int; restart: Bool; }

fn run(capacity: Int) -> Int {
    let where = capacity + 1;
    return where;
}
```

Two declaration positions disallow contextual keywords to avoid ambiguity:

1. **Locus methods cannot use lifecycle hook or mode names:** Writing `fn run()` inside a `locus` is rejected because `run() { ... }` (without `fn`) is the lifecycle hook syntax.
2. **Free functions cannot use recovery statement names:** Writing `fn restart(...)` is rejected because `restart(x);` at statement start is the supervision recovery command.

Core grammatical keywords and reserved future keywords cannot be used as identifiers anywhere, and Hale has no raw/escaped identifier syntax (full list in `spec/tokens.md`):

- **Active keywords:** `fn`, `type`, `let`, `if`, `else`, `match`, `while`, `for`, `in`, `return`, `self`, `locus`, `const`, `true`, `false`, `or`, `fail`
- **Reserved words:** `async`, `await`, `trait`, `impl`, `macro`

Next: [Math, Money & Time](./math.md).
