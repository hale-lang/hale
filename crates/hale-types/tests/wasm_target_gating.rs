//! WASM plan — stdlib target-gating (Phase 2). A program declaring
//! `target wasm` (or `browser_js`) may not call the POSIX-only stdlib
//! (no syscalls in the browser sandbox); typecheck rejects it with
//! guidance. The portable surface (str/bytes/json/math/...) is allowed,
//! and a program with no `target` decl is never gated.

#[path = "support/entries.rs"]
mod entries;
use hale_syntax::parse_source;
use hale_types::capability::ConfiguredTarget;
use entries::check_program;
use hale_types::target::TargetSpec;

fn check(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

/// The check under a configured target, as `hale check --target <t>`
/// configures it.
fn check_for(src: &str, triple: &str) -> Vec<String> {
    let mut program = parse_source(src).expect("parse failed");
    hale_types::desugar_sequence::desugar_before_check(
        &mut [&mut program],
        &hale_types::desugar_sequence::Sequence { import_renames: &[], default_surface: "" },
    );
    let ids = hale_types::snapshot::mint([("", &mut program)], &[]);
    let mut programs = std::collections::BTreeMap::new();
    programs.insert(String::new(), &program);
    let mut bundle = hale_types::Bundle::new(programs);
    bundle.snapshot = ids;
    let spec = TargetSpec::parse(triple).unwrap();
    bundle.target = ConfiguredTarget { name: spec.triple.to_string(), spec, explicit: true };
    entries::check_bundle_opts_whole_program(&bundle, false).into_iter().map(|d| d.message).collect()
}

#[test]
fn target_wasm_rejects_posix_stdlib() {
    // One representative call per gated namespace.
    let cases = [
        ("std::io::fs::write_file(\"/x\", \"h\") or raise;", "std::io::fs::write_file"),
        ("let _ = std::io::tcp::connect(\"h\", 80) or raise;", "std::io::tcp::connect"),
        ("std::io::tls::close(0);", "std::io::tls::close"),
        ("let _ = std::term::is_tty(1);", "std::term::is_tty"),
        ("let _ = std::process::pid();", "std::process::pid"),
    ];
    for (call, path) in cases {
        let src = format!("target wasm {{ }}\nfn main() {{ {} }}", call);
        let msgs = check(&src);
        assert!(
            msgs.iter().any(|m| m.contains(path) && m.contains("target wasm")),
            "expected a `target wasm` gating diagnostic for `{}`, got: {:?}",
            path,
            msgs
        );
    }
}

#[test]
fn target_wasm_allows_portable_stdlib() {
    let src = r#"
        target wasm { }
        fn main() {
            let n = std::str::parse_int("42") or 0;
            let b = std::bytes::BytesBuilder { };
            b.append_u32_le(n);
            println("n=", n);
        }
    "#;
    let msgs = check(src);
    assert!(
        msgs.is_empty(),
        "portable stdlib must not be gated under target wasm, got: {:?}",
        msgs
    );
}

#[test]
fn no_target_decl_does_not_gate() {
    // The same fs call is fine with no `target` decl (native intent).
    let src = r#"fn main() { std::io::fs::write_file("/x", "h") or raise; }"#;
    let msgs = check(src);
    assert!(
        !msgs.iter().any(|m| m.contains("target wasm")),
        "a program with no `target` decl must not be wasm-gated, got: {:?}",
        msgs
    );
}

/// The companion of the case above (design §2.9, paired case 1): the
/// same program checked for `--target wasm32` is gated by the
/// configuration alone, and the refusal names what selected the target.
#[test]
fn target_wasm32_gates_without_a_declaration() {
    let src = r#"fn main() { std::io::fs::write_file("/x", "h") or raise; }"#;
    for alias in ["wasm32", "wasm32-unknown-unknown"] {
        let msgs = check_for(src, alias);
        let want = "`std::io::fs::write_file` is unavailable under `--target wasm32`: filesystem access \
                    isn't available in the browser sandbox; use `fetch` (via an `@ffi(\"js\")` host \
                    import) or a bus message";
        assert!(msgs.iter().any(|m| m == want), "{alias}: {msgs:?}");
    }
    // A declared program under `--target wasm32` keeps the declaration's
    // wording, byte for byte.
    let declared = format!("target wasm {{ }}\n{src}");
    let msgs = check_for(&declared, "wasm32");
    assert!(
        msgs.iter().any(|m| m.starts_with("`std::io::fs::write_file` is unavailable under `target wasm`: ")),
        "{msgs:?}"
    );
    assert!(check_for(src, "x86_64-unknown-linux-gnu").iter().all(|m| !m.contains("is unavailable under")));
}
