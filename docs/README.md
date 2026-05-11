# Aperio docs

The Aperio language docs are a single mdbook served from
`docs/`. Sources live under `docs/src/`; build output lands at
`docs/book/` (gitignored).

Five sections, four onboarding paths plus the formal reference,
all in one TOC:

| Section | Role |
|---|---|
| **Quickstart** | Five-minute install + hello-world tour. |
| **The Grimoire** | Magical onboarding — spell-cast register, four-moment arc (arrival → reveal → vocabulary → emergence). |
| **The Aperio Programming Language** | Technical onboarding — substrate-up, Rust-Book-shaped layered tutorial. |
| **Reference** | Formal grammar + semantics + conventions + glossary. |
| **Standard Library** | Stdlib roadmap + per-module docs (Phases 1–5). |

The grimoire and the technical book teach the same material in
different registers. Readers pick a doorway; the reference and
std sections are the destination either way.

Authoring conventions live in [`STYLE.md`](./STYLE.md).

## Status

**Local-only.** These docs are in active development and not yet
public. The CI workflow validates that `mdbook build docs`
succeeds; nothing is published.

When the language is ready for a public release, the deploy step
lands in `.github/workflows/docs.yml`. Until then, read locally
with `mdbook serve` (below).

## Building locally

Install mdbook and the preprocessors used by the docs:

```bash
cargo install mdbook mdbook-toc
```

(`mdbook-admonish` and `mdbook-linkcheck` are on the roadmap —
both currently have compat issues with mdbook 0.5.x. Add them
back when upstream catches up.)

Serve the book at `http://localhost:3000`:

```bash
mdbook serve docs
```

Or build static output without serving:

```bash
mdbook build docs
```

HTML lands in `docs/book/`.

## Code blocks and syntax highlighting

Aperio code in docs is tagged `aperio`. Code blocks are meant to
compile under `aperio build`; CI doctest enforcement (the
`mdbook-aperio-test` preprocessor) is a follow-up milestone.

Aperio syntax highlighting is provided by a custom
`docs/theme/highlight.js` — a bundle based on mdbook's default
highlight.js with an Aperio language module appended. See
[`docs/theme/README.md`](./theme/README.md) for how to rebuild
it when mdbook upgrades or the keyword set changes.

## Linking convention

First use of a glossary term on a page links to its
[glossary entry](./src/reference/glossary.md); subsequent uses
are bare. See [`STYLE.md`](./STYLE.md) for the full convention
list.
