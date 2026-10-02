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
CapabilityMatrix = { behaviours:  (TargetClass, Capability) → BehaviourCell,
                     invocations: (TargetClass, Invocation) → InvocationCell,
                     obligations: (TargetClass, Obligation) → ObligationCell }

BehaviourCell  = { verdict: Lower(Lowering) | Reject(Refusal),
                   origin:  Source            -- a program construct requests it: the use
                                              -- producer (§1.4) finds it, and Reject is a
                                              -- located diagnostic
                          | Environment,      -- nothing in a program requests it (a signal
                                              -- arrives from outside): Reject produces no
                                              -- diagnostic and exists only as a premise
                   requires: [Capability],    -- e.g. AsyncIoPool requires Threads
                   witness: Witness }
InvocationCell = { verdict: Allowed | Refused(Refusal), witness: Witness }
                                              -- how the artifact may be invoked (run, replay,
                                              -- record); the CLI reads it (OPT:386 becomes a read)
ObligationCell = { verdict: Emit | Omit { premise: Premise }, witness: Witness }
Premise  = Rejects(Capability)                -- that target's behaviour cell is Reject
         | Refuses(Invocation)                -- that target's invocation cell is Refused
         | Proven(ProofId)                    -- a stated proof, registered with its tests (§3.2)
         | All([Premise])
Witness  = { site:   the producer, a stable path::symbol (the registry's convention),
             reason: the one sentence a diagnostic or doc renders,
             spec:   the spec anchor whose sentence the cell implements }
Refusal  = { wording: the diagnostic template (today's text verbatim where one exists),
             guidance: the substitute, if any }
Lowering = the capability's lowering data, if any (ExportSurface's export list,
           ForeignAbi(js)'s marshalling), else none
```

**Behaviours and obligations are distinct types.** A behaviour cell answers "may a program do this on this target, and how is it lowered". An obligation cell answers "does a spine or prelude emit this runtime call on this target". An obligation never is a behaviour's verdict; it **refers to** one as its premise. A behaviour is never `Omit`, and an obligation is never `Reject`. Before this split one cell had to be both: `ProcessSignals` on wasm32 was written as `Lower(Absent { because: ProcessSignals })` and as `Reject` at once. Now `ProcessSignals × Wasm32` is a behaviour cell, `Reject` with origin `Environment` (no signal source exists), and the six emission sites are obligation cells `Omit { Rejects(ProcessSignals) }`. Replay is the other case the old schema could not say honestly: it was "absent because the CLI refuses it", a refusal outside the matrix posing as a matrix rejection. Now the CLI refusal is an invocation cell, and the replay obligations are `Omit { Refuses(Replay) }`.

**Targets** (`TargetClass`). It is derived from `TargetSpec`'s `(arch, os, env)`, not from the triple's name: `PosixAsync` (glibc Linux, macOS), `PosixNoAsync` (musl), `Wasm32`. Windows stays `Planned` and is refused at argument parsing (OPT:292–300). It never reaches the matrix, because a tier is not a capability. Today the matrix has two populated columns, host and wasm32. musl joins only for the `async_io` cell it already has. A new target is a new column. **No cell has a default arm**: a capability added without a cell for every class fails the matrix's own law test (§1.6).

**Capabilities, invocations and obligations.** These are the facts the seven rows decide today, the three lifecycle obligations line 16 names, and the emission sites the old rows decided implicitly:

| capability, invocation or obligation | key | layer | the legacy row it comes from |
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
| `BoundedWait` | `or wait` on a topic with `on_full: fail` capacity (GH #255 phase 2), bound or not | 5 | (none: admitted on every target, CHECK:13103) |
| behaviour `ProcessSignals` (origin `Environment`) | a process signal reaching the program: SIGINT's drain, SIGPIPE | 6 | (none: decided implicitly by the sites below) |
| invocations `Run`, `Replay`, `Record` | `hale run`, `hale replay`, a recording run | 6 | OPT:386–391 |
| obligations `ReplayIngress`, `ObservationIdentity` | replay ingress (INST:4584); observation identity and its eager init (CG:9673, CG:9824) | 6 | `lotus_replay_start_ingress` |
| obligations `SignalInstall`, `DrainObserver`, `DrainTerm` | SIGPIPE and `lotus_io_init` (CG:9444), the drain installer (CG:9849's signal half), the drain observer (CG:33668); the process-flag term in `dissolve.rs:227`, `restart.rs:169`, `stdlib/time.rs:495` | 6 | `is_wasm` |
| obligation `BindingConfig` | `lotus_bus_load_config` (CG:9849's transport half) | 5 | `is_wasm` |
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

**The precedence rule (T1, decided (b)).**

- An explicit `--target` is the effective target.
- With no `--target`, a source declaration makes the effective target wasm32, **for analysis and for artifact emission alike**, on every entry point. `hale build` of a program that declares `target wasm` emits the wasm module and its loader, as `--target wasm32` does. spec/ffi.md:378 ("opts into the wasm backend") becomes true as written.
- With neither, the target is the host.
- An explicit `--target` whose class disagrees with a source declaration is a **located refusal at the declaration**: ``this program declares `target wasm`, and is being checked/built for `<triple>`: build it with `--target wasm32`, or drop the declaration``. This is #911's ruling applied to the other half: "a stated directive that does nothing is worse than a refusal". It is the same refusal on `hale check --target <triple>` and `hale build --target <triple>`.

Today `hale build` with no `--target` on a declared program builds a native binary, and an `@export`-only one is refused late as "program has no `fn main()`" (CG:8787). Under (b) that program builds for wasm32. An `@export`-only program **without** a declaration and without `--target` is a host program; it is refused by `EntryInversion × Host` at its first `@export`: ``a program with no `fn main` is an export-only module, which needs wasm32: declare `target wasm { }` or build with `--target wasm32` ``.

`--wrap-main`'s guard (`build.rs:56–58`) reads the effective target instead of matching strings, so `--target wasm32-unknown-unknown --wrap-main` is admitted. The guard reads the effective target of the **written** sources: the declaration `--wrap-main` injects (`desugar.rs:180–193`) is a consequence of the target and never selects it, so `hale build --wrap-main` with no `--target` and no declaration stays refused.

**The agreement contract is unchanged.** The editor has no `--target`. Its effective target is the source's, else the host, which is now exactly what `hale check` and `hale build` compute without `--target`. "Equivalent configuration" for the agreement test therefore means `hale check` and `hale build` without `--target`, compared with the editor; with `--target`, it means `hale check --target T` compared with `hale build --target T`. Under (b) the three agree on the effective target as well as on admission, so the build of a declared program is judged by the same cells the editor showed.

**Precedence tests** (P3 2 of 3, `crates/hale-cli/tests/target_precedence.rs`, a new `[[test]]` entry or a module of an existing area). The rows are source × configuration, and each cell names the effective target and the expected diagnostics, pinned by wording:

| source \ CLI | none | `native`/host triple | `wasm32` | `wasm32-unknown-unknown` | musl triple |
|---|---|---|---|---|---|
| no declaration | host | host | wasm32 | wasm32 | musl |
| `target wasm` | wasm32, the build emitting the wasm artifact | refusal at the declaration | wasm32 | wasm32 | refusal at the declaration |
| `target browser_js` | as `target wasm` | as `target wasm` | wasm32 | wasm32 | as `target wasm` |
| `fn main` + `--wrap-main` | refused (`--wrap-main` needs wasm32) | refused | wasm32 | wasm32 (**today refused**) | refused |

Each cell runs `hale check`, `hale build`, and the editor's snapshot (`Config::editor` over the same files) where the CLI column is "none", and asserts that the three agree on admission.

### 1.4 Consulted on every entry point, before lowering

The matrix is demanded as a snapshot family, `target_capability`, keyed by the snapshot, so the configured target is in its key. It produces:

- the effective target (§1.3);
- one **use row** per operational use in the program (the producer's contract is below), plus the declaration-level uses: a placement entry naming `pinned`/a pool/`where async_io`, a `bindings` entry, an `@export` declaration, an `@ffi("js")` declaration, a `run()` on an `@export locus`, and the manifest's `[ffi] link`;
- the refusals: every use whose cell is `Reject` for the effective target, as a located `Diag` carrying the cell's witness and the use's witness chain.

**The operational use producer.** Today's gate (CHECK:14811–14825) reads one shape: an `Expr::Call` whose callee is an `Expr::Path` spelled `std::…`. It misses every other way a program reaches a capability. A `target wasm` program that constructs `std::io::tcp::Listener { … }` in a params initializer with no off-main placement (the shape of `crates/hale-cli/tests/target_model.rs:335`, minus its placement) passes it, because the literal is an `Expr::Struct`. So do a method call on a `std::io::tcp::Stream` handle, a call through an imported seed's alias, and a call to a wrapper whose body holds the written use. The producer's contract:

- **Resolved identities, never spelling.** It runs over the resolved graph, under the graph's horizon policy: each expression's resolved declaration, not its path text. No `std::` prefix is matched anywhere; CHECK:14811's segment match goes.
- **What is a use.** A construction (`Expr::Struct` whose resolved type is a stdlib locus or struct carrying a requirement); a receiver call resolved to a stdlib method, whatever the receiver's spelling; a free call resolved to a stdlib function, directly or through an import alias; a call to a function outside the program's own sources (a library's or the stdlib's) whose requirement summary is non-empty; and the implicit obligations a use brings with it: the constructed locus's lifecycle (`birth`, `run()`, `dissolve`; a `Listener`'s accept loop is its `run()`), and the runtime calls its placement or binding implies.
- **Where a requirement comes from.** From the stdlib body, walked through resolved calls, until it reaches a primitive. At a primitive (a codegen-native `lower_std_*`, a `lotus_*` runtime entry) it comes from the audited primitive lowering contract (§2.1's rows), which names the capability. Each function gets a requirement summary, computed once per snapshot.
- **The witness travels.** Each use row carries its chain from the source site to the requirement (use → resolved callee → … → primitive → capability). The refusal is located at the first site in the program's own sources and names the chain's last link. The program's own sources are the horizon: a wrapper inside the program is refused at the written use in its body, once, and its callers carry no duplicate; a callee beyond the horizon (a library, the stdlib) is refused at the call that crosses it.
- **Type-only mentions stay admitted.** A parameter, field, return or annotation type, or a generic argument, is not a use: a portable signature may name `std::io::tcp::Stream` (`target_model.rs:331`).
- **Holes have a policy.** When a use's requirements cannot be resolved (an unresolved callee, a receiver typed `Ty::Unknown`, a summary cycle the walk cannot close), the use is a located refusal on any target whose column holds a `Reject` in that capability family ("cannot establish what `<callee>` requires on wasm32"), and a recorded hole, counted in the shadow and never silent, on a target whose column admits the whole family. An unresolved requirement is never an admission on a restricted target.

`Snapshot::demand_check` appends the refusals beside `build_rule_diags` (SNAP:1088–1091). The check, the build and the editor therefore show the same refusals. The build is gated by them through `demand_lowering` (SNAP:1104–1113), and the editor publishes them as it publishes every check diagnostic. The entry points, all through `Snapshot::load`:

- `hale check`: `run_impl.rs:103`
- `hale build`: OPT:492
- `hale run`, `test`, `bench`: `verbs/test.rs:82`, `verbs/bench.rs:243`, and `run` through `parse_build_options`
- the LSP: `crates/hale-lsp/src/lib.rs:810`
- the test harness: `Config::harness`, SNAP:202

The harness lowers without the check gate (SNAP:197–203). It still reads the cells for lowering, so a harness wasm build emits what a CLI wasm build emits.

Codegen **reads** the matrix through the lowering view and decides nothing. Each SKIP/SUBSTITUTE site of §2.5 becomes `cells.obligation(o)`, and each EXPORT site reads `ExportSurface`'s `Lower` data through `cells.behaviour(c)`.

### 1.5 A capability refusal is not a toolchain failure

The rule: a refusal that depends only on (program, configuration, target) is a cell. It is a located checker diagnostic, on every entry point. A failure that depends on the machine running the compiler stays a build-time `CodegenError::Link`/`LlvmEmit`, unlocated, naming the missing piece.

| failure | class today | class after |
|---|---|---|
| `[ffi] link` on wasm32 (CG:2816) | build-time, and **masked by a missing clang**, because it is checked after the runtime compile (CG:2786–2799) | **cell** (`LinkLibrary × Wasm32 = Reject`). On `hale check` it is a manifest record (T4). On `hale build` it is reported before any tool is probed |
| `@export locus` with `run()` (CG:14130) | build-time (codegen) | cell (`EntryInversion`), same wording |
| `@export`-only program built natively (CG:8787) | build-time, "program has no `fn main()`" | with a declaration: no failure, it builds for wasm32 (T1(b)). Without one: cell (`EntryInversion × Host`), located at the first `@export`, with §1.3's wording |
| clang missing (CG:2793), wasm-ld missing (CG:2876) | build-time | unchanged |
| a `csrc` unit that will not compile freestanding (CG:2835–2842) | build-time | unchanged: it depends on C source the checker does not read |
| zig / target sysroot missing (spec/projects.md:752) | build-time | unchanged |
| `libhale_ts_shim.a` not built (CG:2157) | build-time, native only | unchanged for native. On wasm32 `std::ts` becomes a cell (T3), because today the wasm path returns at CG:1954 and never reaches CG:2157 |
| Windows triple (OPT:292) | argument parsing, "not buildable yet" | unchanged: a tier, not a capability |
| `hale run --target wasm32` (OPT:386) | argument parsing | the invocation cell `Run × Wasm32 = Refused`, still read at argument parsing, wording unchanged: the host cannot execute the artifact. It is a premise for the replay obligations (§2.4), so it lives in the matrix rather than in OPT alone |

Test: on a PATH with no clang (a `Command::env("PATH", …)` on the child, never `set_var`), `hale build --target wasm32` of the `link = ["m"]` app reports the `[ffi] link` refusal, not "is clang installed?".

### 1.6 The matrix's own laws (a unit test in `hale-types`)

- Every `(TargetClass, Capability)`, `(TargetClass, Invocation)` and `(TargetClass, Obligation)` pair has exactly one cell of its own type.
- Every `Reject` has non-empty wording and a spec anchor that exists in `spec/`.
- An approximating witness appears only on a layer-5 or layer-7 capability.
- Every `Omit` premise is validated on its own target: each `Rejects(c)` names a behaviour cell that is `Reject` there, each `Refuses(i)` an invocation cell that is `Refused` there, each `Proven(p)` a registered proof whose tests exist. An obligation whose premise does not validate fails the law.
- The dependency relation is typed and acyclic: obligation → premise (behaviour, invocation, proof), behaviour → the behaviours it `requires`. A `requires` edge is consistent: a behaviour that is `Lower` requires only behaviours that are `Lower` on the same target. A cycle fails the law.
- A behaviour of origin `Environment` has no use producer and renders no diagnostic. A behaviour of origin `Source` that is `Reject` on some target has a use producer.
- Every `StdNamespace` cell's key is a namespace the stdlib defines, and every stdlib namespace has a cell. The second half is checked against `hale-stdlib`'s file list and codegen's `stdlib/` modules.
- Every `Lower` cell for a stdlib namespace or operation on a target without the host's libc cites lowering-contract rows that cover every operation it admits, and every row names its probe (§2.1).

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

**Cell.** `StdNamespace(p) × Wasm32` is `Reject` for the 8 prefixes. The wording is unchanged: `` `std::<path>` is unavailable under `target wasm`: <reason> `` (CHECK:14816–14823). The phrase names what selected the target: `` under `target wasm` `` when the declaration did, `` under `--target wasm32` `` when the configuration did, so every existing pinned wording stays byte-identical. The reasons and substitutes are the table's strings. `StdNamespace(p) × Host` is `Lower` for every namespace.

**The namespaces with no stated cell today (T3).** They are admitted on wasm32 and lowered to stubs of two kinds. **Inline stubs** are defined in the shim header and compile into the module with no import at all: `getenv` returns NULL (SHIM:36), `clock_gettime` writes zero and `nanosleep` returns at once (SHIM:483–488). **Import stubs** are syscalls left undefined, which `--allow-undefined` (CG:2872) and the loader's `() => 0` for every unknown import (CG:2983–2986) turn into silent zeros.

- `std::time`: mixed. The clock reads (`now`, `current`, `monotonic`, `monotonic_ns`; `stdlib/time.rs:115–306`) go through the inline `clock_gettime` and return zero; `sleep` calls `clock_nanosleep` (`stdlib/time.rs:614–618`), an import stubbed to 0. The conversions (`iso8601`, `unix`, `nanos`, `from_nanos`, `from_unix`, `parse_iso8601`, `parse_time`) compute on their argument.
- `std::env`: every operation reads the inline `getenv`, which returns NULL.
- `std::ts`: the wasm link returns at CG:1954, before the native shim check at CG:2157.
- `std::io::unix`, `std::sockopt`, mirror/ring (`stdlib/io_unix.rs`, `sockopt.rs`, `mirror.rs`, `ring.rs`): syscall-backed and not in the table. Unclassified.

**Support is classified by reviewed lowering contracts and operation coverage, never by the import list.** An import list cannot see an inline stub: a namespace of inline no-ops imports nothing and passes any import oracle. So:

- **A contract per operation.** A `Lower` verdict for a stdlib operation on wasm32 cites a contract row saying what the wasm lowering does: computes in the module, calls a named loader import whose semantics the loader states, or calls a declared `@ffi("js")` import. The rows sit beside the cells and are reviewed with them.
- **Coverage.** Every public operation the namespace defines (the stdlib's declarations plus codegen's `lower_std_*` dispatch for the codegen-native modules) has a row, or the namespace is not `Lower`. An operation with no row cannot be admitted by its namespace's default.
- **A probe per row.** A one-call program per operation runs under node, and its result is checked against the row: equal to the native result for a deterministic operation, or the row's stated property (advances, non-zero, waits at least the requested duration) for a clock or a sleep.
- **Mixed namespaces.** A namespace whose operations split gets operation-level cells, `StdOperation(path::fn)`, beside its `StdNamespace` cell, or is refused whole. The cell table says which, and the generated documentation renders the split. Recommended: `std::time` per operation, with the clock reads and `sleep` `Reject` and each conversion `Lower` once its row and probe pass (one that fails joins the `Reject` set); `std::env` and `std::ts` refused whole; `std::io::unix`, `std::sockopt` and mirror/ring refused whole unless contracts are written for them in P3 1 of 3.
- **The known stubs need no measurement.** The time and environment stubs above are rejected directly under T3; P3 1 of 3 writes their cells as `Reject`-to-be in `KNOWN_OPEN`, not as findings of a measurement.
- **The verification is itself tested.** A test build of the runtime with one contracted operation's helper replaced by an injected `static inline` no-op (a test-only define, never set by a build) must fail the support verification, while the module's import list contains nothing forbidden. That is the evidence that the verification observes behaviour, not linkage.

The import list keeps one job, the T7 backstop (§2.3): catching an **unresolved** import that reaches the link. It says nothing about semantics.

**Moves.** The gate reads the effective target instead of `wasm_target`. The seam (`spec/registry.md`: `wasm_unavailable_stdlib(` ×2 in `check.rs`) moves to `capability.rs`. The table becomes the cells, and the function is deleted. The gate stops being a call-form match: uses come from the operational use producer (§1.4), so construction, receiver calls, import aliases and wrappers carry the capability too. A path in type position (`std::io::tcp::Stream` as a parameter type, `crates/hale-cli/tests/target_model.rs:331`) is still not a use. That is a deliberate keep, so a portable signature can name a type.

### 2.2 `wasm_target` (CHECK:589)

**Today.** Does any program declare `target wasm`/`browser_js` at the top level? It gates §2.1 and nothing else. Codegen and the CLI do not read it.

**Cell.** None. It becomes the source term of the effective target (§1.3). `Checker::wasm_target` (CHECK:8149), `target_has_async_io` and `target_label` (CHECK:8152–8153) collapse into one `&CapabilityView` on the checker.

**Moves.** The two other readers of the declaration keep reading syntax: `desugar.rs:181` (wrap-main's "already declared" test) and the parser's top-level rule. `Bundle::target_has_async_io`/`target_label` (symbol.rs:60–63) are deleted. The checker no longer reads the target from the bundle; it reads the matrix's use rows.

### 2.3 `link_wasm` (CG:2739)

**Today.** It holds two refusals: `[ffi] link` (CG:2816) and a `csrc` compile failure (CG:2835). It also holds the export list: `main` if defined, `__heap_base`, `memory`, `lotus_wasm_alloc`, `lotus_wasm_set_inbox`, plus `cx.wasm_exports` (the `@export` wrappers and `_hale_start`), at CG:2855–2869. And it holds the import policy: `--gc-sections --allow-undefined` (CG:2870–2873).

**Cells.**
- `LinkLibrary × Wasm32 = Reject`, with today's wording (CG:2818–2822).
- `LinkLibrary × Host = Lower`.
- `ExportSurface × Wasm32 = Lower`. Its lowering data is the fixed export list plus the `@export` set. `link_wasm` reads that list instead of spelling it.
- `ExportSurface × Host = Lower`: `@export fn` is an unmangled C symbol, and `@export locus` is an ordinary locus (spec/ffi.md:473–478).
- The `csrc` failure stays toolchain (§1.5).

**Moves.** The refusal moves ahead of the clang probe and into the check (T4). The import policy stays a link flag, but P3 3 of 3 adds a **backstop test**: every module the wasm tests build imports only the loader's writer set plus its declared `@ffi("js")` names. It catches an unresolved import, a syscall that reached the link undefined and would become a `() => 0`. It is **not** evidence that the matrix has no silent stub left: an inline stub needs no import, so that evidence is the contracts and probes of §2.1 (T7).

### 2.4 `lotus_replay_start_ingress` (INST:4584) and the lifecycle residue

**Today.** On the main locus, replay ingress is skipped on wasm (INST:4584). The registry also says: "instantiation still emits pool shutdown and wait-abort on wasm where the main exit does not". Measured, that is half right (§3.1). Pool shutdown on wasm is emitted only by the eager spine (INST:4862). Wait-abort is emitted on wasm by the eager spine (INST:4865) **and** by the fall-through, test-failure and return spines, through `emit_frame_teardown`'s ungated `if self.in_main` (CG:6954–6956). Only the deferred spine (CG:7125) omits all three.

**Cells.**
- Invocation cells: `Replay × Wasm32 = Refused` and `Record × Wasm32 = Refused`, with OPT:386's wording; `Run × Wasm32 = Refused` with the same (the host cannot execute the artifact). The loader passes no mode either (CG:2983), so the cell describes every invocation the artifact has, not only the CLI's. On the host all three are `Allowed`.
- Obligation cells: `ReplayIngress × Wasm32 = Omit { Refuses(Replay) }` and `ObservationIdentity × Wasm32 = Omit { All([Refuses(Replay), Refuses(Record)]) }` (CG:9673, CG:9824). On the host both are `Emit`.
- The premise is the invocation policy, stated in the matrix and validated by §1.6. It is not "`getenv` returns NULL" (SHIM:36): that is a stub, and stubs justify nothing (§2.1).
- The lifecycle obligations are §3.

**Moves.** The skip becomes `cells.obligation(ReplayIngress)`, with identical IR. OPT:386 reads the invocation cells instead of spelling its own refusal.

### 2.5 `is_wasm` (CG:3136; 33 references, 30 sites read it, plus two that branch on the target without the name: INST:888, CG:9590)

**Today.** Every site, classified:

| kind | sites | becomes |
|---|---|---|
| backend configuration (13) | CG:1164, 1173, 1182 (unreachable via the CLI), 1310, 1327, 1347, 1384, 1510 (DWARF silently ignored on wasm), 1723, 1790, 1861, 1883, 2080 (dead for wasm: it returns at CG:1954) | `TargetSpec` queries; not cells. CG:1182 and CG:2080's wasm arm are deleted as dead |
| link (1) | CG:1954 | `TargetSpec::is_wasm()` chooses `link_wasm`; the refusals inside are §2.3 |
| exports / entry inversion (3) | CG:8779, CG:9377 (refusals inside at 14130, 14187), INST:888 | `ExportSurface`, `EntryInversion` |
| skip-emit (12) | CG:7125, 9444, 9673, 9824, 9849, 9984, 10029, 22585, 33668; INST:4584, 4859; `locus/restart.rs:169` | `cells.obligation(o)`, one obligation each, whose premise names the behaviour or invocation (table below) |
| substitute (2) | `locus/dissolve.rs:227`, `stdlib/time.rs:495` | `ProcessSignals` |
| target-keyed, not wasm-named (1) | CG:9590 `self.target.has_async_io()`, which **emits** `lotus_coop_pool_enable_async_io` on wasm | `AsyncIoPool` |

The skip/substitute sites by capability:

| capability | sites | what wasm gets today |
|---|---|---|
| `ProcessSignals` | CG:9444 (`lotus_io_init`, SIGPIPE), CG:9849 (`lotus_drain_signals_install`, key extractors, `lotus_bus_load_config`), CG:33668 (drain observer), `dissolve.rs:227` (`self.draining` without the process flag), `restart.rs:169` (no `process_draining` term), `time.rs:495` (a sleep never cut short by drain) | Lowered legitimately: wasm32 has no signal source, the flag exists and is always 0 (RT:1865), and the runtime stubs the installer (RT:10025–10033). Two types of cell: the behaviour `ProcessSignals × Wasm32 = Reject`, origin `Environment` (SIGINT is not a program construct, so nothing is diagnosed), and the obligations `SignalInstall`, `DrainObserver`, `DrainTerm` `× Wasm32 = Omit { Rejects(ProcessSignals) }`. The IR is identical |
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
| `crates/hale-types/tests/wasm_target_gating.rs:16` `target_wasm_rejects_posix_stdlib` (5 programs, fs/tcp/tls/term/process) | refused by CHECK:14811 under the source declaration | the same refusals, through the cell | unchanged: the declaration selected the target, so the wording still says `` under `target wasm` `` |
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
| `iris/examples/wasm-flower/flower.hl` (declares `target wasm`, `@export fn` only) | `hale check` accepts; `hale build` without `--target` fails late in codegen ("program has no `fn main()`", CG:8787, inferred); untested (`crates/hale-cli/tests/iris_seeds_check.rs:31` claims a coverage that does not exist) | `hale check`, the editor and `hale build`, all without `--target`: accepted, effective wasm32, and the build emits `flower.wasm` and its loader (T1(b)). `hale build --target wasm32`: the same artifact. `hale build --target <native triple>`: the located refusal at the declaration | **changed late failure → builds**; flower joins the wasm build tests and the agreement test, with and without `--target` |
| corpus sweep (`crates/hale-codegen/tests/corpus_check_build_agreement.rs:193`): harvested declared-wasm programs `wasm_target.rs` #13 (522), #15 (634), #16 (686); `wasm_target_gating.rs` #0 (39); `crates/hale-syntax/tests/wrap_main.rs` #1 (90) | check-clean (gate fires, nothing gated), then built **natively** | built for their effective target, wasm32 (the sweep reads the snapshot's effective target instead of `build_opts::options()`'s `Native`) | no diagnostic change; the oracle now compares like with like |
| `play/examples/*.hl`, `play/ui.hl`, `play/sim.hl` (`play/build.sh:51,63`; deploy-only, `.github/workflows/docs.yml:121–123`) | build with `--target wasm32 --wrap-main` | the same | unchanged (verified: none places, binds, sleeps or reads env) |
| docs `hale` blocks (`docs/src/systems/webassembly.md:23, 57, 187`), spec/ffi.md:384, 435 | parse only (`docs_snippets.rs`) | the same | unchanged |
| a program using `pinned`, `cooperative(pool = X≠main)`, `where async_io` or a `bindings { }` entry under wasm32 | admitted; never runs | `Reject` (T2) | **new**; no program in the tree is affected |
| a program calling a `std::time` clock read or `sleep`, `std::env`, `std::ts`, or an operation without a passing lowering contract, under wasm32 | admitted; stubbed | `Reject` per T3 | **new**; no program in the tree is affected |
| a program reaching a refused namespace by construction, a handle's method, an import alias or a library wrapper, under wasm32 | admitted (the gate reads `std::` call paths only) | refused at the use, with the witness chain (§1.4) | **new**; no program in the tree is affected |
| an `@ffi("js")` declaration in a native build | native link failure (inferred) | located refusal (T5) | **new located diagnostic**; no native program in the tree declares one |
| `@export locus` with `run()` under wasm32 | codegen refusal (CG:14130) | checker refusal, same wording | **moves to `hale check`**; no program in the tree |

The `--wrap-main` note: `--wrap-main` is a build flag (`BUILD_ONLY_FLAGS`, OPT:30–36). `hale check` takes `--target` but not `--wrap-main`. So `hale check --target wasm32` of a bare-main program judges the unwrapped program under wasm32, which reaches the same stdlib refusals because the effective target, not the injected declaration, gates them. Under equivalent source and configuration (`--target wasm32` on both), check and build agree.

The fragment for the J commit (`unreleased/<pr>.md`):

```text
### Targets
- What a target can do is one table, consulted by `hale check`, `hale build` and the editor alike. `hale check` takes `--target`; `--target wasm32` now gates the browser-unavailable stdlib exactly as a `target wasm { }` declaration does, and an explicit `--target` that contradicts a `target` declaration is refused at the declaration.
- A `target wasm { }` or `target browser_js { }` declaration now selects the wasm32 backend: `hale build` without `--target` emits the module and its loader, where it used to build natively and fail on an export-only program.
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
- R34 is one atomic store (RT:19079–19081). Emitting it where no waiter exists costs nothing; omitting it where one can exist is the hazard (§3.2).

The cost is the `pthread_join`/`pthread_cond_broadcast` imports R20 drags in (RT:10149–10179). `start_all` (CG:9643) already brings in `pthread_create` whenever pools exist.

**The host order is wrong today, on every spine.** C13 emits R35, R20, R34 (INST:4859–4865); C19 the same (CG:7126–7128); C21–C23 emit R35 and R20 (CG:9990–9991, :10030–10031, :22589–22590) and reach R34 only afterwards, inside `emit_frame_teardown` (CG:6954–6956). R20 is `lotus_coop_pool_shutdown_all`, which sets each pool's shutdown flag and then `pthread_join`s every worker (RT:10173–10178). Neither wait loop reads a pool's shutdown flag; both return only when their condition clears or R34's flag is set. A pool worker parked in either wait therefore holds R20's join forever. This is the lifecycle inventory's decision 7, adopted as option (a) in PR #1300 ("every teardown spine aborts the `or wait`s it would otherwise wait on before it joins the workers they block"; KNOWN_OPEN, fixture `l07_pool_or_wait_teardown.hl`).

### 3.2 The rule

An obligation is required on a target exactly when some behaviour the target admits needs it:

| obligation | needed by | wasm32 after T2 |
|---|---|---|
| R20 pool join | a cooperative pool other than `main`, or an `async_io` pool (`Threads`, `AsyncIoPool`) | both `Reject` → `PoolJoin × Wasm32 = Omit { All([Rejects(Threads), Rejects(AsyncIoPool)]) }` |
| R34 wait-abort | an `or wait` publisher parked in either admitted wait form (below): the binding-loss wait (`RemoteTransport`) or the local capacity wait (`BoundedWait`) | the binding-loss wait is refused (`RemoteTransport` `Reject`), the local capacity wait is admitted (`BoundedWait` `Lower`), so the omission needs the single-thread proof below. Until that proof lands, **`WaitAbort × Wasm32 = Emit`**; after it, `Omit { All([Rejects(RemoteTransport), Proven(single_thread_no_live_waiter)]) }` |
| R35 ingress quiesce | a LISTEN binding (`RemoteTransport`) | `IngressQuiesce × Wasm32 = Omit { Rejects(RemoteTransport) }` (identical to today: every spine skips it on wasm) |
| pinned join (C18's `pthread_join`, CG:7182) | `pinned` (`Threads`) | no pinned entry can exist; nothing to gate |

**The admitted wait forms.** `or wait` is legal on a topic that is transport-bound **or** has `on_full: fail` capacity: `*full_fail || self.bound_topics.contains(name)` (CHECK:13103, with the diagnostic at :13106–13111 naming both). Codegen emits both waits at every `or wait` publish, the binding wait first (`bus/dispatch.rs:450`, :515). There are therefore two wait forms, and R34 ends both:

| wait form | runtime loop | admitted by | ended by | behaviour |
|---|---|---|---|---|
| binding-loss wait | `lotus_bus_binding_wait_ready` (RT:19083): parks while a served CONNECT binding of the subject is `lost` | the bound-topic disjunct (the set built at CHECK:604–612) | the loss handler's `restart`, which is dispatched only on the queue owner's thread (RT:19059–19062), or R34 | `RemoteTransport` |
| local capacity wait | `lotus_bus_subject_wait_space` (RT:19124): parks while a registration for the subject sits at its refuse bound | the `full_fail` disjunct, **with no binding at all** | a consumer emptying the queue, or R34 | `BoundedWait` |

Both loops read the same flag (RT:19107, RT:19129) and pump `lotus_bus_queue_drain` every slice, which is owner-guarded: on the queue's owner it runs the pending cells, elsewhere it does nothing. On wasm32 the 1 ms `nanosleep` between slices is the inline no-op (SHIM:486), so a waiter spins on its own pump.

**Each lifecycle cell's omission is derived from the waits actually admitted**, not from the bindings alone. On wasm32 after T2 the binding-loss wait cannot arise, but the local capacity wait can: `BoundedWait × Wasm32 = Lower`, and an `on_full: fail` topic with `or wait` and no binding is a legal wasm32 program. Removing R34 there needs its own proof that no admitted local waiter can be live when a teardown spine runs. Its shape:

1. **One thread.** After T2, wasm32 admits no pool other than `main`, no `pinned` locus and no `async_io` pool (premises: `Threads`, `AsyncIoPool` `Reject`). Every waiter and every spine run on the one thread.
2. **So liveness means reentrancy.** A waiter is live during a spine only if the spine is entered while the waiter's frame is on the stack: from a cell its own pump runs (`lotus_bus_queue_drain` → a handler), or from the host re-entering the module through an export while the waiter is inside a host import.
3. **The Hale side.** No handler reaches a teardown spine. The spines are emitted only in `fn main`'s exits and the main locus's entry and dissolve (C13, C19, C21–C23), so this holds if `main` and the main locus's lifecycle cannot be reached from a cell. That is a claim for a checker rule and a test, not an assumption.
4. **The embedding side.** The loader (CG:2983) and every `@ffi("js")` import must not re-enter `main`, `_hale_start` or any export that runs a spine while an export is on the stack. The loader states no such contract today. The proof needs either an emitted reentrancy guard on those exports (a flag that traps on reentry, observable in a test) or a stated embedding contract that the loader enforces.
5. **Only then** is `WaitAbort × Wasm32` omitted, with the premise `All([Rejects(RemoteTransport), Proven(single_thread_no_live_waiter)])`, the proof registered with its tests. Until both 3 and 4 are shown, R34 stays emitted on wasm32 in **all five** spines (C19 gains it; today it skips it). It is one release store and drags in no import (RT:19079–19081). The other way to discharge the obligation, refusing `or wait` on wasm32 (`BoundedWait × Wasm32 = Reject`), is not recommended: it removes a working feature to save that store.

**The order is the decision's condition.** An obligation is removed for a target only after the matrix rejects, or legitimately lowers, every behaviour that needs it, and every admitted wait form is accounted for. The J commit (P3 2 of 3) lands `Threads`, `AsyncIoPool` and `RemoteTransport` as `Reject` on wasm32 first. P3 3 of 3 then makes all five spines read `cells.obligation(PoolJoin | WaitAbort | IngressQuiesce)` in one commit. The result:

- on wasm32 no spine emits R20 or R35, so C13 loses R20; every spine emits R34 until the proof of points 1–5 lands, so C19 gains it;
- on the host every spine emits all three.

The interim correction line 16 permits (gating C13 like the others) is unnecessary if P3 3 of 3 lands in wave 2. If L-line work needs it earlier, it gates R20 only and leaves R34 in every spine, so the spines do not diverge a third way. The rule is stated in the matrix, not in each spine, so L1's plan reads it: an obligation row exists for `(instance, action)` only if its obligation cell is `Emit` for the snapshot's effective target.

**The matrix selects; the lifecycle plan orders.** The matrix says which obligations a target owes and nothing about their order. The order belongs to L1's plan (PR #1300, `hale-types::lifecycle`), whose obligations carry entry and completion edges, and follows inventory decision 7. The rule: **wait-abort completes before any blocking join whose worker may be parked in a wait it ends is entered.** As edges in the plan:

- R34 completes before R20 is entered (the pool workers; decision 7, the fix);
- R34 completes before C18's pinned joins are entered (today's order, kept);
- R35 completes before R20 is entered (GH #468: ingress drains while pools and subscribers are intact, kept);
- R35 completes before R34 is entered (today's relative order, kept for a reason the plan records: the quiesce's drain runs handlers, and a handler's `or wait` that the drain itself can satisfy must not be aborted into a raise).

So the host spines run R35, R34, R20, then the pinned joins and the cascade. An omitted obligation takes its edges with it, and the plan stays acyclic either way. The plan asserts each edge as a dependency of the obligation it constrains; the IR test asserts that the emitted calls respect the plan's edges. Neither asserts that three calls are present in a spelled sequence.

P3 3 of 3's lifecycle commit depends on the line-7 reorder. If the L line has landed it, the commit only selects; if not, the reorder lands first, as its own commit in P3 3 of 3, taking `l07_pool_or_wait_teardown.hl` out of KNOWN_OPEN.

### 3.3 Tests, one per spine, on both targets

The program shapes:

- **eager:** a statement-position main locus literal, `fn main() { App { }; }` (C13);
- **deferred:** a let-bound one, `fn main() { let a = App { }; }` (C19);
- **return:** `fn main() -> Int { App { }; return 0; }` (C23).

Each is built in four variants that change one behaviour at a time, so each obligation's presence or omission is witnessed by the behaviour it depends on: a pool field only; an `or wait` publish on a bound topic only; an `or wait` publish on a local `on_full: fail` topic with no binding only; none of them. No variant removes the pool and the bound topic together. Per spine (P3 3 of 3, `crates/hale-codegen/tests/target_lifecycle_cells.rs`, joining the area that holds `wasm_target.rs`):

- **Host:** each variant's plan holds exactly the obligations its behaviours select, with the edges of §3.2, and the emitted calls respect those edges ahead of the first pinned join and the cascade. The IR-shape check follows `corpus_oracle.rs`'s module inspection and reads the order to check from the plan, not from a list in the test.
- **wasm32, the pool and bound-topic variants** are refused, and the test pins each refusal at its placement entry or binding. **The local bounded-topic variant and the bare one** build: the module calls neither `lotus_coop_pool_shutdown_all` nor `lotus_bus_ingress_quiesce`, calls `lotus_bus_wait_abort_all` in every spine (until the proof of §3.2 lands), and its import list contains no `pthread_*`, `epoll_*` or `eventfd`. This extends `wasm_build_emits_valid_module`'s byte check (`wasm_target.rs:787`) from one program to the spines.
- **wasm32, the C21/C22 spines too** (fall-through, and a `std::test::assert` failure), so all five agree; they share `emit_frame_teardown`.

The wait forms have their own acceptance tests, beside the spine tests:

- **The local capacity wait, no binding.** A topic with `on_full: fail` capacity, an `or wait` publish and no `bindings` entry: admitted by `hale check`, `hale build` and the editor under wasm32 alike; built and run under node with the queue filled first, so the publish parks and main's own pump empties the queue; the publish completes and stdout is pinned. The same program on the host, with the consumer on a pool, is the case below.
- **The binding-loss wait.** A CONNECT binding with `or wait`: refused at the binding under wasm32 (T2); on the host it emits R34 in every spine (the existing `or_wait_loss_window` tests, now per spine).
- **Teardown with a live waiter (host).** A local-capacity waiter on a pool worker at teardown completes in bounded time and raises (the L line's `l07_pool_or_wait_teardown.hl`, KNOWN_OPEN in PR #1300).
- **A pool worker in a real binding-loss wait (host), per spine: eager, deferred and return.** A pool-placed publisher on a CONNECT binding whose peer the test closes, with an `on_failure` that declines to reconnect, so the binding stays `lost`. Synchronized, not slept: teardown starts only after the runtime has counted the worker's park (the binding's `ctr_waits`, bumped on the first slice at RT:19104, read through the bus stats or a test hook). Then the owner tears down. The process must finish within a deadline (the corpus oracle's), with the parked publish taking the raise path. Today each spine hangs in R20's join; after the reorder each completes.
- **Reentrancy (wasm32, only if the proof is pursued).** An exported fn parks in a local capacity wait; a handler its pump runs calls an `@ffi("js")` import that re-enters `main` or `_hale_start`. The reentry traps, or the embedding contract refuses it, as point 4 chose. Without this test the omission is not taken.
- **L1/L2 linkage:** once L1's plan exists, the plan of each wasm32 program has no R20 or R35 obligation and keeps R34 (until the §3.2 proof), and the host plan has all three. Until then, the IR assertion is the oracle.

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
5. The lowering contracts, their operation coverage and their probes (§2.1), with the injected-no-op test of the verification itself. Their results are T3's evidence for every namespace that is not one of the known stubs.
6. The generated doc region and its test (§4), rendering today's cells.

Closes nothing yet; the registry names `capability.rs` as the producer, with the legacy rows still permitted.

### P3 2 of 3: admission on every entry point (J · Opus pane; the driver writes T1–T5 into spec/ffi.md and docs before the pane starts)

1. The effective target (§1.3) and `hale check --target`; `--wrap-main` reads the effective target; `hale build` takes its `CompileTarget` and output naming (`TargetSpec::filenames`) from the snapshot's effective target, not from OPT:312 alone, so a declared program emits the wasm artifact (T1(b)).
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
| 8 (reverse) | `@export fn go() { }` with no `fn main` and no declaration | refused at the `@export` (`EntryInversion × Host`, §1.3's wording) | lowers |
| 9 (reverse) | `@ffi("js") fn console_log(m: String);` called from main | refused at the declaration (T5) | lowers |
| 10 | `main locus App { params { l: std::io::tcp::Listener = std::io::tcp::Listener { … }; } }`, no placement | lowers | refused at the struct literal; witness `Listener` → its `run()` accept → `StdNamespace(io::tcp)` |
| 11 | `fn serve(conn: std::io::tcp::Stream) { let _ = conn.recv(64) or ""; }` (the receiver shape of `examples/http-hello/main.hl:25`) | lowers | the parameter type admitted; refused at `conn.recv`, witness the resolved method → `StdNamespace(io::tcp)` |
| 12 | `import "clock" as c;` whose `stamp()` calls `std::time::now()`; main calls `c::stamp()` | lowers | refused at `c::stamp()`, witness `c::stamp` → `std::time::now` → the clock-read contract row |
| 13 | `fn pid() -> Int { return std::process::pid(); }` called from main twice | lowers | refused once, at the written call inside `pid`, none at the two call sites |

Cases 10–13 are the use producer's acceptance cases: each asserts the location and the full witness chain, on check, build and the editor alike, and a type-only variant of each (the same declarations with the use removed) is admitted.

### P3 3 of 3: codegen reads the cells; lifecycle; the gate column (M · Sonnet pane for the site switch; S · Opus for the lifecycle commit and the gate column)

1. Every skip/substitute/export site of §2.5 reads a cell, one commit per capability: `ProcessSignals`, `ReplayIngress`, `RemoteTransport`, `ExportSurface`/`EntryInversion`, `ForeignAbi`. The `is_wasm` field is deleted, and backend sites read `self.target`. The oracle: IR identical over the corpus and `wasm_target.rs` for both targets (the module dump before and after).
2. The lifecycle commit (§3): the obligations from the cells in all five spines, ordered by the plan's edges (§3.2), with the spine and wait-form tests of §3.3. It lands after the line-7 reorder, or carries it as the commit before.
3. The gate column (§2.8): `payload_flat` on the gate row, `DispatchFlavor::of` with three legs, codegen reading the flavor, and the shadow against the old predicate at every publish site.
4. The **portable subset**: programs whose stdout agrees byte-for-byte between the native binary and `node <loader>.mjs`. These are `wasm_target.rs` :846 (`STRUCTSUM=330`), :792 (run it; sum of squares 1..10, 385), :747 (`wrapped-main-ran`), :87 and :208 with `console_log` replaced by `println`, `wasm_link_is_quiet.rs:66` (`len=5`, `b0=104`, `hello`), `wasm_target_gating.rs:39` (`n=42`), and `play/examples/{collections,decimal,closure,enums,fallible,jobqueue}.hl` through `--wrap-main`. Each is built for both targets and run on both, and the outputs are compared. The test skips, naming what is missing, only when node or wasm-ld is absent. CI must have both: today `tests.yml` installs `clang-18` and not `lld` explicitly, so P3 3 of 3 states the dependency in the workflow. The comparison proves agreement for these programs and nothing outside them: no namespace or operation is `Lower` because a subset program happens to call it.
5. The import backstop (§2.3, T7), for unresolved imports only.
6. Closes `target_capability` · `link_wasm` (export list), `lotus_replay_start_ingress`, `is_wasm`, `ffi_type_unportable` (as the matrix's column); `dispatch` · `bus_payload_is_flat`. The family becomes Canonical.

## 6. What the rows contradict

1. **The registry's `lotus_replay_start_ingress` row** ("instantiation still emits pool shutdown and wait-abort on wasm where the main exit does not") is half right. Every main exit emits wait-abort on wasm too (CG:6954–6956). Only the deferred entry (CG:7125) emits neither. The inventory's line 16 inherits the same undercount (§3.1).
2. **`TargetSpec::has_async_io` is true for wasm32** (TGT:322), and the runtime agrees (`LOTUS_HAVE_ASYNC_IO` 1, RT:142–147). But the backend's epoll/eventfd calls are `() => 0` imports (CG:2983–2986), so an `async_io` pool on wasm is admitted and never runs.
3. **SHIM:47–53 says pool workers and `cond_wait` are "gated out with `#ifndef __wasm__`"**. They are not: `lotus_coop_pool_start_all` (RT:10075) and `lotus_coop_pool_shutdown_all` (RT:10149) lie outside the gated region (RT:9827–10034), and pinned `pthread_create` (INST:4262–4272) is ungated in codegen.
4. **spec/ffi.md:378 says the declaration "opts into the wasm backend"**. It selects no backend: only `--target` does (OPT:312), and codegen ignores the declaration (CG:13284). docs/src/systems/webassembly.md:19–21 is accurate ("the program declares the target so the typechecker can gate"), so the spec and the book disagree. T1(b) resolves it toward the spec: the declaration selects the backend, and the book's sentence gains the build half in P3 2 of 3.
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
