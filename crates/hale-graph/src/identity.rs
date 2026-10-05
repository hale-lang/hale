//! Identity coverage (F.40 phase 0, step 0.4).
//!
//! Three identities name the compiler that produced something: the
//! replay identity (`HALE_TOOLCHAIN_SHA256`, `crates/hale-cli/build.rs`),
//! the toolchain cache key the DNA host and observer are cached under
//! (`HALE_COMPILER_SRC_HASH`, `crates/hale-iris/build.rs`), and the
//! stale-binary hash (`HALE_CODEGEN_SRC_HASH`, the same script and
//! `crates/hale-cli/src/shared/stale.rs`). Each walked its own list
//! of directories with its own walk, and none of the lists named
//! `hale-model`, so a model-shape change did not bust a cached host or
//! refuse a recording. A semantic producer moving between crates in
//! phase 1 must never make a later edit invisible to any of them.
//!
//! This module is the one list and the one walk. Build scripts link
//! it as a build-dependency (the crate depends on nothing, so there
//! is no cycle), `stale.rs` links it as a dependency, and the walk
//! they share is what makes the stale hash's build-time value and
//! run-time recomputation equal by construction.

use std::path::{Path, PathBuf};

/// The crates whose sources every compiler identity covers, in
/// workspace order: what shapes a compiled program or a recording.
///
/// `hale-cli` is here because the DNA toolchain cache builds a host
/// by invoking the CLI's `build` verb, and that verb's Rust still owns
/// semantic work: the config its snapshot is loaded with (the target,
/// `--env`'s roles and constitutions, read from hale.toml), the
/// `[ffi]` pickup, and the identity it stamps. The load, the pre-check
/// sequence and the mint are hale-frontend's since F.40 phase 2.2b. A
/// change in what the verb still owns changes the binary a cached host
/// is. It leaves this list when that work has moved into hale-frontend
/// and the verb only drives it; the replay identity then keeps walking
/// the CLI as its own extra, since the CLI produces and serves
/// recordings. It is last so the replay identity's file order, and so
/// its digest, is what it was when the CLI was that extra.
///
/// `hale-frontend` loads and merges the seeds and drives the mangling
/// (F.40 phase 2.1a moved it out of the CLI), so it shapes every
/// compiled program.
pub const COVERED_CRATES: &[&str] = &[
    "hale-syntax",
    "hale-types",
    "hale-model",
    "hale-graph",
    "hale-codegen",
    "hale-frontend",
    "hale-stdlib",
    "hale-cli",
];

/// The workspace members no identity covers, each with the reason:
/// none of them shapes what a program compiles to or a recording.
pub const NOT_COVERED: &[(&str, &str)] = &[
    (
        "hale-lsp",
        "serves diagnostics; emits no artifact and no recording",
    ),
    (
        "hale-iris",
        "its embedded observer and DNA trees ride the cache key as files through all_files(); its Rust only materializes them",
    ),
    (
        "hale-dna",
        "its embedded source tree rides the cache key as files and EMBEDDED_DIGEST names it; its Rust only carries them",
    ),
    (
        "hale-corpus",
        "test programs for the workspace's own tests",
    ),
    (
        "hale-ts-shim",
        "its library is hale-codegen/runtime/lotus_treesitter.rs, covered under hale-codegen; what else shapes a std::ts binary is the tree-sitter versions its manifest and the lock file pin, which MANIFEST_FILES carries",
    ),
];

/// Files outside the covered crates' source trees that shape a
/// compiled program: the lock file (every dependency version the
/// compiler and its shims are built with) and the ts-shim manifest
/// (the tree-sitter versions `std::ts` links). Both identities fold
/// them beside the sources, through [`identity_files`].
pub const MANIFEST_FILES: &[&str] = &["Cargo.lock", "crates/hale-ts-shim/Cargo.toml"];

/// The subdirectories of a covered crate that hold sources.
pub const SOURCE_DIRS: &[&str] = &["src", "runtime", "hl"];

/// The source extensions an identity reads.
pub const SOURCE_EXTENSIONS: &[&str] = &["rs", "c", "h", "hl"];

/// Every source file under `dir`, recursively, in path order. A
/// missing directory contributes nothing.
pub fn walk_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    items.sort();
    for p in items {
        if p.is_dir() {
            walk_sources(&p, out);
        } else if p
            .extension()
            .and_then(|s| s.to_str())
            .map(|e| SOURCE_EXTENSIONS.contains(&e))
            .unwrap_or(false)
        {
            out.push(p);
        }
    }
}

/// The source directories of the covered crates plus `extra` crates,
/// under `workspace_root`, in list order: the directories an identity
/// declares `rerun-if-changed` on and walks. Only directories that
/// exist: Cargo treats a missing `rerun-if-changed` path as always
/// stale, which rebuilt hale-iris on every cargo invocation until
/// this filter (F.40 phase-0 review).
pub fn covered_dirs(workspace_root: &Path, extra: &[&str]) -> Vec<PathBuf> {
    COVERED_CRATES
        .iter()
        .chain(extra.iter())
        .flat_map(|krate| {
            SOURCE_DIRS
                .iter()
                .map(move |d| workspace_root.join("crates").join(krate).join(d))
        })
        .filter(|d| d.is_dir())
        .collect()
}

/// The manifest files an identity frames beside the sources, those
/// that exist under `workspace_root`.
pub fn manifest_files(workspace_root: &Path) -> Vec<PathBuf> {
    MANIFEST_FILES
        .iter()
        .map(|f| workspace_root.join(f))
        .filter(|p| p.is_file())
        .collect()
}

/// The files the stale-binary hash covers: `codegen.rs`, the C runtime's
/// `lotus_arena.c`, and every `.hl` seed of the stdlib. One list for
/// `build.rs` (`HALE_CODEGEN_SRC_HASH`) and `stale.rs` (its run-time
/// recomputation), with the same path strings.
pub fn stale_hash_paths(codegen_dir: &Path) -> Vec<PathBuf> {
    let mut paths = vec![
        codegen_dir.join("src").join("codegen.rs"),
        codegen_dir.join("runtime").join("lotus_arena.c"),
    ];
    let mut seeds = Vec::new();
    walk_sources(
        &codegen_dir.join("..").join("hale-stdlib").join("hl"),
        &mut seeds,
    );
    seeds.retain(|p| p.extension().and_then(|s| s.to_str()) == Some("hl"));
    paths.extend(seeds);
    paths
}

/// Every source file of the covered crates plus `extra`, in path
/// order.
pub fn covered_files(workspace_root: &Path, extra: &[&str]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for d in covered_dirs(workspace_root, extra) {
        walk_sources(&d, &mut files);
    }
    files
}

/// What the replay identity and the toolchain cache key hash: every
/// source file of the covered crates, then the manifest files, in
/// that order. One selection for both, so neither can leave out an
/// input the other covers.
pub fn identity_files(workspace_root: &Path) -> Vec<PathBuf> {
    let mut files = covered_files(workspace_root, &[]);
    files.extend(manifest_files(workspace_root));
    files
}

/// A 64-bit FNV-1a fold over `(relative path, NUL, contents, NUL)` for
/// every file, in the order given: the cache key's fold. A renamed,
/// added or removed file moves it, as does one changed byte.
pub fn fold_files(root: &Path, files: &[PathBuf]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    };
    for f in files {
        let rel = f
            .strip_prefix(root)
            .unwrap_or(f)
            .to_string_lossy()
            .replace('\\', "/");
        eat(rel.as_bytes());
        eat(&[0]);
        eat(&std::fs::read(f).unwrap_or_default());
        eat(&[0]);
    }
    h
}

/// How an identity folds what it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fold {
    /// A length-framed SHA-256.
    Sha256,
    /// 64-bit FNV-1a, in one of its framings.
    Fnv64,
    /// `std`'s `DefaultHasher` (SipHash; its algorithm is unspecified,
    /// so the value is stable for one toolchain only).
    DefaultHasher,
    /// No fold: a key compared field by field.
    Structural,
}

/// A class of input an identity can cover. A class, not a file: what
/// matters to a reader is whether an edit of that kind moves the
/// value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// The covered crates' Rust sources ([`COVERED_CRATES`]).
    CompilerSources,
    /// The C runtime's translation units and headers.
    RuntimeC,
    /// The stdlib's `.hl` seeds.
    StdlibSeeds,
    /// The lock file and the ts-shim manifest ([`MANIFEST_FILES`]).
    Manifests,
    CompilerVersion,
    RustcVersion,
    GitCommit,
    /// The program's own source text.
    UserSources,
    /// The paths its source files are named by.
    SourcePaths,
    /// The build options that alter emitted code.
    BuildOptions,
    Target,
    /// The environment's roles and constitutions.
    EnvironmentRoles,
    /// The model's hashed half (the topology artifact's `model`).
    ModelHalf,
    /// The whole model, unhashed sections included.
    Model,
    /// The law rows and their issues.
    LawRows,
    /// The lowering's dispatch plan.
    DispatchPlan,
    /// The DNA source trees a binary embeds.
    EmbeddedDna,
    /// The iris trees a binary embeds.
    EmbeddedIris,
    /// The analyses' hand-bumped semantics version.
    AnalysisVersion,
    /// The stdlib-surface classification registry.
    SurfaceRegistry,
    /// The stdlib path-rename table.
    PathRenames,
    /// Constitution declarations, as a normalized closure.
    Constitutions,
    /// A snapshot's load configuration (the check's switches).
    SnapshotConfig,
    /// The editor buffers a load read over the disk.
    EditorBuffers,
    /// A fleet plan's model text.
    FleetPlan,
    /// The C compiler, its version and its flags.
    CCompiler,
}

/// One identity: a value compared for equality somewhere to decide
/// that two things are the same. Written down here once, with what it
/// covers and what it leaves out, so a contributor changing a producer
/// reads what the value promises and a reviewer sees a coverage change
/// as a change to this table. `spec/registry.md`'s `digests` section is
/// rendered from it.
///
/// The `leaves_out` reasons say which absences are by design and which
/// are defects or gaps with a step that closes them (F.40 phase 4,
/// line I): a reason beginning "a defect" or "a gap" is not a
/// justification.
#[derive(Debug, Clone, Copy)]
pub struct Identity {
    /// The name a contributor greps: `exec_digest`.
    pub name: &'static str,
    /// What two equal values mean, one sentence.
    pub identifies: &'static str,
    /// When the value is computed.
    pub computed: &'static str,
    pub fold: Fold,
    pub covers: &'static [Input],
    /// An input class a reader would expect, with the stated reason it
    /// is absent.
    pub leaves_out: &'static [(Input, &'static str)],
    /// `(file, symbol)`: the one place the value is computed.
    pub producer: (&'static str, &'static str),
    pub consumers: &'static [&'static str],
    /// What a mismatch does.
    pub on_mismatch: &'static str,
    /// How a change of coverage is made visible.
    pub versioned_by: &'static str,
    /// An external contract that pins the value or its shape.
    pub frozen: Option<&'static str>,
}

/// Every identity in the workspace, in the order the registry renders
/// them: the model's, the build's, the compiler's, the cache's, the
/// snapshot's. `identity_coverage.rs` holds each producer to a symbol
/// that exists, the names unique, and the file-walking entries to the
/// selection they call.
pub const IDENTITIES: &[Identity] = &[
    Identity {
        name: "shape_hash",
        identifies: "two programs have the same structural model: the hashed half of the topology artifact, rendered from the model alone",
        computed: "artifact emission (`hale check --dump-topology`) and every build (`model_identity`)",
        fold: Fold::Fnv64,
        covers: &[Input::ModelHalf],
        leaves_out: &[
            (Input::SourcePaths, "by design: a structural identity, so a moved comment or a renamed file does not churn it"),
            (Input::UserSources, "by design: it names what the program is, not the text it was written in"),
            (Input::LawRows, "by design: the law rows are a section of their own, under `law_digest`"),
        ],
        producer: ("crates/hale-types/src/topology_projection.rs", "project_shape_hash"),
        consumers: &["topology / fleet (admission)", "replay (admission, the recording's header)", "dna (schema 1.19, semantics 2)", "baseline gates"],
        on_mismatch: "the artifact is refused (`verify_shape_hash`), a replay is refused, a baseline gate fails",
        versioned_by: "the artifact schema (`TOPOLOGY_SCHEMA`, 1.19)",
        frozen: Some("the 1,793 pinned values of `topology_projection`, the nine of `model_arrangement`, and DNA, which requires schema 1.19 and recomputes it"),
    },
    Identity {
        name: "model_hash",
        identifies: "a binary was built from a model with this `shape_hash`: the same value, stamped into the recording and observation headers",
        computed: "hale build, run, replay (`model_identity`, from the snapshot's model)",
        fold: Fold::Fnv64,
        covers: &[Input::ModelHalf],
        leaves_out: &[(Input::BuildOptions, "by design: the model is not the build; `exec_digest` carries the options")],
        producer: ("crates/hale-cli/src/shared/options.rs", "model_identity"),
        consumers: &["replay (the recording's header)", "the runtime obs header (`lotus_obs_model_hash_set`)", "iris readers"],
        on_mismatch: "replay is refused",
        versioned_by: "moves with `shape_hash`: the artifact schema",
        frozen: Some("the iris protocol's header layout"),
    },
    Identity {
        name: "artifact_digest",
        identifies: "the whole topology artifact, results and provenance included, is the one a producer emitted: an unkeyed tripwire, not a signature",
        computed: "artifact emission",
        fold: Fold::Fnv64,
        covers: &[Input::ModelHalf, Input::Model, Input::LawRows, Input::UserSources, Input::SourcePaths, Input::AnalysisVersion],
        leaves_out: &[(Input::CompilerSources, "by design: it is a checksum of the document; the compiler that wrote it is named by nothing in it")],
        producer: ("crates/hale-types/src/topology.rs", "dump_topology_over"),
        consumers: &["topology / fleet (admission)", "dna (recomputes it)", "`hale fleet` with declared trust roots"],
        on_mismatch: "the artifact is refused",
        versioned_by: "the artifact schema; a section moved by I3 changes values and never the section's shape",
        frozen: Some("DNA requires schema 1.19 exactly and recomputes it"),
    },
    Identity {
        name: "law_digest",
        identifies: "the artifact's law rows and issues are the ones emitted: a fold of their canonical JSON",
        computed: "artifact emission",
        fold: Fold::Fnv64,
        covers: &[Input::LawRows],
        leaves_out: &[],
        producer: ("crates/hale-types/src/topology.rs", "dump_topology_over"),
        consumers: &["admission (`topology_law.rs`, which recomputes it from the rows)"],
        on_mismatch: "the artifact is refused: a row edited under a stale digest",
        versioned_by: "the artifact schema",
        frozen: Some("DNA reads the law section under schema 1.19"),
    },
    Identity {
        name: "claim_table_digest",
        identifies: "an evidence sidecar was derived from this claim table: its rows, origins, laws and provenance store",
        computed: "evidence derivation (`ClaimIrTable::semantic_digest`)",
        fold: Fold::Fnv64,
        covers: &[Input::LawRows],
        leaves_out: &[(Input::ModelHalf, "by design: the sidecar names the model by `model_shape`; two programs of one topology with different `@effects` classes must not accept each other's evidence, which is why the law table is hashed beside it")],
        producer: ("crates/hale-model/src/claim_ir.rs", "semantic_digest"),
        consumers: &["`EvidenceTable::validate` (the judgment)"],
        on_mismatch: "the evidence is refused (`InvalidProvenanceRecord`)",
        versioned_by: "the crate's own FNV hasher, never `DefaultHasher`; a change of what the table holds moves it",
        frozen: None,
    },
    Identity {
        name: "analysis_coverage_digest",
        identifies: "an evidence sidecar was derived over a model whose analysis coverage (the analyzable, analyzed, summarized and ownership bits) is this one",
        computed: "evidence derivation",
        fold: Fold::Fnv64,
        covers: &[Input::Model],
        leaves_out: &[(Input::SourcePaths, "by design: coverage bits per entity, by canonical entity order")],
        producer: ("crates/hale-model/src/application.rs", "analysis_coverage_digest"),
        consumers: &["`EvidenceTable::validate` (the judgment)"],
        on_mismatch: "the evidence is refused (`InvalidProvenanceRecord`)",
        versioned_by: "a change of the bits it folds moves it; `ANALYSIS_SEMANTICS_VERSION` covers the analyses' meaning",
        frozen: None,
    },
    Identity {
        name: "dispatch_plan_digest",
        identifies: "two builds lowered the bus with the same dispatch plan: each subject, its flavor and its subscribers",
        computed: "hale build, run, replay (the lowering view's plan; the default plan under `no_bus_devirt`)",
        fold: Fold::Fnv64,
        covers: &[Input::DispatchPlan],
        leaves_out: &[(Input::DispatchPlan, "the same-domain column is a reserved zero byte that no lowering reads; GH #464 makes it a column again")],
        producer: ("crates/hale-model/src/dispatch_plan.rs", "digest"),
        consumers: &["`exec_digest` frames it"],
        on_mismatch: "none of its own: it moves `exec_digest`, so a replay is refused",
        versioned_by: "a change of what lowering reads moves it, and with it every recording's `exec_digest`",
        frozen: None,
    },
    Identity {
        name: "exec_digest",
        identifies: "a build and a recording were made from the same build inputs: the toolchain, the options, the plan and the user's sources",
        computed: "hale build, run, replay",
        fold: Fold::Sha256,
        covers: &[Input::CompilerSources, Input::RuntimeC, Input::StdlibSeeds, Input::Manifests, Input::RustcVersion, Input::GitCommit, Input::CompilerVersion, Input::BuildOptions, Input::DispatchPlan, Input::UserSources, Input::SourcePaths],
        leaves_out: &[
            (Input::BuildOptions, "a defect, corrected in I2: `debug` is fingerprinted as set by `hale build` and never by `run` or `replay`, so a built binary's recording never replays; the `[ffi]` pickup happens in `build` only, and `replay` accepts `--env` and never resolves it"),
            (Input::SourcePaths, "a defect, corrected in I3: paths are framed relative to the entry's parent and fall back to the bare file name, so a directory build's recording never matches its file and two imports of one file name share an identity; the frame follows absolute paths' order"),
        ],
        producer: ("crates/hale-cli/src/shared/options.rs", "exec_digest"),
        consumers: &["replay (admission)", "the recording header"],
        on_mismatch: "replay refused",
        versioned_by: "moves with every compiler commit (the toolchain half frames the commit)",
        frozen: Some("the default options fingerprint string (`build_env.rs`'s test)"),
    },
    Identity {
        name: "toolchain_digest",
        identifies: "the replay identity's compiler half: a recording was made by a compiler built from these sources, by this rustc, at this commit",
        computed: "compiler build (`HALE_TOOLCHAIN_SHA256`, hale-cli/build.rs)",
        fold: Fold::Sha256,
        covers: &[Input::CompilerSources, Input::RuntimeC, Input::StdlibSeeds, Input::Manifests, Input::RustcVersion, Input::GitCommit],
        leaves_out: &[
            (Input::UserSources, "by design: the toolchain is not the program; `exec_digest` frames the sources beside it"),
            (Input::BuildOptions, "by design: options are a build's, `exec_digest` frames them"),
        ],
        producer: ("crates/hale-cli/build.rs", "toolchain_digest"),
        consumers: &["`exec_digest`"],
        on_mismatch: "replay refused (through `exec_digest`)",
        versioned_by: "moves with every compiler commit; a framing of the one shared selection (`identity_files`)",
        frozen: None,
    },
    Identity {
        name: "codegen_src_hash",
        identifies: "the stale-binary warning's hash: the binary was built from the codegen, runtime and stdlib files now on disk",
        computed: "compiler build (`HALE_CODEGEN_SRC_HASH`, hale-cli/build.rs) and every check, verify, build, run, test, dna and inputs invocation in a development checkout (`compute_codegen_src_hash`)",
        fold: Fold::DefaultHasher,
        covers: &[Input::CompilerSources, Input::RuntimeC, Input::StdlibSeeds],
        leaves_out: &[
            (Input::CompilerSources, "a gap, closed in I4: it folds one of codegen's source files (`codegen.rs`) and none of the other covered crates'"),
            (Input::RuntimeC, "a gap, closed in I4: it folds one of the runtime's C files (`lotus_arena.c`), no other C file or header"),
            (Input::Manifests, "a gap, closed in I4"),
        ],
        producer: ("crates/hale-cli/src/shared/stale.rs", "compute_codegen_src_hash"),
        consumers: &["`check_stale_cli`"],
        on_mismatch: "a warning on stderr",
        versioned_by: "none: its selection is `stale_hash_paths`; a change of coverage is a change of that function",
        frozen: None,
    },
    Identity {
        name: "compiler_src_hash",
        identifies: "the DNA host cache's compiler half: the same file selection as `toolchain_digest`, folded without the rustc version or the commit, because the cache is keyed by what this binary would build",
        computed: "compiler build (`HALE_COMPILER_SRC_HASH`, hale-iris/build.rs)",
        fold: Fold::Fnv64,
        covers: &[Input::CompilerSources, Input::RuntimeC, Input::StdlibSeeds, Input::Manifests],
        leaves_out: &[
            (Input::RustcVersion, "by design: two binaries of one source build one host"),
            (Input::GitCommit, "by design: the commit names no source the files do not"),
            (Input::BuildOptions, "a gap, closed in I5: the build knobs the cache's `hale build` subprocess inherits from the environment (`HALE_DEV`, `LOTUS_NO_DEBUGINFO`, a sanitizer) are in no key"),
        ],
        producer: ("crates/hale-iris/build.rs", "main"),
        consumers: &["`toolchain_hash`"],
        on_mismatch: "the cache directory is new; the host is rebuilt",
        versioned_by: "moves with every compiler source edit; the shared selection (`identity_files`)",
        frozen: None,
    },
    Identity {
        name: "toolchain_hash",
        identifies: "the DNA host cache's key: the compiler's sources, the stdlib and the embedded iris and DNA trees are the ones the cached host was built from",
        computed: "hale iris, hale dna (per invocation)",
        fold: Fold::Fnv64,
        covers: &[Input::CompilerVersion, Input::CompilerSources, Input::RuntimeC, Input::StdlibSeeds, Input::Manifests, Input::EmbeddedDna, Input::EmbeddedIris],
        leaves_out: &[
            (Input::BuildOptions, "a gap, closed in I5: the environment's build knobs are in no key, so a host cached under a sanitizer or `HALE_DEV` is served to a run without it"),
            (Input::StdlibSeeds, "a gap, closed in I5: the stdlib is folded twice, once in `compiler_src_hash` and once as the embedded `AP_FILES`"),
            (Input::RustcVersion, "by design: see `compiler_src_hash`"),
        ],
        producer: ("crates/hale-iris/src/lib.rs", "toolchain_hash"),
        consumers: &["the DNA host cache directory (`~/.cache/hale/iris/<hash>`)"],
        on_mismatch: "the cache directory is new; the host is rebuilt",
        versioned_by: "moves with the version, the compiler's sources and every embedded byte",
        frozen: None,
    },
    Identity {
        name: "embedded_dna_digest",
        identifies: "a binary embeds this DNA source set: the files of the listed directories, by path and content",
        computed: "compiler build (`HALE_DNA_EMBEDDED_DIGEST`, hale-dna/build.rs), and over any checkout's tree (`digest_of_tree`)",
        fold: Fold::Sha256,
        covers: &[Input::EmbeddedDna],
        leaves_out: &[(Input::EmbeddedDna, "by design: the directories are listed non-recursively, with the extensions each contributes (`EMBEDDED_DIRS`); a file outside them is not embedded")],
        producer: ("crates/hale-dna/src/digest.rs", "digest_of_pairs"),
        consumers: &["`hale --version`", "`hale dna status`", "the `vendor/dna` provenance a user's tree records", "the stale-DNA warning"],
        on_mismatch: "a warning on stderr (the stale-DNA check); `hale dna status` reports the difference",
        versioned_by: "a framing tag (`hale-dna-embedded-v1`)",
        frozen: Some("persisted in users' trees (`vendor/dna` provenance): its value never moves with a refactor"),
    },
    Identity {
        name: "analysis_inputs_digest",
        identifies: "evidence was produced by the same analyses: the stdlib source they absorb, the compiler version, the path renames and the surface registry; independent of the program by design",
        computed: "artifact emission and admission (recomputed by the binary that reads it)",
        fold: Fold::Fnv64,
        covers: &[Input::AnalysisVersion, Input::StdlibSeeds, Input::CompilerVersion, Input::PathRenames, Input::SurfaceRegistry],
        leaves_out: &[
            (Input::UserSources, "by design: it names the analyses, never the program"),
            (Input::CompilerSources, "by design: the analyses' Rust is versioned by `ANALYSIS_SEMANTICS_VERSION`, bumped by hand when a certificate's meaning moves"),
        ],
        producer: ("crates/hale-types/src/evidence.rs", "analysis_inputs_digest"),
        consumers: &["the artifact's `law.inputs_digest`", "`EvidenceTable::validate`", "admission (`topology_law.rs`)"],
        on_mismatch: "the artifact is refused: evidence produced under another analysis snapshot",
        versioned_by: "a hand-bumped constant (`ANALYSIS_SEMANTICS_VERSION`, 7), and the compiler version",
        frozen: None,
    },
    Identity {
        name: "source_digest",
        identifies: "a source file has this text: the per-file digest of the snapshot's source map",
        computed: "snapshot load (`source_map`, hale-frontend)",
        fold: Fold::Fnv64,
        covers: &[Input::UserSources],
        leaves_out: &[
            (Input::SourcePaths, "the path rides beside it in the source map, not in the digest"),
            (Input::SourcePaths, "a defect, corrected in I3: a file outside the workspace root keeps its absolute path, so an artifact is specific to the machine that built it"),
        ],
        producer: ("crates/hale-frontend/src/frontend.rs", "source_map"),
        consumers: &["the artifact's `sources` section (so `artifact_digest`)", "the evidence sidecar's source-unit tie"],
        on_mismatch: "the evidence is refused; the artifact's digest differs",
        versioned_by: "the artifact schema",
        frozen: Some("the `sources` section's shape (schema 1.19)"),
    },
    Identity {
        name: "obs_entity_id_digest",
        identifies: "two observed id tables number the entities identically: the `(kind, name, id)` rows a build stamped, which an external consumer recomputes from its own copy of the model",
        computed: "hale build (codegen stamps it into the obs header), and by a consumer from its model",
        fold: Fold::Fnv64,
        covers: &[Input::Model],
        leaves_out: &[(Input::SourcePaths, "by design: keyed by (kind, name), because no artifact carries a site identity; keying it by site would break the consumer and churn on unrelated edits")],
        producer: ("crates/hale-model/src/obs_ids.rs", "digest"),
        consumers: &["the runtime obs header (offset 0x88, proto 0.3)", "iris consumers"],
        on_mismatch: "no match, no join: a consumer refuses the ids",
        versioned_by: "the iris protocol's version (0.3)",
        frozen: Some("nine literal values in `model_arrangement`; `PROTOCOL.md` §3.2"),
    },
    Identity {
        name: "snapshot_key",
        identifies: "two snapshots were loaded from the same inputs and share a result: the entry, the load mode, the target and the three component digests",
        computed: "snapshot load",
        fold: Fold::Structural,
        covers: &[Input::SourcePaths, Input::Target, Input::SnapshotConfig, Input::EditorBuffers, Input::UserSources],
        leaves_out: &[
            (Input::CompilerSources, "by design: the key is snapshot-local; no binary or recording carries it"),
            (Input::BuildOptions, "by design: codegen options are not a load's"),
        ],
        producer: ("crates/hale-frontend/src/snapshot.rs", "SnapshotKey"),
        consumers: &["snapshot equality (two keys share no result)"],
        on_mismatch: "the snapshots are distinct and share no result",
        versioned_by: "its fields: a component added is a field added",
        frozen: None,
    },
    Identity {
        name: "snapshot_config_digest",
        identifies: "the load's configuration is the same: the target and its spec, the api and roles, the environment and the check's switches",
        computed: "snapshot load (`Config::digest`)",
        fold: Fold::Fnv64,
        covers: &[Input::Target, Input::EnvironmentRoles, Input::SnapshotConfig],
        leaves_out: &[],
        producer: ("crates/hale-frontend/src/snapshot.rs", "digest"),
        consumers: &["`snapshot_key`"],
        on_mismatch: "the snapshots are distinct",
        versioned_by: "length-framed fields: a switch added is a field added",
        frozen: None,
    },
    Identity {
        name: "snapshot_overlay_digest",
        identifies: "the editor buffers a load read over the disk are the same",
        computed: "snapshot load (`SourceProvider::overlay_digest`)",
        fold: Fold::Fnv64,
        covers: &[Input::EditorBuffers],
        leaves_out: &[],
        producer: ("crates/hale-frontend/src/source.rs", "overlay_digest"),
        consumers: &["`snapshot_key`"],
        on_mismatch: "the snapshots are distinct",
        versioned_by: "zero for the disk itself; length-framed fields",
        frozen: None,
    },
    Identity {
        name: "snapshot_sources_digest",
        identifies: "a load read the same source text: every unit's path and text, imports included",
        computed: "snapshot load",
        fold: Fold::Fnv64,
        covers: &[Input::UserSources, Input::SourcePaths],
        leaves_out: &[(Input::SourcePaths, "a defect, corrected in I3: the path is hashed absolute, so one program checked out at two roots has two keys")],
        producer: ("crates/hale-frontend/src/snapshot.rs", "load"),
        consumers: &["`snapshot_key`"],
        on_mismatch: "the snapshots are distinct",
        versioned_by: "none: the order is the load's",
        frozen: None,
    },
    Identity {
        name: "constitution_digest",
        identifies: "two constitutions have the same normalized closure: their bases (deduplicated) and entries, rendered as forms",
        computed: "check and artifact emission (`constitution_identities`)",
        fold: Fold::Fnv64,
        covers: &[Input::Constitutions],
        leaves_out: &[(Input::SourcePaths, "by design: the closure's text, not where it was declared")],
        producer: ("crates/hale-types/src/claims.rs", "constitution_digest"),
        consumers: &["the environment matrix (adoption identity comparison)", "the artifact's constitution identities"],
        on_mismatch: "the matrix reports two environments that adopt different closures",
        versioned_by: "none: a change of the rendering moves it",
        frozen: None,
    },
    Identity {
        name: "fleet_shape_hash",
        identifies: "a fleet plan's model text is the one printed: the plan artifact's own shape",
        computed: "fleet plan emission",
        fold: Fold::Fnv64,
        covers: &[Input::FleetPlan],
        leaves_out: &[],
        producer: ("crates/hale-cli/src/fleet.rs", "fnv"),
        consumers: &["`hale fleet` prints it"],
        on_mismatch: "none: nothing in the workspace compares it, it is printed",
        versioned_by: "none",
        frozen: None,
    },
    Identity {
        name: "runtime_object_key",
        identifies: "a cached runtime object was compiled from this C source with these flags by this C compiler",
        computed: "hale build (`compile_cached_runtime_object_with`)",
        fold: Fold::DefaultHasher,
        covers: &[Input::RuntimeC, Input::CCompiler],
        leaves_out: &[(Input::CCompiler, "the host `clang` hashes as it always has, so a cached object stays valid; only another compiler or another release moves the key")],
        producer: ("crates/hale-codegen/src/codegen.rs", "compile_cached_runtime_object_with"),
        consumers: &["the runtime-object cache directory"],
        on_mismatch: "the object is recompiled",
        versioned_by: "a hand-bumped constant (`RT_CACHE_VERSION`)",
        frozen: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_covered_change_moves_the_fold_and_order_is_by_path() {
        let dir = std::env::temp_dir().join(format!("hale-graph-identity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("b")).unwrap();
        std::fs::write(dir.join("b/two.rs"), "fn two() {}").unwrap();
        std::fs::write(dir.join("one.hl"), "fn main() {}").unwrap();
        std::fs::write(dir.join("notes.md"), "not a source").unwrap();
        let mut files = Vec::new();
        walk_sources(&dir, &mut files);
        assert_eq!(files.len(), 2, "only source extensions, recursively");
        assert!(
            files[0].ends_with("b/two.rs") && files[1].ends_with("one.hl"),
            "path order"
        );
        let a = fold_files(&dir, &files);
        std::fs::write(dir.join("one.hl"), "fn main() { }").unwrap();
        let b = fold_files(&dir, &files);
        assert_ne!(a, b, "one changed byte moves the identity");
        std::fs::write(dir.join("three.c"), "int x;").unwrap();
        let mut files2 = Vec::new();
        walk_sources(&dir, &mut files2);
        let c = fold_files(&dir, &files2);
        assert_ne!(b, c, "an added file moves the identity");
        assert_eq!(fold_files(&dir, &files2), c, "the fold is deterministic");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
