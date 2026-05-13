# docs/

Three doorways into Aperio. Each one serves a different
reader; each grows at its own pace. The braid that used to
live here (five mdbook subtrees under one `SUMMARY.md`) sits
in `archive/docs/`.

## The three doorways

### [`language/`](./language/) — Aperio as a programming language

The technical track. What Aperio is, how to write it, how the
substrate works. Substrate-up tutorial, reference grammar +
semantics, stdlib catalog.

For someone who wants to *write Aperio*. Came in cold; knows
Rust or Go or Zig; wants the language proper. Source of
truth: `../spec/`.

### [`bridge/`](./bridge/) — the ferryman flow

The migration track. Point ferryman at your existing codebase;
it emits yaml perspectives (operational / harmonic / domain);
an agent (or you) discusses the code in lotus terms; the
eventual mechanical rewrite produces Aperio source.

For a dev with an existing system they want to *see as lotus*
and migrate from. Go is the v0 target. Source of truth:
`../apps/ferryman/`.

### [`office/`](./office/) — spells × technology × multiverse

The philosophical track. The big idea, written in the register
of the Quiet Office — reconstructed annexes, concordance notes,
gnomic marginalia. Fiction-fused-with-essay rather than docs.

For the curious, the depth-seeker, anyone pulled in by the
larger frame. Seed: `archive/fiction/annex-l-7704.md`.

## archive/

Everything from before the reorganization.

- `archive/docs/` — the previous five-tree mdbook
  (quickstart, grimoire, book, reference, std). Still
  buildable; no longer canonical.
- `archive/fiction/` — `annex-l-7704.md`, the original
  Quiet Office sketch.
- `archive/future/` — room for later archive sweeps.

## Building

Each doorway will eventually be its own mdbook (separate
`book.toml`, separate theme, separate voice). All three are
empty for now; content arrives in purpose-driven sessions.
