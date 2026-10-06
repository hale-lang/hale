# Exact unit catalogue

The compiler's `hale_types::unit_graph` API closes a catalogue of
resolved unit identities and positive rational equations. It is the
declaration-layer arithmetic core for GH #1076 / #1212. A program's
`unit` declarations are closed into one through it, and its scalar
declarations are judged against it (§ Declarations). Expression typing
types the values of identities and ranges (§ Identities and ranges) and
does not consume the catalogue yet; the existing `Time` and `Duration`
primitive behavior is unchanged.

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
`3bp` or `1_250_000USD` (spec/tokens.md § Quantity literals).

### The rows

The declarations are rows of one family, `unit_declarations`
(`hale_types::units::derive_unit_rows`, spec/registry.md), derived
once per snapshot from the programs after the desugar sequence, each
row keyed by its declaration's site. The stdlib declares no unit and
no scalar yet.

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
   witness is the first.

   ```hale,fragment
   unit cent;
   unit cent;      // error: unit `cent` is declared twice
   ```

2. **A named unit is declared.** An equation's target, a
   denomination and an origin name a declared unit; the error is at
   the name, suggesting the nearest declared one.

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

10. **No unit takes a duration suffix's name** (`ns us ms s m h d`):
    the lexer reads `5ms` as a `Duration` literal whatever is
    declared, until `Time` and `Duration` are declarations of the
    time catalogue.

    ```hale,fragment
    unit ms;   // error: `ms` is a built-in duration suffix … name it otherwise (`msec`)
    ```

11. **The dimensionless component.** A unit defined against a number
    is in the one component the number one belongs to, with every
    other such unit; laws 3 and 4 judge it as any component (so
    `quantity Int in bp` and `quantity Int in pct` are two quantities
    of it, and the second is an error).

    ```hale,fragment
    unit bp = 1/10000;
    unit pct = 1/100;   // one component with `bp`: one `pct` is 100 `bp`
    ```

### Quantity and point values arrive with the next step

An identity's and a range's values are typed (§ Identities and
ranges). A quantity's and a point's are not yet: the name of one where
a value would live (a parameter, a field, a `let` annotation, a return
type, a topic's payload, a generic argument, an alias's target, a cast
`Money(5)`) and a quantity literal in an expression are one located
error each ("values of the unit dialect's types are not typed yet
(GH #1076)"), so a program that passes the check holds no value of a
quantity or a point, and their declarations lower to no code. A
declaration the laws refuse has no values either: its name resolves to
nothing, so a use of it is no second error. A cast's name means what it means
at the call: a local, a parameter or a fn of the name is the callee,
and no cast. A struct field's default is evaluated at each literal that
leaves the field, in that literal's scope, so a cast in it is judged
there (once, at the default, however many literals leave it), with the
scopes the default opens, the parameter defaults its calls leave and
the field defaults its own literals leave; a quantity literal in it is
refused at the declaration. Quantity values, literals of declared units
and the conversions between denominations arrive with the next step.

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
is classified per evaluation: its casts' rows are recorded in the typed
body of the declaration that evaluates it, by the evaluation and the
cast, and lowering reads the row of the evaluation it lowers. One
default can be a conversion in one scope and a call of a local in
another:

```hale,fragment
type S { n: ItemId = ItemId(1); }
let a = S {};              // `ItemId(1)`: the conversion
{
    let ItemId = bump;
    let b = S {};          // `ItemId(1)`: the call `bump(1)`
}
```

**Layout.** Wherever a type reaches a representation, an identity or a
range is its `Int`: it prints as its `Int`, it is a legal hashmap key
and routing key, a flat payload's field, an FFI `Int`, a generic
argument whose monomorph lays it out as an `Int` (`Box<OrderId>`) and
an array element, and a topic's
shape string tags its field `i`, as an `Int` field's (decision 9), so
declaring one moves no shape hash.

## Identities and equations

A node is a snapshot `SiteId`. An equation has its own declaration
`SiteId` and states that one unit at `from` equals `p/q` units at `to`.
Display names and source spans do not participate in identity.
Duplicate identities and references to undeclared nodes are errors.
An isolated declared node is a valid component.

Factors are positive, reduced, arbitrary-precision rationals. Zero
and negative factors are rejected when a ratio is constructed. Point
origins are not multiplicative unit equations.

**A program's catalogue.** The unit rows close their catalogue over
each unit declaration's `SiteId` and each equation's. The number one
(`unit pct = 1/100;`) is one more node, under an identity the
snapshot's mint never issues (seed `u32::MAX`; seeds are numbered
from zero, one per seed). That keeps the choice inside the rows: the
API knows only `SiteId`s and has no notion of a pure number, and no
declaration can collide with it. A stdlib catalogue would need
identities apart from the program's, since the stdlib's analysis
copy numbers its sites on its own; the stdlib declares no unit yet.

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
