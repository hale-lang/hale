# Perspectives and projection classes

This chapter covers two related but distinct substrate
mechanisms that share a common framing — both are about *the
same value, viewed in different ways*:

1. **Projection classes** (`Rich`, `Chunked`, `Recognition`) —
   per-locus allocator strategies, each appropriate to a
   different population shape. The compiler picks one, and the
   runtime arena behavior changes accordingly. Same source code,
   different memory mechanics.
2. **Perspectives** — a serializable parameter bundle that
   crosses between an analyst (which fits the bundle's values
   from data) and an executor (which applies them in production).

The two mechanisms hook into different parts of the substrate —
projection classes sit at the allocation layer; perspectives sit
at the cross-process / cross-locus communication layer — but both
embody the substrate's commitment that **the same conceptual
value can have multiple legitimate representations**, and that
the language should make that explicit rather than hidden.

## Projection classes

A locus's *projection class* is an annotation on its declaration
that selects how its arena and the arenas of its accepted
children behave:

```aperio
locus RichCoord : projection rich {
    accept(c: Leaf) { }
    run() {
        let _l1 = Leaf { value: 1 };
        let _l2 = Leaf { value: 2 };
        let _l3 = Leaf { value: 3 };
    }
}

locus ChunkedCoord : projection chunked { /* ... */ }
locus RecognitionCoord : projection recognition { /* ... */ }
```

Three classes, with three corresponding behaviors at the arena
level:

- **`projection rich`** — every child gets an independent
  arena, fully sized for its own state. Suitable when each child
  carries substantial state and the population is small. Per
  child, the runtime calls `lotus_arena_create` separately.
- **`projection chunked`** — the parent carves a contiguous
  subregion out of its own arena for each accepted child, with
  free-list slot reuse on dissolve. Suitable for medium-to-
  large populations of similarly-sized children. The runtime
  calls `lotus_arena_create_subregion`.
- **`projection recognition`** — same allocation path as
  chunked in v0, with a documented stub for a future bitmap-
  pool optimization. Suitable for very large populations where
  the population size dominates the per-child overhead.

All three are observably equivalent at the language surface —
the `14-projection-classes` example exercises all three with
identical loci accepting identical children, and they print
identical output. The difference is in *how* the population's
storage is laid out, not *what* the loci do.

### Default projection class

If a locus that declares `accept` does not annotate its
projection class, the default is `chunked` if the compiler
cannot statically determine the child population size N. (The
spec's design rationale flags this: the framework's discipline
permits inference, so an explicit annotation is optional.)

For programs without parent-child relationships, the projection
class is irrelevant and unused — there is no allocation strategy
choice to make.

### `ProjectionClass` as a generic constraint (F.2)

The three classes are also language-native generic
constructors:

```aperio
fn process<P: ProjectionClass, T>(input: P<T>) -> P<T> {
    // ... operates on P<T> regardless of whether P is
    // Rich, Chunked, or Recognition.
}
```

`P: ProjectionClass` is a built-in *any-of-three* constraint,
per **F.2**. `P` resolves to one of `Rich`, `Chunked`, or
`Recognition` at each call site, and the compiler emits one
specialization per resolution. There is no trait system
underneath — `ProjectionClass` is a recognized name in the
constraint position, not a user-definable concept.

### Multi-implementation contract fields

[Chapter 5](./05-contracts-and-parents.md) introduced contracts
and noted that **F.14** permits *multiple implementations of
the same contract field*. Projection classes are how multi-
implementation typically materializes: a `volume` field with a
`rich` translation that walks every child, a `chunked`
translation that reads from a shared aggregate, and a
`recognition` translation that reads from a population summary.
All three return the same contract type; the parent asks for
whichever it wants, and the locus dispatches.

The annotation syntax for multi-implementation
(`@projection rich fn volume() -> Decimal`) is deferred to a
post-v1 milestone; for v0, contract fields default to "the
param is the implementation," and projection classes affect
allocator strategy only. The full multi-implementation surface
lands when an example forces it.

## Perspectives

A *perspective* is a serializable bundle of parameters that moves
between two loci, ships only when its commit predicate is
satisfied, and presents one face on the wire while carrying
additional state internally. Perspectives are the foundational
interop mechanism between Aperio apps that need shared-but-
evolving state — chat sessions, fitted parameter bundles, schema-
versioned events.

The [chat-fanout capstone](./13-chat-fanout.md) uses a
`SessionPerspective` to ship `Session` records from the server
to clients only once authentication has completed. A second
common shape is *fitter/applier*: one locus fits a parameter
bundle from observed data, another applies the bundle at
production speed. The example below shows that shape.

```aperio
perspective KernelPerspective {
    params {
        kernel: Kernel;
        validation_count: Int = 0;
    }

    stable_when {
        // commit predicate: this perspective ships only after
        // N validations agree.
        return self.validation_count >= 3;
    }

    serialize_as Kernel;
}
```

A perspective has three parts:

- **A parameter bundle** — the values that travel together, of
  any wire-serializable types.
- **A `stable_when { ... }` block** — a commit predicate. The
  fitting locus may hold multiple candidate perspectives in
  flight; only those that satisfy `stable_when` are eligible
  for shipping.
- **An optional `serialize_as TypeV1` annotation** — the
  schema-evolution mechanism (open-question #13). When the
  perspective's field set changes, a `serialize_as` annotation
  preserves wire compatibility with older binaries during
  rolling deployments. v0 does not yet implement
  `serialize_as`; rolling deployment with mismatched schemas
  is not supported.

### Multi-perspective stability

The analyst pattern is to fit *several* candidate perspectives
in parallel — different parameter bundles consistent with
different observation windows or different fitting strategies —
and ship the one(s) that satisfy `stable_when` (typically:
"at least three independent fittings agree"). The executor sees
only the shipped perspective; the in-flight candidates are
internal to the analyst.

This is an architectural pattern, not a syntactic feature —
the language gives you the perspective declaration and the
commit predicate; the multi-perspective fitting is loci you
write to inhabit them.

> **Status.** Perspectives are declared and consumable today,
> but the full multi-perspective stability machinery — running
> N fittings in parallel, deduplicating equivalent ones,
> tracking validation count — is application-level code in v0.
> Substrate support for the pattern is a v1.x roadmap item.

### Why a separate construct, not a `type`?

A perspective looks superficially like a `type` declaration,
but it is a distinct kind of value with two specific
differences:

- **A perspective carries a commit predicate.** A `type` is
  inert data; a perspective declares the rule under which its
  values are eligible to ship. The rule lives with the
  declaration.
- **A perspective is the canonical wire format for fitted
  values.** When the schema evolves, `serialize_as TypeV1`
  knows it is operating on a *perspective*, not an arbitrary
  struct, and applies the perspective-versioning rules.

For values that are not analyst→executor parameter bundles, a
plain `type` is the right choice. Reach for `perspective` when
the value is fitted, has a commit predicate, and crosses
between fitting and applying loci.

## What this chapter does not cover

- **`@projection <class>` annotations on contract fns** — the
  multi-implementation contract surface — deferred to post-v1.
  v0 has the typing rule (a fn satisfying a contract field
  must return the contract's type) but not the dispatch
  syntax.
- **`serialize_as TypeV1`** — the perspective-versioning
  mechanism. Declared in the spec; implemented when the first
  rolling-deployment workload requires it.
- **The full multi-perspective fitting pattern** — how an
  analyst tracks several candidate perspectives, deduplicates
  them, and applies `stable_when`. This is *application-level*
  code in v0; substrate helpers will land in v1.x.

The next chapter, **[Recovery and
supervision](./12-recovery-and-supervision.md)**, returns to
the failure-handling surface introduced in
[chapter 7](./07-closures.md). It covers the four recovery
primitives (`restart`, `restart_in_place`, `quarantine`,
`bubble`) deeply, the `evaluate(closure_name)` explicit
trigger, and the parent's role as the supervision authority
for its descendants.
