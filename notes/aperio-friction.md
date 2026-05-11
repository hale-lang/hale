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

## 2026-05-10 cross-locus-return-deep-copy [FIXED same session]

**Source:** corpus-extraction migration (tower-join, operational-graph)
**Tried:** End a free fn with `return jb.wrap_array(inner);` after the body called another locus method (e.g. `ta.each_body(acc, tag)`).
**Hit:** Caller observes `""` for the returned String. Standalone callsites of `jb.wrap_array("")` work fine; the bug triggers only when the fn first calls a *different* sub-locus's method, then returns the second method's result directly. Reproduced minimally with `let bodies = ta.each_body(...); return jb.wrap_array("");`.
**Workaround:** None needed — fixed in the m49 free-fn epilogue (`emit_fn_exit_epilogue` in `crates/aperio-codegen/src/codegen.rs`). The epilogue used to run `flush_dissolve_frame()` BEFORE the return-value deep-copy; this freed any let-bound sub-locus arenas while `ret_alloca` still pointed into one of them, so the subsequent `lotus_str_clone` read freed memory (the freshly-freed page often still contained the right bytes for a single-locus fn, hiding the bug; a second sub-locus dissolve was enough to clobber the chunk). Fix swaps the order: deep-copy first (caller_arena is the parent caller's region, unaffected by this fn's flush), then flush, then destroy the per-call subregion. Returning `jb.wrap_array(...)` directly now round-trips through the caller boundary correctly.
**Why it matters:** Cross-locus composition is the std seed's whole point. The fix lets `__collect_types_with_motion` (apps/tower-join) and `__collect_section` (apps/operational-graph) return `jb.wrap_array(arr)` directly instead of inline `[ + arr + ]`. Verified byte-identical fixture output for both apps and all 312 workspace tests stay green.

## 2026-05-10 sink-as-tagged-locus

**Source:** ferryman render OOM fix
**Tried:** Define `std::text::Sink` with multiple destination implementations (stdout streaming, in-memory buffer for tests, eventually file) so the renderer threads `sink: Sink` through the walk and writes rows as produced instead of returning concatenated Strings up the recursion.
**Hit:** Aperio v0 has no interfaces / traits. The next-closest shape is one locus with a `dest: String` param branching inside every method: `if self.dest == "string" { self.buf = self.buf + s; } else { print(s); }`. Adding a third destination edits every method; the type system can't see which destinations exist; unused params (`buf` in stdout mode) sit in every instance.
**Workaround:** Wrote the tagged-locus version. Functionally correct, ergonomically poor.
**Why it matters:** Sink-shape polymorphism keeps recurring — `std::log::StdoutSink` is bus-coupled because there was no other way to abstract a destination; the renderer's OOM was caused by the same gap pushing it toward in-memory String accumulation. A real interface mechanism would let StdoutSink / StringSink / FileSink coexist as separate loci with one surface, eliminating the inner dispatch entirely.

## 2026-05-10 single-file-app-monolith

**Source:** ferryman render OOM fix (immediately after the Sink entry above)
**Tried:** Split `apps/ferryman/main.ap` (2,295 lines + ~290 uncommitted) into `skeleton.ap`, `render.ap`, `topology.ap`, `main.ap` — same `apps/ferryman/` dir, shared namespace, like Go's per-directory package.
**Hit:** Aperio has no per-directory package model. Each `.ap` file is its own translation unit; user code can't reference identifiers across files. The build is `aperio build apps/ferryman/main.ap`, one file in.
**Workaround:** Keep everything in `main.ap`. Threading Sink through ~30 functions in a single file is harder to review than threading it through four small files in a directory would be.
**Why it matters:** The stdlib itself already cheats around this — `STDLIB_AP_SOURCE = concat!(include_str!(...))` of 12 files in `crates/aperio-codegen/runtime/stdlib/` proves codegen can swallow multiple files as one seed; the constraint is a surface gap, not an implementation one. Surfacing it to user code (e.g., `aperio build apps/ferryman/` treats the dir as one seed) would unlock app-level file decomposition today and remove the perverse incentive to grow monolith `.ap` files. Compounds with the Sink friction above: missing interfaces + missing per-dir packages together push every "should be 4 small files with 3 implementations" toward one big file with one tagged locus.
