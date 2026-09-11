//! GH #566 F5 — plan schema 1.2: an instance may name a `seed` instead
//! of an `artifact`, and composition cuts the artifact from the seed
//! first; `hale fleet check --in <dir> --if-declared` is the
//! verification step's spelling, a success that says so when the
//! workspace declares no fleet.

use std::path::Path;
use std::process::Command;

fn hale(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).current_dir(cwd).output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

const SVC: &str = r#"topic Tick { payload: Int; subject: "svc.tick"; }
locus Svc {
    params { n: Int = 0; }
    fn bump() { self.n = self.n + 1; }
}
fn main() {
    let s = Svc { };
    s.bump();
    println("svc ", s.n);
}
"#;

#[test]
fn an_instance_with_a_seed_composes_from_the_artifact_cut_from_it() {
    let d = std::env::temp_dir().join(format!("hale_fleet_seed_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("svc")).unwrap();
    std::fs::write(d.join("svc/main.hl"), SVC).unwrap();
    std::fs::write(
        d.join("prod.plan.json"),
        r#"{"schema":"1.2","name":"prod","instances":[
  {"id":"svc-0","seed":"svc","node":"edge-1","labels":["svc"]},
  {"id":"svc-1","seed":"svc","node":"edge-2","labels":["svc"]}]}"#,
    )
    .unwrap();
    let (ok, out) = hale(&["fleet", "check", "prod.plan.json"], &d);
    assert!(ok, "a plan of seeds composes:\n{out}");
    assert!(d.join(".hale/fleet/prod/svc-0.topology.json").is_file() && d.join(".hale/fleet/prod/svc-1.topology.json").is_file(), "the artifacts were cut beside the plan");
    // the workspace's fleets, from elsewhere, and none declared is a
    // success that says so
    std::fs::write(d.join("hale.toml"), "[deps]\n[fleets]\nprod = \"prod.plan.json\"\n").unwrap();
    let (ok, out) = hale(&["fleet", "check", "--in", &d.to_string_lossy(), "--if-declared"], Path::new("/"));
    assert!(ok && out.contains("fleet `prod`") && out.contains("ok:"), "{out}");
    std::fs::write(d.join("hale.toml"), "[deps]\n").unwrap();
    let (ok, out) = hale(&["fleet", "check", "--in", &d.to_string_lossy(), "--if-declared"], Path::new("/"));
    assert!(ok && out.contains("declares no [fleets]"), "{out}");
    let (ok, out) = hale(&["fleet", "check", "--in", &d.to_string_lossy()], Path::new("/"));
    assert!(!ok && out.contains("declares no `[fleets]`"), "without --if-declared, none is still the refusal:\n{out}");
    // neither an artifact nor a seed is a plan that cannot be formed
    std::fs::write(d.join("bad.plan.json"), r#"{"schema":"1.2","name":"bad","instances":[{"id":"x-0","node":"edge-1"}]}"#).unwrap();
    let (ok, out) = hale(&["fleet", "check", "bad.plan.json"], &d);
    assert!(!ok && out.contains("no `artifact` and no `seed`"), "{out}");
    // a seed that does not check has no artifact to compose
    std::fs::write(d.join("svc/main.hl"), "fn main() { let x: Int = \"no\"; }\n").unwrap();
    let (ok, out) = hale(&["fleet", "check", "prod.plan.json"], &d);
    assert!(!ok && out.contains("does not check, so it has no artifact"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}
