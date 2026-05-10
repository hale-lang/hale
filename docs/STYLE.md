# Aperio docs style guide

Conventions for writing pages in `docs/book/`, `docs/reference/`, and
`docs/std/`. Following these makes the docs feel coherent across pages
and makes them work well for both human and AI readers.

## Aperio vs lotus terminology

The split is load-bearing:

- **Aperio** — the language. Source code, the compiler, the toolchain.
- **a lotus** — the runtime data structure an Aperio program *is*.
- **the Lotus** (capitalized) — the universal pattern. Used rarely; only
  when grammar demands the noun be capitalized.
- **a locus** (plural **loci**) — the unit of structure inside a lotus.

Use *Aperio* when describing language behavior at the source level, *a
lotus* / *loci* when describing runtime data structures. Mixing them
confuses readers.

## Per-tree register

Different docs trees use different tonal registers. Both are correct;
match the tree:

- **`quickstart/`** — terse, imperative, instruction-tone. No
  philosophy.
- **`grimoire/`** — meta-spell register. The load-bearing framing:
  *Aperio is the spell of spellcasting itself; the compiler is the
  wand; reaching across the source-runtime boundary is the act of
  spell-casting; each Aperio program is a particular spell now within
  the caster's reach.* The substrate's primitives (`locus`, `birth`,
  `bus`, `closure`, `on_failure`) are framed not as features but as
  the *invariant form every spell on the far side must take*. Magical
  metaphor never contradicts runtime semantics — the language was
  named precisely so the magical word and the technical word point at
  the same thing.
- **`book/`** — neutral systems-language tutorial register, in line
  with the Rust Book.
- **`reference/`** — formal, prescriptive. Synopsis / Grammar /
  Semantics / Examples / See Also. No metaphor.
- **`std/`** — same prescriptive register as the reference, scoped to
  library modules.

The "Avoid" list at the bottom of this guide (no marketing language,
no "we / our") applies to every tree, including `grimoire/`. Magical
register is not a license for hype.

## Headings

- Per-page H1 is the page title.
- H2 sections are major topics within the page.
- H3 / H4 are sub-sections; rarely H5+.
- Stable anchors on every heading: `### Some Section {#some-section}`.
  mdbook auto-generates anchors from heading text, but explicit anchors
  guard against rename churn.

## Glossary linking

First use of a glossary term on a page links to its glossary entry:

```markdown
... a [locus](../reference/src/glossary.md#locus) is the unit of structure ...
```

Subsequent uses on the same page are bare. This is a writing discipline,
not tooling-enforced at v1.

## Code blocks

Aperio code uses fenced blocks tagged `aperio`:

````markdown
```aperio
locus Hello {
    birth() {
        println("hello");
    }
}
```
````

Code blocks are meant to compile under `aperio build`. CI doctest
enforcement is on the roadmap (`mdbook-aperio-test` preprocessor); until
then, treat compilable as a writing discipline. Counterproductive:
pseudocode, fragments, `// imagine X here`.

For Rust, shell, and other languages, use the standard tag (`rust`,
`bash`, `text`). For raw text without highlighting, use `text`.

## Per-reference-page template

Reference pages follow this template:

```markdown
# Construct Name

> Since: v1.x  <!-- only on post-v1.0 features -->

## Synopsis

One paragraph on what this construct is and why it exists.

## Grammar

```
EBNF productions
```

## Semantics

Precise behavior. Compile-time and runtime.

## Examples

```aperio
// minimal compilable example
```

## See Also

- [Related construct](./related.md)
```

Predictable structure is load-bearing for AI affordance — readers (and
RAG retrieval) land on a single page and have full context.

## Versioning annotations

Features added after v1.0 carry a `> Since: v1.x` annotation right under
the H1:

```markdown
# `with backoff(...)`

> Since: v1.2

Modifies the `restart_in_place` recovery op...
```

## Self-contained pages

Avoid `(see §X.Y)` patterns. Inline the relevant context, or quote it
with attribution. RAG retrieval and AI use both prefer pages that stand
alone.

## Inline definitions

The first time a glossary term appears on a page, define it inline (one
phrase) AND link to the glossary:

> A [locus](../reference/src/glossary.md#locus) — the unit of structure
> inside a [lotus](../reference/src/glossary.md#lotus) — has a lifecycle,
> an arena, and ...

This trades a tiny redundancy for self-containedness. Worth it.

## Admonitions

mdbook-admonish (callout boxes) is on the tooling roadmap but not yet
enabled. For now, render notes / warnings / design-rationale callouts
as blockquotes with a leading bold tag:

```markdown
> **Note.** The substrate doesn't promise dispatch order across
> distinct subjects.

> **Warning.** Don't call `restart_in_place` from inside a closure body.

> **Design rationale.** Vertical-only-flow is F.8. The alternative —
> allowing lateral failure routing — creates ambiguity about which
> locus owns recovery.
```

When mdbook-admonish lands, this convention swaps to ` ```admonish ...`
syntax in one find-replace pass.

## File names

- Book chapters: `NN-slug.md` (numbered prefix matches SUMMARY.md order).
- Reference pages: `slug.md` or `subdir/slug.md` (no numbering — the
  reference is browsed by index, not read in sequence).
- Examples accompanying chapters: `docs/book/examples/chapter-NN/<slug>/`.

## Avoid

- "We" / "our" — neutral voice. The docs are the source of truth, not
  a conversation.
- "Of course" / "obviously" / "simply" — readers don't share your priors.
- Marketing language ("powerful", "elegant", "blazing-fast"). The docs
  describe; they don't pitch.
- Pseudocode in `aperio` blocks. Use `text` if it's not real code.
- Heavy linking ("see [X](...) and also [Y](...) for context") — link
  the first instance of each term, not every instance.

## When in doubt

Read the [Rust Book](https://doc.rust-lang.org/book/) and the
[Rust Reference](https://doc.rust-lang.org/reference/) for tone calibration.
Aperio's docs aim for that level of care; less marketing, more substance.
