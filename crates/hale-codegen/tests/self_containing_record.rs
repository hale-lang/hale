//! A record that holds its own kind: refused before lowering when it
//! holds it by value, built and run when it holds it through a
//! `bounded` or an enum.
//!
//! By value (`type Cfg { child: Cfg = Cfg { }; }`) every `Cfg` holds
//! another, and lowering expanded the default until the compiler's stack
//! ran out (downstream handoff). The harness's build skips the check, so
//! it is the harness's lowering view that demands the law
//! (`self_containing_record` in `hale_types::lowering_laws`), and these
//! tests pin that it refuses before lowering starts. A regression does
//! not fail politely: it aborts this test process on a stack overflow.
//!
//! Through a `bounded[Cfg; N]` or an enum variant the type has a floor
//! and is accepted, but every deep copy of one (a push, a return, a
//! store into a locus field, a copy) unrolled the copy by type, around
//! the ring, and overflowed the stack the same way. A deep copy of such a
//! type is now a function of its own (`emit_outlined_deep_copy`), and
//! these programs run.

use std::process::Command;

use hale_codegen::CodegenError;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn build_err(name: &str, source: &str) -> String {
    let bin = harness::unique_bin(&format!("hale_test_selfrecord_{name}"));
    let out = build_opts::build_source(source, &bin, &build_opts::options());
    let _ = std::fs::remove_file(&bin);
    match out {
        Ok(_) => panic!("{name}: expected the self-containing record law to refuse it"),
        Err(CodegenError::Unsupported(msg)) => msg,
        Err(e) => panic!("{name}: expected the law's refusal, got {e:?}"),
    }
}

fn build_and_run(name: &str, source: &str) -> String {
    let bin = harness::unique_bin(&format!("hale_test_selfrecord_{name}"));
    if let Err(e) = build_opts::build_source(source, &bin, &build_opts::options()) {
        panic!("{name}: build: {e}");
    }
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(output.status.success(), "{name}: {:?}\n{}", output.status, String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn the_harness_refuses_a_record_holding_itself_by_value() {
    for (name, src, ring) in [
        (
            "direct",
            "type Cfg { limit: String = \"\"; has_child: Bool = false; child: Cfg = Cfg { }; }\n\
             fn main() { let c = Cfg { limit: \"1\" }; println(c.limit); }\n",
            "`Cfg.child: Cfg`",
        ),
        (
            "ring",
            "type A { b: B = B { }; }\ntype B { a: A = A { }; }\nfn main() { let a = A { }; println(\"x\"); }\n",
            "the ring `A.b: B` → `B.a: A`",
        ),
    ] {
        let msg = build_err(name, src);
        assert!(msg.contains("a record cannot contain itself by value") && msg.contains(ring), "{name}: {msg}");
    }
}

const BOUNDED: &str = "type Cfg { limit: String = \"\"; kids: bounded[Cfg; 4]; }\n";
const ENUM: &str = "type Cfg { limit: String = \"\"; child: Sub = Sub::Nothing; }\n\
                    type Sub = enum { Nothing, One(Cfg) };\n";

#[test]
fn a_record_holding_itself_through_a_bounded_is_copied_at_every_depth() {
    let cases = [
        (
            "push_nested",
            "fn main() {\n    let mut c = Cfg { limit: \"1\" };\n    let mut k = Cfg { limit: \"2\" };\n    \
             push(k.kids, Cfg { limit: \"3\" }) or { return; };\n    push(c.kids, k) or { return; };\n    \
             let g = at(c.kids, 0) or Cfg { };\n    let gg = at(g.kids, 0) or Cfg { };\n    println(gg.limit);\n}\n",
            "3\n",
        ),
        (
            "returned",
            "fn mk() -> Cfg {\n    let mut c = Cfg { limit: \"r\" };\n    push(c.kids, Cfg { limit: \"k\" }) or { return c; };\n    \
             return c;\n}\nfn main() { let c = mk(); let k = at(c.kids, 0) or Cfg { }; println(c.limit, k.limit); }\n",
            "rk\n",
        ),
        (
            "locus_field",
            "locus H {\n    params { c: Cfg = Cfg { }; }\n    run { push(self.c.kids, Cfg { limit: \"k\" }) or { return; }; \
             println(count(self.c.kids)); }\n}\nfn main() { H { }; }\n",
            "1\n",
        ),
        (
            "set_and_copy",
            "fn main() {\n    let mut c = Cfg { };\n    push(c.kids, Cfg { limit: \"a\" }) or { return; };\n    \
             set(c.kids, 0, Cfg { limit: \"b\" }) or { return; };\n    let d = c;\n    for k in d.kids { println(k.limit); }\n}\n",
            "b\n",
        ),
    ];
    for (name, body, want) in cases {
        assert_eq!(build_and_run(&format!("bounded_{name}"), &format!("{BOUNDED}{body}")), want, "{name}");
    }
}

#[test]
fn a_record_holding_itself_through_an_enum_is_copied_at_every_depth() {
    let cases = [
        (
            "returned",
            "fn mk() -> Cfg { return Cfg { limit: \"r\", child: Sub::One(Cfg { limit: \"k\" }) }; }\n\
             fn main() { let c = mk(); match c.child { Sub::One(k) -> { println(k.limit); }, _ -> { println(\"none\"); } } }\n",
            "k\n",
        ),
        (
            "locus_field",
            "locus H {\n    params { c: Cfg = Cfg { }; }\n    run { self.c = Cfg { limit: \"x\", child: Sub::One(Cfg { limit: \"y\" }) }; \
             println(self.c.limit); }\n}\nfn main() { H { }; }\n",
            "x\n",
        ),
    ];
    for (name, body, want) in cases {
        assert_eq!(build_and_run(&format!("enum_{name}"), &format!("{ENUM}{body}")), want, "{name}");
    }
}
