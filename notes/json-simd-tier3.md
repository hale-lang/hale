# JSON Tier 3 — SIMD-accelerated parsing

Status: **scope / proposal.** Nothing built. Written 2026-06-09 after JSON
Tier 2 (`from_json` / `to_json`, scalar, schema-specialized) landed
(#84–#88). This scopes the SIMD substrate the cursor was designed as a
seam for — and, importantly, argues it's two separable levels, the first
much cheaper than "implement simdjson."

## Where we are

`from_json` drives the single-pass object cursor
(`__json_obj_step_span` in `runtime/stdlib/json.hl`), which scans the
input **byte-at-a-time** through a handful of inner loops:

1. skip whitespace / commas,
2. scan a key string to its closing quote (escape-aware),
3. skip whitespace + the `:`,
4. scan the value to its end — a depth + in-string loop that also lets
   unmatched-key values (incl. nested objects/arrays) be skipped whole.

The generated extractor sits *above* the cursor — it calls
`object_next` + the range accessors and is blind to how spans are found.
**That is the seam.** Tier 3 changes how the cursor finds spans; the
generated parsers don't change at all.

`to_json` is emit (string building) — SIMD is irrelevant to it. Tier 3 is
entirely a *parse-side* concern.

## SIMD applies at two levels — and they're very different sizes

**Level A — SIMD-accelerated scanning (small lift).** Keep the cursor's
structure; replace its byte-at-a-time inner loops with SIMD "find the next
byte in class X within the next 16/32 bytes" primitives:

- skip-whitespace → find first non-whitespace,
- key/string scan → find next `"` (then check the preceding byte for `\`),
- value-end scan → find next structural (`{ } [ ] : , "`) to drive the
  depth machine in jumps instead of one byte at a time.

No index buffer, no two-stage parse, **no change to the cursor's
representation or API** — its Hale loops just call a runtime
`lotus_json_simd_find_*(ptr, len, from) -> offset` instead of looping.
This captures most of the SIMD win for the typical document (tens to low
hundreds of bytes, a dozen fields) because those inner scans *are* the hot
loops. It is a contained, well-bounded change.

**Level B — full structural index (simdjson-style, large lift).** Stage 1:
scan the whole input in SIMD-width chunks and emit an index of every
structural character's offset (the "tape"), correctly ignoring structural
bytes inside strings. Stage 2: navigate the index — and crucially *skip an
entire nested object/array by jumping to its matching close* via the
index, never re-scanning its bytes. This is where simdjson's headline
numbers come from, and it pays off on **large** documents (KBs–MBs).

Level B is a different architecture: it needs an index buffer (allocated
per parse, lifetime-managed in the arena), the cursor reworked to consume
the index rather than scan, and the two-stage split. Level A is a
constant-factor speedup of the existing scan; Level B is an algorithmic
change that removes re-scanning of skipped regions.

## The genuinely hard part (shared by both, worse for B)

The **quote/escape mask**. To know which `"`—and which structural
bytes—are *inside* a string, you compute, per chunk, a bitmask of
"string interior" positions. That means finding unescaped quotes, which
means handling backslash runs (`\"` is escaped, `\\"` is not, `\\\"`
is, …) — classically a carry-less multiply (PCLMULQDQ) or a prefix-XOR
over the quote bitmask, with a carry bit threaded across chunks. This is
the part that is subtly wrong in naive implementations and the reason
SIMD JSON is a real engineering artifact, not a weekend optimization.

Level A needs a correct version of "find next unescaped quote" (simpler —
local backslash check at the boundary). Level B needs the full per-chunk
string-interior mask threaded across the whole document.

## Portability + correctness

- **ISAs:** SSE4.2 / AVX2 (x86-64), NEON (aarch64), and a **scalar
  fallback** (which we already have — today's cursor *is* the scalar
  path). Runtime CPU detection to pick AVX2 vs SSE, or compile-time
  target. The scalar fallback must always exist (non-x86/ARM, `-O0`,
  sanitizer builds).
- **Testing:** differential fuzzing against the scalar path — the SIMD
  index/scan must produce byte-identical results to the scalar one on
  arbitrary input. This is non-negotiable for a parser; budget for it.
- Lives as a new `lotus_json_simd.c` runtime (C-with-intrinsics, like the
  rest of `lotus_*`), behind the cursor — no Hale-visible surface change.

## The gating question — is there a large-JSON hot path?

SIMD JSON wins on **throughput over large documents**. Decide this before
building:

- Small records (an API response, a config object, a market-data tick of
  a dozen fields) — Tier 2 scalar is already fast; SIMD's setup cost
  (especially Level B's full index) may not pay back. A **size gate**
  (scalar below ~a few hundred bytes, SIMD above) is likely warranted.
- In *this* ecosystem the hot data path is the binary shm-ring interop,
  not JSON — JSON tends to be control/config/REST, which skews small. If
  that holds, Tier 3 is **premature**: real value needs a genuine
  large-JSON, high-throughput workload to point at.

So the honest precondition: **name the workload.** If there's a real
large-JSON throughput path, Level A is a cheap, high-leverage win and
Level B follows if profiling demands it. If JSON here is mostly small
control-plane data, Tier 2 is the right stopping point and Tier 3 is a
speculative investment.

## Staging (if a workload justifies it)

1. **Level A, scalar-validated.** Add `lotus_json_simd_find_structural` /
   `find_quote` / `skip_ws` (SSE/AVX2 + NEON + scalar fallback, runtime
   dispatch). Rewrite the cursor's four inner loops to call them.
   Differential-fuzz against the current scalar cursor. Ship — the
   generated parsers get faster with zero changes.
2. **Level B**, only if large-document profiling shows the per-step scan
   still dominates: the full structural index + arena-owned index buffer +
   index-driven cursor (skip-by-jump). Differential-fuzz the index against
   a scalar index builder first (its own internal seam), then SIMD-fill it.
3. **Size gate**: pick scalar vs SIMD by input length so small documents
   never pay SIMD setup.

## Recommendation

Don't build either until a large-JSON workload is named — and when one is,
**start with Level A.** It's a contained constant-factor win that reuses
the entire cursor + generated-parser stack unchanged (only the four inner
loops swap to SIMD find-primitives), and it sidesteps Level B's index
buffer, lifetime, and two-stage rework. Level B is the simdjson-grade
endgame, justified only by genuinely large documents where skip-by-jump
beats even a SIMD linear scan. Both keep the seam: the generated `from_json`
never changes.
