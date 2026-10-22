# Math, Money & Time

Hale provides standard arithmetic and bitwise operators alongside first-class types for exact fixed-point decimals (`Decimal`), nanosecond time spans (`Duration`), UTC timestamps (`Time`), and bounded or distinct integers. Financial calculations, unit conversions, and time arithmetic are checked at compile time with explicit rounding policies.

```hale
fn main() {
    let unit_price: Decimal<2> = 19.99d;
    let total = unit_price * 3d;               // 59.97 (exact Decimal<2>)

    let start = `2026-05-08T12:00:00Z`;
    let deadline = start + 1h + 30min;         // 2026-05-08T13:30:00Z
    println(f"total={std::decimal::format(total, 2)} deadline={deadline}");
}
```

## Numeric, Logical, and Bitwise Operators

| Category | Operators | Supported Types | Behavior & Notes |
|---|---|---|---|
| Arithmetic | `+`, `-`, `*`, `/` | `Int`, `Float`, `Decimal`, `Duration` (restricted) | Integer `/` truncates toward zero; `String` also supports `+` |
| Remainder | `%` | `Int`, `Decimal` | Not defined on `Float` |
| Equality | `==`, `!=` | `Int`, `Float`, `Decimal`, `Bool`, `String`, `Duration`, `Time`, `enum`, `distinct Int` | Not defined on structs, tuples, or arrays (compare field by field) |
| Ordering | `<`, `<=`, `>`, `>=` | `Int`, `Float`, `Decimal`, `String`, `Duration`, `Time` | Does not chain; write `a < b && b < c` |
| Logical | `&&`, `\|\|`, `!` | `Bool` | `&&` and `\|\|` short-circuit |
| Bitwise | `&`, `\|`, `^`, `<<`, `>>`, `~` | `Int` | Bitwise AND, OR, XOR, shifts, and NOT |

```hale,fragment
let a = 7 + 3;                         // 10
let d = 7 / 3;                         // 2 (integer division)
let e = 7 % 3;                         // 1 (remainder)

let between = a > 0 && a < 100;        // chained comparisons require &&
let safe = d != 0 && a / d > 2;        // short-circuits before division when d == 0
let masked = (a << 2) & 0xFF;          // bitwise operations on Int
```

> [!NOTE]
> Structs, tuples, and arrays do not overload `==` or other operators. Attempting `p1 == p2` on a struct produces a compile error instructing you to compare its fields explicitly.

## `Int`, `Float`, and `Decimal` Overview

| Type | Representation | Implicit Conversions | Primary Use Case |
|---|---|---|---|
| `Int` | 64-bit signed integer (`i64`) | Widens to `Float` in unambiguous contexts | Counts, indices, bitwise logic |
| `Float` | 64-bit IEEE-754 double (`f64`) | None | Scientific math, ratios, physical measurements |
| `Decimal` (`Decimal<9>`) | 128-bit fixed-point, 9 fractional places | Widens to `Decimal<S>` where `S > 9` via `.rescale(S)` | General exact decimal arithmetic |
| `Decimal<S>` (`S` in `0..=18`) | 128-bit fixed-point, `S` fractional places | None (distinct per scale `S`) | Currency (`Decimal<2>`), high-precision tokens (`Decimal<18>`) |

## `Int` and `Float` Conversions and Math

### Implicit Widening and Explicit Casts

Hale automatically widens `Int` to `Float` in five unambiguous contexts:
1. At a `let` binding with a `: Float` type annotation (`let x: Float = n;`).
2. When passing an `Int` argument to a `Float` function parameter (`std::math::sqrt(16)`).
3. In binary arithmetic or comparisons where the other operand is already `Float` (`2.0 * 3`).
4. In a `Float` field of a `type` struct literal.
5. When returning an `Int` from an infallible `-> Float` function (`fn f(n: Int) -> Float { return n; }`).

> [!IMPORTANT]
> Implicit widening applies **only** to `Int`. Unit quantities (such as `5ms`), `fallible` function return paths, and `locus` `params` fields (`Worker { rate: 3.0 }`) do not implicitly widen.

Where both operands are `Int` or when converting `Float` to `Int`, use the built-in `Float(x)` and `Int(x)` casts or their `std::math` equivalents:

```hale,fragment
let hits = 3;
let total = 8;
let rate = Float(hits) / Float(total);   // 0.375 (widens before division)
let truncated = Int(3.9);                // 3 (truncates toward zero)
```

`Int(x)` and `Float(x)` are the only primitive cast expressions in the language; type names like `String(x)`, `Bool(x)`, and `Bytes(x)` are not casts and are rejected at compile time.

### `std::math` Functions and IEEE-754 Semantics

| Function | Signature | Description |
|---|---|---|
| `abs(x)` | `(T) -> T` (builtin) | Absolute value (`Int`, `Float`, `Decimal`, `Duration`) |
| `min(a, b)` / `max(a, b)` | `(T, T) -> T` (builtin) | Minimum / maximum; returns `NaN` if either `Float` argument is `NaN` |
| `std::math::int_to_float(n)` | `(Int) -> Float` | Converts `Int` to `Float` (equivalent to `Float(n)`) |
| `std::math::float_to_int(x)` | `(Float) -> Int` | Truncates `Float` toward zero (equivalent to `Int(x)`) |
| `std::math::round(x)` | `(Float) -> Int` | Rounds half away from zero (`2.5 -> 3`, `-2.5 -> -3`) |
| `std::math::trunc(x)` | `(Float) -> Int` | Truncates toward zero to `Int` |
| `std::math::floor(x)` / `ceil(x)` | `(Float) -> Float` | Rounds down / up to nearest whole `Float` |
| `std::math::sqrt(x)` / `pow(x, y)` | `(Float, ...) -> Float` | Square root and exponentiation |
| `std::math::exp(x)` / `log(x)` | `(Float) -> Float` | Natural exponential and logarithm |
| `std::math::nan()` / `is_nan(x)` | `() -> Float` / `(Float) -> Bool` | Produces or tests IEEE-754 `NaN` |

`Float` comparisons follow IEEE-754 rules:
- `NaN` is unequal to everything, including itself (`nan != nan` is `true`; `==`, `<`, `<=`, `>`, `>=` are `false`).
- `-0.0 == 0.0` evaluates to `true`.
- Printing a `Float` via `println` or `to_string` outputs the shortest decimal representation that round-trips back to the exact same IEEE-754 double via `std::str::parse_float`. Use `f"{x:.2}"` when formatting for display.

## Exact Fixed-Point Numbers (`Decimal` and `Decimal<S>`)

Use `Decimal` for prices, balances, and financial quantities where binary floating-point drift (`0.1 + 0.2 != 0.3`) is unacceptable. Write `Decimal` literals with a `d` suffix:

```hale,fragment
let price: Decimal<2>  = 19.99d;
let wei:   Decimal<18> = 0.000000000000000001d;
let total = price * 3d;                          // 59.97 (Decimal<2>)
```

### Scale Rules and `.rescale(S)`

- **Parameterized scale:** `Decimal<S>` stores `S` digits after the decimal point (`0 <= S <= 18`). Unparameterized `Decimal` is an alias for `Decimal<9>`.
- **Compile-time scale checking:** A `d` literal adopts the scale expected by its context (type annotation, parameter, struct field, binary operator, or `if`/`match` arm). If the literal has nonzero digits beyond scale `S` (such as `let p: Decimal<2> = 1.505d;` or `0.1234567891d` for `Decimal<9>`), compilation fails at the literal. Similarly, `std::str::parse_decimal` fails into its `or` branch if the input string has more fractional digits than the target scale.
- **No implicit scale or type mixing:** `Decimal<2>`, `Decimal<9>`, and `Decimal<18>` are distinct types, and `Decimal` never mixes implicitly with `Int` or `Float` (multiply by `3d`, not `3`).
- **Arithmetic truncation:** Arithmetic within `Decimal<S>` truncates toward zero past the `S`th fractional digit (`1.0d / 3.0d` is `0.333333333d`).
- **Rescaling:** Use `.rescale(S)` to convert between scales. Widening to a larger `S` is exact; narrowing to a smaller `S` can drop nonzero digits and requires an `or` rounding or failure policy:

```hale,fragment
let precise: Decimal<18> = 1d / 3d;          // 0.333333333333333333
let cents = precise.rescale(2) or half_even; // 0.33 (rounded)
let exact = precise.rescale(2) or raise;     // fails if nonzero digits are dropped
let wide  = cents.rescale(18);               // exact widening, no `or` needed
```

Supported narrowing policies on `.rescale(S)` are `or trunc`, `or floor`, `or ceil`, `or half_up`, `or half_even`, `or <fallback>`, `or raise`, and `or handler(err)`.

### Formatting and Converting `Decimal`

Printing a `Decimal<S>` trims trailing fractional zeros (`12.50d` prints as `12.5`), serializes exact digits in JSON, and occupies 16 bytes on the bus regardless of scale.

| Function | Signature | Description |
|---|---|---|
| `std::decimal::format(d, places)` | `(Decimal<S>, Int) -> String` | Formats with exact fractional `places` using `half_up` rounding |
| `std::decimal::from_int(n)` | `(Int) -> Decimal<9>` | Exact conversion from `Int` (chain `.rescale(S)` for other scales) |
| `std::decimal::from_float(x)` | `(Float) -> Decimal<9> fallible(...)` | Converts `Float` rounded half-away-from-zero at the 9th place; fails on `NaN`, `Inf`, or `\|x\| > ~1.7e29` |
| `std::decimal::to_float(d)` | `(Decimal<S>) -> Float` | Converts `Decimal` to closest IEEE-754 `Float` |

## `Duration` and `Time`

`Duration` and `Time` are defined by the standard library using Hale's unit system: `Duration` is `quantity Int in ns`, and `Time` is `point Duration` (nanoseconds since the Unix epoch in UTC).

### Time Units and `std::time` Reference

| Unit / Function | Definition / Signature | Description |
|---|---|---|
| `ns`, `us`, `ms`, `s`, `min`, `h`, `day` | `1us = 1000ns`, `1ms = 1000us`, `1s = 1000ms`, `1min = 60s`, `1h = 60min`, `1day = 24h` | Duration literal suffixes (write `1h + 30min`, never `1h30m`, `5m`, or `3d`) |
| `std::time::sleep(d)` | `(Duration) -> ()` | Sleeps for duration `d` |
| `std::time::monotonic()` | `() -> Duration` | Monotonic elapsed time since boot (never jumps backward; use for benchmarks/timers) |
| `std::time::current()` | `() -> Time` | Current UTC wall-clock instant (`Time`) |
| `std::time::now()` | `() -> Int` | Current UTC Unix timestamp in whole seconds (`Int`) |
| `std::time::iso8601(t)` | `(Time) -> String` | Formats `Time` as an ISO-8601 UTC string |
| `std::time::parse_time(s)` | `(String) -> Time fallible(ParseError)` | Parses an ISO-8601 UTC string (validates leap years and calendar dates) |
| `std::time::parse_iso8601(s)` | `(String) -> Int fallible(ParseError)` | Parses an ISO-8601 string to Unix epoch seconds (`Int`), supporting dates outside 1677–2262 |
| `std::time::unix(t)` / `nanos(t)` | `(Time) -> Int` | Extracts Unix seconds or epoch nanoseconds from `Time` |
| `std::time::time_from_unix(secs)` / `from_nanos(n)` | `(Int) -> Time` | Constructs `Time` from seconds or nanoseconds |

### `Duration` Arithmetic

Every `Duration` counts nanoseconds (`println(16ms)` outputs `16000000ns`). Durations support addition, subtraction, comparison, and integer scaling:

```hale,fragment
let timeout = 5s;
let frame   = 16ms;
let compound = 1h + 30min;

let backoff = tries * 100ms;            // Int * Duration -> Duration
let slice   = timeout / workers;        // Duration / runtime Int -> Duration (integer division)
let half    = timeout / 2 or floor;     // Duration / literal Int -> requires rounding policy
let frames  = timeout / frame;          // Duration / Duration -> Int
```

Dividing a `Duration` by a compile-time integer literal requires an explicit remainder policy (`or floor`, `or ceil`, `or trunc`, `or half_up`, `or half_even`, or `or <fallback>`). Multiplying two durations (`Duration * Duration`) is a compile error.

### `Time` Instants

Write `Time` literals as UTC ISO-8601 timestamps enclosed in backticks:

```hale,fragment
let launch   = `2026-05-08T12:00:00Z`;
let precise  = `2026-09-14T08:30:15.25Z`;
let deadline = launch + 30min;
let slack    = deadline - std::time::current();   // Duration
```

- **UTC-only literals:** Non-UTC offsets (such as `` `2026-05-08T12:00:00+01:00` ``) are rejected at compile time.
- **64-bit nanosecond range (1677–2262):** `Time` spans years `1677` through `2262`. Out-of-range literals fail at compile time, `parse_time` takes its `or` branch, and runtime arithmetic (`t + d` or `time_from_unix`) that overflows halts with a `violation:`. Use `std::time::parse_iso8601(s)` when working with calendar timestamps outside 1677–2262 as `Int` seconds.

## Distinct Identities, Bounded Ranges, and Custom Units

### Identities (`distinct Int`) and Ranges (`range:`)

Use `distinct Int` to create strongly typed integer IDs that cannot be added or mixed with plain `Int`, and `{ range: low..high; }` to constrain valid integer bounds:

```hale
type OrderId = distinct Int;
type Byte    = Int { range: 0..256; }
type Session = distinct Int { range: 0..64; }

fn main() {
    let id: OrderId = 4;
    let b: Byte = 200;
    let total: Int = b + 55;         // range types widen to Int automatically
    let slot = Session(70) or 0;     // 70 is out of range 0..64 -> substitutes 0
    let top = Byte(total + 10) or clamp;
    let raw: Int = Int(id);          // explicit unwrap of distinct Int
    println(id, " ", total, " ", slot, " ", top, " ", raw);
}
```

Narrowing an integer into a ranged type requires an `or` clause (`or <fallback>`, `or clamp`, `or wrap`, `or handler(err)`, or `or raise`). Out-of-range literals (`let b: Byte = 300;`) fail at compile time.

### Custom Units and Quantities

You can declare domain-specific units (cents, dollars, basis points, bytes, ticks) with `unit`, `quantity`, and `point`:

```hale
unit cent;
unit USD = 100 cent;
unit pct = 1/100;
unit bp = 1/100 pct;

type Money  = quantity Int in cent;
type Ledger = Money { round: half_even; }
type Rate   = quantity Int in bp;

fn main() {
    let fee = 1_250_000USD * 3bp;     // exact: 375000000 Money in 1/10000 cent (375 USD)
    let odd = 1_234_567USD * 3bp;     // exact: 370.3701 USD in 1/10000 cent
    let paid: Ledger = odd;           // rounds via Ledger's half_even policy
    let cents = odd.in(cent) or ceil; // explicit site-level rounding
    println(fee, " ", odd, " ", paid, " ", cents);
}
```

See [Units, End to End](../units.md) for the complete guide to custom units, affine points, unit splitting (`.split(u)`), and `hale check --units`.
