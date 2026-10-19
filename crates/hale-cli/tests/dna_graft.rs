//! GH #1653: a CLI test's organisms are grafted from a habitat of the test
//! process's own (`support/habitat.rs`), born once with no body; two
//! grafted organisms are two children of one parent.

#[path = "support/habitat.rs"]
mod habitat;
#[path = "support/scratch.rs"]
mod scratch;

#[test]
fn a_test_grafts_its_organisms_into_a_habitat_of_its_own() {
    if std::env::var("HALE_DNA_MEMORY_DSN_OWNER").map(|v| v.is_empty()).unwrap_or(true) {
        eprintln!("dna_graft: not exercised (it needs HALE_DNA_MEMORY_DSN_OWNER)");
        return;
    }
    let d = scratch::Scratch::new(format!("hale_dna_graft_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let home = habitat::habitat();
    let parent = std::fs::read_to_string(home.join("genome/hale.toml")).unwrap();
    let parent = parent.lines().find_map(|l| l.trim().strip_prefix("record = \"")).map(|r| r.trim_end_matches('"').to_string()).expect("the habitat's record");
    for name in ["one", "two"] {
        let out = habitat::graft(&d, name, &["--no-library"]);
        let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success() && said.contains(&format!("(parent {}", &parent[..12])), "{name} is grafted as the habitat's child: {said}");
    }
    let one = std::fs::read_to_string(d.join("one/hale.toml")).unwrap();
    let two = std::fs::read_to_string(d.join("two/hale.toml")).unwrap();
    assert_ne!(one, two, "two organisms, two identities");
}
