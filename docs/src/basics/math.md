# Math, money & time

> Arithmetic, and three types that save you from classic bugs.

## Arithmetic

The operators are what you'd expect:

```hale,fragment
let a = 7 + 3;       // 10
let b = 7 - 3;       // 4
let c = 7 * 3;       // 21
let d = 7 / 3;       // 2   — integer division
let e = 7 % 3;       // 1   — remainder
```

Comparison and logic:

```hale,fragment
let bigger = a > b;          // Bool
let between = a > 0 && a < 100;
let either  = ready || forced;
let negated = !ready;
```

Bitwise operators (`& | ^ << >> ~`) are available on `Int`.

Comparisons don't chain: write `a < b && b < c`. On numbers,
`a < b < c` is refused — it would compare the `Bool` from `a < b`
with an `Int` — so the maths-notation habit can't slip through
silently.

`&&` and `||` short-circuit: the right side is evaluated only when
the left side doesn't already decide the answer. So a guard protects
what follows it:

```hale,fragment
let safe = d != 0 && total / d > 10;   // never divides by zero
let found = cached || lookup(key);     // lookup runs only on a miss
```

## Int and Float

`Int` is 64-bit signed; `Float` is a 64-bit IEEE double. Hale
widens `Int` to `Float` automatically where it's unambiguous —
when passing an `Int` to a `Float` parameter, and when one side
of an arithmetic or comparison operator is a `Float`:

```hale,fragment
let r = std::math::sqrt(16);   // 4.0 — the Int argument widens
let y = 2.0 * 3;               // 6.0 — Int 3 promoted to Float
```

A `let` annotation does not widen: `let x: Float = 3;` is refused,
so write `3.0` or `Float(n)`.

Going the other way loses information, so it's explicit:

```hale,fragment
let n = Int(3.9);        // 3 — truncates toward zero
```

`Float(x)` is the same cast in the widening direction, for the
places the implicit rule doesn't reach — mid-expression, most
often:

```hale,fragment
let hits = 3;
let total = 8;
let rate = Float(hits) / Float(total);   // 0.375, not 0
```

Those two are the only casts. `String`, `Bool`, `Bytes` and the
rest are type names, not conversions: `String(x)` calls nothing
and is refused.

When you'd rather name the conversion — or need it mid-expression
where the implicit widening doesn't reach — `std::math` has both
directions as functions:

```hale,fragment
let f = std::math::int_to_float(42);     // 42.0
let m = std::math::float_to_int(3.99);   // 3 — round toward zero
```

They're the same `sitofp` / `fptosi` conversions as the casts,
just callable anywhere — so numeric code never has to launder a
value through `to_string` + `parse_float` to change its type.

When you want a Float *rounded* to an `Int` rather than
truncated — building an integer field out of a Float quantity,
say — reach for `round`; `trunc` is the toward-zero sibling:

```hale,fragment
let a = std::math::round(3.7);          // 4   (Int)
let b = std::math::round(2.5);          // 3   — half away from zero
let c = std::math::round(0.0 - 2.5);    // -3
let d = std::math::trunc(3.7);          // 3   — toward zero, like float_to_int
```

Both return an `Int` directly. (`floor` / `ceil` below return a
`Float`; wrap them in `float_to_int` if you need an `Int`.)

The standard library covers the rest: `std::math::sqrt`,
`exp`, `log`, `pow`, `floor`, `ceil`, the trig functions, and
so on.

## Decimal — exact numbers

`Float` is wrong for money. `0.1 + 0.2` is not `0.3` in any
IEEE-float language, and rounding error compounds. Hale gives
you `Decimal`: a fixed-point type with exact arithmetic. Write
the literal with a `d` suffix.

```hale,fragment
let price = 19.99d;
let qty   = 3.0d;
let total = price * qty;        // 59.97 — exact, no drift
```

Printing trims trailing zeros (`12.50d` prints as `12.5` — the
value is the same number); when a display needs fixed places,
`std::decimal::format(price, 2)` renders exactly two fraction
digits with half-up rounding.

Use `Decimal` for prices, balances, quantities, anything where a
penny of rounding error is a bug. Use `Float` for measurements,
ratios, and math where approximation is fine. The two never mix
implicitly — there is no silent `Decimal`/`Float` conversion, and
not even `Decimal * Int` (the quantity above is `3.0d`, not `3`), so
you can't accidentally launder exactness away.

## Duration — time spans with units

A duration is a length of time, written with a unit: `ns`, `us`,
`ms`, `s`, `min`, `h` or `day`.

```hale,fragment
let timeout = 5s;
let frame   = 16ms;
let shift    = 8h;
let week     = 7day;
let compound = 1h + 30min;      // durations add up
```

A literal has one unit, so a mixed span is a sum: `1h30m` is refused,
the message saying to write `1h + 30min`. Minutes are `min` (`5m` is
refused and says so), and a day is `day`: `3d` is the Decimal `3`
(above), never three days.

No more "is this milliseconds or seconds?" — the unit is part of
the literal. Those units are not built into the compiler: the standard
library declares them, the way a program declares its own units
(`unit tick = 10 ms;` joins them), and `Duration` is the quantity they
count, in nanoseconds:

```hale,fragment
unit ns;
unit us = 1_000 ns;
unit ms = 1_000 us;
unit s = 1_000 ms;
unit min = 60 s;
unit h = 60 min;
unit day = 24 h;
type Duration = quantity Int in ns;
type Time = point Duration;
```

So every duration literal is a count of nanoseconds (`16ms` is
16,000,000), and a duration prints as one: `println(frame)` shows
`16000000ns`. Durations do arithmetic and comparison:

```hale,fragment
let total = timeout + frame;
if elapsed > timeout { /* ... */ }
```

They also scale by plain integers — the shape you need when the
number of units arrives at runtime (a computed retry count, a
millisecond value from an FFI boundary):

```hale,fragment
let backoff = tries * 100ms;            // Int * Duration → Duration
let slice   = timeout / workers;        // Duration / Int → Duration
let half    = timeout / 2 or floor;     // a literal divisor: say how to round
let frames  = timeout / frame;          // Duration / Duration → Int
fn sleep_ms(ms: Int) { std::time::sleep(ms * 1ms); }
```

Dividing by a number you wrote down is a question with a remainder
(5s / 3 is not a whole number of nanoseconds), so the program says
what happens to it: `or floor`, `or ceil`, `or trunc`, `or half_up`,
`or half_even`, or `or <value>` for a duration to use when it does
not divide evenly. A divisor known only at run time divides as
integers do. Two durations divide to a plain count. (`Duration *
Duration` is rejected — ns² isn't a thing.)

This is also what the runtime's sleep takes:

```hale,fragment
std::time::sleep(100ms);
```

## Time — wall-clock instants

A `Time` is a specific instant — nanoseconds since the Unix epoch,
UTC — written as an ISO-8601 literal in backticks:

```hale,fragment
let launch = `2026-05-08T12:00:00Z`;
let precise = `2026-09-14T08:30:15.25Z`;   // a fraction, up to nanoseconds
```

The literal is checked when the program is: `` `2026-05-08T12:00:00+01:00` ``
is a compile error, because a local time read as UTC in silence is the
bug this type exists to prevent. An instant shifts by a `Duration` and
two instants differ by one — those are the only arithmetic it admits —
and it orders:

```hale,fragment
let deadline = launch + 30min;
let slack = deadline - std::time::current();    // a Duration
if std::time::current() > deadline { escalate(); }
println(deadline);                                // 2026-05-08T12:30:00Z
```

`std::time::current()` is the wall clock as a `Time`; `iso8601(t)` and
`parse_time(s)` go to and from text (`parse_time` is fallible, so a
malformed string is an `or` branch, not a sentinel); `unix(t)`,
`nanos(t)`, `from_nanos(n)` and `time_from_unix(secs)` are the integer
views. `now()` keeps returning epoch seconds as an `Int` for code that
wants a number.

For *measuring elapsed time*, reach for the monotonic clock —
it never jumps backward when the wall clock is adjusted:

```hale,fragment
let start = std::time::monotonic();   // a Duration since boot
do_work();
let took = std::time::monotonic() - start;
println("took ", took);
```

`std::time::now()` gives wall-clock seconds since the Unix
epoch when you genuinely need calendar time; `monotonic()` is
the basis for anything timing-related.

## Identities and ranges

Some integers must not mix. An order id is not a sequence number, and
neither is a count you can add to. Some integers have a range: a byte
is `0..256`, a session slot `0..64`. Say so in the type:

```hale
type OrderId = distinct Int;
type Byte    = Int { range: 0..256; }
type Session = distinct Int { range: 0..64; }

fn main() {
    let id: OrderId = 4;             // a literal is checked against the type
    let b: Byte = 200;
    let total: Int = b + 55;         // a byte is an Int, for free
    let slot = Session(70) or 0;     // 70 is outside 0..64: the substitute
    let top = Byte(total + 10) or clamp;
    let raw: Int = Int(id);
    println(id, " ", total, " ", slot, " ", top, " ", raw);
}
```

A `distinct Int` is an **identity**: it compares with itself and with
nothing else, it has no arithmetic (`id + 1` is an error), and it
never passes for an `Int`. You cross explicitly, `OrderId(n)` in and
`Int(id)` out. A type with a `range:` is a **range**: it is an `Int`
wherever an `Int` is expected, and its arithmetic is an `Int`'s.

Going *into* a range can fail, so a narrowing such as `Session(n)`
says what becomes of a value outside it, with `or`:

- `or 0`, any value of the type, held to its range;
- `or clamp`, the nearest bound;
- `or wrap`, around the range (`-1` becomes the top);
- `or handler(err)`, given a `RangeError` with the value and the
  bounds;
- `or raise`, to hand the `RangeError` to the caller.

A literal outside the range is an error where you wrote it (`let b:
Byte = 300;`), and a narrowing with no `or` is too. `spec/units.md §
Identities and ranges` has every rule.

## Your own units

`Duration` is not the only number with a unit, and its units are
declared the way a program declares its own (above). Money is counted
in cents, sizes in bytes, rates in basis points, and a program
declares those units and the integer types counted in them:

```hale
unit cent;
unit USD = 100 cent;
unit pct = 1/100;
unit bp = 1/100 pct;

type Money  = quantity Int in cent;
type Ledger = Money { round: half_even; }
type Rate   = quantity Int in bp;

fn main() {
    let fee = 1_250_000USD * 3bp;     // exactly 375 USD
    let odd = 1_234_567USD * 3bp;     // 370.3701 USD: still exact here
    let paid: Ledger = odd;           // rounded where the policy is
    let cents = odd.in(cent) or ceil; // or name the rounding yourself
    println(fee, " ", odd, " ", paid, " ", cents);
}
```

A `unit` is a name and, optionally, what it equals: one `USD` is
100 `cent`, and `bp` is a hundredth of a percent, itself a hundredth
of the number one. A `quantity` counts an `Int` in one of them. A
literal names its unit with no space, `3bp`, `1_250_000USD`, and
counts in its quantity's unit: `1_250_000USD` is 125,000,000 cents.

Arithmetic keeps every value **exact**. `fee` is not rounded to
cents: multiplying money by a rate gives money counted in the product
of the two units, here a ten-thousandth of a cent, and it prints that
way (`375000000 Money in 1/10000 cent`). Rounding happens only where
you say so, and you have to say so wherever a value goes somewhere
coarser:

- a type with `{ round: … }` (here `Ledger`, `half_even`) rounds what
  is stored into it, with no more ceremony;
- `.in(cent) or floor` (or `ceil`, `trunc`, `half_up`, `half_even`)
  rounds at the site;
- `.in(cent) or 0` keeps the value only when it is a whole number of
  cents.

`let bad: Money = odd;` is an error: `Money` has no policy, and the
message says the division by 10,000 has a remainder to decide about.

Time works the same way, because `Duration` and `Time` are these
declarations too (the standard library's, above): a quantity counted in
nanoseconds and a point over it.

```hale
type Bucket = Duration in 100ms { round: floor; }

fn main() {
    let d = 3s + 500ms;               // 3500000000ns: a Duration counts nanoseconds
    let whole = d.in(s) or floor;     // 3s: the policy says what becomes of the rest
    let (secs, rest) = d.split(s);    // 3 and 500000000ns, nothing lost
    let n: Int = d / 1ms;             // 3500: two durations divide to a count
    let b: Bucket = d;                // 35 buckets of 100ms
    let start = Time(1_000s);
    let later = start + d;            // a point moves by a quantity
    let took = later - start;         // and two points differ by one
    println(d, " ", whole, " ", secs, " ", rest, " ", n, " ", b, " ", took);
}
```

A **point** (`Instant`) is a position, a **quantity** (`Span`) a
distance: two points subtract to a quantity, a point plus a quantity
is a point, and two points do not add. A point may declare where its
zero is (`type Celsius = point TempDelta { origin: 273_150 mK; }`),
and `Kelvin(c)` converts across the two zeros.

Mixing kinds is an error at the operator, naming both declarations:
`5msec + 4KiB` adds time to bytes, `bid + ask` adds two prices,
`d > 0` compares a span with a bare number (write `d > 0msec`). An
`Int` becomes a quantity by a unit (`n * 1cent`), and a quantity's
count is a quotient (`q / 1cent`). `spec/units.md § Quantities and
points` has every rule. The standard library declares the time units
(`ns` … `day`), so a program's own unit takes another name, or joins
them by what it equals (`unit tick = 10 ms;`).

## Why these are in the language

`Decimal`, `Duration`, and `Time` aren't library types you opt
into — `Decimal` is a primitive with its own literal, and `Duration`
and `Time` are declared by the standard library for every program,
with the time units' literals. The reason is
that the bugs they prevent (float drift in money, unit confusion
in time) are *so common* and *so costly* that making them
first-class is worth it. You get the safety without importing
anything or remembering a convention.

Next: [Functions](./functions.md).
