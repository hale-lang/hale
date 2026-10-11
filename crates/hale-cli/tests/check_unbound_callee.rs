//! GH #583 (dna/FRICTION.md F.18) — `hale check <seed>` refuses a call
//! to a bare name that nothing binds, the way `hale build` always did.
//! An organization whose gate is `check` applied a candidate that named
//! a router function nobody had written, and found out at expression.
//! The rule is on for a whole seed (a directory) and for one file, which
//! is checked as `hale build <file>` builds it: alone, no sibling loaded.

use std::path::{Path, PathBuf};
use std::process::Command;

/// `hale check <dir>`: a whole seed, where the rule is on.
fn check(src: &str, tag: &str) -> (bool, String) {
    check_as(src, tag, true)
}

fn check_as(src: &str, tag: &str, whole_seed: bool) -> (bool, String) {
    let d: PathBuf = std::env::temp_dir().join(format!("hale_check_unbound_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("main.hl");
    std::fs::write(&f, src).unwrap();
    let target = if whole_seed { d.to_string_lossy().to_string() } else { f.to_string_lossy().to_string() };
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(["check", &target]).current_dir(Path::new("/")).output().expect("hale");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&d);
    (out.status.success(), text)
}

const REFUSAL: &str = "no free fn, generic fn or fn-pointer binding with that name is in scope";

#[test]
fn a_call_to_nothing_is_refused_in_a_body_a_default_and_an_override() {
    let body = "locus A { params { n: Int = 0; } fn go() -> Int { return nothing_here(); } }\nmain locus M { params { a: A = A { }; } run() { println(self.a.go()); } }\nfn main() { M { }; }\n";
    let (ok, out) = check(body, "body");
    assert!(!ok && out.contains("call to `nothing_here`: ") && out.contains(REFUSAL), "{out}");
    let default = "locus R { params { k: Int = 1; } }\nlocus A { params { r: R = nothing_here(); } fn go() -> Int { return self.r.k; } }\nmain locus M { params { a: A = A { }; } run() { println(self.a.go()); } }\nfn main() { M { }; }\n";
    let (ok, out) = check(default, "default");
    assert!(!ok && out.contains("call to `nothing_here`: "), "{out}");
    // the organization's shape: a router function the model invented,
    // beside the one that exists — the hint names it
    let org = "fn leader_models() -> Int { return 1; }\nfn editor_models() -> Int { return 2; }\nlocus A { params { m: Int = editor_model(); } }\nmain locus M { params { a: A = A { m: leader_models() }; } run() { println(self.a.m); } }\nfn main() { M { }; }\n";
    let (ok, out) = check(org, "override");
    assert!(!ok && out.contains("call to `editor_model`: ") && out.contains("did you mean `editor_models`?"), "{out}");
}

/// A FILE target is a whole program too: `hale check f.hl` loads that
/// file and its imports, never its siblings, exactly as `hale build
/// f.hl` does, so a fn only a sibling defines is unbound in both. It
/// used to keep a permissive reading here, and accepted the call its
/// build refused. The seed (the directory) binds it and checks clean,
/// and the file's refusal says so.
#[test]
fn a_single_file_is_held_to_what_its_build_holds() {
    let src = "locus A { params { n: Int = 0; } fn go() -> Int { return sibling_helper(); } }\nmain locus M { params { a: A = A { }; } run() { println(self.a.go()); } }\nfn main() { M { }; }\n";
    let (ok, out) = check_as(src, "single", false);
    assert!(!ok && out.contains("call to `sibling_helper`: ") && out.contains(REFUSAL), "the file alone: {out}");
    let (ok, out) = check_as(src, "seed", true);
    assert!(!ok && out.contains("call to `sibling_helper`: "), "the seed: {out}");

    let d: PathBuf = std::env::temp_dir().join(format!("hale_check_unbound_{}_sibling", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("main.hl"), src).unwrap();
    std::fs::write(d.join("helpers.hl"), "fn sibling_helper() -> Int { return 7; }\n").unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).current_dir(Path::new("/")).output().expect("hale");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    let file = d.join("main.hl").to_string_lossy().to_string();
    let (ok, out) = run(&["check", &file]);
    assert!(!ok && out.contains("call to `sibling_helper`: ") && out.contains(REFUSAL), "{out}");
    assert!(
        out.contains("was checked alone, as `hale build` builds it; the 1 other .hl file(s) beside it are not loaded")
            && out.contains(&format!("`hale check {}` checks the whole seed", d.display())),
        "the file's refusal names the seed: {out}"
    );
    let (ok, out) = run(&["check", &file, "--json"]);
    assert!(!ok && !out.contains("checked alone"), "`--json` carries the records alone: {out}");
    let (built, out) = run(&["build", &file, "-o", &d.join("bin").to_string_lossy()]);
    assert!(!built && out.contains("call to `sibling_helper`: "), "the build of the file refuses it alike: {out}");
    let (ok, out) = run(&["check", &d.to_string_lossy()]);
    assert!(ok, "the seed binds it: {out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// Every place a call to a fn nobody wrote can stand: `hale check` on
/// the file, `hale check` on its directory and `hale build` all refuse
/// it, at the name, with the one wording (a fn VALUE is a bare
/// identifier, and a qualified call's head is a qualifier, each with
/// its own rule's wording). Each of these checked clean as a file
/// target and failed its build.
#[test]
fn every_shape_of_a_call_to_nothing_is_refused_by_check_and_build() {
    const LOCUS_M: &str = "main locus M { params { a: A = A { }; } run() { println(self.a.go()); } }\nfn main() { M { }; }\n";
    let shapes: Vec<(&str, String, &str)> = vec![
        ("statement", "fn main() { nope_fn(); }\n".into(), "call to `nope_fn`: "),
        ("let", "fn main() { let x = text_of_nope(\"a\"); println(x); }\n".into(), "call to `text_of_nope`: "),
        ("no_args", "fn main() { let x = nope_fn(); println(x); }\n".into(), "call to `nope_fn`: "),
        ("operand", "fn main() { println(1 + nope_fn(2)); }\n".into(), "call to `nope_fn`: "),
        ("condition", "fn main() { if nope_fn() { println(1); } }\n".into(), "call to `nope_fn`: "),
        ("loop", "fn main() { let mut i = 0; while i < 2 { nope_fn(i); i = i + 1; } }\n".into(), "call to `nope_fn`: "),
        ("free_fn", "fn helper() -> Int { return nope_fn(1); }\nfn main() { println(helper()); }\n".into(), "call to `nope_fn`: "),
        ("generic_fn", "fn id<T>(x: T) -> T { return nope_fn(x); }\nfn main() { println(id(1)); }\n".into(), "call to `nope_fn`: "),
        ("const", "const K: Int = nope_fn();\nfn main() { println(K); }\n".into(), "call to `nope_fn`: "),
        ("method", format!("locus A {{ params {{ n: Int = 0; }} fn go() -> Int {{ return nope_fn(); }} }}\n{LOCUS_M}"), "call to `nope_fn`: "),
        ("hook", "main locus M { params { n: Int = 0; } run() { nope_fn(self.n); } }\nfn main() { M { }; }\n".into(), "call to `nope_fn`: "),
        ("default", "locus A { params { n: Int = nope_fn(); } }\nmain locus M { params { a: A = A { }; } run() { println(self.a.n); } }\nfn main() { M { }; }\n".into(), "call to `nope_fn`: "),
        ("handler", "type Ping { seq: Int = 0; }\ntopic Tick { payload: Ping; subject: \"x.tick\"; }\nlocus Sub { params { n: Int = 0; } bus { subscribe Tick as on_tick; } fn on_tick(p: Ping) { nope_fn(p.seq); } }\nmain locus M { params { s: Sub = Sub { }; } bus { publish Tick; } run() { Tick <- Ping { seq: 1 }; } }\nfn main() { M { }; }\n".into(), "call to `nope_fn`: "),
        ("fn_value", "fn main() { let f = nope_fn; println(f(1)); }\n".into(), "unknown identifier `nope_fn`: no binding, param, const or declaration with that name is in scope"),
        ("fn_value_in_method", format!("locus A {{ params {{ n: Int = 0; }} fn go() -> Int {{ let g = nope_fn; return g(1); }} }}\n{LOCUS_M}"), "unknown identifier `nope_fn`: "),
        ("qualified", "fn main() { let x = nomod::nope_fn(\"a\"); println(x); }\n".into(), "`nomod::nope_fn`: `nomod` is not an import or a type of this seed"),
        ("typo", "fn text_of(s: String) -> String { return s; }\nfn main() { println(text_ofx(\"a\")); }\n".into(), "call to `text_ofx`: no free fn, generic fn or fn-pointer binding with that name is in scope — did you mean `text_of`?"),
    ];
    for (tag, src, want) in &shapes {
        for whole_seed in [false, true] {
            let (ok, out) = check_as(src, &format!("shape_{tag}_{whole_seed}"), whole_seed);
            assert!(!ok && out.contains(want), "{tag} (dir: {whole_seed}) must be refused with {want:?}: {out}");
            assert_eq!(out.matches(want).count(), 1, "{tag}: one mistake, one message: {out}");
        }
        let (built, out) = build(src, &format!("shape_{tag}"));
        assert!(!built && out.contains(want), "{tag}: the build refuses it alike: {out}");
    }
}

/// GH #779: the rule exempts the bare names codegen answers itself,
/// and the exemption table had drifted from the dispatch. Each of
/// these programs was refused by `hale check <dir>` and executed
/// happily by `hale run` — the admission gate rejecting correct code.
#[test]
fn every_builtin_codegen_answers_checks_clean_as_a_seed() {
    // The issue's program: the two string predicates, which codegen
    // answers in `lower_str_predicate_builtin`.
    let predicates = "fn main() {\n    let s = \"abc\";\n    let b = starts_with(s, \"a\");\n    let c = contains(s, \"b\");\n    println(b, c);\n}\n";
    let (ok, out) = check(predicates, "779_predicates");
    assert!(ok, "starts_with / contains: {out}");

    // `eprint` — the one printer of the four `lower_print_call`
    // accepts that the table had missed.
    let eprint = "fn main() {\n    eprint(\"x\");\n    eprintln(\"y\");\n    print(\"z\");\n    println(\"w\");\n}\n";
    let (ok, out) = check(eprint, "779_eprint");
    assert!(ok, "eprint: {out}");

    // `check_closures()` — the explicit-epoch closure surface, a
    // statement-position builtin.
    let closures = "locus Ledger {\n    params { debits: Int = 0; credits: Int = 0; }\n    closure balanced { self.debits ~~ self.credits within 0; epoch explicit; }\n    fn post() { self.debits = self.debits + 1; self.credits = self.credits + 1; check_closures(); }\n}\nmain locus M { params { l: Ledger = Ledger { }; } run() { self.l.post(); } }\nfn main() { M { }; }\n";
    let (ok, out) = check(closures, "779_check_closures");
    assert!(ok, "check_closures: {out}");

    // `mean(x)` — accumulator vocabulary inside a closure assertion,
    // beside `count()` and `sum(x)`, which the table already had.
    let accumulators = "main locus T {\n    params { delta: Float = 0.0; }\n    closure mean_in_band { mean(self.delta) ~~ 0.0 within 100.0; epoch tick; }\n    closure counted { count() ~~ 1 within 0; epoch tick; }\n    run() { self.delta = 1.0; }\n}\nfn main() { T { }; }\n";
    let (ok, out) = check(accumulators, "779_accumulators");
    assert!(ok, "mean / count: {out}");
}

#[test]
fn bound_names_still_check() {
    // a free fn, a fn-pointer local, a builtin, a generic fn, a stdlib path
    let src = "fn helper(n: Int) -> Int { return n; }\nfn twice<T>(x: T) -> T { return x; }\nmain locus M { run() { let f = helper; println(f(1), \" \", len(\"ab\"), \" \", to_string(twice(3)), \" \", std::str::pad_left(\"a\", 3, \" \"), \" \", abs(0 - 2)); } }\nfn main() { M { }; }\n";
    let (ok, out) = check(src, "bound");
    assert!(ok, "{out}");
}

/// `hale build` on the same directory, for the cases where the
/// point is that `check`'s verdict and the build's agree.
fn build(src: &str, tag: &str) -> (bool, String) {
    let d: PathBuf = std::env::temp_dir().join(format!("hale_build_unbound_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("main.hl"), src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["build", &d.to_string_lossy()])
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&d);
    (out.status.success(), text)
}

/// GH #800 — the per-name decisions, pinned.
///
/// `BARE_BUILTIN_CALLEES` exempts a bare callee from the F.18
/// strict-callee rule because *codegen answers it*. Ten names were
/// exempt that codegen answers nowhere, so `hale check <dir>` said
/// `ok` and `hale build` on the same directory refused the call at
/// lowering — spanless, from another layer, about a program the
/// author had just been told was fine. One of the ten (`Float`) was
/// a real builtin missing its arm; the other nine were never
/// builtins at all and are dropped. Seven of the nine are in the
/// loop below; `prod` is not, because the parser gives it its own
/// AST node and it can never be written as a bare call; and
/// `Duration`, a bare name when this was written, is since GH #1076
/// (U4) the stdlib's `quantity Int in ns`, so `Duration(x)` is the
/// cast form and is refused by the quantity rule instead (after the
/// loop).
#[test]
fn a_bare_name_the_build_cannot_lower_is_refused_by_check() {
    // Each of these had NO codegen dispatch anywhere:
    //
    //   `hex`      — hexadecimal is a FORMAT SPEC (`f"{n:x}"`),
    //                never a call;
    //   `panic`    — `spec/runtime.md`: "Hale has no `panic(msg)`";
    //                failure is structural (closure violation) or
    //                value-level (`fallible(E)`);
    //   `exit`     — the spelling is `std::process::exit(code)`,
    //                which carries the SYSCALL effect; a bare alias
    //                would be an unclassified way out of the
    //                process;
    //   the five remaining primitive-type names — `Int(x)` and
    //                `Float(x)` are the two numeric CASTS
    //                (`spec/types.md` § "Explicit numeric
    //                conversions"); `String` / `Bool` / `Bytes` /
    //                `Decimal` are types, and there is no
    //                conversion behind them. `to_string(x)`
    //                renders, `std::str::parse_*` reads back.
    for (name, program) in [
        ("hex", "fn main() { let s = hex(255); println(s); }\n"),
        ("panic", "fn main() { panic(\"boom\"); }\n"),
        ("exit", "fn main() { exit(0); }\n"),
        ("String", "fn main() { let s = String(3); println(s); }\n"),
        ("Bool", "fn main() { let b = Bool(1); println(b); }\n"),
        ("Bytes", "fn main() { let b = Bytes(\"ab\"); println(b); }\n"),
        ("Decimal", "fn main() { let d = Decimal(1); println(d); }\n"),
    ] {
        let (ok, out) = check(program, &format!("800_{}", name));
        assert!(
            !ok && out.contains(&format!("call to `{}`: ", name))
                && out.contains(REFUSAL),
            "`{}` must be refused at its own span, not at lowering: {out}",
            name
        );
        // The verdict the diagnostic is standing in for.
        let (built, berr) = build(program, &format!("800b_{}", name));
        assert!(!built, "`{}` unexpectedly builds now: {berr}", name);
    }
    // `Duration(1)`: a cast of an `Int` into the stdlib's quantity,
    // refused by the checker at the call's own span with the quantity
    // rule's wording; the build still refuses the program.
    let program = "fn main() { let d = Duration(1); println(d); }\n";
    let (ok, out) = check(program, "800_Duration");
    assert!(
        !ok && out.contains("`Duration(…)` of an `Int`: a count becomes a quantity by a unit (`n * 1ns`)"),
        "`Duration(1)` must be refused by the check as a cast of an `Int`: {out}"
    );
    let (built, berr) = build(program, "800b_Duration");
    assert!(!built, "`Duration(1)` unexpectedly builds now: {berr}");
}

/// The other half of the same decision: `Float(x)` IS a builtin —
/// `spec/types.md` has specified the pair `Int(x)` / `Float(x)`
/// since v1.x-11 — and it was the one name in the group that only
/// lacked its codegen arm. It keeps its exemption, and now the build
/// agrees. (`tests/hale/float_cast_test.hl` checks what it computes.)
#[test]
fn the_float_cast_checks_and_builds() {
    let src = "fn main() {\n    let n = 5;\n    println(Float(3), \" \", Float(n) / 2.0, \" \", Int(3.9));\n}\n";
    let (ok, out) = check(src, "800_float");
    assert!(ok, "Float cast: {out}");
    let (built, err) = build(src, "800b_float");
    assert!(built, "Float cast must build: {err}");
}

/// A user fn named like a dropped builtin keeps working — the whole
/// point of dropping the name is that nothing answers it but the
/// user. `dna/host/writers.hl` has declared `fn hex(n: Int)` all
/// along; it resolves to the declaration, and always did.
#[test]
fn a_user_fn_may_take_a_dropped_builtins_name() {
    let src = "fn hex(n: Int) -> String { return to_string(n); }\nfn main() { println(hex(255)); }\n";
    let (ok, out) = check(src, "800_user_hex");
    assert!(ok, "{out}");
    let (built, err) = build(src, "800b_user_hex");
    assert!(built, "{err}");
}
