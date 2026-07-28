# iris — forward vision (post-observer)

Deliberately out of scope for the observer milestones
(DESIGN.md §14). Recorded so the commitments survive until
it's time. Nothing in M1–M3 bends toward this; the only
present-day obligation it imposes is keeping `shape_hash`
well-defined and source-derivable (PROTOCOL.md §4, freeze
open item).

---

## The vision

**A visual plug-and-play high-level Hale editor.** A local
folder of prebuilt loci — a parts bin — assembled by drag
and drop on the same canvas the observer renders. Compose a
system from parts, wire topics, hit run, and the diagram you
drew lights up with real telemetry.

## Why this is credible in Hale when visual programming
usually isn't

General-purpose visual programming has a fifty-year
graveyard, and the cause of death is almost always the same:
semantically vague edges — boxes wired by hope. Hale's edges
are **typed, declared contracts**. A locus publishes and
consumes topics with declared payload shapes, so "does this
plug fit this socket" is *decidable at drag time* — and the
compatibility check is the same `shape_hash` the observation
protocol already uses as its fusion join key. One function
serves both: identity for the observer, compatibility for
the editor.

Part manifests need no hand-authoring: a part's consumed and
published topics, params, capacity slots, and supervision
defaults are derivable from its source (per-part
`.hale.topo`, eventually). The parts bin is just a folder of
loci — which is to say, it is **pond**. The existing
vendored-folder package tier is the distribution mechanism;
no registry required.

## The differentiator: the diagram is the instrument

Every prior visual tool separates the design-time diagram
from the run-time system, and the diagram rots. Here the
canvas you compose on is the same scene the observer renders:
run the assembly and the edges you drew become gauges —
throughput, depth, latency, loss — and the parts you dropped
become petals with live lifecycle. Design-time and run-time
are one artifact. The diagram cannot lie because the diagram
is the live view. Nobody else can close this loop, because
nobody else has an observation plane under the canvas.

Try-before-commit falls out: "preview" a candidate assembly
by actually running it sandboxed with the observer attached —
including against mock producer/consumer parts from the bin.

## Load-bearing commitments (recorded now, binding then)

1. **The editor emits Hale source. Never a proprietary graph
   format.** Dropping a part writes the locus declaration and
   bindings entries a human would have typed. Hand-edited
   source still renders; the visual layer is a projection of
   text; text remains truth. This is the difference between
   "high-level Hale editor" and "walled diagram tool with an
   export button."
2. **Structure is visual; behavior is textual.** Wiring loci,
   supervision, placement: canvas. What happens inside a
   handler: source pane. (LabVIEW died making behavior
   visual; Blueprints thrived by not.)
3. **Agent and human assemble through the same write path.**
   This is where the May spike's parked machinery returns:
   the two-channel bus, PermissionGate, and structured-output
   proposal dispatcher were built for concurrent human+agent
   writes. The observer never needed them; the editor is
   precisely their re-entry. Drags and agent proposals hit
   the same validated dispatch, gate enforced at the bus
   layer. Observer (v1–2) reuses the spike's rendering;
   editor (v3) reuses the spike's write machinery.
4. **Wire-validity is contract-checked, with typed
   refusal.** A shape-incompatible connection is refused at
   drag time; the editor may offer an adapter-locus stub, but
   never silently coerces.

## Type-driven assembly

Because compatibility is computable, so is suggestion: given
a part's unbound consumed topics, the palette can highlight
every part in the bin that publishes a compatible shape —
autocomplete for architecture. Same mechanism, inverted.

## The honest bottleneck

The editor is not the hard part; the **parts economy** is. A
bin of mismatched shapes is adapter hell — every wire needing
a glue locus. Pleasant assembly requires curated topic
vocabularies per domain (a pond-content problem, not an iris
problem), and the ecosystem to fill the bin. This is why the
vision is sequenced after the observer earns iris its
audience, not before.

## Sequencing

- v1–v2 (DESIGN.md M1–M4): observer, read-only. No editor
  scope leaks in.
- v3: assembly — the write side returns, with PermissionGate
  and the proposal protocol, on top of a proven observation
  plane.
- Present-day obligations only: `shape_hash` stays
  well-defined and source-derivable; nothing else.
