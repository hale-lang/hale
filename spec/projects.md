# Projects

An Aperio project is a directory tree of `.ap` source files plus
anything those files vendor or reference. This document covers
the on-disk shape: how the directory tree composes into one or
more compiled binaries, how files within a directory share
scope, and how a project reaches into vendored libraries.

Two language-level commitments drive the shape:

- **F.19 — per-directory seed model.** Every `.ap` file in one
  directory compiles as one seed; all top-level decls share
  scope.
- **F.25 — cross-seed imports.** A library is a directory (or
  single file) of `.ap` source; the importer names a namespace
  alias and the resolver finds the source by path.

The recursion is in the import graph, not the directory tree.
The file system is presentation — convenient grouping of bits
that compose into a logical structure at parse time. Two
projects with identical import graphs can ship totally
different on-disk layouts; the lotus shape lives in the code.

See `spec/design-rationale.md` F.19 and F.25 for the design
rationale; `notes/v1.x-IMPORT-handoff.md` for the v1.x-IMPORT
milestone history.

## Project shapes

Three shapes are idiomatic at v1; pick the smallest that fits.

### Single-file script

For one-off programs and tiny utilities. One file, no
directory:

```
script.ap
```

Build: `aperio build script.ap` → `./script` binary.

Imports work — `import "../shared/foo" as foo;` resolves
relative to `script.ap`'s directory — but most scripts have no
imports.

### Single-app project

For an app that's developed and shipped as a unit:

```
myapp/                            # the project — one seed
├── main.ap                       # AppL declaration + fn main()
├── <concern>.ap                  # sibling concerns (F.19; same seed)
├── ...
├── lib/                          # vendored sub-loci (F.25)
│   ├── moa/
│   └── <helper-lib>/
├── README.md                     # what the app does
└── FRICTION.md                   # per-app friction log
```

Build: `aperio build myapp/` → `myapp/myapp` binary (next to
source; directory's basename becomes the binary name).

The center of the project is one named locus declared in
`main.ap` (per the apps-are-loci rule in `spec/styleguide.md`).
Sibling `.ap` files decompose by concern; they share top-level
scope under F.19. Vendored libraries live under `lib/` (by
convention — see "Disk is presentation" below).

There is no `src/` wrapper, no project metadata file, no
build-output directory. The directory IS the package; the
binary lands next to the source.

### Workspace (multi-binary)

For multiple related apps that share vendored libraries:

```
project/                          # workspace root
├── apps/
│   ├── fitter/main.ap            # one app, one seed
│   └── applier/main.ap           # another app, another seed
└── lib/                          # workspace-level shared libs
    ├── moa/
    └── shared/
```

Each app under `apps/` is its own seed; `aperio build
apps/fitter` produces `apps/fitter/fitter`. Imports written
inside an app like `import "lib/shared" as shared;` resolve via
F.25's workspace-root fallback — the entry-relative search
misses (no `apps/fitter/lib/`), and the resolver walks upward
to find a `Cargo.toml` and tries `<workspace-root>/lib/shared/`.

The workspace root is identified by walking upward from the
entry source looking for `Cargo.toml`. A future milestone may
add an `.aperio-workspace` sentinel for non-Cargo trees; until
then, projects shipped outside this monorepo can still use
entry-relative imports but lose the workspace fallback.

## Seeds — the per-directory model (F.19)

**A directory of `.ap` files compiles as one seed.** Every
top-level decl (locus, type, free fn, perspective, const,
interface) declared in any file in the directory is visible to
every other file in the same directory, in one shared scope.

`aperio build <dir>`, `aperio run <dir>`, and `aperio check
<dir>` accept directory targets and bundle every `.ap` file
under them. `aperio build <file.ap>` keeps working for
single-file targets (scripts, one-off cases).

**File order in the merged bundle is alphabetical by filename**
(deterministic). Resolution within the seed is **order-free** —
the typechecker flattens all top-level decls into one bundle
scope before name lookup, so a fn declared in `z.ap` is
callable from `a.ap` without ceremony.

**No per-file visibility.** There is no `pub`, no Go-style
uppercase-exported convention. Anything declared at the top
level of any file is visible to every other file in the seed.
Decompose by *concern* (one file per concern, helpers grouped
with their callers); don't try to encode visibility through
file boundaries.

**No subdirectories.** A subdirectory inside a seed is NOT
part of the seed — it's a separate seed. To reach into it,
import it as a library. This keeps "what's in scope at a given
file" answerable by reading the seed's directory; subdirs would
either silently inject decls (confusing) or require their own
import mechanism (which is what cross-seed imports are for).

## Cross-seed imports (F.25)

Cross-seed imports are how one seed reaches into another. The
imported seed's decls become available under a user-chosen
alias.

### Syntax

```
import "<path>" as <alias>;
```

- **`<path>`** is a string literal naming the library to import.
  The path is resolved per "Resolution order" below.
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
import "lib/finance" as fin;
import "../shared-helpers" as helpers;

fn main() {
    let q = fin::Quote { symbol: "ABC", price: 10 };
    let h = helpers::Formatter { };
}
```

### Resolution order

The compiler tries three locations in order; the first hit wins:

1. **`<importer-dir>/<path>.ap`** — single-file library.
2. **`<importer-dir>/<path>/`** — directory bundle. Every `.ap`
   file in the directory is one library seed (per F.19's
   per-directory model). File order in the merged bundle is
   alphabetical; resolution within the seed is order-free.
3. **`<workspace-root>/<path>/`** — workspace fallback. The
   workspace root is the first directory found by walking
   upward from the importer that contains a `Cargo.toml`.

If none of the three locations resolve, the build fails with a
diagnostic listing all three search paths.

### Mangling scheme

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

### Strict barrier: no re-exports

If library A imports library B, B's decls are NOT visible to
A's importers. Each importer must declare its own dependencies
at its own top level.

The mechanism: imports inside imported library files are not
followed by the resolver. The lib's own source may have
`import` lines (they parse fine), but the build does not
resolve them transitively. Cross-library composition that
needs shared types today must route through the std seed.

A future milestone may relax this — supporting per-library
import scopes would let a lib internally reference another lib
without exposing it — but the v1 commitment is strict.

### No `pub` / `export`

Every top-level decl in an imported seed is exported. There is
no visibility modifier in v1. The whole imported seed becomes
available under the alias.

Adding `pub` doubles the design surface (every decl picks a
visibility; users author the modifier; the typechecker enforces
it); v1 declines that complexity until a workload demonstrates
a real need for export control.

## Disk is presentation

The on-disk hierarchy is decoupled from logical identity. Three
layers to keep straight:

- **Library identity** = the alias the importer assigns
  (`foo`) plus the decl names inside the lib (`Bar`,
  `Greeter`). Stable across moves. References in user code —
  `foo::Bar`, `let g = foo::Greeter { ... }` — never change
  when a lib moves on disk.
- **Library location** = the string in the import line
  (`"lib/moa"`, `"../vendor/moa"`). Per-importer, mutable.
  Moving a lib costs N edits to import lines (one per
  importer) but zero edits to actual code.
- **Library convention** = the fact that vendored libs
  typically live under `lib/` rather than `vendor/` or
  `petals/`. The `lib/` prefix is style, not semantics. The
  resolver does not privilege `lib/`; it walks entry-relative
  then workspace-root for whatever path the importer wrote.

The lotus shape — the recursion tower of a project — lives in
the **import graph**, not the file tree. Parse-time
resolution + merging + mangling is where the recursion
materializes. The filesystem just stores the bits in some
convenient grouping; two projects could have identical import
graphs and totally different disk shapes.

This is by design. Refactoring the on-disk layout (moving a
lib from `lib/` to `vendor/`, splitting a monolithic lib into
two, consolidating two libs into one) only needs to update
import lines — the bulk of the code that references those libs
stays unchanged.

## Workspace root caveat

Workspace-root detection walks upward looking for `Cargo.toml`.
For Aperio programs living inside this Rust monorepo (apps/,
examples/, etc.) the walk hits the workspace's top-level
Cargo.toml and the fallback works as expected.

Standalone-shipped Aperio binaries — sources not under a
Cargo.toml — won't have a workspace root to fall back on. They
can still use entry-relative imports (the single-file and
directory shapes above); only the workspace-fallback path is
unavailable. A future milestone may add an `.aperio-workspace`
sentinel for non-Cargo trees.

## `aperio run` interaction

Cross-seed import resolution threads the per-build path-rename
table only through the codegen path. The interpreter has no
equivalent table; using `alias::Name` paths in a program run
via `aperio run` will fail to resolve the same way
`std::http::Request { ... }` literals already do. Use `aperio
build` and run the resulting binary directly for programs with
cross-seed imports.

## Git-based dependency fetching (`aperio fetch`)

A project may declare git dependencies in an `aperio.toml`
manifest at the repo root; `aperio fetch` clones each into
`vendor/<name>/` and pins resolved commit SHAs in
`aperio.lock`. The cloned source is then picked up
automatically by the import-resolution order above (path 1 of
the resolver looks at `<importer-dir>/<path>/`, which is
exactly where the fetcher places `vendor/<name>/`).

`vendor/` is toolchain-managed and distinct from `lib/`
(hand-maintained, never touched by the fetcher). Both paths
work identically through the import resolver but keeping them
physically separate prevents `aperio fetch` from clobbering
hand-vendored source on a name collision.

See `spec/packages.md` for the full surface — manifest
format, lockfile shape, pin semantics, fetch command behavior,
and library-author conventions.

## What's NOT shipped (v1 boundaries)

Explicit non-features of the v1 project / import system. A
future milestone may relax some of them when concrete friction
demonstrates the need.

- **No transitive resolution.** Imports declared in imported
  libraries are NOT followed (see "Strict barrier" above). If a
  library has its own deps, the consumer must vendor them too.
- **No registry / version ranges / semver.** Dependency pins
  are exact git refs. See `spec/packages.md` § "What's NOT in
  v1" for the full list of package-management non-features.
- **No `pub` / `export` keywords.** Everything top-level in an
  imported seed is exported.
- **No `src/` wrapper.** Source files live at the project root.
- **No build-output directory.** The binary lands next to
  source.

## Implementation entry points

The project / import surface lives in three places:

- `crates/aperio-cli/src/main.rs` — `find_workspace_root`,
  `resolve_import`, `collect_target_files`, `resolve_imports`,
  `parse_with_imports`, `collect_ap_files`. The CLI does file
  resolution + mangling + merging; passes the resulting
  per-build path-rename table to
  `build_executable_with_imports`.
- `crates/aperio-codegen/src/mangle.rs` — `mangle_program`,
  `build_seed_renames`, `mangle_with_renames`. The AST walker
  rewrites decl sites and use sites with a scope-aware
  shadowing stack.
- `crates/aperio-codegen/src/codegen.rs` —
  `build_executable_with_imports`, `Cx::mangled_for_path`,
  the `import_renames` field on `Cx`.

End-to-end coverage lives in
`crates/aperio-codegen/tests/cross_seed_imports.rs` using
`tests/fixtures/lib-toy/` (two-file library) and
`tests/fixtures/import-toy-consumer/main.ap` (consumer with
`import "../lib-toy" as toy;`).
