//! The sealed rule's differential (F.40 phase 4, W4): the law over the
//! typed bodies' `param_accesses` rows (`sealed_access`) against the rule
//! as it was, decided at the access (`check_sealed_access`), over every
//! program the corpus yields that parses, as written and with every
//! locus sealed (the corpus as written seals little, and reaches into a
//! sealed locus nowhere). The whole check's diagnostics, in order, must
//! be equal. Removed with the old path.

use hale_frontend::snapshot::{Config, Snapshot};
use hale_syntax::ast::TopDecl;
use hale_syntax::Diag;

fn seal_everything(items: &mut [TopDecl]) {
    for item in items {
        match item {
            TopDecl::Locus(l) => l.sealed = true,
            TopDecl::Module(m) => seal_everything(&mut m.items),
            _ => {}
        }
    }
}

type Outcome = Result<Option<Result<Vec<Diag>, String>>, ()>;

fn check(program: &hale_syntax::ast::Program) -> Outcome {
    let program = program.clone();
    std::panic::catch_unwind(move || {
        let s = Snapshot::from_program(program, Vec::new(), Config::check(true, false)).ok()?;
        Some(s.demand_check().map(|c| c.diags.clone()).map_err(|b| format!("{b:?}")))
    })
    .map_err(|_| ())
}

#[test]
fn the_law_over_the_rows_is_the_rule_at_the_access() {
    let (mut programs, mut checked, mut panicked, mut blocked) = (0, 0, 0, 0);
    let (mut diags, mut sealed, mut with_sealed) = (0, 0, 0);
    let mut differ: Vec<String> = Vec::new();
    let mut cases = Vec::new();
    for p in hale_corpus::all() {
        let Ok(program) = hale_syntax::parse_source(&p.source) else { continue };
        let mut sealed = program.clone();
        seal_everything(&mut sealed.items);
        cases.push((p.origin.clone(), program));
        cases.push((format!("{} (every locus sealed)", p.origin), sealed));
    }
    for (origin, program) in cases {
        programs += 1;
        let new = check(&program);
        let old = hale_types::check::with_the_old_sealed_rule(|| check(&program));
        if new != old {
            differ.push(format!("{origin}:\n  old: {old:?}\n  new: {new:?}"));
            continue;
        }
        match new {
            Err(()) => panicked += 1,
            Ok(None) => {}
            Ok(Some(Err(_))) => blocked += 1,
            Ok(Some(Ok(ds))) => {
                checked += 1;
                diags += ds.len();
                let n = ds.iter().filter(|d| d.message.contains("is `@sealed`: its `params`")).count();
                sealed += n;
                with_sealed += usize::from(n > 0);
            }
        }
    }
    eprintln!(
        "sealed rule differential: {programs} programs (each parsed one as written and sealed); {checked} checked, {blocked} blocked, \
         {panicked} panicked (alike on both sides); {diags} diagnostics, {sealed} of them the sealed \
         rule's, in {with_sealed} programs; {} differ",
        differ.len()
    );
    assert!(differ.is_empty(), "{} programs differ:\n{}", differ.len(), differ.join("\n"));
    assert!(sealed > 0, "the corpus holds sealed findings to compare");
}
