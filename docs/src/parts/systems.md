# Part IV: Systems Programming & Runtime Internals

Part IV covers low-level memory management, zero-allocation hot paths, lock-free shared-memory messaging, C and WebAssembly interoperability, and runtime inspection tools.

## Core Systems Capabilities

- **Hierarchical Arena Memory:** Each locus owns an arena (`capacity { pool ...; heap ...; }`) with automatic per-method scratch reclamation and whole-field replacement retirement—delivering deterministic memory bounds without a garbage collector or borrow checker.
- **Zero-Allocation Enforcement Ladder:** Progressively tighten memory guarantees from the default unbounded-allocation escape survey to `@bounded_by(<input>)`, `@hot`, `@budget(alloc_per_call = 0)`, and `@phase_effects(birth: {alloc}, run: {})`.
- **Zero-Copy Shared-Memory Bus (`shm_ring`):** Pass messages across processes at hundreds of thousands of frames per second using lock-free shared-memory rings, `Drain<T>` batch handlers, `ring_layout` foreign rings, and `Topic.write(max)` in-place construction.
- **C FFI & WebAssembly:** Bind C libraries directly via `@ffi("c")` with zero-boilerplate struct/slice mapping, or compile `.hl` programs to `wasm32` with generated `.mjs` browser loaders.
- **Live Observability & Deterministic Replay:** Inspect live locus hierarchies, arena utilization, and bus traffic in real time with `hale iris`, or record and deterministically replay executions with `hale replay`.

## Chapters in Part IV

| Area | Chapters |
|---|---|
| **Memory, Layout & Speed** | [Arena Memory & Lifetimes](../systems/memory.md) · [Zero-Allocation Hot Paths](../systems/performance.md) · [Collection Forms Internals](../systems/forms.md) · [Zero-Copy Shared-Memory Bus](../systems/zero-copy-bus.md) · [Execution Modes (`bulk`, `harmonic`, `resolution`)](../systems/modes.md) |
| **FFI & Targets** | [C Interoperability (`@ffi("c")`)](../systems/binding-c.md) · [WebAssembly & Browser Targets](../systems/webassembly.md) |
| **Operations, Inspection & Replay** | [Production Operations & Runtime Knobs](../systems/operations.md) · [Iris: Live Topology & Arena Inspector](../systems/iris.md) · [Deterministic Record & Replay](../systems/replay.md) |
