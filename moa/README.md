# moa — the Memory-Owner Architecture substrate

> The substrate that apps build on. Path prefix `moa::*`. Parallel to
> `std::*`; not part of stdlib; conceptually one layer below
> application code and one layer above the language itself.

## What lives here

### Documentation

| File | Purpose |
|---|---|
| `MOA.md` | The architecture doc — four properties, recording vs projection, ingest classification, the five-step authoring process. Read this before composing a stateful app. |
| `subjects.md` | Standardized bus subject naming conventions. Apps that follow them interoperate by construction. Informative; not enforced at v1. |

### Library code

| File | Status | Purpose |
|---|---|---|
| `types.ap` | **Live** (bundled) | Substrate payload types — `LocusId`, `BraidId`, `Tick`, `RuntimeEvent`. |
| `snapshotable.ap` | **Pending wiring** | F.20 structural interface `Snapshotable`; codifies the broadcast + on-demand snapshot pattern. |
| `clock.ap` | **Pending wiring** | `Clock` self-contained memory-owner; publishes `clock.tick` at a configured cadence. |
| `recorder.ap` | **Pending wiring** | `Recorder` recording memory-owner; subscribes to `runtime.event.**`, saves into a heap slot. |
| `replayer.ap` | **Pending wiring** | `Replayer` skeleton; reads recorded events and re-emits on `runtime.event.replay.**`. Body deferred to v1.x. |

## What gets compiled

Files marked **Live** above are bundled into every emitted binary, the
same way stdlib's `.ap` files are bundled. User code references them
under the `moa::*` path prefix — e.g., `moa::RuntimeEvent`. The
compiler's path-resolution machinery in
`crates/aperio-codegen/src/codegen.rs` maintains a `MOA_PATH_RENAMES`
table parallel to `STDLIB_PATH_RENAMES`; both are searched at
typecheck and codegen.

Files marked **Pending wiring** are valid Aperio source captured in
the repo but not yet bundled. Each file's header carries the two
one-line edits a compiler-session contribution needs to land:
`MOA_AP_SOURCE` concat-block entry + `MOA_PATH_RENAMES` entry. Once
both edits ship for a given file, its declarations become reachable
under `moa::<Name>` paths.

Markdown files (`MOA.md`, `subjects.md`, this README) are
documentation only and are not bundled into binaries.

## What does NOT live here

- **Application-domain types** (Order, Trade, Account; Tower, Flower,
  Braid). Those live in `lib/<domain>/` when shared across MOA apps,
  or in the app's own seed when domain-local.
- **Stdlib namespaces** (`std::io`, `std::log`, `std::http`). Those
  live at `crates/aperio-codegen/runtime/stdlib/*.ap` and resolve
  under `std::*`.
- **Language primitives** (`type`, `locus`, `capacity`, `bus`,
  `contract`). Those are the substrate the substrate builds on; they
  are defined in `spec/`.

## How to add to it

A new substrate declaration lands by:

1. **Aperio source** — add the `type` / `locus` / `interface`
   declaration to an existing `.ap` file under `moa/` if the concern
   fits there, or create a new file (one file per concern is the
   preferred shape). Mangle internal names with the `__Moa` prefix
   to match the existing entries.
2. **Bundle entry** — add the file to `MOA_AP_SOURCE`'s
   `concat!(include_str!(...))` block in
   `crates/aperio-codegen/src/codegen.rs`. Skip if you added to an
   already-bundled file.
3. **Path rename** — add an entry to `MOA_PATH_RENAMES` in the same
   file, mapping `moa::<Name>` to the mangled internal `__Moa<Name>`.
4. **Verify** — `cargo build -p aperio-cli` rebuilds with the new
   bundle; the build.rs hash check picks up file changes
   automatically. Add a coverage test under
   `crates/aperio-codegen/tests/` mirroring
   `moa_path_resolution.rs`'s shape.

## v0 wiring backlog

The following library files are present but **not yet bundled** —
each carries its specific two-line wiring instruction in its file
header. A compiler-session contribution that wires them simply adds
the indicated entries and runs the existing
`moa_path_resolution.rs`-style smoke test:

- `snapshotable.ap` → `moa::Snapshotable` (interface)
- `clock.ap` → `moa::Clock` (locus)
- `recorder.ap` → `moa::Recorder` (locus)
- `replayer.ap` → `moa::Replayer` (locus; skeleton body, full
  body lands in v1.x with heap-slot iteration or serialized event
  blob support)

## Cross-references

- `MOA.md` — the foundational statement of the architectural pattern
- `subjects.md` — bus subject conventions
- `notes/aperio-types-vs-loci.md` — the axiom MOA builds on
- `spec/styleguide.md` — pattern catalog and composition rules
- `spec/design-rationale.md` F.22 — capacity slots, which MOA
  depends on for storage discipline
