# Introduction

Hale is an LLVM-compiled systems and service programming language designed around **hierarchical arena ownership**, **actor-like components (`locus`)**, **typed message channels (`topic`)**, and **compile-time architectural verification (`claims`)**.

Hale compiles directly to native machine code (or WebAssembly) with no garbage collector, no borrow checker, and no shared mutable state across threads:

- **Single-tree memory ownership:** Every stateful component is a `locus` that owns a contiguous memory arena. Child loci allocate from their parent's arena and free their entire region in $O(1)$ time when they dissolve.
- **Data-race-free concurrency:** Loci communicate either vertically (parent calling child methods) or laterally via a typed pub/sub message bus (`topic`). Payloads are copied into the receiving locus's arena or passed across zero-copy shared-memory rings (`shm_ring`).
- **Deployment separated from logic:** Thread assignment (`placement`), cross-process transports (`bindings`), and architectural invariants (`claims`) are declared on the entrypoint (`main locus` or `fleet locus`) rather than hardcoded inside components.
- **Whole-program static verification:** Because component ownership, message routing, and side effects are first-class syntax, `hale check` derives a complete graph of the program (`ApplicationModel`) and checks reachability laws, effect budgets, and memory bounds at compile time.

> **Interactive Playground:** Run and edit Hale programs directly in your browser at [play.hale-lang.org](https://play.hale-lang.org/), or browse runnable examples in the [example gallery](https://hale-lang.org/play/).

---

## Mental Model for Experienced Programmers

| If you know... | How Hale compares |
|---|---|
| **Go** | Replace goroutines + channels + mutexes with `locus` + `topic`. Thread pools (`cooperative`, `pinned`, `where async_io`) are configured once in `main`'s `placement { }` block. Errors use `fallible(E)` + `or` instead of `if err != nil`. |
| **Rust** | Memory is managed by hierarchical arenas tied to locus lifetimes (`birth` $\to$ `run` $\to$ `dissolve`) and per-call scratch regions, avoiding lifetime annotations and borrow checking while remaining GC-free. |
| **Erlang / Elixir (OTP)** | A `locus` is a statically typed process/actor with built-in supervision (`closure` invariants + `on_failure { restart; }` policies) and native C-speed execution. |
| **TypeScript / Python** | Simple scripts start with a 3-line `fn main()`, first-class `Decimal` and `Duration` literals, built-in JSON/HTTP/SQLite/Postgres support, and type inference on local bindings. |

---

## Core Language Vocabulary

Every Hale program is built from a small, orthogonal set of declarations:

| Construct | Purpose | Equivalent Concept |
|---|---|---|
| `type` | Pure value record or union passed by value; has fields and methods, no lifecycle | `struct` / `enum` / data class |
| `locus` | Stateful component with an arena, `params`, methods, lifecycle hooks, and bus wiring | Actor / service / state machine |
| `@form(...)` | Compiler-specialized data structure (`vec`, `hashmap`, `set`, `ring_buffer`, `lru_cache`, `heap`) backed by a locus's arena | `Vec<T>`, `HashMap<K,V>`, `BTreeSet<T>` |
| `topic` | Named, typed pub/sub channel (`Topic <- msg;`, `subscribe Topic as handler;`) | Typed Kafka/NATS subject or Go channel |
| `seam` | Linear typed pipeline routing a set of message types across loci and methods | Typed middleware / railway pipeline |
| `@repo(T)` | Locus managing keyed persistence (`get`, `create`, `put`, `remove`) with state effect tracking | Repository / DAO with compile-time CRUD effects |
| `api` | Typed RPC/streaming contract served over HTTP, REST, Unix sockets, gRPC, MCP, or WebSockets | Protobuf / OpenAPI service definition |
| `fallible(E)` | Value-level error channel on a function signature, handled at the call site with `or` | `Result<T, E>` with mandatory handling |
| `closure` & `on_failure` | Runtime state invariant on a locus + parent supervision policy (`restart`, `bubble`) | OTP supervisor strategy |
| `@effects` & `@budget` | Transitive compile-time bounds on syscalls, blocking, allocations, and domain effects | Static capability / effect system |
| `claims` & `constitution` | Compile-time architectural assertions (isolation, gating, cardinality) over the program graph | Zero-cost static architecture tests |
| `main locus` | Process entrypoint specifying child `placement`, transport `bindings`, and `claims` | `main()` + thread & transport wiring |
| `fleet locus` | Multi-binary composition specifying host placement, cross-process bindings, and fleet claims | Type-checked Docker Compose / K8s manifest |

---

## Code Tour: From Script to Service

A minimal Hale program requires only `fn main()`:

```hale
fn main() {
    let unit_price = 19.99d;
    let total = unit_price * 3.0d;
    println("Total: $", total);
}
```

When your program needs long-lived state and reactive messaging, you define data with `type`, channels with `topic`, and components with `locus`:

```hale
type Player    { id: String; name: String; }
type MatchInfo { match_id: String; size: Int; }

topic JoinQueue  { payload: Player; }
topic MatchReady { payload: MatchInfo; }

locus Matchmaker {
    params {
        target_size: Int = 4;
        queued:      Int = 0;
    }
    bus {
        subscribe JoinQueue as on_join;
        publish   MatchReady;
    }

    fn on_join(p: Player) {
        self.queued = self.queued + 1;
        if self.queued >= self.target_size {
            MatchReady <- MatchInfo { match_id: p.id, size: self.queued };
            self.queued = 0;
        }
    }
}
```

`Matchmaker` does not know whether `JoinQueue` arrives from an in-memory queue, a Unix domain socket, a shared-memory ring buffer, or NATS JetStream, nor which CPU core or thread pool it runs on. Those decisions live in `main locus`, so the component itself stays portable across single-binary tools, multi-process daemons, and WebAssembly targets.

---

## Documentation Map

1. **[Part I: Language & Standard Library](./parts/language.md)** — Primitive types, `Decimal` money and `Duration` time, physical units, functions, control flow, `fallible(E)` error handling, records, `@form` collections, and standard library packages (files, JSON, HTTP, sockets, crypto, CLI, logging, metrics, testing).
2. **[Part II: Concurrency, Services & APIs](./parts/locus-model.md)** — Locus lifecycle (`birth`, `run`, `drain`, `dissolve`), the typed bus, thread placement (`cooperative`, `pinned`, `async_io`), parent-child supervision, seams, `@repo(T)` repositories, `api` surfaces (RPC, REST, MCP, gRPC, WebSockets), and multi-binary `fleet locus` deployments.
3. **[Part III: Static Verification & Architecture Laws](./parts/what-must-hold.md)** — Effect annotations (`@effects`, `@no_syscall`, `@no_block`, `@deterministic`), whole-program `claims`, and reusable `constitution` rules.
4. **[Part IV: Systems Programming & Runtime Internals](./parts/systems.md)** — Arena memory layouts, zero-allocation hot paths (`@hot`, `@budget`), zero-copy `shm_ring` messaging, C FFI (`@ffi("c")`), WebAssembly, live topology inspection (`hale iris`), and deterministic replay (`hale replay`).
5. **[Part V: DNA (Autonomous & Governed Applications)](./parts/organism.md)** — Building self-governing applications with durable PostgreSQL + Git state, NATS event routing, Prometheus health reflexes, Vault secret confinement, and sandboxed agent/human task workflows.
6. **[Part VI: Multi-Record Control Plane (Habitat)](./parts/habitat.md)** — Managing local and remote multi-service environments with `hale habitat`.

### Next Steps

- **[Install the Toolchain](./getting-started/install.md)** — Download a prebuilt binary or build from source.
- **[Quickstart & Toolchain](./getting-started/first-run.md)** — Compile and run your first `.hl` program and explore the CLI.
- **[Tutorial: Build a Job Queue](./tutorial/job-queue.md)** — Evolve a 10-line script into a multi-process service in four steps.
