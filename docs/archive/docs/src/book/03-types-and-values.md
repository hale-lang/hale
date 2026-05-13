# Types and values

[Chapter 2](./02-hello-locus.md) introduced one locus carrying one
`String` parameter. This chapter covers the language's value
model: the built-in primitive types, the composite types you build
out of them, and how mutability works at the binding level.

Locus types themselves are deferred to
[chapter 4](./04-locus-lifecycle.md); this chapter is about the
values that live *inside* loci.

## Built-in primitives

Aperio's primitive types are PascalCase identifiers (per **F.15**:
predefined type names are PascalCase, not keywords).

| Type | Width | Default literal | Example |
|---|---|---|---|
| `Int` | 8 bytes, signed | yes (integer literals) | `42`, `-7` |
| `Float` | 8 bytes, IEEE 754 | yes (float literals) | `3.14`, `-0.5` |
| `Decimal` | 16 bytes, fixed-precision | suffix `d` | `1.50d`, `100.40d` |
| `Bool` | 1 byte | — | `true`, `false` |
| `String` | UTF-8 bytes, arena-resident | yes (string literals) | `"hello"` |
| `Time` | 8 bytes, monotonic instant | — | `time::monotonic()` |
| `Duration` | 8 bytes, time interval | suffix literals | `5s`, `100ms` |
| `Bytes` | 8 bytes, length-prefixed blob ref | — (no literal codegen yet) | see [`std::bytes`](../std/bytes.md) |

`Bytes` is the binary-safe sibling of `String` — same single-pointer
ABI, but the underlying blob carries an explicit length prefix so
embedded NUL bytes survive. Reach for `Bytes` whenever a payload
might contain a zero byte (binary protocols, image bodies, file
uploads, masked WebSocket frames). The full surface
(`read_bytes`, `recv_bytes`, `at`, `slice`, `from_string`,
`from_bytes`) lives under [`std::bytes`](../std/bytes.md) and
[`std::str`](../std/str.md).

Numeric literals without a suffix default to `Int` (whole numbers)
or `Float` (with a decimal point). The `d` suffix on a numeric
literal makes it `Decimal`; this is the disambiguating tag because
Aperio does not implicitly convert between `Float` and `Decimal`.

`Decimal` is the type to reach for whenever a value represents
money, a price, or anything where rounding to binary fractions is
unacceptable. `Float` is for measurements where the fixed exponent
representation is the right shape.

```aperio
let bid: Decimal = 100.40d;
let ask: Decimal = 100.45d;
let spread: Decimal = ask - bid;
let mid: Decimal = (bid + ask) / 2.0d;
```

## Mutability

Bindings are immutable by default. `let x = 0;` produces an
immutable binding; reassigning `x` is a compile-time error. To
permit reassignment, use `let mut`.

```aperio
let n = 0;
n = 1;        // compile error: cannot assign to immutable binding `n`

let mut m = 0;
m = 1;        // ok
m += 5;       // ok — compound assignment also permitted
```

Mutability is a per-binding property, not a per-type property.
There is no `Mut<T>` wrapper; the binding either is or is not
`mut`. Locus parameter fields are an exception: `self.x = ...` is
permitted regardless because a locus's parameter struct is its
mutable state bundle (per **F.3**).

## Strings

Strings are UTF-8 byte sequences. They live in the arena where
they were produced — string literals come from a static region;
results of concatenation or slicing land in the caller's current
arena (a locus's own arena, or the program-wide arena from
`main`). Wholesale arena free at locus dissolve cleans them up
along with everything else.

The string surface:

```aperio
let s = "hello, world";
let n = len(s);             // byte length: Int
let g = "hi, " + name;      // concatenation; result in current arena
let eq = (s == "hello");    // equality is byte-wise
let head = s[0..5];         // exclusive slice: "hello"
let body = s[7..=11];       // inclusive slice: "world"
```

Slicing bounds are *clamped* rather than panicking — an
out-of-range index produces a (possibly empty) substring rather
than aborting the program. This matches the substrate's
"best-effort, predictable" stance over panic-on-error.

## User-defined records

A `type` declaration introduces a named record:

```aperio
type Point {
    x: Int;
    y: Int;
}

type Greeting {
    text: String;
    sender: String;
    priority: Int;
}
```

Fields are listed with their types, separated by semicolons.
Construct a record by naming it and supplying every field:

```aperio
let p = Point { x: 3, y: 4 };
let g = Greeting {
    text: "hello",
    sender: "alice",
    priority: 7,
};
```

Field order in the literal does not have to match the
declaration. Field access is `.name`:

```aperio
println("p.x=", p.x, " p.y=", p.y);
let q = Point { x: p.x + 10, y: p.y * 2 };
```

Records are the substrate of bus payloads — a typed bus subject
always carries a `type`. (The bus is introduced in
[chapter 6](./06-the-bus.md).)

## Tuples

Tuples are anonymous heterogeneous records — fixed arity, no field
names. They carry multi-value returns and provide multi-scrutinee
match without forcing a one-off `type`:

```aperio
fn divmod(a: Int, b: Int) -> (Int, Int) {
    return (a / b, a % b);
}

let result = divmod(17, 5);
println("quotient = ", result.0);     // numeric field access
println("remainder = ", result.1);

let (q, r) = divmod(23, 4);           // flat destructure
```

Tuples must have at least two elements — there is no unit `()`
type. Nested tuple destructuring (e.g. `let (a, (b, c)) = ...`)
is not in v0; flat destructure is supported, both in `let` and in
`match` arms.

## Arrays

Fixed-size arrays use `[T; N]` where `N` is a compile-time integer
literal:

```aperio
let nums = [10, 20, 30, 40, 50];     // [Int; 5]
let xs = [1, 2, 3, 4];                // [Int; 4]
println("first = ", nums[0]);
println("third = ", nums[2]);

for x in nums {
    println(x);
}

for i in 0..4 {
    println(xs[i]);
}
```

Element type is inferred from the literal's first element. Array
storage lives in the enclosing arena — a free function's array
dies when `main`'s program-wide arena tears down; a
locus-method's array dies with the locus.

There is no growable / dynamic array in v0. The substrate's
region allocator is wholesale-free, not per-object free, so a
dynamic `Vec` would need a separate growth-and-realloc lifetime
story. Fixed-size locks down the surface for v1; dynamic arrays
land later if a workload demands.

## Enums

An enum is a tagged union — one of several named variants, each
optionally carrying payload fields.

### No-payload enums

```aperio
type Light = enum { Red, Yellow, Green };

fn next(l: Light) -> Light {
    let mut out = Light::Red;
    match l {
        Light::Red    -> { out = Light::Green; },
        Light::Green  -> { out = Light::Yellow; },
        Light::Yellow -> { out = Light::Red; },
    }
    return out;
}
```

Construct a variant with `EnumName::VariantName`. Match arms
listing each variant make the match exhaustive (per **F.18**); a
non-exhaustive match is a compile-time error unless a wildcard
arm is present.

No-payload enums are represented as a 32-bit tag at runtime —
plain value semantics, no allocation.

### Payload enums

Variants may carry positional payload fields:

```aperio
type Event = enum {
    Tick(Int),
    Trade(Decimal, Int),
    Halt,
};

type Result = enum {
    Ok(Int),
    Err(String),
};

fn handle(e: Event) {
    match e {
        Event::Tick(0)            -> println("tick zero"),
        Event::Tick(n)            -> println("tick #", n),
        Event::Trade(price, size) -> println("trade ", size, " @ ", price),
        Event::Halt               -> println("halt"),
    }
}
```

A variant with payload is constructed by passing positional
arguments: `Event::Tick(7)`, `Event::Trade(99.95d, 100)`. Match
arms bind each payload position to a name — `Event::Tick(n)` binds
the integer payload to `n` for the arm body. Literal sub-patterns
(`Event::Tick(0)`) match a specific value before more general arms
in the same match.

Payload enums are stored as a pointer to a `{ tag, body }` struct
allocated in the current arena; the body is sized to the largest
variant's payload. Mixing payload-bearing and no-payload variants
in the same enum is permitted; pure no-payload enums keep the
plain-tag representation.

The standard library's `Result<T, E>` and `Option<T>` are
generic payload enums (introduced in
[chapter 10](./10-generics.md)).

## Operators

Aperio's operator surface is conventional. Full precedence and
associativity tables live in `spec/precedence.md` in the source
tree; the headlines:

- **Arithmetic.** `+ - * / %` on `Int`, `Float`, `Decimal`. One
  implicit widening: `Int → Float` fires at let-binding type
  ascriptions (`let nf: Float = n;` where `n: Int`) and at
  function-argument sites where the parameter is `Float` and
  the argument is `Int`. The widening is one-way only —
  `Float → Int` narrowing stays explicit, and `Decimal` never
  participates in implicit cross-type conversion.
- **Comparison.** `< > <= >=` are *non-associative* — `a < b < c`
  is a parse error. Use `a < b && b < c`. This avoids the C
  chained-comparison surprise.
- **Equality.** `== !=`. Deep equality across records, tuples,
  arrays, and payload enums.
- **Logical.** `&& ||`, short-circuiting. `!` for negation.
- **Bitwise.** `& | ^ ~ << >>` on integer types.
- **Range.** `..` (exclusive) and `..=` (inclusive). Used in
  `for` headers and string / array slicing.
- **Assignment.** `=` plus the compound forms `+= -= *= /= %= &=
  |= ^=`.

Two operators are non-conventional and load-bearing for the
substrate; both are introduced in their own chapters:

- **`~~`** — approximate equality, permitted only inside a
  `closure` block's assertion clause. See
  [chapter 7](./07-closures.md).
- **`<-`** — bus send, statement position only. See
  [chapter 6](./06-the-bus.md).

## Block-value expressions

An `{ ... }` block ending in an expression *without* a trailing
`;` is itself an expression — the trailing expression is the
block's value. `if` works the same way when both arms have a
value-shaped tail: the result of the chosen arm is the if's
value.

```aperio
fn main() {
    let cond: Bool = true;
    let x: Int = if cond { 10 } else { 20 };
    println("x=", x);
}
```

A block can carry its own let-bindings before the tail —
they're scoped to the block:

```aperio
let r: Int = if cond {
    let t: Int = 21;
    t * 2          // block tail — `t` doesn't escape
} else {
    0
};
```

If-as-expression requires an `else` branch (an `if` without
`else` has no value to merge on the missing path); the two
arms' tail types must match. Mixed-type arms are a typecheck
error.

The same trailing-expression form also lets `{ }` be used as
an expression directly:

```aperio
let phase: Int = { let n = step + 1; n % 4 };
```

Use this when a short multi-statement computation feeds one
let-binding; for anything larger, factor into a free fn.

## What this chapter does not cover

- **Locus types** — `locus L { ... }` — appear throughout the
  rest of the book, beginning with the lifecycle chapter.
- **Function types as values** — `fn(A, B) -> C` as a parameter
  type — appears in later chapters where it matters (closures,
  higher-order helpers).
- **Generics** — `Stack<T>`, `Result<T, E>`, the `Numeric`
  bound — see [chapter 10](./10-generics.md).
- **Projection classes** — `Rich<T>` / `Chunked<T>` /
  `Recognition<T>` and the `ProjectionClass` constraint (per
  **F.2**) — see
  [chapter 11](./11-perspectives.md).
- **Perspective types** — `perspective P { ... }` — also in
  chapter 11.

The next chapter, **[Locus lifecycle](./04-locus-lifecycle.md)**,
returns to the runtime side: every locus's existence is the same
four beats (`birth` / `run` / `drain` / `dissolve`), and that
shape is the foundation everything else builds on.
