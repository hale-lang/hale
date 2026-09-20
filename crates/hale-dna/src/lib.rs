//! The DNA core, embedded (GH #521 / #527 B6 / #528).
//!
//! `dna/core` is compiler-versioned Hale source the toolchain carries
//! the way `hale-stdlib` carries the stdlib. Today one consumer
//! materializes it: `hale iris`, whose observer imports the core's
//! typed control topics (`dna.review.verdict`, `dna.intent.offered`)
//! so a verdict or an intent published from the browser is the SAME
//! declaration the organism binds. Track C (`hale dna init` /
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
    "budget",
    "decision",
    "editing",
    "exchange",
    "forge",
    "handoff",
    "infrastructure",
    "journal",
    "knowledge",
    "models",
    "org",
    "ownership",
    "performers",
    "principal",
    "process",
    "record",
    "review",
    "routing",
    "tape",
    "topics",
    "types",
    "verification",
    "work_system",
    "workflow_definition",
    "workflow_events",
    "workflow_execution",
    "workflow_projection",
    "workspace",
];

/// The seed path, relative to the materialization root.
pub const CORE_SEED: &str = "dna/core";

/// The membrane client (`dna/membrane`): publishes one typed fact on
/// the organism's control topics and exits. `hale dna ask` builds
/// and execs it from the toolchain cache beside the core it imports.
pub const MEMBRANE_CLIENT: EmbeddedFile = EmbeddedFile {
    path: "dna/membrane/main.hl",
    content: include_str!("../../../dna/membrane/main.hl"),
};
pub const MEMBRANE_SEED: &str = "dna/membrane";
pub const MEMBRANE_BIN: &str = "dna/membrane/membrane";

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
/// person's name, the membrane relay, the supervision — as a Hale
/// program `hale dna` builds once into the toolchain cache and execs
/// with the project resolved.
pub const HOST_FILES: &[EmbeddedFile] = host!["connections", "forge_github", "genome", "host", "infra", "main", "node", "procs", "projection", "record", "record_verbs", "verbs", "writers"];
pub const HOST_SEED: &str = "dna/host";
pub const HOST_BIN: &str = "dna/host/host";

macro_rules! at {
    ($($path:literal),* $(,)?) => {
        &[$(EmbeddedFile {
            path: $path,
            content: include_str!(concat!("../../../", $path)),
        }),*]
    };
}

/// The knowledge graph as a service (GH #583 K1): the store library
/// (`dna/knowledge`: the `KnowledgeStore` interface, `Mem`, `Pq`, the
/// record's tail), the service program (`dna/knowledge/service`), and
/// pond's Postgres driver pinned beside them (`dna/pond/{db,pq}`).
pub const KNOWLEDGE_FILES: &[EmbeddedFile] = at![
    "dna/knowledge/embed.hl",
    "dna/knowledge/store.hl",
    "dna/knowledge/protected.hl",
    "dna/knowledge/tail.hl",
    "dna/knowledge/ledger.hl",
    "dna/knowledge/service/main.hl",
    "dna/pond/db/args.hl",
    "dna/pond/db/db.hl",
    "dna/pond/db/types.hl",
    "dna/pond/pq/pool.hl",
    "dna/pond/pq/pq.hl",
    "dna/pond/pq/scram.hl",
    "dna/pond/pq/stream.hl",
    "dna/pond/pq/wire.hl",
];
pub const KNOWLEDGE_SEED: &str = "dna/knowledge/service";
pub const KNOWLEDGE_BIN: &str = "dna/knowledge/service/service";

/// Every embedded file as a `(path, content)` pair: the core, the
/// host, the membrane client, the surface and the knowledge set —
/// the whole of what `EMBEDDED_DIGEST` names.
pub fn embedded_pairs() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for f in FILES.iter().chain(HOST_FILES).chain(KNOWLEDGE_FILES) {
        out.push((f.path.to_string(), f.content.to_string()));
    }
    for f in [&MEMBRANE_CLIENT, &UI_MAIN, &UI_HTML] {
        out.push((f.path.to_string(), f.content.to_string()));
    }
    out
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
        assert!(MEMBRANE_CLIENT.content.contains("main locus Client"));
        assert!(UI_MAIN.content.contains("std::http::Server") && UI_HTML.content.contains("<title>hale dna</title>"));
        let host_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dna/host");
        let mut host_on_disk: Vec<String> = std::fs::read_dir(&host_dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".hl")).collect();
        host_on_disk.sort();
        let mut host_embedded: Vec<String> = HOST_FILES.iter().map(|f| f.path.rsplit('/').next().unwrap().to_string()).collect();
        host_embedded.sort();
        assert_eq!(host_embedded, host_on_disk, "a dna/host file was added or removed without updating hale-dna");
        // the knowledge set: every .hl under dna/knowledge and dna/pond
        let mut know_on_disk: Vec<String> = Vec::new();
        for d in ["dna/knowledge", "dna/knowledge/service", "dna/pond/db", "dna/pond/pq"] {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(d);
            for e in std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()) {
                let n = e.file_name().to_string_lossy().to_string();
                if n.ends_with(".hl") {
                    know_on_disk.push(format!("{d}/{n}"));
                }
            }
        }
        know_on_disk.sort();
        let mut know_embedded: Vec<String> = KNOWLEDGE_FILES.iter().map(|f| f.path.to_string()).collect();
        know_embedded.sort();
        assert_eq!(know_embedded, know_on_disk, "a dna/knowledge or dna/pond file was added or removed without updating hale-dna");
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
