# trellis-pair

The spec-anchor program. Two binaries — an analyst and an
executor — communicating via a shared compiled-in schema and a
typed bus. Phase 0's exit gate per the delivery plan.

## Files

- `shared.lt` — types compiled into both binaries: `Book`,
  `TradeKernel`, `Intent`, `Fill`, plus the
  `TradeKernelPerspective` declaration with stable_when.
- `analyst.lt` — `AnalystL`. Consumes Book; computes a fitted
  TradeKernel; publishes when stable. Resolution-mode regime.
- `executor.lt` — `ExecutorL`. Consumes Book + TradeKernel +
  Fill; publishes Intent. Bulk-mode regime.
- `deployment.yaml` — transport bindings (UDP multicast for
  market data, NATS for control-plane).

## What this exercises

Every primitive the spec defines:

- **Locus declarations** with full param surface + lifecycle
  + bus interface.
- **Locus state via params** (running totals, current kernel,
  counts; mutable through `self.x = ...`; per F.3).
- **Type declarations** for typed messages (`type Book`,
  `type TradeKernel`, `type Intent`, `type Fill`).
- **Perspective declaration** with `stable_when` predicate
  and `serialize_as` annotation. Per F.10.
- **Bus interface** with subscribe + publish across multiple
  channels; transport-agnostic source paired with deployment-
  config transport binding.
- **`publish(...)` builtin** for explicit outbound messaging
  (per F.12).
- **Closure tests** at both loci with different epochs:
  - Analyst: `epoch tick` — published kernels track validation
    rate continuously.
  - Executor: `epoch dissolve` — emitted intents balance
    against received fills at shutdown (the canonical audit
    cycle).
- **Decimal arithmetic** (`book.bid_price *
  self.current_kernel.multiplier`) producing typed `decimal`
  values per F.4.
- **Time literals** (`` `2026-01-01T00:00:00Z` ``) for the
  initial kernel's `valid_after`.
- **Struct literal expressions** for typed message
  construction (`Intent { side: ..., price: ..., ... }`).
- **Mutable bindings** in handler bodies (`let i = ...`).

## What it tests at the framework level

- **Substrate-derivation discipline by architecture.** Analyst
  and executor both consume `trellis.book` from the same UDP
  multicast group. The executor cannot fit on the data it's
  trading because it has no fitting machinery and no path back
  to the analyst's state. Anchor isolation by deployment shape.
- **Multi-perspective stability operationalized.** Analyst
  holds many candidate kernels; only ones whose
  `validation_count >= 3` ship as perspectives. The framework's
  N≥3 commit-rule is now a runtime gate.
- **Cyclic-closure at the analyst↔executor boundary.** The
  executor's `intent_fill_balance` closure asserts every
  emitted intent eventually becomes a fill. If divergence
  exceeds tolerance: explosion at dissolve, parent receives
  ClosureViolation, the discrepancy is a typed event with diff,
  closure name, and locus identity (per F.9).
- **Transport-as-mode-projection.** Same source code uses UDP
  multicast for line-rate market data and NATS for slow-cadence
  reliable kernel updates. The bus kernel is one primitive;
  transports are mode-projections (per the bus refactor in
  commit 5db0864).
- **Form-completeness vs. content-perspective-locality.** Both
  binaries compile from the same shared.lt; the form is shared,
  the content (parameter values flowing on the bus) is
  perspective-local. Schema agreement at the type level; no
  handshake protocol needed.

## What this does not exercise

Acknowledged gaps that are real but out of scope for v0:

- `self.children` iteration (only AggregatorL in 04 used it;
  trellis pair doesn't need it because its coordination is
  bus-mediated, not parent-child).
- mode declarations (analyst and executor are single-mode loci;
  modes serve internal kernel projection, which trellis pair
  doesn't need at this granularity).
- `accept` lifecycle (no parent-child structure here).
- Recovery primitives (no `on_failure` handlers; failures
  bubble to runtime root).
- Multi-implementation per contract field (deferred until
  forced; per F.14 follow-on).

These are intentional simplifications. The trellis pair
exercises the bus + perspective + closure surface; richer
primitives belong to other examples.

## What the runtime would do

(Conceptual; no compiler exists yet.)

### Boot

1. Operator starts both binaries via systemd / k8s / `bin/run`.
2. Each binary's runtime reads `deployment.yaml`. UDP multicast
   group joined for `trellis.book`; NATS connections opened for
   the rest.
3. AnalystL and ExecutorL each instantiate as anonymous
   long-lived children of their respective `main`'s implicit
   locus (per the updated §A rule for unbound + bus
   subscriptions).

### Steady state

- Market data flows in. Both binaries see the same Books
  (UDP multicast).
- Analyst processes each book; updates `latest_kernel`;
  increments `validation_count`. After `validation_count >= 3`,
  publishes a kernel on `trellis.kernel`.
- Executor receives each kernel; hot-swaps `current_kernel`.
  Receives each book; computes Intent using current kernel;
  publishes Intent on `trellis.intent`.
- Gateway (separate, not in this example) processes intents,
  produces fills.
- Executor receives fills; increments `fills_received`.
- Analyst's closure (`epoch tick`) fires periodically; if the
  publication lags validation by more than ±1, an explosion
  flag is set.

### Shutdown

1. SIGINT triggers drain on each binary's runtime root.
2. Cascade reaches each `main`'s implicit locus → reaches
   AnalystL / ExecutorL.
3. Each locus drains: stops accepting new bus messages;
   in-flight handlers complete.
4. Closure tests fire one final time at the dissolve epoch.
   Executor's `intent_fill_balance` checks
   `intents_emitted ~~ fills_received within 5`. Pass = clean
   collapse. Fail = explosion; main's implicit locus has no
   on_failure handler, so failure bubbles to runtime root,
   process exits non-zero with a structured ClosureViolation
   report identifying the executor and the diff.

## Phase 0 exit gate

The delivery plan's Phase 0 exit gate is:

> trellis trader/analyst pair compiles cleanly (against a
> typechecker, before any codegen). Spec is empirically
> anchored.

Currently no typechecker exists, so this README-level review is
the proxy. The program is consistent with the v0 spec across
all 10 spec documents (grammar, tokens, precedence,
design-rationale, runtime, stdlib, testing, memory, types,
semantics).

When the Phase 1 typechecker exists, this program will be the
first test of the spec's empirical adequacy. Any incompatibility
will trigger spec revision.

## Surfaced for spec revision

Three things this example pulled on that could use formalization
in subsequent passes:

1. **Perspective `is_stable()` invocation syntax.** The example
   uses `p.is_stable()` to check the `stable_when` predicate.
   The grammar / semantics docs should explicitly note that
   `is_stable()` is a built-in method on perspective values
   that runs the stable_when block. Documented as a
   follow-up.

2. **`import "trellis-pair/shared"` resolution.** The example
   uses Go-style filesystem-relative imports. Module
   resolution rules need a single source of truth (probably
   stdlib.md or a new `spec/modules.md`). For v0,
   imports are filesystem-relative paths; package registry
   deferred per delivery plan.

3. **Compile-time-known initial values for non-primitive
   params.** The example declares
   `latest_kernel: TradeKernel = TradeKernel { ... }` —
   a struct-literal default. The compiler needs to verify
   such defaults are compile-time-evaluable.
   `types.md` should document this rule explicitly.

These are not blocking; they're refinements to lock in the
next pass.
