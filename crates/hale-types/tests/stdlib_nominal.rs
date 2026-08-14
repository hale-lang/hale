//! GH #470: stdlib values carry their real nominal types.
//!
//! The change has two failure directions and both need pins:
//!
//!   - fail-open (the bug): a wrong-arity method coerced to a stdlib
//!     interface unchecked and corrupted memory at the fat-pointer
//!     call — the negative pins here keep that impossible;
//!   - false errors (the risk of the fix): the tightened checker
//!     rejecting CORRECT stdlib use — the positive pins here (and
//!     `tests/hale/router_middleware_test.hl`, which also RUNS the
//!     accepted program) keep the acceptance surface honest.
//!
//! Plus one consistency canary: the literal path keeps a tolerant
//! branch for renamed-but-not-Hale-declared handles. Today that
//! branch is DEAD (every `__Std` rename target is declared in the
//! `.hl` stdlib); the canary makes adding a rename without a
//! declaration a conscious act rather than a silent regression to
//! the old permissiveness.

use hale_syntax::parse_source;

fn errors(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect()
}

const GOOD_MW: &str = "
    locus GoodMw {
        fn before(ctx: std::http::Context) -> std::http::Context {
            return ctx;
        }
        fn after(ctx: std::http::Context, resp: std::http::Response) -> std::http::Response {
            return resp;
        }
    }
";

const BAD_MW: &str = "
    locus BadMw {
        fn before(ctx: std::http::Context) -> std::http::Context {
            return ctx;
        }
        fn after(ctx: std::http::Context) -> std::http::Context {
            return ctx;
        }
    }
";

#[test]
fn the_correct_middleware_contract_is_accepted() {
    let src = format!(
        "{GOOD_MW}
        fn main() {{
            let r = std::http::Router {{}};
            r.use(GoodMw {{}});
        }}"
    );
    assert!(
        errors(&src).is_empty(),
        "the tightened checker must not reject the CORRECT contract: {:?}",
        errors(&src)
    );
}

#[test]
fn the_wrong_arity_middleware_is_refused_with_the_public_spelling() {
    let src = format!(
        "{BAD_MW}
        fn main() {{
            let r = std::http::Router {{}};
            r.use(BadMw {{}});
        }}"
    );
    let errs = errors(&src);
    assert!(
        errs.iter()
            .any(|m| m.contains("after") && m.contains("arity")),
        "wrong-arity `after` must be refused: {:?}",
        errs
    );
    // The diagnostic must speak the user's spelling — a mangled
    // `__StdHttpMiddleware` appears nowhere in their source.
    assert!(
        errs.iter().any(|m| m.contains("std::http::Middleware")),
        "the refusal must name the PUBLIC interface spelling: {:?}",
        errs
    );
}

#[test]
fn stdlib_interfaces_in_user_signatures_coerce_and_refuse() {
    // A USER fn taking a stdlib interface: the same structural
    // verification applies at the user call site.
    let ok = format!(
        "{GOOD_MW}
        fn take(m: std::http::Middleware) {{ }}
        fn main() {{ take(GoodMw {{}}); }}"
    );
    assert!(
        errors(&ok).is_empty(),
        "a satisfying locus must coerce at a user fn site: {:?}",
        errors(&ok)
    );
    let bad = format!(
        "{BAD_MW}
        fn take(m: std::http::Middleware) {{ }}
        fn main() {{ take(BadMw {{}}); }}"
    );
    assert!(
        errors(&bad)
            .iter()
            .any(|m| m.contains("after") && m.contains("arity")),
        "a non-satisfying locus must be refused at a user fn site: {:?}",
        errors(&bad)
    );
}

#[test]
fn stdlib_method_calls_check_name_and_arity() {
    // The tolerance that hid an invalid workspace fixture for
    // months: `Router.get` does not exist (the method is `add`).
    let unknown = "
        fn main() {
            let r = std::http::Router {};
            r.get(\"/\");
        }
    ";
    assert!(
        !errors(unknown).is_empty(),
        "a nonexistent stdlib method must be an error"
    );
    let wrong_arity = "
        fn main() {
            let r = std::http::Router {};
            r.add(\"GET\", \"/\");
        }
    ";
    assert!(
        !errors(wrong_arity).is_empty(),
        "a wrong-arity stdlib method call must be an error"
    );
}

#[test]
fn stdlib_type_annotations_are_real_types() {
    let ok = "
        fn main() {
            let r: std::http::Router = std::http::Router {};
        }
    ";
    assert!(
        errors(ok).is_empty(),
        "a stdlib literal must satisfy its own annotation: {:?}",
        errors(ok)
    );
    let bad = "
        fn main() {
            let x: Int = std::http::Router {};
        }
    ";
    assert!(
        errors(bad)
            .iter()
            .any(|m| m.contains("std::http::Router")),
        "the mismatch must name the stdlib type publicly: {:?}",
        errors(bad)
    );
}

#[test]
fn non_stdlib_qualified_literals_keep_the_historical_tolerance() {
    // Only `std::` roots gained the unknown-name error; other
    // qualified literals (cross-seed shapes with no renames in a
    // single-seed check) stay permissive exactly as before.
    let src = "
        fn main() {
            somelib::Widget {};
        }
    ";
    assert!(
        errors(src).is_empty(),
        "non-std qualified literals must stay permissive: {:?}",
        errors(src)
    );
}

#[test]
fn every_renamed_std_target_is_declared_or_consciously_exempt() {
    // The literal path keeps a tolerant branch for a rename whose
    // target has no Hale-source declaration (a Rust-implemented
    // handle). That branch is UNREACHABLE today; a rename added
    // without a declaration would silently reopen the old
    // permissive typing for that name. Make it a decision instead.
    const EXEMPT: &[&str] = &[];
    let program = hale_syntax::parse_source(hale_stdlib::AP_SOURCE)
        .expect("the bundled stdlib source must parse");
    let mut declared = std::collections::BTreeSet::new();
    for item in &program.items {
        match item {
            hale_syntax::ast::TopDecl::Locus(l) => {
                declared.insert(l.name.name.clone());
            }
            hale_syntax::ast::TopDecl::Type(t) => {
                declared.insert(t.name.name.clone());
            }
            hale_syntax::ast::TopDecl::Interface(i) => {
                declared.insert(i.name.name.clone());
            }
            _ => {}
        }
    }
    let mut undeclared: Vec<String> = Vec::new();
    for (path, target) in hale_stdlib::PATH_RENAMES {
        if target.starts_with("__Std")
            && !declared.contains(*target)
            && !EXEMPT.contains(target)
        {
            undeclared.push(format!("{} -> {}", path.join("::"), target));
        }
    }
    assert!(
        undeclared.is_empty(),
        "renamed __Std targets with no .hl declaration (add the \
         declaration, or add to EXEMPT with a comment explaining \
         why permissive typing is intended): {:?}",
        undeclared
    );
}

#[test]
fn fallible_handle_methods_are_enforced_at_check_time() {
    // The checker twin of codegen's must-address backstop (PR
    // #217): with handles nominal, a bare fallible handle method
    // is a CHECK error, not a codegen surprise.
    let bare = "
        fn work() -> Int fallible(IoError) {
            let f = std::io::file::open(\"/tmp/x\", \"w\") or raise;
            f.write_line(\"hi\");
            return 1;
        }
        fn main() { let n = work() or 0; }
    ";
    assert!(
        errors(bare)
            .iter()
            .any(|m| m.contains("not addressed")),
        "a bare fallible handle method must be refused: {:?}",
        errors(bare)
    );
    let addressed = "
        fn work() -> Int fallible(IoError) {
            let f = std::io::file::open(\"/tmp/x\", \"w\") or raise;
            f.write_line(\"hi\") or raise;
            let line = f.read_line();
            return len(line);
        }
        fn main() { let n = work() or 0; }
    ";
    assert!(
        errors(addressed).is_empty(),
        "the documented File pattern must typecheck: {:?}",
        errors(addressed)
    );
}

#[test]
fn renamed_stdlib_fns_check_against_their_real_signatures() {
    // The surface table's stale fd-era `open -> Int` row used to
    // win; the registered Hale-source wrapper is authoritative now.
    let wrong_arity = "
        fn work() -> Int fallible(IoError) {
            let f = std::io::file::open(\"/tmp/x\") or raise;
            return 1;
        }
        fn main() { let n = work() or 0; }
    ";
    assert!(
        !errors(wrong_arity).is_empty(),
        "open takes (path, mode) — one arg must be refused"
    );
    let nominal_return = "
        fn work() -> Int fallible(IoError) {
            let f = std::io::file::open(\"/tmp/x\", \"w\") or raise;
            let b: Bool = f;
            return 1;
        }
        fn main() { let n = work() or 0; }
    ";
    assert!(
        errors(nominal_return)
            .iter()
            .any(|m| m.contains("std::io::file::File")),
        "open's success type is the File handle, named publicly: {:?}",
        errors(nominal_return)
    );
}
