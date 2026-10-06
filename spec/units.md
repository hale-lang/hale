# Exact unit catalogue

The compiler's `hale_types::unit_graph` API closes a catalogue of
resolved unit identities and positive rational equations. It is the
declaration-layer arithmetic core for GH #1076 / #1212. A program's
`unit` declarations are closed into one through it, and its scalar
declarations are judged against it (§ Declarations). Expression typing
types the values of identities and ranges (§ Identities and ranges) and
of quantities and points, reading the catalogue's factors (§
Quantities and points). `Time` and `Duration` are two of those
declarations, the stdlib's, over its time catalogue (§ The stdlib's
time catalogue).

## Declarations

A program declares units and the scalar types counted in them. The
words are contextual (spec/tokens.md § Unit dialect words); the
productions are `unit_decl` and `type_decl`'s `scalar_body` in
spec/grammar.ebnf.

**A unit.** `unit tick;` declares a unit with no equation, its own
component. `unit us = 1_000 ns;` declares `us` and states that one
`us` is 1000 `ns`. A factor is a positive integer or a ratio of two
(`unit inch = 127/5 mm;`), and a zero in either place is a parse
error. With no target the unit is that multiple of the pure number:
`unit pct = 1/100;`. The magnitude and the target may be written
apart or together (`1_000 ns`, `1_000ns`, `1024KiB`); the two
spellings declare the same equation. A unit is seed-global: its
name is never mangled across an import.

**A scalar type.** A `type` declaration whose body is

```text
[quantity | point | distinct] BASE [in DENOMINATION] [{ CLAUSE; ... }]
```

with at least one of the kind word, the `in` and the clause block
(with none of the three it is an alias):

```hale
type Duration  = quantity Int in ns;
type Time      = point Duration;
type Celsius   = point TempDelta { origin: 273_150 mK; }
type Session   = distinct Int { range: 0..64; }
type Byte      = Int { range: 0..256; }
type WireStamp = Time in us { round: floor; }
type Bucket    = quantity Int in 100ms { round: floor; }
```

A denomination is a unit and a positive integer multiple of it
(`ns`, `100ms`, `100 ms`). The clauses are `range: LO..HI` (or
`LO..=HI`), `round: POLICY` and `origin: N UNIT`; each appears at
most once, and any other name is a parse error listing the three. A
clause block closes the declaration, so a `;` after it is optional.
A scalar type takes no generic parameters.

**A quantity literal** is an integer written against a unit name,
`3bp`, `1_250_000USD` or `500ms` (spec/tokens.md § Quantity
literals).

### The rows

The declarations are rows of one family, `unit_declarations`
(`hale_types::units::derive_unit_rows`, spec/registry.md), derived
once per snapshot from the stdlib's seed and then the programs after
the desugar sequence, each row keyed by its declaration's site in the
universe that minted it (§ Identities and equations): the stdlib's
time catalogue and its `Duration` and `Time` come first, then the
program's own.

- **A unit row** per `unit` declaration, and **an equation row** per
  equation: one `unit` is `p/q` of its target, which is a unit or the
  number one.
- **A reference row** per place a declaration names a unit (an
  equation's target, a denomination, an origin), resolved by name to
  the first unit of that name, or to none.
- **Components.** The units of every equation whose target resolves
  are joined; a unit with no equation is its own component. The
  number one is a node like a unit, so every unit written against a
  number (`unit pct = 1/100;`, `unit dozen = 12;`) is in one
  component, the dimensionless one.
- **The catalogue** is closed once from the unit and equation rows
  (§ Closure and conversion). It is kept only when no catalogue law
  fails: a unit declared twice, an equation naming no declared unit,
  a cycle that does not multiply to one.
- **A scalar row** per scalar `type` declaration:
  - its **kind**: its word's (`quantity`, `point`, `distinct` is an
    identity); with no word, its base's (a refinement of a quantity
    is a quantity, of a point a point, of `Int`, an identity or a
    range a range; `Int in D` is a quantity missing its word, which
    law 5 refuses);
  - its **parent**, the scalar it is written over;
  - a quantity's or a point's **denomination**, a value (`Denom`: a
    unit and an exact positive multiple), its own `in` or else its
    parent's, and its **component**, its parent's or else its
    denomination's unit's;
  - whether it is its component's **quantity** (law 4), and its
    **`of`**: a point's quantity, a range's parent, and for every
    other quantity of the component, that one quantity;
  - a point's **origin** as an exact count of its own denomination
    (`origin: 273 K` on a point in `mK` is 273000), through the
    catalogue;
  - its **policy** (`floor`, `ceil`, `trunc`, `half_even`,
    `half_up`) and its **range**, half-open (`..=` stores its upper
    bound plus one).

```hale
unit cent;
unit USD = 100 cent;
type Money = quantity Int in cent;
type Ledger = quantity Int in cent { round: half_even; }
```

`Money` is the quantity of the component `{cent, USD}`; `Ledger` is a
boundary denomination of it: a quantity row whose `of` is `Money`,
with the policy `half_even`.

### The laws

Each law is a registered structural rule (spec/verification.md §
Structural & design rules), judged over the rows, and reported as a
located error whose related notes are its witness.

1. **A unit is declared once.** At the second declaration's name; the
   witness is the first. A program's unit of a name the stdlib's time
   catalogue declares is that unit declared again: the message names
   the catalogue, whose declaration is in no file of the program.

   ```hale,fragment
   unit cent;
   unit cent;      // error: unit `cent` is declared twice
   unit ms;        // error: … the stdlib's time catalogue declares `ms` (`std::time`) …
   unit tick = 10 ms;   // a unit of the program's, in the time component
   ```

2. **A named unit is declared.** An equation's target, a
   denomination and an origin name a declared unit; the error is at
   the name, suggesting the nearest declared one (a program's own
   before the stdlib's).

   ```hale,fragment
   unit cent;
   unit USD = 100 cnet;   // error: … names `cnet`, which no `unit` declares; did you mean `cent`?
   ```

3. **Every cycle multiplies to one.** At the equation the closure
   found inconsistent, stating what it claims and what the other
   equations imply; the witness is the other path, each step at its
   declaration.

   ```hale,fragment
   unit a = 1/31 c;
   unit b = 6 a;
   unit c = 5 b;   // error: makes one `c` 5 `b`, and the other equations make it 31/6 `b`
   ```

4. **One quantity per component.** A component has at most one
   quantity declared with `quantity` and no policy. A second is an
   error at its name, the first its witness. A quantity declaration
   with `{ round: … }`, and a refinement `Q in D { … }`, is a
   boundary denomination of the component's quantity, and a
   refinement of a quantity is denominated in its component. A
   component with boundary denominations and no quantity is not an
   error here.

   ```hale,fragment
   unit g;
   unit kg = 1000 g;
   type Mass = quantity Int in g;
   type Weight = quantity Int in kg;   // error: … already have their quantity, `Mass`
   type Heavy = quantity Int in kg { round: floor; }   // a denomination of `Mass`
   ```

5. **A quantity counts an `Int` in a denomination:** `quantity Int
   in <unit>`. `quantity Float`, a quantity with no `in`, and `Int in
   D` with no word are errors saying what to write.

   ```hale,fragment
   type F = quantity Float in g;   // error: a quantity counts an `Int`
   type I = Int in g;              // error: write `quantity Int in g`
   ```

6. **A point is over a quantity**, or refines a point. An `origin:`
   is a point's, and a point's denomination and origin are units of
   its quantity's component.

   ```hale,fragment
   type At = point Mass;
   type P = point Int;                      // error: a point is over a quantity
   type C = point Mass { origin: 3 cent; }  // error: `cent` is not a unit of `Mass`'s component
   ```

7. **`round:` and `range:` mean something.** `round:` names one of
   the five policies, on a quantity or a point. A `range:`'s bounds
   are integer literals (a leading minus allowed) spanning at least
   one value, and a refinement's range sits inside its nearest
   ancestor's; the witness is that range.

   ```hale,fragment
   type Byte = Int { range: 0..256; }
   type Wide = Byte { range: 0..300; }   // error: not inside `Byte`'s `0..256`
   ```

8. **An identity is `distinct Int`**, with no denomination. A range
   type refines `Int`, an identity or another range, has no
   denomination, and a refinement of `Int` states its range.

   ```hale,fragment
   type F = distinct Float;          // error: an identity is `distinct Int`
   type R = Float { range: 0..1; }   // error: a range type refines `Int`, an identity or another range
   ```

9. **A refinement refines a scalar.** A declaration's base is a
   declared type; a refinement (no kind word) refines a quantity, a
   point, an identity, a range or `Int`, never a struct, an enum, an
   alias of one or another non-scalar, and never itself through a
   chain.

   ```hale,fragment
   type Order { id: Int; }
   type O = Order { range: 0..3; }   // error: `Order` is a struct
   ```

10. **The dimensionless component.** A unit defined against a number
    is in the one component the number one belongs to, with every
    other such unit; laws 3 and 4 judge it as any component (so
    `quantity Int in bp` and `quantity Int in pct` are two quantities
    of it, and the second is an error).

    ```hale,fragment
    unit bp = 1/10000;
    unit pct = 1/100;   // one component with `bp`: one `pct` is 100 `bp`
    ```

### Where values are typed

Every scalar's values are typed: an identity's and a range's (§
Identities and ranges), a quantity's and a point's (§ Quantities and
points). The declarations themselves lower to no code. A declaration
the laws refuse has no values: its name resolves to nothing, so a use
of it is no second error. A cast's name means what it means at the
call: a local, a parameter or a fn of the name is the callee, and no
cast. A struct field's default is evaluated at each literal that
leaves the field, in that literal's scope, so it is typed there (with
the scopes it opens, the parameter defaults its calls leave and the
field defaults its own literals leave), and a parameter's default at
each call that leaves it; a quantity rule's error found there is
reported once, at its place.

## Identities and ranges

An **identity** (`type OrderId = distinct Int;`, with or without a
range) and a **range** (`type Byte = Int { range: 0..256; }`, or a
range over an identity or another range) are nominal types: the name
survives resolution, and two of them never unify. Each is represented
as an `Int`. The checker knows one by its scope entry
(`TypeKind::Scalar`) and its scalar row (kind, parent, range), and
applies the rules below (`hale_types::unit_values`).

```hale
type OrderId    = distinct Int;
type SeqNo      = distinct Int { range: 0..65536; }
type Session    = distinct Int { range: 0..64; }
type Byte       = Int { range: 0..256; }
type Nibble     = Byte { range: 0..16; }
type RegisterId = Int { range: 0..16; }
```

A value's **range** is its type's, or its nearest ancestor's; an
identity with no range has none.

**Literals.** An integer literal where a value of an identity or a
range is expected (a `let` annotation, an argument, a return, a field,
an `or` substitute, an element of an array literal of them, the other
side of a comparison with an identity) is a value of that type, and one
outside its range is refused at the literal. A literal with no expected
type is an `Int`.

```hale,fragment
let s: Session = 12;       // a Session
let r: RegisterId = 20;    // error: `20` is outside `RegisterId`'s range `0..16`
let n = 5;
let o: OrderId = n;        // error: `Int` is not `OrderId`: an identity is reached
                           // only through the explicit conversion `OrderId(…)`
```

**Widening is free.** A range is a value of its parent, and a range
rooted at `Int` is an `Int`, wherever one is expected: an assignment, an
argument, a return, an operand. An identity widens to nothing; its
`Int` is `Int(id)`. Each implicit widening is a conversion row (below)
and is lowered to nothing.

```hale,fragment
let b: Byte = 200;
let n: Int = b;            // free
let x: Int = o;            // error: `OrderId` is an identity and widens to nothing:
                           // write `Int(…)` for its `Int`
let low: Nibble = b;       // error: `Byte` does not narrow to `Nibble` implicitly:
                           // write `Nibble(…) or …`, which says what becomes of a value outside `0..16`
```

**Equality and ordering** (`==`, `!=`, `<`, `>`, `<=`, `>=`) hold between
two values of one identity or one range, and along a widening (a
sub-range and its parent, a range rooted at `Int` and an `Int`); the
result is `Bool`. Two distinct identities, an identity and an `Int`,
and two unrelated scalar types are refused, naming both.

```hale,fragment
let same = o == o;
let bad = o == q;          // error: `OrderId` and `SeqNo` are distinct identities; convert one explicitly
let worse = o < 5 + 0;     // error: `OrderId` and `Int` are distinct types; convert one explicitly
```

**Arithmetic.** An identity has none, and neither has a range of one.
A range's arithmetic is its `Int`'s: the sum of two `Byte`s is an
`Int`, and storing it back into a `Byte` is a narrowing. A compound
assignment (`x += 1`) is the operator and the store.

```hale,fragment
let next = o + 1;          // error: `OrderId` is an identity; it has no arithmetic
let sum = b + b;           // an `Int`
b += 1;                    // error: `Int` does not narrow to `Byte` implicitly: …
```

**Conversions** are named: `T(x)`, where `T` is an identity, a range or
`Int`. The checker classifies each and records it in the typed bodies'
`conversions` column (spec/registry.md, `expression_typing`), keyed by
its call; lowering reads the row.

| conversion | kind |
|---|---|
| `OrderId(n)`: an `Int` into an identity with no range | total |
| `Int(o)`: an identity's `Int` | total |
| `Int(b)`, `Byte(nib)`: into an ancestor | widening |
| `Session(n)`, `Nibble(b)`: into a range the value may be outside of | narrowing |
| `SeqNo(o)`, `OrderId(b)`: across two families | refused: a conversion between them goes through `Int` (`SeqNo(Int(o))`) |

A **narrowing** is fallible, and its `or` says what becomes of a value
outside the range. Its failure is a `RangeError { kind: String; value:
Int; low: Int; high: Int }` (the target's name, the value, the
half-open range), a builtin type injected where a type declares a
range.

| discharge | result outside the range |
|---|---|
| `or <value>` | the value, a value of the target held to its range |
| `or clamp` | the nearest bound |
| `or wrap` | `low + ((v - low) mod w)`, `w = high - low`, the remainder taken non-negative, for every `Int` `v` (its extremes included), computed without overflow |
| `or handler(err)` | the handler's result, given the `RangeError` |
| `or raise` | the enclosing fallible fn fails with the `RangeError` |
| `or fail <payload>` | the enclosing fallible fn fails with its own payload |

After a narrowing, `clamp` and `wrap` are policies (decision 6 of the
plan); the value of a local of that name is written in parentheses,
`or (clamp)`. A bare narrowing is refused by the `bare_fallible` law;
a total conversion or a widening under an `or` is refused as not
fallible.

```hale,fragment
let sess = Session(hdr.session) or 0;
let seq = SeqNo(n) or wrap;
let top = Byte(n) or clamp;
let s = Session(n);        // error: `Session(…)` narrows `Int` into `Session`'s range `0..64` and this
                           // conversion says nothing about a value outside it: write `or <fallback>` …
```

**Lowering** reads each cast's row and decides nothing: a total
conversion and a widening emit nothing; a narrowing emits its two
comparisons and, from the row's policy, two selects (`clamp`), an
unsigned remainder of the distance from `low` on whichever side of it
the value is, exact in an `Int`'s 64 bits (`wrap`), or the value and the
`RangeError` the `or`'s join takes. Lowering decides nothing by a
name: a call is a conversion when the checker recorded a row for it,
and a call with no row is the ordinary call the checker resolved, so a
local or a parameter named like the type (`let Money = id;`) is the
callee of `Money(1)`. A default is evaluated, and typed, at each place
that leaves it (a struct literal that leaves a field, a call that
leaves a parameter), in that place's scope, so a default's conversion
is classified per evaluation, along the chain of defaults that reaches
it: every conversion row in it is recorded in the typed body of the
declaration that evaluates it, by the evaluation path (every place on
the way in, from the outermost: the literal or call that leaves a
default, then the literal or call in that default that leaves the next)
and the site, and lowering reads the row of the path it lowers. That
holds for a cast (a quantity's `.in(u)` and `.split(u)` included), and
equally for a quantity literal's count, a value converted where it
stands, a division by a literal and a printed value: what a value flows
into is that scope's too, so `Bucket(2000msec)` hands its literal to the
cast as 2 `sec` in one scope and to a local `fn(d: Span)` shadowing
`Bucket` as 2000 `msec` in another. One default can be a conversion in
one scope and a call of a local in another, at the top or inside
another default:

```hale,fragment
type S { n: ItemId = ItemId(1); }
let a = S {};              // `ItemId(1)`: the conversion
{
    let ItemId = bump;
    let b = S {};          // `ItemId(1)`: the call `bump(1)`
}
type Pair {
    a: S = S {};                           // the conversion
    b: S = { let ItemId = bump; S {} };    // the call `bump(1)`
}
```

**Layout.** Wherever a type reaches a representation, an identity or a
range is its `Int`: it prints as its `Int`, it is a legal hashmap key
and routing key, a flat payload's field, an FFI `Int`, a generic
argument whose monomorph lays it out as an `Int` (`Box<OrderId>`) and
an array element, and a topic's
shape string tags its field `i`, as an `Int` field's (decision 9), so
declaring one moves no shape hash.

## Quantities and points

A **quantity** (`type Money = quantity Int in cent;`, and a boundary
denomination of one, `type Ledger = Money { round: half_even; }`) and a
**point** (`type Price = point Tick;`, `type Celsius = point TempDelta
{ origin: 273_150 mK; }`, and a refinement of one, `type WireStamp =
Instant in usec { round: floor; }`) are nominal types, each represented
as the `Int` it counts. The checker knows one by its scope entry
(`TypeKind::Scalar`) and its scalar row (kind, component,
denomination, quantity, origin, policy), and applies the rules below
(`hale_types::unit_quantities`); the stdlib's `Duration` and `Time`
are a quantity and a point like these, represented as their own class
(§ The stdlib's time catalogue). The examples use this catalogue, a
program's own time units (`nsec` … `sec`) beside the stdlib's:

```hale
unit nsec;
unit usec = 1_000 nsec;
unit msec = 1_000 usec;
unit sec = 1_000 msec;
unit cent;
unit USD = 100 cent;
unit pct = 1/100;
unit bp = 1/100 pct;
unit tick;
unit mK;
unit K = 1_000 mK;
type Span      = quantity Int in nsec;
type Instant   = point Span;
type WireStamp = Instant in usec { round: floor; }
type Bucket    = Span in 100 msec { round: floor; }
type Seconds   = Span in sec;
type Money     = quantity Int in cent;
type Rate      = quantity Int in bp;
type Ledger    = Money { round: half_even; }
type Tick      = quantity Int in tick;
type Price     = point Tick;
type TempDelta = quantity Int in mK;
type Kelvin    = point TempDelta;
type Celsius   = point TempDelta { origin: 273_150 mK; }
```

### A type and its denomination

An expression's quantity or point type carries its denomination, and
the denomination need not be one a declaration pinned. A declared type
is at its row's denomination. Any other is **synthesized**: the same
kind, keyed by its quantity (a point's: its frame, the nearest point
on its chain stating an origin, else the point declared with the word)
and the denomination, displayed and named `Span in sec`, `Money in
1/100 cent`, `Span in 100 msec`. The spelling is the first unit of the
component the denomination is one of, else the unit it is the
smallest whole multiple of, else a fraction of its quantity's unit. A
synthesized type has no policy and no range, and is an ordinary type
for a `let`, an argument or a return; at its own quantity's
denomination it is that quantity. Two types of one quantity convert by
a factor alone (below), whatever their names.

### Literals

A **quantity literal** (`3bp`, `1_250_000USD`, `500msec`, `500ms`: an
integer adjacent to a unit name, decision 3) names its unit in the
units' namespace, which is its own: a local named `sec` neither
shadows the unit `sec` nor is shadowed by it. The literal is its
unit's component's quantity, at the quantity's denomination when the
literal is a whole count of it (U4): `3sec` is 3,000,000,000 of
`Span`, `1_250_000USD` 125,000,000 of `Money`, `3bp` 3 of `Rate`, and
`500ms` 500,000,000 of `Duration`. A literal that is no whole count
of its quantity's denomination is at its own unit: in a `Mass`
counted in grams, `5mg` is a `Mass in mg`. A literal of a unit no
`unit` declares, or of a unit whose component has no quantity, is
refused at the literal.

Where a literal flows (a binding, an argument, a return, an operand,
a field or a default), it is **converted at compile time**: its row
holds its count in the denomination it flows into, which lowering
emits as a constant. A whole number of that denomination is exact; one
that is not is a narrowing like any other (§ Conversions), which the
target's `round:` discharges or the check refuses. Literal arithmetic
is not folded: `3sec + 500msec` is two constants, 3,000,000,000 and
500,000,000 of `Span`, and an addition.

```hale,fragment
let d: Span = 3sec;        // the constant 3000000000
let b: Bucket = 150msec;   // 1: Bucket rounds down
let s: Seconds = 2_000msec; // 2, exact
let t: Seconds = 1_500msec; // error: `Seconds` from `Span` divides by 1,000,000,000: say what happens
                            // to the remainder: convert explicitly (…), or give `Seconds` a `round:` policy
let q = 3xyz;              // error: `3xyz`: no `unit` declares `xyz`
```

### The algebra

| operands | result |
|---|---|
| quantity `±` quantity, one component | the quantity, at the finer of the two denominations (the meet), each operand widened exactly |
| quantity `*` `Int`, `Int` `*` quantity | the quantity, its denomination |
| quantity `*` dimensionless quantity, either order | the quantity at the product of the two denominations, exactly: nothing is rounded at the product |
| quantity `/` `Int` | the quantity; a runtime divisor is integer division, an integer literal other than one a narrowing the site discharges (`spread / 2 or floor`) |
| quantity `/` quantity, one component | `Int`, the quotient of the two counts at their meet |
| point `-` point, one origin | the quantity, at the meet |
| point `±` quantity, quantity `+` point | the point, at the meet |
| comparison, one quantity or two points of one origin | `Bool`, exact at the meet |
| point `+` point; a quantity `/` a dimensionless one; any other product of two quantities; anything across components, across a quantity and a point, with an `Int`, an identity or a range; `%` and the bitwise operators | refused, naming both declarations |

A quantity negates; a point does not. An integer literal `0` is a
value of every quantity (zero counts the same in every denomination);
any other `Int` reaches a quantity by a unit (`n * 1cent`), and a
quantity's count in a unit is a quotient (`q / 1cent`).

```hale,fragment
let d = 3sec + 500msec;           // Span: 3500000000
let n: Int = d / 1msec;           // 3500
let fee = 1_250_000USD * 3bp;     // Money in 1/10000 cent: 375000000, exact
let spread = ask - bid;           // Tick
let mid = bid + (spread / 2 or floor);
let bad = 5msec + 4KiB;           // error: `Span` + `ByteCount`: different
                                  // quantities, `Span` and `ByteCount`; `+` holds within one
let no = bid + ask;               // error: `Price` + `Price`: two points do not add; …
```

### Conversions

Wherever a value of one denomination meets another (a binding, an
argument, a return, an operand, a struct field or a default, a
compound assignment, `.in(u)`, `.split(u)`, a cast `T(x)`, a division
by a literal), the checker classifies the site and records it in the
typed bodies' `conversions` column (spec/registry.md,
`expression_typing`): the exact **factor** `p/q` from the source's
denomination to the target's, a point's **shift** across two origins,
and how the site is **discharged**.

- **Widening**: `q = 1`. Exact and implicit; lowering multiplies by
  `p`.
- **Narrowing**: `q > 1`. The remainder of a division by `q` has to
  become something, and the program says what: at the site, with the
  `or` after `.in(u)`, a cast or a division by a literal; or by the
  **target type's `round:`** (a boundary type), which needs no `or`.
  An implicit conversion has no `or` of its own: into a type with no
  policy it is refused, pointing at the explicit conversion or the
  policy. A narrowing nothing discharges is the `bare_fallible` law's
  error: "`Bucket` from `Span` divides by 100,000,000: say what happens
  to the remainder: `or floor`, `or <value>`, `or raise`, or give
  `Bucket` a `round:` policy".

| discharge | result |
|---|---|
| `or floor`, `or ceil`, `or trunc` | rounded toward minus infinity, plus infinity, zero |
| `or half_up`, `or half_even` | to the nearest, a half away from zero, or to the even quotient |
| a type's `{ round: … }` | its policy, the same five |
| `or <value>` | exact, or the value (a value of the target) |
| `or handler(err)` | exact, or the handler's result, given the `InexactError` |
| `or raise` | exact, or the enclosing fallible fn fails with the `InexactError` |

A division that leaves a remainder fails with an `InexactError { kind:
String; value: Int; divisor: Int }` (the target's name, the count
divided, the divisor), a builtin type injected where a type is a
quantity or a point. After a conversion that divides, the five
rounding words are policies, as `clamp` and `wrap` are after a range's
narrowing; a policy belongs to its narrowing's family, so `or clamp`
there, or `or floor` on a range's narrowing, is refused.

- **`x.in(u)`** is `x` at `u`'s denomination (`u` a unit, `.in(sec)`,
  or a multiple of one, `.in(100msec)`): a conversion like any other.
- **`x.split(u)`** is `(whole: Int, rest)`: the floored quotient by
  `u`, and the remainder, never negative, at the finer of `x`'s and
  `u`'s denominations. It is total.
- **`T(x)`**, `T` a quantity or a point: `x` of `T`'s component
  converted into `T`. A quantity into a point is the point that far
  from the point's origin (`Instant(d)`); a point into a point of
  another origin shifts by the two origins' difference (`Kelvin(c)`);
  across components, a point into a quantity, or an `Int` into either,
  is refused.

```hale,fragment
let whole = d.in(sec) or 0;           // 3.5 seconds is no whole number of them: 0
let secs  = d.in(sec) or floor;       // 3sec
let (sec_count, rest) = d.split(sec); // 3 and 500000000nsec
let odd   = 1_234_567USD * 3bp;       // Money in 1/10000 cent: 370370100, exact
let paid: Ledger = odd;               // 37037 cent: Ledger's half_even
let bad: Money = odd;                 // error: `Money` from `Money in 1/10000 cent` divides by 10,000: …
let k = Kelvin(Celsius(100_000mK));   // 373150 mK
```

A position the checker does not classify refuses a value counted in a
denomination no declaration names, with a located error, and never
stores it as the wrong count: a generic literal's field whose type is
known only where the literal flows, a generic fn's argument (a
monomorph is named by declared types), two arms of an `if` or a
`match` at different denominations.

### Printing, layout, overflow

A quantity **prints** as its count and its denomination: the unit
when the denomination is one of a unit (`1500msec`), else its type
(`3 Bucket`, `370370100 Money in 1/10000 cent`); a point prints as its
count. Printing in another unit is `.in(u)` first. A quantity inside a
record or a sequence prints as its count. The stdlib's `Duration`
prints as it always has, its nanoseconds (`1500000000ns`), and `Time`
its instant (`2026-05-08T12:00:00Z`): decision 8, its class's own
rendering; `d.in(ms) or floor` prints `1500ms`.

Wherever a type reaches a representation, a quantity or a point is its
`Int`: a hashmap key and a routing key, a flat payload's field, an FFI
`Int`. The stdlib's `Duration` and `Time` are their own class there,
as they always were. On the wire (decision 9) a quantity's or a point's field is its
integer, tagged by its denomination in the topic's shape string: `q(`
the denomination as its nearest declaration writes it `)` (`q(cent)`,
`q(100 msec)`), a point's adding `point` and its origin when one is
written (`q(mK point 273150 mK)`), so two processes whose fields count
in different denominations disagree in the shape hash. A program with
no quantity renders every shape as before.

Overflow follows `Int`'s arithmetic (decision 7): a widening's
multiplication is the `Int` multiplication. A factor no `Int` holds is
an error at the conversion when the program is built, never a wrap; a
literal's converted count that overflows is refused by the check.

**Lowering** reads each row and decides nothing: a widening is one
multiplication by `p` (and a point's shift, one addition); a narrowing
one division by `q`, then for a rounding the one correction its policy
is (none for `trunc`; down on a negative remainder for `floor`, up on
a positive one for `ceil`; away from zero at half or more for
`half_up`; past half, or at half to an even quotient, for
`half_even`), or for a checked discharge the remainder's test, the
quotient on one path and the `InexactError` on the other, joined by
the `or`. A literal is its row's count. A value converted where it
stands has its row at its span. An arithmetic operator has its row at
its span, its result's type, which says what representation the
operation is emitted in (`Int * Duration` a `Duration`, `Time - Time`
a `Duration`). A row is read from the body being emitted, the
declaration the checker recorded it in, and never from another (a
stdlib body's spans overlap the first file's). A literal with no row
is a missing required row, refused where it is written, save in a body
the checker types no row in (the stdlib's own): there a time literal
is its count from the stdlib's catalogue, the literal's own row in
every program.

## The stdlib's time catalogue

The stdlib's seed (`std::time`, `hale-stdlib/hl/time.hl`) declares the
time catalogue and the two time types, as a program declares its own
(GH #1076, step U4):

```hale
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

Its rows are the first of every program's (§ The rows), in the
stdlib's universe. A type position reads `Duration` and `Time` as
these declarations: their scalar rows name the primitive each is
(`ScalarRow::primitive`), so a value keeps the representation class
the primitives always had: i64 nanoseconds (a `Time`, since the epoch
the runtime defines; the point has no `origin:`), printed as it always
was (`1500000000ns`, an ISO-8601 instant), a topic field tagged `u`
and `t` (so no shape hash, payload hash or observer hash moves), its
own FFI class, the runtime's nanosecond entry points. Every
`std::time` function takes and returns them as before.

What the declarations change is who decides. A time literal is a
quantity literal of one of these units (`500ms` is 500,000,000 of
`Duration`, the catalogue's factor; the lexer knows no suffix); the
algebra of quantities and points types their arithmetic (`Duration ±
Duration`, `Int * Duration`, `Duration / Int`, `Time ± Duration`,
`Duration + Time`, `Time - Time`, the comparisons; everything else is
refused naming both), and lowering emits each operator as its row
says; `.in(u)`, `.split(u)`, `Duration(x)` and `Time(x)` are the
dialect's conversions (`d.in(ms) or floor` is a `Duration in ms`,
printed `1500ms`). The names are `ns us ms s min h day` (decision 4):
`m` and `d` are no time units. A program's unit may join the
component by an equation against one of these (`unit tick = 10
ms;`), and never take one's name (law 1). Two quotients changed with
the declarations (U4's second correction, decision 5): `Duration /
Duration` is the `Int` every quantity's quotient by itself is (`1h /
1min` is 60), and a `Duration` divided by an integer literal other
than one is a narrowing like any quantity's, its `or` saying what
becomes of the remainder (`timeout / 2 or floor`); a runtime divisor
is the integer division it always was.

## Identities and equations

A node is a unit declaration's identity, a `SiteRef`: the universe
that minted the declaration and its `SiteId` there (`unit_graph::UnitId`,
U4). An equation has its own declaration identity and states that one
unit at `from` equals `p/q` units at `to`. Display names and source
spans do not participate in identity. Duplicate identities and
references to undeclared nodes are errors. An isolated declared node
is a valid component.

Factors are positive, reduced, arbitrary-precision rationals. Zero
and negative factors are rejected when a ratio is constructed. Point
origins are not multiplicative unit equations.

**A program's catalogue.** The unit rows close one catalogue over the
stdlib's time catalogue and the program's units: each unit
declaration's identity and each equation's. The stdlib's analysis copy
numbers its sites on its own, from seed 0 and index 0 as the snapshot
does, so a stdlib unit and a program's can share a `SiteId`; their
universes keep them two nodes, and the stdlib's catalogue and a
program's are disjoint components unless the program writes an
equation against a stdlib unit. The number one (`unit pct = 1/100;`)
is one more node, under an identity no mint issues (seed `u32::MAX`
in the program's universe; seeds are numbered from zero, one per
seed). That keeps the choice inside the rows: the API knows only unit
identities and has no notion of a pure number, and no declaration can
collide with it.

## Closure and conversion

`UnitGraph::close` proves every cycle's product is one. Failure returns
errors and no graph. Each inconsistency carries the claimed and
implied ratios and a closed sequence of equation identities and
directions whose product is not one.

Each connected component uses a traversal root internally. It has no
semantic status: changing identity ordering or declaration ordering
cannot change any conversion ratio. The representation stores a
forest and relative scales rather than an all-pairs table.

`conversion(from, to)` reads the closure and returns the reduced
factor and a path of equation witnesses. Unknown nodes and nodes in
different components have no conversion. A factor with denominator
one is an exact denomination conversion; a greater denominator
requires a named loss decision from its consumer. The catalogue
chooses no rounding policy. Exact denomination conversion does not
prove that the result fits a runtime range or machine width.

## Denomination meet

`meet(inputs)` computes the coarsest denomination into which every
input converts by an integer factor. For positive reduced rational
scales this is the gcd of numerators divided by the lcm of
denominators. A declared node need not spell the result: inputs with
scales `3/2` and `5/2` have meet `1/2`.

The result belongs to its originating catalogue and retains an
irredundant set of input positions as its explanation. Removing any
retained input changes the meet. Redundant inputs are omitted, with
earlier inputs preferred when explanations are interchangeable.
There is no fixed two-input bound: the meet of `6`, `10`, and `15` is
`1`, but no pair proves it. An empty input set, unknown input, or
inputs in different components cannot produce a denomination.

A boundary's pinned denomination is queried separately with
`factor_to(target)`. Computing an unpinned meet never changes that
target. The returned factor exposes any narrowing that the boundary
consumer must address.

## Denominations as values

A `Denom` is a declared unit and an exact positive multiple of it
(`100 ms` is `{ ms, 100 }`). It is stored and compared without the
catalogue; two values may denote one denomination (`{ ms, 1 }` and
`{ us, 1000 }`), which only the catalogue tells: `factor(from, to)`
is how many `to` one `from` is, exact, and none for units in
different components. A meet is a value too (`Denomination::value`,
stated against its first input's unit).

A factor reaches machine integers only through `Ratio::to_machine`:
two `i64`s, or `FactorOverflow` carrying the factor when either part
does not fit. Nothing truncates.
