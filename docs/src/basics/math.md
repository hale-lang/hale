# Math, money & time

> Arithmetic, and three types that save you from classic bugs.

## Arithmetic

The operators are what you'd expect:

```hale
let a = 7 + 3;       // 10
let b = 7 - 3;       // 4
let c = 7 * 3;       // 21
let d = 7 / 3;       // 2   — integer division
let e = 7 % 3;       // 1   — remainder
```

Comparison and logic:

```hale
let bigger = a > b;          // Bool
let between = a > 0 && a < 100;
let either  = ready || forced;
let negated = !ready;
```

Bitwise operators (`& | ^ << >> ~`) are available on `Int`.

Comparisons don't chain: `a < b < c` is a parse error — write
`a < b && b < c`. This is deliberate; chained comparison is a
common source of silent bugs.

## Int and Float

`Int` is 64-bit signed; `Float` is a 64-bit IEEE double. Hale
widens `Int` to `Float` automatically where it's unambiguous —
at a `let` with a `Float` annotation, when passing an `Int` to a
`Float` parameter, and when one side of an arithmetic or
comparison operator is a `Float`:

```hale
let x: Float = 3;        // 3.0 — widened
let y = 2.0 * 3;         // 6.0 — Int 3 promoted to Float
```

Going the other way loses information, so it's explicit:

```hale
let n = Int(3.9);        // 3 — truncates toward zero
```

When you'd rather name the conversion — or need it mid-expression
where the implicit widening doesn't reach — `std::math` has both
directions as functions:

```hale
let f = std::math::int_to_float(42);     // 42.0
let m = std::math::float_to_int(3.99);   // 3 — round toward zero
```

They're the same `sitofp` / `fptosi` conversions as the casts,
just callable anywhere — so numeric code never has to launder a
value through `to_string` + `parse_float` to change its type.

The standard library covers the rest: `std::math::sqrt`,
`exp`, `log`, `pow`, `floor`, `ceil`, the trig functions, and
so on.

## Decimal — exact numbers

`Float` is wrong for money. `0.1 + 0.2` is not `0.3` in any
IEEE-float language, and rounding error compounds. Hale gives
you `Decimal`: a fixed-point type with exact arithmetic. Write
the literal with a `d` suffix.

```hale
let price = 19.99d;
let qty   = 3;
let total = price * 3;          // 59.97d — exact, no drift
```

Use `Decimal` for prices, balances, quantities, anything where a
penny of rounding error is a bug. Use `Float` for measurements,
ratios, and math where approximation is fine. The two never mix
implicitly — there is no silent `Decimal`/`Float` conversion, so
you can't accidentally launder exactness away.

## Duration — time spans with units

A duration is a length of time, written with a unit suffix:

```hale
let timeout = 5s;
let frame   = 16ms;
let day      = 24h;
let compound = 1h30m;          // durations add up
```

No more "is this milliseconds or seconds?" — the unit is part of
the literal. Durations do arithmetic and comparison:

```hale
let total = timeout + frame;
if elapsed > timeout { /* ... */ }
```

This is also what the runtime's sleep takes:

```hale
std::time::sleep(100ms);
```

## Time — wall-clock instants

A `Time` is a specific instant, written as an ISO-8601 literal in
backticks:

```hale
let launch = `2026-05-08T12:00:00Z`;
```

For *measuring elapsed time*, reach for the monotonic clock —
it never jumps backward when the wall clock is adjusted:

```hale
let start = std::time::monotonic();   // a Duration since boot
do_work();
let took = std::time::monotonic() - start;
println("took ", took);
```

`std::time::now()` gives wall-clock seconds since the Unix
epoch when you genuinely need calendar time; `monotonic()` is
the basis for anything timing-related.

## Why these are in the language

`Decimal`, `Duration`, and `Time` aren't library types you opt
into — they're primitives with their own literals. The reason is
that the bugs they prevent (float drift in money, unit confusion
in time) are *so common* and *so costly* that making them
first-class is worth it. You get the safety without importing
anything or remembering a convention.

Next: [Functions](./functions.md).
