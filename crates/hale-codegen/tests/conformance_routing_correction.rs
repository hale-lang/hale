//! F.40 phase 3, E4: storage routing requires a conformance witness, not
//! a name match (a classified correction).
//!
//! A fresh locus literal inside a fn declared `-> I` for an interface `I`
//! goes to the program-lifetime payload arena when it could be the
//! returned value, since the coercion's fat pointer outlives the fn's
//! frame. The question used to compare method names only, so it also
//! said yes to two literals the checker never lets be returned as `I`:
//! a locus whose methods match `I`'s by name and not by signature, and a
//! generic locus's specialization. Both now stay the frame's; the
//! returned literal, which satisfies `I`, still goes to the arena. What
//! the programs print, and their exit, does not move.

use std::process::Command;

use hale_codegen::build_executable_with_options;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// `Odd.greet` matches `Greeter.greet` by name and not by return type.
const NAME_ONLY: &str = r#"
interface Greeter { fn greet() -> Int; }
locus Hi { params { n: Int = 1; } fn greet() -> Int { return self.n; } }
locus Odd { params { n: Int = 2; } fn greet() -> String { return "x"; } }
fn make() -> Greeter {
    let o = Odd { };
    println(o.n);
    return Hi { n: 4 };
}
fn main() {
    let g = make();
    println(g.greet());
}
"#;

/// `Holder_Int` has `Greeter`'s method by name; no specialization
/// satisfies an interface.
const SPECIALIZATION: &str = r#"
interface Greeter { fn greet() -> Int; }
locus Hi { params { n: Int = 1; } fn greet() -> Int { return self.n; } }
locus Holder<T> { params { v: T; } fn greet() -> Int { return 7; } }
fn make() -> Greeter {
    let h: Holder<Int> = Holder { v: 3 };
    return Hi { n: 4 };
}
fn main() {
    let g = make();
    println(g.greet());
}
"#;

/// The body of the free fn `make`, from its `define` to its closing brace.
fn make_body(tag: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("conf_routing_ir_{tag}"));
    let ir = harness::build_ir_text(&program, &bin).expect("build");
    let _ = std::fs::remove_file(&bin);
    let start = ir.find("define ptr @make(").expect("`make` defined");
    let end = ir[start..].find("\n}").map_or(ir.len(), |i| start + i);
    ir[start..end].to_string()
}

fn run(tag: &str, src: &str) -> (String, Option<i32>) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("conf_routing_run_{tag}"));
    build_executable_with_options(&program, &bin, &[], &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.code())
}

/// Program-lifetime storage for `locus`'s literal is a payload-arena
/// allocation named `%<locus>.self.heap`; the frame's is `%<locus>.self`,
/// an `alloca`.
fn assert_frame_owned(body: &str, locus: &str) {
    assert!(
        body.contains(&format!("%{locus}.self = alloca %locus.{locus}")),
        "{locus}'s literal is the frame's:\n{body}"
    );
    assert!(
        !body.contains(&format!("%{locus}.self.heap = call ptr @lotus_bus_payload_arena_alloc")),
        "{locus}'s literal no longer goes to the payload arena:\n{body}"
    );
}

fn assert_returned_routed(body: &str) {
    assert!(
        body.contains("%Hi.self.heap = call ptr @lotus_bus_payload_arena_alloc"),
        "the returned literal, which satisfies the interface, still goes to the payload arena:\n{body}"
    );
}

/// IR diff against the name comparison: `%Odd.self.heap = call ptr
/// @lotus_bus_payload_arena_alloc(...)` becomes `%Odd.self = alloca
/// %locus.Odd`, and the literal's field and lifecycle addresses follow.
#[test]
fn a_name_only_match_stays_the_frame_s() {
    let body = make_body("name_only", NAME_ONLY);
    assert_frame_owned(&body, "Odd");
    assert_returned_routed(&body);
    assert_eq!(run("name_only", NAME_ONLY), ("2\n4\n".to_string(), Some(0)));
}

/// The same diff for `%Holder_Int.self`.
#[test]
fn a_specialization_stays_the_frame_s() {
    let body = make_body("specialization", SPECIALIZATION);
    assert_frame_owned(&body, "Holder_Int");
    assert_returned_routed(&body);
    assert_eq!(run("specialization", SPECIALIZATION), ("4\n".to_string(), Some(0)));
}
