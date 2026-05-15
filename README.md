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

```sh
git clone <this repo>
cd lotus-lang
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

Then `aperio fetch` clones each into `lib/<name>/` and pins the
resolved commits to `aperio.lock`. The existing `import
"lib/helpers" as h;` directive picks them up — no extra
configuration needed.

## Where to go next

- **`spec/`** — the language reference. Start with
  [`spec/semantics.md`](./spec/semantics.md), then
  [`spec/grammar.ebnf`](./spec/grammar.ebnf) and
  [`spec/styleguide.md`](./spec/styleguide.md).
- **`docs/`** — narrative documentation (work in progress; the
  spec is the canonical source until docs catch up).
- **`agents/`** — role-organized briefs for collaborating with
  AI agents on this codebase. Three docs:
  [`app-dev.md`](./agents/app-dev.md) (writing Aperio
  programs), [`library-dev.md`](./agents/library-dev.md)
  (extending the stdlib), and
  [`compiler-dev.md`](./agents/compiler-dev.md) (working on the
  compiler itself).
- **`crates/aperio-codegen/tests/fixtures/examples/`** — small
  example programs the parser is anchored against. Read them as
  a tour of language features.

## Layout

```
spec/                       grammar + semantics + design rationale
agents/                     role-organized briefs for AI-assisted dev
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
~70-program `examples/` fixture set plus per-feature tests under
`crates/aperio-codegen/tests/`.

Breaking changes are expected; pin to a commit if you build on it.

## License

TBD. The project is in design exploration; licensing decisions
follow first public release.
