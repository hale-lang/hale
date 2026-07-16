# Changelog

Behavior changes by release. The canonical spec lives in
[`spec/`](./spec/) — each file there represents *current*
behavior.

---

## Unreleased — compiler allocation enforcement (2026-07-16)

Make the allocation-free hot path the path of least resistance, and
let a fn *certify* it. Two compiler additions, one advisory and one
enforced.

- **New default-on advisory: hot-path allocation lint.** `hale check`
  now warns on two loop-scoped anti-patterns: a **locus** or a
  `std::bytes::BytesBuilder` instantiated inside a loop (a fresh arena
  / heap buffer every iteration — hoist it to a reused field), and an
  **allocating `recv`** (`recv` / `recv_bytes` / `recv_with_source`)
  in a loop (use `recv_into` with a reused `BytesBuilder`). Both
  accumulate in the method scratch until the enclosing method returns,
  and a `run()` read loop never returns. A plain value struct/type
  literal isn't flagged, and an instantiation outside a loop isn't —
  only the unambiguous per-iteration case. Warning, never a build
  failure.
- **New opt-in contract: `@budget(alloc_per_call = N)`.** The dual of
  `@unbounded` — an explicit per-call allocation ceiling on a `fn`
  (free or method), enforced as a **hard error**. The compiler counts
  the arena allocations it can see (literals, `@form` inserts) —
  transitively through resolved callees — plus the known-allocating
  `recv` family, and errors if the fn allocates more than `N` per
  call; a loop-nested allocation, a call to an allocating fn in a
  loop, or recursion is unbounded per call. `N = 0` is the zero-alloc
  certificate for a per-datagram handler or decode helper. fn-only;
  mutually exclusive with `@unbounded`. A violation reports the
  measured count and pinpoints every offending allocation with the
  fast-path fix. Reuses the item-1 (`--dump-alloc-summary`) allocation
  summary + call graph. See `spec/verification.md`,
  `docs/src/systems/performance.md`.

## Unreleased — downstream-handoff substrate fixes (2026-07-14)

Eight substrate findings from a downstream service built on hale
0.10.0; six fixed here, two filed as issues.

- **`recv_into` now parks on `async_io` pools (timed park).**
  `std::io::tcp/udp/tls` `recv_into` / `recv_stamped_into` (and
  `recv_bytes`) on a `where async_io` pool park the coroutine on
  epoll until the fd is readable or the fd's `set_recv_timeout`
  deadline expires — `-2` again means "deadline expired" on every
  pool type, never an instant would-block. Fixes pond/websocket's
  liveness machinery tearing down every idle connection on
  async_io pools. Two contract alignments: `recv_bytes` now honors
  `set_recv_timeout` on async_io (it parked indefinitely before),
  and `udp::recv_into` returns `-2` retryable on timeout (was `-1`
  fatal).
- **`std::http::Server` reassembles split-written requests.** The
  per-connection loop reads to the header terminator, then to
  `Content-Length` body bytes, so python-urllib-style clients
  (headers and body in separate segments) work. New guards: 1 MiB
  request cap (413 on declared overflow) and a 5s recv timeout.
- **New warning: cooperative pool starvation.** Two or more
  statically non-returning `run()` bodies on one cooperative pool
  (including fields with no placement entry and the main locus's
  own `run()`) warn naming every offender — the second-born
  `run()` never starts, and the failure was silent.
- **`self.<scalar>` in nested-literal param defaults works.**
  `conn: Ws = Ws { conn_fd: self.fd }` now resolves `self`
  lexically (the declaring locus) even when the instantiation
  happens inside another locus's method body; call-site overrides
  keep resolving to the caller (F.4). A default reading a
  later-declared sibling is now a compile error instead of an
  uninitialized read.
- **Unbounded-alloc lint: `fail`/`return` payloads in loops no
  longer flag.** Both diverge — the payload allocates at most once
  per invocation. Removes the false-positive class on strict
  parsers (`fail E { … }` inside `while`).
- **Parser: reserved keywords in binding position are named.**
  `let accept = …` now says ``expected variable name, but `accept`
  is a reserved lifecycle keyword in Hale — pick another name``.
- **BREAKING: `Stream.send` / `send_bytes` / `recv` / `recv_bytes`
  are `fallible(IoError)`** (#209, finding 5). Every call site
  must address the error (`or raise` / `or discard` / `or
  <fallback>` / `or handler(err)`). send/send_bytes succeed with
  Unit (the old Int was only ever a 0/-1 status). recv/recv_bytes
  fail **only on genuine I/O errors** — EOF and a
  `set_recv_timeout` expiry still return empty, so liveness loops
  keep their shape. `IoError` is now declared in the stdlib seed
  and can be constructed / `fail`ed from user code. Bonus:
  `Stream.recv` joins the async_io timed park (its siblings got
  it in the recv_into fix above). Migration for sentinel-checking
  callers: `let n = s.send(x); if n < 0 {…}` becomes
  `s.send(x) or handler(err);`.
- **Fixed: SIGSEGV under cross-pool ingest load** (downstream
  handoff 2026-07-15) — three layered runtime bugs:
  (1) the global cooperative queue now drains **only on its owner
  thread** — a pinned publisher's scope-exit flush used to execute
  main-pool subscribers' handlers on the publisher's thread,
  concurrently with main's drains (two threads in one locus);
  (2) `lotus_arena_retire_str` records the honest blob size —
  the old 16-byte floor let the freelist flush write a 16-byte
  node over smaller same-arena-skipped concat/slice blobs
  (heap corruption at high `indexed_by` churn, even
  single-threaded);
  (3) non-flat bus payloads for **cross-thread** subscribers are
  now enqueued as wire bytes and deserialized into the
  subscriber's arena on its OWNER thread at drain — dispatch used
  to deserialize into foreign arenas on the publisher's thread
  (TSan-verified race). Same-thread publishes keep the
  deserialize-at-dispatch fast path. See spec/runtime.md
  § Owner-executed handlers.
- **Fixed: P0 memory leak on cross-thread bus dispatch to a
  parked `async_io` subscriber** (downstream handoff 2026-07-15).
  The owner-routed wire-cell path above deserialized each
  delivery's payload straight into the subscriber's locus arena —
  fine for a subscriber that dissolves, but a per-delivery leak on
  a long-lived one whose `run()` is parked forever (the canonical
  accept/recv server loop): the arena never dissolves, so every
  message's String/Bytes fields accumulated unboundedly (~320 MiB
  over 20k 16-KiB deliveries; flat afterward). Each wire cell now
  deserializes into a per-delivery subregion destroyed the instant
  the handler returns. Retention patterns are unchanged —
  `self.saved = msg` still deep-copies into the locus arena. Only
  the leaking cross-thread wire path is affected; same-thread and
  main-pool delivery were never impacted. See spec/memory.md
  § Cross-thread wire cell per-delivery reclaim.
- **Fixed: N readers can share one `async_io` pool** (downstream
  handoff 2026-07-15, item 3). The Bytes-returning
  `std::io::udp::recv` / `recv_with_source` did a blocking
  `recvfrom`, pinning the single pool worker inside the syscall —
  so a second reader locus's `run()` queued behind it on the same
  pool never started (with no recv timeout, never at all; the
  drain otherwise hung at shutdown). They now park on EPOLLIN like
  the tcp/tls siblings, bounded by the socket's `set_recv_timeout`
  deadline (or indefinitely when unset), yielding the worker so
  every reader parked on its own socket is serviced concurrently.
  Also fixes a latent use-after-free the concurrency exposed: a
  coro's caller-arena (where its stdlib allocations land) is now
  snapshotted across a park and restored on resume, so a resumed
  reader no longer allocates through an arena a sibling coro tore
  down while it was parked. See spec/runtime.md § `where async_io`
  and spec/stdlib.md `std::io::udp`.
- **BREAKING: TCP listeners bind exclusively** (downstream handoff
  2026-07-15, item 4). `std::io::tcp::listen_socket` (and the
  `Listener` / `http::Server` that use it) no longer set
  `SO_REUSEPORT` — only `SO_REUSEADDR`, which still covers the
  restart-within-`TIME_WAIT` case. `SO_REUSEPORT` let two live
  processes both bind the same host:port and have the kernel
  round-robin connections between them, so a second server booted
  by accident got no error and clients were silently split-brained
  across two divergent-state processes. A second live bind now
  fails with `EADDRINUSE`, matching Go/Rust. Only affects the
  accidental-dual-bind case; a single server is unchanged.
  Intentional multi-process port sharing would need an explicit
  opt-in (none today). See spec/stdlib.md `std::io::tcp`.
- **Fixed: unaddressed fallible `Stream` call is a clean error, not
  an LLVM ICE** (downstream handoff 2026-07-15, item 5). After #209
  made `Stream.send` / `send_bytes` / `recv` / `recv_bytes`
  `fallible(IoError)`, a call site that omitted the `or` clause (a
  bare statement or a plain value-binding) reached codegen's
  non-fallible method-call lowering and emitted a call to the
  fallible callee with the wrong arity — surfacing only as `module
  verification failed … Incorrect number of arguments passed to
  called function`. The typechecker can't catch it because a
  `std::io::tcp::Stream` literal types as `Unknown` there (stdlib
  handle loci aren't in the type table), so codegen now rejects the
  call by name: `error not addressed: \`std::io::tcp::Stream.send_bytes\`
  is fallible — handle its error with an \`or\` clause`. A
  typecheck-time diagnostic would need stdlib handle loci in the
  type table (a larger follow-on); this removes the ICE, which was
  the defect.
- **Fixed: two `@form` instances on two pools no longer need twin
  types** (downstream handoff 2026-07-15, item 5). The F.31
  cross-pool-method check pinned a `@form` (or any) locus **type**
  to one pool (first placement seen), so two loci that each held
  their own field of that type on different pools false-flagged
  every owner but the first with a "cross-pool method call" error —
  forcing byte-identical twin types as a workaround. The receiver's
  pool is now inferred per **instance** at the call site: the
  enclosing locus's own placement of the field, else the field
  co-locates with its owner. Two separate `self.<field>` maps, each
  touched only by its owner's pool, are single-threaded and no
  longer flagged (they never needed a sync discipline). A genuine
  cross-pool access — a form field explicitly placed off its owner —
  still flags and still carries the sync-discipline hint.
- Filed as an issue: implicit error propagation on tail-position
  `return` (finding 8).

---

## v0.10.0 — topology-aware placement + perspectives (live redeploy)

- **Topology-aware placement (Phase 1).** Describe the host machine
  and map loci onto its NUMA/cache/core hierarchy, memory co-located
  to the thread. `pinned(cores = A..B | A..=B | {a, b, c})` sets a
  thread's affinity mask to a core *set* (a range carves out an
  isolation domain); a `topology { }` block declares the
  socket → NUMA node → L3 domain → core hierarchy with
  `pinned(node = N)` / `pinned(l3 = name)`; a node-pinned locus
  allocates its *arena* on that node via a raw `mbind` (no libnuma
  dependency) — the thread+memory co-location payoff; and
  `replicas = K` fans a locus into K single-threaded instances, one
  per core in the range (parallelism as more single-threaded units,
  so the lock-free / devirtualization invariants survive). Linux-only
  optimization; degrades to advisory no-ops on macOS/other. Opt-in —
  existing placement lowers byte-identically.

- **Perspectives — live redeploy (Phase 2–3).** A perspective is now
  a first-class, live-rebindable handle to a *contract*: program
  against a stable ABI (`serves`) reached through a single swappable
  slot, and `reperspective` swaps the implementation behind it at
  pointer-flip cost — no restart, no global pause. Bus
  subscribe/publish edges are part of the swappable contract and
  re-point across a swap; a layout-identity swap repoints code at the
  existing arena (zero data movement), while a changed footprint runs
  a `migrate`.

- **macOS (Apple Silicon) support — phase 1.** The runtime builds and
  runs on macOS 14. `async_io` is gated behind a clear compile
  diagnostic pending a kqueue backend, and Linux-only socket options
  (`SO_PRIORITY` / `IP_PKTINFO`) + CPU affinity degrade to no-ops.
  Prebuilt, reproducible self-contained Linux releases ship via
  Docker.

- **`@form(lru_cache)`** — a bounded LRU cache form.

- **`hale test`** — discover + run `*_test.hl` (see
  [`spec/testing.md`](./spec/testing.md)).

- **Anchor-retirement freelist double-free fixed.** A String-keyed
  `@form(hashmap)` whose value struct carries the `indexed_by` field
  aliases one clone as both the map key and that field; it was
  retired twice, self-linking the reuse freelist and crashing under
  multi-key churn. Retirement now dedups within the call; block reuse
  is preserved.

- **DI verifier fix — synthesized fn-exit epilogues now carry a
  !dbg location.** A fallible fn that dissolves a local locus at
  scope exit emitted the dissolve-cascade calls with no !dbg while
  the fn carried a DISubprogram — the DWARF verifier rejected the
  whole module ("inlinable function call in a function with debug
  info must have a !dbg location"). First reproducer: pond
  http/client's round_trip_oneshot (keepalive) dissolving its local
  HttpConn, which broke a downstream app build. The epilogue
  emitters now pin the LLVM-sanctioned synthetic location (line 0
  in the fn's scope) when the per-statement location was cleared,
  and unset it on completion so it can't leak into the next
  function ("!dbg attachment points at wrong subprogram").

- **Anchor retirement — the TP-3 leak class is fixed for
  @form(hashmap).** Overwriting or removing a map row used to orphan
  the old cell's String clones in the locus arena forever (the
  audit's biggest true-positive class: 53 corpus sites; a downstream
  service's marks/on_mark shape leaked per market-data frame). Now: sync=none
  string-celled maps carry a String-field offset descriptor
  (installed at instantiation from TargetData layout); set/remove
  retire the replaced clones (pointer-difference guarded, so the
  RMW key-reuse idiom and grow-rebuild stay no-ops); retired blobs
  flush to a size-classed freelist at the USER activation boundary
  (sound by the method-scratch argument — bytes stay intact while
  any legal holder can exist); `lotus_str_clone` reuses flushed
  blocks (16-byte floor so every clone can carry a freelist node).
  Steady-state churn (4M sets, 16 keys, fresh strings per set):
  4.8 MB flat RSS, was 207 MB. Synced maps, vec cells, and compound
  self-store retire are staged in notes/anchor-retirement.md.

- **Batched @form(hashmap) iteration — walk_large 0.30 → 0.82 vs
  Rust.** `for e in m.entries` now fills a 64-entry stack batch per
  C call instead of one call per element: plain (sync = none,
  single-pool) maps take a POINTER-mode batch (zero copies — the
  loop var references slot storage directly; sound because unsynced
  maps have no concurrent writers and mutation-during-iteration is
  already contractually unsupported), synced maps copy values out
  under one lock/epoch per batch. 100k-entry walk: 301 µs → 109 µs
  (and 5.3× ahead of the hand-written C comparator). The journey
  from the original key_at walk: 1.31 ms → 109 µs, 12×.

- **Typecheck: fallible stdlib calls rejected as direct `or`
  handlers.** `x() or std::io::fs::read_file(p)` compiled but
  silently yielded the un-addressed sret value ("" / 0) when the
  handler ITSELF failed, instead of propagating — found while
  compile-testing doc examples. Now a typecheck error with the
  exact rewrite ("write `or (std::io::fs::read_file(p) or raise)`
  so its own failure has a path") until the codegen handler
  classifier covers stdlib paths. Zero hits across pond + downstream
  apps + examples.

- **Aliasing stage 2 (tier 1) — `noalias self` on provably
  non-reentrant locus methods.** Rust's `&mut`-style guarantee,
  earned from Hale's own invariants: a method in the elidable
  fixpoint (non-allocating ⇒ cannot publish, and its callees never
  drain the cooperative queue) with all-scalar params cannot be
  re-entered through the bus registry nor handed an aliasing
  pointer — so `self` is `noalias` and field loads can stay in
  registers across calls. MODES join the elidable fixpoint under
  their synthetic names (bulk/harmonic/resolution — the brain-tower
  pull surface — qualify, and sibling `self.bulk()` calls now
  classify non-allocating for scratch elision too). Contract pinned
  by IR tests (positive + both unsound channels stay unmarked).

- **Builds are 2.3–5.8× faster: dead-stdlib elimination before the
  backend.** Every module carries the full merged stdlib; it was
  being O3-optimized and machine-emitted on every build, used or
  not (224 ms of a 462 ms trivial build). Defined fns except `main`
  are now internalized and a leading `globaldce` strips the
  unreferenced stdlib before the pipeline runs. Trivial builds
  462 → 80 ms; the largest app 1.2 s → 526 ms. Plus:
  `HALE_TIME=1` prints per-phase wall times; `hale build --dev`
  (or HALE_DEV=1) selects an O1 pipeline for latency-critical
  loops; `hale check --json` emits NDJSON diagnostics on stdout
  (file/line/col/severity/kind/message) — with `hale check` at
  ~10 ms on the largest apps, this is the LSP groundwork: an
  editor save-hook needs nothing more. The staged rest (prebuilt
  stdlib object, `hale lsp`, per-seed caching) is in
  notes/build-latency-and-lsp.md.

- **Unbounded-allocation warnings are DEFAULT-ON.** (M3 stage 5
  complete — Riley's flip call after the full-corpus audit.) Every
  `hale check`/`build` now surveys the whole program; run-to-exit
  programs (a `main` with no `run` loop and no bus handler) warn
  nothing, `@unbounded fn` stays the carve-out, and
  `--no-warn-unbounded-alloc` is the opt-out (the old
  `--warn-unbounded-alloc` spelling is accepted-and-ignored).
  Warnings never fail the build. Expect real findings on the
  downstream daemons: the audit confirmed 103 true accumulation
  sites across them and the pond libraries — that visibility is the
  point of the flip.

- **M3 stage 5 (part 2) — run-to-exit programs don't warn; a
  tempting loop-bound extension rejected by the empirical model.**
  A program whose bundle has a `main` but no `run` loop and no bus
  handler is run-to-exit — per the tool's own philosophy it owes no
  memory-bound proof, so smoke binaries and scripts no longer warn
  (the model still ranks their sites; only the diagnostic surface
  is gated). Libs checked standalone (no `main`) keep ALL warnings —
  per-dir consumer checks don't re-bundle vendored libs, so the lib
  check is where pond/websocket's real per-message leaks surface.
  Also documented in-code: ranking runtime-invariant loop ceilings
  (len()/params) as bounded was implemented and REVERTED — the
  RSS-validated test is the authority that a param-ceiling loop in
  a scratchless frame accumulates linearly in the input (3M iters ≈
  190 MB), which is exactly what unbounded means here. Warning
  totals across the corpus: 402 (pre-audit) → ~160, all audited
  true positives preserved. Default-on remains blocked at ~36%
  residual FP (accepted D/E-lib/F limitations + one-shot-shaped
  app code) — the flip is now a policy call, not an engineering
  gap.

- **M3 stage 5 (part 1) — unbounded-alloc analysis: audited + three
  gap fixes.** A fresh-context audit triaged all 402
  `--warn-unbounded-alloc` warnings across pond + downstream apps +
  examples: 103 true (26%) — including live production leaks (a
  downstream service's `marks.set` per md frame, pond websocket's
  `last_message.kind` per message; the per-set anchor-clone class is
  filed as a downstream runtime issue) — and 299 false (74%). Three
  classifier gaps fixed:
  (A) `Returned` values consumed inside a member fn's per-call
  scratch no longer flag — only returns consumed by a scratch-less
  long-lived frame (`main`/`run`/free-fn chains therefrom) accumulate;
  (B) in-loop `Local`s in scratch-ful frames are bounded per
  activation (reclaimed at method exit) — EXCEPT inside a literal
  `while true`, where the exit never comes;
  (C) whole-value `self.field = Struct{...}` replaces whose inits
  are all scalar/static-literal are in-place memcpys, not arena
  growth (a single fresh heap subfield re-flags — that's the
  anchor-clone leak).
  Result: ~402 → ~165 warnings with every audited true positive
  preserved (downstream-app counts audit-exact);
  bounded[T; N] eviction loops no longer warn. Remaining for
  default-on: len()/param loop-bound recognition (the ~35% residual
  FP is main-reached runtime-bounded loops) and the accepted E/F
  limitations (one-shot binaries, return-then-publish aliasing).

- **Typecheck M3 stage 3 (tranche 2) — generic STRUCT literals +
  monomorph unification.** `Box_Int { ... }` literals now resolve
  against the generic template with the type args substituted:
  wrong-typed fields, unknown fields, and missing fields are caught
  at typecheck; field READS on monomorph values type as the
  substituted field (`b.value` on a `Box_Int` is `Int`). And
  `Box<Int>` type-exprs now resolve to the mangled monomorph name
  (previously the bare `Box`), so a `Box<Int>`-typed field and a
  `Box_Int` literal unify — and a `Box_String` literal in a
  `Box<Int>` slot is a caught mismatch. This also FIXES generic
  structs being unusable through the CLI: `hale check` rejected
  every mangled-monomorph literal as "unknown type", so only
  codegen unit tests (which skip the checker) could use them.

- **Typecheck M3 stage 3 (tranche 1) — generic fn call validation.**
  Call sites of generic fn templates are now checked at typecheck
  with source spans — the Ty-level mirror of codegen's m62
  inference: arity ("takes 3 arguments, got 2"), binding conflicts
  ("parameter `T` bound to both `Int` and `String` by this call's
  arguments"), unpinned generics ("cannot infer `T` from this
  call"), and args vs SUBSTITUTED param types. The call types as
  the substituted return (fallible payloads substituted too), so a
  generic call's result participates in downstream checking instead
  of passing through as Unknown. Permissive exactly where inference
  is blind (Unknown args, generic-arg'd nested shapes). Tranche 2:
  generic STRUCT literal field validation. Also fixed en route: a
  DWARF location leak at the mid-statement generic-synthesis site
  (the caller's active location poisoned the synthesized fn's entry
  allocas — "!dbg attachment points at wrong subprogram" — on any
  debug-info build using generics).

- **bounded[T; N]: `set(f, i, x)` + `truncate(f, n)` intrinsics.**
  `set` overwrites a live slot (fallible IndexError, arena-anchors
  pointer-shaped elements like push); `truncate` clamps the count
  down (never grows; returns the new count). Together they make the
  drop-front/FIFO idiom expressible — shift live slots left with
  set, then truncate — which unblocked migrating
  pond/agent/conversation's history eviction off its TSV walker.

- **`bounded[T; N]` — fixed-capacity counted collections in types.**
  Types can now hold a real bounded collection instead of the
  delimited-string workaround: `type Recent { vals: bounded[Int;
  32]; }` lays out inline as `{ i64 len, [N x T] }` (capacity is
  part of the type — K made value-level per F.22). The operations
  are grammar INTRINSICS, not methods, so the types-are-pure-data
  axiom holds: `push(f, x)` (fallible `CapacityError { cap, count }`
  when full — displacement policy lives in the caller's `or` arm),
  `at(f, i)` (fallible IndexError), `count(f)`, `clear(f)`, and
  `for x in f` iterates the live slots. Fields auto-initialize
  EMPTY — literal init and whole-field assignment are rejected
  (the intrinsics are the only mutation surface). Works in `type`
  fields and locus `params`; whole-struct copies carry elements and
  count by construction; scalar-element bounded is flat under
  `zero_copy`. v1 covers scalar elements (Int/Float/Bool/Decimal/
  Duration) AND pointer-shaped elements — `bounded[String; N]`,
  `bounded[Bytes; N]`, `bounded[SomeStruct; N]` (stage 1, same
  day): push arena-anchors each element into the receiver's owning
  arena (a scratch-built String pushed from another fn survives —
  the same-arena gates make re-anchoring idempotent, no realloc
  storms), and whole-struct copies anchor live slots with a runtime
  [0, len) loop. `type RouteParams { keys: bounded[String; 16];
  ... }` replaces the pond TSV idiom directly. On the bus:
  scalar-element bounded travels as flat bytes; pointer-element
  bounded cross-process is post-v1 polish (focused reject).

- **Typecheck M3 stage 2, tranche 2 — signatures for the I/O
  namespaces + dual-mode fallible semantics.** 60 more rows:
  io::fs/file/tcp/tls/udp, process child management, text
  predicates, term/diag/os. Two semantic fixes the corpus forced:
  (1) stdlib fallible path-calls are DUAL-MODE at codegen — with
  `or` they use the fallible ABI, bare they're the legacy direct
  form with per-fn returns (read_file → the String, write_file →
  an Int status) — so bare calls now stay permissive (Unknown)
  while `or` positions get precise success/payload types from the
  table (the Or arm consults it directly); (2) a statement-position
  `call() or handler(err);` discards its value, so the fallback/
  handler-return type no longer needs to match the success type
  (a common production pattern). Handle args at the
  path-call level are plain Int fds. Still excluded-not-guessed:
  all std::json / std::http rows and process stdio (routed through
  Hale-stdlib __ fns — no codegen-level ground truth), the 7
  spec'd-but-unimplemented std::io::tls fns, tcp
  set_recv/send_timeout, io::file::write_line, io::fs::list_dir.
  Gate: zero new errors across pond, downstream apps, and examples; the
  three bring-up hits (a downstream app's refdata, pond logfmt, io-demo) were
  exactly the two semantic gaps above — all three now pass.

- **Typecheck M3 stage 2 — stdlib signatures for the scalar-heavy
  namespaces.** 118 functions across std::math/time/env/decimal/
  process(scalar)/str/io::stdin/io::stdout/bytes/crypto/
  text::base64/rand now have full signature rows: arity and arg
  types are enforced, and calls return their REAL type instead of
  the permissive Unknown — `std::math::sqrt("four")`,
  `std::math::pow(2.0)`, and `std::time::sleep(100)` (Int where
  Duration is required) are now typecheck errors with spans.
  Fallible rows return `Ty::Fallible`, so `parse_int(s) or ""`
  is caught (`or` substitute checked against the Int success type).
  The table's coercions mirror what each lowering actually does
  (verified per-fn): math sitofp-coerces Int args, every String
  position accepts StringView, readers accept the whole Bytes
  family. Uncertain rows are names-only, not guessed —
  str::builder_* (opaque handles) and can_parse_decimal (in the
  spec, NOT in the dispatch — spec bug, flagged). io::fs/tcp/tls/
  udp/file are the string-heavy tranche 2. Gate: zero new type
  errors across pond, downstream apps, and the example corpus (the two hits
  found were verified pre-existing at the unmodified baseline).

- **Typecheck M3 stage 4 — expose-side contract validity + exposed-mode
  syntax.** Every `expose` entry must now bind against something real
  on the declaring locus — a params field, a mode, or a `fn` member —
  at a matching type. Previously `expose no_such_field: Int;` and
  `expose value: String;` over an Int field compiled silently (codegen
  treats contract members as pure declaration, so typecheck is the
  only enforcement point) and a consuming parent type-checked against
  fiction. The consume-side checks (missing expose, type mismatch,
  consume-without-accept) already existed. Also: mode keywords are now
  admitted in contract-name position (`expose bulk: Float;`), making
  the spec's exposed-mode pull rule (semantics.md — a parent may call
  a child's mode iff contract-exposed) expressible for the first time;
  the exposed type is checked against the mode's declared return.
  Gate: zero errors across pond, downstream apps, and the example corpus (51
  real contract lines, including pond websocket).

- **Typecheck M3 stage 1 — stdlib typo detection.** A call to an
  unknown function in a TABLED `std::` namespace is now a typecheck
  error with a did-you-mean (`std::str::parse_itn` → "did you mean
  `std::str::parse_int`?"). The table covers 26 namespaces
  (mechanically extracted from the codegen dispatch's
  `["std", ...]` patterns, unioned with spec/stdlib.md); namespaces
  with non-literal dispatch (io::sockopt, io::mirror, shm, ts) stay
  permissive, so table incompleteness degrades to the old Unknown
  behavior, never to a false error. Gate: zero new errors across
  pond, downstream apps, and the full example corpus. This is the first slice
  of the M3 plan (notes/typecheck-m3.md); signatures (killing the
  Unknown returns) are stage 2.

- **@form iteration surface — `for e in m.entries` / `for x in
  v.items`.** Hashmap iteration lowers to a cluster-aware
  slot-cursor walk (`lotus_hashmap_iter_next`): O(cap) for a full
  walk, where the index-based `key_at`/`entry_at` pair rescans from
  slot 0 per element (O(cap×len) — the quadratic behavior that put
  form_hashmap_walk_large 13× behind Rust). Vec iteration is a fully
  inline buf walk with zero per-element calls. Loop var is a copy
  (hashmap) / reference-to-cell (vec struct cells); mutation during
  iteration is unsupported; break/continue work. Measured on
  walk_large (100k entries): 1.22 ms → 0.30 ms — 4× faster and now
  1.9× ahead of the hand-written C comparator; Rust's SwissTable
  iterator still leads 3.4× (one C call per element remains — a
  batched iterator is the follow-on). Ring iteration deferred.

- **Fn-call protocol at C shape — exit-drain elision + fn-pointer
  classifier refinement.** Two changes driven by the first Rust/C bench
  comparators (fn_call/fn_modular ratio was 0.40 vs all three):
  (1) a proven-non-allocating body cannot have published (payload
  copies allocate), so its scope-exit flush skips the per-call
  `lotus_bus_queue_drain` when the deferred-dissolve frame is also
  empty — fn exit is NOT a spec-required yield point (handler exits,
  lifecycle transitions, `yield`, and `sleep` still drain). A
  minimal free fn drops from `push+lea+load+call drain+pop+ret` to
  `lea; ret` — literally C's shape. BEHAVIOR NOTE: a cooperative
  compute-only loop that relied on helper-call exits as its delivery
  points never had that guarantee by spec and now won't get it —
  use `yield;` (that's what it's for).
  (2) a call through a fn-pointer PARAM with a numeric-scalar return
  no longer marks the caller allocating: the callee scratches off the
  threaded caller arena and a scalar return leaves nothing behind —
  callback-style code (`fn outer(x: Int, g: fn(Int) -> Int)`) stays
  elidable instead of paying subregion+drain+destroy per call.
  Measured (opaque-pointer bench variants, ratio vs clang -O3 C):
  fn_call 0.40 → 0.77, fn_modular 0.40 → 0.98 (15.77 ms vs C's
  15.4 ms — parity). The bench .hl files now call through
  pid-selected opaque fn pointers (Hale has no noinline surface; the
  direct-call versions inline + fold to nothing post-elision).

- **Fallible `or` handlers — `call() or handler(err)` now accepts a
  handler that is itself `fallible(E2)`.** The handler's success value
  substitutes; its failure propagates through the ENCLOSING fn's error
  path (implicit `or raise` — sugar for the already-legal nested form
  `call() or (handler(err) or raise)`). E2 must be assignable to the
  enclosing fn's fallible payload; targeted diagnostics otherwise
  ("handler's failure has nowhere to go" / "propagated payload must
  match"). Free-fn, imported-path, and locus-member handlers are
  classified; `@form` synthesized methods and stdlib path-calls still
  need the explicit nested spelling. This closes the pond stash-bridge
  idiom: `jobs::Queue`'s DbError→JobError conversion no longer needs
  private stash fields, removing its non-reentrancy hazard.

- **DWARF debug info — `hale build` binaries now carry line tables for
  Hale code and full debug info for the runtime.** Every statement gets
  a file:line location (emission kind LineTablesOnly, DWARF 5); the
  lotus runtime TUs compile with `-g`. gdb sets breakpoints on `.hl`
  lines, backtraces show `FxL.at () at inlarr.hl:7` with inline frames,
  addr2line resolves Hale addresses, and ASAN reports carry real
  file:line through both Hale and runtime frames. Zero runtime cost —
  frame pointers are deliberately NOT forced (measured +22% on
  bus_dispatch from `-fno-omit-frame-pointer` on the runtime's
  dispatch fast paths); profile with `perf record --call-graph dwarf`.
  Opt out with `LOTUS_NO_DEBUGINFO=1`. Stdlib and synthesized `__*`
  helper bodies carry no line info (their spans live in other
  coordinate spaces); `__lib_*` cross-seed imports keep theirs. The
  module is verified whenever debug info is enabled, so a codegen
  location bug surfaces as a readable error (dumped to a .ll file)
  instead of a backend abort. Implementation notes: statement
  locations are managed by a save/restore stack that never restores a
  location across a function boundary (mid-expression fn synthesis),
  and `alloca_in_entry`'s `position_before` — which silently ADOPTS
  the target instruction's empty location per LLVM's SetInsertPoint
  semantics — re-asserts the statement location after repositioning.
  Inkwell's `get_current_debug_location` is avoided entirely (its
  legacy value-based API materializes an empty MDNode for "none",
  which then verifier-fails as `!dbg !{}`).

- **Inline fixed arrays — scalar `[T; N]` fields are now laid out inline
  in their containing struct.** Previously every array field lowered to
  an out-of-line arena pointer, so a "flat" struct with an array field
  was secretly `{…, ptr}`: `is_flat_shapeable` said flat, the shm slot
  carried a dangling pointer cross-process (the bench xproc segfault),
  and every whole-value replace persisted a fresh copy in the locus
  arena. Scalar-element arrays (Int/Float/Bool/Decimal/Duration) are now
  `[N x T]` in the struct body; the array's SSA value is unchanged (a
  ptr to storage — field reads yield the slot address, field writes
  memcpy elements). Covers user types, locus params, struct literals,
  locus params-init, self-field reads/indexed assigns, the lvalue
  walker, deep-copy/anchor walks, and the m70 wire codec.
  `is_flat_shapeable` accepts scalar arrays again to match; non-scalar
  element arrays keep the out-of-line layout and stay rejected under
  `zero_copy`. Verified cross-process: the idiomatic
  `type Blob { tag: Int; data: [Int; 511]; }` round-trips a 4 KB payload
  over `shm_ring … where zero_copy` with a correct checksum — no more
  512 hand-spelled scalar fields. Whole-value scalar-array replace
  (`self.recent = […]`) no longer leaks a persisted copy per assign
  (~35 MB over 3M trips removed; the RHS literal's scratch growth in a
  single long activation remains and is still flagged by
  `--warn-unbounded-alloc`).

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
  on next use; a downstream app's reconnect crash), while the old value
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
  yet exercised by real workloads" premise is false — a downstream app's orderbook
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
