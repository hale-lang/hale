//! `hale doc` — the API-reference generator over the `///`
//! doc-comment convention. Markdown by default, `--json` for
//! structured records; decorator lines between the docs and the
//! declaration are stepped over; `__`-prefixed and `main` names
//! are skipped as internal.

use std::process::Command;

const SRC: &str = r#"/// A chat message.
type Msg {
    room: String = "";
    text: String = "";
}

/// A chat room.
locus Room {
    params { name: String = "lobby"; }

    /// Handle one message.
    fn on_post(m: Msg) { println(self.name, m.text); }
}

/// Clamp v into [lo, hi].
@hot
fn clamp(v: Int, lo: Int, hi: Int) -> Int {
    if v < lo { return lo; }
    if v > hi { return hi; }
    return v;
}

fn __internal() -> Int { return 0; }
fn main() { Room { }; }
"#;

fn write_fixture() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hale_doc_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("app.hl");
    std::fs::write(&f, SRC).expect("write");
    f
}

#[test]
fn markdown_reference_with_docs_and_members() {
    let f = write_fixture();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("doc")
        .arg(&f)
        .output()
        .expect("run doc");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let md = String::from_utf8_lossy(&out.stdout);

    assert!(md.contains("### Msg"), "{}", md);
    assert!(md.contains("A chat message."), "{}", md);
    // Doc above a decorator still attaches to the fn.
    assert!(
        md.contains("fn clamp(v: Int, lo: Int, hi: Int) -> Int"),
        "{}",
        md
    );
    assert!(md.contains("Clamp v into [lo, hi]."), "{}", md);
    // Locus methods render as members with their docs.
    assert!(md.contains("`fn on_post(m: Msg)` — Handle one message."), "{}", md);
    // Internal + entry-point names are skipped.
    assert!(!md.contains("__internal"), "{}", md);
    assert!(!md.contains("### main"), "{}", md);

    let _ = std::fs::remove_dir_all(f.parent().unwrap());
}

#[test]
fn json_records() {
    let f = write_fixture();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["doc", "--json"])
        .arg(&f)
        .output()
        .expect("run doc --json");
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("valid json");
    let items = v.as_array().expect("array");
    assert_eq!(items.len(), 3, "{:?}", items);
    let clamp = items
        .iter()
        .find(|i| i["name"] == "clamp")
        .expect("clamp entry");
    assert_eq!(clamp["kind"], "fn");
    assert_eq!(clamp["doc"], "Clamp v into [lo, hi].");

    let _ = std::fs::remove_dir_all(f.parent().unwrap());
}
