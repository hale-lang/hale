//! A statement `or` builds no value.
//!
//! `x.pop() or 0;` throws the success value away, so the IR the
//! statement form emits has nothing to merge: no `or.result` phi at the
//! join, and no load of the success slot on the ok branch. A dead phi of
//! exactly this shape sat twice in a pool's teardown (`while
//! !self.fds.is_empty() { self.fds.pop() or 0; }`). The same call bound
//! by a `let` still merges both arms, and still runs: the statement form
//! pops, the bound form reads what it popped.

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const SRC: &str = r#"
@form(vec)
locus Ints { capacity { heap items of Int; } }
locus Pool {
    params { fds: Ints = Ints { }; }
    fn empty_all() { while !self.fds.is_empty() { self.fds.pop() or 0; } }
    fn last_or_zero() -> Int { let n = self.fds.pop() or 0; return n; }
}
fn main() {
    let p = Pool { };
    p.fds.push(3);
    p.fds.push(4);
    println("last=", p.last_or_zero());
    p.empty_all();
    println("len=", p.fds.len());
}
"#;

/// The IR of the method named `name`, before optimization.
fn method_ir<'a>(ir: &'a str, name: &str) -> Vec<&'a str> {
    let start = ir
        .lines()
        .position(|l| l.starts_with("define") && l.contains(&format!("Pool.{name}(")))
        .unwrap_or_else(|| panic!("no `Pool.{name}` in the module"));
    ir.lines().skip(start).take_while(|l| *l != "}").collect()
}

#[test]
fn a_statement_or_merges_no_value_and_a_bound_one_does() {
    let bin = harness::unique_bin("or_statement_value");
    let ir = harness::build_source_ir_text(SRC, &bin).expect("lowers");
    let out = std::process::Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(stdout.contains("last=4") && stdout.contains("len=0"), "{stdout}");

    let stmt = method_ir(&ir, "empty_all");
    assert!(stmt.iter().any(|l| l.starts_with("or.join")), "the statement still branches:\n{}", stmt.join("\n"));
    let merged: Vec<_> = stmt.iter().filter(|l| l.contains("or.result") || l.contains("or.ok.val")).collect();
    assert!(merged.is_empty(), "a statement `or` built a value: {merged:#?}");

    let bound = method_ir(&ir, "last_or_zero");
    assert!(
        bound.iter().any(|l| l.contains("%or.result = phi i64")),
        "a bound `or` merges both arms:\n{}",
        bound.join("\n")
    );
}
