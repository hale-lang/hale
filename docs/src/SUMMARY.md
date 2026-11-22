# Summary

# Getting Started

- [What Is Hale?](./introduction.md)
- [Install](./getting-started/install.md)
- [Hello World & the Toolchain](./getting-started/first-run.md)
- [Your First Program](./basics/first-program.md)
- [Tutorial: Build a Job Queue](./tutorial/job-queue.md)
- [Running Without a Habitat](./getting-started/without-a-habitat.md)

# Part I: The Language

- [Overview](./parts/language.md)
- [Values, Types & Variables](./basics/values.md)
- [Functions](./basics/functions.md)
- [Control Flow & Pattern Matching](./basics/control-flow.md)
- [Records, Enums & Set Types](./everyday/records.md)
- [Strings & Bytes](./basics/strings.md)
- [Numbers, Money & Time](./basics/math.md)
- [Units of Measure](./units.md)
- [Error Handling (`fallible`)](./basics/fallible.md)
- [Collections (`@form`)](./everyday/collections.md)
- [Generics](./everyday/generics.md)
- [Introduction to Loci](./everyday/locus-gently.md)
- [Testing & Benchmarking](./everyday/testing.md)

# Part II: The Standard Library

- [Overview](./parts/stdlib.md)
- [CLI Arguments & Environment](./everyday/cli-config.md)
- [Filesystem I/O](./everyday/files.md)
- [JSON Encoding & Decoding](./everyday/json.md)
- [HTTP Servers & Clients](./everyday/http.md)
- [TCP, UDP, Unix Sockets & TLS](./everyday/sockets.md)
- [Cryptography, Hashing & Randomness](./everyday/crypto.md)
- [Structured Logging](./everyday/logging.md)
- [Metrics](./everyday/metrics.md)
- [Observability & Probes](./everyday/observing.md)

# Part III: Services & Concurrency

- [Overview](./parts/locus-model.md)
- [Locus Lifecycle](./services/lifecycle.md)
- [The Typed Message Bus](./services/bus.md)
- [Concurrency & Thread Placement](./services/concurrency.md)
- [Ownership, Children & Flow Reclamation](./services/parents-children.md)
- [Supervision & Structural Failure](./services/failure.md)
- [Interfaces & Contracts](./services/interfaces.md)
- [Seams & Typed Pipelines](./services/seams.md)
- [Service Composition Patterns](./services/patterns.md)
- [Repositories (`@repo`)](./services/repositories.md)
- [Tutorial: Build a Typed API](./services/build-an-api.md)
- [API Surfaces, RPC, REST, MCP & gRPC](./services/api.md)
- [API Wire Schemas & Compatibility](./services/api-messages.md)
- [Multi-Binary Systems, Fleets & Deployment](./services/multi-binary.md)

# Part IV: Verification & Architecture Laws

- [Overview](./parts/what-must-hold.md)
- [The Application Model](./the-model.md)
- [Effect Contracts & Resource Budgets](./effects.md)
- [Architectural Claims & Topology Proofs](./claims.md)
- [Constitutions & Multi-Environment Laws](./constitutions.md)

# Part V: Systems & Runtime Internals

- [Overview](./parts/systems.md)
- [Arena Memory & Lifetimes](./systems/memory.md)
- [Zero-Allocation Hot Paths](./systems/performance.md)
- [Collection Forms Internals](./systems/forms.md)
- [Zero-Copy Shared-Memory Bus](./systems/zero-copy-bus.md)
- [C Interoperability (`@ffi("c")`)](./systems/binding-c.md)
- [WebAssembly & Browser Targets](./systems/webassembly.md)
- [Production Operations & Runtime Knobs](./systems/operations.md)
- [Iris: Live Topology & Arena Inspector](./systems/iris.md)
- [Deterministic Record & Replay](./systems/replay.md)

# Part VI: DNA (Autonomous & Governed Applications)

- [Overview](./parts/organism.md)
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

# Part VII: The Habitat

- [Overview](./parts/habitat.md)
- [Habitat Architecture & CLI (`hale habitat`)](./habitat.md)

# Reference

- [Language & CLI Reference](./reference.md)
- [Stability & Compatibility](./stability.md)
- [Package Management & `pond` Catalog](./libraries.md)
- [Verification & Model-Checking Reference](./verification.md)
- [Formal Design Model (`%DESIGN v1`)](./the-design.md)
