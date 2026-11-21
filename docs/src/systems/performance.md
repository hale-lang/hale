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
| Scratch-local free `fn` (scalars, `String` and plain records in, a scalar or `String` out; may build records and fill their `bounded` lists locally; no arrays, loci or method calls) | Uses its own function-local scratch region | Zero |
| Scalar field or array-element write (`self.x = v`, `self.arr[i] = v`) | Overwritten in place | Zero |
| Whole-field replace in a method or handler (`self.latest = Thing { ... }`) | Old `String`/`Bytes` and nested records/tuples/arrays retire at return and recycle | Bounded by peak value size |
| Fixed scalar scratch array (`let mut t: [Int; 16] = [0; 16];` indexed via `t[i]`) | Allocated on CPU stack frame (up to 8 KiB per function) | Zero |
| Appending to a local or a `self` field in a loop (`out = out + row`, `out += row`) | Appends into a buffer the place owns, grown by half again when full | Linear in the text |
| Appending in a loop that also hands the text to a call, or to another place (`rec.body = rec.body + row`) | Copies the whole text on every iteration | Quadratic within scratch (flagged by `hale check`) |
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

### Building Strings and Buffers Incrementally

An appending assignment (`out = out + render_row(i)`, `out += …`, `out = f"{out}…"`) on a local declared `let mut`, or on a `String` field of `self`, appends into a buffer that place owns and grows geometrically, so it is linear in time and memory (spec/stdlib.md § "Building a String"). Before GH #1947 every `+` made a new String that stayed in scratch until the function returned, and the issue's loop of 44-byte rows peaked as follows; it now holds the text once:

| Output Size | Appends | Before (peak RSS) | Now (peak RSS) |
|---|---|---|---|
| 43 KB | 1,000 | 80 MB | 3.1 MB |
| 175 KB | 4,000 | 1.18 GB | 3.5 MB |
| 350 KB | 8,000 | 4.5 GB | 4.0 MB |

The buffer is the local's only while nothing else holds the text: a read that may keep the value (the local passed to a call, bound, stored or returned) gives it up, and the next append copies the text first. Inside the loop, read the growing text only as an operand of `+` or a comparison, in a slice, or as an argument of a `std::` reader returning a number or a `Bool` (`len`, `std::str::contains`), and hand it on after the loop. A loop that appends and also hands the text to a call, and an append to any place but a local or a `self` field, copy the whole text each iteration; the hot-path lint flags both (`hale verify`, or `hale check --warn-hot-path`). For those, and for a buffer held across calls, use `std::bytes::BytesBuilder` (or `std::json::Builder` for JSON) to grow a single extensible buffer in place (Styleguide rule S15):

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

### Measuring a String

A `String` carries no length, so measuring one walks it to its terminator. A local or parameter of type `String` that its function measures (`len(s)`, a slice `s[lo..hi]`, `std::str::substring`, `index_of_from`, `cp_at`, `cp_size`) remembers the length of the value it holds: the first measure walks the text, later ones read the length, and every assignment to the local, its `let` and each append refresh it (an append knows the new length without walking). So a byte loop over a parameter and a loop that tests `len(out)` while appending to it are linear. Before GH #1947 both were quadratic, and so was every slice's clamp:

| Loop (M1) | 128 KiB | 512 KiB | Before |
|---|---|---|---|
| `while i < len(s) { let c = s[i..(i + 1)]; … }` | 1.1 ms | 4.2 ms | 0.71 s, 11.0 s |

A String read from a field, an element or a call's result is measured on every use, as before: bind it to a local before a loop that measures it. A slice still copies the bytes it names, so re-slicing the rest of a text (`rest = rest[(nl + 1)..]`) is still quadratic.

### Zero-Copy Views (`StringView` and `BytesView`)

The same quadratic trap occurs when parsing text by repeatedly slicing the remainder (`rest = rest[(nl + 1)..]`), which copies the tail on every line (Styleguide rule S16). Instead, create a `BytesView` once with `std::str::bytes_view(s)` and advance an integer cursor across it (see [Strings & Bytes](../basics/strings.md#walking-large-strings-with-byteview)).

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
| 1. Default survey | `hale check` / `hale build` | Advisory warning | Flags values escaping to `self` in unbounded loops, growing `@form(vec)`/`@form(hashmap)` inserts, and `accept` without `release` on daemons. With `hale check --warn-hot-path` (and always under `hale verify`), also `let`-bound loci/builders/factories in loops or handlers and allocating `recv` in loops. |
| 2. Bound annotations | `@bounded_by(<name>)` / `@unbounded` | Silences warning | Documents that an accumulation is intentionally bounded by an external input (`@bounded_by(open_requests)`) or operator-sized (`@unbounded`). |
| 3. Hot-path mode | `@hot` | Compile error | Promotes the hot-path lint's findings in the function to hard errors, and rejects `.snapshot()`/`.finish()` in loops and whole-struct `self` field replacements. |
| 4. Allocation budget | `@budget(alloc_per_call = N)` | Compile error | Statically verifies that the function performs at most `N` heap/arena allocations per invocation. |
| 5. Effect contracts | `@effects(none: { ... })` / `@no_block` / `@no_syscall` / `@deterministic` / `@no_recursion` | Compile error | Proves transitively across the call graph that the function cannot block, invoke syscalls, read non-deterministic sources, or recurse. |

### Stage 1: Default Escape & Hot-Path Survey

By default, `hale check` analyzes all long-lived programs (any binary with a `run()` loop or bus subscriber; run-to-exit scripts and `while i < N` loops with constant bounds are automatically exempt). Pass `--no-warn-unbounded-alloc` to disable the survey for a run.

The default survey warns on:
- **Unbounded escapes into `self`**: Allocations inside a runtime-bounded loop that accumulate in `self` without retirement.
- **Unbounded collection growth**: Calling `.push(x)` on `@form(vec)` or `.set(x)` on `@form(hashmap)` in an unbounded loop or handler (where the receiver has a declared type). Replace with capacity-bounded `@form(ring_buffer)` or `@form(lru_cache)`.
- **Missing `release` on daemon parents**: Declaring `accept(c: C)` without `release(c: C)` on a locus whose `run()` loops indefinitely.

The **hot-path lint** is asked for, not default: `hale check --warn-hot-path` shows it, `hale verify` always reports it, and inside a `@hot` fn its findings are errors. An ordinary loop in `fn main` or a child born in a handler is not a path anyone certified hot, so a first program is not buried in these. When asked, it warns on:
- **Per-iteration / per-message heavy allocations**: Instantiating a `let`-bound locus literal, calling a factory function that returns a locus (`let m = zeros(r, c)` or `mat::zeros(r, c)`), or creating a `std::bytes::BytesBuilder` inside a loop or bus handler. (Unbound statement-level calls reclaim immediately and do not warn.)
- **Allocating socket reads in loops**: Calling `recv`, `recv_bytes`, or `recv_with_source` inside a loop. Use `recv_into` with a hoisted `BytesBuilder` instead.
- **Appends that copy the whole text**: An appending assignment (`out += row`) in a loop that also passes `out` to a call, binds, stores or returns it, or an append to a place other than a local or a `self` field (`rec.body = rec.body + row`). Each iteration copies the whole String; read the local after the loop, build in a local, or use a `BytesBuilder`.

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

### Loops That Must Vectorize (`@vectorize`)

A kernel that silently stops vectorizing still computes the right answer, several times slower, and no test notices. `@vectorize` on a fn makes that a build failure: every innermost loop written in its body (an element chain's loop included) must be vectorized by the build's optimizer on the build's target. `@vectorize(min_width: N)` also asks for at least `N` lanes.

```hale
@vectorize
fn total(xs: bounded[Int; 1024]) -> Int {
    let mut s = 0;
    for x in xs {
        s = s + x;
    }
    return s;
}
```

It asks the optimizer for nothing and changes nothing the program computes; it reads what LLVM's loop vectorizer reported about each loop and fails the build at the loop that was not vectorized, with the target and the reason:

```text
main.hl:5:5: codegen error: `@vectorize` on `until`: this loop does not vectorize on arm64-apple-darwin (apple-m1): its trip count is not known when the loop starts (an exit that depends on what the loop computes) (LLVM: could not determine number of loop iterations)
```

What no target vectorizes, `hale check` refuses first, at the place that stops the loop: a `break` or `return` beside the loop's own exit, a call to a fn with effects (printing, I/O, time, an allocating fn), an allocation or a locus built in the loop, a publish or send, and a call to a fn that loops itself.

Points to know:

- **The verdict is the target's.** A `Float` sum keeps its written order, so it vectorizes on aarch64 (which has an in-order vector reduction) and not on x86-64. A 64-bit integer multiply has no fast vector form on NEON. Mark the kernels that are meant to vectorize on the targets you ship, and build for those targets.
- **Only innermost loops are held.** An outer loop is never vectorized, so it may do anything between its inner loops.
- **It is checked where the pipeline vectorizes.** A dev build (`--dev`, `HALE_DEV`), `hale test` (both O1), an AddressSanitizer build and a wasm build do not check it; `hale build` prints a note when it skipped.
- **The optimized program is what is judged.** An inlined `@vectorize` fn is judged in every copy; a loop the optimizer removed entirely passes.

See spec/semantics.md § Vectorized loops for the full rules.

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
| `std::diag::heap_alloc_count()` | Returns the thread's cumulative heap allocation count; assert a delta of `0` across a steady-state test loop. |
| `std::diag::syscall_count(name)` | Returns the count of a named syscall (e.g., `"recv"`) to verify exact I/O batching in tests. |

For live arena residency profiling (`LOTUS_ARENA_RESIDENCY=1`, `LOTUS_ARENA_LOG_CHUNK_ATTACH=N`, `LOTUS_CHUNK_POOL_STATS=1`), see [Operations & Debugging](./operations.md).

## Build-Time Optimization Flags

By default, `hale build` and every other verb compile native binaries at `-O3` tuned for the host CPU (`--target-cpu native`), enabling host-specific SIMD autovectorization (such as AVX-512 or SVE). A binary is built on the machine that runs it (the habitat installs the toolchain on each host and builds there), or for a machine you name. `hale build`'s `built:` line says which CPU the binary is tuned for.

| Option / Variable | Effect | When to Use |
|---|---|---|
| `--target-cpu native` *(default)* | Optimizes and vectorizes for the build machine's exact CPU. Not portable across microarchitectures. | Local execution and a binary that runs where it is built, or on identical hosts. |
| `--target-cpu portable` | Targets the portable baseline: `x86-64-v3` (AVX2 + BMI2 + FMA) on x86-64, generic on aarch64. `baseline` and `x86-64-v3` are accepted spellings. | A published image or any artifact whose machine is unknown: it then runs on any machine of its architecture that meets the floor. |
| `--target-cpu <cpu>` | One CPU by LLVM's name (`znver4`, `neoverse-v2`, `apple-m3`, `x86-64-v4`). A name the compiler's LLVM does not know is refused before codegen, listing the CPUs it knows. | A binary built here for a known machine elsewhere. |
| `LOTUS_LTO=1` | Enables full Link-Time Optimization across generated code and the C runtime (requires `lld` on `PATH`; links ~3–4x slower). | Release and benchmark builds (`LOTUS_LTO=1 hale build myservice/`). |

### Building a container image

An image runs on whatever machine pulls it, so build the binary it holds with `--target-cpu portable`. A host-tuned build inside a container build or on CI prints a `warning:` saying so, because the runner's CPU is rarely the one the image lands on, and a binary that uses an extension that CPU lacks dies with `SIGILL`:

```dockerfile
FROM ubuntu:24.04 AS build
# … install the hale toolchain …
COPY . /src
RUN hale build /src -o /out/app --target-cpu portable

FROM ubuntu:24.04
COPY --from=build /out/app /app
ENTRYPOINT ["/app"]
```

## Runtime Performance Profile

Hale's codegen and runtime are optimized around message dispatch, `@form` collections, and tight numerical loops:
- **Subject-keyed bus dispatch**: Static devirtualization and lock-free queues outperform Go channels by ~2.4x and hand-written C dispatch tables by ~1.9x on matched fan-out workloads. `where async_io` subscribers reuse coroutine stacks from a bounded per-worker pool.
- **Tight loops and `@form` access**: Native LLVM codegen matches `clang -O3` within ±15% on arithmetic loops, `@form(vec)` reads, and `@form(hashmap)` lookups, while contended `@form(hashmap)` writes run ~1.3–1.4x faster than hand-rolled C tables.
- **Known overheads**: Cross-function call protocol (~1.25x behind C/Rust), locus instantiation (2–3x due to arena setup), and heap-payload copying. Resolve handles at boot, keep hot state in `@form` slots, and spawn loci at topology changes rather than per message.

Full benchmark suites and methodology are maintained in [hale-lang/bench](https://github.com/hale-lang/bench).
