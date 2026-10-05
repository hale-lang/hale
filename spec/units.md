# Exact unit catalogue

The compiler's `hale_types::unit_graph` API closes a catalogue of
resolved unit identities and positive rational equations. It is the
declaration-layer arithmetic core for GH #1076 / #1212. The source
syntax of the declarations parses (§ Declarations); neither it nor
expression typing consumes this API yet, and the existing `Time` and
`Duration` primitive behavior is unchanged.

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

A program that declares a unit or a scalar type, or writes a
quantity literal, is refused by `hale check` with one located error
for each ("the unit dialect's declarations are parsed and not yet
checked (GH #1076)") until the dialect's declaration rows and laws
land.

## Identities and equations

A node is a snapshot `SiteId`. An equation has its own declaration
`SiteId` and states that one unit at `from` equals `p/q` units at `to`.
Display names and source spans do not participate in identity.
Duplicate identities and references to undeclared nodes are errors.
An isolated declared node is a valid component.

Factors are positive, reduced, arbitrary-precision rationals. Zero
and negative factors are rejected when a ratio is constructed. Point
origins are not multiplicative unit equations.

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
