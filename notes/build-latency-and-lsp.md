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

v2 (2026-07-17, same day): **hover + `hale/busGraph` shipped.**
Hover is token-at-position → TopScope/stdlib-signature resolution:
fn signatures with `fallible(E)` AND enforcement status (`@hot` /
`@budget(N)` read from the AST decl), locus params/accept/bus
counts, struct fields / enum variants, topic payload + `keyed_by`,
interface methods, `self.<field>` through the enclosing locus, and
`std::` paths through the stdlib signature table. `hale/busGraph`
returns the seed's full graph — per subject: publishers,
subscribers (locus + handler + placement), payload types, and the
static-dispatch verdict with its honest ineligibility reason.
Both re-analyze on demand (the 10 ms front-end again). Known
polish: a user fn's fallible payload naming a stdlib-injected
error type (IoError) hovers as `?`.

v3 (2026-07-18): **definition + references + `hale/placement` +
`hale/allocSummary` shipped**, and the `fallible(?)` hover polish
(payload name recovered from the AST decl when the resolved Ty is
Unknown). Definition resolves through TopScope symbol spans
(self.<field> → the exact ParamInfo span) demuxed back to
file+range; references is a seed-wide Ident-token scan — honest
name-based semantics (hale's flat per-seed namespace makes it
accurate for top-level symbols; shadowing locals over-report, a
documented limitation). `hale/placement` returns the main locus's
field→placement map (explicit entries rendered from the AST spec +
where-constraints; unlisted fields default cooperative(pool=main),
flagged explicit:false). `hale/allocSummary` returns the survey's
leak sites with positions plus the full text dump. No position
index anywhere — still the 10 ms re-analysis per request.

v4 (2026-07-18): **completion shipped** — `self.` members
(params as fields + user methods with signatures), `std::` paths
namespace-by-namespace off the stdlib surface tables (fns carry
their signatures + fallibility as detail), and bare-word top-level
symbols + keywords + primitive types. Context detection is
text-based (works mid-keystroke when the buffer doesn't parse);
the symbol side falls back to the on-disk seed when the overlay
is unparseable. Trigger characters `.` and `:`. Also shipped
2026-07-18: `hale fmt` (see spec/testing.md) — an LSP
documentFormattingProvider over it is a natural follow-up.

v5 (2026-07-19): **formatting + document symbols +
`hale/enforcement` shipped.** documentFormattingProvider returns
one whole-document edit from the hale fmt core (null on an
unlexable buffer — never eat text); documentSymbol gives the
hierarchical outline (locus → params fields + methods) from a
file-local parse; `hale/enforcement` returns every user fn/method
with its @hot / @budget / fallible / @unbounded contract — the
certification map an agent consults before touching a hot path.

Remaining LSP ideas (unstaged): scope-aware references, rename,
workspace symbols, semantic tokens.

## Shipped (2026-07-18) — lld link + stdlib-cache re-scope

Re-measured before building staged item 1, and the data
redirected the fix. On HEAD (post-DCE), per-phase dev times:

| phase | hello (1 line) | Server+metrics app |
|---|---|---|
| front-end + codegen | 18 ms | 19 ms |
| llvm-passes | 4 ms | 34 ms |
| obj-emit | 2 ms | 36 ms |
| emit+link | 70 ms | 71 ms |
| **total** | **~100 ms** | **~159 ms** |

Two conclusions:

1. The 2026-07-02 internalize+globaldce work already ate most of
   the stdlib-object cache's projected win: the stdlib-attributable
   llvm work on a stdlib-heavy app is ~65 ms, not the hundreds of
   ms the original projection assumed. The dominant flat cost was
   the LINK — clang's default bfd ld spends ~120 ms scanning the
   ~27 MB tree-sitter shim staticlib on every build.
2. **lld fixes that for ~15 lines**: measured 148 ms (bfd) vs
   26 ms (lld) on the identical link line. Shipped: the non-LTO
   link probes for `ld.lld` once per process and uses
   `-fuse-ld=lld` when present (Linux only; HALE_NO_LLD=1 opts
   out; silent fallback to the default linker otherwise). Dev
   builds: hello 100 → 55 ms, Server+metrics 159 → 119 ms; release
   links speed up identically.

**Stdlib-object cache: RE-SCOPED, deferred.** The remaining win
(~50-65 ms of llvm-passes+obj-emit for used stdlib fns on a
stdlib-heavy app) no longer justifies the architecture: stdlib
lowering is NOT app-independent today — bus-devirt ids, the
`no_pinned` static-dispatch flag, and `program_has_offthread` are
computed from the merged app+stdlib program and baked into
stdlib-fn bodies (a cached stdlib .o would need an all-dynamic /
always-locked conservative build), plus split-module emission
requires symbol-name stability for every stdlib fn, form-locus
synthesized method, and closure the app references. That's a
multi-session change for a sub-70 ms win on a 119 ms build.
Revisit only if apps get big enough that the llvm-passes phase
dominates again — and then item 2 (per-seed caching) subsumes it.

## Staged next (in value order)

1. **Per-seed object caching (release)**: content-hash each import
   seed → cache its .o; only re-emit changed seeds; final link
   combines. Real work (cross-seed inlining boundaries); only
   worth it when apps reach ~50k+ lines or fleet builds hurt.
   Subsumes the deferred stdlib-object cache (the stdlib is just
   another seed with the caveats above).
2. **Watch mode** (`hale build --watch`): trivial; the whole dev
   rebuild is ~55-120 ms now.
