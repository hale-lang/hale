# Aperio seeds — the exportable unit

> Captured 2026-05-10. Names the concept that has been
> conspicuously absent from Aperio's vocabulary: what is the
> unit that one Aperio codebase exports for another to draw
> from? This note is the source of truth for the term **seed**.
> The styleguide and refactor proposal both reference it.

## What is a seed

A **seed** is the exportable unit of an Aperio codebase.

It bundles **types** (shapes) and **loci** (flows) and exposes
them for other code to draw from. Per the types-vs-loci axiom
(`notes/aperio-types-vs-loci.md`), those are the only two
export categories — there is no third. Free fns, constants,
private state, and internal helpers are not exported; they
either fold into methods on namespace loci or remain as
locus-internal implementation detail.

A seed is **not a running program.** It's a body of
patterns ready to be incorporated into one. A program (a
*spell* in Aperio's grimoire register) draws from one or
more seeds at compile time, germinates them into its own
locus tower, and runs. The seed is dormant until cast.

## The metaphor

A seed contains the *potential* for spells. Drawing from it
means cultivating its forms and flows into your own running
spell. The seed isn't itself living code — it's the body
another caster germinates from.

Aperio's existing register is consistent here:

- The compiler is the **wand**.
- The language itself is the **meta-spell** (the spell of
  spellcasting).
- A running program is a **spell** being cast.
- A locus is the invariant **form every spell must take**.
- A seed is the **packed form** that another caster grows
  into their own spell.

The metaphor admits adjacent vocabulary that we keep parked
here in the design note (the styleguide doesn't load it all
into agent context — only the noun *seed* itself crosses
that boundary):

| Verb       | Programmer action                                 |
|------------|---------------------------------------------------|
| plant      | add a seed dependency                             |
| cultivate  | extend / refine a seed (develop it further)       |
| harvest    | use a particular type or locus from a seed       |
| bloom      | a fully-cast, running program (drawn from seeds)  |
| garden     | a developer's set of available seeds              |
| roots      | a seed's own dependencies on other seeds          |
| soil       | the runtime substrate (the lotus C-runtime)       |
| wither     | deprecate a seed                                  |

This vocabulary is **casual register**, not normative. It does
not appear in the spec; it appears in grimoire-register prose
where the metaphor helps land the idea.

## What's in v0

A single seed exists at v0: the Aperio standard library.

- **Source location:** `crates/aperio-codegen/runtime/stdlib/*.ap`
- **Exposed under:** the magic `std::*` path-call mechanism.
- **Bundling:** every Aperio binary compiled by codegen has the
  std seed concatenated into its source via `STDLIB_AP_SOURCE`
  (see `crates/aperio-codegen/src/codegen.rs`).
- **What it exports:** types like `std::http::Request`, loci
  like `std::log::Logger`, namespace lotuses like
  `std::lang::Lang` — currently 9 type/locus exports through
  `STDLIB_PATH_RENAMES`, plus path-call free fns reachable via
  `lower_stdlib_path_call_expr` arms.

User-defined seeds are **not shipped at v0.** An Aperio app
today is a single `apps/<name>/main.ap` file that draws only
from `std`. Cross-app shared code is duplicated or migrates
into `std::*` — both are interim measures, tracked as friction
that v1 user-defined seeds will resolve.

## What v1+ will likely ship

A sketch only — actual design waits for the multi-file
Aperio modules milestone:

- **Layout.** A seed is a directory of `.ap` files plus a
  manifest (proposed: `Seed.ap` for an Aperio-shaped manifest,
  or `seed.toml` for parser-light configuration). The manifest
  approach is open.
- **Manifest contents.** Seed name; version; explicit list of
  exported types; explicit list of exported loci; declared
  dependency seeds; minimum language version.
- **Imports.** A `use seed::path::Item` form (or similar)
  brings non-std seeds into scope. The magic `std::*` mechanism
  continues working unchanged for backward compatibility — std
  is just one seed among many, with privileged path resolution.
- **Visibility.** Items declared at the seed's top level are
  exported by default if they appear in the manifest's exports
  list. Private declarations stay file-local; cross-file-
  internal items stay seed-local. The leading-`__` convention
  for private/mangled names continues for stdlib but is no
  longer load-bearing for non-std seeds (the manifest is the
  source of truth).
- **Compilation.** Each seed compiles independently to an
  intermediate form (LLVM bitcode + a per-seed type registry).
  User-program compilation links the per-seed artifacts plus
  the user's own `.ap` files. Caching at the seed boundary
  becomes possible.
- **Distribution.** TBD. Likely git-URL-based at v1 (no central
  registry); registry comes later if the ecosystem needs one.

## Friction surfaced in v0 by *not* having user seeds

Sibling apps in early experiments duplicated identical helper
code: newline-string iteration, tagged-accumulator parsing,
file-stem-to-CamelCase conversion — all repeated across apps
because there's no way to share helper code short of bundling
into the std seed or copying.

The interim path: bundle the shared utilities into the std seed
for now. When v1 user seeds ship, migrate them out of std into
dedicated community seeds (`aperio-iter`, `aperio-tagged`,
`aperio-name`, `aperio-json`) where they belong.

## Naming conventions for seeds

When v1 ships, expected conventions:

- Seed names are lowercase, hyphen-separated. (Snake-case is
  reserved for internal identifiers; hyphen-case marks the
  package boundary, matching Cargo and Go-module conventions.)
- The `aperio-` prefix marks first-party seeds maintained by
  the Aperio language project. Examples: `aperio-iter`,
  `aperio-tagged`, `aperio-json`.
- Community seeds use any prefix or no prefix. Domain-specific
  seeds may include the domain name: `acme-billing`,
  `darkforest-events`.
- A seed's `std::` namespace is reserved for the standard
  library; user seeds expose via their own top-level namespace
  matching the seed name (or via path declared in the
  manifest).

How the seed grows once named is governed by the *rolling* rule —
see "Rolling the design" in `spec/styleguide.md`. Naming a seed
is the boundary act; rolling is what keeps the catalog inside
that boundary small enough to hold in head.

## Why seed (and not grimoire / arcanum / spell / package)

The user picked **seed** from a small set of candidates.
Captured here so the choice doesn't get re-litigated:

- **grimoire** — strong in-register fit, but conflicts with
  `docs/src/grimoire/` (the teaching grimoire). Disambiguating
  "The Grimoire (docs)" vs. "a grimoire (code unit)" is workable
  but adds cognitive load in a register that's already metaphor-
  heavy.
- **arcanum** — clean register fit, no collision. Less
  approachable than seed for newcomers. Plural form *arcana* is
  elegant for namespacing but the singular is academic.
- **spell** — conflicts with the existing meaning where a
  *running program* is "a spell being cast." Reusing for the
  packaged unit muddies the metaphor: the packaged form ISN'T
  running yet, only its incorporator's full spell is.
- **package** — boring but unambiguous; would match Go/Python/
  etc. precedent. Loses Aperio's distinctive register entirely.
- **seed** — the chosen term. Implies *potential for growth*
  and *something incorporated into a host*. Sits adjacent to
  but outside the casting register, which gives it room to
  develop its own metaphor (plant / cultivate / harvest /
  bloom) without competing with the wand / spell / cast
  vocabulary already in place.

The seed metaphor also has one virtue the others lack: a seed
is **dormant**. A grimoire is a book to be read; an arcanum is
knowledge to be learned; a spell is a casting; but a seed *waits*
until the conditions are right and then grows into the host
spell. That dormancy maps cleanly onto how Aperio compilation
works — the seed is bytes on disk until a program imports it
and the codegen weaves it into the running locus tower.

## Cross-references

- `notes/aperio-types-vs-loci.md` — the axiom that constrains
  seed exports to types + loci.
- `spec/styleguide.md` — applies the axiom and the seed
  concept to writing idiomatic Aperio.
- `crates/aperio-codegen/src/codegen.rs` — `STDLIB_AP_SOURCE`
  + `STDLIB_PATH_RENAMES` are the v0 std-seed implementation.

## Open questions for v1 design

Tracked here so the language milestone that ships user-defined
seeds doesn't relitigate them:

1. **Manifest syntax.** `Seed.ap` (Aperio-shaped, parsed by the
   compiler) vs. `seed.toml` (parser-light, separate format).
   Aperio-shaped is more in-register; toml is more practical
   for tooling. Resolution waits for the milestone.

2. **Versioning and compatibility.** How does a seed declare
   compatibility with prior versions of itself? With other
   seeds? Is semver appropriate, or does Aperio want a richer
   shape-compatibility check based on type/locus signatures?

3. **Re-exports.** Can seed A re-export a type/locus from seed
   B without making B a transitive dependency? Most modern
   languages allow this; whether Aperio does depends on how
   strictly we want seed boundaries to compose.

4. **Generic seeds.** When generics ship (currently sketched
   but not v1-stable), can a seed export a generic locus that
   gets specialized at the consuming site? How does that
   interact with separate compilation?

5. **The `std` seed's privileged status.** Continues working
   unchanged via `std::*` path resolution in codegen, OR moves
   to the same `use std::...` syntax as other seeds, OR both
   forms supported. Backward compatibility argues for both;
   the spec eventually picks one.

6. **Seed-local vs. seed-public visibility.** Does a seed need
   a `pub`/`private` keyword, or is the manifest export list
   sufficient? The manifest is the simpler shape; the keyword
   is more conventional.

These are deferred. The seed concept lands first; its
implementation details get hammered out when the multi-file-
module milestone arrives.

## See also

- `moa/MOA.md` — Memory-Owner Architecture; the composition
  discipline that shapes any stateful seed. The architecture
  ships as a separate top-level path prefix `moa::*`, parallel
  to `std::*`, with its source-of-truth directory at `/moa/`
  in the repo root.
- `lib/README.md` — domain types shared across MOA apps;
  populated when cross-seed imports unblock the sharing
  question Q18 above.
- `notes/aperio-types-vs-loci.md` — the foundational axiom
  these layers build on.
