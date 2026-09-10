//! GH #566 F5 — the fleet is the expression. Two nodes, each a clone of
//! the governed repository, run one instance of the API each from the
//! record (`hale node`); the host beside the organization deploys an
//! approved candidate to them (`fleet.deploy`), watches the window over
//! both instances (`instance.up` / `instance.exited`), and the
//! organization retains or rolls back. Killing one instance inside the
//! window rolls the whole fleet back, with the instance named.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn git(args: &[&str], cwd: &Path) -> String {
    let out = Command::new("git").args(["-c", "user.name=riley", "-c", "user.email=r@l"]).args(args).current_dir(cwd).output().expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn journal(app: &Path) -> Vec<(u64, String, String, String)> {
    let out = Command::new("git").args(["-C", &app.to_string_lossy(), "show", "refs/dna/journal:journal.jsonl"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .map(|v| {
            let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
            (v["seq"].as_u64().unwrap_or(0), s("kind"), s("entity"), s("body"))
        })
        .collect()
}

fn wait_row(app: &Path, secs: u64, pred: impl Fn(&(u64, String, String, String)) -> bool) -> bool {
    let dl = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < dl {
        if journal(app).iter().any(&pred) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

fn dump(app: &Path) -> String {
    journal(app).iter().map(|(q, k, e, b)| format!("{q} {k} {e} {}", b.chars().take(110).collect::<String>())).collect::<Vec<_>>().join("\n")
}

const DRIVER: &str = r#"import "vendor/dna" as dna;

fn main() {
    let cur = std::io::fs::read_file("main.hl") or "";
    let core = dna::Dna {
        journal: dna::GitJournal { repo: "." },
        gateway: dna::MutationGateway {
            leases: dna::GitLeases { repo: "." },
            workspaces: dna::IsolatedWorktrees { repo: ".", root: ".hale/dna/worktrees" },
            repo: dna::LocalGit { repo: "." }
        },
        verification: dna::HaleVerification { receipts: dna::GitReceipts { repo: "." }, scratch: ".hale/dna/scratch", repo: ".", seed: "." },
        editor: dna::SourceEditor {
            name: "editor",
            models: dna::ModelRouter {
                quick: dna::FakeModel { name: "quick", answer: cur + "// documented for the fleet\n" },
                deep: dna::FakeModel { name: "deep", answer: "docs_coverage +" }
            }
        },
        review_policy: dna::OrgPolicy { },
        membrane: dna::Board { who: "board" },
        boundary: dna::AutonomyBoundary { child: "fleetapp", grant: dna::Grant { child: "fleetapp", classes: "docs refactor", max_magnitude: 4, review: "pre" } }
    };
    println(core.mutate("t0", "docs", "document the entrypoint in main.hl", "main.hl", 1));
}
"#;

struct Fleet {
    d: PathBuf,
    app: PathBuf,
    bare: PathBuf,
    edges: Vec<PathBuf>,
    procs: Vec<std::process::Child>,
    base: String,
    cand: String,
}

impl Fleet {
    fn hale(&self, args: &[&str], cwd: &Path) -> (bool, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_hale"))
            .args(args)
            .current_dir(cwd)
            .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
            .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
            .output()
            .expect("hale");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    }
    fn spawn(&mut self, args: &[&str], cwd: &Path) {
        let c = Command::new(env!("CARGO_BIN_EXE_hale"))
            .args(args)
            .current_dir(cwd)
            .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
            .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn");
        self.procs.push(c);
    }
    fn stop(&mut self) {
        for p in self.procs.iter_mut() {
            let _ = p.kill();
            let _ = p.wait();
        }
        for f in ["org.pid", "app.pid"] {
            if let Ok(pid) = std::fs::read_to_string(self.app.join(".hale/dna").join(f)) {
                let _ = Command::new("kill").args(["-9", pid.trim()]).status();
            }
        }
        for (i, e) in self.edges.iter().enumerate() {
            let nd = e.join(".hale/node").join(format!("edge-{}", i + 1));
            if let Ok(rd) = std::fs::read_dir(&nd) {
                for f in rd.flatten() {
                    if f.path().extension().map(|x| x == "pid").unwrap_or(false) {
                        if let Ok(pid) = std::fs::read_to_string(f.path()) {
                            let _ = Command::new("kill").args(["-9", pid.trim()]).status();
                        }
                    }
                }
            }
        }
    }
}

/// An application with its DNA, a plan of two API instances on two
/// nodes, an origin, two node clones, and one Mutation waiting for its
/// Review; then the organization, the two nodes, and the base deployed.
fn bring_up(tag: &str) -> Fleet {
    let d = std::env::temp_dir().join(format!("hale_dna_fleet_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let mut f = Fleet { d: d.clone(), app: d.join("fleetapp"), bare: d.join("origin.git"), edges: vec![d.join("edge-1"), d.join("edge-2")], procs: vec![], base: String::new(), cand: String::new() };
    let (ok, out) = f.hale(&["dna", "new", "fleetapp"], &d);
    assert!(ok, "{out}");
    let app = f.app.clone();
    std::fs::write(
        app.join("fleet.plan.json"),
        r#"{"schema":"1.2","name":"production","instances":[
  {"id":"api-0","seed":".","node":"edge-1","labels":["api"]},
  {"id":"api-1","seed":".","node":"edge-2","labels":["api"]}]}
"#,
    )
    .unwrap();
    let mut manifest = std::fs::read_to_string(app.join("hale.toml")).unwrap();
    manifest.push_str("\n[fleets]\nproduction = \"fleet.plan.json\"\n\n[dna]\nfleet = \"production\"\n");
    std::fs::write(app.join("hale.toml"), manifest).unwrap();
    git(&["init", "-q", "--bare", "-b", "main", &f.bare.to_string_lossy()], &d);
    git(&["config", "user.name", "riley"], &app);
    git(&["config", "user.email", "r@l"], &app);
    git(&["add", "-A"], &app);
    git(&["commit", "-q", "-m", "the app and its fleet"], &app);
    git(&["remote", "add", "origin", &f.bare.to_string_lossy()], &app);
    git(&["push", "-q", "origin", "main", "refs/dna/*:refs/dna/*"], &app);
    f.base = git(&["rev-parse", "HEAD"], &app);
    for e in &f.edges {
        git(&["clone", "-q", &f.bare.to_string_lossy(), &e.to_string_lossy()], &d);
    }
    // one Mutation up to its Review, offline
    std::fs::create_dir_all(app.join("mutate")).unwrap();
    std::fs::write(app.join("mutate/main.hl"), DRIVER).unwrap();
    let (ok, out) = f.hale(&["run", "mutate"], &app);
    assert!(ok && out.contains("m1: review"), "driver:\n{out}");
    f.cand = journal(&app).iter().find(|(_, k, e, _)| k == "mutation.candidate" && e == "m1").map(|r| r.3.clone()).expect("candidate");
    // the organization, then the nodes, then the base deployed
    f.spawn(&["dna", "run", ".", "--no-iris", "--observe", "4"], &app);
    let edges = f.edges.clone();
    for (i, e) in edges.iter().enumerate() {
        f.spawn(&["node", &format!("edge-{}", i + 1), "--repo", &e.to_string_lossy(), "--tick", "300"], &d);
    }
    let dl = Instant::now() + Duration::from_secs(60);
    while !app.join(".hale/dna/org.pid").exists() && Instant::now() < dl {
        std::thread::sleep(Duration::from_millis(200));
    }
    let (ok, out) = f.hale(&["dna", "deploy", "HEAD"], &app);
    if !ok {
        f.stop();
        panic!("deploy: {out}");
    }
    assert!(out.contains("touching api-0 api-1"), "{out}");
    let base = f.base.clone();
    let up = wait_row(&app, 90, |(_, k, e, b)| k == "instance.up" && e == "api-0" && b.contains(&base)) && wait_row(&app, 90, |(_, k, e, b)| k == "instance.up" && e == "api-1" && b.contains(&base));
    if !up {
        let dump = dump(&app);
        f.stop();
        panic!("the base was not expressed by both nodes:\n{dump}");
    }
    f
}

#[test]
fn an_approval_redeploys_both_instances_and_the_window_retains() {
    let mut f = bring_up("ok");
    let app = f.app.clone();
    let cand = f.cand.clone();
    let (ok, out) = f.hale(&["dna", "review", "m1", "approve", "--as", "riley"], &app);
    if !ok {
        f.stop();
        panic!("approve: {out}");
    }
    let retained = wait_row(&app, 150, |(_, k, e, _)| k == "mutation.retained" && e == "m1");
    let rows = journal(&app);
    let fleet_out = f.hale(&["dna", "fleet"], &app).1;
    let dump = dump(&app);
    f.stop();
    assert!(retained, "the candidate was retained:\n{dump}");
    let deploy = rows.iter().find(|(_, k, e, b)| k == "fleet.deploy" && e == "m1" && b.contains("\"reason\":\"apply\"")).expect("the apply's deploy row");
    assert!(deploy.3.contains(&cand) && deploy.3.contains("\"touched\":[\"api-0\",\"api-1\"]"), "{}", deploy.3);
    let ups: Vec<&(u64, String, String, String)> = rows.iter().filter(|(q, k, _, b)| *q > deploy.0 && k == "instance.up" && b.contains(&cand)).collect();
    assert_eq!(ups.len(), 2, "both instances came up at the candidate:\n{dump}");
    let hash = |b: &str| serde_json::from_str::<serde_json::Value>(b).unwrap()["model_hash"].as_str().unwrap().to_string();
    assert_eq!(hash(&ups[0].3), hash(&ups[1].3), "both report the same model hash");
    assert!(ups.iter().any(|r| r.3.contains("\"node\":\"edge-1\"")) && ups.iter().any(|r| r.3.contains("\"node\":\"edge-2\"")), "one per node");
    let observed = rows.iter().find(|(_, k, e, _)| k == "expression.observed" && e == "m1").expect("observed");
    assert!(observed.3.starts_with(&format!("healthy {} 2 instance(s) up", hash(&ups[0].3))), "{}", observed.3);
    assert!(fleet_out.contains("api-0") && fleet_out.contains("edge-2") && fleet_out.matches(" up ").count() == 2, "hale dna fleet shows both up:\n{fleet_out}");
    assert_eq!(git(&["rev-parse", "HEAD"], &f.edges[0]), cand, "edge-1 checked out the candidate");
    let _ = std::fs::remove_dir_all(&f.d);
}

#[test]
fn an_instance_killed_inside_the_window_rolls_the_fleet_back_by_name() {
    let mut f = bring_up("kill");
    let app = f.app.clone();
    let cand = f.cand.clone();
    let base = f.base.clone();
    let (ok, out) = f.hale(&["dna", "review", "m1", "approve", "--as", "riley"], &app);
    if !ok {
        f.stop();
        panic!("approve: {out}");
    }
    // api-1 comes up at the candidate on edge-2; kill it inside the window
    let c = cand.clone();
    if !wait_row(&app, 120, |(_, k, e, b)| k == "instance.up" && e == "api-1" && b.contains(&c)) {
        let dump = dump(&app);
        f.stop();
        panic!("api-1 never came up at the candidate:\n{dump}");
    }
    let pid = std::fs::read_to_string(f.edges[1].join(".hale/node/edge-2/api-1.pid")).expect("api-1's pid");
    let _ = Command::new("kill").args(["-9", pid.trim()]).status();
    let rolled = wait_row(&app, 120, |(_, k, e, _)| k == "mutation.rolled_back" && e == "m1");
    // the rollback deploy brings both back to the base, after the row
    let rolled = rolled && wait_row(&app, 60, |(_, k, e, b)| k == "fleet.deploy" && e == "m1" && b.contains("\"reason\":\"rollback\""));
    let rb_seq = journal(&app).iter().find(|(_, k, e, b)| k == "fleet.deploy" && e == "m1" && b.contains("\"reason\":\"rollback\"")).map(|r| r.0).unwrap_or(u64::MAX);
    let b = base.clone();
    let back = rolled
        && wait_row(&app, 120, |(q, k, e, body)| *q > rb_seq && k == "instance.up" && e == "api-0" && body.contains(&b))
        && wait_row(&app, 120, |(q, k, e, body)| *q > rb_seq && k == "instance.up" && e == "api-1" && body.contains(&b));
    let rows = journal(&app);
    let dump = dump(&app);
    f.stop();
    assert!(rolled, "the organization rolled back:\n{dump}");
    let crashed = rows.iter().find(|(_, k, e, _)| k == "expression.crashed" && e == "m1").expect("the crash is in the record");
    assert!(crashed.3.contains("instance api-1 on edge-2 exited"), "{}", crashed.3);
    let observed = rows.iter().find(|(_, k, e, b)| k == "expression.observed" && e == "m1" && b.starts_with("crashed")).expect("observed crashed");
    assert!(observed.3.contains("instance api-1 on edge-2"), "{}", observed.3);
    let rb = rows.iter().find(|(_, k, e, b)| k == "fleet.deploy" && e == "m1" && b.contains("\"reason\":\"rollback\"")).expect("the rollback's deploy row");
    assert!(rb.3.contains(&base) && rb.3.contains("\"touched\":[\"api-0\",\"api-1\"]"), "{}", rb.3);
    let ups_base_after: Vec<_> = rows.iter().filter(|(q, k, _, b)| *q > rb.0 && k == "instance.up" && b.contains(&base)).collect();
    assert!(back && ups_base_after.len() == 2, "both instances came back at the base after the rollback:\n{dump}");
    assert_eq!(git(&["rev-parse", "HEAD"], &f.edges[1]), base, "edge-2 is back at the base");
    assert!(rows.iter().any(|(_, k, e, _)| k == "mutation.retained" && e == "m1") == false, "nothing was retained:\n{dump}");
    let _ = std::fs::remove_dir_all(&f.d);
}
