//! `hale check --adopt <constitution>`: a constitution adopted into the
//! entry's `main locus` for one check, from outside its source, as an
//! environment's `constitution = …` is (spec/verification.md § Adopting
//! from outside the source). `std::law::Standard` is the stdlib's: its
//! one clause, `logs_structured`, says a program reaches no write to
//! standard output or standard error except through `std::log`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A served program whose handler writes a bare line.
const BARE: &str = "\
api Desk {
    rpc Counter::bump;
}

locus Counter {
    params { log: std::log::Logger = std::log::Logger { name: \"counter\" }; }
    fn bump(n: Int) -> Int {
        println(\"bumped \", n);
        return n + 1;
    }
}

main locus App {
    params {
        counter: Counter = Counter { };
        f: std::api::test::Rpc = std::api::test::Rpc { };
        h: api::Exposure = api::serve(Desk, self.f, as: \"desk\", receivers: { Counter: self.counter }, bound: 8, on_full: refuse);
    }
}
";

const VIOLATED: &str = "claim `logs_structured` violated: `program` reaches `effects(stdio)` — witness: `Counter::bump`";

/// The same program logging through its logger.
fn logged() -> String {
    BARE.replace("println(\"bumped \", n);", "self.log.info(\"bumped\");")
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_check_adopt_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn seed(tag: &str, text: &str, manifest: Option<&str>) -> PathBuf {
    let d = scratch(tag);
    std::fs::write(d.join("main.hl"), text).unwrap();
    if let Some(m) = manifest {
        std::fs::write(d.join("hale.toml"), m).unwrap();
    }
    d
}

fn check(seed: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(seed)
        .args(args)
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn the_standard_law_refuses_a_served_program_writing_a_bare_line() {
    let d = seed("bare", BARE, None);
    let (ok, out) = check(&d, &["--adopt", "std::law::Standard"]);
    assert!(!ok, "{out}");
    assert_eq!(out.matches(VIOLATED).count(), 1, "{out}");
    assert!(out.contains("the forbidden destination `Counter` is declared here"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_program_logging_through_its_logger_holds_the_standard_law() {
    // the stdlib's own lines (a listener that cannot bind, a ready
    // signal) are written through `std::log`, so a served program
    // reaches no bare write through them
    let d = seed("logged", &logged(), None);
    let (ok, out) = check(&d, &["--adopt", "std::law::Standard"]);
    assert!(ok, "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_program_that_adopts_nothing_is_unaffected() {
    let d = seed("none", BARE, None);
    let (ok, out) = check(&d, &[]);
    assert!(ok, "{out}");
    assert!(!out.contains("logs_structured"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_unknown_stdlib_constitution_is_refused_by_name() {
    let d = seed("unknown", BARE, None);
    let (ok, out) = check(&d, &["--adopt", "std::law::Lenient"]);
    assert!(!ok, "{out}");
    assert!(out.contains("no stdlib constitution `std::law::Lenient`: the stdlib declares std::law::Standard"), "{out}");
    let (ok, out) = check(&d, &["--adopt", "Lenient"]);
    assert!(!ok, "{out}");
    assert!(out.contains("unknown constitution `Lenient`"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_same_constitution_from_the_flag_and_the_manifest_is_one_adoption() {
    let manifest = "[claims]\nno_base = true\n\n[environments.prod]\nconstitution = \"std::law::Standard\"\nentrypoints = [\".\"]\n";
    let d = seed("twice", BARE, Some(manifest));
    let (ok, out) = check(&d, &["--env", "prod", "--adopt", "std::law::Standard", "--adopt", "std::law::Standard"]);
    assert!(!ok, "{out}");
    assert_eq!(out.matches(VIOLATED).count(), 1, "{out}");
    assert!(!out.contains("declared by two"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_flag_adopts_a_constitution_the_source_adopts_once() {
    let src = format!(
        "constitution Quiet {{\n    quiet: forbid reaches(program, effects(stdio));\n}}\n\n{}",
        BARE.replace("main locus App {\n", "main locus App {\n    claims { adopt Quiet; }\n")
    );
    let d = seed("source", &src, None);
    let (ok, out) = check(&d, &["--adopt", "Quiet"]);
    assert!(!ok, "{out}");
    assert_eq!(out.matches("claim `quiet` violated").count(), 1, "{out}");
    let _ = std::fs::remove_dir_all(&d);
}
