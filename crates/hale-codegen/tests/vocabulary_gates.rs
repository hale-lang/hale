//! Two shared vocabulary tables, gated in both directions.
//!
//! `hale check` accepting a program `hale build` refuses is the worst
//! failure mode the toolchain has (see the module doc of
//! `corpus_check_build_agreement`), and the shape it keeps taking is
//! always the same: codegen's dispatch knows a set of names or forms,
//! the checker keeps its own idea of that set, and the two drift.
//! `BARE_BUILTIN_CALLEES` is the case that taught the lesson — GH #779
//! found five names the table was missing (so the admission gate
//! refused working programs) and GH #800 found ten it invented (so
//! `hale check` accepted calls that died at lowering). It is honest
//! now only because two tests keep it honest, one per direction.
//!
//! This file does the same for the two vocabularies GH #911 B3 folds
//! in — and for both, the table itself is now SHARED rather than
//! copied, so agreement is structural and these tests are the proof
//! rather than the mechanism:
//!
//!   * **generic arguments** (GH #907). `Box<Int>` is lowered as the
//!     monomorph `Box_Int`, and the mangler minted a token for seven
//!     primitives while the checker had no opinion at all — so
//!     `type Holder { b: Box<Bytes>; }` typechecked and then died at
//!     build, although a `Bytes` FIELD is ordinary everywhere else in
//!     the language. The vocabulary now lives in
//!     `hale_types::ty::GENERIC_ARG_PRIMS`, which codegen reads for the
//!     token and the checker reads for the refusal.
//!
//!   * **statement position** (GH #845). A bare builtin call whose
//!     value is discarded (`Int(3);`, `len(s);`, `min(1, 2);`) was
//!     lowered by a different dispatch than the same call in
//!     expression position, and that dispatch fell through to
//!     `lower_print_call` — which knows the four printers and nothing
//!     else. Statement position now lowers the expression and drops
//!     the value, so the positions cannot know different names; the
//!     table below is the statement-position twin of
//!     `corpus_check_build_agreement`'s `BARE_BUILTIN_PROGRAMS`.
//!
//! Every program here is written as an ordinary (non-raw) string
//! literal on purpose: `hale_corpus::embedded` harvests `r#"…"#`
//! literals out of test sources into the corpus, and a probe matrix
//! that moves four committed baselines every time a row is added is a
//! probe matrix nobody will add a row to.

use std::collections::BTreeSet;
use std::process::Command;

use hale_codegen::build_executable;
use hale_syntax::ast::{TopDecl, TypeDeclBody, TypeExpr};

#[path = "support/harness.rs"]
mod harness;

/// Does the CHECKER accept this program? The verdict `hale check`
/// reports, as the agreement sweep reads it.
fn check_accepts(program: &hale_syntax::ast::Program) -> Result<(), String> {
    let errs: Vec<String> = hale_types::check_program(program)
        .iter()
        .filter(|d| d.is_error())
        .map(|d| d.message.clone())
        .collect();
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs.join("; "))
    }
}

/// Does CODEGEN accept it? Builds to a unique path and removes the
/// binary; the error is rendered so a failure names the refusal.
fn build_accepts(
    program: &hale_syntax::ast::Program,
    tag: &str,
) -> Result<(), String> {
    let bin = harness::unique_bin(tag);
    match build_executable(program, &bin) {
        Ok(()) => {
            let _ = std::fs::remove_file(&bin);
            Ok(())
        }
        Err(e) => Err(format!("{:?}", e)),
    }
}

// ===========================================================
// Table 1: which primitives may be a generic argument
// ===========================================================

/// `type Holder { b: Box<SPELLING>; }` — the smallest program that
/// asks codegen for a monomorph name. Declaration only: CONSTRUCTING
/// one still needs the mangled name (`Box_Bytes { … }`), which the
/// checker does not resolve — the separate generic-monomorph item on
/// GH #911, and the reason the run-time probe below uses
/// `build_executable` directly the way `generics.rs` does.
fn generic_arg_program(spelling: &str) -> String {
    format!(
        "type Box<T> {{\n    item: T;\n}}\n\n\
         type Holder {{\n    b: Box<{}>;\n}}\n\n\
         fn main() {{\n}}\n",
        spelling
    )
}

/// GH #911 B3 (#907) — every primitive, as a generic argument, gets
/// the same answer from both layers.
///
/// The matrix is driven by the parser's own list of primitive
/// spellings, so a new primitive type joins it without anyone
/// remembering to, and the EXPECTED verdict is read from the shared
/// table rather than restated here: whatever
/// `generic_arg_mangle_token` says about the `PrimType` the parser
/// produced is what both layers must do.
#[test]
fn every_primitive_as_a_generic_argument_agrees_between_check_and_build() {
    let mut disagreements: Vec<String> = Vec::new();
    let mut wrong_verdict: Vec<String> = Vec::new();

    for spelling in hale_syntax::parser::PRIMITIVE_TYPE_NAMES {
        let src = generic_arg_program(spelling);
        let program = hale_syntax::parse_source(&src)
            .unwrap_or_else(|d| panic!("`{}` must parse: {:?}", spelling, d));

        // The argument the parser actually produced, so the
        // expectation comes from the shared table and not from a
        // second list of spellings kept in this file.
        let arg = sole_generic_argument(&program);
        let TypeExpr::Primitive(p, _) = arg else {
            panic!(
                "`Box<{}>`'s argument parsed as {:?}, not a primitive — \
                 the matrix is testing the wrong thing",
                spelling, arg
            );
        };
        let expected_ok = hale_types::ty::generic_arg_mangle_token(*p).is_some();

        let checked = check_accepts(&program);
        let built = build_accepts(&program, &format!("hale_vg_ga_{}", spelling));

        match (&checked, &built) {
            (Ok(()), Err(e)) => disagreements.push(format!(
                "  `Box<{}>`: check accepts, build refuses — {}",
                spelling, e
            )),
            (Err(e), Ok(())) => disagreements.push(format!(
                "  `Box<{}>`: check refuses, build accepts — {}",
                spelling, e
            )),
            _ => {}
        }
        if checked.is_ok() != expected_ok {
            wrong_verdict.push(format!(
                "  `Box<{}>`: the shared table says {}, check says {} ({})",
                spelling,
                if expected_ok { "supported" } else { "refused" },
                if checked.is_ok() { "supported" } else { "refused" },
                checked.clone().err().unwrap_or_default()
            ));
        }
    }

    assert!(
        disagreements.is_empty(),
        "a generic argument must get the same answer from both layers \
         (GH #911 B3, #907):\n{}\n\nThe vocabulary is \
         `hale_types::ty::GENERIC_ARG_PRIMS`: codegen reads it for the \
         mangle token, the checker reads it for the refusal. A row that \
         diverges means one of the two stopped reading it.",
        disagreements.join("\n")
    );
    assert!(
        wrong_verdict.is_empty(),
        "the shared table and the checker disagree about which \
         primitives may be a generic argument:\n{}",
        wrong_verdict.join("\n")
    );
}

/// The argument of `Holder`'s `Box<…>` field, as the parser built it.
fn sole_generic_argument(program: &hale_syntax::ast::Program) -> &TypeExpr {
    for item in &program.items {
        let TopDecl::Type(t) = item else { continue };
        if t.name.name != "Holder" {
            continue;
        }
        let TypeDeclBody::Struct(fields) = &t.body else { continue };
        for f in fields {
            if let TypeExpr::Named { generic_args, .. } = &f.ty {
                if generic_args.len() == 1 {
                    return &generic_args[0];
                }
            }
        }
    }
    panic!("the probe program lost its `Box<…>` field");
}

/// The other direction: the shared table must decide EVERY primitive
/// the parser can spell — no silent third state where a primitive is
/// neither in `GENERIC_ARG_PRIMS` nor knowingly refused.
///
/// A new `PrimType` variant already fails to compile
/// (`generic_arg_mangle_token` matches exhaustively); this catches the
/// other slip, a variant answered `None` by the match and left out of
/// the list, which would shrink the vocabulary the diagnostic names
/// without refusing anything.
#[test]
fn the_vocabulary_partitions_every_primitive_the_parser_spells() {
    let supported: BTreeSet<&str> = hale_types::ty::GENERIC_ARG_PRIMS
        .iter()
        .copied()
        .map(hale_types::ty::prim_name)
        .collect();
    assert_eq!(
        supported.len(),
        hale_types::ty::GENERIC_ARG_PRIMS.len(),
        "`GENERIC_ARG_PRIMS` lists a primitive twice"
    );
    for p in hale_types::ty::GENERIC_ARG_PRIMS {
        assert_eq!(
            hale_types::ty::generic_arg_mangle_token(*p),
            Some(hale_types::ty::prim_name(*p)),
            "`{}` is in the vocabulary but mints no token",
            hale_types::ty::prim_name(*p)
        );
    }

    let all: BTreeSet<&str> =
        hale_syntax::parser::PRIMITIVE_TYPE_NAMES.iter().copied().collect();
    let refused: Vec<&&str> = all.difference(&supported).collect();
    // `Uint` is the whole of the refused set: parser-recognized with
    // no codegen representation in ANY storage position
    // (spec/types.md), so there is no monomorph to name. Anything else
    // arriving here is a primitive nobody decided about.
    assert_eq!(
        refused,
        vec![&"Uint"],
        "the refused set is supposed to be exactly `Uint`; the \
         vocabulary and the primitive spellings have drifted"
    );
    // And the refusal names the supported set, so an author reading it
    // learns what to write instead.
    let msg = hale_types::ty::generic_arg_refusal(
        hale_syntax::ast::PrimType::Uint,
    );
    assert!(msg.contains("`Uint`"), "the refusal must name it: {}", msg);
    for name in &supported {
        assert!(
            msg.contains(*name),
            "the refusal must name `{}` as supported: {}",
            name,
            msg
        );
    }
}

/// A mangle token is a NAME; this is the part that proves the four
/// tokens GH #907 added produce a working monomorph — a `Bytes`
/// stored in one and read back out.
///
/// Built through `build_executable` rather than the CLI because
/// constructing a monomorph needs its mangled name (`Box_Bytes { … }`)
/// and the checker has no monomorph path for that spelling yet — the
/// separate generic-monomorph item on GH #911. The DECLARATION half,
/// which is what #907 was about, is check-clean and covered by the
/// matrix above.
#[test]
fn a_bytes_generic_argument_round_trips_at_run_time() {
    let src = "type Box<T> {\n    item: T;\n}\n\n\
               type Holder {\n    b: Box<Bytes>;\n}\n\n\
               fn main() {\n    \
               let inner = Box_Bytes { item: std::bytes::from_string(\"abcd\") };\n    \
               let h = Holder { b: inner };\n    \
               println(\"b0=\", std::bytes::at(h.b.item, 0));\n}\n";
    let program = hale_syntax::parse_source(src).expect("parses");
    let bin = harness::unique_bin("hale_vg_bytes_monomorph");
    build_executable(&program, &bin).expect("a Bytes monomorph must build");
    let out = Command::new(&bin).output().expect("runs");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "exited {:?}: {}", out.status, stdout);
    // `"abcd"` — byte 0 is `a`, 97.
    assert!(
        stdout.contains("b0=97"),
        "the Bytes stored in `Box_Bytes` did not read back: {:?}",
        stdout
    );
}

// ===========================================================
// Table 2: the statement-position twin of BARE_BUILTIN_PROGRAMS
// ===========================================================

/// One whole program per `BARE_BUILTIN_CALLEES` entry, calling that
/// name in STATEMENT position — the same table
/// `corpus_check_build_agreement`'s `BARE_BUILTIN_PROGRAMS` covers in
/// expression position, so the two files read against each other.
///
/// The call sits on its own line and ends the statement, which is what
/// `statement_position_lowers_every_bare_builtin` asserts before it
/// builds anything: a program that lost its statement-position call
/// during an edit would build happily and prove nothing.
const STATEMENT_POSITION_PROGRAMS: &[(&str, &str)] = &[
    // Discarded values: every one of these died at build with
    // "builtin `NAME`" before GH #845.
    ("len", "fn main() {\n    len(\"ab\");\n}\n"),
    ("to_string", "fn main() {\n    to_string(7);\n}\n"),
    ("Int", "fn main() {\n    Int(3.9);\n}\n"),
    ("Float", "fn main() {\n    Float(3);\n}\n"),
    ("abs", "fn main() {\n    abs(0 - 2);\n}\n"),
    ("min", "fn main() {\n    min(1, 2);\n}\n"),
    ("max", "fn main() {\n    max(1, 2);\n}\n"),
    (
        "starts_with",
        "fn main() {\n    starts_with(\"ab\", \"a\");\n}\n",
    ),
    ("contains", "fn main() {\n    contains(\"ab\", \"b\");\n}\n"),
    ("__fmt", "fn main() {\n    __fmt(255, \"x\");\n}\n"),
    // The four printers: statement position is their natural home,
    // and they are here because the arm that serves them is now
    // chosen by name rather than by being the fallback.
    ("println", "fn main() {\n    println(\"x\");\n}\n"),
    ("print", "fn main() {\n    print(\"x\");\n}\n"),
    ("eprintln", "fn main() {\n    eprintln(\"x\");\n}\n"),
    ("eprint", "fn main() {\n    eprint(\"x\");\n}\n"),
    // The explicit-epoch closure surface — statement position only.
    (
        "check_closures",
        "locus Ledger {\n    params {\n        debits: Int = 0;\n        \
         credits: Int = 0;\n    }\n    closure balanced {\n        \
         self.debits ~~ self.credits within 0;\n        epoch explicit;\n    }\n    \
         fn post() {\n        self.debits = self.debits + 1;\n        \
         self.credits = self.credits + 1;\n        check_closures();\n    }\n}\n\n\
         main locus M {\n    params {\n        l: Ledger = Ledger { };\n    }\n    \
         run() {\n        self.l.post();\n    }\n}\n\n\
         fn main() {\n    M { };\n}\n",
    ),
    // bounded[T; N] intrinsics. `count` / `clear` / `truncate` have
    // their own statement-position arm ahead of this change's
    // fallback; `push` / `set` are fallible, so their statement
    // spelling carries the `or` disposition.
    (
        "count",
        "type B {\n    vals: bounded[Int; 4];\n}\n\n\
         fn main() {\n    let b = B { };\n    count(b.vals);\n}\n",
    ),
    (
        "clear",
        "type B {\n    vals: bounded[Int; 4];\n}\n\n\
         fn main() {\n    let b = B { };\n    clear(b.vals);\n}\n",
    ),
    (
        "truncate",
        "type B {\n    vals: bounded[Int; 4];\n}\n\n\
         fn main() {\n    let b = B { };\n    truncate(b.vals, 1);\n}\n",
    ),
    (
        "push",
        "type B {\n    vals: bounded[Int; 4];\n}\n\n\
         fn main() {\n    let b = B { };\n    push(b.vals, 7) or raise;\n}\n",
    ),
    (
        "at",
        "type B {\n    vals: bounded[Int; 4];\n}\n\n\
         fn main() {\n    let b = B { };\n    push(b.vals, 7) or raise;\n    \
         at(b.vals, 0) or 0;\n}\n",
    ),
    (
        "set",
        "type B {\n    vals: bounded[Int; 4];\n}\n\n\
         fn main() {\n    let b = B { };\n    push(b.vals, 7) or raise;\n    \
         set(b.vals, 0, 9) or raise;\n}\n",
    ),
];

/// The entries of `BARE_BUILTIN_CALLEES` that have no statement
/// position at all, with the reason — so the set comparison below is
/// exhaustive and a new name has to be classified rather than
/// forgotten.
const NO_STATEMENT_POSITION: &[(&str, &str)] = &[(
    "mean",
    "accumulator vocabulary: `mean(x)` is answered only inside a \
     closure assertion, and an assertion is an expression, so there \
     is no statement position to lower. (Outside one, `mean(x)` is \
     refused at build in BOTH positions while `hale check` accepts \
     it — the context-free half of the F.18 exemption, which is not \
     a statement-position gap.)",
)];

/// GH #911 B3 (#845) — a bare builtin call in statement position must
/// check clean AND build, for every name the checker exempts from the
/// F.18 strict-callee rule.
///
/// `BARE_BUILTIN_CALLEES` is the checker's promise that codegen
/// answers a name; before this, the promise was only ever tested in
/// expression position, and ten of the names were refused in
/// statement position — `hale check` accepting `Int(3);` and `hale
/// build` refusing it with "unsupported in codegen v0: builtin
/// `Int`", from another layer, with no span.
#[test]
fn statement_position_lowers_every_bare_builtin() {
    let tabled: BTreeSet<&str> =
        hale_types::check::BARE_BUILTIN_CALLEES.iter().copied().collect();
    let covered: BTreeSet<&str> =
        STATEMENT_POSITION_PROGRAMS.iter().map(|(n, _)| *n).collect();
    let exempt: BTreeSet<&str> =
        NO_STATEMENT_POSITION.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        covered.len(),
        STATEMENT_POSITION_PROGRAMS.len(),
        "two programs for one name in STATEMENT_POSITION_PROGRAMS"
    );
    assert!(
        covered.is_disjoint(&exempt),
        "a name is both covered and exempt: {:?}",
        covered.intersection(&exempt).collect::<Vec<_>>()
    );
    let classified: BTreeSet<&str> =
        covered.union(&exempt).copied().collect();
    assert_eq!(
        tabled,
        classified,
        "\n`BARE_BUILTIN_CALLEES` and this file's statement-position \
         table name different sets.\nIn the table only (needs a \
         statement-position program here, or an entry in \
         NO_STATEMENT_POSITION saying why it has none): {:?}\nHere \
         only (needs an entry in `crates/hale-types/src/check.rs`): \
         {:?}",
        tabled.difference(&classified).collect::<Vec<_>>(),
        classified.difference(&tabled).collect::<Vec<_>>(),
    );

    let mut failures: Vec<String> = Vec::new();
    for (i, (name, src)) in STATEMENT_POSITION_PROGRAMS.iter().enumerate() {
        // The vacuity guard: the call must really be a statement —
        // its own line, ending the statement.
        let in_statement_position = src.lines().any(|l| {
            let t = l.trim();
            t.starts_with(&format!("{}(", name)) && t.ends_with(';')
        });
        assert!(
            in_statement_position,
            "the program for `{}` has no statement-position call to \
             it:\n{}",
            name, src
        );
        let program = match hale_syntax::parse_source(src) {
            Ok(p) => p,
            Err(ds) => {
                let msgs: Vec<&str> =
                    ds.iter().map(|d| d.message.as_str()).collect();
                failures.push(format!(
                    "  `{}` does not parse: {}",
                    name,
                    msgs.join("; ")
                ));
                continue;
            }
        };
        let tag = format!("hale_vg_stmt_{}", i);
        match (check_accepts(&program), build_accepts(&program, &tag)) {
            (Ok(()), Ok(())) => {}
            (Err(e), Ok(())) => failures.push(format!(
                "  `{}` — check refuses a statement-position call the \
                 build accepts: {}",
                name, e
            )),
            (Ok(()), Err(e)) => failures.push(format!(
                "  `{}` — check accepts a statement-position call the \
                 build refuses: {}",
                name, e
            )),
            (Err(ce), Err(be)) => failures.push(format!(
                "  `{}` — neither layer accepts it: check {} / build {}",
                name, ce, be
            )),
        }
    }

    assert!(
        failures.is_empty(),
        "{} bare builtin(s) do not agree between check and build in \
         STATEMENT position (GH #911 B3, #845):\n{}\n\n\
         Statement position lowers the expression and discards its \
         value (`lower_stmt_inner`'s `Stmt::Expr(Expr::Call)` arm), so \
         a name that lowers in expression position must lower here \
         too. Either give it an arm, or drop it from \
         `BARE_BUILTIN_CALLEES` so the call gets the located \
         unknown-callee diagnostic in both positions.",
        failures.len(),
        failures.join("\n")
    );
}
