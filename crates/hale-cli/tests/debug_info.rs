//! Debug story stage 3 (2026-07-18): DWARF variable info. Stage 2
//! gave line tables (stop on a .hl line); stage 3 attaches
//! dbg.declare to param + let allocas so gdb can INSPECT the frame:
//! typed params and locals, with String mapped to `char*` (gdb
//! prints the text, not an address). Verified structurally via
//! readelf over the emitted DWARF — no gdb dependency in CI.

use std::process::Command;

fn readelf_available() -> bool {
    Command::new("readelf")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn dwarf_carries_typed_params_and_locals() {
    if !readelf_available() {
        eprintln!("skipping: readelf not on PATH");
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "hale_dwarf_vars_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let src_path = dir.join("dbgvars.hl");
    std::fs::write(
        &src_path,
        r#"type Rec {
    key: String = "";
    n:   Int = 0;
}
fn helper(n: Int, label: String) -> Int {
    let doubled = n * 2;
    let msg = label + "!";
    let frac = 0.5;
    println(msg, doubled, frac);
    return doubled;
}
fn main() {
    let r = Rec { key: "k" + "1", n: 7 };
    let x = helper(r.n, r.key);
    println("x=", x);
}
"#,
    )
    .expect("write");

    // Build through the CLI (that's the layer that wires
    // options.debug). Dev profile keeps more variables live.
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["build"])
        .arg(&src_path)
        .env("HALE_DEV", "1")
        .output()
        .expect("hale build");
    assert!(
        out.status.success(),
        "build failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bin = dir.join("dbgvars");

    let dump = Command::new("readelf")
        .args(["--debug-dump=info"])
        .arg(&bin)
        .output()
        .expect("readelf");
    let info = String::from_utf8_lossy(&dump.stdout);

    // Full emission (not LineTablesOnly): formal parameters and
    // local variables exist as DIEs, named, with resolvable types.
    assert!(
        info.contains("DW_TAG_formal_parameter"),
        "no formal parameters in DWARF"
    );
    assert!(info.contains("DW_TAG_variable"), "no variables in DWARF");
    for name in ["doubled", "msg", "frac", "label"] {
        assert!(
            info.contains(&format!(": {}", name))
                || info.contains(&format!("DW_AT_name        : {}", name)),
            "variable `{}` missing from DWARF",
            name
        );
    }
    // The Hale type names surface as DWARF base types; String is a
    // char pointer so debuggers print the text.
    assert!(info.contains("Int"), "Int base type missing");
    assert!(info.contains("char"), "char (String pointee) missing");
    assert!(
        info.contains("DW_TAG_pointer_type"),
        "String pointer type missing"
    );
    // Stage 4: user structs carry REAL member info — the Rec
    // structure type exists with named members at layout offsets.
    assert!(
        info.contains("DW_TAG_structure_type"),
        "struct type missing"
    );
    assert!(info.contains("DW_TAG_member"), "struct members missing");
    for m in ["key", "Rec"] {
        assert!(
            info.contains(&format!(": {}", m))
                || info.contains(&format!("DW_AT_name        : {}", m)),
            "`{}` missing from struct DWARF",
            m
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}
