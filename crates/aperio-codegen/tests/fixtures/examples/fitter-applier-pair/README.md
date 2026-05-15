# fitter-applier-pair

The substrate-anchor program. Two binaries — a fitter and an
applier — communicating via a shared compiled-in schema and a
typed bus. The capstone example for the Aperio language.

## Files

- `shared.ap` — types compiled into both binaries: `Observation`,
  `Kernel`, `Action`, `Receipt`, plus the `KernelPerspective`
  declaration with `stable_when`.
- `fitter.ap` — `FitterL`. Consumes `Observation`; computes a
  fitted `Kernel`; publishes when stable. Resolution-mode regime.
- `applier.ap` — `ApplierL`. Consumes `Observation` + `Kernel` +
  `Receipt`; publishes `Action`. Bulk-mode regime.
- `deployment.yaml` — transport bindings (UDP multicast for the
  observation stream, NATS for control-plane).

## What this exercises

Every primitive the spec defines:

- **Locus declarations** with full param surface + lifecycle
  + bus interface.
- **Locus state via params** (running totals, current kernel,
  counts; mutable through `self.x = ...`; per F.3).
- **Type declarations** for typed messages (`type Observation`,
  `type Kernel`, `type Action`, `type Receipt`).
- **Perspective declaration** with `stable_when` predicate
  and `serialize_as` annotation. Per F.10.
- **Bus interface** with subscribe + publish across multiple
  channels; transport-agnostic source paired with deployment-
  config transport binding.
- **Closure tests** at both loci with different epochs:
  - Fitter: `epoch tick` — published kernels track validation
    rate continuously.
  - Applier: `epoch dissolve` — emitted actions balance
    against received receipts at shutdown (the canonical
    audit cycle).
- **Decimal arithmetic** (`obs.value_low *
  self.current_kernel.scale`) producing typed `Decimal`
  values per F.4.
- **Time literals** (`` `2026-01-01T00:00:00Z` ``) for the
  initial kernel's `valid_after`.
- **Struct literal expressions** for typed message
  construction (`Action { kind: ..., magnitude: ..., ... }`).
- **Mutable bindings** in handler bodies (`let a = ...`).

## What it tests at the substrate level

- **Substrate-derivation discipline by architecture.** Fitter
  and applier both consume `fitter/applier.observation` from the same
  UDP multicast group. The applier cannot fit on the data
  it's acting on because it has no fitting machinery and no
  path back to the fitter's state. Anchor isolation by
  deployment shape.
- **Multi-perspective stability operationalized.** Fitter
  holds many candidate kernels; only ones whose
  `validation_count >= 3` ship as perspectives. The ancient
  texts' N≥3 commit-rule is now a runtime gate.
- **Cyclic-closure at the applier↔fitter boundary.** The
  applier's `action_receipt_balance` closure asserts every
  emitted action eventually becomes a receipt. If divergence
  exceeds tolerance: explosion at dissolve, parent receives
  `ClosureViolation`, the discrepancy is a typed event with
  diff, closure name, and locus identity (per F.9).
- **Transport-as-mode-projection.** Same source code uses UDP
  multicast for the high-frequency observation stream and
  NATS for slow-cadence reliable kernel updates. The bus
  kernel is one primitive; transports are mode-projections.
- **Form-completeness vs. content-perspective-locality.** Both
  binaries compile from the same `shared.ap`; the form is
  shared, the content (parameter values flowing on the bus)
  is perspective-local. Schema agreement at the type level;
  no handshake protocol needed.

## What this does not exercise

Acknowledged gaps that are real but out of scope:

- `self.children` iteration. fitter/applier-pair's coordination is
  bus-mediated, not parent-child.
- Mode declarations (`bulk` / `harmonic` / `resolution`).
  Fitter and applier are single-mode loci; modes serve
  internal kernel projection.
- `accept` lifecycle. No parent-child structure here.
- Recovery primitives (`on_failure` handlers). Failures
  bubble to runtime root.
- Multi-implementation per contract field (deferred until
  forced; per F.14 follow-on).

These are intentional simplifications. The fitter/applier pair
exercises the bus + perspective + closure surface; richer
primitives belong to other examples.

## What the runtime does

### Boot

1. Operator starts both binaries (`./fitter` and `./applier`).
2. Each binary's runtime reads `LOTUS_BUS_CONFIG`. UDP
   multicast group joined for `fitter/applier.observation`; NATS
   connections opened for the rest.
3. `FitterL` and `ApplierL` each instantiate as anonymous
   long-lived children of their respective `main`'s implicit
   locus.

### Steady state

- Observations flow in. Both binaries see the same
  `Observation` values (UDP multicast).
- Fitter processes each observation; updates `latest_kernel`;
  increments `validation_count`. After `validation_count >= 3`,
  publishes a kernel on `fitter/applier.kernel`.
- Applier receives each kernel; hot-swaps `current_kernel`.
  Receives each observation; computes `Action` using current
  kernel; publishes `Action` on `fitter/applier.action`.
- Downstream sink (separate, not in this example) processes
  actions, produces receipts.
- Applier receives receipts; increments `receipts_received`.
- Fitter's closure (`epoch tick`) fires periodically; if the
  publication lags validation by more than ±1, an explosion
  flag is set.

### Shutdown

1. SIGINT triggers drain on each binary's runtime root.
2. Cascade reaches each `main`'s implicit locus → reaches
   `FitterL` / `ApplierL`.
3. Each locus drains: stops accepting new bus messages;
   in-flight handlers complete.
4. Closure tests fire one final time at the dissolve epoch.
   Applier's `action_receipt_balance` checks
   `actions_emitted ~~ receipts_received within 5`. Pass =
   clean collapse. Fail = explosion; `main`'s implicit locus
   has no `on_failure` handler, so failure bubbles to runtime
   root, process exits non-zero with a structured
   `ClosureViolation` report identifying the applier and the
   diff.

## Why this is the capstone

The fitter-applier-pair pulls on every substrate primitive at once:

- Two processes, each opening its own lotus.
- A schema shared by compilation, not by handshake.
- A perspective with a commit predicate (`stable_when`).
- A bus carrying typed payloads across processes via the
  m70 wire format.
- Closure audits with both `tick` and `dissolve` epochs.
- The full F.4 / F.7 / F.8 / F.9 / F.14 framings, all live.

If the spec hangs together, this program reads cleanly and
runs without surprise. If something is wrong, the fitter-applier-pair
is where it surfaces first.
