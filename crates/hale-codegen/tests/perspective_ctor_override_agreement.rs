//! GH #525 item 2 / PR #531 review: check and build must agree on
//! perspective designation at a construction site.
//!
//! The locus case is accepted by the checker AND lowered by
//! `lower_locus_instantiation`, so it builds and routes through the
//! override. The data-type case has no designation path in
//! `populate_user_type_fields`; the checker must refuse it, because
//! a program that checks and then fails to build is the worst
//! failure mode the toolchain has (see corpus_check_build_agreement).

#[path = "../../hale-types/tests/support/entries.rs"]
mod entries;
use std::process::Command;

use hale_syntax::parse_source;
use entries::check_program;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const PERSPECTIVE: &str = r#"
perspective Router {
    fn route(code: Int) -> Int;
}
locus RouterV1 : serves Router {
    fn route(code: Int) -> Int { return code + 100; }
}
locus RouterV2 : serves Router {
    fn route(code: Int) -> Int { return code + 200; }
}
"#;

#[test]
fn locus_field_override_checks_and_builds() {
    let src = format!(
        "{PERSPECTIVE}
locus Gateway {{
    params {{ router: perspective(Router) = RouterV1 {{ }}; }}
    fn handle(code: Int) -> Int {{ return self.router.route(code); }}
}}
main locus App {{
    params {{ gw: Gateway = Gateway {{ router: RouterV2 {{ }} }}; }}
    run() {{ println(self.gw.handle(1)); }}
}}
fn main() {{ App {{ }}; }}
"
    );
    let program = parse_source(&src).expect("parse");
    let diags = check_program(&program);
    assert!(
        diags.iter().all(|d| !d.message.contains("expects `Router`")),
        "checker refused the locus override: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    let bin = harness::unique_bin("hale_test_persp_ctor_locus");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("201"), "override not routed: {:?}", stdout);
}

#[test]
fn data_type_field_override_is_refused_by_the_checker_not_the_build() {
    let src = format!(
        "{PERSPECTIVE}
type Holder {{ router: perspective(Router); }}
fn main() {{
    let holder = Holder {{ router: RouterV2 {{ }} }};
    println(holder.router.route(1));
}}
"
    );
    let program = parse_source(&src).expect("parse");
    let diags = check_program(&program);
    assert!(
        diags.iter().any(|d| d.message.contains("type `Holder`")
            && d.message.contains("field `router` expects `Router`, got `RouterV2`")),
        "checker must refuse a data-type designation (codegen cannot lower it): {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

/// GH #731: a `perspective(P)` field with no initializer holds the slot.
/// The checker lets a construction omit it; the build refuses a program
/// in which no lowered instantiation designates `P`, naming the holder.
#[test]
fn held_slot_with_no_designation_checks_and_is_refused_by_the_build() {
    let src = format!(
        "{PERSPECTIVE}
locus Holder {{
    params {{ router: perspective(Router); }}
    fn handle(code: Int) -> Int {{ return self.router.route(code); }}
}}
main locus App {{
    params {{ h: Holder = Holder {{ }}; }}
    run() {{ println(self.h.handle(1)); }}
}}
fn main() {{ App {{ }}; }}
"
    );
    let program = parse_source(&src).expect("parse");
    let diags = check_program(&program);
    assert!(
        diags.iter().all(|d| !d.message.contains("missing field")),
        "an omitted perspective field is a hold, not a missing field: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    let bin = harness::unique_bin("hale_test_persp_hold_undesignated");
    let err = build_opts::build_source(&src, &bin, &build_opts::options())
        .expect_err("a held slot nothing designates must not build");
    let _ = std::fs::remove_file(&bin);
    let msg = err.to_string();
    assert!(
        msg.contains("perspective `Router`")
            && msg.contains("`Holder.router` holds its slot without designating it")
            && msg.contains("nothing in the program designates it"),
        "the refusal names the perspective and the holder: {msg}"
    );
}

/// GH #731: a holder that calls through the slot before any designation
/// ran stops the program naming the perspective, instead of jumping
/// through an empty slot. Field order is construction order, so `early`
/// is born (and calls) before `router` designates.
#[test]
fn held_slot_called_before_designation_stops_naming_the_perspective() {
    let src = format!(
        "{PERSPECTIVE}
locus Early {{
    params {{ router: perspective(Router); seen: Int = 0; }}
    birth() {{ self.seen = self.router.route(1); }}
}}
main locus App {{
    params {{ early: Early = Early {{ }}; router: perspective(Router) = RouterV1 {{ }}; }}
    run() {{ println(self.early.seen); }}
}}
fn main() {{ App {{ }}; }}
"
    );
    let bin = harness::unique_bin("hale_test_persp_hold_early");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert_eq!(out.status.code(), Some(1), "exit: {:?}", out.status);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("perspective `Router`: `route` was called before anything designated the perspective"),
        "stderr: {stderr}"
    );
}
