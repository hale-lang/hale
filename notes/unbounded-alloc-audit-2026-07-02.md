# --warn-unbounded-alloc false-positive audit (M3 stage 5)

Date: 2026-07-02. Scope: every .hl dir in pond + fathom apps/lib +
hale examples (261 dirs). Method: fresh-context agent, 7 parallel
triage passes, EVERY warning read-the-code triaged, calibrated
against codegen ground truth. Per-warning record:
notes/audit/merged.tsv + findings.txt.

## Result: 402 warnings — 103 TRUE (26%), 299 FALSE (74%)

VERDICT (audit time): not clean enough to default-on. UPDATE same day: gaps A, B (+while-true refinement), C fixed — ~402 → ~165 warnings, all audited TPs preserved. Remaining to flip: len()/param loop bounds; D accepted (population domains unknowable without annotations); E/F accepted as documented limitations. Prerequisite gap fixes
below; projected residual after A+B+C+D ≈ 26 warnings (~6%) against
103 genuine findings.

## True positives worth acting on (production leaks, live today)

- TP-1 free-fn/run-loop scratch accumulation (41): pond/tui event
  structs leak per frame tick/keypress for the session lifetime;
  pond/jobs pool.hl:194.
- TP-2 populations without eviction (9): riskgw open_orders
  (tombstoned but never deleted), fills_seen, recent_terms (window
  checked, never pruned); ledger fills_seen + attr.
- TP-3 per-set anchor-clone (53): every hashmap.set / compound
  self-store with fresh String subfields leaks the old clone —
  riskgw marks.set per md frame, dashboard wireskew, prober
  mark_set, websocket last_message.kind per message. SAME MECHANISM
  as the 2026-05-25 kraken bigcell OOM. Filed in FRICTION.md as a
  runtime issue — an arena-side fix (in-place String reuse for
  same-shape re-anchors) would moot ~half the TPs.

## Classifier gaps (fix order by impact)

- **Gap A (78 FPs)**: `Returned` values consumed inside MEMBER FNS
  (per-call scratch) are marked accumulating —
  `persists_across_calls()` returns true for Returned
  unconditionally and `unbounded_invoked()` propagates through
  every call edge without tracking the consuming frame. Fix: a
  return consumed in a member fn is reclaimed; only returns flowing
  transitively into run/main loop bodies accumulate. Also sharpens
  the TP-1 story (tui/jobs are exactly the run-consumed case).
- **Gap B (155 FPs)**: ForIter/While loops are unconditionally
  unbounded (only const-init/const-ceiling WhileCounter proves
  bounds). len()/param/field-bounded loops over one message or one
  config file all flag. Blunt fix that kills most: a Local alloc in
  a loop inside a fn that RETURNS (not run/main) reclaims at method
  exit.
- **Gap C (23)**: in-place self-field assignment
  (emit_self_field_inplace_assign memcpy, lotus_str_assign_in_place,
  static-literal subfields) modeled as accumulation.
- **Gap D (17)**: all-scalar-cell hashmap/vec set over a bounded
  key domain overwrites in place — no anchor alloc.
- **Gap G (6, HARD REQUIREMENT)**: bounded[T; N] must never warn —
  conversation.hl:110 (the bounded eviction shift loop) warns today.
- Gap E (13, ship as documented limitation): no program-lifetime
  model (one-shot smoke binaries).
- Gap F (7, ship as documented limitation): return-then-publish
  aliasing (payload arena reclaims per dispatch).

Also: soften the diagnostic wording for INVOKED-reason sites —
"accumulates until the locus dissolves" is factually wrong for
method-scratch locals.
