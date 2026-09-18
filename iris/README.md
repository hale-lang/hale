# iris

A realtime observer for running Hale systems. Attaches to
live processes over shared memory, fuses N processes into one
system graph, and renders the locus tower, message topology,
and transport behavior as they actually happen — live edges
with throughput/depth/latency/loss, supervision events as
they fire, flight-recorder scrubbing after the fact.

This branch (`observer`) is a ground-up redesign and a clean
slate. **Start with [`DESIGN.md`](./DESIGN.md).** The shm
contract lives in [`PROTOCOL.md`](./PROTOCOL.md); the
post-observer editor vision in
[`VISION-FORWARD.md`](./VISION-FORWARD.md).

The human cockpit proposal is tracked in
[#690](https://github.com/hale-lang/hale/issues/690). Its proposed browser/API
boundary is in [`COCKPIT-CONTRACT.md`](./COCKPIT-CONTRACT.md), with current DNA
readiness and the first live slice in
[`COCKPIT-READINESS.md`](./COCKPIT-READINESS.md). The shared service API, Compose
deployment and staged implementation cards are in
[`SERVICE-DEVELOPMENT-PLAN.md`](../dna/SERVICE-DEVELOPMENT-PLAN.md).
These describe the broader product and its remaining implementation work.

The [browser cockpit](cockpit/README.md) reads declared Organization structure,
real Practices and Reviews through
the [DNA read API](../dna/api/README.md). It serves from the same origin as the
Hale API, shows source revisions and content availability, and links to the
existing Runtime observer. Knowledge and Definitions remain core workspaces
awaiting their typed read adapters; semantic position bindings and administrative commands are not
yet available. Runtime observation itself continues to work independently of DNA.
The [delivery checklist](cockpit/DELIVERY.md) retains the full product scope and
its outstanding end-to-end gates.

The previous spike (modeling-cockpit framing, May 2026) lives
on `main` — it remains the artifact archive (raylib bindings,
lotus_viz, heron, pane system, MCP server). Pieces get pulled
in here if and as needed; nothing is inherited by default.
