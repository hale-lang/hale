# Rolling the design

The pattern catalog is small on purpose; new primitives must
**roll into** the existing seed, not sit beside it. Rolling
means two things at once:

- **Continuity in shape.** A new locus mirrors the shape of an
  existing locus — same params/methods/lifecycle pattern,
  different domain. A reader who knows one knows the new one
  at a glance.
- **Interlock in composition.** A new locus's outputs are
  valid inputs to existing primitives. The seed forms a graph:
  `Walk` feeds newline-separated Strings into `iter::Lines`;
  `Resolver` produces values that flow into struct literals
  into App loci; `Morpheme` consumes names produced by
  `Convention`. Each new primitive slots into that graph or it
  doesn't roll.

Both conditions matter. A primitive that mirrors an existing
shape but produces an isolated output is recognizable but
useless. A primitive that interlocks but invents a new shape
is useful but foreign.

## Good in the code AND good in the machine

The frame is the same lens applied twice:

- **In the code** (reader-side): patterns repeat, so the
  reader's mental model doesn't fragment per-feature.
  Recognizing `std::cli::Resolver` is one step from
  recognizing `std::lang::Morpheme` — same shape, different
  job. Cognitive load amortizes across the catalog instead of
  compounding.
- **In the machine** (composition-side): outputs interlock,
  so each primitive's results flow into the next without
  glue. The medium is shared (newline-separated String,
  tagged-row String, tree-sitter Int node, struct-literal
  value). A primitive that speaks a foreign medium forces
  every consumer to bridge — glue code grows quadratically
  with the number of primitives.

Both flow from the same root: the catalog stays small enough
to hold in head, and every member speaks the shared medium.
Break either and you've broken both — a primitive that needs
glue at runtime also needs explanation at read time.

## The test for a new primitive

When proposing one — a new namespace lotus, a new shape type,
a new free fn that wants to graduate — ask:

1. **Which existing pattern does this mirror?** Params +
   self-composing methods → namespace lotus per
   `std::lang::Morpheme`. Birth/run/dissolve + `on_X`
   callback → service locus per
   `std::io::tcp::Listener`. If nothing existing mirrors
   what you have, you might be inventing a category — pause.
2. **What consumes my output?** New outputs should slot into
   existing consumers without per-call adaptation. If
   nothing existing reads what you produce, you've created
   an island.
3. **Could a reader who knows the catalog recognize this
   immediately?** If your primitive needs a paragraph of
   "this works differently from the others", you're adding a
   category, not rolling one.

If all three answers are clean, you're rolling. If any is "I
don't know" or "this one is special", look at the catalog
again before proceeding.

## Worked examples

**Rolled.** `std::cli::Resolver` mirrors `std::lang::Morpheme`
exactly — namespace lotus, params hold per-instance config,
methods are self-composing pure queries. Its output is plain
String/Int that flows into a struct-literal Config and from
there into an App locus. Recognizable shape, interlocking
output.

**Rolled.** `std::source::Walk` mirrors
`std::io::tcp::Listener` — params plus an `on_X` callback, a
single drive method, the callback's returned String
concatenates into the walker's output. That output walks via
`std::iter::Lines` and `std::tagged::Accumulator`. Recognizable
shape, interlocking output.

**Didn't roll (rejected during design).** A flavor of
`Resolver` that held `defaults: "dir=foo;flavor=go"` as a
string-encoded mini-DSL inside the locus. Shape mirrored
nothing in the catalog; medium was foreign (`;`-delimited
`key=value` isn't part of the String conventions other seeds
speak). Picking the call-site-defaults form rolled the
existing shape forward instead.

## The negative formulation

When you find yourself reaching for a foreign pattern —
TOML/JSON inside a locus, fluent-builder chains that mutate
self, methods on a shape type, decorators, singletons in
disguise — stop. The right move is almost always to find the
existing seed shape that fits. The catalog is small not
because Aperio is missing features; it's small because each
new primitive has to earn its slot by rolling, not by sitting
beside.

## See Also

- [Pattern catalog](./patterns.md) — the six shapes new
  primitives have to mirror.
- [Composition](./composition.md) — the medium new primitives
  have to speak.
- [Anti-patterns](./anti-patterns.md) — concrete cases of
  primitives that didn't roll.
