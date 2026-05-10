# The Aperio Reference

This is the formal reference for Aperio (the language). For a tutorial
reading path, see [The Aperio Programming Language](../../book/book/index.html)
instead.

Each reference page follows the same template:

- **Synopsis** — a short prose description.
- **Grammar** — the EBNF productions that define the construct.
- **Semantics** — the precise runtime / compile-time behavior.
- **Examples** — minimal runnable Aperio code that exercises the construct.
- **See Also** — pointers to related reference pages.

Code blocks tagged `aperio` are valid Aperio source. They're meant to
compile under `aperio build`; CI enforcement is on the
[code-block-testing roadmap](../../book/book/01-why-aperio.html) for a
follow-up milestone.

## Versioning

Aperio is at v1. Pages documenting features added after v1.0 carry a
`> Since: v1.x` annotation under the heading. The full feature/version
matrix lives in the [grammar appendix](./grammar.md).

## Aperio vs lotus

This reference uses two terms with care:

- **Aperio** — the language being described.
- **lotus** — the runtime data structure an Aperio program instantiates.
  Used in this reference where the construct's semantics involve runtime
  behavior.

See [glossary: Aperio](./glossary.md#aperio) and [glossary: lotus](./glossary.md#lotus)
for the formal definitions.
