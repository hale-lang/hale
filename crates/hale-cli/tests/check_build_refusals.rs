//! Shapes `hale check` accepted and `hale build` refused, from codegen,
//! with a message that named lowering rather than the program and, for
//! most, no file or line (downstream handoff). Each is refused by the
//! check now, as `file:line:col` at the construct, and the build of the
//! same file stops at the check's message rather than in codegen; a
//! shape that is lowered instead checks and builds.
//! `corpus_check_build_agreement` holds the same shapes to the
//! agreement sweep's verdict.

use std::path::{Path, PathBuf};
use std::process::Command;

fn hale(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).current_dir(Path::new("/")).output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

/// `hale check` and `hale build` on one file; the scratch dir is removed.
fn check_and_build(src: &str, tag: &str) -> ((bool, String), (bool, String)) {
    let d: PathBuf = std::env::temp_dir().join(format!("hale_check_build_refusals_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("main.hl");
    std::fs::write(&f, src).unwrap();
    let file = f.to_string_lossy().to_string();
    let checked = hale(&["check", &file]);
    let built = hale(&["build", &file, "-o", &d.join("bin").to_string_lossy()]);
    let _ = std::fs::remove_dir_all(&d);
    (checked, built)
}

/// The check refuses `src` at `main.hl:<at>` with `head`, and the build
/// stops at the same message, never reaching codegen.
fn refused(src: &str, tag: &str, at: &str, head: &str) {
    let ((ok, out), (built, bout)) = check_and_build(src, tag);
    assert!(!ok && out.contains(&format!("main.hl:{at}: type error: {head}")), "check: {out}");
    assert!(!built && bout.contains(head), "build: {bout}");
    assert!(!bout.contains("codegen error"), "the build stops at the check: {bout}");
}

#[test]
fn a_locus_held_by_a_type_is_refused_at_the_field() {
    refused(
        "locus M { params { n: Int = 1; } }\ntype Layer { w: M; }\nfn main() { let l = Layer { w: M { } }; println(l.w.n); }\n",
        "locus_field",
        "2:17",
        "`type Layer`: field `w` holds the locus `M`. A `type` is plain data with no lifecycle",
    );
    refused(
        "type Conn { s: std::io::tcp::Stream; }\nfn main() { println(\"x\"); }\n",
        "stdlib_locus_field",
        "1:16",
        "`type Conn`: field `s` holds the locus `std::io::tcp::Stream`.",
    );
}

#[test]
fn an_uncarried_topic_payload_is_refused_where_it_is_delivered() {
    let src = "topic Chunk { payload: String; }\nlocus Sub {\n    bus { subscribe Chunk as on_chunk; }\n    fn on_chunk(s: String) { println(s); }\n}\nlocus Pub {\n    bus { publish Chunk; }\n    run() { Chunk <- \"hi\"; }\n}\nfn main() { let s = Sub { }; Pub { }; }\n";
    let ((ok, out), (built, bout)) = check_and_build(src, "topic");
    assert!(!ok, "{out}");
    assert!(
        out.contains("main.hl:3:11: type error: subscribe `Chunk`: topic `Chunk` declares `payload: String`, which the bus cannot carry.")
            && out.contains("main.hl:8:13: type error: `Chunk <- ...`: topic `Chunk` declares `payload: String`")
            && out.contains("wrap it in a user `type` and declare the topic's `payload:` as that"),
        "{out}"
    );
    assert!(!built && !bout.contains("codegen error"), "{bout}");
}
