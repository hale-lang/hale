//! The form rows (F.40 phase 3, C1): one row per `@form` declaration,
//! with the author's `sync` configuration and the discipline the form
//! gets kept apart, and the two queries the readers ask.

use std::collections::BTreeMap;

use hale_syntax::ast::{Expr, Program, TopDecl};
use hale_types::form_rows::{form_rows, Discipline, FormRows, SyncConfig};
use hale_types::Bundle;

fn rows_of(program: &Program) -> FormRows {
    let mut programs = BTreeMap::new();
    programs.insert(String::new(), program);
    let bundle = Bundle::new(programs);
    let (top, diags) = hale_types::resolve::build_top_scope(&bundle);
    let entry = hale_types::entry::entry_row(&bundle);
    form_rows(&bundle, &top, &entry, diags.is_empty())
}

fn rows(src: &str) -> FormRows {
    rows_of(&hale_syntax::parse_source(src).expect("parse"))
}

/// Two workers on two pools, each calling `set` on a `Registry` field
/// from a bus handler: two writer pools on a hot path, which inference
/// answers `striped`. `SYNC` is the registry's `sync` argument.
fn two_writers(sync: &str) -> String {
    format!(
        r#"
type Entry {{ k: Int; v: Int; }}
type Tick {{ n: Int; }}

@form(hashmap{sync})
locus Registry {{
    capacity {{ pool entries of Entry indexed_by k; }}
}}

locus IoWorker {{
    params {{ reg: Registry = Registry {{ }}; }}
    bus {{ subscribe "tick" as on_tick of type Tick; }}
    fn on_tick(t: Tick) {{ self.reg.set(Entry {{ k: t.n, v: 1 }}); }}
}}

locus CompWorker {{
    params {{ reg: Registry = Registry {{ }}; }}
    bus {{ subscribe "tick" as on_tick of type Tick; }}
    fn on_tick(t: Tick) {{ self.reg.set(Entry {{ k: t.n, v: 2 }}); }}
}}

main locus App {{
    params {{
        io: IoWorker = IoWorker {{ }};
        cpu: CompWorker = CompWorker {{ }};
    }}
    placement {{
        io: cooperative(pool = io);
        cpu: cooperative(pool = compute);
    }}
    bus {{ publish "tick" of type Tick; }}
    run() {{ }}
}}

fn main() {{ App {{ }}; }}
"#
    )
}

#[test]
fn an_omitted_discipline_is_inferences() {
    let r = rows(&two_writers(""));
    let row = r.named("Registry").expect("a row for the form");
    assert_eq!(row.config, SyncConfig::Omitted);
    assert_eq!(row.effective, Discipline::Striped, "two writer pools on a hot path");
    assert!(row.inferred.is_some(), "the reasoning travels with the pick");
    assert!(!row.explicitly_configured());
    assert!(row.safe_for_cross_domain_access());
}

#[test]
fn an_explicit_none_suppresses_inference_and_is_not_safe() {
    let r = rows(&two_writers(", sync = none"));
    let row = r.named("Registry").expect("a row for the form");
    assert_eq!(row.config, SyncConfig::Explicit(Discipline::None));
    assert_eq!(row.effective, Discipline::None, "inference does not override the author");
    assert!(row.inferred.is_none());
    assert!(row.explicitly_configured());
    assert!(!row.safe_for_cross_domain_access());
}

#[test]
fn each_written_mode_is_configured_and_safe() {
    for (arg, d) in [
        (", sync = serialized", Discipline::Serialized),
        (", sync = striped", Discipline::Striped),
        (", sync = lockfree", Discipline::Lockfree),
        (", sync = lockfree, cap = 64", Discipline::Lockfree),
    ] {
        let r = rows(&two_writers(arg));
        let row = r.named("Registry").expect("a row for the form");
        assert_eq!(row.config, SyncConfig::Explicit(d), "{arg}");
        assert_eq!(row.effective, d, "{arg}: the written mode, not inference's");
        assert!(row.inferred.is_none(), "{arg}");
        assert!(row.explicitly_configured(), "{arg}");
        assert!(row.safe_for_cross_domain_access(), "{arg}");
    }
}

#[test]
fn an_argument_naming_no_discipline_is_configured_and_not_safe() {
    for arg in [", sync = fast", ", sync = 3"] {
        let r = rows(&two_writers(arg));
        let row = r.named("Registry").expect("a row for the form");
        assert_eq!(row.config, SyncConfig::Invalid, "{arg}");
        assert_eq!(row.effective, Discipline::None, "{arg}");
        assert!(row.explicitly_configured(), "{arg}: inference does not run");
        assert!(!row.safe_for_cross_domain_access(), "{arg}");
    }
}

#[test]
fn a_plain_map_touched_from_one_domain_stays_unsynchronized() {
    let r = rows(
        r#"
type Entry { k: Int; v: Int; }
@form(hashmap)
locus Registry { capacity { pool entries of Entry indexed_by k; } }
@form(vec)
locus Log { capacity { pool lines of Entry; } }
main locus App {
    params { reg: Registry = Registry { }; }
    placement { reg: cooperative(pool = io); }
    run() { self.reg.set(Entry { k: 1, v: 1 }); }
}
fn main() { App { }; }
"#,
    );
    let reg = r.named("Registry").expect("the map's row");
    assert_eq!(reg.config, SyncConfig::Omitted);
    assert_eq!(reg.effective, Discipline::None, "one writer pool: nothing to synchronize");
    assert!(!reg.safe_for_cross_domain_access());
    let log = r.named("Log").expect("every form has a row, not only maps");
    assert_eq!((log.form.as_str(), log.config, log.effective), ("vec", SyncConfig::Omitted, Discipline::None));
    assert!(log.inferred.is_none(), "inference reads hashmap forms only");
}

#[test]
fn a_module_nested_form_has_a_row_and_no_inference() {
    let r = rows(&two_writers("").replace(
        "@form(hashmap)\nlocus Registry {\n    capacity { pool entries of Entry indexed_by k; }\n}",
        "module store {\n@form(hashmap)\nlocus Registry {\n    capacity { pool entries of Entry indexed_by k; }\n}\n}",
    ));
    let row = r.named("Registry").expect("a nested form has its row");
    assert!(row.module_nested);
    assert_eq!(row.effective, Discipline::None, "inference reads the top level, as it always has");
}

/// The value the load's pre-pass (`apply_sync_inference`) wrote as a
/// `sync` argument on `l`'s form, if it wrote one.
fn written_sync(program: &Program, locus: &str) -> Option<String> {
    program.items.iter().find_map(|i| match i {
        TopDecl::Locus(l) if l.name.name == locus => l.form.as_ref()?.args.iter().find_map(|a| {
            match (&a.value, a.name.name == "sync") {
                (Expr::Ident(i), true) => Some(i.name.clone()),
                _ => None,
            }
        }),
        _ => None,
    })
}

/// Each form of `program` its author left unconfigured gets the
/// discipline the load's pre-pass injects, or none where it injects
/// nothing. Returns the forms seen and how many the pre-pass inferred.
fn agree_with_the_pre_pass(program: &Program, origin: &str) -> (usize, usize) {
    let r = rows_of(program);
    let mut injected = program.clone();
    let _ = hale_types::apply_sync_inference(&mut injected);
    let (mut forms, mut inferred) = (0, 0);
    for row in r.rows() {
        forms += 1;
        if row.config != SyncConfig::Omitted || row.module_nested {
            continue;
        }
        let pre_pass = written_sync(&injected, &row.locus);
        assert_eq!(row.effective.label(), pre_pass.as_deref().unwrap_or("none"), "{origin}: `{}`", row.locus);
        inferred += usize::from(pre_pass.is_some());
    }
    (forms, inferred)
}

/// The rows say what the pre-pass wrote, over every corpus program and
/// over the pinned program inference answers.
#[test]
fn the_rows_agree_with_the_injected_argument() {
    let (mut forms, mut inferred) = (0, 0);
    for p in hale_corpus::fixtures() {
        let Ok(program) = hale_syntax::parse_source(&p.source) else { continue };
        let (f, i) = agree_with_the_pre_pass(&program, &p.origin);
        forms += f;
        inferred += i;
    }
    assert!(forms > 0, "the corpus declares forms");
    eprintln!("form rows over the corpus: {forms} forms, {inferred} inferred");
    let pinned = hale_syntax::parse_source(&two_writers("")).expect("parse");
    assert_eq!(agree_with_the_pre_pass(&pinned, "two writers"), (1, 1), "the pre-pass infers the pinned form");
}
