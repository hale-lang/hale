# Changelog

Behavior changes by release. The canonical spec lives in
[`spec/`](./spec/) — each file there represents *current*
behavior.

---

## Unreleased

- **Accept'd-child struct recycling — churn daemons no longer grow by
  sizeof(child struct) per child.** Interest-based ownership (v0.9.2)
  allocates an accept'd/bubbled child's locus struct in the owner's
  arena so `owner.__children` reads stay valid cross-lifecycle — but
  arena allocations are never individually freed, so a churn shape
  (one flow child per connection/message) leaked ~100–200 B per child
  *forever*, O(total children ever) instead of the O(peak alive) the
  F.3 free-list contract promises. Reclaim (flow run-completion,
  `terminate;`, parent cascade) now pushes the dead struct onto an
  intrusive per-owner free-list (`lotus_child_struct_release`);
  instantiation pops a size-matched block before bump-allocating
  (`lotus_child_struct_alloc`). Covers both subregion-owning children
  and arena-elidable (empty-lifecycle) children. Measured: accept-churn
  at K=4M flat at 5.5 MB maxrss (was 443 MB). Resident children (no
  `release(c)` on the parent) still accumulate until parent dissolve —
  that's the documented flow-vs-resident semantics, not a leak.
- **Owner-arena child structs now allocated 16-byte aligned** (was 8):
  an accept'd child with a `Decimal` param could take a `movaps` trap —
  same genre as the 2026-05-20 arena-alignment fix.
- **Cross-seed locus-field whole-reassignment now takes the WS1#4
  lifecycle path.** `self.conn = wsx::Conn { … }` (qualified/imported
  RHS type) previously fell through the `segments.len() == 1` gate to
  the plain value lowering — the field ended up pointing at a
  method-scoped stack temp, the exact dangle WS1#4 exists to prevent
  (its cross-seed test only survived by benign garbage). Qualified
  paths now resolve through the import-rename table, same as
  statement-position instantiation.

## v0.9.2 — interest-based ownership (accept bubbling)

- **`accept()` now collects descendants, not just direct children — a locus
  bubbles to its nearest accepting ancestor.** When a locus `I{}` is instantiated
  somewhere its *direct* enclosing locus does not `accept(I)`, it now stitches to
  the nearest enclosing ancestor that does (innermost-wins), instead of falling
  through to a transient throwaway. A top-level `World` can `accept(Ship)` and
  collect every `Ship` spawned anywhere beneath it — past intermediaries that
  don't care about Ships — with no manual registration. It's the structural dual
  of the bus: where the bus is ephemeral *messaging*, this is ephemeral
  *ownership* (a live projection the ancestor iterates and reclaims).
  **Backward-compatible by construction:** innermost-wins picks the direct parent
  whenever it accepts, so no existing parent↔child relationship changes; the
  feature only *adds* an owner where a child was previously transient (the whole
  corpus is byte-identical with the feature on vs off). Ownership stays opt-in via
  `accept` — an `I{}` with no accepting ancestor is a transient locus, never an
  error. Resolution is fully static (no polymorphic instantiation → the
  closed-world graph fixes every owner edge at compile time; no runtime ancestor
  walk). Three tiers, each proven inert on shipped code and ASan-clean:
  - **Same-tower, singleton owner** — the owner (a `main locus` / `@export`) is a
    compile-time constant; bubbling lowers to direct pointer wiring + a projection
    append + the existing reclaim cascade. Zero runtime cost over direct parenting.
  - **Same-tower, multiple owner instances** — the owner pointer is threaded down
    the birth chain via hidden per-locus fields, giving **instance isolation**:
    two `World`s each collect only the entities in their own subtree.
  - **Cross-pool** — a consumer on a worker pool spawning into a main-thread
    registry. The child is born on the owner's thread via an async handoff over the
    bus queue (reusing the lock-free post+wake), so teardown stays the owner's
    same-thread cascade — no cross-thread reclaim. Necessarily **async
    fire-and-forget**: a cross-pool `I{}` may only be a bare statement; using the
    instance as a value is a compile error.
  `LOTUS_NO_OWNERSHIP_BUBBLE=1` disables the whole mechanism (used as the
  backward-compat differential).

## v0.9.1 — pinned-Decimal bus-payload alignment fix

- **Fixed a segfault when a pinned bus subscriber stores or does arithmetic on a
  received `Decimal`.** A `Decimal` (an inline `i128`, align-16) delivered to a
  *pinned* subscriber landed in an 8-aligned mailbox payload cell, so an aligned
  SSE access (`vmovaps`) `#GP`-trapped — silent UB on ordinary type-correct code
  in the hot path of any bus consumer carrying money. Root cause:
  `lotus_bus_cell_t.payload_inline` had only the cell's natural align 8 (its
  widest member is a pointer), and the pinned drain hands the handler
  `&cell.payload_inline` directly — whereas a cooperative drain copies into a
  16-aligned scratch, which is why only the *pinned* path crashed. (It looked
  flaky because at `-O3` LLVM scalarizes individual i128 *field* ops into
  misalignment-tolerant paired 64-bit moves, so only a whole-struct payload copy
  reliably tripped the aligned `vmovaps`.) Fix: force the mailbox cell to 16-byte
  alignment (one struct attribute makes every cell copy 16-aligned uniformly), and
  bump the two nested-struct wire-deserialize allocations from 8 to 16 (a latent
  trap for remote/cross-process payloads carrying a nested Decimal-bearing struct).
  The downstream "never hold a bus-received Decimal — `to_string` it at the seam"
  workaround is no longer needed. Regression test: `bus_decimal_store` — three
  pinned-subscriber cases (`@form(vec)` push, `@form(hashmap)` cell, plain `self`
  field) asserting the *exact* round-tripped values + an accumulated sum, ASan-
  clean; SIGSEGVs on the pre-fix compiler.

## v0.9.0 — lock-free bus, static dispatch devirtualization, native codegen

- **Lock-free bus messaging + static dispatch devirtualization — coordination
  is no longer the weak spot.** The pinned-locus mailbox and cooperative-pool
  queues are now lock-free MPSC rings (Vyukov bounded ring + signal-only-when-
  parked wake, genmc-verified) in place of the per-message mutex + `cond_broadcast`
  handoff; and statically-eligible local bus subjects (closed-world programs, no
  transport adapter / wildcard / cross-seed) skip the `g_bus_entries` registry
  scan + the runtime dispatch entirely — a *quiet* same-thread handler (mutates
  only its own `self`, no I/O, no republish) is lowered to a **direct synchronous
  call**, proven byte-identical to the deferred dynamic path by a differential
  test harness. Net on the bench grid (vs Go): `bus_dispatch` went from ~4× behind
  to **2.4× ahead** (1.79 ms → 196 µs), `bus_dispatch_cross_pool` from 1.6× behind
  to **1.26× ahead** (10.7 → 5.0 ms), `stream_aggregator` from ~23× behind to **1.9×
  behind** (5.26 ms → 436 µs), `pipeline_3stage` ~2.4× faster. Footprint trade-off:
  the lock-free rings **pre-allocate** their cap (~4.3 MB per pinned mailbox /
  cooperative pool at the default 8192) rather than growing — lower
  `LOTUS_BUS_QUEUE_CAP` for pinned-/pool-heavy programs (see `spec/runtime.md`).

- **Native-tuned codegen + O3 by default, with `--target-cpu native|baseline`.**
  A native `hale build` now tunes generated code to the host CPU (autovectorization,
  AVX-512 where the host supports it — carried via per-function `target-features`)
  and runs LLVM's aggressive (O3) pipeline. **Consequence:** native binaries are no
  longer portable across microarchitectures — build distributed artifacts with
  `--target-cpu baseline`, which pins a portable `x86-64-v3` (AVX2 + BMI2 + FMA).
  `wasm32` is unaffected (stays generic / O2).

- **`LOTUS_LTO=1` — opt-in full-LTO build.** Emits the Hale module as LLVM bitcode
  and compiles the lotus C runtime with `-flto`, so the arena bump-allocator,
  string helpers, and shm-ring fast paths inline across the TU boundary into the
  Hale-generated callers. A few percent on allocation/coordination-heavy code,
  neutral on vectorized loops (host tuning preserved via the function attributes
  above). Off by default — the LTO link is ~3-4× slower and requires `lld`; native
  non-sanitizer builds only.

- **Collection-op inlining, bounds-check elimination, non-allocating-method
  scratch elision.** `@form(vec)` / `@form(hashmap)` `.get` / `.set` / `.pop` /
  `.push` are inlined at codegen (typed GEP + load/store, no `lotus_*` C-call
  boundary); `v.get(i)` indexed by a counted-loop variable (`for i in 0..v.len()`
  with `v` unmutated in the body) drops the per-element bounds check and the read
  vectorizes; and a method proven non-allocating — now including one whose only
  reads are scalar fields of a struct parameter (e.g. a bus handler doing
  `self.sum = self.sum + s.value`) — skips its per-call arena subregion. On the
  grid Hale now leads Go on `form_vec_get` (3.2×), `form_vec_push` (3.8×),
  `vec_amortized` (4.2×), `fn_scratch_work` (8.7×), `json_parse` (2.3×), and ties
  on `form_hashmap_get`.

- **Fixed `String + Int` (and `to_string(Int)` / `to_string(Float)`) emitting
  empty under `--target wasm32`.** The wasm libc shim's `snprintf` was a
  no-op stub (`buf[0] = 0; return 0;`) on the assumption it only built
  diagnostic labels — but `lotus_str_from_int` / `lotus_str_from_float` /
  `lotus_str_from_duration` (the `to_string` / `+`-concat paths) format their
  result through it, so every interpolated Int/Float vanished on wasm while
  native was correct (`"n=" + 5` → `"n="`). Replaced the stub with a real
  minimal `(v)snprintf` (the wasm-only shim — native uses libc, untouched):
  `%d/%i %u %x/%X %c %s %p`, the `l`/`ll`/`z` length modifiers, zero-pad width
  (`%018llu`), and `%g/%f/%e` for doubles matching glibc's default `%g`
  (6 sig digits, `%e`/`%f` selection, trailing zeros stripped) — verified
  byte-identical to native for the decimal magnitudes app/protocol data uses
  (`1e-05`, `1e+06`, `0.0001`, … all match). It also returns the would-be
  length (C semantics), which the Decimal formatter relies on
  (`p += snprintf(...)`). Test:
  `tests/wasm_target.rs::wasm_string_int_concat_formats`.

  (A follow-up — see the next entry — fixed `Decimal` on wasm too, which
  this fix had surfaced as garbage.)

- **Fixed `Decimal` under `--target wasm32` (i128 builtins).** clang lowers
  `__int128` multiply / divide / →double to compiler-rt libcalls
  (`__multi3` / `__udivti3` / `__umodti3` / `__divti3` / `__modti3` /
  `__floatuntidf`), and Ubuntu's clang ships no `libclang_rt.builtins-wasm32.a`,
  so `wasm-ld --allow-undefined` turned them into imports the JS loader stubbed
  to 0 — every `Decimal` (the i128 mantissa at scale 9: arithmetic *and*
  `to_string` *and* `std::decimal::to_float`) came out garbage. The bundled
  wasm libc (`runtime/wasm/lotus_wasm_libc.c`) now **defines** those builtins,
  with bodies that use only 64-bit ops (32-bit partial-product multiply,
  shift-subtract divmod, `f64.convert_i64_u`-based i128→double) so they never
  recurse into the very builtins they provide. Decimal on wasm now matches
  native byte-for-byte (`5.0d`→`5`, `19.99d * 3.0d`→`59.97`, `10.0d / 4.0d`→
  `2.5`, `to_float(19.99d)`→`19.99`). Test:
  `tests/wasm_target.rs::wasm_decimal_i128_builtins`.

- **`@ffi("js")` marshals `Int` / `Duration` as a JS `number` (f64), not a
  `BigInt` (i64).** A Hale `Int` passed to a host import used to arrive in JS
  as a `BigInt`, forcing every handler to `Number(x)` before using it (and a
  host import returning `Int` had to hand back a `BigInt`). Now i64-class
  scalars cross the `@ffi("js")` boundary as f64: the runtime `sitofp`s args
  before the call and `fptosi`s the return, the import's wasm signature uses
  f64, and the JS handler sees a plain `number`. Trade-off: f64's 53-bit
  integer range — an `Int` beyond 2^53 loses precision across the boundary
  (pass it as a `String`/`Bytes` payload instead). Scoped to `@ffi("js")`;
  `@ffi("c")` keeps i64 (those resolve to linked C symbols expecting i64).
  Test: `tests/wasm_target.rs::wasm_ffi_js_int_marshals_as_number`. See
  `spec/ffi.md` § WASM host interface.

- **`std::math::round` / `std::math::trunc` — Float→Int with a chosen
  rounding mode.** Both return an `Int` directly: `round(f)` is round-half-
  away-from-zero (`3.7 → 4`, `2.5 → 3`, `-2.5 → -3`), `trunc(f)` is round-
  toward-zero (an alias of the existing `float_to_int`). `round` is the
  spelling numeric code wants when building an integer field from a Float
  quantity — previously there was a toward-zero conversion (`Int(f)` /
  `std::math::float_to_int`) but no rounding one, forcing the round into the
  caller (e.g. JS, for a wasm client). Both lower to pure LLVM — `fptosi`,
  plus a compare/select half-shift for `round` (no `llvm.round` intrinsic) —
  so they need **no libm symbol and no host import on the `wasm32` target**
  (unlike `floor`/`ceil`, which stay libm and return `Float`). Native +
  wasm32 covered by `tests/ws3_int_float_conversion.rs` and
  `tests/wasm_target.rs::wasm_round_trunc_host_free`. See `spec/types.md`
  § "Explicit numeric conversions" and the `std::math` row in
  `spec/stdlib.md`.

- **Fixed a use-after-free race in the TLS handle table.** `lotus_tls_connect`
  `realloc`s (and thus *moves*) the global handle table when it grows on
  connect, while `recv_into`/`recv_bytes`/`send_bytes` read
  `g_tls_entries[handle]` lock-free. A connect on one connection that crossed
  a growth boundary while a *sibling* connection was mid-recv/send indexed a
  freed base → a wrong/garbage SSL object on the other connection (presents as
  "a busy connection silently kills a quiet sibling after enough
  reconnect churn"). The handle→SSL/fd resolution now happens under the table
  lock — held only for the table read, never across the blocking
  `SSL_read`/`SSL_write`, so concurrent connections still proceed in parallel.
  Same class as the udp remote-table relocation race fixed in #19.

- **TLS recv/send timeouts + a distinguishable recv-timeout sentinel.** Added
  `std::io::tls::set_recv_timeout(handle, d)` / `set_send_timeout` — the
  handle-aware siblings of the `std::io::tcp` timeout setters (TLS connections
  are addressed by handle, not raw fd), wrapping `SO_RCVTIMEO`/`SO_SNDTIMEO`
  on the underlying socket. And `recv_into` (TCP + TLS) now returns `-2`
  ("timed out, retryable") rather than `-1` ("fatal") on a `SO_RCVTIMEO`
  timeout (TCP `EAGAIN`; TLS `SSL_ERROR_WANT_READ`), so a long-lived client
  can bound a blocking read and run connection-liveness work instead of
  hanging forever on a half-open connection. Backward-compatible (`-2` only
  arises once a recv timeout is set). This is the language-side prerequisite
  for the pond `WsClient` liveness fix — see
  `notes/ws-readmsg-liveness-handoff.md` and the corrected verdict in
  `notes/tls-concurrent-recv-starvation.md`.

- **Whole-value reassignment of a locus-typed field is now a lifecycle
  transition (post-audit WS1#4 — soundness fix).** `self.conn = WsClient
  { … }` from a member fn previously lowered the RHS locus literal as a
  scope-bound temporary: birth ran, the pointer was stored, then the
  temporary was dissolved at the method's exit — leaving the field pointing
  at a torn-down locus (closed `@ffi` handles / freed arena → use-after-free
  on next use; the fathom refgw-evm reconnect crash), while the old value
  leaked. It now reclaims the old instance (its `drain`/`dissolve` run) and
  constructs the new one into the owning locus's arena, owned by the field
  and not scope-dissolved. Clean-compile→segfault closed; regression-gated by
  `ws1_ffi_handle_reassign`. In-place mutation (`self.conn.url = …`) remains
  the cheaper path for "same instance, reconfigure." See `spec/types.md`.

- **Docs-truth pass (post-audit WS5).** New book chapters: *Operations &
  debugging* (the bus-drop / arena-residency / backpressure diagnostics with
  two worked triage walkthroughs) and *Composition patterns* (the three-locus
  gateway, demand-driven discovery, the hot-path-counter/CQRS-rejection
  migration, the publish-policy gate, the view-lifetime rule) — the latter
  also condensed into AGENTS.md. Catalog refresh: `libraries.md` adds
  `http`/`term`/`tui`/`agent`/`ml`/`math` and corrects the stale `subprocess`
  "placeholder" note. Corrected a stale "no-payload-only enums" comment in
  codegen and a "deferred" enum-pattern note in design-rationale — payload-
  bearing enum variants + exhaustiveness have shipped since (verified against
  fixture 45-enum-payloads). (Modes were left un-bannered: the audit's "not
  yet exercised by real workloads" premise is false — fathom's orderbook
  declares `mode bulk/harmonic/resolution`.)

- **SQLite stays a library, not a language primitive (post-audit WS4).** The
  audit proposed shipping `std::db::sqlite::*`; on review that's the wrong
  layer — a third-party database belongs in a library, and Hale already has
  the general C-ABI binding surface for it (`@ffi("c")`, "no stdlib expansion
  required to bind a new library"). No `std::db::*` was added. Verified the
  one capability a driver leans on that lacked a test — a `String` *return*
  from `@ffi` (C `const char *` → usable Hale String, for `column_text`) —
  and gated it (`ffi_string_return`). The pond-side `@ffi` recipe to build
  the driver (glue.c + extern decls + `link=["sqlite3"]` + fallible wrapper)
  is in `notes/sqlite-via-ffi-recipe.md`; pond/sqlite is unblocked now, no
  compiler change.

- **Nested-param shm_ring subscribers verified + gated (post-audit WS3.5).**
  An shm_ring subscriber instantiated as a nested locus param
  (`params { sub: Sub = Sub { }; }`) — including as a param of the main
  gateway locus — spawns its reader thread and dispatches correctly; it is
  not the top-level-only silent no-op pond reported. A new regression test
  (`shm_ring_nested_param_subscriber`) covers the gateway and
  intermediate-parent shapes.

- **Two-hop qualified-name literals verified + gated (post-audit WS3.4).**
  A qualified struct/locus *literal* in expression or return position inside
  an intermediate library — `b::Thing { ... }` / `b::SomeLocus { ... }` where
  `app → b → c` and `b` instantiates `c`'s types — resolves correctly at HEAD
  (the "G34" shape pond reported as blocking library composition). The
  existing three-hop test only covered qualified *types* and *fn calls*; a
  new regression test (`two_hop_qualified_literal`) locks in the literal
  position, single- and multi-file intermediate libs, through both
  `hale build` and `hale run`.

- **`hale run <dir>` resolves cross-seed imports (post-audit WS3.3).** A
  directory `hale run` now resolves `import "..." as ...;` directives and
  threads the path-rename table into codegen, exactly as `hale build <dir>`
  already did — previously it bundled the directory's files but silently
  dropped every import, so a directory-seed app importing a vendored library
  failed on `alias::Name` references (and a topic decl appeared to need to
  live in the same file as its publisher). `run` and `build` no longer
  diverge on imports. Cross-file bus topics (`publish T` / `T <- v` resolving
  a `topic T` from a sibling file) work across both. See `spec/projects.md`
  § `hale run` interaction.

- **Nested `if` as a block tail value (post-audit WS3.2).** A
  *value-producing* trailing `if` (every arm ends in a tail expression) is
  now the block's tail expression, so `if` composes as a block value:
  `let x = if a { if b { p } else { q } } else { r };` typechecks instead
  of failing with `then=() else=Float`. A side-effect `if` (no `else`, or an
  arm with no tail) stays a statement — behavior unchanged. Matches
  docs/basics "if is an expression." See `spec/semantics.md` § Expressions —
  `if` and block tails.

- **`std::math::int_to_float` / `float_to_int` (post-audit WS3.1).** The two
  named numeric conversions now lower in any expression position (`sitofp`
  widening / `fptosi` narrowing, round-toward-zero) instead of erroring with
  "unsupported in codegen v0." Previously numeric consumers round-tripped
  through ASCII (`to_string` + `parse_*`) to change a value's type. They're
  the same conversions as the `Int(x)` / `Float(x)` casts, just callable as
  functions. See `spec/types.md` § Explicit numeric conversions.

- **Bounded cooperative bus queue + backpressure (GitHub #125).** The
  cooperative bus dispatch queue no longer grows without bound. It's capped
  at `LOTUS_BUS_QUEUE_CAP` cells (default 8192 ≈ 4.5 MB; env-overridable,
  floor 64); once a single-threaded producer that outruns its consumer hits
  the cap, it **back-pressures** — draining the queue inline (running the
  oldest handlers) to make space — instead of buffering the whole backlog.
  A `birth()` publishing 2M messages went from ~1 GB resident to ~54 MB,
  every message still delivered. Side effect: the `bus_dispatch` microbench
  got *faster* (8.7 → 3.0 ms) — the bounded queue is far more cache-friendly
  than the old unbounded one. **Cross-pool (any → pinned) backpressure** is
  also in: each pinned locus's mailbox is bounded at the same cap, and a
  cross-thread producer that hits it blocks on a condvar until the pinned
  consumer drains (a 2M any → pinned flood: ~1 GB → 54 MB, no deadlock). The
  cross-*cooperative*-pool path (multiple drainers) still grows — a
  follow-on.

- **Memory-bound warnings on by default (GitHub #18 item 1).**
  `hale check` now emits unbounded-allocation warnings without a flag.
  They're **advisory** — they print but don't fail the build (only errors
  do); `--no-warn-unbounded-alloc` opts out. The analysis reached zero
  corpus false positives first: escape-awareness (a non-escaping local in a
  per-message handler is reclaimed at the per-delivery method-scratch
  destroy, so it isn't flagged) and loop-ranking (a `while v < N` const
  counter is proven bounded). The warning flags a value that's allocated in
  a per-message handler / unbounded loop, escapes, and accumulates until
  the locus dissolves — e.g. a whole-value field replace
  `self.f = Struct{…}`, which bump-allocates a fresh value each time. The
  fix it points at is **in-place mutation** (`self.f.x = v` /
  `self.a[i] = v`), a capacity-bounded `@form`, the bus, or a per-iteration
  child locus. The `22-moving-average` and fitter examples were updated to
  mutate in place.

---

## v0.8.3 — verification track, SHM-ring interop, fast JSON

The largest release since v0.8.0 (cumulative since v0.8.2). Four
headline arcs, no source-level breaking changes:

- the compile-time **verification track** (GitHub issue #18) — six
  candidate analyses, four built, one a substrate gate, one parked;
- **binary shared-memory-ring interop** — read/write foreign SHM
  rings by declaring their layout, plus `std::bytes` packing;
- a **JSON parse/emit performance pass** that lands near V8;
- retirement of the tree-walking interpreter and a new `std::term`
  primitive surface.

### Compile-time verification (GitHub issue #18)

The verification roadmap, addressed. The canonical catalog is the
new `spec/verification.md` (#47).

- **Bus-graph property checks (item 4)** — fully landed, runs by
  default. Interprocedural blocking-call detection (warning, #44),
  orphan-topic check (#45), bus-cycle warning + re-entrant
  sync-deadlock error (#46), backpressure check (#48), and bus
  subject type-mismatch (#49).
- **Race-completeness for substrate primitives (item 2)** — a GenMC
  model-checking gate (#50–53) over the lockfree hashmap, the
  pinned-locus mailbox, and the cooperative-pool bus queue under all
  C11 interleavings, wired into CI (#52). A substrate quality bar,
  not a user-facing check.
- **Memory-bound proofs (item 1)** — opt-in
  (`hale check --warn-unbounded-alloc`, `--dump-alloc-summary`). A
  per-method allocation summary + call-graph escape/loop dataflow
  (#100), an empirically-validated reclamation model (#101) that
  **corrected the spec** (#102 — value allocations live until the
  enclosing locus dissolves; free-fn returns do *not* reclaim per
  call), a bound solver with call-graph propagation (#103),
  call-result escape tagging (#112), and **loop-ranking** that proves
  a `while v < N` const counter bounded (#117). Kept off-default
  deliberately (#118) pending an `@unbounded` escape valve, since the
  warnings include legitimately bounded-by-design patterns.
- **Resource-budget tracking (item 5)** — opt-in. Static counts of
  pinned threads / cooperative pools / bus subjects / fd-acquisition
  sites (`--dump-resource-budget`, #111/#115/#116), a CI ceiling gate
  (`--check-resource-budget budget.toml`, #113), and fd-leak
  detection (`--warn-resource-leak`, #112).
- **Closure-assertion lifting (item 3)** — scoped and **deliberately
  parked** (#114): the tractable constant case is already handled by
  typecheck, and the remaining symbolic case is low-leverage for a
  niche feature.

### Binary shared-memory-ring interop

Read and write *externally-defined* binary SHM broadcast rings by
declaring their layout — no hand-written FFI.

- **`std::bytes` binary packing** (#55, #56) — bounds-checked
  little/big-endian readers (`read_u8` … `read_u64_{le,be}`, signed +
  float variants) and `BytesBuilder` writers (`append_u16_le` …
  `append_pad`).
- **`ring_layout` declaration** (#57) — a top-level decl describing a
  foreign ring's magic / version / cursor / framing / overflow; a
  `shm_ring(..., layout: N)` binding kwarg (#58) binds a topic to it.
  Read-only consumer (#59), producer (#61), and `ring_layout` ↔
  payload conformance checks (#60), cataloged in `spec/verification.md`
  (#66).
- **Raw `BytesView` payload mode** (#72, #77) — a bounded view per
  record for heterogeneous rings, with a symmetric producer path;
  native-ring `slots` framing reachable through the same abstraction
  (#75).
- **Go-style struct field tags** (#80) + **repr-tagged field
  accessors** (#81, #82) — direct typed field access over a raw frame
  at compile-computed offsets.
- **Zero-copy ring write surface** (#78, #79) — a reserve/commit split
  for writing records in place. OOB-hole fixes at the foreign-producer
  boundary (#67), under UBSan in CI (#68).

### JSON performance

A parse + emit pass bringing generated JSON codecs near V8.

- **Tier 2 — generated codecs from `json:` tags** (#84–88): a
  single-pass object-member cursor, `Type::from_json` (including
  nested structs), and a symmetric `Type::to_json`.
- **Tier 3 — SIMD** (#90–92): SIMD-accelerated object/array cursors
  with an AVX2 path for the scan primitives.
- **Inline leaf primitives** (#93–97): the generated parser inlined
  (no per-field cursor structs), the unescape copy skipped for
  escape-free strings, and `byte_at` / `range_eq` inlined to
  gep+load / direct compares. A representative parse went ~291 ms →
  ~58 ms — within range of V8.

### Standard library & runtime

- **`std::term` + raw byte I/O** (#108–110): `is_tty(fd) -> Bool`,
  `size() -> TermSize`, the `RawMode` guard locus (atexit-backed
  termios restore), `std::io::stdout::write_bytes`, and
  `std::io::stdin::read_byte` — terminal hygiene with no vendored FFI
  glue.
- **Interpreter retired** (#41, #42): `hale run` now compiles + execs
  via codegen; the tree-walking `hale-runtime` crate is deleted, so
  there is no interpreter/codegen parity to maintain.
- **Stale-view panic via `exit()`** (#106) so `atexit` cleanup (e.g.
  the `RawMode` restore) runs on a panic path.
- **`BytesBuilder.append_str`** (#105) + a clarified StringView
  non-coercion rule at `@ffi` params.
- **ECDSA P-256** gains a `fallible(CryptoError)` form (#43).
- **Locus method names no longer mangled** (#104) — fixes inline /
  `accept`'d loci referenced in method bodies.

### Language surface

- **CQRS at the locus boundary (#18.6 / #81).** Methods on loci
  may not return locus values. The compiler rejects
  `fn lookup(id: String) -> Counter` on a registry locus at
  typecheck. The rule keeps the substrate model honest — a
  returned locus would be a stranger in the caller's scope, with
  no lifecycle tower above it. Three canonical alternatives:
  parent-child + contract (`accept`'d children, pair with an
  index slot for name-based lookup), bus topic (publish typed
  commands keyed by name), or delegation (collapse the per-child
  operation onto the parent). See `spec/semantics.md § Locus
  method dispatch`.

- **`resets_per_epoch(...)` closure clause (F.34, #75).**
  Closes the `low_corrupt_rate`-shaped friction (per-window rate
  budgets). A closure paired with `epoch duration(N)` may now
  declare `resets_per_epoch(field1, field2, ...);` — the
  runtime zeros the named fields AFTER the assertion fires at
  each duration boundary. Ordering matters: the assertion sees
  the window's accumulated value, the reset prepares the next
  window. Typecheck rejects pairing with non-duration epochs and
  non-numeric fields. See `spec/semantics.md § Per-epoch field
  reset` + `spec/design-rationale.md § F.34`.

  ```hale
  closure low_corrupt_rate {
      self.corrupt_per_min ~~ 0 within 10;
      epoch duration(1m);
      resets_per_epoch(corrupt_per_min);
  }
  ```

- **Nested long-running cooperative children rejected at typecheck
  (#76 / F.31-followup).** A non-main locus with a non-trivial
  `run()` body holding a `params` field of a locus type whose own
  `run()` is also non-trivial — including `std::http::Server` and
  the other entries on the known-long-running stdlib allowlist —
  is now a compile error pointing at the sibling-in-main +
  placement fix. The runtime starvation that motivated this rule
  was silent (parent's `run()` simply never executed), so the
  type-side rejection converts a class of hard-to-diagnose
  runtime bugs into a clear compile-time signal. See
  `spec/runtime.md § Long-running cooperative children`.

### Diagnostics

- **`@form(hashmap)` cell-locus rejection improved (#77).** The
  pre-existing rule (cells may not be locus references) now
  produces a diagnostic that names the three canonical
  alternatives (parent-child + index, bus topic, delegation) and
  cross-references `spec/semantics.md § Locus method dispatch`.
  Same framing as #18.6 at the form-synthesis layer.

- **`LOTUS_BUS_LOG_DESERIALIZE_DROP=1`.** Surfaces silent drops
  in the udp:// reader thread when no deserializer is registered
  for the inbound subject, or the deserializer returns ≤ 0 (size
  mismatch, bounded-read failure). Off by default; the silent-skip
  on cross-routed multicast noise stays correct in steady state.
  Three udp:// bring-up handoffs this week traced back to silent
  drops on `deserialize → local-dispatch`; the lack of any signal
  was load-bearing on debug cycles. Same env-gated pattern as the
  existing `LOTUS_BUS_LOG_UNMATCHED`.

### Internals

- **Codegen refactor (#22).** `crates/hale-codegen` reorganized:
  per-domain submodules (`locus/`, `bus/`, `shared/`, `stdlib/`),
  `codegen.rs` reduced by 56.2%. No surface-level changes.

### Documentation

- **`docs/src/concepts/the-locus.md`** — CQRS rule paragraph.
- **`docs/src/concepts/the-bus.md`** — routing keys +
  `on_unmatched` policies (covering machinery shipped in v0.8.2).
- **`docs/src/concepts/capacity-storage.md`** — hashmap cell-
  locus rule with alternatives.
- **`docs/src/concepts/error-handling.md`** — `resets_per_epoch`
  coverage in the closures intro.
- **`docs/src/how-tos/threading.md`** — nested-long-running
  rejection in "What you can't do".
- **`docs/src/how-tos/keeping-memory-bounded.md`** — factory /
  cached-handle sections rewritten around the boot-time Int-
  index resolution pattern (the previous example used the
  now-rejected `reg.counter().inc()` shape).
- **`spec/design-rationale.md`** — new F.34 entry.
- **`spec/verification.md`** (new) — the canonical catalog of all
  static checks: the default bus-graph rules, the `ring_layout`
  conformance + geometry checks, and the opt-in memory/resource
  analyses (with the `--check-resource-budget` TOML schema).
- **`spec/memory.md`** — corrected to the shipped reclamation model
  (value allocations live until the enclosing locus dissolves;
  free-fn returns don't reclaim per call).
- **`spec/stdlib.md`** — `std::term` + `std::io::{stdin,stdout}` raw
  I/O rows; the `std::bytes` binary-pack reader/writer family;
  `BytesBuilder.append_str`.
- **`spec/ffi.md`** / **`spec/semantics.md`** / **`spec/grammar.ebnf`**
  — StringView non-coercion at `@ffi` params; the `ring_layout`
  declaration grammar + foreign-ring payload modes.
- **mdBook** — `systems/performance.md` gains a "Catching it at
  compile time" section (the analysis flags); `everyday/cli-config.md`
  gains "Interactive terminal I/O" (`std::term` / raw byte I/O).

---

## v0.8.1 — F.32 cache-aware substrate + #24 narrowing

Cumulative changes since v0.8.0. No source-level breaking
changes; one rule narrowing (open-question #24) lifts a
previous restriction.

### Language surface

- **`fallible(E)` on user-declared locus member fns**
  (open-question #24). The blanket "locus methods cannot
  declare `fallible(E)`" rule narrowed to "substrate-facing
  surfaces cannot." User-declared `fn` member fns now
  carry `fallible(E)` like free fns do, with the full `or
  raise` / `or <substitute>` / `or <handler(err)>` /
  `or discard` disposition surface. Heap-bearing success
  and err payloads (`String`, `Bytes`, nested-struct-with-
  heap-fields) are supported via the same TLS caller-arena
  snapshot non-fallible heap-returning locus methods use.

  Still rejected (substrate-facing surfaces, no caller
  frame to address the value channel): lifecycle methods
  (`birth` / `run` / `accept` / `drain` / `dissolve` /
  `on_failure`), mode methods (`bulk` / `harmonic` /
  `resolution`), closure assertions, and bus-subscribed
  handlers. Bus-handler rejection fires at the subscribe
  site, not the fn decl. See `spec/semantics.md`
  § "Where each channel lives".

- **`@locality(L1|L2|L3|any)` annotation on a locus**
  (F.32-2 v0.2). Pins a per-locus cache-tier budget the
  working-set estimator evaluates against. `any`
  explicitly opts out of any global gate. Stacks with
  `@form(...)` in either order; max one of each. See
  `spec/grammar.ebnf` § `locality_annotation` +
  `spec/types.md` § "Working-set estimator (F.32-2)".

### Cross-pool `@form(hashmap)` sync disciplines

The cross-pool exemption that admitted plain `@form(hashmap)`
loci into concurrent-write paths was found to corrupt the
runtime's hashmap on concurrent grow (`lotus_hashmap_set` /
`_grow` are non-atomic single-threaded code).

- **F.32-0**: cross-pool exemption reverted; plain
  `@form(hashmap)` is single-pool by default. Cross-pool
  use requires an explicit `sync = X` opt-in.
- **`sync = serialized`** (α): per-map mutex. Simplest
  correct cross-pool path.
- **`sync = striped`** (β2-v2): cell-level CAS + per-map
  rwlock for grow + cache-padded cells. Parallel writers;
  grow path serializes.
- **`sync = lockfree, cap = N`** (γ-v1): fixed-cap,
  cell-level CAS, no rwlock or mutex. Highest measured
  throughput on the false-sharing bench (1.30× over α at
  2 cores, AMD Ryzen 9800X3D); no grow, no remove.

Discipline-picker table in `spec/forms.md` § "Cross-pool
sync disciplines". Inference (closed-world picks one of
α/β/γ from the pool-propagation graph) lands as a
typecheck-diagnostic enhancement; explicit pasting still
required to apply (auto-apply deferred).

### Working-set estimator (F.32-2)

Compile-time analysis projecting each locus's bytes
against a cache-tier budget. Opt-in via:

- **`hale build --locality-report`** — informational
  per-locus table on stderr; build proceeds.
- **`hale build --target-cache l1|l2|l3`** — over-budget
  loci warn on stderr; build proceeds.
- **`hale build --target-cache lN --strict`** — over-budget
  loci fail the build before codegen (exit 1).
- **Per-locus `@locality(...)`** — annotation wins over
  global `--target-cache`; `@locality(any)` opts out.

Tier sizes auto-detect from
`/sys/devices/system/cpu/cpu0/cache/index{0,2,3}/size` on
Linux (cached for the build's lifetime); static fallbacks
32 KB / 512 KB / 8 MB apply elsewhere.

Estimator accounts for alignment padding (struct interior
padding + final padding to struct alignment); previous
packed-layout assumption under-estimated by ~10-20% on
mixed-alignment shapes.

### Codegen substrate work

- **Codegen-aware per-pool chunk-size hint** (F.32-3).
  Loci instantiated on a non-`main` cooperative pool get
  a chunk-size hint sized to `target_L2_per_core /
  loci_on(pool) / typical_chunks_per_locus`, clamped to
  `[4K, 64K]`. The runtime's `lotus_arena_create_labeled_sized`
  honors the hint; env override
  (`LOTUS_ARENA_CHUNK_BYTES_OVERRIDE`) still wins via the
  upper bound.
- **Locus struct field reorder by access frequency**
  (F.32-1b). User-declared `params { }` fields are sorted
  by `self.<field>` access count, with a 10^depth
  multiplier per loop nesting level. Hot fields land on
  the first cache line of `self`.
- **Bus-dispatch prefetch hint** (F.32-4-prefetch). Producer
  emits `__builtin_prefetch(slot, 1, 3)` after the memcpy
  in `lotus_coop_pool_post` and friends. A/B toggle via
  `LOTUS_DISABLE_PREFETCH=1` at build time.
- **Huge-page-backed arenas + `mlockall`** (F.32-4a / 4c).
  Operator-tunable via `LOTUS_HUGE_PAGES=1` /
  `LOTUS_LOCK_MEMORY=1` env vars; documented in
  `docs/src/how-tos/keeping-memory-bounded.md`.

### Tooling

- **highlight.js mode** (mdbook docs site): `placement`
  and `discard` now style as keywords. `@locality(...)`
  picks up the generic `@<ident>` annotation rule.
- **heron tree-sitter grammar** (sibling repo): adds
  `placement_block` + `placement_spec` + `locality_annotation`
  + `locality_tier`. Editor highlighting + the future LSP
  parse both new constructs. Released as
  `hale-lang/pond@5d8202d`.

### Documentation

- **README** rewritten with substrate-pluralization framing:
  matchmaker example walkthrough (every phrase maps to a
  syntactic slot), "One language. Every substrate." section
  (native + browser shipped via hale-js; mobile / embedded /
  GPU / robotics / edge characterized as workload-pull, not
  roadmap), "Try it on code you already have" zero-install
  demo via AGENTS.md drop-in, "what the compiler is doing
  for you" enumeration with F.32 as receipt.
- **Spec sweep** for #24 + F.32-2: `spec/types.md`
  declaration restrictions narrowed and new "Working-set
  estimator" section; `spec/semantics.md` "Where each
  channel lives" rewritten; `spec/styleguide.md` two-channel
  rule references narrowed; `spec/stdlib.md` TCP Stream
  sentinel-shape framing updated; `spec/grammar.ebnf`
  picks up `locality_annotation`.

### Internals

- Sync inference walker covers all `Expr` arms
  (`Sum` / `Prod` / `Approx` / `Range` / `ArrayRepeat` / `Or`);
  previously the catch-all `_ => {}` arm under-counted
  `self.<field>` references inside closure assertions,
  range expressions, and `or`-substitute RHS.
- Working-set estimator's `BudgetBreach` records carry
  `tier: CacheTier` + `source: BudgetSource` so per-breach
  diagnostics name whether the contract came from
  `@locality` or `--target-cache`.

### Not in this release

The deliberately-deferred items per `notes/f32-cache-aware-delivery-plan.md`:

- **F.32-1γ-v2** (lockfree grow + tombstones). Needs tsan /
  relacy concurrency validation and a downstream workload
  that hits γ-v1's fixed-cap ceiling. Default: do not
  pursue until both gates clear.
- **Auto-applied sync inference**. The inference engine
  picks `sync = X` from the pool-propagation graph; v0.2
  will inject the kwarg into the AST so codegen honors
  it without the user pasting. v0.8.1 ships diagnostic
  enhancement only.
- **NUMA-aware placement** (`pinned(numa = N)`). No
  workload pulling yet.

---

## v0.8.0 — initial release

The language surface is stable. A few small additions are
planned, but most work from here to v1 is bugs, stability, and
performance — not new syntax or new semantics. Pin to a commit
if you build on it; small additions still land. The reference
contract is the spec under `spec/` plus the in-tree fixture
programs under `crates/hale-codegen/tests/fixtures/examples/`.
