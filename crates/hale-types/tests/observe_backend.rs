//! GH #1592 — an observe backend is a param of the main locus,
//! recognized by its type, as a hub is.
//!
//! `metrics: std::observe::Scrape = std::observe::Scrape { port: … };`
//! among the main locus's params: its default is checked as any param's
//! (it may read an earlier param); the compiler numbers it, so a
//! construction that supplies the param is refused; a main locus holds
//! one; the topology artifact carries its endpoint.

#[path = "support/entries.rs"]
mod entries;
use entries::check_program;
use hale_syntax::parse_source;

fn check(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect()
}

fn app(params: &str, main: &str) -> String {
    format!(
        r#"
main locus App {{
    params {{
        port: Int = 9100;
{params}
    }}
    placement {{ metrics: pinned; }}
    run() {{ }}
}}
fn main() {{ {main} }}
"#
    )
}

const SCRAPE: &str = "        metrics: std::observe::Scrape = std::observe::Scrape { port: 9100 };";

#[test]
fn a_scrape_param_checks() {
    for ok in [
        SCRAPE,
        "        metrics: std::observe::Scrape = std::observe::Scrape { port: self.port, path: \"/m\", host: \"0.0.0.0\" };",
        "        metrics: std::observe::Scrape = std::observe::Scrape { port: std::str::parse_int(std::env::var(\"METRICS_PORT\")) or 9100 };",
    ] {
        assert_eq!(check(&app(ok, "App { };")), Vec::<String>::new(), "{ok}");
    }
}

#[test]
fn a_scrape_param_is_the_programs_one() {
    // a second
    let two = format!("{SCRAPE}\n        again: std::observe::Scrape = std::observe::Scrape {{ port: 9101 }};");
    let errs = check(&app(&two, "App { };"));
    assert!(errs.iter().any(|e| e.contains("param `again` is a second `std::observe::Scrape`")), "{errs:?}");
    // a construction that supplies it
    let errs = check(&app(SCRAPE, "App { metrics: std::observe::Scrape { port: 1 } };"));
    assert!(errs.iter().any(|e| e.contains("param `metrics` is the program's observe backend")), "{errs:?}");
    // the port is required, and typed
    let errs = check(&app("        metrics: std::observe::Scrape = std::observe::Scrape { };", "App { };"));
    assert!(errs.iter().any(|e| e.contains("port")), "{errs:?}");
}

#[test]
fn the_topology_artifact_carries_the_endpoint() {
    use std::collections::BTreeMap;
    let dump = |src: &str| {
        let program = parse_source(src).expect("parse");
        let mut programs = BTreeMap::new();
        programs.insert("app.hl".to_string(), &program);
        entries::dump_topology(&hale_types::Bundle::new(programs))
    };
    let a = dump(&app(SCRAPE, "App { };"));
    assert!(
        a.contains(
            "\"observe\": [{\"param\": \"metrics\", \"kind\": \"metrics\", \"backend\": \"std::observe::Scrape\", \
             \"host\": \"127.0.0.1\", \"port\": 9100, \"path\": \"/metrics\"}]"
        ),
        "{a}"
    );
    assert_eq!(hale_types::topology::verify_artifact_digest(&a), Some(true), "the section is digest-covered");
    // a port the program computes is known at run, not here
    let a = dump(&app(
        "        metrics: std::observe::Scrape = std::observe::Scrape { port: self.port, path: \"/m\" };",
        "App { };",
    ));
    assert!(a.contains("\"port\": null, \"path\": \"/m\"}"), "{a}");
    // a program that holds none has no section
    assert!(!dump("main locus App { run() { } }\nfn main() { App { }; }\n").contains("\"observe\""));
}
