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
    "editing",
    "journal",
    "knowledge",
    "models",
    "org",
    "performers",
    "process",
    "review",
    "topics",
    "types",
    "verification",
    "work_system",
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
    }
}
