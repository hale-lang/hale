<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/hale-banner-dark.svg">
  <img alt="Hale — hypergraph programming" src="docs/assets/hale-banner-light.svg" width="100%">
</picture>

[![Tests](https://github.com/hale-lang/hale/actions/workflows/tests.yml/badge.svg)](https://github.com/hale-lang/hale/actions/workflows/tests.yml)
[![Docs](https://github.com/hale-lang/hale/actions/workflows/docs.yml/badge.svg)](https://hale-lang.github.io/hale/)
[![License](https://img.shields.io/badge/license-Apache_2.0-blue.svg)](./LICENSE)
[![LLVM](https://img.shields.io/badge/LLVM-18-red.svg)](https://llvm.org/)
[![Status](https://img.shields.io/badge/status-stabilizing-blue.svg)](#status)
[![lotuses](https://img.shields.io/badge/lotuses-native_%2B_browser-8957e5.svg)](#one-language-every-substrate)
[![GC](https://img.shields.io/badge/GC-0-brightgreen.svg)](#state-of-the-culture)
[![async/await](https://img.shields.io/badge/async%2Fawait-0-brightgreen.svg)](#state-of-the-culture)
[![native](https://img.shields.io/badge/native-human_%2B_agent-8957e5.svg)](./AGENTS.md)

A language where the shape of your code matches the shape of your
thinking — and runs everywhere your stack runs.

You know that feeling when you describe a system out loud —
*"a matchmaker holds a queue of waiting players, spawns a match when
enough are queued, then goes back to waiting"* — and then the code
you actually have to write bears no resemblance to those words?
Mutexes appear. Async machinery. Lifecycle wiring. Five files.

Hale is a bet that gap doesn't have to be there.

## A matchmaker, in Hale

```hale
type Player    { id: String; name: String; }
type MatchInfo { match_id: String; players: [Player]; }

topic JoinQueue  { payload: Player; }
topic MatchReady { payload: MatchInfo; }

@form(vec)
locus Matchmaker {
    params   { target_size: Int = 4; }
    capacity { heap waiting of Player; }
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

Every phrase from the description has a syntactic home, in roughly
the order you thought about them:

- *"a service"* → `locus Matchmaker`
- *"a queue of waiting players"* → `capacity { heap waiting of Player; }`
  (the `@form(vec)` annotation gives it `push`, `get`, `len`, and friends)
- *"receives players wanting matches"* → `subscribe JoinQueue as on_join`
- *"announces matches"* → `publish MatchReady`
- *"when enough are queued"* → the `if` inside `on_join`

`@form(vec)` is a real decision, not a syntactic flourish.
`@form(ring_buffer)` would give the same shape with bounded capacity
and drop-on-full; `@form(hashmap)` keyed by player id would give
natural ID-based cancellation. You declare the access discipline; the
compiler picks the layout — and revises it later as the language
learns more about how loci interact, without you editing a line.

## One language. Every substrate.

The language is **Hale**. *Lotuses* are the runtime substrates Hale
programs run on. Today there are two shipped:

- **Native** — the LLVM-backed C-runtime in this repo. Servers,
  daemons, CLIs, long-running services.
- **Browser** — [hale-js](https://github.com/hale-lang/hale-js).
  The same `.hl` source, hosted on a JS lotus with a browser-tier
  capability profile.

Both host the same `locus`, the same `bus`, the same `capacity` —
with substrate-specific capability profiles declared at the build
target:

```hale
target browser-js {
    arenas.epoch_view,
    time.monotonic, time.wallclock,
    random.csprng,
    gfx.canvas2d,
}
```

Programs that reach for capabilities a target doesn't offer fail at
the translation boundary with `CAP-MISSING`. Substrate divergences
are named and documented, not papered over.

The bus crosses substrates. A browser-tier locus subscribes to topics
published by a native binary via the `TransportBridge` adapter
(WebSocket); two native binaries exchange topics via `unix("/path")`;
a hale binary talks to NATS or MQTT via a user-supplied
`__StdBusAdapter` locus. The same `subscribe` / `publish` code
doesn't change — only the `bindings { }` block in `main` does.

### Where the structure goes

The locus is unusually substrate-invariant — and that's a
property of the *shape*, not of any particular runtime. A
locus's commitments — bounded capacity, message-typed bus,
explicit capability profile, deterministic dissolve, vertical
failure flow — are the same constraints every substrate
already enforces under different names. Server runtimes call
them threads + queues + allowed syscalls. Browsers call them
isolates + postMessage + Permissions API. Mobile platforms
call them activities / view controllers + intents +
entitlements. Embedded systems call them real-time loops +
IPC + watchdogs. The locus isn't *abstracting over* those
things — it's the shape they all converge to. Substrate
variation lives in the capability profile and the transport
adapter; the program above doesn't change.

Two substrates ship:

| Substrate | What the locus maps to | Status |
|---|---|---|
| **Servers / backends** | Native loci on OS threads + cooperative pools, region memory, AF_UNIX/TCP bus | ✓ shipped (C-runtime) |
| **Browser** | Loci over GC arenas + RAF/microtask scheduling, WebSocket transport bridge | ✓ shipped (hale-js) |

The same triple — locus + bus + capability-profile — suggests
clean fits for **mobile** (lifecycle objects ↔ loci;
intents/XPC ↔ bus), **embedded / IoT** (real-time loops +
watchdogs ↔ pinned loci + parent supervisors), **GPU**
(kernels lowered via `mode bulk/harmonic/resolution`),
**robotics** (ROS nodes ↔ loci; ROS topics ↔ hale topics —
the naming overlap isn't accidental), and **edge / Wasm**
(short-lived loci with capability profiles ≈ WASI imports).
The structural fit is concrete in each case — the table
column "what the locus maps to" already exists for them; only
the runtime doesn't.

These aren't roadmap promises. Building a new lotus is real
engineering: a runtime, a capability profile, a transport
adapter, codegen if the substrate's execution model differs
from the C-runtime's. That work happens **when a downstream
workload pulls for it** — when "the same `.hl` on this
substrate too" is the load-bearing reason someone picks Hale
over the local alternative. The two-lotus proof shows the
design idiom survives the move from native to browser; the
rest follows demand, not a roadmap.

What's shipped is the two-lotus proof. The bet is that the
locus + bus + capability-profile triple is a coherent
solution to substrate variation: **one design idiom, N
substrates, honest about divergences**. No nine-language
polyglot stack; no impedance mismatch at substrate
boundaries; one `AGENTS.md` across the stack.

## Try it on code you already have

Before you install anything: in
[Claude Code](https://claude.ai/code), Cursor, or whatever LLM tool
you use, drop this project's [`AGENTS.md`](./AGENTS.md) into the
agent's context and ask it to re-read a module or service from your
existing codebase **as loci, contracts, and bus topics**.

What usually comes back is a structural decomposition that matches
your mental model of the system — because the agent is reasoning in
the same recursive vocabulary you already use when thinking about
your code. If the decomposition looks right, you've felt the
structural fit from the reading side without writing a line of Hale.
If it doesn't, the thesis fails for your codebase — open an issue,
that's useful feedback.

## What the language is doing for you

Every block on a locus declares intent on one axis where systems
languages normally make you pick a mechanism. The compiler picks the
mechanism with cross-locus, cross-pool, cross-binary knowledge no
individual author has:

- **`capacity { heap/pool of T }`** declares bounded storage
  discipline. The compiler picks arena chunk size, slab vs free-list,
  cache-line padding, huge-pages backing, lock-memory policy.
- **`@form(vec/hashmap/ring_buffer/…)`** declares access discipline.
  The compiler picks the physical container *and* the sync strategy.
  v0.8.0's F.32 swapped cross-pool hashmap sync from a global mutex
  to cache-line-padded striped CAS across the entire language without
  users editing a line of `.hl` code.
- **`topic` + `bus { subscribe/publish }`** declares what crosses
  between loci. The binary picks transport — in-process queue,
  AF_UNIX, TCP, WebSocket, NATS, MQTT — without changing the program.
- **`placement { }` in `main`** declares where loci live. Pinned
  cores, cooperative pools, migration — chosen at the deployment
  seam, not baked into library code.
- **`mode bulk/harmonic/resolution`** declares execution regime. The
  compiler picks vectorised, cache-tiled, or scalar codegen per mode.

The organizing principle (`spec/design-rationale.md` F.31): *a
library author declares what a locus is; a binary author declares
where it runs; a lotus declares what its substrate offers.* The
compiler picks how, refuses what won't fit, and keeps the
application code stable across substrates.

This is also why Hale is shaped for LLM authoring. The choices LLMs
reliably hallucinate on — which mutex flavour, which container
variant, which transport, which thread, which substrate — aren't
choices the LLM (or you) needs to make. The language has moved them
into the compiler's hands.

## Why one shape works across human, LLM, and machine

The locus is substrate-invariant for a structural reason. When K
things attach to one coordination point, the working state needed
to hold them together costs K log₂ K bits. The same ceiling,
k̄ ∈ [4, 10], shows up in human working memory (Miller's 7 ± 2),
spans of control, surgical teams, mixture-of-experts active counts,
and multi-agent LLM saturation. A Hale program is the literal shape
of that bound: loci are vertices, topics are hyperedges, capacity
declarations bound each vertex's K. The math and cross-substrate
evidence are in
[hale-lang/papers](https://github.com/hale-lang/papers).

This is why translation across the human → LLM → machine boundary
stays cheap: each layer uses the same vertices and edges. No
representation gets rebuilt in a foreign idiom. And it's the same
reason the locus survives the transition from a C-runtime to a
browser to a future embedded/GPU/contract lotus — substrate variance
doesn't reach into the shape.

## What Hale leaves out

- **No `class`, `module`, `package`** — the **locus** subsumes them.
  Apps, services, caches, handlers, libraries: all loci, all the way
  down.
- **No `Vec<T>`** — see `@form` above. Storage discipline is part of
  the declaration, not the type.
- **No `async`/`await`** — concurrency lives on the typed bus.
- **No GC, no borrow checker** — the locus hierarchy is explicit, so
  dissolve is deterministic. (The browser lotus uses the JS GC as
  its arena backend; the lifecycle contract still holds.)

## The ecosystem

The names mean things, and they fit together:

- **hale** — the language. From the Old English *hāl*, "whole,
  sound, uninjured." Same root as *whole*, *heal*, *health*.
- **lotus** — a runtime substrate. The C-runtime in this repo is one
  lotus; [hale-js](https://github.com/hale-lang/hale-js) is another.
  C-runtime symbols are prefixed `lotus_*`.
- **pond** — the contributed library catalog, where loci live.
  *Many lotus grow in a pond.* HTTP, SQLite, sessions, jobs,
  supervisors, tracing, metrics, LLM clients, embeddings, neural
  nets — see [hale-lang/pond](https://github.com/hale-lang/pond).
- **heron** — the tree-sitter grammar that watches over the pond.
  Editors, syntax highlighters, and the future LSP all drink from
  heron.
- **iris** — the workbench for designing and visualizing locus
  structures. Concurrent human + agent work on the shape of a system.

**Hale** is what you write; **lotuses** are what run it.

## Try it

```sh
git clone https://github.com/hale-lang/hale
cd hale
cargo build --release
cargo test --release --workspace
```

Requires Rust 1.95+, LLVM 18, `clang`, and `git`. Platform-specific
install commands for Debian/Ubuntu, macOS Homebrew, and Fedora are in
[`docs/src/getting-started/install.md`](./docs/src/getting-started/install.md).

Run a program:

```sh
# Interpreted (fast feedback)
cargo run -p hale-cli --bin hale -- run hello.hl

# Native binary via LLVM
cargo run -p hale-cli --bin hale -- build hello.hl
./hello
```

For the browser lotus, see
[hale-js](https://github.com/hale-lang/hale-js) — same `.hl` source,
different target.

If your project depends on Hale libraries hosted in git repos,
declare them in `hale.toml`:

```toml
[deps]
pond    = { git = "https://github.com/hale-lang/pond", tag = "v0.1.0" }
helpers = { git = "https://github.com/me/helpers", rev = "abc123" }
```

Then `hale fetch` clones each into `vendor/<name>/` and pins the
resolved commits to `hale.lock`. `import "vendor/pond/router" as router;`
picks them up — no extra configuration. Pond's "no transitive
dependencies in v1" rule means every package your program pulls in
is visible in your lockfile, not hidden behind a chain of imports.

## Where to go next

- **[Docs site](https://hale-lang.github.io/hale/)** — the friendly
  tour. Start here if you're new.
- **[`spec/`](./spec/)** — canonical language reference. The compiler
  enforces exactly what these documents describe. Start with
  [`spec/styleguide.md`](./spec/styleguide.md), then
  [`spec/semantics.md`](./spec/semantics.md) and
  [`spec/grammar.ebnf`](./spec/grammar.ebnf).
- **[`CHANGELOG.md`](./CHANGELOG.md)** — historical record of
  behavior changes.
- **[`AGENTS.md`](./AGENTS.md)** — load-bearing prompt for AI agents
  writing `.hl` programs. Role briefs live under
  [`agents/`](./agents/).
- **[`crates/hale-codegen/tests/fixtures/examples/`](./crates/hale-codegen/tests/fixtures/examples/)**
  — ~70 working `.hl` programs. Read these to see real shape.
- **[`hale-lang/pond`](https://github.com/hale-lang/pond)** —
  contributed libraries (web, observability, supervision, AI/agent).
  Vendor via `hale.toml` → `hale fetch`.
- **[`hale-lang/hale-js`](https://github.com/hale-lang/hale-js)** —
  browser-tier lotus. Same `.hl`, different substrate.
- **[`hale-lang/papers`](https://github.com/hale-lang/papers)** —
  the structural-mathematics foundation.
- **[`hale-lang/bench`](https://github.com/hale-lang/bench)** —
  comparative benchmarks against Go, Node, and Python.
- **Sibling repos** —
  [examples](https://github.com/hale-lang/examples),
  [iris](https://github.com/hale-lang/iris).

## Layout

```
spec/                       grammar + semantics + design rationale
CHANGELOG.md                historical record (spec/ has current state)
AGENTS.md                   load-bearing prompt for .hl-authoring agents
agents/                     role briefs for compiler / stdlib work
docs/                       narrative documentation
notes/                      surviving design notes
crates/
  hale-syntax/            lexer + parser + AST
  hale-types/             symbol resolution + typechecker
  hale-runtime/           tree-walking interpreter
  hale-codegen/           LLVM codegen + bundled C runtime + stdlib
  hale-cli/               the `hale` binary
  hale-ts-shim/           tree-sitter staticlib (powers std::ts)
```

## State of the culture

Hale commits hard and tells you about it:

- **Three projection classes** (`Rich`, `Chunked`, `Recognition`).
  No fourth.
- **Three modes** (`bulk`, `harmonic`, `resolution`). No fourth.
  These map to real hardware execution regimes — vectorised
  throughput, cache-tiled per-class, single-decision scalar — not
  arbitrary minimalism.
- **One form per locus.** Compose at the locus level, not the form
  level.
- **Vertical-only failure flow.** Parent-policy decides recovery.
- **Region-based memory, deterministic dissolve.** No GC, no ARC,
  no reference counting on the native lotus; lifecycle contract
  holds across substrates regardless of the underlying allocator.
- **Closure assertions as language constructs.** Yes, the runtime
  audits your invariants. Yes, that's the point.
- **Capability profiles per lotus.** A program reaches only what its
  target offers; reaching further fails at build, not at runtime.

If your problem decomposes cleanly into loci + bus + capacity +
closure, you'll move fast. If it doesn't, the language will tell you
so. There is no permissive escape hatch — that's the feature, not
the bug.

## Status

The language surface is **stable**. A few small additions are still
on the way (tracked in `spec/` and `notes/`), but most work between
now and v1 is bugs, stability, and performance — not new syntax or
new semantics. Pin to a commit if you build on it; small additions
still land, and stability fixes occasionally tighten previously-
accepted code.

The native compiler self-hosts the topic system, structural
interfaces, `@form(...)` lowerings (vec, hashmap, ring_buffer),
`fallible(T)` error model, capacity-tuple memory discipline,
cooperative + pinned schedulers, and AF_UNIX / TCP cross-process bus
transports. The browser lotus
([hale-js](https://github.com/hale-lang/hale-js)) hosts the same
language surface against a capability-restricted target; codegen
emits directly to its runtime contract. The reference test suite
is the ~70 in-tree fixture programs under
`crates/hale-codegen/tests/fixtures/examples/` plus per-feature
tests under `crates/hale-codegen/tests/`.

**Performance.** Measured at v0.8.0 on AMD Ryzen 7 9800X3D
(Linux 6.18). On a coordinated workload (`tree_fanout`,
substrate-flexing): 8.6× faster than Node, 20.5× faster than
Python. On isolated loop overhead (`loop_only`, no
coordination): 283× slower than Python. Coordination cost is
substrate cost, and Hale is shaped to pay it; pure-loop work
isn't where the language earns its overhead. Methodology +
current numbers: [hale-lang/bench](https://github.com/hale-lang/bench).

See [`CHANGELOG.md`](./CHANGELOG.md) for what's moved recently.

## License

Licensed under the [Apache License, Version 2.0](./LICENSE).
Attribution and any third-party notices are tracked in
[`NOTICE`](./NOTICE).

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in Hale shall be licensed as above, without
additional terms or conditions.
