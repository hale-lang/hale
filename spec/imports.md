# Imports

Cross-seed imports are how an Aperio program reaches into a
vendored library directory. v1 ships source-level imports only —
no package registry, no fetch, no versioning, no lockfile. A
library is a directory of `.ap` files copied into the project's
tree (typically `lib/`); the importer references it by literal
path with a required alias.

Within one seed (one directory) the per-directory model from
F.19 already gives shared top-level scope across files; imports
exist to cross *between* seeds. The two compose: an importer
seed sees its own files' decls at bare name, and an imported
seed's decls at `alias::Name`.

See `spec/design-rationale.md` F.25 for the design rationale,
F.19 for the per-directory predecessor. See
`notes/v1.x-IMPORT-handoff.md` for the milestone history.

## Syntax

```
import "<path>" as <alias>;
```

- **`<path>`** is a string literal naming the library to import.
  The path is resolved per "Resolution order" below; both
  entry-relative paths and workspace-rooted paths are honored.
- **`<alias>`** is an identifier naming the namespace at the
  import site. It is **required** — bare `import "<path>";` is
  a parse error. Cross-seed references in the importing seed
  read as `<alias>::Name`.

The alias-required rule is the same forcing-function discipline
v1.x-3 enforces for `: projection recognition` (no default
sub-mode) and v1.x-FORM-2 enforces for the two-channel rule
(locus methods can't declare `fallible(E)`): the user names the
commitment at the surface so a downstream reader doesn't have
to reconstruct the namespace from the path.

Imports appear at the top of a file, before any top-level
declaration. Multi-file seeds may declare imports in any file;
the build merges every file's imports into one set against the
seed's directory + workspace root.

### Examples

```aperio
import "lib/moa" as moa;
import "../shared-helpers" as helpers;
import "lib/grease-finance" as grease;

fn main() {
    let id = moa::LocusId { name: "BookL", path: "apps/x.ap" };
    let h = helpers::Formatter { };
}
```

## Resolution order

The compiler tries three locations in order; the first hit wins:

1. **`<importer-dir>/<path>.ap`** — single-file library.
2. **`<importer-dir>/<path>/`** — directory bundle. Every `.ap`
   file in the directory is one library seed (per F.19's
   per-directory model). File order in the merged bundle is
   alphabetical; resolution within the seed is order-free.
3. **`<workspace-root>/<path>/`** — workspace fallback. The
   workspace root is the first directory found by walking
   upward from the importer that contains a `Cargo.toml`. Used
   so apps in different subdirectories can share a single
   vendored `lib/`.

If none of the three locations resolve, the build fails with a
diagnostic listing all three search paths.

There is no implicit `lib/` prefix. `import "moa" as moa;`
resolves `moa/` (workspace-root fallback if not entry-relative);
`import "lib/moa" as moa;` resolves `lib/moa/`. The user writes
the literal path the source lives at — no resolver magic. This
keeps "where does this come from?" answerable by reading the
import line.

There is no `$APERIO_PATH` or global library cache. v1 imports
are entirely local to the source tree.

## Workspace root caveat

Workspace-root detection walks upward looking for `Cargo.toml`.
For Aperio programs living inside this Rust monorepo (apps/,
examples/, etc.) the walk hits the workspace's top-level
Cargo.toml and the fallback works as expected.

Standalone-shipped Aperio binaries — sources not under a
Cargo.toml — won't have a workspace root to fall back on. They
can still use entry-relative imports (the single-file and dir
shapes above); only the workspace-fallback path is unavailable.
A future milestone may add an `.aperio-workspace` sentinel for
non-Cargo trees.

## Mangling scheme

Each imported library's top-level decls are rewritten with a
flat prefix so they never collide with the importer's symbols.
The mangled form is:

```
__lib_<alias>_<file_stem>_<name>
```

- **`<alias>`** is the importer-supplied namespace.
- **`<file_stem>`** is the basename of the source file the decl
  lives in, sans `.ap`. So two files in the same library can
  share a decl name without colliding.
- **`<name>`** is the original decl name as written in source.

Example: `lib-toy/greet.ap` declaring `locus Greeter { ... }`,
imported as `import "lib-toy" as toy;`, becomes
`__lib_toy_greet_Greeter` in the merged program.

Mangling is recursive: every reference to an imported decl
inside the imported seed itself — bare names in fn bodies,
struct literals, type expressions, capacity-slot element
types, `as_parent_for` clauses, etc. — is rewritten through a
unified rename map built across the whole library. Locals (let
bindings, fn params, lifecycle params, for-loop vars, pattern
bindings, generic params) shadow top-level names per ordinary
lexical scope rules; the mangler tracks scope so a local named
`Greeter` inside an imported fn body does NOT rewrite.

The user never writes the mangled form. Their import-site
references go through a per-build path-rename table that maps
`<alias>::<Name>` → `__lib_<alias>_<stem>_<Name>`, analogous to
the static `STDLIB_PATH_RENAMES` and `MOA_PATH_RENAMES` tables.
The codegen's `Cx::mangled_for_path` method consults all three
tables in order: static stdlib, static moa, per-build imports.

The mangling shape mirrors the existing hand-spelled
`__StdLangMorpheme` / `__MoaBraidId` prefixes the bundled
stdlib and moa seeds carry; cross-seed imports extend the same
discipline automatically.

## Strict barrier: no re-exports

If library A imports library B, B's decls are NOT visible to
A's importers. Each importer must declare its own dependencies
at its own top level.

The mechanism: imports inside imported library files are not
followed by the resolver. The lib's own source may have
`import` lines (they parse fine), but the build does not
resolve them transitively. Cross-library composition that needs
shared types today must route through the std seed.

A future milestone may relax this — supporting per-library
import scopes would let a lib internally reference another lib
without exposing it — but the v1 commitment is strict.

## No `pub` / `export`

Every top-level decl in an imported seed is exported. There is
no visibility modifier in v1. The whole imported seed becomes
available under the alias.

Adding `pub` doubles the design surface (every decl picks a
visibility; users author the modifier; the typechecker enforces
it); v1 declines that complexity until a workload demonstrates
a real need for export control.

## What's NOT shipped (v1 boundaries)

These are explicit non-features of the v1 import system. A
future milestone may relax some of them when concrete friction
demonstrates the need.

- **No package manager / registry.** Libraries are vendored —
  copied into the project's tree as source. The `lib/` directory
  is convention, not a search path.
- **No fetch / download.** The compiler reads only what's on
  disk.
- **No versioning / lockfile.** Source-level vendoring means
  pinning happens at the file system level (committed source).
- **No transitive resolution.** Imports declared in imported
  libraries are NOT followed (see "Strict barrier" above).
- **No `pub` / `export` keywords.** Everything top-level in an
  imported seed is exported.
- **`aperio run` (interpreter).** Cross-seed import resolution
  threads the per-build path-rename table only through the
  codegen path. The interpreter has no equivalent table; using
  `alias::Name` paths in a program run via `aperio run` will
  fail to resolve the same way `std::http::Request { ... }`
  literals already do. Use `aperio build` and run the resulting
  binary directly for programs with cross-seed imports.

## Implementation entry points

The implementation lives in two places:

- `crates/aperio-cli/src/main.rs` — `find_workspace_root`,
  `resolve_import`, `collect_target_files`, `resolve_imports`,
  `parse_with_imports`. The CLI does file resolution + mangling
  + merging; passes the resulting per-build path-rename table
  to `build_executable_with_imports`.
- `crates/aperio-codegen/src/mangle.rs` — `mangle_program`,
  `build_seed_renames`, `mangle_with_renames`. The AST walker
  rewrites decl sites and use sites with a scope-aware
  shadowing stack.
- `crates/aperio-codegen/src/codegen.rs` — `build_executable_with_imports`,
  `Cx::mangled_for_path`, the `import_renames` field on `Cx`.

End-to-end coverage lives in
`crates/aperio-codegen/tests/cross_seed_imports.rs` using
`tests/fixtures/lib-toy/` (two-file library) and
`tests/fixtures/import-toy-consumer/main.ap` (consumer with
`import "../lib-toy" as toy;`).
