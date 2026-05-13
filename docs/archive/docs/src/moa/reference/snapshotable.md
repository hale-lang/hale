# `moa::Snapshotable`

F.20 structural interface that codifies the broadcast +
on-demand snapshot pattern (see
`../patterns/broadcast-snapshot.md`).

## Signature

```aperio
interface Snapshotable {
    fn emit_snapshot();
}
```

Any locus that declares a method named `emit_snapshot()` with the
same arity and signature structurally satisfies `Snapshotable`.
There is no `impl Snapshotable for L` declaration; satisfaction is
implicit (Go-shaped, not Rust-shaped — see
`docs/src/reference/types/interfaces.md`).

## What the interface marks

`emit_snapshot()` is a method the memory-owner runs to **re-emit
its current state** as a stream of snapshot deltas on its public
subject family. The interface does not specify which subjects;
each implementing locus owns its own subject namespace.

A locus that satisfies `Snapshotable` typically also:

- Subscribes to `<concern>.request.snapshot` (the on-demand
  trigger).
- Publishes `<concern>.snapshot.begin` /
  `<concern>.snapshot.level` / `<concern>.snapshot.end` (the
  emission stream).
- Calls `self.emit_snapshot()` from its
  `on_request_snapshot` handler.

None of that is enforced by the interface — only the method's
existence and signature. The pattern shape is convention.

## Example

```aperio
locus BookL {
    bus {
        publish "book.snapshot.begin" of type SnapshotBeginMsg;
        publish "book.snapshot.level" of type SnapshotLevelMsg;
        publish "book.snapshot.end"   of type SnapshotEndMsg;
        subscribe "book.request.snapshot" as on_req of type SnapshotRequest;
    }
    fn on_req(_: SnapshotRequest) {
        self.emit_snapshot();
    }
    fn emit_snapshot() {
        "book.snapshot.begin" <- SnapshotBeginMsg { seq: self.seq };
        // ... emit each cached level
        "book.snapshot.end"   <- SnapshotEndMsg { seq: self.seq };
    }
}
```

`BookL` structurally satisfies `moa::Snapshotable` because it
declares `fn emit_snapshot()`. A polymorphic consumer can accept
a `Snapshotable`-typed parameter and invoke `s.emit_snapshot()`
regardless of which concrete locus is behind the interface; F.20
vtable dispatch handles the call.

## Why the signature is intentionally minimal

One method, no return, no parameters. The interface marks intent
(this locus can re-emit its state) without prescribing the state
shape, the subject names, or the snapshot encoding. Each
memory-owner owns those decisions.

A richer interface (e.g.
`fn emit_snapshot_to(subject_prefix: String)`) would let one
polymorphic dispatcher route snapshot emissions to different
subject namespaces — but the v1 use case for that doesn't yet
exist, and the styleguide's rolling rule says interfaces grow
when forced by a real workload.

## v0 wiring status

`moa::Snapshotable` is declared in `moa/snapshotable.ap` but not
yet bundled into `MOA_AP_SOURCE`. The wiring is two single-line
edits in `crates/aperio-codegen/src/codegen.rs` (see the file
header in `moa/snapshotable.ap`). Until then, locus declarations
satisfying the interface compile fine — they just can't be used
as `moa::Snapshotable`-typed values across function boundaries.

## Cross-references

- `../patterns/broadcast-snapshot.md` — the pattern this
  interface codifies
- `docs/src/reference/types/interfaces.md` — F.20 structural
  interface semantics
- `spec/design-rationale.md` §F.20 — the design rationale
