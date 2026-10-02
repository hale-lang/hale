//! The form rows (F.40 phase 3, C1): one row per `@form` declaration,
//! with the author's `sync` configuration and the discipline the form
//! gets kept apart, and the two queries the readers ask.

use std::collections::BTreeMap;

use hale_syntax::ast::{Program, TopDecl};
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
locus Log { capacity { heap lines of Entry; } }
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

/// `App` reads a `Registry` placed on another pool, and a `Writer` on
/// a third pool writes one: one writer pool and two reader pools, which
/// inference answers `serialized`. `SYNC` is the registry's `sync`
/// argument.
fn cross_pool(sync: &str) -> String {
    format!(
        r#"
type Entry {{ k: Int; v: Int; }}
type Tick {{ n: Int; }}

@form(hashmap{sync})
locus Registry {{
    capacity {{ pool entries of Entry indexed_by k; }}
}}

locus Writer {{
    params {{ reg: Registry = Registry {{ }}; }}
    bus {{ subscribe "tick" as on_tick of type Tick; }}
    fn on_tick(t: Tick) {{ self.reg.set(Entry {{ k: t.n, v: 1 }}); }}
}}

main locus App {{
    params {{
        reg: Registry = Registry {{ }};
        w: Writer = Writer {{ }};
    }}
    placement {{
        reg: cooperative(pool = io);
        w: cooperative(pool = compute);
    }}
    bus {{ publish "tick" of type Tick; }}
    run() {{ let _ = self.reg.has(1); }}
}}

fn main() {{ App {{ }}; }}
"#
    )
}

fn cross_pool_errors(sync: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(&cross_pool(sync)).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error() && d.message.contains("cross-pool method call"))
        .map(|d| d.message)
        .collect()
}

/// The F.31 cross-pool check admits a call into a map whose discipline
/// sync inference gave it, as it admitted the `sync =` argument the load
/// used to write into the program for it.
#[test]
fn an_inferred_discipline_admits_a_cross_pool_call() {
    let r = rows(&cross_pool(""));
    assert_eq!(r.named("Registry").unwrap().effective, Discipline::Serialized);
    assert_eq!(cross_pool_errors(""), Vec::<String>::new());
    assert_eq!(cross_pool_errors(", sync = serialized"), Vec::<String>::new());
}

/// The F.31 cross-pool check asks one question of the receiver's form
/// row, `safe_for_cross_domain_access`. An explicit `sync = none` is
/// configured, so inference leaves it alone, and it is not safe: the
/// cross-pool read is refused, as it was before the rows (the checker's
/// own predicate admitted only `serialized`, `striped` and `lockfree`).
#[test]
fn the_cross_pool_check_asks_whether_the_form_is_safe() {
    for admitted in ["", ", sync = serialized", ", sync = striped", ", sync = lockfree, cap = 64"] {
        assert_eq!(cross_pool_errors(admitted), Vec::<String>::new(), "{admitted}");
    }
    let none = cross_pool_errors(", sync = none");
    assert_eq!(none.len(), 1, "{none:?}");
    assert!(none[0].contains("`self.reg.has`"), "{}", none[0]);
    let r = rows(&cross_pool(", sync = none"));
    let row = r.named("Registry").unwrap();
    assert!(row.explicitly_configured() && !row.safe_for_cross_domain_access());
    assert!(row.inferred.is_none(), "inference does not run over an explicit none");

    assert_eq!(cross_pool_errors(", sync = fast").len(), 1, "an argument naming no discipline is not safe");

    // A plain map: only `App` touches it, so inference leaves it
    // unsynchronized, and the read across pools is refused.
    let plain = cross_pool("").replace("self.reg.set(Entry { k: t.n, v: 1 });", "");
    let program = hale_syntax::parse_source(&plain).expect("parse");
    let r = rows_of(&program);
    assert_eq!(r.named("Registry").unwrap().effective, Discipline::None);
    let errors: Vec<_> = hale_types::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error() && d.message.contains("cross-pool method call"))
        .collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
}

fn decl<'a>(program: &'a Program, locus: &str) -> &'a hale_syntax::ast::LocusDecl {
    program
        .items
        .iter()
        .find_map(|i| match i {
            TopDecl::Locus(l) if l.name.name == locus => Some(l),
            _ => None,
        })
        .expect("the declaration")
}

/// The readers that ask whether a form synchronizes as one question
/// (the model's `sync_form`, the effects certificate engine, the
/// instance-aliasing rule) ask safe-for-cross-domain-access alone: a
/// written discipline that synchronizes, and inference's pick, count.
/// An explicit `sync = none`, an argument naming no discipline and a
/// map inference left unsynchronized take no lock and do not.
#[test]
fn synchronizes_is_safe_for_cross_domain_access() {
    for arg in ["", ", sync = serialized", ", sync = lockfree, cap = 64"] {
        let program = hale_syntax::parse_source(&two_writers(arg)).expect("parse");
        let r = rows_of(&program);
        assert!(r.synchronizes(decl(&program, "Registry")), "{arg}");
    }
    for arg in [", sync = none", ", sync = fast"] {
        let program = hale_syntax::parse_source(&two_writers(arg)).expect("parse");
        let r = rows_of(&program);
        assert!(!r.synchronizes(decl(&program, "Registry")), "{arg}: takes no lock");
    }
    let one_pool = hale_syntax::parse_source(&two_writers("").replace("pool = compute", "pool = io")).expect("parse");
    let r = rows_of(&one_pool);
    assert_eq!(r.named("Registry").unwrap().effective, Discipline::None, "one pool");
    assert!(!r.synchronizes(decl(&one_pool, "Registry")));
    assert!(!r.synchronizes(decl(&one_pool, "App")), "not a form");
}

/// A declaration the rows do not hold (the stdlib's, merged into the
/// program lowering walks) reads its written argument; one they hold
/// keeps its row when the written rows are added beside them.
#[test]
fn a_declaration_without_a_row_reads_its_written_argument() {
    let program = hale_syntax::parse_source(
        r#"
type Entry { k: Int; v: Int; }
@form(hashmap, sync = serialized)
locus Store { capacity { pool entries of Entry indexed_by k; } }
@form(hashmap)
locus Plain { capacity { pool entries of Entry indexed_by k; } }
"#,
    )
    .expect("parse");
    let none = FormRows::default();
    assert_eq!(none.effective(decl(&program, "Store")), Discipline::Serialized);
    assert!(none.synchronizes(decl(&program, "Store")));
    assert_eq!(none.effective(decl(&program, "Plain")), Discipline::None);
    assert!(!none.synchronizes(decl(&program, "Plain")));

    let pinned = hale_syntax::parse_source(&two_writers("")).expect("parse");
    let merged = rows_of(&pinned).extended(FormRows::configured(&pinned.items));
    assert_eq!(merged.rows().len(), 1, "the written row of a held declaration is not added");
    assert_eq!(merged.effective(decl(&pinned, "Registry")), Discipline::Striped, "the held row wins");
}

