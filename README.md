<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/aperio-banner-dark.svg">
  <img alt="Aperio — hypergraph programming" src="docs/assets/aperio-banner-light.svg" width="100%">
</picture>

[![Tests](https://github.com/aperio-lang/aperio/actions/workflows/tests.yml/badge.svg)](https://github.com/aperio-lang/aperio/actions/workflows/tests.yml)
[![Docs](https://github.com/aperio-lang/aperio/actions/workflows/docs.yml/badge.svg)](https://aperio-lang.github.io/aperio/)
[![License](https://img.shields.io/badge/license-Apache_2.0-blue.svg)](./LICENSE)
[![LLVM](https://img.shields.io/badge/LLVM-18-red.svg)](https://llvm.org/)
[![Status](https://img.shields.io/badge/status-experimental-yellow.svg)](#status)
[![GC](https://img.shields.io/badge/GC-0-brightgreen.svg)](#state-of-the-culture)
[![async/await](https://img.shields.io/badge/async%2Fawait-0-brightgreen.svg)](#state-of-the-culture)
[![native](https://img.shields.io/badge/native-human_%2B_agent-8957e5.svg)](./AGENTS.md)

> **Experimental language. Breaking changes welcome.**
> **v0.x — still in the part of the curve where the design is negotiable.**

Aperio operationalizes a research program on the structural mathematics
of systems — a substrate-invariant recursive hypergraph of typed,
lifecycled units called **loci**, governed by a capacity-allocation
discipline. The grammar is the operational surface of that structure:
every declaration in an Aperio program is either a locus or a relation
between loci. The compiler enforces that the structure you write is
the structure the program runs as.

A locus owns a memory region, has a lifecycle, and talks to other loci
over a typed bus. Everything named and structural in an Aperio program
is a locus. Apps are loci. Services are loci. Caches are loci. Handlers
are loci. Libraries are loci. Loci nest inside loci all the way down.

What the language deliberately doesn't have:

- No `class`, no `module`, no `package` — the locus subsumes them.
- No `Vec<T>` — write `@form(vec)` on a locus and storage discipline becomes
  part of the declaration.
- No `async`/`await` — concurrency lives on the typed bus.
- No garbage collector and no borrow checker — the hierarchy is explicit in
  the source, so dissolve is deterministic.
- No `try`/`catch` in lifecycle methods — failures flow vertically to the
  parent's `on_failure` handler.
- No visibility modifiers, no traits — v0 doesn't need them.

The intended primary author is an LLM. The intended primary reader is a
person. The language is shaped for both — small primitive surface, low
decision-overhead per statement, opinionated enough that there's usually a
right answer before you write the code.

Aperio compiles to native binaries via LLVM 18 and ships a tree-walking
interpreter for fast iteration. The stdlib (`std::io::tcp`, `std::io::fs`,
`std::http`, `std::time`, `std::str`, ...) is bundled into every program.

## A small program

```aperio
type Tick { n: Int; }
topic Beats { payload: Tick; }

locus Counter {
    params { sum: Int = 0; }
    bus { subscribe Beats as on_beat; }
    fn on_beat(t: Tick) { self.sum = self.sum + t.n; }
}

locus Pulse {
    params { iters: Int = 4; }
    bus { publish Beats; }
    run() {
        let mut i = 1;
        while i <= self.iters {
            Beats <- Tick { n: i };
            i = i + 1;
        }
    }
}

fn main() {
    let c = Counter { };
    Pulse { iters: 4 };
    print("sum=");
    println(c.sum);
}
```

Two loci communicate over a typed topic. `Counter` subscribes; `Pulse`
publishes. Lifecycle is implicit: `Pulse { iters: 4 }` constructs the locus,
runs its `run()` body to completion, then dissolves. The result printed is
`sum=10`.

## Try it

**Prerequisites:** a Rust toolchain (1.95+), **LLVM 18** dev libraries with
`llvm-config-18` on `PATH` (or `LLVM_SYS_180_PREFIX` set), `clang` (used as
the linker for `aperio build`), and `git`. Platform-specific install commands
for Debian/Ubuntu, macOS Homebrew, and Fedora are in
[`docs/src/getting-started/install.md`](./docs/src/getting-started/install.md).
LLVM 17 / 19 / 20 will not work — the codegen crate pins `inkwell` to
`llvm18-0`.

```sh
git clone https://github.com/aperio-lang/aperio
cd aperio
cargo build --release
cargo test --release --workspace
```

Run a program:

```sh
# Interpreted (fast feedback)
cargo run -p aperio-cli --bin aperio -- run hello.ap

# Native binary via LLVM
cargo run -p aperio-cli --bin aperio -- build hello.ap
./hello
```

The `aperio` CLI accepts a single `.ap` file or a directory; a directory
bundles every `.ap` in it as one program (one binary). See `aperio --help`
for the full surface.

If your project depends on Aperio libraries hosted in git repos, declare
them in `aperio.toml`:

```toml
[deps]
helpers = { git = "https://github.com/me/helpers", rev = "abc123" }
finance = { git = "https://github.com/me/finance", tag = "v0.1.0" }
```

Then `aperio fetch` clones each into `vendor/<name>/` and pins the resolved
commits to `aperio.lock`. The existing `import "vendor/helpers" as h;`
directive picks them up — no extra configuration needed. (Hand-vendored
libraries stay under `lib/<name>/`; the toolchain only writes to `vendor/`.)

## Where to go next

- **Docs site** — <https://aperio-lang.github.io/aperio/> (built from
  `docs/` via mdbook).
- **`spec/`** — the canonical language reference. Start with
  [`spec/styleguide.md`](./spec/styleguide.md), then
  [`spec/semantics.md`](./spec/semantics.md) and
  [`spec/grammar.ebnf`](./spec/grammar.ebnf).
- **[`CHANGELOG.md`](./CHANGELOG.md)** — historical record of behavior
  changes. The spec files represent current state; `CHANGELOG.md`
  records what shipped when.
- **[`AGENTS.md`](./AGENTS.md)** — load-bearing prompt for AI agents writing
  `.ap` programs. Compiler / stdlib / spec work has separate briefs under
  [`agents/`](./agents/).
- **[`apps/`](./apps)** — working programs built in Aperio (`cli-demo`,
  `log-router`, `ssg`, `tcp-echo`, `ws-echo`, ...). Read these to see real
  shape.
- **`crates/aperio-codegen/tests/fixtures/examples/`** — small per-feature
  anchor programs the parser is checked against.
- **[`aperio-lang/pond`](https://github.com/aperio-lang/pond)** —
  community-contrib libraries (protocols, parsers, common shapes too
  specific for stdlib). Many lotus grow in a pond. Vendor via `aperio.toml`
  → `aperio fetch`.
- **[`aperio-lang/papers`](https://github.com/aperio-lang/papers)** —
  the structural-mathematics work the language is grounded in
  (substrate, capacity allocation, the hypergraph model). Read here
  for the *why* of every commitment under "state of the culture".
- **Sibling repos** — <https://github.com/aperio-lang/examples> and
  <https://github.com/aperio-lang/bench>.

## Layout

```
spec/                       grammar + semantics + design rationale
CHANGELOG.md                historical record (spec/ has current state)
AGENTS.md                   load-bearing prompt for .ap-authoring agents
agents/                     role briefs for compiler / stdlib work
apps/                       working programs built in Aperio
docs/                       narrative documentation (in progress)
notes/                      surviving design notes
crates/
  aperio-syntax/            lexer + parser + AST
  aperio-types/             symbol resolution + typechecker
  aperio-runtime/           tree-walking interpreter
  aperio-codegen/           LLVM codegen + bundled C runtime + stdlib
  aperio-cli/               the `aperio` binary
  aperio-ts-shim/           tree-sitter staticlib (powers std::ts)
```

The C runtime symbols are prefixed `lotus_*`. That's not a relic — it's the
design. **Aperio** is the language; **lotus** is the runtime substrate
Aperio programs run on. Two names, two layers, one project.

## State of the culture

Aperio commits hard and tells you about it:

- **Three projection classes** (`Rich`, `Chunked`, `Recognition`). No fourth.
- **Three modes** (`bulk`, `harmonic`, `resolution`). No fourth.
- **One form per locus.** Compose at the locus level, not the form level.
- **Vertical-only failure flow.** Parent-policy decides recovery.
- **Region-based memory, deterministic dissolve.** No GC, no ARC, no
  reference counting.
- **Closure assertions as language constructs.** Yes, the runtime audits
  your invariants. Yes, that's the point.

If your problem decomposes cleanly into loci + bus + capacity + closure,
you'll move fast. If it doesn't, the language will tell you so. There is
no permissive escape hatch — that's the feature, not the bug.

If you're looking for "express anything," this isn't it. If you're looking
for "express what production systems actually need without 700 lines of
ceremony," keep reading.

## Status

Experimental. The compiler self-hosts the topic system, structural
interfaces, `@form(...)` lowerings (vec, hashmap, ring_buffer),
`fallible(T)` error model, capacity-tuple memory discipline,
cooperative + pinned schedulers, and AF_UNIX / TCP cross-process bus
transports. The reference test suite is the ~70 in-tree fixture
programs under `crates/aperio-codegen/tests/fixtures/examples/` plus
per-feature tests under `crates/aperio-codegen/tests/`.

Breaking changes are expected; pin to a commit if you build on it.
See [`CHANGELOG.md`](./CHANGELOG.md) for what's moved recently.

## License

Licensed under the [Apache License, Version 2.0](./LICENSE). Attribution
and any third-party notices are tracked in [`NOTICE`](./NOTICE).

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in Aperio shall be licensed as above, without
additional terms or conditions.
