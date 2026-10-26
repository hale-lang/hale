//! Per-instance generic ancestor routing preserves concrete layouts, tracking
//! and the unowned path through a shared producer.
use std::process::Command;
#[path = "support/build.rs"]
mod build_opts;
#[path = "../../hale-types/tests/support/entries.rs"]
mod entries;
#[path = "support/harness.rs"]
mod harness;

const NOMINAL: &str = r#"
locus Child { dissolve() { println("child gone"); } }
locus Maker { fn make() { Child { }; println("made"); } }
locus Owner<T> {
 params { padding: T; label: String; seen: Int = 0; released: Int = 0; maker: Maker = Maker { }; }
 fn act() { self.maker.make(); }
 accept(c: Child) { self.seen = self.seen + 1; println("accept ",self.label," ",self.seen); }
 dissolve() { println("parent ",self.label," ",self.seen," ",self.released); }
}
fn main() {
 let a: Owner<Int> = Owner { padding: 7, label: "int" };
 let b: Owner<Decimal<2>> = Owner { padding: 1.25d, label: "decimal" };
 a.act(); b.act(); a.act();
 let outside = Maker { }; outside.make();
}
"#;
const PARAMETER: &str = r#"
locus Child { dissolve() { println("child gone"); } }
locus Other { }
locus Maker { fn make() { Child { }; println("made"); } }
locus Owner<T> {
 params { label: String; seen: Int = 0; maker: Maker = Maker { }; }
 fn act() { self.maker.make(); }
 accept(c: T) { self.seen = self.seen + 1; println("accept ",self.label," ",self.seen); }
 dissolve() { println("parent ",self.label," ",self.seen); }
}
fn main() { let a: Owner<Child> = Owner { label: "child" }; let b: Owner<Other> = Owner { label: "other" }; a.act(); b.act(); let outside=Maker { }; outside.make(); }
"#;

fn run(source: &str, tag: &str, disabled: bool) -> Vec<String> {
    let program = hale_syntax::parse_source(source).unwrap();
    let diags = entries::check_program(&program);
    assert!(!diags.iter().any(|d| d.is_error()), "{diags:?}");
    let bin = harness::unique_bin(tag);
    let options = hale_codegen::BuildOptions {
        asan: true,
        no_ownership_bubble: disabled,
        ..build_opts::options()
    };
    build_opts::build_source(source, &bin, &options).unwrap();
    let mut command = Command::new(&bin);
    if cfg!(target_os = "macos") {
        command.env("ASAN_OPTIONS", "detect_leaks=0");
    }
    let output = command.output().unwrap();
    let _ = std::fs::remove_file(bin);
    assert!(
        output.status.success(),
        "{:?}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}
fn selected(lines: &[String], prefix: &str) -> Vec<String> {
    lines
        .iter()
        .filter(|s| s.starts_with(prefix))
        .cloned()
        .collect()
}


#[test]
fn different_generic_layouts_share_a_producer_without_crossing_owners() {
    let lines = run(NOMINAL, "generic_owner_layouts", false);
    assert_eq!(
        selected(&lines, "accept "),
        ["accept int 1", "accept decimal 1", "accept int 2"]
    );
    assert_eq!(
        selected(&lines, "parent "),
        ["parent decimal 1 0", "parent int 2 0"]
    );
    assert_eq!(lines.iter().filter(|s| *s == "child gone").count(), 4);
    let control = run(NOMINAL, "generic_owner_disabled", true);
    assert!(selected(&control, "accept ").is_empty());
    assert_eq!(
        selected(&control, "parent "),
        ["parent decimal 0 0", "parent int 0 0"]
    );
    assert_eq!(control.iter().filter(|s| *s == "child gone").count(), 4);
}
#[test]
fn type_parameter_accepts_and_unowned_instances_take_distinct_paths() {
    let lines = run(PARAMETER, "generic_owner_parameter", false);
    assert_eq!(selected(&lines, "accept "), ["accept child 1"]);
    assert_eq!(
        selected(&lines, "parent "),
        ["parent other 0", "parent child 1"]
    );
    assert_eq!(lines.iter().filter(|s| *s == "child gone").count(), 3);
}
#[test]
fn completed_flows_release_into_their_actual_owner_and_leave_no_second_reclaim() {
    let source = flow_source();
    let lines = run(&source, "generic_owner_flow", false);
    assert_eq!(
        selected(&lines, "release "),
        ["release int 1", "release decimal 1", "release int 2"]
    );
    assert_eq!(
        selected(&lines, "parent "),
        ["parent decimal 1 1", "parent int 2 2"]
    );
    assert_eq!(lines.iter().filter(|s| *s == "child gone").count(), 4);
}

fn flow_source() -> String {
    NOMINAL.replace("dissolve() { println(\"child gone\"); }","run() { println(\"child run\"); } dissolve() { println(\"child gone\"); }")
        .replace(" dissolve() { println(\"parent", " release(c: Child) { self.released = self.released + 1; println(\"release \",self.label,\" \",self.released); }\n dissolve() { println(\"parent")
}

#[test]
fn generic_owners_reuse_their_own_recognition_slot_after_flow_completion() {
    for mode in ["fixed_cell", "shared_slab"] {
        let source = flow_source().replace(
            "locus Owner<T> {",
            &format!("locus Owner<T> : projection recognition(cap=1, {mode}) {{"),
        );
        let lines = run(&source, "generic_owner_recognition", false);
        assert_eq!(
            selected(&lines, "accept "),
            ["accept int 1", "accept decimal 1", "accept int 2"]
        );
        assert_eq!(
            selected(&lines, "release "),
            ["release int 1", "release decimal 1", "release int 2"]
        );
        assert_eq!(
            selected(&lines, "parent "),
            ["parent decimal 1 1", "parent int 2 2"]
        );
        assert_eq!(lines.iter().filter(|s| *s == "child gone").count(), 4);
    }
}

#[test]
fn nested_generic_defaults_forward_the_actual_owner_to_a_shared_producer() {
    let source = NOMINAL
        .replace(
            "locus Owner<T> {",
            "locus Middle<T> { params { maker: Maker = Maker { }; } fn make() { self.maker.make(); } }\nlocus Owner<T> {",
        )
        .replace(
            "maker: Maker = Maker { }; }\n fn act()",
            "middle: Middle<T> = Middle { }; }\n fn act()",
        )
        .replace("fn act() { self.maker.make(); }", "fn act() { self.middle.make(); }");
    let lines = run(&source, "generic_owner_nested", false);
    assert_eq!(
        selected(&lines, "accept "),
        ["accept int 1", "accept decimal 1", "accept int 2"]
    );
    assert_eq!(
        selected(&lines, "parent "),
        ["parent decimal 1 0", "parent int 2 0"]
    );
    assert_eq!(lines.iter().filter(|s| *s == "child gone").count(), 4);
}
#[test]
fn value_bindings_keep_the_orphan_alive_until_its_scope_ends() {
    let source = NOMINAL
        .replace(
            "locus Child {",
            "locus Child { params { value: String = std::str::upper(\"heap value\"); } fn ping() { println(self.value); }",
        )
        .replace(
            "Child { }; println(\"made\");",
            "let child = Child { }; child.ping(); println(\"made\");",
        );
    let lines = run(&source, "generic_owner_value", false);
    assert_eq!(lines.iter().filter(|s| *s == "HEAP VALUE").count(), 4);
    assert_eq!(lines.iter().filter(|s| *s == "child gone").count(), 4);
    assert_eq!(
        selected(&lines, "parent "),
        ["parent decimal 1 0", "parent int 2 0"]
    );
}
#[test]
fn constructor_operands_execute_once_in_the_selected_owner_arm() {
    let source=NOMINAL.replace("locus Child {", "locus Child { params { n: Int = 0; }")
        .replace("locus Maker { fn make() { Child { }; println(\"made\"); } }", "locus Maker { params { evaluated: Int = 0; } fn next() -> Int { self.evaluated = self.evaluated + 1; return self.evaluated; } fn make() { Child { n: self.next() }; println(\"made \",self.evaluated); } }");
    let lines = run(&source, "generic_owner_operands", false);
    assert_eq!(
        selected(&lines, "made "),
        ["made 1", "made 1", "made 2", "made 1"]
    );
    assert_eq!(lines.iter().filter(|s| *s == "child gone").count(), 4);
}

const CROSS_POOL: &str = r#"
locus Child { params { n: Int = 1; } birth() { println("child ",self.n); } dissolve() { println("child gone"); } }
locus Maker { run() { Child { }; } }
locus Owner { params { maker: Maker = Maker { }; } accept(c: Child) { println("owner"); } }
main locus App { params { owner: Owner = Owner { }; maker: Maker = Maker { }; seen: Int = 0; } placement { maker: cooperative(pool = workers); } accept(c: Child) { self.seen = self.seen + 1; println("app"); } run() { let mut waited = 0; while self.seen < 1 && waited < 100 { std::time::sleep(50ms); waited = waited + 1; } println("completed ",self.seen); } }
fn main() { App { }; }
"#;

#[test]
fn a_shared_producer_uses_its_local_owner_or_posts_to_its_singleton_owner() {
    for generic in [false, true] {
        let source = if generic {
            CROSS_POOL
                .replace(
                    "locus Owner { params {",
                    "locus Owner<T> { params { padding: T;",
                )
                .replace(
                    "owner: Owner = Owner { }",
                    "owner: Owner<Int> = Owner { padding: 7 }",
                )
        } else {
            CROSS_POOL.to_string()
        };
        let source = source.replace(
            "fn main() { App { }; }",
            "fn main() { App { }; Maker { }; }",
        );
        let lines = run(&source, "owner_alternative_crosspool", false);
        assert_eq!(lines.iter().filter(|s| *s == "owner").count(), 1);
        assert_eq!(lines.iter().filter(|s| *s == "app").count(), 1);
        assert_eq!(lines.iter().filter(|s| *s == "child 1").count(), 3);
        assert_eq!(lines.iter().filter(|s| *s == "child gone").count(), 3);
        assert!(lines.iter().any(|s| s == "completed 1"), "{lines:?}");
    }
}

#[test]
fn alternative_posts_complete_flows_and_reuse_each_owners_recognition_capacity() {
    for mode in ["fixed_cell", "shared_slab"] {
        let source = CROSS_POOL
            .replace("locus Owner {", &format!("locus Owner : projection recognition(cap=1, {mode}) {{"))
            .replace("main locus App {", &format!("main locus App : projection recognition(cap=1, {mode}) {{"))
            .replace("accept(c: Child) { println(\"owner\"); }", "accept(c: Child) { println(\"owner\"); } release(c: Child) { println(\"release owner\"); }")
            .replace("accept(c: Child) { self.seen", "release(c: Child) { println(\"release app\"); } accept(c: Child) { self.seen")
            .replace("run() { Child { }; }", "run() { Child { }; Child { }; }")
            .replace("self.seen < 1", "self.seen < 2");
        let lines = run(&source, "owner_alternative_flow_capacity", false);
        assert_eq!(lines.iter().filter(|s| *s == "owner").count(), 2);
        assert_eq!(lines.iter().filter(|s| *s == "app").count(), 2);
        assert_eq!(lines.iter().filter(|s| *s == "release owner").count(), 2);
        assert_eq!(lines.iter().filter(|s| *s == "release app").count(), 2);
        assert_eq!(lines.iter().filter(|s| *s == "child gone").count(), 4);
        assert!(lines.iter().any(|s| s == "completed 2"), "{lines:?}");
    }
}

#[test]
fn unused_generic_locus_parameters_still_specialize_typed_defaults() {
    let source = CROSS_POOL
        .replace("locus Owner {", "locus Owner<T> {")
        .replace("owner: Owner =", "owner: Owner<Int> =");
    let lines = run(&source, "unused_generic_owner_default", false);
    assert_eq!(lines.iter().filter(|s| *s == "owner").count(), 1);
    assert_eq!(lines.iter().filter(|s| *s == "app").count(), 1);
    assert_eq!(lines.iter().filter(|s| *s == "child gone").count(), 2);
    assert!(lines.iter().any(|s| s == "completed 1"), "{lines:?}");
}

#[test]
fn a_pinned_holder_reached_through_aliases_retains_its_default_child() {
    let source = r#"
locus Child {
    params { value: String = std::str::upper("payload"); }
    birth() { println("child born"); }
    fn peek() -> String { return self.value; }
    dissolve() { println("child gone"); }
}
locus Holder {
    params { child: Child = Child { }; }
    birth() { println("holder born ", self.child.peek()); }
    dissolve() { println("holder gone ", self.child.peek()); }
}
type Held = Holder;
type Alias = Held;
main locus App {
    params { holder: Alias = Holder { }; }
    placement { holder: pinned; }
}
fn main() { App { }; }
"#;
    let lines = run(source, "aliased_pinned_holder_default", false);
    assert_eq!(selected(&lines, "child born"), ["child born"]);
    assert_eq!(selected(&lines, "holder born"), ["holder born PAYLOAD"]);
    assert_eq!(selected(&lines, "holder gone"), ["holder gone PAYLOAD"]);
    assert_eq!(selected(&lines, "child gone"), ["child gone"]);
    assert!(lines.iter().position(|s| s == "holder gone PAYLOAD").unwrap()
        < lines.iter().position(|s| s == "child gone").unwrap(), "{lines:?}");
}
