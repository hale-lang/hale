//! Typecheck M3 stage 1 (2026-07-02): stdlib fn-name validation.
//! Within a tabled namespace an unknown name is an error with a
//! did-you-mean; untabled namespaces stay permissive; locus paths
//! and valid names never flag.

use hale_syntax::parse_source;
use hale_types::check_program;

fn msgs(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

#[test]
fn typo_in_tabled_namespace_is_caught_with_suggestion() {
    let m = msgs(
        r#"
        fn main() {
            let n = std::str::parse_itn("42") or 0;
            println(n);
        }
    "#,
    );
    assert!(
        m.iter().any(|s| s.contains("unknown stdlib function")
            && s.contains("std::str::parse_itn")
            && s.contains("did you mean `std::str::parse_int`")),
        "got: {:?}",
        m
    );
}

#[test]
fn valid_names_do_not_flag() {
    let m = msgs(
        r#"
        fn main() {
            let n = std::str::parse_int("42") or 0;
            let f = std::math::sqrt(4.0);
            let t = std::time::monotonic_ns();
            let b = std::bytes::from_string("x");
            let r = std::io::fs::read_file("/dev/null") or "";
            println(n, f, t, b, r);
        }
    "#,
    );
    assert!(
        !m.iter().any(|s| s.contains("unknown stdlib function")),
        "got: {:?}",
        m
    );
}

#[test]
fn untabled_namespace_stays_permissive() {
    // std::io::sockopt dispatches non-literal names (constant table)
    // — deliberately untabled, so no name errors even for nonsense.
    let m = msgs(
        r#"
        fn main() {
            let v = std::io::sockopt::TOTALLY_MADE_UP();
            println(v);
        }
    "#,
    );
    assert!(
        !m.iter().any(|s| s.contains("unknown stdlib function")),
        "got: {:?}",
        m
    );
}

#[test]
fn locus_paths_never_flag() {
    let m = msgs(
        r#"
        fn main() {
            let b = std::bytes::BytesBuilder { };
            let _ = b;
        }
    "#,
    );
    assert!(
        !m.iter().any(|s| s.contains("unknown stdlib function")),
        "got: {:?}",
        m
    );
}
