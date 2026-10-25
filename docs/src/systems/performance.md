# Performance & Zero-Allocation Hot Paths

Hale bounds most memory growth automatically through per-method scratch regions and whole-field replacement retirement. For latency-critical or high-throughput services, the compiler provides a layered enforcement ladder—from default escape warnings to `@hot`, `@budget(alloc_per_call = N)`, and transitive `@effects` proofs—to guarantee zero-allocation execution on hot paths.

```hale,fragment
@no_block @no_syscall @deterministic @no_recursion @hot
@budget(alloc_per_call = 0)
fn on_tick(a: Int, b: Int) -> Int {
    return a + b;
}
```

## What Is Allocation-Free vs. What Accumulates

Inside any locus method or bus handler, a scratch sub-region opens on entry and resets on return. Intermediate values—such as temporary string concatenations, formatted strings, and parsed JSON tokens—allocate in scratch and free automatically when the activation returns.

| Pattern | Allocation Behavior | Steady-State Arena Growth |
|---|---|---|
| Transient values inside a method or handler | Allocated in per-call scratch region; freed on return | Zero |
| Scratch-local free `fn` (primitives/`String` in and out, no structs, arrays, or method calls) | Uses its own function-local scratch region | Zero |
| Scalar field or array-element write (`self.x = v`, `self.arr[i] = v`) | Overwritten in place | Zero |
| Whole-field replace in a method or handler (`self.latest = Thing { ... }`) | Old `String`/`Bytes` and nested records/tuples/arrays retire at return and recycle | Bounded by peak value size |
| Fixed scalar scratch array (`let mut t: [Int; 16] = [0; 16];` indexed via `t[i]`) | Allocated on CPU stack frame (up to 8 KiB per function) | Zero |
| String concatenation in a loop (`out = out + row`) | Allocates and copies prefix on every iteration | Quadratic within scratch |
| Growing `@form(vec)` / `@form(hashmap)` in an unbounded loop or handler | Appends to locus capacity slot until dissolve | Unbounded (flagged by `hale check`) |
| `let`-bound locus or `BytesBuilder` constructed inside a loop or handler | Allocates a new region or heap buffer per iteration | Unbounded in loops / high churn |
| `accept(c: Conn)` without `release(c: Conn)` on a daemon | Accepted child arena stays resident until parent dissolves | Unbounded with connection count |

### Whole-Field Replacement vs. In-Place Mutation

When a method or handler replaces a field on `self`—whether a struct (`self.latest = Thing { ... }`), tuple, array, payload enum (`self.state = State::Ready(name)`), or a `String`/`Bytes` field whose length changes between writes—the runtime retires the displaced heap blocks at the activation boundary and recycles them at their size-class capacity on subsequent stores. Steady-state replacements therefore hold the locus arena flat.

In-place scalar mutation (`self.latest.field = v`, `self.arr[i] = v`) remains faster because it skips the clone-and-retire cycle altogether. In-place mutation is also required when mutating state inside a long-running `run()` loop, which does not cross a method activation boundary between iterations.

### Stack-Allocated Fixed Scratch Tables

A fixed-size scalar array bound to a local variable and accessed exclusively via index reads and writes (`counts[i]`) lives on the function's CPU stack frame rather than in an arena:

```hale
fn top_bucket(samples: [Int; 64]) -> Int {
    let mut counts: [Int; 16] = [0; 16];
    let mut i = 0;
    while i < 64 {
        let b = samples[i] % 16;
        counts[b] = counts[b] + 1;
        i = i + 1;
    }
    let mut best = 0;
    let mut j = 1;
    while j < 16 {
        if counts[j] > counts[best] {
            best = j;
        }
        j = j + 1;
    }
    return best;
}
```

Because a non-scratch free `fn` allocates from its caller's region, stack placement prevents per-call arena growth when calling helpers in a loop. An array stays on the stack frame only when all of the following hold:
- Every use of the binding is a direct index read or write (`t[i]`). Passing the array to a function, returning it, storing it in a field, aliasing it with `let`, or iterating over it with `for` moves it to the arena.
- The element type is a scalar primitive.
- Total stack-table storage in the function does not exceed 8 KiB (cooperative coroutine stacks are 64 KiB). Larger tables remain valid but allocate on the arena.

## Avoiding Loop Accumulation

### Building Strings and Buffers Incrementally (`BytesBuilder`)

Concatenating strings in a loop (`out = out + render_row(i)`) allocates a fresh string on every iteration while keeping all previous intermediates alive in scratch until the function returns. This produces quadratic memory usage even in a single call:

| Output Size | Rows (500 B each) | `out = out + row` Peak RSS | `BytesBuilder` Peak RSS |
|---|---|---|---|
| 500 KB | 1,000 | +250 MB | +2 MB |
| 1 MB | 2,000 | +990 MB | +4 MB |
| 2 MB | 4,000 | +3.9 GB | +8 MB |

Use `std::bytes::BytesBuilder` (or `std::json::Builder` for JSON) to grow a single extensible buffer in place (Styleguide rule S15):

```hale
fn render_row(i: Int) -> String {
    return f"row {i}\n";
}

fn render(rows: Int) -> String {
    let b = std::bytes::BytesBuilder { };
    let mut i = 0;
    while i < rows {
        b.append_str(render_row(i));
        i = i + 1;
    }
    return std::str::from_bytes(b.finish() or raise);
}
```

### Zero-Copy Views (`StringView` and `BytesView`)

The same quadratic trap occurs when parsing text by repeatedly slicing the remainder (`rest = rest[(nl + 1)..len(rest)]`), which copies the tail on every line (Styleguide rule S16). Instead, create a `BytesView` once with `std::str::bytes_view(s)` and advance an integer cursor across it (see [Strings & Text](../basics/strings.md#walking-large-strings-with-byteview)).

`StringView` and `BytesView` (and `BytesBuilder.view()` / `.text_view()`) borrow underlying storage without copying:
- A view over a reused buffer or `BytesBuilder` is valid only until the buffer is next modified, reset, or overwritten. Accessing a stale view traps immediately at runtime.
- To persist a slice beyond the current buffer turn, copy it explicitly with `std::str::clone`.
- Inside loops, prefer `b.view()` or `b.text_view()` over `b.snapshot()` or `b.finish()`, which allocate and copy the builder's contents.

### Loci Constructed in Loops

A lightweight locus that declares no `birth()` hook borrows its caller's arena rather than allocating its own. If you construct such a locus inside a long loop with dynamically allocated fields (`Worker { name: f"made {i}" }`), those field allocations remain in the caller's region until the enclosing function returns. Hoist reusable loci to fields on `self`, or use statement-level fire-and-forget literals when appropriate.

### Resolving Keyed Handles at Boot

Looking up a metric or registry item by string key on a hot path allocates search temporaries on every call. Resolve handles once at initialization and store them in `params`:

```hale
type Msg { n: Int; }

locus Service {
    params { ticks: std::metrics::Counter; }
    fn dispatch(m: Msg) {
        self.ticks.inc();
    }
}

fn main() {
    let reg = std::metrics::Registry { namespace: "demo" };
    let s = Service {
        ticks: std::metrics::counter(reg, "ticks_total", std::metrics::labels_empty()),
    };
    s.dispatch(Msg { n: 1 });
    s.dispatch(Msg { n: 2 });
    print(reg.render());
}
```

### Reclaiming Per-Connection Children (`release`)

If a server accepts a child locus per connection via `accept(c: Conn)` without declaring `release(c: Conn)`, each child remains resident until the server dissolves, causing RSS to climb with total historical connections. Always pair `accept(c: Conn)` with `release(c: Conn)` for flow children so each connection's arena frees as soon as its `run()` exits.

## The Compiler Enforcement Ladder

Hale enforces memory and hot-path discipline through a five-stage opt-in ladder:

| Stage | Mechanism | Severity | What It Enforces |
|---|---|---|---|
| 1. Default survey | `hale check` / `hale build` | Advisory warning | Flags values escaping to `self` in unbounded loops, growing `@form(vec)`/`@form(hashmap)` inserts, `let`-bound loci/builders/factories in loops or handlers, allocating `recv` in loops, and `accept` without `release` on daemons. |
| 2. Bound annotations | `@bounded_by(<name>)` / `@unbounded` | Silences warning | Documents that an accumulation is intentionally bounded by an external input (`@bounded_by(open_requests)`) or operator-sized (`@unbounded`). |
| 3. Hot-path mode | `@hot` | Compile error | Promotes all Stage 1 advisories in the function to hard errors, and rejects `.snapshot()`/`.finish()` in loops and whole-struct `self` field replacements. |
| 4. Allocation budget | `@budget(alloc_per_call = N)` | Compile error | Statically verifies that the function performs at most `N` heap/arena allocations per invocation. |
| 5. Effect contracts | `@effects(none: { ... })` / `@no_block` / `@no_syscall` / `@deterministic` / `@no_recursion` | Compile error | Proves transitively across the call graph that the function cannot block, invoke syscalls, read non-deterministic sources, or recurse. |

### Stage 1: Default Escape & Hot-Path Survey

By default, `hale check` analyzes all long-lived programs (any binary with a `run()` loop or bus subscriber; run-to-exit scripts and `while i < N` loops with constant bounds are automatically exempt). Pass `--no-warn-unbounded-alloc` to disable the survey for a run.

The default survey warns on:
- **Unbounded escapes into `self`**: Allocations inside a runtime-bounded loop that accumulate in `self` without retirement.
- **Unbounded collection growth**: Calling `.push(x)` on `@form(vec)` or `.set(x)` on `@form(hashmap)` in an unbounded loop or handler (where the receiver has a declared type). Replace with capacity-bounded `@form(ring_buffer)` or `@form(lru_cache)`.
- **Per-iteration / per-message heavy allocations**: Instantiating a `let`-bound locus literal, calling a factory function that returns a locus (`let m = zeros(r, c)` or `mat::zeros(r, c)`), or creating a `std::bytes::BytesBuilder` inside a loop or bus handler. (Unbound statement-level calls reclaim immediately and do not warn.)
- **Allocating socket reads in loops**: Calling `recv`, `recv_bytes`, or `recv_with_source` inside a loop. Use `recv_into` with a hoisted `BytesBuilder` instead.
- **Missing `release` on daemon parents**: Declaring `accept(c: C)` without `release(c: C)` on a locus whose `run()` loops indefinitely.

> [!NOTE]
> `module { ... }` blocks are purely namespace boundaries; functions and locus methods inside modules undergo the exact same escape and `@hot` analysis as top-level declarations.

### Stage 2: `@bounded_by` and `@unbounded`

When a function or lifecycle hook intentionally accumulates state in memory:
- Prefer **`@bounded_by(<name>)`** when the growth is bounded by a known domain quantity (for example, `@bounded_by(record)` or `@bounded_by(open_requests)`).
- Use **`@unbounded`** when the growth is operator-sized or not yet bounded by a named input.
- Run `hale check --report-unbounded-use` to audit every `@bounded_by` and `@unbounded` annotation in your program and see how many warnings each silences. (`@bounded` on a locus is accepted for backward compatibility but redundant with the default check.)

### Stages 3–5: `@hot`, `@budget`, and `@effects`

Stack `@hot`, `@budget(alloc_per_call = 0)`, and effect shorthands on critical functions to make performance regressions fail the build:

```hale,fragment
@no_block @no_syscall @deterministic @no_recursion @hot
@budget(alloc_per_call = 0)
fn on_tick(a: Int, b: Int) -> Int { ... }
```

If any callee in the transitive call graph violates an effect contract or exceeds the allocation budget, `hale check` fails and prints the exact offending call chain. See [Effects & Contracts](../effects.md) for the full effect system, including `@phase_effects(birth: {alloc}, run: {})` on a locus.

### Locality & Allocation Reports

Use compiler reporting flags to inspect memory layout and working-set sizes at build time:
- `hale check --locality-report <dir>`: Reports each locus's working-set size against CPU cache-tier budgets (L1/L2/L3).
- `hale check --dump-alloc-summary <dir>`: Lists every allocation site, its escape classification (`local`, `returned`, `stored-to-self`, `sent`), its target `self.<field>` or capacity slot, and its boundedness verdict.

## Hot-Path I/O & Runtime Assertions

The standard library provides zero-allocation networking and diagnostic primitives for latency-sensitive paths:

| Primitive | Purpose |
|---|---|
| `std::io::udp::Reader { addr, port, cap }` | Event-driven, zero-copy UDP ingest handle. Calling `r.next() or raise` parks on `EPOLLIN` on a `where async_io` pool (zero idle CPU) and returns a `BytesView` over a single reused buffer. |
| `std::io::tcp::set_nodelay(fd, true)` | Disables Nagle's algorithm (`TCP_NODELAY`; also available on `std::io::tls`) so small writes flush immediately without 40 ms coalescing delays. |
| `recv_stamped_into` + `last_recv_kernel_ns()` | Reads into a reused `BytesBuilder` while capturing the kernel RX timestamp from `recvmsg` for wire-to-handler latency measurement. |
| `std::io::MirrorRing` | Double-maps a circular virtual memory region so slices crossing the ring wrap boundary remain contiguous in memory without copying. |
| `std::diag::heap_alloc_count()` | Returns the process's cumulative heap allocation count, or `-1` when unavailable. With valid readings, test a steady-state loop's allocation delta. |
| `std::diag::syscall_count(name)` | Returns the process's count of a named syscall (e.g., `"recv"`), or `-1` when unavailable or the name is unknown. With valid readings, test I/O batching. |

The diagnostic counters require a Linux native build without sanitizers.
They are unavailable on macOS. Check availability before comparing readings.

For live arena residency profiling (`LOTUS_ARENA_RESIDENCY=1`, `LOTUS_ARENA_LOG_CHUNK_ATTACH=N`, `LOTUS_CHUNK_POOL_STATS=1`), see [Operations & Debugging](./operations.md).

## Build-Time Optimization Flags

By default, `hale build` compiles native binaries at `-O3` targeting the host CPU architecture (`--target-cpu native`), enabling host-specific SIMD autovectorization (such as AVX2 or AVX-512).

| Option / Variable | Effect | When to Use |
|---|---|---|
| `--target-cpu native` *(default)* | Optimizes and vectorizes for the build machine's exact CPU. | Local execution (`hale run`) and homogeneous production hosts. |
| `--target-cpu baseline` | Targets portable `x86-64-v3` (AVX2 + BMI2 + FMA). | Shipping compiled binaries across mixed hardware fleets. |
| `LOTUS_LTO=1` | Enables full Link-Time Optimization across generated code and the C runtime (requires `lld` on `PATH`; links ~3–4x slower). | Release and benchmark builds (`LOTUS_LTO=1 hale build myservice/`). |

Native LTO requires Clang and LLD versions compatible with the compiler's
LLVM 18 bitcode. Changing the host C compiler also requires a fresh
runtime-object cache so previously compiled bitcode is not reused.

## Runtime Performance Profile

Hale's codegen and runtime are optimized around message dispatch, `@form` collections, and tight numerical loops:
- **Subject-keyed bus dispatch**: Static devirtualization and lock-free queues outperform Go channels by ~2.4x and hand-written C dispatch tables by ~1.9x on matched fan-out workloads. `where async_io` subscribers reuse coroutine stacks from a bounded per-worker pool.
- **Tight loops and `@form` access**: Native LLVM codegen matches `clang -O3` within ±15% on arithmetic loops, `@form(vec)` reads, and `@form(hashmap)` lookups, while contended `@form(hashmap)` writes run ~1.3–1.4x faster than hand-rolled C tables.
- **Known overheads**: Cross-function call protocol (~1.25x behind C/Rust), locus instantiation (2–3x due to arena setup), and heap-payload copying. Resolve handles at boot, keep hot state in `@form` slots, and spawn loci at topology changes rather than per message.

Full benchmark suites and methodology are maintained in [hale-lang/bench](https://github.com/hale-lang/bench).
