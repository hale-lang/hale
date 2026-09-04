# iris — design (observer reframe)

Ground-up redesign, 2026-07-22. Supersedes the modeling-cockpit
framing (`VISION.md` on `main`). The spike on `main` remains the
artifact archive; this branch is a clean slate. Pull spike code
in if and as needed — nothing is inherited by default.

---

## 1. What iris is

**iris is a realtime observer for running Hale systems.** It
attaches to live processes over shared memory, fuses their
views into one system graph, and renders the graphs the
compiler and runtime already know — message topology,
locus lifecycle, ownership/child trees, supervision
transitions, transport behavior — as they actually happen.

Not a modeling tool. Not an IDE. A debugger-class instrument:
the flower renders **ground truth**, the observed state of an
actual process, never a description of intent alone.

The prior framing ("agent writes Hale to model any domain")
asked the renderer to display structure an agent invented.
Every claim in this design is checkable against a running
binary instead. The universal-modeling thesis is not dead —
it returns, if ever, through the front door: model a domain by
writing *executable* Hale, then observe it.

Prior art and the gap: Erlang's `observer` watches a
supervision tree that only exists at runtime — it cannot say
what the program intended. tokio-console watches anonymous
tasks. Hale's topology is **declared**: typed topics, locus
tower, bindings, placement, supervision policy are all static
facts. iris is the observer that knows the score before the
performance starts.

## 2. Value center: networked bindings

The static readout of a codebase is a compiler-warnings-grade
feature — a coordinate system, not a product. The value is
watching a **distributed** Hale system run, because networked
bus bindings (unix/udp/tcp topics across processes and
machines) drive the actual behavioral characteristics of
nearly all Hale apps: latency, loss, queue depth,
backpressure, restart cascades across process boundaries.

Consequences:

- The observation domain is **the system, not the process**.
  iris attaches to N processes and fuses.
- Milestone 1 is a *running two-process system over a real
  binding*, not a static visualization (§14).
- The instrumentation design treats networked edges as
  first-class instruments (§7), not decorations.

## 3. Architecture: two layers

1. **Static skeleton** — compile-time metadata emitted into
   the binary (`.hale.topo` ELF section; self-describing
   artifacts, no sidecar to lose). Role: **plumbing, not
   product.** It is (a) the *decoder ring* for packed runtime
   records (§5) and (b) the *join key* for multi-process
   fusion (§8). It also enables intent-vs-actual overlays
   (declared-but-silent topics, hotter-than-designed edges,
   restart counts exceeding declared retry caps) — rendered
   diffs only a declared-topology language can draw.
2. **Dynamic overlay** — runtime events and counters streamed
   through per-process shared-memory rings (§5), fused and
   rendered live: births grow petals, dissolves wilt,
   restarts shimmer, traffic drives edge gauges.

## 4. Observation plane: one-way glass

- Every Hale binary is **always instrumented, dormant** —
  USDT-style probes compiled in unconditionally, guarded by a
  flag word in the shm header. No `--observe` build. The
  claim is: *attach to any running production Hale binary, no
  rebuild, no redeploy.* Observability is a property of the
  language, not of a build profile.
- The observer maps the region **read-only** except for the
  designated control words (mode mask, §6). The observed
  process cannot be corrupted by the observer.
- The observer must never backpressure the observed app.
  Ring-full policy: overwrite-oldest + a drop counter. The
  drop counter is itself telemetry — "you are observing
  faster than you consume" is a fact iris renders, not hides.

## 5. Runtime emission: packed records, per-scheduler rings

**Rings.** One SPSC ring per scheduler (Hale runs one
scheduler per core; rings map 1:1). No cross-thread atomics
on the hot path — local store + fence; the consumer merges
N rings. LTTng/tokio-console shape.

**Records.** Fixed-width packed records; enum-heavy. This
works *because* Hale's enum spaces are closed at compile
time — topics, locus types, event kinds, supervision
transitions are finite sets the compiler owns, so small-int
encodings are fully decodable against `.hale.topo`. Generic
runtimes intern strings at runtime to get this; we get it
from the language.

Baseline layout (v0 sketch, to be tuned):

```
u64:  topic_or_locus_id:20 | event_kind:4 | size_class:8 | ts_delta:32
u128: adds seq:32, aux:32 (rich variants; networked sends, structural events)
```

Discipline: **freeze the format brutally.** One or two fixed
layouts, a version byte in the shm header, no per-record
variability. Variable width kills consumer merge speed and
reasoning ability.

**Cost model.** At the 19M msgs/sec cross-thread benchmark
ceiling, 8-byte records ≈ 150 MB/s — single-digit % of memory
bandwidth; real apps run orders below the ceiling. The
expensive field is the **timestamp**, not the payload:
cycle-counter deltas, low-res, batch-stamped — tunable. Per
event cost target: a few ns uncontended vs ~50 ns/hop at the
benchmark rate.

**Structural events** (birth, dissolve, restart, supervision
transition, placement change, arena watermark, gate/binding
lifecycle) are low-rate and always emitted in rich form.

## 6. The dial: per-topic emission modes

Emission is a **per-topic dial, not a global choice** — a
mode mask in the shm header indexed by topic id,
observer-tunable through the control words:

```
OFF → COUNTERS → PACKED (8–16B/event) → SAMPLED-RICH (1-in-N full) → FIREHOSE (one topic, everything)
```

Shipping default: **PACKED everywhere.** Per-event records
in-process buy what counters cannot: in-process sequence,
interleaving, causality chains, latency histograms. The
in-process/networked distinction is packed-vs-rich, not
counters-vs-telemetry.

## 7. Networked edges are instruments

The happy asymmetry: in-process hops are ~50 ns; networked
sends are socket-bound and orders slower — so networked
messages can afford rich per-message metadata.

Per (topic, binding), the transport emits:

- **sequence numbers** → sent-vs-delivered per edge →
  **message loss is a rendered, first-class quantity.** A
  lossy edge glows. The silent-loss failure mode
  (hale#227) becomes structurally impossible to miss.
- **timestamps both ends** → per-edge latency distributions.
  Same host: shared monotonic clock. Cross-machine: offset
  estimation in the relay (§8), deferred.
- **queue depth, socket-buffer occupancy, send-block time,
  retry counts** → the behavioral characteristics. Edges
  render as gauges (throughput / depth / latency / loss),
  and backpressure propagation is watchable through the tree.

**Coordination note (time-sensitive):** the transport layer
is currently open on the bench for hale#227/#231. Sequence
numbers and per-binding counters should be designed into
that rework — a counter struct and a seq field in the
framing — not retrofitted. File the linked issue before that
work closes.

## 8. Fusion: many processes, one flower

- **Topic identity across binaries.** Both sides of an edge
  are built from the same topic declaration
  (monolith-to-distributed story), so topic id = content
  hash of qualified name + payload shape, emitted in
  `.hale.topo`. Fusion joins on it exactly; no string
  heuristics.
- **Discovery (v1).** Observer-enabled processes drop a
  registration file in a per-user runtime dir
  (`/run/user/<uid>/hale/`); iris watches the dir.
  Attach-by-pid stays as manual fallback. If a stronger
  deployment/session identity emerges from placement design,
  it supersedes this. [OPEN]
- **Cross-machine (deferred).** A per-host relay reads local
  shm and streams to iris over tcp; adds clock-offset
  estimation. The observer shape is transport-agnostic —
  NATS/MQTT adapters (Wave B) slot into the same per-binding
  telemetry contract if they ever land.

## 9. Flight recorder

The rings are history. Two features fall out nearly free:

- **Scrubbing** — drag backward through the retained window;
  watch the tree as it was before the interesting thing.
- **Post-mortem mode** — on crash/abort the runtime dumps
  rings to a file; iris opens the dump with the identical
  UI. Live observation and crash forensics are the same
  tool. A supervision cascade replayed frame-by-frame after
  a real crash is the single most persuasive artifact this
  ecosystem can produce.

## 10. Intra-locus stacks

Two distinct notions of "stack," both rendered; neither adds
hot-path cost.

**Message-chain stacks (in-protocol, free).** Bus delivery is
synchronous — a publish invokes subscribers in the same call
stack (spike finding, `main` FRICTION.md). So in-process
message chains nest exactly like frames:
`deliver(A) → handler → publish(B) → deliver(B) → …`. The v0
record set (BUS_PUBLISH/BUS_DELIVER with per-topic seq,
total order within a scheduler) reconstructs these chains as
**topic-typed stacks**: click a petal mid-scrub, see
"handling `orders.fill` ← `risk.check` ← NET_DELIVER
`orders.new`." Frames are named by declared topics and the
chain walks the flower across loci. This is the Hale-native
stack; no other runtime can label frames with declared,
typed topics.

**Native callstacks (consumer-side sampling).** For
"why is this handler slow": iris runs a sampling profiler
(`perf_event_open`, same-user permissions) against the
observed process. Zero runtime cooperation, zero ring
bandwidth — sample data never enters the segment (wrong rate
class). The join from sample `(tid, ts, stack)` to petal is
the per-scheduler `current_locus` gauge (PROTOCOL §9): each
petal expands into a **time-scoped flamegraph**, scrubber-
synced since samples share CLOCK_MONOTONIC with the rings —
flight-recorder flamegraphs fall out.

Dependencies (quality, not blocking): demangling Hale's
symbol scheme (`__lib_..._Type` pattern — mechanical);
source-line frame quality tracks the C backend's debug info
(upstream quality ask, someday).

Anti-scope: **no per-function-call instrumentation through
the rings.** Function calls run 2–3 orders hotter than
messages; sampling exists precisely to make stack visibility
rate-independent. LOCUS_ENTER/EXIT events (PROTOCOL §8,
reserved) can later upgrade flamegraph scoping from
sampled-approximate to span-exact — they're message-rate,
so PACKED absorbs them.

## 11. The app: what survives from the spike

- **Trinity survives, reweighted.** viz = the flower as
  instrument; source = the code the petals map to (petal →
  jump-to-declaration via source spans in `.hale.topo`);
  agent = **co-debugger**: it reads the same observation
  plane via MCP reads (`viz.snapshot`, `bus.recent`,
  `viz.find` become queries against observed state) and
  answers "why did OrderRouter restart three times in the
  last minute" from event history. Shell drawer unchanged.
- **Concurrent human+agent survives**, grounded in a real
  2026 workload: debugging live systems with an agent.
- **Read/write split becomes physics.** v1 is strictly
  read-only — there are no writes to gate, so PermissionGate
  is out of scope until intervention features (pause locus,
  inject message) arrive in some v2; it returns then as the
  same mechanical bus-level gate.
- **Spike artifacts eligible for pull-in:** raylib bindings,
  lotus_viz (scene/layout/animator), heron (tree-sitter),
  pane/layout system, mcp_server, pty/http/sse libs. Nothing
  moves until needed.
- **Render target — DECIDED (2026-07-27): browser over an
  HTTP/SSE backend on fuse, not a native TUI/raylib app.**
  The flower renders in the browser; fuse grows a serve mode
  (JSON snapshot + SSE stream at a capped cadence). Why:
  (a) the backend isn't extra code — the same query surface
  is what the MCP co-debugger reads (`viz.snapshot` et al.
  above), so one API serves both clients; (b) remote
  observation falls out for free (consumer stays shm-attached
  on the observed host, renderer connects over HTTP) — a step
  toward M4 without doing M4; (c) read-only v1 means no
  writes to gate over the network; (d) top-end visual
  expression. A disposable `peek`/top harness remains the
  cheap channel proof (§14 M1). The raylib/lotus_viz pull-in
  path is abandoned for the flower. Frontend language: a
  wasm spike (examples/wasm-flower) proved hale's wasm32
  target hosts a render loop with ~100× frame-budget headroom
  (`@export fn frame` + `_hale_start` persistent arena +
  `@ffi("js")` canvas imports); the M1 frontend may start as
  plain JS for velocity, with Hale→wasm the committed
  direction once fused-state ingestion into wasm is designed.
  The UI frame-caps itself (~30fps, rAF pauses in hidden
  tabs) and the SSE cadence is server-capped — the observer
  must not be the CPU hog in the system it observes. The
  flower is 2D (an instrument, not a scene): precise
  hit-testing, labels, and scrubbing outrank depth; WebGL is
  an acceptable raster backend later, but the scene stays 2D.

## 12. Verification

The ring + mode-mask concurrency (SPSC producers, merging
consumer, observer-written control words) gets the **GenMC
treatment** like every other runtime primitive, modeled in
`hale/verification/`. The observability layer must not be
the one unverified concurrent component in a runtime whose
pitch is model-checked concurrency.

## 13. Ownership

iris does not do compiler or runtime work — the hale team
owns that repo. The split:

**iris team (this repo):**

- The **shm protocol spec** — ring layout, record formats,
  header/mode-mask/control words. iris authors and freezes
  it; it is the contract both sides build against. Lives
  here as `PROTOCOL.md`.
- Consumer library (ring attach, merge, decode), fusion,
  rendering, the flower, any `hale top`-style TUI harness.
- A **synthetic emitter** — a mock process that writes the
  ring format — so the entire consumer/fusion/render stack
  is buildable and testable before any runtime emits.
- An **`observe` library emitter** (ordinary pond lib +
  FFI, no compiler changes): M1 demo apps import it and
  emit their own structural/bus events into a ring.
  Weaker coverage than native probes (no scheduler-level
  events, manual lifecycle emission) but produces real
  running-process data for M1–M2.

**hale team (upstream asks, filed and owned there):**

- Transport seq numbers + per-binding counters (hale#236).
- Native dormant probes in the runtime + `.hale.topo`
  emission (§4–5) — the asks that graduate iris from
  library-emitter coverage to whole-runtime coverage;
  filed when the protocol spec is frozen.
- F.10 (blocks building spike code pulled from `main`).

Sequencing consequence: nothing in M1–M2 waits on upstream.
Native-probe asks are made from a working iris, against a
frozen protocol, with the library emitter as the reference
implementation — a much easier yes.

**Status 2026-08-11 — all three asks shipped.** The bet in
that last paragraph paid: asking from a working iris against
a written protocol, with a reference emitter to diff against,
converted every ask. Native probes landed and then went
through nine rounds of field hardening (handoffs 1–9, 21
findings filed, 19 resolved); transport seq + per-binding
counters shipped with them; the wire header became an opt-in
(`LOTUS_OBS_WIRE=1`) after iris found that observation alone
was silently partitioning fleets. The two-way channel is the
durable asset here — upstream now carries iris's acceptance
check as a gating CI test
(`crates/hale-codegen/tests/obs_fleet_contract.rs`: three
processes over a real multicast group, asserting the
consumer-visible contract), and the contract tests decode
with `protocol.h`'s own shifts, so emitter and consumer
cannot silently disagree about a layout again. That last
change came from handoff-7, where upstream computed the right
locus for four releases and shipped it in the wrong bits
while its own tests stayed green by decoding with the
emitter's layout.

Open on that board as of 2026-08-12:

- **P20** — dynamically spawned (`accept()`-spawned) publishers
  count `CT_PUBLISHED` = 0 on remote-only planes, while their
  messages demonstrably deliver. Filed handoff-9 (2026-07-29)
  with the discriminating table; still unanswered.
- ~~**P22**~~ — the per-binding backpressure cells (`queue_depth`,
  `send_block_ns`, `retries`) were written by no release. Filed
  handoff-10; **shipped upstream 2026-08-12 (hale PR #461)** —
  `lotus_obs_binding_cell_add` / `_gauge` in `lotus_obs.c` populate
  all three (counters-tier, no observer gate). Reading them into
  the snapshot is iris-side work now (INSPECTOR §8 item 5).

**P23** (an intra-subtree publish left no trace — no probes, no
counters, no manifest row) was filed handoff-11 and fixed
upstream the next day. Worth recording how it was found, since
it is a new capability rather than a lucky catch: it came out of
joining a topology artifact cut from source against the live
manifest, which is the first time "what did I declare that the
system has never mentioned?" was an answerable question. Nine
rounds of fleet field-testing had not surfaced it. See
[`INSPECTOR.md`](./INSPECTOR.md).

## 14. Milestones

M0 is not "visualize the codebase." Every milestone is a
running system.

- **M1 — two processes, one unix binding, live fused view.**
  Producer + consumer binaries; one edge with
  throughput/depth/loss/latency; petals for both processes'
  loci; kill the consumer on camera and watch supervision
  absorb it. (A throwaway `hale top` TUI may precede the
  flower to prove the channel end-to-end.) This also makes
  iris the first thing in the ecosystem that exercises
  `bindings{}` for real — the exact gap the external review
  called out.
- **M2 — N processes, mixed unix+udp, loss and backpressure
  made visible.** Deliberately overload a consumer; watch
  depth climb and the supervisor react.
- **M3 — flight recorder + post-mortem dumps.**
- **M4 — cross-machine relay + clock offset.** [DEFERRED
  until M1–M3 hold]

Gate on all of it: **hale F.10**
(`codegen-unknown-cross-seed-type-in-signature`, Rect-vs-
Color registration asymmetry) still blocks `hale build` of
the spike code; anything pulled from `main` hits it.

**Where this stands, 2026-08-11.** M1 is done against the
native runtime rather than the library emitter, and M2's
first half with it: the milestone round read 19 seq-matched
cross-process edges with single-digit-µs means, `lost = 0`,
and a one-send five-listener multicast fan-out matched
exactly per listener — the N-process, mixed-transport,
loss-visible shape. Per-locus attribution and the
supervision/lifecycle overlay are live, and the flower
renders it in the browser over fuse-hl's HTTP/SSE surface
(§11's decided target). The observation plane iris was built
to need now exists and is CI-gated upstream.

**M2's backpressure half is not done — and as of 2026-08-12
it is no longer blocked upstream.** *(Correction 2026-09-04: the
paragraph below was written on 2026-08-11 and was true that day;
the cells shipped the next day in hale PR #461, per
`UPSTREAM-NOTE-2026-09-01.md`. What remains is the demand side:
fuse-hl fuses topic lines only and surfaces no binding line. Kept
as written because the mechanism it describes — supply and demand
missing in the same place — is the lesson.)* Loss is rendered;
depth is not, and it couldn't be. PROTOCOL §6 has reserved
`queue_depth`, `send_block_ns` and `retries` per binding
since v0, and §7 builds the whole "edges are instruments"
claim on them — but the native emitter writes only cells
0/1/2 (sent, delivered, bytes) on a binding line and
0/1/2 (published, delivered, bytes) on a topic line. Cells
3–5 have never been populated by any release. Nothing caught
it because iris never read them either: the demand side and
the supply side were missing in the same place, so the gap
stayed invisible through nine handoff rounds of field
testing.

So "deliberately overload a consumer, watch depth climb and
the supervisor react" is an upstream ask (handoff-10), not
iris plumbing — and the three cells are exactly the numbers
that distinguish *lossy* from *saturated*, which is the
distinction M2 exists to render. Loss we can already show;
the queue filling up ahead of it we cannot. Consumer side is
cheap once the cells are live: they are counter reads on a
line fuse-hl already fuses.

Then **M3.** Its two halves have
different weights: post-mortem dumps are nearly built —
PROTOCOL §12 defines the format, `synth` writes one on
SIGUSR1, and "live attach and forensics are one decode path"
is already true by construction, so what is missing is the
runtime's crash handler upstream and a consumer entry point,
not a design. Scrubbing is the real work: the rings are
history, but nothing retains them consumer-side yet — fuse-hl
drains to fused *totals* and keeps only an 8-entry event
tail, which is the right shape for a live view and the wrong
one for a recorder. A retention buffer and a
scrub-to-timestamp path through the same tables is the M3
build.

## 15. Open questions

- Discovery/session identity: registration-dir vs a real
  deployment identity from placement design (§8).
- Timestamp strategy: cycle-delta resolution, batch
  stamping, per-ring epoch records — needs a measurement
  pass, not a debate.
- Retention window sizing / ring dimensions per scheduler,
  and whether FIREHOSE gets a dedicated ring.
- Whether `hale top` ships as a real subcommand (cheap,
  useful, markets the channel) or stays a test harness.
- Record layout v0: exact field widths above are a sketch;
  freeze after M1 measurement, then version-byte forever.
