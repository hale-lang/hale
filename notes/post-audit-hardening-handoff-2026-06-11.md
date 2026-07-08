# Hale post-audit hardening + docs-truth pass (language repo only)

You are working in ~/code/hale-lang/hale (v0.8.3+). This punch list comes from an
external audit of the public surface cross-checked against two consumer repos.
Constraints from CLAUDE.md apply: the spec is the canonical contract — any
user-visible behavior change updates spec/ in the same commit; record negative
results and scope decisions in notes/.

EVIDENCE SOURCES (read-only — do not modify these repos):
- the downstream issue tracker  — last reviewed 2026-05-28; STALE. Treat
  every OPEN entry as "unverified," not "open." Also: per-app FRICTION.md under
  apps/*/ and lib/venues/*/, and MEMORY-LEAK-HUNTING-GOTCHAS.md.
- ~/code/hale-lang/pond/*/FRICTION.md and pond/CLAUDE.md "codegen-v0 limitations".
Downstream-side and pond-side follow-up actions: do NOT implement; collect them in
notes/<this-session>-downstream-actions.md for separate handoffs.

NON-GOALS for this pass: no pond source changes; no new language features (in
particular, no closures-with-capture work); no perf work.

## WS0 — Stale-friction verification (do first; cheap; prevents wasted work)

For each downstream issue-tracker item marked OPEN, check HEAD for a closing commit
before doing anything. Known likely-closed candidates to verify against the
original repro shapes:
- async_io inline-instantiation / subscriber starvation → f5e82a7 (wake_fd poke),
  7a22f7b (in-method-body pool inheritance). Verify against the dashboard
  WsDispatcher/PerConn shape described in the downstream issue tracker.
- big-cell @form(hashmap).set fresh-alloc → cc090e4 (compound-pointer anchor).
  Verify it covers cells with fixed-size array fields (~2KB, [T; 100] × 2).
- http::Server-as-child starvation class → ab4fbdf / 60d649b / 60e3007 / 99f352a
  diagnostics + c8aeff1. Verify the refstore shape (subscriber-only sibling +
  server; main run() never fires) is now either working or diagnosed.

Output: a notes/ table — item, verdict (closed-by <sha> / still open), and a
minimal fixture for anything closed that lacks one.

## WS1 — Soundness: clean-compile → runtime-segfault classes (highest priority)

These four contradict the memory-model guarantee. For each: write the minimal
.hl fixture in crates/hale-codegen/tests reproducing it BEFORE fixing; fixture
stays as the regression gate.

1. Locus populated with N≥3 @form(hashmap) children returned through a
   fallible(E) fn: children 3+ corrupt (garbage len(), dangling storage).
   100% reproducible per the downstream report. Repro shape: a downstream persistence module
   header comment + apps/smoke-refdata-persist history (load_snapshot).
2. Cross-seed struct literal whose Decimal fields come from a bus-deserialized
   struct → flaky segfault (heap corruption signature). Repro shape:
   a downstream app a downstream service fwd_grease_order comment (FRICTION § a downstream service P2 item 1).
3. @form(hashmap).set with a wide struct cell (10 fields) → segfault
   (FRICTION § a downstream service P2 item 2). May be the same root as WS0's cc090e4
   verification — confirm or split.
4. Wholesale reassignment of a nested-locus param from a member fn
   (self.conn = ws::WsClient { ... }) → half-initialized locus, null fields,
   crash on first use (FRICTION § a market-data gateway item 1). Acceptable outcomes:
   make it work, or reject at typecheck with a clear diagnostic. Silent
   half-init is the only unacceptable outcome.

## WS2 — Systematic copy-path hardening (after WS1 roots are understood)

All four WS1 bugs live in boundary deep-copy machinery. Don't stop at the
instances — map the class. Build a generator-driven sweep (property test or
exhaustive small-shape enumeration) over:

    {fallible return, plain return, bus deserialize→literal, hashmap.set,
     nested-locus reassign} × {0..4 @form children, Decimal/String/Bytes/array
     fields, narrow..wide cells, same-seed/cross-seed types}

asserting round-trip integrity (write → boundary-cross → read-back equality)
under ASAN. Gate it in CI like the GenMC job. Record coverage + any new
findings in notes/. If the sweep is too expensive for CI, land a reduced
nightly tier and say so in the workflow file.

## WS3 — Codegen/typecheck gaps (each: fixture + fix + spec touch)

1. std::math::int_to_float / float_to_int unimplemented in expression position.
   Every numeric consumer round-trips through ASCII today. Ship the trivial
   sitofp/fptosi lowering; spec/types.md numeric-coercion section gets the
   explicit-conversion story.
2. Nested `if` as a block's tail value yields () (outer then-arm types as Unit).
   `let x = if a { if b { p } else { q } } else { r };` must typecheck.
   Contradicts docs/basics "if is an expression."
3. Bus-topic file-locality: `publish T` fails when the topic decl lives in a
   different file/seed than the publisher. This forces single-file libraries
   downstream. Either lift it (preferred — topics are top-level decls; make
   cross-file/cross-seed references resolve like types) or spec it explicitly
   with the literal-subject idiom as the blessed workaround.
4. Two-hop cross-seed imports (downstream "G34"): qualified-name struct
   literals / qualified types in expression+return position break codegen when
   A imports B imports C. Repro shapes in pond/_util/README.md and
   pond/jobs/FRICTION.md § 11. Fixture with a 3-seed workspace.
5. shm_ring subscriber instantiated as a nested locus-param silently no-ops
   (no reader thread). Make it a typecheck error naming the constraint, or
   wire it. Silent no-op is the bug (FRICTION § shm-ring remaining nit).

## WS4 — Stdlib: std::db::sqlite::* primitives

pond/sqlite/FRICTION.md contains the exact requested signatures (open/exec/
query cursor/finalize, fallible(SqliteError)). Ship them (link -lsqlite3,
same pattern as -lcrypto), with hale-side integration tests. Do NOT touch
pond/sqlite — just unblock it.

## WS5 — Docs-truth pass (docs/src + AGENTS.md + spec; no behavior changes)

1. NEW chapter "Operations & debugging" in the book's services or systems
   tier: LOTUS_BUS_LOG_DROP, LOTUS_BUS_LOG_UNMATCHED, LOTUS_ARENA_RESIDENCY,
   std::process::dump_pool_residency / dump_arena_residency / rss_bytes,
   --dump-alloc-summary / --dump-resource-budget / --locality-report, and a
   worked "my publish isn't arriving" + "my RSS is growing" triage walkthrough.
2. Pattern-catalog additions (AGENTS.md + a docs page), distilled from
   production usage (describe shapes generically; no a downstream app references):
   a. The three-locus gateway: pinned reader → cooperative manager that
      accept()s → keyed per-entity children (subscribe ... where key == self.id).
      This is the canonical answer to "N dynamic keyed children with
      lifecycle" and to the locus-in-hashmap rejection.
   b. Demand-driven discovery: subscribe-triggered accept() spawning; zero
      hardcoded topology.
   c. Hot-path counters/gauges: document the #18.6 CQRS rejection (locus
      methods returning locus values) WITH its migration path — pre-allocated
      handles at boot, or bus-routed single-writer store relying on the
      closed-world rewrite. The rejection without the replacement pattern
      strands users.
   d. The publish-policy gate (tick() with time/volume triggers).
   e. View-lifetime rule for span/JSON APIs: a view is valid until the next
      recv/overwrite; copy out to persist (now panic-guarded via c242a71 —
      document the panic message users will see).
3. Demote modes and perspectives: keep the chapters but add a clearly-worded
   status banner (designed + parsed + spec'd, not yet exercised by real
   workloads; surface may evolve). Same banner style for where async_io
   anywhere the dormant gating still applies — verify first.
4. libraries.md catalog refresh: subprocess is NOT a placeholder; add
   agent/, ml/, math/, term/, tui/ entries; mark sqlite/jobs/migrations as
   blocked-on-WS4 until it ships (then flip). Check whether 66dece5 already
   covered some of this before editing.
5. Sweep docs/spec for the enum-payload status mismatch: a codegen comment
   says payload-bearing enum variants are deferred while docs/everyday teaches
   Event::Trade(price, size) matching. Determine which is true at HEAD; fix
   whichever artifact is stale.

ORDER: WS0 → WS1 → (WS3.1, WS3.2 are small, can interleave) → WS2 → WS3.3-5 →
WS4 → WS5. WS5 can start in parallel once WS0's verdicts exist (the ops
chapter depends on knowing which diagnostics are current).

DONE means: every WS1 item has a committed fixture; WS2 sweep is in CI at some
tier; each WS3 item is fixed-or-spec'd (no silent gaps); WS4 primitives pass
integration tests; WS5 pages render in the book build; spec/ and docs/ changes
landed in the same commits as their behavior changes; downstream-actions note
written for the pond/a downstream app handoffs.
