//! GH #529 D7: a `main locus` that arrives through `import` keeps its
//! `bindings { }` inert. Found by the DNA's own verification: `hale
//! test <worktree>` on a governed application built a test binary that
//! imported the application's main — and bound (unlinking) the
//! organism's live membrane sockets from under it.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn a_test_binary_importing_a_bound_main_does_not_touch_its_sockets() {
    let d = std::env::temp_dir().join(format!("hale_bind_import_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let app: PathBuf = d.join("app");
    std::fs::create_dir_all(app.join("tests")).unwrap();
    let sock = d.join("membrane.sock");
    std::fs::write(&sock, "a file standing where the application's socket would be").unwrap();
    std::fs::write(app.join("hale.toml"), "[deps]\n").unwrap();
    std::fs::write(
        app.join("main.hl"),
        format!(
            r#"type Fact {{ n: Int = 0; }}
topic Facts {{ payload: Fact; subject: "app.facts"; }}
locus Counter {{
    params {{ seen: Int = 0; }}
    bus {{ subscribe Facts as on_fact; }}
    fn on_fact(f: Fact) {{ self.seen = self.seen + 1; }}
}}
main locus App {{
    params {{ c: Counter = Counter {{ }}; }}
    bindings {{ Facts: unix("{}", role: listen); }}
    run() {{ }}
}}
fn main() {{ App {{ }}; }}
"#,
            sock.display()
        ),
    )
    .unwrap();
    std::fs::write(
        app.join("tests/main_test.hl"),
        "import \"..\" as app;\nfn main() {\n    let c = app::Counter { };\n    std::test::assert_eq_int(c.seen, 0, \"fresh\");\n}\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).arg("test").arg(&app).output().expect("hale test");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && text.contains("1 passed, 0 failed"), "{text}");
    let still = std::fs::read_to_string(&sock).unwrap_or_default();
    assert!(still.starts_with("a file standing"), "the test binary bound the imported main's socket path (the file is gone or replaced)");
    let _ = std::fs::remove_dir_all(&d);
}
