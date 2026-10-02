//! The file-entry verbs (`hale run <file>`, `hale test`, `hale replay`,
//! `hale bench`) mint the snapshot before any desugar runs, and the
//! resolved-program step then runs the desugars and mints again. A
//! desugar that copies a subtree with its identities makes the second
//! mint find one id on two sites, which is a panic (the api surface's
//! copies did exactly that, review of phase 1, finding 19). Every
//! desugar that copies clears the copy's ids; nothing guards the rule
//! itself except this: every corpus program, minted first and resolved
//! second, resolves without a panic.

#[test]
fn every_corpus_program_survives_a_mint_before_the_resolved_program() {
    let programs =
        hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok());
    assert!(programs.len() > 1000, "the corpus is not there ({})", programs.len());
    let mut resolved = 0usize;
    for p in &programs {
        let Ok(mut program) = hale_syntax::parse_source(&p.source) else {
            continue;
        };
        // The file-entry order: mint over the raw program, no source map
        // (its seed is the program's ordinal), no desugar before it.
        let _ = hale_types::snapshot::mint([(p.origin.as_str(), &mut program)], &[]);
        // Then the resolved-program step, which runs every desugar and
        // mints again. A refusal (`Err`) is a program the resolver
        // refuses on its merits; a panic here is a copied identity.
        if hale_types::resolved::resolve_program(&program, &[], &[], None, None, &hale_types::form_rows::FormRows::default(), &hale_types::binding_rows::BindingRows::default()).is_ok() {
            resolved += 1;
        }
    }
    assert!(resolved > 500, "too few programs resolved ({resolved} of {})", programs.len());
}
