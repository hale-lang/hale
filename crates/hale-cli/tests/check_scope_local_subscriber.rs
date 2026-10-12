//! The dropped scope-local delivery advisory (spec/verification.md,
//! "Dropped scope-local delivery"): a bus subscriber built and dissolved
//! inside one call that runs on a classic cooperative pool registers with
//! that pool (spec/runtime.md § "Subscriptions follow the tower"), whose
//! cells are handler-atomic, so every delivery to it is dropped. `hale
//! check` warns at the literal, naming the locus, the call and the pool;
//! not on an `async_io` pool, not on main, and not when the subscriber
//! outlives the call.
//!
//! The programs are escaped string literals rather than raw strings:
//! `hale-corpus` harvests raw-string literals out of test files, and these
//! are deliberately diagnostic.

use std::path::{Path, PathBuf};
use std::process::Command;

/// `Host`, placed as `placement`, whose `work` (or the free fn `helper`
/// it calls, when `in_free_fn`) builds `Peer` as `build` writes it.
fn program(placement: &str, host_extra: &str, build: &str, in_free_fn: bool) -> String {
    let (helper, call) = if in_free_fn {
        (format!("fn helper() {{ {build} std::time::sleep(10ms); }}\n"), "helper();".to_string())
    } else {
        (String::new(), format!("{build} std::time::sleep(10ms);"))
    };
    format!(
        "type Msg {{ n: Int; }}
locus Peer {{
    params {{ got: Int = 0; }}
    bus {{ subscribe \"peer.msg\" as on_msg of type Msg; }}
    fn on_msg(m: Msg) {{ self.got = self.got + 1; }}
}}
{helper}locus Host {{
    params {{ n: Int = 0; {host_extra} }}
    bus {{ publish \"peer.msg\" of type Msg; }}
    fn work() {{ {call} \"peer.msg\" <- Msg {{ n: 1 }}; }}
    run() {{ self.work(); }}
}}
main locus App {{
    params {{ h: Host = Host {{ }}; }}
    placement {{ h: {placement}; }}
    run() {{ std::time::sleep(50ms); }}
}}
fn main() {{ App {{ }}; }}
"
    )
}

fn check(tag: &str, text: &str) -> (bool, String) {
    let root: PathBuf =
        std::env::temp_dir().join(format!("hale_check_scope_local_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&root);
    let seed = root.join("app");
    std::fs::create_dir_all(&seed).unwrap();
    std::fs::write(seed.join("main.hl"), text).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&seed)
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    let _ = std::fs::remove_dir_all(&root);
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

const FINDING: &str = "builds it and dissolves it inside one call of";

#[test]
fn a_let_bound_subscriber_on_a_classic_pool_is_warned() {
    let (ok, out) = check("classic_let", &program("cooperative(pool = cls)", "", "let p = Peer { };", false));
    assert!(ok, "a warning, not an error:\n{out}");
    assert_eq!(out.matches(FINDING).count(), 1, "one finding:\n{out}");
    assert!(
        out.contains("`Peer` subscribes on the bus, and this literal builds it and dissolves it inside one call of `Host.work`, which runs on the classic cooperative pool `cls`."),
        "names the locus, the call and the pool:\n{out}"
    );
    assert!(out.contains("Place the pool `where async_io`"), "{out}");
    assert!(out.contains("keep the subscriber beyond the call"), "{out}");
    assert!(out.contains("warning"), "{out}");
}

#[test]
fn a_bare_statement_subscriber_in_a_free_fn_on_a_classic_pool_is_warned() {
    let (ok, out) = check("classic_free_fn", &program("cooperative(pool = cls)", "", "Peer { };", true));
    assert!(ok, "{out}");
    assert!(out.contains("inside one call of `helper`, which runs on the classic cooperative pool `cls`"), "{out}");
}

#[test]
fn not_on_an_async_io_pool() {
    let (ok, out) =
        check("async", &program("cooperative(pool = aio) where async_io", "", "let p = Peer { };", false));
    assert!(ok && !out.contains(FINDING), "{out}");
}

#[test]
fn not_on_main() {
    let (ok, out) = check("main", &program("cooperative(pool = main)", "", "let p = Peer { };", false));
    assert!(ok && !out.contains(FINDING), "{out}");
}

#[test]
fn not_when_the_subscriber_outlives_the_call() {
    // stored in a field
    let (ok, out) = check(
        "field",
        &program("cooperative(pool = cls)", "peer: Peer = Peer { };", "self.peer = Peer { };", false),
    );
    assert!(ok && !out.contains(FINDING), "a field holds it:\n{out}");
    // accepted
    let (ok, out) = check("accepted", &program("cooperative(pool = cls)", "} accept(c: Peer) {", "Peer { };", false));
    assert!(ok && !out.contains(FINDING), "an accept owns it:\n{out}");
}

/// `program`, with `Peer` built through `wrapper` (declarations naming a
/// non-subscribing `Wrapper`) instead of directly.
fn wrapped(placement: &str, wrapper: &str) -> String {
    let text = program(placement, "", "let w = Wrapper { };", false);
    text.replacen("locus Host {", &format!("{wrapper}\nlocus Host {{"), 1)
}

// Review of #1513: a scope-local locus that does not subscribe itself
// but keeps a subscriber — built by its params default, two defaults
// down, or in its birth and accepted — drops that subscriber's
// deliveries the same way.
#[test]
fn a_wrapper_that_keeps_a_subscriber_on_a_classic_pool_is_warned() {
    for (tag, wrapper) in [
        ("wrap_default", "locus Wrapper { params { p: Peer = Peer { }; } }"),
        (
            "wrap_nested_default",
            "locus Inner { params { p: Peer = Peer { }; } }\nlocus Wrapper { params { i: Inner = Inner { }; } }",
        ),
        ("wrap_accepts", "locus Wrapper { accept(c: Peer) { } birth() { Peer { }; } }"),
    ] {
        let (ok, out) = check(tag, &wrapped("cooperative(pool = cls)", wrapper));
        assert!(ok, "{tag}:\n{out}");
        assert!(
            out.contains("`Wrapper` holds a bus subscriber for its whole life")
                && out.contains("inside one call of `Host.work`, which runs on the classic cooperative pool `cls`"),
            "{tag}:\n{out}"
        );
        let (ok, out) = check(&format!("{tag}_async"), &wrapped("cooperative(pool = aio) where async_io", wrapper));
        assert!(ok && !out.contains(FINDING), "{tag} on async_io:\n{out}");
    }
    // a wrapper that keeps no subscriber
    let (ok, out) = check("wrap_plain", &wrapped("cooperative(pool = cls)", "locus Wrapper { params { n: Int = 0; } }"));
    assert!(ok && !out.contains(FINDING), "{out}");
}
