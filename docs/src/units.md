# Units, End to End

Hale includes a compile-time unit, quantity, affine-point, and integer-range type system designed around a single invariant: **widening is automatic and exact; narrowing requires an explicit rounding or failure policy.** Multiplication by ratios preserves exact fractional denominations until you explicitly round into a target unit or boundary type.

```hale
unit cent;
unit USD = 100 cent;
unit bp  = 1 / 10000;

type Money  = quantity Int in cent;
type Ratio  = quantity Int in bp;
type Ledger = quantity Int in cent { round: half_even; }

fn main() {
    let odd  = 1_234_567USD * 3bp;      // exact: Money in 1/10000 cent (370.3701 USD)
    let paid: Ledger = odd;             // 37037cent (rounded via Ledger's half_even policy)
    let up   = odd.in(cent) or ceil;    // 37038cent (rounded at the call site)
    println(odd, " | ", paid, " | ", up);
}
```

## Syntax and Construct Summary

| Construct | Declaration / Expression Syntax | Description |
|---|---|---|
| **Base unit** | `unit cent;` / `unit B;` / `unit tick;` | Starts a new unit component (dimension) |
| **Derived unit** | `unit USD = 100 cent;` / `unit KiB = 1024 B;` | Defines a unit as an exact multiple of another unit in the same component |
| **Dimensionless unit** | `unit bp = 1 / 10000;` / `unit pct = 100 bp;` | Defines a dimensionless ratio relative to `1` |
| **Quantity type** | `type Money = quantity Int in cent;` | 64-bit integer counted in a specific base denomination |
| **Redenominated alias** | `type Seconds = Duration in s;` | Names an existing quantity at a coarser or finer unit |
| **Boundary type** | `type Ledger = Money { round: half_even; }` | Automatically rounds narrower/finer inputs using `floor`, `ceil`, `trunc`, `half_up`, or `half_even` |
| **Affine point** | `type Price = point Tick;` | Position on a quantity axis (`Point - Point -> Quantity`, `Point + Quantity -> Point`) |
| **Shifted origin point** | `type Celsius = point TempDelta { origin: 273_150 mK; }` | Affine point with a non-zero origin offset |
| **Distinct identity** | `type OrderId = distinct Int;` | Strongly typed ID with no arithmetic; converts via `OrderId(n)` and `Int(id)` |
| **Bounded range** | `type Byte = Int { range: 0..256; }` | Integer constrained to `[0, 256)` (or `0..=255`); narrows via `or clamp`, `or wrap`, `or <val>`, `or raise` |
| **Explicit conversion** | `d.in(s) or floor` | Converts quantity `d` to unit `s` using the specified remainder policy |
| **Lossless split** | `let (sec, rem) = d.split(s);` | Splits `d` into whole units `(Int)` and the remainder `(Quantity)` without losing precision |

## Declaring Units and Quantities

Units connected by `=` equations belong to the same **component** (dimension):

```hale,fragment
unit B;
unit KiB = 1024 B;

unit cent;
unit USD = 100 cent;
unit bp = 1 / 10000;     // basis point (dimensionless ratio)
unit pct = 100 bp;

unit tick;               // standalone component
unit mK;
```

Each component has a primary `quantity` type counted in its base unit:

```hale,fragment
type ByteCount = quantity Int in B;
type Money     = quantity Int in cent;
type Ratio     = quantity Int in bp;
type Tick      = quantity Int in tick;
type TempDelta = quantity Int in mK;
```

- **No shadowing builtin type names:** Neither a `unit` nor a `type` may use a built-in type name (for example, `Bytes` is the binary buffer primitive, so a byte quantity is named `ByteCount`).
- **Literal token rules:** In expressions, a number and its unit suffix must be written as a single token with no space (`1_250_000USD`, `4KiB`, `500ms`). In declarations (`unit` equations, `in <unit>`, and `origin:` clauses), whitespace between the multiplier and unit is optional (`1024 B` or `1024B`).
- **Standard library time units:** The standard library pre-declares `ns`, `us`, `ms`, `s`, `min`, `h`, and `day`, along with `Duration` (`quantity Int in ns`) and `Time` (`point Duration`). User programs can extend the time component by defining derived units (`unit frame_tick = 16 ms;`).

## Conversions, `.split()`, and Boundary Types

### Site-Level Conversions (`.in(u)` and `.split(u)`)

Converting a quantity to a finer unit (such as `2_000ms` into `Seconds`) is exact when the value is a compile-time multiple or when widening (`Seconds` to `Duration`). Converting to a coarser unit divides by the scale factor and requires an `or` clause to handle any remainder:

```hale
type Seconds = Duration in s;

fn main() {
    let d = 3s + 500ms;               // 3500000000ns
    let whole = d.in(s) or 0;         // 0s (3.5s is not a whole second, so fallback 0 applies)
    let secs  = d.in(s) or floor;     // 3s (rounded down)
    let (sec, rem) = d.split(s);      // sec = 3 (Int), rem = 500000000ns (Duration)
    let n: Int = d / 1ms;             // 3500 (Quantity / Quantity -> Int count)
    let exact: Seconds = 2_000ms;     // 2s (exact compile-time multiple, no `or` needed)
    println(d, " ", whole, " ", secs, " ", sec, " ", rem, " ", n, " ", exact);
}
```

### Boundary Types with `{ round: ... }`

When values cross into a coarser representation—such as a wire timestamp in microseconds, a histogram bucket, or a cent-denominated ledger—attach `{ round: <policy>; }` to the type declaration:

```hale
type WireStamp = Time in us { round: floor; }
type Bucket    = quantity Int in 100ms { round: floor; }

fn main() {
    let a = Time(1_700_000_000_123_456_789 * 1ns);
    let out: WireStamp = a;           // narrows ns -> us automatically via `floor`
    let d = 3s + 500ms;
    let bucket: Bucket = d;           // 35 Bucket
    let early: Bucket  = 150ms;       // 1 Bucket (rounded at compile time)
    println(out, " ", bucket, " ", early);
}
```

Assigning a finer-grained value to a boundary type applies the type's `round:` policy automatically without requiring an `or` clause at each assignment.

## Exact Ratio Arithmetic (Money and Basis Points)

Multiplying a quantity by a dimensionless ratio (`unit bp = 1 / 10000;`) does not round at the multiplication step. Instead, the product has type `Money in 1/10000 cent`—an exact compound denomination:

```hale
unit cent;
unit USD = 100 cent;
unit bp  = 1 / 10000;

type Money  = quantity Int in cent;
type Ratio  = quantity Int in bp;
type Ledger = quantity Int in cent { round: half_even; }

fn main() {
    let fee = 1_250_000USD * 3bp;     // 375000000 Money in 1/10000 cent (exact 375 USD)
    let odd = 1_234_567USD * 3bp;     // 370370100 Money in 1/10000 cent (exact 370.3701 USD)
    let paid: Ledger = odd;           // 37037cent (half_even from Ledger)
    let cents = odd.in(cent) or ceil; // 37038cent (explicit site rounding)
    println(fee, " | ", odd, " | ", paid, " | ", cents);
}
```

Each ratio multiplication refines the denomination and reduces the maximum magnitude that fits in a signed 64-bit `Int` (which `hale check --units` calculates and reports).

## Affine Points and Shifted Origins (`point` and `origin:`)

A `point` represents a location on a quantity axis (such as a timestamp on a timeline, a price on a tick ladder, or a temperature on a thermodynamic scale):

- **Difference of two points (`p2 - p1`):** Produces the underlying `quantity`.
- **Translating a point (`p + q`, `p - q`):** Shifts the point by a `quantity`.
- **Adding two points (`p1 + p2`):** Compile error.

```hale
unit tick;
type Tick  = quantity Int in tick;
type Price = point Tick;

fn main() {
    let bid = Price(100tick);
    let ask = Price(103tick);
    let spread = ask - bid;                  // 3tick (Tick)
    let mid = bid + (spread / 2 or floor);   // 101 (Price)
    println(spread, " ", mid);
}
```

### Multiple Origins (Affine Scales)

When two scales measure the same quantity from different zero points (such as Kelvin and Celsius), declare both as `point` types over the same quantity and specify `{ origin: ...; }` on the shifted scale:

```hale
unit mK;
type TempDelta = quantity Int in mK;
type Kelvin    = point TempDelta;
type Celsius   = point TempDelta { origin: 273_150 mK; }

fn main() {
    let boil = Celsius(100_000mK);
    let k    = Kelvin(boil);                 // 373150 (adds 273_150 mK origin shift)
    let rise = boil - Celsius(0mK);          // 100000mK (TempDelta)
    println(k, " ", rise);
}
```

## Distinct Identities and Bounded Ranges

Use `distinct Int` for identifiers that must not participate in arithmetic, and `{ range: low..high; }` (half-open) or `{ range: low..=high; }` (inclusive) for bounded integer widths:

```hale
type OrderId = distinct Int;
type SeqNo   = distinct Int { range: 0..65536; }
type Session = distinct Int { range: 0..64; }
type Byte    = Int { range: 0..256; }

fn main() {
    let id   = OrderId(7);
    let sess = Session(70) or 0;          // out of 0..64 -> substitutes 0
    let seq  = SeqNo(70_000) or wrap;     // 70000 mod 65536 -> 4464
    let b: Byte = 200;
    let top  = Byte(b + 100) or clamp;    // clamped to upper bound -> 255
    println(Int(id), " ", sess, " ", seq, " ", top);
}
```

| Range Narrowing Policy | Behavior |
|---|---|
| `or <fallback>` | Substitutes `<fallback>` (must itself be within the range) |
| `or clamp` | Clamps to the nearest valid bound (`low` or `high - 1`) |
| `or wrap` | Wraps modulo the range width (`low + (v - low) mod width`, always non-negative; `SeqNo(-1) or wrap == 65535`) |
| `or handler(err)` | Passes `RangeError` (containing the value and bounds) to `handler` |
| `or raise` | Propagates `RangeError` to the caller |

## Common Compile-Time Diagnostics

| Mistake | Invalid Example | Why It Fails & How to Fix |
|---|---|---|
| **Mixing incompatible quantities** | `let x = 5ms + 4KiB;` | `Duration` and `ByteCount` belong to different unit components. |
| **Adding two points or identities** | `let p = bid + ask;` / `id + id` | Points only subtract (`ask - bid`) or shift by a quantity (`bid + 1tick`); `distinct Int` has no arithmetic. |
| **Narrowing without a policy** | `let bad: Money = odd;` | Converting `1/10000 cent` to `cent` divides by `10,000`; add `.in(cent) or floor` or use a `{ round: ... }` boundary type. |
| **Bare `Int` vs. quantity** | `if d > 0 { ... }` / `let w: Duration = n;` | Quantities only compare with their own kind (`d > 0ns`). Convert `Int` to a quantity by multiplying (`n * 1ms`) and extract a count by dividing (`d / 1ms` or `x / 1cent`). |

## Auditing Units with `hale check --units`

Run `hale check --units` (or `hale check --units --json` for machine-readable output) to inspect every unit, quantity, range, and narrowing site in a seed:

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

The report lists:
- Every declared quantity, point, identity, and range, along with the smallest hardware integer width (`u8`, `u16`, etc.) a range fits in.
- Every narrowing conversion across the program, its scale factor, the rounding policy that discharged it, the source location of that policy, and the `i64` overflow headroom at the finer denomination (for example, `Money in 1/10000 cent` has a headroom of `±9.22 trillion USD`).
- Paths relative to the checked seed directory so the output can be committed and diffed deterministically in CI. If a program declares no units, `hale check --units` prints `units: no quantity is declared`.

> [!NOTE]
> **Current Scope (`spec/units.md`):** Cross-dimension rate types (such as bytes per second, `ByteCount / Duration`), `Float` quantities, calendar arithmetic, and automatic bit-packing of narrow ranges into sub-64-bit struct fields are not yet implemented. Dividing two quantities of the *same* component (`d / 1ms`) always returns a plain `Int` count.

Next: [Functions](./basics/functions.md).
