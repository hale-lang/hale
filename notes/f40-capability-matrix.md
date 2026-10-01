# F.40 phase 3, P3: the capability matrix

**Anchor.** hale-lang/hale#1212, the Final direction: "Approximate is legitimate only in layers 5 and 7 … everywhere else a backend lowers or rejects. Consolidating today's wasm restrictions (the `check.rs` stdlib table, `link_wasm`, the per-site codegen skips) into one matrix is in F.40; new backends are not." The phase-3 plan (`notes/f40-phase3-plan.md` on `f40/phase3-plan`, § Line P, P3, and §6). The wave-2 decisions: P3 proceeds as a correction, under these conditions: check, build and the editor agree on admission; the effective target has a precedence; a capability refusal is not a missing linker or SDK; no target leaks across snapshots; thread-dependent lifecycle obligations go only after the matrix rejects or lowers what needs them; and the eager, deferred and return spines are each tested. Lifecycle line 16 is decided (b), with P3 as the authority: the target's lifecycle obligations come from the matrix, and gating the eager spine is an interim correction. The registry rows: `target_capability` × 7 and `dispatch` × 1 (`bus_payload_is_flat`).

**What this is.** The D step of P3. It is read-only on code and measured on `main` 13e5f352. It fixes the matrix's shape, the cell that replaces each legacy row, the intentional diagnostic changes, the lifecycle cells, the generated documentation and the order of the three PRs. The consolidation pane runs from this after the driver's review. The decisions it needs are numbered **T1–T7** in §7. Each carries a recommendation.

Abbreviations: **CG** `crates/hale-codegen/src/codegen.rs`, **INST** `crates/hale-codegen/src/locus/instantiation.rs`, **CHECK** `crates/hale-types/src/check.rs`, **RT** `crates/hale-codegen/runtime/lotus_arena.c`, **SHIM** `crates/hale-codegen/runtime/wasm/lotus_wasm_shim.h`, **SNAP** `crates/hale-frontend/src/snapshot.rs`, **OPT** `crates/hale-cli/src/shared/options.rs`, **TGT** `crates/hale-codegen/src/target.rs`.

## 0. What the tree does today

Where a target decision is made, and who sees it:

| decision | where | which entry points see it |
|---|---|---|
| backend selection | `--target` only: OPT:281–318 parses it via `TargetSpec::parse` (TGT:266; aliases `wasm`, `wasm32`, `wasm32-unknown-unknown`) and sets `CompileTarget::Wasm32` (OPT:312). Codegen ignores a source `target` declaration (CG:13284 is a no-op arm). | build |
| the checker's target | `Bundle::target_has_async_io` / `target_label` (`crates/hale-types/src/symbol.rs:60–63`; default host at :98), set from `Config::target` (SNAP:818). `Config::check` and `Config::editor` pin `Target::host()` (SNAP:163–175, :208). `hale build` names its target (OPT:487–499). `hale check` has **no `--target`** (`crates/hale-cli/src/verbs/check/run_impl.rs:103`). | build only, for anything target-named |
| the stdlib gate | `wasm_target` = a top-level `target wasm`/`browser_js` in any program (CHECK:589–594); the table `wasm_unavailable_stdlib` (CHECK:8088–8126), consulted for call forms only (CHECK:14811–14825). Never set from `--target wasm32`. | check, build, editor, but only for source-declared wasm |
| `async_io` | `TargetSpec::has_async_io` (TGT:319–325): true for wasm32 (`TargetOs::None`), false for musl and Windows. The checker refuses `where async_io` when it is false (CHECK:10081–10094). | build only (`hale check` always has the host's true) |
| `--wrap-main` | a string match on `--target wasm32`/`wasm` (`crates/hale-cli/src/verbs/build.rs:56–58`), separate from `TargetSpec::parse` | build |
| `[ffi] link` on wasm | `link_wasm` (CG:2816–2824), **after** clang has compiled the runtime (CG:2786–2799) | build, and only when clang is present |
| entry inversion | `has_exports` relaxes "program has no `fn main()`" only `if self.is_wasm` (CG:8779–8789); `@export locus` with `run()` refused in codegen (CG:14130–14135) | build |
| run / replay | `hale run`/`replay --target wasm32` refused at argument parsing (OPT:386–391) | run, replay |
| FFI type portability | `ffi_type_unportable` (CHECK:8012), target-independent, for `@export` (CHECK:12106–12160) and `@ffi` (CHECK:12161–12195) | all |
| the direct-call gate's third leg | `bus_payload_is_flat` (`crates/hale-codegen/src/bus/wire.rs:1932`), read at the publish site (`bus/dispatch.rs:706`, :1388) | build |

The snapshot key already carries the target. `SnapshotKey.target` is the configured target's name (SNAP:456, :483), and the config digest covers its name, `has_async_io` and label (SNAP:215–219). The test at SNAP:1525–1560 pins that a musl snapshot of the same seed has a different key and computes nothing it was not asked for.

## 1. The matrix's shape

### 1.1 Rows

```text
CapabilityMatrix = { (TargetClass, Capability) → Cell }

Cell     = { verdict: Lower(Lowering) | Reject(Refusal), witness: Witness }
Witness  = { site:   the producer, a stable path::symbol (the registry's convention),
             reason: the one sentence a diagnostic or doc renders,
             spec:   the spec anchor whose sentence the cell implements }
Refusal  = { wording: the diagnostic template (today's text verbatim where one exists),
             guidance: the substitute, if any }
Lowering = Emit | Absent { because: Capability }       -- Absent: the obligation or runtime
                                                       -- call is not emitted, legitimately,
                                                       -- because the cell it depends on
                                                       -- rejects (or the CLI refuses) every
                                                       -- behaviour that needs it
```

**Targets** (`TargetClass`). It is derived from `TargetSpec`'s `(arch, os, env)`, not from the triple's name: `PosixAsync` (glibc Linux, macOS), `PosixNoAsync` (musl), `Wasm32`. Windows stays `Planned` and is refused at argument parsing (OPT:292–300). It never reaches the matrix, because a tier is not a capability. Today the matrix has two populated columns, host and wasm32. musl joins only for the `async_io` cell it already has. A new target is a new column. **No cell has a default arm**: a capability added without a cell for every class fails the matrix's own law test (§1.6).

**Capabilities.** These are the facts the seven rows decide today, plus the three lifecycle obligations line 16 names:

| capability | key | layer | the legacy row it comes from |
|---|---|---|---|
| `StdNamespace(path)` | every `std::` namespace the stdlib defines (`crates/hale-stdlib/src/lib.rs:185–212` plus the codegen-native modules `crates/hale-codegen/src/stdlib/*.rs`) | 8 | `wasm_unavailable_stdlib`, `wasm_target` |
| `LinkLibrary` | `[ffi] link` / `--link` | 8 | `link_wasm` |
| `ExportSurface` | `@export fn`, `@export locus`, `_hale_start`, the fixed exports | 8 | `link_wasm` (export list), CG:8779, CG:9377 |
| `EntryInversion` | an `@export`-only program with no `fn main`; `@export locus` with `run()`; `--wrap-main` | 8 | CG:8779–8789, CG:14130, `build.rs:56` |
| `ForeignAbi(c \| js)` | `@ffi("c")`, `@ffi("js")` | 8 | CG:4268, CG:14351 (marshalling) |
| `FfiType(ty, abi)` | the FFI-portable type set | 2 | `ffi_type_unportable` |
| `AsyncIoPool` | `where async_io` | 5 | `TargetSpec::has_async_io` |
| `Threads` | `pinned` placement; `cooperative(pool = X)`, X ≠ `main` | 5 | (none: admitted silently, §2.5) |
| `RemoteTransport(kind)` | a `bindings { }` entry: `shm_ring`, `unix`, `udp`, CONNECT roles | 5 | (none: spec/ffi.md:423 says "unavailable in the sandbox"; CG:9813 lowers the prelude) |
| `ReplayIngress` | record, replay, observation identity | 6 | INST:4584, CG:9673, CG:9824 |
| `ProcessSignals` | the SIGINT drain flag, SIGPIPE, the drain observer | 6 | CG:9444, CG:9849, CG:33668, `dissolve.rs:227`, `restart.rs:169`, `stdlib/time.rs:495` |
| obligations `PoolJoin` (R20), `WaitAbort` (R34), `IngressQuiesce` (R35) | the teardown spines | 6 | INST:4859–4865, CG:7125, CG:6954, CG:9984, CG:10029, CG:22585 |

`bus_payload_is_flat` is **not** a target cell. It is a property of a topic's payload type and is target-independent today. It is wire-format specific, but both targets share the wire format (spec/ffi.md:414–425). It becomes a column of `dispatch`'s gate row (§2.8). P3 carries it because the registry assigns it there and because the gate's last leg is the one codegen still decides.

`FfiType` is target-independent today too (§2.6). It is kept as a matrix column keyed by ABI so the `js` marshalling (Int crosses as f64, CG:14351–14358) has one home, but every cell is identical across targets.

**Approximation.** The verdict is binary. A layer-5 or layer-7 capability may be `Lower` with a witness citing F.38 ("placement is semantics-free") for a collapse. No cell uses that today, and T2 keeps it that way for wasm32. The law in §1.6 refuses an approximating witness on any other layer.

**What is not a cell.** Target-specific *emission choices* stay `TargetSpec` queries, never cells. The Final direction: "Target-specific emission choices and walking a typed body are not decisions in that sense." These are the 13 backend sites of §2.5: the triple, CPU, optimization level, LTO, pass pipeline, DWARF, attributes and pointer width. Also not cells: tiers (`Supported`, `Cross`, `ObjectOnly`, `Planned`), output file naming (`TargetSpec::filenames`, TGT:343) and the local toolchain.

### 1.2 Where it lives

`TargetSpec` moves from `hale-codegen` to `hale-types::target`. It is plain data with no LLVM in it (TGT:1–30 says it is the model of a target). Codegen re-exports it, so every `hale_codegen::target::*` path keeps compiling. `hale_frontend::snapshot::Target` (SNAP:101–107) becomes `{ name, spec: TargetSpec }`, and `has_async_io`/`label` become queries on the spec. The matrix is `hale-types::capability`: a `const` table of cells plus `fn cell(TargetClass, Capability) -> &Cell`. The registry row `target_capability` names it as its producer.

### 1.3 The effective target

Three sources:

1. **The source declaration.** A top-level `target wasm { }` or `target browser_js { }`. A nested one is a parse error (GH #901, `crates/hale-syntax/tests/target_in_module.rs:58–74`). `--wrap-main` injects one (`crates/hale-syntax/src/desugar.rs:180–193`).
2. **The configuration.** `--target` on `hale build`, and on `hale check`, which gains the flag in P3 2 of 3, parsed by the same `TargetSpec::parse`.
3. **The host**, when neither names one.

**The precedence rule (T1).**

- An explicit `--target` is the effective target.
- With no `--target`, a source declaration makes the effective target wasm32, for admission on every entry point.
- With neither, the target is the host.
- An explicit `--target` whose class disagrees with a source declaration is a **located refusal at the declaration**: ``this program declares `target wasm`, and is being checked/built for `<triple>`: build it with `--target wasm32`, or drop the declaration``. This is #911's ruling applied to the other half: "a stated directive that does nothing is worse than a refusal".
- `hale build` with **no** `--target` on a program that declares `target wasm`: today it builds a native binary, refused late as "program has no `fn main()`" when the program is `@export`-only (CG:8787). Under T1(a) it is the same located refusal, naming `--target wasm32`, so artifact selection stays on the command line. Under T1(b) it selects wasm32 and makes spec/ffi.md:378 ("opts into the wasm backend") true.

`--wrap-main`'s guard (`build.rs:56–58`) reads the effective target instead of matching strings, so `--target wasm32-unknown-unknown --wrap-main` is admitted.

The editor has no `--target`. Its effective target is the source's, else the host. "Equivalent configuration" for the agreement test therefore means `hale check` and `hale build` without `--target`, compared with the editor. With `--target`, it means `hale check --target T` compared with `hale build --target T`.

**Precedence tests** (P3 2 of 3, `crates/hale-cli/tests/target_precedence.rs`, a new `[[test]]` entry or a module of an existing area). The rows are source × configuration, and each cell names the effective target and the expected diagnostics, pinned by wording:

| source \ CLI | none | `native`/host triple | `wasm32` | `wasm32-unknown-unknown` | musl triple |
|---|---|---|---|---|---|
| no declaration | host | host | wasm32 | wasm32 | musl |
| `target wasm` | wasm32 (build: T1) | refusal at the declaration | wasm32 | wasm32 | refusal at the declaration |
| `target browser_js` | as `target wasm` | as `target wasm` | wasm32 | wasm32 | as `target wasm` |
| `fn main` + `--wrap-main` | refused (`--wrap-main` needs wasm32) | refused | wasm32 | wasm32 (**today refused**) | refused |

Each cell runs `hale check`, `hale build`, and the editor's snapshot (`Config::editor` over the same files) where the CLI column is "none", and asserts that the three agree on admission.

### 1.4 Consulted on every entry point, before lowering

The matrix is demanded as a snapshot family, `target_capability`, keyed by the snapshot, so the configured target is in its key. It produces:

- the effective target (§1.3);
- one **use row** per capability use in the program: a `std::` call path (the site the CHECK:14811 gate reads today), a placement entry naming `pinned`/a pool/`where async_io`, a `bindings` entry, an `@export` declaration, an `@ffi("js")` declaration, a `run()` on an `@export locus`, plus the manifest's `[ffi] link`;
- the refusals: every use whose cell is `Reject` for the effective target, as a located `Diag` carrying the cell's witness.

`Snapshot::demand_check` appends the refusals beside `build_rule_diags` (SNAP:1088–1091). The check, the build and the editor therefore show the same refusals. The build is gated by them through `demand_lowering` (SNAP:1104–1113), and the editor publishes them as it publishes every check diagnostic. The entry points, all through `Snapshot::load`:

- `hale check`: `run_impl.rs:103`
- `hale build`: OPT:492
- `hale run`, `test`, `bench`: `verbs/test.rs:82`, `verbs/bench.rs:243`, and `run` through `parse_build_options`
- the LSP: `crates/hale-lsp/src/lib.rs:810`
- the test harness: `Config::harness`, SNAP:202

The harness lowers without the check gate (SNAP:197–203). It still reads the cells for lowering, so a harness wasm build emits what a CLI wasm build emits.

Codegen **reads** the matrix through the lowering view and decides nothing. Each SKIP/SUBSTITUTE site of §2.5 becomes `cells.lowering(cap)`, and each EXPORT site reads `ExportSurface`'s `Lower` data.

### 1.5 A capability refusal is not a toolchain failure

The rule: a refusal that depends only on (program, configuration, target) is a cell. It is a located checker diagnostic, on every entry point. A failure that depends on the machine running the compiler stays a build-time `CodegenError::Link`/`LlvmEmit`, unlocated, naming the missing piece.

| failure | class today | class after |
|---|---|---|
| `[ffi] link` on wasm32 (CG:2816) | build-time, and **masked by a missing clang**, because it is checked after the runtime compile (CG:2786–2799) | **cell** (`LinkLibrary × Wasm32 = Reject`). On `hale check` it is a manifest record (T4). On `hale build` it is reported before any tool is probed |
| `@export locus` with `run()` (CG:14130) | build-time (codegen) | cell (`EntryInversion`), same wording |
| `@export`-only program built natively (CG:8787) | build-time, "program has no `fn main()`" | cell, located at the first `@export` (T1 decides the wording) |
| clang missing (CG:2793), wasm-ld missing (CG:2876) | build-time | unchanged |
| a `csrc` unit that will not compile freestanding (CG:2835–2842) | build-time | unchanged: it depends on C source the checker does not read |
| zig / target sysroot missing (spec/projects.md:752) | build-time | unchanged |
| `libhale_ts_shim.a` not built (CG:2157) | build-time, native only | unchanged for native. On wasm32 `std::ts` becomes a cell (T3), because today the wasm path returns at CG:1954 and never reaches CG:2157 |
| Windows triple (OPT:292) | argument parsing, "not buildable yet" | unchanged: a tier, not a capability |
| `hale run --target wasm32` (OPT:386) | argument parsing | unchanged: the host cannot execute the artifact, which is a fact about the host |

Test: on a PATH with no clang (a `Command::env("PATH", …)` on the child, never `set_var`), `hale build --target wasm32` of the `link = ["m"]` app reports the `[ffi] link` refusal, not "is clang installed?".

### 1.6 The matrix's own laws (a unit test in `hale-types`)

- Every `(TargetClass, Capability)` pair has exactly one cell.
- Every `Reject` has non-empty wording and a spec anchor that exists in `spec/`.
- An approximating witness appears only on a layer-5 or layer-7 capability.
- Every `Absent { because }` names a capability whose cell on that target is `Reject`, or one the CLI refuses on that target (`ReplayIngress` on wasm32: OPT:386).
- Every `StdNamespace` cell's key is a namespace the stdlib defines, and every stdlib namespace has a cell. The second half is checked against `hale-stdlib`'s file list and codegen's `stdlib/` modules.

### 1.7 No target leaks across snapshots

- The effective target is a function of the key's configured target and the sources. Both are in the key: SNAP:456 `target`, SNAP:483 `sources_digest`, and the editor's `overlay_digest`. A cached `target_capability` result therefore cannot serve another target's snapshot.
- `Target` grows `spec`, and `Config::digest` folds the spec's `(arch, os, env)` beside the name (SNAP:215–219). This keeps two configured targets that share a name from sharing a digest.
- The runtime object cache is keyed by its compile flags, which include the target (CG:2080 and spec/projects.md:752: "one target's never serve another"). That is unchanged.
- Test (P3 1 of 3), extending SNAP:1525–1560 with a wasm32 config over a seed whose one file calls `std::process::pid()` and has no declaration: the host snapshot reports nothing, the wasm32 snapshot reports the refusal, `builds()["target_capability"]` is 1 in each, and the keys differ in `target` and `config_digest`.
- Test (P3 2 of 3): one LSP session that edits a `target wasm` line in and out of a file. The refusal appears and disappears with the line, and the build count shows one capability derivation per snapshot.

## 2. One section per legacy row

Each section gives the question the row answers today, the cell that replaces it, and what moves. The **diagnostic change list**, program by program over the wasm corpus and tests, is §2.9. Every cell not named there keeps today's answer.

### 2.1 `wasm_unavailable_stdlib` (CHECK:8088)

**Today.** For a `std::` call path under a source-declared wasm target: is it one of the 8 rejected prefixes, and if so, with what reason? The prefixes are `io::tcp`, `io::udp`, `io::tls`, `io::fs`/`io::file`, `io::stdin`/`io::stdout`, `term`, `process`, `http`. Everything else passes, including namespaces whose wasm lowering is a stub (below). The comment at CHECK:8083–8087 lists `time`, `env` and `rand` as portable.

**Cell.** `StdNamespace(p) × Wasm32` is `Reject` for the 8 prefixes. The wording is unchanged: `` `std::<path>` is unavailable under `target wasm`: <reason> `` (CHECK:14816–14823). Once `--target wasm32` can trigger it, T1's wording adds "or `--target wasm32`". The reasons and substitutes are the table's strings. `StdNamespace(p) × Host` is `Lower` for every namespace.

**The namespaces with no stated cell today (T3).** They are admitted on wasm32 and lowered to stubs. `--allow-undefined` (CG:2872) and the loader's `() => 0` for every unknown import (CG:2983–2986) turn their syscalls into silent zeros:

- `std::time`: `clock_gettime` returns 0 and `nanosleep` returns at once (SHIM:483–488); `sleep` calls `clock_nanosleep` (`stdlib/time.rs:614–618`), an import stubbed to 0.
- `std::env`: `getenv` returns NULL (SHIM:36).
- `std::ts`: the wasm link returns at CG:1954, before the native shim check at CG:2157.
- `std::io::unix`, `std::sockopt`, mirror/ring (`stdlib/io_unix.rs`, `sockopt.rs`, `mirror.rs`, `ring.rs`): syscall-backed and not in the table. Unmeasured.

P3 1 of 3 classifies each by an objective oracle: build a one-call program per namespace for wasm32, list the module's function imports (`WebAssembly.Module.imports`, as the loader does at CG:2983), and call a namespace `Lower` only if every import is in the loader's writer set or a declared `@ffi("js")`. A namespace that fails the oracle is a T3 line, `Reject` by recommendation.

**Moves.** The gate reads the effective target instead of `wasm_target`. The seam (`spec/registry.md`: `wasm_unavailable_stdlib(` ×2 in `check.rs`) moves to `capability.rs`. The table becomes the cells, and the function is deleted. Call forms only, as today: a path in type position (`std::io::tcp::Stream` as a parameter type, `crates/hale-cli/tests/target_model.rs:331`) is not a use. That is a deliberate keep, so a portable signature can name a type.

### 2.2 `wasm_target` (CHECK:589)

**Today.** Does any program declare `target wasm`/`browser_js` at the top level? It gates §2.1 and nothing else. Codegen and the CLI do not read it.

**Cell.** None. It becomes the source term of the effective target (§1.3). `Checker::wasm_target` (CHECK:8149), `target_has_async_io` and `target_label` (CHECK:8152–8153) collapse into one `&CapabilityView` on the checker.

**Moves.** The two other readers of the declaration keep reading syntax: `desugar.rs:181` (wrap-main's "already declared" test) and the parser's top-level rule. `Bundle::target_has_async_io`/`target_label` (symbol.rs:60–63) are deleted. The checker no longer reads the target from the bundle; it reads the matrix's use rows.

### 2.3 `link_wasm` (CG:2739)

**Today.** It holds two refusals: `[ffi] link` (CG:2816) and a `csrc` compile failure (CG:2835). It also holds the export list: `main` if defined, `__heap_base`, `memory`, `lotus_wasm_alloc`, `lotus_wasm_set_inbox`, plus `cx.wasm_exports` (the `@export` wrappers and `_hale_start`), at CG:2855–2869. And it holds the import policy: `--gc-sections --allow-undefined` (CG:2870–2873).

**Cells.**
- `LinkLibrary × Wasm32 = Reject`, with today's wording (CG:2818–2822).
- `LinkLibrary × Host = Lower`.
- `ExportSurface × Wasm32 = Lower(Emit)`. Its data is the fixed export list plus the `@export` set. `link_wasm` reads that list instead of spelling it.
- `ExportSurface × Host = Lower(Emit)`: `@export fn` is an unmangled C symbol, and `@export locus` is an ordinary locus (spec/ffi.md:473–478).
- The `csrc` failure stays toolchain (§1.5).

**Moves.** The refusal moves ahead of the clang probe and into the check (T4). The import policy stays a link flag, but P3 3 of 3 adds a **backstop test**: every module the wasm tests build imports only the loader's writer set plus its declared `@ffi("js")` names. That is the oracle that the matrix has no silent stub left (T7).

### 2.4 `lotus_replay_start_ingress` (INST:4584) and the lifecycle residue

**Today.** On the main locus, replay ingress is skipped on wasm (INST:4584). The registry also says: "instantiation still emits pool shutdown and wait-abort on wasm where the main exit does not". Measured, that is half right (§3.1). Pool shutdown on wasm is emitted only by the eager spine (INST:4862). Wait-abort is emitted on wasm by the eager spine (INST:4865) **and** by the fall-through, test-failure and return spines, through `emit_frame_teardown`'s ungated `if self.in_main` (CG:6954–6956). Only the deferred spine (CG:7125) omits all three.

**Cells.**
- `ReplayIngress × Wasm32 = Lower(Absent { because: ReplayIngress refused at the CLI })`. `hale run`/`replay --target wasm32` are refused (OPT:386), and `getenv` returns NULL (SHIM:36), so no replay mode can arise.
- `ReplayIngress × Host = Lower(Emit)`.
- The same cell covers CG:9673 (observation identity) and CG:9824 (`lotus_obs_eager_init`).
- The obligations are §3.

**Moves.** The skip becomes `cells.lowering(ReplayIngress)`, with identical IR.

### 2.5 `is_wasm` (CG:3136; 33 references, 30 sites read it, plus two that branch on the target without the name: INST:888, CG:9590)

**Today.** Every site, classified:

| kind | sites | becomes |
|---|---|---|
| backend configuration (13) | CG:1164, 1173, 1182 (unreachable via the CLI), 1310, 1327, 1347, 1384, 1510 (DWARF silently ignored on wasm), 1723, 1790, 1861, 1883, 2080 (dead for wasm: it returns at CG:1954) | `TargetSpec` queries; not cells. CG:1182 and CG:2080's wasm arm are deleted as dead |
| link (1) | CG:1954 | `TargetSpec::is_wasm()` chooses `link_wasm`; the refusals inside are §2.3 |
| exports / entry inversion (3) | CG:8779, CG:9377 (refusals inside at 14130, 14187), INST:888 | `ExportSurface`, `EntryInversion` |
| skip-emit (12) | CG:7125, 9444, 9673, 9824, 9849, 9984, 10029, 22585, 33668; INST:4584, 4859; `locus/restart.rs:169` | `cells.lowering(cap)`, one capability each (table below) |
| substitute (2) | `locus/dissolve.rs:227`, `stdlib/time.rs:495` | `ProcessSignals` |
| target-keyed, not wasm-named (1) | CG:9590 `self.target.has_async_io()`, which **emits** `lotus_coop_pool_enable_async_io` on wasm | `AsyncIoPool` |

The skip/substitute sites by capability:

| capability | sites | what wasm gets today |
|---|---|---|
| `ProcessSignals` | CG:9444 (`lotus_io_init`, SIGPIPE), CG:9849 (`lotus_drain_signals_install`, key extractors, `lotus_bus_load_config`), CG:33668 (drain observer), `dissolve.rs:227` (`self.draining` without the process flag), `restart.rs:169` (no `process_draining` term), `time.rs:495` (a sleep never cut short by drain) | Lowered legitimately: wasm32 has no signal source, the flag exists and is always 0 (RT:1865), and the runtime stubs the installer (RT:10025–10033). The cell is `Lower(Absent { because: ProcessSignals })`, with `ProcessSignals × Wasm32 = Reject` and no source construct to reject, since SIGINT is not a program construct. The IR is identical |
| `ReplayIngress` | INST:4584, CG:9673, CG:9824 | §2.4 |
| `RemoteTransport` | CG:9849 (`lotus_bus_load_config`); quiesce at CG:7125, 9984, 10029, 22585, INST:4859 | **Not refused.** `emit_bindings_prelude` (CG:9813) is ungated, and the transports' syscalls are no-op imports. T2 |
| `Threads`, `AsyncIoPool` | pool join skipped at CG:7125, 9984, 10029, 22585 and emitted at INST:4862. **Ungated:** pinned `pthread_create` (INST:4262–4272), pinned `pthread_join` (CG:7182), `lotus_coop_pool_register`/`start_all` (CG:9546–9647), `enable_async_io` (CG:9590) | **Not refused, and they never run.** `pthread_create` is a `() => 0` import. Pool start and shutdown are not under `#ifndef __wasm__` (RT:10075, RT:10149, outside RT:9827–10034), despite SHIM:47–53 saying so. `LOTUS_HAVE_ASYNC_IO` is 1 on wasm (RT:142–147), over epoll/eventfd imports that return 0. So a pinned locus or a pool's `run()` is posted and never executes. T2 |

**Moves.** The field `is_wasm` goes. Backend sites read `self.target` (already present: CG:3137). The rest read cells. The registry row closes with "emission configuration through `TargetSpec` only; every capability through the matrix".

### 2.6 `ffi_type_unportable` (CHECK:8012)

**Today.** Is a type in the FFI-portable set, for an `@export` or `@ffi` parameter or return? It is target-independent. The one target-dependent part of the boundary, the `js` ABI's f64 marshalling of i64-class scalars (CG:14351–14358, CG:14465, CG:14539, CG:15631, CG:15725), is not in the row.

**Cell.** `FfiType(ty, abi) × T` is identical for every `T`, today's verdict per type. The wording is unchanged (CHECK:12109–12190).

`ForeignAbi(js) × Host` is a new question. An `@ffi("js")` declaration on a native build is not refused today. Codegen declares it as an external symbol, so a call fails at the native link as an undefined reference. That is inferred from CG:4268 and not tested. T5 recommends `Reject`, located at the declaration.

`ForeignAbi(js) × Wasm32` and `ForeignAbi(c) × *` are `Lower`.

**Moves.** None in diagnostics. The predicate becomes the matrix's column, and the marshalling sites read `ForeignAbi(js)`'s lowering data instead of `a.abi == "js"`.

### 2.7 `TargetSpec` (TGT:128)

**Today.** It holds `has_async_io`, true for wasm32 (TGT:322, pinned by `async_io_follows_the_libc`, TGT:638–649), and the platform label. The checker sees them only under `hale build` (§0).

**Cells.**
- `AsyncIoPool × PosixAsync = Lower`.
- `AsyncIoPool × PosixNoAsync = Reject`, with today's wording (CHECK:10084–10093).
- `AsyncIoPool × Wasm32 = Reject` (T2). This is new: today it is admitted and never runs (§2.5).

`TargetSpec::has_async_io` stays as the runtime-shape fact. It must agree with `LOTUS_HAVE_ASYNC_IO`, which is 1 on wasm. The *capability* is the cell, and the test at TGT:645 changes from asserting `has_async_io` to asserting the cell.

**Moves.** `hale check --target`, and the matrix on every entry point (§1.4). The checker's async_io gate (CHECK:10081) reads the cell.

### 2.8 `bus_payload_is_flat` (the `dispatch` row)

**Today.** The plan (`crates/hale-model/src/dispatch_plan.rs:24–34`) calls `StaticDirect` "the quiet/flat/same-thread tier". The plan computes it from `direct_call_eligible`, which is same-thread and quiet (`crates/hale-types/src/bus_graph.rs:320–331`). Flatness is ANDed only at the publish site (`bus/dispatch.rs:1388`). A non-flat payload on a direct-eligible subject therefore lowers as a static enqueue while the plan, and the execution digest that frames it (`resolved.plan.digest()`, OPT, the registry's consumer list), say `static_direct`. The registry invariant "which flavour a subject gets is a plan conclusion" does not hold for those subjects.

**Column.** `payload_flat: bool` on the gate row, computed in the frontend from the topic's resolved payload `Ty`. Every publish to a subject carries the topic's declared payload type, so this is a per-subject fact. The rule is `bus_payload_is_flat`'s, moved verbatim onto resolved types: a struct whose every field is `Int | Float | Bool | Decimal | Duration` or a no-payload enum, and nothing else (wire.rs:1932–1975). It keeps the superset-safe default-false. `DispatchFlavor::of` takes it as the third argument. Codegen reads the flavor and stops calling the predicate.

**Moves.** The emitted code is identical. The plan rows of non-flat direct-eligible subjects change from `static_direct` to `static_bucket` in `--dump-model`'s plan and in the execution digest. That is an intentional fact change, in the J commit's fragment. The shadow (P3 3 of 3) compares the column with codegen's predicate at every publish site over the corpus and the matrix sample, with zero divergences expected. `bus/wire.rs` keeps the codec. Only the predicate moves.

### 2.9 The diagnostic change list (the J commit)

Every wasm-relevant program in the tree is listed below. There are 33 in tracked files: 31 in Rust test strings, `iris/examples/wasm-flower/flower.hl`, and the `play/` set. "Unchanged" means identical diagnostics on every entry point.

| program | today | after P3 | change |
|---|---|---|---|
| `crates/hale-types/tests/wasm_target_gating.rs:16` `target_wasm_rejects_posix_stdlib` (5 programs, fs/tcp/tls/term/process) | refused by CHECK:14811 under the source declaration | the same refusals, through the cell | wording gains "or `--target wasm32`" only if T1's wording is taken; otherwise unchanged |
| `wasm_target_gating.rs:38` `target_wasm_allows_portable_stdlib` | accepted | accepted | unchanged |
| `wasm_target_gating.rs:57` `no_target_decl_does_not_gate` | accepted (host) | accepted (host) | unchanged; the companion `--target wasm32` case is new (paired case 1) |
| `crates/hale-codegen/tests/wasm_target.rs`, 15 programs (lines 50, 87, 139, 208, 256, 322, 381, 460, 522, 574, 634, 686, 747, 792, 846) | harness builds for `Wasm32`, no check | the same | unchanged: none calls a gated namespace, places a locus, binds a topic or uses `std::time`/`env`/`ts` |
| `crates/hale-cli/tests/wasm_package_csrc.rs:115` `a_system_link_dependency_is_refused_on_wasm` | `hale build --target wasm32` refused at CG:2816, after clang | the same wording, before any tool is probed; **`hale check --target wasm32` now refuses it too** (T4) | **new on `hale check`** |
| `wasm_package_csrc.rs:89` `a_broken_package_csrc_fails_the_wasm_build` | build-time csrc compile failure | unchanged (toolchain class) | unchanged |
| `wasm_package_csrc.rs:79` `a_package_csrc_builds_for_wasm` | builds | builds | unchanged |
| `crates/hale-cli/tests/target_model.rs:329` `async_io_is_judged_against_the_target` | `hale build --target <musl>` refused by CHECK:10081; `hale check` (host) accepts | the build is unchanged; **`hale check --target <musl>` refuses**, same wording | **new on `hale check`** (a new flag, so no existing invocation changes) |
| `target_model.rs:449` `the_existing_aliases_are_unchanged`, `crates/hale-cli/tests/build_output_path.rs:125`, `crates/hale-cli/tests/wasm_link_is_quiet.rs:58`, `:63` | build | build | unchanged |
| `crates/hale-cli/tests/check_arg_parsing.rs:465` (`hale run --target wasm32`) | refused at OPT:386 | the same | unchanged |
| `--target wasm32-unknown-unknown --wrap-main` (no test today) | refused by `build.rs:56` | admitted | **a refusal removed**; pinned by a new test |
| `--wrap-main` program calling `std::process` (no test today) | refused by `hale build --target wasm32 --wrap-main`; `hale check` accepts | the same; `hale check --target wasm32` refuses it as well, because the check of a `--wrap-main` program is a check of the wrapped program only when the flag is given (see the note after this table) | new on `hale check --target wasm32` |
| `iris/examples/wasm-flower/flower.hl` (declares `target wasm`, `@export fn` only) | `hale check` accepts; `hale build` without `--target` fails late in codegen ("program has no `fn main()`", CG:8787, inferred); untested (`crates/hale-cli/tests/iris_seeds_check.rs:31` claims a coverage that does not exist) | `hale check`: accepted, effective wasm32. `hale build` without `--target`: **T1's located refusal**. `hale build --target wasm32`: builds | **changed late failure → located refusal**; flower joins the wasm build tests |
| corpus sweep (`crates/hale-codegen/tests/corpus_check_build_agreement.rs:193`): harvested declared-wasm programs `wasm_target.rs` #13 (522), #15 (634), #16 (686); `wasm_target_gating.rs` #0 (39); `crates/hale-syntax/tests/wrap_main.rs` #1 (90) | check-clean (gate fires, nothing gated), then built **natively** | built for their effective target, wasm32 (the sweep reads the snapshot's effective target instead of `build_opts::options()`'s `Native`) | no diagnostic change; the oracle now compares like with like |
| `play/examples/*.hl`, `play/ui.hl`, `play/sim.hl` (`play/build.sh:51,63`; deploy-only, `.github/workflows/docs.yml:121–123`) | build with `--target wasm32 --wrap-main` | the same | unchanged (verified: none places, binds, sleeps or reads env) |
| docs `hale` blocks (`docs/src/systems/webassembly.md:23, 57, 187`), spec/ffi.md:384, 435 | parse only (`docs_snippets.rs`) | the same | unchanged |
| a program using `pinned`, `cooperative(pool = X≠main)`, `where async_io` or a `bindings { }` entry under wasm32 | admitted; never runs | `Reject` (T2) | **new**; no program in the tree is affected |
| a program calling `std::time`/`env`/`ts` (or a namespace failing the import oracle) under wasm32 | admitted; stubbed | `Reject` per T3 | **new**; no program in the tree is affected |
| an `@ffi("js")` declaration in a native build | native link failure (inferred) | located refusal (T5) | **new located diagnostic**; no native program in the tree declares one |
| `@export locus` with `run()` under wasm32 | codegen refusal (CG:14130) | checker refusal, same wording | **moves to `hale check`**; no program in the tree |

The `--wrap-main` note: `--wrap-main` is a build flag (`BUILD_ONLY_FLAGS`, OPT:30–36). `hale check` takes `--target` but not `--wrap-main`. So `hale check --target wasm32` of a bare-main program judges the unwrapped program under wasm32, which reaches the same stdlib refusals because the effective target, not the injected declaration, gates them. Under equivalent source and configuration (`--target wasm32` on both), check and build agree.

The fragment for the J commit (`unreleased/<pr>.md`):

```text
### Targets
- What a target can do is one table, consulted by `hale check`, `hale build` and the editor alike. `hale check` takes `--target`; `--target wasm32` now gates the browser-unavailable stdlib exactly as a `target wasm { }` declaration does, and an explicit `--target` that contradicts a `target` declaration is refused at the declaration.
- Refused under wasm32 at check time, where they were admitted and never ran: `pinned` placement, cooperative pools other than `main`, `where async_io`, and transport `bindings`; <T3's namespaces>.
- `[ffi] link` on wasm32 is refused by `hale check --target wasm32` and, in a build, before any tool is probed, so a machine without clang reports the capability rather than the toolchain.
- The dispatch plan's `static_direct` flavor now requires a flat payload, as lowering always did; `--dump-model` and the execution digest change for subjects with a managed payload.
```

## 3. The lifecycle cells

### 3.1 Measured

The spines on wasm32 today. The C-numbers are `notes/f40-lifecycle-inventory.md`'s rows; R20, R34 and R35 are its runtime entry points.

| spine | site | R35 ingress quiesce | R20 pool join | R34 wait-abort |
|---|---|---|---|---|
| C13 eager main-locus dissolve | INST:4854–4866 | skipped (4859) | **emitted** (4862) | **emitted** (4865) |
| C19 deferred main-locus entry | CG:7125–7129 | skipped | skipped | skipped |
| C21 `fn main` fall-through | CG:9984 → 9993 → `emit_frame_teardown` | skipped | skipped | **emitted** (CG:6954–6956, `if self.in_main`) |
| C22 test-failure exit | CG:10029 → 10034 | skipped | skipped | **emitted** (CG:6955) |
| C23 `return` from main | CG:22585 → 22625 | skipped | skipped | **emitted** (CG:6955) |

The inventory's line 16 ("C13 emits R20 and R34 on wasm; the others do not") and the registry's sentence both undercount R34. The divergence is C13 against C19 for R20, and C19 against the other four for R34. Both are harmless today for incidental reasons:

- R20 is emitted only when `main_cooperative_pools` is non-empty (CG:6784, `emit_coop_pool_shutdown_all`).
- R34 is one atomic store (RT:19079–19081).

The cost is the `pthread_join`/`pthread_cond_broadcast` imports R20 drags in (RT:10149–10179). `start_all` (CG:9643) already brings in `pthread_create` whenever pools exist.

### 3.2 The rule

An obligation is required on a target exactly when some behaviour the target admits needs it:

| obligation | needed by | wasm32 after T2 |
|---|---|---|
| R20 pool join | a cooperative pool other than `main`, or an `async_io` pool (`Threads`, `AsyncIoPool`) | both `Reject` → `PoolJoin × Wasm32 = Lower(Absent { because: Threads })` |
| R34 wait-abort | an `or wait` publisher parked on a full bounded queue. `or wait` is legal only on a transport-bound topic (GH #255, the bound-topic set at CHECK:596–618), so it needs `RemoteTransport`; and only a pool or pinned consumer drains concurrently (`Threads`) | `RemoteTransport` `Reject` → `WaitAbort × Wasm32 = Lower(Absent { because: RemoteTransport })` |
| R35 ingress quiesce | a LISTEN binding (`RemoteTransport`) | `Absent { because: RemoteTransport }` (identical to today: every spine skips it on wasm) |
| pinned join (C18's `pthread_join`, CG:7182) | `pinned` (`Threads`) | no pinned entry can exist; nothing to gate |

**The order is the decision's condition.** An obligation is removed for a target only after the matrix rejects, or legitimately lowers, every behaviour that needs it. The J commit (P3 2 of 3) lands `Threads`, `AsyncIoPool` and `RemoteTransport` as `Reject` on wasm32 first. P3 3 of 3 then makes all five spines read `cells.lowering(PoolJoin | WaitAbort | IngressQuiesce)` in one commit. The result:

- on wasm32 no spine emits any of the three, so C13 and C21–C23 lose R20/R34 there;
- on the host every spine emits all three exactly where it does today.

The interim correction line 16 permits (gating C13 like the others) is unnecessary if P3 3 of 3 lands in wave 2. If L-line work needs it earlier, it must gate R20 **and** R34 together and leave C21–C23 alone, so the spines do not diverge a third way. The rule is stated in the matrix, not in each spine, so L1's plan reads it: an obligation row exists for `(instance, action)` only if its cell is `Lower(Emit)` for the snapshot's effective target.

### 3.3 Tests, one per spine, on both targets

The program shapes:

- **eager:** a statement-position main locus literal, `fn main() { App { }; }` (C13);
- **deferred:** a let-bound one, `fn main() { let a = App { }; }` (C19);
- **return:** `fn main() -> Int { App { }; return 0; }` (C23).

Each is built twice, once with a pool field and an `or wait` publish on a bound topic, once without. Per spine (P3 3 of 3, `crates/hale-codegen/tests/target_lifecycle_cells.rs`, joining the area that holds `wasm_target.rs`):

- **Host, with pool and bound topic:** the emitted module calls `lotus_coop_pool_shutdown_all`, `lotus_bus_wait_abort_all` and `lotus_bus_ingress_quiesce`, in that order, before the first pinned join or the cascade. The IR-shape assertion follows `corpus_oracle.rs`'s module inspection.
- **wasm32, without them** (the program with them is refused, and the test pins the refusal): the module calls none of the three, and its import list contains no `pthread_*`, `epoll_*` or `eventfd`. This extends `wasm_build_emits_valid_module`'s byte check (`wasm_target.rs:787`) from one program to the three spines.
- **wasm32, the C21/C22 spines too** (fall-through, and a `std::test::assert` failure), so all five agree; they share `emit_frame_teardown`.
- **L1/L2 linkage:** once L1's plan exists, the plan of each wasm32 program has no R20/R34/R35 obligation and the host plan has them. Until then, the IR assertion is the oracle.

## 4. The generated documentation

`docs/src/systems/webassembly.md` keeps its prose. Its **capability statement** becomes a generated region:

```text
<!-- capability-matrix: wasm32 (generated by hale_types::capability::render_markdown; do not edit) -->
…
<!-- /capability-matrix -->
```

The region holds:

- the namespace table (the cells `StdNamespace × Wasm32`, Reject first, each with its substitute; today's hand table at spec/ffi.md:402–411 is the same data);
- the placement, transport and `async_io` refusals;
- the `[ffi] link` refusal;
- the export surface (the fixed exports and the `@export` rules);
- the available-everything-else sentence.

It replaces the hand-written passages at webassembly.md:31–36 (the namespace prose), :42–45 (the bus paragraph's transport sentence) and :243–245 (`link = [...]`). spec/ffi.md's table (:402–411) becomes the same generated region. The spec is canonical, and a second hand-kept copy of the table is exactly the drift the registry invariant ("the docs' target statement is generated, never hand-maintained") forbids.

The test is `crates/hale-types/tests/capability_doc_matches.rs`, on the `registry_matches_spec.rs` precedent (`crates/hale-graph/tests/registry_matches_spec.rs:20–44`):

- It renders each region and compares it byte-for-byte with the text between the markers in both files.
- On a difference it names the first differing line and the regeneration command, `HALE_REGEN_CAPABILITY_DOC=1 cargo test -p hale-types --test capability_doc_matches`. The variable is read, never set, by the test, as the registry precedent does.
- It fails if either file lacks the markers.
- The rendered rows read every reason and substitute from the cells. Nothing is spelled twice.

P3 1 of 3 lands the region rendering **today's** cells. The doc's content changes only by format, from prose to the table the spec already uses. The J commit's new cells then change the doc and the spec in the same commit as the code, as the repo's spec rule requires.

## 5. Consolidation order: three PRs

Each PR is one of the plan's shapes (§6). Each carries its registry re-horizon and closes its rows with the code.

### P3 1 of 3: the matrix as data, shadowed (M then S · Sonnet pane; the driver reviews the cell table)

1. `TargetSpec` moves to `hale-types::target`; codegen re-exports it (M).
2. `hale-types::capability`: `TargetClass`, `Capability`, `Cell`, the table with **today's** verdicts (T2/T3 not yet applied: a capability admitted today is `Lower`), and §1.6's laws. Today's admissions are written as they are, with a `KNOWN_OPEN` list (the `ownership_matrix.rs` precedent) naming each cell that T2/T3 will flip, asserted to be today's answer.
3. The `target_capability` snapshot family and `Target { name, spec }`, with the digest folding the spec (§1.7) and the leak test.
4. The **shadow**: for every program in §2.9 and the corpus, under host and wasm32, the legacy answers equal the cells' answers. The legacy answers are `wasm_unavailable_stdlib` under the source declaration, `has_async_io`, `link_wasm`'s refusal, and every `is_wasm` site's branch. Zero unclassified divergences.
5. The namespace import oracle (§2.1) runs and records its measurements as T3's evidence.
6. The generated doc region and its test (§4), rendering today's cells.

Closes nothing yet; the registry names `capability.rs` as the producer, with the legacy rows still permitted.

### P3 2 of 3: admission on every entry point (J · Opus pane; the driver writes T1–T5 into spec/ffi.md and docs before the pane starts)

1. The effective target (§1.3) and `hale check --target`; `--wrap-main` reads the effective target.
2. The checker consults the family; `wasm_target`, `wasm_unavailable_stdlib`, `Bundle::target_has_async_io`/`target_label` go.
3. The flipped cells (T2, T3, T5) and the moved refusals (`[ffi] link` ahead of the toolchain and into the check, T4; `@export locus` with `run()`; the `@export`-only native build), each with its wording pinned, in one J commit with the fragment of §2.9.
4. The tests:
   - the precedence table (§1.3);
   - the **paired cases** (below);
   - the **agreement test**: for every program of §2.9 and each paired case, `hale check [--target T]`, `hale build [--target T]` and the editor's snapshot agree on admission and on the located refusals, compared as `file:line:col message` sets;
   - the toolchain-masking test (§1.5);
   - the LSP leak test (§1.7);
   - the corpus sweep reading the effective target.
5. Closes `target_capability` · `wasm_unavailable_stdlib`, `wasm_target`, `TargetSpec`; `link_wasm`'s refusal half.

**The paired cases.** Each is one program, allowed natively and rejected under wasm32 (or the reverse), with the pinned witness:

| # | program | host | wasm32 (source declaration, and `--target wasm32`, separately) |
|---|---|---|---|
| 1 | `fn main() { let _ = std::process::pid(); }` (and one per rejected namespace) | lowers | `` `std::process::pid` is unavailable under `target wasm`: OS process control (`std::process`) isn't available in the browser `` at the call |
| 2 | the `async_io` listener of `crates/hale-cli/tests/target_model.rs:330` without its tcp use | lowers (glibc, macOS); musl refuses (today's wording) | refused, `AsyncIoPool` wording naming wasm32, at the placement entry |
| 3 | a `pinned` child | lowers | refused at the placement entry |
| 4 | `cooperative(pool = workers)` | lowers | refused at the placement entry |
| 5 | a `bindings { T: unix(...) }` entry | lowers | refused at the binding |
| 6 | an app with `[ffi] link = ["m"]` | lowers | refused (manifest record on check; before clang on build) |
| 7 | `@export locus L { fn run() { } }` | lowers (an ordinary locus) | refused at `run` |
| 8 (reverse) | `@export fn go() { }` with no `fn main` | refused at the declaration (T1's wording) | lowers |
| 9 (reverse) | `@ffi("js") fn console_log(m: String);` called from main | refused at the declaration (T5) | lowers |

### P3 3 of 3: codegen reads the cells; lifecycle; the gate column (M · Sonnet pane for the site switch; S · Opus for the lifecycle commit and the gate column)

1. Every skip/substitute/export site of §2.5 reads a cell, one commit per capability: `ProcessSignals`, `ReplayIngress`, `RemoteTransport`, `ExportSurface`/`EntryInversion`, `ForeignAbi`. The `is_wasm` field is deleted, and backend sites read `self.target`. The oracle: IR identical over the corpus and `wasm_target.rs` for both targets (the module dump before and after).
2. The lifecycle commit (§3): the obligations from the cells in all five spines, with the spine tests of §3.3.
3. The gate column (§2.8): `payload_flat` on the gate row, `DispatchFlavor::of` with three legs, codegen reading the flavor, and the shadow against the old predicate at every publish site.
4. The **portable subset**: programs whose stdout agrees byte-for-byte between the native binary and `node <loader>.mjs`. These are `wasm_target.rs` :846 (`STRUCTSUM=330`), :792 (run it; sum of squares 1..10, 385), :747 (`wrapped-main-ran`), :87 and :208 with `console_log` replaced by `println`, `wasm_link_is_quiet.rs:66` (`len=5`, `b0=104`, `hello`), `wasm_target_gating.rs:39` (`n=42`), and `play/examples/{collections,decimal,closure,enums,fallible,jobqueue}.hl` through `--wrap-main`. Each is built for both targets and run on both, and the outputs are compared. The test skips, naming what is missing, only when node or wasm-ld is absent. CI must have both: today `tests.yml` installs `clang-18` and not `lld` explicitly, so P3 3 of 3 states the dependency in the workflow.
5. The import backstop (§2.3, T7).
6. Closes `target_capability` · `link_wasm` (export list), `lotus_replay_start_ingress`, `is_wasm`, `ffi_type_unportable` (as the matrix's column); `dispatch` · `bus_payload_is_flat`. The family becomes Canonical.

## 6. What the rows contradict

1. **The registry's `lotus_replay_start_ingress` row** ("instantiation still emits pool shutdown and wait-abort on wasm where the main exit does not") is half right. Every main exit emits wait-abort on wasm too (CG:6954–6956). Only the deferred entry (CG:7125) emits neither. The inventory's line 16 inherits the same undercount (§3.1).
2. **`TargetSpec::has_async_io` is true for wasm32** (TGT:322), and the runtime agrees (`LOTUS_HAVE_ASYNC_IO` 1, RT:142–147). But the backend's epoll/eventfd calls are `() => 0` imports (CG:2983–2986), so an `async_io` pool on wasm is admitted and never runs.
3. **SHIM:47–53 says pool workers and `cond_wait` are "gated out with `#ifndef __wasm__`"**. They are not: `lotus_coop_pool_start_all` (RT:10075) and `lotus_coop_pool_shutdown_all` (RT:10149) lie outside the gated region (RT:9827–10034), and pinned `pthread_create` (INST:4262–4272) is ungated in codegen.
4. **spec/ffi.md:378 says the declaration "opts into the wasm backend"**. It selects no backend: only `--target` does (OPT:312), and codegen ignores the declaration (CG:13284). docs/src/systems/webassembly.md:19–21 is accurate ("the program declares the target so the typechecker can gate"), so the spec and the book disagree.
5. **spec/ffi.md:423–425 and webassembly.md:42–45 say the network transports are "unavailable in the sandbox"**, but nothing refuses a `bindings` entry. `emit_bindings_prelude` (CG:9813) is ungated, and the transports' syscalls become no-op imports.
6. **CHECK:8083–8087 lists `time`, `env` and `rand` as the portable surface**, while the shim makes the clock read 0, sleep return at once (SHIM:483–488) and `getenv` return NULL (SHIM:36).
7. **The `is_wasm` row says "a dozen sites"**. There are 33 references; 30 sites read it. 13 of them are emission configuration, which the Final direction says is not a decision, so they are not capabilities. Two are dead: CG:1182 is unreachable through the CLI, and CG:2080's wasm arm comes after the wasm return.
8. **`ffi_type_unportable` is listed as a target capability, but it does not vary by target.** The target-dependent part of the FFI boundary, `js` marshalling (CG:14351 and four more sites), is not in any row.
9. **`dispatch`'s invariant** ("which flavour a subject gets is a plan conclusion") fails for non-flat direct-eligible subjects. The plan says `static_direct`, and lowering emits a static enqueue (`bus/dispatch.rs:1388`). The execution digest records the plan's answer, not the lowered one.
10. **`crates/hale-cli/tests/iris_seeds_check.rs:31` excludes `wasm-flower` as "covered by the wasm example tests"**. No test builds, checks or runs it.
11. **`--wrap-main`'s guard (`build.rs:56–58`) is a third wasm test**, string-matched, that disagrees with `TargetSpec::parse` on `wasm32-unknown-unknown`.
12. **No test compares `hale check` and `hale build` on wasm admission.** The agreement sweep is native-only (`corpus_check_build_agreement.rs`), and the `wasm_target.rs` programs are never checked: the harness does not gate (SNAP:197–203).

## 7. Decisions for the driver (and Riley where a diagnostic changes)

- **T1. Precedence.** An explicit `--target` wins. A declaration that contradicts it is a located refusal at the declaration. A declaration with no `--target` sets admission's target on every entry point. For `hale build` with no `--target` on a declared-wasm program: **(a) a located refusal naming `--target wasm32`** (recommended: artifact selection stays on the command line, as #911 kept the build's output unchanged), or (b) the build selects wasm32 (spec/ffi.md:378 becomes true).
- **T2. Threads, `async_io` and transport bindings on wasm32: `Reject`** (recommended), at the placement entry or binding. A collapse onto the host event loop is a legitimate layer-5 lowering for later, with its own review. Rejecting is also what lets §3's obligations go. No program in the tree is affected.
- **T3. The stub namespaces** (`time`, `env`, `ts`; `io::unix`, `sockopt`, mirror/ring pending the import oracle): **`Reject` where the oracle finds a stub** (recommended), with guidance naming an `@ffi("js")` host import. Real browser implementations (`performance.now`, timers) are a new lowering, not P3. No program in the tree is affected.
- **T4. `[ffi] link` on `hale check`.** A record against the manifest: file `hale.toml`, the `link` key's line, the cell's wording. This follows the `"kind":"io error"` precedent of positionless records (spec/projects.md:746). On `hale build` it is reported before any tool is probed.
- **T5. `@ffi("js")` on a native target: `Reject`, located at the declaration** (recommended), instead of an unlocated native link failure.
- **T6. spec/ffi.md's namespace table becomes a generated region** beside the book's (recommended), so the spec stays canonical without a second hand copy.
- **T7. The import backstop is a test, not a build error, in P3.** Making an import outside the allowed set a build-time refusal would change `--allow-undefined`'s contract for user `@ffi("c")` symbols that a package supplies at instantiation, and that deserves its own line.
