//! a downstream tool F.10 — cross-seed type forward-ref in struct field +
//! locus-method signature.
//!
//! Pre-fix: `apply_qualified_path_renames` collapsed
//! `gfx::Rect` to a single-segment mangled name, but the
//! single-segment branch of `type_expr_to_codegen_ty` only
//! consulted `user_types` without the `pending_type_names`
//! fallback. So Surface's `bounds: gfx::Rect` field errored
//! with "unknown type name in signature" when the field was
//! processed before the lib's Rect TypeDecl in items order.
//! Worked for some downstream consumer types only by accident
//! of declaration order.
//!
//! Post-fix: single-segment branch consults
//! `pending_type_names` as a forward-ref fallback (mirror of
//! the existing multi-segment branch behavior).

use std::path::PathBuf;
use std::process::Command;

fn hale_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hale"))
}

fn fixtures_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p
}

#[test]
fn qualified_type_forward_ref_resolves_across_seeds() {
    let app_dir = fixtures_dir().join("qualified-type-fwd-app");
    let built_bin = app_dir.join("qualified-type-fwd-app");
    let _ = std::fs::remove_file(&built_bin);

    let out = Command::new(hale_bin())
        .arg("build")
        .arg(&app_dir)
        .output()
        .expect("invoke hale build");
    assert!(
        out.status.success(),
        "hale build failed: status={:?} stdout={} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let run_out = Command::new(&built_bin)
        .output()
        .expect("run qualified-type-fwd-app");
    let _ = std::fs::remove_file(&built_bin);
    assert!(
        run_out.status.success(),
        "binary exit {:?}: stderr={}",
        run_out.status,
        String::from_utf8_lossy(&run_out.stderr),
    );
    let stdout = String::from_utf8_lossy(&run_out.stdout).to_string();
    assert!(
        stdout.contains("bounds.x=1 bounds.w=30"),
        "expected 'bounds.x=1 bounds.w=30' in stdout; got: {}",
        stdout
    );
}
