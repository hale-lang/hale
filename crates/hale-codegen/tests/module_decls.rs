//! Declarations inside `module { }` are lowered like top-level ones
//! (GH #884), and qualified paths written inside one are resolved
//! like top-level ones (GH #854).
//!
//! A module is a NAMESPACE, not a boundary. `resolve::register_top_decls`
//! recurses into `TopDecl::Module` and keys the bundle scope by the BARE
//! name, so `module geo { type Point { x: Int; } }` makes `Point` — not
//! `geo::Point` — the name every use site spells. Codegen read the same
//! AST with `program.items.iter()` and no `TopDecl::Module` arm, so the
//! declaration simply did not exist for it:
//!
//! ```text
//! $ hale check repro.hl
//! ok: 1 file(s) typechecked
//! $ hale run repro.hl
//! build error: Unsupported("expression form Discriminant(12)")
//! ```
//!
//! One error per kind, all from the same missing recursion: a type as
//! `Discriminant(12)` (the struct-literal arm), an enum variant as
//! "unresolved path `Kind::Round`", a free fn as "no free fn / generic
//! fn / fn-pointer binding with that name is in scope", a locus as the
//! type error again.
//!
//! The assertions here are about COMPILER OUTPUT — a program the
//! checker accepts and the builder refuses — so they live in Rust
//! rather than in `tests/hale`. Each program is put through BOTH
//! layers: `hale_types::check_program` (which `build_executable` does
//! not run) and then codegen, because half of the bug is that the two
//! disagreed.

use std::process::Command;

use hale_codegen::{build_executable, build_executable_with_imports, mangle};
use hale_syntax::ast::{
    LocusMember, Program, TopDecl, TypeDeclBody, TypeExpr,
};
use hale_syntax::parse_source;

#[path = "support/harness.rs"]
mod harness;

fn fixtures_dir() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p
}

/// Typecheck, build and run one single-seed program; return stdout.
///
/// Panics with the diagnostics if the checker refuses it and with the
/// codegen error if the build does — the two halves of the divergence
/// this file pins.
fn check_build_run(tag: &str, src: &str) -> String {
    let program = parse_source(src).expect("parse");
    let errors: Vec<String> = hale_types::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message.clone())
        .collect();
    assert!(errors.is_empty(), "check refused it: {:?}", errors);

    let bin = harness::unique_bin(tag);
    build_executable(&program, &bin)
        .unwrap_or_else(|e| panic!("build refused a check-clean program: {:?}", e));
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero exit {:?}, stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn a_type_declared_in_a_module_builds() {
    // The issue's reproduction, verbatim.
    let src = r#"
module geo { type Point { x: Int = 0; } }

fn main() {
    let p = Point { x: 4 };
    println("x=", p.x);
}
"#;
    let out = check_build_run("hale_module_type", src);
    assert!(out.contains("x=4"), "got: {:?}", out);
}

#[test]
fn an_enum_declared_in_a_module_builds() {
    // Both the variant CONSTRUCTION and the variant PATTERN: the
    // construction died as "unresolved path", and a match arm would
    // have died as "constructor pattern: unknown enum".
    let src = r#"
module shapes {
    type Kind = enum { Round, Square };
}

fn main() {
    let k = Kind::Square;
    let name = match k {
        Kind::Round -> "round",
        Kind::Square -> "square",
    };
    println("kind=", name);
}
"#;
    let out = check_build_run("hale_module_enum", src);
    assert!(out.contains("kind=square"), "got: {:?}", out);
}

#[test]
fn a_locus_declared_in_a_module_builds() {
    let src = r#"
module engine {
    locus Counter {
        params {
            start: Int = 3;
        }
        fn bump() -> Int {
            return self.start + 1;
        }
    }
}

fn main() {
    let c = Counter { };
    println("bump=", c.bump());
}
"#;
    let out = check_build_run("hale_module_locus", src);
    assert!(out.contains("bump=4"), "got: {:?}", out);
}

#[test]
fn a_free_fn_and_a_const_declared_in_a_module_build() {
    let src = r#"
module util {
    const SCALE: Int = 3;
    fn triple(n: Int) -> Int {
        return n * SCALE;
    }
}

fn main() {
    println("triple=", triple(5));
}
"#;
    let out = check_build_run("hale_module_fn_const", src);
    assert!(out.contains("triple=15"), "got: {:?}", out);
}

#[test]
fn declarations_two_modules_deep_build() {
    // One level of recursion is not the fix; the walk flattens any
    // depth, as the resolver's does.
    let src = r#"
module outer {
    module inner {
        type Deep { v: Int = 0; }
        fn twice(n: Int) -> Int { return n * 2; }
    }
}

fn main() {
    let d = Deep { v: 21 };
    println("twice=", twice(d.v));
}
"#;
    let out = check_build_run("hale_module_two_deep", src);
    assert!(out.contains("twice=42"), "got: {:?}", out);
}

#[test]
fn an_interface_declared_in_a_module_builds() {
    // The vtable synthesis and the interface-method call both look
    // the contract up by its bare name in `program.items`.
    let src = r#"
module contracts {
    interface Reading {
        fn value() -> Int;
    }

    locus Sensor {
        params { base: Int = 10; }
        fn value() -> Int { return self.base + 1; }
    }

    locus Gauge {
        params { source: Reading = Sensor { base: 100 }; }
        fn read() -> Int { return self.source.value(); }
    }
}

fn main() {
    let g = Gauge { };
    println("gauge=", g.read());
}
"#;
    let out = check_build_run("hale_module_interface", src);
    assert!(out.contains("gauge=101"), "got: {:?}", out);
}

#[test]
fn a_topic_declared_in_a_module_is_delivered() {
    // Not just "it builds": the drain-elision gate in
    // `build_executable_with_options` decides a program is bus-inert
    // by walking the same declaration list. A module-nested topic it
    // cannot see is a program whose queue is never drained, so the
    // handler would not run and this test would see no line at all.
    let src = r#"
module wiring {
    type Ping { n: Int = 0; }

    topic Beat {
        payload: Ping;
    }

    locus Listener {
        params { heard: Int = 0; }
        bus {
            subscribe Beat as on_beat;
        }
        fn on_beat(p: Ping) {
            self.heard = self.heard + 1;
            println("heard=", p.n);
        }
    }
}

main locus App {
    params { listener: Listener = Listener { }; }
    bus {
        publish Beat;
    }
    birth() {
        Beat <- Ping { n: 7 };
    }
}

fn main() {
    App { };
}
"#;
    let out = check_build_run("hale_module_topic", src);
    assert!(out.contains("heard=7"), "got: {:?}", out);
}

// === GH #854: qualified paths across an import, inside a module ====

/// Replicate the CLI's resolve-and-mangle flow for one imported lib
/// (the shape `cross_seed_imports.rs` established), and return the
/// merged program plus the per-build rename table.
///
/// `apply_qualified_path_renames` is applied here because the CLI
/// applies it (`main.rs`, the merge step) and because it is half of
/// what #854 is about: it is the only pass that collapses a
/// qualified type written in a DECLARATION's signature, and it
/// walked top-level declarations only.
fn merge_consumer_and_lib(
    consumer_rel: &str,
    lib_rel: &str,
    alias: &str,
) -> (Program, Vec<(Vec<String>, String)>) {
    let lib_dir = fixtures_dir().join(lib_rel);
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&lib_dir)
        .expect("read lib dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("hl"))
        .collect();
    files.sort();
    let mut parsed: Vec<(String, Program)> = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f).expect("read lib file");
        let prog = parse_source(&src).expect("parse lib file");
        let stem =
            f.file_stem().and_then(|s| s.to_str()).unwrap_or("x").to_string();
        parsed.push((stem, prog));
    }
    let stem_refs: Vec<(String, &Program)> =
        parsed.iter().map(|(s, p)| (s.clone(), p)).collect();
    let seed_renames = mangle::build_seed_renames(&stem_refs, alias);
    let mut renames: Vec<(Vec<String>, String)> = Vec::new();
    for (name, mangled) in &seed_renames {
        renames.push((
            vec![alias.to_string(), name.clone()],
            mangled.clone(),
        ));
    }
    renames.sort();

    let consumer_src =
        std::fs::read_to_string(fixtures_dir().join(consumer_rel))
            .expect("read consumer");
    let mut merged = parse_source(&consumer_src).expect("parse consumer");
    merged.imports.clear();
    for (_, mut prog) in parsed {
        mangle::mangle_with_renames(&mut prog, &seed_renames);
        merged.items.extend(prog.items);
    }
    mangle::apply_qualified_path_renames(&mut merged, &renames);
    (merged, renames)
}

#[test]
fn qualified_paths_inside_a_module_body_resolve_across_an_import() {
    let (merged, renames) = merge_consumer_and_lib(
        "import-module-decls-consumer/main.hl",
        "lib-module-decls",
        "lib",
    );
    let errors: Vec<String> = hale_types::check_program(&merged)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message.clone())
        .collect();
    assert!(errors.is_empty(), "check refused it: {:?}", errors);

    let bin = harness::unique_bin("hale_module_xseed");
    build_executable_with_imports(&merged, &bin, &renames)
        .expect("build consumer + lib");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero exit {:?}, stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // A qualified TYPE in a signature, and a qualified struct
    // LITERAL in a body, both inside `module wrap { }`.
    assert!(stdout.contains("row=7"), "got: {:?}", stdout);
    // A qualified CALL inside a module body.
    assert!(stdout.contains("bump=5"), "got: {:?}", stdout);
    // A qualified enum VARIANT, in construction and in pattern
    // position, inside a module body.
    assert!(stdout.contains("variant=1"), "got: {:?}", stdout);
    // A declaration that lives inside the LIBRARY's own module,
    // reached through the same alias: it needs a row in the rename
    // table (`build_seed_renames`) to be spellable at all.
    assert!(stdout.contains("deep=15"), "got: {:?}", stdout);
}

#[test]
fn a_module_nested_library_declaration_is_mangled() {
    // The other half of the row above: the decl inside the library's
    // module must be RENAMED, or it reaches the merged program under
    // its bare name and collides with whatever the importer calls by
    // the same one.
    let lib_src = r#"
type Row { v: Int = 0; }
module nested {
    type Deep { w: Int = 0; }
}
"#;
    let prog = parse_source(lib_src).expect("parse");
    let renames = mangle::build_seed_renames(
        &[("lib".to_string(), &prog)],
        "libid",
    );
    assert_eq!(
        renames.get("Row").map(String::as_str),
        Some("__lib_libid_lib_Row"),
    );
    assert_eq!(
        renames.get("Deep").map(String::as_str),
        Some("__lib_libid_lib_Deep"),
        "a module-nested declaration needs a rename row too: {:?}",
        renames,
    );

    let mut mangled = prog.clone();
    mangle::mangle_with_renames(&mut mangled, &renames);
    let mut names: Vec<String> = Vec::new();
    collect_type_names(&mangled.items, &mut names);
    names.sort();
    assert_eq!(
        names,
        vec![
            "__lib_libid_lib_Deep".to_string(),
            "__lib_libid_lib_Row".to_string(),
        ],
    );
}

fn collect_type_names(items: &[TopDecl], out: &mut Vec<String>) {
    for item in items {
        match item {
            TopDecl::Type(t) => out.push(t.name.name.clone()),
            TopDecl::Module(m) => collect_type_names(&m.items, out),
            _ => {}
        }
    }
}

#[test]
fn the_rename_pre_pass_collapses_a_type_inside_a_module() {
    // `apply_qualified_path_renames` is the ONLY pass that resolves
    // a qualified type in a declaration's signature before typecheck
    // (PR #851 moved the body-position `let x: lib::T` case into the
    // resolver instead). It skipped `TopDecl::Module` entirely.
    let src = r#"
module wrap {
    fn take(r: lib::Row) -> Int { return r.v; }
    type Holder { r: lib::Row; }
}
"#;
    let mut prog = parse_source(src).expect("parse");
    let renames = vec![(
        vec!["lib".to_string(), "Row".to_string()],
        "__lib_x_lib_Row".to_string(),
    )];
    mangle::apply_qualified_path_renames(&mut prog, &renames);

    let mut seen: Vec<String> = Vec::new();
    collect_named_type_paths(&prog.items, &mut seen);
    assert_eq!(
        seen,
        vec![
            "__lib_x_lib_Row".to_string(),
            "__lib_x_lib_Row".to_string(),
        ],
        "a qualified type inside a module was left as written",
    );
}

/// Every `TypeExpr::Named` path in `items`, joined by `::`.
fn collect_named_type_paths(items: &[TopDecl], out: &mut Vec<String>) {
    fn joined(t: &TypeExpr, out: &mut Vec<String>) {
        if let TypeExpr::Named { path, .. } = t {
            out.push(
                path.segments
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join("::"),
            );
        }
    }
    for item in items {
        match item {
            TopDecl::Fn(f) => {
                for p in &f.params {
                    joined(&p.ty, out);
                }
            }
            TopDecl::Type(t) => {
                if let TypeDeclBody::Struct(fields) = &t.body {
                    for f in fields {
                        joined(&f.ty, out);
                    }
                }
            }
            TopDecl::Module(m) => collect_named_type_paths(&m.items, out),
            _ => {}
        }
    }
}

#[test]
fn the_rename_pre_pass_collapses_a_mode_param() {
    // GH #854's second finding: the `LocusMember::Mode` arm rewrote
    // `ret` only, unlike every sibling arm — and a mode DOES take
    // typed params (`mode bulk(n: Int) -> T`, spec grammar
    // `mode_decl`), which the full mangler's `walk_fn_like` walks.
    let src = r#"
locus Tower {
    params { n: Int = 0; }
    mode bulk(r: lib::Row) -> lib::Row {
        return r;
    }
}
"#;
    let mut prog = parse_source(src).expect("the mode fixture must parse");
    let renames = vec![(
        vec!["lib".to_string(), "Row".to_string()],
        "__lib_x_lib_Row".to_string(),
    )];
    mangle::apply_qualified_path_renames(&mut prog, &renames);

    let mut param_paths: Vec<String> = Vec::new();
    let mut ret_paths: Vec<String> = Vec::new();
    for item in &prog.items {
        let TopDecl::Locus(l) = item else { continue };
        for m in &l.members {
            let LocusMember::Mode(md) = m else { continue };
            for p in &md.params {
                if let TypeExpr::Named { path, .. } = &p.ty {
                    param_paths.push(
                        path.segments
                            .iter()
                            .map(|s| s.name.as_str())
                            .collect::<Vec<_>>()
                            .join("::"),
                    );
                }
            }
            if let Some(TypeExpr::Named { path, .. }) = &md.ret {
                ret_paths.push(
                    path.segments
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join("::"),
                );
            }
        }
    }
    // The return was always collapsed; the param is the fix.
    assert_eq!(ret_paths, vec!["__lib_x_lib_Row".to_string()]);
    assert_eq!(
        param_paths,
        vec!["__lib_x_lib_Row".to_string()],
        "a mode param kept the qualified spelling its sibling arms collapse",
    );
}
