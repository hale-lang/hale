# Audit checklist

A practical checklist for verifying that a stateful Aperio app is
MOA-shaped. Use it before merging significant changes, during code
review, or when onboarding to a codebase you didn't write. Each
section maps to one of the four properties in `properties.md`; if
all four pass, the app is structurally MOA.

The checklist is ordered for a single read-through of the app's
source. It expects ~30 minutes for a 1,000-line app; less if the
app is already familiar.

## 1 — Identify the memory-owners

Walk the locus declarations in source order. For each `locus L { ... }`:

- [ ] Does L declare a `capacity { ... }` block, or does it hold
      state in its `params { ... }` block (the v0 fallback for
      F.22 features that haven't shipped)?
- [ ] Does L declare `bus { publish ... }` for one or more subject
      families?
- [ ] Does L have non-trivial state — counters, accumulators,
      caches, projections, recordings, registries?

If any of those is **yes**, L is a memory-owner. If all three are
**no**, L is either an orchestrator or a pure-leaf helper locus.

Output: a list of memory-owners with their declared concern. Save
this list; you'll use it in section 3.

## 2 — Verify the orchestrators

For each non-memory-owner locus from section 1:

- [ ] Does it route bus events to its memory-owning children (via
      method calls or by republishing on different subjects)?
- [ ] Does it manage the lifecycle of its memory-owning children
      (instantiate at birth; let them drain via scope-exit)?
- [ ] Does it carry NO state of its own beyond config that's set
      at birth and read but not mutated?

If any of those is **no** for a locus you classified as an
orchestrator, it's actually a memory-owner in disguise. Promote it
(rename, document, declare capacity); or refactor its state into a
new memory-owning child.

## 3 — Verify capacity + ingest discipline on memory-owners

For each memory-owner from section 1:

- [ ] Does it declare its storage shape — either `capacity { pool
      X of T; heap Y of T; }` or, at v0, fixed-cap params arrays
      with the migration documented inline?
- [ ] Does every `subscribe` line in its `bus` block carry a
      `/// ingest: discard | save | transform — <rationale>`
      doc-comment immediately above it?
- [ ] Does the implementation match the classification? (A handler
      classified `save` should only append to a slot or array;
      a handler classified `transform` should mutate derived
      state; a handler classified `discard` should be a no-op or
      filter.)
- [ ] Is the locus a recording (saves only) or a projection
      (transforms only) or both (mixed)? If mixed, is the mix
      documented in the locus's header comment so a reader knows
      which subjects feed which side?

If any ingest is unclassified, or the classification doesn't match
the implementation, that's a regression — fix it before merge.

## 4 — Verify the bus topology

For each unique subject family in the app's `bus` blocks
(`<concern>.<shape>.*`):

- [ ] Does exactly **one memory-owner** publish on it? (Per the
      one-publisher-per-family rule.)
- [ ] Do the subscribers reference it by name only — not via
      computed strings, not via runtime-resolved patterns (m94 `**`
      wildcards on the subscribe side are fine; m94 on the publish
      side is the family-authorization mechanism)?
- [ ] Is the payload type a copy-safe value type (primitive,
      String, Bytes, nested user struct — not a `LocusRef`, not a
      cell handle)?

If two memory-owners publish on the same family, **split the
family**: give each a distinct name. If a subscriber computes
subjects dynamically, **restructure**: dynamic-subject routing is a
sign that you actually have routing-as-state and need a dispatcher
memory-owner.

## 5 — Verify no cross-concern pointers

Static scan for forbidden inter-locus access patterns:

- [ ] No locus reads another locus's field directly across a
      sibling boundary. (Vertical contract reads are fine —
      parent reads child.expose_field; child reads
      parent.consume_field. Sibling reads are forbidden.)
- [ ] No bus payload contains a `LocusRef` or a capacity cell
      handle. (Both are caught by the typechecker, but a manual
      scan catches code that was written before the check
      tightened.)
- [ ] No memory-owner's data flows out except via its declared
      `publish` subjects.

If the typechecker catches these, great. If you find one the
typechecker missed, file a friction entry — the substrate's
vertical-only-flow rule should be compile-time enforced where
possible.

## 6 — Verify boundary disciplines (broadcast vs private)

For each request/response interaction in the app:

- [ ] If the response is broadcast (per-recipient is unnecessary),
      does the request channel sit at `<concern>.request.<verb>`
      with the response on `<concern>.<noun>.*` (e.g.
      `<concern>.snapshot.*` for snapshot ping)?
- [ ] If the response is private (per-recipient is necessary), is
      the carve-out reason documented? One of: privacy/auth,
      volume, per-client owner state. Otherwise, broadcast is the
      default.

See `patterns/broadcast-snapshot.md` and `patterns/private-streams.md`.

## 7 — Verify orchestrator config-loading

For the app's top-level orchestrator (`main()` or its equivalent
outer locus):

- [ ] If config parsing is non-trivial (multiple flags, env, files,
      validation), is it factored into a separate config
      memory-owner (`std::cli::Resolver` or a domain Resolver)?
- [ ] Is `main()` reading from the config memory-owner, not from
      argv directly?
- [ ] Does the config memory-owner expose typed accessors or a
      contract surface — not raw string lookups at every call site?

See `patterns/config-loader.md`.

## What to do when the checklist fails

A failed check is a friction signal, not an emergency. Three escalation
paths, in order:

1. **Fix in place.** Most failures are small — an unclassified
   ingest comment, a stateful orchestrator that should be a
   memory-owner, a missing capacity declaration. Document the fix
   in the commit message.
2. **Refactor for one PR.** If the violation requires moving state
   between loci or splitting a subject family, scope it as its own
   PR with a "what this changes" preamble.
3. **File a friction entry.** If the violation reveals a v0
   substrate gap (e.g. you genuinely need cross-locus method
   access that doesn't exist yet), log it in
   `notes/aperio-friction.md` with the smallest reproducer.
   Don't paper over substrate gaps with non-MOA shape — the
   discipline matters most where it's hardest.

## Audit complete

If all six sections pass, the app is MOA-shaped. The next reviewer
who reads it cold should be able to identify each memory-owner's
concern, each ingest's discipline, and each subject family's
canonical publisher within the first read. That's the audit
surface MOA exists to create.

## Cross-references

- `properties.md` — the four properties the checklist validates
- `patterns/broadcast-snapshot.md` and `patterns/private-streams.md`
  — the boundary disciplines section 6 references
- `patterns/config-loader.md` — the orchestrator config pattern
  section 7 references
- `apps/market-book/` (in the repo) — a worked example to audit
  against as practice
