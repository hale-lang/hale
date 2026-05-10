# Aperio docs

This directory holds the Aperio language docs as five mdbook subtrees,
arranged as four onboarding paths plus the formal reference:

| Tree | Title | Role |
|---|---|---|
| `quickstart/` | **The Aperio Quickstart** | Five-minute install + hello-world tour. |
| `grimoire/` | **The Aperio Grimoire** | Magical onboarding — spell-cast register, four-moment arc (arrival → reveal → vocabulary → emergence). |
| `book/` | **The Aperio Programming Language** | Technical onboarding — substrate-up, Rust-Book-shaped layered tutorial. |
| `reference/` | **The Aperio Reference** | Formal grammar + semantics + glossary. |
| `std/` | **The Aperio Standard Library** | Stdlib roadmap + per-module docs (Phases 1–5). |

The grimoire and the technical book teach the same material in
different registers. Readers pick a doorway; the reference and std
trees are the destination either way.

Authoring conventions live in [`STYLE.md`](./STYLE.md).

## Status

**Local-only.** These docs are in active development and not yet
public. The CI workflow validates that `mdbook build` succeeds and
linkcheck passes; nothing is published.

When the language is ready for a public release, the deploy step lands
in `.github/workflows/docs.yml`. Until then, read locally with `mdbook
serve` (below).

## Building locally

Install mdbook and the preprocessors used by the docs:

```bash
cargo install mdbook mdbook-toc
```

(`mdbook-admonish` and `mdbook-linkcheck` are on the roadmap — both
currently have compat issues with mdbook 0.5.x. Add them back when
upstream catches up.)

Serve a subtree at `http://localhost:3000`:

```bash
mdbook serve docs/quickstart    # the five-minute tour
mdbook serve docs/grimoire      # the spell-cast onboarding
mdbook serve docs/book          # the technical tutorial
mdbook serve docs/reference     # the formal reference
mdbook serve docs/std           # the stdlib roadmap
```

(They use the default port 3000; serve them one at a time, or pass
`-p <port>` for multiple.)

To build static output without serving:

```bash
mdbook build docs/quickstart
mdbook build docs/grimoire
mdbook build docs/book
mdbook build docs/reference
mdbook build docs/std
```

Each tree's HTML lands in `docs/<tree>/book/`.

## Code blocks

Aperio code in docs is tagged `aperio`. Code blocks are meant to compile
under `aperio build`; CI doctest enforcement (the
`mdbook-aperio-test` preprocessor) is a follow-up milestone.

## Linking convention

First use of a glossary term on a page links to its
[glossary entry](./reference/src/glossary.md); subsequent uses are bare.
See [`STYLE.md`](./STYLE.md) for the full convention list.
