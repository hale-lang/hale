# Precision refinements for `--warn-unbounded-alloc` (GH #18 item 1)

Status: **scoped** (2026-06-11). The path to making the memory-bound
warning precise enough to eventually be **default-on** — replacing the
rejected `@unbounded` escape-hatch idea (an escape hatch lets people opt
out of the substrate discipline the check exists to enforce; the right
move is to make the warning *right*, not silenceable). See the default-on
decision in [[memory-bound-proofs]] and the escape-hatch discussion that
killed `@unbounded`.

## What the warning actually does today (grounded)

`leak_sites` / `final_verdict` (`alloc_summary.rs`) flags a Struct / Array /
`[v; N]` / Bytes literal when **either**:

- it's inside an **unbounded loop** (`while true`, runtime `for`-iter, a
  non-const-counter `while`), **or**
- its owner fn is **unbounded-invoked** (a per-message bus handler, or
  reached through a call in an unbounded loop) **and** its reclaim scope is
  `EnclosingLocus`.

`EnclosingLocus` is **everything except a bus `Sent`** — so `Local`,
`Returned`, and `StoredToSelf` all qualify. There is **no escape filter and
no lifetime filter.** That is the imprecision.

## The framing correction (vs. my first sketch)

"Replace-vs-append reduces false positives" is **wrong** as stated. In a
bump arena a `self.f = new_value` *replace* also leaks resident memory —
the old value's bytes are never reclaimed until the locus dissolves (bump
pointers don't go backwards). So a replace in a *long-running* locus is a
**true positive**, not noise. The real false-positive sources are
different, and replace-vs-append earns its keep for *message precision* and
*runtime-fix targeting*, not for silencing.

## The actual false-positive sources

1. **Transient local scratch in a handler.** `fn on_msg(m) { let t =
   Struct{…}; use(t); }` — `t` never escapes. The method-scratch
   sub-region is reclaimed at method return (the leak that *was* there was
   fixed — see [[compiled-corpus-oracle-harness]]), so `t` does **not**
   accumulate across messages. But `final_verdict` flags it anyway
   (`EnclosingLocus`, owner unbounded-invoked). **This is the biggest false
   positive class.**
2. **Accumulation in a short-lived locus.** The same `self.x = alloc` is a
   leak in a long-running service and *fine* in a per-connection
   `accept`'d child that `release`s (its whole region is reclaimed when the
   connection ends) or a one-shot batch `main` (reclaimed at exit). The
   check is blind to the owner's lifetime.

A long-running-locus *replace* (the moving-average `self.window = [...]`)
is **not** in this list — it genuinely leaks resident memory and should
stay flagged. The author's "it's a bounded 4-window" belief is wrong about
the runtime; the fix is to route it over the bus, use a fixed slot, or the
runtime sidecar below — not to suppress.

## R1 premise — CONFIRMED (via codegen, definitively)

**Does a non-escaping local literal in a per-message handler accumulate
across messages? No.** Settled by reading the codegen rather than an RSS
proxy: a locus method / bus handler **opens a method-scratch subregion at
entry** (`open_method_scratch`, `method.rs:656`) and **destroys it at exit,
per delivery** (`emit_method_scratch_destroy`, `method.rs:728` — the comment
warns that skipping the destroy "leaks one subregion per delivery").
Transients allocate into the scratch and are freed per call; escaping
values are copied out to `self` / the caller first. So `Local` is reclaimed
per invocation; the `alloc_summary` model's `Local → EnclosingLocus →
accumulates` over-flags. **R1 is a soundness fix.** The in-loop case is
unaffected (a local in a loop *within one call* accumulates until the call
returns — RSS-validated — so R1 leaves the in-loop verdict path untouched).

## The three refinements

### R1 — escape-awareness on the cross-invocation path — **LANDED**

In the **unbounded-invoked** path only, require the site to **escape its
handler** (`StoredToSelf` / `Returned`, via `Escape::persists_across_calls`)
before flagging; a non-escaping `Local` is reclaimed per invocation (method
scratch). The **in-loop** path (`site.verdict() == AccumulatesUnbounded`) is
untouched. A one-clause change in `final_verdict`.

**Corpus impact: 4 → 3 warnings.** R1 removed a genuine false positive —
`fitter.hl`'s `on_observation` builds `let p = KernelPerspective { … }`,
calls `p.is_stable()`, and discards it (the publish sends
`self.latest_kernel`, not `p`). `p` is a transient reclaimed per delivery;
it was being wrongly flagged. The three survivors are all real
`self.X = …` stores (true positives). So the earlier "4 genuine flags, zero
FPs" claim was off by one — R1 found it.

### R2 — replace-vs-append on the escaping case (message precision + runtime-fix targeting)

Among `StoredToSelf` sites, split by a syntactic test on the assignment:
`self.f = E` is **grow/append** iff `E` reads `self.f` (e.g.
`self.buf = self.buf + chunk`, a push onto `self.f`); otherwise **replace**
(`self.window = compute(m)`).

- **Append** — incorporates all prior values → unbounded by construction.
  Warn; advice = "bound / cap / route."
- **Replace** — one live value, but a **resident leak** in a bump arena
  (the old copies linger until the locus dissolves). Still warn (it's a
  true positive), with a replace-specific message: *"one value is live, but
  the old one isn't reclaimed until the locus dissolves — use a fixed
  `capacity` slot, route over the bus."* Replace-of-a-fixed-size field is
  the runtime-fix candidate (below) — that's what makes it genuinely
  bounded, not a lifetime exemption.

### R3 — lifetime-awareness — **investigated, NOT built (unsound)**

The idea was: exempt accumulation in a short-lived / flow locus, since its
region is reclaimed when it dissolves. **It doesn't survive scrutiny — it
would introduce false negatives, violating "never a false bounded."**

The cross-invocation warning fires only on **per-message handlers**, which
belong to **bus subscribers** — long-lived by nature. The only short-lived
subscriber is a *flow* (an `accept`'d + `release`'d child). But a flow can
be **persistent** (a streaming connection): its handler runs unboundedly
over its life, and `self.buf = self.buf + chunk` accumulates the whole time,
reclaimed only at `release` — effectively never for a persistent
connection. That's a **real leak**, and whether a flow handles bounded vs
unbounded messages before release is **runtime data**, not statically
knowable. Exempting flows would hide it.

The genuinely short-lived loci (local guard loci à la `BytesBuilder`) have
no bus handler, so they're never in the flagged set to begin with. There is
no sound, tractable lifetime exemption for the cross-invocation case. (The
earlier "bounded by the connection lifetime" reasoning silently assumed the
connection is short — the same flaw as the original replace-vs-append
claim.)

**Conclusion:** the post-R1 warnings are genuine. The remaining lever for
fewer warnings + eventual default-on is **not** lifetime exemption — it's
making *replace* genuinely bounded via the runtime sidecar (in-place slot
reuse for fixed-size replaced `self` fields), then R2 recognizing it.

## How they compose

| site | verdict after refinements |
|---|---|
| non-escaping `Local` in a handler | **silent** (R1 — reclaimed per delivery) |
| `Local` in an unbounded loop | **warn** (accumulates within the call) |
| `StoredToSelf` *append* (`self.f = self.f + x`) | **warn** — unbounded by construction |
| `StoredToSelf` *replace* (`self.f = E`) | **warn** — resident leak; bounded only with the runtime sidecar (R2 message) |

(No lifetime column — R3 showed it can't soundly bound the cross-invocation
case.) The corpus's moving-average / fitter warnings are `StoredToSelf`
sites, so they **stay** (true positives) — with sharper messages. What goes
away is transient handler scratch (R1) and any short-lived-locus
accumulation (R3). That's the point: after the refinements the surviving
warnings should all be true positives, which is the precondition for
default-on.

## Runtime sidecar (optional, makes replace genuinely bounded)

A **free-on-overwrite / in-place slot reuse** for a *fixed-size* heap-typed
`self` field: allocate the field's storage once, overwrite in place on
reassignment. Turns long-running fixed-size replace (the moving-average
window) from a resident leak into a true constant. Variable-size replace (a
grown String) can't reuse a fixed slot and stays a real leak. With this,
R2's replace case can drop to "bounded" for fixed-size fields. Out of scope
for the analysis work; flagged here as the principled fix the replace
diagnostic should point at.

## Staging

0. **Validate** the handler-scratch reclaim question — **DONE** (confirmed
   via codegen; see above).
1. **R1** — escape-awareness on the cross-invocation path — **LANDED**
   (corpus 4 → 3, removed a transient-local FP).
2. ~~**R3** — lifetime classification~~ — **dropped (unsound)**; see above.
3. **R2** — replace-vs-append *messaging* (not suppression) — distinguish
   append (`self.f = self.f + x`) from replace (`self.f = E`) for a sharper
   diagnostic, and flag replace-of-fixed-size as the runtime-fix candidate.
4. **Runtime sidecar** — in-place slot reuse for fixed-size replaced `self`
   fields. The actual lever: it makes replace genuinely bounded, so R2's
   replace case can drop to "bounded."
5. **Re-evaluate default-on** — once the runtime sidecar + R2 land, the
   surviving warnings are all genuine unbounded growth (true positives) —
   the precondition for a default-on warning that needs no escape hatch.

## Risk

- **R1's premise is empirical** — if handler scratch is *not* reclaimed
  per-invocation, R1 is unsound and the model is right to flag locals;
  hence the validation gate. Don't ship R1 on the assumption.
- **Lifetime classification must stay conservative** — a misclassified
  long-running locus as "short-lived" would hide a real leak. Unknown →
  long-running.

## GH #1490 (2026-10-10): the audit's two largest false-positive classes

An audit of DNA's ~1,275 `@unbounded` decorators put ~22% on survey
false positives; the two largest classes were measured against the
runtime before the model moved.

**C2 — a returned loop accumulator** (`out = out + x` in the fn's own
loop, then `return out`). It was a false positive wherever the fn's
activation has a boundary: the superseded values die there, and the
value returned leaves once per call. Measured, 200k calls of a
20-iteration accumulator: 4.7 MB consumed in a method (scratch-local
callee), 4.6 MB (callee landing in its caller's arena), 4.7 MB (a
method callee), 9.2 MB in a bus handler against a 9.1 MB control —
flat; consumed directly in `run`'s loop, 33 MB (scratch-local callee:
only the results stay), 49 MB (method callee), 1.9 GB (callee in
`run`'s arena: every iteration's value stays). The model now judges the
returned site once per invocation when the owner is not a scratchless
long-lived frame and the loop is not a literal `while true`
(`returned_loop_bounded_by_activation`), and GAP A decides by the
consumer; the scratchless case keeps its in-loop verdict. DNA: 76 more
decorators silence nothing anywhere (847 → 923), `hale check dna/host`
308 → 305 warnings.

**C3 — a `String` field replaced whole** (`self.f = <fresh String>`):
**not a false positive**, so the model did not move. The store retires
the replaced block for reuse at the activation boundary, but reuse is
by size (best fit within an 8-block probe window, up to size + size/4
+ 16 bytes; first fit below 16 bytes), and a reused block is retired
again at its new value's `strlen + 1`, so a block only ever shrinks.
Measured in a method, 1M replaces: lengths 7–8, `"" + i`, a fixed
length, or alternating 20/30 or 20/60 all stay at the 4.5–4.7 MB
floor; lengths 6–15 reach 19.7 MB and 6–55 reach 35.7 MB (2M: 66.8 MB,
linear, ~31 B per replace; the `Rec` arena alone 15.6 MiB at 500k with
`LOTUS_ARENA_RESIDENCY=1`); a bus handler, 500k messages, 24.5 MB
against 9.3 MB; directly in `run`'s loop 115 MB; an append
(`self.log = self.log + "x"`, 20k) 206 MB. The R2 framing above holds
for Strings: variable-size replace is a real leak. The same reuse
limit applies to Gap D's struct replace, which the model excuses: a
one-String-field struct replaced with lengths 6–15 grows 19.7 MB / 1M
and 35.0 MB / 2M (lengths 7–8: 5.2 MB) — a known false negative of
the flip, documented in spec/verification.md. The lever is the
runtime's reuse (keep a block's true size across reuse, or bucket by
size class), not the model.

**Step 3, the spelling** (structure-and-bounds plan): the survey
recorded a String `+` only where an operand was a literal or a name
whose declared type was `String`, and the second never matched (the
type parses as a primitive, which the name map does not hold), so a
chain without a literal was no site at all — `out = out + piece(i)`,
`self.buf = self.buf + chunk`, `acc = wrap(acc) + sep`. Recognition is
now typed from declarations (`StrTy` in `alloc_summary.rs`: params,
fields, annotated and initialized `let`s, `for` binders over String
arrays, fns and methods declared `-> String`, `to_string`, scoped as
written). This only adds sites, and C2's rule then judges the new
accumulators like the old ones. Measured over every DNA program: 19
warning lines more and none fewer — nine intermediate concatenations
(`out + f(x)` inside `out + f(x) + " "`) in loops of frames with no
boundary, beside the outer site already reported; four in the books
acceptance's `hex_of` (`out + digits[..] + digits[..]`, no literal at
all), a helper with no boundary of its own; four String field stores (`self.buf =
self.buf + chunk` in a test; two replaces in the NATS adapter and one in
a test, which step 4 makes store-latest); and `dna/host`'s
`unescape_arg`, two sites of an accumulator whose result lands in
`route_verb`'s frame once per argument word — real growth bounded by the
command line, now `@bounded_by(argv)` beside `parse_args`. No decorator
stopped silencing; three silence more (`GitJournal::landed`,
`Review::consider`, `Host::fleet_window`).

**Step 4, the replaced field — the runtime lever, then the model**
(structure-and-bounds plan). C3's measurement named the lever; it is
built. Once a locus replaces a `String` field, the field's values take
blocks from size classes of their own (`lotus_slot_alloc` in
`lotus_arena.c`: 16-byte steps to 128 bytes, then four classes per power
of two, to 16 MiB), each its class's whole capacity. A per-arena
capacity table (block -> class; allocated at the arena's first replace,
one entry per field value it keeps) lets the retire
(`lotus_slot_retire`) file the block at its real size whatever value it
last held, on its class's own list, where the next value of that class
takes it from the head — no probe window, no shrinking. The compound
store's anchor walk clones a struct's top-level `String` fields into the
same blocks (`lotus_str_clone_slot`), so Gap D is covered by the same
fix, by measurement. Design notes: the table is the smallest correct
record of a block's capacity — a String carries only its length, and a
header would have to be readable on every block a slot can hold,
including ones no field allocated (a field's first value, set at
instantiation); a block the table does not know is retired at its
length, as before. Nothing changes on the arena's fast path
(`lotus_arena_alloc`), and the general freelist (map cells, vec
elements, Bytes) is untouched. Cost: rounding a field value up to its
class (under 16 bytes, or a quarter above 128), and ~200 bytes of
malloc'd state per locus that replaces a String field.

Measured (self-RSS / max RSS, release build): 1M replaces in a method at
lengths 6–55, 35.5 MB -> 4.7 MB and 1.11 s -> 0.08 s; a one-String-field
struct at lengths 6–55, 36 MB -> 4.7 MB and 0.62 s -> 0.08 s; a bus
handler, 500k messages at lengths 6–55, 20.9 MB -> 5.1 MB; the steady
length, a two-String struct churn, a hashmap value churn and the C2
accumulator unchanged (4.7–5.1 MB, same times); all clean under
AddressSanitizer with the chunk pool off. The model then moved: a
concatenation that is the whole new value of a `String` field of `self`,
or a whole field of a struct replaced whole, and does not read the field
it replaces, is `retired_store` (store-latest, judged as Gap D: not
reported from a frame with an activation boundary; reported in a `run()`
loop, a scratchless frame, or as an append). DNA: 29 decorators now
silence nothing in any program (0 before); `hale check` warning lines
over every DNA program 590 -> 586 (the NATS adapter's two replaces, one
test's, and `Ui::discovered`); the survey sites the decorators silence
fall from 205 / 168 / 176 / 156 to 146 / 113 / 121 / 102 (host,
operations, api, core).
