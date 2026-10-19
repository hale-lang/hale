//! GH #1477 S1 — a `probes { http(PORT); }` block's port is an `Int`
//! expression of the main locus, checked as a method's body is (it reads
//! as the method `__probes$http`, `self` in scope).

#[path = "support/entries.rs"]
mod entries;

fn diags(src: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(src).expect("parse");
    entries::check_program(&program).into_iter().map(|d| d.message).collect()
}

#[test]
fn the_port_is_an_int_of_the_main_locus() {
    let ok = diags("main locus App {\n    params { port: Int = 9100; }\n    probes { http(self.port); }\n    run() { }\n}\nfn main() { App { }; }\n");
    assert!(ok.is_empty(), "{ok:?}");
    let ds = diags("main locus App {\n    probes { http(\"9100\"); }\n    run() { }\n}\nfn main() { App { }; }\n");
    assert!(!ds.is_empty(), "a String port is refused");
    let ds = diags("main locus App {\n    probes { http(self.nope); }\n    run() { }\n}\nfn main() { App { }; }\n");
    assert!(ds.iter().any(|m| m.contains("nope")), "{ds:?}");
}
