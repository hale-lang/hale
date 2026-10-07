//! F.42: a value-returning fn that may violate is
//! `fallible(ClosureViolation)`.
//!
//! The law refuses one that is not, naming the path; a fn that may
//! violate and is `fallible(E)` with another `E` is refused too; a fn
//! returning nothing that may violate is warned about (the lint), and
//! `check` passes with it. "May violate" is the direct reading: a
//! `violate` in the body, or a call to a VALUE-RETURNING violator that is
//! not `fallible`; a call to a method returning nothing carries nothing.
//! Lifecycle bodies, bus handlers and `fn main` are exempt.
//!
//! The stdlib's two such methods, `BytesBuilder.snapshot` and `finish`,
//! carry the declaration, and their callers follow the bare-call law
//! (GH #738) with the violation as the error: the message names
//! `ClosureViolation` (it read `(?)` while the checker resolved the
//! stdlib's signatures without the builtin type), and the `err` an `or`
//! sees at the call is typed.

#[path = "support/entries.rs"]
mod entries;
use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_syntax::parse_source;
use hale_syntax::Diag;
use hale_types::violate_fallible::{fn_decl_rows, may_violate, Declared};

fn diags(src: &str) -> Vec<Diag> {
    let program = parse_source(src).expect("parse");
    entries::check_program(&program)
}

fn errors(src: &str) -> Vec<String> {
    diags(src).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect()
}

/// The law's and the lint's diagnostics, as (is_error, message).
fn verdicts(src: &str) -> Vec<(bool, String)> {
    diags(src)
        .into_iter()
        .filter(|d| d.message.contains("may violate"))
        .map(|d| (d.is_error(), d.message))
        .collect()
}

const LAW: &str = "returns a value and may violate";
const TAIL: &str = "declare it `fallible(ClosureViolation)`; its callers then say what happens when the locus fails";

#[test]
fn a_value_method_with_a_violate_is_refused() {
    let src = r#"
locus Picker {
    params { picked: Int = 0; }
    closure exhausted { epoch inline; }
    fn take() -> Int {
        if self.picked >= 3 { violate exhausted; }
        self.picked = self.picked + 1;
        return self.picked;
    }
}
main locus App {
    params { p: Picker = Picker { }; }
    on_failure(c: Picker, err: ClosureViolation) { }
    run() { println(self.p.take()); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    let law: Vec<&Diag> = ds.iter().filter(|d| d.message.contains("may violate")).collect();
    assert_eq!(law.len(), 1, "{ds:#?}");
    assert!(law[0].is_error());
    assert_eq!(
        law[0].message,
        format!("`Picker.take` {LAW} (`violate exhausted`): {TAIL}")
    );
    // At the name, with the `violate` as its note.
    assert_eq!(&src[law[0].span.start.as_usize()..law[0].span.end.as_usize()], "take");
    assert_eq!(law[0].related.len(), 1);
    assert_eq!(law[0].related[0].label, "`violate exhausted` here");
}

#[test]
fn a_call_to_a_value_violator_carries_it_and_names_the_path() {
    let src = r#"
locus Picker {
    params { picked: Int = 0; }
    closure exhausted { epoch inline; }
    fn take() -> Int {
        if self.picked >= 3 { violate exhausted; }
        return 1;
    }
    fn twice() -> Int {
        let a = self.take();
        return a + 1;
    }
}
fn main() { Picker { }; }
"#;
    let ds = diags(src);
    let via = ds.iter().find(|d| d.message.starts_with("`Picker.twice`")).expect("twice is refused");
    assert!(via.is_error());
    assert_eq!(via.message, format!("`Picker.twice` {LAW} (through `Picker.take`): {TAIL}"));
    let labels: Vec<&str> = via.related.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, ["the call to `Picker.take`", "`violate exhausted` here"]);
    // The call's note is at the call.
    let call = &via.related[0];
    assert_eq!(&src[call.span.start.as_usize()..call.span.end.as_usize()], "self.take()");
}

#[test]
fn a_violation_in_a_method_returning_nothing_is_not_the_caller_s() {
    // `poke` returns nothing: it exits as a return does and its caller
    // goes on to compute its own value. Only the lint, at `poke`.
    let src = r#"
locus Kid {
    params { n: Int = 0; }
    closure fuse { captures: n; epoch inline; }
    fn poke() {
        if self.n > 0 { violate fuse; }
    }
    fn count() -> Int {
        self.poke();
        return self.n;
    }
}
fn main() { Kid { }; }
"#;
    assert_eq!(
        verdicts(src),
        [(
            false,
            format!("`Kid.poke` may violate (`violate fuse`): {TAIL}. This becomes a law in a later release")
        )]
    );
    assert_eq!(errors(src), Vec::<String>::new(), "check passes with the lint");
}

#[test]
fn a_fallible_violation_method_is_accepted_and_its_calls_follow_the_bare_call_law() {
    let src = r#"
locus Picker {
    params { picked: Int = 0; }
    closure exhausted { epoch inline; }
    fn take() -> Int fallible(ClosureViolation) {
        if self.picked >= 3 { violate exhausted; }
        return 1;
    }
    fn twice() -> Int {
        let a = self.take() or 7;
        return a + 1;
    }
}
main locus App {
    params { p: Picker = Picker { }; }
    on_failure(c: Picker, err: ClosureViolation) { }
    run() {
        let n = self.p.take() or { println(err.closure); 0 };
        let m = self.p.take();
        println(n + m);
    }
}
fn main() { App { }; }
"#;
    assert_eq!(verdicts(src), Vec::<(bool, String)>::new());
    let errs = errors(src);
    assert_eq!(errs.len(), 1, "{errs:#?}");
    assert!(
        errs[0].starts_with("`self.p.take` can fail (ClosureViolation) and this call says nothing about it"),
        "{errs:#?}"
    );
}

#[test]
fn a_violator_with_another_error_type_is_refused() {
    let src = r#"
type Busy { why: String; }
locus Picker {
    params { picked: Int = 0; }
    closure exhausted { epoch inline; }
    fn take() -> Int fallible(Busy) {
        if self.picked > 9 { fail Busy { why: "full" }; }
        if self.picked >= 3 { violate exhausted; }
        return 1;
    }
}
fn main() { Picker { }; }
"#;
    let v = verdicts(src);
    assert_eq!(
        v,
        [(
            true,
            "`Picker.take` may violate (`violate exhausted`) and is declared `fallible(Busy)`: a fn that may \
             violate is `fallible(ClosureViolation)`, and a fn has one error type"
                .to_string()
        )]
    );
}

#[test]
fn the_runtime_s_callees_are_exempt() {
    // A lifecycle body, a bus handler that declares a reply type, and
    // `fn main` calling a refused method: the runtime is their caller.
    // Only `take` itself is judged.
    let src = r#"
type Ping { n: Int; }
topic Pings { payload: Ping; subject: "pings"; }
locus Picker {
    params { picked: Int = 0; }
    closure exhausted { epoch inline; }
    bus { subscribe Pings as on_ping; }
    birth() { if self.picked < 0 { violate exhausted; } }
    run() { if self.picked < 0 { violate exhausted; } }
    fn on_ping(p: Ping) -> Int {
        if p.n < 0 { violate exhausted; }
        return p.n;
    }
    fn take() -> Int {
        if self.picked >= 3 { violate exhausted; }
        return 1;
    }
}
main locus App {
    params { p: Picker = Picker { }; }
    run() { Pings <- Ping { n: 1 }; }
}
fn main() {
    let p = Picker { };
    println(p.take());
}
"#;
    let v = verdicts(src);
    assert_eq!(v, [(true, format!("`Picker.take` {LAW} (`violate exhausted`): {TAIL}"))], "{v:#?}");
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// `target`'s check diagnostics, loaded as `hale check <target>` loads
/// it. On a thread of its own: a whole seed's walk is deep.
fn check_target(target: &Path) -> Vec<Diag> {
    let path = target.to_path_buf();
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn_scoped(s, move || {
                let config = Config::check(path.is_dir(), false);
                let snap = Snapshot::load(&path, LoadMode::WholeSeed, &Disk, config).ok().expect("the target loads");
                snap.demand_typing().expect("the typing is not blocked").diags.clone()
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

/// The stdlib's own fns are held to the law here (the check judges a
/// program's fns, and treats the stdlib's analysis copy as a callee only):
/// none returns a value and may violate unless it is
/// `fallible(ClosureViolation)`, and the ones the lint would warn about
/// are the builder's writers and the histogram's `observe`.
#[test]
fn the_stdlib_holds_to_the_law() {
    let target = root().join("tests/hale/violate_in_value_method_test.hl");
    let path = target.clone();
    let (refused, linted, declared) = std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn_scoped(s, move || {
                let snap =
                    Snapshot::load(&path, LoadMode::WholeSeed, &Disk, Config::check(false, false)).ok().expect("loads");
                let summary = snap.demand_alloc_summary().expect("summarized");
                let decls = fn_decl_rows(&[]);
                let may = may_violate(summary, &decls);
                let (mut refused, mut linted, mut declared) = (Vec::new(), Vec::new(), Vec::new());
                for key in may.keys() {
                    let Some(d) = decls.get(key).filter(|d| d.stdlib) else { continue };
                    match (&d.declared, d.returns_value) {
                        (Declared::Violation, _) => declared.push(d.name.clone()),
                        (Declared::Infallible, false) => linted.push(d.name.clone()),
                        _ => refused.push(d.name.clone()),
                    }
                }
                (refused, linted, declared)
            })
            .unwrap()
            .join()
            .unwrap()
    });
    assert_eq!(refused, Vec::<String>::new());
    let mut declared = declared;
    declared.sort();
    assert_eq!(declared, ["std::bytes::BytesBuilder.finish", "std::bytes::BytesBuilder.snapshot"]);
    let mut linted = linted;
    linted.sort();
    assert_eq!(
        linted,
        [
            "std::bytes::BytesBuilder.append",
            "std::bytes::BytesBuilder.append_f32_le",
            "std::bytes::BytesBuilder.append_f64_be",
            "std::bytes::BytesBuilder.append_f64_le",
            "std::bytes::BytesBuilder.append_i16_be",
            "std::bytes::BytesBuilder.append_i16_le",
            "std::bytes::BytesBuilder.append_i32_be",
            "std::bytes::BytesBuilder.append_i32_le",
            "std::bytes::BytesBuilder.append_i64_be",
            "std::bytes::BytesBuilder.append_i64_le",
            "std::bytes::BytesBuilder.append_i8",
            "std::bytes::BytesBuilder.append_pad",
            "std::bytes::BytesBuilder.append_slice",
            "std::bytes::BytesBuilder.append_str",
            "std::bytes::BytesBuilder.append_u16_be",
            "std::bytes::BytesBuilder.append_u16_le",
            "std::bytes::BytesBuilder.append_u32_be",
            "std::bytes::BytesBuilder.append_u32_le",
            "std::bytes::BytesBuilder.append_u64_be",
            "std::bytes::BytesBuilder.append_u64_le",
            "std::bytes::BytesBuilder.append_u8",
            "std::bytes::BytesBuilder.xor_mask",
            "std::metrics::Histogram.observe",
        ]
    );
}

/// What the law and the lint say over the in-tree programs that hold a
/// `violate` (the `tests/hale` programs and the codegen fixtures; the DNA
/// seeds are measured with `hale check --workspace dna`, which warns on
/// `NatsConn.audit` and `NatsFake.audit` and refuses nothing): no
/// refusal, and the lint at the methods returning nothing that violate.
#[test]
fn the_tree_s_violators() {
    let root = root();
    let mut targets: Vec<PathBuf> = Vec::new();
    for dir in ["tests/hale", "crates/hale-codegen/tests/fixtures/failure_delivery", "crates/hale-codegen/tests/fixtures/lifecycle"] {
        for e in std::fs::read_dir(root.join(dir)).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "hl") {
                targets.push(p);
            }
        }
    }
    for e in std::fs::read_dir(root.join("crates/hale-codegen/tests/fixtures/examples")).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            targets.push(p);
        }
    }
    let holds_a_violate = |p: &Path| -> bool {
        let text = |f: &Path| std::fs::read_to_string(f).unwrap_or_default().contains("violate ");
        if p.is_dir() {
            std::fs::read_dir(p).unwrap().any(|f| text(&f.unwrap().path()))
        } else {
            text(p)
        }
    };
    targets.retain(|p| holds_a_violate(p));
    targets.sort();
    let mut found: Vec<String> = Vec::new();
    for t in &targets {
        for d in check_target(t) {
            if d.message.contains("may violate") {
                let shown = t.strip_prefix(&root).unwrap().display().to_string();
                let verdict = if d.is_error() { "refused" } else { "lint" };
                let name = d.message.split('`').nth(1).unwrap_or("?").to_string();
                found.push(format!("{shown}: {verdict} {name}"));
            }
        }
    }
    assert_eq!(
        found,
        [
            "crates/hale-codegen/tests/fixtures/failure_delivery/fd_pool_owner_worker.hl: lint Kid.poke",
            "tests/hale/default_child_supervision_test.hl: lint Boom.check",
            "tests/hale/on_failure_after_params_test.hl: lint Boom.check",
            "tests/hale/on_failure_per_child_type_test.hl: lint Alpha.go",
            "tests/hale/on_failure_per_child_type_test.hl: lint Beta.go",
            "tests/hale/passed_child_supervision_test.hl: lint Boom.check",
        ],
        "over {} programs",
        targets.len()
    );
}
#[test]
fn a_bare_snapshot_names_the_violation_it_can_fail_with() {
    let src = r#"
main locus App {
    run() {
        let b = std::bytes::BytesBuilder { };
        b.append_str("x");
        let s = b.snapshot();
        println(len(s));
    }
}
fn main() { App { }; }
"#;
    let errs = errors(src);
    assert_eq!(errs.len(), 1, "{errs:#?}");
    assert!(
        errs[0].starts_with("`b.snapshot` can fail (ClosureViolation) and this call says nothing about it"),
        "{errs:#?}"
    );
}

#[test]
fn err_at_a_finish_call_is_the_violation() {
    // A handler reads `err.closure` at a `finish()` site: typed, so the
    // read is accepted and a field the record does not have is refused.
    let src = r#"
fn note(e: ClosureViolation) -> Bytes {
    println("builder failed: ", e.closure, " in ", e.locus);
    return std::bytes::from_string("");
}
main locus App {
    run() {
        let b = std::bytes::BytesBuilder { };
        let t = b.finish() or note(err);
        let u = b.snapshot() or { println(err.closure, " ", err.diff); std::bytes::from_string("") };
        let s = b.snapshot() or raise;
        println(len(t) + len(u) + len(s));
    }
}
fn main() { App { }; }
"#;
    assert_eq!(errors(src), Vec::<String>::new());

    let wrong = r#"
main locus App {
    run() {
        let b = std::bytes::BytesBuilder { };
        let u = b.finish() or { println(err.last_error); std::bytes::from_string("") };
        println(len(u));
    }
}
fn main() { App { }; }
"#;
    let errs = errors(wrong);
    assert!(errs.iter().any(|m| m.contains("no field `last_error` on `ClosureViolation`")), "{errs:#?}");
}
