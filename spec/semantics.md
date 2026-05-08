# Operational semantics

This document specifies what lotus programs *do* when they
run. Where `grammar.ebnf` says what's syntactically valid and
`types.md` says what's meaningfully valid, this document says
what happens at runtime.

The semantics are described informally (engineering-grade prose),
not as formal small-step rules. Formal operational semantics are
deferred to v1+; the framework's substrate-invariance claim is
not blocked on full formalization.

## Program startup

1. The runtime initializes:
   - Region allocator, schedulers (one per CPU core; cooperative
     within each), bus router, lifecycle dispatcher.
   - Reads deployment config (e.g., `deployment.yaml`) for
     transport bindings.
   - Establishes the **runtime root locus** as the implicit
     parent of `main`'s implicit locus.
2. Bus channels declared in the program are wired to transport
   adapters per deployment config.
3. `fn main()` is invoked.

## Function call

A `fn name(args) -> ret { body }` invocation:

1. Caller passes args by value.
2. Implicit locus is allocated for the function's scope.
3. Body executes statement-by-statement.
4. Locally bound handles are bound to local names.
5. Anonymous-child loci with ongoing-work surface attach to
   the implicit locus.
6. On `return value;`, control flows back to caller. Before
   return, the implicit locus drains and dissolves (waits for
   all children to finish; depth-first cascade per F.4).

## Locus instantiation

`LocusName { params }`:

1. Compute params (overrides applied to declared defaults).
2. Locus type's `accept(c)` (if instantiated inside a parent's
   lifecycle method) runs first; if it rejects, instantiation
   fails (no region allocated).
3. Region allocated as sub-region of enclosing locus's region;
   size determined by projection class.
4. `birth(args)` runs synchronously.
5. Bus subscriptions wire up.
6. Modes are reachable for invocation.
7. If `run` declared, scheduled to run on the locus's
   scheduler.
8. Expression returns the locus handle.

If unbound and ephemeral (only birth + params), the handle is
discarded; locus dissolves at enclosing statement boundary.

If unbound and long-lived (run / bus / modes), locus becomes
anonymous child of enclosing scope.

## Lifecycle method invocation

### `birth()`

Runs once, synchronously, after region allocation and before
the locus is "live" for any other purpose. Failure during
birth: region freed, parent's `on_failure(self,
StructuralFailure { ... })` invoked.

### `accept(c)`

Runs **before** child c's region is allocated (per F.7).
Receives c's declared params (not its running state). Can:

- Return normally (accept) — child proceeds to allocation +
  birth.
- Panic / return error (reject) — child instantiation fails.

After accept returns normally, child registers in
`self.children` (per F.11).

### `run()`

Runs continuously until drain is requested or run returns
naturally. Cooperative — yields at every bus dispatch, every
`time::sleep`, every explicit yield point. The scheduler may
run other loci while this run is yielded.

If run() returns naturally, the locus exits run-state and
proceeds to drain.

If run() panics, parent's `on_failure(self, StructuralFailure
{ ... })` invoked.

### `drain()`

Runs once, when the locus is asked to drain. Drain *cascades
depth-first* (per F.4): drain runs on all children first,
synchronously; then runs on self.

During drain:
- New child accepts are refused.
- In-flight handler invocations complete.
- New bus messages are not accepted; in-flight messages on
  bus subscriptions are delivered.
- Closure tests at `tick` epoch may fire (if not already
  fired).

Default drain: no-op (just transitions state from running to
drained).

### `dissolve()`

Runs once, after drain completes. Executes user-supplied
cleanup code if any. Then:

- Closure tests at `dissolve` epoch fire (per F.9). Failure
  records explosion flag.
- Region freed wholesale.
- If exploded, parent's `on_failure(self, ClosureViolation
  { ... })` invoked alongside region release.
- Otherwise, parent sees normal child-dissolution.

Default dissolve: free region.

### `on_failure(c, err)`

Runs when a child of self fails (any failure type:
StructuralFailure, ClosureViolation, etc.). Receives the
child handle and the typed error.

The handler may:
- Return normally (absorb): treat as collapsed; parent
  forgets about the child.
- Call `restart(c)`: re-instantiate the child with the same
  params.
- Call `restart_in_place(c)`: re-init in place (preserve
  arena).
- Call `quarantine(c)`: keep child in a halted state with
  arena preserved; future inspection possible.
- Call `bubble(err)`: pass the failure to self's parent.
- Call `dissolve(c)` explicitly: free child's region.

Default on_failure: `bubble(err)`. The runtime root's default
is process exit with stack trace.

## Mode invocation

`self.bulk()` / `self.harmonic()` / `self.resolution()` invoke
mode declarations. Modes are:

- Synchronous functions taking the receiver as implicit
  argument.
- Read/write the locus's arena directly (no copies).
- Compiled to per-projection-class implementations.

Mode invocation from outside the locus (e.g., `child.bulk()`
from a parent) is permitted iff `bulk` is contract-exposed
on the child; goes through the contract's typed surface (per
F.14).

## Bus subscription dispatch

A `bus { subscribe SUBJECT as HANDLER of type T; }` declaration
wires:

1. The runtime registers HANDLER as the receiver for SUBJECT
   on the bound transport.
2. Inbound messages on SUBJECT are decoded as `T`, then
   HANDLER(payload) is invoked.
3. HANDLER runs in the locus's scheduler context. It may
   call `publish(SUBJECT, msg)` to emit responses (subject
   to `bus { publish ... ; }` declarations).
4. HANDLER yields naturally on completion; scheduler returns
   to other loci.

If HANDLER panics:
- The current message is dropped.
- `on_failure(self, BusHandlerFailure { subject, payload, err })`
  invoked on the parent if any.
- The subscription itself is *not* removed; future messages
  continue to dispatch.

## Closure-test evaluation

For each `closure NAME { LEFT ~~ RIGHT within TOL; epoch
... }`:

1. At each declared epoch boundary, runtime evaluates LEFT
   and RIGHT in the locus's scope.
2. Computes `|LEFT - RIGHT|`.
3. If `<= TOL`: closure passes silently.
4. If `> TOL`: flips the locus's "exploded" flag; emits a
   typed `ClosureViolation` event.

Epoch boundaries:

- `epoch dissolve` (default): fires once, as part of dissolve
  sequence.
- `epoch tick`: fires on each runtime tick (configurable
  cadence).
- `epoch duration(d)`: fires every `d` of monotonic time.
- `epoch birth`: fires once, after birth completes.
- `epoch explicit`: fires only when user code calls
  `epoch_advance(NAME)`.

## Perspective hot-load

For each `perspective P { ... }` instance currently active:

1. New perspective arrives via bus (or explicit
   `load_perspective(P, bytes)` call).
2. Runtime decodes against P's compiled-in schema. Type-
   mismatch → reject; emit `PerspectiveDecodeError`.
3. Validates `stable_when` predicate. If false → reject;
   emit `PerspectiveNotStable`.
4. Atomically swaps the active perspective:
   - Pause all readers (readers within the locus see the
     pre-swap perspective).
   - Replace.
   - Resume.
5. Emit `PerspectiveLoaded` event.

Old perspective is freed only after the swap completes; no
torn read possible.

## Recovery primitives

### `restart(child)`

1. Schedule child for dissolution.
2. Once dissolved, instantiate a new child with the same
   declared params.
3. New child's birth runs; old child's state is gone.

### `restart_in_place(child)`

1. Set child's "restarting" flag.
2. Wait for current handler / mode invocation to complete
   (cooperative yield point).
3. Reset locus to post-birth state, preserving the arena.
4. Re-run birth().
5. Mark restart complete.

Useful for transient failures that don't invalidate the
locus's structural commitments (e.g., the locus's k_max is
fine; just had a bad message).

### `quarantine(child) [for d]`

1. Pause child (no new messages dispatched, no new accepts
   permitted, no run scheduling).
2. Preserve arena and state.
3. If `for d` clause given, automatically restart after `d`.
4. Otherwise wait until parent explicitly resolves.

### `reorganize(child, ...)`

Reserved syntax. Semantics TBD; expected: relocate child's
sub-children to a sibling. Not in v0.

### `bubble(err)`

Re-raise the error to self's parent. Equivalent to:

```
on_failure(c, err) { bubble(err); }  // for self's failures
```

Fully traverses the lotus tower upward until a handler
absorbs.

### `dissolve(child)`

Force-dissolve child immediately. Skips drain; closure tests
at non-dissolve epochs do not fire on this path. Used for
forced-shutdown scenarios.

## Drain cascade (whole-process)

SIGINT or SIGTERM:

1. Signal handler in the runtime root locus calls
   `drain(self)` on itself.
2. Drain cascades depth-first to all children of root.
3. Each child cascades to its children, etc.
4. Leaves drain first; in-flight messages complete; bus
   subscriptions stop accepting.
5. Each parent waits for all its children to drain before
   draining itself.
6. Root drains last.
7. Runtime tears down schedulers, bus router, allocator.
8. Process exits 0.

## Closure-failure cascade

A closure violation at any epoch:

1. Runtime emits `ClosureViolation` event.
2. Locus's exploded flag is set.
3. Subsequent epochs may also fail; flag persists.
4. At dissolve, parent's `on_failure(self, ClosureViolation
   { ... })` invoked.
5. Parent's policy decides: absorb, recover, bubble.
6. If bubbled, propagates to grandparent; recursively until
   absorbed or reaching root (process exit).

## Scheduler dispatch

Per `runtime.md`: multi-scheduler cooperative.

1. N schedulers, one per CPU core (configurable).
2. Each scheduler holds a queue of runnable loci.
3. Scheduler picks a locus, runs it until cooperative yield.
4. At yield, scheduler picks next.
5. Cross-scheduler communication via bus (typed messages).
6. Loci may be migrated between schedulers transparently for
   load balancing.

## Failure-traversal flow

Failures flow upward:

1. Child failure → child's parent's `on_failure`.
2. Parent's handler decides absorb / restart / bubble /
   quarantine / dissolve.
3. If bubble, → grandparent's `on_failure`.
4. If reaches runtime root, process exits with structured
   error report.

Failures never flow laterally (sibling-to-sibling) — the
framework's vertical-only-flow expressed at the runtime layer.

## Region lifetime guarantees

Per `memory.md`:

- A locus's region is freed atomically on dissolve.
- Sub-regions are freed before the parent's region (drain
  cascade ensures this).
- No pointer-into-a-freed-region is reachable after region
  release (compile-time-checked + region-lifetime-checked).

## What's deferred

- **Formal small-step semantics.** Engineering-grade prose for
  v0; formal operational rules in v1+ if needed for compiler
  correctness proofs.
- **Concurrency-correctness proofs.** Cooperative scheduler
  + per-locus arena makes most concurrency questions trivial,
  but full formal modeling deferred.
- **Memory-model formalization** as a happens-before relation.
  Currently informal; formal in v1+.
- **Async / await semantics.** Reserved keywords; no operational
  semantics in v0.
