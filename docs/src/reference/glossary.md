# Glossary

Terms specific to Aperio and the lotus substrate. First use of any term in
the [Book](../book/introduction.md) or this Reference is rendered as a
link to its entry here; subsequent uses on the same page are bare. This is
a writing convention, not a tooling enforcement.

Entries are sorted by topic-cluster (language identity → substrate → memory
→ runtime → composition), not alphabetically. For alphabetical jump-to,
see the [Index](#index) at the bottom.

## Language identity

### Aperio {#aperio}

The language. Pronunciation: ah-PEH-ree-oh. Latin: *I open / I reveal.*
A spell cast at compile time; running it opens a [lotus](#lotus).

The toolchain is `aperio` (`aperio build`, `aperio run`, `aperio check`).
Source files use the `.ap` extension.

### lotus {#lotus}

The runtime data structure an Aperio program instantiates. A tree of
[loci](#locus) communicating via [vertical-only-flow](#vertical-only-flow)
over a shared [bus](#bus), with [per-region arenas](#per-region-memory)
and [closure-asserted](#closure) invariants.

Lowercase. Indefinite ("a lotus") refers to a specific running instance;
definite uppercase ("the Lotus") to the universal pattern.

The C runtime symbols (`lotus_arena_*`, `lotus_bus_*`, `lotus_transport_*`,
`lotus_str_*`) carry the *lotus* name on purpose — they implement the
substrate's mechanics.

## Substrate

### locus {#locus}

The unit of structure inside a [lotus](#lotus). A locus has:

- a [lifecycle](#lifecycle): birth → run → drain → dissolve.
- an [arena](#arena) (per-region memory).
- optional [params](#params), [bus subscriptions / publications](#bus),
  [closures](#closure), and [fn members](#fn-member).
- a [contract](#contract) surface (`accept`, `expose`, `consume`).

Plural: **loci**. Latin keeps Latin.

### lifecycle {#lifecycle}

The fixed sequence of methods every [locus](#locus) runs:
**birth** (initialization) → **run** (active phase) → **drain** (clean-up
of bus subscriptions) → **dissolve** (final teardown). [F.4 cascade](#f4)
guarantees depth-first order across child loci.

### birth {#birth}

A locus's first lifecycle method. Runs synchronously when the locus is
instantiated. Often used to set up children, declare bus publishes,
register subscriptions.

### run {#run}

A locus's main active method. Optional. Runs after birth completes.

### drain {#drain}

A locus's pre-dissolve method. Optional. Bus dispatch is paused for the
locus before drain runs; closures with `epoch dissolve` fire after drain
returns.

### dissolve {#dissolve}

A locus's final teardown method. Per-region arena destroyed afterwards.

### accept {#accept}

The contract method by which a parent locus instantiates a child. The
parent's `accept(c: ChildLocus)` runs in the parent's frame and is the
only way a child enters the lotus tree.

### contract {#contract}

The typed interface between parent and child loci. Three kinds:
**accept** (instantiation), **expose** (parent reads child state),
**consume** (parent disposes / repurposes child).
[F.14 three-way interface.](#f14)

### bus {#bus}

The substrate's typed pub-sub system. Loci declare `subscribe "subj" as
handler of type T` and `publish "subj" of type T`; messages cross via the
operator `<-`. Cooperative subscribers run on a process-wide cell queue;
[pinned](#pinned-scheduler) subscribers have per-locus mailboxes.

### subject {#subject}

A string identifier for a [bus](#bus) channel. Source-level transport-
agnostic; the [deployment config](#deployment-yaml) maps subjects to
transports (in-process / AF_UNIX / TCP / NATS / UDP multicast).

### dispatch {#dispatch}

The act of delivering a published payload to subscribed handlers. Local
dispatch enqueues struct bytes on the cooperative queue; remote dispatch
serializes through the per-payload `__serialize_T` and fans out to
CONNECT-role transports.

### mailbox {#mailbox}

A per-pinned-locus cell queue. Cross-thread publishers post cells via
mutex+condvar; the pinned thread's main loop drains one cell at a time,
copying its inline payload into the locus's arena before invoking the
handler.

### closure {#closure}

An auditable assertion declared on a locus. Form: `closure name {
left ~~ right within tolerance; epoch tick | dissolve; }`. Fires at the
declared epoch; on violation, fails with a typed
[ClosureViolation](#closureviolation) record routed to the parent's
[on_failure](#on-failure) handler.

Not the same thing as a JavaScript / Rust *closure* (anonymous function
capturing its environment). Aperio's closures are syntactic audit
assertions, not function values.

### ClosureViolation {#closureviolation}

The built-in record type carrying a closure-failure event: locus name,
closure name, diff. Receivable by an `on_failure(c: ChildL, err:
ClosureViolation)` handler on the parent.

## Failure handling

### on_failure {#on-failure}

A locus method that receives a child's [ClosureViolation](#closureviolation)
and chooses among recovery primitives:

- [`restart_in_place(c)`](#restart-in-place) — reset child state, keep alive.
- [`quarantine(c) for 30s`](#quarantine) — pause the child.
- [`bubble(err)`](#bubble) — propagate upward.
- [`dissolve(c)`](#dissolve) — clean shutdown (default if no handler).

### restart_in_place {#restart-in-place}

A recovery op that resets a child locus's params and re-runs its birth
without lateral migration. Children re-attach.

### quarantine {#quarantine}

A recovery op that pauses a child for a duration; the child receives no
bus messages during quarantine and resumes after.

### bubble {#bubble}

A recovery op that propagates the failure upward to the next ancestor's
on_failure handler. If no ancestor handles, the process exits non-zero
with the structured ClosureViolation report.

### vertical-only-flow {#vertical-only-flow}

The framework's failure-traversal commitment: failures flow up the locus
tree (child → parent → root); never sideways (child → sibling) and never
across processes. Recovery is always a local parent decision.
[F.8](#f8).

## Memory

### arena {#arena}

A per-locus memory region. All allocations within the locus (struct
literals, closure captures, bus payload copies) come from this arena.
Destroyed wholesale at [dissolve](#dissolve). No GC.

### per-region memory {#per-region-memory}

The framework's memory commitment: each locus owns an arena, allocations
are scoped to the arena's lifetime, cross-locus references go through
copies (bus dispatch, accept). [F.3](#f3).

### substrate {#substrate}

The set of guarantees the lotus runtime provides every Aperio program: per-
region memory, vertical-only-flow, closure auditing, deterministic
dispatch order. Often used as "substrate-up" — building higher-level
behavior from these primitives.

### region {#region}

Synonym for [arena](#arena) in some contexts. The Aperio reference favors
*arena* for the runtime data structure and *region* for the conceptual
ownership boundary.

## Composition

### perspective {#perspective}

A locus's parameter bundle, declared in the `params { ... }` block.
A perspective's stability rules (e.g., `stable_when validation_count >=
3`) gate when the locus's outputs are committed.

### projection class {#projection-class}

A type-system tag on a perspective declaring how a locus reads its inputs:

- **Rich** — full per-event detail.
- **Chunked** — bounded-window summary.
- **Recognition** — bitmap-pool exists/has-occurred.

Used as a generic bound (`fn f<T: ProjectionClass>(...)`) to constrain
which loci a fn applies to.

### Numeric (bound) {#numeric-bound}

A generic bound permitting arithmetic operations on the parameter:
`+ - * /` and ordering comparisons. Implemented on Int, Float, Decimal,
Duration as of m64.

### monomorphization {#monomorphization}

The codegen pass that converts generic templates (`type Box<T>`,
`fn first<T>(...)`, `locus Cache<K, V>`) into concrete instantiations
named via the [mangled-name](#mangled-name) convention. Aperio
monomorphizes at compile time; no runtime generic dispatch.

### mangled name {#mangled-name}

The name codegen synthesizes for a generic instantiation, formed by
joining the template name and arg names with underscores: `Box<Int>` →
`Box_Int`, `Result<Int, String>` → `Result_Int_String`.

### perspective versioning {#perspective-versioning}

Future feature for cross-binary schema evolution: `serialize_as TypeV1`
annotation on a perspective declares its wire schema. Open-question #13;
not in v1.

## Scheduling

### cooperative scheduler {#cooperative-scheduler}

The default scheduler for loci without a `: schedule pinned` annotation.
Bus dispatch is deferred to a process-wide FIFO queue; cells run between
substrate yield points (between handler completions, at explicit
`yield;` statements).

### pinned scheduler {#pinned-scheduler}

The scheduler for loci annotated `: schedule pinned`. Each pinned locus
spawns a real pthread at instantiation; the locus's full lifecycle runs
on that thread. Cross-thread bus messages route via per-locus
[mailboxes](#mailbox).

## Toolchain artifacts

### deployment.yaml {#deployment-yaml}

The deployment-time config file mapping bus subjects to transports. Read
at boot via `LOTUS_BUS_CONFIG=<path>`. Source-level Aperio is transport-
agnostic; binding lives in the config.

### params {#params}

The `params { ... }` block on a locus declaration. Declares the locus's
typed parameters with optional defaults. Forms part of the
[perspective](#perspective).

### fn member {#fn-member}

A fn declared inside a `locus { ... }` block. Has implicit `self: LocusType`
in scope. Distinct from free fns (top-level fn declarations).

## Design framings

### F.4 {#f4}

The locus-tree depth-first cascade commitment. Lifecycle methods run in
DFS order across children: a parent's drain doesn't begin until all
children have drained.

### F.8 {#f8}

The vertical-only-flow commitment. See [vertical-only-flow](#vertical-only-flow).

### F.9 {#f9}

The closure-test runtime commitment: collapse (clean dissolution on no
violation), absorb (parent handles ClosureViolation in on_failure),
bubble (no handler → exits the process via the runtime root).

### F.14 {#f14}

The three-way interface commitment: parent ↔ locus ↔ contract are three
distinct surfaces, not a single OOP-object.

## Index

(Alphabetical jump-to, populated as entries grow.)

[accept](#accept) ·
[Aperio](#aperio) ·
[arena](#arena) ·
[birth](#birth) ·
[bubble](#bubble) ·
[bus](#bus) ·
[closure](#closure) ·
[ClosureViolation](#closureviolation) ·
[contract](#contract) ·
[cooperative scheduler](#cooperative-scheduler) ·
[deployment.yaml](#deployment-yaml) ·
[dispatch](#dispatch) ·
[dissolve](#dissolve) ·
[drain](#drain) ·
[F.4](#f4) ·
[F.8](#f8) ·
[F.9](#f9) ·
[F.14](#f14) ·
[fn member](#fn-member) ·
[lifecycle](#lifecycle) ·
[locus](#locus) ·
[lotus](#lotus) ·
[mailbox](#mailbox) ·
[mangled name](#mangled-name) ·
[monomorphization](#monomorphization) ·
[Numeric (bound)](#numeric-bound) ·
[on_failure](#on-failure) ·
[params](#params) ·
[per-region memory](#per-region-memory) ·
[perspective](#perspective) ·
[perspective versioning](#perspective-versioning) ·
[pinned scheduler](#pinned-scheduler) ·
[projection class](#projection-class) ·
[quarantine](#quarantine) ·
[region](#region) ·
[restart_in_place](#restart-in-place) ·
[run](#run) ·
[subject](#subject) ·
[substrate](#substrate) ·
[vertical-only-flow](#vertical-only-flow)
