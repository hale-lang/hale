# moa — the Memory-Owner Architecture substrate

> The substrate that apps build on. Path prefix `moa::*`. Parallel to
> `std::*`; not part of stdlib; conceptually one layer below
> application code and one layer above the language itself.

## What lives here

| File | Purpose |
|---|---|
| `MOA.md` | The architecture doc — four properties, recording vs projection, ingest classification, the five-step authoring process. Read this before composing a stateful app. |
| `subjects.md` | Standardized bus subject naming conventions. Apps that follow them interoperate by construction. Informative; not enforced at v1. |
| `types.ap` | Substrate payload types — `LocusId`, `BraidId`, `Tick`, `RuntimeEvent`. Compiled into every Aperio binary; resolves under the `moa::*` prefix. |

## What gets compiled

Files ending in `.ap` are bundled into every emitted binary, the same
way stdlib's `.ap` files are bundled. User code references them under
the `moa::*` path prefix — e.g., `moa::RuntimeEvent`. The compiler's
path-resolution machinery in `crates/aperio-codegen/src/codegen.rs`
maintains a `MOA_PATH_RENAMES` table parallel to `STDLIB_PATH_RENAMES`;
both are searched at typecheck and codegen.

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

A new substrate type lands by:

1. Adding the `type` declaration to `types.ap` (or a new `.ap` file
   if the concern is genuinely separate).
2. Adding the entry to `MOA_PATH_RENAMES` in
   `crates/aperio-codegen/src/codegen.rs`, mapping `moa::<Name>` to
   the mangled internal `__Moa<Name>`.
3. Adding the file to `MOA_AP_SOURCE`'s `concat!(include_str!(...))`
   block in the same file (only if a new `.ap` file was created).
4. Re-running `cargo build -p aperio-cli` — the build.rs hash check
   will pick up the change automatically.

## Cross-references

- `MOA.md` — the foundational statement of the architectural pattern
- `subjects.md` — bus subject conventions
- `notes/aperio-types-vs-loci.md` — the axiom MOA builds on
- `notes/agent-onboarding/aperio-styleguide.md` — pattern catalog and
  composition rules
- `spec/design-rationale.md` F.22 — capacity slots, which MOA
  depends on for storage discipline
