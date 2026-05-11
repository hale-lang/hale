# Anti-patterns

The shape these violate is *almost always* "I'm avoiding the
language's primitive in favor of an old habit from another
language."

## Bare `fn main()` with helpers and no outer locus

An app's outer encapsulation must be a locus, per the
apps-are-loci rule. A `fn main()` that calls a few free
functions with no `<Name>L` locus owning the run misses the
substrate's lifecycle and supervision machinery — the
*shape* of the rest of the program no longer recurses
upward to the top.

The fix: wrap the run in an `<Name>L` locus, give it
`params` for argv-derived configuration, put the work in
`run()`, keep `fn main()` to a few lines of argv parsing
plus a single statement-position locus literal.

## Coherent helper vocabulary stranded as `__free_fns`

When `__split_camel`, `__lookup_morpheme`, `__suffix_rule`,
and `__name_to_motion` accumulate at the top of a file, they
form a coherent vocabulary that wants extracting into a
[namespace lotus](./patterns.md#2-namespace-lotus--empty-params-only-methods).
The free fns hide the coherence; the lotus surfaces it.

The fix: lift the helpers into a locus with empty `params`,
then call them through a single instantiation.

## `type` for things that have flow

If a noun has a lifecycle implied (a Cache that's
loaded/probed/evicted; a Server that starts/serves/stops),
it is a locus. Declaring it as `type` strands the lifecycle
in client code that has to remember the load/probe/evict
discipline.

The fix: declare it as a locus, give it `birth`/`run`/
`dissolve` (or just `params` + methods if it's a namespace
lotus), let the substrate own the lifecycle.

## Methods on a `type` record

Not supported at v0 — the language tells you "you wanted a
locus." If you reach for a method on a shape, you're
discovering it wasn't a shape.

The fix: convert the `type` to a locus with empty `params`.
The cost is one allocation per instantiation. Negligible.

## Inappropriate `__` prefix

The leading `__` marks "don't call this across the seed
boundary." If the function IS the surface — the thing other
seeds (or this app's own code) calls — drop the prefix.

## "Util" namespaces of unrelated helpers

Group by *vocabulary*, not by "everything that didn't fit
elsewhere." A namespace lotus should answer to one question
("noun-to-motion", "tagged-accumulator parsing"), not many.
A `UtilsL` that holds string trimming and time formatting
and JSON parsing is three namespace lotuses pretending to
be one.

## See Also

- [Pattern catalog](./patterns.md) — the six shapes these
  anti-patterns are avoiding.
- [Rolling the design](./rolling.md) — the framework that
  catches these at design time, not at refactor time.
