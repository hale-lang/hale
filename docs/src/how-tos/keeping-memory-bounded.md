# Keeping memory bounded

Aperio's memory model is *arena-based*: a locus owns a region;
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

Aperio chunks are 64 KB. When a method's scratch arena
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

```aperio
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

```aperio
fn dispatch(m: ws::WsMessage) {
    self.metrics.counter("ticks_total", lbl).inc();  // ← name str_clone'd into store arena each call
}
```

Even though each call returns a handle and the handle goes
out of scope, the *literal name* loses its rodata status at the
function boundary and gets deep-copied into the callee's arena.
Each call grows the cross-locus store arena.

## Patterns that avoid bursts

### Use `BytesBuilder` for accumulators

`std::bytes::BytesBuilder` is the canonical accumulator: a single
extensible buffer that grows in place. One arena allocation
(plus whatever the buffer's internal growth strategy requires)
rather than N intermediate Strings:

```aperio
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

### Cache cross-locus handles

If you call into a different locus to look something up by
string-key on a hot path, pre-resolve at boot:

```aperio
// At boot, in main():
let c_ticks = reg.counter("ticks_total", lbl);
KrakenMdgw { c_ticks: c_ticks, ... };

// On the hot path:
fn dispatch(m: ws::WsMessage) {
    self.c_ticks.inc();   // ← zero per-call alloc
}
```

The cached handle is constructed once; the per-call `.inc()` is
a direct slot write. See
[the leak-hunt writeup in fathom][leak-hunt] for the discovery
context.

[leak-hunt]: https://github.com/aperio-lang/fathom/blob/main/MEMORY-LEAK-HUNTING-GOTCHAS.md

### Prefer substrate primitives over ASCII roundtrips

```aperio
// BAD — allocates a String per call
let ns = di.to_ns(std::time::monotonic());

// GOOD — routes through std::time::monotonic_ns() directly
let ns = di.now_ns();
```

Same for `Decimal -> Float` (`std::decimal::to_float` vs
ASCII roundtrip). Always check whether a direct primitive exists
before reaching for a `to_string` + `parse_X` bridge.

## Operational knobs

When code-level fixes aren't enough or are deferred, glibc
behavior can be tuned at process start:

| Env var | Effect |
|---|---|
| `MALLOC_TRIM_THRESHOLD_=65536` | Trim sbrk break when 64 KB+ is free at the top. Default 128 KB. Lower = more aggressive shrink, slightly higher per-`free()` cost. |
| `MALLOC_ARENA_MAX=1` | Force glibc to use one arena. Default is 8× CPU cores. Single-arena avoids cross-arena fragmentation but serializes malloc calls across threads — acceptable for binaries with one hot thread, costly if you have many. |
| `LOTUS_GLIBC_ARENA_MAX=1` | Aperio-runtime alias for `MALLOC_ARENA_MAX=1`. Set by the runtime via `mallopt(M_ARENA_MAX, 1)` at startup. |

For diagnosing what's growing, the substrate exposes:

| Env var | Effect |
|---|---|
| `LOTUS_ARENA_RESIDENCY=1` | Enable in-program arena snapshots. Call `std::process::dump_arena_residency()` from a heartbeat to dump. |
| `LOTUS_ARENA_LOG_CHUNK_ATTACH=N` | Log every chunk attach ≥ N bytes, with `arena=ptr label=... kind=root|sub` per event. |
| `LOTUS_ARENA_LOG_BIG_CHUNKS=N` | Big-chunk-only filter (subset of `CHUNK_ATTACH`). |
| `LOTUS_CHUNK_POOL_STATS=1` | At thread exit: print pool hits/misses/stores/overflows. |
| `LOTUS_CHUNK_POOL_PREFILL=N` | Warm the pool to N chunks at first touch. |

See [the diagnostic workflow][diag] for how these compose to
narrow down a leak.

[diag]: https://github.com/aperio-lang/fathom/blob/main/MEMORY-LEAK-HUNTING-GOTCHAS.md#operational-primitives--diagnostics

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

## Validated: Aperio holds the language-layer line

A May 2026 long-running production service (fathom mdgw against
the Kraken WS feed, 10 symbols, ~250 frames/sec, hot recv +
dispatch + bus-publish loop) was instrumented with
`LOTUS_ARENA_RESIDENCY=1` +
`LOTUS_ARENA_LOG_CHUNK_ATTACH=4096`. Over a 12-minute burn:

- Every named arena was flat at boot residency (no growth).
- Every `kind=root` chunk attach occurred at boot — handle
  pre-registration, subscribe encoding. **Zero per-frame
  attaches to long-lived arenas.**
- `g_bus_payload_arena` stayed at 0 chunks across the entire
  burn — confirming bus publishes do not accumulate.

The same workload's VmRSS grew at ~0.12 MB/min. That growth was
in `[heap]` but did NOT correspond to any Aperio arena event.
`MALLOC_TRIM_THRESHOLD_` + `MALLOC_ARENA_MAX` tuning had no
measurable effect, ruling out glibc internal fragmentation.
The conclusion: the residual is in upstream C-library code
(OpenSSL `SSL_read` record-reassembly buffers being the prime
suspect, with glibc stdio buffers for `/proc` reads as a
secondary), not anything Aperio is responsible for.

**Takeaway: the patterns above are sufficient. If your code
follows them and you still see RSS creep, you've found a
glibc/OpenSSL/kernel-buffer issue, not an Aperio one.** The
diagnostic workflow below tells you definitively which side
of that line you're on.

## Known issues + future work

- **`LOTUS_CHUNK_POOL_CAP` is compile-time** (hardcoded 256 in
  `crates/aperio-codegen/runtime/lotus_arena.c`). Making it
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
- The fathom leak-hunt writeup at
  `~/code/fathom/MEMORY-LEAK-HUNTING-GOTCHAS.md` for the
  longer-form story of how the language got tight in this area.
