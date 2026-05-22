# Changelog

Historical record of behavior changes. The canonical spec lives
in [`spec/`](./spec/) — each file there represents *current*
behavior. This file records *when* each piece of current
behavior shipped and what (if anything) it superseded.

Entries are grouped by date and tagged with the affected area:
`[lang]` (parse/typecheck/lower), `[runtime]` (C substrate),
`[stdlib]`, `[codegen]`, `[memory]` (allocator + arena), `[spec]`
(documentation only). Where a session shipped multiple related
changes they're rolled up under a single header.

Stable navigation tags like `§ F.20`, `§ F.22`, `§ F.27` are
preserved in [`spec/design-rationale.md`](./spec/design-rationale.md);
this changelog uses them too where applicable. Milestone tags
(`m71`, `m95`, …) and phase tags (`Phase 2c`, `Phase 4`, …) live
here as historical labels — the spec no longer carries them.

---

## 2026-05-22 — leak hunt + view ABI compaction + diagnostic harness

Long-running daemon (fathom KrakenMdgw) hit a hard memory cap
in ~13 minutes against live market data. Session goal: identify
and structurally close every locus-arena leak in the substrate.
Sequence of commits:

- `cbbebbc` **[runtime] [codegen] [spec]** F.30b view ABI
  compaction — `lotus_view_t` shrunk from 24-byte heap-allocated
  to 16-byte by-value (`{src, epoch}`); SysV AMD64 returns the
  view in `{rax, rdx}`. Underlying data pointer recomputed at
  unpack via `((builder*)src)->buf - 8` (Bytes) or `buf` (Str).
  Static-data sentinel `epoch == -1` replaces the pre-rework
  `builder=NULL` convention. Zero arena allocation per
  `view()` / `text_view()` / `lotus_view_from_static_data` call.
  See [`spec/types.md` `BytesView` / `StringView` rows](./spec/types.md)
  and [`spec/runtime.md` view-layout section](./spec/runtime.md).
- `d9335bf` **[codegen]** Same-arena outer-struct skip at
  cross-arena store boundaries (validated null on fathom's
  fresh-from-delta workload — kept for the RMW pattern it does
  catch). See [`spec/memory.md` Phase-4 perf follow-on #4](./spec/memory.md).
- `d226d95` **[runtime] [codegen] [stdlib]**
  `LOTUS_ARENA_RESIDENCY=1` per-arena byte counter harness;
  Aperio surface `std::process::dump_arena_residency()`.
- `8ad917b` **[runtime] [codegen]** Labeled arenas via
  `lotus_arena_create_labeled(name)` — every locus arena
  carries the locus name; residency dump emits `label=<name>`
  per arena. Backtrace capture depth bumped from 8 to 24.
- `fb2a769` **[codegen]** Multi-return method-scratch destroy
  fix. `close_method_scratch` previously cleared
  `current_method_scratch = None` after emitting the destroy
  IR; methods with multiple `return` statements leaked scratch
  on all-but-one return path. Save/restore the scratch state
  around the call in `lower_return` so every return emits its
  own destroy. Validated against fathom: KrakenMdgw locus
  arena 10 MiB/min → 0.
- `f042806` **[codegen]** Anchor-in-place at
  `@form(hashmap).set` — drop the wasted outer-struct
  allocation (hashmap stores cells inline; runtime memcpys
  bytes into the slot). Walk source fields, anchor heap
  fields in dest arena via existing clone helpers' same-arena
  skip, return source pointer. Validated against fathom:
  MetricMap 0.38 MiB/min → 0. See [`spec/memory.md` Phase-4
  perf follow-on #5](./spec/memory.md).
- `5b96380` **[codegen]** In-place mutation at
  `self.X = Struct{}` and `self.X[i] = Struct{}` —
  pre-allocated slots get mutate-in-place semantics; the slot's
  pointer doesn't change under repeated assigns. Validated
  against fathom: SymbolBook 0.53 MiB/min → 0, WsClient
  0.15 MiB/min → 0. See [`spec/memory.md` Phase-4 perf
  follow-on #6](./spec/memory.md).

Session-cumulative result against fathom KrakenMdgw: 13-minute
projected OOM → effectively unbounded (every long-lived arena
flat across a 60s burn vs live Kraken).

## 2026-05-21 — Phase-4 per-method scratch reclaim + chunk pool

- `7cc4439` **[codegen] [spec]** Phase-4 method-scratch reclaim
  — locus methods open a per-call subregion of `self.__arena`
  on entry, route transient allocations through it, destroy
  the subregion on exit. Closes the multi-MB/sec growth on
  hot dispatch paths (every JSON parse / metric op landed in
  `self.__arena` directly pre-fix). See [`spec/memory.md`
  Phase-4 section](./spec/memory.md).
- `5300071` / `d435e9b` / `ea0a609` **[codegen]** Cross-arena
  deep-copy at `@form(vec).push` / `@form(hashmap).set` /
  `@form(vec).set`. Heap-pointer fields in a freshly-built
  cell live in caller scratch; the slot memcpy would dangle
  on method exit. Deep-copy into the receiver locus's arena
  before the store.
- `9a5497a` **[runtime] [stdlib]** Interruptible
  `std::http::Server` accept (C-iii). Server.shutdown
  unblocks the accept loop so it can exit cleanly.
- `c2b214a` **[runtime] [codegen]** `std::process::rss_bytes()`
  observability primitive via `getrusage(RUSAGE_SELF)`.
- `5198e6a` **[runtime]** `read_file` size-tolerant for
  `/proc` and `/sys` (synthesized files report `st_size=0`).
- `368bfbf` **[runtime]** Per-thread chunk pool — amortize
  method-scratch malloc/free. 16 → 256 slot cap (after
  observing 99.6% miss rate at hot churn); 32-chunk prefill
  on first touch. `LOTUS_CHUNK_POOL_STATS=1`,
  `LOTUS_CHUNK_POOL_PREFILL=<N>`,
  `LOTUS_GLIBC_ARENA_MAX=<N>` env vars.
- `41e0437` **[spec]** Document D + B v1 constraints —
  cooperative children block parent run() under v0
  scheduling; yield mailbox drain gap.
- `edd56ea` / `be843fc` / `c7cddc9` / `21ffbdb`
  **[runtime] [codegen]** Allocation diagnostic suite —
  `LOTUS_ARENA_LOG_BIG_CHUNKS=<bytes>` + companion env vars,
  libc-allocator linker `--wrap` interception (gated by
  `-DLOTUS_ENABLE_WRAP_MALLOC` so sidecar tests still link
  cleanly), `-rdynamic` baked into every link for resolvable
  backtraces.
- `6a56d7c` / `f0857ef` **[runtime] [codegen]**
  `lotus_str_clone` / `lotus_bytes_clone` skip optimizations:
  static-literal skip (src in `.rodata`) + same-arena skip
  (src already in dest arena). Catches the dominant
  `Counter.inc` / `Gauge.set` pattern.
- `10f51b0` **[runtime] [codegen]** `std::decimal::to_float`
  direct i128 → f64 conversion (skip ASCII round-trip).
- `dab03b7` / `4c43c9a` / `d7f3646` **[codegen]** Indexed
  self-assign deep-copy + field-init dangling-pointer fix +
  16-byte alignment for Decimal-bearing struct returns
  (`movdqa` segfault when align=8).
- `2026-05-21` **[spec]** Locus arena hierarchy spec catchup
  — current-arena-ptr priority, m49 calling convention,
  Phase-4 scratch reclaim invariants.

## 2026-05-20 — bytes-builder + views + bus + file locus

- F.28 **[lang] [stdlib] [spec]** `std::bytes::BytesBuilder`
  locus — growing-buffer accumulator with `view()` /
  `text_view()` zero-copy reads. Replaces the prior
  `std::str::builder_*` ad-hoc surface.
- F.29 **[lang] [codegen] [spec]** Locus-typed param fields
  with lifecycle cascade — `conn: ws::WsClient =
  ws::WsClient { ... }`-shape fields get parent-owned dissolve.
- F.30 **[lang] [codegen] [spec]** `BytesView` / `StringView`
  — non-owning view as typecheck-distinct types. Storage
  rejects view-into-owned; explicit `std::bytes::clone(v)` /
  `std::str::clone(v)` upgrade path.
- F.30b **[runtime] [codegen] [spec]** Mutation-while-live
  runtime guard — view stamps the source builder's
  `mutation_epoch`; read sites unpack via
  `lotus_bytes_view_data` / `lotus_str_view_data` with epoch
  check + `lotus_view_stale_panic` on mismatch.
- F.27 v2 **[lang]** `violate.birth_check` ergonomic at locus
  birth — catches alloc-fail-on-birth without per-locus
  hand-written guards.
- Form H **[codegen]** Fixed-size array bus-payload fields
  (`[T; N]` for primitive / TypeRef leaves).
- Form I **[runtime]** `bin/aperio` publish bundles
  `libaperio_ts_shim.a`.
- Form J / K design + ship — bus constraint substrate +
  zero-copy slot-as-locus publisher API. Compile-time route
  matrix per (bus_scope, payload_shape).
- F.19 (per-directory seed) and F.20 (structural interfaces
  Phase B vtable dispatch) reached design-shipped state.

## 2026-05-19 — F.27 inline closure violation + F.28 BytesBuilder

- F.27 **[lang] [codegen] [spec]** Inline closure violation —
  `violate closure_name;` syntax + `on_failure` handler
  routing for closures with no auto-epoch.
- F.28 Phase 1 **[stdlib] [runtime]** BytesBuilder locus —
  initial heap-backed accumulator.

## 2026-05-18 — ship-everything session

Six commits closing the v1.x compiler gap:

- `closed-world tower optimization` **[codegen]** Parent→child
  single-hop tower rewrite in `desugar_intra_locus_topics`.
- `bus transport redesign Wave A` **[lang] [stdlib]**
  Tcp/Nats/InMemory variants gone; role kwarg + inference;
  Adapter contract in stdlib.
- `File locus` **[stdlib]** `std::io::file::File` held-open
  locus with Option C arena routing.
- `read_file size-tolerant for /proc` (foreshadowed later
  fix landing 2026-05-21).
- Spec catchup for the above.

## Earlier (pre-2026-05-18)

The session-by-session record from project bootstrap through
m95 lives in the `notes/v1.x-checkpoint.md` running log and
the per-handoff documents under `notes/`. The canonical
labeled milestones referenced from spec sections:

- **m12** — bus message router. Per-subject dispatch with
  payload-copy semantics.
- **m20** — per-locus arenas. Replaces program-wide
  allocator with one arena per locus instance.
- **m22** — chunked-class subregion accept. Parent arena
  hands out subregion slots for child loci.
- **m25–m28c** — schedule classes (cooperative / pinned /
  pinned+core), mailbox-based cross-thread bus dispatch.
- **m47** — has-payload enums.
- **m49** — free-fn `__caller_arena` calling convention.
- **m51** — heap-typed free-fn returns with recursive
  deep-copy.
- **m54** — locus mode/fn-method default params (suffix-only).
- **m57** — AF_UNIX SEQPACKET bus transport.
- **m60** — m70 wire format for cross-process bus payloads.
- **m70** — bus serialize/deserialize, per-subject codec.
- **m71** — `std::*` magic-path resolver, `std::process::pid`.
- **m72** — `lotus_tcp_*` C substrate.
- **m73** — `std::io::tcp::Listener` stdlib locus.
- **m74** — `lotus_fs_*` C substrate.
- **m75** — `std::io::fs::*` Aperio surface.
- **m76** — capstone io-demo.
- **m77** — argv/env plumbing.
- **m78** — `std::str::parse_int` / `can_parse_int`
  (flipped to fallible 2026-05-17).
- **m79** — `std::time::sleep` / `monotonic`,
  `std::process::exit`.
- **m80** — function-pointer language addition.
- **m81** — Stream locus + non-self method calls.
- **m82** — locus-all-the-way-down (let-bound locus
  literal deferred dissolve).
- **m83–m86** — `std::http` Phase 3 (multi-accept Listener,
  request parser, response writer, end-to-end server).
- **m87** — `std::test` assertions.
- **m88** — Phase 2 v0.1 assertions, fallible flip-overs.
- **m89** — Bytes / String separation, length-prefixed
  Bytes ABI.
- **m90** — Locus instantiation routing.
- **m91** — Phase 4 v0.1 markdown surface.
- **m92** — doc-server capstone.
- **m93** — stdlib reorganization (per-domain `.ap`).
- **m94** — bus subject wildcards.
- **m95** — `std::log` namespace.
- **907837a** — free-fn allocation routing direct to
  `__caller_arena` (replaced the per-call subregion route
  that proved unsound without escape analysis).

The F.N design-commitment series in
[`spec/design-rationale.md`](./spec/design-rationale.md)
documents each major language commitment. The numbering is
stable; this changelog cross-references the F.N tag where
relevant rather than duplicating the rationale here.
