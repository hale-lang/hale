# Brief: working on the Hale compiler

For agents (or humans) changing the compiler, runtime, analyses or
spec. Read [`CLAUDE.md`](../CLAUDE.md) first (build, test and
naming rules; not all repeated here) and [`AGENTS.md`](../AGENTS.md)
for the language as users write it. **Hale** is the language;
**lotus** is the runtime substrate, and C-runtime symbols stay
`lotus_*` by design.

## Setup

- Work in a git worktree, not the main checkout, with its own
  `CARGO_TARGET_DIR`. Build scripts bake absolute paths
  (`HALE_STALE_ROOT` in `crates/hale-cli/build.rs`,
  `CARGO_MANIFEST_DIR` in `hale-corpus` and the ts-shim locator), so
  a shared target dir makes one checkout's binary read another tree.
- **LLVM 18 only**: `llvm-config-18` on PATH or
  `LLVM_SYS_180_PREFIX`; `inkwell = "0.5"` with `llvm18-0`. 17/19/20
  will not link. The `dynamic-llvm` feature links the shared libLLVM
  (CI uses it); release builds stay static.
- **Build the whole workspace** (`cargo build --release`), never
  `-p hale-cli`. `hale-ts-shim` is a `staticlib`, so nothing can
  depend on it, and `std::ts` programs link
  `libhale_ts_shim.a`. `locate_ts_shim_staticlib` (`codegen.rs`)
  looks at `HALE_TS_SHIM_A`, beside the running exe, then
  `<workspace>/target/{release,debug}`; test binaries under a custom
  target dir need `HALE_TS_SHIM_A`.
- `hale` on PATH may be a symlink to some checkout's dev build
  (`readlink -f "$(which hale)"`). Spot-check and A/B with an
  explicit binary: `$CARGO_TARGET_DIR/release/hale run|build|check
  path/to/prog.hl`.

## Crate map

```text
hale-cli      -> codegen, types, syntax, model, lsp, iris, dna
hale-codegen  -> types, syntax, model, stdlib (+ inkwell, LLVM 18)
hale-lsp      -> types, syntax, stdlib
hale-types    -> syntax, model, stdlib
hale-iris     -> dna
hale-syntax, hale-model, hale-stdlib, hale-dna: no workspace deps
hale-corpus   dev-dependency of syntax, types, codegen
hale-ts-shim  staticlib; no dependents; linked by path
```

- `hale-syntax`: lexer, parser, AST, `Diag`, desugar passes,
  `hale fmt` (`fmt.rs`), `keywords.rs`, chains, f-strings, JSON
  parser generation (`json_gen.rs`).
- `hale-types`: resolve, check, and every analysis (effects,
  frontier, callgraph, alloc summary, budgets, bus graph, model
  builder, claim lowering, judgment, topology artifact).
- `hale-model`: the semantic model schema and its laws. Depends on
  nothing, by law (`crates/hale-model/tests/architecture.rs`).
- `hale-stdlib`: the Hale-source stdlib (`hl/*.hl` into
  `AP_SOURCE`) plus `PATH_RENAMES`; upstream of `hale-types` so the
  analyzer walks the bodies codegen compiles.
- `hale-codegen`: inkwell lowering, the C runtime (`runtime/`, which
  also holds `lotus_treesitter.rs`, the ts-shim's `[lib].path`), link.
- `hale-corpus`: Hale programs for corpus properties: fixtures, the
  stdlib, and programs embedded in Rust test strings.
- `hale-lsp` (`hale lsp`, stdio, re-checks the whole seed per event);
  `hale-iris` / `hale-dna`: embedded `iris/` and `dna/` sources.
- `hale-cli`: the `hale` binary (`src/main.rs` plus `fleet.rs`,
  `replay.rs`, `iris.rs`, `dna.rs`, `mcp.rs`, `pkg.rs`, `sign.rs`,
  `topology_*.rs`); owns import resolution and seed loading.

## The pipeline

1. **Load**: `hale-frontend/src/snapshot.rs` `Snapshot::load`
   (`load_whole_seed`: a file target stands for its seed, a dir is one
   seed, every `import` followed); cross-seed names are mangled here.
2. **Parse**: `hale-syntax/src/lexer.rs`, `parser.rs`. Parse-time
   sugar: `@no_*` (`effect_assert_for`), chains
   (`chains::desugar_chains`).
3. **Pre-check passes**: the verb's own prefix
   (`hale_types::apply_sync_inference`, `--wrap-main`, build's
   `--api` / `--env` binding), then the one desugar sequence every
   entry point runs before it mints and checks,
   `hale_types::desugar_sequence::desugar_before_check`: JSON parsers,
   the api surface, unit returns, construction aliases, qualified bus subjects, the omitted
   `run` (marked `LifecycleDecl::synthesized`), repr accessors.
4. **Resolve + check**: `hale_types::check_bundle_opts_scoped`
   (`hale-types/src/lib.rs`): `resolve::build_top_scope`, then
   `check::check_bundle_scoped` (`check.rs`).
5. **Model**: `model_builder::derive_application_model_over`, on
   demand (`Snapshot::demand_model`), over the snapshot's scope, bus
   graph, ownership graph and handler rows.
6. **Judgment**: `judgment::claim_law_diags`, from the check path
   only when no non-`Claim` error exists and claims are present.
7. **The resolved program**: `hale_types::resolved::resolve_program`
   (the two lowering rewrites, kept as relations, the stdlib merge,
   the snapshot mint with the bundle's source map, the ownership,
   handler-routing and bus tables; the envelope keeps the renames and
   api it was resolved with, its top scope, and hands out its bundle
   view), then **codegen**: `hale_codegen::build_resolved`
   (`codegen.rs`), which refuses options whose api disagrees with the
   envelope's. The test harness builds as the verbs do:
   `crates/hale-codegen/tests/support/build.rs` loads a seed
   (`build_source` from text, `build_seed_dir` from disk) with the
   harness configuration, demands the lowering view from the snapshot
   and hands it to `build_resolved`; a test whose subject is a
   `Program` it made uses `build_program` (`Snapshot::from_program`).
8. **Runtime**: `crates/hale-codegen/runtime/*.c`, compiled once per
   (source, flags) key into a cache, linked by clang.

Inside `resolve_program` (hale-types), in order: the two lowering rewrites in
`hale-syntax/src/desugar.rs`, `desugar_intra_locus_topics` and
`desugar_topics`, **which run after check, so the checker sees topic
references as written**: they are not desugars (each erases a
declaration reference the checker's laws and the model read), and each
returns what it rewrote as a relation (`intra_locus`,
`topic_rewrites`, recorded on the bus graph's subjects); the stdlib
merge (`desugar_sequence::bundled_stdlib`, the stdlib put through the
same declaration-shaping passes); the snapshot mint over the merged
program (the stdlib's sites under their own seed); the ownership
pre-pass (`ownership::resolve_owners`, F.39
in `spec/decisions.md`: it numbers nothing and refuses an unminted
literal or call, and an instantiation with no owner row is a
`CodegenError`), the binding
facts and the fresh factories; the ownership graph and the bubble
plans; the handler-routing rows; the bus graph feeding
`DispatchPlan::from_gates`. Then inside `build_resolved` (hale-codegen):
`lower_program` (A0 types, A loci, A3 serializers, B fn decls, C
lifecycle bodies, D fn bodies); object emission and link. What
lowering still derives for itself is listed in `spec/registry.md`
as legacy rows with their removal conditions.

`hale build` derives the model to stamp its identity into the binary
(`model_identity`, `topology::model_shape_hash`); `hale check` builds
one only when claims exist (`HALE_MODEL_TRACE=1` shows derivations).
`HALE_TIME=1` prints build phase times. No interpreter: `hale run`
compiles to a temp binary and execs it.

**Two type representations**, deliberately: `hale_types::Ty`
(`hale-types/src/ty.rs`, no LLVM context) and `CodegenTy`
(`codegen.rs`, layout-aware). Don't unify them.

**Codegen layout.** `codegen.rs` holds `Cx<'ctx, 'p>`, the entry
points, `lower_program` and the `lower_stmt` / `lower_expr`
dispatch. Beside it: `stdlib/<ns>.rs` (one `pub(crate) trait
<Ns>Stdlib<'ctx>` on `Cx` per namespace), `bus/`, `locus/`, `form/`,
`types/`, `channels/` (`fallible(E)`, failure routing),
`shared/builtins.rs`, `mangle.rs`, `ownership.rs`, `target.rs`.
Which `std::` namespaces are Hale source and which are builtins:
`crates/hale-stdlib/hl/README.md`. Don't split `codegen.rs` for
length alone.

## The semantic model

Contract: [`spec/model.md`](../spec/model.md); tutorial:
`docs/src/the-model.md`; rustdoc at `/api/hale_model`.

- **One constructor**:
  `hale_types::model_builder::derive_application_model_over(&Bundle,
  &ModelInputs)`, over a *checked* bundle; a test without a snapshot
  calls `derive_application_model(&Bundle)`, which builds the inputs
  and derives over them. No artifact-to-model, no plan-to-model, no
  hand-authored model format; a test that needs a shape derives a
  real model and edits its tables.
- The law: `Bundle -> ApplicationModel`; `Bundle + Model ->
  ClaimIrTable` (`claim_lowering::lower_claims`); `Bundle + Model +
  ClaimIr -> EvidenceTable` (`evidence.rs`, input to the certificate
  and budget judgments, kept outside the model); `Model + ClaimIr
  [+ Evidence] -> verdicts` (`judgment.rs`); `Model -> DispatchPlan`;
  `Model -> artifact model half` (`topology.rs`). The fleet tier
  builds a weaker `ComponentModel` from artifact JSON
  (`hale-cli/src/fleet_model.rs`).
- A model is typed entity and relation tables, typed **holes**
  (unknowns as data), positive **capabilities**, and provenance.
  Unknown is not absent: relevant reachable holes fail a judgment
  closed. `ApplicationModel::validate()` enforces the laws.
- Derive a fact once, ask each question once (`model_query.rs`:
  `may_deliver`, `endpoint_incomplete`, `effects_of`), traverse with
  the one reachability engine (`model_graph.rs`). A new judgment
  family follows `spec/model.md` § "Adding a judgment family".

## Effects and the classified frontier (GH #265)

- Surface: `@effects(none: {…})`, `publish: {…}`, `causes:`,
  `depends:`. `@no_syscall`, `@no_block`, `@no_ffi`, `@no_publish`,
  `@no_spawn`, `@no_recursion` and `@deterministic` are **parse-time
  sugar** for `none:` sets, so the checker interprets one shape.
  `@no_panic` is a separate analysis.
- Engine: `callgraph.rs` (path-carrying walk; `witness_path` is the
  chain a diagnostic prints) feeds `effects.rs` (assertions) and
  `frontier.rs` (`infer_effects`, `causes:`, `@supervised`, `@secret`
  taint, symbolic cost). `quantitative.rs`:
  `@budget(stack_bytes | block_points | publish | fanout)`;
  `budget_check.rs`: `@budget(alloc_per_call = N)` over
  `alloc_summary.rs`.
- The frontier is the registry: `stdlib_surface.rs` gives every
  `std::` fn an `EffectSet`. An unclassified row and a `std::` path
  with no row both fail closed. Hale-source stdlib bodies are walked
  (`stdlib_bodies.rs`), not hand-classified. A new stdlib fn needs a
  classified row or the corpus conformance tests fail.
- Manifest: `hale check --dump-effects-manifest` /
  `--check-effects-manifest <path>`. The repo baseline
  `.effects-baseline/corpus.effects` is gated by
  `crates/hale-cli/tests/effects_baseline_gate.rs`; read the diff,
  then regenerate with `scripts/effects-baseline.sh`.
- Spec: `spec/verification.md` § "Default-on & opt-in analyses";
  docs: `docs/src/effects.md`.

## Claims and judgment

- `claims.rs` is **law selection** only: clause enumeration (world
  and library tiers), constitution adoption and digest, group
  resolution. It no longer evaluates.
- `claim_lowering.rs` lowers every law surface (claims blocks,
  constitutions, `@effects` / `@no_panic` / `@budget` /
  `@phase_effects`) to `ClaimIrTable`; fleet plan rows lower in
  `hale-cli/src/fleet.rs`.
- `judgment.rs` is the one authority for `hale check` and the
  artifact. Verdicts: `holds | violated | uncertified | invalid`
  (`verdict.rs`); every non-`holds` verdict carries a diagnostic.
- Spec: `spec/verification.md`; docs: `docs/src/claims.md`. Also
  `hale topology`, `hale model dump|diff`, `hale verify` (every
  finding fails, advisories included).

## Runtime, observation, record/replay

- `runtime/lotus_arena.c` (arenas, bus, pools, most builtins),
  `lotus_obs.c` (observation and recording, own TU), `lotus_tls.c`,
  `lotus_compress.c`, `lotus_shm_ring.c`, `obs_protocol.h`, `wasm/`,
  all `include_str!`'d by `codegen.rs`. `verification/*.c` are
  hand-transcribed GenMC models of the lock-free paths (`genmc` CI
  job); keep each in step with its C counterpart.
- `LOTUS_OBS=1` feeds iris and never changes the wire; cross-process
  edges need `LOTUS_OBS_WIRE=1` fleet-wide (`spec/runtime.md`).
- Record/replay (GH #296): `LOTUS_OBS_RECORD=<path>` writes the
  journal; `hale replay <recording> <prog.hl>` (`--diff`, `--feed`,
  `--at`) refuses fail-closed on an identity mismatch. The reader,
  `hale-cli/src/replay.rs`, and its test twin
  `crates/hale-codegen/tests/support/obs.rs` must track the format
  `lotus_obs.c` writes. Docs: `docs/src/systems/replay.md`.

## Embedded sources and stale binaries

The `hale` binary carries source that only a rebuild refreshes:

- **stdlib**: `crates/hale-stdlib/hl/*.hl`; a new file joins
  `AP_SOURCE` and `PATH_RENAMES` in `crates/hale-stdlib/src/lib.rs`.
- **DNA**: `crates/hale-dna/src/lib.rs` embeds the dirs listed in
  `EMBEDDED_DIRS` (`src/digest.rs`: `dna/core` with its vendored
  pond `db`, `pq` and `realtime/nats`, `host`, `operations`,
  `organization_*`, `ui`). `build.rs` digests the
  on-disk tree into `EMBEDDED_DIGEST` and a unit test holds it equal
  to the compiled-in set, so a new file is listed in `lib.rs` too.
  Organism fixtures and `hale dna` verbs run the **embedded** copy:
  after editing `dna/**`, rebuild before testing or you measure the
  old core. Compare: `hale dna --embedded-digest --from-tree <dir>`.
- **iris**: `crates/hale-iris/src/lib.rs`. The cache key
  (`toolchain_hash`) covers the version and embedded bytes, **not
  codegen or the runtime**, and a cached binary is exec'd without a
  staleness check. After a codegen or runtime change, delete
  `~/.cache/hale/iris/` before trusting `hale iris` / `hale dna`
  (verify).
- **spec**: `spec/*.md`, for `hale mcp`.

The stale-binary warning (`check_stale_cli`, `main.rs`) runs on
every source-reading command (`check`, `verify`, `build`, `run`,
`test`, `dna`, `inputs`) and has two halves: it holds every
identity-covered source (`hale_graph::identity::identity_files`: the
covered crates' `src`, `runtime` and `hl`, the lock file and the
ts-shim manifest) against the fold `build.rs` baked (F.40 phase 4,
I4), statting each file and reading them only when one is newer than
the binary; and since GH #785 it digests the workspace's `dna/**`
against what the binary embeds (`hale_dna::EMBEDDED_DIGEST`), so an
edited `dna/core` that was never rebuilt in is announced before a
fixture runs the old one. Edits outside those trees will not trip it.
`HALE_SKIP_STALE_CHECK=1` silences both. The replay identity
`HALE_TOOLCHAIN_SHA256` frames the same selection plus rustc and
`git HEAD`.

## Tests

Where a test goes:

- "Run a program, check what it computed": `tests/hale/*_test.hl`
  (`hale test tests/hale`, or `cargo test -p hale-cli --test
  hale_native_suite`). It is typechecked; the Rust codegen tests'
  harness build (`build_source` in `support/build.rs`) is not.
- Compiler output (diagnostics, IR shape, leak counts, artifact
  JSON): Rust, in the owning crate's `tests/`. Artifact tests parse
  the JSON rather than matching substrings.
- DNA CLI/host behaviour: `dna/tests/*_test.hl` (rules in CLAUDE.md).
- Examples: `crates/hale-codegen/tests/fixtures/examples/<NN>-<name>/`,
  parsed by `hale-syntax/tests/examples.rs`, run by the corpus oracle.

Rules (CLAUDE.md has the reasons):

- `cargo nextest run --release --workspace`, in parallel. Never
  `--test-threads=1` or a serializing attribute as a fix; a test
  group in `.config/nextest.toml` is only for shared state that
  cannot be removed.
- Binary paths come from `harness::unique_bin`
  (`crates/hale-codegen/tests/support/harness.rs`), ports from
  `ports::free_port()` (`support/ports.rs`).
  `harness_paths_are_unique.rs` enforces this and bans
  `set_var(` / `remove_var(` in `hale-codegen/tests/*.rs` and in all
  `crates/*/src`.
  Build knobs ride `hale_codegen::BuildOptions` (`dump_ir`, `asan`,
  `no_bus_devirt`, `no_ownership_bubble`, `lto`) or a child's
  `Command::env`.
- `cargo test` must agree with nextest. Doc comments compile as
  doctests, indented blocks included: fence non-Rust as `text`;
  `ignore` only for real Rust that cannot stand alone, with the
  reason. Check: `cargo test --release --doc --workspace`.
- An embedded Hale program in a Rust test joins the corpus and can
  move the pinned snapshots in `crates/hale-types/tests/fixtures/`
  (`HALE_REGEN_CLAIM_DIAGS=1`, `HALE_REGEN_LAW_ROWS=1`,
  `HALE_REGEN_TOPOLOGY_BASELINE=1`) and the effects baseline. Read
  the diff before regenerating.
- **Before you push a new test** (one that embeds a `.hl` program in a
  Rust string, or a new `tests/*.rs` file — the file's programs count
  too): the corpus is pinned, so the new programs move baselines that
  CI's `quick` job checks and a local run of your own test never
  touches. Run these four, in `hale-types` and `hale-cli`, before the
  push:

  ```text
  HALE_REGEN_TOPOLOGY_BASELINE=1 cargo test --release -p hale-types --test topology_projection
  cargo test --release -p hale-types --test artifact_law_projection
  cargo test --release -p hale-types --test claim_diags_snapshot
  cargo test --release -p hale-cli --test effects_baseline_gate
  ```

  The first regenerates `topology_shape_baseline.txt`: read `git diff`
  and confirm it is additions for your file only (one line per embedded
  program), then commit it with the test. The other three run WITHOUT
  regeneration and must pass; if one moves, your program changed a law
  row, a claim diagnostic or an effect, which is a finding to read, not
  a baseline to refresh. Separately, a new `tests/*.rs` in `hale-codegen`
  or `hale-cli` must join a binary: a `[[test]]` entry in that crate's
  `Cargo.toml`, or a `#[path = "<file>.rs"] mod <file>;` line in the
  area root it belongs to (`every_test_file_is_built` fails otherwise,
  and an unlisted file would never run; GH #1205).
- **A fixture that shells out** (`std::process::run`, `sh -c`, a
  `Command` in Rust) runs on macOS too: a CI gate, with no Mac to try
  it on first. Write it for both:
  - `/tmp` is a symlink to `/private/tmp`: compare a path as it was
    given, or canonicalize both sides.
  - BSD tools differ from GNU: `touch -t 200101010000`, not a free-form
    `touch -d`; no `stat -c`; `sed -i` needs a suffix (`sed -i.bak '…' f
    && rm -f f.bak` works on both); no `date -d`, `mv -T` or `readlink -f`;
    `ls` flags differ; `wc -l` pads its count (trim it: `tr -d ' '`); no
    `timeout` or `sha256sum` on a stock Mac; bash 3.2 fails on an empty
    array under `set -u`.
  - A fixture under `dna/` uses the helpers that already cover these:
    `dna::file_mode`, `dna::free_port`, `dna::run_tool_timeout`,
    `dna::sha256_file`, and `dna::scratch_root` / `dna::scratch_done` (a
    scratch root the suite collects).
  - A compose-backed fixture takes its host ports from its own seed: give
    the seed a per-run name (the pid in it) and read the ports back from
    it (`docker compose -f dna/compose.yaml port <service> <port>`, or the
    URLs `hale dna memory migrate` / `nerves migrate` print), never a
    fixed `5432` or `4222`. The ports come from a hash of the name into
    100 slots per range, so a per-run name narrows a collision; it does
    not rule one out.
  - A step only Linux has (`/proc`) is gated on `uname -s` or
    `cfg(target_os = "linux")`, and says so.
- New keyword: `hale-syntax/src/keywords.rs`, then
  `UPDATE_KEYWORDS=1 cargo test -p hale-syntax --test keyword_sync`.
- Never delete a test to make it pass; if the spec changed, update it
  in the same commit and say why.

Corpus oracles:

- `corpus_oracle.rs` (hale-codegen) runs every runnable fixture
  under the **exit** and **deadline** oracles in the normal suite,
  and under **ASan/LSan** with `LOTUS_ASAN=1 cargo test --release -p
  hale-codegen --test corpus_oracle -- --ignored --test-threads=1`.
  Its leak quarantines (`KNOWN_CLOSURE_LEAKS`,
  `LEAKS_UNMASKED_BY_NO_CHUNK_POOL`) are empty and never excuse a
  UAF, overflow, crash or hang.
- ASan builds default `LOTUS_NO_CHUNK_POOL` on (GH #816) so a
  recycled chunk cannot hide a use-after-free; set
  `LOTUS_NO_CHUNK_POOL=1` on an ordinary build to chase one.
  `LOTUS_ARENA_RESIDENCY=1` reports arenas live at exit.
- A sanitizer test that checks a value survives reads it from
  heap-allocated data (`std::str::upper(..)`, a concatenation), never
  a literal: a literal lives in static memory, so reading it after its
  locus's arena is freed is not a use-after-free and ASan says
  nothing. That is how #1065's test missed the bug #1067 fixed.
- `ownership_matrix.rs`: 27 positions × 5 types × 7 contexts = 945
  cells, four oracles (dissolve tags, residency, ASan, inline-vs-`let`
  differential); ~90-cell sample by default, `HALE_MATRIX=full` for
  all. `KNOWN_OPEN` is empty. A listed cell must FAIL: a regression
  needs an entry with a reason and an issue, and its fix deletes it.
- `corpus_check_build_agreement.rs` (`--ignored`): `hale check` must
  not accept what `hale build` refuses, against a recorded baseline.

CI (`.github/workflows/tests.yml`): `scope` (prose-only diffs skip
the substrate jobs), `quick` (LLVM-free crates), `docs`, `cli` ×3
(part 1 runs `hale fmt --check .` on the whole repo), `codegen` ×2,
`dna` ×2, `face-api`, `face-browser`, `cross`, `parity` (doctests +
shared-state binaries under libtest threads), `tsan`
(`LOTUS_TSAN=1`), `asan` (agreement, corpus ASan, `LOTUS_UBSAN=1`
ring suite), `genmc`. Run `hale fmt --check .` before committing any
`.hl`; the failing shard prints only the path. CI is the full gate:
run targeted tests locally and let the PR run the rest.

## Spec and docs

- `spec/` is the contract and describes shipped behaviour only. A
  user-visible change updates its `spec/<topic>.md` in the same
  commit; on removal, both sides delete.
- A new locked design commitment appends an `F.N` record to
  `spec/decisions.md` (append-only, never renumbered);
  `spec/design-rationale.md` holds rationale for the current surface.
- A user-facing change updates its `docs/src/` chapter (mdBook,
  `docs/src/SUMMARY.md`) in the same change. `hale` blocks there must
  parse (`crates/hale-syntax/tests/docs_snippets.rs`); `spec/styleguide.md`
  snippets must typecheck (`crates/hale-types/tests/styleguide_snippets.rs`).
- User-visible changes get a CHANGELOG fragment,
  `unreleased/<pr-number>.md`, added once the PR has a number
  (CLAUDE.md § "CHANGELOG entries are fragments"); a closed deferred
  question is resolved in `notes/open-questions.md`.

## Performance

Benches live in the sibling repo `hale-lang/bench`;
bench lowering, C-runtime and bus-dispatch changes before and after.
`run.sh` picks `$HALE_BIN`, then `../hale/target/release/hale`, then
PATH (pass `HALE_BIN` from a worktree) and exits 1 on a Hale
regression. `--update-baselines` is retired: `./rebaseline.sh [runs]
[iters]`, then commit the medians with a reason.

## Design stances

- `fallible(E)` is the value-error protocol; structural failure is
  closure violation. No `Result` / `Option`. `interface` is
  structural; `trait` is reserved with no semantics.
- No `lotus_*` to `hale_*` rename. No feature flags for staged
  rollout (`LOTUS_NO_*` switches are diagnostics). `unsafe` only
  where inkwell/LLVM or libc FFI demand it.
- Keep `mNN`, `F.N` and `GH #N` references in comments. Prefer
  records (`Ty`, `Span`) over stateful structs (`Cx`, `Parser`)
  unless the thing genuinely accumulates state.

## Working rules

- The user owns commit cadence: never commit, push, tag or release
  without an explicit ask. Don't skip hooks (`--no-verify`).
- Recent history uses `<area>: <what changed> (GH #N)` subjects,
  bodies wrapped at 72 columns.
- No planning, status or progress markdown in the repo.
- Downstream project names never appear in anything public: write
  "downstream handoff" and rename domain identifiers.
- GitHub PR and issue bodies go through `--body-file` / `-F`, are
  not hard-wrapped, and never carry claude.ai session links or
  session trailers. Never put backticks inside a double-quoted bash
  string.
- Don't invent grammar or stdlib paths: spec first, then implement.
