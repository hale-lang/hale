# WS0 — stale-friction verification (post-audit hardening)

Date: 2026-06-11. Verifies the OPEN items in the consumer repos'
FRICTION docs against language-repo HEAD, per
`post-audit-hardening-handoff-2026-06-11.md`. Evidence sources
(read-only): the downstream issue tracker (last reviewed 2026-05-28 — STALE),
the downstream leak-hunting notes, the downstream app issue logs,
`pond/*/FRICTION.md`, `pond/CLAUDE.md`.

Method: candidate closing commits confirmed present in HEAD; for each
class, ran the in-repo regression gate and/or built a minimal `.hl`
repro and ran it under codegen. "Verdict" is empirical, not from the
doc markers.

## Headline

The **three candidate-closed classes the handoff named are confirmed
closed-or-diagnosed and gated.** Of the four WS1 soundness classes:
- **#1** (N≥3 hashmap children via fallible) — already closed + gated.
- **#2** (cross-seed Decimal-from-bus literal) — **now closed + gated**
  by the new F2 carrier; root was the 2026-05-20 i128-alignment fix.
- **#3** (wide-struct `.set`) — does **not** reproduce; not a width bug.
- **#4** (nested-locus reassign half-init) — does not reproduce across
  plain / birth() / **cross-seed** axes (new probe gates all three).
  The general reassignment path is sound; any residual is specific to
  `ws::WsClient`'s @ffi opaque-handle + network `birth()`, unmodellable
  in-repo without writing `@ffi` glue.

Net: of four reported "clean-compile → segfault" classes, three are
closed + gated at HEAD; the fourth's general path is sound + gated and
its only residual is one pond FFI type. The single-seed AND cross-seed
surfaces are clean. **The handoff's premise — "four open soundness
bugs" — does not hold at HEAD.** WS2's generator-driven sweep is
therefore low-yield against these classes (width / field-count /
Decimal / cross-seed reassignment all verified clean); if run, it
should target the one residual axis — an `@ffi`-opaque-handle locus
reassigned whole — rather than the value-shape axes.

## Candidate-closed classes (handoff WS0)

| Class | Closing commit(s) | Gate at HEAD | Verdict |
|---|---|---|---|
| async_io inline-instantiation / subscriber starvation | f5e82a7 (wake_fd poke), 7a22f7b (in-method-body coop-pool inheritance) | `async_io_park_resume`, `coop_pool_run_dispatch`, `placement_where_async_io` (7), `coop_to_pinned_mid_program` — all pass | **CLOSED** (pool-inheritance + park/resume). *Fixture gap:* the precise "inline child that carries its **own bus subscription**, instantiated inside a handler on an `async_io` pool" shape has no direct fixture — see Follow-up F1. |
| big-cell `@form(hashmap).set` fresh-alloc | cc090e4 (compound-pointer anchor) | `form_hashmap_codegen::hashmap_set_bigcell_with_array_field_does_not_leak` (array-field cell, chunk count flat across 400 same-key sets) | **CLOSED** for the *leak*. The a downstream app-cited *segfault* (a downstream service P2.2, 10-field struct) does **not** reproduce single-seed (see WS1#3) → not width-driven. |
| http::Server-as-child starvation | ab4fbdf (reject nested long-running coop children), 60d649b / 60e3007 / 99f352a (dead-receiver + blocking-syscall diagnostics), c8aeff1 (classic-pool shutdown) | `nested_long_running_child` (5), `http_server_classic_pool_shutdown` | **DIAGNOSED** — the shape is now a typecheck rejection, not a silent starve. Acceptable per handoff ("working or diagnosed"). |

## WS1 soundness classes — single-seed reproduction at HEAD

| WS1 | A downstream app shape | Single-seed repro result | Verdict |
|---|---|---|---|
| #1 — N≥3 `@form(hashmap)` children through `fallible(E)` | refdata/persist load_snapshot | `locus_fallible_return_multichild` (4 children, fallible + plain) passes | **CLOSED + gated** (fix: `instantiating_into_payload_arena` routes child literals to payload arena). |
| #3 — wide-struct `@form(hashmap).set` segfault | a downstream service OpenOrder 10-field | `/tmp/ws0_widecell.hl` (10 scalar) and `/tmp/ws0_widecell2.hl` (String + Decimal) both exit 0, correct reads | **Does NOT reproduce.** Not a cell-width bug. Folds into #2 (the downstream service OpenOrder is built cross-seed from bus data). |
| #4 — nested-locus param reassign half-init | mdgw-evm `self.conn = ws::WsClient {…}` | Single-seed plain + birth()-bearing pass; **cross-seed probe built** (`ws1_cross_seed_locus_reassign`, imported `wsx::Conn` with String params + a `birth()`-set Int handle): after whole-reassignment `url`=new value, `birth()` re-ran (`fd=7 ready=1`), `read_msg()` ran without crash. Passes at HEAD. | **Does NOT reproduce** across plain / birth() / cross-seed axes. The general reassignment-lowering path is sound. **Residual:** the downstream app failure is specific to `ws::WsClient`'s real **@ffi opaque-handle field + network `birth()`**, which the synthetic `Int` handle does not model. Definitive closure needs either the pond `ws` type (downstream verify) or a synthetic `@ffi`-opaque-handle locus (C glue) — see Downstream note. |
| #2 — cross-seed struct literal, Decimal fields from a **bus-deserialized** struct | a downstream service `gx::GreaseOrderRequest { px: oi.px, … }` from `d::OrderIntent` | **F2 built** (`ws1_cross_seed_bus_decimal`, 3-seed `d`+`gx`+app): 3 Decimal shapes round-trip the bus + cross-seed i128 literal intact, and the persisted locus-arena sum is bit-exact (`acc=112345.660001`). Passes at HEAD. | **CLOSED** — by the 2026-05-20 i128-alignment fix (`lotus_arena_off_for`, lotus_arena.c:1491). The a downstream app "flaky segfault" was the `movaps`-on-8-byte-aligned-i128 trap; the arena now aligns the real pointer. F2 is the deterministic gate. |

## The unifying hypothesis

#2, #3, #4 all share: **the source value crosses a boundary deep-copy
before the failing construction** — bus deserialization (#2, #3) or a
reassigned FFI-backed locus (#4). The single-seed paths that don't
cross that boundary all pass. So WS2's generator-driven copy-path
sweep should be **centered on the cross-seed + bus-deserialize +
FFI-handle axes**, not on cell width / field count (which are clean).

## Follow-up fixtures to add (the "closed but ungated" gap)

- **F1** — inline child carrying its own `bus { subscribe … }`,
  instantiated inside a handler on a `where async_io` pool, with a
  publisher; assert the child's handler fires. Closes the one fixture
  gap in the async class (the exact WsDispatcher/PerConn shape).
- **F2 (WS1)** — a 3-seed workspace: seed A publishes a struct with
  Decimal fields → seed B subscribes, deserializes, and constructs a
  seed-C qualified struct literal from those fields. The deterministic
  carrier for #2/#3. Build it **first** in WS1 (it is the regression
  gate before any fix).

## Commits confirmed in HEAD

f5e82a7, 7a22f7b, cc090e4, ab4fbdf, 60d649b, 60e3007, 99f352a,
c8aeff1, c242a71 (view-stale panic), 66dece5 (book/catalog refresh).
