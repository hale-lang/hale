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
boundary is in [`COCKPIT-CONTRACT.md`](./COCKPIT-CONTRACT.md), and its visual
direction is in [`cockpit/DESIGN.md`](./cockpit/DESIGN.md). The issue defines the
full product scope.

The [browser cockpit](cockpit/README.md) reads declared Organization structure,
real Practices and Reviews through
the [DNA read API](../dna/api/README.md). It serves from the same origin as the
Hale API, shows source revisions and content availability, and connects to a
configured native Runtime observer for in-cockpit process/locus inspection.
Runtime also works from an independent static host without DNA. Knowledge and Definitions views require compatible
providers advertising their read capabilities. Practice proposals and
exact-candidate Review decisions each require their own advertised capability
from an explicitly composed command provider; the standalone API remains read-only.
Static declarations do not establish semantic position authority. Runtime
observation itself continues to work independently of DNA.

From a checkout, `./iris/cockpit/start.sh PROJECT` builds and starts the native
DNA API with the browser. An application's composed API can be supplied through
`--api BINARY`; existing body/state/observer processes remain independent.

Ordinary Hale applications can compose the [generic application service](service/README.md)
and expose an application-owned control in the same cockpit. The
[intake-control example](examples/intake-control/README.md) demonstrates a real
mode change, observed work behavior and durable receipt recovery across process
restarts. Its database and authority belong to that application; the generic
head does not require DNA or implement a shared command store.

The previous spike (modeling-cockpit framing, May 2026) lives
on `main` — it remains the artifact archive (raylib bindings,
lotus_viz, heron, pane system, MCP server). Pieces get pulled
in here if and as needed; nothing is inherited by default.
