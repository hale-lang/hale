# #8 — Build latency + LSP groundwork

Status: measured + first wins SHIPPED (2026-07-02); the rest is
staged below.

## The measurements that reframed the task

`HALE_TIME=1` (shipped) prints per-phase wall times. Baseline on
this host, before the fixes:

| phase | tiny (1 line) | a downstream service (3.6k lines) |
|---|---|---|
| front-end + codegen | 10 ms | 35 ms |
| LLVM O3 passes | 123 ms | 362 ms |
| obj-emit | 224 ms | ~500 ms |
| link (clang, non-LTO) | ~105 ms | ~250 ms |
| **total** | **462 ms** | **1.2 s** |

Two facts kill the classic incremental-compilation plan:
1. The Hale FRONT-END is ~free (35 ms whole-program on the largest
   app; `hale check` is 10 ms). Caching parse/typecheck buys nothing.
2. 97% of wall time is LLVM — and most of THAT was the merged
   stdlib being O3'd and machine-emitted in every module, used or
   not.

## Shipped (2026-07-02)

- **Internalize + leading globaldce**: every defined fn except
  `main` goes Internal before the pipeline; `globaldce` strips the
  unreferenced stdlib before O3 touches it. Address-taken fns (bus
  handlers, pinned entries, fn-pointer callees) survive through
  their uses; anything referenced by name from C would fail LOUDLY
  at link (nothing does — full suite green).
  Result: tiny 462 → 80 ms (5.8×), a downstream service 1.2 s → 526 ms (2.3×) —
  release AND dev.
- **`hale build --dev` / HALE_DEV=1**: O1 pipeline + Less machine
  codegen. After DCE the delta vs release is small on small apps;
  it scales with app size.
- **`hale check --json`**: NDJSON diagnostics on stdout (file/line/
  col/severity/kind/message) — with the 10 ms check, an editor
  integration needs nothing more than a save-hook. This IS the LSP
  groundwork: a future `hale lsp` server wraps the same
  parse+check path; every M3 diagnostic already carries spans.

## Shipped (2026-07-17) — `hale lsp` v1

Staged item 2 landed: a stdio LSP server (`crates/hale-cli/src/
lsp.rs`, `hale lsp`) over the existing parse+check. v1 =
publishDiagnostics only: initialize/initialized/shutdown/exit +
didOpen/didChange(full sync)/didSave(includeText)/didClose. Every
document event re-checks the changed file's whole SEED (its
directory, F.19) with overlay text winning over disk, then
publishes for every file in the seed — empties clear stale
squiggles with zero bookkeeping. The 10 ms check is what makes
the no-incrementality design correct. Diagnostics carry the full
check set (parse/type errors severity 1, advisories severity 2)
with UTF-16 columns. Protocol test: crates/hale-cli/tests/lsp.rs.

v2 candidates, in value order: hover (type + fallibility +
enforcement status at position), goto-definition/references (the
resolver's TopScope has the symbols; needs a position index), and
the hale-only custom methods no generic LSP has — `hale/busGraph`
(who publishes/subscribes a topic), `hale/placement`,
`hale/allocSummary` — all already computed by the checker. These
are the agent-facing wins: harnesses speak LSP natively now.

## Staged next (in value order)

1. **Prebuilt stdlib object (dev mode)**: cache the stdlib's .o per
   compiler build (like ~/.cache/hale/runtime); dev links against
   it instead of re-lowering + re-emitting stdlib fns the app DOES
   use. Loses stdlib inlining in dev (fine). Projected: a downstream service dev
   ≈ 150–250 ms.
2. **Per-seed object caching (release)**: content-hash each import
   seed → cache its .o; only re-emit changed seeds; final link
   combines. Real work (cross-seed inlining boundaries); only
   worth it when apps reach ~50k+ lines or fleet builds hurt.
3. **Watch mode** (`hale build --watch`): trivial once 1 lands.
