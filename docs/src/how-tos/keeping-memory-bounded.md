# Keeping memory bounded

Hale's memory model is *arena-based*: a locus owns a region;
the region grows by chunk allocation and is wholesale-freed at
dissolve (see [Memory model][memory] in the spec). Inside a
method, a per-method scratch subregion absorbs intermediate
allocations and destroys on method exit. Together these make
most code allocation-bounded by construction.

But the underlying allocator is glibc `malloc`. Long-running
binaries that hammer arenas with bursty workloads can grow
resident memory even when every locus arena's *residency* is
flat. This page documents the shape of that growth, the patterns
that cause it, and how to avoid them.

[memory]: ../../../spec/memory.md

## The shape: bursty stair-steps in `[heap]`

A long-running service (mdgw, an HTTP gateway, anything with a
hot recv/dispatch loop) that has all its locus arenas pinned
flat may still show RSS growth on the order of **0.1–0.3 MB/min**.
The growth is concentrated in `[heap]` (smaps-visible glibc
sbrk region), not anonymous mappings, not file-backed pages, and
not the FD table.

The micro-shape is bursty, not smooth: long stretches of zero
growth punctuated by **stair-steps of ~64 KB or 128 KB** (one
or two chunks at a time). On a 30-second sampling cadence, a
typical pattern is:

```
30s window  delta     cumulative
─────────────────────────────────
+0s        +0 MB       0 MB
+30s       +0.12 MB    0.12 MB    (2 chunks)
+60s       +0 MB       0.12 MB
+90s       +0.12 MB    0.24 MB    (2 more chunks)
+120s      +0 MB       0.24 MB
...
```

## Why it happens

Hale chunks are 64 KB. When a method's scratch arena
exhausts its current chunk, it pulls another from the per-thread
**chunk pool**. When the method exits and its scratch destroys,
chunks return to the pool for reuse.

The pool has a fixed capacity (`LOTUS_CHUNK_POOL_CAP`,
currently hardcoded at 256 — see `lotus_arena.c`). When more
chunks are returned than the pool can hold, the surplus is
`free()`d back to glibc.

**Glibc does not always return `free()`d memory to the OS.** The
`M_TRIM_THRESHOLD` heuristic (default 128 KB) requires
contiguous free space at the top of the sbrk break to shrink.
Mid-heap free regions are kept on glibc's internal free list and
reused for future mallocs — but the address-space footprint
(`VmData`) doesn't shrink, and `VmRSS` follows accumulated
touched pages.

So: steady-state churn that occasionally bursts past the pool's
ceiling produces a slow, bursty stair-step in `[heap]` that
never reverses. Every arena's residency is genuinely flat (each
locus's *live* chunk count is bounded); the leak is in the
malloc bookkeeping layer underneath.

## Patterns that cause bursts

### String concat in a loop

```hale
fn render() -> String {
    let mut out = "";
    let mut i = 0;
    while i < n {
        out = out + render_row(i);   // ← N intermediate Strings, scratch peaks at N
        i = i + 1;
    }
    return out;
}
```

Each `out = out + render_row(i)` allocates a fresh String and
makes the previous `out` unreachable within the method-scratch.
The scratch's chunk demand peaks at the total bytes of all
intermediate Strings. For 50-row inputs with 100 byte rows,
that's 50 × 100 + concat overhead = several KB; for 500 rows or
larger rows, it crosses 64 KB and pulls a second chunk. The
chunks free back to the pool on method exit — but if the pool
is already full, the surplus is the stair-step.

### Variable-length scratch builders

Any pattern that builds a result incrementally in scratch —
JSON Builders, log-line construction, exposition rendering —
has the same shape. The peak scratch demand drives chunk
allocation.

### Per-frame factory calls that bridge arenas

```hale
fn dispatch(m: ws::WsMessage) {
    self.metrics.counter("ticks_total", lbl).inc();  // ← compile error: methods on loci may not return locus values
}
```

This shape is now rejected at typecheck per
`spec/semantics.md § Locus method dispatch` (the CQRS rule): a
method on the metrics locus cannot return a Counter locus to
the caller. The historical allocation cost — the name string
gets deep-copied into the callee's arena on every call —
remains a useful illustration of *why* the pattern is bad, but
the compiler now closes the door before runtime gets a chance.

The three canonical alternatives:

- **Parent-child + index** — `metrics` becomes a parent locus
  that owns its counters as children. The caller resolves the
  name to an `Int` index once at boot, then calls
  `self.metrics.inc(idx)` on the hot path (Int-keyed, no string
  clone).
- **Bus topic** — publish an "increment counter X" command;
  the metrics subscriber dispatches to the right counter.
- **Delegation** — collapse the per-counter operation onto the
  parent (`self.metrics.inc_named("ticks")`). Cheap when the
  caller hits a small fixed set of names; the per-call string
  shows up in the parent's arena, but the surface is simpler.

## Patterns that avoid bursts

### Use `BytesBuilder` for accumulators

`std::bytes::BytesBuilder` is the canonical accumulator: a single
extensible buffer that grows in place. One arena allocation
(plus whatever the buffer's internal growth strategy requires)
rather than N intermediate Strings:

```hale
fn render() -> String {
    let b = std::bytes::BytesBuilder { };
    let mut i = 0;
    while i < n {
        b.append(std::bytes::from_string(render_row(i)));
        i = i + 1;
    }
    return std::bytes::to_string(b.finish());
}
```

This compresses the scratch peak from O(N × row_size) down to
O(largest_row + buffer_doublings).

For pure string output, `std::json::Builder` is the right
choice when the output is JSON (it handles escaping correctly
in the bargain — see
[Build a wire-format parser](./wire-format-parsers.md) for the
inverse direction).

### Resolve string keys to Int indices at boot

If you call into a different locus to look something up by
string-key on a hot path, pre-resolve the key to an `Int`
index at boot and pass that index on the hot path:

```hale
locus Service {
    params {
        metrics:   MetricsRegistry = MetricsRegistry { };
        ticks_idx: Int             = 0;
    }
    birth() {
        // register() returns Int — value-typed, satisfies the
        // CQRS rule from `spec/semantics.md § Locus method
        // dispatch`. String-clone happens here, exactly once.
        self.ticks_idx = self.metrics.register("ticks_total");
    }

    fn dispatch(m: ws::WsMessage) {
        self.metrics.inc(self.ticks_idx);   // ← zero per-call alloc
    }
}
```

One boot-time string clone, an Int-keyed hot path. See
[`agents/memory-patterns.md`][patterns] for the discovery context
and the full catalog of substrate-closed leak shapes.

[patterns]: ../../../agents/memory-patterns.md

### Prefer substrate primitives over ASCII roundtrips

```hale
// BAD — allocates a String per call
let ns = di.to_ns(std::time::monotonic());

// GOOD — routes through std::time::monotonic_ns() directly
let ns = di.now_ns();
```

Same for `Decimal -> Float` (`std::decimal::to_float` vs
ASCII roundtrip). Always check whether a direct primitive exists
before reaching for a `to_string` + `parse_X` bridge.

### Reclaim per-connection state with flow children

The patterns above bound a *method's* scratch. The other place
unbounded growth hides is a **daemon that accepts one child locus
per connection** (a server). Each accept'd child's state lives in
the child's own arena — but by default that arena is freed only
when the *parent* dissolves, and a daemon's parent never does. So
per-connection arenas pile up for the life of the process: one
leaked region per connection, forever.

The fix is to make the child a **flow** — its lifetime is its own
`run()`, not the parent's. Declare `release(c: Conn)` on the
parent; then when a `Conn`'s `run()` completes (its recv loop
returns on close), the runtime reclaims it — drains it, hands it
to the parent's `release` for a last look, dissolves it, frees its
arena — while the daemon keeps running.

```hale
locus Conn {
    params {
        conn_fd: Int               = -1;
        rx:      std::bytes::BytesBuilder
               = std::bytes::BytesBuilder { initial_cap: 4096 };
    }
    run() {
        // run() IS the connection's lifetime. When the client
        // closes, recv returns empty, this loop ends, run()
        // returns — and Conn is reclaimed.
        let stream = std::io::tcp::Stream { conn_fd: self.conn_fd, owns_fd: false };
        loop {
            let chunk = stream.recv(4096);
            if len(chunk) == 0 { return; }   // ← EOF → reclaim
            // ... handle chunk
        }
    }
}

locus Server {
    accept(c: Conn)  { }
    release(c: Conn) { }   // ← declaring this marks Conn a *flow*
}
```

Without a `release` declaration (or an explicit `terminate;`), an
accept'd child is a **resident** — it lives until the parent
dissolves. That's correct for a fixed cohort of long-lived
workers, but it's the leak for connections. If you accept a child
per connection and RSS climbs with connection count, you have a
resident that should be a flow. See
[Lifecycle & time](../concepts/lifecycle-time.md) for `terminate`
and `release`, and [Build an HTTP server](./http-server.md) for
the full server shape.

## Operational knobs

When code-level fixes aren't enough or are deferred, glibc
behavior can be tuned at process start:

| Env var | Effect |
|---|---|
| `MALLOC_TRIM_THRESHOLD_=65536` | Trim sbrk break when 64 KB+ is free at the top. Default 128 KB. Lower = more aggressive shrink, slightly higher per-`free()` cost. |
| `MALLOC_ARENA_MAX=1` | Force glibc to use one arena. Default is 8× CPU cores. Single-arena avoids cross-arena fragmentation but serializes malloc calls across threads — acceptable for binaries with one hot thread, costly if you have many. |
| `LOTUS_GLIBC_ARENA_MAX=1` | Hale-runtime alias for `MALLOC_ARENA_MAX=1`. Set by the runtime via `mallopt(M_ARENA_MAX, 1)` at startup. |

For diagnosing what's growing, the substrate exposes:

| Env var | Effect |
|---|---|
| `LOTUS_ARENA_RESIDENCY=1` | Enable in-program arena snapshots. Call `std::process::dump_arena_residency()` from a heartbeat to dump. |
| `LOTUS_ARENA_LOG_CHUNK_ATTACH=N` | Log every chunk attach ≥ N bytes, with `arena=ptr label=... kind=root|sub` per event. |
| `LOTUS_ARENA_LOG_BIG_CHUNKS=N` | Big-chunk-only filter (subset of `CHUNK_ATTACH`). |
| `LOTUS_CHUNK_POOL_STATS=1` | At thread exit: print pool hits/misses/stores/overflows. |
| `LOTUS_CHUNK_POOL_PREFILL=N` | Warm the pool to N chunks at first touch. |

## F.32 cache-aware env vars (2026-05-25)

Operator-tunable knobs from the F.32 cache-aware substrate
work. Each is opt-in; defaults match shipped behavior.

**Runtime env vars** (read by the running binary):

| Env var | Effect |
|---|---|
| `LOTUS_ARENA_CHUNK_BYTES_OVERRIDE=N` | Override the default 64 KB arena chunk size. N must be a power of 2 in `[4096, 16777216]`. For multi-locus-per-pool deployments where the default chunk dwarfs L2-per-core, set N smaller (e.g. 16384) so each locus's hot chunk fits in cache across pool rotations. (F.32-3.) |
| `LOTUS_HUGE_PAGES=1` | Use `mmap(MAP_HUGETLB \| MAP_HUGE_2MB)` for arena chunks ≥ 2 MB. Falls back to plain malloc on failure (no diagnostic — check `perf stat -e dTLB-load-misses` to verify the path took). Operator prereq: `sudo sysctl vm.nr_hugepages=N` first to reserve N 2-MB huge pages in the kernel pool. (F.32-4a.) |
| `LOTUS_LOCK_MEMORY=1` | Call `mlockall(MCL_CURRENT \| MCL_FUTURE)` at startup to lock all current + future pages against paging out. Eliminates worst-case page-fault stalls on hot-path allocation. Operator prereq: `ulimit -l unlimited` or `CAP_IPC_LOCK` on the binary. On failure: stderr warning + continue unlocked. (F.32-4c.) |

**Build-time env vars** (read by `hale build`):

| Env var | Effect |
|---|---|
| `LOTUS_DISABLE_PREFETCH=1` | Compile the runtime with the bus-dispatch `__builtin_prefetch` hint stubbed out. Default = enabled (matches shipped behavior). Use for A/B measuring the prefetch's contribution on your hardware — the hint's win varies wildly with L2/L3 size and interconnect speed. (F.32-4-prefetch.) |

**Language-surface sync disciplines** (NOT env vars — these
are declared on `@form(hashmap, sync = X)` at the source level;
see [`spec/forms.md`](../../spec/forms.md) "Cross-pool sync disciplines"):
`sync = serialized` (F.32-1α), `sync = striped` (F.32-1β2-v2),
`sync = lockfree, cap = N` (F.32-1γ-v1).

See [the diagnostic workflow][diag] for how these compose to
narrow down a leak.

[diag]: ../../../agents/memory-patterns.md#operational-primitives--diagnostics

## Diagnostic workflow

1. **Smaps diff over a 15-min window** — confirms whether growth
   is in `[heap]` (malloc-driven), file-backed (dlopen/dirty
   page cache), or anonymous mmaps (mmap'd outside glibc).
2. **If `[heap]`**, check Prometheus or `cat /proc/$PID/status`
   delta over 30s windows. A bursty stair-step pattern (deltas
   quantized to 64 KB / 128 KB / etc.) confirms chunk-pool
   overflow. A smooth ramp suggests libc-internal buffer growth
   (TLS state, stdio, getaddrinfo cache).
3. **If bursty pattern**, set `LOTUS_ARENA_LOG_CHUNK_ATTACH=4096`
   + `LOTUS_ARENA_RESIDENCY=1` and re-run. The trace shows which
   locus's arena is bursting and at what call-site offsets.
4. **From the call-site offsets**, grep the source for `+ ` in
   loops (`out = out + ...`) and chains of `String + String + ...`.
   Replace with `BytesBuilder` or `json::Builder` accumulators.

## Validated: Hale holds the language-layer line

A May 2026 long-running production service (a market-data
gateway against an upstream WS feed, 10 streams, ~250 frames/sec,
hot recv + dispatch + bus-publish loop) was instrumented with
`LOTUS_ARENA_RESIDENCY=1` +
`LOTUS_ARENA_LOG_CHUNK_ATTACH=4096`. Over a 12-minute burn:

- Every named arena was flat at boot residency (no growth).
- Every `kind=root` chunk attach occurred at boot — handle
  pre-registration, subscribe encoding. **Zero per-frame
  attaches to long-lived arenas.**
- `g_bus_payload_arena` stayed at 0 chunks across the entire
  burn — confirming bus publishes do not accumulate.

The same workload's VmRSS grew at ~0.12 MB/min. That growth was
in `[heap]` but did NOT correspond to any Hale arena event.
`MALLOC_TRIM_THRESHOLD_` + `MALLOC_ARENA_MAX` tuning had no
measurable effect, ruling out glibc internal fragmentation. The
growth was traced to OpenSSL holding ~16-32 KiB of read/write
buffer state per long-lived TLS connection between records — the
prime suspect of the bisection, confirmed by the well-known
`SSL_MODE_RELEASE_BUFFERS` knob being absent from the substrate's
`SSL_CTX` setup. Subsequent commit set the mode flag in
`lotus_tls__ctx_get`; OpenSSL now releases the per-connection
buffers back to libc malloc on idle. Re-validation against the
same workload is pending; the substrate's own arenas remained
flat throughout, so the post-fix expectation is near-zero
structural drift.

**Takeaway: the patterns above are sufficient at the Hale layer.
The substrate also configures the C libraries it links against
(OpenSSL, glibc) conservatively for long-running workloads. If
your code follows the patterns AND you still see RSS creep, the
diagnostic workflow below isolates whether the source is inside
or outside Hale's arena allocator.**

## Known issues + future work

- **`LOTUS_CHUNK_POOL_CAP` is compile-time** (hardcoded 256 in
  `crates/hale-codegen/runtime/lotus_arena.c`). Making it
  env-configurable would let operators tune the pool ceiling
  per-deployment. Filed as a substrate ask.
- **Chunks that overflow the pool `free()` to glibc rather than
  `munmap()`-ing**. Returning oversized-pool chunks via
  `munmap` would let the OS reclaim the page outright instead
  of leaving it on glibc's free list. Substrate change; deferred
  pending evidence that the pool overflow rate justifies it.
- **No automatic accumulator in `std::json::Builder` for the
  "build a long flat string of key:value pairs" case**, which is
  exactly what Prometheus exposition does. `BytesBuilder` works
  but the ergonomics aren't great for the common case.

## See also

- [Memory model spec][memory] — the foundational rules
- [Capacity & storage](../concepts/capacity-storage.md) — locus
  storage classes
- [Lifecycle & time](../concepts/lifecycle-time.md) — when
  arenas dissolve
- [`agents/memory-patterns.md`](../../../agents/memory-patterns.md)
  — author-facing brief on hot-path memory shapes, mirrors the
  substrate's Phase-4 perf follow-ons list with carve-outs for
  "when not to worry."
