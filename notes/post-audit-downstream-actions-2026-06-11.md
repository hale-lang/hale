# Downstream (pond / fathom) follow-up actions — post-audit pass

Collected per `post-audit-hardening-handoff-2026-06-11.md` ("Fathom-side
and pond-side follow-up actions: do NOT implement; collect them here for
separate handoffs"). These are **consumer-repo** cleanups, most gated on
a language-repo change shipping first. Nothing here is language work.

## Gated on a language fix landing (do after the cited WS ships)

| Consumer | Action | Unblocks when |
|---|---|---|
| pond/sqlite | Restore `Db.exec/query/...` from free-fn shims to locus **methods**; drop `conn_handle=0` stubs | WS4 ships `std::db::sqlite::*` |
| pond/jobs, pond/migrations | Replace `JobError{kind:"unsupported"}` / `db_path: String` workarounds with real `db::Db` qualified types in params + bodies (§11, §2) | WS3.4 lifts the two-hop / qualified-type-in-signature codegen gap |
| pond/logfmt, pond/tracing | Restore `import "../http/client"`; un-stub `OtlpSink.__post_batch` / `export_otlp` to real `http::post` | WS3.3 (file-locality) + the no-transitive-import rule decision |
| pond/term, pond/tui, pond/_util | Collapse local copies (`paint`/`badge`, glue) into `term::Styler` imports | the "G34" two-hop qualified-name-struct-literal codegen gap lifts |
| pond/router | Restore `use(m)` method name; collapse 3 parallel vecs into one `RouteEntry{handler: Handler}`; restore `Context{req: std::http::Request}` | LocusRef→Interface coercion at method-arg/struct-field sites + pass-A0 ordering fix |
| pond/metrics | Re-split the consolidated `metrics.hl` back into per-concern files | cross-file pass-A registration walk (WS3-adjacent: same family as WS3.4) |
| fathom (book_consistency) | Wire the shipped `std::crypto::crc32` (hale #14) into the consistency check | already shipped — fathom-side only |
| fathom (mdgw silence) | Wire `std::io::tcp::set_recv_timeout` (hale #15) into `WsClient` + wake the silence check | already shipped — pond/websocket-side only |

## Resolved this pass (WS3.3) — downstream can revert workarounds

- **`hale run <dir>` now resolves cross-seed imports** (was the gap;
  `hale build <dir>` already worked). Any downstream workflow that
  avoided `hale run` on a directory-seed app with imports — or hit
  "qualified type not in path-renames table" / "qualified-name
  struct literal in expression position" *under `run`* — can use
  `hale run <dir>` directly now.
- **Cross-file bus topics confirmed working** (`publish T` / `T <- v`
  resolving a `topic T` declared in a sibling file, intra-seed and
  through an imported lib). The pond **tracing** and **agent/llm**
  single-file collapses (topic decl forced into the publisher's
  file) are **no longer necessary** — a `topics.hl` sibling resolves.
  Verified by `hale-codegen` `ws3_topic_cross_file` and `hale-cli`
  `run_dir_resolves_imports`. (Note: the literal-subject idiom still
  works too and stays valid for ≥2-publisher topics.)
- **G34 two-hop qualified literals work at HEAD (WS3.4).** A
  qualified struct/locus *literal* in expression/return position
  inside an intermediate lib (`app → b → c`, `b` instantiates
  `c`'s types by `c::Thing { ... }`) builds and runs — single- and
  multi-file intermediate libs, via `build` and `run`. The pond
  `_util`/`logfmt`/`term`/`tui` "keep a local copy, G34 blocks
  importing each other" workarounds can be **collapsed into real
  cross-seed imports**. Gated by `hale-cli` `two_hop_qualified_literal`.
  (Caveat: the *re-export* barrier still holds — an app must import
  a lib itself to name its types; libs may only use their own
  imports internally. That is by design, not a bug.)
- **Nested-param shm_ring subscribers work at HEAD (WS3.5).** An
  shm_ring subscriber as a nested locus param — including a param
  of the main gateway locus — spawns its reader thread and
  dispatches; the "must be top-level in `fn main()`" no-op is
  stale. The gateway pattern (a gateway locus owning the shm_ring
  binding + a child subscriber param) is supported. Gated by
  `hale-codegen` `shm_ring_nested_param_subscriber`.

## Not gated — consumer design choices already taken (record only)

- **fathom/metrics factory leak** — resolved consumer-side via a
  `MetricsCollector` child subscribing to a `MetricUpdate` topic
  (closed-world synchronous publish). No language action; this is the
  blessed pattern WS5.2c should document.
- **fathom low_corrupt_rate windowed counter** — wants windowed-counter
  machinery. Explicit language **non-goal** this pass (no
  closures-with-capture). Leave to a future language proposal.
- **pond/supervisor** single-`accept`-type — F.11 design choice
  (multi-accept deferred). No action.

## Downstream verification (needs a consumer-repo type to settle)

- **WS1#4 against real `ws::WsClient`** — the in-repo probe
  (`ws1_cross_seed_locus_reassign`) proves the general cross-seed
  whole-reassignment path is sound, but cannot model `ws::WsClient`'s
  @ffi opaque-handle field + network `birth()`. To settle whether the
  fathom mdgw-evm half-init is live at HEAD or was an artifact of an
  older build, re-run the actual `self.conn = ws::WsClient { … }`
  reconnect at HEAD in mdgw-evm (read-only repro on fathom's side). If
  it still half-inits, hand back a minimal `@ffi`-handle locus repro
  for an in-repo fix; if clean, close the item.

## Test-fidelity finding (in-repo, WS2-adjacent)

- The `hale-codegen` `build_executable` test harness lowers a
  parsed program straight to codegen **without running
  `check_bundle` (typecheck)**. So tests built that way don't
  catch typecheck-level rules — e.g. `shm_ring_hale_subscriber`
  uses `subscribe Tick … of type T` on a topic-ref, which the real
  CLI now *rejects* ("`of type T` is forbidden; the topic carries
  the payload type"), yet the test passes. This is the same
  test-vs-CLI divergence class that hid the WS3.3 `hale run <dir>`
  import gap. Worth a sweep: route codegen integration tests
  through the CLI (or call `check_bundle` in the harness) so the
  typed surface is actually exercised. Folds naturally into WS2.

## Verification follow-ups that belong in THIS repo (not downstream)

Tracked in `ws0-friction-verification-2026-06-11.md` Follow-up F1/F2 —
the inline-async-bus-child fixture and the 3-seed bus→Decimal carrier.
Listed here only so they are not mistaken for downstream work.
