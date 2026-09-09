# iris — the inspector (ride-along, draft)

A design sketch for the editor-side surface: iris attached to a
running system *and* to the source tree you are editing, so that
writing Hale happens next to the system the writing describes.

Status: **DRAFT for argument.** Nothing here is committed.
DESIGN.md §14 governs sequencing; this is a candidate for what
comes after M2's backpressure half, and some of it is cheap
enough to steal earlier.

Naming: this reads better as the **locus** inspector. The spec
moved "lotus shape" → "locus shape" in August, and the thing
being inspected is a tower of loci; `lotus_` survives as the
runtime's C prefix, not as the noun. Bikeshed open.

---

## 1. The thesis

**In a Hale program, an unusual fraction of the text is a promise
about runtime behavior. The inspector is the standing verdict on
those promises, rendered at the line where each one is written.**

Count what a Hale source file declares rather than computes: a
`topic` promises a payload shape and a wire subject; a `bus`
block promises who publishes and who consumes; `params` promise
the locus tower; `on_failure` promises a supervision policy;
`bindings` promise a transport; `claims`, `@effects`, `@budget`,
`@sealed` promise law. Every one of those is a statement about
what will happen, written before it happens, in a form the
compiler already parses.

Every one of them is therefore **falsifiable by observation** —
and iris is the falsifier. That is a different product from a
debugger, which discovers structure, and from an APM, which
aggregates without knowing what was intended. Here the intent is
typed, spanned, and sitting in the editor buffer.

## 2. Why this only works here: the two blind halves

The interesting property is not that Hale has an observer. It is
that hale's checker and iris are blind in *exactly complementary*
directions, and both blindnesses are principled.

**hale is deployment-blind, on purpose.** `check --workspace`
says it outright: it "does NOT connect seeds. Each stays its own
closed world; two binaries publishing and subscribing one topic
are not linked by this, because nothing about a deployment is
visible from source and inventing those edges would certify a
system nobody deploys." The claim vocabulary says the same thing
about instances — `count publishers(topic T) == 1` counts
"distinct **declared** loci… counts over declarations, not a
runtime census of replicated instances."

**iris is source-blind.** It sees topic ids, sequence numbers,
shape hashes, locus instances. It has never known why any of it
happened, or where in a file the thing that did it is written.

Neither gap is a defect and neither side should close its own:
the compiler certifying an undeclared deployment would be a lie,
and an observer inferring intent from traffic is how every
"service map" product becomes wrong. But **almost every question
worth asking about a distributed Hale app lives in the join**,
and right now nothing occupies it:

> `count publishers(topic Orders) == 1` is a claim the compiler
> evaluates over declarations and explicitly declines to evaluate
> over a running fleet. iris can evaluate the same predicate
> against the fleet that is actually up. Same claim, second tier
> of evidence: **declared** and **witnessed**.

That sentence is the whole product. Everything below is
mechanism.

## 3. Four tiers

### Tier 0 — Identity. *Is what I'm watching what I'm editing?*

Non-negotiable, and first, because every other tier is a lie
without it. The cardinal sin of a ride-along is attributing live
numbers to source that has since changed.

Most tools guess from mtimes. Hale hands us exact machinery:

- The segment carries **`model_hash`** (PROTOCOL §3.1, proto 0.2)
  — the identity of the model the running binary was *compiled
  from*. This is the load-bearing one, and it landed 2026-08-12:
  before it, Tier 0 inferred the answer from artifact file
  digests, which is a guess about the build rather than a fact
  about the process. Now the inspector compares the model it just
  cut from the working tree against what the process was actually
  built from, and *knows*.
- `sources[]` in the artifact carries a **per-file digest**, so
  drift is attributable to the *file*, not the whole view — the
  declarations in untouched files stay trustworthy.
- `artifact_digest` (schema 1.3) covers the whole body, which is
  what makes an artifact iris did not produce safe to trust —
  the check PROTOCOL §4 now says iris owes.

**Two axes, deliberately.** Model identity excludes payload field
shape, so a payload edit moves the topic hashes and not the
model, while adding a locus or rewiring moves the model and not
the topic hashes. Both are reported, because a tool checking only
one is blind to half the drift — verified by editing a payload
and adding a locus against the same running process and watching
each verdict move alone.

Three states per declaration, and the UI never shows a number
without one: **in sync** (live data is about this text),
**drifted** (this file changed since the binary was built —
values desaturate, they do not disappear), **absent** (declared,
never observed — see Tier 1).

### Tier 1 — The ledger. *What is this declaration doing?*

Each declaration gets its live counterpart, joined as PROTOCOL §4
already specifies:

| Declaration | Live counterpart | Join |
|---|---|---|
| `topic T` | publishers, subscribers, rate, totals, per-binary shape agreement | (name, shape_hash) — manifest ↔ artifact `topics[]` |
| `locus L` | live instances, births, dissolves, restarts | type name → LOCUS_BIRTH instances |
| `subscribe T as h` | deliveries, last delivery, reaching chain | BUS_DELIVER (topic, locus instance) |
| `T <- payload` | publishes, and where they landed | BUS_PUBLISH + NET pairing |
| `bindings { }` | up/down, loss, latency, **depth** | binding counter line (cells now live; iris-side work — §7) |

**The loudest signal is silence.** A declared topic that nothing
publishes; a handler nothing triggers; a binding that never came
up. This is the bug class the type system cannot catch — every
binary is internally consistent, the wiring is just wrong — and
it costs nothing to compute, because the native emitter
registers manifest entries **lazily, on first use**. A topic
declared and never carried simply never appears. So
"declared-but-silent" is a set difference between the artifact's
`sorts.topics` and the segment's manifest, and it falls out of
the join for free.

### Tier 2 — Consequence. *What did that edit just do to the running system?*

The tier that earns the name "ride-along", and the one with the
highest value per line of code.

**Wire-shape drift, on save.** You add a field to `type Order`.
Before you have finished the line:

> `orders` payload shape `id:i;label:s` → `id:i;label:s;qty:i`
> — hash `0x64667e25…` → `0x9a12…`. The 3 processes attached are
> on the old hash. A mixed fleet will not fuse, and headered
> datagrams will not deserialize at the old peers.

This is handoff-4's P16 — the landmine where enabling
observation silently partitioned a fleet, and a stale receiver
dropped every datagram with no error anywhere. It is invisible to
the type checker *by construction*, since each binary is
perfectly consistent with itself. It needs exactly the join iris
has. And it costs one `hale check --json` (~10 ms on the largest
apps) plus a hash comparison.

**It is also buildable today with zero upstream work** — the
manifest already carries per-topic `shape_hash`, and the
artifact's `topics[].payload_hash` is the same function
(`hale_types::topic_identity`, re-verified against the pinned
vectors on 2026-08-11). Everything needed is on disk right now.

Neighbours in the same tier, same mechanism:

- A shared topic declared without `subject:` — PROTOCOL §4 says
  it carries the declaring binary's local, possibly mangled
  spelling and **will not fuse**. Today that is a rule in a
  document; it should be a squiggle.
- A new publish site that falsifies an `@effects(publish: {…})`
  set, or a claim whose verdict flips — hale computes both; the
  inspector's job is only to attribute the flip to *the edit you
  just made* rather than to the file at large. (August work made
  `check --json` friendlier here: diagnostics now carry a
  `related` array of secondary locations, added only when
  non-empty so existing consumers see an unchanged shape.)

### Tier 3 — Causality. *Why did this handler run?*

DESIGN §10's message-chain stacks, surfaced at the source. Bus
delivery is synchronous, so in-process chains nest like frames:
click a handler, get the topic-typed stack that reached it —
`orders.fill` ← `risk.check` ← NET_DELIVER `orders.new` from
pid 1234 — with each frame a jump target, because
`provenance.subscribes` carries the span.

Frames named by declared topics is a thing no other runtime can
render. It is also the most work here and it wants M3's retention
buffer to be worth using. Correctly last.

### Tier 4 — The agent

Already committed in DESIGN §11: the co-debugger reads the same
query surface. Worth noting only that its context here is
strictly richer than either a code agent (source, no run) or an
observability agent (run, no source) can assemble — it gets the
join, which is the thing neither product category can buy.

## 4. Signal discipline: why this doesn't become noise

The obvious failure is a hundred live numbers vibrating in the
gutter. The division of labor that avoids it:

> **The flower reports state. The inspector reports change and
> contradiction.**

Which is also the answer to "why not just dock the flower in the
editor" — a canvas of live gauges is the wrong instrument for a
buffer you are typing into.

Consequences: ambient state is **one status-bar line** ("3
processes · 12 topics live · 2 declared-silent · in sync"), not
a hundred inlays. Gutter marks fire only on anomaly —
declared-silent, drifted, claim falsified, restarts at the
declared cap. Full per-declaration inlays are a toggle, not a
default. And an anomaly is a link *into the flower at that
timestamp* once M3 scrubbing lands: editor answers "where in the
code", flower answers "what was happening", one state, two
projections.

Non-negotiable, inherited from DESIGN §4: **no special build, no
special run mode.** `LOTUS_OBS=1` on the thing you already run,
or nothing. An inspector that needs its own build profile is a
profiler, and we already refused that trade.

Out of scope, per DESIGN §11: any write path. v1 is read-only
because there is nothing to gate; intervention (pause a locus,
inject a message) brings PermissionGate back with it, and that is
a different milestone.

## 5. Architecture: mostly already built

The pleasant surprise is how little is new. DESIGN §11 already
committed to fuse-hl's HTTP/SSE surface double-serving as the
query API ("one API serves both clients"). The inspector is the
third client of that same surface.

What exists: fuse-hl attaches, fuses N segments, serves
`/snapshot` and `/events`. What the inspector adds is a **source
index** —

1. fuse-hl ingests topology artifacts (`hale check
   --dump-topology`), verifying `artifact_digest` first.
2. It joins them to attached segments on (name, shape_hash) for
   topics and on type name for loci — the join PROTOCOL §4
   already defines.
3. It indexes by `sources[].path` + byte span, and serves
   `/inspect?file=…` → spans with verdicts. SSE pushes deltas.
4. The editor extension is thin by design: ask, render inlays and
   gutter marks, open links. Editor-agnostic because the
   interesting work is server-side.

Byte spans convert to line/col client-side. That keeps the seam
narrow enough that a second editor is an afternoon.

**On not being a language server (revised 2026-08-12).** When
this was drafted, hale's editor story was `check --json`. It is
now a real language server — `hale lsp` serves hover, completion,
definition, references, documentSymbol, formatting and
diagnostics, and August work added go-to-definition through
`std::` paths. That makes the "don't fight it" instinct stronger,
not weaker, and it sharpens what the inspector should be:

- **Not** a competitor for hover/definition/references. hale's
  LSP owns those, correctly — they are questions about *the
  code*, and it has the compiler.
- **A second server that only publishes what it uniquely knows.**
  Editors run multiple language servers against one language
  happily. An iris server that publishes *only* diagnostics
  (drift, declared-silent, claim falsified) and inlay hints
  (live counts) composes with hale's rather than duplicating it,
  and needs no cooperation from upstream to exist.

The tempting shape is the wrong one: live counts would read most
naturally *inside* hale's hover card, and that would require the
compiler's LSP to reach into the observation plane — coupling the
compiler to the observer, which DESIGN §13's ownership split
exists to prevent. If that integration is ever wanted, the right
form is upstream exposing a hover-contribution hook, not iris
growing compiler features or hale growing observer ones. Not
worth asking for until the standalone version has proven it earns
the screen space.

## 6. What the substrate gives us today (verified 2026-08-11)

Better than expected:

- `provenance.decls` — every locus, type and free fn as
  name → `{source, span}`.
- `provenance.publishes` / `.subscribes` — the publish and
  subscribe **sites**, spanned, with fn/locus/handler/subject.
- `relations.calls` — the static call graph.
- `sources[]` — path plus **per-file digest** (Tier 0).
- `topics[]` — name, wire subject, canonical shape, payload hash.
- Manifest carries per-topic `shape_hash`; registration is lazy,
  which is what makes silence detectable.

## 7. What was missing — the board, cleared 2026-08-12

Every gap this document identified was filed as handoff-12 and
resolved upstream in one batch (`41c357f`). Recorded as history
rather than deleted, because the sequence is the point: the
design named four things it could not do, and naming them
precisely is what got them built.

- ~~**Model identity in the segment**~~ — shipped as
  `model_hash` at header `0x80`, proto 0.2 (PROTOCOL §3.1).
  Tier 0 is now a fact, not an inference.
- ~~**Topics have no declaration span**~~ — every name in
  `sorts.topics` now has a `provenance.decls` entry, so a lens
  can anchor on the `topic` line itself.
- ~~**Supervision is absent from the artifact**~~ — schema 1.10
  adds a hashed `supervision` section: supervising locus,
  supervised child and error types, the recovery ops the body
  invokes, and a literal retry bound when written, with spans in
  `provenance.supervision`. A policy change now moves the model
  identity. This unblocks the annotation §3's Tier 1 called the
  most valuable one available: *declared retry cap 3, observed 3
  in 40 s*.
- ~~**Binding cells 3–5 unwritten**~~ — `queue_depth`,
  `send_block_ns` and `retries` are populated (counters-tier,
  measured only under `LOTUS_OBS`). iris has not yet consumed
  them: fuse-hl fuses topic lines and does not surface binding
  lines at all, so the bindings row in §3's table is now blocked
  on **iris**, not upstream. That is the M2 backpressure work.

**Accepted coarseness, not an ask:** when one locus type has two
publish sites to the same topic, records cannot say which fired —
the 16-byte slot is full and buying a site id is not worth a
protocol break. Attribute to the type, list the sites.

## 8. Sequencing

Ordered by value over cost, not by tier number:

1. ~~**Artifact ingestion + `artifact_digest` verification**~~
   **DONE** (`inspect/`, 2026-08-11). Pure Hale, no FFI — a
   client of fuse-hl's HTTP surface rather than a second attach
   path, so it works remotely for free. FNV-1a/64 is computable
   in Hale (`Int` is i64 and wraps two's-complement, verified
   against PROTOCOL §4's pinned vectors), so the digest check
   needed no C.
2. ~~**Wire-shape drift on save**~~ **DONE.** Edit a payload
   type, re-cut the artifact, and the running fleet's old hash
   is named against the tree's new one. Exit status 1 on drift,
   so it gates a pre-deploy hook rather than only reading out.
3. ~~**Declared-but-silent**~~ **DONE**, and it earned its keep
   immediately — see below.
4. ~~**Tier 0 model identity**~~ **DONE** (2026-08-12), the same
   day `model_hash` landed. Verified on both axes against one
   running process: a payload edit moves the topic hashes alone,
   adding a locus moves the model alone, and each is reported
   without the other. Also distinguishes *no model* (the synth
   emitter, a real 0) from *unknown* (a proto 0.1 emitter), which
   is the distinction the protocol insists on.
5. **Binding counters into the snapshot** ← next, and newly
   unblocked. fuse-hl fuses topic lines only; the binding lines
   carrying depth / block-time / retries are live upstream and
   nothing reads them. This is M2's backpressure half and it is
   now entirely iris-side work.
6. **Source index + `/inspect`**, then a thin extension.
7. Supervision annotations — newly possible (schema 1.10), and
   the richest Tier 1 annotation available.
8. Causality (Tier 3) — after M3 retention.

**What slice 1 found on its first run — and how it closed.**
Pointed at a 40-line toy app, the declared-but-silent check
reported a topic as never observed while it was visibly driving
the app's downstream topic. The behavior was real: a topic whose
subscribers all sit inside the publisher's own locus subtree,
with no transport binding, was rewritten by
`desugar_intra_locus_topics` into a direct call to the handler —
before lowering, so the handoff-5 fix that gave the *codegen*
direct-dispatch flavors their probes never reached it. Filed as
handoff-11 P23 with a controlled repro and its control
(`inspect/upstream-repro/`).

**Fixed upstream the next day** (`5567bf2`), and fixed at the
level that matters rather than the level asked for: the ask was
ranked "probes ideally, but at minimum register the topic
anyway," and the desugared call site now emits both probes with
full attribution. The standing witness in `examples/inspect-demo`
flipped from `declared-silent` to `in sync` at pub 80 / dlv 80,
while the genuinely-unpublished topic stayed silent — the verdict
moved where it should and held where it should. Absence once
again means "never mentioned at runtime", uniformly across
dispatch flavors, which is the property the whole Tier-1 ledger
rests on.

Two lessons, and the second one is the uncomfortable one.

**The join earns its keep.** Nine rounds of fleet field-testing
did not surface this, and no amount of further staring at live
data would have: "what did I declare that the system has never
mentioned?" is not answerable from the observation plane alone,
nor from the source alone. The first tool to occupy the gap found
something in its first minute.

**And the first characterization of it was wrong.** The initial
handoff blamed the publish being in a `main locus`'s `run()` —
which fit every repro, and was false: two axes were confounded
(main-vs-child, and publisher-is-parent-of-subscriber), and the
real axis was the second, with a third variable (keyed vs plain)
never varied at all. A passing upstream test that contradicted
the claim is what caught it. The correction is worth writing into
the design because this tool's entire output is *inference from a
join*, and a join makes wrong hypotheses feel evidenced — the
verdict is only ever as good as the axis it varied. Slice 4's
per-declaration verdicts inherit that hazard directly, which
argues for verdicts that name what they compared, not just what
they concluded.

## 10. Witnessed law (built 2026-09-03)

The claims surface refactored hard through August — the evaluator
was retired, the artifact reached schema 1.17 — and in doing so it
grew the two sections this whole idea needed. Claims, groups,
constitutions are now emitted machine-readable in `law.rows`
(structured: `kind`, `cmp`, `n`, resolved group/topic refs with
spans — no form-string parsing), and, decisively, the compiler
publishes **its own epistemic status**:

- `adequacy` — per judgment family (`reachability`, `endpoint`,
  `bound`, …), `exact` or `degraded`.
- `capabilities` — `exact_publishes`, `exact_routes`, … and the
  permanent `exact_delivery_guarantees: false`, because delivery
  is not statically knowable.

That is the seam. iris does not reinterpret law; it fills in
exactly the boxes the compiler has marked unfilled. The claims
chapter says it in prose — "what is not knowable statically is
exposed as a boundary, never silently approximated" — and iris is
the thing that observes the boundary.

**Shipped: the law perspective (`inspect`-independent, in the
flower).** `fuse-hl` takes an optional artifact path, verifies its
`artifact_digest` (refusing, not warning, on mismatch — the
`law`/`groups` sections are exactly what `shape_hash` does not
cover), re-ingests on digest change at 1 Hz, and emits a `law`
section in `/snapshot`: every claim with its **static** verdict
beside a **witnessed** state evaluated over the observed fleet.
Five witnessed states, two of them new to the world:

| static | witnessed | meaning |
|---|---|---|
| holds | consistent | law confirmed in the field (names its basis) |
| holds | **contradicted** | the deployment does what the model could not represent |
| holds | **not_exercised** | the wiring exists; no traffic ever pressured it |
| holds | unwitnessed | traffic exists but attribution can't bound the claim |
| any | unsupported | this claim form has no witnessing rule yet |

`contradicted` is the payoff: `count publishers(topic Metrics)
== 1` counts declared loci **in one seed**, and hale refuses on
principle to look across a deployment. A second binary publishing
the same subject is invisible to the checker and plain to the
observer — witnessed contradicted, both writers ringed red on the
canvas. `not_exercised` is coverage for law: a `forbid` holding
because the boundary was pressured by real traffic is a different
fact from it holding because a wing never ran.

**The render (perspective `[3]`).** The process flowers, with the
model's law drawn on the running system: each `group` becomes a
hull around its live member petals; clicking a claim in the DOM
ledger focuses it (participants bright, the rest dim); a
`contradicted` `forbid` draws its observed one-hop route in alarm;
a `contradicted` `count` rings every observed writer red. Epistemic
status is a render property, not a footnote — a claim under a
`degraded` proof family draws a hollow dot and a dashed hull, and
the ledger flags when witness evidence comes from a process built
from a **different** `model_hash` (its evidence is about other
law). Demo: `examples/claims-demo` (app + a rogue second writer),
verified live in the browser across every state.

**The asymmetry is load-bearing and preserved.** iris can falsify
a universal or report non-exercise; it can never prove one — a
`forbid` "consistent" is only "no counterexample on the paths
taken", so it always names its basis. And claims count declared
loci while iris counts live instances; every verdict says which it
counted. A join makes wrong hypotheses feel evidenced (§8 learned
this the hard way with P23), so the verdicts name what they
compared, not only what they concluded.

## 9. The honest bottleneck

Not the editor integration; that is a weekend once the index
exists. It is that **the inspector is only as good as the fleet
running under it**, and the tight authoring loop this document
imagines — edit, save, see consequence — is a *single-process,
local* loop, while every hour of field hardening iris has spent
was on a multi-process production fleet. Those have different
failure modes. A local one-binary app has no cross-process edges
to fuse, no wire shapes to disagree about, and its "fleet" is one
segment: precisely the case where Tier 2's headline feature has
nothing to compare against.

So the sharpest version of this tool is for someone editing **one
service of a system that is already up** — which is the real
workflow, and worth being deliberate about rather than
discovering later. The pure-local case degrades to Tier 1 plus
`hale check`, which is pleasant but is not the thing.
