//! Pattern-matching semantics for `std::http::is_route`, pinned
//! against a prefilter that made the matcher ~4x cheaper.
//!
//! `__http_match_pattern_into` used to walk both strings segment by
//! segment before it could reject — re-scanning and allocating a
//! substring twice per segment, ~390ns per candidate route. On a
//! route table nearly every candidate is a miss, so the whole walk
//! was being paid to discover that.
//!
//! It now tests the pattern's literal head as a prefix first, which
//! is implied by the per-segment equality the walk enforces, so it
//! can only reject where the walk would.
//!
//! "Can only reject where the walk would" is the entire safety
//! argument, and the first version of it was WRONG: matching is
//! trailing-slash tolerant on both sides, so pattern `/users/` must
//! match path `/users` — and testing the slash as part of the prefix
//! rejected exactly that pair. Nothing in the suite caught it. These
//! are the cases that would have.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

/// Every case is `(path, pattern, expected)`, run through the real
/// `build_context` + `is_route` surface rather than the internals,
/// so this pins the behaviour a user sees.
const CASES: &[(&str, &str, bool)] = &[
    // Trailing slash is tolerated on BOTH sides. The prefilter's
    // first version broke the first of these and kept the second,
    // which is exactly the asymmetry to guard.
    ("/users", "/users/", true),
    ("/users/", "/users", true),
    ("/users/", "/users/", true),
    ("/users", "/users", true),
    // Captures.
    ("/users/42", "/users/:id", true),
    ("/users/42/", "/users/:id", true),
    ("/u", "/:id", true),
    ("/a/b", "/:x/:y", true),
    // A literal head that PREFIXES the path but is not a segment
    // match: the prefilter passes it and the walk must still reject.
    ("/usersXYZ", "/users", false),
    ("/users/42", "/user/:id", false),
    // Segment-count mismatch, both directions.
    ("/users/42/extra", "/users/:id", false),
    ("/users", "/users/:id", false),
    // The query half is split off before matching.
    ("/users/42?q=1", "/users/:id", true),
    // A miss in the literal head — the case the prefilter exists for.
    ("/orders/42", "/users/:id", false),
];

#[test]
fn route_patterns_match_exactly_as_documented() {
    let checks: String = CASES
        .iter()
        .map(|(path, pat, _)| {
            format!(
                "    t(\"{}\", \"{}\");\n",
                path, pat
            )
        })
        .collect();
    let src = format!(
        r#"
fn t(path: String, pat: String) {{
    let req = std::http::Request {{
        method: "GET", path: path, body: "", conn_fd: 0 - 1
    }};
    let ctx = std::http::build_context(req);
    println(path, " ", pat, " ", std::http::is_route(ctx, "GET", pat));
}}
fn main() {{
{}}}
"#,
        checks
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let bin = harness::unique_bin("hale_test_route_matching");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let got: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(got.len(), CASES.len(), "output:\n{}", stdout);

    let mut wrong = Vec::new();
    for ((path, pat, want), line) in CASES.iter().zip(got.iter()) {
        let actual = line.ends_with("true");
        if actual != *want {
            wrong.push(format!(
                "  {} vs {} -> {} (want {})",
                path, pat, actual, want
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "route matching changed:\n{}",
        wrong.join("\n")
    );
}

/// The method compare rejects before any pattern work, and the
/// captures are cleared on that path — an if-ladder is first-match
/// wins, so a miss must not leave the previous check's captures
/// visible to the next arm.
#[test]
fn a_method_miss_clears_captures() {
    let src = r#"
fn main() {
    let req = std::http::Request {
        method: "GET", path: "/users/42", body: "", conn_fd: 0 - 1
    };
    let ctx = std::http::build_context(req);
    // Matches, filling a capture.
    println("hit=", std::http::is_route(ctx, "GET", "/users/:id"));
    println("id=", std::http::path_param(ctx.params, "id"));
    // Method miss: must clear, so the stale `id` is not readable.
    println("miss=", std::http::is_route(ctx, "POST", "/users/:id"));
    println("after=", std::http::path_param(ctx.params, "id"));
}
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("hale_test_route_clear");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("hit=true"), "{:?}", stdout);
    assert!(stdout.contains("id=42"), "{:?}", stdout);
    assert!(stdout.contains("miss=false"), "{:?}", stdout);
    assert!(
        stdout.contains("after=") && !stdout.contains("after=42"),
        "a method miss must not leave the previous capture readable: {:?}",
        stdout
    );
}
