//! A namespace-lotus method spelling on an IMPORTED locus must be
//! refused by `hale check`, not accepted-then-crashed in codegen.
//!
//! `mat::Grid { }.make(3)` calls `make` as if it were a method on the
//! imported `Grid` locus; it is a free fn in `Grid`'s seed, so `Grid`
//! has no such method. A same-file `Grid { }.make(3)` was correctly
//! rejected ("no field `make` on `Grid`"), but the IMPORTED literal
//! `mat::Grid { }` typed as `Ty::Unknown` — a multi-segment path
//! literal was waved through — so field/method access behind it was
//! unchecked and the program passed `check`, only to die at build with
//! `codegen error: unsupported in codegen v0: locus ... has no method
//! make` (GH: downstream handoff). The checker now resolves an imported
//! qualified literal to its merged symbol, so the missing method is a
//! typecheck error at the call site, in the author's spelling.

use std::path::PathBuf;
use std::process::Command;

fn hale_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hale"))
}

fn app_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("p3-namespace-method-app");
    p
}

#[test]
fn imported_namespace_method_is_refused_at_check() {
    let out = Command::new(hale_bin())
        .arg("check")
        .arg(app_dir())
        .output()
        .expect("invoke hale check <dir>");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}\n{stderr}");

    assert!(
        !out.status.success(),
        "hale check must REJECT `mat::Grid {{ }}.make(3)` (no such \
         method on the imported locus), not pass it to codegen:\n{combined}"
    );
    assert!(
        combined.contains("no field `make`"),
        "expected a `no field `make`` diagnostic at the call site, got:\n{combined}"
    );
    // The diagnostic names the author's spelling, not the mangled
    // merged symbol (`__lib_..._Grid`).
    assert!(
        combined.contains("mat::Grid") && !combined.contains("__lib_"),
        "diagnostic should name the author spelling `mat::Grid`, not the \
         mangled symbol:\n{combined}"
    );
}
