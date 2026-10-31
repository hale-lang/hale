//! GH #1477 S1 — the runtime's probes are a param of the main locus of
//! type `std::observe::Probes`, recognized by its type: one, on the main
//! locus, built by its default (spec/semantics.md § Probes).

#[path = "support/entries.rs"]
mod entries;

/// The probes are the main locus's one param, built by its default:
/// another locus declaring them, a second one, and a construction of the
/// main locus that supplies them are refused.
#[test]
fn the_probes_are_one_param_of_the_main_locus() {
    let check = |src: &str| {
        let program = hale_syntax::parse_source(src).expect("parse");
        entries::check_program(&program).into_iter().map(|d| d.message).collect::<Vec<_>>()
    };
    let ds = check(
        "locus W {\n    params { probes: std::observe::Probes = std::observe::Probes { port: 1 }; }\n}\n\
         main locus App { params { w: W = W { }; } }\nfn main() { App { }; }\n",
    );
    assert!(ds.iter().any(|m| m.contains("which is not the main locus")), "{ds:?}");
    let ds = check(
        "main locus App {\n    params {\n        a: std::observe::Probes = std::observe::Probes { port: 1 };\n        \
         b: std::observe::Probes = std::observe::Probes { port: 2 };\n    }\n}\nfn main() { App { }; }\n",
    );
    assert!(ds.iter().any(|m| m.contains("declares one `std::observe::Probes`")), "{ds:?}");
    let ds = check(
        "main locus App {\n    params { probes: std::observe::Probes = std::observe::Probes { port: 1 }; }\n}\n\
         fn main() { App { probes: std::observe::Probes { port: 2 } }; }\n",
    );
    assert!(ds.iter().any(|m| m.contains("is the process's probes")), "{ds:?}");
    let ds = check(
        "main locus App {\n    params {\n        port: Int = 9100;\n        \
         probes: std::observe::Probes = std::observe::Probes { port: self.port };\n    }\n}\nfn main() { App { }; }\n",
    );
    assert!(ds.is_empty(), "the port is a param default like any other: {ds:?}");
}
