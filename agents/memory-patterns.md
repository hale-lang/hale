# Memory patterns — folded into the styleguide

This file's content moved into
[`spec/styleguide.md`](../spec/styleguide.md) (2026-07-17), which
is now the single author-facing guide:

- The mental model ("arenas don't free per-allocation" + the
  reclamation mechanisms) → styleguide §1 "The memory model in
  one page".
- The hot-path rules (in-place assignment, boot-time handle
  caching, reused buffers, inline returns, grow-path Strings) →
  styleguide §4 "Speed rules" + "What's already free".
- The leak-hunt diagnostic workflow (`LOTUS_ARENA_RESIDENCY`,
  `LOTUS_ARENA_LOG_CHUNK_ATTACH`, chunk-pool stats, composition
  recipe) → styleguide Appendix A.
- The closed-bug history table → `spec/memory.md` "Phase-4 perf
  follow-ons" (the normative substrate contract) remains the
  canonical record.

One pond-specific table (substrate primitives vs ASCII bridges:
`di.now_ns()` over `to_ns(monotonic())`, `df.to_float` over
parse-of-to-string) lives on in pond's own docs; the general rule
— substrate primitives over ASCII round-trips, always — is
styleguide material.

Read `spec/styleguide.md` §1 and §4 before writing hot-path
`.hl` code.
