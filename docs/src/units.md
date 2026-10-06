# Units, end to end

[Math, money & time](./basics/math.md) introduced the pieces: a
`unit`, a `quantity` counted in one, a `point` over a quantity, an
identity, a range. This chapter puts them together in one program, the
example the dialect was designed around, then shows the three mistakes
the compiler catches most often, word for word, and the report that
tells you what every number in a program counts.

The rule underneath all of it fits on one line: **widening is free,
and narrowing is named.** A conversion that multiplies by a whole
number is exact, so Hale does it for you. A conversion that divides,
or that may not fit, loses something, so the program says what: at
the place it happens (`or floor`), or once, on the type it goes into
(`{ round: half_even; }`). Never by a default.

## The catalogue

A program declares the units it counts in, and what each equals:

```hale,fragment
unit B;
unit KiB = 1024 B;

unit cent;
unit USD = 100 cent;
unit bp = 1 / 10000;     // a basis point, against the number one
unit pct = 100 bp;

unit tick;               // a unit with no edges is its own component
unit mK;
```

The units connected by equations form a **component**: `cent` and
`USD` are one, `B` and `KiB` another, `bp` and `pct` a third, the
dimensionless one, because `bp` is defined against the number. Time is
a component you already have: the standard library declares `ns`,
`us`, `ms`, `s`, `min`, `h` and `day` the same way, and the types
`Duration` (a quantity in `ns`) and `Time` (a point over it).

Each component has one **quantity**, the integer type counted in it:

```hale,fragment
type ByteCount = quantity Int in B;
type Money     = quantity Int in cent;
type Ratio     = quantity Int in bp;
type Tick      = quantity Int in tick;
type TempDelta = quantity Int in mK;
```

No quantity, and no unit, takes a builtin type's name: `Bytes` is the
buffer type, so the byte count is `ByteCount`.

A literal names its unit with no space, and counts in its quantity's
denomination: `1_250_000USD` is 125,000,000 cents, `4KiB` is 4096 `B`,
`500ms` is 500,000,000 nanoseconds.

## Time

```hale
type Seconds = Duration in s;

fn main() {
    let d = 3s + 500ms;               // a Duration: 3500000000ns
    let whole = d.in(s) or 0;         // 3.5 s is no whole number of seconds: 0
    let secs = d.in(s) or floor;      // 3s: the policy says what becomes of the rest
    let (sec, rem) = d.split(s);      // 3, and 500000000ns: nothing lost
    let n: Int = d / 1ms;             // 3500: a quantity over a quantity is a count
    let exact: Seconds = 2_000ms;     // 2s, exact, so no policy is needed
    println(d, " ", whole, " ", secs, " ", sec, " ", rem, " ", n, " ", exact);
}
```

`d.in(s)` divides by 1,000,000,000 (a `Duration` counts nanoseconds),
so it has a remainder to decide about: `or 0` takes the value only when
it is a whole number of seconds, `or floor` rounds down. `d.split(s)`
never loses anything: the whole seconds, and the rest. `Seconds` is a
name for `Duration` counted in seconds; storing `2_000ms` into it is a
whole number of seconds, so it is free.

## Boundary types

A type can carry its own rounding, for the place a value crosses into
a coarser denomination, such as a wire format in microseconds or a
histogram in buckets:

```hale
type WireStamp = Time in us { round: floor; }
type Bucket = quantity Int in 100ms { round: floor; }

fn main() {
    let a = Time(1_700_000_000_123_456_789 * 1ns);
    let out: WireStamp = a;           // ns -> us narrows; the type says how
    let d = 3s + 500ms;
    let bucket: Bucket = d;           // 35 Bucket
    let early: Bucket = 150ms;        // 1 Bucket, rounded down at compile time
    println(out, " ", bucket, " ", early);
}
```

No `or` is needed on these lines: the type's `round:` is the policy.

## Money and ratios

Multiplying money by a rate is where unit systems usually round too
early. Hale doesn't round at the product at all: `Money` times a
`Ratio` is `Money` counted in the product of the two units, here a
ten-thousandth of a cent, so it is exact. The rounding happens where
the value is stored as cents, once, by the policy that place declares.

```hale
unit cent;
unit USD = 100 cent;
unit bp = 1 / 10000;

type Money = quantity Int in cent;
type Ratio = quantity Int in bp;
type Ledger = quantity Int in cent { round: half_even; }

fn main() {
    let fee = 1_250_000USD * 3bp;     // 375 USD, exact
    let odd = 1_234_567USD * 3bp;     // 370.3701 USD, still exact here
    let paid: Ledger = odd;           // 37037cent: half_even, from Ledger
    let cents = odd.in(cent) or ceil; // 37038cent: or name the rounding at the site
    println(fee, " | ", odd, " | ", paid, " | ", cents);
}
```

`fee` prints as `375000000 Money in 1/10000 cent`: a type no
declaration names, made up of the quantity and its denomination, and
an ordinary type for a binding, an argument or a return. Each ratio
factor makes the denomination finer and the range an `Int` can hold
smaller, which the report below shows.

## Ticks and prices

A **point** is a position on a quantity's line. Two prices differ by a
number of ticks, a price moves by ticks, and two prices never add:

```hale
unit tick;
type Tick = quantity Int in tick;
type Price = point Tick;

fn main() {
    let bid = Price(100tick);
    let ask = Price(103tick);
    let spread = ask - bid;                  // 3tick
    let mid = bid + (spread / 2 or floor);   // half a tick does not exist: 101
    println(spread, " ", mid);
}
```

Dividing by a number you wrote down is a question with a remainder,
so `spread / 2` says what to do with it; a divisor that arrives at run
time divides as integers do.

## Identities and widths

Some integers must never mix. An identity has no arithmetic and
compares only with itself; a range is an `Int` with bounds, and a
width is a range:

```hale
type OrderId = distinct Int;
type SeqNo = distinct Int { range: 0..65536; }
type Session = distinct Int { range: 0..64; }
type Byte = Int { range: 0..256; }

fn main() {
    let id = OrderId(7);
    let sess = Session(70) or 0;          // outside 0..64: the substitute
    let seq = SeqNo(70_000) or wrap;      // around the range: 4464
    let b: Byte = 200;
    let top = Byte(b + 100) or clamp;     // the nearest bound: 255
    println(Int(id), " ", sess, " ", seq, " ", top);
}
```

Going into a range can fail, so `Session(n)` says what becomes of a
value outside it: a value of the type, `clamp`, `wrap`, a handler given
the `RangeError`, or `raise`.

## Two origins

Celsius and Kelvin count the same quantity from different zeros. Two
point types over one quantity, one with an `origin:`, are all it
takes; no affine edges:

```hale
unit mK;
type TempDelta = quantity Int in mK;
type Kelvin = point TempDelta;
type Celsius = point TempDelta { origin: 273_150 mK; }

fn main() {
    let boil = Celsius(100_000mK);
    let k = Kelvin(boil);                    // 373150: the shift is the origin
    let rise = boil - Celsius(0mK);          // 100000mK, a TempDelta
    println(k, " ", rise);
}
```

## Three common mistakes

**Mixing quantities.** Adding time to bytes is refused at the
operator, naming both:

```hale,fragment
let bad = 5ms + 4KiB;
```

```text
type error: `Duration` + `ByteCount`: different quantities, `Duration` and `ByteCount`; `+` holds within one
```

The same holds for two prices (`bid + ask`: "two points do not add;
their difference is a quantity (\`b - a\`), and a point moves by a
quantity (\`a + d\`)") and for an identity (`id + id`: "\`OrderId\` is an
identity; it has no arithmetic").

**A narrowing with no policy.** `Money` has no `round:`, so storing a
value counted in ten-thousandths of a cent into it would have to drop
something, and nothing says what:

```hale,fragment
let bad: Money = odd;
```

```text
type error: `Money` from `Money in 1/10000 cent` divides by 10,000: say what happens to the remainder: convert explicitly (`.in(u) or floor`, `Money(…) or half_even`, `or <value>`, `or raise`), or give `Money` a `round:` policy
```

Every form a narrowing takes is held to this: `.in(u)`, a cast, a
division by a literal, a binding, an argument, a return, a field, a
compound assignment, a literal that is no whole count, and a range's
narrowing (`Session(n)` alone: "…this conversion says nothing about a
value outside it: write \`or <fallback>\` …").

**A bare number where a quantity goes.** A count is not a duration
until you say in what:

```hale,fragment
if d > 0 { /* ... */ }
let wait: Duration = n;
```

```text
type error: `Duration` > `Int`: a quantity or a point compares with its own kind, never an `Int`; compare with a quantity (`0ns`)
type error: `Int` is not `Duration`: a count becomes a quantity by a unit (`n * 1ns`)
```

Write `d > 0ns` and `n * 1ms`; the other way, a quantity's count is a
quotient, `d / 1ms`.

## The report

`hale check --units` prints what every number in the program counts:
each declaration's denomination and where it was fixed, its policy,
the headroom an `Int` has at that denomination, and what a declared
range fits in; then each narrowing, with its factor, the policy that
discharged it, and where that policy came from. Two entries from the
program the chapter's sections are cut from:

```text
units: 15 declarations, 7 narrowings

committed_form.hl:46:1  type SeqNo = distinct Int { range: 0..65536; }
    kind         : identity
    range        : 0..65536
    fits in      : u16

committed_form.hl:81:24  odd
    Money in 1/10000 cent -> Ledger, factor 1/10000
    policy       : half_even, the `round:` of `type Ledger = quantity Int in cent { round: half_even; }` (committed_form.hl:53:1)
    headroom     : ±922337203685477 cent (9223372036854 USD), of Money in 1/10000 cent
```

The second entry is the money example's one rounding: a factor of
1/10,000, `half_even`, from `Ledger`; and the product it rounds holds
at most about 9.2 trillion USD, where `Money` holds ten thousand times
that. The text is stable, so a project can record it and review its
diff like any other artifact; `--json` gives the same fields as one
object. A program with no unit declarations prints one line, `units:
no quantity is declared`.

## What it does not do

Not yet: a rate type such as bytes per second (one quantity over
another of a different kind is refused; over the same kind it is a
plain count), `Float` quantities, calendars, and packing a narrow range
into a narrow field (the report says what a range would fit in; every
value is still an `Int`). `spec/units.md` is the whole contract.

Next: [Functions](./basics/functions.md).
