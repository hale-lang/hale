# Stability policy

Hale is pre-1.0. This page says what a release may change, how a change
reaches you, and what 1.0 will promise. It replaces the earlier rule
that nothing was promised before 1.0 (GH #1950). It binds the language
and the standard library. The toolchain's commands and the libraries
built in Hale (DNA, the habitat) follow the same release rules but
carry no 1.0 promise of their own.

## Releases

Hale's version is `0.MINOR.PATCH`.

**A patch release fixes bugs.** It never changes the language or the
stdlib's meaning for a correct program. It may:

- fix a crash, a miscompile, a leak, a hang or a wrong answer, so that
  a program does what the spec already says;
- refuse a program that never worked, such as a program the checker
  passed and the build then failed on, or one that crashed the compiler;
- improve a diagnostic, a performance profile, the docs or the tools.

A fix that changes what a correct program observably does waits for the
next minor release, even if the old behaviour was a mistake. Examples
are an exit code, a value a stdlib function returns, whether a store
copies, or the order of an output. "Correct" means the program builds
and does what the spec of its day says. A program that only worked
because of a bug the spec contradicts is not covered, but the fix is
still listed under Breaking when such programs are likely to exist.
Two patch releases broke this rule and are why it is written down:
SIGTERM's exit code moved from 143 to 0 in v0.32.8, and bounded
`push`/`set` started copying in v0.32.13.

**A minor release may break programs.** Each break follows the
deprecation rule below (stable tier) and is listed under its release's
Breaking heading with its migration.

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

## Deprecation

A form of the stable tier that is removed, or whose meaning changes,
goes through a warning before it is refused:

1. **Warn.** In the minor release that introduces the replacement,
   `hale check`, `build`, `run` and `test` still accept the old form.
   They emit a warning of the **`deprecated`** class: an ordinary
   non-fatal diagnostic (`DiagKind::Warn`) whose message begins
   `deprecated:`. It names the replacement and the release that will
   refuse the old form, for example `deprecated: ... write ... instead;
   v0.N+1 refuses this`. The LSP shows it at the use site like any
   warning. The change is listed under that release's Breaking heading
   as a deprecation.
2. **Refuse.** No earlier than the next minor release, the old form
   becomes an error. The error keeps pointing at the replacement. This
   release lists the refusal under Breaking again.

The window is at least one minor release. It may be longer, and the
warning says which release refuses. A deprecation never starts in a
patch release. The precedent is F.42: a `fn` that may violate and
returns nothing is warned about until a later release makes the rule
the law.

The experimental tier has no window. A change to it may land in any
minor release, and it is still listed under Breaking.

## Tiers

Every stdlib namespace and every file of `spec/` is **stable** or
**experimental**. The table is [`spec/stability.toml`](./spec/stability.toml).
The book renders it in [Stability & Compatibility](./docs/src/stability.md).

- **stable:** covered by the deprecation rule now, and by the 1.x
  promise at 1.0.
- **experimental:** may change in any minor release without a window.
  Each experimental row says why.

A stable namespace may hold experimental functions (`experimental_fns`).
A stable spec file may hold experimental sections (`experimental_sections`,
by their `## ` heading). A function whose name begins with `__` is a
compiler hook, never surface, whatever its tier. The graph registry's
Reserved families (`spec/registry.md`) are experimental until they have
a producer.

The first assignment puts the core language in the stable tier: types,
semantics, memory, forms, tokens, the grammar and the FFI. It also
holds the stdlib's everyday namespaces, among them `std::str`,
`std::bytes`, `std::json`, `std::time`, `std::http`, `std::io::*`,
`std::crypto` and `std::decimal`.

The experimental tier holds the areas that are new or still moving.
In the stdlib: `std::u256` and `std::i256`, `std::io::h2`,
`std::observe` and `std::metrics`, `std::api`, the internal transports
(`std::io::mirror`, `std::ring`, `std::shm`), and the DNA-facing
namespaces (`std::secret`, `std::tar`, `std::ts`). In the spec: the
unit dialect (`spec/units.md`), the api binding (`spec/api.md`), the
canonical model and its registry, `packages.md`, and DNA and the habitat.

A new namespace or spec file declares its tier in the same PR.
`crates/hale-types/tests/stability_tiers.rs` fails on one without a
tier, on a row that names nothing, and on a book table that has drifted.
Promoting an entry to stable is a `### Changed` entry in that release's
CHANGELOG. Demoting a stable entry is a break, and follows the
deprecation rule.

## Keywords

The keyword list is frozen. [`spec/keywords.txt`](./spec/keywords.txt)
holds every hard and contextual keyword.
`crates/hale-syntax/tests/keyword_freeze.rs` fails when it differs from
`crates/hale-syntax/src/keywords.rs`, or when the lexer reserves a word
the list does not name.

Adding, removing or reclassifying a keyword is a language change and
needs an RFC. An RFC is an issue titled `RFC: …`, discussed in the open,
and decided by the maintainers. Its decision is recorded as an F-series
entry in [`spec/decisions.md`](./spec/decisions.md). The implementing PR
edits `spec/keywords.txt` and lists the change under Breaking. A new
hard keyword breaks every program that uses the word as a name. A new
contextual keyword (F.43) breaks only programs that use the word in its
new position, which is why a new word is contextual by default.

The five words reserved for future use (`trait`, `impl`, `async`,
`await`, `macro`) have a proposed decision, F.47 in `spec/decisions.md`,
awaiting confirmation. It is settled before 1.0.

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

A fix for either may ship in a patch release (see Releases). A
soundness fix that refuses programs which used to build is listed under
Breaking, even in a patch.

## What 1.0 promises

**Source compatibility for the stable tier within 1.x.** A program that
builds under 1.0 and uses only stable surface builds under every 1.x
release and means the same thing. Only bug fixes and soundness fixes
may change that, and each is listed under Breaking with its migration.
A 1.x minor release may add surface, deprecate a form, which then warns
for the rest of 1.x, and promote experimental surface to stable. It
never removes or changes stable surface. A removal waits for 2.0. The
experimental tier keeps its rules inside 1.x.

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
   keywords (F.47), and a tier for every namespace and spec file. The
   test keeps the second true.

GH #1950 tracks the gates.
