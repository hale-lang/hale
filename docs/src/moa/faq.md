# FAQ

Common questions about MOA, answered as tightly as possible. If a
question keeps coming up that isn't here, add it.

## Do I need MOA for a tiny app?

If the app holds **no state across events** — a converter, a
formatter, a one-shot tool — MOA doesn't apply. Stick with the
six patterns in `spec/styleguide.md`.

If the app holds **any state** across more than one bus event or
across more than one fn call — even a counter, even a cache —
MOA's overhead is small and the audit surface it creates is worth
it. The minimum viable MOA app is one orchestrator + one
memory-owner, which is a few lines more than the unprincipled
version.

The cutover threshold is "do I have ongoing state?", not "is the
app big?"

## How is this different from CQRS / event sourcing / actor model?

Each names one face of the same shape:

- **CQRS** separates write side from read side. MOA does this
  per concern — each memory-owner is its own CQRS unit — and the
  read side is delta-sourced rather than table-projected.
- **Event sourcing** makes deltas canonical. MOA does, with the
  runtime bus as transport rather than a persisted log.
- **Actor model** has message-passing-as-coordination. MOA has it
  *hierarchically* with region semantics and capacity discipline.

MOA's distinctive composition is *hierarchical-region-disciplined-
actor-with-typed-delta-bus*. No piece is new; the combination
forces choices the pieces individually don't. See `glossary.md`
and `properties.md` for the architectural framing.

## Why is the bus the only inter-concern channel?

Because the framework's substrate enforces it. Cross-arena
pointers are forbidden by the typechecker and by region-lifetime
rules. The bus is the only typed cross-locus communication
primitive, and its copy-at-boundary semantic means publishers and
subscribers have independent state lifetimes. Trying to route
around the bus produces compile errors or silent data corruption.

MOA names what the substrate makes coherent; it doesn't add
restrictions.

## Why is the broadcast pattern the default?

Four reasons, summarized from `patterns/broadcast-snapshot.md`:

1. One publisher per subject family stays *strict* — no
   per-recipient fan-out.
2. No correlation ids — clients don't need to identify themselves
   for routing.
3. All subscribers are symmetric — the system has no notion of
   "this delta is for client X."
4. Idempotent recovery — the snapshot IS the recovery seed.

Per-recipient response streams are the carve-out, justified only
by privacy, volume, or per-client owner state. When none of those
apply, broadcast is right.

## What about request/response if I really need correlation?

Use private streams (see `patterns/private-streams.md`), or layer
correlation on top of broadcast:

- The shared request channel carries a correlation id in its
  payload.
- The memory-owner publishes its response with the same
  correlation id in the response payload.
- Subscribers ignore responses that don't carry their id.

Both subscribers see all responses, but each filters by id
client-side. The bus is still broadcast; the client-side filter
adds the correlation semantic. Suitable when the correlation
need is light and volume doesn't justify per-recipient streams.

## Does every memory-owner need a capacity block at v1?

No. F.22's `capacity { pool/heap }` slots are a substrate
*choice* — they earn their place when storage discipline differs
from the default (the locus's arena freed wholesale at dissolve).
If your state fits in `params` and is mutated in place, that's a
valid v1 declaration of capacity discipline; the F.22 lift is a
future-proofing move, not a requirement.

The ingest classification doc-comment IS expected on every
subscription, however. That's what the styleguide section on MOA
asks for, and what `audit-checklist.md` verifies.

## What if my memory-owner has many subjects?

Then declare them all in one `bus { ... }` block with one ingest
classification per subscribe line. The line count is the
publisher's auditability footprint; high count is fine if each
classification is clear.

If you find yourself with 20+ subscriptions on one locus, that's a
signal the locus has too many concerns. Factor it into multiple
memory-owners — each with its own concern and its own subject
families — coordinated by an orchestrator that routes intents
between them.

## Can two apps share a memory-owner?

No — memory-owners are loci, and loci are app-local. What two
apps can share is the **subject family conventions** (so that one
app's `book.delta` publish reaches the other's subscribe) and the
**substrate types** (`moa::RuntimeEvent`, `moa::Tick`, etc.) that
they exchange on the bus.

Cross-app interop in MOA is bus-level, not locus-level. Two
MOA apps following `moa/subjects.md` conventions interoperate
without bespoke protocol code; neither owns the other's state.

## Why doesn't MOA enforce its rules at compile time?

Two reasons:

1. **The rules are graded.** "Is this orchestrator carrying
   state?" depends on what counts as state — a config field set
   at birth and read but not mutated is fine; a counter that
   advances per event is not. The line is reader-visible but not
   easy to encode in a type rule.
2. **Workload-driven.** Compile-time enforcement is a v2 idea on
   `roadmap.md`. v1 ships discipline + audit; if the discipline
   holds, compile-time enforcement is a tightening, not a new
   feature.

For now, `audit-checklist.md` is the enforcement surface. Run it
before merging significant changes.

## What's the minimum I should read?

`quickstart.md`. ~10 minutes. If your app then matches one of the
patterns mentioned there, you have your answer; if it doesn't,
read `properties.md` and the relevant pattern page.

## What if I find a case the docs don't cover?

File a friction entry. The discipline grows from real friction,
not from speculation. The minimum reproducer + the question makes
the doc tract able to update — see `notes/aperio-friction.md` for
the existing log shape.

## Cross-references

- `quickstart.md` — the one-page summary
- `properties.md` — the four-property statement
- `audit-checklist.md` — review-time verification
- `roadmap.md` — what's available vs pending
- `glossary.md` — terminology
