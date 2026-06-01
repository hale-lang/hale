# Performance

> **Coming from Rust / C++?** You're used to controlling
> allocation and watching it. Hale's arena model makes most code
> allocation-bounded by construction — a per-method scratch region
> absorbs intermediate allocations and frees them at method exit —
> but a few patterns can still grow a long-running process. This
> chapter is the shape of that growth and how to keep it flat.

## The default is already bounded

Inside any locus method, a *scratch sub-region* opens on entry
and is destroyed on return. Transient allocations — string
concatenations, JSON parsing, format building — land in scratch
and are reclaimed when the method returns. Values you *persist*
(`self.field = ...`) are deep-copied into the locus's own arena
first, so they outlive the scratch. The net effect: a hot
`run()` loop that allocates transiently doesn't grow the locus's
lifetime arena. You get this without doing anything.

So the question isn't "how do I free?" — it's "which patterns
defeat the automatic bounding?"

## The pattern that bites: accumulating in a loop

```hale
fn render(rows: Int) -> String {
    let mut out = "";
    let mut i = 0;
    while i < rows {
        out = out + render_row(i);     // a fresh String each iteration
        i = i + 1;
    }
    return out;
}
```

Each `out + ...` allocates a new string; scratch demand peaks at
the total size of every intermediate. For large inputs that
crosses a chunk boundary. The fix is an accumulator that grows
*one* buffer in place:

```hale
fn render(rows: Int) -> String {
    let b = std::bytes::BytesBuilder { };
    let mut i = 0;
    while i < rows {
        b.append(std::bytes::from_string(render_row(i)));
        i = i + 1;
    }
    return std::str::from_bytes(b.finish());
}
```

`BytesBuilder` is the canonical accumulator — one extensible
buffer instead of N throwaway strings. Use it (or
`std::json::Builder` for JSON output) anywhere you build a result
incrementally.

## Resolve string keys to ints at boot

If a hot path looks something up by string key in another locus,
the string gets copied on every call. Resolve the key to an `Int`
index once at startup and pass the index on the hot path:

```hale
locus Service {
    params { metrics: MetricsRegistry = MetricsRegistry { }; ticks_idx: Int = 0; }
    birth() {
        self.ticks_idx = self.metrics.register("ticks_total");  // clone once
    }
    fn dispatch(m: Msg) {
        self.metrics.inc(self.ticks_idx);                        // zero per-call alloc
    }
}
```

## Reclaim per-connection state

The other place growth hides is a daemon that
[accepts a child per connection](../services/parents-children.md).
If those children are *residents*, their regions live until the
(never-dissolving) parent does, and memory climbs with connection
count. Make them **flows** — declare `release(c: Conn)` on the
parent — so each child's region is reclaimed when its connection
ends. If RSS tracks connection count, this is almost always why.

## Knobs for when it's not your code

The substrate exposes diagnostics and glibc tuning via
environment variables — `LOTUS_ARENA_RESIDENCY=1` to dump live
arena sizes from a heartbeat, `LOTUS_ARENA_LOG_CHUNK_ATTACH=N` to
trace which arena is growing, `LOTUS_CHUNK_POOL_STATS=1` for
chunk-pool hit rates, and the `MALLOC_*` family for glibc's
trim/arena behavior. The full table is in `spec/memory.md` and
the *keeping memory bounded* spec material. The workflow:
smaps-diff over a window → if it's `[heap]`, check 30s deltas →
bursty 64KB steps mean chunk-pool overflow (a loop accumulator)
→ fix with `BytesBuilder`.

## Where Hale earns its overhead

Hale is shaped to pay *coordination* cost well — bus dispatch,
region setup, lifecycle — and it's competitive there. Pure
tight-loop arithmetic with no coordination is not where it
shines; that's substrate overhead with nothing to amortize it
against. Reach for Hale's structure where the work is
coordination-shaped, which is most real systems.

Next: what `@form` actually compiles to — [Forms under the
hood](./forms.md).
