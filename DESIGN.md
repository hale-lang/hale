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
  binding*, not a static visualization (§12).
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

## 10. The app: what survives from the spike

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

## 11. Verification

The ring + mode-mask concurrency (SPSC producers, merging
consumer, observer-written control words) gets the **GenMC
treatment** like every other runtime primitive, modeled in
`hale/verification/`. The observability layer must not be
the one unverified concurrent component in a runtime whose
pitch is model-checked concurrency.

## 12. Ownership

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

## 13. Milestones

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

## 14. Open questions

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
