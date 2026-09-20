//! GH #725: a reserved word used as a name is reported ONCE, where it
//! is written.
//!
//! Hale reserves ordinary English words — `epoch`, `where`, `rich`,
//! `restart`, `tier`, `capacity` — and an author who names a field or
//! a local after one gets a parse error. That is intended. What was
//! not intended is what the author had to read: the parser abandoned
//! the whole declaration, so the FIRST line of output was often
//! something else entirely (a `}` further down reported as a stray
//! token, a literal of the type reported as a block) and the reserved
//! word was buried or, in a struct field, never named at all
//! (`expected member name, got Rich`).
//!
//! The rule now: name the word, name the declaration it was written
//! in, point at it, recover as if it had been named — and say nothing
//! more about that word, because every later occurrence is the same
//! mistake read back.
//!
//! Spans are asserted as `line:col` through the renderer, which is
//! what the author actually sees.

use hale_syntax::keywords::HARD_KEYWORDS;
use hale_syntax::parse_source;

/// Every diagnostic a source produces, as `line:col message`.
fn diags(src: &str) -> Vec<String> {
    match parse_source(src) {
        Ok(_) => Vec::new(),
        Err(ds) => ds
            .iter()
            .map(|d| {
                let (line, col) = d.span.line_col(src);
                format!("{}:{} {}", line, col, d.message)
            })
            .collect(),
    }
}

/// The one diagnostic, or a panic naming everything that came out.
fn only_diag(src: &str) -> String {
    let ds = diags(src);
    assert_eq!(
        ds.len(),
        1,
        "expected exactly one diagnostic, got {}:\n{}",
        ds.len(),
        ds.join("\n")
    );
    ds.into_iter().next().unwrap()
}

const TAIL: &str = "rename it (Hale has no escaped-identifier form \
                    — spec/tokens.md lists every reserved word)";

// === the positions the issue names ===========================

#[test]
fn params_field() {
    let src = "\
locus Counter {
    params {
        epoch: Int = 0;
    }
    fn value() -> Int { return 1; }
}
fn main() { }
";
    assert_eq!(
        only_diag(src),
        format!(
            "3:9 `epoch` is a reserved word and cannot name a params \
             field; {}",
            TAIL
        )
    );
}

#[test]
fn local_binding() {
    // And the use site: `where` is read back on the next line, which
    // is the same mistake — one diagnostic, not two.
    let src = "\
fn main() {
    let where = 3;
    println(\"{}\", where);
}
";
    assert_eq!(
        only_diag(src),
        format!(
            "2:9 `where` is a reserved word and cannot name a \
             variable; {}",
            TAIL
        )
    );
}

#[test]
fn free_fn_name() {
    let src = "\
fn restart() { println(\"x\"); }
fn main() { restart(); }
";
    assert_eq!(
        only_diag(src),
        format!(
            "1:4 `restart` is a reserved word and cannot name a \
             function; {}",
            TAIL
        )
    );
}

#[test]
fn locus_method_name_and_its_call() {
    // The call `w.restart()` used to add `expected member name, got
    // Restart` — a second diagnostic, in `{:?}`, about a word the
    // author had already been told about.
    let src = "\
locus Worker {
    fn restart() { println(\"x\"); }
}
fn main() {
    let w = Worker { };
    w.restart();
}
";
    assert_eq!(
        only_diag(src),
        format!(
            "2:8 `restart` is a reserved word and cannot name a \
             function; {}",
            TAIL
        )
    );
}

#[test]
fn struct_field_and_its_literal() {
    // Two regressions in one shape: the field position went through
    // the member-name path, which never mentioned "reserved" at all,
    // and the literal below then failed the struct-literal lookahead
    // and reported `expected ;, got LBrace` on a line the author had
    // no reason to suspect.
    let src = "\
type Report {
    rich: Int;
    plain: Int;
}
fn main() {
    let r = Report { rich: 1, plain: 2 };
    println(\"{}\", r.plain);
}
";
    assert_eq!(
        only_diag(src),
        format!(
            "2:5 `rich` is a reserved word and cannot name a field; {}",
            TAIL
        )
    );
}

#[test]
fn method_parameter_and_its_body() {
    let src = "\
locus Ranker {
    fn rank(tier: Int) -> Int { return tier + 1; }
}
fn main() {
    let r = Ranker { };
    println(\"{}\", r.rank(2));
}
";
    assert_eq!(
        only_diag(src),
        format!(
            "2:13 `tier` is a reserved word and cannot name a \
             parameter; {}",
            TAIL
        )
    );
}

#[test]
fn locus_name() {
    let src = "locus release { }\nfn main() { }\n";
    assert_eq!(
        only_diag(src),
        format!(
            "1:7 `release` is a reserved word (a lifecycle keyword) \
             and cannot name a locus; {}",
            TAIL
        )
    );
}

#[test]
fn type_name() {
    let src = "type capacity { n: Int; }\nfn main() { }\n";
    assert_eq!(
        only_diag(src),
        format!(
            "1:6 `capacity` is a reserved word and cannot name a \
             type; {}",
            TAIL
        )
    );
}

#[test]
fn const_name() {
    let src = "const publish: Int = 1;\nfn main() { }\n";
    assert_eq!(
        only_diag(src),
        format!(
            "1:7 `publish` is a reserved word and cannot name a \
             const; {}",
            TAIL
        )
    );
}

#[test]
fn loop_variable() {
    let src = "\
fn main() {
    for of in 0..3 { println(\"{}\", of); }
}
";
    assert_eq!(
        only_diag(src),
        format!(
            "2:9 `of` is a reserved word and cannot name a loop \
             variable; {}",
            TAIL
        )
    );
}

// === the cascade this replaced ================================

#[test]
fn the_rest_of_the_file_still_parses() {
    // The declaration after the bad one used to be swallowed by
    // recovery: the parser skipped to the next top-level keyword,
    // took the locus's own `fn` as a top-level fn, and then reported
    // the locus's closing brace as a stray token. Recovery now
    // resumes inside the declaration, so nothing after it is
    // misread — one diagnostic for the whole file.
    let src = "\
locus Counter {
    params {
        epoch: Int = 0;
    }
    fn value() -> Int { return 1; }
}
type Shape { sides: Int; }
fn main() { println(\"{}\", Shape { sides: 3 }.sides); }
";
    let ds = diags(src);
    assert_eq!(ds.len(), 1, "one diagnostic; got:\n{}", ds.join("\n"));
    assert!(
        !ds[0].contains("RBrace"),
        "no stray-brace follow-on; got: {}",
        ds[0]
    );
}

#[test]
fn two_different_reserved_words_are_both_reported() {
    // Dedup is per WORD, not per file: two distinct mistakes are two
    // diagnostics. Only the repeat of one word is folded away.
    let src = "\
fn main() {
    let where = 1;
    let epoch = 2;
    println(\"{}{}\", where, epoch);
}
";
    let ds = diags(src);
    assert_eq!(ds.len(), 2, "one per word; got:\n{}", ds.join("\n"));
    assert!(ds[0].contains("`where`"), "got: {}", ds[0]);
    assert!(ds[1].contains("`epoch`"), "got: {}", ds[1]);
}

#[test]
fn a_keyword_in_expression_position_keeps_its_own_error() {
    // The recovery is narrow on purpose: a keyword in expression
    // position with no misnamed declaration behind it is a DIFFERENT
    // mistake, and silently reading it as an identifier would let a
    // nonsense program through the parser.
    let src = "fn main() { let x = dissolve; }\n";
    let d = only_diag(src);
    assert!(d.contains("expected expression"), "got: {d}");
}

#[test]
fn a_non_keyword_token_keeps_the_debug_fallback() {
    let src = "fn main() { let 5 = 1; }\n";
    let d = only_diag(src);
    assert!(d.contains("expected variable name, got"), "got: {d}");
    assert!(!d.contains("reserved"), "got: {d}");
}

// === controls: every legitimate use still means what it meant ===

#[test]
fn lifecycle_and_closure_keywords_still_parse_in_position() {
    let src = "\
type Sample { value: Int; }

locus Job: tier 1, projection rich {
    params { delta: Int = 0; }
    capacity { heap items of Int; }
    contract { expose delta: Int; }
    closure within_band {
        sum(self.delta) ~~ 0 within 100;
        epoch tick;
        persists_through (quarantine);
    }
    bus { subscribe \"data\" as on_data of type Sample; }
    fn on_data(s: Sample) { self.delta = s.value; }
    birth { println(\"born\"); }
    run { println(\"run\"); }
    drain { println(\"drain\"); }
    dissolve { println(\"gone\"); }
}

main locus App {
    params { j: Job = Job { }; }
    run { println(\"{}\", self.j.delta); }
}
";
    assert_eq!(diags(src), Vec::<String>::new());
}

#[test]
fn bus_and_reclamation_keywords_still_parse_in_position() {
    // `where` in a keyed subscription, `publish` / `accept` /
    // `release` as members, `keyed_by` / `payload` / `subject` in a
    // topic — the same words the diagnostic refuses as names.
    let src = "\
type Nudge { key: String = \"\"; n: Int = 0; }
topic Nudges { payload: Nudge; subject: \"c725.nudge\"; keyed_by key; }

locus Kid {
    params { key: String = \"\"; }
    bus { subscribe Nudges as on_nudge where key == self.key; }
    fn on_nudge(n: Nudge) { println(\"{}\", n.n); }
}

main locus App {
    bus { publish Nudges; }
    accept(c: Kid) { }
    release(c: Kid) { }
    run() {
        let k = Kid { key: \"a\" };
        Nudges <- Nudge { key: \"a\", n: 1 };
        println(\"{}\", k.key);
    }
}
";
    assert_eq!(diags(src), Vec::<String>::new());
}

#[test]
fn keywords_admitted_as_field_names_are_unaffected() {
    // `run` / `birth` / `tier` / `capacity` are deliberately
    // admissible as field names (v1.x-8) — the reserved-word
    // diagnostic must not reach them.
    let src = "\
type Cmd {
    run: Int;
    birth: Int;
    tier: Int;
    capacity: Int;
}
fn main() {
    let c = Cmd { run: 1, birth: 2, tier: 3, capacity: 4 };
    println(\"{}\", c.run + c.tier);
}
";
    assert_eq!(diags(src), Vec::<String>::new());
}

#[test]
fn contextual_keywords_are_still_free_as_names() {
    // The contextual half of the keyword table is recognized only in
    // position, and this fix must not have promoted any of it.
    let src = "\
fn main() {
    let mode = 1;
    let pool = 2;
    let with = 3;
    let seed = 4;
    let count = 5;
    println(\"{}\", mode + pool + with + seed + count);
}
";
    assert_eq!(diags(src), Vec::<String>::new());
}

// === the list the diagnostic points at ========================

#[test]
fn the_spec_lists_every_reserved_word_the_diagnostic_cites() {
    // The message ends "spec/tokens.md lists every reserved word".
    // That is a claim about a file, so check it: an author sent there
    // for a word the section forgot learns nothing.
    let spec = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../spec/tokens.md"),
    )
    .expect("read spec/tokens.md");
    let missing: Vec<&str> = HARD_KEYWORDS
        .iter()
        .copied()
        .filter(|kw| {
            !spec.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|w| w == *kw)
        })
        .collect();
    assert!(
        missing.is_empty(),
        "reserved words the diagnostic sends authors to spec/tokens.md \
         for, but which it does not mention: {:?}",
        missing
    );
}
