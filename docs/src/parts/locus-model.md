# Part II: Concurrency, Services & APIs

Part II covers building concurrent, long-running services, API servers, and multi-binary distributed systems using the **locus model**.

## Core Concepts

- **Lifecycle (`birth` $\to$ `run` $\to$ `drain` $\to$ `dissolve`):** Every running locus follows a deterministic four-phase lifecycle managed by the runtime, with teardown cascading depth-first through child subtrees.
- **Typed Message Bus (`topic` & `bus`):** Loci communicate asynchronously by publishing and subscribing to typed `topic` declarations. Keyed topics (`keyed_by`) route traffic directly to the owning child or worker replica.
- **Thread Placement (`placement { }`):** Loci are single-threaded inside their own methods. Concurrency is configured in `main locus` by assigning children to `cooperative(pool = ...)` (with optional `where async_io` epoll/kqueue coroutines) or `pinned(core = N, replicas = K)`.
- **Supervision (`closure` & `on_failure`):** Loci declare state invariants (`closure`) and parents define recovery policies (`restart`, `restart_in_place`, `bubble`).
- **Seams & Repositories (`seam` & `@repo(T)`):** Linear typed pipelines (`seam`) route union message sets (`type Inbound = A | B;`) with compile-time exhaustiveness checks, while `@repo(T)` standardizes keyed CRUD persistence and state effects.
- **Typed API Surfaces (`api` & `@rpc`):** Declare an `api` contract once and serve it over HTTP RPC, REST (`http::Rest`), Unix sockets (`unix::Rpc`), gRPC (`grpc::Rpc`), MCP (`mcp::Rpc`), or WebSockets (`ws::Hub`), or consume external OpenAPI/MCP specs with `hale api import`.
- **Multi-Binary Fleets (`bindings` & `fleet locus`):** Rebind topics across processes (`unix`, `shm_ring`, broker adapters) and verify multi-binary topologies in source with `fleet locus` and `hale deploy render`.

## Chapters in Part II

| Topic | Chapters |
|---|---|
| **Lifecycle, Bus & Concurrency** | [Locus Lifecycle](../services/lifecycle.md) · [The Typed Message Bus](../services/bus.md) · [Concurrency & Thread Placement](../services/concurrency.md) · [Ownership, Children & Flow Reclamation](../services/parents-children.md) · [Supervision & Structural Failure](../services/failure.md) |
| **Boundaries & Data Access** | [Interfaces, Contracts & Perspectives](../services/perspectives.md) · [Seams & Typed Pipelines](../services/seams.md) · [Repositories (`@repo`)](../services/repositories.md) · [Service Composition Patterns](../services/patterns.md) |
| **APIs & Distributed Systems** | [Tutorial: Build a Typed API](../services/build-an-api.md) · [API Surfaces, RPC, REST, MCP & gRPC](../services/api.md) · [API Wire Schemas & Compatibility](../services/api-messages.md) · [Multi-Binary Systems, Fleets & Deployment](../services/multi-binary.md) · [The Application Model](../the-model.md) |
