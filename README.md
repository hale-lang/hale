# Aperio

> **Aperio** /ah-PEH-ree-oh/ — Latin: *I open. I reveal.*
>
> A programming language whose primitives are the **lotus** framework's
> coordination primitives. An Aperio program is a spell cast at compile
> time; running it opens a *lotus* — a structure that grows itself, audits
> its own correctness, and dissolves cleanly when its work is done.

The language is **Aperio**. The runtime substrate it produces is a **lotus**:
a tree of *loci* communicating via vertical-only-flow over a shared bus, with
per-region arenas and closure-asserted invariants. Aperio is the spell; the
lotus opens under it.

(The crate name `crates/aperio-codegen/runtime/lotus_arena.c` and C-runtime
symbols like `lotus_arena_*` / `lotus_bus_*` retain the *lotus* name on
purpose — they're the substrate's mechanics, not Aperio's user-facing
toolchain. Aperio is the language; lotus is what an Aperio program *is*
at runtime.)

## Docs

Aperio's documentation lives under [`docs/`](./docs/) as three mdbook
subtrees:

- **The Aperio Programming Language** ([`docs/book/`](./docs/book/)) —
  layered tutorial.
- **The Aperio Reference** ([`docs/reference/`](./docs/reference/)) —
  formal grammar + semantics.
- **The Aperio Standard Library** ([`docs/std/`](./docs/std/)) —
  stdlib roadmap (libraries in active development).

The docs are local-only until the language is ready for public release.
See [`docs/README.md`](./docs/README.md) for local build/preview
instructions; [`docs/STYLE.md`](./docs/STYLE.md) for authoring
conventions.

**Status.** v0 compiler runs Aperio programs end-to-end via a
tree-walking interpreter AND emits native ELF binaries via LLVM
for a substantial subset of the language including the full
lifecycle quartet (`birth` / `accept` / `run` / `drain` /
`dissolve`), user-defined `type` declarations + composite
defaults + nested field reads + heap-allocated literals, the
**bus router** (typed pub-sub via `<-`), `Decimal` / `Time`
primitives, `self.method()` calls, `return n` from main →
process exit code, the **full closure-test runtime** (collapse
+ absorb + bubble per F.9), built-in `ClosureViolation`,
**modes** (`mode bulk()` / `harmonic()` / `resolution()`),
**`self.children`** + `for child in self.children { ... }`,
locus literals in expression position, (m19→m23) the
**lotus region allocator** with **per-locus arenas**, **bus
copy semantics**, and **per-projection-class strategies** —
every locus carries a `__arena: ptr` field freed wholesale at
dissolve, chunked-class parents carve sub-regions for accepted
children with free-list slot reuse, recognition is a documented
stub behind the same path, and `<-` copies payloads from the
publisher's arena into the subscriber's arena per
spec/memory.md, (m24) **match expressions** with Literal /
Wildcard / Binding patterns, (m25) **bimodal schedule-class
annotations** (`: schedule cooperative | pinned`), (m26)
**cooperative scheduler semantics** — bus dispatch is deferred
via a process-wide FIFO queue; cells run between substrate
yield points instead of nesting in publisher frames, and (m26b)
**explicit `yield`** as the user-placed cell boundary for
long-internal-loop bodies, (m27 + m28a) **pinned threads with
full lifecycle** — pinned-class loci spawn a real pthread at
instantiation; the locus's full declared lifecycle (birth /
run / drain / dissolve) executes on that thread in order,
(m28b) **cross-thread bus mailboxes** — pinned loci can
subscribe and publish; cells route between threads via
per-locus mutex+condvar mailboxes carrying inline payloads,
with coordinated shutdown via shutdown-flag-then-join, and
(m28c) **`pinned(core = N)` CPU affinity** — pinned loci
optionally bind their thread to a specific logical CPU via
`pthread_setaffinity_np`. **54 of 55 examples build to native
ELF — every single-binary example is a build target.** Phase 3
(codegen) is past milestone 56: substrate parity now covers
the full F.9 closure-epoch matrix (Birth + Dissolve + Tick +
Duration + Explicit) with `sum` / `count` / `mean` accumulators
and `restart` / `restart_in_place` / `quarantine` recovery,
exact i128 fixed-point Decimal arithmetic in both backends
(m48), and full-fidelity tagged-union enums with payload
variants — including deep `==`, payload-rendering `println`
/ `to_string`, bus dispatch carrying enum values, literal
sub-patterns in match arms, and per-enum representation that
keeps no-payload enums as i32 tags while has-payload enums
become arena-allocated `{ i32 tag, [N x i8] body }` storage
pointers. m49 + m51 + m53 ship the full **free-fn implicit-
locus arenas** arc: every non-main free fn opens a per-call
subregion of its caller's arena, allocations route through
it, heap-typed return values deep-copy back into the caller's
arena (across String / Tuple / Array / TypeRef-struct /
has-payload-Enum), and any long-lived loci bound in the fn
body get drained + dissolved at fn return per spec/memory.md
"Free `fn` functions". m50 enforces immutable bindings (`let
x = 0; x = 1;` is now the compile-time error spec/types.md
always said it should be). m52 closes the cells-leaked-during-
dissolves gap with an in-loop drain after each iteration of
the dissolve loop. m54 lifts default param values into mode
methods. m55 cleans the recovery vocabulary down to its
five-primitive minimum (drain/dissolve removed; they're
lifecycle methods, not recovery ops). m56 is a docs-only pass
that locks seven design decisions per The Design (runtime/
stdlib transport split, fan-out semantics, `reorganize` =
`restart_in_place` lifted, `Numeric` bound for v1 generics).
Detailed per-milestone history lives in `CHECKPOINT.md`. The codegen toolchain also covers literals +
arithmetic, `let`/`let mut` + assignment + compound ops,
`if`/`else`/`while` + `break`/`continue`, `time::sleep` on
`CLOCK_MONOTONIC` with EINTR retry, `time::monotonic()` +
Duration arithmetic / comparisons, user-defined fns (typed
params + return + recursion), the **locus runtime ABI** (each
locus → LLVM struct, lifecycle methods take `self_ptr`,
`self.X` reads/writes via `getelementptr`), parent-child
**`accept()` lifecycle** with F.7 ordering (accept fires before
child birth), **`drain()` / `dissolve()` lifecycle methods**
with F.4 depth-first cascade (children dissolve before parent),
**user-defined `type` declarations** (struct literals + GEP
field access), and the **bus router** (heap-grown C-runtime
dynamic vec backing subscriptions; long-lived loci with `bus
subscribe` defer drain/dissolve to enclosing-scope exit so they
outlive synchronous publishes).

Phase 0 (spec stabilization + example ladder) and Phase 1
(compiler frontend: lex / parse / typecheck) are complete. The
v0 runtime (Phase 2 first cut) is a tree-walking interpreter
that executes 54 of 55 example projects end-to-end (only
multi-binary trellis-pair waits on cross-process bus),
including the trellis-demo pipeline. The bus router has a Transport
trait with two implementations (sync dispatch, LMAX-style ring
buffer); the typechecker enforces the framework's distinctive
primitives (F.8 contract compatibility, closure cycle
existence, match exhaustiveness, k_max as a built-in field).

Quick start:

```
cargo build
cargo run --bin lotus -- run   examples/hello-world/main.lt
cargo run --bin lotus -- build examples/02-parent-child/main.lt
./examples/02-parent-child/main                       # 3× "greeting from child"
cargo test --workspace                                # 90 tests
```

Working CLI commands: `lex`, `parse`, `check`, `run`, `build` —
single-file or whole-project (multi-file bundle) targets.

Full delivery plan to team-wide-internal v1.0:
`~/.claude/plans/witty-foraging-lightning.md`

## What this is

Lotus is a compile-time language designed around the alpha-
conjecture program's substrate-invariant coordination primitives.
Concretely:

- **Loci as first-class entities.** Each locus declares its
  capacity parameters (B, c, σ, φ); the compiler computes its
  k_max and enforces it as a static invariant.
- **Projection classes (rich / chunked / recognition) as a
  type-system primitive.** Same source code, different generated
  allocator depending on declared / inferred N.
- **Three modes (bulk / harmonic / resolution) as a kernel-
  application primitive.** Define a kernel once; the compiler
  generates three projections sharing the locus's arena.
- **Region-based memory** with contract-graded visibility. Each
  locus's arena is a sub-region of its parent's; access between
  loci is mediated by typed contracts; deeper looking costs more.
  No GC, no borrow checker. Per-arena defrag for bookkeeping
  reclamation.
- **Cyclic-closure tests as syntactic constructs.** The language
  enforces audit invariants the framework already commits to.
  Closure failure produces a typed `ClosureViolation` event,
  distinct from structural failures (panic). Collapse vs.
  explosion as the two dissolution modes.
- **Hot-load of perspectives** as a first-class language feature.
  Stable perspectives cross from analyst-locus to executor-locus
  as typed parameter bundles within a shared compiled schema.
- **Lifecycle as a parent-policy-driven state machine.** Failure
  capture, recovery primitives (`restart`, `quarantine`,
  `reorganize`, `bubble`), and dissolution are language-native.
  `drain()` always cascades depth-first.
- **Three-way interface (locus + parent + contract).**
  Translation functions injected by a locus into its arena are
  bounded above by the contract's typed surface. Contract is
  the interface; translations are implementations; multiple
  implementations per field can coexist.
- **Multi-scheduler cooperative runtime** (BEAM-shaped, not
  M:N). Per-scheduler region allocators; failures within a
  scheduler are stack walks; cross-scheduler is bus-mediated.
- **Transport-agnostic typed bus.** NATS, UDP multicast, TCP,
  Unix sockets, in-memory all implement `std::bus::Adapter`.
  Source declares subjects + types; deployment maps subjects to
  transports.

## Design commitments locked

The v0 spec locks the following commitments (see `spec/design-
rationale.md` §F.1–F.18):

| Ref | Commitment |
|---|---|
| F.1 | Optimize for runtime perf over compile-time perf, behavior preserved |
| F.2 | `ProjectionClass` as built-in any-of-three constraint |
| F.3 | Per-arena defrag/free-list, no whole-program GC |
| F.4 | `drain()` always cascades depth-first |
| F.5 | Mode projections share the locus's arena |
| F.6 | Lifecycle methods are not implicit loci |
| F.7 | `accept()` runs before child birth |
| F.8 | Contract compatibility is type-checked across coordinator/coordinatee |
| F.9 | Collapse vs. explosion + parent on_failure routing (absorb / bubble) |
| F.10 | Mode keywords accepted post-dot as member names |
| F.11 | `self.children` typing and lifecycle |
| F.12 | Bus send is the `<-` operator; subscribe is declarative |
| F.13 | Bus subscription handler signature |
| F.14 | Three-way interface; translation return type ⊆ contract |
| F.15 | Predefined type names are PascalCase, not keywords |
| F.16 | `self.k_max` as built-in computed field (F.1 made executable) |
| F.17 | Strict field-access; method types on locus / perspective values |
| F.18 | Match exhaustiveness checked at typecheck |

## Design lineage

This language is the natural compile-time collapse of the
alpha-conjecture program's existing design-time work:

- `~/notes/alpha-conjecture/` — the research program: framework
  primitives (capacity-allocation, multi-perspective stability,
  substrate-derivation discipline, cyclic-closure), paper-4
  closed-horizon-recursion, theory & methodology.
- `~/code/brain3/` — the existing software-substrate
  operationalization (production deployment); the empirical
  anchor at the software substrate.
- `~/notes/alpha-conjecture/lotus/` — the portable agent-facing
  distillation of the framework for software design.
- `~/code/grease/` — bus pattern, decimal usage, harness shape;
  closest existing exemplar of "lotus-shaped Go program."

The language is a recognition event: the form is already
constrained by the closed graph above. This repo formalizes it.

## Layout

```
spec/
  grammar.ebnf            483 lines  formal grammar (source of truth)
  tokens.md               327 lines  lexical structure
  precedence.md           123 lines  operator precedence
  design-rationale.md   1,329 lines  why each construct looks this way
  runtime.md              258 lines  what the lotus binary ships with
  stdlib.md               272 lines  batteries-included module map
  testing.md              247 lines  3-layer testing pipeline design
  memory.md               406 lines  formal memory model + codegen ABI
  types.md                320 lines  type system rules
  semantics.md            357 lines  operational semantics

examples/
  hello-world/            minimal Aperio program (one locus, birth)
  01-locus-with-run/      run() lifecycle, mut bindings, time::sleep
  02-parent-child/        contract expose/consume, accept, parent-child
                          memory hierarchy
  03-closure-test/        closure declaration, ~~ operator, collapse
                          (clean dissolution)
  03b-closure-absorbed/   F.9 absorb path: parent on_failure handles
                          ClosureViolation
  03c-closure-bubbled/    F.9 bubble path: bubble(err), no further
                          handler → process exits non-zero
  04-modes/               bulk/harmonic/resolution, self.children
                          iteration
  05-bus/                 typed pub-sub via in-process router; sender
                          + echo + ack-logger
  06-mutable-counter/     `let mut` + plain/compound assignment via
                          codegen (hand-unrolled, no control flow)
  07-control-flow/        if/else/while + break/continue via codegen;
                          folds 06's counter into a loop
  08-monotonic-sleep/     time::sleep on CLOCK_MONOTONIC with EINTR
                          retry; identical observable behavior on
                          interpreter and codegen paths
  09-functions/           user-defined fns: typed params + return,
                          recursion, void fns, calls in expression
                          and statement position
  10-stateful-locus/      locus runtime ABI: locus → LLVM struct,
                          lifecycle methods take self_ptr, self.X
                          reads/writes via getelementptr; state
                          flows from birth → run through the same
                          alloca'd struct
  11-drain-dissolve/      drain() and dissolve() lifecycle methods:
                          F.4 depth-first cascade via synchronous
                          instantiation; identical interpreter +
                          codegen output
  12-user-types/          user-defined `type` declarations as plain
                          data records: struct literals + GEP field
                          access; substrate for the bus router
  13-decimal-and-exit/    Decimal type + arithmetic + return-from-main
                          mapping to process exit code
  14–42/                  control flow, arrays, ranges, tuples,
                          strings, scheduler classes, pinned threads,
                          F.9 closure epochs (birth / dissolve / tick
                          / duration / explicit), recovery primitives
                          (restart / restart_in_place / quarantine),
                          accumulators (sum / count / mean), bus
                          (cooperative + pinned + multi-instance)
  43-enums/               m47: tagged-union enums; pure no-payload
                          variants represented as i32 tags with
                          variant-coverage exhaustiveness in match
  44-decimal-exact/       m48: exact i128-backed Decimal arithmetic;
                          1d/3d → 0.333333333 across both backends
  45-enum-payloads/       m47-payloads: payload-bearing enum variants
                          with bindings + literal sub-patterns,
                          deep ==, payload-rendering println /
                          to_string, bus dispatch
  46-fn-arenas/           m49: per-call free-fn subregion + return-
                          copy for value types + String + Tuple
  47-fn-arenas-extras/    m51: deep-copy completion for Array /
                          TypeRef-struct / has-payload-Enum returns
  48-publish-during-      m52: dissolve-time publish dispatched to
    dissolve/             still-alive subscribers via in-loop drain
  49-fn-handle-rooting/   m53: long-lived loci bound in a free fn
                          drain + dissolve at fn.exit per spec
  50-mode-defaults/       m54: default param values on mode methods
  trellis-demo/           full producer→analyst→executor→logger
                          pipeline as one process; F.4 program-end
                          dissolve fires the analyst's audit closure
  trellis-pair/           production form: analyst + executor as
                          separate binaries on shared schema. Spec-
                          gate program; runs as two-binary form when
                          cross-process bus + entry-point selection
                          land.

notes/
  open-questions.md       deferred decisions and future directions

crates/                   (Phase 1 + 2 v0 + Phase 3 milestones 0–56 + free-fn arena arc + immutable-binding enforcement + recovery-vocabulary cleanup)
  lotus-syntax/           lexer + parser + AST + diagnostics
  lotus-types/            symbol resolution + type checker (F.8,
                          field strictness, closure cycle, match
                          exhaustiveness, k_max recognition)
  lotus-runtime/          tree-walking interpreter + bus router
                          (Transport trait: SyncDispatch / RingBuffer)
                          + dissolve cascade (F.4 + F.9, with drain
                          bodies invoked); time::sleep / monotonic
                          via libc::clock_* on CLOCK_MONOTONIC
  lotus-codegen/          LLVM codegen (inkwell + llvm-18). Subset:
                          literals, arithmetic, let mut + assignment,
                          control flow, time::sleep + monotonic,
                          user-defined fns, the locus runtime ABI,
                          the full lifecycle quartet (birth + accept
                          w/ F.7 ordering + run + drain + dissolve
                          w/ F.4 cascade) + self.X / g.X via GEP +
                          user `type` decls (struct literals + field
                          reads + composite defaults + nested field
                          reads + heap allocation) + locus `fn`
                          members + bus router (subscribe / publish /
                          `<-` dispatch with long-lived deferral) +
                          closures with F.9 collapse / absorb / bubble
                          + ClosureViolation built-in type + modes +
                          self.children + for-loops + locus literals
                          in expression position
  lotus-cli/              `lotus` binary (lex / parse / check / run /
                          build)
  lotus-codegen/runtime/  C source for the lotus region allocator,
                          bundled into the codegen crate via
                          include_str! and compiled + linked into
                          every binary alongside the generated
                          object file (m19 substrate).
```

Example ladder: 30 projects from hello-world → trellis-pair;
~1,350 lines of source + ~1,400+ lines of README walk-throughs.
91 tests across the workspace; 29 of 30 projects run end-to-end
under `lotus run` (only multi-binary trellis-pair waits on the
cross-process bus). **29 of 30 projects** also build to native
ELF via `lotus build` — every single-binary example. Only
`trellis-pair` (cross-process bus + entry-point selection) is
not a build target.

## Toolchain

Working today:

```
lotus lex   <file>           tokenize and print tokens
lotus parse <file>           parse and print the AST
lotus check <file | dir>     parse + typecheck (the full F-rules)
lotus run   <file | dir>     parse + typecheck + interpret
lotus build <file>           parse + typecheck + emit native ELF
                              (Phase 3, milestone-18 subset)
```

Per `spec/testing.md`, the planned full surface adds:

```
lotus test        run all *_test.lt files
lotus bench       run all *_bench.lt files
lotus bench -compare  build + run external-language equivalents alongside
lotus verify      framework-discipline checks specifically (no execution)
lotus fmt         canonical formatter (zero config)
```

JSON output for CI consumption; tree-sitter grammar derived from
EBNF for editor support.

## Implementation phases

Per the delivery plan:

- **Phase 0** — Spec stabilization. *Complete.*
- **Phase 1** — Compiler frontend in Rust (parse + typecheck).
  *Complete (v0).* All F.1–F.18 typecheck rules enforced;
  9/9 example projects parse and typecheck cleanly.
- **Phase 2** — Reference runtime in Rust. *v0 complete
  (interpreter).* 8/9 example projects execute end-to-end;
  bus router with pluggable transports; closure semantics
  with collapse / absorb / bubble; program-end dissolve.
  Region allocator + cooperative scheduler are the remaining
  Phase 2 deep-pushes.
- **Phase 3** — Codegen in Rust targeting LLVM. *In progress;
  milestone 30 of N complete.* Working subset: literals, arithmetic,
  `let`/`let mut` + assignment + compound ops, mixed-type println,
  if/else/while + break/continue, `time::sleep` + `time::monotonic`
  on `CLOCK_MONOTONIC` with EINTR retry, Duration / Decimal /
  Time primitives, user-defined fns (typed params + return +
  recursion), the locus runtime ABI, the full lifecycle quartet
  (birth + accept w/ F.7 + run + drain + dissolve w/ F.4
  cascade), user-defined `type` declarations + struct literals +
  field reads + composite locus param defaults + heap-allocated
  literals + nested field reads, the bus router (`<-` dispatch +
  long-lived locus deferral), `self.method()` calls, `return n`
  from main → process exit code, the full closure-test runtime
  (F.9 collapse / absorb / bubble + built-in `ClosureViolation`),
  modes (`mode bulk()` etc.), `self.children` + `for child in
  self.children { ... }` iteration, locus literals in
  expression position, the **lotus region allocator substrate**
  (m19), **per-locus arenas + bus copy semantics** (m20), and
  **per-projection-class strategies** (m22 + m23) — every locus
  carries a `__arena: ptr` field freed wholesale at dissolve,
  chunked-class parents carve sub-regions for accepted children
  with free-list slot reuse, recognition is a documented stub
  behind the same path, and `<-` copies payloads between
  publisher / subscriber arenas as the spec requires,
  (m24) **match expressions** with Literal / Wildcard / Binding
  patterns (Tuple / Constructor + guards remain
  interpreter-only), (m25) **bimodal schedule-class annotations**
  (`: schedule cooperative | pinned` — no third "greedy" class
  per bimodality), (m26) **cooperative scheduler semantics** —
  bus dispatch is deferred via a process-wide FIFO queue; cells
  run between substrate yield points instead of nesting in
  publisher frames, (m26b) **explicit `yield`** as the rare
  user-placed substrate cell boundary for long-internal-loop
  bodies, (m27 + m28a) **pinned threads with full lifecycle** —
  pinned-class loci spawn a real pthread at instantiation; the
  full declared lifecycle (birth → run → drain → dissolve, each
  only if declared) executes on that thread, in order; deferred
  `pthread_join` at scope exit, and (m28b) **cross-thread bus
  mailboxes** — pinned loci can subscribe and publish; bus
  dispatch routes cells via per-locus mutex+condvar mailboxes
  with inline payloads (handler runs on the pinned thread,
  publisher unaffected); coordinated shutdown via
  shutdown-flag-then-join. The substrate cost lives at the
  layer boundary, not inside either layer's arena. Spec-aligned
  per `spec/runtime.md::Schedule classes`. **23 of 24 example
  projects compile to native ELF — every single-binary
  example.** Only `trellis-pair` (cross-process bus +
  entry-point selection) remains, gated on substantial new
  infrastructure.
- **Phase 4** — Stdlib v0 in lotus + Rust FFI shims. Overlaps
  Phase 3.
- **Phase 5** — Toolchain. Overlaps Phase 3–4.
- **Phase 6** — Self-host (compiler rewrite in lotus).
- **Phase 7** — Trellis production deployment. Parallel.
- **Phase 8** — v1.0 stabilization.

Implementation strategy: **Rust bootstrap → self-host in lotus**.
The compiler-in-lotus milestone is the empirical anchor for the
framework's substrate-invariance claim at the compiler-internals
substrate.

## Naming

The framework's existing meta-framework is called "lotus" (see
the `lotus/` subdirectory of the alpha-conjecture program). This
language is named "lotus" for the same reason: it's the same
form, projected from design-time into compile-time. The two are
expected to converge.

File extension: `.lt`.

## License

TBD. Project status is design exploration; licensing decisions
follow first compiler work.
