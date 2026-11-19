# Stability policy

Hale is pre-1.0 and still changing quickly. This page says what a
release may change, how a change reaches you, and what 1.0 will
promise. It binds the language and the standard library. The
toolchain's commands and the libraries built in Hale (DNA, the habitat)
follow the same release rules but carry no 1.0 promise of their own.

## Before 1.0

Hale's version is `0.MINOR.PATCH`.

- **Any release may change the language or the stdlib.** A patch
  release is usually a bug fix, but nothing holds a change back for a
  minor one.
- **A change that breaks programs migrates the in-repo code in the same
  PR** (the stdlib, DNA, iris, the tests, the examples, the book), and
  is listed under its release's `### Breaking` heading with a one-line
  migration.
- **There are no deprecation windows before 1.0.** A form that goes
  away is refused in the release that removes it, and its error points
  at the replacement. A change may still warn first when that is the
  easier migration, but it owes no window.

## Every release names its breaking changes

Every release's CHANGELOG section opens with a `### Breaking` heading.
It lists each change that can make a program that built under the
previous release stop building, or behave differently. If there are
none, the heading says `None.` A tool counting releases with no break
reads one heading.

That heading is assembled from the PRs' CHANGELOG fragments. Each
`unreleased/<pr>.md` declares its breakage explicitly. It either
carries a `### Breaking` section with a bullet per break and its
migration, or a line reading exactly `Breaking: none`. Saying nothing
is refused. `scripts/changelog-check.sh` enforces this in CI (the
`changelog fragments` job). `scripts/changelog-fold.sh` refuses to fold
a fragment that declares neither, gathers every fragment's Breaking
bullets under the release's one heading, and drops the `Breaking: none`
lines.

## Tiers

Every stdlib namespace and every file of `spec/` is marked **stable** or
**experimental** in [`spec/stability.toml`](./spec/stability.toml). The
book renders it in [Stability & Compatibility](./docs/src/stability.md).
Before 1.0 the table is information, not a rule: it says which surface
is settled and which is new or still moving, and each experimental row
says why. Both tiers change under the rules above. At 1.0 the stable
tier becomes the 1.x promise (below).

A stable namespace may hold experimental functions (`experimental_fns`),
and a stable spec file experimental sections (`experimental_sections`,
by their `## ` heading). A function whose name begins with `__` is a
compiler hook, never surface. The graph registry's Reserved families
(`spec/registry.md`) are experimental until they have a producer.

A new namespace or spec file declares its tier in the same PR.
`crates/hale-types/tests/stability_tiers.rs` fails on one without a
tier, on a row that names nothing, and on a book table that has drifted.

## Keywords

The keyword list is pinned. [`spec/keywords.txt`](./spec/keywords.txt)
holds every hard and contextual keyword.
`crates/hale-syntax/tests/keyword_freeze.rs` fails when it differs from
`crates/hale-syntax/src/keywords.rs`, or when the lexer reserves a word
the list does not name, so a change to it is deliberate.

Before 1.0 a change needs no RFC. Adding, removing or reclassifying a
keyword needs a decision entry (an F-series entry in
[`spec/decisions.md`](./spec/decisions.md)) and a CHANGELOG fragment.
The implementing PR edits `spec/keywords.txt`, and lists the change
under Breaking when it can break a program. A new hard keyword breaks
every program that uses the word as a name. A new contextual keyword
(F.43) breaks only programs that use the word in its new position,
which is why a new word is contextual by default.

F.47 freed the five words once reserved for future use (`trait`,
`impl`, `async`, `await`, `macro`) and `export`, which is an annotation
name (`@export`), not a reserved word.

## Bugs and soundness

Every defect has an issue, so the defect surface is visible from the
issue tracker and not only from PRs:

- **`bug`**: something does not do what the spec or the docs say:
  a crash, a miscompile, a wrong answer, a misleading diagnostic.
- **`soundness`**: memory safety or semantic unsoundness, added
  alongside `bug`. Examples: a use-after-free or data race in a
  program the checker accepts, a law the checker states but does not
  enforce, or an ownership or lifecycle rule that a program can break
  without a diagnostic.

A soundness fix that refuses programs which used to build is listed
under Breaking, like any other break.

## What 1.0 promises

**Source compatibility for the stable tier within 1.x.** A program that
builds under 1.0 and uses only stable surface builds under every 1.x
release and means the same thing. Only bug fixes and soundness fixes
may change that, and each is listed under Breaking with its migration.
A 1.x minor release may add surface, deprecate a form, which then warns
for the rest of 1.x, and promote experimental surface to stable. It
never removes or changes stable surface. A removal waits for 2.0. The
experimental tier may still change in a 1.x minor release, listed under
Breaking.

**No editions.** Hale 1.x has one language, and a break waits for 2.0.
Editions exist mainly so a language can claim new keywords, and so
crates written against different revisions can link into one build.
Hale needs neither:

- Contextual keywords (F.43) let new syntax take a word without
  reserving it.
- Cross-seed imports are vendored source compiled by one toolchain
  (F.25), not separately versioned binaries.

An edition switch would double the checker's rule surface and the spec
for every rule it touches. If 2.0 comes, it ships with a mechanical
migration (`hale fmt`-style), not an edition flag.

1.0 does not promise ABI stability, the layout of the compiler's own
tables, the exact text of diagnostics, the artifact formats' internals
beyond their documented digests, or the toolchain's performance.

## The v1 exit gates

> **Draft.** These gates are being redone with the maintainer: what 1.0
> promises about stability, the performance targets it holds to, the
> platforms it supports, and its scope. Until that lands, the list below
> is the previous draft, kept for reference.

Hale is called 1.0 when all of these hold together:

1. **Three consecutive minor releases with no language or stdlib
   breaking change**: their `### Breaking` headings say `None.` for
   the stable tier.
2. **The parity gate is green**: the `parity` job in
   `.github/workflows/tests.yml`, which runs the workspace doctests and
   the test binaries that share process-wide state under libtest's
   threads.
3. **The full ownership and lifecycle matrices are green nightly**:
   `HALE_MATRIX=full` on `crates/hale-codegen/tests/ownership_matrix.rs`
   (1519 cells) and on `lifecycle_matrix.rs`, with every `KNOWN_OPEN`
   entry closed.
4. **The open questions this policy names are settled**: the reserved
   keywords (settled by F.47), and a tier for every namespace and spec
   file. The test keeps the second true.

GH #1950 tracks the gates.
