//! GH #738 — a bare fallible call (no `or`) is an error, in `hale
//! check`, `hale verify` and `hale build` alike, user or stdlib (F.40
//! phase 3, E4: one law over the typed-body table's fallible column).
//! A handled call and a deliberately discarded one say nothing. The
//! legacy Int-status form is the bare form and is refused the same way.
//! `--strict-fallible` is gone: there is no mode in which the bare call
//! is accepted.

use std::path::{Path, PathBuf};
use std::process::Command;

fn run(verb: &str, flags: &[&str], src: &str, tag: &str) -> (bool, String) {
    let d: PathBuf = std::env::temp_dir()
        .join(format!("hale_strict_fallible_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("main.hl");
    std::fs::write(&f, src).unwrap();
    let mut args: Vec<&str> = vec![verb];
    args.extend_from_slice(flags);
    let target = f.to_string_lossy().to_string();
    args.push(&target);
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(&args)
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&d);
    (out.status.success(), text)
}

const NOTICE: &str = "can fail (IoError) and this call says nothing about it";

const UNHANDLED: &str = "fn main() {\n    std::io::fs::write_file(\"/tmp/hale-738-t\", \"x\");\n    println(\"done\");\n}\n";
const DISCARDED: &str = "fn main() {\n    std::io::fs::write_file(\"/tmp/hale-738-t\", \"x\") or discard;\n    println(\"done\");\n}\n";
const HANDLED: &str = "fn main() {\n    std::io::fs::write_file(\"/tmp/hale-738-t\", \"x\") or raise;\n    let s = std::io::fs::read_file(\"/tmp/hale-738-t\") or \"\";\n    let n = std::str::parse_int(\"7\") or 0;\n    println(s, n);\n}\n";
const LEGACY_STATUS: &str = "fn main() {\n    let r: Int = std::io::fs::write_file(\"/tmp/hale-738-t\", \"x\");\n    println(r);\n}\n";

#[test]
fn an_unhandled_call_is_an_error_in_check_verify_and_build() {
    for verb in ["check", "verify", "build"] {
        let (ok, out) = run(verb, &[], UNHANDLED, verb);
        assert!(!ok, "{verb} must refuse the bare call: {out}");
        assert!(
            out.contains("`std::io::fs::write_file`") && out.contains(NOTICE),
            "{verb}: the error names the callee, its payload and the missing disposition: {out}"
        );
        assert!(
            out.contains("or raise") && out.contains("or discard") && out.contains("or handler(err)"),
            "{verb}: {out}"
        );
        assert!(out.contains("error since v0.22.0"), "{verb}: {out}");
    }
}

#[test]
fn a_discarded_or_handled_call_says_nothing() {
    for (src, tag) in [(DISCARDED, "discard"), (HANDLED, "handled")] {
        let (ok, out) = run("check", &[], src, tag);
        assert!(ok && !out.contains(NOTICE), "{tag}: {out}");
        let (ok, out) = run("verify", &[], src, &format!("{tag}_v"));
        assert!(ok && out.contains("0 findings"), "{tag} under verify: {out}");
    }
}

#[test]
fn the_legacy_status_form_is_the_bare_form() {
    let (ok, out) = run("check", &[], LEGACY_STATUS, "legacy");
    assert!(!ok && out.contains(NOTICE), "{out}");
}

#[test]
fn the_strict_flag_is_gone() {
    let (ok, out) = run("check", &["--strict-fallible"], DISCARDED, "flag");
    assert!(!ok && out.contains("--strict-fallible"), "an unknown flag is refused, not ignored: {out}");
}

// ── One rule for user and stdlib calls (F.40 phase 3, E4) ──
//
// Only an `or` handles a fallible call. Every other position is the
// bare call, with the GH #738 wording at the call, whatever the callee:
// a fn the program declares, a locus method, an array's `get`, a stdlib
// entry point. Before, a user call in an argument or a comparison was a
// type mismatch, and a `match` over one passed `hale check` and failed
// at the build (an unsupported call, or IR the verifier rejected).

const PRELUDE: &str = "type Fault { why: String = \"\"; }\n\
fn f(n: Int) -> Int fallible(Fault) {\n    if n > 5 { fail Fault { why: \"big\" }; }\n    return n;\n}\n\
fn take(n: Int) -> Int { return n; }\n\
fn main() {\n";

fn bare(callee: &str, payload: &str) -> String {
    format!(
        "type error: `{callee}` can fail ({payload}) and this call says nothing about it: write `or raise` to \
         hand the failure to the caller, `or <fallback>` for a value to use instead, `or handler(err)` to deal \
         with it here, or `or discard` when losing it is the intent. A bare call to a fallible entry point is an \
         error since v0.22.0 (GH #738)."
    )
}

/// `main`'s one line (line 8 of the program) and where its bare call
/// stands, for each position and each kind of callee.
const POSITIONS: &[(&str, &str, &str, &str, &str)] = &[
    ("u_arg", "    println(take(f(3)));", "f", "Fault", "8:18"),
    ("s_arg", "    println(take(std::str::parse_int(\"3\")));", "std::str::parse_int", "ParseError", "8:18"),
    ("u_cmp", "    if f(5) > 1 { println(\"x\"); }", "f", "Fault", "8:8"),
    ("s_cmp", "    if std::str::parse_int(\"5\") > 1 { println(\"x\"); }", "std::str::parse_int", "ParseError", "8:8"),
    ("u_match", "    match f(2) { _ -> { println(\"m\"); }, }", "f", "Fault", "8:11"),
    ("s_match", "    match std::str::parse_int(\"2\") { _ -> { println(\"m\"); }, }", "std::str::parse_int", "ParseError", "8:11"),
    ("a_match", "    let arr = [1, 2, 3]; match arr.get(1) { _ -> { println(\"m\"); }, }", "arr.get", "IndexError", "8:32"),
    ("u_let", "    let a = f(1); println(a);", "f", "Fault", "8:13"),
    ("s_let", "    let a = std::str::parse_int(\"1\"); println(a);", "std::str::parse_int", "ParseError", "8:13"),
    ("u_stmt", "    f(2);", "f", "Fault", "8:5"),
    ("s_stmt", "    std::str::parse_int(\"2\");", "std::str::parse_int", "ParseError", "8:5"),
];

#[test]
fn every_position_but_an_or_is_the_bare_call_user_and_stdlib_alike() {
    for (tag, line, callee, payload, at) in POSITIONS {
        let src = format!("{PRELUDE}{line}\n}}\n");
        let (ok, out) = run("check", &[], &src, tag);
        assert!(!ok, "{tag}: refused: {out}");
        let want = format!("main.hl:{at}: {}", bare(callee, payload));
        assert!(out.contains(&want), "{tag}: wanted `{want}`:\n{out}");
        assert_eq!(out.matches("type error").count(), 1, "{tag}: the law's one error, no type mismatch:\n{out}");
    }
}

/// The positions `hale check` used to accept and `hale build` refused:
/// both now refuse them with the same error.
#[test]
fn check_and_build_refuse_a_match_over_a_fallible_call_alike() {
    let method = "type Fault { why: String = \"\"; }\n\
locus Reader {\n    params { base: Int = 1; }\n\
    fn read(n: Int) -> Int fallible(Fault) {\n        if n > 5 { fail Fault { why: \"big\" }; }\n        return n + self.base;\n    }\n\
    run() {\n        match self.read(2) {\n            3 -> { println(\"three\"); },\n            _ -> { println(\"other\"); },\n        }\n    }\n}\n\
fn main() {\n    let r = Reader { };\n}\n";
    let user = format!("{PRELUDE}    match f(2) {{ 2 -> {{ println(\"two\"); }}, _ -> {{ println(\"other\"); }}, }}\n}}\n");
    for (tag, src, want) in [
        ("method", method.to_string(), format!("main.hl:9:15: {}", bare("self.read", "Fault"))),
        ("user", user, format!("main.hl:8:11: {}", bare("f", "Fault"))),
    ] {
        for verb in ["check", "build"] {
            let (ok, out) = run(verb, &[], &src, &format!("match_{tag}_{verb}"));
            assert!(!ok && out.contains(&want), "{verb} {tag}: wanted `{want}`:\n{out}");
            assert!(!out.contains("codegen error"), "{verb} {tag}: refused before lowering:\n{out}");
        }
    }
}

/// An `or`'s handler takes the implicit `or raise` where lowering
/// supports it, a fn the program declares; any other fallible handler
/// is refused at the `or` with the nested spelling.
#[test]
fn a_fallible_handler_lowering_does_not_support_is_refused_with_the_nested_spelling() {
    let handler = |decl: &str, payload: &str, rhs: &str| {
        format!(
            "{PRELUDE}    println(h(9) or 0);\n}}\n{decl}\
fn h(n: Int) -> Int fallible({payload}) {{\n    let arr = [1, 2, 3];\n    let x = f(n) or {rhs};\n    return x + len(arr);\n}}\n"
        )
    };
    let cases = [
        (
            "stdlib",
            handler("", "ParseError", "std::str::parse_int(\"9\")"),
            "main.hl:12:13: type error: `or std::str::parse_int(...)`: a fallible stdlib call can't be the handler \
             directly yet — write the nested form `or (std::str::parse_int(...) or raise)` so its own failure has a path",
        ),
        (
            "array",
            handler("", "IndexError", "arr.get(1)"),
            "main.hl:12:13: type error: `or arr.get(...)`: only a fallible fn or locus method the program declares \
             can be the handler directly yet — write the nested form `or (arr.get(...) or raise)` so its own failure \
             has a path",
        ),
        (
            "generic",
            handler("fn pick<T>(x: T) -> T fallible(Fault) { return x; }\n", "Fault", "pick(4)"),
            "main.hl:13:13: type error: `or pick(...)`: only a fallible fn or locus method the program declares \
             can be the handler directly yet — write the nested form `or (pick(...) or raise)` so its own failure \
             has a path",
        ),
    ];
    for (tag, src, want) in cases {
        for verb in ["check", "build"] {
            let (ok, out) = run(verb, &[], &src, &format!("handler_{tag}_{verb}"));
            assert!(!ok && out.contains(want), "{verb} {tag}: wanted `{want}`:\n{out}");
            assert_eq!(out.matches("type error").count(), 1, "{verb} {tag}: one error:\n{out}");
        }
    }
}

/// Every `or` form handles a fallible call, user and stdlib alike, and
/// a declared fn as the handler takes the implicit `or raise`.
#[test]
fn the_or_forms_handle_a_fallible_call() {
    let user = "type Fault { why: String = \"\"; }\ntype Other { code: Int = 0; }\n\
fn f(n: Int) -> Int fallible(Fault) {\n    if n > 5 { fail Fault { why: \"big\" }; }\n    return n;\n}\n\
fn g(n: Int) fallible(Fault) {\n    if n > 5 { fail Fault { why: \"big\" }; }\n}\n\
fn recover(e: Fault) -> Int { return 40; }\n\
fn convert(e: Fault) -> Int fallible(Fault) { return 50; }\n\
fn raises(n: Int) -> Int fallible(Fault) {\n    let a = f(n) or raise;\n    let b = f(9) or convert(err);\n    return a + b;\n}\n\
fn translates(n: Int) -> Int fallible(Other) {\n    let a = f(n) or fail Other { code: 7 };\n    return a;\n}\n\
fn main() {\n    let a = f(1) or 0;\n    let b = f(9) or recover(err);\n    g(9) or discard;\n    let c = raises(2) or 0;\n\
    let d = translates(9) or 0;\n    let e = f(9) or f(3) or 0;\n    let arr = [1, 2, 3];\n    let x = arr.get(9) or 7;\n    println(a, b, c, d, e, x);\n}\n";
    let stdlib = "type Other { code: Int = 0; }\n\
fn recover(e: ParseError) -> Int { return 40; }\n\
fn raises(s: String) -> Int fallible(ParseError) {\n    let a = std::str::parse_int(s) or raise;\n    return a;\n}\n\
fn translates(s: String) -> Int fallible(Other) {\n    let a = std::str::parse_int(s) or fail Other { code: 7 };\n    return a;\n}\n\
fn main() {\n    let a = std::str::parse_int(\"1\") or 0;\n    let b = std::str::parse_int(\"x\") or recover(err);\n\
    std::io::fs::write_file(\"/tmp/hale-738-or-forms\", \"x\") or discard;\n    let c = raises(\"2\") or 0;\n\
    let d = translates(\"y\") or 0;\n    let e = std::str::parse_int(\"z\") or std::str::parse_int(\"3\") or 0;\n    println(a, b, c, d, e);\n}\n";
    for (tag, src, printed) in [("or_user", user, "14052037"), ("or_stdlib", stdlib, "140203")] {
        let (ok, out) = run("check", &[], src, tag);
        assert!(ok && !out.contains("type error"), "{tag}: {out}");
        let (ok, out) = run("run", &[], src, &format!("{tag}_run"));
        assert!(ok && out.lines().any(|l| l == printed), "{tag} runs: {out}");
    }
}

/// A bundled stdlib fn written as Hale is a fn lowering resolves, so as
/// an `or`'s handler it takes the implicit `or raise` as the program's
/// own fns do: its value substitutes, and its failure takes the
/// enclosing fn's error path. (Before, the GH #738 pass refused it as a
/// bare call.)
#[test]
fn a_bundled_stdlib_fn_as_the_handler_takes_the_implicit_or_raise() {
    let src = |second: &str| {
        format!(
            "fn h() -> Int fallible(IoError) {{\n    let g = std::io::file::open(\"/nonexistent/hale-738/a\", \"r\") \
             or std::io::file::open(\"{second}\", \"r\");\n    return 7;\n}}\nfn main() {{\n    println(h() or 0);\n}}\n"
        )
    };
    let here = std::env::temp_dir().join(format!("hale_738_handler_{}", std::process::id()));
    std::fs::write(&here, "x").unwrap();
    for (tag, second, printed) in [
        ("handler_ok", here.to_string_lossy().to_string(), "7"),
        ("handler_fails", "/nonexistent/hale-738/b".to_string(), "0"),
    ] {
        let (ok, out) = run("run", &[], &src(&second), tag);
        assert!(ok && out.lines().any(|l| l == printed), "{tag}: {out}");
    }
    let _ = std::fs::remove_file(&here);
}
