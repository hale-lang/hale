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

The previous spike (modeling-cockpit framing, May 2026) lives
on `main` — it remains the artifact archive (raylib bindings,
lotus_viz, heron, pane system, MCP server). Pieces get pulled
in here if and as needed; nothing is inherited by default.
