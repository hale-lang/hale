# `moa::*` substrate types

Domain-agnostic payload types every MOA-shaped app uses. Declared
in `moa/types.ap`; resolved under the `moa::*` path prefix.

## `moa::LocusId`

Canonical identifier for a locus in any MOA app.

```aperio
type LocusId {
    name: String;
    path: String;
}
```

- `name` — the locus's declared identifier (e.g. `"BookL"`).
- `path` — source-file path where the locus is declared.

Use when one MOA component needs to refer to another locus
unambiguously in a bus payload: runtime events, audit reports,
debugger introspection.

## `moa::BraidId`

Canonical identifier for a bus subscription. A *braid* connects a
publisher to a subscriber across the locus tower; the id triple
uniquely names the connection.

```aperio
type BraidId {
    subject: String;
    from_path: String;
    to_path: String;
}
```

- `subject` — the subject the braid carries.
- `from_path` — source path of the publishing locus.
- `to_path` — source path of the subscribing locus.

Used by tooling that introspects bus topology (the IDE's lotus
visualizer, the codebase-onboarder, audit reports).

## `moa::Tick`

Monotonic clock pulse.

```aperio
type Tick {
    now_ns: Int;
    seq: Int;
}
```

- `now_ns` — monotonic nanoseconds since the runtime's reference
  instant (per `spec/runtime.md` monotonic-only scheduling).
- `seq` — per-publisher monotonically-increasing sequence number
  for ordering within a scheduler. Subscribers detect dropped ticks
  via seq gaps.

Published by `moa::Clock` (see `clock.md`) on `clock.tick`. Any
locus that subscribes to `clock.tick` consumes this type.

## `moa::RuntimeEvent`

Observation envelope for events occurring inside a watched
program's bus. Published on `runtime.event.**` (m94 wildcards) by
runtime-introspection layers and consumed by `moa::Recorder` (see
`recorder.md`) and tooling.

```aperio
type RuntimeEvent {
    kind: Int;
    origin: LocusId;
    subject: String;
    payload_size: Int;
    timestamp_ns: Int;
}
```

- `kind` — discriminator: `0=bus_send`, `1=lifecycle`,
  `2=closure_violation`, `3=scheduler` (extensible at v1.x).
- `origin` — the locus the event happened in. The field is
  named `origin` rather than `locus` because `locus` is a reserved
  declaration keyword.
- `subject` — the bus subject the event was published on (for
  `kind=bus_send`), or a synthetic identifier for non-bus events.
- `payload_size` — byte size of the original payload. The payload
  itself does not cross — only the envelope.
- `timestamp_ns` — monotonic timestamp at event emission.

## Naming convention

User-facing names are PascalCase under `moa::*` — `moa::LocusId`,
`moa::Tick`, etc. The compiler resolves these to mangled internal
names with the `__Moa` prefix — `__MoaLocusId`, `__MoaTick`. The
mangling makes collision with user-declared identifiers impossible.

Cross-reference: `crates/aperio-codegen/src/codegen.rs`'s
`MOA_PATH_RENAMES` table for the canonical user-path-to-internal
mapping.
