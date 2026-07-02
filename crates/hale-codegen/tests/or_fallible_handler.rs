//! 2026-07-02 fallible handlers: `call() or handler(err)` where the
//! handler is itself `fallible(E2)`. Semantics: the handler's success
//! value substitutes; its FAILURE propagates through the enclosing
//! fn's error path (implicit `or raise` — sugar for the already-legal
//! `call() or (handler(err) or raise)`). Closes the pond stash-bridge
//! idiom that made jobs::Queue non-reentrant (DbError→JobError
//! conversion couldn't `fail` from inside an `or` clause).

use std::process::Command;

use hale_codegen::build_executable;
use hale_syntax::parse_source;
use hale_types::check_program;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    static NEXT: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    bin.push(format!(
        "hale_or_fallible_{}_{}_{}",
        name,
        std::process::id(),
        n
    ));
    build_executable(&program, &bin).expect("build");
    bin
}

const CONVERT_CHAIN: &str = r#"
    type DbErr { code: Int; }
    type JobErr { msg: Int; }

    fn dbget(k: Int) -> Int fallible(DbErr) {
        if k < 0 { fail DbErr { code: 7 }; }
        return k * 10;
    }
    fn convert(e: DbErr) -> Int fallible(JobErr) {
        if e.code > 5 { fail JobErr { msg: e.code + 100 }; }
        return 0;
    }
    fn fetch(k: Int) -> Int fallible(JobErr) {
        let v = dbget(k) or convert(err);
        return v;
    }
    fn probe() -> Int fallible(JobErr) {
        // convert(code 3) succeeds -> its value (0) substitutes.
        let w = dbget(0 - 1) or convert(DbErr { code: 3 });
        return w;
    }

    fn main() {
        let b = fetch(0 - 2) or (0 - err.msg);
        println("b=", b);
        let a = fetch(4) or (0 - err.msg);
        println("a=", a);
        let w = probe() or (0 - 99);
        println("w=", w);
    }
"#;

#[test]
fn fallible_handler_propagates_and_substitutes() {
    let bin = build("chain", CONVERT_CHAIN);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // DbErr{code:7} → convert fails → JobErr{msg:107} propagates
    // through fetch's error path; outer handler reads err.msg.
    assert!(stdout.contains("b=-107"), "got: {:?}", stdout);
    // Success path untouched.
    assert!(stdout.contains("a=40"), "got: {:?}", stdout);
    // Handler SUCCESS value substitutes.
    assert!(stdout.contains("w=0"), "got: {:?}", stdout);
}

// Same chain as a LOCUS METHOD handler (`self.convert(err)`) — the
// exact pond jobs::Queue shape.
const METHOD_CHAIN: &str = r#"
    type DbErr { code: Int; }
    type JobErr { msg: Int; }

    locus Q {
        params { n: Int = 0; }
        fn dbget(k: Int) -> Int fallible(DbErr) {
            if k < 0 { fail DbErr { code: 7 }; }
            return k * 10;
        }
        fn convert(e: DbErr) -> Int fallible(JobErr) {
            if e.code > 5 { fail JobErr { msg: e.code + 100 }; }
            return 0;
        }
        fn fetch(k: Int) -> Int fallible(JobErr) {
            let v = self.dbget(k) or self.convert(err);
            return v;
        }
    }

    fn main() {
        let q = Q { };
        let b = q.fetch(0 - 2) or (0 - err.msg);
        println("b=", b);
        let a = q.fetch(4) or (0 - err.msg);
        println("a=", a);
    }
"#;

#[test]
fn fallible_method_handler_on_self() {
    let bin = build("method", METHOD_CHAIN);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("b=-107"), "got: {:?}", stdout);
    assert!(stdout.contains("a=40"), "got: {:?}", stdout);
}

fn check_msgs(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

#[test]
fn fallible_handler_outside_fallible_fn_is_rejected() {
    let msgs = check_msgs(
        r#"
        type DbErr { code: Int; }
        type JobErr { msg: Int; }
        fn dbget(k: Int) -> Int fallible(DbErr) { return k; }
        fn convert(e: DbErr) -> Int fallible(JobErr) { return 0; }
        fn notfallible(k: Int) -> Int {
            let v = dbget(k) or convert(err);
            return v;
        }
        fn main() { println(notfallible(1)); }
    "#,
    );
    assert!(
        msgs.iter().any(|m| m.contains("nowhere to go")),
        "expected the targeted enclosing-not-fallible diagnostic; got: {:?}",
        msgs
    );
}

#[test]
fn fallible_handler_payload_mismatch_is_rejected() {
    let msgs = check_msgs(
        r#"
        type DbErr { code: Int; }
        type JobErr { msg: Int; }
        type OtherErr { x: Int; }
        fn dbget(k: Int) -> Int fallible(DbErr) { return k; }
        fn convert(e: DbErr) -> Int fallible(JobErr) { return 0; }
        fn wrongpayload(k: Int) -> Int fallible(OtherErr) {
            let v = dbget(k) or convert(err);
            return v;
        }
        fn main() { println(wrongpayload(1) or 0); }
    "#,
    );
    assert!(
        msgs.iter()
            .any(|m| m.contains("propagated payload must match")),
        "expected the payload-mismatch diagnostic; got: {:?}",
        msgs
    );
}
