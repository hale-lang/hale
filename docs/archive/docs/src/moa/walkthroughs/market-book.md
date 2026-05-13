# Walkthrough — `apps/market-book/`

A guided reading of the canonical MOA-shaped Aperio app, with
file:line references and locus-to-role correspondence. Use this
to ground the abstract patterns from `properties.md` and
`patterns/` in working code.

The app is ~1,000 lines across six `.ap` files; the walkthrough
focuses on the three loci that carry MOA roles. Open the source
in a parallel window if you can — the doc gives orientation, the
code carries the load.

## App overview

`market-book` is an embeddable market-book library demonstrating
the snapshot + delta envelope every market-data consumer
reconstructs from. Two loci cover the full pattern:

- **`MdGatewayL`** owns the connectivity layer (synthetic feed in
  v0; real venue when `std::bus::tcp` ships). Publishes the typed
  delta + snapshot stream.
- **`BookL`** is the embeddable consumer. Maintains a per-side
  sorted price ladder; exposes a stability query surface and a
  contract for parent loci to read through.

Plus three support pieces:

- `messages.ap` — typed payload shapes (`SnapshotBeginMsg`,
  `SnapshotLevelMsg`, `SnapshotEndMsg`, `DeltaMsg`).
- `level.ap` — single-row data type.
- `fixed.ap` — money math (Int with implicit 10^10 scale).

## MOA role: orchestrator (`main()`)

**File**: `apps/market-book/main.ap:62`

`main()` is the orchestrator. It:

1. Instantiates one `MdGatewayL` and two `BookL`s (~lines 72-74).
2. Drives the synthetic feed (`gateway.emit_synthetic_feed()`,
   line 91).
3. Drains the cooperative bus queue (`yield;`, line 95) so every
   published message dispatches to every subscriber before
   assertions run.
4. Runs assertions through the books' contract surface only —
   never directly into private state.

```aperio
let gateway = MdGatewayL { coord: "gateway.synth" };
let book_a  = BookL       { coord: "book.a" };
let book_b  = BookL       { coord: "book.b" };

gateway.emit_synthetic_feed();
yield;

// every assertion reads through BookL's contract surface
assert_eq_raw(book_a.depth_bids(), 3, "book_a has 3 bids");
```

The orchestrator holds **no state** of its own. The local
bindings (`gateway`, `book_a`, `book_b`) are handles into the
memory-owning children, not state. The orchestrator's lifetime
is the duration of `main()`'s body; the children's arenas dissolve
wholesale when their handles' scopes exit.

This matches **property #3** from `properties.md`: orchestrators
carry no state. The pattern matches `patterns/config-loader.md`
in shape, though the v1 market-book reads argv directly rather
than going through `std::cli::Resolver` — a small refactor
opportunity that doesn't change the MOA shape.

## MOA role: recording memory-owner (`MdGatewayL`)

**File**: `apps/market-book/gateway.ap`

`MdGatewayL` is a **recording memory-owner**. Its state is a
single Int seq counter (`apps/market-book/gateway.ap:31`); every
event it publishes is stamped with the current seq, advancing it
forward. The locus IS the log of seq values it has issued, in
that any replay of its synthetic feed produces the same seq
sequence deterministically.

```aperio
locus MdGatewayL {
    params {
        coord: String = "gateway";
        venue: String = "synthetic";
        seq: Int = 0;
    }

    bus {
        publish "book.snapshot.begin" of type SnapshotBeginMsg;
        publish "book.snapshot.level" of type SnapshotLevelMsg;
        publish "book.snapshot.end"   of type SnapshotEndMsg;
        publish "book.delta"          of type DeltaMsg;
    }

    fn begin_snapshot() {
        self.seq = self.seq + 1;
        let m = SnapshotBeginMsg { seq: self.seq, ... };
        "book.snapshot.begin" <- m;
    }
    // ... emit_level, end_snapshot, emit_delta similar
}
```

The MOA header comment at the top of `gateway.ap` names the role
explicitly:

> **MOA role**: recording memory-owner.
> **Memory**: a single Int seq counter; everything else is pure egress.
> **Publishes**: book.snapshot.begin / book.snapshot.level /
>                book.snapshot.end / book.delta (one publisher
>                per family per moa/subjects.md).
> **Ingest**: none — gateway is a pure publisher in v0. Real venue
>             integration would add an "ingest: transform" handler
>             on raw socket bytes.

Notice the **one publisher per subject family** rule
(property #4): `MdGatewayL` is the sole publisher of each of the
four `book.*` families. No `BookL` ever publishes on these.

The `book.snapshot.*` family fits the **tree-shaped state**
convention from `moa/subjects.md` (`<concern>.<shape>.added /
.removed / .updated / .sweep.complete`) — though market-book uses
`begin / level / end` as the family names because the
snapshot-rebuild pattern needs a delimited stream, not an
incremental tree. Both shapes are legitimate; the convention is a
default, not a requirement.

## MOA role: projection memory-owner (`BookL`)

**File**: `apps/market-book/book.ap`

`BookL` is a **projection memory-owner**. Its state is the
8-level price ladder per side (`bid_prices`, `bid_qtys`,
`ask_prices`, `ask_qtys`, plus counts), reconstructed from the
incoming snapshot + delta stream. The locus IS NOT the log of
its inputs — it's a **view** of them. Replay-from-zero requires
the initial snapshot to seed the projection.

```aperio
locus BookL {
    params {
        // ... per-side ladder arrays, counts, snapshot flag
    }
    bus {
        /// ingest: transform — clears ladder state; folds snapshot epoch start into self
        subscribe "book.snapshot.begin" as on_snap_begin of type SnapshotBeginMsg;
        /// ingest: transform — inserts level into sorted ladder at correct index
        subscribe "book.snapshot.level" as on_snap_level of type SnapshotLevelMsg;
        /// ingest: save — flips in_snapshot flag; no ladder mutation
        subscribe "book.snapshot.end"   as on_snap_end   of type SnapshotEndMsg;
        /// ingest: transform — applies delta to ladder; updates crossed-state
        subscribe "book.delta"          as on_delta      of type DeltaMsg;
    }
    contract {
        expose in_snapshot: Bool;
        expose bid_n: Int;
        expose ask_n: Int;
        expose crossed: Int;
        expose last_seq: Int;
    }
}
```

The four subscribe-line ingest classifications (`apps/market-book/book.ap:87-94`)
are the canonical worked example of MOA's per-subscription
ingest discipline:

- Three `transform` handlers (`on_snap_begin`, `on_snap_level`,
  `on_delta`) mutate ladder state. These are the "this delta
  changes derived state" cases — projection memory-owner
  behavior.
- One `save` handler (`on_snap_end`) only flips the
  `in_snapshot: Bool` flag. The flag IS the state being recorded;
  no ladder mutation happens here — recording memory-owner
  behavior within a projection memory-owner. The mix is documented
  in the locus's header comment.

This **save-and-transform within one locus** is the case
`properties.md` "Two memory-owner kinds" names: when a
memory-owner's state has both a log component and a derived
component, the per-subscription classification carries the
distinction.

`BookL`'s **contract** (lines after the bus block) exposes only
five fields — every reader outside the locus sees state through
these, never directly into the ladder arrays. This is **property
#4** at the contract level: vertical reads via contract; no
sibling reads; cross-process replicas reconstruct from the bus,
not from pointer copy.

## MOA role: pure data (`messages.ap`, `level.ap`, `fixed.ap`)

**Files**: `apps/market-book/messages.ap`, `level.ap`, `fixed.ap`

These are not loci. They are `type` records — pure shape, no
lifecycle, no flow. Per `notes/aperio-types-vs-loci.md`'s axiom:
if a thing has lifecycle, it is a locus; if it is pure data, it
is a type.

`messages.ap` defines the four bus payload types. Their fields are
copy-safe (primitives + String), so they cross the bus boundary
without issue (m20 copy-at-boundary semantics).

`level.ap` defines a single `Level { price: Int; qty: Int; }`
record used internally by `BookL`. `fixed.ap` defines the
fixed-point math types and helpers.

None of these participate in MOA structure. They're the wire
format and the math substrate the loci compose.

## What this app does NOT do (the friction surface)

Three things market-book deliberately doesn't do, each documented
as friction:

1. **F.22 capacity lift on BookL's ladder arrays.** The natural
   declaration would be `capacity { pool bid_prices of Int; ... }`
   but BookL's indexed-access pattern (`bid_prices[i] = px`)
   needs the `Cell<T>` load/store-through-pointer surface that
   lands in v1.x. Until then, the fixed-cap `[Int; 8]` arrays in
   params carry the same capacity commitment in a different
   syntactic form. Inline comment in `book.ap` documents the
   migration path.
2. **The on-demand snapshot ping pattern.** Current market-book
   has the gateway emit its synthetic feed unconditionally on a
   kicked `emit_synthetic_feed()` call from `main()`. The pattern
   from `patterns/broadcast-snapshot.md` would have `MdGatewayL`
   subscribe to `book.request.snapshot` and re-emit on demand;
   `BookL.birth()` would ping for the snapshot rather than
   depend on the orchestrator's call order. Not yet shipped.
3. **`std::cli::Resolver` for argv.** `main()` reads argv directly
   via `std::env::args_count` / `arg`. The factor-out to a
   Resolver is small and documented at
   `patterns/config-loader.md`.

Each friction is a v0.x follow-up. The app's structural shape
matches MOA today; the v0.x work tightens the realization.

## Reading order

Read the source in MOA-role order:

1. `apps/market-book/gateway.ap` — simplest memory-owner
   (recording shape).
2. `apps/market-book/book.ap` — interesting memory-owner
   (projection shape with mixed save+transform).
3. `apps/market-book/main.ap` — orchestrator.
4. `apps/market-book/messages.ap` — wire format (types only).
5. `apps/market-book/level.ap`, `fixed.ap` — internal helpers.

The MOA header at the top of each `.ap` file names the role
explicitly so cold-reading is well-anchored.

## Cross-references

- `apps/market-book/README.md` (in the repo) — the app's own
  README with the locus inventory and "What this app teaches"
  section
- `../properties.md` — the four properties this app satisfies
- `../patterns/broadcast-snapshot.md` — the request/response
  pattern market-book would adopt to add the on-demand snapshot
  ping
- `../patterns/config-loader.md` — the argv-handling pattern
  market-book would adopt to factor out the CLI concern
- `../reference/types.md` — substrate types the app could reach
  for if it needed bus-level introspection
