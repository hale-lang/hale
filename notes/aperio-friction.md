# Aperio friction log — global

> Append-only. Each entry is a real moment where the language
> got in the way of writing what should be a correct program.
> The compiler session reads this file at the start of each
> iteration and uses it to triangulate priorities for the next
> milestone.

This is the **global** friction log — entries that came up
across sessions, or that don't belong to any one app. Per-app
logs live at `apps/<name>/FRICTION.md`.

## Format

Each entry is one Markdown section, dated, with a short tag.
Don't reformat or rewrite earlier entries. Append.

```
## YYYY-MM-DD <short-tag>

**Source:** <session or app name>
**Tried:** <one sentence: what you wanted to write>
**Hit:** <one sentence: what happened — error message, missing primitive, surprising semantics>
**Workaround:** <one sentence: what you did instead, or "blocked">
**Why it matters:** <one sentence: what feature this gates, or "minor papercut">
```

## What counts as friction

A friction entry describes a moment where the language or
stdlib resisted writing a program a competent reader would
agree should be writable. Three flavours:

- **Missing primitive.** "I needed X; X does not exist."
- **Surprising semantics.** "I wrote what I thought was right;
  it compiled but did the wrong thing." (Especially valuable.)
- **Friction in shape.** "I wrote what I needed; it works; but
  the path I took feels wrong, and I want a record of it
  before the next person rediscovers it."

What is **not** a friction entry:

- A bug in your own program logic that the compiler caught.
- A stylistic preference (e.g., "I wish `let` was `var`").
- A general feature wish disconnected from a specific moment of
  resistance ("Aperio should have generics" — yes, we know;
  log when generics' absence blocked a *concrete program*).
- A bug report against the compiler (file those as compiler
  issues, not friction).

## Entries

<!-- Append below this line. Do not edit existing entries. -->
