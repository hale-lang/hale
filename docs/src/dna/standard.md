# DNA Architectural Engineering Standard

This document defines the architectural engineering standard for DNA organisms and the DNA runtime itself. While individual Hale language features (surfaces, topics, effect classes, units, constitutional claims) are opt-in for general-purpose programs, a distributed DNA organism requires uniform conventions across every process so the compiler can verify cross-process contracts, authority boundaries, memory bounds, and units end to end.

> [!NOTE]
> **Enforced Today vs. Roadmap Goals:** Each rule below states its engineering requirement, what pattern it replaces, how the toolchain enforces it, and its current status in the DNA codebase (**Enforced Today** vs. **Roadmap Goal**).

## Core Principle

**Every boundary crossing must be declared in source code and statically checked by the compiler.** In DNA, boundaries include process boundaries, locus ownership boundaries, authority/principal transitions, physical/monetary units of measure, and secret material.

## Summary Matrix: Today vs. Roadmap

| Domain | Rule | Status in `dna/` Today |
| --- | --- | --- |
| **Structure** | Single locus tree per program (`accept` / `dissolve` lifecycle) | **Enforced Today** (`dna::Dna` tower; workflows/tasks/steps are accepted children) |
| **Structure** | Thread and pool placement via `placement { }` on `main locus` | **Enforced Today** (host, head, reflexes, and generated `dna/org`) |
| **Structure** | Multi-process composition via `fleet locus` (`hale fleet check`) | **Roadmap Goal** (bindings currently duplicated across host and `dna/org`) |
| **Boundaries** | Inter-process RPC via `api` surfaces (`api::serve` + `hale api client`) | **Partial** (`HeadCommands` mutations use `api`; head reads and face client are a **Roadmap Goal**) |
| **Boundaries** | External system calls via consumed `api::client` surfaces | **Enforced Today** (`spec/api.md` § Calling a surface) |
| **Boundaries** | Asynchronous events via typed `topic` declarations and `bindings { }` | **Enforced Today** in `dna/core/topics.hl` (~50 topics); typed heart events are a **Roadmap Goal** |
| **Boundaries** | Committed OpenAPI/MCP/Schema/client artifacts verified by tests | **Enforced Today** in the reference todo application |
| **Boundaries** | Surface and interface failures declared with `fallible(E)` | **Roadmap Goal** (head commands currently return `{ok, code}`; awaits fallible interface methods) |
| **Data** | Record rows defined as typed structs with `json` tags | **Roadmap Goal** (rows are currently built and parsed as raw JSON strings) |
| **Data** | Closed string sets represented as `enum` types | **Roadmap Goal** |
| **Data** | Currency as `Decimal`, time as `Time`/`Duration`, measures as `quantity` | **Roadmap Goal** (spend is currently integer micro-dollars; timestamps are `Int`) |
| **Data** | Boundary lists typed as `bounded[T; N]` or collection loci | **Roadmap Goal** (currently passed as newline-delimited strings) |
| **Authority** | Shared base constitution (`[claims] base`) across all programs | **Partial** (`dna/org/law.hl` is checked under `no_base`; host/head/reflexes/face are a **Roadmap Goal**) |
| **Authority** | Direct OS syscalls attributed via `require attributed(all syscall)` | **Partial** (`dna/core/types.hl` annotates core carriers; host/head binaries are a **Roadmap Goal**) |
| **Authority** | Record projections marked `@no_syscall` and `@deterministic` | **Partial** (1 projection annotated today; remaining projections are a **Roadmap Goal**) |
| **Authority** | Secrets confined to `@sealed` loci via `std::secret` | **Enforced Today** for model keys and NATS passwords; provisioning/DB/forge tokens are a **Roadmap Goal** |
| **Authority** | Principal tracking via `runs_under(locus, principal)` | **Roadmap Goal** |
| **Resources** | All long-lived allocations proven with `@bounded_by(<input>)` | **Enforced Today** across all of `dna/` (270 functions annotated, zero `@unbounded`, gated by `hale verify`) |
| **Resources** | Health/readiness/capacity served via `std::observe::Probes` | **Roadmap Goal** for `dna/` binaries (supported by the runtime today) |
| **Quality** | Verification pipeline reads structured JSON output from `hale` tools | **Partial** (`dna/core/verification.hl` runs the suite; `hale test --json` and facet topology diffing are a **Roadmap Goal**) |
| **Quality** | `hale verify` zero-finding CI gate | **Enforced Today** on `dna/core`, `dna/host`, `dna/host/verb`, `dna/operations`, and `dna/api` |
| **Quality** | API surfaces tested via `std::api::test::Rpc` and models via `FakeModel` / `RecordedModel` | **Enforced Today** |
| **Observability** | Unified metrics via `std::metrics` and structured logs via `std::log` | **Roadmap Goal** (`standard` nursery pack proposes structured logging as law) |

---

## 1. Structure: One Locus Tree and One Fleet Plan

### Single Locus Tree per Process
Every program is structured as a single locus hierarchy where child components are declared as `params`, dynamic per-request or per-execution units are spawned via `accept` and reclaimed on `dissolve`, and structural failures route upward to the parent's `on_failure` handler (`spec/semantics.md` § Locus instantiation).
- **Replaces:** Unsupervised background threads, side-table registries, or ad-hoc process trees.
- **Status (`dna/`):** **Enforced Today.** The organization runtime is a single locus tower rooted at `dna::Dna` (`dna/core/assembly.hl`), and workflow executions, tasks, and steps are accepted children.

### Placement on `main locus`
Thread pools, core pinning, and worker replicas are declared exclusively in the `placement { }` block on the `main locus`—never via ad-hoc threading calls or CLI flags.
- **Status (`dna/`):** **Enforced Today** across the host, head, reflexes, and generated `dna/org`.

### Multi-Process Composition via `fleet locus`
All processes of an organism (host, organization, head, application heart, and reflexes) should be declared as instances inside a single `fleet locus` whose `bindings` wire inter-process topics, verified by `hale fleet check` and rendered by deployment backends.
- **Replaces:** Hand-maintained Docker Compose files, PID files, and duplicated port/topic bindings across entrypoints.
- **Enforcement:** `hale fleet check` during candidate verification.
- **Status (`dna/`):** **Roadmap Goal.** Bindings are currently maintained separately across the host, `dna/org`, and reference organism.

---

## 2. Boundaries: Typed Surfaces and Topics

### Inter-Process RPC via `api` Surfaces
Every synchronous call between processes is declared on an `api` surface, served via `api::serve` over the configured transport (`unix::Rpc`, `http::Rpc`, `mcp::Rpc`) with role guards on each operation, and called through generated clients (`hale api client`) verified in CI.
- **Replaces:** Hand-written HTTP routers, manual JSON serialization, and hand-rolled HTTP client wrappers.
- **Status (`dna/`):** **Enforced Today** for all mutations (`HeadCommands`, served over Unix socket and HTTP with a digest-verified client in `dna/core/legs/head_commands`). **Roadmap Goal** for head HTTP read routes, project head operations, and the browser face client.

### External Calls via Consumed `api::client` Surfaces
Calls to external services use an `api NAME from openapi|mcp` declaration imported via `hale api import` (or a peer Hale surface) and invoked through a typed `api::client(NAME, http::Client { … } | mcp::Client { … })` parameter backed by vault credentials.
- **Why:** Provides five typed transport outcomes, drift detection (`hale api drift`), compile-time effect tracking (`client(NAME)`), and automatic agent tool schema export (`hale api export --tools-for`).
- **Status (`dna/`):** **Enforced Today** (`spec/api.md` § Calling a surface).

### Events via Declared Topics and Adapters
Asynchronous events between components use typed `topic` declarations bound on `main locus` via `bindings { T: Adapter }`, using `keyed_by` subscriptions to filter traffic at the bus layer.
- **Status (`dna/`):** **Enforced Today** inside the organism (~50 topics in `dna/core/topics.hl`). **Roadmap Goal** for application heart events.

### Generated Machine Artifacts and `fallible(E)` Handlers
- **Machine artifacts:** OpenAPI, MCP, `.description.json`, JSON Schema, and TypeScript/JS client files are generated from the `api` surface, committed to the repository, and kept current by a test (**Enforced Today** in the reference todo app).
- **Declared handler failures:** Surface handlers and interface methods that can fail should declare `fallible(E)` rather than returning `{ok: Bool, code: String}` envelopes (**Roadmap Goal**; awaits `fallible` support on interface methods).

---

## 3. Data: Explicit Types, Enums, and Units

- **Typed Record Rows:** Every row kind in the record (`record_rows` and `ledger_rows`) should be a Hale `type` with `json` field tags, serialized via its generated codec and mapped by type in `dna/core/routing.hl`. (**Roadmap Goal:** rows are currently constructed and parsed as raw JSON strings.)
- **Closed Sets as Enums:** Fixed state machines and category sets should be `enum` declarations rather than free-form `String` fields. (**Roadmap Goal.**)
- **Quantities and Units:** Monetary values must use `Decimal` with an explicit currency; timestamps and intervals must use `Time` and `Duration`; and domain measures must use `quantity Int in <unit>` (`spec/units.md`). (**Roadmap Goal:** spend currently uses integer micro-dollars and timestamps use integer milliseconds/seconds.)
- **Collections over Delimited Strings:** Lists crossing boundaries should use `bounded[T; N]` (when fixed-cap) or collection loci (`@form(vec)`) rather than newline-delimited strings. (**Roadmap Goal.**)

---

## 4. Authority: Constitutions, Attributed Effects, and `@sealed` Secrets

### Shared Base Constitution
Every program in an organism should adopt a `constitution` in `hale.toml` over a shared base (`[claims] base = "…"`), using `dna/org/law.hl` as the foundational law.
- **Status (`dna/`):** **Partial.** Generated `dna/org` enforces `constitution Org` (`dna/org/law.hl`), while extending a shared base across the host, head, reflexes, and face is a **Roadmap Goal**.

### Attributed OS Syscalls (`require attributed(all syscall)`)
Every function that directly invokes a built-in `syscall` effect must carry a domain-specific `@effects(is: { … })` annotation so constitutional claims (`forbid reaches(…)`) can constrain *why* a boundary is crossed, not just *where*.
- **Status (`dna/`):** **Partial.** Core effect carriers in `dna/core/types.hl` are annotated; enforcing `require attributed(all syscall)` across host and head entrypoints is a **Roadmap Goal**.

### Pure Record Projections (`@no_syscall`, `@deterministic`)
Functions that project state from record rows must be marked `@no_syscall` (and `@deterministic` where applicable) to guarantee deterministic replay.
- **Status (`dna/`):** **Partial** (1 projection annotated today; remaining projections are a **Roadmap Goal**).

### Secret Confinement (`@sealed` and `std::secret`)
Secrets are held exclusively inside `@sealed` loci via `std::secret` and revealed only inside the expression that writes to the wire—never read from environment variables into `String` bindings or passed on child process command lines.
- **Status (`dna/`):** **Enforced Today** for all LLM API keys (`HostedCredential`) and NATS broker passwords. Extending `std::secret` to vault provisioning writes, Postgres role passwords, and `FORGE_TOKEN` is a **Roadmap Goal**.

### Principal Tracking (`runs_under`)
Associating loci with runtime principals (`runs_under(locus, principal)`) to project backend ACLs (Postgres roles, NATS accounts, Vault policies) directly from the compiler graph is a **Roadmap Goal** ([The habitat, as designed](../habitat.md)).

---

## 5. Resources: Proven Memory Bounds and Health Probes

### Memory-Bound Proofs (`@bounded_by`)
Because DNA daemons run continuously for months over arena-allocated loci, every growing allocation flagged by the compiler's memory-bound survey must be either refactored to run in a per-iteration helper method (whose scratch arena reclaims on return) or explicitly proven bounded with `@bounded_by(<input>)`:
- **Production code:** Names the finite input that bounds the allocation—such as a journal slice loaded once (`@bounded_by(self.j)`), a parsed document (`@bounded_by(document)`), or CLI arguments (`@bounded_by(argv)`).
- **Tests:** Because test drivers spawn loci with handlers, the survey treats them as long-lived; test helpers annotate their bound as `@bounded_by(repo)`, `@bounded_by(tries)`, or `@bounded_by(script)`.
- **No silent `@unbounded`:** `@unbounded` is reserved strictly for unanalyzed growth accompanied by a `// GH #<issue>` tracking comment. Unused annotations are rejected by `scripts/unbounded-unused.sh`.
- **Status (`dna/`):** **Enforced Today** across all of `dna/`. Every `@unbounded` annotation has been eliminated, 270 functions carry verified `@bounded_by` proofs, `scripts/unbounded-unused.sh` reports zero unused annotations, and `hale verify` gates CI.

### Runtime Health and Capacity Probes (`std::observe::Probes`)
Long-lived programs attach `std::observe::Probes { port }` on their `main locus` to expose liveness, readiness, and arena/queue capacity from runtime telemetry ([Observing a program](../everyday/observing.md)).
- **Status (`dna/`):** **Roadmap Goal** to wire `std::observe::Probes` into all `dna/` daemons and map them to deployment platform health checks.

---

## 6. Quality Gates, Observability, and Project Layout

### Automated Toolchain Gates (`hale verify`)
Candidate verification (`dna/core/verification.hl`) runs `hale fmt --check`, `hale check`, `hale verify`, `hale test`, `hale fleet check`, and `hale model diff`.
- **CI Gate:** `dna_core_host_operations_api_verify_clean` (`crates/hale-cli/tests/dna_native_suite.rs`) enforces zero `hale verify` findings across `dna/core`, `dna/host`, `dna/host/verb`, `dna/operations`, and `dna/api` on every PR and merge.
- **Surface & Model Testing:** API surfaces are tested through `std::api::test::Rpc` rather than synthetic contexts, and automated tests use `FakeModel` or `RecordedModel` tapes—never live LLM network calls (**Enforced Today**).

### Unified Metrics and Structured Logging
- **Metrics (`std::metrics`):** Series should be declared alongside the code that updates them and exposed on a single endpoint scraped by both the organism's senses and the deployment platform (**Roadmap Goal**).
- **Structured Logging (`std::log`):** Every log line emitted by a running component must be a structured record (timestamp, level, message, and key-value fields) via `std::log` rather than unstructured `print` / `println` calls. The built-in `standard` nursery pack (`hale dna pack apply standard`) proposes this rule as organizational law (**Roadmap Goal** for compiler stdio effect enforcement).

### Standard Directory Layout
- **Seeds:** Each component is a self-contained seed directory with its own `hale.toml` and `*_test.hl` / `tests/` suite.
- **Contracts (`spec/`):** Generated OpenAPI, MCP, and `.description.json` contracts are committed under `spec/`.
- **Organization (`dna/org/` vs. `vendor/dna/`):** Project-owned governance and topology live in `dna/org/` and change only through reviewed mutations; `vendor/dna/` is managed exclusively by `hale dna upgrade`.
- **Configuration:** Passed as typed `params` on `main locus` and bound per environment in `hale.toml`, reading environment variables only at the process entrypoint edge.
