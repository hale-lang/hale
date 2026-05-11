# Aperio

> **Aperio** /ah-PEH-ree-oh/ — Latin: *I open. I reveal.*

A programming language whose primitives are the **lotus**
framework's coordination primitives. An Aperio program runs by
*opening a lotus* — a tree of *loci* communicating via a typed
bus, with per-region arenas, lifecycle invariants, and
closure-asserted correctness.

The language is **Aperio**. The runtime substrate it produces
is **a lotus**. C-runtime symbols (`lotus_arena_*`, `lotus_bus_*`,
`lotus_tcp_*`) keep the *lotus* prefix on purpose — they are
substrate mechanics, not Aperio's user-facing toolchain.

File extension: `.ap`. Source is ASCII-only outside string
literals and comments.

## Quick start

```
cargo build --workspace
cargo run -p aperio-cli --bin aperio -- run examples/hello-world/main.ap
cargo test --workspace
```

Working CLI:

```
aperio lex   <file>           tokenize and print tokens
aperio parse <file>           parse and print the AST
aperio check <file | dir>     parse + typecheck
aperio run   <file | dir>     parse + typecheck + interpret
aperio build <file>           parse + typecheck + emit native binary
```

## Docs

Five mdbook subtrees under [`docs/`](./docs/), each a different
doorway into the same material:

- [`docs/quickstart/`](./docs/quickstart/) — five-minute install + tour.
- [`docs/grimoire/`](./docs/grimoire/) — meta-spell register: Aperio
  as the spell of spellcasting.
- [`docs/book/`](./docs/book/) — substrate-up tutorial.
- [`docs/reference/`](./docs/reference/) — formal grammar + semantics
  + glossary + [behavior index with implementation status](./docs/reference/src/behavior-index.md).
- [`docs/std/`](./docs/std/) — stdlib reference + roadmap +
  [capability matrix for app developers](./docs/std/src/ready-today.md).

See [`docs/README.md`](./docs/README.md) for local build/preview
and [`docs/STYLE.md`](./docs/STYLE.md) for authoring conventions.

For agent sessions building Aperio programs:
[`notes/agent-onboarding/app-dev-brief.md`](./notes/agent-onboarding/app-dev-brief.md).

## Status

**Phases 1–5 of the v1.x stdlib roadmap are sealed; Phase 6
substrate (forced by the IDE design plan) is underway.**

Compiler:

- **Frontend:** lex / parse / typecheck. F.1–F.18 type rules
  enforced (contract compatibility, closure cycle, match
  exhaustiveness, k_max, immutable bindings, etc.).
- **Reference runtime (interpreter):** executes the example
  ladder end-to-end except multi-binary fitter-applier-pair, which
  waits on cross-process entry-point selection.
- **Native codegen (LLVM):** emits ELF binaries. Full
  lifecycle quartet (birth / accept / run / drain / dissolve),
  per-locus arenas, bus router with sync + ring-buffer
  transports, F.9 closure-epoch matrix, recovery primitives,
  cooperative + pinned schedule classes with cross-thread
  mailboxes, AF_INET / AF_UNIX cross-process bus framing,
  function pointers, scope-bound dissolve for let-bound loci,
  bus subject wildcards.

Stdlib (`std::*` namespaces, all shipped today): `process`,
`env`, `str`, `time`, `io::tcp`, `io::fs`, `http`, `text`,
`test`, `log`. Phase 6 in progress: `bus::expose`, `fs::watch`,
graphics, UI, shell, MCP, compiler self-introspection (planned).

For a per-feature implementation status table:
[`docs/reference/src/behavior-index.md`](./docs/reference/src/behavior-index.md).

For per-phase shipped-history tables:
[`spec/stdlib.md`](./spec/stdlib.md).

## What this is

Aperio's primitives — capacity-allocation, multi-perspective
stability, cyclic-closure, region-based memory, contract-graded
visibility — are language-native, not library conventions.
Concretely:

- **Loci as first-class entities.** Each locus declares its
  capacity parameters (B, c, σ, φ); the compiler computes
  k_max and enforces it as a static invariant.
- **Projection classes** (rich / chunked / recognition) as a
  type-system primitive. Same source code, different generated
  allocator depending on declared / inferred N.
- **Three modes** (bulk / harmonic / resolution) as a
  kernel-application primitive: define a kernel once; the
  compiler generates three projections sharing the locus's
  arena.
- **Region-based memory** with contract-graded visibility.
  Each locus's arena is a sub-region of its parent's; access
  between loci is mediated by typed contracts. No GC, no
  borrow checker.
- **Cyclic-closure tests** as syntactic constructs. Closure
  failure produces a typed `ClosureViolation` event, distinct
  from structural failures. Collapse vs. explosion as the two
  dissolution modes.
- **Lifecycle as a parent-policy state machine.** Failure
  capture, recovery primitives (`restart`, `quarantine`,
  `bubble`, `restart_in_place`), and dissolution are
  language-native. `drain()` always cascades depth-first.
- **Transport-agnostic typed bus.** Source declares subjects
  + types; deployment maps subjects to transports (NATS, UDP,
  TCP, AF_UNIX, in-memory). Subject wildcards (`**`) for
  cascade-style routing.

## Design commitments locked

The v0 spec locks the following commitments (see
[`spec/design-rationale.md`](./spec/design-rationale.md)):

| Ref  | Commitment |
|------|------------|
| F.1  | Optimize for runtime perf over compile-time perf, behavior preserved |
| F.2  | `ProjectionClass` as built-in any-of-three constraint |
| F.3  | Per-arena defrag/free-list, no whole-program GC |
| F.4  | `drain()` always cascades depth-first |
| F.5  | Mode projections share the locus's arena |
| F.6  | Lifecycle methods are not implicit loci |
| F.7  | `accept()` runs before child birth |
| F.8  | Contract compatibility is type-checked across coordinator/coordinatee |
| F.9  | Collapse vs. explosion + parent on_failure routing (absorb / bubble) |
| F.10 | Mode keywords accepted post-dot as member names |
| F.11 | `self.children` typing and lifecycle |
| F.12 | Bus send is `<-`; subscribe is declarative |
| F.13 | Bus subscription handler signature |
| F.14 | Three-way interface; translation return type ⊆ contract |
| F.15 | Predefined type names are PascalCase, not keywords |
| F.16 | `self.k_max` as built-in computed field (F.1 made executable) |
| F.17 | Strict field-access; method types on locus / perspective values |
| F.18 | Match exhaustiveness checked at typecheck |

## Design lineage

The substrate-invariant coordination primitives Aperio inherits
come from **the ancient texts** — a body of older
coordination-primitives work whose form Aperio inherits and
makes executable. The shapes were already there; this repo
formalizes them in compilable language.

## Layout

```
spec/                       formal grammar + memory model + design rationale
examples/                   ~50 pedagogical programs (hello-world → fitter-applier-pair)
apps/                       production-shape Aperio programs (real apps, not demos)
docs/{quickstart,grimoire,book,reference,std}/   five mdbook trees
notes/                      design plans, friction logs, agent-onboarding
crates/
  aperio-syntax/            lexer + parser + AST + diagnostics
  aperio-types/             symbol resolution + typechecker
  aperio-runtime/           tree-walking interpreter + bus router
  aperio-codegen/           LLVM codegen + bundled C runtime under runtime/
  aperio-cli/               the `aperio` binary
```

## Naming

- **Aperio** — the language; the toolchain (`aperio build`,
  `aperio run`).
- **a lotus** — the runtime substrate an Aperio program *is*.
  Lowercase except where grammar demands capitalization.
- **a locus** (plural **loci**) — the unit of structure inside
  a lotus.

The runtime substrate concept *predates Aperio* in the ancient
texts. Aperio is the same form projected into compilable
surface.

## License

TBD. Project status is design exploration; licensing decisions
follow first public release.
