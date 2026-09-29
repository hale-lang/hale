//! A wasm32 build links without a word from wasm-ld.
//!
//! Every wasm build printed `wasm-ld: warning: function signature
//! mismatch: lotus_str_view_data` with `defined as (i32, i64) -> i32` in
//! the program's object and `defined as (i32) -> i32` in the runtime's.
//! The runtime helper took the two-word view struct BY VALUE, which the
//! wasm32 C ABI passes indirectly (one pointer) while an LLVM aggregate
//! argument lowers to two scalars, so caller and callee disagreed about
//! the arity of a call the warning called undefined behaviour. The
//! helpers now take the view's two fields, which is the same call on
//! every target.
//!
//! The programs exercise the helpers: one that never reads a view (the
//! symbol was still referenced), and one that reads both a byte view and
//! a string view.

use std::path::PathBuf;
use std::process::Command;

fn build_wasm(tag: &str, src: &str) -> (bool, String, Vec<PathBuf>) {
    let dir = std::env::temp_dir().join(format!("hale_wasm_quiet_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    let file = dir.join("main.hl");
    std::fs::write(&file, src).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["build"])
        .arg(&file)
        .args(["--target", "wasm32"])
        .output()
        .expect("run hale");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let produced: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "wasm"))
        .collect();
    let ok = out.status.success();
    let _ = std::fs::remove_dir_all(&dir);
    (ok, text, produced)
}

fn assert_quiet(tag: &str, src: &str) {
    let (ok, text, _) = build_wasm(tag, src);
    assert!(ok, "{tag}: the build fails: {text}");
    assert!(
        !text.contains("signature mismatch") && !text.contains("wasm-ld: warning"),
        "{tag}: the linker has something to say:\n{text}"
    );
}

#[test]
fn an_empty_program_links_in_silence() {
    assert_quiet("empty", "fn main() { }\n");
}

#[test]
fn a_program_reading_byte_and_string_views_links_in_silence() {
    assert_quiet(
        "views",
        r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("hello"));
            let v = b.view();
            println("len=", len(v));
            println("b0=", std::bytes::at(v, 0) or -1);
            let t = b.text_view();
            println(t);
        }
        "#,
    );
}
