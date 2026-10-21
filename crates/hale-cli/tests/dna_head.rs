//! `hale dna head`: the head started from an installed toolchain, with no
//! checkout. The toolchain carries what the head is built from
//! (`hale_dna::HEAD_FILES`, held equal to the tree by hale-dna's own unit
//! test) and materializes it into its cache beside the host; the verb
//! builds the four seeds there and runs the materialized launcher.
//!
//! Building the seeds takes minutes, and CI's face jobs build them from
//! the checkout already, so the proof that the embedded tree is complete
//! is the cheap one: every seed `hale check`s clean in a materialized
//! tree that holds nothing the toolchain did not carry. A seed that
//! imports a directory no list embeds fails here.

#[path = "support/scratch.rs"]
mod scratch;
#[path = "support/vault.rs"]
mod vault;
#[path = "support/habitat.rs"]
mod habitat;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

fn scratch(tag: &str) -> scratch::Scratch {
    let d = scratch::Scratch::new(format!("hale_dna_head_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn check(dir: &Path) -> (bool, String) {
    let out = vault::hale().arg("check").arg(dir).output().expect("invoke hale check");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn the_materialized_head_checks_without_a_checkout() {
    let root = scratch("tree");
    hale_iris::materialize_into(&root).expect("materialize the toolchain's tree");
    let launcher = root.join(hale_dna::HEAD_LAUNCHER);
    assert!(launcher.is_file(), "the launcher is materialized at {}", launcher.display());
    for f in hale_dna::HEAD_FILES.iter().filter(|f| f.path.starts_with("dna/face/web/")) {
        assert!(root.join(f.path).is_file(), "{} is served from the materialized tree", f.path);
    }
    let mut failed = Vec::new();
    for s in [&hale_dna::HEAD_API, &hale_dna::HEAD_SERVICE, &hale_dna::HEAD_FACE, &hale_dna::HEAD_OIDC, &hale_dna::HEAD_REFLEXES] {
        let (ok, out) = check(&root.join(s.seed));
        if !ok {
            failed.push(format!("--- {}\n{out}", s.seed));
        }
    }
    assert!(failed.is_empty(), "the head's seeds do not check in the toolchain's materialized tree (an import the embedded lists do not carry?):\n{}", failed.join("\n"));
    // and the check is a proof of the closure: a materialized tree missing
    // one directory a seed imports fails it
    std::fs::remove_file(root.join("dna/oidc/stub.hl")).unwrap();
    let (ok, out) = check(&root.join(hale_dna::HEAD_OIDC.seed));
    assert!(!ok, "a seed whose import is not materialized must not check:\n{out}");
}

#[test]
fn head_help_prints_its_usage() {
    let out = vault::hale().args(["dna", "head", "--help"]).output().expect("invoke hale");
    assert!(out.status.success(), "--help exits 0");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("usage: hale dna head [project]"), "{text}");
    for flag in ["--port", "--head-port", "--api-port", "--oidc-port", "--commands-port", "--source-drafts", "dna/face/start.sh", "hale habitat migrate"] {
        assert!(text.contains(flag), "the usage names {flag}:\n{text}");
    }
    // a flag the verb does not take is refused before anything is built
    let out = vault::hale().args(["dna", "head", "--api", "/bin/true"]).output().expect("invoke hale");
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    // and the verb is on `hale dna`'s own usage
    let out = vault::hale().args(["dna", "--help"]).output().expect("invoke hale");
    assert!(String::from_utf8_lossy(&out.stderr).contains("hale dna head [project]"));
}

// The containers compose labels as `project`'s `service`, by id.
fn service_containers(project: &str, service: &str) -> String {
    let out = Command::new("docker")
        .args(["ps", "-a", "-q", "--filter"])
        .arg(format!("label=com.docker.compose.project={project}"))
        .arg("--filter")
        .arg(format!("label=com.docker.compose.service={service}"))
        .output()
        .expect("docker ps");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// GH #1653: an organism's memory is its allocation in the local habitat:
/// the habitat's compose project's `memory-<id>` service, `<id>` its record
/// identity. A container of another project (an older toolchain's
/// `hale-dna-<seed>` from the checkout) or of another organism's allocation
/// in the same habitat is no evidence of it: with no owner's DSN, `hale dna
/// show` and `hale dna head` say memory is not up and name `hale habitat
/// migrate`, and neither brings the organism's allocation up — no command
/// but the habitat's acts provisions. Needs docker, a local pgvector image
/// and an owner's DSN to make the organism with; without them it says so
/// and exercises nothing.
#[test]
fn a_container_outside_the_allocation_is_no_evidence_of_the_organisms_memory() {
    let owner = std::env::var("HALE_DNA_MEMORY_DSN_OWNER").unwrap_or_default();
    let image = Command::new("docker").args(["image", "inspect", "pgvector/pgvector:pg16"]).output().is_ok_and(|o| o.status.success());
    if owner.is_empty() || !image {
        eprintln!("dna_head: the decoy-container case is not exercised (it needs HALE_DNA_MEMORY_DSN_OWNER and a local pgvector/pgvector:pg16)");
        return;
    }
    let root = scratch("decoy");
    habitat::habitat();
    let made = vault::hale().args(["habitat", "graft", "decoy", "--no-pack"]).current_dir(&*root).env("HALE_DNA_DISCOVER", "off").output().expect("hale habitat graft");
    assert!(made.status.success(), "{}{}", String::from_utf8_lossy(&made.stdout), String::from_utf8_lossy(&made.stderr));
    let app = root.join("decoy").canonicalize().unwrap();
    // the organism's DSN was the owner's; take it away, so its memory is
    // its allocation in the habitat, which nothing has brought up
    std::fs::remove_file(app.join(".hale/dna/memory.env")).unwrap();
    let manifest = std::fs::read_to_string(app.join("hale.toml")).unwrap();
    let id = manifest.lines().find_map(|l| l.trim().strip_prefix("record = \"")).map(|r| r.trim_end_matches('"').to_string()).expect("the record pin");
    let service = format!("memory-{}", &id[..12]);
    // the habitat `vault::hale()` runs under: this test process's own
    let habitat = vault::dir().join("data/hale/habitat/local");
    let project = {
        let h = openssl::sha::sha256(habitat.to_string_lossy().as_bytes());
        format!("hale-habitat-local-{}", h[..4].iter().map(|b| format!("{b:02x}")).collect::<String>())
    };
    let decoy = |name: String, project: &str, service: &str, wd: &Path| {
        let started = Command::new("docker")
            .args(["run", "-d", "--name", &name, "--label"])
            .arg(format!("com.docker.compose.project={project}"))
            .arg("--label")
            .arg(format!("com.docker.compose.service={service}"))
            .arg("--label")
            .arg(format!("com.docker.compose.project.working_dir={}", wd.display()))
            .args(["--entrypoint", "sleep", "pgvector/pgvector:pg16", "600"])
            .output()
            .unwrap();
        assert!(started.status.success(), "{}", String::from_utf8_lossy(&started.stderr));
        name
    };
    let pid = std::process::id();
    let decoys = [
        // an older toolchain's, from the checkout under the seed's name
        decoy(format!("hale-dna-decoy-knowledge-db-{pid}"), "hale-dna-decoy", "knowledge-db", &app.join("dna")),
        // another organism's allocation in this very habitat
        decoy(format!("{project}-memory-000000000000-{pid}"), &project, "memory-000000000000", &habitat),
    ];
    // `show` reads memory as the head: the organism's is not up
    let show = vault::hale().args(["dna", "show", "org"]).arg(&app).env_remove("HALE_DNA_MEMORY_DSN_OWNER").env("HALE_DNA_DISCOVER", "off").output().unwrap();
    let said = format!("{}{}", String::from_utf8_lossy(&show.stdout), String::from_utf8_lossy(&show.stderr));
    assert!(service_containers(&project, &service).is_empty(), "show brought the organism's memory up: {said}");
    assert!(!app.join(".hale/dna/memory.env").exists(), "and kept no DSN: {said}");
    // `head` says memory is not up and names the habitat's act
    let mut head = vault::hale()
        .args(["dna", "head"])
        .arg(&app)
        .env_remove("HALE_DNA_MEMORY_DSN_OWNER")
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut said = String::new();
    for line in BufReader::new(head.stderr.take().unwrap()).lines() {
        let Ok(line) = line else { break };
        said.push_str(&line);
        said.push('\n');
        if line.contains("hale habitat migrate") || line.contains("reads Knowledge from memory") {
            break;
        }
    }
    let _ = head.kill();
    let _ = head.wait();
    for name in &decoys {
        let _ = Command::new("docker").args(["rm", "-f", name]).output();
    }
    assert!(said.contains("hale dna head: memory is not up") && said.contains("hale habitat migrate"), "head took a decoy for the organism's memory:\n{said}");
    assert!(service_containers(&project, &service).is_empty(), "head brought the organism's memory up:\n{said}");
}
