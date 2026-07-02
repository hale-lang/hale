# Bounded collections in types

Status: Stage 2 Option B SHIPPED for scalar elements (2026-07-02) —
`bounded[T; N]` in types + locus params, `{ i64 len, [N x T] }`
inline layout, push (fallible CapacityError { cap, count }) / at
(fallible IndexError) / count / clear intrinsics, `for x in f`
set/truncate (2026-07-02, second pass — drop-front/FIFO now
expressible), iteration, auto-empty init (literal init and whole-field assignment
rejected), flat under zero_copy for scalar T. Stage 1 SHIPPED same
day: pointer-shaped elements (String/Bytes/user structs) — push
arena-anchors elements into the receiver's owning arena
(self-rooted → locus arena, else current arena; the _ptr helper's
same-arena gates make re-anchoring idempotent), and struct copies
run a live-slot anchor loop ([0, len), no null-skip needed).
Scalar-element bounded travels the bus as flat bytes;
pointer-element bounded cross-process is post-v1 polish (focused
reject). The RouteParams/LlmRequest.messages TSV idiom is now
directly replaceable in-process. Stage 0 shipped earlier via
inline fixed arrays.

## The problem

Types can't hold collections, so pond encodes every list that
lives inside a `type` as a delimited string: `RouteParams` /
`LlmRequest.messages` / `Conversation.history` are tab-separated
walkers, `agent/embeddings` round-trips vectors through CSV. The
constraint is real — a type needs a fixed size — but the
workaround costs parsing on every access, forbids tabs in
payloads, and reads like a wire format where a value shape was
meant.

Riley's direction (2026-07-01): "a type needs a fixed size, so we
could probably be okay with a fixed-size collection."

## The design axiom to respect

Types are proto-loci: pure data, no flow, **no methods**. A
`v.push(x)` surface on a type field would either breach that or
need `push` to be "not really a method" the way `len(s)` isn't.
Both options below stay inside the axiom.

## Stage 0 — SHIPPED: scalar `[T; N]` fields are real inline data

Since the inline-fixed-arrays change, a counted buffer is just:

    type Recent {
        len:  Int;
        data: [Float; 32];
    }

with `r.data[i]` reads/writes in place, whole-struct copies
deep-correct by construction, and the shape flat under
`zero_copy` when T is scalar. This already covers the
numeric-vector cases (embeddings, price rings, histograms).

What it does NOT cover: (a) String/TypeRef elements — the layout
work deliberately scoped to scalar elements; (b) the operations
vocabulary — bounds discipline is hand-rolled per site.

## Stage 1 — PROPOSED: `[T; N]` for pointer-shaped elements

Extend the inline layout to `[String; N]` / `[SomeType; N]`
fields: N inline POINTER slots (same SSA story as today — reads
yield slot addresses, writes memcpy the pointer array). Costs:

- the deep-copy / anchor walk must iterate elements (String
  clone per live slot, struct anchor per slot) — the machinery
  task 3 skipped. `field_needs_anchor` returns true for these,
  and the anchor arm walks `len`-many slots... except the walk
  can't KNOW `len` (it's a sibling field). v1: walk all N slots
  with NULL-skip; require slots zero-initialized. This is the
  main implementation risk — needs the zero-init guarantee at
  literal/params init.
- stays NON-flat for `zero_copy` (pointers don't cross
  processes); the m70 wire codec already has the
  array-of-TypeRef serialize path, so bus payloads work.

This kills the TSV idiom for real: `RouteParams { n: Int; keys:
[String; 16]; vals: [String; 16]; }`.

## Stage 2 — PROPOSED: the vocabulary, two options

**Option A — free-fn vocabulary (no grammar change, ships in
stdlib).** Bounds-disciplined helpers over the counted-pair
shape, blessed in the styleguide:

    std::bounded::push(v.data, &mut v.len, x) -> Bool   // false at cap
    std::bounded::get(v.data, v.len, i) -> T fallible(IndexError)

Honest downside: `&mut v.len` isn't a thing Hale spells today —
the helpers would take the CONTAINING struct ptr + field
offsets, which is FFI-shaped, not Hale-shaped. Verdict: poor fit
unless the helpers are compiler-known.

**Option B — `bounded` as a TYPE-LEVEL form (recommended).**

    type Msgs {
        history: bounded[String; 64];
    }

`bounded[T; N]` is a compiler-known field type that lowers to
`{ i64 len, [N x T] }` inline. Because the compiler knows the
layout, it can synthesize the accessor surface as COMPILER
INTRINSICS (like `len(s)` / `sum(...)` — grammar-level, not
methods, so the type/locus axiom holds):

    push(m.history, x)      -> Bool          // false at cap (K/H7 displacement is the caller's policy)
    at(m.history, i)        -> T fallible(IndexError)
    count(m.history)        -> Int
    clear(m.history)

- Capacity is spelled in the TYPE — K made value-level, exactly
  the F.22 philosophy.
- Flatness: scalar T → flat (zero_copy OK, len travels in the
  bytes); pointer T → non-flat, wire codec serializes count-many
  elements.
- The alloc model treats a full `bounded` field as bounded by
  construction — no `--warn-unbounded-alloc` noise, unlike
  @form(vec) inserts.
- `@form(vec)` stays the unbounded, locus-owned collection; the
  docs sentence is: "unbounded data lives on a locus; bounded
  data can live in a type."

## Recommendation

Ship Stage 1 (pointer-element inline arrays + zero-init + anchor
walk) and Stage 2 Option B (`bounded[T; N]` + four intrinsics).
Estimated as one focused compiler session each. Pond migrations
that fall out: router::RouteParams, agent/llm messages,
agent/conversation history, agent/embeddings vectors (stage 0
already suffices for embeddings).

Surface decisions — LOCKED by Riley 2026-07-02:
1. Keyword: **`bounded[T; N]`**.
2. push-at-cap: **`fallible(CapacityError)`** (consistent with the
   fallible surface; callers that want H7 displacement write the
   policy in the `or` arm).
3. Iteration: **yes** — `for x in m.history` iterates live slots
   (0..count), same lowering as the @form iteration surface.
