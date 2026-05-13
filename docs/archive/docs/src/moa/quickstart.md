# Quickstart

For the impatient: what to do today to make a new Aperio app
MOA-shaped, in ten minutes. The full statement lives in
`properties.md`; this page is the shortest viable path.

## The one-sentence definition

> **State lives at one memory-owner per concern. Everything above
> is routing.**

If you can identify, for each piece of state in your app, exactly
one locus that owns it, and route all other access through copied
bus deltas, you have MOA.

## Five-step authoring

Apply this for every new state-carrying concern in your app:

1. **Identify the concern.** What state is involved? (Cache?
   Counter? Projection? Recording? Registry?)
2. **Designate the memory-owner.** Create a locus (or extend an
   existing one) that holds this state and only this state.
3. **Declare capacity.**
   ```aperio
   capacity { heap entries of MyEntry; }
   ```
   At v0, if F.22 doesn't yet support your access pattern, fall
   back to a fixed-cap params array with the migration documented
   inline.
4. **Classify ingest per subscription.** Above each subscribe
   line:
   ```aperio
   /// ingest: transform — folds the delta into derived state
   subscribe "concern.event" as on_event of type Event;
   ```
   One of `discard`, `save`, `transform`. See `properties.md`.
5. **Publish deltas, not state.** Declare your subject family
   once; every observer subscribes by name.

## Three patterns to recognize

When your app needs request/response, ask first: **broadcast or
private?**

- **Broadcast (the default).** Public delta + snapshot streams;
  observers subscribe first, ping a public request channel for
  on-demand snapshots. No correlation ids; no per-recipient
  streams. See `patterns/broadcast-snapshot.md`.
- **Private (the carve-out).** Per-recipient response subjects.
  Justified only by privacy, volume, or per-client owner state.
  See `patterns/private-streams.md`.

When `main()` is parsing more than 2-3 flags, ask: **is the
config loading its own concern?**

- **Yes.** Factor it into a CLI memory-owner — `std::cli::Resolver`
  for typical needs; a domain Resolver for non-trivial cases.
  See `patterns/config-loader.md`.

## Anti-patterns to avoid

The three smells that mean you've drifted out of MOA:

- **Orchestrator with growing state.** If `main()` or a service
  locus is accumulating data, promote that concern to a new
  memory-owner child.
- **Sibling reading sibling.** No locus reads another locus's
  fields directly across a sibling boundary. If you find yourself
  wanting to, the routing should be bus-mediated and the cross
  arena needs vertical contract surface.
- **Multiple publishers per subject family.** Two memory-owners
  publishing on `book.delta` makes the system unauditable. Give
  each its own family name.

## The substrate you can reach for today

| If you need… | Reach for… |
|---|---|
| Cross-app event observation envelope | `moa::RuntimeEvent` |
| Bus subject identity | `moa::BraidId` |
| Locus identity in payloads | `moa::LocusId` |
| Monotonic tick stamp | `moa::Tick` |

All four resolve under `moa::*` today; see `reference/types.md`.

The four loci/interface — `Snapshotable`, `Clock`, `Recorder`,
`Replayer` — are declared in `moa/*.ap` but not yet bundled; see
`roadmap.md` for the wiring status.

## Verification before merge

Walk `audit-checklist.md`. Seven sections; ~30 minutes for a
1,000-line app. If all seven pass, the app is MOA-shaped.

## Where to read deeper

- `properties.md` — the four properties in full
- `patterns/` — recognized compositions
- `reference/` — substrate type and locus reference pages
- `walkthroughs/market-book.md` — a worked-example tour of a
  small MOA app

## Skipping MOA

If your app is genuinely stateless — a pure converter, a
formatter, a one-shot tool — MOA doesn't apply. Stick with the
six patterns in `spec/styleguide.md`.
MOA enters the picture as soon as the app holds anything across
events.
