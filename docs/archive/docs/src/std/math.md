# `std::math`

Six libm-backed floating-point primitives. Phase 2c
(2026-05-11) shipped the first batch — `sqrt`, `exp`, `log`,
`floor`, `ceil` (unary) and `pow` (binary) — alongside
implicit `Int → Float` widening at let-binding and
function-argument sites. Together these unlock float-heavy
math without the workaround pattern of carrying parallel
`nf: Float` fields next to every `n: Int` counter.

Each function is a path-call dispatch that lowers directly to
the corresponding libm symbol; there is no Aperio-side wrapper
adding overhead. Arguments coerce `Int → Float` at the call
site so `std::math::sqrt(2)` works without an explicit
`2.0` literal.

## Numeric conversion at the call site

Phase 2c also shipped an `Int → Float` widening rule (via
LLVM `sitofp`) that fires at two surfaces:

1. **let-binding type ascription:**
   `let nf: Float = self.n;` where `self.n: Int` works
   directly — no `to_string` round-trip, no manual cast.
2. **function-argument coercion:**
   `std::math::sqrt(n)` with `n: Int` widens `n` to a Float
   before the libm call. Same rule applies to user-declared
   fns taking `Float` parameters.

The widening is **one-way only**. `Float → Int` narrowing
stays explicit (round / floor / ceil + an explicit `to_int`,
or `std::math::floor` followed by an int-cast in a future
milestone). `Decimal` and other lossy mixes still reject.

## Functions

### `std::math::sqrt`

#### Synopsis

```aperio
fn sqrt(x: Float) -> Float
```

Square root via `sqrt(3)` from libm. Domain `x >= 0`; behavior
on negative input follows the platform libm (typically NaN).

#### Examples

```aperio
fn main() {
    let r = std::math::sqrt(2.0);
    println("sqrt(2) ≈ ", r);
}
```

Pearson correlation `r` (not just `r²`) — the canonical case
that was blocked pre-2c by the missing `sqrt`:

```aperio
fn pearson_r(r_squared: Float) -> Float {
    return std::math::sqrt(r_squared);
}
```

### `std::math::exp`

#### Synopsis

```aperio
fn exp(x: Float) -> Float
```

`e^x` via `exp(3)` from libm. The decay primitive — time-weighted
moving averages, softmax components, exponential smoothing.

#### Examples

Time-weighted EMA decay (one-sample update form):

```aperio
fn decay(prev: Float, sample: Float, dt: Float, tau: Float) -> Float {
    let alpha = std::math::exp(-dt / tau);
    return alpha * prev + (1.0 - alpha) * sample;
}
```

### `std::math::log`

#### Synopsis

```aperio
fn log(x: Float) -> Float
```

Natural logarithm (base e) via `log(3)` from libm. Domain
`x > 0`; behavior on `x <= 0` follows the platform libm.

For base-10 / base-2 logarithms, compose with a constant:
`log10(x) = log(x) / log(10.0)`.

#### Examples

```aperio
fn main() {
    let lg2 = std::math::log(2.0) / std::math::log(10.0);
    println("log10(2) ≈ ", lg2);
}
```

### `std::math::pow`

#### Synopsis

```aperio
fn pow(base: Float, exp: Float) -> Float
```

`base^exp` via `pow(3)` from libm. Binary, both arguments
Float (Int widens automatically).

#### Examples

```aperio
fn main() {
    let kib = std::math::pow(2.0, 10.0);
    println("2^10 = ", kib);
}
```

### `std::math::floor`

#### Synopsis

```aperio
fn floor(x: Float) -> Float
```

Largest integer-valued Float `<= x`. Note the return type is
`Float` (matches libm `floor(3)`); convert to `Int` with an
explicit cast when an Int is required downstream.

### `std::math::ceil`

#### Synopsis

```aperio
fn ceil(x: Float) -> Float
```

Smallest integer-valued Float `>= x`. Same shape as `floor`.

#### Examples — floor / ceil together

```aperio
fn main() {
    let x = 3.7;
    println("floor=", std::math::floor(x),
            " ceil=",  std::math::ceil(x));
}
```

## Worked example — windowed variance

Pulls everything together. Phase 2c made this shape direct
where previously it required parallel Int/Float counters and
explicit conversion plumbing.

```aperio
fn variance(samples: [Float; 8], count: Int) -> Float {
    // Int → Float widening at the let-binding: count is Int,
    // n is Float.
    let n: Float = count;
    if count == 0 {
        return 0.0;
    }
    let mut sum: Float = 0.0;
    let mut i = 0;
    while i < count {
        sum = sum + samples[i];
        i = i + 1;
    }
    let mean = sum / n;
    let mut sq: Float = 0.0;
    i = 0;
    while i < count {
        let d = samples[i] - mean;
        sq = sq + d * d;
        i = i + 1;
    }
    return sq / n;
}

fn stddev(samples: [Float; 8], count: Int) -> Float {
    return std::math::sqrt(variance(samples, count));
}
```

## Limitations

- **Float only.** No `Decimal` variants. The financial /
  fixed-precision substrate stays in `Decimal` operators;
  libm doesn't extend to it.
- **No `sin` / `cos` / `tan`.** Trigonometric primitives wait
  until a workload forces them.
- **No `min` / `max` / `abs` / `round`.** These would naturally
  fit but haven't shipped — use control-flow forms (`if a < b
  { a } else { b }`) for now. Phase 2b's if-as-expression
  makes that one-liner.
- **No explicit `Float → Int` narrowing primitive.** `floor`
  and `ceil` return Float; an `to_int(f)` builtin (or a path
  call) lands when needed.

## See Also

- [`std::str`](./str.md) — `parse_int` / `to_string`; the
  String → Int half of numeric conversion. (Float parsing is
  not yet shipped.)
- [What you can build today](./ready-today.md) — capability
  matrix including the Phase 2c shipped surface.
- [Roadmap](./roadmap.md) — trig, hyperbolic, and additional
  math primitives are sketched in the Phase 1+ stdlib
  build-out plan.
