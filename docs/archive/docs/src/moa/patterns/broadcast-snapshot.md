# Broadcast + on-demand snapshot

The default request/response shape under MOA. Use it when a
memory-owner publishes a stream of deltas that many observers want
to consume, and any observer should be able to re-sync from cold
start or after drift.

## The shape

A memory-owner publishes on **public** subject families:

- `<concern>.delta` — continuous stream of incremental updates
- `<concern>.snapshot.begin` / `<concern>.snapshot.level` /
  `<concern>.snapshot.end` — full-state rebuilds, emitted on demand

A **public** request channel lets any observer trigger a snapshot
emit:

- `<concern>.request.snapshot` — many publishers (any client),
  one subscriber (the memory-owner)

A new observer:

1. **Subscribes first** — to deltas and to all three snapshot
   subjects. Aperio's bus block declares subscriptions structurally;
   they are wired before `run()` fires, so the subscribe-first
   ordering is guaranteed without manual sequencing.
2. **Pings** the request channel — publishes a single
   `<concern>.request.snapshot` event.
3. **Receives** the snapshot.begin → level → end sequence along
   with every other subscriber (the response is broadcast, not
   private).
4. **Follows** the delta stream from that point.

## Why broadcast beats per-client streams

Four reasons it's tighter than per-recipient response streams:

- **One publisher per subject family** holds strictly. The
  memory-owner is the sole publisher of `<concern>.snapshot.*` and
  `<concern>.delta`. Private streams introduce per-recipient
  publishers (or a single publisher fanning out to many subjects)
  — the rule is preserved in letter but blurred in spirit.
- **No correlation ids.** The observer doesn't need to identify
  itself for routing purposes. Whether it identifies itself for
  its own logic ("have I synced yet?") is local state, not bus
  protocol.
- **Symmetric subscribers.** All observers are equal. The system
  has no notion of "this delta is for observer X." Every
  subscriber sees every event. The only thing that varies is
  *when* each observer joined.
- **Idempotent recovery.** The snapshot IS the recovery seed.
  Anyone — joining late, recovering from drift, debugging — pings
  the channel and gets the canonical state. Same primitive serves
  cold-start, resync, and forensic inspection.

This is the gossip / replication pattern: state reconstructed from
delta stream + on-demand snapshot rebuilds. Same shape as Kafka log
compaction, CRDT snapshot resync, Raft snapshots — the framework's
bus gives it to us without ceremony.

## Subject-family asymmetry

The pattern has two asymmetries worth naming:

- **Data streams** (`<concern>.delta`, `<concern>.snapshot.*`) are
  **one-to-many**: one publisher, many subscribers.
- **Request stream** (`<concern>.request.snapshot`) is
  **many-to-one**: many publishers (any observer), one subscriber
  (the memory-owner).

Both are still "one *concern* per family," so the MOA rule holds.
The asymmetry is in publisher cardinality, not in canonical-author
identity.

## Worked sketch

A market-book gateway (recording memory-owner) and a late-joining
book reader:

```aperio
locus MdGatewayL {
    params { seq: Int = 0; /* + cached state */ }

    bus {
        publish "book.snapshot.begin" of type SnapshotBeginMsg;
        publish "book.snapshot.level" of type SnapshotLevelMsg;
        publish "book.snapshot.end"   of type SnapshotEndMsg;
        publish "book.delta"          of type DeltaMsg;

        /// ingest: transform — re-emits current state as a snapshot
        subscribe "book.request.snapshot" as on_request_snapshot
            of type SnapshotRequest;
    }

    fn on_request_snapshot(_: SnapshotRequest) {
        self.emit_snapshot();
    }

    fn emit_snapshot() {
        self.seq = self.seq + 1;
        "book.snapshot.begin" <- SnapshotBeginMsg { seq: self.seq };
        // emit each cached level on book.snapshot.level
        "book.snapshot.end"   <- SnapshotEndMsg { seq: self.seq };
    }
}

locus BookL {
    bus {
        /// ingest: transform — fold snapshot/delta into ladder
        subscribe "book.snapshot.begin" as on_snap_begin of type SnapshotBeginMsg;
        subscribe "book.snapshot.level" as on_snap_level of type SnapshotLevelMsg;
        subscribe "book.snapshot.end"   as on_snap_end   of type SnapshotEndMsg;
        subscribe "book.delta"          as on_delta      of type DeltaMsg;

        /// publishes: book.request.snapshot when self needs a sync
        publish "book.request.snapshot" of type SnapshotRequest;
    }

    birth {
        // Subscriptions are wired structurally; ping for sync.
        "book.request.snapshot" <- SnapshotRequest { client_coord: self.coord };
    }
}
```

The `MdGatewayL` already implements the
`moa::Snapshotable` structural interface (its `fn emit_snapshot()`
satisfies the signature). The pattern composes cleanly with the
F.20 interface system.

## When the pattern does not apply

Three carve-outs justify falling back to private response streams
(see `private-streams.md`):

1. **Privacy / auth** — observer X must not see observer Y's data.
2. **Volume** — broadcasting everything to everyone is too
   expensive at scale.
3. **Per-client owner state** — the memory-owner needs to track
   per-observer positions, acks, or cursors.

None of these apply to most stateful apps; broadcast is the right
default. When the carve-out *does* apply, layer the private pattern
on top of the broadcast default — don't replace.

## Cross-references

- `properties.md` — the four properties the pattern preserves
- `private-streams.md` — the carve-out alternative
- `../reference/snapshotable.md` — the `moa::Snapshotable`
  interface that memory-owners satisfy when implementing this
  pattern
- `moa/subjects.md` — bus subject naming conventions
