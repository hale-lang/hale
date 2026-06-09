# JSON parse — closing the V8 gap by inlining leaf primitives

Status: **scope / proposal.** Nothing built. Written 2026-06-09 after the
inline-parser (#93) + string-fast-path (#94) brought `from_json` to ~75ms
(200k × 7-field), beating Go ~2× and Python ~2.9×, **~1.47× behind V8's
JSON.parse (~51ms)**. This scopes closing that last gap — and the key
finding is that the lever is *not* a JSON-specific LLVM rewrite.

## Where the 75ms floor actually is

The generated parser (`__json_parse_<T>`, Hale source) is now inline — no
cursor structs, `Int` offsets, field dispatch unrolled. What remains is
that every leaf operation is a **C call**:

- `std::str::byte_at_unchecked(s, i)` lowers to `call lotus_str_byte_at_unchecked`
  — **a function call to read one byte.** The scan + dispatch + ws-trim do
  ~40–50 of these per parse. At ~4ns/call that's ~30ms of the 75 — roughly
  **half the total**.
- `std::str::range_eq` (key compare) and `std::str::range_parse_int`
  (value) are also calls — a handful per field.
- `std::json::next_struct_or_quote` / `next_quote_or_bs` / `next_non_ws`
  (the SIMD scan) are calls too, but **few** per parse (~5/field) and each
  does real SIMD work — leave them.

V8's JSON.parse is one tight C++ function with **zero** per-byte calls.
Our floor is the per-byte/per-token C-call overhead, not the scanning.

## The reframe: inline the leaf primitives, don't build a JSON IR-gen

The obvious-sounding move — emit the whole parser as hand-built LLVM IR —
is the wrong first step: it's large, JSON-specific, error-prone (re-coding
the scan/dispatch/number-parse in inkwell), and duplicates logic the
source-gen already gets right. The cheaper, more general, lower-risk lever
is to make the **hot leaf primitives lower to inline IR** instead of a
call, wherever they appear:

- **`byte_at_unchecked(s, i)` → `getelementptr` + `load i8`** (1–2 IR
  instructions, no call). This is trivial to emit and **general** — every
  byte-scanning routine in the stdlib + user code speeds up, not just
  JSON. Given it's ~half the parse time, this alone likely takes ~75ms
  toward ~40–45ms — *past* V8 — with no JSON-specific code.
- **`range_eq(s, a, b, lit)` → inline** a length check + a small
  fixed-length compare against the literal (the literal length is known at
  the call site), or an inlined `memcmp`. Removes the dispatch calls.
- **`range_parse_int(s, a, b)` → inline** a digit-accumulation loop
  (sign + `*10 + (c - '0')`), falling back to the C fn only on the error
  path. Removes the per-int-field call.

The SIMD `next_*` primitives stay calls. So this is "lower a few leaf
String/JSON primitives to IR," not "generate a parser in IR."

## Why this is the right shape

- **General, not JSON-only.** `byte_at_unchecked` inlining helps the
  object/array cursors, the pack readers, any hand-rolled scan — the whole
  byte-processing surface. JSON is just the loudest beneficiary.
- **Reuses the proven parser.** The source-gen `__json_parse_<T>` stays
  exactly as is; it simply compiles to faster code once its leaf calls
  inline. No re-implementation, so the existing json suite remains the
  full correctness net (no new code path to differentially fuzz).
- **Incremental + measurable.** Each primitive is independent; inline one,
  measure, decide whether the next is worth it.

## Staging

1. **Inline `byte_at_unchecked`** (`gep` + `load i8`). The big rock,
   trivially correct (the "unchecked" contract already says no bounds), and
   general. Re-measure `json_parse` — likely beats V8 here.
2. **Inline `range_eq`** against a string-literal RHS (known length →
   unrolled/`memcmp`). Re-measure.
3. **Inline `range_parse_int`** (digit loop; C fn on the fallible/error
   path). Re-measure.
4. **STOP when the bench says so.** If (1) already beats V8 on the target
   record shapes, (2)/(3) may not be worth the codegen surface.

## Only if (1)–(3) still don't reach the target: full IR-gen (Approach B)

Generate `__json_parse_<T>` as LLVM IR directly — the member loop,
length+content dispatch, value extraction, struct stores, and the
fallible result all as basic blocks, with `next_*` as the only calls.
This is the simdjson-grade endgame. It is a real, large, JSON-specific
lift (re-coding the scan in inkwell, a `FallibleCallResult`-shaped return,
nested-type recursion in IR, differential-fuzzed against the source-gen),
and it should not be attempted before the cheap primitive-inlining (1)–(3)
is measured — that may make B unnecessary.

## Risks

- **Correctness:** primitive inlining must be byte-identical to the C fn.
  The json suite + the str/bytes suites are the net; differential-fuzz
  `range_parse_int` (sign, leading zeros, overflow — match the C fn's
  exact behavior, including its overflow/`fallible` semantics).
- **Inlining `byte_at_unchecked`** trusts the caller's bound (that's its
  contract); a buggy caller that previously got a benign C-level read now
  gets a raw OOB `load`. Same UB the "unchecked" name promises; ASan on the
  test corpus guards the in-tree callers.
- **Codegen surface:** these become recognized-and-inlined intrinsics in
  the call lowering — keep the C fns as the fallback (non-inlined call
  sites, e.g. behind a fn pointer, still resolve).

## Recommendation

Do **(1) inline `byte_at_unchecked` and measure** before anything else.
It's a small, general codegen change that the analysis says removes ~half
the parse time, and it likely clears V8 on its own. Treat range_eq /
range_parse_int as follow-ons gated on the measurement, and the full
IR-gen (B) as a last resort that the cheap inlining probably retires.
