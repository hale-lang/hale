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

## 2026-05-22 — leak hunt + view ABI compaction + diagnostic harness + user-extensible FFI + path-based DTO identity + iris/fathom friction sweep

Long-running daemon (fathom mdgw) hit a hard memory cap
in ~13 minutes under live upstream load. Session goal: identify
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
  own destroy. Validated against fathom: receiver locus
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

- **[codegen]** m49 sret-pattern return-arena routing —
  method-with-scratch aggregate returns lower the return-
  expression under `current_arena_override = caller_arena`,
  so fresh allocations (struct literals, nested calls) land
  directly in caller storage. `emit_method_return_deep_copy`
  contains-checks via `emit_cross_arena_store_deep_copy_ptr`
  and passes the value through unchanged. Closes the
  SweepResult / BookSignalSnapshot return-value leak class.
  `populate_user_type_fields` field-level deep-copy is gated
  on `current_arena_override.is_some()` so ordinary struct
  construction (hashmap.set's Cell arg, locus param defaults)
  stays on the pre-rework path and downstream anchor same-
  arena skips remain intact. See [`spec/memory.md` Phase-4
  perf follow-on #8](./spec/memory.md).

- **[runtime] [spec]** `LOTUS_ARENA_LOG_CHUNK_ATTACH=<N>` —
  diagnostic env var that logs every chunk attachment (pool-
  recycled AND fresh-malloc paths) with `arena=<ptr>
  kind=<root|sub> label=<resolved>` so the destination arena
  is attributable. Closed the diagnostic blind spot where
  `LOTUS_ARENA_LOG_BIG_CHUNKS` (malloc-path only) missed pool-
  recycled chunk attachments — those dominate the trace volume
  once method scratch destroys recycle chunks via the per-
  thread pool. Filter `kind=root label=<name>` to isolate
  actual arena-growing callsites.

- **[runtime] [codegen] [spec]** `lotus_str_assign_in_place(
  arena, old, new)` + `lotus_bytes_assign_in_place(arena, old,
  new)` — reuse the existing slot's buffer when the new value
  fits (`strlen(new) <= strlen(old)` for String; `new_len <=
  old_cap` against the Bytes header). Fall back to the
  respective clone helper when old is static / null / too
  small. Both wired into `emit_self_field_inplace_assign` for
  `self.X = String|Bytes` inside a method-with-scratch (dispatch
  is by slot type). Closes the per-update heap-field-
  reassignment leak class — measured against a per-frame
  `self.last_ts = ts` pattern in fathom's mdgw: receiver locus
  arena ~+1-3 chunks per instance per 4 min → flat. See
  [`spec/memory.md` Phase-4 perf follow-on #7](./spec/memory.md).

- **[docs]** `agents/memory-patterns.md` — author-facing
  catalog of memory-shape patterns: which assignment / return /
  lookup shapes the substrate makes allocation-free automatically
  vs. which require care. Mirrors `spec/memory.md`'s Phase-4
  follow-ons list, adds the "When NOT to worry" carve-outs,
  documents the diagnostic workflow that pinned fathom's
  per-instance arena residual. Cross-linked from `AGENTS.md`.

- **[runtime] [spec]** `SSL_MODE_RELEASE_BUFFERS` set on the
  process-global TLS client context in `lotus_tls__ctx_get`.
  OpenSSL holds ~16-32 KiB of read/write buffer state per
  long-lived connection between records by default; setting the
  mode releases those buffers back to libc malloc on idle.
  Closes the diagnosed-but-unattributed ~0.12 MB/min VmRSS
  residual the post-leak-hunt fathom burn surfaced (every
  Aperio arena flat, but the process heap segment still grew —
  bisected to OpenSSL's read-buffer-retain default). Cost:
  one libc malloc/free pair per TLS record on the active path,
  negligible at typical WS-frame rates. See [`spec/stdlib.md`
  `std::io::tls` row](./spec/stdlib.md).

Session-cumulative result against fathom mdgw: 13-minute
projected OOM → effectively unbounded (every long-lived arena
flat across a 60s burn under live upstream load). Subsequent
verification burns confirmed RSS slope dropped from 0.79 →
0.195 MB/min mid-session, then to near-zero structural drift
after the sret + String in-place fixes landed.

- `f034893` **[docs] [spec] [changelog]** Removed every named
  trading-venue reference from the docs (Kraken, XBT/USD, etc.)
  + redirected outbound links from the keeping-memory-bounded
  how-to to the in-tree `agents/memory-patterns.md`. The
  examples / measurements stay intact under generic naming
  (`Service`, `mdgw`, `ws.upstream`, `wire-format timestamps`,
  etc.). Aperio is a general-purpose language; the docs ship
  domain-agnostic.

- `65e3c06` **[notes]** `notes/ffi-design.md` — design memo
  for the user-extensible `@ffi("c")` mechanism. Captures the
  pivot away from "ship `std::raylib` / `std::pty` in stdlib"
  toward letting library authors land bindings in pond (or any
  community repo) via `@ffi` decls + `aperio.toml [ffi]`
  sections. Three-stage rollout — staging table inside the
  memo, all three now shipped.

- `a5f71c7` **[lang] [codegen] [runtime] [spec]** Stage 1 of
  the FFI mechanism: parser accepts `@ffi("c") fn name(args) ->
  ret;` annotations on top-level free fns; typecheck validates
  parameter / return types against the FFI-portable subset
  (scalars + String/Bytes/views/Duration/Time, plus named
  user-type structs); codegen emits LLVM `declare` (not
  `define`) so the linker resolves against C glue at link time;
  `aperio build` learns repeatable `--link <name>` /
  `--csrc <path>` flags. Vertical-slice regression test
  builds + links + runs an Aperio program with a hand-shipped
  `.c` file end-to-end. See [`spec/ffi.md`](./spec/ffi.md) for
  the canonical contract.

- **[codegen] [cli] [spec]** Path-based mangler for cross-seed
  imports. The mangler used to embed the importer's chosen
  alias in symbol names (`__lib_<alias>_<stem>_<name>`), which
  meant two apps importing the same shared seed under different
  aliases produced different symbols — a real sharp edge for
  DTO seeds exchanged on a bus, where the wire bytes match but
  the in-language types diverged. Switched to path-based
  identity: the mangler uses a stable `<lib_id>` derived from
  the lib's canonical path relative to the workspace root,
  yielding `__lib_<lib_id>_<stem>_<name>`. Two apps importing
  the same lib now see symbol-identical types regardless of
  alias.

  `find_workspace_root` now anchors on `aperio.toml` (the
  natural Aperio repo manifest) in addition to `Cargo.toml` —
  previously it only walked up looking for cargo workspaces,
  which broke path-relative computation for standalone Aperio
  repos. Canonicalizes the starting path first so relative
  entries (`aperio build apps/a/main.ap` from the repo root)
  walk real ancestor directories.

  Collision avoidance preserved: different libs live at
  different paths, get different `<lib_id>`s. Importer's alias
  is still load-bearing at the call-site reference layer
  (`alias::Name` resolves via the path-rename table); only the
  symbol-namespace key changed.

  Updated: spec/projects.md (the canonical mangling-scheme
  description), spec/semantics.md (the cross-seed-import
  walkthrough), spec/styleguide.md + spec/design-rationale.md
  (referenced shapes), plus the three_hop_import test which
  explicitly defended the OLD invariant — now defends the new
  one.

- `018f926` **[codegen] [build] [docs] [spec]** Stages 2 + 3
  of the FFI mechanism:

  * Struct-by-pointer + sret-style returns. User-type structs
    pass as `ptr` at the boundary (C glue dereferences);
    struct returns gain a hidden first arg, callee fills.
    Sidesteps per-platform struct-by-value ABI classification
    (SysV register-class / Win64 shadow store / aarch64 HVA)
    without per-target lowering passes. Unblocks any non-
    trivial binding (raylib Color/Vec3, sqlite handles, etc.).
  * `aperio.toml [ffi]` auto-pickup. When `aperio build`
    resolves an `import` against a directory containing
    `aperio.toml`, the file's `[ffi]` section's `link` +
    `csrc` accumulate into the build's clang invocation
    automatically. Consumers just `import "pond/raylib" as
    ray;` — no `--link` / `--csrc` flags needed.
  * Mangler — `@ffi` fn names get identity renames so their
    LLVM symbol stays as the literal C symbol the glue
    exports. Cross-lib uniqueness is a library-author concern
    (recommendation: prefix every `@ffi` fn with the lib's
    identifier).
  * `agents/binding-packages.md` — authoring brief for binding
    libs. File layout, three-layer Aperio surface, C-glue
    skeleton with by-pointer/sret conventions, naming, optional
    helpers (idempotent init, error-sentinel translation),
    testing approach, when `@ffi` is the wrong answer.
    Cross-linked from `AGENTS.md`.

  Closes the FFI work end-to-end. Side effect: the memory
  note `project_sqlite_deferred_to_pond` becomes unblocked —
  sqlite + curl + SDL + ffmpeg etc. all bind through the same
  mechanism with zero compiler-side change.

- `773c4b8` **[codegen] [spec]** Path-based mangler identity
  for cross-seed imports. The mangler used to embed the
  importer's chosen alias in symbol names
  (`__lib_<alias>_<stem>_<name>`), so two apps importing the
  same shared seed under different aliases produced different
  symbols. Broke the DTO-on-a-bus pattern (wire bytes matched
  but in-language types diverged) and made a single binary's
  multi-alias-import of the same source carry two distinct
  type identities.

  Switched to `__lib_<lib_id>_<stem>_<name>` where `<lib_id>` is
  the lib's canonical path relative to the workspace root
  (sanitized to identifier chars, runs of `_` collapsed). Two
  consumers see identical mangled symbols regardless of alias.
  The alias is still load-bearing at the call-site reference
  layer (`alias::Name` resolves through the per-build path-
  rename table); only the symbol namespace key changed.

  `find_workspace_root` now anchors on `aperio.toml` (the
  natural manifest) in addition to `Cargo.toml`; canonicalizes
  the starting path so relative entries
  (`aperio build apps/a/main.ap`) walk real ancestor dirs
  instead of stopping at "" / "." after a few `parent()` calls.

  Spec catch-up: `spec/projects.md` (canonical mangling
  description), `spec/semantics.md` (cross-seed-import
  walkthrough), `spec/styleguide.md` + `spec/design-rationale.md`
  (referenced shapes). The `three_hop_import` test inverted —
  asserts the path-derived symbol present + the old alias-based
  symbol absent.

- `1c3146f` **[docs] [spec]** Surface the FFI mechanism +
  path-based DTO identity in user-facing docs. The FFI work
  (a5f71c7 / 018f926) and the mangler change (773c4b8) shipped
  but the docs hadn't caught up.

  * `docs/src/how-tos/ffi-bindings.md` (new) — "Bind a C
    library" walks the minimum end-to-end (3-file doubler lib)
    and points readers at `spec/ffi.md` +
    `agents/binding-packages.md` for the full contract. Listed
    in `SUMMARY.md`.
  * `docs/src/how-tos/multi-binary-bus.md` — callout in the
    shared-seed section noting type identity is path-based, so
    two binaries importing the same DTO seed under different
    aliases still see identical symbols (the property that
    makes the shared-DTO pattern work on the bus).
  * `docs/src/reference/stdlib.md` — top-of-page hook
    explaining that `std::*` is the bundled stdlib; the FFI
    mechanism is the alternative for non-stdlib C bindings.
  * `docs/src/reference/language.md` — new "Foreign-function
    interface" section in the reference index pointing at the
    canonical sources.
  * `spec/packages.md` — flipped "Don't include `aperio.toml`
    in a library" to "include it when (and only when) the lib
    declares `@ffi` bindings." Transitive `[deps]` resolution
    is still NOT in v1; that part of the old guidance stands.

- `962a745` **[codegen]** `emit_set_caller_arena` before
  `std::io::stdin::read_line` (iris F.5).
  `lower_std_io_stdin_read_line` direct-called the C-side
  `lotus_stdin_read_line` WITHOUT emitting the standard
  `lotus_set_caller_arena` prologue that every other String-
  returning stdlib primitive emits.

  Without the prologue, the C-side `lotus_bus_payload_arena_
  alloc` (which routes through the caller_arena TLS when set)
  read whatever stale value the last nested call left
  behind. In a long-lived method body that loops on
  `stdin::read_line` interleaved with helper calls (iris's
  MCP stdio loop), the second iteration's read landed in a
  destroyed sub-region from the previous handle's scratch
  and segfaulted in `lotus_arena_alloc`.

  One-line fix: emit the prologue. Matches `str_lower` /
  `str_upper` / `trim` / `pad_left` etc., all of which already
  emit it. iris's `--mcp` mode now handles arbitrary-length
  multi-request streams instead of being bounded to single-
  shot per process.

- `e75d2ef` **[codegen] [lang] [spec] [ci]** Five-item friction
  sweep from the iris + fathom backlog catalogued in
  `project_session_handoff_2026_05_22_evening.md`, plus the CI
  race on the new path-based mangler test.

  F.1 — top-level `const X: T = EXPR;` accepts non-literal
  initializers. Struct literals + arithmetic + any
  representable expression are stored in a parallel
  `user_const_exprs: BTreeMap<String, (Expr, TypeExpr)>` and
  re-lowered through `lower_expr` at each use site (same
  lifetime as inlining by hand). Intra-seed (bare `X`) and
  cross-seed (`lib::X`) reads both honor it.

  F.2 — `mode` is now a **contextual keyword** (same shape as
  `bindings` / `birth_check` / `pool` / `heap`): lexes as Ident,
  recognized as a member-introducer at locus-member position
  only. Frees the name for params/fields (raylib bindings:
  `cam.mode: Int`). Spec catch-up in `spec/tokens.md`
  (moved out of the hard-keyword list) and `spec/grammar.ebnf`
  (dropped `| "mode"` from `member_name` — now covered by
  `IDENTIFIER`).

  F.3 — `aperio build .` produces `<dirname>/<dirname>` instead
  of `<dirname>/main`. `Path::file_name()` returns None for `.`,
  so the dir-build path fell through to the `"main"` fallback;
  added a canonicalize-then-file_name fallback to recover the
  actual directory basename.

  L.1 + L.2 — generic lvalue walker. New
  `resolve_lvalue_chain` + `finish_lvalue_assign` helpers walk
  arbitrary-depth tails (Field / Index segments) with step-into
  for `TypeRef` / `LocusRef` / `Cell<TypeRef>` pointers and
  `Array` slots. Closes the codegen-v0 errors "assignment
  target with N segment(s) not yet supported" and "non-self
  field/index assignment target." Self-rooted heap-typed slots
  reached through deeper paths route through
  `emit_self_field_inplace_assign` for the same anchor + memcpy
  treatment the 1-segment fast path has. Existing fast paths
  (1-seg self.X, 2-seg self.X[i], local=v, Cell.field=v,
  arr[i]=v) are untouched — the walker is invoked only from
  the fallback branches that previously errored.

  CI — `.config/nextest.toml` adds a `shared-fixture-dir`
  test-group with `max-threads = 1` filtered to
  `binary(three_hop_import)`. The two tests in that file race
  on the same on-disk fixture binary; serializing just that
  binary is the lightest correct fix. Other tests stay fully
  parallel.

- `8acd9d5` **[codegen] [lang]** Iris F.7 / F.8 + brained F.8 —
  clang link-order reorder, caller-arena prologue sweep, lexer
  non-ASCII in comments.

  Iris F.7: `-l<libs>` came BEFORE `csrc` files in the clang
  invocation, so any csrc that referenced symbols in a linked
  static lib (raylib `glue.c` → `libraylib.a`) silently
  surfaced as `undefined reference`. GNU ld resolves
  left-to-right; libs only resolve currently-unresolved
  references, and the csrc hadn't compiled yet at the
  `-lraylib` slot. Reordered to csrc-first, matching GCC
  convention. Iris's workaround (pass `libraylib.a` as a
  csrc-shaped entry) can now drop.

  Iris F.8 + brained F.6-class: six String/Bytes-returning
  C-runtime lowerings were missing the `emit_set_caller_arena()`
  prologue — `str_builder_finish`, `bytes_builder_finish`,
  `fs::list_dir_at`, `udp::recv`, `process::pipe_read`,
  `file::read_line`. All allocate the returned value via
  `lotus_bus_payload_arena_alloc` / `lotus_caller_or_global_
  bytes_create`, both of which read the
  `lotus_current_caller_arena` TLS. Without the prologue the
  TLS carried whatever stale value the last nested call left
  behind — typically a destroyed sub-region from a prior
  call's scratch. Verified by removing iris's F.8 workaround
  (`demo.step.flush` dummy publish) and running four
  consecutive `iris.demos.run name=hello` calls through
  `--mcp` cleanly.

  Brained F.8: `skip_ws_and_comments`'s line- and block-comment
  scanners advanced `self.pos += 1` per byte even on multi-
  byte UTF-8 leaders (em-dash, box-draw, arrow); the next
  char-boundary check on `&self.source[..]` then panicked
  inside `lex_op_or_punct`. `spec/tokens.md` explicitly
  permits non-ASCII inside both string literals and comments.
  Switched to char-boundary-aware advance when the leading
  byte is >= 0x80, mirroring how `lex_string` already
  handles it.

- `67e29cf` **[lang] [codegen]** Brained F.9 + F.2 —
  lex_op_or_punct UTF-8 panic + `to_string(Time)`.

  Brained F.9: after 8acd9d5's comment-scanner fix, a related
  panic surfaced for string literals whose body begins with a
  non-ASCII char (`println("─x")`). Control reached
  `lex_op_or_punct` at the `(` immediately before the string;
  its 3-char and 2-char multi-op detection sliced
  `&self.source[pos..pos+3]` / `[pos..pos+2]` as `&str`,
  panicking when pos+N landed inside the trailing multi-byte
  codepoint. All multi-char operators are pure ASCII, so the
  fix is to match on the raw byte slice (`b"..="`, `b"=="`, …)
  rather than &str — no char-boundary precondition.

  Brained F.2: `to_string(Time)` errored with "not supported
  for type Time" even though `std::time::time_from_unix(n)
  -> Time` produced a perfectly usable ISO 8601 string at
  runtime. Root cause: v0 Time at the CodegenTy level is
  already a ptr to a NUL-terminated string (literals lower
  to a global string; `time_from_unix` returns a freshly-
  formatted gmtime_r + strftime buffer). The String ABI is
  the same single-pointer shape, so `to_string(Time)` is
  identity — but the dispatch in `value_to_string` didn't
  include the Time arm and fell through to the catch-all
  error. Added the identity arm. Real i64-since-epoch Time
  representation stays deferred per `spec/types.md`'s v0
  placeholder note.

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
