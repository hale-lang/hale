# Projects

A Hale project is a directory tree of `.hl` source files plus
anything those files vendor or reference. This document covers
the on-disk shape: how the directory tree composes into one or
more compiled binaries, how files within a directory share
scope, and how a project reaches into vendored libraries.

Two language-level commitments drive the shape:

- **F.19 — per-directory seed model.** Every `.hl` file in one
  directory compiles as one seed; all top-level decls share
  scope.
- **F.25 — cross-seed imports.** A library is a directory (or
  single file) of `.hl` source; the importer names a namespace
  alias and the resolver finds the source by path.

The recursion is in the import graph, not the directory tree.
The file system is presentation — convenient grouping of bits
that compose into a logical structure at parse time. Two
projects with identical import graphs can ship totally
different on-disk layouts; the locus shape lives in the code.

See `spec/decisions.md` F.19 and F.25 for the design
rationale.

## Project shapes

Three shapes are idiomatic at v1; pick the smallest that fits.

### Single-file script

For one-off programs and tiny utilities. One file, no
directory:

```
script.hl
```

Build: `hale build script.hl` → `./script` binary.

Imports work — `import "../shared/foo" as foo;` resolves
relative to `script.hl`'s directory — but most scripts have no
imports.

### Single-app project

For an app that's developed and shipped as a unit:

```
myapp/                            # the project — one seed
├── main.hl                       # AppL declaration + fn main()
├── <concern>.hl                  # sibling concerns (F.19; same seed)
├── ...
├── lib/                          # vendored sub-loci (F.25)
│   ├── moa/
│   └── <helper-lib>/
├── README.md                     # what the app does
└── FRICTION.md                   # per-app friction log
```

Build: `hale build myapp/` → `myapp/myapp` binary (next to
source; directory's basename becomes the binary name).

The center of the project is one named locus declared in
`main.hl` (per the apps-are-loci rule in `spec/styleguide.md`).
Sibling `.hl` files decompose by concern; they share top-level
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
│   ├── fitter/main.hl            # one app, one seed
│   └── applier/main.hl           # another app, another seed
└── lib/                          # workspace-level shared libs
    ├── moa/
    └── shared/
```

Each app under `apps/` is its own seed; `hale build
apps/fitter` produces `apps/fitter/fitter`. Imports written
inside an app like `import "lib/shared" as shared;` resolve via
F.25's workspace-root fallback — the entry-relative search
misses (no `apps/fitter/lib/`), and the resolver walks upward
to find a `Cargo.toml` and tries `<workspace-root>/lib/shared/`.

The workspace root is identified by walking upward from the
entry source looking for `Cargo.toml`. A future milestone may
add an `.hale-workspace` sentinel for non-Cargo trees; until
then, projects shipped outside this monorepo can still use
entry-relative imports but lose the workspace fallback.

## Seeds — the per-directory model (F.19)

**A directory of `.hl` files compiles as one seed.** Every
top-level decl (locus, type, free fn, perspective, const,
interface) declared in any file in the directory is visible to
every other file in the same directory, in one shared scope.

`hale build <dir>`, `hale run <dir>`, and `hale check
<dir>` accept directory targets and bundle every `.hl` file
under them. `hale build <file.hl>` keeps working for
single-file targets (scripts, one-off cases).

**File order in the merged bundle is alphabetical by filename**
(deterministic). Resolution within the seed is **order-free** —
the typechecker flattens all top-level decls into one bundle
scope before name lookup, so a fn declared in `z.hl` is
callable from `a.hl` without ceremony.

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
(substrate-facing surfaces — lifecycle, mode, closure assertions,
bus handlers — can't declare `fallible(E)`; user-declared `fn`
members and free fns can): the user names the commitment at the
surface so a downstream reader doesn't have to reconstruct the
namespace from the path.

Imports appear at the top of a file, before any top-level
declaration. Multi-file seeds may declare imports in any file;
the build merges every file's imports into one set against the
seed's directory + workspace root.

### Examples

```hale
import "lib/finance" as fin;
import "../shared-helpers" as helpers;

fn main() {
    let q = fin::Quote { symbol: "ABC", price: 10 };
    let h = helpers::Formatter { };
}
```

### Resolution order

The compiler tries three locations in order; the first hit wins:

1. **`<importer-dir>/<path>.hl`** — single-file library.
2. **`<importer-dir>/<path>/`** — directory bundle. Every `.hl`
   file in the directory is one library seed (per F.19's
   per-directory model). File order in the merged bundle is
   alphabetical; resolution within the seed is order-free.
3. **`<workspace-root>/<path>/`** — workspace fallback. The
   workspace root is the first directory found by walking
   upward from the importer that contains a `Cargo.toml`.

If none of the three locations resolve, the build fails with a
**located** diagnostic at the import's path literal — the file,
line and column of the `import` that names nothing, with the three
search paths as the message body (2026-09-20, GH #860). It is a
finding about the program like any other, so every channel carries
it: a record under `hale check --json` / `hale verify --json`, the
located line with a caret under the path in text mode, and the same
located line from `build`, `run` and `test`, which have no
machine-readable channel. (Before #860 it was a sentence on stderr
plus a bare non-zero exit, so `--json` answered an unresolvable
import with an empty stream.)

A path that DOES resolve — to a directory with no `.hl` files in
it, or to a file that will not open — is an `"io error"` record
about that path instead: the import was resolved, and what failed
was reading what it named.

**A library's identity is its seed directory.** `main.hl` is a
seed's entry file, not a library of its own, so a rule-1 hit on a
`main.hl` resolves as rule 2 against the directory around it:
`import "../lib/main"` and `import "../lib"` name the **same**
library — one file set, one `lib_id`, one set of mangled symbols —
and an importer that uses one spelling in one of its files and the
other spelling in another gets one library under both aliases. Any
other single file is a library of its own under rule 1, bringing
in that file and nothing else. The one exception is a `main.hl`
in the importer's own directory, which stays a rule-1 single-file
library: a seed does not import itself.

**A file belongs to exactly one library.** So a rule-1 single-file
import of a file that a directory import somewhere in the same
build also takes is **refused**, with a located error at the
single-file import's path literal naming the library that already
holds it and where that import is written:

```text
/tmp/app/other.hl:1:8: type error: `../lib/helper` is already part of the library imported as `a` at /tmp/app/main.hl:1; a single file of a directory-imported library is not a library of its own — reach its declarations as `a::<name>` and drop this import
    import "../lib/helper" as h;
           ^^^^^^^^^^^^^^^
```

The two identities are genuinely different — one file against the
whole seed — and they are not reconcilable the way the two
spellings of one library are: resolution parses each file once, so
`helper.hl` cannot carry two manglings. Before the refusal the
resolver's file set was global and whichever identity resolved
second was handed only the files the first had not taken, so half
of one alias's names resolved to nothing with no diagnostic
anywhere (2026-09-20, GH #820). The refusal is order-free: it names
the file spelling whichever of the two resolved first, and the
directory import is left alone. Both spellings of the SAME library
(`../lib` and `../lib/main`) are unaffected — they are one
`lib_key`, not two.

### Mangling scheme

Each imported library's top-level decls are rewritten with a
flat prefix so they never collide with the importer's symbols.
The mangled form is:

```
__lib_<lib_id>_<file_stem>_<name>
```

- **`<lib_id>`** is a stable, sanitized identifier for the lib
  derived from its canonical path relative to the workspace
  root (the nearest ancestor directory containing `hale.toml`
  or `Cargo.toml`). Two consumers importing the same lib
  produce the same `lib_id` regardless of which alias each
  consumer chose, and regardless of which of the two spellings
  of "Resolution order" above each consumer wrote. Non-identifier
  characters in the path collapse to `_`; runs of underscores
  collapse to one.
- **`<file_stem>`** is the basename of the source file the decl
  lives in, sans `.hl`. So two files in the same library can
  share a decl name without colliding.
- **`<name>`** is the original decl name as written in source.

Example: `<repo>/shared/messages/messages.hl` declaring `type
Order { ... }`, imported by app A as `msgs` and by app B as `m`,
both produce `__lib_shared_messages_messages_Order` in the
merged program. The shared identity is the natural shape for
DTO seeds exchanged on a bus — both apps see Order as
symbol-identical, and the wire bytes match by construction.

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
`<alias>::<Name>` → `__lib_<lib_id>_<stem>_<Name>`, analogous to
the static `STDLIB_PATH_RENAMES` and `MOA_PATH_RENAMES` tables.
The codegen's `Cx::mangled_for_path` method consults all three
tables in order: static stdlib, static moa, per-build imports.
The `<alias>` is the importer's local namespace choice (used
only at the call-site reference layer); the `<lib_id>` is the
lib's canonical identity.

Collision avoidance: two different libs live at different paths,
get different `<lib_id>`s, never collide regardless of what
aliases their importers picked.

`<lib_id>` fallback when no workspace root is in scope (e.g., a
one-off `hale build foo.hl` outside any toml-rooted repo):
the lib's directory base name (or file stem for single-file
imports), sanitized. Less collision-safe but the only stable
thing available; rare in practice.

The mangling shape mirrors the existing hand-spelled
`__StdLangMorpheme` / `__MoaBraidId` prefixes the bundled
stdlib and moa seeds carry; cross-seed imports extend the same
discipline automatically.

### An alias and a value may share a name

An import alias lives in its own namespace: it may coincide with
the name of a fn, const, locus or topic the seed declares.

```hale
import "../core" as core;

fn core(args: String) -> String { return "[" + args + "]"; }

fn go() -> String {
    return core::greet("mid") + " " + core("x");
}
```

Both references resolve. A **qualified path** (`core::greet`)
resolves its head against the seed's import aliases — the head of
a two-segment path is never a value. A **bare name** (`core("x")`,
`core`) resolves against the seed's own declarations. Locals still
shadow both, per the scope rules above.

The one head that is not an alias is a **type**: `Color::Red` is
an enum-variant path, so a seed declaring both `import "…" as
Color;` and `type Color = enum { … }` resolves `Color::…` against
its own enum, and the alias is unreachable in path position from
that seed. Aliases are conventionally lower-case for exactly this
reason.

This holds identically whether the seed is compiled directly or
reached through an import: the import rewrite renames a path head
only when the head names one of the seed's own **type** decls, so
an alias head survives mangling and the per-build path-rename
table resolves it as it does at the import site. (GH #714: the
rewrite used to rename any head that matched a seed decl, so
`core::greet` became `__lib_<lib_id>_main_core::greet` — a path
through the free fn's mangled symbol. `hale check` passed, since
it resolves the alias, and the build failed. A seed that ran on
its own became unbuildable the moment someone imported it.)

### Scoped imports (A4)

If library A imports library B, B's decls become reachable
**inside A's body only** under the alias A chose for B. A's own
importers (apps or other libraries) do NOT see B unless they
import it themselves. The mechanism: the resolver recurses into
each imported library's `import` directives with the library's
own directory as the new importer dir, so relative paths resolve
the way the library author wrote them.

This is the per-library-scoped-import shape called out as future
work in the v1 IMPORT milestone — A4 lifts the v1 strict barrier
to unblock the `pond/_util/*` retrofit (libraries that share
internal helper libs without forcing every consumer to vendor
them).

"Reachable inside A's body" is full reach, including **expression
and return position**: A may name a B type as a field/param/return
type, call B's free fns, *and* instantiate B's types and loci by
qualified literal — `b::Thing { ... }`, `b::SomeLocus { ... }` —
inside A's own fns, even when A is itself only reached two hops
down (`app → A → B`) and across A's multiple files. (This is the
"G34" shape; verified at HEAD, WS3.4 2026-06-11. The re-export
rule below still holds — the *app* cannot name `b::Thing` unless
it imports B itself.)

**A qualified type is checked like a local one.** In a WHOLE
program — one where every `import` is resolved, which is what
`hale check <dir>`, `build`, `run` and `test` hand the checker —
`lib::Thing` in any annotation position (`let`, fn parameter, fn
return, struct field, `params` field, a `capacity` slot, an alias
target) denotes the imported declaration and is typed as it. The
annotation constrains what may fill it, the same way a locally
declared type does: `let t: lib::Thing = "x";` is a located type
error naming `lib::Thing`, and field and method access through the
binding resolves against the imported declaration. Two aliases for
one library are one type, because the declaration they name is one
declaration.

**A call into an imported seed is checked like a local one.** In a
whole program, `lib::f(..)` is typed with the library's own
signature: too few arguments (fewer than its params without a
default), too many, an argument of the wrong type, `or` on a fn
that cannot fail, and the type of the value it returns are the same
located errors a same-seed call gets, naming the callee as written
(fn `lib::add3` takes at least 3 arguments, got 2). The value an
imported factory returns is typed too, so a method called on it is
held to its signature. (GH #1028: the path was typed as unknown,
and a call with an argument too few passed `hale check` to fail in
`hale build`.)

One thing stays permissive, because the declaration genuinely is
not in the bundle: a check of a single FILE of a multi-file seed,
where the `import` line may live in a sibling — one file is not a
whole program, the same boundary the unbound-identifier rule
draws. (GH #833: until then the annotation typed as unknown, so
nothing was checked against it and the mismatch surfaced at build,
unlocated, or not at all.)

**An unresolvable qualified name is a located error.** The other
half of the same rule. In a whole program, a qualified path that
resolves to *nothing* is an error located at the path, in every
position a path can stand in — an annotation, a call, a struct or
locus literal, a const, an enum variant. Two shapes, two messages:

```text
main.hl:4:13: type error: `zz::f`: `zz` is not an import or a type of this seed
main.hl:6:13: type error: `b::Greeting` is not declared by the library imported as `b`; `b` provides: Mood, Tick, hello
```

The first is a head that names nothing — not `std::`, not an
import this build resolved, not a declaration of this seed. The
second is a head an import *does* answer, with a name behind it
that the library never declared; it carries a did-you-mean when a
spelling is close and the library's own surface when none is. A
`bindings { }` entry needs no separate rule: a topic nothing
declares has always been a located error there, qualified or not.

Permissive, and for the same reason as above — the declaration is
genuinely absent, or this is not a whole program: an import the
bundle never RESOLVED (a consumer holding one seed without its
libraries, such as the language server's per-directory bundle),
and a check of a single FILE of a multi-file seed. `std::` is
exempt because the stdlib tables answer a `std::` path and report
their own typos, and a head that names a declaration
(`Color::Red`, a `type C2 = Color;` alias) is not an import at
all. So are `time::sleep` and `time::monotonic`, the two paths the
compiler still lowers without the `std::` prefix — the rule refuses
nothing the build accepts. Every *other* unprefixed stdlib path is a
dropped prefix, and says so where it stands: "`env::args_count` is
unresolved — did you mean `std::env::args_count`?", which is the
message the build has always given for it.

(GH #803: until then every one of these passed `hale check` and
`hale verify` and died in codegen — `path call zz::f in
expression position`, `unknown qualified name b::Nope`,
`qualified type zz::T not in stdlib path-renames table` — late,
from another layer, and with no source location.)

**No re-exports.** B's decls are not visible to A's importers
unless they declare their own dependency on B. The `<lib_id>`
in B's mangled prefix is derived from B's canonical path, NOT
from any importer's alias — so a util library reached through
two different importers gets ONE shared identity in the merged
program (path-deduplicated by canonical path in the visited
set). 2026-05-22 changed this from the original per-importer
mangling, removing the "DTO seed across two apps" sharp edge
where the same source produced different symbols.

**Cycles.** The CLI's canonical-path `visited` set bounds the
walk; a lib that imports itself or two libs that mutually import
each other resolve once each and stop.

**An alias binds in its own seed only, through the rename table
too.** Two seeds in one build may choose the same alias for
*different* libraries: if `a` says `import "../libx" as u;` and
the app says `import "../liby" as u;`, `u::f()` in `a` is
`libx`'s `f` and `u::f()` in the app is `liby`'s. Each reference
resolves against the aliases of the seed it is written in, and
the seed's files share one alias namespace exactly as they share
one declaration namespace.

The build holds that guarantee through the per-build path-rename
table (`alias::Name -> mangled symbol`), which is keyed by the
alias as written. When two seeds bind one alias name to different
libraries the compiler gives each binder a scoped head of its own
and rewrites that seed's own references to match, so the two
cannot be confused; the alias the author wrote is what
diagnostics show. Until GH #746 the table was flat: the last
binding registered won, both seeds resolved to one library, and
nothing reported it (`hale check` passed and the binary computed
the wrong value).

**A reference must name an alias its own seed declares.** The
guarantee above holds in both directions, so a qualified path whose
head is an import alias that some *other* seed of the build declares
— and this one does not — is a check error located at the path,
naming the seed that does declare it. This closes the other half of
the flat-table hole: a seed that imported nothing at all could write
`u::f()` and have the one table answer it out of an importer's row,
so a library silently called whatever library its app happened to
spell `u` (and the same library, compiled from a different app,
called something else). Until GH #762 that was accepted in silence
whenever the alias was uncontested, and reported only when two seeds
contested it.

Exempt from the rule: `std::`, the bundled namespace no seed
imports, and a head naming one of the seed's own declarations
(`Color::Red` is an enum variant, not an alias). A head NO seed in
the build declares is not this rule's — nothing resolves through it
in either direction — and is refused by the unresolvable-qualified
rule above, at the path.

One seed whose own files disagree — the same alias bound to two
libraries inside a single namespace — resolves to one of them, as
it always has; the compiler does not (yet) reject that shape.

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

The locus shape — the recursion tower of a project — lives in
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
For Hale programs living inside this Rust monorepo (apps/,
examples/, etc.) the walk hits the workspace's top-level
Cargo.toml and the fallback works as expected.

Standalone-shipped Hale binaries — sources not under a
Cargo.toml — won't have a workspace root to fall back on. They
can still use entry-relative imports (the single-file and
directory shapes above); only the workspace-fallback path is
unavailable. A future milestone may add an `.hale-workspace`
sentinel for non-Cargo trees.

## `hale run` interaction

`hale run` and `hale build` share the same codegen path and, as
of GH #904, the same build OPTIONS: one parser, one
`BuildOptions`, and `run` fingerprints what it built with rather
than the defaults (see *Build flags + environment* above). As of
WS3.3 they also share the same *import* path: both the
single-file form and the directory form (`hale run ./dir`)
resolve `import "..." as ...;` directives, build the per-build
path-rename table, and rewrite qualified `alias::Name` references
identically. A directory `hale run` now produces the same
merged-and-resolved program as `hale build ./dir` — it execs it
instead of writing a binary.

(Previously the directory `hale run` form bundled the directory's
files *without* threading the path-rename table, so cross-seed
`alias::Name` references — and a topic decl referenced from a
sibling file — failed under `run` though they worked under
`build`. That gap is closed; the two commands no longer diverge on
imports. `hale check` had the last surviving divergence: a
multi-file seed with no imports was kept as per-file programs, and
a pre-pass resolved each file alone — so a `topic` declared in one
file and subscribed from a sibling reported "unknown topic" under
`check` while `build` and `run` resolved it. Closed 2026-08-11: a
multi-file seed merges before checking, exactly like the
import-bearing path.)

### What `hale run` starts, `hale run` ends

`hale run` is a foreground wrapper, and everything it starts is
bounded by it: the compiled program, and under `--observe` the iris
session beside it and the `hale build` that materializes the
observer. On Linux each of them is spawned with a parent-death
signal (`PR_SET_PDEATHSIG`, SIGTERM) and stays in `hale`'s process
group, so a `hale` that dies — a CI cancel, a SIGKILL — takes them
with it, and a group-directed signal (a shell's Ctrl-C, `timeout`
without `--foreground`) still reaches them as it always did. Nothing
is left running that only a human knows to kill.

`hale` does not die first on the signals a program drains on (GH
#1039, `semantics.md` § "Drain cascade (whole-process)"): once the
program is running, `hale` ignores SIGINT — a Ctrl-C reaches the
program through the process group, and `hale` waits for the drain —
and forwards a SIGTERM sent to its own pid (`timeout`, `kill $pid`)
to the program, then keeps waiting. Either way it reports how the
program ended: exit 0 for a drain that finished, "killed by
SIGTERM" for one that outlived its grace.

The observed session also gets its OWN stdout and stderr, which
`hale` relays to its stderr. The program's stdout is the command's
output: an observer must not interleave its chatter into it, and —
the reason this is a rule rather than a preference — a caller
reading that stdout through a pipe must see EOF when `hale` exits.
Until 2026-09-20 (GH #905) it did not: the session inherited the
descriptor, so `hale run --observe prog.hl | cat` never returned.
`hale` reaps the `hale iris` it started, but fuse-hl UNDER that
`hale iris` was left running with the pipe's write end and nothing
was left to close it — the program ending was enough to hang the
caller, and a `hale` killed before it could reap anything left the
whole session behind.

## Git-based dependency fetching (`hale fetch`)

A project may declare git dependencies in an `hale.toml`
manifest at the repo root; `hale fetch` clones each into
`vendor/<name>/` and pins resolved commit SHAs in
`hale.lock`. The cloned source is then picked up
automatically by the import-resolution order above (path 1 of
the resolver looks at `<importer-dir>/<path>/`, which is
exactly where the fetcher places `vendor/<name>/`).

`vendor/` is toolchain-managed and distinct from `lib/`
(hand-maintained, never touched by the fetcher). Both paths
work identically through the import resolver but keeping them
physically separate prevents `hale fetch` from clobbering
hand-vendored source on a name collision.

See `spec/packages.md` for the full surface — manifest
format, lockfile shape, pin semantics, fetch command behavior,
and library-author conventions.

## What's NOT shipped (v1 boundaries)

Explicit non-features of the v1 project / import system. A
future milestone may relax some of them when concrete friction
demonstrates the need.

- **Deduplication is by canonical path, not content.** A library
  reached through two different importers gets ONE shared identity
  — its `<lib_id>` is derived from its canonical path and
  deduplicated in the resolver's visited set (the 2026-05-22
  change; see § "No re-exports" above), so the same source yields
  the same symbols across consumers. What's *not* done is
  content-level unification: two libraries vendored at different
  paths are distinct symbols even if byte-identical.
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

- `crates/hale-cli/src/main.rs` — `find_workspace_root`,
  `resolve_import`, `collect_target_files`, `resolve_imports`,
  `parse_with_imports`, `collect_ap_files`. The CLI does file
  resolution + mangling + merging; passes the resulting
  per-build path-rename table to
  `build_executable_with_imports`.
- `crates/hale-codegen/src/mangle.rs` — `mangle_program`,
  `build_seed_renames`, `mangle_with_renames`. The AST walker
  rewrites decl sites and use sites with a scope-aware
  shadowing stack.
- `crates/hale-codegen/src/codegen.rs` —
  `build_executable_with_imports`, `Cx::mangled_for_path`,
  the `import_renames` field on `Cx`.

End-to-end coverage lives in
`crates/hale-codegen/tests/cross_seed_imports.rs` using
`tests/fixtures/lib-toy/` (two-file library) and
`tests/fixtures/import-toy-consumer/main.hl` (consumer with
`import "../lib-toy" as toy;`).

## Build flags + environment

A flag may stand on either side of the target: the first argument
that is not a flag IS the target, so `hale build --dev app.hl` and
`hale build app.hl --dev` are one command (2026-09-20, GH #861;
before it `build`'s flag parsing started at argv[3] and a flag in
front of the target was read as the target itself). Value-taking
flags — `--link`, `--csrc`, `--target`, `--target-cpu`,
`--target-cache` — take the next argument as their value, so that
argument is never mistaken for the target. `hale run` follows the
same rule up to the target and then stops: everything after the
target is the PROGRAM's argv, which is why `run`'s own `--observe`
goes in front of it.

`run` takes the same build options `build` does, from the same
parser (2026-09-20, GH #904; before it, `run` compiled with
`BuildOptions::default()` no matter what it was given, so every
build option was first read as the target and then — GH #900 —
named and refused, and the documented spot-check `hale run
prog.hl` could exercise neither a dev build nor an FFI program).
So does `hale replay`, which recompiles the program it admits a
recording against. Both refuse, by name, the flags that report ON
a build rather than change it — `--locality-report`,
`--target-cache`, `--strict`, `--wrap-main` — and `run` refuses a
target it cannot exec (`--target wasm32`), since it compiles in
order to execute. The options are part of the execution identity
(runtime.md § *Replay*): a recording made under `hale run --dev`
is admitted by `hale replay --dev` and refused by a default
`hale replay`, with the identity message.

| Surface | Effect |
|---|---|
| `hale build --dev` / `hale run --dev` / `hale replay --dev` / `HALE_DEV=1` | Latency mode: LLVM O1 pipeline + Less machine codegen instead of the O3/`target-cpu=native` release default. For edit-build-run loops. |
| `hale check --json` | NDJSON diagnostics on stdout, one object per line (`file`/`line`/`col`/`severity`/`kind`/`message`, plus `related`: an array of `{file, line, col, note}` secondary locations, present only when a diagnostic has them — e.g. a duplicate name's previous declaration; 2026-08-11) — editor/LSP consumption. EVERY finding that fails the command is a record, including a lexical or syntactic one: a file that does not parse — the target's own or any file reached through an `import` — emits one record per diagnostic, `"kind":"parse error"` (or `"lex error"`), at that file's own line and column (2026-09-19, GH #777; before it the parse path printed text to stderr and left `--json` empty, so a gate saw a non-zero exit with nothing explaining it). An input that could not be READ is a record too, `"kind":"io error"`, `"file"` the path it is about and `"message"` the OS error, at `"line":0,"col":0` — no position, because there is no text to have a position in: a target that does not exist, a `.hl` file of the seed that will not open, a file of the import graph that will not open (2026-09-20, GH #806; these printed a sentence on stderr and left the stream empty, an environment failure wearing the same shape as a crash). An `import` that resolves to NOTHING is a located record like any other finding, `"kind":"type error"` at the line and column of the path literal that names nothing, with the three search paths tried in its `message` (2026-09-20, GH #860; it was the last failure on this path still reported only as a sentence on stderr). A finding whose position is inside the **embedded stdlib** — the witness leaf of a violated effect assertion, which the analyzer reached by walking into a Hale-source stdlib body — is a positionless record too, `"file":""` at `"line":0,"col":0`, with the stdlib file and line in its `message` (`in the standard library, io_tcp.hl:118:18`); the same holds for a `related` entry raised there (2026-09-20, GH #856). The stdlib parses at base 0 in its own coordinate space, so such an offset is not a position in any file of the seed, and a `file` field is a join key: before this it named whichever seed file the number happened to fall inside. An empty stream therefore means a clean seed, and nothing else does. `hale verify --json` is the same stream under the stricter gate. `hale check` runs in ~10 ms on the largest apps. |
| `HALE_TIME=1` | Per-phase build wall times on stderr (front-end+codegen, llvm-passes, obj-emit, emit+link). |
| `--no-warn-unbounded-alloc` | Opts a run out of the default-on memory-bound survey (see verification.md). |
| `hale check --sealable` | Reports which loci could take `@sealed` and what it would cost: per locus, the sites outside it that read or write its `params`. Empty means sealing is a no-op. The survey reruns the real check against an all-sealed clone rather than approximating the rule, so it cannot disagree with the checker. |
| `hale check --strict-secret` | The fail-closed `@secret` walk: every branch, alias propagation through `let` and tuple destructuring, and `uncertified` for anything it cannot follow. Opt-in, and loud by design; the default `@secret` pass is a lint (see verification.md § Secrets). |
| `hale check --strict-fallible` | A bare call to a `fallible` stdlib entry point — no `or` — is an error instead of the default warning (semantics.md § "A bare stdlib call is a warning, then an error"); `hale verify` fails on the warning already. The default becomes an error at the next minor (GH #738). |
| `--target native\|wasm32\|<triple>` | The target to build for; `hale --list-targets` names every triple with its tier as seen from this host. `native` (default) and the host's own triple build and link the host platform; `wasm32` builds from any host. Another host's native triple is a cross target (GH #970; before it, GH #969, such a triple silently built the host): codegen uses the target's own backend, triple and a generic CPU. A **Linux** triple — gnu (`--target x86_64-unknown-linux-gnu` on a Mac, or the other architecture on Linux) or musl (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`: linked static, one file that runs on any Linux, a cross target from every host; no `async_io` pools — musl has no ucontext, so the check refuses `where async_io` for it as it does for macOS) — is then **linked** here: the lotus runtime is compiled and the program linked with `zig cc -target` (zig on PATH or `HALE_ZIG`; glibc pinned at 2.31, `HALE_TARGET_GLIBC` overrides), against a **target sysroot** holding OpenSSL and zlib as static archives and, when built, the tree-sitter shim — `scripts/target-sysroot.sh <triple>` builds one under `<cache>/hale/sysroot/<triple>` — OpenSSL and zlib from pinned source tarballs, compiled with zig against the same glibc floor (a distribution's archives are built against its own newer glibc and cannot be linked at the floor), `HALE_TARGET_SYSROOT` names another. The emitted gnu binary depends on the target's glibc alone; a musl one on nothing. Cached runtime objects are keyed by the compiler, so one target's never serve another. A missing zig or sysroot fails before the link, naming the piece and how to get it; sanitizers are host-only and refused. A **Darwin** triple from anywhere else has no toolchain here and is emitted as a relocatable object, `<stem>.o`, with a note. `run` and `test` refuse every foreign triple, as they refuse `wasm32`. A Windows triple is refused at argument parsing as not buildable yet (GH #445). The check judges target questions — `where async_io` — against the named target, not the host. |
| `--target-cpu native\|baseline` | Backend CPU tuning (native = host, default; baseline = portable x86-64-v3). |
