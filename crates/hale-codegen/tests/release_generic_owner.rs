//! A `release(c: T)` clause whose `T` is its owner's type parameter
//! makes the specialization's argument a flow (outside review of
//! #1295, finding 1). The flow row is surveyed over the merged program,
//! where `Manager<T>`'s clause names no declared locus; lowering
//! creates `Manager<Worker>` later, in its generic-instantiation queue,
//! and asks the row whether `Worker` is a flow. The row answers for the
//! specialization through the template's clause with lowering's own
//! substitution applied, so the release hook runs once, at the child's
//! `run()` end, and the child is reclaimed there — before the owner's
//! next statement — exactly as for a concrete owner.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn run(name: &str, src: &str) -> Vec<String> {
    let bin = harness::unique_bin(name);
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "{name}: exit {:?}\n{}", out.status, String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect()
}

const WORKER: &str = r#"
locus Worker {
    run() { println("WORKER_RUN"); }
    dissolve() { println("WORKER_DISSOLVE"); }
}
"#;

/// A flow child under its owner: the hook, then the child's dissolve,
/// both before the owner's next statement.
const FLOW: [&str; 5] = ["WORKER_RUN", "RELEASE", "WORKER_DISSOLVE", "AFTER_WORKER", "MANAGER_DISSOLVE"];

#[test]
fn a_concrete_owner_releases_its_flow_child() {
    let src = format!(
        "{WORKER}
locus Manager {{
    accept(c: Worker) {{ }}
    release(c: Worker) {{ println(\"RELEASE\"); }}
    run() {{ Worker {{ }}; println(\"AFTER_WORKER\"); }}
    dissolve() {{ println(\"MANAGER_DISSOLVE\"); }}
}}
fn main() {{ let m: Manager = Manager {{ }}; }}
"
    );
    assert_eq!(run("hale_test_release_concrete_owner", &src), FLOW);
}

#[test]
fn a_generic_owner_releases_its_argument() {
    let src = format!(
        "{WORKER}
locus Manager<T> {{
    accept(c: T) {{ }}
    release(c: T) {{ println(\"RELEASE\"); }}
    run() {{ Worker {{ }}; println(\"AFTER_WORKER\"); }}
    dissolve() {{ println(\"MANAGER_DISSOLVE\"); }}
}}
fn main() {{ let m: Manager<Worker> = Manager {{ }}; }}
"
    );
    assert_eq!(run("hale_test_release_generic_owner", &src), FLOW);
}

/// An alias as the argument names the locus it resolves to.
#[test]
fn an_aliased_argument_names_its_locus() {
    let src = format!(
        "{WORKER}
type Job = Worker;
locus Manager<T> {{
    accept(c: T) {{ }}
    release(c: T) {{ println(\"RELEASE\"); }}
    run() {{ Worker {{ }}; println(\"AFTER_WORKER\"); }}
    dissolve() {{ println(\"MANAGER_DISSOLVE\"); }}
}}
fn main() {{ let m: Manager<Job> = Manager {{ }}; }}
"
    );
    assert_eq!(run("hale_test_release_aliased_argument", &src), FLOW);
}

/// Two specializations of one template: each makes its own argument a
/// flow, and each owner's own hook runs once, over its own child.
#[test]
fn two_specializations_release_their_own_arguments() {
    let src = r#"
locus Alpha {
    run() { println("ALPHA_RUN"); }
    dissolve() { println("ALPHA_DISSOLVE"); }
}
locus Beta {
    run() { println("BETA_RUN"); }
    dissolve() { println("BETA_DISSOLVE"); }
}
locus Pool<T> {
    params { which: Int = 0; }
    accept(c: T) { }
    release(c: T) { println("RELEASE ", self.which); }
    run() {
        if self.which == 1 { Alpha { }; } else { Beta { }; }
        println("AFTER ", self.which);
    }
}
fn main() {
    let a: Pool<Alpha> = Pool { which: 1 };
    let b: Pool<Beta> = Pool { which: 2 };
}
"#;
    assert_eq!(
        run("hale_test_release_two_specializations", src),
        [
            "ALPHA_RUN", "RELEASE 1", "ALPHA_DISSOLVE", "AFTER 1",
            "BETA_RUN", "RELEASE 2", "BETA_DISSOLVE", "AFTER 2",
        ]
    );
}

/// A generic child as the argument: `T → Cell<Int>` names the
/// monomorph lowering creates for `Cell<Int>`.
#[test]
fn a_generic_child_argument_is_a_flow() {
    let src = r#"
locus Cell<T> {
    params { n: Int = 0; }
    run() { println("CELL_RUN"); }
    dissolve() { println("CELL_DISSOLVE"); }
}
locus Manager<T> {
    accept(c: T) { }
    release(c: T) { println("RELEASE"); }
    run() {
        let c: Cell<Int> = Cell { n: 1 };
        println("AFTER_CELL");
    }
    dissolve() { println("MANAGER_DISSOLVE"); }
}
fn main() { let m: Manager<Cell<Int>> = Manager { }; }
"#;
    assert_eq!(
        run("hale_test_release_generic_child", src),
        ["CELL_RUN", "RELEASE", "CELL_DISSOLVE", "AFTER_CELL", "MANAGER_DISSOLVE"]
    );
}
