# Lotus — session checkpoint

**Read this first** if you're picking up the lotus language work in a
new session. State as of codegen milestone 24 (`match` expressions
in codegen) on top of the m19→m23 region-allocator arc. **19 of
20 examples build to native ELF — every single-binary example,
including `14-projection-classes` (m22+m23 smoke test) and
`15-match` (m24 smoke test).** Only `trellis-pair`
(multi-binary, cross-process bus) remains, gated on substantial
new infrastructure.

Two design decisions landed in the prior session and are now
guiding the substrate work: the runtime/stdlib split for bus
transports (kernel primitives in runtime; protocols in stdlib)
and the observation that producer/consumer cardinality on a
subject is emergent from locus connectivity, not a transport
configuration. Both documented below in their own sections.

**Active arc — region allocator (F.3).** The user has signaled
focus on substrate-level work (deeper locus, away from
application surface): trellis-pair waits until the language is
done, since the application will flex the runtime's full
surface anyway. Per-projection-class arenas are the next
deep-push. m19 (this commit) replaces libc malloc with a
lotus-controlled bump allocator wholesale-freed at program
exit; subsequent milestones will scope arenas to loci (m20),
add bus copy semantics (m21), and bring chunked + recognition
strategies online (m22, m23). Task IDs 19–23 track the arc.

This is part of the alpha-conjecture program (see
`~/notes/alpha-conjecture/CLAUDE.md`). Lotus is the language-substrate
arm — a programming language whose primitives are the framework's
coordination primitives.

## Where we are

A working compiler that **runs** lotus programs end-to-end (tree-
walking interpreter) AND **produces** native ELF binaries (LLVM via
inkwell) for a substantial subset including loci with `run()` and
parent-child `accept()` lifecycle methods. 91 tests pass across
the workspace.

```
$ lotus run examples/02-parent-child/main.lt    # interpreter path
greeting from child: hello
greeting from child: hi
greeting from child: yo

$ lotus build examples/02-parent-child/main.lt  # codegen path
built: examples/02-parent-child/main
$ ./examples/02-parent-child/main
greeting from child: hello
greeting from child: hi
greeting from child: yo
```

Phase status:
- **Phase 0** (spec stabilization) — complete
- **Phase 1** (lex / parse / typecheck) — complete; F.1–F.18 enforced
- **Phase 2 v0** (interpreter + bus router) — 17 of 18 example
  projects execute end-to-end via `lotus run` (only multi-binary
  trellis-pair waits on cross-process bus)
- **Phase 3 milestone 24** (`match` expressions) — complete.
  Match statements lower to LLVM as a chain of test-blocks +
  body-blocks, falling through to the next arm on mismatch.
  Patterns supported: `Literal` (Int / Bool / Duration / Float /
  Decimal), `Wildcard`, and `Binding(x)` (binds the scrutinee to
  `x` for the arm body, with shadow/restore of any prior local
  with the same name). `Tuple` / `Constructor` patterns + arm
  guards remain interpreter-only. F.18 exhaustiveness is
  enforced upstream by the typechecker, so the post-arms
  fallthrough block is unreachable for well-typed programs.
  Match arm bodies handle `Call` exprs by routing through
  `lower_stmt` (so `println` / void-returning user fns work
  identically to statement-position calls). New
  `examples/15-match/` exercises Int + Bool + Binding patterns.
- **Phase 3 milestones 22 + 23** (per-projection-class arena
  strategies) — complete. Each locus's projection class
  resolves from `: projection <class>` annotation or per-spec
  default rule (chunked if accept declared, rich otherwise) at
  declare-locus-struct time. m22 wires chunked parents through
  `lotus_arena_create_subregion`: each accepted child gets a
  sub-region carved from the parent's bookkeeping space, with
  slot indices reused via a free-list when children dissolve.
  m23 lights up the recognition annotation behind the same
  sub-region path — the pre-allocated bitmap-cell pool
  optimization is deliberately deferred until a workload
  exercises it, and that gap is documented in
  `spec/memory.md`. New `examples/14-projection-classes/`
  exercises all three classes end-to-end.
- **Phase 3 milestone 20** (locus-owned arenas + bus copy
  semantics) — complete. Every locus struct now carries a
  synthetic `__arena: ptr` field at struct slot 0; instantiation
  fills it via `lotus_arena_create()`; the per-locus arena is
  wholesale-freed via `lotus_arena_destroy` after `dissolve()`
  runs (both the ephemeral path and the deferred long-lived
  flush). Allocations route through three tiers: an explicit
  override (used during locus-instantiation field init so
  composite-default literals land in the new locus's arena), the
  enclosing locus's arena field (when `current_self` is set), or
  the program-wide arena (`@lotus.arena.global`, used in `main`
  and free fns). Bus dispatch implements the spec's "typed
  message crossing a locus boundary is a copy, not a pointer"
  rule: each `<-` passes the payload's compile-time size to
  `lotus.bus_dispatch`, which allocates `size` bytes in each
  matching subscriber's arena (loaded from `self_ptr + 0`,
  the fixed arena-field offset), memcpy's the payload, and
  passes the COPY to the subscriber's handler. Trellis-demo's
  `self.current_kernel = msg` pattern now actually works under
  per-locus arenas — subscriber's stored copy outlives publisher
  locus dissolution.
- **Phase 3 milestone 19** (region allocator substrate) —
  complete. The codegen path now links a small C arena runtime
  (`crates/lotus-codegen/runtime/lotus_arena.c`, bundled into the
  compiler via `include_str!`) into every emitted binary. ABI:
  `lotus_arena_create()` / `lotus_arena_alloc(arena, size, align)`
  / `lotus_arena_destroy(arena)`. An arena is a linked list of
  bump chunks (default 64 KiB; oversized requests get a fresh
  chunk sized to fit); allocation is pointer-bump in the head
  chunk, destruction walks + frees wholesale.
- **Phase 3 milestone 18** (codegen subset). **17 of 18 example
  projects build to native ELF — every single-binary example.**
  Modes (lowered as locus methods named bulk/harmonic/resolution;
  callable via `self.<mode>()`), built-in `self.children` array
  (fixed-cap 16, embedded after user fields on every locus that
  declares accept; appended at accept dispatch + counter bumped),
  `for child in self.children { ... }` lowered as an indexed
  loop with the var bound as a LocusRef-typed local, and locus
  literals in expression position so `let _l1 = LeafL { ... }`
  works. Interpreter parity: replaced the m10 dedup-pop with a
  `dissolved: Cell<bool>` flag on LocusHandle so ephemeral
  handles stay in parent.children (for `for child in
  self.children`) but the parent's later cascade skips
  already-dissolved children.
- **Phase 3 next** — cooperative scheduler (BEAM-shaped multi-
  scheduler runtime so loci with `run()` can yield + resume +
  receive bus messages out of band). Big arc; the natural
  follow-on after the region-allocator substrate is solid.
  `trellis-pair` (cross-process bus + entry-point selection)
  remains deferred until the substrate is ready — the
  application exercises the full runtime, but the substrate
  doesn't bend toward it.

## Transport layering (decided 2026-05-08)

Runtime / stdlib split for bus transports:

- **Runtime owns kernel-level IO primitives** + thin `Transport`
  adapters that wrap them: shared memory (`shm_open` + `mmap` +
  atomic indices), Unix domain sockets (`AF_UNIX`), TCP/UDP
  (`AF_INET` + multicast). Direct syscall plumbing wired into
  the bus router. `io_uring` / `epoll` / `kqueue` integration
  also lives here when the cooperative scheduler lands.
- **Stdlib owns protocols on top of those primitives**:
  `std::bus::nats` (NATS frames over TCP), `std::bus::mqtt`,
  `std::bus::http_sse`, `std::bus::grpc`. TLS lives in
  stdlib too (`std::tls`); serialization (json/protobuf/msgpack)
  in `std::encoding`.

This matches `spec/runtime.md`'s "transport-agnostic" framing —
runtime defines the `Adapter` interface, specific protocols
plug in from stdlib. The new clarification is that the runtime
*also* directly exposes the kernel primitives those protocols
need, rather than forcing every adapter to vendor its own
syscall wrappers.

## Producer/consumer cardinality is emergent (insight, 2026-05-08)

The standard MPSC / SPSC / SPMC / MPMC taxonomy doesn't
describe a transport configuration — it describes
**locus connectivity** on a subject. Count the loci with
`publish "X"` and the loci with `subscribe "X"` at link time:

| Publishers on X | Subscribers on X | Required machinery |
|---|---|---|
| 1 | 1 | SPSC — wait-free, no claim ticket |
| 1 | N | SPMC — Disruptor's natural shape |
| N | 1 | MPSC — fan-in queue with producer claim |
| N | N | MPMC — atomics on both sides |

In trellis-demo all subjects are SPSC / SPMC; **no MPMC
machinery needed**. That's a real speedup vs a uniform
"every subject is MPMC" runtime — SPSC rings are 5-10x faster
than MPMC ones.

The current `BusRouter` doesn't exploit this — it's uniform
MPMC-shaped. When the substrate gets more serious, per-subject
specialization is exactly the kind of optimization the
framework's coordination primitives unlock that a general
pub-sub library can't: **the locus surface carries the shape
information; the substrate gets to specialize.** Connects to
F.14 (three-way interface: locus + parent + contract) — the
contract surface declares data flow shape; bus declarations
declare connectivity shape; together that's enough to pick
the cheapest correct primitive.

## Codegen milestone arc (Phase 3 progress)

Each milestone below is one focused commit + a CHECKPOINT/README
refresh. The arc moved fast: nineteen milestones (m0–m18)
landed across two sessions in 2026-05-08, taking the codegen
path from "no-op stub" to "every single-binary example is a
build target." Each load-bearing piece was intentionally narrow:

```
m0  Phase 3 milestone 0: lotus build → native ELF via LLVM      (77b977f)
m1  Codegen milestone 1: Int / Float / Bool params + println    (5c9b6f7)
m2  Codegen milestone 2: let + Int/Float arithmetic + cmp       (5224d53)
m3  Codegen milestone 3: let mut + assignment                   (03c2f55)
m4  Codegen milestone 4: if / while / break / continue          (cae8c9a)
m5  Codegen milestone 5: time::sleep on CLOCK_MONOTONIC         (929efa2)
m6  Codegen milestone 6: multi-fn programs                      (9955bea)
m7  Codegen milestone 7: locus runtime ABI    ← load-bearing    (206fbd0)
m8  Codegen milestone 8: accept() + parent-child wiring         (d5afffd)
m9  Codegen milestone 9: time::monotonic() + Duration arith     (cdd7353)
m10 Codegen milestone 10: drain() / dissolve() lifecycle        (3ba3e05)
m11 Codegen milestone 11: user `type` decls + struct literals   (5cb4882)
m12 Codegen milestone 12: bus router (subscribe + <- + deferral)(5645eaa)
m13 Codegen milestone 13: self.method() calls                   (b036c7f)
m14 Codegen milestone 14: Decimal + return-from-main exit code  (b036c7f)
m15 Codegen milestone 15: closures (collapse-only path)         (9bf21c1)
m16 Codegen milestone 16: Time + composite defaults + heap lits (e33e8ee)
                          ⇒ trellis-demo builds to native ELF
m17 Codegen milestone 17: on_failure routing (absorb / bubble)  (4bf84e3)
                          ⇒ 03b / 03c build to native ELF
m18 Codegen milestone 18: modes + self.children + for + locus  (d48df6b)
                          literal in expression position
                          ⇒ 04-modes builds; 17/18 single-binary
                            examples are build targets
m19 Codegen milestone 19: region allocator substrate           (ea4892b)
                          ⇒ libc malloc removed; lotus_arena_*
                            backs every type-literal + ClosureViolation
                            allocation; same example ladder still passes
m20 Codegen milestone 20: locus-owned arenas + bus copy        (d511670)
                          ⇒ __arena field on every locus struct
                            (slot 0), lifecycle-bound; bus dispatch
                            copies payloads between publisher /
                            subscriber arenas per spec
m22 Codegen milestone 22: chunked-class sub-regions            (this commit)
                          ⇒ chunked parents allocate accepted
                            children via lotus_arena_create_subregion;
                            free-list bookkeeping reuses slot
                            indices as children dissolve
m23 Codegen milestone 23: recognition-class stub               (010db7a)
                          ⇒ recognition annotation parses /
                            resolves / dispatches; behaviorally
                            equivalent to chunked at v0; bitmap-
                            pool optimization deliberately deferred
                          + examples/14-projection-classes
m24 Codegen milestone 24: match expressions                    (this commit)
                          ⇒ Literal / Wildcard / Binding patterns
                            in codegen; Tuple / Constructor +
                            guards remain interpreter-only;
                            F.18 exhaustiveness still enforced at
                            typecheck
                          + examples/15-match
```

The architectural pivots are **m7** (locus → LLVM struct,
lifecycle methods take `self_ptr`, `self.X` via GEP) and **m8**
(accept's child param as `LotusType::LocusRef(String)`,
parent-aware child instantiation, F.7 dispatch ordering).
Everything before m7 was scalar-only fn-bodies; everything after
m7 builds on the struct ABI.

## What runs vs. what builds

| Primitive | Interpreter | Codegen |
|---|---|---|
| `fn main()` entry | ✅ | ✅ |
| Int / Float / Bool / String literals + params | ✅ | ✅ |
| `let` bindings | ✅ | ✅ |
| Arithmetic, comparisons, logical ops | ✅ | ✅ |
| `self.X` reads (in lifecycle methods) | ✅ | ✅ (runtime GEP+load) |
| Locus instantiation + `birth()` | ✅ | ✅ (ephemeral only) |
| Mixed-type println (single printf) | ✅ | ✅ |
| `let mut` + assignment (incl. compound `+=` etc.) | ✅ | ✅ |
| `if` / `else` / `else if` / `while` + `break` / `continue` | ✅ | ✅ |
| `time::sleep` on CLOCK_MONOTONIC + EINTR retry | ✅ | ✅ |
| `time::monotonic()` + Duration ± Duration / cmp | ✅ | ✅ |
| User-defined fns called from main / each other | ✅ | ✅ |
| `run()` lifecycle method | ✅ | ✅ |
| `self.X = ...` mutation in lifecycle methods | ✅ | ✅ |
| `accept()` lifecycle method (F.7 ordering) + child `g.X` reads | ✅ | ✅ |
| `drain()` / `dissolve()` lifecycle methods (F.4 cascade) | ✅ | ✅ |
| User `type` decls + struct literals + field reads | ✅ | ✅ |
| Locus `fn` members (called from bus dispatch, etc.) | ✅ | ✅ |
| Bus router (`<-` send + subscribe dispatch) | ✅ | ✅ |
| Long-lived locus deferred drain/dissolve (subscribers) | ✅ | ✅ |
| `self.method()` calls inside lifecycle / fn bodies | ✅ | ✅ |
| `Decimal` type + arithmetic + comparisons (f64 v0) | ✅ | ✅ |
| `return n;` from main → process exit code | ✅ | ✅ |
| Closures: collapse on pass, exit-non-zero on fail | ✅ | ✅ |
| Closures: parent absorb / bubble routing (F.9) | ✅ | ✅ |
| Built-in `ClosureViolation` type (locus/closure/diff fields) | ✅ | ✅ |
| Modes (`mode bulk()` etc.) + self-method dispatch | ✅ | ✅ |
| `self.children` (fixed-cap array on accept-declaring loci) | ✅ | ✅ |
| `for child in self.children { ... }` iteration | ✅ | ✅ |
| Locus literals in expression position (`let l = L { }`) | ✅ | ✅ |
| Time literals + Time as a typechecked primitive | ✅ | ✅ (string-spelling v0) |
| Composite locus param defaults | ✅ | ✅ |
| Nested field reads (self.x.y, expr-receiver-of-Field) | ✅ | ✅ |
| Heap-allocated user-type literals (escape via bus) | ✅ | ✅ |
| Contracts (typecheck only — F.8) | ✅ | ✅ (skipped at codegen) |
| `match` (Literal / Wildcard / Binding patterns) | ✅ | ✅ |
| `match` (Tuple / Constructor patterns + guards) | ✅ | — |
| generic `for` (over arrays / ranges) | ✅ | — |
| Closure runtime (collapse / absorb / bubble) | ✅ | — |
| Modes as methods | ✅ | — |
| Recovery primitives (bubble) | ✅ | — |
| Recovery primitives (restart / quarantine etc.) | parsed | — |
| Region allocator (per-projection-class arenas) | — | — |
| Cooperative scheduler | — | — |

## Locked design commitments (F.1–F.18)

Spec source: `spec/design-rationale.md`. Summary:

- **F.1** k_max = B / [(1−φ)c + φσ] is the framework equation.
- **F.2** `ProjectionClass` as built-in any-of-three constraint.
- **F.3** Per-arena defrag/free-list, no whole-program GC.
- **F.4** `drain()` always cascades depth-first.
- **F.5** Mode projections share the locus's arena.
- **F.6** Lifecycle methods are not implicit loci.
- **F.7** `accept()` runs before child birth.
- **F.8** Contract compatibility type-checked across coordinator /
  coordinatee.
- **F.9** Collapse vs. explosion + parent on_failure routing
  (absorb / bubble).
- **F.10** Mode keywords accepted post-dot as member names.
- **F.11** `self.children` typing and lifecycle.
- **F.12** Bus send is `<-`; subscribe is declarative.
- **F.13** Bus subscription handler signature.
- **F.14** Three-way interface: locus + parent + contract.
- **F.15** Predefined type names are PascalCase, not keywords.
- **F.16** `self.k_max` as built-in computed field (F.1 executable).
- **F.17** Strict field-access; method types on locus / perspective.
- **F.18** Match exhaustiveness checked at typecheck.

## Files to read for orientation

In order:

1. `README.md` — overview, status, F-table, example list, toolchain.
2. `spec/design-rationale.md` — why each construct is shaped the way
   it is. Source of truth for F.1–F.18.
3. `spec/grammar.ebnf` — formal grammar.
4. `spec/tokens.md` — lexical structure.
5. `spec/precedence.md` — operator precedence table.
6. `spec/memory.md` — memory model + the "Codegen ABI (v0)" section
   documenting the locus struct lowering, F.7 dispatch ordering,
   and ephemeral-only constraint (added in m7, extended in m8).
7. `spec/runtime.md` — runtime semantics + the "Time" section
   documenting the monotonic-only-scheduling discipline (m5, m9).
8. `examples/hello-world/main.lt` → `examples/10-stateful-locus/`
   → `examples/trellis-demo/main.lt` — the example ladder.
   06-10 are the codegen-arc demos; trellis-demo exercises the
   full interpreter pipeline.
9. `crates/lotus-syntax/src/lib.rs` — public API of the parser/AST.
10. `crates/lotus-types/src/lib.rs` — typechecker entry + unit
    tests that lock the F.x rules.
11. `crates/lotus-runtime/src/lib.rs` + `eval.rs` + `bus.rs` +
    `builtins.rs` — interpreter, dissolve cascade, bus router,
    `time::sleep` / `time::monotonic` via libc::clock_*.
12. `crates/lotus-codegen/src/codegen.rs` — current LLVM lowering.
    The biggest single file in the workspace; the locus runtime
    ABI is what makes it interesting. Worth a careful read if
    extending codegen.
13. `crates/lotus-codegen/runtime/lotus_arena.c` — the lotus
    region allocator (m19). Bundled into the compiler via
    `include_str!`, written next to each generated `.o` file
    at link time, compiled + linked into the final binary. The
    surface every `arena_alloc` call site in codegen.rs targets.
13. `crates/lotus-cli/src/main.rs` — CLI dispatch (lex / parse /
    check / run / build).
14. `~/.claude/plans/witty-foraging-lightning.md` — the original
    delivery plan to team-wide internal v1.0 (~18–30 months total).
15. `notes/open-questions.md` — tracked deferrals, including the
    spec-vs-impl gap on immutable-binding compile-time
    enforcement (§23).

For broader program context:

- `~/notes/alpha-conjecture/CLAUDE.md` — the master project guide.
  Lotus is one substrate-arm among several; paper 4 is the program's
  foundational anchor (read its memory file too).
- `~/notes/alpha-conjecture/lotus/` — the design-time meta-framework
  that lotus-the-language is the compile-time projection of.

## Strategic preferences locked in

These are user (Riley) directions saved into auto-memory at
`~/.claude/projects/-home-riley-notes-alpha-conjecture/memory/`:

- **Greenfield cleanup as we go** — pre-ship code is greenfield;
  drop "preserved old behavior" / fallback patterns; clean up
  rather than accumulate compatibility cruft. (See
  `feedback_greenfield_cleanup.md`.)
- **Stay focused on lotus** for the foreseeable session — don't
  swing back to paper-4 / theory work without explicit redirect.
- **LLVM is the codegen target** — committed; toolchain installed
  (llvm-18 + clang + lld + libpolly-18-dev). inkwell 0.5 +
  llvm-sys 180.0.0 against system LLVM.
- **Trellis informs but doesn't dictate** — production trellis-pair
  (analyst/executor as separate binaries) is the eventual real-world
  use case, but we're not building specifically toward it. It's a
  milestone we'll hit when the pieces are right; for now,
  `examples/trellis-demo/` is the single-process surrogate that
  exercises the full pipeline.

## User context (Riley)

Junior partner at small finance firm. Deep software-architecture
expertise via brain3 (production deployment at the firm,
brained.dev). The trellis trading system is the natural first
real-world use case for lotus.

## Recent commit history (last 30, newest first)

```
cdd7353 Codegen milestone 9: time::monotonic() + Duration arithmetic
73d6002 CHECKPOINT.md + README: refresh for milestone 8 (accept lifecycle)
d5afffd Codegen milestone 8: accept() lifecycle + parent-child wiring
7c93f69 CHECKPOINT.md + README: refresh for milestone 7 (locus runtime ABI)
206fbd0 Codegen milestone 7: locus runtime ABI
79ae75f CHECKPOINT.md: refresh for milestone 6
9955bea Codegen milestone 6: multi-fn programs
29c8bdf README + open-questions: sync to milestone-5 state
fd53a6d CHECKPOINT.md: refresh for milestone 5
929efa2 Codegen milestone 5: time::sleep on CLOCK_MONOTONIC
cd01f9a CHECKPOINT.md: refresh for milestone 4
cae8c9a Codegen milestone 4: if / while / break / continue
76992f1 CHECKPOINT.md: refresh for milestone 3
03c2f55 Codegen milestone 3: let mut + assignment
5224d53 Codegen milestone 2: let + Int/Float arithmetic + comparisons
5c9b6f7 Codegen milestone 1: Int / Float / Bool params + mixed-type println
77b977f Phase 3 milestone 0: lotus build → native ELF via LLVM
4b5b00c Spec sync: F.16 / F.17 / F.18 added; F.8 / F.9 / closure refined
ed81e56 Match exhaustiveness check at typecheck
34c188f F.1: self.k_max as computed field on locus values
6e630e1 Closure-cycle existence check: reject pure-literal assertions
dd325fe Strict field-access checking + locus/perspective method types
72c5036 F.8: contract compatibility checked across coordinator/coordinatee
13ba006 match expressions execute: literals, wildcards, bindings, tuples
2fe0ca9 Program-end dissolve: long-lived locus closures actually fire
c3dbe94 F.9 closes: collapse / absorb / bubble — three separate demos
22c27bf F.9 routing: parent on_failure absorbs ClosureViolation
c738e9e Closure-test runtime: F.9 collapse vs explosion fires
efe0358 trellis-demo: full pipeline runs end-to-end + Decimal arithmetic
bb1910e Bus: Transport trait + SyncDispatch + RingBuffer impls
ef752d9 v0 bus router: `<-` actually delivers; 05-bus runs end-to-end
e07b3ce Phase 2 v0: tree-walking interpreter — `lotus run` works
07c3e58 Phase 1 milestone 2: type checker (resolve + check passes)
8cc583b v0.1.8: PascalCase predefined types + bus-send `<-` operator
5a961f0 Phase 1 milestone 1: lex / parse / AST threaded through
```

43 commits ahead of origin/master at checkpoint time.

## Next steps in priority order

Conceptual locus depth (deepest = substrate-touching, shallowest =
user-facing). Each is a focused single-commit chunk unless noted.

**Codegen surface expansion (Tier 4, the LLVM path):**

1. **trellis-pair (multi-binary, cross-process bus).** The only
   remaining example. Requires process-level entry-point
   selection (one source file compiles to a per-binary entry,
   e.g. via `--bin analyst`) AND a cross-process bus transport
   (shared-memory ring buffer or NATS bridge). Both are
   infra-level pieces deferred from v0 codegen scope.
2. **Tightening Decimal/Time precision** — the `printf %g` vs
   Rust `Display` divergence is documented; trellis-grade
   fixed-point Decimal lands when the substrate cares.
3. **Region allocator + cooperative scheduler** — the deeper
   Phase-2 deep-pushes. Region allocator replaces libc malloc
   for type literals, with per-projection-class arenas (rich /
   chunked / recognition per F.3). Cooperative scheduler
   replaces the synchronous-instantiation model with a real
   BEAM-shaped multi-scheduler runtime so loci with `run()` can
   yield + resume + receive bus messages out of band.

**Smaller follow-ups available in any commit:**
- `return n;` from main → process exit code (one-line lowering
  once the special-cased main path can lift `return`)
- Default param values on user fns (already in AST; declare time
  rejects them today)
- Locus param defaults that aren't literals (current constraint:
  literal-only at declare time; lift by deferring default eval to
  the instantiation site through `lower_expr`)
- Decimal arithmetic (needed for `trellis-demo`)

**Runtime side (Tier 0/1, deferred):**

- Region allocator (per-projection-class strategies)
- Cooperative scheduler (BEAM-shaped)
- Cross-process shared-memory ring buffer (production trellis-pair)
- Recovery primitives execution (restart / quarantine — needs
  scheduler + region allocator)

**Outstanding deferrals worth tracking:**

- Generic instantiation (record args, no substitution yet)
- Module / import resolution (parsed only)
- Tree-sitter grammar derivation from EBNF
- LSP server
- Self-hosting (Phase 6, distant)

## Toolchain state

System has:

- `llvm-config` 18.1.3 at `/usr/bin/llvm-config`
- `clang` 18.1.3 at `/usr/bin/clang`
- `lld` at `/usr/bin/lld`
- `libpolly-18-dev` (required by llvm-sys for static link)
- `gcc` 13.x

Cargo workspace builds clean. `cargo test --workspace --tests` passes
all 91 tests (the locus-with-run test runs 3×500ms sleeps so the
runtime + codegen integration buckets clock ~1.5s each).

## How to verify the checkpoint

```
cd ~/code/lotus-lang
cargo test --workspace --tests           # 91 passed
cargo run --bin lotus -- run examples/trellis-demo/main.lt
cargo run --bin lotus -- build examples/hello-world/main.lt
./examples/hello-world/main              # prints "hello, world"
rm examples/hello-world/main             # clean up artifact
cargo run --bin lotus -- build examples/01-locus-with-run/main.lt
./examples/01-locus-with-run/main        # tick 0..2 over 1.5s
rm examples/01-locus-with-run/main       # clean up artifact
cargo run --bin lotus -- build examples/02-parent-child/main.lt
./examples/02-parent-child/main          # 3× "greeting from child: ..."
rm examples/02-parent-child/main         # clean up artifact
cargo run --bin lotus -- build examples/06-mutable-counter/main.lt
./examples/06-mutable-counter/main       # prints "n=2"
rm examples/06-mutable-counter/main      # clean up artifact
cargo run --bin lotus -- build examples/07-control-flow/main.lt
./examples/07-control-flow/main          # prints "sum=29 stopped at n=9"
rm examples/07-control-flow/main         # clean up artifact
cargo run --bin lotus -- build examples/08-monotonic-sleep/main.lt
./examples/08-monotonic-sleep/main       # prints tick 0..2 + done; ≥150ms
rm examples/08-monotonic-sleep/main      # clean up artifact
cargo run --bin lotus -- build examples/09-functions/main.lt
./examples/09-functions/main             # prints square(7)=49 / fib(12)=144 / ...
rm examples/09-functions/main            # clean up artifact
cargo run --bin lotus -- build examples/10-stateful-locus/main.lt
./examples/10-stateful-locus/main        # prints total=160 / step=30
rm examples/10-stateful-locus/main       # clean up artifact
cargo run --bin lotus -- build examples/11-drain-dissolve/main.lt
./examples/11-drain-dissolve/main        # parent: birth, child-a/b drain+dissolve, parent: drain+dissolve
rm examples/11-drain-dissolve/main       # clean up artifact
cargo run --bin lotus -- build examples/12-user-types/main.lt
./examples/12-user-types/main            # p.x=3 p.y=4, q.x=13 q.y=8, alice says hello (priority 7)
rm examples/12-user-types/main           # clean up artifact
cargo run --bin lotus -- build examples/05-bus/main.lt
./examples/05-bus/main                   # got: hello from sender-1, ack: hello
rm examples/05-bus/main                  # clean up artifact
cargo run --bin lotus -- build examples/13-decimal-and-exit/main.lt
./examples/13-decimal-and-exit/main      # bid/ask/spread/mid/fee printed
rm examples/13-decimal-and-exit/main     # clean up artifact
cargo run --bin lotus -- build examples/03-closure-test/main.lt
./examples/03-closure-test/main          # collapsed cleanly.
rm examples/03-closure-test/main         # clean up artifact
cargo run --bin lotus -- build examples/trellis-demo/main.lt
./examples/trellis-demo/main             # 5x intent + 3x kernel hot-load
rm examples/trellis-demo/main            # clean up artifact
cargo run --bin lotus -- build examples/03b-closure-absorbed/main.lt
./examples/03b-closure-absorbed/main     # AuditL absorbs the violation, exits 0
rm examples/03b-closure-absorbed/main
cargo run --bin lotus -- build examples/03c-closure-bubbled/main.lt
./examples/03c-closure-bubbled/main      # bubble → exits non-zero
rm examples/03c-closure-bubbled/main
cargo run --bin lotus -- build examples/04-modes/main.lt
./examples/04-modes/main                 # bulk=60, harmonic=3, resolution=30
rm examples/04-modes/main
```

If all twenty work, the checkpoint is intact.
