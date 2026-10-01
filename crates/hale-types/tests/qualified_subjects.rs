//! F.40 phase 3, C2 — a qualified bus subject is resolved once, by the
//! desugar sequence, before the check.
//!
//! `subscribe alias::Topic`, `publish alias::Topic` and
//! `alias::Topic <- v` name an imported topic. The sequence rewrites a
//! path the build's renames name to the topic's own single-segment
//! (mangled) name, so the checker reads the declaration's payload; a
//! path no rename names stays as written, for the checker's own
//! fallback and the diagnostic that cites it. The resolved-program
//! step does not resolve it again.

use hale_syntax::parse_source;
use hale_types::desugar_sequence::{desugar_before_check, Sequence};
use hale_types::symbol::Bundle;

const MANGLED: &str = "__lib_source_topics_Heartbeat";

fn program(send: &str, path: &str) -> String {
    format!(
        "type Beat {{ n: Int; }}\n\
         topic {MANGLED} {{ payload: Beat; subject: \"src.heartbeat\"; }}\n\
         locus Sub {{\n\
             bus {{ subscribe {path} as on_beat; }}\n\
             fn on_beat(b: Beat) {{ println(b.n); }}\n\
         }}\n\
         locus Pub {{\n\
             bus {{ publish {path}; }}\n\
             run() {{ {path} <- {send}; }}\n\
         }}\n\
         fn main() {{ let s = Sub {{ }}; Pub {{ }}; }}\n"
    )
}

fn renames() -> Vec<(Vec<String>, String)> {
    vec![(vec!["source".to_string(), "Heartbeat".to_string()], MANGLED.to_string())]
}

fn shaped(src: &str, renames: &[(Vec<String>, String)]) -> hale_syntax::ast::Program {
    let mut p = parse_source(src).expect("parse");
    desugar_before_check(
        &mut [&mut p],
        &Sequence { import_renames: renames, api: None, api_roles: None },
    )
    .expect("no --api, nothing to refuse");
    p
}

#[test]
fn the_sequence_resolves_a_qualified_subject_the_renames_name() {
    let p = shaped(&program("Beat { n: 1 }", "source::Heartbeat"), &renames());
    let rendered = format!("{:?}", p.items);
    assert!(!rendered.contains("QualifiedTopic"), "{rendered}");
    assert!(rendered.contains(MANGLED), "{rendered}");
}

#[test]
fn a_path_no_rename_names_stays_as_written() {
    let p = shaped(&program("Beat { n: 1 }", "source::Heartbeat"), &[]);
    let rendered = format!("{:?}", p.items);
    assert!(rendered.contains("QualifiedTopic"), "{rendered}");
}

fn diags(program: &hale_syntax::ast::Program, renames: &[(Vec<String>, String)]) -> Vec<String> {
    let mut programs = std::collections::BTreeMap::new();
    programs.insert("test.hl".to_string(), program);
    let mut bundle = Bundle::new(programs);
    bundle.import_renames = renames.to_vec();
    let (scope, mut ds) = hale_types::resolve::build_top_scope(&bundle);
    ds.extend(hale_types::check::check_bundle(&bundle, &scope, true));
    ds.iter().map(|d| d.message.clone()).collect()
}

/// What resolving before the check buys: the checker reads the
/// declared payload, so a send of anything else is refused; left
/// qualified, the checker knows nothing about the payload and lets it
/// through.
#[test]
fn the_checker_reads_the_resolved_topics_payload() {
    let src = program("7", "source::Heartbeat");
    let resolved = diags(&shaped(&src, &renames()), &renames());
    assert!(
        resolved.iter().any(|m| m.contains("Beat")),
        "a payload that is not the declared one must be refused: {resolved:?}"
    );
    let unresolved = diags(&shaped(&src, &[]), &[]);
    assert!(
        !unresolved.iter().any(|m| m.contains("Beat")),
        "left qualified, the payload is Unknown to the checker: {unresolved:?}"
    );
}
