# Aperio

> **Experimental language. Breaking changes welcome.**

Aperio is a small programming language for systems built out of
**loci** — typed, lifecycled units that publish and subscribe to
each other through a typed bus. Apps, services, handlers, caches,
schedulers, libraries: everything is a locus. Composition is
recursive — loci nest inside loci all the way down.

The language compiles to native binaries via LLVM, has a
tree-walking interpreter for fast iteration, and ships a small
stdlib (`std::io::tcp`, `std::io::fs`, `std::http`, `std::time`,
`std::str`, ...) bundled into every program.

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

Two loci communicate over a typed topic. `Counter` subscribes;
`Pulse` publishes. Lifecycle is implicit: `Pulse { iters: 4 }`
constructs the locus, runs its `run()` body to completion, then
dissolves. The result printed is `sum=10`.

## Try it

**Prerequisites:** a Rust toolchain (1.95+), **LLVM 18** dev
libraries with `llvm-config-18` on `PATH` (or
`LLVM_SYS_180_PREFIX` set), `clang` (used as the linker for
`aperio build`), and `git`. Platform-specific install commands
for Debian/Ubuntu, macOS Homebrew, and Fedora are in
[`docs/src/getting-started/install.md`](./docs/src/getting-started/install.md).
LLVM 17 / 19 / 20 will not work — the codegen crate pins
`inkwell` to `llvm18-0`.

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

The `aperio` CLI accepts a single `.ap` file or a directory; a
directory bundles every `.ap` in it as one program (one binary).
See `aperio --help` for the full surface.

If your project depends on Aperio libraries hosted in git
repos, declare them in `aperio.toml`:

```toml
[deps]
helpers = { git = "https://github.com/me/helpers", rev = "abc123" }
finance = { git = "https://github.com/me/finance", tag = "v0.1.0" }
```

Then `aperio fetch` clones each into `vendor/<name>/` and pins
the resolved commits to `aperio.lock`. The existing `import
"vendor/helpers" as h;` directive picks them up — no extra
configuration needed. (Hand-vendored libraries stay under
`lib/<name>/`; the toolchain only writes to `vendor/`.)

## Where to go next

- **Docs site** — <https://aperio-lang.github.io/aperio/>
  (built from `docs/` via mdbook).
- **`spec/`** — the canonical language reference. Start with
  [`spec/styleguide.md`](./spec/styleguide.md), then
  [`spec/semantics.md`](./spec/semantics.md) and
  [`spec/grammar.ebnf`](./spec/grammar.ebnf).
- **[`AGENTS.md`](./AGENTS.md)** — load-bearing prompt for AI
  agents writing `.ap` programs. Compiler / stdlib / spec work
  has separate briefs under [`agents/`](./agents/).
- **[`apps/`](./apps)** — working programs built in Aperio
  (`cli-demo`, `log-router`, `ssg`, `tcp-echo`, `ws-echo`,
  ...). Read these to see real shape.
- **`crates/aperio-codegen/tests/fixtures/examples/`** — small
  per-feature anchor programs the parser is checked against.
- **[`aperio-lang/pond`](https://github.com/aperio-lang/pond)** —
  community-contrib libraries (protocols, parsers, common shapes
  too specific for stdlib). Many lotus grow in a pond. Vendor
  via `aperio.toml` → `aperio fetch`.
- **Sibling repos** — <https://github.com/aperio-lang/examples>
  and <https://github.com/aperio-lang/bench>.

## Layout

```
spec/                       grammar + semantics + design rationale
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

The C runtime symbols are prefixed `lotus_*` (arena, bus, tcp,
transport). That's intentional: *Aperio* is the language; *lotus*
is the runtime substrate Aperio programs run on.

## Status

Experimental. The compiler self-hosts the topic system, structural
interfaces (F.20), `@form(...)` lowerings (vec, hashmap,
ring_buffer), `fallible(T)` error model, capacity-tuple memory
discipline, cooperative + pinned schedulers, and AF_UNIX / TCP
cross-process bus transports. The reference test suite is the
~70 in-tree fixture programs under
`crates/aperio-codegen/tests/fixtures/examples/` plus per-feature
tests under `crates/aperio-codegen/tests/`.

Breaking changes are expected; pin to a commit if you build on it.

## License

TBD. The project is in design exploration; licensing decisions
follow first public release.
