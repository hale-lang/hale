//! The checker / parser items of the 2026-10-12 friction sweep, held at
//! the CLI: what `hale check` says, and that `hale build` agrees.
//!
//! - F.43: the framework vocabulary is contextual. `run`, `epoch`,
//!   `restart`, `capacity`, `bulk` and the rest name fields, params,
//!   locals and fns; a locus method named like a hook is still refused,
//!   with a message that says the name IS the hook.
//! - `or discard` on a value-bearing fallible is legal as a whole
//!   statement (the value is thrown away with the error) and refused
//!   where the value is used, with the default to write instead.
//! - A topic that arrives only through an adapter's inbound side
//!   (`std::bus::__local_dispatch`) is not "subscribed but never
//!   published", at `hale check` or `hale build`; a truly unpublished
//!   one still is.
//! - A topic named after its payload type is a duplicate top-level name
//!   whose message says topics and types share one namespace, and names
//!   the rename.
//! - `@unbounded` / `@bounded_by(<name>)` may precede a `mode`, which
//!   is where the unbounded-allocation warning in a mode body asks for
//!   one; any other decorator there is refused.
//! - A chain terminal with no stage on a fixed array (`xs.sum(0.0d)`)
//!   says to write a chain (`xs.map(it).sum(…)`), and that chain runs.
//! - `const X = 1;` takes the literal's type; a non-literal initializer
//!   without a type says the annotation is required.
//! - A trailing comma in call arguments is accepted.
//! - A `@form(vec)` walked by its declared slot name (`v.cells`) is
//!   refused with a hint naming `.items`, the form's iteration surface.
//! - An interface-typed parameter offers only the interface's methods:
//!   a field read or an undeclared method is refused at the check (it
//!   passed the check and failed the build).

use std::path::{Path, PathBuf};
use std::process::Command;

/// A scratch seed of one file, unique to this process and `tag`.
fn seed(src: &str, tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_friction_sweep_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("main.hl"), src).unwrap();
    d
}

fn hale(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).current_dir(Path::new("/")).output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

/// `hale check` and `hale build` of one source: (check ok, check output,
/// build ok, build output).
fn check_and_build(src: &str, tag: &str) -> (bool, String, bool, String) {
    let d = seed(src, tag);
    let file = d.join("main.hl").to_string_lossy().to_string();
    let (checked, check_out) = hale(&["check", &file]);
    let (built, build_out) = hale(&["build", &file, "-o", &d.join("bin").to_string_lossy()]);
    let _ = std::fs::remove_dir_all(&d);
    (checked, check_out, built, build_out)
}

#[test]
fn the_framework_words_name_fields_params_locals_and_fns() {
    let src = "type Job { run: Int; epoch: Int; restart: Bool = false; }\n\
               fn run(capacity: Int) -> Int { let where = capacity + 1; return where; }\n\
               locus Worker { params { tier: Int = 1; bulk: Int = 2; } fn restart() -> Int { return self.tier + self.bulk; } }\n\
               fn main() { let j = Job { run: 1, epoch: 2 }; let w = Worker { }; println(run(j.run + j.epoch) + w.restart()); }\n";
    let (checked, out, built, bout) = check_and_build(src, "words");
    assert!(checked, "check: {out}");
    assert!(built, "build: {bout}");
}

#[test]
fn a_locus_method_named_like_a_hook_is_refused_by_check_and_build() {
    let src = "locus Worker {\n    fn run() { }\n}\nfn main() { }\n";
    let (checked, out, built, bout) = check_and_build(src, "hook");
    assert!(!checked && out.contains("a locus method cannot be named `run`: that name is the `run` lifecycle hook"), "check: {out}");
    assert!(out.contains("declare the hook as `run() { … }` (no `fn`)"), "the fix: {out}");
    assert!(!built && bout.contains("cannot be named `run`"), "build: {bout}");
}

#[test]
fn or_discard_drops_a_value_in_a_statement_and_is_refused_where_it_is_used() {
    let stmt = "type Oops { why: String = \"\"; }\n\
                fn parse(s: String) -> Int fallible(Oops) { if s == \"bad\" { fail Oops { why: s }; } return 7; }\n\
                fn main() { parse(\"bad\") or discard; parse(\"ok\") or discard; println(\"ok\"); }\n";
    let (checked, out, built, bout) = check_and_build(stmt, "discard_stmt");
    assert!(checked, "check: {out}");
    assert!(built, "build: {bout}");

    let used = "type Oops { why: String = \"\"; }\n\
                fn parse(s: String) -> Int fallible(Oops) { return 7; }\n\
                fn main() { let x = parse(\"1\") or discard; println(x); }\n";
    let (checked, out, built, _) = check_and_build(used, "discard_used");
    assert!(!checked, "a used value is refused: {out}");
    assert!(
        out.contains("`or discard` has no value to give where the result is used: the call succeeds with a `Int`"),
        "{out}"
    );
    assert!(out.contains("Substitute a default (`or <default>`)"), "the fix: {out}");
    assert!(!built, "the build refuses it alike");
}

#[test]
fn an_ingress_counts_as_a_publisher_at_check_and_build() {
    let program = |dispatch: &str| {
        format!(
            "type Note {{ n: Int = 0; }}\n\
             topic In {{ payload: Note; subject: \"in\"; }}\n\
             locus Conn {{ fn send(subject: String, bytes: Bytes) {{ }} fn pump() {{ {dispatch} }} }}\n\
             locus Sink {{ params {{ got: Int = 0; }} bus {{ subscribe In as on_in; }} fn on_in(n: Note) {{ self.got = self.got + 1; }} }}\n\
             main locus App {{ params {{ s: Sink = Sink {{ }}; c: Conn = Conn {{ }}; }} run() {{ self.c.pump(); }} }}\n\
             fn main() {{ App {{ }}; }}\n"
        )
    };
    const WARNING: &str = "bus topic `In` is subscribed but never published";
    let (checked, out, built, bout) =
        check_and_build(&program("std::bus::__local_dispatch(\"in\", b\"x\");"), "ingress");
    assert!(checked && !out.contains(WARNING), "check: {out}");
    assert!(built && !bout.contains(WARNING), "build: {bout}");
    let (checked, out, built, bout) = check_and_build(&program(""), "no_ingress");
    assert!(checked && out.contains(WARNING), "check: {out}");
    assert!(built && bout.contains(WARNING), "build: {bout}");
}

#[test]
fn a_topic_named_after_its_payload_says_the_namespace_is_shared() {
    let src = "type Tick { p: Int; }\ntopic Tick { payload: Tick; }\nfn main() { println(\"ok\"); }\n";
    let (checked, out, built, bout) = check_and_build(src, "topicdup");
    assert!(!checked, "{out}");
    assert!(
        out.contains("duplicate top-level name `Tick`: a topic and its payload type share one namespace, so `Tick` cannot name both"),
        "{out}"
    );
    assert!(out.contains("`topic TickTopic { payload: Tick; … }`"), "the rename: {out}");
    assert!(!built && bout.contains("share one namespace"), "build: {bout}");
    // Another pair names both kinds.
    let src = "type Tick { p: Int; }\nlocus Tick { }\nfn main() { }\n";
    let (checked, out, _, _) = check_and_build(src, "locusdup");
    assert!(!checked && out.contains("`Tick` is declared as a type and as a locus"), "{out}");
}

#[test]
fn a_mode_takes_the_unbounded_acknowledgement_its_warning_asks_for() {
    let program = |deco: &str| {
        format!(
            "@form(vec)\nlocus V {{ capacity {{ heap items of String; }} }}\n\
             locus L {{\n    params {{ v: V = V {{ }}; n: Int = 3; }}\n    {deco}\n    mode bulk() {{\n        let mut i = 0;\n        \
             while i < self.n {{ self.v.push(\"x\" + to_string(i)); i = i + 1; }}\n    }}\n    run() {{ self.bulk(); }}\n}}\n\
             fn main() {{ let l = L {{ }}; }}\n"
        )
    };
    const WARNING: &str = "unbounded allocation: this vec-insert inside an unbounded loop accumulates in `L::bulk`'s region";
    let (checked, out, _, _) = check_and_build(&program(""), "mode_bare");
    assert!(checked && out.contains(WARNING), "{out}");
    assert!(out.contains("`@unbounded` on the enclosing fn, lifecycle hook or mode"), "{out}");
    for deco in ["@unbounded", "@bounded_by(n)"] {
        let (checked, out, built, bout) = check_and_build(&program(deco), "mode_ack");
        assert!(checked && !out.contains("unbounded allocation"), "{deco}: {out}");
        assert!(built, "{deco}: {bout}");
    }
    let (checked, out, _, _) = check_and_build(&program("@hot"), "mode_hot");
    assert!(!checked && out.contains("`@hot` can't precede a mode — a mode takes `@unbounded`"), "{out}");
}

#[test]
fn a_stage_less_chain_on_an_array_says_to_write_a_stage() {
    let src = "locus F {\n    params { ps: [Decimal; 2] = [1.5d, 2.0d]; qs: [Int; 3] = [1, 2, 3]; }\n    run { println(self.ps.sum(0.0d)); println(self.qs.count()); }\n}\nfn main() { let f = F { }; }\n";
    let (checked, out, _, _) = check_and_build(src, "stageless");
    assert!(!checked, "{out}");
    assert!(
        out.contains("no field `sum` on `[Decimal; 2]` — an element chain needs a stage before its terminal: write `….map(it).sum(…)`"),
        "{out}"
    );
    assert!(out.contains("no field `count` on `[Int; 3]` — an element chain needs a stage"), "{out}");
    let fixed = src.replace("self.ps.sum(0.0d)", "self.ps.map(it).sum(0.0d)").replace("self.qs.count()", "self.qs.map(it).count()");
    let (checked, out, built, bout) = check_and_build(&fixed, "stageful");
    assert!(checked, "{out}");
    assert!(built, "{bout}");
}

#[test]
fn a_literal_const_takes_its_literal_type() {
    let src = "const LIMIT = 100;\nconst NEG = -2;\nconst RATE = 0.5;\nconst NAME = \"hale\";\nconst ON = true;\nconst PRICE = 2.50d;\n\
               fn main() { let n: Int = LIMIT + NEG; let f: Float = RATE * 2.0; let s: String = NAME; if ON { println(f\"{n} {f} {s} {PRICE}\"); } }\n";
    let (checked, out, built, bout) = check_and_build(src, "const_lit");
    assert!(checked, "{out}");
    assert!(built, "{bout}");
    let (checked, out, _, _) = check_and_build("const TWICE = 2 * 3;\nfn main() { println(TWICE); }\n", "const_expr");
    assert!(!checked, "{out}");
    assert!(out.contains("`const TWICE` needs its type written: only a literal's type is inferred"), "{out}");
    // A mistyped use is still a type error against the inferred type.
    let (checked, out, _, _) = check_and_build("const LIMIT = 100;\nfn main() { let s: String = LIMIT; println(s); }\n", "const_mistyped");
    assert!(!checked && out.contains("Int"), "{out}");
}

#[test]
fn a_trailing_comma_in_call_arguments_is_accepted() {
    let src = "fn add(a: Int, b: Int) -> Int { return a + b; }\nfn main() {\n    println(add(1, 2,));\n    println(add(\n        3,\n        4,\n    ));\n}\n";
    let (checked, out, built, bout) = check_and_build(src, "trailing_comma");
    assert!(checked, "{out}");
    assert!(built, "{bout}");
    // A lone comma is still not an argument list.
    let (checked, out, _, _) = check_and_build("fn f() { }\nfn main() { f(,); }\n", "lone_comma");
    assert!(!checked, "{out}");
}

#[test]
fn a_vec_walked_by_its_slot_name_is_pointed_at_items() {
    let src = "@form(vec)\nlocus V { capacity { heap cells of Int; } }\nfn main() { let v = V { }; v.push(1); v.push(2); for c in v.cells { println(c); } }\n";
    let (checked, out, _, _) = check_and_build(src, "slot_name");
    assert!(!checked, "{out}");
    assert!(
        out.contains("no field `cells` on `V` — `cells` is `V`'s capacity slot, and a slot is not a field: a `@form(vec)` is iterated as `.items`"),
        "{out}"
    );
    let (checked, out, built, bout) = check_and_build(&src.replace("v.cells", "v.items"), "slot_items");
    assert!(checked, "{out}");
    assert!(built, "{bout}");
}

#[test]
fn an_interface_parameter_offers_only_its_methods() {
    let src = "interface T { fn x() -> Int; }\n\
               locus A { params { v: Int = 1; } fn x() -> Int { return self.v; } }\n\
               fn f(c: T) -> Int { return c.x() + c.v; }\n\
               fn main() { let a = A { }; println(f(a)); }\n";
    let (checked, out, built, bout) = check_and_build(src, "iface_field");
    assert!(!checked, "{out}");
    assert!(
        out.contains("no field `v` on `T` — `T` is an interface: a value of it offers only its methods (`x`), never the fields of the locus behind it"),
        "{out}"
    );
    assert!(!built && bout.contains("no field `v` on `T`"), "the build refuses it at the check alike: {bout}");
    let (checked, out, built, bout) = check_and_build(&src.replace(" + c.v", ""), "iface_method");
    assert!(checked, "{out}");
    assert!(built, "{bout}");
}
