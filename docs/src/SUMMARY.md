# Summary

[Introduction](./introduction.md)
[Install](./getting-started/install.md)
[Quickstart & Toolchain](./getting-started/first-run.md)
[Tutorial: Build a Job Queue](./tutorial/job-queue.md)

# Part I: Language & Standard Library

- [Language Overview](./parts/language.md)
- [Values, Types & Variables](./basics/values.md)
- [Numbers, Money & Time](./basics/math.md)
- [Units of Measure](./units.md)
- [Functions](./basics/functions.md)
- [Control Flow & Pattern Matching](./basics/control-flow.md)
- [Strings & Bytes](./basics/strings.md)
- [Error Handling (`fallible`)](./basics/fallible.md)
- [Program Structure & `main`](./basics/first-program.md)
- [Introduction to Loci](./everyday/locus-gently.md)
- [Records, Enums & Set Types](./everyday/records.md)
- [Collections (`@form`)](./everyday/collections.md)
- [Filesystem I/O](./everyday/files.md)
- [JSON Encoding & Decoding](./everyday/json.md)
- [HTTP Servers & Clients](./everyday/http.md)
- [TCP, UDP, Unix Sockets & TLS](./everyday/sockets.md)
- [Cryptography, Hashing & Randomness](./everyday/crypto.md)
- [CLI Arguments & Environment](./everyday/cli-config.md)
- [Structured Logging](./everyday/logging.md)
- [Metrics](./everyday/metrics.md)
- [Observability & Probes](./everyday/observing.md)
- [Testing & Benchmarking](./everyday/testing.md)

# Part II: Concurrency, Services & APIs

- [The Locus Model](./parts/locus-model.md)
- [Locus Lifecycle](./services/lifecycle.md)
- [The Typed Message Bus](./services/bus.md)
- [Concurrency & Thread Placement](./services/concurrency.md)
- [Ownership, Children & Flow Reclamation](./services/parents-children.md)
- [Supervision & Structural Failure](./services/failure.md)
- [Interfaces, Contracts & Perspectives](./services/perspectives.md)
- [Seams & Typed Pipelines](./services/seams.md)
- [Repositories (`@repo`)](./services/repositories.md)
- [Service Composition Patterns](./services/patterns.md)
- [Tutorial: Build a Typed API](./services/build-an-api.md)
- [API Surfaces, RPC, REST, MCP & gRPC](./services/api.md)
- [API Wire Schemas & Compatibility](./services/api-messages.md)
- [Multi-Binary Systems, Fleets & Deployment](./services/multi-binary.md)
- [The Application Model](./the-model.md)

# Part III: Static Verification & Architecture Laws

- [Compile-Time Guarantees](./parts/what-must-hold.md)
- [Effect Contracts & Resource Budgets](./effects.md)
- [Architectural Claims & Topology Proofs](./claims.md)
- [Constitutions & Multi-Environment Laws](./constitutions.md)

# Part IV: Systems Programming & Runtime Internals

- [Systems Control](./parts/systems.md)
- [Arena Memory & Lifetimes](./systems/memory.md)
- [Zero-Allocation Hot Paths](./systems/performance.md)
- [Collection Forms Internals](./systems/forms.md)
- [Zero-Copy Shared-Memory Bus](./systems/zero-copy-bus.md)
- [Execution Modes (`bulk`, `harmonic`, `resolution`)](./systems/modes.md)
- [C Interoperability (`@ffi("c")`)](./systems/binding-c.md)
- [WebAssembly & Browser Targets](./systems/webassembly.md)
- [Production Operations & Runtime Knobs](./systems/operations.md)
- [Iris: Live Topology & Arena Inspector](./systems/iris.md)
- [Deterministic Record & Replay](./systems/replay.md)

# Part V: DNA (Autonomous & Governed Applications)

- [DNA Overview](./parts/organism.md)
- [Architecture of a DNA System](./dna/index.md)
- [DNA Quickstart](./dna/getting-started.md)
- [End-to-End Task Lifecycle](./dna/workflow.md)
- [Record & State Store (PostgreSQL & Git)](./dna/memory.md)
- [Workflow Coordinator (`spine`)](./dna/spine.md)
- [Event Broker (`nerves` / NATS JetStream)](./dna/nerves.md)
- [Application Supervisor & Deployment (`heart` & `body`)](./dna/heart.md)
- [Telemetry, Health Reflexes & Upstream Watches (`senses`)](./dna/senses.md)
- [Secrets, Vault & Violation Absorption (`skin`)](./dna/skin.md)
- [Worker Agents, Sandboxed Tools & LLM Routing (`legs`)](./dna/legs.md)
- [Recurring Schedules](./dna/schedules.md)
- [Edge API, Web UI, Board & Reviews (`head` & `face`)](./dna/head.md)
- [Governance, Positions, Grants & Packs](./dna/shaping.md)
- [The `dna::Dna` Locus & Custom Topologies](./dna/dna.md)
- [DNA Engineering Standard](./dna/standard.md)
- [DNA Troubleshooting](./dna/troubleshooting.md)
- [DNA CLI & Schema Reference](./dna/reference.md)

# Part VI: Multi-Record Control Plane (Habitat)

- [Habitat Overview](./parts/habitat.md)
- [Habitat Architecture & CLI (`hale habitat`)](./habitat.md)

---

- [Language & CLI Reference](./reference.md)
- [Package Management & `pond` Catalog](./libraries.md)
- [Verification & Model-Checking Reference](./verification.md)
- [Formal Design Model (`%DESIGN v1`)](./the-design.md)
