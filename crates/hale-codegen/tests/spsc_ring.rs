//! GH #244: the SPSC observation ring as a lotus primitive.
//! Two layers: (1) the C driver exercises the concurrent
//! produce/consume contract (torn-record, monotonicity, and
//! delivered+overruns accounting asserts live in the driver);
//! (2) a Hale program proves the `std::ring::__spsc_*` surface
//! lowers and links end-to-end.

use std::path::PathBuf;
use std::process::Command;

use hale_codegen::build_executable;

#[test]
fn spsc_driver_concurrent_contract_holds() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut bin = std::env::temp_dir();
    bin.push("lotus_spsc_driver_test");
    let status = Command::new("clang")
        .arg(manifest.join("tests").join("spsc_driver.c"))
        .arg(manifest.join("runtime").join("lotus_arena.c"))
        .arg("-O2")
        .arg("-lpthread")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building spsc driver");
    let out = Command::new(&bin).output().expect("run driver");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "spsc contract violated.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok "));
}

#[test]
fn hale_surface_lowers_and_links() {
    // The primitives operate on caller-provided memory the
    // program would normally get from shm glue; here the calls
    // sit behind a false branch so nothing dereferences, proving
    // dispatch + declaration + link without a segment.
    let src = r#"
        fn main() {
            let never = 1 == 2;
            if never {
                let seg = 0;
                std::ring::__spsc_init(seg, 64, 1, 0);
                std::ring::__spsc_emit(seg, seg, 64, 1, 2);
                std::ring::__spsc_note_drop(seg);
                std::ring::__spsc_set_tag_b(seg, 3);
                let n = std::ring::__spsc_read(seg, seg, 64, 0, 0, 0, 8);
                println(n);
            }
            println("linked");
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("hale_test_spsc_surface");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("linked"));
}
