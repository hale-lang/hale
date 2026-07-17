# Hale

**You describe a system — the services, the messages between them, who
owns what — and that description *is* the program.**

One primitive, the **locus**, scales from a single function to a fleet of
services wired over a typed message bus. There's no translation layer
between the sentence you'd say out loud and the code you write.

[![Tests](https://github.com/hale-lang/hale/actions/workflows/tests.yml/badge.svg)](https://github.com/hale-lang/hale/actions/workflows/tests.yml)
[![Docs](https://github.com/hale-lang/hale/actions/workflows/docs.yml/badge.svg)](https://hale-lang.github.io/hale/)
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
> [playground](https://hale-lang.github.io/hale/play/).

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
[docs](https://hale-lang.github.io/hale/) are organized as exactly this
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
[Verification →](https://hale-lang.github.io/hale/verification.html)

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

**No install — [run Hale in your browser](https://hale-lang.github.io/hale/play/).**
The playground is real Hale, compiled to WebAssembly, running on the page
(the UI itself is a Hale `@export locus` — the same `.hl` source runs native
or in the browser).

**Prebuilt Linux binaries** are on the
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
hale lsp                     # stdio Language Server — live diagnostics
```

Point any LSP-speaking editor (or coding-agent harness — they speak
LSP natively now) at `hale lsp` and you get the full `hale check`
surface as you type: type errors, plus the analyses no generic
tooling has — the unbounded-allocation survey, the hot-path
allocation lint, placement/starvation warnings. The whole program
re-checks in ~10 ms per keystroke, so there's no indexing step, no
warm-up, no configuration. (Scripted integrations can use
`hale check app.hl --json` — one JSON object per diagnostic —
instead.)

Platform-specific setup (Linux, macOS/Apple Silicon) is in
[the install guide](./docs/src/getting-started/install.md).

## Where the language stands

The language surface is **stable** — most work from here is bugs,
performance, and polish, not new syntax.

The proven core is the typed topic bus, `placement` / `bindings` deployment,
`@form` collections, structural `interface`s, `@ffi` C bindings, and the
`fallible(T)` error model — all self-hosted by the native compiler. The
**frontier**: NUMA-aware `topology` placement with `replicas`, and live
`reperspective` hot-swap. (`mode` projections and `closure` assertions round
out the surface; reach for them when your problem calls for them.)

**Performance** is a lead, not a cost: at matched workloads, message dispatch
and `@form` collections run ahead of Go after the lock-free bus and
static-dispatch devirtualization, and native codegen brings tight loops to
parity with `clang -O3`. Methodology and current numbers live in
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
- **heron** — the tree-sitter grammar; editors and the future LSP drink from
  it.

## Where to go next

- **[Docs site](https://hale-lang.github.io/hale/)** — the level-by-level
  tour. Start here.
- **[`spec/`](./spec/)** — the canonical reference; the compiler enforces
  what it describes.
- **[`AGENTS.md`](./AGENTS.md)** — the load-bearing prompt for coding models
  writing `.hl` (and a tight read for humans).
- **[Examples](./crates/hale-codegen/tests/fixtures/examples/)** — ~70
  working `.hl` programs.
- **[pond](https://github.com/hale-lang/pond)** · contributed libraries.
  **[CONTRIBUTING](./CONTRIBUTING.md)** · how to build + send a change.
  **[Issues](https://github.com/hale-lang/hale/issues)** · questions, ideas,
  bugs.

Why one shape carries across native, browser, human, and model is written up
in [hale-lang/papers](https://github.com/hale-lang/papers).

## License

[Apache License 2.0](./LICENSE). Third-party notices in [`NOTICE`](./NOTICE).
