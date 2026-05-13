# Glossary

Terms specific to Memory-Owner Architecture, plus framework terms
used in load-bearing ways. Cross-references to `spec/`, `notes/`,
and the broader Aperio docs are noted where one-sentence
definitions aren't enough.

## A

**Aperio** — the language. Distinct from *lotus* (the runtime
substrate). MOA is an architectural pattern in Aperio; the
substrate it depends on is implemented in Aperio + C.

**Arena** — a locus's slot-0 storage, freed wholesale at locus
dissolve. Every locus has one implicitly; F.22 slots 1..N are
additional storage with the same lifetime but different
allocation discipline (pool / heap). See
`spec/memory.md` §F.22.

**Audit** — the practice of verifying an app is MOA-shaped by
inspection. See `audit-checklist.md`.

## B

**Braid** — colloquial for a bus subscription connection between a
publisher locus and a subscriber locus. The `moa::BraidId` payload
type names a specific braid for cross-app introspection. The
metaphor: the locus tower has vertical stems (parent-child
contracts) and lateral braids (bus subscriptions) running through
it.

**Broadcast pattern** — the default request/response shape under
MOA: public delta + snapshot streams, on-demand snapshot ping. See
`patterns/broadcast-snapshot.md`.

**Bus** — the typed lateral message bus. Every cross-concern
interaction in an MOA app routes through it. Each subscription is
a `subscribe SUBJECT as HANDLER of type T`; each publish is a
`SUBJECT <- VALUE`. See `spec/runtime.md` and
`docs/src/book/06-the-bus.md`.

## C

**Capacity** — one of the two disciplines an MOA memory-owner
declares. The `capacity { pool X of T; heap Y of T; }` block names
the F.22 slots that hold the locus's state. Paired with **ingest**.
See `properties.md` property #2.

**Cell** — the value type returned by `acquire()` / `alloc()` on a
capacity slot. `Cell<T>` is the typed handle to a slot cell; at v1
it carries the cell's T-typed memory. v1.x adds richer access.
See `spec/types.md`.

**Closure** — a locus-attached audit assertion (`closure NAME {
LEFT ~~ RIGHT within TOL; }`). Fires at epoch boundaries; on
violation, emits a typed `ClosureViolation` event up the locus
tower. The framework's audit primitive; MOA apps use closures to
verify recording completeness and projection consistency. See
`spec/runtime.md` and `docs/src/reference/closures/`.

**Concern** — what a memory-owner is the source of truth for. One
memory-owner per concern; one concern per memory-owner. The unit
of MOA decomposition.

**Config-loading orchestrator** — the pattern where argv parsing
is factored into a small memory-owner (typically
`std::cli::Resolver`) that the top-level orchestrator reads at
startup. Keeps `main()` stateless. See `patterns/config-loader.md`.

## D

**Delta** — an incremental update to a memory-owner's state,
published as a typed bus message. The output channel of every
memory-owner. MOA's communication primitive — "publish deltas,
not state."

**Dispatcher** — a memory-owner whose state is a routing table.
Earns its place when the routing decision itself is state
(membership-based, policy-based, lookup-based). Distinct from a
*router*, which is what the bus already is. Pattern documented
as a deferred entry; lands when a concrete workload forces it.

## I

**Ingest** — one of the two disciplines an MOA memory-owner
declares. Per-subscription classification: `discard | save |
transform`. Captured as a doc-comment above the `subscribe` line
at v1; future grammar surface at v1.x. See `properties.md`.

## L

**Lateral** — the bus direction. Cross-concern; cross-sibling;
typed pub/sub. Distinct from **vertical**, which is the contract
direction (parent-child within a single concern).

**Locus** — Aperio's primary unit of computation. Carries
lifecycle (birth → accept → run → drain → dissolve), contracts
(expose / consume), bus participation, and capacity slots. Every
memory-owner is a locus; not every locus is a memory-owner. See
`spec/types.md` and `notes/aperio-types-vs-loci.md`.

## M

**MOA** — Memory-Owner Architecture. The application-level
pattern that falls out when the framework's substrate
commitments are applied coherently. See `introduction.md`.

**Memory-owner** — a locus that is the canonical source of truth
for one concern. Declares capacity for storage and ingest for
consumption; is the canonical publisher of one bus subject family.
The unit of state ownership in an MOA app.

**moa::** — the path prefix for substrate-level declarations.
Parallel to `std::*`; resolves to declarations in `moa/*.ap`
files bundled into every Aperio binary. See `reference/types.md`.

## O

**Orchestrator** — a thin locus that routes deltas between
sibling memory-owners and manages their lifecycle. Carries no
state of its own. The shape of `main()` and of intermediate
service loci in a multi-layer MOA app. See `properties.md`
property #3.

## P

**Pattern** — a recognized composition of MOA primitives into a
recurring shape. Three are documented (broadcast-snapshot,
config-loader, private-streams); others surface as workloads
demand them. See `patterns/`.

**Private streams** — the carve-out from broadcast: per-recipient
response subjects when privacy, volume, or per-client state
require it. See `patterns/private-streams.md`.

**Projection class** — `rich | chunked | recognition`. Governs a
locus's slot-0 allocation strategy and its acceptance discipline
for children. Orchestrators are typically `rich`; memory-owners
with heavy state are typically `chunked`. See `spec/memory.md`.

**Projection memory-owner** — a memory-owner whose state is a
derived view of its input deltas (transform classification). Most
memory-owners in non-trivial apps. Replay-from-zero requires seed
state. See `properties.md` "Two memory-owner kinds."

## R

**Recording memory-owner** — a memory-owner whose state IS the
log of received deltas (save classification). Replay-from-zero is
trivial. See `properties.md` "Two memory-owner kinds."

**Resolver** — colloquial for the small CLI memory-owner pattern.
`std::cli::Resolver` is the canonical stdlib version; domain
Resolvers are also common. See `patterns/config-loader.md`.

## S

**Snapshot** — a full state re-emission from a memory-owner.
Distinct from a *delta*: snapshot is the recovery seed; delta is
the incremental update. Pattern documented at
`patterns/broadcast-snapshot.md`.

**Snapshotable** — the F.20 structural interface a memory-owner
satisfies by declaring `fn emit_snapshot()`. See
`reference/snapshotable.md`.

**Substrate** — used in two senses in MOA documents. (1) The
framework's primitives (regions, contracts, bus, capacity slots,
projection class). (2) Specifically, `moa::*` as a top-level
namespace — the architectural substrate apps build on. Context
disambiguates.

**Subject family** — a group of bus subjects with a shared prefix
(`book.snapshot.begin`, `book.snapshot.level`, `book.snapshot.end`
are one family; `book.delta` is another). Conventions:
`<concern>.<shape>.{added,removed,updated,sweep.complete}` for
tree-shaped state; `<concern>.<shape>.{fired,completed}` for
streams; `<concern>.<shape>.changed` for singles. See
`moa/subjects.md`.

## T

**Transform** — one of the three ingest classifications: the
handler mutates derived state, possibly emitting downstream
deltas as a result. The most common classification on projection
memory-owners.

## V

**Vertical** — the contract direction. Parent reads child via
`expose`; child reads parent via `consume`. Cross-sibling reads
are forbidden by `notes/aperio-types-vs-loci.md`'s axiom and by
the substrate (siblings have isolated arenas). Distinct from
**lateral**.

## Cross-references

- `properties.md` — the load-bearing definitions live here
- `patterns/` — composed shapes referenced above
- `reference/` — types and loci named in this glossary
- `notes/aperio-types-vs-loci.md` — the axiom MOA builds on
- `spec/design-rationale.md` — language design that backs MOA's
  substrate primitives
