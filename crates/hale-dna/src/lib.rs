//! The DNA core, embedded (GH #521 / #527 B6 / #528).
//!
//! `dna/core` is compiler-versioned Hale source the toolchain carries
//! the way `hale-stdlib` carries the stdlib. Today one consumer
//! materializes it: `hale iris`, whose observer imports the core's
//! typed control topics (`dna.review.verdict`, `dna.intent.offered`)
//! so a verdict or an intent published from the browser is the SAME
//! declaration the organism binds. Track C (`hale habitat graft` /
//! `upgrade`) materializes the same set into a project's `vendor/dna`.
//!
//! Paths are relative to a materialization root and keep the repo's
//! layout (`dna/core/<file>.hl`), so `import "../core"` from a sibling
//! seed resolves unchanged.
//!
//! What a binary embeds has a NAME (GH #726): `EMBEDDED_DIGEST`, a
//! framed SHA-256 over the sorted `(path, content)` pairs of every
//! file below, computed by `build.rs` over the tree it was built
//! from. Two binaries reporting one `hale --version` can carry
//! different source; the digest is what tells them apart.

/// The digest of the embedded source set, and the algorithm the
/// build script shares with the crate (see the file's own notes).
pub mod digest;

pub use digest::{digest_of_pairs, digest_of_tree, EMBEDDED_DIRS};

/// The DNA source this binary embeds, as one 64-hex SHA-256 over the
/// sorted, length-framed `(path, content)` pairs of every file in
/// `embedded_pairs()`. Computed by `build.rs` from the working tree
/// it was built from; `hale dna --embedded-digest` prints it, and
/// `--from-tree <dir>` prints another tree's for comparison.
pub const EMBEDDED_DIGEST: &str = env!("HALE_DNA_EMBEDDED_DIGEST");

/// The first 16 hex digits of `EMBEDDED_DIGEST` — enough to tell two
/// builds apart at a glance (`hale --version`, `hale dna status`).
pub fn embedded_short() -> &'static str {
    &EMBEDDED_DIGEST[..16]
}

pub struct EmbeddedFile {
    pub path: &'static str,
    pub content: &'static str,
}

macro_rules! core {
    ($($name:literal),* $(,)?) => {
        &[$(EmbeddedFile {
            path: concat!("dna/core/", $name, ".hl"),
            content: include_str!(concat!("../../../dna/core/", $name, ".hl")),
        }),*]
    };
}

/// Every file of `dna/core`, in the order the repo lists them.
pub const FILES: &[EmbeddedFile] = core![
    "assembly",
    "attempts",
    "batch",
    "budget",
    "cases",
    "codebase_reviews",
    "codebases",
    "completion",
    "concerns",
    "contracts",
    "correlation",
    "decision",
    "declared_secrets",
    "declared_uis",
    "editing",
    "evidence",
    "exchange",
    "face",
    "forge",
    "forgejo",
    "handoff",
    "hat",
    "infrastructure",
    "journal",
    "knowledge",
    "knowledge_binding",
    "knowledge_edge_review",
    "knowledge_node_request",
    "knowledge_record",
    "memory_embed",
    "memory_exchange",
    "memory_ledger",
    "memory_protected",
    "memory_record",
    "memory_schema",
    "memory_spine",
    "memory_store",
    "model_map",
    "models",
    "native_json",
    "nerves",
    "org",
    "org_source",
    "organization_launch",
    "organization_source_request",
    "ownership",
    "performers",
    "placement",
    "positions",
    "practices",
    "principal",
    "record",
    "record_rows",
    "scheduler",
    "review",
    "reviews",
    "roles",
    "route",
    "routing",
    "senses",
    "source_check",
    "spending",
    "ssh_provider",
    "steps",
    "tape",
    "teardown",
    "tools",
    "topics",
    "types",
    "verification",
    "work_system",
    "workflow_definition",
    "workflow_events",
    "workflow_execution",
    "workflow_projection",
    "workflow_runtime",
    "workspace",
];

/// The seed path, relative to the materialization root.
pub const CORE_SEED: &str = "dna/core";

/// The DNA surface (`dna/ui`, GH #566 F6): an HTTP server over the
/// offline verbs of `hale dna`, from the record alone. `hale dna ui`
/// builds and execs it from the toolchain cache.
pub const UI_MAIN: EmbeddedFile = EmbeddedFile {
    path: "dna/ui/main.hl",
    content: include_str!("../../../dna/ui/main.hl"),
};
pub const UI_HTML: EmbeddedFile = EmbeddedFile {
    path: "dna/ui/index.html",
    content: include_str!("../../../dna/ui/index.html"),
};
pub const UI_SEED: &str = "dna/ui";
pub const UI_BIN: &str = "dna/ui/ui";

macro_rules! host {
    ($($name:literal),* $(,)?) => {
        &[$(EmbeddedFile {
            path: concat!("dna/host/", $name, ".hl"),
            content: include_str!(concat!("../../../dna/host/", $name, ".hl")),
        }),*]
    };
}

/// The host (`dna/host`, GH #566 F8): what `hale dna` does beside the
/// compiler — the projections, the record's sync and appends in a
/// person's name, the relay onto the nerves, the supervision — as Hale
/// programs `hale dna` builds once into the toolchain cache and execs
/// with the project resolved. This seed is the host's body (`run`,
/// `dev`, `node`, `body-fence`: the `main locus` and the renewer beside
/// it), and the library the verb program imports.
pub const HOST_FILES: &[EmbeddedFile] = host!["connections", "forge_github", "forgejo_habitat", "forget", "genome", "holes", "host", "infra", "main", "memory", "nerves", "node", "organization_launch", "placement", "procs", "projection", "pulse", "record", "reflex", "record_verbs", "secrets", "show", "verbs", "writers"];
pub const HOST_SEED: &str = "dna/host";
pub const HOST_BIN: &str = "dna/host/host";

/// The host's verb program (`dna/host/verb`, GH #1490): every one-shot
/// verb, one per process, over the host's seed imported. It starts no
/// locus that runs or handles the bus, so it runs to exit.
pub const HOST_VERB_FILES: &[EmbeddedFile] = &[EmbeddedFile {
    path: "dna/host/verb/main.hl",
    content: include_str!("../../../dna/host/verb/main.hl"),
}];
pub const HOST_VERB_SEED: &str = "dna/host/verb";
pub const HOST_VERB_BIN: &str = "dna/host/verb/verb";

/// The verbs the host's body runs: each starts a process that runs until
/// it is stopped. Every other host verb is the verb program's.
pub const HOST_BODY_VERBS: [&str; 4] = ["run", "dev", "node", "body-fence"];

/// The seed and the binary that answer `verb`: the body for
/// [`HOST_BODY_VERBS`], the verb program for every other.
pub fn host_program_for(verb: &str) -> (&'static str, &'static str) {
    if HOST_BODY_VERBS.contains(&verb) {
        (HOST_SEED, HOST_BIN)
    } else {
        (HOST_VERB_SEED, HOST_VERB_BIN)
    }
}

macro_rules! at {
    ($($path:literal),* $(,)?) => {
        &[$(EmbeddedFile {
            path: $path,
            content: include_str!(concat!("../../../", $path)),
        }),*]
    };
}

/// The toolchain's nursery (`dna/nursery`): its index, the packs it lists,
/// which `hale dna pack` offers when no `--from` names another, and the
/// trees of the saplings it carries itself (`saplings/<name>`, a sapling
/// whose repository is `toolchain:saplings/<name>`, with no remote). Not
/// part of the embedded DNA source set (`EMBEDDED_DIRS`): a pack is pinned
/// by its own digest when it is applied.
pub const NURSERY_FILES: &[EmbeddedFile] = at![
    "dna/nursery/nursery.toml",
    "dna/nursery/packs/content-team.pack",
    "dna/nursery/packs/dev-team.pack",
    "dna/nursery/packs/founder.pack",
    "dna/nursery/packs/hale-organism.pack",
    "dna/nursery/packs/habitat.pack",
    "dna/nursery/packs/small-startup.pack",
    "dna/nursery/packs/standard.pack",
    "dna/nursery/saplings/hale/README.md",
    "dna/nursery/saplings/hale/hale.toml",
    "dna/nursery/saplings/hale/main.hl",
];

/// GH #1653: the habitat's genome, which ships with the toolchain as the
/// foundational law and the nursery do: what `hale habitat up` births the
/// habitat from, the one organism that may be born without a parent. Its
/// positions are the nursery's `habitat` pack, and its law the `standard`
/// pack and the `law` items its document marks.
pub const HABITAT_FILES: &[EmbeddedFile] = at![
    "dna/habitat/README.md",
    "dna/habitat/ui/alerts.js",
    "dna/habitat/ui/hardware.js",
    "dna/habitat/ui/infrastructure.js",
    "dna/habitat/ui/organisms.js",
    "dna/habitat/ui/head-client.js",
    "dna/habitat/ui/host-label.js",
    "dna/habitat/ui/index.html",
    "dna/habitat/ui/operator.css",
    "dna/habitat/ui/operator.js",
];

/// Typed query operations shared by the host and service API. These must
/// travel with the host: a source-tree import alone would break the installed CLI.
pub const OPERATION_FILES: &[EmbeddedFile] = at![
    "dna/operations/attempt_command.hl",
    "dna/operations/context.hl",
    "dna/operations/definition_drafts.hl",
    "dna/operations/definitions.hl",
    "dna/operations/governance_admission.hl",
    "dna/operations/governance_command.hl",
    "dna/operations/governance_policy.hl",
    "dna/operations/governance_projection.hl",
    "dna/operations/graph.hl",
    "dna/operations/graph_holes.hl",
    "dna/operations/graph_ingest.hl",
    "dna/operations/graph_mandates.hl",
    "dna/operations/graph_show.hl",
    "dna/operations/knowledge_admission.hl",
    "dna/operations/knowledge_binding_admission.hl",
    "dna/operations/knowledge_binding_command.hl",
    "dna/operations/knowledge_binding_projection.hl",
    "dna/operations/knowledge_command.hl",
    "dna/operations/knowledge_edge_admission.hl",
    "dna/operations/knowledge_edge_command.hl",
    "dna/operations/knowledge_edge_projection.hl",
    "dna/operations/knowledge_node_admission.hl",
    "dna/operations/knowledge_node_command.hl",
    "dna/operations/knowledge_node_projection.hl",
    "dna/operations/knowledge_policy.hl",
    "dna/operations/memory_tail.hl",
    "dna/operations/organization.hl",
    "dna/operations/organization_admission.hl",
    "dna/operations/organization_application.hl",
    "dna/operations/organization_command.hl",
    "dna/operations/organization_impact.hl",
    "dna/operations/organization_json.hl",
    "dna/operations/organization_policy.hl",
    "dna/operations/organization_projection.hl",
    "dna/operations/organization_review_command.hl",
    "dna/operations/person_retirement.hl",
    "dna/operations/queries.hl",
    "dna/operations/structure_command.hl",
    "dna/operations/task_command.hl",
    "dna/operations/task_create_command.hl",
    "dna/operations/task_policy.hl",
    "dna/operations/task_projection.hl",
    "dna/operations/usage.hl",
];

/// The organization's runtime and source seeds (GH #690): what the host
/// and the service API import beside the core to launch an organization
/// process under its authority (`dna/organization_runtime`) and to read,
/// guard and publish its declared source (`dna/organization_source`).
/// They travel with the host for the same reason `dna/operations` does.
pub const ORGANIZATION_FILES: &[EmbeddedFile] = at![
    "dna/organization_runtime/authority.hl",
    "dna/organization_runtime/launch.hl",
    "dna/organization_runtime/process.hl",
    "dna/organization_source/organization_dependencies.hl",
    "dna/organization_source/organization_source.hl",
    "dna/organization_source/publication_guard.hl",
];

/// pond's Postgres driver (`dna/core/pond/{db,pq}`), which the core imports
/// to open memory as a role (GH #985), and pond's NATS client and bus
/// adapter (`dna/core/pond/realtime/nats`), the nerves (GH #986);
/// vendored under `vendor/dna/pond`.
pub const POND_FILES: &[EmbeddedFile] = at![
    "dna/core/pond/db/args.hl",
    "dna/core/pond/db/db.hl",
    "dna/core/pond/db/types.hl",
    "dna/core/pond/pq/pool.hl",
    "dna/core/pond/pq/pq.hl",
    "dna/core/pond/pq/scram.hl",
    "dna/core/pond/pq/stream.hl",
    "dna/core/pond/pq/wire.hl",
    "dna/core/pond/realtime/nats/adapter.hl",
    "dna/core/pond/realtime/nats/client.hl",
    "dna/core/pond/realtime/nats/fake.hl",
    "dna/core/pond/realtime/nats/jetstream.hl",
    "dna/core/pond/realtime/nats/log.hl",
    "dna/core/pond/realtime/nats/proto.hl",
    "dna/core/pond/realtime/nats/types.hl",
];

/// The legs seed (`dna/core/legs`, GH #946 slice 4): a leg's verbs as
/// API clients, the renderer, the performers and the hands, which a
/// project's `dna/org/work.hl` imports as `vendor/dna/legs`; vendored
/// beside the core it imports as `..`.
pub const LEGS_FILES: &[EmbeddedFile] = at![
    "dna/core/legs/client.hl",
    "dna/core/legs/codebase.hl",
    "dna/core/legs/hands.hl",
    "dna/core/legs/head_commands/client.hl",
    "dna/core/legs/model.hl",
    "dna/core/legs/performer.hl",
    "dna/core/legs/render.hl",
    "dna/core/legs/tool_loop.hl",
    "dna/core/legs/verbs.hl",
];

/// The head (`hale dna head`): what `dna/face/start.sh` builds and serves,
/// so a person with an installed toolchain and no checkout can start the
/// face. The five seeds the launcher builds — the per-project API
/// (`dna/api/practice_review`), the project service (`dna/api/project_service`),
/// the standard face (`dna/face/standard`),
/// the stub OpenID provider (`dna/oidc/serve`) and the reflexes
/// (`dna/reflexes`) — with the seeds they import that no list above
/// carries (`dna/api`, `dna/oidc`), the browser's assets (`dna/face/web`),
/// and the launcher itself, which `hale dna head` runs from the
/// materialized tree.
///
/// NOT part of `embedded_files()`, so not of `EMBEDDED_DIGEST`: that
/// digest names what `vendor/dna` is materialized from and is persisted
/// in users' trees, and the head is never vendored. The toolchain
/// cache's key (`hale_iris::toolchain_hash`) covers these files, so a
/// changed head is rebuilt.
pub const HEAD_FILES: &[EmbeddedFile] = at![
    "dna/api/bearer.hl",
    "dna/api/commands.hl",
    "dna/api/context.hl",
    "dna/api/definition_drafts.hl",
    "dna/api/habitat_brief.hl",
    "dna/api/habitat_hosts.hl",
    "dna/api/habitat_organisms.hl",
    "dna/api/habitat_placed.hl",
    "dna/api/habitat_placement.hl",
    "dna/api/habitat_rows.hl",
    "dna/api/http.hl",
    "dna/api/knowledge_commands.hl",
    "dna/api/knowledge.hl",
    "dna/api/knowledge_memory.hl",
    "dna/api/main.hl",
    "dna/api/organization_candidate.hl",
    "dna/api/organization_drafts.hl",
    "dna/api/organization_impact.hl",
    "dna/api/organization_status.hl",
    "dna/api/people.hl",
    "dna/api/record_index.hl",
    "dna/api/solo_tasks.hl",
    "dna/api/source.hl",
    "dna/api/tasks.hl",
    "dna/api/upstream.hl",
    "dna/api/web.hl",
    "dna/api/workflows.hl",
    "dna/api/practice_review/main.hl",
    "dna/api/practice_review/provider.hl",
    "dna/api/project_service/children.hl",
    "dna/api/project_service/head.hl",
    "dna/api/project_service/journal.hl",
    "dna/api/project_service/main.hl",
    "dna/api/project_service/operations.hl",
    "dna/api/project_service/operations_s3.hl",
    "dna/api/project_service/policy.hl",
    "dna/api/project_service/proxy.hl",
    "dna/api/project_service/push.hl",
    "dna/api/project_service/registry.hl",
    "dna/api/project_service/types.hl",
    "dna/oidc/stub.hl",
    "dna/oidc/serve/main.hl",
    "dna/reflexes/main.hl",
    "dna/face/standard/index.hl",
    "dna/face/standard/main.hl",
    "dna/face/web/app.js",
    "dna/face/web/application.js",
    "dna/face/web/definition-draft.js",
    "dna/face/web/index.html",
    "dna/face/web/knowledge-draft.js",
    "dna/face/web/organization-draft.js",
    "dna/face/web/projects.js",
    "dna/face/web/styles.css",
    "dna/face/web/task-administration.js",
    "dna/face/web/task-create.js",
    "dna/face/start.sh",
];

/// The directories `HEAD_FILES` takes whole, listed non-recursively, with
/// the extensions each contributes (`*` for every file): the unit test
/// holds the list equal to what these directories hold, so a file added
/// to one cannot be left out of the head an installed toolchain builds.
pub const HEAD_DIRS: &[(&str, &[&str])] = &[
    ("dna/api", &["hl"]),
    ("dna/api/practice_review", &["hl"]),
    ("dna/api/project_service", &["hl"]),
    ("dna/oidc", &["hl"]),
    ("dna/oidc/serve", &["hl"]),
    ("dna/reflexes", &["hl"]),
    ("dna/face/standard", &["hl"]),
    ("dna/face/web", &["*"]),
];

/// The launcher `hale dna head` runs from the materialized tree.
pub const HEAD_LAUNCHER: &str = "dna/face/start.sh";

/// One seed the head is built from: its path and the binary `hale build`
/// makes of it, both relative to the materialization root.
pub struct HeadSeed {
    pub seed: &'static str,
    pub bin: &'static str,
}

/// The per-project native API the head starts as its child.
pub const HEAD_API: HeadSeed = HeadSeed { seed: "dna/api/practice_review", bin: "dna/api/practice_review/practice_review" };
/// The project service: the head itself.
pub const HEAD_SERVICE: HeadSeed = HeadSeed { seed: "dna/api/project_service", bin: "dna/api/project_service/project_service" };
/// The standard face: the application that serves the browser's shell and
/// the project's `ui/` in front of the head, which serves surfaces only.
pub const HEAD_FACE: HeadSeed = HeadSeed { seed: "dna/face/standard", bin: "dna/face/standard/standard" };
/// The stub OpenID provider the head signs a person in through (GH #989).
pub const HEAD_OIDC: HeadSeed = HeadSeed { seed: "dna/oidc/serve", bin: "dna/oidc/serve/serve" };
/// The reflexes, started beside an attached project (GH #988).
pub const HEAD_REFLEXES: HeadSeed = HeadSeed { seed: "dna/reflexes", bin: "dna/reflexes/reflexes" };

/// Every embedded file: the core, the host, the surface, pond's
/// copies, the legs and the UI — the whole of what `EMBEDDED_DIGEST`
/// names (the head's files, `HEAD_FILES`, are not among them).
pub fn embedded_files() -> impl Iterator<Item = &'static EmbeddedFile> {
    FILES
        .iter()
        .chain(HOST_FILES)
        .chain(HOST_VERB_FILES)
        .chain(OPERATION_FILES)
        .chain(ORGANIZATION_FILES)
        .chain(POND_FILES)
        .chain(LEGS_FILES)
        .chain([&UI_MAIN, &UI_HTML])
}

/// Every embedded file as a `(path, content)` pair.
pub fn embedded_pairs() -> Vec<(String, String)> {
    embedded_files().map(|f| (f.path.to_string(), f.content.to_string())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_set_is_the_repo_directory() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dna/core");
        let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".hl"))
            .collect();
        on_disk.sort();
        let mut embedded: Vec<String> = FILES
            .iter()
            .map(|f| f.path.rsplit('/').next().unwrap().to_string())
            .collect();
        embedded.sort();
        assert_eq!(embedded, on_disk, "a dna/core file was added or removed without updating hale-dna");
        for f in FILES {
            assert!(!f.content.is_empty(), "{} is empty", f.path);
        }
        assert!(UI_MAIN.content.contains("std::http::Server") && UI_HTML.content.contains("<title>hale dna</title>"));
        let host_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dna/host");
        let mut host_on_disk: Vec<String> = std::fs::read_dir(&host_dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".hl")).collect();
        host_on_disk.sort();
        let mut host_embedded: Vec<String> = HOST_FILES.iter().map(|f| f.path.rsplit('/').next().unwrap().to_string()).collect();
        host_embedded.sort();
        assert_eq!(host_embedded, host_on_disk, "a dna/host file was added or removed without updating hale-dna");
        let verb_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(HOST_VERB_SEED);
        let mut verb_on_disk: Vec<String> = std::fs::read_dir(&verb_dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".hl")).map(|n| format!("{HOST_VERB_SEED}/{n}")).collect();
        verb_on_disk.sort();
        let mut verb_embedded: Vec<String> = HOST_VERB_FILES.iter().map(|f| f.path.to_string()).collect();
        verb_embedded.sort();
        assert_eq!(verb_embedded, verb_on_disk, "a dna/host/verb file was added or removed without updating hale-dna");
        for (seed, bin) in [(HOST_SEED, HOST_BIN), (HOST_VERB_SEED, HOST_VERB_BIN)] {
            let name = seed.rsplit('/').next().unwrap();
            assert_eq!(bin, format!("{seed}/{name}"), "`hale build` names a seed's binary after its directory");
        }
        let operations_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dna/operations");
        let mut operations_on_disk: Vec<String> = std::fs::read_dir(&operations_dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".hl")).collect();
        operations_on_disk.sort();
        let mut operations_embedded: Vec<String> = OPERATION_FILES.iter().map(|f| f.path.rsplit('/').next().unwrap().to_string()).collect();
        operations_embedded.sort();
        assert_eq!(operations_embedded, operations_on_disk, "a dna/operations file was added or removed without updating hale-dna");
        let mut org_on_disk: Vec<String> = Vec::new();
        for d in ["dna/organization_runtime", "dna/organization_source"] {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(d);
            for e in std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()) {
                let n = e.file_name().to_string_lossy().to_string();
                if n.ends_with(".hl") {
                    org_on_disk.push(format!("{d}/{n}"));
                }
            }
        }
        org_on_disk.sort();
        let mut org_embedded: Vec<String> = ORGANIZATION_FILES.iter().map(|f| f.path.to_string()).collect();
        org_embedded.sort();
        assert_eq!(org_embedded, org_on_disk, "a dna/organization_runtime or dna/organization_source file was added or removed without updating hale-dna");
        // pond's copies: every .hl under dna/core/pond/{db,pq,realtime/nats}
        let mut know_on_disk: Vec<String> = Vec::new();
        for d in ["dna/core/pond/db", "dna/core/pond/pq", "dna/core/pond/realtime/nats"] {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(d);
            for e in std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()) {
                let n = e.file_name().to_string_lossy().to_string();
                if n.ends_with(".hl") {
                    know_on_disk.push(format!("{d}/{n}"));
                }
            }
        }
        know_on_disk.sort();
        let mut know_embedded: Vec<String> = POND_FILES.iter().map(|f| f.path.to_string()).collect();
        know_embedded.sort();
        assert_eq!(know_embedded, know_on_disk, "a dna/core/pond file was added or removed without updating hale-dna");
        // the legs: every .hl under dna/core/legs
        let legs_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dna/core/legs");
        let mut legs_on_disk: Vec<String> = std::fs::read_dir(&legs_dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".hl")).map(|n| format!("dna/core/legs/{n}")).collect();
        // and the generated client of the head's surface in its own directory
        let head_commands_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dna/core/legs/head_commands");
        legs_on_disk.extend(std::fs::read_dir(&head_commands_dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".hl")).map(|n| format!("dna/core/legs/head_commands/{n}")));
        legs_on_disk.sort();
        let mut legs_embedded: Vec<String> = LEGS_FILES.iter().map(|f| f.path.to_string()).collect();
        legs_embedded.sort();
        assert_eq!(legs_embedded, legs_on_disk, "a dna/core/legs file was added or removed without updating hale-dna");
    }

    /// GH #1490: the CLI execs the body for `HOST_BODY_VERBS` and the verb
    /// program for every other verb; the body's `main` dispatches exactly
    /// those, and the verb program refuses each of them.
    #[test]
    fn a_host_verb_runs_in_its_program() {
        let body = HOST_FILES.iter().find(|f| f.path == "dna/host/main.hl").unwrap().content;
        let verbs = HOST_VERB_FILES[0].content;
        for v in HOST_BODY_VERBS {
            assert_eq!(host_program_for(v), (HOST_SEED, HOST_BIN));
            let spelled = format!("verb == \"{v}\"");
            assert!(body.contains(&spelled), "the body's main dispatches `{v}`");
            assert!(verbs.contains(&spelled), "the verb program refuses `{v}`");
        }
        for v in ["status", "review", "apply", "sync", "memory-migrate", "record-seed"] {
            assert_eq!(host_program_for(v), (HOST_VERB_SEED, HOST_VERB_BIN));
        }
    }

    /// `hale dna head` builds the head from `HEAD_FILES` alone: every file
    /// of the directories it takes (`HEAD_DIRS`) is listed, and every
    /// listed file is in one of them or is the launcher. A seed that
    /// imports a directory no list carries fails the CLI's check of the
    /// materialized tree instead (hale-cli's `dna_head.rs`).
    /// A habitat born from an installed toolchain has only the files
    /// `HABITAT_FILES` lists: every file of its operator face
    /// (`dna/habitat/ui`) is listed, so a screen's script cannot ship in the
    /// checkout and be missing from every installed habitat (alerts.js was).
    #[test]
    fn the_habitat_set_is_its_readme_and_its_ui() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut on_disk: Vec<String> = vec!["dna/habitat/README.md".to_string()];
        for e in std::fs::read_dir(root.join("dna/habitat/ui")).unwrap().filter_map(|e| e.ok()) {
            if e.path().is_file() {
                on_disk.push(format!("dna/habitat/ui/{}", e.file_name().to_string_lossy()));
            }
        }
        on_disk.sort();
        let mut listed: Vec<String> = HABITAT_FILES.iter().map(|f| f.path.to_string()).collect();
        listed.sort();
        assert_eq!(listed, on_disk, "a file of the habitat's operator face (dna/habitat/ui) was added or removed without updating HABITAT_FILES in hale-dna");
    }

    #[test]
    fn the_head_set_is_the_repo_directories() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut on_disk: Vec<String> = vec![HEAD_LAUNCHER.to_string()];
        for (dir, exts) in HEAD_DIRS {
            for e in std::fs::read_dir(root.join(dir)).unwrap_or_else(|e| panic!("{dir}: {e}")).filter_map(|e| e.ok()) {
                let p = e.path();
                if !p.is_file() {
                    continue;
                }
                let name = e.file_name().to_string_lossy().to_string();
                let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
                if exts.contains(&"*") || exts.contains(&ext) {
                    on_disk.push(format!("{dir}/{name}"));
                }
            }
        }
        on_disk.sort();
        let mut listed: Vec<String> = HEAD_FILES.iter().map(|f| f.path.to_string()).collect();
        listed.sort();
        assert_eq!(listed, on_disk, "a file of the head (HEAD_DIRS, or the launcher) was added or removed without updating HEAD_FILES in hale-dna");
        for f in HEAD_FILES {
            assert!(!f.content.is_empty(), "{} is empty", f.path);
            assert!(embedded_files().all(|e| e.path != f.path), "{} is in the digested set too", f.path);
        }
        for s in [&HEAD_API, &HEAD_SERVICE, &HEAD_OIDC, &HEAD_REFLEXES] {
            let dir = format!("{}/", s.seed);
            assert!(HEAD_FILES.iter().any(|f| f.path.starts_with(&dir)), "{} has no source in HEAD_FILES", s.seed);
            let name = s.seed.rsplit('/').next().unwrap();
            assert_eq!(s.bin, format!("{}/{name}", s.seed), "`hale build` names a seed's binary after its directory");
        }
        // the launcher refuses to start without every asset it serves;
        // each one it names is embedded
        let launcher = HEAD_FILES.iter().find(|f| f.path == HEAD_LAUNCHER).unwrap().content;
        for f in HEAD_FILES.iter().filter(|f| f.path.starts_with("dna/face/web/")) {
            let name = f.path.rsplit('/').next().unwrap();
            assert!(launcher.contains(name), "start.sh does not name the asset {name}");
        }
    }

    /// GH #726: the build's snapshot is coherent. `build.rs` digested
    /// the on-disk `dna/` tree; `embedded_pairs()` is what the
    /// compiler actually included. They are the same source set or
    /// this binary cannot say what it carries — a file added without
    /// updating the lists above, or source edited *while* the build
    /// ran, fails here rather than shipping an unverifiable digest.
    #[test]
    fn the_digest_names_exactly_what_is_embedded() {
        assert_eq!(EMBEDDED_DIGEST.len(), 64, "a SHA-256 in lower hex: {EMBEDDED_DIGEST}");
        assert!(EMBEDDED_DIGEST.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "lower hex: {EMBEDDED_DIGEST}");
        assert_eq!(digest_of_pairs(&embedded_pairs()), EMBEDDED_DIGEST, "the compiled-in set and the tree build.rs digested are the same source");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert_eq!(digest_of_tree(&root).expect("digest the repository's dna/ tree"), EMBEDDED_DIGEST, "the working tree this test runs against is the one the binary embeds");
        assert_eq!(embedded_short(), &EMBEDDED_DIGEST[..16]);
    }

    /// The digest is a digest: one byte of content, one path, one
    /// file more or fewer changes it, and order does not.
    #[test]
    fn the_digest_follows_content_paths_and_membership() {
        let p = |a: &str, b: &str| (a.to_string(), b.to_string());
        let base = vec![p("dna/core/a.hl", "locus A { }\n"), p("dna/core/b.hl", "locus B { }\n")];
        let edited = vec![p("dna/core/a.hl", "locus A { }\n"), p("dna/core/b.hl", "locus B { }\n// one comment\n")];
        let renamed = vec![p("dna/core/a.hl", "locus A { }\n"), p("dna/core/c.hl", "locus B { }\n")];
        let added = vec![p("dna/core/a.hl", "locus A { }\n"), p("dna/core/b.hl", "locus B { }\n"), p("dna/core/c.hl", "")];
        let reordered = vec![p("dna/core/b.hl", "locus B { }\n"), p("dna/core/a.hl", "locus A { }\n")];
        let d = digest_of_pairs(&base);
        assert_ne!(d, digest_of_pairs(&edited), "a file's content is in the digest");
        assert_ne!(d, digest_of_pairs(&renamed), "a file's path is in the digest");
        assert_ne!(d, digest_of_pairs(&added), "an added file is in the digest");
        assert_eq!(d, digest_of_pairs(&reordered), "the order the pairs are collected in is not");
        // and the framing is not defeated by moving bytes across the
        // boundary between a path and its content
        assert_ne!(digest_of_pairs(&vec![p("dna/core/ab.hl", "x")]), digest_of_pairs(&vec![p("dna/core/a", "b.hlx")]));
    }
}
