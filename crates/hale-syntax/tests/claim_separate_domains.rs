//! GH #1327: execution-domain separation uses the existing claim surface.

use hale_syntax::ast::{ClaimForm, Program, TopDecl};
use hale_syntax::{parse_source, parse_source_at};

fn operands(program: &Program) -> (&hale_syntax::ast::Ident, &hale_syntax::ast::Ident) {
    let TopDecl::Claims(block) = &program.items[0] else {
        panic!("claims block")
    };
    let ClaimForm::RequireSeparateDomains { left, right } = &block.entries[0].form else {
        panic!("separate_domains form")
    };
    (left, right)
}

#[test]
fn separate_domains_preserves_both_group_names_and_spans() {
    let source = "claims { apart: require separate_domains(all workers, all ingress); }";
    let program = parse_source(source).expect("valid claim");
    let (left, right) = operands(&program);
    assert_eq!((&*left.name, &*right.name), ("workers", "ingress"));
    let mut names = Vec::new();
    hale_syntax::names::for_each_spelled(&program.items, &mut |spelled| {
        if let hale_syntax::names::Spelled::Name(name) = spelled {
            names.push(name);
        }
    });
    assert!(names.contains(&"workers") && names.contains(&"ingress"));

    let mut moved = program.clone();
    hale_syntax::shift::shift_program(&mut moved, 4096);
    let parsed_at = parse_source_at(source, 4096).expect("shifted claim");
    for ((original, shifted), reparsed) in [left, right]
        .into_iter()
        .zip([operands(&moved).0, operands(&moved).1])
        .zip([operands(&parsed_at).0, operands(&parsed_at).1])
    {
        assert_eq!(shifted.span, reparsed.span);
        assert_eq!(
            shifted.span.start.as_usize(),
            original.span.start.as_usize() + 4096
        );
    }
}

#[test]
fn separate_domains_requires_two_universal_group_operands() {
    for args in [
        "some workers, all ingress",
        "all workers, some ingress",
        "workers, ingress",
        "all workers",
        "all workers, all ingress, all extra",
    ] {
        let source = format!("claims {{ apart: require separate_domains({args}); }}");
        assert!(parse_source(&source).is_err(), "accepted {args}");
    }
}

#[test]
fn separate_domains_is_available_in_constitutions_and_formats() {
    let source =
        "constitution Isolation { apart: require separate_domains(all workers, all ingress); }";
    let parsed = parse_source(source).expect("constitution clause");
    let TopDecl::Constitution(decl) = &parsed.items[0] else {
        panic!("constitution")
    };
    assert!(matches!(
        decl.entries[0].form,
        ClaimForm::RequireSeparateDomains { .. }
    ));
    let formatted = hale_syntax::fmt::format_source(source).expect("formats");
    parse_source(&formatted).expect("formatted constitution parses");
    assert_eq!(
        hale_syntax::fmt::format_source(&formatted).unwrap(),
        formatted
    );
}
