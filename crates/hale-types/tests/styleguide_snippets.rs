//! Every ```hale snippet in `spec/styleguide.md` must be real code.
//!
//! `spec/styleguide.md` is the most PRESCRIPTIVE document in the repo
//! — it tells people what idiomatic Hale is — and until now nothing
//! checked a line of it. That is how it came to assert, in §7, that
//! `type Option<T> = enum { Some(T), None };` "compiles, constructs,
//! and matches today". It does not construct. Anyone following the
//! styleguide hit an unsupported error, and the claim survived
//! because no gate had ever executed it.
//!
//! ## Why this checks more than the docs harness
//!
//! `docs_snippets.rs` checks that snippets PARSE. That is the right
//! bar for a tutorial full of partial sketches, but parsing is not
//! what the Option claim failed — `type Opt<T> = enum { … };` parses
//! fine and builds fine. It fails at USE.
//!
//! So a complete styleguide snippet must also TYPECHECK. That is the
//! cheapest gate that would have caught the claim, and it catches the
//! whole family: a method that does not exist, an arity that changed,
//! a form shape that no longer validates.
//!
//! ## Info-string conventions
//!
//! - ```hale            — a complete program: must parse AND typecheck
//! - ```hale,fragment   — a partial sketch (a method body, an elided
//!                        locus): skipped, as in the docs harness
//! - ```hale,counter    — deliberately-wrong code shown as an
//!                        anti-pattern. NOT checked, and named
//!                        distinctly from `fragment` so a reader of
//!                        the source can tell "incomplete" from
//!                        "wrong on purpose".
//!
//! mdBook and Starlight both key highlighting off the first token, so
//! rendering is unchanged either way (the `rust,ignore` convention).

#[path = "support/entries.rs"]
mod entries;
use std::path::{Path, PathBuf};

fn styleguide() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("spec")
        .join("styleguide.md")
}

struct Snippet {
    line: usize,
    body: String,
}

fn extract(path: &Path) -> Vec<Snippet> {
    let text = std::fs::read_to_string(path).expect("read styleguide");
    let mut out = Vec::new();
    let mut in_block = false;
    let mut skip = false;
    let mut start = 0usize;
    let mut body = String::new();
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if !in_block {
            if let Some(info) = trimmed.strip_prefix("```") {
                let info = info.trim();
                if info == "hale" || info.starts_with("hale,") {
                    in_block = true;
                    skip = info.contains("fragment") || info.contains("counter");
                    start = i + 1;
                    body.clear();
                }
            }
        } else if trimmed.starts_with("```") {
            in_block = false;
            if !skip {
                out.push(Snippet { line: start, body: body.clone() });
            }
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    out
}

#[test]
fn every_styleguide_snippet_parses_and_typechecks() {
    let path = styleguide();
    let snippets = extract(&path);
    assert!(
        !snippets.is_empty(),
        "no ```hale snippets found in {} — path wiring or the \
         info-string convention broke, and a silently-empty gate is \
         worse than no gate",
        path.display()
    );

    let mut failures = Vec::new();
    for s in &snippets {
        let program = match hale_syntax::parse_source(&s.body) {
            Ok(p) => p,
            Err(e) => {
                failures.push(format!(
                    "spec/styleguide.md:{}: parse: {:?}",
                    s.line, e
                ));
                continue;
            }
        };
        // The bar that matters: a snippet that parses but does not
        // typecheck is exactly the Option-claim failure mode.
        let errs: Vec<String> = entries::check_program(&program)
            .into_iter()
            .filter(|d| d.message.contains("error") || d.is_error())
            .map(|d| d.message)
            .collect();
        if !errs.is_empty() {
            failures.push(format!(
                "spec/styleguide.md:{}: typecheck: {}",
                s.line,
                errs.join("; ")
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} styleguide ```hale snippet(s) are not valid Hale.\n\
         Mark a partial sketch ```hale,fragment and a deliberately-\n\
         wrong anti-pattern ```hale,counter.\n\n{}",
        failures.len(),
        snippets.len(),
        failures.join("\n")
    );
}

// ---- pinning the §7 boundary CLAIMS --------------------------------
//
// An honest note about the test above: it would NOT have caught the
// Option bug. That claim lived in §7 prose with inline code, not in a
// ```hale block, so a snippet gate never saw it.
//
// §7 is where the styleguide makes assertions ABOUT THE LANGUAGE —
// "deliberate absences", "open gaps", "sharp edges". Those are the
// claims most likely to rot, because they describe what does NOT work
// and nobody re-tests absence. When a gap closes, the prose keeps
// saying it is open; when someone believes an absence is a mechanism
// gap rather than an idiom choice, the prose says the opposite of the
// compiler.
//
// So the claims are pinned. Each test asserts the CURRENT truth; if
// the language changes, the test fails and whoever changed it is
// pointed at the styleguide line to update. That is the only
// mechanism that keeps prose and reality together.
//
// The claims that are about CODEGEN limits rather than typecheck
// limits live in `hale-cli/tests/styleguide_claims.rs`, because this
// crate cannot build. That split is itself worth noticing: §7's two
// headline absences — no parametric collections, generic enums do not
// construct — are both invisible to `hale check` and surface only at
// `hale build`. A reader who trusts the checker sees no problem.

fn build_errs(src: &str) -> Vec<String> {
    match hale_syntax::parse_source(src) {
        Err(ds) => ds.into_iter().map(|d| d.message).collect(),
        Ok(p) => entries::check_program(&p)
            .into_iter()
            .map(|d| d.message)
            .collect(),
    }
}

/// §7 sharp edges: "no char-level `s[i]`" — now qualified by the
/// UTF-8 accessors, which must exist for that qualification to hold.
#[test]
fn claim_utf8_accessors_exist() {
    assert!(
        build_errs(
            "fn main() {\n\
                 println(std::str::cp_count(\"a\"));\n\
                 println(std::str::cp_at(\"a\", 0));\n\
                 println(std::str::cp_size(\"a\", 0));\n\
             }"
        )
        .is_empty(),
        "spec/styleguide.md §7 points at cp_count / cp_at / cp_size as \
         the code-point route. They must exist for that to be true."
    );
}

/// C6b: `only:` is the closed form and catches a class the contract
/// never names. If this stops holding, the rule stops being advice
/// and becomes wrong.
#[test]
fn claim_only_catches_an_unnamed_class() {
    let ds = build_errs(
        "effect money;\n\
         @effects(is: { money })\n\
         fn charge(n: Int) -> Int { return n; }\n\
         @effects(only: { alloc })\n\
         fn quote(n: Int) -> Int { return charge(n); }\n\
         fn main() { println(quote(1)); }",
    );
    assert!(
        ds.iter().any(|m| m.contains("money")),
        "styleguide C6b claims `only:` catches classes it never names \
         — that is the entire reason to prefer it over `none:`: {:?}",
        ds
    );
}

/// S13: a chain is legal under a zero-alloc budget. The rule tells
/// people to prefer chains ON HOT PATHS; if that stopped being true
/// the advice would be actively harmful.
#[test]
fn claim_a_chain_is_zero_alloc() {
    let ds = build_errs(
        "@form(vec)\n\
         locus Nums { capacity { heap items of Int; } }\n\
         @budget(alloc_per_call = 0)\n\
         fn big(v: Nums) -> Int { return v.filter(it > 2).count(); }\n\
         fn main() { let v = Nums { }; v.push(5); println(big(v)); }",
    );
    assert!(
        ds.is_empty(),
        "styleguide S13 tells people to use chains on hot paths. A \
         chain must stay legal under @budget(alloc_per_call = 0): {:?}",
        ds
    );
}

/// §7 deliberate absences: "A locus is never a `@form` cell", with
/// the compiler's own reason. A struct from the stdlib is a cell type
/// (C7 and 2.5 say so; the qualified-path refusal is gone).
#[test]
fn claim_a_locus_is_never_a_form_cell() {
    let ds = build_errs(
        "locus Worker { params { id: Int = 0; } }\n\
         @form(vec)\n\
         locus Workers { capacity { heap items of Worker; } }\n\
         fn main() { let w = Workers { }; println(w.len()); }",
    );
    assert!(
        ds.iter().any(|m| m.contains("cell type cannot be a locus")),
        "spec/styleguide.md §7 lists a locus cell as a deliberate \
         absence and quotes its diagnostic. If a locus cell is accepted \
         now, the absence is gone — move or delete the entry: {:?}",
        ds
    );
    let ok = build_errs(
        "@form(vec)\n\
         locus Reqs { capacity { heap items of std::http::Request; } }\n\
         fn main() { let r = Reqs { }; println(r.len()); }",
    );
    assert!(
        ok.is_empty(),
        "spec/styleguide.md 2.5 and C7 say a stdlib struct is a cell \
         type like an in-seed one: {:?}",
        ok
    );
}

/// §7 sharp edges: "An `Int` does not widen into a `Float` let or
/// field", although spec/types.md § "Numeric coercion" lists both.
/// A fix is in progress; when it lands this fails, and the entry goes
/// (spec/types.md is then true again).
#[test]
fn claim_int_does_not_widen_into_a_float_let_or_field() {
    let widens = build_errs(
        "fn half(x: Float) -> Float { return x / 2.0; }\n\
         fn main() { let i = 4; println(half(i), \" \", i + 0.5); }",
    );
    assert!(
        widens.is_empty(),
        "§7 says a call's argument and a mixed binop still widen an \
         `Int`: {:?}",
        widens
    );
    for (what, src) in [
        ("a literal", "fn main() { let x: Float = 5; println(x); }"),
        ("a local", "fn main() { let i = 4; let x: Float = i; println(x); }"),
        (
            "a struct field",
            "type Cfg { timeout: Float; }\n\
             fn main() { let c = Cfg { timeout: 7 }; println(c.timeout); }",
        ),
    ] {
        let ds = build_errs(src);
        assert!(
            ds.iter().any(|m| m.contains("expects `Float`, got `Int`")
                || m.contains("expected `Float`, got `Int`")),
            "spec/styleguide.md §7 says {} of type `Int` is refused where \
             a `Float` is declared. It widens now — delete the \"An `Int` \
             does not widen into a `Float` let or field\" sharp edge: {:?}",
            what,
            ds
        );
    }
}

/// C8: the bare narrowing is refused with the diagnostic the guide
/// quotes, the policy discharges it, and a quantity's count in a unit
/// is the one division that takes none.
#[test]
fn claim_a_narrowing_names_its_policy() {
    let catalogue = "unit cent;\nunit USD = 100 cent;\ntype Money = quantity Int in cent;\n";
    let bare = build_errs(&format!(
        "{catalogue}fn main() {{ let fee: Money = 250cent; println(fee / 2); }}"
    ));
    assert!(
        bare.iter().any(|m| m.contains(
            "`Money` divided by 2 leaves a remainder: say what happens to \
             the remainder: `or floor`, `or <value>`, `or raise`, or give \
             `Money` a `round:` policy"
        )),
        "spec/styleguide.md C8 quotes this refusal word for word; \
         update the quote (or the rule, if it no longer refuses): {:?}",
        bare
    );
    let named = build_errs(&format!(
        "{catalogue}fn main() {{\n\
             let fee: Money = 250cent;\n\
             let count: Int = (fee.in(USD) or half_even) / 1USD;\n\
             println(fee / 2 or floor, \" \", fee / 1USD, \" \", count);\n\
         }}"
    ));
    assert!(
        named.is_empty(),
        "C8: a policy discharges the narrowing, and `fee / 1USD` is an \
         `Int` quotient that takes none. If the quotient now asks for a \
         policy, C8's last paragraph is wrong: {:?}",
        named
    );
}

/// S9: what a `where zero_copy` binding accepts. A scalar-element
/// fixed array is inline and flat; an array of structs is refused.
#[test]
fn claim_zero_copy_takes_inline_scalar_arrays_only() {
    let shape = |field: &str| {
        build_errs(&format!(
            "type P {{ a: Int; }}\n\
             type Pay {{ n: Int; {field} }}\n\
             topic Tk {{ payload: Pay; }}\n\
             main locus App {{\n\
                 bindings {{ Tk: shm_ring(\"/sg-claim\", slot_count: 8, on_overflow: drop) where zero_copy; }}\n\
             }}\n\
             fn main() {{ App {{ }}; }}"
        ))
    };
    let inline = shape("xs: [Int; 4]; ds: [Duration; 2];");
    assert!(
        inline.is_empty(),
        "styleguide S9 and spec/semantics.md § Payload-shape \
         compatibility say a fixed array of scalars is flat: {:?}",
        inline
    );
    for refused in ["ps: [P; 2];", "ts: [Time; 2];", "xs: [Int];", "s: String;"] {
        let ds = shape(refused);
        assert!(
            ds.iter().any(|m| m.contains("is not flat-shapeable")),
            "styleguide S9 says `{}` is refused on a zero_copy binding: {:?}",
            refused,
            ds
        );
    }
}
