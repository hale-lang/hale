# Aperio docs

This directory holds the Aperio language docs as three mdbook subtrees:

- `book/` — **The Aperio Programming Language**, a layered tutorial.
- `reference/` — **The Aperio Reference**, formal grammar + semantics.
- `std/` — **The Aperio Standard Library** (placeholder; see roadmap).

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
mdbook serve docs/book          # the tutorial
mdbook serve docs/reference     # the formal reference
mdbook serve docs/std           # the stdlib roadmap
```

(They use the default port 3000; serve them one at a time, or pass
`-p <port>` for multiple.)

To build static output without serving:

```bash
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
