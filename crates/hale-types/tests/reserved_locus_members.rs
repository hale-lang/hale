//! GH #734: a locus member may not be spelled like a member the
//! compiler injects.
//!
//! `children` (the accept'd-child collection), `k_max` (F.1
//! displacement bound) and `draining` (F.27 drain flag) are
//! resolved ahead of anything a locus declares, in both the
//! checker's `field_ty` and codegen's field lowering. A locus that
//! declared one of those names therefore had a member it could
//! never read back, and the failure landed somewhere else:
//!
//! - `params { children: Int = 3; }` + `return self.children`
//!   reported `return: expected Int, got [?]` at the RETURN — the
//!   synthetic collection's type, with nothing pointing at the
//!   declaration (the reproducer in the issue, from a downstream
//!   handoff building recursive ownership);
//! - `params { k_max: Float = 7.5; }` typechecked clean, because
//!   the synthetic `k_max` is a Float too, and failed only at
//!   codegen with `k_max requires param B on locus Holder` — a
//!   message about params the program never wrote.
//!
//! The names are now reserved at the declaration, so the error sits
//! on the line that has to change and a collision never reaches
//! codegen. Loci that legitimately accept children are untouched:
//! the synthetic members keep their meaning, they just cannot be
//! shadowed.

use hale_syntax::parse_source;
use hale_types::check_program;

/// The GH #734 reserved-member diagnostics raised for `src`.
fn reserved_diags(src: &str) -> Vec<(String, String)> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog)
        .into_iter()
        .filter(|d| d.message.contains("is a reserved locus member"))
        .map(|d| (d.message.clone(), d.span.slice(src).to_string()))
        .collect()
}

const ISSUE_REPRODUCER: &str = r#"
locus Holder {
    params { children: Int = 3; }
    fn read() -> Int { return self.children; }
}
fn main() { let h = Holder { }; println(h.read()); }
"#;

#[test]
fn a_params_field_named_children_is_rejected_at_its_declaration() {
    let hits = reserved_diags(ISSUE_REPRODUCER);
    assert_eq!(
        hits.len(),
        1,
        "exactly one reserved-member diagnostic: {hits:?}"
    );
    let (msg, at) = &hits[0];
    // Located AT the declaration — the span covers the declared
    // name, not the `return self.children` that used to be the only
    // thing reported.
    assert_eq!(at, "children", "span points at the declared name: {msg}");
    assert!(
        msg.starts_with("params field `children`:"),
        "names what was declared: {msg}"
    );
    assert!(
        msg.contains("accept'd-child collection"),
        "identifies the synthetic member it collides with: {msg}"
    );
    assert!(
        msg.contains("Rename it (`own_children`, say)"),
        "offers a usable workaround: {msg}"
    );
}

#[test]
fn every_synthetic_member_name_is_reserved_for_params_fields() {
    for (name, phrase) in [
        ("children", "accept'd-child collection"),
        ("k_max", "F.1 displacement bound"),
        ("draining", "F.27 drain flag"),
    ] {
        let src = format!(
            "locus Holder {{ params {{ {name}: Int = 3; }} run() {{ }} }}\n\
             fn main() {{ Holder {{ }}; }}\n"
        );
        let hits = reserved_diags(&src);
        assert_eq!(hits.len(), 1, "`{name}` is reserved: {hits:?}");
        assert!(
            hits[0].0.starts_with(&format!("params field `{name}`:"))
                && hits[0].0.contains(phrase),
            "`{name}` diagnostic explains the collision: {}",
            hits[0].0
        );
    }
}

#[test]
fn methods_and_capacity_slots_are_reserved_too() {
    // A method is reachable as `self.<name>()` today, but the read
    // spelling `self.<name>` is the synthetic member's, so the two
    // cannot share a name either.
    for name in ["children", "k_max", "draining"] {
        let src = format!(
            "locus Holder {{\n\
             \x20   params {{ n: Int = 3; }}\n\
             \x20   fn {name}() -> Int {{ return self.n; }}\n\
             }}\n\
             fn main() {{ Holder {{ }}; }}\n"
        );
        let hits = reserved_diags(&src);
        assert_eq!(hits.len(), 1, "method `{name}` is reserved: {hits:?}");
        assert!(
            hits[0].0.starts_with(&format!("method `{name}`:")),
            "names the member kind: {}",
            hits[0].0
        );
        assert_eq!(hits[0].1, name, "span points at the method name");
    }

    // F.22 capacity slots are read as `self.<slot>` as well.
    let src = r#"
@form(vec)
locus Bag {
    params { n: Int = 0; }
    capacity { heap children of Int; }
}
fn main() { Bag { }; }
"#;
    let hits = reserved_diags(src);
    assert_eq!(hits.len(), 1, "capacity slot is reserved: {hits:?}");
    assert!(
        hits[0].0.starts_with("capacity slot `children`:"),
        "names the member kind: {}",
        hits[0].0
    );
}

/// The synthetic members themselves keep working: a locus that
/// `accept`s a child type still iterates and counts `self.children`
/// with no diagnostic at all.
#[test]
fn an_accepting_parent_still_reads_self_children() {
    let src = r#"
locus Child {
    params { id: Int = 0; }
    fn id_of() -> Int { return self.id; }
}

locus Parent {
    params { made: Int = 0; }
    accept(c: Child) { }
    fn spawn(n: Int) {
        Child { id: n };
        self.made = self.made + 1;
    }
    fn total() -> Int {
        let mut sum = 0;
        for c in self.children {
            sum = sum + c.id_of();
        }
        return sum;
    }
    fn how_many() -> Int { return self.children.count; }
}

fn main() {
    let p = Parent { };
    p.spawn(4);
    p.spawn(5);
    println(p.total());
    println(p.how_many());
}
"#;
    let prog = parse_source(src).expect("parse failed");
    let diags = check_program(&prog);
    assert!(
        diags.is_empty(),
        "an accepting parent checks clean: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

/// A `type` is not a locus: it carries no synthetic members, so a
/// struct field named `children` stays legal (the rule is about the
/// locus namespace, not the spelling).
#[test]
fn a_struct_field_named_children_is_untouched() {
    let src = r#"
type Node { children: Int; }
fn main() {
    let n = Node { children: 2 };
    println(n.children);
}
"#;
    assert!(
        reserved_diags(src).is_empty(),
        "type fields are not locus members"
    );
}
