# The unit dialect

A program says what its integers count. It declares **units** and the
equations between them, and **scalar types** counted in them: a
**quantity** (an amount: `Money`, `Duration`), a **point** (a position
on a quantity's line: `Price`, `Time`), an **identity** (an integer
that never mixes with another: `OrderId`) and a **range** (an integer
with bounds: `Byte`). Every value of one is the `Int` it counts. The
compiler knows each conversion between two of them as one exact
rational factor, and holds one rule: **widening is free and narrowing
is named**. A conversion that multiplies by a whole factor is exact and
implicit; one that divides, or that may leave a range, is written with
what becomes of the remainder or of the value outside, at the site
(`or floor`, `or clamp`, `or 0`) or on the target type (`{ round:
half_even; }`), never by a default. `Duration` and `Time` are two of
these declarations, the stdlib's (§ The stdlib's time catalogue).

GH #1076; the committed form is the comment on #1212 of 2026-09-28,
"The unit dialect, concretely". The productions are `unit_decl` and
`type_decl`'s `scalar_body` in spec/grammar.ebnf; the words are
spec/tokens.md § Unit dialect words; a literal's spelling is
spec/tokens.md § Quantity literals. The examples below use this
catalogue, beside the stdlib's time catalogue:

```hale
unit B;
unit KiB = 1024 B;
unit cent;
unit USD = 100 cent;
unit bp = 1 / 10000;
unit pct = 100 bp;
unit tick;
unit mK;

type ByteCount = quantity Int in B;
type Money     = quantity Int in cent;
type Ratio     = quantity Int in bp;
type Tick      = quantity Int in tick;
type Price     = point Tick;
type TempDelta = quantity Int in mK;
type Kelvin    = point TempDelta;
type Celsius   = point TempDelta { origin: 273_150 mK; }
type OrderId   = distinct Int;
type SeqNo     = distinct Int { range: 0..65536; }
type Session   = distinct Int { range: 0..64; }
type Byte      = Int { range: 0..256; }
type Nibble    = Byte { range: 0..16; }
type WireStamp = Time in us { round: floor; }
type Bucket    = quantity Int in 100ms { round: floor; }
type Ledger    = quantity Int in cent { round: half_even; }
type Seconds   = Duration in s;
```

## Declarations

**A unit.** `unit tick;` declares a unit with no equation, its own
component. `unit KiB = 1024 B;` declares `KiB` and states that one
`KiB` is 1024 `B`. A factor is a positive integer or a ratio of two
(`unit inch = 127/5 mm;`); a zero in either place is a parse error.
With no target the unit is that multiple of the number one: `unit bp
= 1 / 10000;`. The magnitude and the target may be written apart or
together (`1_000 ns`, `1_000ns`); the two spellings are one equation.
A unit is seed-global: its name is never mangled across an import.

**A scalar type.** A `type` declaration whose body is

```text
[quantity | point | distinct] BASE [in DENOMINATION] [{ CLAUSE; ... }]
```

with at least one of the kind word, the `in` and the clause block
(with none of the three it is an alias). What it declares:

| kind | written | example |
|---|---|---|
| quantity | `quantity Int in D` | `type Money = quantity Int in cent;` |
| boundary denomination of a quantity | `Q in D`, `Q { round: …; }`, or `quantity Int in D { round: …; }` over a component that has its quantity | `type Ledger = quantity Int in cent { round: half_even; }`, `type Seconds = Duration in s;` |
| point | `point Q`, optionally `{ origin: N UNIT; }` | `type Celsius = point TempDelta { origin: 273_150 mK; }` |
| refinement of a point | `P in D { … }` | `type WireStamp = Time in us { round: floor; }` |
| identity | `distinct Int`, optionally `{ range: LO..HI; }` | `type Session = distinct Int { range: 0..64; }` |
| range | `Int { range: LO..HI; }`, or over an identity or a range | `type Byte = Int { range: 0..256; }` |

A denomination is a unit and a positive integer multiple of it (`ns`,
`100ms`, `100 ms`). The clauses are `range: LO..HI` (or `LO..=HI`),
`round: POLICY` and `origin: N UNIT`; each appears at most once, and
any other name is a parse error listing the three. A clause block
closes the declaration, so a `;` after it is optional. A scalar type
takes no generic parameters. The words `unit`, `quantity`, `point` and
`distinct` are contextual: each is recognized in its one position and
is an ordinary identifier everywhere else.

**A quantity literal** is an integer written against a unit name, with
no space: `3bp`, `1_250_000USD`, `500ms` (§ Literals). `1_250_000 USD`
is two tokens and a parse error ("expected ;, got Ident("USD")").

**`x.in(D)`** is its own form (`in` is a keyword elsewhere), and
`x.split(u)` a method call (§ Conversions).

## The rows

The declarations are rows of one family, `unit_declarations`
(`hale_types::units::derive_unit_rows`, spec/registry.md), derived once
per snapshot from the stdlib's seed and then the programs after the
desugar sequence, and not gated on the typing: it reads declarations
only. Each row is keyed by its declaration's site in the universe that
minted it (§ The catalogue): the stdlib's time catalogue and its
`Duration` and `Time` come first, then the program's own.

- **A unit row** per `unit` declaration, and **an equation row** per
  equation: one `unit` is `p/q` of its target, a unit or the number
  one.
- **A reference row** per place a declaration names a unit (an
  equation's target, a denomination, an origin), resolved by name to
  the first unit of that name, or to none.
- **Components.** The units of every equation whose target resolves
  are joined; a unit with no equation is its own component. The number
  one is a node like a unit, so every unit written against a number
  (`unit bp = 1 / 10000;`, `unit dozen = 12;`) is in one component, the
  dimensionless one.
- **The catalogue** is closed once from the unit and equation rows
  (§ The catalogue), and kept only when no catalogue law fails: a unit
  declared twice, an equation naming no declared unit, a cycle that
  does not multiply to one.
- **A scalar row** per scalar `type` declaration:
  - its **kind**: its word's (`distinct` is an identity); with no word,
    its base's (a refinement of a quantity is a quantity, of a point a
    point, of `Int`, an identity or a range a range; `Int in D` is a
    quantity missing its word, which law 5 refuses);
  - its **parent**, the scalar it is written over;
  - a quantity's or a point's **denomination**, a value (a unit and an
    exact positive multiple): its own `in`, else its parent's; and its
    **component**, its parent's, else its denomination's unit's;
  - whether it is its component's **quantity** (declared with
    `quantity` and no policy: law 4), and its **`of`**: a point's
    quantity, a range's parent, and for every other quantity of the
    component, that one quantity;
  - a point's **origin**, an exact count of its own denomination
    (`origin: 273 K` on a point in `mK` is 273000);
  - its **policy** (`floor`, `ceil`, `trunc`, `half_even`, `half_up`)
    and its **range**, half-open (`..=` stores its upper bound plus
    one).

`Money` is the quantity of the component `{cent, USD}`; `Ledger` is a
boundary denomination of it, a quantity row whose `of` is `Money`,
with the policy `half_even`; `Bucket` is one of the stdlib's
`Duration`.

## The laws

Each law is a registered structural rule (spec/verification.md §
Structural & design rules), judged over the rows by `unit_laws`, and
reported as a located error whose related notes are its witness. The
wordings are the compiler's.

1. **A unit is declared once.** At the second declaration's name, the
   first its witness: "unit \`cent\` is declared twice: a unit is one
   node of the catalogue, so it is declared once, with at most one
   equation; remove this declaration or give the unit another name"
   (note: "\`cent\` is first declared here"). A program's unit of a
   name the stdlib's time catalogue declares is that unit declared
   again, the message naming the catalogue, whose declaration is in no
   file of the program: "unit \`ms\` is declared twice: the stdlib's
   time catalogue declares \`ms\` (\`std::time\`), and a unit is one
   node of the catalogue; write \`ms\` for that unit (\`unit tick = 10
   ms;\` joins a unit of the program's to it), or give this one another
   name".

   ```hale,fragment
   unit cent;
   unit cent;          // error: unit `cent` is declared twice
   unit ms;            // error: … the stdlib's time catalogue declares `ms` …
   unit tick = 10 ms;  // a unit of the program's, in the time component
   ```

2. **A named unit is declared.** An equation's target, a denomination
   and an origin name a declared unit; the error is at the name and
   suggests the nearest declared one (a program's own before the
   stdlib's), or says what a retired time unit is now: "unit \`USD\`:
   its equation names \`cnet\`, which no \`unit\` declares: declare it
   (\`unit cnet;\`); did you mean \`cent\`?"; "type \`Wait\`: its
   denomination names \`m\`, which no \`unit\` declares: declare it
   (\`unit m;\`); minutes are \`min\`".

   ```hale,fragment
   unit USD = 100 cnet;   // error: … names `cnet`, which no `unit` declares …
   ```

3. **Every cycle multiplies to one.** At the equation the closure
   found inconsistent, stating what it claims and what the others
   imply; the witness is the other path, one note per step at its
   declaration: "unit \`c\`: \`unit c = 5 b\` makes one \`c\` 5 \`b\`,
   and the other equations make it 31/6 \`b\`: every cycle of equations
   multiplies to one, so one equation on this cycle is wrong; correct
   it (to \`unit c = 31/6 b;\` if it is this one)" (notes: "\`unit a =
   1/31 c\`: one \`c\` is 31 \`a\`", "\`unit b = 6 a\`: one \`a\` is 1/6
   \`b\`").

   ```hale,fragment
   unit a = 1/31 c;
   unit b = 6 a;
   unit c = 5 b;   // error: … and the other equations make it 31/6 `b` …
   ```

4. **One quantity per component.** A component has at most one quantity
   declared with `quantity` and no policy; a second is an error at its
   name, the first its witness. A quantity declared with a `round:`, or
   a refinement `Q in D { … }`, is a boundary denomination of the
   component's quantity, and a refinement of a quantity stays in its
   component. "type \`Weight\`: the units of \`kg\` already have their
   quantity, \`Mass\`: a component of the catalogue has one quantity,
   and every other type over it is a denomination of that one; write
   \`type Weight = Mass in kg;\`, or give it a \`round:\` policy". The
   stdlib's `Duration` is the time component's quantity, so `type
   Duration = quantity Int in ns;` in a program is this error, naming
   the stdlib's.

   ```hale,fragment
   type Mass = quantity Int in g;
   type Weight = quantity Int in kg;                   // error: … already have their quantity, `Mass` …
   type Heavy = quantity Int in kg { round: floor; }   // a denomination of `Mass`
   ```

5. **A quantity counts an `Int` in a denomination.** "type \`F\`: a
   quantity counts an \`Int\`, and \`Float\` is not one: write
   \`quantity Int in g\`"; a quantity with no `in` and `Int in D` with
   no word are refused saying what to write.

   ```hale,fragment
   type F = quantity Float in g;   // error: a quantity counts an `Int` …
   type I = Int in g;              // error: `Int in g` is a quantity without its word …
   ```

6. **A point is over a quantity**, or refines a point; an `origin:` is
   a point's, and a point's denomination and origin are units of its
   quantity's component: "type \`P\`: a point is over a quantity, and
   \`Int\` is a primitive"; "type \`C\`: the origin is in \`cent\`, which
   is not a unit of \`Mass\`'s component: an origin is a count of the
   point's own quantity".

   ```hale,fragment
   type P = point Int;                      // error: a point is over a quantity …
   type C = point Mass { origin: 3 cent; }  // error: the origin is in `cent` …
   ```

7. **`round:` and `range:` mean something.** `round:` names one of the
   five policies, on a quantity or a point; a `range:`'s bounds are
   integer literals (a leading minus allowed) spanning at least one
   value, and a refinement's range sits inside its nearest ancestor's,
   which is the witness: "type \`Wide\`: \`range: 0..300\` is not inside
   \`Byte\`'s \`0..256\`: a refinement narrows its parent's range, never
   widens it".

   ```hale,fragment
   type Wide = Byte { range: 0..300; }        // error: … is not inside `Byte`'s `0..256` …
   type Mass = quantity Int in g { round: flor; }   // error: `flor` is not a rounding policy …
   ```

8. **An identity is `distinct Int`**, with no denomination; a range
   type refines `Int`, an identity or another range, has no
   denomination, and a refinement of `Int` states its range: "type
   \`F\`: an identity is \`distinct Int\`, and \`Float\` is not \`Int\`:
   write \`distinct Int\`".

   ```hale,fragment
   type F = distinct Float;          // error: an identity is `distinct Int` …
   type R = Float { range: 0..1; }   // error: a range type refines `Int`, an identity or another range …
   ```

9. **A refinement refines a scalar.** A base is a declared type; a
   refinement refines a quantity, a point, an identity, a range or
   `Int`, never a struct, an enum, an alias of one, or itself through a
   chain: "type \`O\`: a refinement refines a quantity, a point, an
   identity, a range or \`Int\` (with a \`range:\`), and \`Order\` is a
   struct".

   ```hale,fragment
   type O = Order { range: 0..3; }   // error: … and `Order` is a struct
   ```

10. **The dimensionless component** is a fact of the rows, no rule:
    every unit written against a number is in the one component the
    number one belongs to, and laws 3 and 4 judge it as any other (so
    `quantity Int in bp` and `quantity Int in pct` would be two
    quantities of it, and the second an error). With one equation per
    unit no cycle passes through the number, so nothing here could be
    violated.

    ```hale,fragment
    unit bp = 1 / 10000;
    unit pct = 100 bp;   // one component with the number: one `pct` is 1/100
    ```

11. **No unit is named like a literal's suffix.** The lexer reads a
    number's own suffixes before a unit: `d` alone is the Decimal
    literal's (`3d` is the Decimal `3`), and `e` or `E` followed by a
    digit is a Float's exponent. "unit \`d\`: a unit named \`d\`
    collides with the Decimal literal's suffix: \`3d\` is the Decimal
    \`3\`; give it another name".

    ```hale,fragment
    unit d;    // error: … collides with the Decimal literal's suffix …
    unit e5;   // error: … collides with a Float literal's exponent …
    ```

A declaration the laws refuse has no values: its name resolves to
nothing, so a use of it is no second error. A scalar declaration named
like a primitive is not refused by a law, but a type position reads the
primitive, so its name is unusable: `Bytes` is the buffer type (hence
`ByteCount` above, decision 10).

## The algebra

One table (`ScalarTypes::quantity_binop`, `hale_types::unit_quantities`;
the identities' and ranges' rules in `hale_types::unit_values`), read by
the checker and, through its rows, by lowering:

| operands | result |
|---|---|
| quantity `±` quantity, one component | the quantity, at the finer of the two denominations (their meet), each operand widened exactly |
| quantity `*` `Int`, `Int` `*` quantity | the quantity, its denomination |
| quantity `*` dimensionless quantity, either order | the quantity at the product of the two denominations, exactly: nothing is rounded at the product |
| quantity `/` `Int` | the quantity; a runtime divisor is integer division, an integer literal other than one a narrowing the site discharges (`spread / 2 or floor`) |
| quantity `/` quantity, one component | `Int`, the quotient of the two counts at their meet |
| point `-` point, one origin | the quantity, at the meet |
| point `±` quantity, quantity `+` point | the point, at the meet |
| comparison of one quantity, or of two points of one origin | `Bool`, exact at the meet |
| comparison of one identity, of one range, or along a range's widening | `Bool` |
| a range's arithmetic | its `Int`'s: `b + b` is an `Int` |
| point `+` point; a quantity `/` a dimensionless quantity; any other product of two quantities; anything across components, across a quantity and a point, with an `Int`, an identity or a range; any arithmetic on an identity; `%` and the bitwise operators | refused, naming both declarations |

A quantity negates; a point does not. Each refusal is at the operator,
with a note at each declaration:

```hale,fragment
let d = 3s + 500ms;            // Duration: 3500000000
let n: Int = d / 1ms;          // 3500
let fee = 1_250_000USD * 3bp;  // Money in 1/10000 cent: 375000000, exact
let spread = ask - bid;        // Tick
let mid = bid + (spread / 2 or floor);
let bad = 5ms + 4KiB;          // error: `Duration` + `ByteCount`: different quantities,
                               // `Duration` and `ByteCount`; `+` holds within one
let no = bid + ask;            // error: `Price` + `Price`: two points do not add; their difference
                               // is a quantity (`b - a`), and a point moves by a quantity (`a + d`)
let nope = id + id;            // error: `OrderId` is an identity; it has no arithmetic
let k = Kelvin(300_000mK) - c; // error: `Kelvin` - `Celsius`: two points of different origins;
                               // convert one to the other's first (`Kelvin(…)`)
let sq = d * d;                // error: `Duration` * `Duration`: a product of two quantities is a
                               // quantity only when one is dimensionless (a ratio); neither is
```

**A quantity and an `Int`.** An `Int` is no quantity: `d + n` is
refused ("\`Duration\` + \`Int\`: an \`Int\` is no quantity; a count
becomes one by a unit (\`n * 1ns\`)"), and so is `d > 0` ("…compare
with a quantity (\`0ns\`)"). A count becomes a quantity by a unit (`n *
1cent`), and a quantity's count in a unit is a quotient (`m / 1cent`).
Where a value of a quantity is expected (a binding, an argument, an
`or` substitute) the literal `0` is one, since zero counts the same in
every denomination: `let z: Money = 0;`, `d.in(s) or 0`.

**Identities and ranges.** Equality and ordering hold between two
values of one identity or one range, and along a widening (a sub-range
and its parent, a range rooted at `Int` and an `Int`); two distinct
identities, an identity and an `Int`, and two unrelated scalar types
are refused: "\`OrderId\` and \`SeqNo\` are distinct identities; convert
one explicitly". A compound assignment (`x += 1`) is the operator and
the store.

## Literals

A **quantity literal** names its unit in the units' namespace, which is
its own: a local named `s` neither shadows the unit `s` nor is shadowed
by it. The literal is its unit's component's quantity, **at the
quantity's denomination when it is a whole count of it**, else at its
own unit: `3s` is 3,000,000,000 of `Duration`, `1_250_000USD`
125,000,000 of `Money`, `3bp` 3 of `Ratio`; in a `Mass` counted in
grams, `5mg` is a `Mass in mg`. A literal of a unit no `unit` declares,
or of a unit whose component has no quantity, is refused at the
literal ("\`3xyz\`: no \`unit\` declares \`xyz\`"); so is a literal of
several units ("\`1h30m\`: a quantity literal has one unit; write \`1h +
30min\`").

Where a literal flows (a binding, an argument, a return, an operand, a
field or a default) it is **converted at compile time**: its row holds
its count in the denomination it flows into, which lowering emits as a
constant. A whole number of that denomination is exact; one that is
not is a narrowing like any other, which the target's `round:`
discharges at compile time or the check refuses. Literal arithmetic is
not folded: `3s + 500ms` is two constants and an addition.

```hale,fragment
let d: Duration = 3s;      // the constant 3000000000
let b: Bucket = 150ms;     // 1 Bucket: Bucket rounds down, at compile time
let t: Seconds = 2_000ms;  // 2, exact
let u: Seconds = 1500ms;   // error: `Seconds` from `Duration` divides by 1,000,000,000: …
```

An **integer literal** where a value of an identity or a range is
expected (a binding, an argument, a return, a field, an `or` substitute,
an element of an array literal of them, the other side of a comparison
with an identity) is a value of that type, and one outside its range is
refused at the literal: "\`300\` is outside \`Byte\`'s range \`0..256\`".
With no expected type it is an `Int`, and an `Int` reaches an identity
only through its conversion: "\`Int\` is not \`OrderId\`: an identity is
reached only through the explicit conversion \`OrderId(…)\`".

## Conversions

Wherever a value of one denomination or range meets another (a binding,
an argument, a return, an operand, a struct field or a default, a
compound assignment, `.in(u)`, `.split(u)`, a cast `T(x)`, a division by
a literal), the checker classifies the site and records it as a row of
the typed bodies' `conversions` column (spec/registry.md,
`expression_typing`): the exact **factor** `p/q` from the source's
denomination to the target's, a point's **shift** across two origins,
a range narrowing's **range**, a literal's converted **count**, and the
**policy** that discharges it.

- **Widening**: a factor with `q = 1`, or a range into its parent (a
  range rooted at `Int` into `Int`). Exact and implicit.
- **Narrowing**: `q > 1`, or into a range the value may be outside of.
  The program says what becomes of the remainder or of the value
  outside: at the site, with the `or` after `.in(u)`, a cast or a
  division by a literal; or, for a ratio, by the **target type's
  `round:`**, which needs no `or`. An implicit conversion has no `or`
  of its own: into a type with no policy it is refused, pointing at
  the explicit conversion or the policy.
- **Total**: an `Int` into an identity with no range, an identity's
  `Int`. Nothing can be lost; an `or` after one is refused ("\`SeqNo\`
  is not fallible (it returns \`SeqNo\`); drop the \`or\` clause").

**No narrowing is implicit.** A narrowing nothing discharges is the
`bare_fallible` law's error, at the conversion, whatever its form:

```hale,fragment
let x = d.in(s);           // error: `Duration in s` from `Duration` divides by 1,000,000,000:
                           // say what happens to the remainder: `or floor`, `or <value>`, `or raise`
let half = spread / 2;     // error: `Tick` divided by 2 leaves a remainder: …, or give `Tick` a `round:` policy
let bad: Money = odd;      // error: `Money` from `Money in 1/10000 cent` divides by 10,000: say what
                           // happens to the remainder: convert explicitly (`.in(u) or floor`,
                           // `Money(…) or half_even`, `or <value>`, `or raise`), or give `Money` a `round:` policy
let sess = Session(n);     // error: `Session(…)` narrows `Int` into `Session`'s range `0..64` and this
                           // conversion says nothing about a value outside it: write `or <fallback>` …
let s: Session = n;        // error: `Int` does not narrow to `Session` implicitly: write
                           // `Session(…) or …`, which says what becomes of a value outside `0..64`
```

**The policies.** After a narrowing the words below are policies; the
value of a local of that name is written in parentheses, `or (floor)`.
A policy belongs to its narrowing's family, so a range's word after a
ratio, or a rounding after a range, is refused ("\`or clamp\` is a
range's policy, and this conversion divides: …").

| discharge | a ratio's narrowing (`q > 1`) | a range's narrowing |
|---|---|---|
| `or floor`, `or ceil`, `or trunc` | rounded toward minus infinity, plus infinity, zero | refused |
| `or half_up`, `or half_even` | to the nearest; a half away from zero, or to the even quotient | refused |
| a type's `{ round: … }` | its policy, the same five, with no `or` | — |
| `or clamp` | refused | the nearest bound |
| `or wrap` | refused | `low + ((v - low) mod w)`, `w = high - low`, the remainder non-negative, for every `Int` `v`, computed without overflow |
| `or <value>` | exact, or the value (a value of the target) | the value, held to the range |
| `or handler(err)` | exact, or the handler's result, given the `InexactError` | the handler's result, given the `RangeError` |
| `or raise` | exact, or the enclosing fallible fn fails with the `InexactError` | the enclosing fallible fn fails with the `RangeError` |
| `or fail <payload>` | the enclosing fallible fn fails with its own payload | the same |

A division that leaves a remainder fails with an `InexactError { kind:
String; value: Int; divisor: Int }` (the target's name, the count
divided, the divisor), injected where the program declares a quantity
or a point; a value outside a range with a `RangeError { kind: String;
value: Int; low: Int; high: Int }`, injected where a type declares a
range.

```hale,fragment
let whole = d.in(s) or 0;                 // 3.5 s is no whole number of seconds: 0
let secs = d.in(s) or floor;              // 3s
let paid: Ledger = odd;                   // 37037 cent: Ledger's half_even
let cents = odd.in(cent) or half_even;    // the same, said at the site
let sess = Session(hdr.session) or 0;     // 0 when the session is outside 0..64
let seq = SeqNo(n) or wrap;               // around 0..65536
let top = Byte(n) or clamp;               // 255 for anything above
```

**The forms.**

- **`x.in(D)`** is `x` at `D`'s denomination (`D` a unit, `.in(s)`, or a
  multiple of one, `.in(100ms)`): a conversion like any other, into a
  type that may be synthesized (`Duration in s`). A unit outside the
  component is refused ("\`.in(…)\`: \`cent\` is not a unit of
  \`Duration\`: \`Duration\` counts in the units of \`Duration\`").
- **`x.split(u)`** is `(whole: Int, rest)`: the floored quotient by `u`
  and the remainder, never negative, at the finer of `x`'s and `u`'s
  denominations. It is total: `let (sec, rem) = d.split(s);` is 3 and
  `500000000ns`.
- **`T(x)`**, `T` a quantity or a point: `x` of `T`'s component
  converted into `T`. A quantity into a point is the point that far
  from the point's origin (`Time(n * 1ns)`); a point into a point of
  another origin shifts by the two origins' difference (`Kelvin(c)`).
  Across components, a point into a quantity, and an `Int` into either
  are refused ("\`Money(…)\` of an \`Int\`: a count becomes a quantity by
  a unit (\`n * 1cent\`)"). A literal's cast is the literal flowing into
  `T`, at compile time.
- **`T(x)`**, `T` an identity, a range or `Int`:

  | conversion | kind |
  |---|---|
  | `OrderId(n)`: an `Int` into an identity with no range | total |
  | `Int(o)`: an identity's `Int` | total |
  | `Int(b)`, `Byte(nib)`: into an ancestor | widening |
  | `Session(n)`, `Nibble(b)`: into a range the value may be outside of | narrowing |
  | `SeqNo(o)`, `OrderId(b)`: across two families | refused: "\`SeqNo\` and \`OrderId\` are distinct identities; a conversion between them goes through \`Int\`: \`SeqNo(Int(…))\`" |

  A range widens to its parent, and a range rooted at `Int` to `Int`,
  wherever one is expected, for free; an identity widens to nothing
  ("\`OrderId\` is an identity and widens to nothing: write \`Int(…)\`
  for its \`Int\`"). `Int(d)` of a quantity is refused, its count being
  a quotient ("\`Int(…)\` of \`Duration\`: a quantity's count in a unit
  is a quotient (\`q / 1ns\`), …").

A cast's name means what it means at the call: a local, a parameter or
a fn of the name is the callee, and no cast (`let Money = id;` makes
`Money(1)` a call of `id`). A **default** is evaluated, and typed, at
each place that leaves it (a struct literal that leaves a field, a call
that leaves a parameter), in that place's scope: every conversion row
in it is recorded in the typed body of the declaration that evaluates
it, by the evaluation path (every place on the way in, from the
outermost) and the site, so one default can be a conversion in one
scope and a call of a local in another, and a literal in it is
converted into what each scope flows it into (`Bucket(2000ms)` is 20
buckets in one scope and 2,000,000,000 ns handed to a local `fn(d:
Duration)` shadowing `Bucket` in another).

A **position the checker does not classify** refuses a value counted in
a denomination no declaration names, with a located error, and never
stores it as the wrong count: a generic literal's field whose type is
known only where the literal flows, a generic fn's argument (a
monomorph is named by declared types), two arms of an `if` or a `match`
at different denominations.

**Lowering** reads each row from the body being emitted and decides
nothing: a widening is one multiplication by `p` (a point's shift one
addition); a narrowing one division by `q`, then for a rounding the
one correction its policy is (none for `trunc`; down on a negative
remainder for `floor`, up on a positive one for `ceil`; away from zero
at half or more for `half_up`; past half, or at half to an even
quotient, for `half_even`), or for a checked discharge the remainder's
test, the quotient on one path and the `InexactError` on the other,
joined by the `or`; a range's narrowing its two comparisons and, by its
policy, two selects (`clamp`), an unsigned remainder of the distance
from `low` (`wrap`), or the value and the `RangeError` the `or`'s join
takes. A total conversion and an identity's or range's widening emit
nothing. A call with no row is no conversion, whatever its callee is
named. An arithmetic operator over a quantity or a point has its row,
its result's type, which says the representation the operation is
emitted in (`Int * Duration` a `Duration`). A row is never read from
another body (a stdlib body's spans overlap the first file's). A
literal with no row is a missing required row, refused where it is
written, save in a body the checker types no row in (the stdlib's own),
where a time literal is its count from the stdlib's catalogue.

## Synthesized denominations

An expression's quantity or point type carries its denomination, and
the denomination need not be one a declaration pinned: `d.in(s)` is
`Duration` in seconds, a sum is at its operands' meet, a ratio product
at the product of its operands' denominations. Such a type is
**synthesized**, as a monomorph is: the same kind, keyed by its
quantity (a point's: its frame, the nearest point on its chain stating
an origin, else the point declared with the word) and its
denomination, and named `Duration in s`, `Money in 1/10000 cent`,
`Duration in 100 ms`. The spelling is the first unit the denomination
is one of, else the unit it is the smallest whole multiple of, else a
fraction of its quantity's unit. A synthesized type has no policy and
no range, and is an ordinary type for a `let`, an argument or a return;
at its own quantity's denomination it is that quantity. It converts
like any other: freely into a finer denomination, into a coarser one
only under a policy.

```hale,fragment
let odd = 1_234_567USD * 3bp;   // Money in 1/10000 cent: 370370100, exact
let paid: Ledger = odd;         // the one narrowing: q = 10,000, half_even from Ledger
```

Each further ratio factor makes the denomination finer and the
headroom of its `Int` smaller (§ The witness report): a `Money in
1/10000 cent` holds 922,337,203,685,477 cents, a ten-thousandth of what
a `Money` holds.

## Points

A **point** is a position on its quantity's line, a quantity a
distance: two points of one origin differ by their quantity, a point
moves by a quantity, and two points never add. A point's zero is its
quantity's unless it states an `origin:` (where its zero sits above the
quantity's, as a count of its own denomination); a point and its
refinements share their frame. A conversion between two points of
different origins shifts by the difference, `x_to = (x_from +
o_from)·f − o_to`:

```hale,fragment
let boil = Celsius(100_000mK);
let k = Kelvin(boil);          // 373150 mK
let back = Celsius(k);         // boil
let rise = boil - Celsius(0mK);   // 100000mK, a TempDelta
```

`Time` is a point over `Duration` with no origin: the runtime's epoch
is its zero, and `WireStamp`, a refinement in `us`, shares that frame.

## Printing

A quantity **prints** as its count and its denomination: the unit when
the denomination is one of a unit (`37037cent`, `3tick`, `3s`), else its
type (`35 Bucket`, `370370100 Money in 1/10000 cent`). A point prints
as its count (`373150`); `Time` as its instant. Printing in another
unit is `.in(u)` first: printing is formatting, a projection to
`String`. An identity or a range prints as its `Int`. A quantity inside
a record or a sequence prints as its count. The stdlib's `Duration`
prints as it always has, its nanoseconds (`1500000000ns`): decision 8,
its class's own rendering; `d.in(ms) or floor` prints `1500ms`.

## Layout and the wire

Wherever a type reaches a representation an identity, a range, a
quantity or a point is its `Int`: codegen's `i64`, a hashmap key and a
routing key, a flat payload's field, an array element, a generic
argument whose monomorph lays it out as an `Int` (`Box<OrderId>`), an
FFI `Int`. The stdlib's `Duration` and `Time` are their own class
there, as they always were.

On the **wire** (decision 9) a field is its integer. A topic's shape
string tags an identity's and a range's field `i`, as an `Int`'s, so
declaring one moves no shape hash. A quantity's or a point's field is
tagged by its denomination: `q(` the denomination as its nearest
declaration writes it `)` (`q(cent)`, `q(100 ms)`), a point's adding
`point` and its origin when one is written (`q(mK point 273150 mK)`).
Two processes whose fields count in different denominations disagree
in the shape hash the observer protocol already compares. A program
with no quantity renders every shape as before, and `Duration` and
`Time` keep `u` and `t`.

```hale,fragment
type Fill { order: OrderId; notional: Money; fee: Ledger; }
topic Fills { payload: Fill; subject: "desk.fills"; }
// shape: order:i;notional:q(cent);fee:q(cent)
```

**Overflow** follows `Int`'s arithmetic (decision 7): a widening's
multiplication is the `Int` multiplication. A factor no `Int` holds is
an error at the conversion when the program is built ("…by the factor
1000000000000000000000000, which no \`Int\` holds"), never a wrap; a
literal's converted count that overflows is refused by the check. A
quantity's `range:` is a row the witness report reads (what it fits
in); no conversion checks it at v1.

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

Its rows are the first of every program's, in the stdlib's universe. A
type position reads `Duration` and `Time` as these declarations, whose
scalar rows name the primitive each is (`ScalarRow::primitive`).

**What a `Duration` keeps from before.** Its representation class: an
i64 count of nanoseconds (a `Time` since the epoch the runtime
defines), printed as it always was (`1500000000ns`, an ISO-8601
instant), a topic field tagged `u` and `t` (no shape hash, payload hash
or observer hash moves), its own FFI class, the runtime's nanosecond
entry points, and every `std::time` signature.

**What the declarations change** is who decides. A time literal is a
quantity literal of one of these units (`500ms` is 500,000,000 of
`Duration`, the catalogue's factor; the lexer knows no unit); the
algebra types their arithmetic (`Duration ± Duration`, `Int *
Duration`, `Duration / Int`, `Time ± Duration`, `Duration + Time`,
`Time - Time`, the comparisons; everything else is refused naming
both), and lowering emits each operator as its row says; `.in(u)`,
`.split(u)`, `Duration(x)` and `Time(x)` are the dialect's conversions.
Two quotients changed (decision 5): `Duration / Duration` is an `Int`
(`1h / 1min` is 60), and a `Duration` divided by an integer literal
other than one is a narrowing (`timeout / 2 or floor`); a runtime
divisor is the integer division it always was. The names are `ns us
ms s min h day` (decision 4): `5m` is refused ("\`5m\`: no \`unit\`
declares \`m\`; minutes are \`min\`"), `3d` is the Decimal `3` and no
unit may be named `d` (law 11), and `1h30m` is refused as one literal
of several units. A program's unit joins the component by an equation
against one of these (`unit tick = 10 ms;`), never by taking one's
name (law 1), and a program's quantity over the component is a
boundary denomination of `Duration` (`type Bucket = quantity Int in
100ms { round: floor; }`).

## The witness report

`hale check --units` (spec/projects.md) prints, on stdout, what the
rows and the conversions column say about a program, and re-checks
nothing (`hale_types::unit_report`). It is a development report,
outside the hashed model half: no shape hash reads it.

- **Per scalar declaration of the program**, in program order: its
  kind (and its quantity, or a range's parent); its denomination and
  the declaration whose `in` fixed it (the stdlib's `Duration` named by
  its stdlib file and line); its policy and the declaration that writes
  it; a point's origin; its range; the **headroom** of an `Int` at its
  denomination, `Int`'s range counted in the unit the denomination is
  written against and in the coarsest unit of its component that still
  counts one; and what a declared range **fits in**, the narrowest of
  `u8`, `u16`, `u32`, `u64` that holds it (`i64` for a negative bound).
- **Per narrowing the check recorded**, by site: its source and target
  types, its factor or its range, a literal's count converted at
  compile time, the policy that discharged it and where the policy came
  from (`at the site`, or the `round:` of a named declaration), and the
  headroom of a source whose denomination no declaration names. The
  evaluations of one default are one line.

The text is stable (declarations in program order, narrowings by
site), so it can be recorded and diffed; `--json` prints the same
fields as one object, a count as a string. With nothing to report it
is the one line `units: no quantity is declared`. From the committed
form's report:

```text
committed_form.hl:52:1  type Bucket = quantity Int in 100ms { round: floor; }
    kind         : quantity, a denomination of Duration
    denomination : 100 ms, fixed by its own `in`
    policy       : floor, from its own `round:`
    headroom     : ±922337203685477580700 ms (10675199116730 day)

committed_form.hl:81:24  odd
    Money in 1/10000 cent -> Ledger, factor 1/10000
    policy       : half_even, the `round:` of `type Ledger = quantity Int in cent { round: half_even; }` (committed_form.hl:53:1)
    headroom     : ±922337203685477 cent (9223372036854 USD), of Money in 1/10000 cent
```

## What the dialect does not do

At v1, by the committed form's deferral or the plan's own decision:

- **Derived dimensions.** `Bytes / Duration` is no rate type: a
  quotient of two quantities is a count only within one quantity, and
  `quantity / quantity -> Int` covers throughput arithmetic until
  exponent vectors arrive.
- **`Float` quantities** and a physics catalogue: a quantity counts an
  `Int` (law 5); an irrational edge is lossy by definition.
- **Calendars.** `day` is 24 hours; months, years and time zones are
  projections, not units.
- **Packing by width.** The report says what a range fits in; no layout
  reads it, and every scalar is an `i64`.
- **Rounding functions declared by a program**: a handler after `or` is
  already a program's own function. **A JSON form for a quantity**:
  `Duration` has none either.

## The catalogue

`hale_types::unit_graph` closes a catalogue of resolved unit identities
and positive rational equations; it is the declaration layer's
arithmetic core, and the unit rows close a program's through it.

**Identities and equations.** A node is a unit declaration's identity,
a `SiteRef`: the universe that minted the declaration and its `SiteId`
there (`unit_graph::UnitId`). An equation has its own declaration
identity and states that one unit at `from` equals `p/q` units at `to`.
Display names and source spans do not participate in identity.
Duplicate identities and references to undeclared nodes are errors. An
isolated declared node is a valid component. Factors are positive,
reduced, arbitrary-precision rationals; zero and negative factors are
rejected when a ratio is constructed. Point origins are not
multiplicative unit equations.

**A program's catalogue.** The unit rows close one catalogue over the
stdlib's time catalogue and the program's units. The stdlib's analysis
copy numbers its sites on its own, from seed 0 and index 0 as the
snapshot does, so a stdlib unit and a program's can share a `SiteId`;
their universes keep them two nodes, and the stdlib's catalogue and a
program's are disjoint components unless the program writes an
equation against a stdlib unit. The number one is one more node, under
an identity no mint issues (seed `u32::MAX` in the program's universe),
so the API knows only unit identities and no declaration can collide
with it.

**Closure and conversion.** `UnitGraph::close` proves every cycle's
product is one; failure returns errors and no graph, each inconsistency
carrying the claimed and implied ratios and a closed sequence of
equation identities and directions whose product is not one. Each
connected component uses a traversal root internally, with no semantic
status: changing identity or declaration order cannot change any
conversion ratio. `conversion(from, to)` returns the reduced factor and
a path of equation witnesses; unknown nodes and nodes in different
components have none. A factor with denominator one is an exact
denomination conversion; a greater denominator requires a named loss
decision from its consumer, and the catalogue chooses no rounding
policy. Exactness does not prove that a result fits a range or a
machine width.

**Denomination meet.** `meet(inputs)` computes the coarsest
denomination into which every input converts by an integer factor: the
gcd of the numerators over the lcm of the denominators. A declared node
need not spell it: inputs with scales `3/2` and `5/2` meet at `1/2`. The
result retains an irredundant set of input positions as its
explanation, earlier inputs preferred; there is no fixed two-input
bound (the meet of `6`, `10` and `15` is `1`, and no pair proves it).
An empty input set, an unknown input, or inputs in different
components produce no denomination. A boundary's pinned denomination is
queried separately with `factor_to(target)`, which exposes any
narrowing its consumer must address.

**Denominations as values.** A `Denom` is a declared unit and an exact
positive multiple of it (`100 ms` is `{ ms, 100 }`), stored and
compared without the catalogue; two values may denote one denomination
(`{ ms, 1 }` and `{ us, 1000 }`), which only the catalogue tells:
`factor(from, to)` is how many `to` one `from` is, exact, and none
across components. A meet is a value too (`Denomination::value`, stated
against its first input's unit). A factor reaches machine integers
only through `Ratio::to_machine`: two `i64`s, or `FactorOverflow`
carrying the factor when either part does not fit. Nothing truncates.
