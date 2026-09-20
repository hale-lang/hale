//! A program the checker accepts must also build.
//!
//! `hale check` passing and `hale build` failing is the worst
//! failure mode the toolchain has: the error arrives late, from a
//! different layer, usually without a source location, and it tells
//! the author their *working* program is unbuildable.
//!
//! Three of these were found in a single afternoon, by accident,
//! while writing ordinary example code:
//!
//!   * `restart(c) for N` — checked, modelled in the artifact, and
//!     refused by codegen ("recovery modifier not lowered")
//!   * `handler: u` naming a sibling param — checked, then "unknown
//!     identifier `u`" with no location
//!   * `std::http::Server { port: 9100 }` — checked, then "param
//!     `handler` is required"
//!
//! They shared a hiding place. The 92 fixtures under
//! `tests/fixtures/examples/` are compiled and RUN by
//! `corpus_oracle`, but the ~1400 programs embedded in Rust test
//! strings were only ever typechecked — and all three lived there.
//!
//! So this closes the gap: every embedded program the checker
//! accepts is put through codegen. Two exclusions, both principled:
//!
//!   * programs the checker REJECTS are diagnostic fixtures; being
//!     unbuildable is their purpose;
//!   * programs that `import` a sibling seed, because the harvester
//!     takes one seed at a time — the other half is not present, so
//!     "unknown qualified name `t::Intent`" is the harvester
//!     speaking, not the compiler.
//!
//! ## The third exclusion, removed (GH #829)
//!
//! There used to be a third: "programs with no entry point cannot be
//! built by definition". That was wrong, and it was a hole in the
//! gate exactly the shape of the programs this file exists to catch.
//! Codegen synthesizes the C `main` itself and lowers every
//! declaration whether or not a user `fn main` calls it, so an
//! entry-point-less program compiles fine — and the checks that
//! DIVERGE (a stdlib call with a wrong-typed argument, a construct
//! codegen never learned to lower) live in those declarations, not
//! in the entry point. A `locus`-only snippet embedded in a Rust
//! test — the harvester's staple, since `program_like` admits any
//! literal with a top-level `locus` — was therefore typechecked and
//! never compiled.
//!
//! `crates/hale-types/tests/bus_graph.rs`'s
//! `std::io::tcp::recv_into(0, 0, 64)` was one such program: check
//! clean, build refused. So every harvested program now goes through
//! codegen; one with no entry point gets a synthetic `fn main() { }`
//! appended ([`with_synthetic_main`]) and is built like any other.
//!
//! Slow (it lowers and links each one), so it is `#[ignore]`d like
//! the oracle's sanitizer sweep and run explicitly in CI. The
//! programs are independent, so the sweep hands them out to worker
//! threads and aggregates by index — see `sweep_threads`, and
//! `HALE_CORPUS_SWEEP_THREADS=1` to walk it serially.
//!
//! ## The other direction (GH #779)
//!
//! `strict_check_refuses_nothing_the_build_accepts` walks the same
//! corpus for the converse failure: `hale check <dir>` refusing a
//! program `hale build` accepts. It is NOT ignored, because it only
//! builds the programs the strict rule refuses — a handful, not 1400.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use hale_codegen::build_executable;
use hale_syntax::ast::TopDecl;

#[path = "support/harness.rs"]
mod harness;

/// Does something in this program start it?
fn has_entry_point(program: &hale_syntax::ast::Program) -> bool {
    program.items.iter().any(|i| match i {
        TopDecl::Fn(f) => f.name.name == "main",
        TopDecl::Locus(l) => l.is_main,
        _ => false,
    })
}

/// GH #829: a program with no entry point, with `fn main() { }`
/// appended — the smallest thing that makes it a program without
/// changing what codegen must lower. Every declaration it carries is
/// still lowered (codegen walks the decls, not the call graph), so
/// the divergences that hide in a library-shaped snippet are reached.
///
/// A program that already has an entry point is returned untouched:
/// a second `main` would be the harness inventing a compile error.
fn with_synthetic_main(
    program: &hale_syntax::ast::Program,
) -> std::borrow::Cow<'_, hale_syntax::ast::Program> {
    if has_entry_point(program) {
        return std::borrow::Cow::Borrowed(program);
    }
    let stub = hale_syntax::parse_source("fn main() { }\n")
        .expect("the synthetic entry point must parse");
    let mut wrapped = program.clone();
    wrapped.items.extend(stub.items);
    std::borrow::Cow::Owned(wrapped)
}

/// The sweep's verdict on one harvested program.
#[derive(Debug, PartialEq)]
enum Verdict {
    /// Not the sweep's business (see the exclusions in the module
    /// doc); it carries the reason so a test can say which.
    Skipped(&'static str),
    Built,
    /// `hale check` accepted it and `hale build` refused it — the
    /// divergence this file exists to catch. Carries the codegen
    /// error, rendered.
    Refused(String),
}

/// One program, decided the way the sweep decides it.
///
/// A function rather than the body of the loop so the coverage can
/// be pinned by a fast test: `an_entry_point_less_program_is_built`
/// asks this for its verdict on a program with no entry point, and
/// restoring the GH #829 skip makes that test fail immediately
/// instead of quietly shrinking the ~1500-program sweep.
fn sweep_verdict(source: &str, bin_tag: &str) -> Verdict {
    let Ok(program) = hale_syntax::parse_source(source) else {
        return Verdict::Skipped("does not parse");
    };
    // Diagnostic fixtures are SUPPOSED to fail; their being
    // unbuildable is not a divergence.
    if hale_types::check_program(&program).iter().any(|d| d.is_error()) {
        return Verdict::Skipped("the checker rejects it");
    }
    // Cross-seed: the sibling seed is not in this fragment.
    // Matched on the source because an import is not a
    // `TopDecl` — it is consumed before the AST.
    if source.contains("import \"") {
        return Verdict::Skipped("imports a sibling seed");
    }
    // GH #829: no entry point is not a reason to skip; it is a
    // reason to add one.
    let program = with_synthetic_main(&program);
    let bin = harness::unique_bin(bin_tag);
    match build_executable(&program, &bin) {
        Ok(()) => {
            let _ = std::fs::remove_file(&bin);
            Verdict::Built
        }
        Err(e) => Verdict::Refused(format!("{:?}", e)),
    }
}

/// How many programs the sweep decides at once.
///
/// `HALE_CORPUS_SWEEP_THREADS=1` restores the serial walk, which is
/// what to reach for when a failure needs to be read without
/// interleaving — the verdicts are per-program and the aggregation
/// is by index, so the two runs report identically.
fn sweep_threads() -> usize {
    if let Ok(v) = std::env::var("HALE_CORPUS_SWEEP_THREADS") {
        if let Ok(n) = v.parse::<usize>() {
            if n > 0 {
                return n;
            }
        }
    }
    // Capped like `corpus_oracle`'s pool: each worker runs a clang
    // link, so the useful width is bounded well before the core
    // count on a big machine, and CI runners have four.
    std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4)
}

#[test]
#[ignore = "compiles ~1500 programs; run explicitly (see corpus_oracle)"]
fn every_check_clean_corpus_program_also_builds() {
    let mut checked = 0usize;
    let mut built = 0usize;
    // origin -> the codegen error, deduplicated by message so a
    // single unlowered construct reports once with its sites rather
    // than a hundred times.
    let mut failures: BTreeMap<String, Vec<String>> = BTreeMap::new();

    // One program's verdict does not depend on another's, and
    // `build_executable` is already driven from eight threads of one
    // process by `corpus_oracle`: the LLVM `Context` is created per
    // call, the object path derives from the caller's
    // `harness::unique_bin` (pid + an atomic process-local counter),
    // and the cached runtime objects are written to a per-call temp
    // path and renamed into place precisely so concurrent same-process
    // builds cannot clobber each other. So the sweep is handed out
    // across workers rather than walked.
    //
    // The ORDER is preserved deliberately: each verdict is stored at
    // its program's index and aggregated afterwards in corpus order,
    // so `checked`, `built` and the ratchet's rendered list are
    // byte-identical to the serial run. Nothing here may depend on
    // completion order.
    let programs =
        hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok());
    let next = AtomicUsize::new(0);
    let n_workers = sweep_threads().min(programs.len().max(1));
    let programs = &programs;
    let per_worker: Vec<Vec<(usize, Verdict)>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..n_workers)
            .map(|_| {
                let next = &next;
                scope.spawn(move || {
                    let mut mine: Vec<(usize, Verdict)> = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(p) = programs.get(i) else { break };
                        // The tag is the program's index rather than a
                        // running count of checked programs: it only
                        // names a temp file, and an index is unique
                        // without a shared counter.
                        mine.push((
                            i,
                            sweep_verdict(&p.source, &format!("hale_cb_{}", i)),
                        ));
                    }
                    mine
                })
            })
            .collect();
        handles
            .into_iter()
            // Re-raise a worker's panic as its own panic rather than
            // burying the message in a join error.
            .map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    });

    let mut verdicts: Vec<Option<Verdict>> =
        (0..programs.len()).map(|_| None).collect();
    for (i, v) in per_worker.into_iter().flatten() {
        verdicts[i] = Some(v);
    }

    for (p, v) in programs.iter().zip(verdicts) {
        let v = v.expect("every swept program is handed out exactly once");
        match v {
            Verdict::Skipped(_) => continue,
            Verdict::Built => {
                checked += 1;
                built += 1;
            }
            Verdict::Refused(e) => {
                checked += 1;
                failures.entry(e).or_default().push(p.origin.clone());
            }
        }
    }

    // The sweep must actually cover something: a harvester change
    // that silently matched nothing would otherwise pass forever.
    assert!(
        checked > 200,
        "only {} check-clean buildable programs found — the corpus \
         sweep is broken, not the compiler",
        checked
    );

    // A RATCHET, not a clean bill of health. 44 divergences exist
    // today over 1507 swept programs (GH #829 widened the sweep by
    // the 88 programs with no entry point, which found four more
    // and fixed four); each is a check the compiler performs in
    // codegen that the checker could perform earlier, with a span.
    // They are recorded so that:
    //
    //   * a NEW divergence fails immediately — that is the point;
    //   * a FIXED one also fails, so the list cannot quietly rot
    //     into a list of things that are no longer true.
    //
    // Shrinking it is the work. Regenerate with
    // HALE_REGEN_CHECK_BUILD=1 only after reading the diff.
    let mut current: Vec<String> = failures
        .iter()
        .flat_map(|(_, origins)| origins.iter().cloned())
        .collect();
    current.sort();
    current.dedup();
    let rendered = current.join("\n") + "\n";

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/check_build_divergences.txt");
    if std::env::var("HALE_REGEN_CHECK_BUILD").as_deref() == Ok("1") {
        std::fs::write(&path, &rendered).expect("write baseline");
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    if rendered == expected {
        return;
    }

    let exp: std::collections::BTreeSet<&str> =
        expected.lines().filter(|l| !l.is_empty()).collect();
    let cur: std::collections::BTreeSet<&str> =
        current.iter().map(|s| s.as_str()).collect();
    let new_divergences: Vec<&&str> =
        cur.difference(&exp).collect();
    let fixed: Vec<&&str> = exp.difference(&cur).collect();

    let mut msg = String::new();
    if !new_divergences.is_empty() {
        msg.push_str(&format!(
            "\n{} NEW check/build divergence(s) — `hale check` accepts \
             what `hale build` refuses, which tells an author their \
             working program is broken, late, from another layer:\n",
            new_divergences.len()
        ));
        for o in &new_divergences {
            let err = failures
                .iter()
                .find(|(_, v)| v.iter().any(|x| x == **o))
                .map(|(k, _)| k.as_str())
                .unwrap_or("?");
            msg.push_str(&format!("  {}\n    {}\n", o, err));
        }
    }
    if !fixed.is_empty() {
        msg.push_str(&format!(
            "\n{} recorded divergence(s) now build — good. Regenerate \
             the baseline so it keeps meaning what it says:\n",
            fixed.len()
        ));
        for o in &fixed {
            msg.push_str(&format!("  {}\n", o));
        }
    }
    panic!(
        "{}\n{} of {} check-clean programs do not build.\n\
         Regenerate: HALE_REGEN_CHECK_BUILD=1 cargo test --release \
         -p hale-codegen --test corpus_check_build_agreement -- --ignored",
        msg,
        checked - built,
        checked
    );
}

/// GH #829 — the coverage the sweep gained, pinned cheaply.
///
/// Two locus-only programs, neither with an entry point, both
/// check-clean. One lowers; the other is refused by codegen. Before
/// #829 the sweep skipped BOTH on "no entry point", so the refused
/// one — a check-accepts/build-refuses divergence, the exact thing
/// this file is the gate for — was invisible to it.
///
/// The verdicts come from [`sweep_verdict`], which is the sweep's
/// own decision procedure, so restoring the skip fails here in
/// seconds rather than silently shrinking an `#[ignore]`d sweep
/// nobody runs by hand.
///
/// The programs are deliberately PLAIN string literals: the corpus
/// harvester scrapes `r#"…"#` literals out of test sources, and a
/// program written to be unbuildable would otherwise harvest itself
/// into the sweep's own ratchet baseline. Do not "tidy" them into
/// raw strings.
#[test]
fn an_entry_point_less_program_is_built() {
    let lowers = "locus Counter {\n\
                  params { n: Int = 0; }\n\
                  fn bump() { self.n = self.n + 1; }\n\
                  }\n";
    // `or discard` needs a Unit success type; `std::process::run`
    // yields a value. The checker does not say so (that is GH #791's
    // territory) and codegen does, with no span — a divergence.
    let refused = "locus Runner {\n\
                   fn go() { std::process::run(\"true\") or discard; }\n\
                   }\n";

    for src in [lowers, refused] {
        let program = hale_syntax::parse_source(src).expect("parses");
        assert!(
            !has_entry_point(&program),
            "this test is about programs with NO entry point; this one \
             has one, so it proves nothing:\n{}",
            src
        );
        let errors: Vec<String> = hale_types::check_program(&program)
            .into_iter()
            .filter(|d| d.is_error())
            .map(|d| d.message)
            .collect();
        assert!(
            errors.is_empty(),
            "a program the checker rejects is a diagnostic fixture and \
             is skipped for that reason instead — this one must check \
             clean to test what it claims to: {:?}\n{}",
            errors,
            src
        );
    }

    assert_eq!(
        sweep_verdict(lowers, "hale_cb_ep_lowers"),
        Verdict::Built,
        "an entry-point-less program that lowers must be BUILT by the \
         sweep, not skipped"
    );

    match sweep_verdict(refused, "hale_cb_ep_refused") {
        Verdict::Refused(e) => assert!(
            e.contains("or discard"),
            "expected the `or discard` refusal, got: {}",
            e
        ),
        other => panic!(
            "an entry-point-less program the checker accepts and codegen \
             refuses is a check/build DIVERGENCE the sweep must see; got \
             {:?}",
            other
        ),
    }
}

/// The bare name in ``call to `X`: no free fn, generic fn or
/// fn-pointer binding with that name is in scope``, if that is what
/// this diagnostic is.
fn strict_callee_name(message: &str) -> Option<&str> {
    let rest = message.strip_prefix("call to `")?;
    let (name, tail) = rest.split_once('`')?;
    tail.starts_with(
        ": no free fn, generic fn or fn-pointer binding with that name \
         is in scope",
    )
    .then_some(name)
}

/// GH #779 — the admission gate must not refuse a program the build
/// accepts.
///
/// `hale check <dir>` checked a WHOLE seed, so it turns on F.18's
/// strict-callee rule: a call to a bare name nothing binds is an
/// error. The rule exempts the names codegen answers itself, read
/// from `hale_types`' `BARE_BUILTIN_CALLEES` — a hand-maintained
/// table standing in for a dispatch spread across a dozen match arms
/// in three lowering positions (expression, statement, and the
/// fallible `or` path). It drifted, silently: `starts_with`,
/// `contains`, `eprint`, `check_closures` and `mean` were all
/// missing, so `hale check` refused four corpus fixtures that build
/// and run — the worst thing an admission gate can do, because the
/// author's program is correct and the gate is the thing that is
/// wrong.
///
/// So the table is not trusted; it is checked. Every corpus program
/// goes through the strict rule, and any program the rule refuses is
/// BUILT. If codegen accepts it, the table is missing a name and this
/// test says which. Only refused programs are compiled, which is why
/// this one runs in the ordinary suite while its sibling above is
/// `#[ignore]`d.
#[test]
fn strict_check_refuses_nothing_the_build_accepts() {
    let mut swept = 0usize;
    let mut refused = 0usize;
    // bare name -> the origins whose build succeeded anyway
    let mut divergences: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for p in hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok())
    {
        let Ok(program) = hale_syntax::parse_source(&p.source) else {
            continue;
        };
        // Diagnostic fixtures already fail the permissive check; the
        // strict rule's opinion of them is beside the point.
        if hale_types::check_program(&program)
            .iter()
            .any(|d| d.is_error())
        {
            continue;
        }
        if !has_entry_point(&program) {
            continue;
        }
        // Cross-seed: the sibling seed is not in this fragment, so a
        // name it defines reads as unbound here for a reason that is
        // the harvester's, not the compiler's.
        if p.source.contains("import \"") {
            continue;
        }
        swept += 1;

        // What `hale check <dir>` runs: whole-seed strictness, both
        // flags on.
        let mut programs: BTreeMap<String, &hale_syntax::ast::Program> =
            BTreeMap::new();
        programs.insert(String::new(), &program);
        let bundle = hale_types::Bundle::new(programs);
        let names: BTreeSet<String> =
            hale_types::check_bundle_opts_scoped(&bundle, false, true, true)
                .iter()
                .filter(|d| d.is_error())
                .filter_map(|d| strict_callee_name(&d.message))
                .map(|n| n.to_string())
                .collect();
        if names.is_empty() {
            continue;
        }
        refused += 1;

        // The rule refused it. Does codegen answer these names anyway?
        let bin = harness::unique_bin(&format!("hale_strict_{}", refused));
        if build_executable(&program, &bin).is_ok() {
            let _ = std::fs::remove_file(&bin);
            for n in names {
                divergences.entry(n).or_default().push(p.origin.clone());
            }
        }
    }

    // A corpus walk that silently matched nothing would pass forever.
    assert!(
        swept > 200,
        "only {} check-clean buildable programs swept — the corpus walk \
         is broken, not the compiler",
        swept
    );

    assert!(
        divergences.is_empty(),
        "`hale check <dir>` refuses {} bare name(s) that codegen answers \
         itself, so the admission gate rejects programs `hale run` \
         executes (GH #779):\n{:#?}\n\n\
         Add each name to `BARE_BUILTIN_CALLEES` in \
         `crates/hale-types/src/check.rs` — it is the checker's copy of \
         codegen's bare-callee dispatch, and this is the test that keeps \
         the copy honest. ({} of {} swept programs are refused by the \
         strict rule.)",
        divergences.len(),
        divergences,
        refused,
        swept
    );
}

/// One whole program per entry of `BARE_BUILTIN_CALLEES`, calling
/// that name in the position its dispatch site serves.
///
/// Keep it sorted the way the table is grouped, so the two read
/// against each other.
const BARE_BUILTIN_PROGRAMS: &[(&str, &str)] = &[
    // lower_expr's `Expr::Call` arms.
    ("len", "fn main() { println(len(\"ab\")); }\n"),
    ("to_string", "fn main() { println(to_string(7)); }\n"),
    ("Int", "fn main() { println(Int(3.9)); }\n"),
    ("Float", "fn main() { println(Float(3)); }\n"),
    ("abs", "fn main() { println(abs(0 - 2)); }\n"),
    ("min", "fn main() { println(min(1, 2)); }\n"),
    ("max", "fn main() { println(max(1, 2)); }\n"),
    // lower_str_predicate_builtin.
    (
        "starts_with",
        "fn main() { println(starts_with(\"ab\", \"a\")); }\n",
    ),
    ("contains", "fn main() { println(contains(\"ab\", \"b\")); }\n"),
    // lower_print_call's four printers.
    ("println", "fn main() { println(\"x\"); }\n"),
    ("print", "fn main() { print(\"x\"); }\n"),
    ("eprintln", "fn main() { eprintln(\"x\"); }\n"),
    ("eprint", "fn main() { eprint(\"x\"); }\n"),
    // The explicit-epoch closure surface, statement position.
    (
        "check_closures",
        "locus Ledger { params { debits: Int = 0; credits: Int = 0; }\n\
         closure balanced { self.debits ~~ self.credits within 0; epoch explicit; }\n\
         fn post() { self.debits = self.debits + 1; self.credits = self.credits + 1; check_closures(); } }\n\
         main locus M { params { l: Ledger = Ledger { }; } run() { self.l.post(); } }\n\
         fn main() { M { }; }\n",
    ),
    // Accumulator vocabulary inside a closure assertion.
    (
        "count",
        "main locus T { params { delta: Float = 0.0; }\n\
         closure counted { count() ~~ 1 within 0; epoch tick; }\n\
         run() { self.delta = 1.0; } }\n\
         fn main() { T { }; }\n",
    ),
    (
        "mean",
        "main locus T { params { delta: Float = 0.0; }\n\
         closure mean_in_band { mean(self.delta) ~~ 0.0 within 100.0; epoch tick; }\n\
         run() { self.delta = 1.0; } }\n\
         fn main() { T { }; }\n",
    ),
    // bounded[T; N] intrinsics. `clear` / `truncate` lower direct;
    // `push` / `at` / `set` are fallible and go through the `or`
    // path (`try_lower_bounded_fallible_intrinsic`).
    (
        "clear",
        "type B { vals: bounded[Int; 4]; }\n\
         fn main() { let b = B { }; clear(b.vals); }\n",
    ),
    (
        "truncate",
        "type B { vals: bounded[Int; 4]; }\n\
         fn main() { let b = B { }; println(truncate(b.vals, 1)); }\n",
    ),
    (
        "push",
        "type B { vals: bounded[Int; 4]; }\n\
         fn main() { let b = B { }; push(b.vals, 7) or raise; }\n",
    ),
    (
        "at",
        "type B { vals: bounded[Int; 4]; }\n\
         fn main() { let b = B { }; push(b.vals, 7) or raise; let v = at(b.vals, 0) or 0; println(v); }\n",
    ),
    (
        "set",
        "type B { vals: bounded[Int; 4]; }\n\
         fn main() { let b = B { }; push(b.vals, 7) or raise; set(b.vals, 0, 9) or raise; }\n",
    ),
    // lower_fmt_builtin. Both spellings: the f-string the author
    // writes, and the desugared call the table actually names.
    (
        "__fmt",
        "fn main() { println(f\"{255:x}\"); println(__fmt(255, \"x\")); }\n",
    ),
];

/// GH #800 — the mirror of the sweep above: a bare name the table
/// ADMITS must be one codegen can lower.
///
/// #779 tested one direction (a name codegen answers must be
/// listed) and deliberately left the other alone, so the table kept
/// ten names with no codegen arm anywhere: `hex`, `panic`, `exit`,
/// the six primitive-type spellings `Float` / `String` / `Bool` /
/// `Bytes` / `Decimal` / `Duration`, and `prod`. `hale check <dir>`
/// accepted a call to each of them and `hale build` refused it, at
/// lowering, with no span — the check-accepts/build-refuses
/// divergence this whole file exists to catch, sitting inside the
/// table the file's other test reads.
///
/// So the table is now exact in both directions, and this is the
/// half that keeps it exact: every entry gets a whole program that
/// calls it, and every program must BUILD. A name added to the
/// table without a program here fails on the set comparison; a name
/// whose arm is removed or narrowed fails on the build.
#[test]
fn every_bare_builtin_callee_lowers() {
    let tabled: BTreeSet<&str> =
        hale_types::check::BARE_BUILTIN_CALLEES.iter().copied().collect();
    let covered: BTreeSet<&str> =
        BARE_BUILTIN_PROGRAMS.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        covered.len(),
        BARE_BUILTIN_PROGRAMS.len(),
        "two programs for one name in BARE_BUILTIN_PROGRAMS"
    );
    assert_eq!(
        tabled,
        covered,
        "\n`BARE_BUILTIN_CALLEES` and this file's program table name \
         different sets.\nIn the table only (needs a program here, or \
         it is a name `hale check` admits and nothing proves \
         buildable): {:?}\nIn the programs only (needs an entry in \
         `crates/hale-types/src/check.rs`): {:?}",
        tabled.difference(&covered).collect::<Vec<_>>(),
        covered.difference(&tabled).collect::<Vec<_>>(),
    );

    let mut failures: Vec<String> = Vec::new();
    for (i, (name, src)) in BARE_BUILTIN_PROGRAMS.iter().enumerate() {
        // Guards the vacuous pass: a program that lost its call
        // during an edit would build happily and prove nothing.
        assert!(
            src.contains(name),
            "the program for `{}` does not mention it:\n{}",
            name,
            src
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
        let bin = harness::unique_bin(&format!("hale_bbc_{}", i));
        match build_executable(&program, &bin) {
            Ok(()) => {
                let _ = std::fs::remove_file(&bin);
            }
            Err(e) => {
                failures.push(format!("  `{}` — {:?}", name, e));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} bare name(s) that `BARE_BUILTIN_CALLEES` exempts from the \
         F.18 strict-callee rule cannot be lowered, so `hale check \
         <dir>` accepts a call `hale build` refuses (GH #800):\n{}\n\n\
         Either give codegen the arm, or drop the name from \
         `BARE_BUILTIN_CALLEES` in `crates/hale-types/src/check.rs` so \
         the call gets the located unknown-callee diagnostic instead.",
        failures.len(),
        failures.join("\n")
    );
}

/// GH #800 — the fourth parallel list. `IMPURE_BARE_BUILTINS`
/// (`crates/hale-types/src/purity.rs`) is consulted only for an
/// `Expr::Ident` in callee position, so a name it lists that the
/// compiler does not answer as a bare callee describes the purity
/// of a call that cannot compile. Nothing forced the two to agree;
/// this does.
#[test]
fn purity_bare_builtins_are_bare_callees() {
    let callees: BTreeSet<&str> =
        hale_types::check::BARE_BUILTIN_CALLEES.iter().copied().collect();
    let orphans: Vec<&&str> = hale_types::purity::impure_bare_builtins()
        .iter()
        .filter(|n| !callees.contains(**n))
        .collect();
    assert!(
        orphans.is_empty(),
        "`IMPURE_BARE_BUILTINS` names {:?}, which `BARE_BUILTIN_CALLEES` \
         does not — a bare call to those is refused, so their purity is \
         the purity of a program that does not compile.",
        orphans
    );
}

/// The names a free `fn` may not take: every bare builtin the
/// compiler answers at a call site ahead of the program's own fns.
///
/// Mirrors `BUILTIN_CALL_FORMS` in `crates/hale-syntax/src/parser.rs`
/// (private to that crate) and is kept honest against
/// `BARE_BUILTIN_CALLEES` by `every_bare_builtin_name_is_classified`.
const CLAIMED_BUILTIN_NAMES: &[&str] = &[
    // GH #863: claimed by the parser (`sum` / `prod`) or by
    // codegen's math-builtin arm (`min` / `max`).
    "sum",
    "prod",
    "min",
    "max",
    // GH #880: the rest of codegen's unconditional `Expr::Call` arms
    // plus the statement-position printers and closure surface.
    "abs",
    "to_string",
    "Int",
    "Float",
    "len",
    "starts_with",
    "contains",
    "print",
    "println",
    "eprint",
    "eprintln",
    "check_closures",
    hale_syntax::parser::FMT_BUILTIN,
];

/// The bare-builtin names a free `fn` may still take, because no call
/// site claims them unconditionally.
///
/// The first eight are spec/tokens.md's built-in identifier table
/// minus the claimed names — framework spellings and two
/// conventionally reserved words that nothing dispatches on. The rest
/// are the `bounded[T; N]` intrinsics and the accumulator vocabulary,
/// whose codegen arms fire only when the argument IS a bounded
/// receiver or a closure assertion is being evaluated. `count` is the
/// load-bearing one: `dna/tests/books_slice_test.hl` declares a free
/// `fn count(app, kind, entity, needle)` and calls it.
///
/// GH #892: "may take the name" has to mean at the bounded receiver
/// too, or the six intrinsic names are only half free — see
/// [`BOUNDED_INTRINSIC_NAMES`] and the third column of the probe.
const UNCLAIMED_BUILTIN_NAMES: &[&str] = &[
    "B",
    "c",
    "sigma",
    "phi",
    "k_max",
    "span_max",
    "length",
    "empty",
    "count",
    "mean",
    "clear",
    "truncate",
    "push",
    "at",
    "set",
];

/// The `bounded[T; N]` intrinsic names, each with the arity its
/// intrinsic takes — the third column of the probe (GH #892).
///
/// These are the names whose codegen arms are GUARDED on the argument
/// type rather than unconditional, so #880's two columns (declared,
/// and declared-under-a-free-name) both passed for them while the one
/// argument shape the guard admits still hijacked the declaration.
/// Every name here must also be in [`UNCLAIMED_BUILTIN_NAMES`] —
/// `the_bounded_intrinsics_are_probed_as_unclaimed` holds that.
const BOUNDED_INTRINSIC_NAMES: &[(&str, usize)] = &[
    ("count", 1),
    ("clear", 1),
    ("truncate", 2),
    ("push", 2),
    ("at", 2),
    ("set", 3),
];

/// The sentinel the probe's user body returns. A hijacked call
/// returns the BUILTIN's answer instead, so its absence from stdout
/// is the hijack.
const PROBE_SENTINEL: i64 = 8801;

/// `fn NAME(params) -> Int { return 8801; }` plus a `main` that
/// prints the call.
///
/// Built from a template rather than by renaming the claimed spelling
/// out of a finished program: `src.replace("println(", …)` would also
/// rewrite `main`'s own printer and quietly turn the control into a
/// different test.
fn builtin_probe_source(decl: &str, arity: usize) -> String {
    let (params, args) = if arity == 1 {
        ("a: Int", "1")
    } else {
        ("a: Int, b: Int", "1, 2")
    };
    format!(
        "fn {d}({params}) -> Int {{\n    return {s};\n}}\n\n\
         fn main() {{\n    println(\"v=\", {d}({args}));\n}}\n",
        d = decl,
        params = params,
        args = args,
        s = PROBE_SENTINEL,
    )
}

/// The same probe, called with a BOUNDED RECEIVER — the one argument
/// shape the guarded arms claim (GH #892).
///
/// The declaration takes the receiver's own `bounded[Int; 4]` as its
/// first parameter, which is exactly the shape the shadow rule
/// recognizes, and the remaining parameters fill out the intrinsic's
/// arity so the declaration is the natural spelling a reader would
/// reach for.
fn bounded_probe_source(decl: &str, arity: usize) -> String {
    let (params, args) = match arity {
        1 => ("", ""),
        2 => (", i: Int", ", 0"),
        _ => (", i: Int, x: Int", ", 0, 9"),
    };
    format!(
        "type ProbeBuf {{ vals: bounded[Int; 4]; }}\n\n\
         fn {d}(xs: bounded[Int; 4]{params}) -> Int {{\n    \
         return {s};\n}}\n\n\
         fn main() {{\n    let b = ProbeBuf {{ }};\n    \
         println(\"v=\", {d}(b.vals{args}));\n}}\n",
        d = decl,
        params = params,
        args = args,
        s = PROBE_SENTINEL,
    )
}

/// Build and run `src`, returning its stdout — or the reason it never
/// produced any.
fn build_and_run_probe(src: &str, tag: &str) -> Result<String, String> {
    let program = hale_syntax::parse_source(src).map_err(|ds| {
        let msgs: Vec<&str> = ds.iter().map(|d| d.message.as_str()).collect();
        format!("does not parse: {}", msgs.join("; "))
    })?;
    let errs: Vec<String> = hale_types::check_program(&program)
        .iter()
        .filter(|d| d.is_error())
        .map(|d| d.message.clone())
        .collect();
    if !errs.is_empty() {
        return Err(format!("`hale check` refuses it: {}", errs.join("; ")));
    }
    let bin = harness::unique_bin(&format!("hale_builtin_probe_{}", tag));
    build_executable(&program, &bin)
        .map_err(|e| format!("`hale build` refuses it: {:?}", e))?;
    let out = std::process::Command::new(&bin)
        .output()
        .map_err(|e| format!("could not run the built binary: {}", e));
    let _ = std::fs::remove_file(&bin);
    let out = out?;
    if !out.status.success() {
        return Err(format!("the built binary exited {}", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// GH #863 / GH #880 — a name the compiler claims at a bare call site
/// cannot also be DECLARED, and a name it does not claim must run the
/// declaration.
///
/// The same divergence this file exists for, entered from the
/// declaration side. A free `fn sum(a: Int) -> Int { … }` passed
/// `hale check` and was refused by `hale build` with `unsupported in
/// codegen v0: 'sum(...)' outside a closure assertion`, unlocated —
/// the parser gives `sum(` its own production, so the user's fn was
/// never the callee. A two-arg `fn min(a: Int, b: Int)` was worse
/// than a divergence: it BUILT, and ran codegen's math builtin
/// instead of the body, silently. GH #880 probed the whole
/// bare-builtin table the same way and found four more of the silent
/// kind (`abs`, `to_string`, `Int`, `Float`) and six of the late kind
/// (`len`, `starts_with`, `contains`, `print`, `eprintln`, `__fmt`).
///
/// Neither corpus sweep above can see the silent kind, because those
/// programs BUILD. That is what the third column here is for: the
/// probe's body returns a sentinel and the built binary is RUN, so a
/// hijack shows up as the sentinel missing from stdout rather than as
/// a green build.
///
/// Three assertions per name, at one and two arguments:
///
///   * a CLAIMED name is refused at parse, by the declaration rule —
///     so `hale check` and `hale build` refuse the same program with
///     the same located sentence;
///   * its control, the identical program under `NAME_of`, checks,
///     builds and prints the sentinel, so the refusal is about the
///     name and not the shape;
///   * an UNCLAIMED name checks, builds and prints the sentinel —
///     the rule must not widen onto a name that works today.
///
/// ## The fourth, on a bounded receiver (GH #892)
///
/// The three above call the declaration with `Int` arguments, which
/// is precisely the argument shape the `bounded[T; N]` arms do NOT
/// claim — so all three passed for `count` / `clear` / `truncate` /
/// `push` / `at` / `set` while the argument shape those arms DO claim
/// still took the call. `fn count(xs: bounded[Int; 8]) -> Int` beside
/// `count(w.samples)` printed the live count, silently, in exactly
/// the way #880's `abs` and `min` did.
///
/// So each of the six is probed a fourth time, declared over the
/// receiver's own bounded type and called with it. Same sentinel,
/// same meaning: its absence is the hijack.
#[test]
fn a_fn_named_after_a_bare_builtin_agrees_and_runs_its_own_body() {
    let want = format!("v={}", PROBE_SENTINEL);
    let mut failures: Vec<String> = Vec::new();

    for (i, word) in CLAIMED_BUILTIN_NAMES.iter().enumerate() {
        for arity in [1usize, 2] {
            let src = builtin_probe_source(word, arity);
            match hale_syntax::parse_source(&src) {
                // Say what the unrefused declaration then DOES, so
                // the failure separates the two classes without a
                // second run: a silent hijack (it builds and prints
                // the builtin's answer), a late unlocated refusal,
                // or the codegen backstop catching it.
                Ok(_) => {
                    let then = match build_and_run_probe(
                        &src,
                        &format!("h{}_{}", i, arity),
                    ) {
                        Ok(stdout) if stdout.contains(&want) => {
                            "builds and runs the body".to_string()
                        }
                        Ok(stdout) => format!(
                            "BUILDS and prints {:?}, not {:?} — the \
                             builtin answered, silently",
                            stdout, want
                        ),
                        Err(why) => why,
                    };
                    failures.push(format!(
                        "  `fn {}` / {} arg(s) still parses, so \
                         `hale check` accepts it; it then {}",
                        word, arity, then
                    ));
                }
                Err(ds) => {
                    let msgs: Vec<&str> =
                        ds.iter().map(|d| d.message.as_str()).collect();
                    if !msgs.iter().any(|m| {
                        m.contains(
                            "is a built-in call form and cannot name a fn",
                        )
                    }) {
                        failures.push(format!(
                            "  `fn {}` / {} arg(s) is refused, but not by \
                             the declaration rule: {}",
                            word,
                            arity,
                            msgs.join("; ")
                        ));
                    }
                }
            }

            // The control: the same shape under a name nothing
            // claims must check, build AND run its own body.
            let control = format!("{}_of", word);
            let src = builtin_probe_source(&control, arity);
            match build_and_run_probe(&src, &format!("c{}_{}", i, arity)) {
                Ok(stdout) if stdout.contains(&want) => {}
                Ok(stdout) => failures.push(format!(
                    "  control `fn {}` / {} arg(s) built but printed {:?}, \
                     not {:?} — the body did not run",
                    control, arity, stdout, want
                )),
                Err(why) => failures.push(format!(
                    "  control `fn {}` / {} arg(s) {}",
                    control, arity, why
                )),
            }
        }
    }

    for (i, word) in UNCLAIMED_BUILTIN_NAMES.iter().enumerate() {
        for arity in [1usize, 2] {
            let src = builtin_probe_source(word, arity);
            match build_and_run_probe(&src, &format!("u{}_{}", i, arity)) {
                Ok(stdout) if stdout.contains(&want) => {}
                Ok(stdout) => failures.push(format!(
                    "  `fn {}` / {} arg(s) built but printed {:?}, not \
                     {:?} — a builtin answered the call instead of the \
                     declaration",
                    word, arity, stdout, want
                )),
                Err(why) => failures.push(format!(
                    "  `fn {}` / {} arg(s) {} — this name is supposed to \
                     stay available to a free fn",
                    word, arity, why
                )),
            }
        }
    }

    // GH #892: the same declaration, handed the bounded receiver the
    // guarded arms dispatch on.
    for (i, (word, arity)) in BOUNDED_INTRINSIC_NAMES.iter().enumerate() {
        let src = bounded_probe_source(word, *arity);
        // Guards the vacuous pass, as above: the generated program
        // must actually carry a bounded receiver.
        assert!(
            src.contains("bounded[Int; 4]") && src.contains("b.vals"),
            "the bounded probe for `{}` lost its receiver:\n{}",
            word,
            src
        );
        match build_and_run_probe(&src, &format!("b{}_{}", i, arity)) {
            Ok(stdout) if stdout.contains(&want) => {}
            Ok(stdout) => failures.push(format!(
                "  `fn {}({} arg(s), first one bounded)` built but \
                 printed {:?}, not {:?} — the bounded intrinsic \
                 answered the call instead of the declaration",
                word, arity, stdout, want
            )),
            Err(why) => failures.push(format!(
                "  `fn {}({} arg(s), first one bounded)` {} — a \
                 declaration over its own receiver type is supposed \
                 to answer the call",
                word, arity, why
            )),
        }
    }

    assert!(
        failures.is_empty(),
        "{} bare-builtin name(s) disagree between `hale check`, \
         `hale build` and what the built program actually runs \
         (GH #863, GH #880, GH #892):\n{}\n\n\
         The rule lives in `BUILTIN_CALL_FORMS` / \
         `reject_builtin_call_form_as_fn` in \
         `crates/hale-syntax/src/parser.rs`, with \
         `reject_builtin_over_user_fn` in \
         `crates/hale-codegen/src/codegen.rs` as the backstop; the \
         bounded-receiver column is \
         `user_fn_shadows_bounded_intrinsic`, held in \
         `crates/hale-codegen/src/form/bounded.rs` and in \
         `crates/hale-types/src/check.rs`.",
        failures.len(),
        failures.join("\n")
    );
}

/// The bounded column probes names the other columns call UNCLAIMED,
/// so the two lists must agree — otherwise a name moved into
/// `CLAIMED_BUILTIN_NAMES` would be asserted both refusable and
/// runnable, and the failure would read as a compiler bug.
#[test]
fn the_bounded_intrinsics_are_probed_as_unclaimed() {
    let unclaimed: BTreeSet<&str> =
        UNCLAIMED_BUILTIN_NAMES.iter().copied().collect();
    let stray: Vec<&str> = BOUNDED_INTRINSIC_NAMES
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| !unclaimed.contains(n))
        .collect();
    assert!(
        stray.is_empty(),
        "{:?} are probed on a bounded receiver but are not in \
         `UNCLAIMED_BUILTIN_NAMES` — a free `fn` of that name is \
         refused at the declaration, so the bounded probe cannot \
         build it (GH #892).",
        stray
    );
}

/// Every bare-name builtin the checker knows about is classified —
/// either a free `fn` may not take the name, or the probe above
/// proves it may.
///
/// Without this, adding a codegen arm (and its `BARE_BUILTIN_CALLEES`
/// row) would quietly reopen GH #880 for the new name: nothing else
/// notices a name that is in neither list.
#[test]
fn every_bare_builtin_name_is_classified() {
    let classified: BTreeSet<&str> = CLAIMED_BUILTIN_NAMES
        .iter()
        .chain(UNCLAIMED_BUILTIN_NAMES.iter())
        .copied()
        .collect();
    let unclassified: Vec<&str> = hale_types::check::BARE_BUILTIN_CALLEES
        .iter()
        .copied()
        .filter(|n| !classified.contains(n))
        .collect();
    assert!(
        unclassified.is_empty(),
        "{:?} are bare-name builtins that this file does not classify. \
         Decide for each whether a free `fn` of that name is claimed \
         (add it to `BUILTIN_CALL_FORMS` in \
         `crates/hale-syntax/src/parser.rs` and to \
         `CLAIMED_BUILTIN_NAMES` here) or stays free (add it to \
         `UNCLAIMED_BUILTIN_NAMES`, where the probe proves it runs \
         its own body) — GH #880.",
        unclassified
    );
}
