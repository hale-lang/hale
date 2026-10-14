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
        &Sequence { import_renames: renames, default_surface: "" },
    );
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

fn located(program: &hale_syntax::ast::Program, renames: &[(Vec<String>, String)]) -> Vec<hale_syntax::Diag> {
    let mut programs = std::collections::BTreeMap::new();
    programs.insert("test.hl".to_string(), program);
    let mut bundle = Bundle::new(programs);
    bundle.import_renames = renames.to_vec();
    let (scope, mut ds) = hale_types::resolve::build_top_scope(&bundle);
    ds.extend(hale_types::check::check_bundle(&bundle, &scope, true));
    ds
}

fn diags(program: &hale_syntax::ast::Program, renames: &[(Vec<String>, String)]) -> Vec<String> {
    located(program, renames).iter().map(|d| d.message.clone()).collect()
}

/// [`program`] with a `bindings` entry for the topic as well (in a
/// main locus, the one place a binding is written): the four places a
/// qualified topic is written.
fn bound_program(send: &str, path: &str) -> String {
    program(send, path).replace(
        "fn main() { let s = Sub { }; Pub { }; }",
        &format!(
            "main locus App {{\n\
                 bindings {{ {path}: unix(\"/tmp/hale-c2-subjects.sock\", role: listen); }}\n\
                 run() {{ let s = Sub {{ }}; Pub {{ }}; }}\n\
             }}"
        ),
    )
}

/// What each of the four places names after the sequence, as written
/// in its own AST position: (subscribe, publish, send, binding).
fn subjects(p: &hale_syntax::ast::Program) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    use hale_syntax::ast::{BusMember, BusSubject, Expr, LocusMember, Stmt, TopDecl};
    let name = |s: &BusSubject| match s {
        BusSubject::Topic(i) => i.name.clone(),
        BusSubject::QualifiedTopic(q) => {
            q.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::")
        }
        other => format!("{other:?}"),
    };
    let (mut sub, mut publ, mut send, mut bind) = (vec![], vec![], vec![], vec![]);
    for item in &p.items {
        let TopDecl::Locus(l) = item else { continue };
        for m in &l.members {
            match m {
                LocusMember::Bus(b) => {
                    for bm in &b.members {
                        match bm {
                            BusMember::Subscribe { subject, .. } => sub.push(name(subject)),
                            BusMember::Publish { subject, .. } => publ.push(name(subject)),
                        }
                    }
                }
                LocusMember::Lifecycle(lc) => {
                    for s in &lc.body.stmts {
                        if let Stmt::Send { subject, .. } = s {
                            send.push(match subject {
                                Expr::Ident(i) => i.name.clone(),
                                Expr::Path(q) => q
                                    .segments
                                    .iter()
                                    .map(|s| s.name.as_str())
                                    .collect::<Vec<_>>()
                                    .join("::"),
                                other => format!("{other:?}"),
                            });
                        }
                    }
                }
                LocusMember::Bindings(bb) => {
                    bind.extend(bb.entries.iter().map(|e| e.topic.name.clone()));
                }
                _ => {}
            }
        }
    }
    (sub, publ, send, bind)
}

/// Each of the four places is resolved on its own: the subscription,
/// the publish declaration, the send and the binding entry all name
/// the imported declaration. (A rendered program contains the mangled
/// name through the topic's own declaration whatever the four say, so
/// each is read where it stands.)
#[test]
fn every_place_a_qualified_topic_is_written_names_the_declaration() {
    let p = shaped(&bound_program("Beat { n: 1 }", "source::Heartbeat"), &renames());
    let (sub, publ, send, bind) = subjects(&p);
    assert_eq!(sub, vec![MANGLED.to_string()], "subscribe");
    assert_eq!(publ, vec![MANGLED.to_string()], "publish");
    assert_eq!(send, vec![MANGLED.to_string()], "send");
    assert_eq!(bind, vec![MANGLED.to_string()], "binding entry");
    let clean = diags(&p, &renames());
    assert!(clean.is_empty(), "the declared payload checks at all four: {clean:?}");
}

/// A wrong payload is refused AT the send that carries it: the
/// diagnostic's span lies inside the send statement as written.
#[test]
fn a_wrong_payload_on_a_qualified_topic_is_refused_at_its_send() {
    let src = bound_program("7", "source::Heartbeat");
    let ds = located(&shaped(&src, &renames()), &renames());
    let send_at = src.find("source::Heartbeat <- 7").expect("the send") as u32;
    let send_end = send_at + "source::Heartbeat <- 7".len() as u32;
    let refusal = ds
        .iter()
        .find(|d| d.message.contains("Beat"))
        .unwrap_or_else(|| panic!("the payload is refused: {:?}", ds.iter().map(|d| &d.message).collect::<Vec<_>>()));
    let at = refusal.span.start.as_usize() as u32;
    assert!(
        (send_at..send_end).contains(&at),
        "the refusal sits on the send ({send_at}..{send_end}), not at {at}: {}",
        refusal.message
    );
}

/// A qualified path no rename names is left as written at all four
/// places, and the binding entry's refusal cites it in the author's
/// spelling. (The subscription, publish and send are refused one layer
/// up, by the verbs' qualified-path pass, which
/// `hale-cli/tests/import_qualified_topic.rs` pins.)
#[test]
fn an_unresolved_qualified_topic_keeps_its_spelling_and_its_diagnostic() {
    let p = shaped(&bound_program("Beat { n: 1 }", "nowhere::Missing"), &renames());
    let (sub, publ, send, bind) = subjects(&p);
    let written = vec!["nowhere::Missing".to_string()];
    assert_eq!((&sub, &publ, &send, &bind), (&written, &written, &written, &written));
    let ds = diags(&p, &renames());
    assert!(
        ds.iter().any(|m| m.contains("binding references unknown topic `nowhere::Missing`")),
        "an unresolved binding is refused in the spelling the author wrote: {ds:?}"
    );
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
