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

## Catching it at compile time

The growth patterns above — a per-message handler that allocates
into `self`, a connection child left resident — have a static
shape, and `hale check` flags them before you ever measure RSS.
These are advisory warnings, not build failures:

- `hale check app.hl` flags (by default — no flag needed) an allocation that
  accumulates without bound: a struct / array / bytes value created
  in a per-message bus handler (or a runtime-bounded loop) that
  escapes into `self`, where it lives until the locus dissolves —
  e.g. a whole-value replace `self.latest = Thing{…}`, which
  bump-allocates a fresh value each message. The fix is usually
  **in-place mutation** (`self.latest.field = v`, `self.arr[i] = v`)
  instead of replacing the whole value, or the moves from this
  chapter — a capacity-bounded `@form`, route it over the bus, or a
  per-iteration child. A `while i < N { … }` counter with a constant
  bound is *proven* bounded and left alone. Run-to-exit programs (a
  `main` with no `run` loop and no bus handler) are exempt
  automatically — a script that allocates and exits owes nothing.
  Opt out of a run with `--no-warn-unbounded-alloc`; annotate the long-lived locus
  `@bounded` to get the check on every `hale check` without the flag,
  and `@unbounded` (on a `fn` or a lifecycle hook) to acknowledge an
  intentional accumulation and silence it.
- The same check flags an **insert into a growing collection** —
  `v.push(x)` / `m.set(x)` where `v` / `m` is a `@form(vec)` or
  `@form(hashmap)` — when it runs in an unbounded context. The backing
  buffer grows with population and frees only at dissolve, so a push
  per message accumulates. A `@form(ring_buffer)` / `@form(lru_cache)`
  is cap-bounded and never flagged; switching to one (or bounding the
  loop) is the fix. (Detection reads the receiver's *declared* type, so
  it sees `fn f(v: IntVec)` and `self.buf: IntVec` but not an untyped
  `let`.)
- `hale check app.hl --warn-resource-leak` is the same idea for file
  descriptors: an `open` / `connect` / `accept` whose result is
  stored resident in an unbounded context, so fds pile up.

For the resource *surface* — thread / pool / subject / fd counts,
not a leak — there's a budget you can read or gate on:

```sh
hale check app.hl --dump-resource-budget
# OS threads (pinned loci):  1
# cooperative pools:         1  [io]
# bus subjects:              4
# fd acquisition sites:      2
```

Drop a ceiling file in CI and the build fails when a count climbs
past it — *"this PR added a pinned thread; bump the ceiling if you
meant to."* Every key is optional:

```toml
# budget.toml
pinned_threads = 4
bus_subjects   = 16
```

```sh
hale check app.hl --check-resource-budget budget.toml
```

None of these run by default — they're tools you reach for when a
program's memory or fd surface is something you want to hold the
line on.

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

## Hot-path I/O primitives

For latency-sensitive sockets, the stdlib exposes the knobs you'd
reach for in C, without an FFI shim:

- **Disable Nagle** — `std::io::tcp::set_nodelay(fd, true)` (and the
  `std::io::tls` sibling) so small writes hit the wire immediately
  instead of waiting ~40ms to coalesce. The first thing a
  request/response or market-data socket wants.
- **Wire-arrival timestamps** — `recv_stamped_into` is `recv_into`
  plus a kernel RX timestamp captured in the same `recvmsg`; read it
  with `last_recv_kernel_ns()` right after. True wire time, not the
  post-scheduling receipt clock — for measuring real I/O latency.
- **Wrap-free parsing** — `std::io::MirrorRing` double-maps a buffer
  so any window is one contiguous slice even across the wrap point; a
  stream parser never special-cases the seam. Opt-in (it costs 2×
  address space) — for the ordinary case a `BytesBuilder` accumulator
  is the right tool.

And the run-time complement to the compile-time
`--warn-unbounded-alloc` check: `std::diag::heap_alloc_count()` and
`std::diag::syscall_count(name)` let a test *assert* a steady-state
region did what you think — read the counter before and after and
check the delta is zero ("this loop allocated nothing", "exactly one
`recv` per poll").

## Build-time tuning

`hale build` already tunes to the machine you build on: native
builds compile for the **host CPU** at **O3**, so generated code
autovectorizes to whatever the host supports (AVX2, AVX-512, …).
Two knobs matter when that default isn't what you want:

- **`--target-cpu baseline`** — pins a portable **`x86-64-v3`**
  target (AVX2 + BMI2 + FMA) instead of the host. Reach for this
  when you **ship a binary to other machines**: the default
  host-tuned build may use instructions an older CPU lacks.
  `--target-cpu native` (the default) is right for `hale run` and
  for binaries you execute on the build host (e.g. a service on
  hardware you control).
- **`LOTUS_LTO=1`** — an opt-in full-LTO build that inlines the
  lotus runtime (the arena allocator, string helpers, shm ring)
  *into your code* across the compile boundary it otherwise can't
  cross. A few percent on allocation- and coordination-heavy
  programs — exactly the shape Hale is built for — and it keeps
  the host vectorization, so there's no loop it slows down. It's
  off by default because the link is ~3–4× slower and needs
  `lld` on PATH; turn it on for release/perf builds, not the
  edit-compile loop:

  ```sh
  LOTUS_LTO=1 hale build myservice/
  ```

## Where Hale earns its overhead

Hale is shaped to pay *coordination* cost well — bus dispatch,
region setup, lifecycle — and it's competitive there. Pure
tight-loop arithmetic with no coordination is not where it
shines; that's substrate overhead with nothing to amortize it
against. Reach for Hale's structure where the work is
coordination-shaped, which is most real systems.

Next: what `@form` actually compiles to — [Forms under the
hood](./forms.md).
