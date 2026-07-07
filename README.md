# Hale

**A concurrent systems language with a model-checked, GC-free runtime** — typed message-bus concurrency, data-race-free by design.

[![Tests](https://github.com/hale-lang/hale/actions/workflows/tests.yml/badge.svg)](https://github.com/hale-lang/hale/actions/workflows/tests.yml)
[![Docs](https://github.com/hale-lang/hale/actions/workflows/docs.yml/badge.svg)](https://hale-lang.github.io/hale/)
[![License](https://img.shields.io/badge/license-Apache_2.0-blue.svg)](./LICENSE)
[![LLVM](https://img.shields.io/badge/LLVM-18-red.svg)](https://llvm.org/)
[![Status](https://img.shields.io/badge/status-stabilizing-blue.svg)](#status)
[![GC](https://img.shields.io/badge/GC-0-brightgreen.svg)](#what-hale-leaves-out)
[![async/await](https://img.shields.io/badge/async%2Fawait-0-brightgreen.svg)](#what-hale-leaves-out)
[![native](https://img.shields.io/badge/native-human_%2B_agent-8957e5.svg)](./AGENTS.md)

**A language whose shape matches the shape of your thinking.**

You know that feeling when you describe a system out loud —
*"a matchmaker holds a queue of waiting players, spawns a match when
enough are queued, then goes back to waiting"* — and then the code
you actually write bears no resemblance to those words? Mutexes
appear. Async machinery. Lifecycle wiring. Five files. By the time
it works, the sentence you started with is buried.

Hale is a bet that the gap doesn't have to be there.

## A matchmaker, in Hale

<!-- Rendered SVG (GitHub can't highlight `hale` itself). Source:
     assets/readme/matchmaker.hl; regenerate with
     `python3 tools/hale_svg.py assets/readme/matchmaker.hl assets/readme/matchmaker.svg`.
     The copyable source is in the <details> below; keep the two in sync. -->
![A matchmaker, in Hale](assets/readme/matchmaker.svg)

<details>
<summary>Source</summary>

```hale
type Player    { id: String; name: String; }
type MatchInfo { match_id: String; players: [Player]; }

topic JoinQueue  { payload: Player; }
topic MatchReady { payload: MatchInfo; }

locus Matchmaker {
    params { target_size: Int = 4; }
    bus {
        subscribe JoinQueue as on_join;
        publish   MatchReady;
    }

    fn on_join(p: Player) {
        self.waiting.push(p);
        if self.waiting.len() >= self.target_size {
            MatchReady <- assemble_match(self.waiting, self.target_size);
        }
    }
}
```

</details>

Every phrase from the description has a syntactic home, in the order
you thought them:

- *"a service"* → `locus Matchmaker`
- *"receives players wanting matches"* → `subscribe JoinQueue as on_join`
- *"announces matches"* → `publish MatchReady`
- *"when enough are queued"* → the `if`

That's the whole service. No mutex to choose, no channel types, no
`async`/`await`, no lifecycle wiring, no error handling at every
boundary. You wrote down the idea; the idea is the program.

## Write it at any altitude

Most languages pick a level and stay there — Python and JavaScript
high, Go in the middle, Rust and C++ low. Hale is one language you
write at any of those levels, moving between them without changing
tools. There's a single primitive — the **locus** — and the only
thing that changes as you go down is how much of it you choose to
see.

| Altitude | You write… | Feels like… |
|---|---|---|
| **The basics** | variables, math, functions, control flow | a clean scripting language |
| **Everyday programs** | files, JSON, HTTP, loci as objects | Python / Node |
| **Concurrent services** | a typed bus, lifecycle, supervision | Go |
| **Systems control** | memory layout, lifetime, zero-copy I/O, C bindings | Rust / C++ |

Each level is self-contained. You can stop after the second one and
write real applications; a function you wrote at the top still works
at the bottom — you've just learned to see more of what was always
there. The [docs](https://hale-lang.github.io/hale/) are organized
as exactly this descent, so you go only as deep as you need.

## What Hale leaves out

A lot of the appeal is what *isn't* there to trip over:

- **No `class`, `module`, `package`** — the **locus** is all of
  them. Apps, services, caches, handlers, libraries: all loci.
- **No `Vec<T>` / `Map<K,V>` ceremony** — declare a collection with
  `@form` on a locus and get `push` / `get` / `len` synthesized.
- **No `async` / `await`** — concurrency lives on a typed message
  bus and the locus lifecycle. There's no function-coloring problem
  because there are no async functions to color.
- **No GC, and no borrow checker** — the locus hierarchy is
  explicit, so cleanup is deterministic when a locus dissolves. You
  never write `free`, and you never fight a lifetime annotation.
- **No exceptions, no `panic` / `assert`** — a call that can fail
  says so in its type, and you address it right at the call site.
  Nothing propagates invisibly.

## You declare intent; the compiler picks the mechanism

Each block on a locus states *what you mean* on an axis where other
languages make you hand-pick a mechanism:

- **`topic` + `bus { subscribe/publish }`** — what crosses between
  loci. The compiler/binary picks the transport (in-process queue,
  socket, broker) without changing your code.
- **`placement { }`** — where loci run (a shared cooperative pool or
  a dedicated thread), decided at deployment, not baked into the
  logic.
- **`@form(vec / hashmap / ring_buffer)`** — a collection's access
  discipline; you get a tight, type-specialized implementation.
- **`mode bulk / harmonic / resolution`** — an execution regime; the
  compiler emits vectorized, cache-tiled, or scalar code to match.

The choices that are easy to get wrong — which lock, which
container, which transport — stop being choices you make at the call
site. That's also why the language is unusually pleasant to write
*with* an LLM: the things models hallucinate on aren't in the code.

## Verified by construction

The substrate you stand on is checked, not hoped. Every concurrent
primitive in the runtime — the lock-free map, the mailbox, the bus
queue, the arena — is **model-checked under every interleaving**
([GenMC](https://github.com/MPI-SWS/genmc)) on each CI run. Above it,
the bus topology is a typed graph the compiler walks at build time:
orphaned topics, re-entrant cycles, unbounded backpressure, and
payload type-mismatches are caught before the program runs. You don't
get a "verified" sticker on your whole program — you get a foundation
whose coordination can't silently race. See
[Verification](https://hale-lang.github.io/hale/verification.html).

## Wire the whole system in `main`

Two of those axes — *where loci run* and *how their messages travel*
— come together in the `main` locus, in a `placement { }` block and
a `bindings { }` block. This is the control panel for the entire
program. The loci themselves don't mention threads or transports;
`main` does, in one place:

<!-- Rendered SVG; source assets/readme/placement.hl, regenerate with
     `python3 tools/hale_svg.py assets/readme/placement.hl assets/readme/placement.svg`.
     Copyable source is in the <details>; keep the two in sync. -->
![The main locus: placement and bindings](assets/readme/placement.svg)

<details>
<summary>Source</summary>

```hale
main locus App {
    params {
        region_us: GameRegion     = GameRegion { name: "us-east" };
        region_eu: GameRegion     = GameRegion { name: "eu-west" };
        sessions:  SessionWorkers = SessionWorkers { };
        metrics:   MetricsServer  = MetricsServer { port: 9100 };
        admin:     AdminConsole   = AdminConsole { };
    }

    placement {
        region_us: pinned(core = 1);                       // its own core
        region_eu: pinned(core = 2);                       // a sibling, on another
        sessions:  cooperative(pool = ws) where async_io;  // 1 thread, 1000s of players
        metrics:   cooperative(pool = io);                 // shares the io pool
        // admin is unlisted -> cooperative(pool = main)
    }

    bindings {
        // PlayerInput: not listed -> delivered by an in-process queue
        MatchReady:    unix("/run/match.sock");                       // AF_UNIX, role inferred
        WorldSnapshot: shm_ring("/world", slot_count: 1024, on_overflow: drop)
                      where intra_machine, zero_copy;                 // shared-memory, no copy
        ChatRelay:     NatsAdapter { url: "nats://chat:4222" };       // a locus you wrote
        Replay:        unix("/run/replay.sock") codec(JsonCodec { }); // JSON on the wire
    }
}
```

</details>

**`placement { }` — where each locus runs.** Same-type siblings can
sit on different cores; it keys on the field name, not the type.

- `cooperative(pool = X)` — share pool `X`'s OS thread (the default
  pool is `main`).
- `pinned` / `pinned(core = N)` — a dedicated OS thread, optionally
  pinned to a CPU core.
- `where async_io` — turn a cooperative pool into an event loop, so
  a blocking `recv` *parks* instead of stalling the thread. One
  thread serves thousands of connections.

**`bindings { }` — how each topic is delivered.** The publisher's
`Topic <- value;` and the subscriber's `subscribe Topic` are
identical no matter which line below you pick.

- *(absent)* — an in-process cooperative queue. The default; no
  entry needed.
- `unix("/path")` — AF_UNIX framed bytes; listen/connect role
  inferred from who publishes vs subscribes.
- `udp://host:port` — datagrams, including IPv4 multicast.
- `NatsAdapter { ... }` — any locus you write with a single `send`
  method: NATS, MQTT, a custom broker.
- `codec(JsonCodec { })` — JSON, protobuf, or a custom wire format
  per binding, so a Python or Go peer can read it. Different
  bindings on the same topic can carry different codecs.
- `shm_ring(...) where zero_copy` — a shared-memory ring with no
  copy at the locus boundary, for the hottest same-host routes.

Here's the part that matters: **not one line of `GameRegion`,
`SessionWorkers`, or `MetricsServer` changes** whether `MatchReady`
is an in-process queue or a Unix socket, whether `region_us` is
pinned to a core or cooperative on the main thread. You design the
system once, and redeploy it — test, single binary, multi-binary,
multi-host — by editing `main`.

## See it on your own code

Before you install anything: in
[Claude Code](https://claude.ai/code), Cursor, or whatever LLM tool
you use, drop this project's [`AGENTS.md`](./AGENTS.md) into the
agent's context and ask it to re-read a module from your existing
codebase **as loci, contracts, and bus topics**. What usually comes
back is a decomposition that matches your mental model — because
the agent is reasoning in the same vocabulary you already use about
your code. If it looks right, you've felt the fit without writing a
line of Hale.

## Try it

**No install — [run Hale in your browser](https://hale-lang.github.io/hale/play/).** The
playground runs real Hale compiled to WebAssembly, right on the page (the UI itself is a
Hale `@export locus`). To build it locally:

```sh
git clone https://github.com/hale-lang/hale
cd hale
cargo build --release
```

Requires Rust 1.95+, LLVM 18, `clang`, and `git`. Platform-specific
install commands are in
[`docs/src/getting-started/install.md`](./docs/src/getting-started/install.md).

Write `hello.hl`:

```hale
fn main() {
    println("Hello from Hale.");
}
```

Run it interpreted for fast feedback, or compile a native binary:

```sh
cargo run -p hale-cli --bin hale -- run   hello.hl
cargo run -p hale-cli --bin hale -- build hello.hl && ./hello
```

Depending on Hale libraries? Declare them in `hale.toml`, then
`hale fetch` vendors and pins them:

```toml
[deps]
pond = { git = "https://github.com/hale-lang/pond", tag = "v0.1.0" }
```

## Where to go next

- **[Docs site](https://hale-lang.github.io/hale/)** — the
  level-by-level tour. Start here.
- **[`spec/`](./spec/)** — the canonical reference; the compiler
  enforces exactly what it describes. Begin with
  [`spec/styleguide.md`](./spec/styleguide.md).
- **[`AGENTS.md`](./AGENTS.md)** — the load-bearing prompt for
  agents writing `.hl` (and a tight read for humans).
- **[`crates/hale-codegen/tests/fixtures/examples/`](./crates/hale-codegen/tests/fixtures/examples/)**
  — ~70 working `.hl` programs.
- **[hale-lang/pond](https://github.com/hale-lang/pond)** —
  contributed libraries: web, databases, observability, AI clients.

## The ecosystem

The names mean things, and they fit together:

- **hale** — the language. From the Old English *hāl*, "whole,
  sound, uninjured." Same root as *whole*, *heal*, *health*.
- **lotus** — the runtime substrate. C-runtime symbols are prefixed
  `lotus_*`.
- **pond** — the contributed library catalog. *Many lotus grow in a
  pond.*
- **heron** — the tree-sitter grammar; editors and the future LSP
  drink from it.

## State of the culture

Hale commits hard and tells you about it:

- **One form per locus.** Compose at the locus level.
- **Three modes** (`bulk`, `harmonic`, `resolution`). No fourth —
  they map to real hardware execution regimes.
- **Vertical-only failure flow.** A parent decides recovery for its
  children; failures never travel sideways.
- **Region-based memory, deterministic dissolve.** No GC, no
  reference counting.
- **Closure assertions as language constructs.** The runtime audits
  the invariants you declare. That's the point.

If your problem decomposes cleanly into loci + bus + closures,
you'll move fast. If it doesn't, the language tells you so — there's
no permissive escape hatch, and that's the feature.

## Status

The language surface is **stable**. Most work between now and v1 is
bugs, stability, and performance — not new syntax. Pin to a commit
if you build on it. The native compiler self-hosts the topic system,
structural interfaces, the `@form(...)` collections, the
`fallible(T)` error model, cooperative + pinned schedulers, and
cross-process bus transports.

**Performance** (v0.9.2, AMD Ryzen 7 9800X3D): coordination is
now a lead, not a cost. After v0.9.0's lock-free bus and
static-dispatch devirtualization, `bus_dispatch` went from ~4×
behind Go to **2.4× ahead** (1.79 ms → 196 µs) and
`bus_dispatch_cross_pool` to **1.26× ahead**. Collections lead
too (vec 3–4×, json_parse 2.3× vs Go), and native codegen brought
tight-loop `fn_call` to **parity with clang -O3 C**. Headline
shape: lock-free bus, static-dispatch devirtualization, native
codegen, and interest-based ownership (accept bubbling).
Methodology and current numbers:
[hale-lang/bench](https://github.com/hale-lang/bench).

## Beyond the native runtime

Hale isn't tied to one runtime. The same `.hl` source also runs in
the browser on [hale-js](https://github.com/hale-lang/hale-js) — the
same `locus`, `bus`, and lifecycle against a browser capability
profile — and the locus + bus shape is a deliberate fit for other
substrates over time. The structural reason one shape carries across
runtimes (and across how humans, LLMs, and machines each represent
it) is written up in
[hale-lang/papers](https://github.com/hale-lang/papers).

## License

Licensed under the [Apache License, Version 2.0](./LICENSE).
Attribution and third-party notices are tracked in
[`NOTICE`](./NOTICE).
