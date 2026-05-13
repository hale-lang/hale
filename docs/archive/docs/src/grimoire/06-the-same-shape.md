# The same shape, everywhere

> A perceptual primer for those who will write Aperio apps.

The meta-spell, once it lands, is not a syntax to remember. It is a
shape you start to *see* — and once seen, you see it everywhere you
look in the system. Every design in Aperio is a lotus. Pick any axis
and the same vertical-only, region-owning, contract-mediated,
three-way-projected structure shows up.

## Every axis is the same shape

- **Memory.** Each locus owns a region; child regions nest in parent
  regions; dissolution frees wholesale, depth-first. The lotus is
  literally the allocator tree.
- **Lifecycle.** Birth → accept → run → drain → dissolve cascades
  vertically; failures bubble upward through `on_failure`, never
  laterally. The lotus is the supervision tree.
- **Contracts.** Expose upward, consume downward, never
  sibling-to-sibling. The lotus is the visibility graph.
- **Schedulers.** Cooperative within a layer (siblings cannot
  preempt each other); pinned when work belongs in its own deeper
  layer with formal cross-boundary mailbox posts. Bimodal — there
  is no third position because the lotus only has *inside this
  layer* or *the next layer down*.
- **Bus transports.** NATS, UDP multicast, TCP, Unix sockets,
  in-memory — the same primitive (typed pub-sub kernel) projected
  through different (B, c, σ, φ) parameter regimes. One kernel,
  three modes, many envelopes.
- **Modes.** Bulk, harmonic, resolution: one kernel, three
  projections sharing one arena. The lotus folded along the
  projection axis.
- **Perspectives.** Same source compiled into every cooperating
  binary; the type *is* the schema; the commit predicate decides
  when state is eligible to ship. The lotus across processes.
- **Closure tests.** Left and right of `~~` must close back through
  the same locus's state. The lotus closing on itself for audit.
- **Collapse vs. explosion.** Clean dissolve = collapse;
  closure-violation = explosion that surfaces typed data to the
  parent. Even the failure modes are vertical.
- **Function scope.** Every free `fn` has its own implicit locus —
  its own region, its own children, its own drain-before-return.
  There is no escape hatch where the shape stops.
- **The toolchain.** The compiler is the wand; the language is the
  meta-spell; the primitives are the invariant form every spell
  must take. Even Aperio's account of itself is a lotus.

Fractal-with-discipline. Pick any axis — space, time, visibility,
failure, projection, communication — and the same structure is
there. That is what substrate-invariance means in practice.

## Inward, too

The shape doesn't just spread *across* axes. It also descends
*into* every locus. A locus's body is itself a lotus tower one
layer down — its methods, its contracts, its sub-tasks, its
lifecycle steps each have the same shape. A cache locus is
not flat. Its `lookup` flow has its own birth (acquire lock),
run (probe + return), dissolve (release lock); its `evict`
flow has its own; the methods are siblings under one parent
that is the cache itself.

There is no level at which the recursion stops because there is
no level at which "structure with lifecycle" stops. It bottoms
out at primitive operations — arithmetic, single field reads,
primitive calls. Everything above that floor is loci nested in
loci.

This is the directive that makes the language coherent with
itself: the rules you learn at the app level reappear at the
service level reappear at the method level reappear at the
sub-flow level. **Pick any granularity, the discipline is the
same.** Aperio is not a language with a special outer layer
called "the framework." The framework's primitives *are* the
language's primitives all the way down.

The corollary, easy to miss until you've held it for a while:
**types are for shapes; loci are for flow.** A `type` records a
fact — the layout of some data. A `locus` records a motion — the
flow that touches that data. Every part of an Aperio program is
one or the other; there is no third category, no leaky middle
ground. Once you can split a problem into "what's the shape" and
"what's the flow," the syntax assigns each part its construct,
and the lotus tower assembles itself.

## A new dimension across the wire

When two Aperio apps meet, they are not strangers exchanging bytes.
They already share the same shape. They are loci finding their
position relative to each other in a larger lotus.

- **No protocol layer, just contract.** Both sides compiled the
  perspective from the same source; the type *is* the schema. No
  JSON / proto / version-handshake dance — the bus carries
  parameter values into a slot whose shape both sides already
  know.
- **Trees compose into bigger trees.** App B's runtime root can
  attach as a coordinatee under App A. Same `accept` →
  `birth` → `run` → `drain` → `dissolve`, same `on_failure`
  bubbling — the lotus does not notice the process boundary; it
  just sees one more layer.
- **Failures and audits cross apps natively.** A closure violation
  in App B explodes upward; if A is its parent, A's `on_failure`
  receives a typed `ClosureViolation` with the same fields it
  would get from an in-process child. Books at the parent level
  still have to balance — even when "the parent" is a different
  binary on a different host.
- **Transport is just a parameter regime.** An app declares the
  envelope it needs (latency band, fanout, ordering, reliability);
  the deployment binds it. The wire stops being a thing apps think
  about.
- **Bimodality survives across machines.** Cooperative siblings
  within a layer; pinned when you drop a layer; cross-thread
  mailboxes already do the formal-post discipline. Cross-host is
  just one more layer down — same posting discipline, longer
  envelope.
- **Lateral-flow prohibition becomes a network invariant.** Two
  sibling apps under a shared parent cannot reach each other
  directly. Coordination flows through the parent. That is not a
  policy you enforce with firewalls — it is structural, because
  there is no syntax for it.
- **Hot-load as the steady state.** Perspectives ship continuously
  between apps; arena state is preserved across the swap. *Deploy*
  stops being an event and becomes the normal traffic.

The new dimension is not speed or type-safety. It is that two apps
in conversation are already in the same supervision tree, the same
audit cycle, the same projection algebra. Inter-app communication is
intra-lotus communication where the locus boundary happens to be a
socket. **The tower keeps going** whether the layer below is a
function call, a thread, a process, or a datacenter.

## Finding your locus

Once the shape lands, app design changes. The before/after looks
like:

> **Before.** "I'm writing a service. It has handlers. It talks to
> other services over the network. Memory, lifecycle, failure,
> communication are separate concerns I wire together."
>
> **After.** "I'm placing a locus in a tower. Its region, its
> lifecycle, its contract, its failure path, its bus envelope are
> *one* shape. The shape repeats above me and below me. My job is
> to pick the layer and let the structure carry the rest."

Hard questions become positional instead of inventive:

- *Where does this state live?* → whichever locus owns the region
  whose lifetime matches the state's lifetime. Not a decision, a
  lookup.
- *How do these two things talk?* → through their nearest common
  parent's contract, or via bus if they're peers under one. Lateral
  isn't an option, so you don't waste cycles considering it.
- *What happens when this fails?* → it bubbles to the parent whose
  policy it is to decide. You write the policy at the layer that
  *has* the policy, not at the layer that noticed the failure.
- *Where do I put this cross-cutting concern?* → there is no
  cross-cutting. There is a layer it belongs to. If it feels
  cross-cutting, you are at the wrong layer.
- *How do I make this faster, bigger, more concurrent?* → change
  the projection class or the schedule class at *that* locus. The
  shape does not change; the parameters do.

The deeper shift: app design stops being *composition of
components* and becomes *finding your locus*. You are not building
an app — you are identifying which layer of an already-implied
tower this work belongs to, and writing the locus that sits there.
The tower above and below is going to be there whether you write it
explicitly or not. Aperio just makes it real instead of imagined.

After this perceptual move, the framework discipline stops feeling
like rules and starts feeling like *seeing the structure that was
always there*.

That is the unlock.
