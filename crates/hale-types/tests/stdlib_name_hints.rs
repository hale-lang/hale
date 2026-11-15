//! GH #1948: a stdlib name written where it is not (`std::str::Builder`,
//! a bare `substring(…)`) is refused with the name that is.

#[path = "support/entries.rs"]
mod entries;
use entries::check_program;
use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

#[test]
fn std_str_builder_points_at_bytes_builder() {
    let ds = diags("fn main() {\n    let b = std::str::Builder {};\n}\n");
    assert!(
        ds.iter().any(|m| m.contains("unknown stdlib type `std::str::Builder`")
            && m.contains("did you mean `std::bytes::BytesBuilder`?")),
        "{:?}",
        ds
    );
}

#[test]
fn a_bare_stdlib_fn_points_at_its_path() {
    let ds = diags("fn main() {\n    let s = substring(\"hello\", 0, 2);\n    println(s);\n}\n");
    assert!(
        ds.iter().any(|m| m.contains("call to `substring`") && m.contains("did you mean `std::str::substring`?")),
        "{:?}",
        ds
    );
}

#[test]
fn a_type_in_the_wrong_namespace_points_at_the_one_of_its_name() {
    let ds = diags("fn main() {\n    let b = std::str::BytesBuilder {};\n}\n");
    assert!(
        ds.iter().any(|m| m.contains("did you mean `std::bytes::BytesBuilder`?")),
        "{:?}",
        ds
    );
}
