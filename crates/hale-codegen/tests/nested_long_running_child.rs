//! Integration tests for the nested-long-running-child antipattern
//! diagnostic. Closes the std::http::Server-blocks-parent friction
//! by detecting the shape at typecheck and pointing at the
//! canonical sibling-in-main + placement fix.

fn typecheck_diags(source: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert("main".to_string(), &program);
    let bundle = hale_types::Bundle { programs };
    let (scope, _) = hale_types::resolve::build_top_scope(&bundle);
    let diags = hale_types::check::check_bundle(&bundle, &scope, true);
    diags.into_iter().map(|d| d.message).collect()
}

#[test]
fn nested_user_locus_with_run_inside_parent_with_run_is_rejected() {
    // The textbook antipattern: a non-main parent holds a child
    // whose run() doesn't terminate, and the parent has its own
    // run() that wants to do work. Substrate puts both on the
    // same OS thread.
    let src = r#"
        locus Worker {
            run() {
                std::time::sleep(1m);
            }
        }
        locus Parent {
            params {
                w: Worker = Worker { };
            }
            run() {
                std::time::sleep(100ms);
            }
        }
        fn main() { Parent { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("Parent")
            && m.contains("Worker")
            && m.contains("placement")
            && m.contains("siblings")),
        "expected antipattern diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn nested_std_http_server_inside_parent_with_run_is_rejected() {
    // The specific shape the friction names. We can't see std's
    // body but the known-long-running stdlib allowlist covers it.
    let src = r#"
        locus Gateway {
            params {
                metrics: std::http::Server = std::http::Server { port: 9100 };
            }
            run() {
                std::time::sleep(100ms);
            }
        }
        fn main() { Gateway { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("Gateway")
            && m.contains("std::http::Server")
            && m.contains("placement")),
        "expected antipattern diagnostic naming std::http::Server, \
         got: {:?}",
        diags
    );
}

#[test]
fn parent_with_no_run_body_is_not_flagged() {
    // If the parent has no run() (or an empty run()), there's
    // nothing to block — the child can hog the thread without
    // starving anyone.
    let src = r#"
        locus Worker {
            run() {
                std::time::sleep(1m);
            }
        }
        locus Holder {
            params {
                w: Worker = Worker { };
            }
            birth() {
                println("constructed");
            }
        }
        fn main() { Holder { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        !diags.iter().any(|m| m.contains("nested cooperative")),
        "no antipattern should fire when parent has no run(), \
         got: {:?}",
        diags
    );
}

#[test]
fn child_with_no_run_body_is_not_flagged() {
    // If the child has no run() body, it can't block — the
    // canonical Hale parent-child shape (child via accept() +
    // bus, no run() on the child) is unaffected.
    let src = r#"
        locus Worker {
            params {
                tag: String = "w";
            }
        }
        locus Parent {
            params {
                w: Worker = Worker { };
            }
            run() {
                std::time::sleep(100ms);
            }
        }
        fn main() { Parent { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        !diags.iter().any(|m| m.contains("nested cooperative")),
        "no antipattern should fire when child has no run(), \
         got: {:?}",
        diags
    );
}

#[test]
fn main_locus_with_long_running_children_is_not_flagged() {
    // The canonical sibling-in-main pattern: main locus holds
    // both, placement separates them. The check explicitly
    // skips `main locus` because it's the canonical fix.
    let src = r#"
        locus Worker {
            run() {
                std::time::sleep(1m);
            }
        }
        main locus App {
            params {
                w: Worker = Worker { };
                metrics: std::http::Server = std::http::Server { port: 9100 };
            }
            placement {
                metrics: cooperative(pool = io);
            }
        }
        fn main() { App { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        !diags.iter().any(|m| m.contains("nested cooperative")),
        "no antipattern should fire on main locus siblings, \
         got: {:?}",
        diags
    );
}
