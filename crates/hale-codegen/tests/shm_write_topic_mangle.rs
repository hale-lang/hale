//! A1 follow-up (2026-06-09): the `Topic.write(max) { ... }` construct's
//! topic reference must be import-mangled like a bus publish/subscribe
//! subject, so a producer writing to an imported topic resolves to the
//! same (renamed) subject the binding registers. Before the fix the
//! mangle pass walked the construct's body but skipped its topic ident.

use std::collections::HashMap;

use hale_codegen::mangle::mangle_with_renames;
use hale_syntax::ast::{LocusMember, Stmt, TopDecl};
use hale_syntax::parse_source;

#[test]
fn shm_write_topic_ref_is_import_mangled() {
    let src = r#"
        topic Recs { payload: BytesView; }
        locus Producer {
            bus { publish Recs; }
            birth() {
                Recs.write(8) { w =>
                    std::bytes::write_i64_le(w, 0, 1) or raise;
                    8
                };
            }
        }
    "#;
    let mut prog = parse_source(src).expect("parse");
    let mut renames = HashMap::new();
    renames.insert("Recs".to_string(), "lib_x_Recs".to_string());
    mangle_with_renames(&mut prog, &renames);

    let mut topic_ref = None;
    let mut publish_subj = None;
    for item in &prog.items {
        if let TopDecl::Locus(l) = item {
            for m in &l.members {
                match m {
                    LocusMember::Lifecycle(lc) => {
                        for s in &lc.body.stmts {
                            if let Stmt::ShmWrite { topic, .. } = s {
                                topic_ref = Some(topic.name.clone());
                            }
                        }
                    }
                    LocusMember::Bus(bb) => {
                        for bm in &bb.members {
                            if let hale_syntax::ast::BusMember::Publish {
                                subject: hale_syntax::ast::BusSubject::Topic(id),
                                ..
                            } = bm
                            {
                                publish_subj = Some(id.name.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // The construct's topic ref is mangled to the same name as the
    // publish subject — both follow the binding.
    assert_eq!(topic_ref.as_deref(), Some("lib_x_Recs"));
    assert_eq!(publish_subj.as_deref(), Some("lib_x_Recs"));
}
