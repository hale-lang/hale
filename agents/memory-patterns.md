# Memory patterns for `.hl` authors

This file collects the leak-class fixes the substrate now closes
structurally, plus the patterns that still require care from
`.hl` authors. Read before writing hot-path code.

Canonical reference for the substrate-side machinery is
[`spec/memory.md`](../spec/memory.md) — particularly the
"Phase-4 perf follow-ons" list, which this file's first table
mirrors. The substrate spec is normative; this file is the
author-facing distillation.

## What got closed structurally

Things the compiler / runtime takes care of. Future engineers
don't need to know about these — write the natural Hale shape
and the substrate handles it.

| Bug shape | Fixed in | Notes |
|---|---|---|
| Multi-return method scratch leaks chunks | `fb2a769` | `close_method_scratch` now fires on every return path, not just the last. Was 92% of all our growth. |
| Indexed-self-assign of struct literal allocates each call | `5b96380` | `self.bids[i] = BookLevel { ... }` mutates the existing slot in place. Generalized to `self.X = Struct{...}` too. |
| `hashmap.set(k, fresh_struct)` on existing key allocates | `f042806` | Anchor-in-place at hashmap.set boundary. **Counter.inc / Gauge.set are now zero-alloc per call** because the canonical RMW shape (`e = store.get(k); store.set(MetricEntry { key: e.key, ... })`) reads field pointers that already live in the store's arena — `lotus_str_clone`'s same-arena skip fires, the anchor walk is a no-op, the hashmap memcpys directly onto the existing cell. |
| Method-with-scratch return-value deep-copy | `f2a4cf4` | Return-expression's allocation routes directly into caller's arena via `current_arena_override = caller_arena`. Covers `peek_header`, `sweep_buy/sell`, all fresh-literal returns. |
| 24-byte view alloc per `.view()` call | `cbbebbc` | `BytesBuilder.view()` returns 16-byte struct in registers. WS frame peel no longer allocates. |
| String / Bytes field reassignment leaks old buffer | `f2a4cf4` | `self.X = String\|Bytes_value` reuses the existing slot's buffer when `new_len ≤ old_len` via `lotus_str_assign_in_place` / `lotus_bytes_assign_in_place`. |
| @form(hashmap) cells with ≥3 Decimal fields segfault | `d7f3646` | Pre-hunt. 16-byte alignment fix. |
| Book Decimal scaling corruption | `dab03b7` + `4c43c9a` | Pre-hunt. Indexed-self-assign + field-init dangling-pointer regressions. |

(Canonical reference: [`spec/memory.md`](../spec/memory.md)
Phase-4 perf follow-ons #1–#8.)

### When NOT to worry

Field types that are always free to assign regardless of frequency
— don't waste cycles optimizing assignment patterns for these:

- **Scalars**: `Int`, `Float`, `Bool`, `Decimal`, `Time`, `Duration`.
  Stored inline; assignment is a register write.
- **`LocusRef`**: a stable pointer to a long-lived locus. Assigning
  it just stores a pointer.
- **Views**: `StringView`, `BytesView`. 16-byte by-value struct in
  registers (post-`cbbebbc`); no allocation per assign.
- **`Cell` types**: live inline in their container's storage; the
  container handles lifetime.
- **String fields holding static literals** (e.g. `params { label:
  String = "ws.upstream"; }`): `lotus_str_clone` short-circuits on
  `.rodata` pointers via `lotus_str_is_static_literal`, so reads /
  returns / cross-arena copies of static-literal-initialized
  String fields are free.

## What still requires care

Patterns that are not substrate-fixed and where careless code will
quietly leak or perform badly.

### 1. Cache cross-locus handles when called per-frame

Per-call handle lookups into a different locus by string key do
work (the lookup walks the hashmap, allocates a small per-call
search struct in scratch, returns). If you call the lookup on a
hot path, do it once at boot instead:

```hale
// BAD — allocates a handle each frame
fn dispatch(m: ws::WsMessage) {
    self.metrics.counter("ticks_total", lbl).inc();
}

// GOOD — pre-register once at boot, cache the handle as a field
params {
    c_ticks: metrics::Counter;  // required, threaded in from main()
}
fn dispatch(m: ws::WsMessage) {
    self.c_ticks.inc();
}
```

**Rule of thumb:** if a hot path calls into a different locus to
look something up by string-key, pre-resolve at boot and cache
the returned handle as a locus field.

(This is structural to the arena-allocator + name-keyed lookup
shape; it's how Go / Rust idiomatic metrics code works too, not
an Hale-specific footgun.)

### 2. Prefer substrate primitives over ASCII roundtrips

When the substrate offers a direct primitive, use it. The legacy
`to_string` + `parse_X` shapes in pond are kept only for back-
compat with pre-primitive callers:

| Want | Use | Don't use |
|---|---|---|
| Monotonic clock as Int ns | `di.now_ns()` (routes through `std::time::monotonic_ns()`) | `di.to_ns(std::time::monotonic())` |
| Monotonic clock as Int seconds | `di.now_seconds()` | `di.to_seconds(std::time::monotonic())` |
| Decimal → Float | `df.to_float(d)` (routes through `std::decimal::to_float`) | hand-rolled `parse_float(to_string(d))` |
| Process RSS | `std::process::rss_bytes()` | reading /proc/self/statm manually |

**The Duration-arg path on `DurationInt.to_ns(d)` is still ASCII.**
Callers that have a `Duration` value (not a fresh `monotonic()`
call) will hit the slow path. Avoid this by holding clock readings
as Int from the start.

### 3. Return literals inline, not via locals (perf, not correctness)

The sret fix routes the return expression's allocation directly
into the caller's slot — but only when the return *is* the literal.
A local binding followed by a return doesn't get that treatment:

```hale
// GOOD — fresh literal in return position, routes via sret
fn project() -> BookSignalSnapshot {
    return BookSignalSnapshot {
        symbol: self.applier.book.symbol,
        // ...
    };
}

// LESS GOOD — local binding, then return. Forces a deep-copy
//             from scratch to caller arena even though it's correct.
fn project() -> BookSignalSnapshot {
    let snap = BookSignalSnapshot { ... };
    return snap;
}
```

This is a perf tip, not a correctness rule: the deep-copy path
still works, it just allocates extra scratch bytes and runs an
extra memcpy. Mechanism: the sret override fires during
`lower_return`'s expression lowering. `return snap` lowers as a
bare `Path` (variable lookup) — no allocation happens during the
return-expression lowering, so the override has nothing to route.
The local `snap` already lives in scratch; the boundary deep-copy
clones it into the caller's arena. Inline-return shape is the
safe default for hot paths.

### 4. `return self.field` across arenas still deep-copies

If `self.X` is a heap-typed field (`Bytes`, struct with heap
fields, dynamically-allocated `String`) and you return it from a
method, the value gets deep-copied from `self.__arena` into the
caller's arena. The sret fix only covers fresh literals, not
pre-existing field reads.

**Carve-outs that are free:**

- Pure scalars (`Int`, `Decimal`, `Float`, ...): no allocation
  either way.
- `String` fields whose value is a static literal: `lotus_str_clone`'s
  static-literal skip catches these, even when crossing arena
  boundaries. `return self.label` where the field was initialized
  with a string literal at instantiation is free.

Watch out only for dynamically-allocated `Bytes` / `String` and
nested-struct fields whose contents live in the receiver's arena.

### 5. Strings / Bytes that genuinely grow in length still leak

`self.X = String|Bytes_value` is in-place when `new_len ≤
old_len`. If the new value is *longer* than the existing slot's
buffer, the substrate falls back to a fresh clone and the old
buffer is unreachable but unfreeable.

For fields with bounded length variance (e.g. wire-format
timestamps — always ~30 chars; fixed-size frame headers;
checksums), this is
a non-issue: equal-or-shorter writes hit the memcpy path. For
genuinely variable-length fields on hot paths, the substrate
workaround is a `BytesBuilder` over a known-cap buffer + a
`StringView` / `BytesView` exposed to consumers; the builder's
internal buffer lives in `self.__arena` and is re-used across
all writes regardless of length.

There's also a slow degradation in oscillating-length workloads:
the in-place helpers track capacity via the same field as logical
length, so a `long → short → long` sequence loses capacity on the
first reduce and falls back to clone on the second grow. For the
typical bounded-variance pattern this never bites; for genuinely
variable-length fields use the BytesBuilder pattern.

### 6. Hand-spelled per-instance code is a maintenance footgun

`@form(vec)` of locus types isn't ergonomic yet. Per-symbol code
in gateway is hand-spelled across ~9 places per symbol; adding the
11th symbol means 9 edits. Not a correctness problem, but a real
maintenance cost.

When `@form(vec) of <locus>` becomes practical, hand-spelled
per-instance fields should collapse into one vec.

## Three rules for new Hale code

1. **Assignment on a hot path should be in place.** Trust the
   compiler for struct literals (5b96380), hashmap.set on existing
   keys (f042806), indexed assigns (5b96380), and String/Bytes
   fields that don't grow (f2a4cf4). If you're sure your code
   matches one of these shapes, it should be allocation-free.
   Verify with `LOTUS_ARENA_LOG_CHUNK_ATTACH=4096
   LOTUS_ARENA_RESIDENCY=1` if uncertain — look for `kind=root`
   events targeting your locus arena.

2. **Cross-arena handle lookups should happen once at boot.**
   Pattern the Counter / Gauge cache. Pre-resolve any "look up by
   string-key in another locus" call at boot, cache the handle,
   call it from the hot path.

3. **Substrate primitives over ASCII bridges.** Always.

## Operational primitives — diagnostics

When something's leaking, the workflow is:

```bash
# 1. In-program arena snapshot (gated by env var)
LOTUS_ARENA_RESIDENCY=1 ./binary
#    + call std::process::dump_arena_residency() from a hot path
#    (e.g. heartbeat handler at 1Hz)

# 2. Every allocation logged, with arena=ptr label=... kind=root|sub
LOTUS_ARENA_LOG_CHUNK_ATTACH=4096 LOTUS_ARENA_LOG_BIG_MAX_EVENTS=0 \
  LOTUS_ARENA_RESIDENCY=1 ./binary

# 3. Filter for residency-growing allocations
#    awk '$0 ~ /kind=root/ && $0 ~ /label=__lib_<locus>_/ {...}'
#    on the trace to see which call site is growing which arena.

# 4. Pool stats at thread exit
LOTUS_CHUNK_POOL_STATS=1 ./binary
#    [chunk_pool thread-exit tid=N] hits=X misses=Y stores=Z overflows=W pool_size=S

# 5. Pre-warm the pool (rare; only for short-lived workers)
LOTUS_CHUNK_POOL_PREFILL=N ./binary

# 6. Force glibc to single-arena (pre-leak-hunt mitigation, no
#    longer needed but documented):
LOTUS_GLIBC_ARENA_MAX=1 ./binary
```

Full env-var reference: [`spec/runtime.md`](../spec/runtime.md)
"Diagnostic env vars" table.

**Compose these:** the diagnostic workflow that pinned a real
per-instance arena residual was

1. `LOTUS_ARENA_RESIDENCY=1` + heartbeat dump → confirm leak exists,
   identify growing arena by label.
2. `LOTUS_ARENA_LOG_CHUNK_ATTACH` with arena labels → grep
   `kind=root label=<grower>` → identify call site of fresh allocs.
3. Source-code grep on the call site's offsets → identify the
   exact AST shape that's allocating.

**Caveat — pool stats are atexit-only.** `LOTUS_CHUNK_POOL_STATS`
only dumps at thread exit. Long-running daemons that block in
`main()` (HTTP servers, accept loops, etc.) never reach atexit;
the dump won't fire. To capture stats, temporarily exit early
after the workload completes (e.g., a fixed-duration sleep then
return from `main`).

## The mental model

The bugs from the May 2026 leak hunt all had the same shape: *fresh
allocation crosses an arena boundary into a long-lived arena; the
allocation is small but happens 30–300×/sec on a hot path; the
arena never reclaims*.

The Hale model is **arenas don't free per-allocation**. Bounded
residency requires either:

(a) The alloc lives in a scratch arena that destroys at method exit.
    Method-with-scratch is the cleanest shape; the compiler routes
    scratch lifecycle automatically.

(b) The alloc reuses the existing memory slot (in-place mutation).
    Substrate gives this to you for `self.X = ...`, `self.X[i] = ...`,
    `hashmap.set(k, v)` on existing keys, and String / Bytes
    reassign with same-or-shorter length.

(c) The alloc lives in an `accept`'d child locus that is reclaimed
    when *it* ends — not when the parent does. This is the
    connection/per-request shape: a long-lived parent `accept`s one
    child per connection, each child's state lives in the child's
    own arena, and the child is reclaimed the moment its flow ends.
    Make the child a **flow** by declaring `release(c: Child)` on the
    parent; the child's `run()` is its lifetime (a recv/park loop
    that returns on close), and run-completion reclaims it. Or end it
    explicitly with `terminate;`. Without one of these, an accept'd
    child is a *resident* — it lives until the parent dissolves (a
    daemon never dissolves → unbounded growth, the May-2026
    accept-loop leak). See spec/semantics.md § "release(c) and flow
    children". The canonical daemon shape:

        locus Conn {
            params { fd: Int = -1; rx: std::bytes::BytesBuilder = ...; }
            run() { while true { let f = recv(...); if f.closed { return; } ... } }
        }
        locus Server {
            accept(c: Conn) { }
            release(c: Conn) { }   // ← marks Conn a flow: reclaim on run() return
        }

If you write a hot-path method that fits *none* of these categories,
you'll leak. The compiler closes more shapes than you'd expect — but
the underlying constraint is structural.

## Cross-references

- [`spec/memory.md`](../spec/memory.md) — canonical substrate
  contract. Phase-4 perf follow-ons list mirrors the closed-bug
  table above.
- [`spec/runtime.md`](../spec/runtime.md) — diagnostic env var
  reference.
- pond `06b4cfa` — `_util` migrated to substrate primitives
  (`std::time::monotonic_ns`, `std::decimal::to_float`).
- hale `fb2a769`, `f042806`, `5b96380`, `cbbebbc`, `f2a4cf4`
  — the substrate fixes referenced in the closed-bug table.
