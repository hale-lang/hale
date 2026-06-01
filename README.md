<!--
  DRAFT README — refreshed lead, focused on the language and the
  level it operates at. The capacity-bounds model is intentionally
  not mentioned here. Review against the live ./README.md; promote
  when you're happy with it.
-->
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
[![GC](https://img.shields.io/badge/GC-0-brightgreen.svg)](#what-hale-leaves-out)
[![async/await](https://img.shields.io/badge/async%2Fawait-0-brightgreen.svg)](#what-hale-leaves-out)
[![native](https://img.shields.io/badge/native-human_%2B_agent-8957e5.svg)](./AGENTS.md)

A language whose shape matches the shape of your thinking — from a
quick script down to a systems program, without changing tools.

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

Every phrase from the description has a syntactic home, in roughly
the order you thought them:

- *"a service"* → `locus Matchmaker`
- *"receives players wanting matches"* → `subscribe JoinQueue as on_join`
- *"announces matches"* → `publish MatchReady`
- *"when enough are queued"* → the `if` inside `on_join`

No mutex to pick, no channel types, no async ceremony, no lifecycle
wiring. The code keeps the shape of the sentence.

## What level does Hale operate at?

All of them. Most languages pick an altitude and stay there —
Python and JavaScript high, Go in the middle, Rust and C++ low.
Hale is one language you write at any of those levels, moving
between them without switching tools. The same file can read like a
script at the top and like a systems program at the bottom, because
there's a single primitive — the **locus** — and the only thing
that changes as you descend is how much of it you choose to see.

| Altitude | You write… | Reach for it like… |
|---|---|---|
| **The basics** | variables, math, functions, control flow | a clean scripting language |
| **Everyday programs** | files, JSON, HTTP, loci as objects | Python / Node |
| **Concurrent services** | a typed bus, lifecycle, supervision | Go |
| **Systems control** | memory layout, lifetime, zero-copy I/O, C bindings | Rust / C++ |

Each level is self-contained and expands on the one before without
contradicting it. A function you wrote at the top still works at the
bottom — you've just learned to see more of what was always there.
The [docs site](https://hale-lang.github.io/hale/) is organized as
exactly this descent.

## One language. Every substrate.

The language is **Hale**. *Lotuses* are the runtime substrates Hale
programs run on. Two ship today:

- **Native** — the LLVM-backed C-runtime in this repo. Servers,
  daemons, CLIs, long-running services.
- **Browser** — [hale-js](https://github.com/hale-lang/hale-js).
  The same `.hl` source on a JS lotus with a browser capability
  profile.

Both host the same `locus`, the same `bus`, the same lifecycle —
with substrate-specific capability profiles declared at the build
target:

```hale
target browser_js {
    arenas.epoch_view,
    time.monotonic, time.wallclock,
    random.csprng,
    gfx.canvas2d,
}
```

A program that reaches for a capability its target doesn't offer
fails at the translation boundary with `CAP-MISSING` — at build,
not at runtime. Substrate divergences are named, not papered over.

The bus crosses substrates, too. A browser locus subscribes to
topics a native binary publishes via a WebSocket bridge; two native
binaries exchange topics over `unix("/path")`; a Hale binary talks
to NATS or MQTT through a user-supplied adapter locus. The
`subscribe` / `publish` code never changes — only the `bindings { }`
block in `main` does.

The bet: the **locus + bus + capability-profile** triple is a
coherent answer to substrate variation — one design idiom, N
substrates, honest about divergences. The shipped two-lotus proof
(native + browser) shows the idiom survives the move; further
substrates (mobile, embedded, GPU, edge/Wasm) follow real demand,
not a roadmap.

## What you declare; what the compiler decides

A locus declares *intent* on the axes where systems languages
normally make you hand-pick a mechanism. The compiler picks the
mechanism — with cross-locus, cross-binary knowledge no single
author has — and keeps your application code stable:

- **`topic` + `bus { subscribe/publish }`** declares what crosses
  between loci. The binary picks the transport — in-process queue,
  AF_UNIX, TCP, WebSocket, NATS — without changing the program.
- **`placement { }` in `main`** declares where loci run — pinned
  cores, cooperative pools — at the deployment seam, not baked into
  library code.
- **`@form(vec / hashmap / ring_buffer)`** declares the access
  discipline of a collection; the compiler emits a tight,
  type-specialized implementation.
- **`mode bulk / harmonic / resolution`** declares an execution
  regime; the compiler emits vectorized, cache-tiled, or scalar
  code per mode.

The organizing principle (`spec/design-rationale.md` F.31): *a
library author declares what a locus is; a binary author declares
where it runs; a lotus declares what its substrate offers.* The
compiler picks how, refuses what won't fit.

This is also why Hale suits LLM authoring. The choices models
reliably hallucinate on — which mutex flavour, which transport,
which thread, which substrate — aren't choices anyone needs to make
at the call site. The language moved them into the compiler's
hands.

## One shape, three minds

There's a structural reason the matchmaker decomposes the same way
on paper, in Hale, and inside an LLM's plan: it's the same recursive
shape in each. A Hale program is a structurally-constrained
hypergraph — loci are vertices, topics are the hyperedges binding
publishers to subscribers — and that's the same shape your mental
model takes when you describe the system, and the same shape a
model's hidden state organizes when it plans one.

So translation across the human → LLM → machine boundary stays
cheap: each layer uses the same vertices and edges; no
representation gets rebuilt in a foreign idiom. It's also why the
locus survives the jump from native to browser to any future
substrate — the variance lives in the capability profile and the
transport, never in the shape. The structural-mathematics
foundation is in
[hale-lang/papers](https://github.com/hale-lang/papers).

## Try it on code you already have

Before you install anything: in
[Claude Code](https://claude.ai/code), Cursor, or whatever LLM tool
you use, drop this project's [`AGENTS.md`](./AGENTS.md) into the
agent's context and ask it to re-read a module or service from your
existing codebase **as loci, contracts, and bus topics**.

What usually comes back is a structural decomposition that matches
your mental model — because the agent is reasoning in the same
recursive vocabulary you already use about your code. If it looks
right, you've felt the structural fit from the reading side without
writing a line of Hale. If it doesn't, the thesis fails for your
codebase — open an issue, that's useful feedback.

## What Hale leaves out

- **No `class`, `module`, `package`** — the **locus** subsumes them.
  Apps, services, caches, handlers, libraries: all loci.
- **No `Vec<T>` / `Map<K,V>`** — declare a collection with `@form`
  on a locus instead.
- **No `async` / `await`** — concurrency lives on the typed bus and
  the lifecycle; there's no function-coloring problem because there
  are no async functions, only loci that yield.
- **No GC, no borrow checker** — the locus hierarchy is explicit, so
  cleanup is deterministic at dissolve. (The browser lotus uses the
  JS GC as its backing allocator; the lifecycle contract still
  holds.)
- **No exceptions, no `panic`/`assert`** — failure is either a
  value-level `fallible(E)` addressed at the call site, or a
  structural violation routed to a parent's recovery policy.

## The ecosystem

The names mean things, and they fit together:

- **hale** — the language. From the Old English *hāl*, "whole,
  sound, uninjured." Same root as *whole*, *heal*, *health*.
- **lotus** — a runtime substrate. The C-runtime here is one lotus;
  [hale-js](https://github.com/hale-lang/hale-js) is another.
  C-runtime symbols are prefixed `lotus_*`.
- **pond** — the contributed library catalog. *Many lotus grow in a
  pond.* HTTP, SQLite, sessions, jobs, supervisors, tracing,
  metrics, LLM clients — see
  [hale-lang/pond](https://github.com/hale-lang/pond).
- **heron** — the tree-sitter grammar that watches over the pond.
  Editors and the future LSP drink from heron.
- **iris** — the workbench for designing and visualizing locus
  structures.

**Hale** is what you write; **lotuses** are what run it.

## Try it

```sh
git clone https://github.com/hale-lang/hale
cd hale
cargo build --release
cargo test --release --workspace
```

Requires Rust 1.95+, LLVM 18, `clang`, and `git`. Platform-specific
install commands are in
[`docs/src/getting-started/install.md`](./docs/src/getting-started/install.md).

```sh
# Interpreted (fast feedback)
cargo run -p hale-cli --bin hale -- run hello.hl

# Native binary via LLVM
cargo run -p hale-cli --bin hale -- build hello.hl
./hello
```

If your project depends on Hale libraries in git repos, declare them
in `hale.toml`:

```toml
[deps]
pond    = { git = "https://github.com/hale-lang/pond", tag = "v0.1.0" }
helpers = { git = "https://github.com/me/helpers", rev = "abc123" }
```

Then `hale fetch` clones each into `vendor/<name>/` and pins the
commits to `hale.lock`; `import "vendor/pond/router" as router;`
picks them up.

## Where to go next

- **[Docs site](https://hale-lang.github.io/hale/)** — the
  level-by-level tour. Start here if you're new.
- **[`spec/`](./spec/)** — the canonical language reference; the
  compiler enforces exactly what these documents describe. Start
  with [`spec/styleguide.md`](./spec/styleguide.md), then
  [`spec/semantics.md`](./spec/semantics.md).
- **[`AGENTS.md`](./AGENTS.md)** — load-bearing prompt for agents
  writing `.hl` programs.
- **[`crates/hale-codegen/tests/fixtures/examples/`](./crates/hale-codegen/tests/fixtures/examples/)**
  — ~70 working `.hl` programs.
- **[hale-lang/pond](https://github.com/hale-lang/pond)** —
  contributed libraries. **[hale-js](https://github.com/hale-lang/hale-js)**
  — the browser lotus. **[papers](https://github.com/hale-lang/papers)**
  — the structural foundation. **[bench](https://github.com/hale-lang/bench)**
  — comparative benchmarks.

## Layout

```
spec/                       grammar + semantics + design rationale
CHANGELOG.md                historical record (spec/ has current state)
AGENTS.md                   load-bearing prompt for .hl-authoring agents
agents/                     role briefs for compiler / stdlib work
docs/                       narrative documentation
crates/
  hale-syntax/            lexer + parser + AST
  hale-types/             symbol resolution + typechecker
  hale-runtime/           tree-walking interpreter
  hale-codegen/           LLVM codegen + bundled C runtime + stdlib
  hale-cli/               the `hale` binary
```

## State of the culture

Hale commits hard and tells you about it:

- **Three modes** (`bulk`, `harmonic`, `resolution`). No fourth —
  they map to real hardware execution regimes, not arbitrary
  minimalism.
- **One form per locus.** Compose at the locus level, not the form
  level.
- **Vertical-only failure flow.** Parent-policy decides recovery.
- **Region-based memory, deterministic dissolve.** No GC, no
  reference counting on the native lotus; the lifecycle contract
  holds across substrates regardless of the underlying allocator.
- **Closure assertions as language constructs.** The runtime audits
  your invariants. That's the point.
- **Capability profiles per lotus.** A program reaches only what its
  target offers; reaching further fails at build, not at runtime.

If your problem decomposes cleanly into loci + bus + closures,
you'll move fast. If it doesn't, the language tells you so. There's
no permissive escape hatch — that's the feature.

## Status

The language surface is **stable**. Most work between now and v1 is
bugs, stability, and performance — not new syntax. Pin to a commit
if you build on it; stability fixes occasionally tighten
previously-accepted code.

The native compiler self-hosts the topic system, structural
interfaces, the `@form(...)` collections, the `fallible(T)` error
model, cooperative + pinned schedulers, and AF_UNIX / TCP
cross-process bus transports. The browser lotus
([hale-js](https://github.com/hale-lang/hale-js)) hosts the same
surface against a capability-restricted target.

**Performance.** Measured at v0.8.0 on AMD Ryzen 7 9800X3D. On a
coordinated workload (`tree_fanout`): 8.6× faster than Node, 20.5×
faster than Python. On isolated loop overhead (`loop_only`, no
coordination): much slower than Python — coordination cost is
substrate cost, and Hale is shaped to pay it; pure-loop work isn't
where the language earns its keep. Methodology + current numbers:
[hale-lang/bench](https://github.com/hale-lang/bench).

## License

Licensed under the [Apache License, Version 2.0](./LICENSE).
Attribution and third-party notices are tracked in
[`NOTICE`](./NOTICE).
