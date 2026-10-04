# Exact unit catalogue

The compiler's `hale_types::unit_graph` API closes a catalogue of
resolved unit identities and positive rational equations. It is the
declaration-layer arithmetic core for GH #1076 / #1212. Source syntax
and expression typing do not yet consume this API; the existing
`Time` and `Duration` primitive behavior is unchanged.

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
