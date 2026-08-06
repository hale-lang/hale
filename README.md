# Hale

**You describe a system — the services, the messages between them, who
owns what — and that description *is* the program.**

One primitive, the **locus**, scales from a single function to a fleet of
services wired over a typed message bus. There's no translation layer
between the sentence you'd say out loud and the code you write.

**[hale-lang.org](https://hale-lang.org)** — docs, [playground](https://hale-lang.org/playground), packages, [features](https://hale-lang.org/features).

[![Tests](https://github.com/hale-lang/hale/actions/workflows/tests.yml/badge.svg)](https://github.com/hale-lang/hale/actions/workflows/tests.yml)
[![Docs](https://github.com/hale-lang/hale/actions/workflows/docs.yml/badge.svg)](https://hale-lang.org/docs)
[![License](https://img.shields.io/badge/license-Apache_2.0-blue.svg)](./LICENSE)
[![LLVM](https://img.shields.io/badge/LLVM-18-red.svg)](https://llvm.org/)

You know the feeling: you describe a service out loud — *"a chat room takes
each message posted to it and relays it out to everyone in the room"* — and
the code you actually write bears no resemblance to the sentence. A
connection registry. A member list, and a lock around it. A broadcast loop.
Async plumbing. By the time it works, the idea you started with is buried.
**Hale is a bet that the gap doesn't have to be there.**

## A chat room, in Hale

```hale
type Msg { room: String; user: String; text: String; }

topic Posted    { payload: Msg; keyed_by room; }   // routed by room name
topic Broadcast { payload: Msg; }

locus Room {
    params { name: String = "lobby"; }
    bus {
        subscribe Posted as on_post where key == self.name;
        publish   Broadcast;               // fan it out to everyone here
    }

    fn on_post(m: Msg) {
        Broadcast <- m;                    //  <-  sends on the bus
    }
}
```

Every phrase from the description has a home, in the order you thought it:

- *"a chat room"* → `locus Room`
- *"each message posted to it"* → `subscribe Posted as on_post`
- *"in the room"* (only this room's traffic) → `keyed_by room` +
  `where key == self.name` — the bus itself is the routing table, so a
  message for `"lobby"` is delivered only to the lobby `Room`, and the
  handler body never filters
- *"relays it out to everyone"* → `publish Broadcast` / `Broadcast <- m`,
  and the bus fans it out to every subscriber

No connection registry, no member list to lock, no broadcast loop, no
`async`/`await`, no lifecycle wiring — and no dispatch code either: with
ten rooms, a posted message costs one delivery, not ten filtered ones.
You wrote down the idea; the idea is the program. (Rooms here are wired
at startup; a lobby that mints brand-new rooms at runtime still declares
them — dynamic subject creation is on the roadmap.)

> GitHub can't syntax-highlight Hale yet, so the snippets here render in a
> single color. For highlighted, runnable Hale, open the
> [playground](https://play.hale-lang.org/).

## One primitive, at any altitude

Most languages pick a level and stay there — Python and JavaScript high, Go
in the middle, Rust and C++ low. Hale is one language you write at any of
them, moving between levels without changing tools. There's a single
building block — the **locus** — and the only thing that changes as you go
down is how much of it you choose to see.

| Altitude | You write… | Feels like… |
|---|---|---|
| **The basics** | variables, math, functions, control flow | a clean scripting language |
| **Everyday programs** | files, JSON, HTTP, loci as objects | Python / Node |
| **Concurrent services** | a typed bus, lifecycle, supervision | Go / Erlang |
| **Systems control** | memory layout, lifetime, zero-copy I/O, C bindings | Rust / C++ |

A function you wrote at the top still works at the bottom — you've just
learned to see more of what was always there. The
[docs](https://hale-lang.org/docs) are organized as exactly this
descent, so you go only as deep as you need.

## Deploy the same system anywhere — by editing `main`

The loci describe *what your system is*. A single **`main` locus** describes
*where it runs and how its messages travel* — and nothing else in the
program mentions a thread or a transport:

```hale
main locus App {
    params {
        region_us: GameRegion     = GameRegion { name: "us-east" };
        region_eu: GameRegion     = GameRegion { name: "eu-west" };
        sessions:  SessionWorkers = SessionWorkers { };
        metrics:   MetricsServer  = MetricsServer { port: 9100 };
    }

    placement {
        region_us: pinned(node = 0);                       // thread + memory on NUMA node 0
        region_eu: pinned(node = 1);                       // a sibling, on the other node
        sessions:  cooperative(pool = ws) where async_io;  // 1 thread, thousands of players
        metrics:   cooperative(pool = io);                 // shares the io pool
    }

    bindings {
        MatchReady:    unix("/run/match.sock");                       // AF_UNIX, role inferred
        WorldSnapshot: shm_ring("/world", slot_count: 1024, on_overflow: drop)
                      where intra_machine, zero_copy;                 // shared memory, no copy
        ChatRelay:     NatsAdapter { url: "nats://chat:4222" };       // a locus you wrote
        Replay:        unix("/run/replay.sock") codec(JsonCodec { }); // JSON on the wire
    }
}
```

Not one line of `GameRegion`, `SessionWorkers`, or `MetricsServer` changes
whether `MatchReady` is an in-process queue or a Unix socket, or whether
`region_us` owns a NUMA node or shares the main thread. You design the
system once and redeploy it — test, single binary, many hosts — by editing
`main`.

This isn't aspirational — the same source runs as a test, one binary, or a
mesh of binaries wired over sockets and shared memory, with the loci
themselves oblivious to how they're deployed. Only `main` changes.

And you can redeploy a system **while it runs.** A `perspective` is a live,
swappable handle to a contract; `reperspective` re-points it at a new
implementation with a single atomic store — hot code-swap at pointer-flip
cost, no restart, the running state carried across:

```hale
reperspective self.router as RouterV2;   // every caller sees V2 on its next call
```

`topology { }` to describe the machine, `placement { }` to map components
onto its cores and memory, `reperspective` to redeploy them live —
Kubernetes-shaped, in a single address space, at nanosecond cost.

It all comes from one idea — **you declare intent, and the compiler picks
the mechanism** — applied on every axis where other languages make you
hand-pick:

| You write… | …the compiler picks |
|---|---|
| `topic` + `bus { subscribe / publish }` | in-process queue, socket, shared-memory ring, or a broker adapter |
| `placement { }` / `topology { }` | a shared pool, a dedicated thread, a pinned core, a NUMA node |
| `@form(vec / hashmap / ring_buffer / lru_cache)` | a tight, type-specialized container |

The choices easy to get wrong — which lock, which container, which
transport — stop being choices you make at the call site.

## What you don't write

A lot of the appeal is what *isn't* there to trip over — or to make a
coding model hallucinate:

- **No `class`, `module`, `package`** — the **locus** is all of them. Apps,
  services, caches, handlers, libraries: all loci.
- **No `Vec<T>` / `Map<K,V>` ceremony** — declare a collection with `@form`
  on a locus and get `push` / `get` / `len` synthesized, type-specialized to
  your element.
- **No `async` / `await`** — concurrency lives on the typed bus and the locus
  lifecycle. No function-coloring problem, because there are no async
  functions to color.
- **No GC, and no borrow checker** — the locus hierarchy is explicit, so
  cleanup is deterministic when a locus dissolves. You never write `free`,
  and you never fight a lifetime annotation.
- **No exceptions, no `panic` / `assert`** — a call that can fail says so in
  its type, and you address it right at the call site. Nothing propagates
  invisibly.

## Say what a function may *do*

A signature tells you the types and nothing else — not whether a function
touches the filesystem, blocks on a socket, reads the clock, or allocates.
Hale lets you say so, and holds you to it:

```hale
@no_syscall @deterministic
fn price(book: OrderBook, qty: Int) -> Decimal { ... }
```

That's a contract, not a hint. The compiler proves it across everything
reachable — through helpers, through methods called on a handle, into
imported libraries, into the standard library — and a violation names the
path that gets there, not just the function:

```
effect assertion violated: `price` must not reach `syscall`, but reaches
  price -> Book::reload [std::io::fs::read_file — a syscall-class operation].
```

The classes are `syscall`, `block`, `time`, `entropy`, `env`, `ffi`,
`publish`, `spawn`, `recursion` and `alloc`. They compose into a one-line
hot-path certificate — nothing that waits, nothing that reaches the kernel,
nothing non-deterministic, no allocation:

```hale
@no_block @no_syscall @deterministic @no_recursion @hot
@budget(alloc_per_call = 0)
fn on_tick(a: Int, b: Int) -> Int { ... }
```

It's entirely opt-in: a program with no annotations behaves exactly as
before. And because the compiler already infers what every function does, it
will also *report* that — so a handler that quietly starts doing filesystem
I/O is a one-line diff in review, even though nothing annotated changed.
[Effects & contracts →](https://hale-lang.org/docs/effects)

## State your architecture as law

Effects bind one function at a time. Architecture is a property of the
*whole* graph — "billing must never reach research", "exactly one thing
writes settlements" — and saying that with per-function contracts means
scattering them across every position and hoping you got them all.

A **claim** is one named sentence about the program graph, declared in
`main` and checked by `hale check` as an error:

```hale
import "../pay" as pay;
import "../research" as research;

group billing  = { pay::* };
group research_wing = { research::Sandbox };

main locus Org {
    params { ledger: pay::Ledger = pay::Ledger { }; }
    claims {
        tenant_iso: forbid reaches(billing, research_wing);
        one_writer: count publishers(topic pay::Settled) == 1;
    }
}
```

Violate one and you get a minimal countermodel in your own spelling —
never a mangled symbol, never a rule number:

```
claim `tenant_iso` violated: `billing` reaches `research_wing` — witness:
  `pay::Ledger::on_close` -(publishes "pay::Audit")-> `research::Sandbox::on_audit`
```

…followed by the three places you'd actually edit: the publish that
crosses the boundary, the subscription that receives it, and the
declaration of the destination.

Six forms cover the shapes worth stating. `forbid reaches(A, B)` for
absence, optionally narrowed `via { calls }` or `via { bus }`;
`require subscribes(some G, topic T)` for existence; `count
publishers(topic T) <= 1` for cardinality; `cover topic in seed(a):
subscribed_by(some G)` so a new topic can't be quietly orphaned;
`only edges A -> B { publish T; }` for a reviewable boundary
inventory; and `bound alloc <= N on paths from G` for cost.

Three things make this hold up in practice:

- **Groups are vocabulary, not patterns.** A misspelt member is an
  error with a did-you-mean, and an empty group is a vacuity error
  unless it says `may_be_empty` — a `forbid` satisfied by an empty
  set is a fail-open wearing formal clothing.
- **Unknown means violation.** An indirect call the compiler can't
  resolve refuses to certify rather than proving absence it can't see.
- **Libraries carry their own laws.** A seed states claims about what
  it can see, and they travel into every application that imports it —
  so an app that wires a second subscriber onto a library topic breaks
  the *library's* `count` claim, attributed to the library's own line.

Claims cost nothing at runtime — they compile to no code. And the graph
they were evaluated against is exportable, so review and CI can gate on
it directly:

```sh
hale check .                              # claims are errors
hale check . --dump-topology=topo.json    # the model, as JSON
hale check . --check-topology-shape topo.json   # fail if the graph moved
```

[Claims →](https://hale-lang.org/docs/claims)

## Verified where it counts

The substrate you stand on is checked, not hoped. Every concurrent primitive
in the runtime — the lock-free map, the mailbox, the bus queue, the arena —
is **model-checked under every legal thread interleaving**
([GenMC](https://github.com/MPI-SWS/genmc)) on each CI run. Above it, the
compiler walks your bus topology as a typed graph at build time: orphaned
topics, re-entrant cycles, unbounded backpressure, and payload
type-mismatches are caught before the program runs.

You don't get a "verified" sticker on your whole program. You get a
foundation whose coordination can't silently race — and because messages are
copies and loci never reach sideways, programs that are **data-race-free by
construction**, with no GC and no borrow checker.
[Verification →](https://hale-lang.org/docs/verification)

## Built for humans and models

The small surface and the missing footguns aren't only pleasant to read —
they're what make Hale unusually easy for a coding model to *write*. There
are no async functions to mis-color, no lifetimes to get wrong, no lock to
pick; the shapes a model tends to hallucinate simply aren't in the language.

You can feel the fit before installing anything: drop this repo's
[`AGENTS.md`](./AGENTS.md) into your coding assistant and ask it to re-read a
module from your own codebase **as loci, contracts, and bus topics**. What
comes back is usually a decomposition that matches your mental model —
because it's reasoning in the same vocabulary you already use about your
system.

## Try it

**No install — [write and run Hale in your browser](https://play.hale-lang.org/).**
Your source is compiled to WebAssembly by a compile service written in
Hale itself, and the result runs fully client-side — the same compiler
you install locally. Prefer a guided start? The
[example gallery](https://hale-lang.org/play/) walks curated programs,
each precompiled from real Hale (the gallery UI is itself a Hale
`@export locus` — the same `.hl` source runs native or in the browser).

**Prebuilt Linux and macOS binaries** are on the
[releases page](https://github.com/hale-lang/hale/releases) — download,
extract, put `hale` on your `PATH`. Or build from source:

```sh
git clone https://github.com/hale-lang/hale
cd hale
cargo build --release   # needs Rust 1.95+, LLVM 18, clang, git
```

```hale
// hello.hl
fn main() { println("Hello from Hale."); }
```

```sh
hale run   hello.hl          # compile + run
hale build hello.hl && ./hello
```

**Editor & agent integration** is one command:

```sh
hale fmt                     # canonical formatter (zero config; --check for CI)
hale verify                  # check + FAIL on any advisory — the CI discipline gate
hale doc                     # API reference from /// doc comments (--json for agents)
hale bench                   # run *_bench.hl benchmarks — ns/op + allocs/op
hale lsp                     # stdio Language Server — live diagnostics
hale mcp                     # stdio MCP server — the same tools for shell-less agent hosts
```

Point any LSP-speaking editor (or coding-agent harness — they speak
LSP natively now) at `hale lsp` and you get the full `hale check`
surface as you type: type errors, plus the analyses no generic
tooling has — the unbounded-allocation survey, the hot-path
allocation lint, placement/starvation warnings. Hover shows a
symbol's signature *with its contracts*: fallibility
(`fallible(IoError)`), `@hot` / `@budget` enforcement status,
a topic's routing key. Completion offers `self.` members with
signatures, the `std::` surface namespace-by-namespace, and the
seed's own symbols. Go-to-definition, find-references, document
outlines, and format-on-save (the same canonical form as
`hale fmt`) work across the seed. And the custom methods return what agents
otherwise grep for: `hale/busGraph` (the whole message topology —
who publishes and subscribes every topic, with placements),
`hale/placement` (every component's thread/pool assignment), and
`hale/allocSummary` (the allocation-bound survey's leak sites,
with positions). The whole program re-checks in ~10 ms
per keystroke, so there's no indexing step, no warm-up, no
configuration. (Scripted integrations can use
`hale check app.hl --json` — one JSON object per diagnostic —
instead.) Agent hosts without a shell (Claude Desktop, MCP
clients) get the same toolchain via `hale mcp`: check/verify/
build/run/test/bench/fmt/doc as typed tools, the bus-graph/
placement/enforcement analyses as direct calls, and a spec
search over the language specification embedded in the binary —
`claude mcp add hale -- hale mcp` and there is nothing else to
install or keep in sync.

Platform-specific setup (Linux, macOS/Apple Silicon) is in
[the install guide](./docs/src/getting-started/install.md).

## Where the language stands

The language surface has taken **no breaking changes since v0.10.0
(2026-07-07)** — everything since has been additive (`@hot` / `@budget`
enforcement, `match` expressions, String routing keys) plus runtime
fixes. It's pre-1.0 because the frontier below is still moving.

The proven core is the typed topic bus, `placement` / `bindings` deployment,
`@form` collections, structural `interface`s, `@ffi` C bindings, and the
`fallible(T)` error model — all self-hosted by the native compiler. The
**frontier**: NUMA-aware `topology` placement with `replicas`, and live
`reperspective` hot-swap. (`mode` projections and `closure` assertions round
out the surface; reach for them when your problem calls for them.)

**Performance, scoped honestly:** Hale is faster than Go at message
dispatch, JSON parsing, and `@form` collections — and slower at raw
function-call and spawn overhead. From the cross-language snapshot
(Hale v0.9.0 grid, 2026-06-30, Ryzen 7 9800X3D; the same workload shape
in each language):

| Bench | Hale | Go | vs Go |
|---|---:|---:|---|
| `bus_dispatch` (100k typed messages) | 196 µs | 471 µs | **2.4× faster** |
| `json_parse` (200k 7-field parses) | 58.0 ms | 150.0 ms | **2.6× faster** |
| `form_vec_push` (500k) | 573 µs | 2.76 ms | **4.8× faster** |
| `loop_overhead` (100M xor-reduce) | 1.59 ms | 19.7 ms | **12.4× faster**\* |
| `fn_call` (10M free-fn calls) | 19.1 ms | 7.7 ms | 2.5× slower |
| `locus_instantiation` (100k) | 1.25 ms | 153 µs | 8.2× slower |
| `coord_with_churn` (2000 children) | 42.8 µs | 2.4 µs | 18× slower |

\* Not dead code on either side (both xor-accumulate and print the
result) — but LLVM autovectorizes the reduction to AVX-512 while Go
compiles it scalar, so it measures vectorization on reducible loops,
not general loop speed.

The split is the design showing through: every locus owns an arena, so
calls and spawns pay region setup that dispatch and collections
amortize away. On the roadmap: a cheaper call protocol and spawn path,
and extending static devirtualization across pipeline hops. The full
grid — including the losses, plus C and Rust comparators and
reproduction instructions — lives in
[hale-lang/bench](https://github.com/hale-lang/bench).

## Opinionated by design

There's no permissive escape hatch, and that's the feature. **One form per
locus** — you compose at the locus level, not inside it. **Failures travel
only vertically** — a parent decides recovery for its children; nothing fails
sideways. **An invariant you care about is a `closure` the runtime audits**,
not a comment you hope someone reads. If your problem decomposes cleanly into
loci + bus, you move fast. If it doesn't, the language tells you so — early,
at compile time.

## The names

They mean things, and they fit together:

- **hale** — the language. From the Old English *hāl*: "whole, sound,
  uninjured." Same root as *whole*, *heal*, *health*.
- **lotus** — the runtime substrate. C-runtime symbols are `lotus_*`.
- **pond** — the contributed library catalog (web, databases, observability,
  AI clients), much of it thin `@ffi` bindings to C libraries and `interface`
  seams you swap. *Many lotus grow in a pond.*
- **heron** — the tree-sitter grammar, now at
  [tree-sitter-hale](https://github.com/hale-lang/tree-sitter-hale);
  editor highlighting drinks from it (the LSP ships in the `hale`
  binary itself).

## Where to go next

- **[Docs site](https://hale-lang.org/docs)** — the level-by-level
  tour. Start here.
- **[`spec/`](./spec/)** — the canonical reference; the compiler enforces
  what it describes.
- **[`AGENTS.md`](./AGENTS.md)** — the load-bearing prompt for coding models
  writing `.hl` (and a tight read for humans).
- **[Examples](./crates/hale-codegen/tests/fixtures/examples/)** — 83
  working example programs (88 `.hl` files), compiled and run in CI.
- **[pond](https://github.com/hale-lang/pond)** · contributed libraries.
  **[CONTRIBUTING](./CONTRIBUTING.md)** · how to build + send a change.
  **[Issues](https://github.com/hale-lang/hale/issues)** · questions, ideas,
  bugs.

Why one shape carries across native, browser, human, and model is written up
in [hale-lang/papers](https://github.com/hale-lang/papers).

## License

[Apache License 2.0](./LICENSE). Third-party notices in [`NOTICE`](./NOTICE).
