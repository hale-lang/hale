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

#[path = "../../hale-types/tests/support/entries.rs"]
mod entries;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use hale_syntax::ast::TopDecl;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

// The loaded-seed harness held to the bare-program harness over the same
// corpus (F.40 phase 4, T1), built in this binary.
#[path = "loaded_seed_agreement.rs"]
mod loaded_seed_agreement;

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

/// Whether the program's effective target, checked with no `--target`,
/// is wasm32: the effective-target row over it, as `hale check` derives
/// it.
fn declares_wasm32(program: &hale_syntax::ast::Program) -> bool {
    let mut programs: BTreeMap<String, &hale_syntax::ast::Program> = BTreeMap::new();
    programs.insert(String::new(), program);
    hale_types::capability::target_row(&hale_types::Bundle::new(programs)).is_wasm32()
}

/// Whether this box builds wasm32: wasm-ld, and a clang with the wasm32
/// backend (bare or `-18`), probed once.
fn wasm32_toolchain() -> bool {
    static HAS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HAS.get_or_init(|| {
        let runs = |n: &str| std::process::Command::new(n).arg("--version").output().is_ok_and(|o| o.status.success());
        if !(runs("wasm-ld") || runs("wasm-ld-18")) {
            return false;
        }
        let c = harness::unique_bin("hale_cb_wasm_probe").with_extension("c");
        let _ = std::fs::write(&c, "int x;\n");
        let ok = ["clang", "clang-18"].iter().any(|cc| {
            std::process::Command::new(cc)
                .args(["--target=wasm32", "-c"])
                .arg(&c)
                .arg("-o")
                .arg(c.with_extension("o"))
                .output()
                .is_ok_and(|o| o.status.success())
        });
        let _ = std::fs::remove_file(&c);
        let _ = std::fs::remove_file(c.with_extension("o"));
        ok
    })
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
    //
    // `check_program` holds the whole-program rules since GH #911 B1,
    // so this reads the corpus exactly as `hale check <dir>` reads a
    // seed — which is the comparison the ratchet is for.
    if entries::check_program(&program).iter().any(|d| d.is_error()) {
        return Verdict::Skipped("the checker rejects it");
    }
    // Cross-seed: the sibling seed is not in this fragment.
    // Matched on the source because an import is not a
    // `TopDecl` — it is consumed before the AST.
    if source.contains("import \"") {
        return Verdict::Skipped("imports a sibling seed");
    }
    // An `@ffi` declaration names a symbol the program's HOST side
    // provides (a C object the test compiles beside it, a wasm loader's
    // import); the sweep links none of those, so such a program cannot
    // build here whatever the checker says, and its link failure would
    // only shout in the log as if something broke. Matched on the source
    // like the import above: the annotation is on the declaration.
    if source.contains("@ffi(") {
        return Verdict::Skipped("declares an @ffi host import, whose host side is the test's");
    }
    // GH #829: no entry point is not a reason to skip; it is a
    // reason to add one.
    let program = with_synthetic_main(&program);
    // The program is built for the target it was checked for, its
    // effective target (T1(b)): a `target wasm { }` declaration selects
    // wasm32, so comparing its check with a native build compared
    // unlike things.
    let mut options = build_opts::options();
    if declares_wasm32(&program) {
        if !wasm32_toolchain() {
            return Verdict::Skipped("declares wasm32, and this box has no wasm32 clang or wasm-ld");
        }
        options.target = hale_codegen::CompileTarget::Wasm32;
    }
    let bin = harness::unique_bin(bin_tag);
    // `program` is the swept AST with a synthetic entry point added, which no source text spells: built from the AST
    match build_opts::build_program(&program, &bin, &[], &options) {
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
/// Locus-only programs, none with an entry point, all check-clean.
/// Before #829 the sweep skipped them on "no entry point", so a
/// check-accepts/build-refuses divergence in one — the exact thing
/// this file is the gate for — was invisible to it. Each must now be
/// BUILT by the sweep.
///
/// This test also pinned the refused half with a program the check
/// accepted and codegen refused (`std::process::run(…) or discard;`,
/// then `accept(c: Interface)`). Each such program is a divergence the
/// work is closing — the first builds since a statement `or discard`
/// drops a value (friction sweep 2026-10-12), the second is being moved
/// to the check — so a pinned example only lives until its fix lands.
/// The sweep's own ratchet (`check_build_divergences.txt`) holds the
/// divergences that remain; this test holds that an entry-point-less
/// program reaches the build at all. The second program is the
/// divergence the friction sweep closed in codegen: a method whose
/// `match` returns in every arm was refused as falling through.
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
    let match_returns = "locus Namer {\n\
                         fn name(n: Int) -> String {\n\
                         match n {\n\
                         0 -> { return \"zero\"; },\n\
                         _ -> { return \"many\"; },\n\
                         }\n\
                         }\n\
                         }\n";

    for (src, tag) in [(lowers, "hale_cb_ep_lowers"), (match_returns, "hale_cb_ep_match")] {
        let program = hale_syntax::parse_source(src).expect("parses");
        assert!(
            !has_entry_point(&program),
            "this test is about programs with NO entry point; this one \
             has one, so it proves nothing:\n{}",
            src
        );
        let errors: Vec<String> = entries::check_program(&program)
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
        assert_eq!(
            sweep_verdict(src, tag),
            Verdict::Built,
            "an entry-point-less program that lowers must be BUILT by the \
             sweep, not skipped:\n{}",
            src
        );
    }
}

/// GH #1076 (U1, review): a quantity literal in a default checked clean
/// and was refused by the build. A parameter's default is typed at the
/// invocation in a walk whose findings the check discards, which took
/// the not-yet boundary's error with them; a field's default was not
/// typed at all. The check refuses each now, at the literal, so the
/// build is never reached. U3: the literal's unit has no quantity here,
/// and the quantity rules' errors survive the discarded walk.
///
/// Plain string literals, for the reason given above.
#[test]
fn a_unit_value_in_a_default_is_refused_by_the_check_not_the_build() {
    let param = "unit cent;\n\
                 fn take(n: Int = 3cent) { println(n); }\n\
                 fn main() { take(); }\n";
    let field = "unit cent;\n\
                 type S { n: Int = 3cent; }\n\
                 fn main() { let s = S {}; println(s.n); }\n";
    for src in [param, field] {
        let program = hale_syntax::parse_source(src).expect("parses");
        let errors: Vec<hale_syntax::Diag> =
            entries::check_program(&program).into_iter().filter(|d| d.is_error()).collect();
        assert_eq!(errors.len(), 1, "one error from the check: {:?}\n{}", errors, src);
        assert_eq!(errors[0].span.slice(src), "3cent", "located at the literal:\n{}", src);
        assert!(errors[0].message.starts_with("`3cent`: the units of `cent` have no quantity"), "{}", errors[0].message);
        assert_eq!(
            sweep_verdict(src, "hale_cb_unit_default"),
            Verdict::Skipped("the checker rejects it"),
            "the check refuses it, so the build is never reached:\n{}",
            src
        );
    }
}

/// GH #1076 (U5): a quantity named `Bytes` checked clean and was refused
/// by the build. A type position reads `Bytes` as the buffer type before
/// any declaration, so `let n: Bytes = 4B;` checked against the buffer
/// while lowering found no row for `4B` ("quantity literal `4B` has no
/// required `expression_typing` row"). Law 12 refuses the declaration at
/// its name, so the build is never reached.
///
/// Plain string literals, for the reason given above.
#[test]
fn a_quantity_named_like_a_builtin_type_is_refused_by_the_check_not_the_build() {
    let src = "unit B;\n\
               type Bytes = quantity Int in B;\n\
               fn main() { let n: Bytes = 4B; println(f\"{len(n)}\"); }\n";
    let program = hale_syntax::parse_source(src).expect("parses");
    let errors: Vec<hale_syntax::Diag> =
        entries::check_program(&program).into_iter().filter(|d| d.is_error()).collect();
    assert_eq!(errors.len(), 1, "one error from the check: {:?}", errors);
    assert_eq!(errors[0].span.slice(src), "Bytes", "located at the name");
    assert_eq!(
        sweep_verdict(src, "hale_cb_unit_builtin_name"),
        Verdict::Skipped("the checker rejects it"),
        "the check refuses it, so the build is never reached"
    );
}

/// GH #1076 (U1, review): the not-yet boundary refused `Money(1)` as a
/// cast to the scalar type `Money` before the callee was resolved, so a
/// local of that name holding a fn was refused too. The local is the
/// callee: the program checks, builds and runs it, as it does when
/// `Money` is an alias. With no local, a quantity's cast of an `Int` is
/// refused by the check before the build (U3: a count becomes a quantity
/// by a unit); an identity's is a conversion (U2), which builds and
/// prints 1.
///
/// U2 (fix): lowering decided a conversion by the callee's name, so the
/// shadowing local of an identity's or a range's name, which the checker
/// resolves as the callee and records no conversion row for, was refused
/// by the build as a cast with no row. A call is a conversion when its
/// row says so; the local is the callee for every kind of type.
///
/// Plain string literals, for the reason given above.
#[test]
fn a_local_that_shadows_a_unit_type_is_the_callee() {
    let shadows = |ty: &str| {
        format!(
            "unit cent;\n\
             type Money = {};\n\
             fn id(n: Int) -> Int {{ return n; }}\n\
             fn main() {{ let Money = id; println(Money(1)); }}\n",
            ty
        )
    };
    for ty in ["quantity Int in cent", "Int", "distinct Int", "Int { range: 0..64; }"] {
        let src = shadows(ty);
        assert_eq!(build_and_run_probe(&src, "unit_shadow"), Ok("1\n".to_string()), "{}", src);
    }
    let cast = "unit cent;\n\
                type Money = quantity Int in cent;\n\
                fn main() { println(Money(1)); }\n";
    assert_eq!(
        sweep_verdict(cast, "hale_cb_unit_cast"),
        Verdict::Skipped("the checker rejects it"),
        "the check refuses the cast, so the build is never reached"
    );
    let identity = cast.replace("quantity Int in cent", "distinct Int");
    assert_eq!(build_and_run_probe(&identity, "unit_identity_cast"), Ok("1\n".to_string()), "{}", identity);
}

/// A fn value bound in scope shadows a fn of the same name in a call:
/// the call runs the binding, as the code reads, in the check and the
/// build alike (the desugar sequence calls the binding as a value). The
/// call ran the top-level fn: `apply(g, 2)` beside `fn f` printed 3, and
/// `{ let helper = zero; helper() }` printed 1. A parameter, a `let`, a
/// `for` binder, a `match` pattern's binding, and a generic fn of the
/// name; each checks, builds and runs the binding.
///
/// Plain string literals, for the reason given above.
#[test]
fn a_fn_value_in_scope_shadows_a_fn_in_a_call() {
    let rows: [(&str, &str); 5] = [
        (
            "fn f(n: Int) -> Int { return n + 1; }\n\
             fn g(n: Int) -> Int { return n * 10; }\n\
             fn apply(f: fn(Int) -> Int, x: Int) -> Int { return f(x); }\n\
             fn main() { println(apply(g, 2)); }\n",
            "20\n",
        ),
        (
            "fn helper() -> Int { return 1; }\n\
             fn zero() -> Int { return 0; }\n\
             fn main() { let helper = zero; println(helper()); }\n",
            "0\n",
        ),
        (
            "fn helper() -> Int { return 1; }\n\
             fn zero() -> Int { return 0; }\n\
             fn main() { for helper in [zero, zero] { println(helper()); } }\n",
            "0\n0\n",
        ),
        (
            "fn helper() -> Int { return 1; }\n\
             fn zero() -> Int { return 0; }\n\
             fn main() {\n\
                 let pair = (zero, 5);\n\
                 match pair {\n\
                     (helper, n) -> println(helper() + n),\n\
                     _ -> println(\"none\"),\n\
                 }\n\
             }\n",
            "5\n",
        ),
        (
            "fn helper<T>(x: T) -> T { return x; }\n\
             fn plus(x: Int) -> Int { return x + 100; }\n\
             fn main() { let helper = plus; println(helper(5)); }\n",
            "105\n",
        ),
    ];
    for (i, (src, wanted)) in rows.iter().enumerate() {
        assert_eq!(build_and_run_probe(src, &format!("shadowing_fn_value_{i}")), Ok(wanted.to_string()), "{src}");
    }
}

/// GH #1076 (U1, review 2): a struct field's default is evaluated in the
/// scope of the literal that leaves the field, so `Money(1)` in it calls
/// the constructing fn's local `Money` (a fn), not the scalar type the
/// declaration's scope would name. The check judges the cast there, and
/// the program checks, builds and prints 1; with no local, the check
/// refuses the cast before the build.
///
/// Plain string literals, for the reason given above.
#[test]
fn a_struct_default_cast_is_judged_in_the_literals_scope() {
    let shadows = "unit cent;\n\
                   type Money = quantity Int in cent;\n\
                   type S { n: Int = Money(1); }\n\
                   fn id(n: Int) -> Int { return n; }\n\
                   fn main() { let Money = id; let s = S {}; println(s.n); }\n";
    assert_eq!(build_and_run_probe(shadows, "unit_default_shadow"), Ok("1\n".to_string()), "{}", shadows);
    let cast = "unit cent;\n\
                type Money = quantity Int in cent;\n\
                type S { n: Int = Money(1); }\n\
                fn main() { let s = S {}; println(s.n); }\n";
    assert_eq!(
        sweep_verdict(cast, "hale_cb_unit_default_cast"),
        Verdict::Skipped("the checker rejects it"),
        "the check refuses the cast, so the build is never reached"
    );
}

/// GH #1076 (U3): a quantity in a default converts into its field's or
/// its parameter's type where the default is evaluated, as a value does
/// wherever it flows: `3USD` into a `Money` counted in cents is 300, and
/// a build that lowered the literal at its own unit would print 3.
///
/// Plain string literals, for the reason given above.
#[test]
fn a_quantity_default_converts_into_its_fields_and_parameters_type() {
    let decls = "unit cent;\n\
                 unit USD = 100 cent;\n\
                 type Money = quantity Int in cent;\n";
    let field = format!("{decls}type S {{ m: Money = 3USD; }}\nfn main() {{ let s = S {{}}; println(s.m); }}\n");
    let param = format!("{decls}fn take(m: Money = 3USD) -> Money {{ return m; }}\nfn main() {{ println(take()); }}\n");
    for (src, tag) in [(field, "unit_quantity_field_default"), (param, "unit_quantity_param_default")] {
        assert_eq!(build_and_run_probe(&src, tag), Ok("300cent\n".to_string()), "{}", src);
    }
}

/// GH #1076 (U1, review 3): an omitted struct default is typed where it
/// is evaluated, so the check follows the scopes it opens (a block binding
/// `Money` itself), the parameter defaults its calls leave and the field
/// defaults its own literals leave, each in the constructing fn's scope.
/// Each shadowing program checks, builds and prints 1. With nothing
/// shadowing the nested literal's cast, the check refuses it once, at
/// `Inner`'s default, before the build (`Outer {}` evaluates `Inner {}`,
/// whose omitted `n` the build would otherwise reach).
///
/// Plain string literals, for the reason given above.
#[test]
fn an_omitted_struct_default_is_checked_where_it_is_evaluated() {
    let block = "unit cent;\n\
                 type Money = quantity Int in cent;\n\
                 fn id(n: Int) -> Int { return n; }\n\
                 type S { n: Int = { let Money = id; Money(1) }; }\n\
                 fn main() { let s = S {}; println(s.n); }\n";
    let through_fn = "unit cent;\n\
                      type Money = quantity Int in cent;\n\
                      fn id(n: Int) -> Int { return n; }\n\
                      fn take(n: Int = Money(1)) -> Int { return n; }\n\
                      type S { n: Int = take(); }\n\
                      fn main() { let Money = id; let s = S {}; println(s.n); }\n";
    let nested = "unit cent;\n\
                  type Money = quantity Int in cent;\n\
                  fn id(n: Int) -> Int { return n; }\n\
                  type Inner { n: Int = Money(1); }\n\
                  type Outer { inner: Inner = Inner {}; }\n\
                  fn main() { let Money = id; let o = Outer {}; println(o.inner.n); }\n";
    for (src, tag) in [(block, "unit_block_default"), (through_fn, "unit_fn_default"), (nested, "unit_nested_default")] {
        assert_eq!(build_and_run_probe(src, tag), Ok("1\n".to_string()), "{}", src);
    }
    let cast = "unit cent;\n\
                type Money = quantity Int in cent;\n\
                type Inner { n: Int = Money(1); }\n\
                type Outer { inner: Inner = Inner {}; }\n\
                fn main() { let o = Outer {}; println(o.inner.n); }\n";
    let program = hale_syntax::parse_source(cast).expect("parses");
    let errors: Vec<hale_syntax::Diag> =
        entries::check_program(&program).into_iter().filter(|d| d.is_error()).collect();
    assert_eq!(errors.len(), 1, "one error from the check: {:?}", errors);
    assert_eq!(errors[0].span.start.as_usize(), cast.find("Money(1)").expect("the cast"), "at `Inner`'s default");
    assert!(errors[0].message.starts_with("`Money(…)` of an `Int`: "), "{}", errors[0].message);
    assert_eq!(
        sweep_verdict(cast, "hale_cb_unit_nested_default_cast"),
        Verdict::Skipped("the checker rejects it"),
        "the check refuses the cast, so the build is never reached"
    );
}

/// U2 (fix): an identity's or a range's cast in an omitted struct
/// default checked clean and the build refused it as a cast with no row.
/// Two causes: the walk that types an omitted default at the literal ran
/// only when the program declared a quantity or a point, and it put the
/// typed-body record back as it found it, conversion rows included. The
/// default is typed at the literal whenever the program declares any
/// scalar type, and its casts' rows are kept in the constructing
/// declaration's body, so each program below checks, builds and prints
/// 1: an identity's default, a range's narrowing under its `or`, the
/// nested `Outer {}` → `Inner {}`, two literals leaving the field, and
/// the identity beside a declared quantity (the walk ran there before
/// the fix; only its rows were lost).
///
/// Plain string literals, for the reason given above.
#[test]
fn a_scalars_cast_in_an_omitted_struct_default_is_lowered_from_its_row() {
    let identity = "type Money = distinct Int;\n\
                    type S { n: Money = Money(1); }\n\
                    fn main() { let s = S {}; println(s.n); }\n";
    let range = "type Money = Int { range: 0..64; }\n\
                 type S { n: Money = Money(1) or clamp; }\n\
                 fn main() { let s = S {}; println(s.n); }\n";
    let nested = "type Money = distinct Int;\n\
                  type Inner { n: Money = Money(1); }\n\
                  type Outer { inner: Inner = Inner {}; }\n\
                  fn main() { let o = Outer {}; println(o.inner.n); }\n";
    let two_literals = "type Money = distinct Int;\n\
                        type S { n: Money = Money(1); }\n\
                        fn one() -> Money { let s = S {}; return s.n; }\n\
                        fn main() { let s = S {}; println(Int(s.n) * Int(one())); }\n";
    let beside_a_quantity = "unit cent;\n\
                             type Cents = quantity Int in cent;\n\
                             type Money = distinct Int;\n\
                             type S { n: Money = Money(1); }\n\
                             fn main() { let s = S {}; println(s.n); }\n";
    for (src, tag) in [
        (identity, "scalar_default_identity"),
        (range, "scalar_default_range"),
        (nested, "scalar_default_nested"),
        (two_literals, "scalar_default_two"),
        (beside_a_quantity, "scalar_default_quantity"),
    ] {
        assert_eq!(build_and_run_probe(src, tag), Ok("1\n".to_string()), "{}", src);
    }
}

/// U2 (review 2): one default evaluated in two scopes has two answers.
/// A local that shadows the type in one scope makes the default's
/// `ItemId(1)` a call of the local there, and the cast elsewhere: each
/// evaluation's row is keyed by the evaluation (the literal that leaves
/// the field, the call that leaves the parameter), and lowering reads
/// the one it lowers. Before, the first evaluation's row was every
/// evaluation's, and each program below printed `1` for the shadowed
/// one: the call of `bump` was dropped. Both orders of the two
/// constructions, the shadowed one alone, and a parameter's default.
///
/// Plain string literals, for the reason given above.
#[test]
fn a_default_evaluated_in_two_scopes_lowers_each_scopes_meaning() {
    let shadowed_second = "type ItemId = distinct Int;\n\
                           type S { n: ItemId = ItemId(1); }\n\
                           fn bump(n: Int) -> ItemId { return ItemId(n + 10); }\n\
                           fn main() {\n\
                           let a = S {};\n\
                           { let ItemId = bump; let b = S {}; println(Int(b.n)); }\n\
                           println(Int(a.n));\n\
                           }\n";
    let shadowed_first = "type ItemId = distinct Int;\n\
                          type S { n: ItemId = ItemId(1); }\n\
                          fn bump(n: Int) -> ItemId { return ItemId(n + 10); }\n\
                          fn main() {\n\
                          { let ItemId = bump; let b = S {}; println(Int(b.n)); }\n\
                          let a = S {};\n\
                          println(Int(a.n));\n\
                          }\n";
    let shadowed_alone = "type ItemId = distinct Int;\n\
                          type S { n: ItemId = ItemId(1); }\n\
                          fn bump(n: Int) -> ItemId { return ItemId(n + 10); }\n\
                          fn main() { let ItemId = bump; let b = S {}; println(Int(b.n)); }\n";
    let parameter = "type ItemId = distinct Int;\n\
                     fn bump(n: Int) -> ItemId { return ItemId(n + 10); }\n\
                     fn take(n: ItemId = ItemId(1)) -> Int { return Int(n); }\n\
                     fn main() {\n\
                     let a = take();\n\
                     { let ItemId = bump; println(take()); }\n\
                     println(a);\n\
                     }\n";
    for (src, tag, printed) in [
        (shadowed_second, "default_scopes_second", "11\n1\n"),
        (shadowed_first, "default_scopes_first", "11\n1\n"),
        (shadowed_alone, "default_scopes_alone", "11\n"),
        (parameter, "default_scopes_param", "11\n1\n"),
    ] {
        assert_eq!(build_and_run_probe(src, tag), Ok(printed.to_string()), "{}", src);
    }
}

/// U2 (review 3): the nested form. One `Outer {}` evaluates `Inner`'s
/// default twice, once in each field's default, and `b`'s scope shadows
/// the type. Keyed by the outermost evaluation alone, the two shared one
/// key, so `a`'s row lowered `b`'s call of `bump` as the cast and each
/// shadowed program below printed `1` where it says `11`. The key is the
/// whole evaluation path (`Outer {}`, then the `Inner {}` or the call in
/// the field's default), so each nested evaluation reads its own. The
/// reviewer's program, its fields reversed, a parameter's default left by
/// a call in a field's default, the shadowed field alone, and two nested
/// evaluations in one scope: those have two paths and two rows, equal,
/// and print `1` twice.
///
/// Plain string literals, for the reason given above.
#[test]
fn a_nested_default_evaluated_in_two_scopes_lowers_each_scopes_meaning() {
    let reviewed = "type ItemId = distinct Int;\n\
                    fn bump(n: Int) -> ItemId { return ItemId(n + 10); }\n\
                    type Inner { n: ItemId = ItemId(1); }\n\
                    type Outer {\n\
                    a: Inner = Inner {};\n\
                    b: Inner = { let ItemId = bump; Inner {} };\n\
                    }\n\
                    fn main() { let o = Outer {}; println(Int(o.a.n)); println(Int(o.b.n)); }\n";
    let reversed = "type ItemId = distinct Int;\n\
                    fn bump(n: Int) -> ItemId { return ItemId(n + 10); }\n\
                    type Inner { n: ItemId = ItemId(1); }\n\
                    type Outer {\n\
                    a: Inner = { let ItemId = bump; Inner {} };\n\
                    b: Inner = Inner {};\n\
                    }\n\
                    fn main() { let o = Outer {}; println(Int(o.a.n)); println(Int(o.b.n)); }\n";
    let parameter = "type ItemId = distinct Int;\n\
                     fn bump(n: Int) -> ItemId { return ItemId(n + 10); }\n\
                     fn take(n: ItemId = ItemId(1)) -> ItemId { return n; }\n\
                     type Outer {\n\
                     a: ItemId = take();\n\
                     b: ItemId = { let ItemId = bump; take() };\n\
                     }\n\
                     fn main() { let o = Outer {}; println(Int(o.a)); println(Int(o.b)); }\n";
    let shadowed_alone = "type ItemId = distinct Int;\n\
                          fn bump(n: Int) -> ItemId { return ItemId(n + 10); }\n\
                          type Inner { n: ItemId = ItemId(1); }\n\
                          type Outer { b: Inner = { let ItemId = bump; Inner {} }; }\n\
                          fn main() { let o = Outer {}; println(Int(o.b.n)); }\n";
    let same_scope = "type ItemId = distinct Int;\n\
                      type Inner { n: ItemId = ItemId(1); }\n\
                      type Outer { a: Inner = Inner {}; b: Inner = Inner {}; }\n\
                      fn main() { let o = Outer {}; println(Int(o.a.n)); println(Int(o.b.n)); }\n";
    for (src, tag, printed) in [
        (reviewed, "nested_scopes_reviewed", "1\n11\n"),
        (reversed, "nested_scopes_reversed", "11\n1\n"),
        (parameter, "nested_scopes_param", "1\n11\n"),
        (shadowed_alone, "nested_scopes_alone", "11\n"),
        (same_scope, "nested_scopes_same", "1\n1\n"),
    ] {
        assert_eq!(build_and_run_probe(src, tag), Ok(printed.to_string()), "{}", src);
    }
}

/// GH #1076 (U3 polish A): every conversion in a default is the
/// evaluation's, not only a cast's. Where a local shadows `Bucket`, the
/// default's `Bucket(2000msec)` is a call of `fake`, and its literal flows
/// into `fake`'s `Span` (2000); elsewhere it is the cast, and its literal
/// flows into `sec` (2). Keyed by its span alone, the literal had one row,
/// the cast's count, and the shadowed evaluation printed `5sec`. The
/// same for a value converted where it stands: `milli()` flows into
/// `fake`'s `Span` in `usec` (widened by 1,000) in the shadowed scope
/// only, and the cast's evaluation, reading that row too, converted it
/// twice and printed `2000sec`. The literal in both orders of the two
/// evaluations.
///
/// Plain string literals, for the reason given above.
#[test]
fn a_literal_and_a_value_in_a_default_convert_per_evaluation() {
    let literal = |shadowed_first: bool| {
        let (first, second) = if shadowed_first {
            ("{ let Bucket = fake; let m = L {}; println(m.b); }\n", "let l = L {};\nprintln(l.b);\n")
        } else {
            ("let l = L {};\n", "{ let Bucket = fake; let m = L {}; println(m.b); }\nprintln(l.b);\n")
        };
        format!(
            "unit msec;\n\
             unit sec = 1_000 msec;\n\
             type Span = quantity Int in msec;\n\
             type Bucket = Span in sec {{ round: floor; }}\n\
             fn fake(d: Span) -> Bucket {{ return Bucket(d + 5000msec); }}\n\
             type L {{ b: Bucket = Bucket(2000msec); }}\n\
             fn main() {{\n{first}{second}}}\n"
        )
    };
    let value = "unit usec;\n\
                 unit msec = 1_000 usec;\n\
                 unit sec = 1_000 msec;\n\
                 type Span = quantity Int in usec;\n\
                 type Milli = Span in msec;\n\
                 type Bucket = Span in sec { round: floor; }\n\
                 fn milli() -> Milli { return 2000msec; }\n\
                 fn fake(d: Span) -> Bucket { return Bucket(d + 5sec); }\n\
                 type L { b: Bucket = Bucket(milli()); }\n\
                 fn main() {\n\
                 let l = L {};\n\
                 { let Bucket = fake; let m = L {}; println(m.b); }\n\
                 println(l.b);\n\
                 }\n";
    for (src, tag) in [
        (literal(false), "default_literal_second"),
        (literal(true), "default_literal_first"),
        (value.to_string(), "default_value"),
    ] {
        assert_eq!(build_and_run_probe(&src, tag), Ok("7sec\n2sec\n".to_string()), "{}", src);
    }
}

/// GH #1076 (U3, review 1): a default's value flows into its declared type
/// as a binding's initializer does, whatever the type's shape. The default
/// walks converted a default only when the field's or parameter's type, or
/// the default, was itself a quantity, so `[3USD, 2USD]` into `[Money; 2]`
/// kept its literals' counts in `USD` and each program below printed
/// `3cent` and `2cent`. Each element now has its row on the evaluation's
/// path: the reviewer's field and parameter defaults, and the field's
/// default reached through a nested one.
///
/// Plain string literals, for the reason given above.
#[test]
fn a_quantity_in_an_array_default_converts_into_the_element_type() {
    let decls = "unit cent;\n\
                 unit USD = 100 cent;\n\
                 type Money = quantity Int in cent;\n";
    let field = format!(
        "{decls}type S {{ p: [Money; 2] = [3USD, 2USD]; }}\n\
         fn main() {{ let s = S {{}}; println(s.p[0]); println(s.p[1]); }}\n"
    );
    let param = format!(
        "{decls}fn take(p: [Money; 2] = [3USD, 2USD]) -> [Money; 2] {{ return p; }}\n\
         fn main() {{ let p = take(); println(p[0]); println(p[1]); }}\n"
    );
    let nested = format!(
        "{decls}type S {{ p: [Money; 2] = [3USD, 2USD]; }}\n\
         type Outer {{ s: S = S {{}}; }}\n\
         fn main() {{ let o = Outer {{}}; println(o.s.p[0]); println(o.s.p[1]); }}\n"
    );
    for (src, tag) in [
        (field, "unit_array_field_default"),
        (param, "unit_array_param_default"),
        (nested, "unit_array_nested_default"),
    ] {
        assert_eq!(build_and_run_probe(&src, tag), Ok("300cent\n200cent\n".to_string()), "{}", src);
    }
}

/// GH #1076 (U3, review 1): a constant's initializer is its own
/// evaluation. It is lowered again at each use, and a use inside a default
/// looked its rows up on the default's evaluation path, where the checker,
/// which types the constant once in its own body with no path, recorded
/// none: `Money(3USD)` was lowered as a call and the build refused each
/// program below ("call to `Money`: no free fn …") after the check
/// accepted it. A struct field's default, a parameter's, and a nested
/// struct default reading the constant each print `300cent`.
///
/// Plain string literals, for the reason given above.
#[test]
fn a_constant_read_in_a_default_is_lowered_from_its_own_rows() {
    let decls = "unit cent;\n\
                 unit USD = 100 cent;\n\
                 type Money = quantity Int in cent;\n\
                 const AMOUNT: Money = Money(3USD);\n";
    let field = format!("{decls}type S {{ m: Money = AMOUNT; }}\nfn main() {{ let s = S {{}}; println(s.m); }}\n");
    let param = format!("{decls}fn take(m: Money = AMOUNT) -> Money {{ return m; }}\nfn main() {{ println(take()); }}\n");
    let nested = format!(
        "{decls}type Inner {{ m: Money = AMOUNT; }}\n\
         type Outer {{ i: Inner = Inner {{}}; }}\n\
         fn main() {{ let o = Outer {{}}; println(o.i.m); }}\n"
    );
    for (src, tag) in [
        (field, "unit_const_field_default"),
        (param, "unit_const_param_default"),
        (nested, "unit_const_nested_default"),
    ] {
        assert_eq!(build_and_run_probe(&src, tag), Ok("300cent\n".to_string()), "{}", src);
    }
}

/// GH #1076 (U3, review 2): each element of an array literal converts from
/// its own type. The literal was typed by its first element, so in
/// `[3cent, 2USD]` into `[Money; 2]` the whole array was already a `Money`
/// array and `2USD` stored `2`; in `[3USD, 2cent]` every element converted
/// as a `USD` and `2cent` stored `200`. Each order as a field's default, a
/// parameter's and an annotated binding; a constant beside a literal in
/// each of the three; and an array nothing types, whose elements meet at
/// the finer denomination.
///
/// Plain string literals, for the reason given above.
#[test]
fn an_array_literals_elements_each_convert_from_their_own_type() {
    let decls = "unit cent;\n\
                 unit USD = 100 cent;\n\
                 type Money = quantity Int in cent;\n";
    for (array, want, order) in [("[3cent, 2USD]", "3cent\n200cent\n", "cent_usd"), ("[3USD, 2cent]", "300cent\n2cent\n", "usd_cent")] {
        let field =
            format!("{decls}type S {{ a: [Money; 2] = {array}; }}\nfn main() {{ let s = S {{}}; println(s.a[0]); println(s.a[1]); }}\n");
        let param = format!(
            "{decls}fn take(a: [Money; 2] = {array}) -> [Money; 2] {{ return a; }}\n\
             fn main() {{ let a = take(); println(a[0]); println(a[1]); }}\n"
        );
        let binding = format!("{decls}fn main() {{ let a: [Money; 2] = {array}; println(a[0]); println(a[1]); }}\n");
        for (src, shape) in [(field, "field"), (param, "param"), (binding, "binding")] {
            let tag = format!("unit_mixed_array_{shape}_{order}");
            assert_eq!(build_and_run_probe(&src, &tag), Ok(want.to_string()), "{}", src);
        }
    }
    let constant = format!(
        "{decls}const AMOUNT: Money = Money(3USD);\n\
         type S {{ a: [Money; 2] = [AMOUNT, 2USD]; }}\n\
         fn take(a: [Money; 2] = [AMOUNT, 2USD]) -> [Money; 2] {{ return a; }}\n\
         fn main() {{ let s = S {{}}; println(s.a[0]); println(s.a[1]); let t = take(); println(t[0]); println(t[1]); \
         let b: [Money; 2] = [AMOUNT, 2USD]; println(b[0]); println(b[1]); }}\n"
    );
    assert_eq!(
        build_and_run_probe(&constant, "unit_mixed_array_constant"),
        Ok("300cent\n200cent\n300cent\n200cent\n300cent\n200cent\n".to_string()),
        "{}",
        constant
    );
    let meet = "unit msec;\nunit sec = 1_000 msec;\ntype Span = quantity Int in msec;\n\
                fn main() { let a = [1sec, 1_500msec]; println(a[0]); println(a[1]); }\n";
    assert_eq!(build_and_run_probe(meet, "unit_mixed_array_meet"), Ok("1000msec\n1500msec\n".to_string()), "{}", meet);
}

/// `Decimal<S>` (review of #1532): a composite literal holding a Decimal
/// flows element by element into its place. Two divergences: the check
/// accepted `[1.0d]` where `[Decimal; 2]` is expected (adopting the
/// elements' scale skipped the length, and the build read past a
/// one-element allocation), and refused nothing but built nothing for
/// `[a, 2.34d]` into `[Decimal<2>; 2]` (the literal kept scale 9 because
/// its neighbour was typed, and the build refused the element type).
/// The first is now the check's refusal; the second, and the nested,
/// tuple, repeated, bounded, generic-field, `if` and `match` shapes,
/// check, build and print.
///
/// Plain string literals, for the reason given above.
#[test]
fn a_decimal_composite_literal_flows_element_by_element() {
    let short = "fn main() { let values: [Decimal; 2] = [1.0d]; println(values[1]); }\n";
    let program = hale_syntax::parse_source(short).expect("parses");
    let errors: Vec<hale_syntax::Diag> =
        entries::check_program(&program).into_iter().filter(|d| d.is_error()).collect();
    assert_eq!(errors.len(), 1, "one error from the check: {:?}", errors);
    assert_eq!(errors[0].span.slice(short), "[1.0d]");
    assert_eq!(sweep_verdict(short, "hale_cb_decimal_short_array"), Verdict::Skipped("the checker rejects it"));
    let mixed = "type Box<T> { value: T; }\n\
                 type W { s: bounded[Decimal<2>; 4]; }\n\
                 fn main() {\n\
                     let a: Decimal<2> = 1.23d;\n\
                     let xs: [Decimal<2>; 2] = [a, 2.34d];\n\
                     let ys: [Decimal<2>; 2] = [2.35d, a];\n\
                     let n: [[Decimal<2>; 2]; 2] = [[a, 0.01d], [0.02d, 0.03d]];\n\
                     let t: (Decimal<18>, Int, Decimal<2>) = (0.000000000000000001d, 3, a);\n\
                     let w = W { };\n\
                     push(w.s, 0.04d) or raise;\n\
                     let b: Box<Decimal<18>> = Box { value: 0.000000000000000005d };\n\
                     let r: [Decimal<2>; 3] = [0.07d; 3];\n\
                     let k = 1;\n\
                     let y = if k > 0 { 0.06d } else { a };\n\
                     let m = match k { 0 -> a, _ -> 0.08d, };\n\
                     println(xs[1]); println(ys[0]); println(n[0][1]); println(n[1][0]);\n\
                     println(t.0); println(t.2); println(f\"{w.s}\"); println(b.value);\n\
                     println(r[2]); println(y); println(m);\n\
                 }\n";
    assert_eq!(
        build_and_run_probe(mixed, "decimal_mixed_composites"),
        Ok("2.34\n2.35\n0.01\n0.02\n0.000000000000000001\n1.23\n[0.04]\n0.000000000000000005\n0.07\n0.06\n0.08\n".to_string()),
        "{}",
        mixed
    );
}

/// GH #1076 (U4, review 1): a literal is its quantity at the quantity's
/// denomination when its count there is whole, whatever its unit's factor
/// alone is. The rule tested the factor, so `1000mg` of a `Mass` counted
/// in grams stayed a `Mass in mg`, and `1000tick` of a unit a thousandth
/// of a nanosecond a `Duration in tick`: both arms below were refused as
/// two types by the check and the build alike. Each checks, builds and
/// prints its one whole count.
///
/// Plain string literals, for the reason given above.
#[test]
fn a_whole_count_literal_of_a_finer_unit_is_its_quantity() {
    let mass = "unit g;\n\
                unit mg = 1/1000 g;\n\
                type Mass = quantity Int in g;\n\
                fn main() { let x = if true { 1000mg } else { 1g }; println(x); println(5mg); }\n";
    assert_eq!(build_and_run_probe(mass, "unit_whole_count_mass"), Ok("1g\n5mg\n".to_string()), "{}", mass);
    let tick = "unit tick = 1/1000 ns;\n\
                fn main() { let x = if true { 1000tick } else { 1ns }; println(x); }\n";
    assert_eq!(build_and_run_probe(tick, "unit_whole_count_tick"), Ok("1ns\n".to_string()), "{}", tick);
}

/// GH #1076 (U4, review 2): a quantity's negation is the quantity, and
/// lowering negated only an `Int`, a `Float` and a `Decimal`. The rule
/// above made `-2000tick` the stdlib's `Duration`, so it checked and the
/// build refused it ("unop Neg on Duration"), where before it printed
/// `-2000tick`. A `Duration`'s negation is its count's, still a
/// `Duration`, a literal's and a value's alike; a program's own quantity,
/// an `Int` underneath, negated before and still does.
///
/// Plain string literals, for the reason given above.
#[test]
fn a_negated_quantity_checks_builds_and_keeps_its_unit() {
    let tick = "unit tick = 1/1000 ns;\n\
                fn main() { println(-2000tick); println(-5tick); println(-(1s)); let d = 3s; println(-d); }\n";
    assert_eq!(
        build_and_run_probe(tick, "unit_negated_tick"),
        Ok("-2ns\n-5tick\n-1000000000ns\n-3000000000ns\n".to_string()),
        "{}",
        tick
    );
    let mass = "unit g;\n\
                unit mg = 1/1000 g;\n\
                type Mass = quantity Int in g;\n\
                fn main() { println(-2000mg); let m = 2000mg; println(-m); let n = 5mg; println(-n); }\n";
    assert_eq!(build_and_run_probe(mass, "unit_negated_mass"), Ok("-2g\n-2g\n-5mg\n".to_string()), "{}", mass);
}

/// The check with the whole-program rules OFF — what a caller holding
/// a fragment gets, and what `check_program` was before GH #911 B1.
///
/// One caller below needs it: a sweep whose subject is "the programs
/// the strict rule refuses" cannot use the strict rule to decide which
/// programs to consider.
fn permissive_check(
    program: &hale_syntax::ast::Program,
) -> Vec<hale_syntax::Diag> {
    let mut programs: BTreeMap<String, &hale_syntax::ast::Program> =
        BTreeMap::new();
    programs.insert(String::new(), program);
    entries::check_bundle_opts(&hale_types::Bundle::new(programs), false)
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
        //
        // PERMISSIVE on purpose, and load-bearing: this test asks
        // "which programs does the strict rule refuse, and does the
        // build accept any of them", so a prefilter that already holds
        // the strict rule would skip exactly the programs the test
        // exists to compile and pass forever on an empty set. That is
        // why it cannot be `check_program`, which has held the
        // whole-program rules since GH #911 B1.
        if permissive_check(&program).iter().any(|d| d.is_error()) {
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
            entries::check_bundle_opts_scoped(&bundle, false, true, true)
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
        if build_opts::build_source(&p.source, &bin, &build_opts::options()).is_ok() {
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

    // And a walk that refuses NOTHING builds nothing, so it proves
    // nothing. This is the vacuity the prefilter above can cause: hold
    // the strict rule there and every program the rule would refuse is
    // skipped before it is reached, leaving an empty set that passes
    // forever.
    assert!(
        refused > 0,
        "{} programs swept and the strict rule refused none of them — \
         the prefilter is holding the rule this test exists to isolate, \
         so nothing was built and nothing was proven",
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
        if let Err(ds) = hale_syntax::parse_source(src) {
            let msgs: Vec<&str> =
                ds.iter().map(|d| d.message.as_str()).collect();
            failures.push(format!(
                "  `{}` does not parse: {}",
                name,
                msgs.join("; ")
            ));
            continue;
        }
        let bin = harness::unique_bin(&format!("hale_bbc_{}", i));
        match build_opts::build_source(src, &bin, &build_opts::options()) {
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
    let errs: Vec<String> = entries::check_program(&program)
        .iter()
        .filter(|d| d.is_error())
        .map(|d| d.message.clone())
        .collect();
    if !errs.is_empty() {
        return Err(format!("`hale check` refuses it: {}", errs.join("; ")));
    }
    let bin = harness::unique_bin(&format!("hale_builtin_probe_{}", tag));
    build_opts::build_source(src, &bin, &build_opts::options())
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

/// A shape `hale check` accepted and `hale build` refused, with a
/// message that named lowering rather than the program and, for most,
/// no file or line (downstream handoff), now refused by the check: its
/// one error, the source it points at, the head of its message, and
/// the sweep's verdict. Each case is `(program, located at, message
/// head)`; the programs are plain string literals, for the reason given
/// above.
fn assert_refused_by_the_check(cases: &[(&str, &str, &str)], tag: &str) {
    for (i, (src, at, head)) in cases.iter().enumerate() {
        let program = hale_syntax::parse_source(src).expect("parses");
        let errors: Vec<hale_syntax::Diag> =
            entries::check_program(&program).into_iter().filter(|d| d.is_error()).collect();
        assert_eq!(errors.len(), 1, "one error from the check: {:?}\n{}", errors, src);
        assert_eq!(errors[0].span.slice(src), *at, "located at the construct:\n{}", src);
        assert!(errors[0].message.starts_with(head), "{}\n{}", errors[0].message, src);
        assert_eq!(
            sweep_verdict(src, &format!("hale_cb_{tag}_{i}")),
            Verdict::Skipped("the checker rejects it"),
            "the check refuses it, so the build is never reached:\n{}",
            src
        );
    }
}

/// The neighbour of a refused shape that does lower, so the rule stays
/// exactly as wide as the build's refusal.
fn assert_built(src: &str, tag: &str) {
    assert_eq!(sweep_verdict(src, tag), Verdict::Built, "check-clean and buildable:\n{}", src);
}

/// A `type` holding a locus, as a field or an enum variant's payload:
/// lowering looked the locus up as a type ("unknown type name `M` in
/// signature"). A `type` holding an interface (a fat pointer) lowers.
#[test]
fn a_locus_in_a_type_is_refused_by_the_check_not_the_build() {
    assert_refused_by_the_check(
        &[
            (
                "locus M { params { n: Int = 1; } }\n\
                 type Layer { w: M; }\n\
                 fn main() { let l = Layer { w: M { } }; println(l.w.n); }\n",
                "M",
                "`type Layer`: field `w` holds the locus `M`. A `type` is plain data",
            ),
            (
                "locus M { params { n: Int = 1; } }\n\
                 type Layer { ws: [M; 2]; }\n\
                 fn main() { println(\"x\"); }\n",
                "[M; 2]",
                "`type Layer`: field `ws` holds the locus `M`.",
            ),
            (
                "locus M { params { n: Int = 1; } }\n\
                 type Pick = enum { One(M), Neither };\n\
                 fn main() { println(\"x\"); }\n",
                "M",
                "`type Pick`: variant `One` holds the locus `M`.",
            ),
        ],
        "locus_in_type",
    );
    assert_built(
        "interface Sized { fn size() -> Int; }\n\
         locus Box { params { v: Int = 1; } fn size() -> Int { return self.v; } }\n\
         type Holder { item: Sized; }\n\
         fn main() { let h = Holder { item: Box { } }; println(h.item.size()); }\n",
        "hale_cb_interface_in_type",
    );
}

/// A topic whose payload the bus cannot carry, subscribed ("m60
/// requires a TypeRef...") or sent ("bus send payload must be a
/// user-type"). A topic declared `publish` and never sent lowers.
#[test]
fn an_uncarried_topic_payload_is_refused_by_the_check_not_the_build() {
    assert_refused_by_the_check(
        &[
            (
                "topic Chunk { payload: String; }\n\
                 locus Sub {\n\
                 bus { subscribe Chunk as on_chunk; }\n\
                 fn on_chunk(s: String) { println(s); }\n\
                 }\n\
                 fn main() { let s = Sub { }; }\n",
                "subscribe Chunk as on_chunk;",
                "subscribe `Chunk`: topic `Chunk` declares `payload: String`, which the bus cannot carry.",
            ),
            (
                "topic Chunk { payload: Int; }\n\
                 locus Pub { bus { publish Chunk; } run() { Chunk <- 4; } }\n\
                 fn main() { Pub { }; }\n",
                "Chunk <- 4;",
                "`Chunk <- ...`: topic `Chunk` declares `payload: Int`, which the bus cannot carry.",
            ),
        ],
        "uncarried_topic",
    );
    assert_built(
        "topic Chunk { payload: String; }\n\
         locus Pub { bus { publish Chunk; } run() { } }\n\
         fn main() { Pub { }; }\n",
        "hale_cb_publish_only_topic",
    );
}

/// The recovery modifiers lowering does not implement: `quarantine(c)
/// for d` (spec/semantics.md § `quarantine(child) [for d]`) and
/// `until`, refused by codegen with no span.
#[test]
fn an_unlowered_recovery_modifier_is_refused_by_the_check_not_the_build() {
    let desk = |stmt: &str| {
        format!(
            "locus Book {{\n\
             params {{ credits: Int = 0; debits: Int = 0; }}\n\
             closure balanced {{ self.credits ~~ self.debits within 0; }}\n\
             }}\n\
             locus Desk {{\n\
             params {{ n: Int = 0; }}\n\
             on_failure(c: Book, err: ClosureViolation) {{ {stmt} }}\n\
             run() {{ Book {{ credits: 100, debits: 93 }}; }}\n\
             }}\n\
             fn main() {{ let d = Desk {{ }}; }}\n"
        )
    };
    let quarantine = desk("quarantine(c) for 5s;");
    let until = desk("restart(c) until self.n < 3;");
    let bubble = desk("bubble(err) for 2;");
    assert_refused_by_the_check(
        &[
            (
                &quarantine,
                "quarantine(c) for 5s;",
                "`quarantine(c) for d` (an automatic restart after a duration) is specified but not implemented",
            ),
            (&until, "restart(c) until self.n < 3;", "`until` (restart while a condition holds) is specified but not implemented"),
            (&bubble, "bubble(err) for 2;", "`for` bounds how many times a child is restarted"),
        ],
        "recovery_modifier",
    );
    assert_built(&desk("restart(c) for 2;"), "hale_cb_restart_for_n");
}

/// A bare expression statement: codegen refused it with no file or
/// line ("expression statement other than locus literal or builtin
/// call"). A call in statement position lowers.
#[test]
fn an_unused_expression_statement_is_refused_by_the_check_not_the_build() {
    assert_refused_by_the_check(
        &[
            ("fn main() { let a = 1; a + 2; println(\"x\"); }\n", "a + 2", "this expression's value is unused"),
            (
                "type T { a: Int = 0; }\n\
                 fn main() { let t = T { }; t.a; println(\"x\"); }\n",
                "t.a",
                "this expression's value is unused",
            ),
        ],
        "unused_expression",
    );
    assert_built("fn ping() -> Int { return 1; }\nfn main() { ping(); println(\"x\"); }\n", "hale_cb_discarded_call");
}

/// `return;` in an `on_failure` body checked clean and the build
/// refused it ("`return` outside a user fn"), though the same statement
/// lowered in `accept()` and `drain()`. It is lowered now, as the
/// handler's fall-through taken early; `tests/hale/on_failure_return_test.hl`
/// runs it. A `return` carrying a value has nowhere to go, and the
/// check says so.
#[test]
fn a_return_in_an_on_failure_body_is_built() {
    let desk = |stmt: &str| {
        format!(
            "locus Book {{\n\
             params {{ credits: Int = 0; debits: Int = 0; }}\n\
             closure balanced {{ self.credits ~~ self.debits within 0; }}\n\
             }}\n\
             locus Desk {{\n\
             params {{ seen: Int = 0; }}\n\
             on_failure(c: Book, err: ClosureViolation) {{ {stmt} }}\n\
             run() {{ Book {{ credits: 100, debits: 93 }}; }}\n\
             }}\n\
             fn main() {{ let d = Desk {{ }}; }}\n"
        )
    };
    assert_built(&desk("if self.seen > 2 { return; } self.seen = self.seen + 1;"), "hale_cb_on_failure_return");
    let valued = desk("return 5;");
    assert_refused_by_the_check(
        &[(&valued, "5", "`return` in an `on_failure` body carries no value")],
        "on_failure_return_value",
    );
}

/// `accept(c: T)` with `T` neither a locus nor an interface (a
/// primitive, a `type`, a perspective): lowering refused it with no
/// span ("accept() param must be a locus type"). Accepting a locus
/// lowers; an interface-typed `accept` is the spec's accept bubbling and
/// is not judged by the check.
#[test]
fn a_non_locus_accept_param_is_refused_by_the_check_not_the_build() {
    let prim = "locus H { params { n: Int = 0; } accept(c: Int) { self.n = c; } }\n\
                fn main() { let h = H { }; println(h.n); }\n";
    let record = "type T { a: Int = 0; }\n\
                  locus H { params { n: Int = 0; } accept(c: T) { self.n = c.a; } }\n\
                  fn main() { let h = H { }; println(h.n); }\n";
    let persp = "perspective P { fn x() -> Int; }\n\
                 locus H { params { n: Int = 0; } accept(c: P) { } }\n\
                 fn main() { let h = H { }; println(h.n); }\n";
    assert_refused_by_the_check(
        &[
            (prim, "Int", "`accept(c: Int)`: `accept()` takes a locus or an interface"),
            (record, "T", "`accept(c: T)`: `accept()` takes a locus or an interface"),
            (persp, "P", "`accept(c: P)`: `accept()` takes a locus or an interface"),
        ],
        "non_locus_accept",
    );
    assert_built(
        "locus Kid { params { n: Int = 1; } }\n\
         main locus H { params { n: Int = 0; } accept(c: Kid) { self.n = c.n; } birth() { Kid { }; } run() { println(self.n); } }\n\
         fn main() { H { }; }\n",
        "hale_cb_locus_accept",
    );
}

/// `return <value>;` in a body that declares no return type (a fn with
/// no `-> T`, a lifecycle hook): lowering refused it with no span ("fn
/// declared with no return type but `return e;` carries a value").
/// `fn main`'s `return n;` is the exit code and builds.
#[test]
fn a_value_returned_from_a_body_that_returns_nothing_is_refused_by_the_check_not_the_build() {
    assert_refused_by_the_check(
        &[
            (
                "locus D { params { n: Int = 0; } run() { if self.n == 0 { return 5; } } }\n\
                 fn main() { let d = D { }; }\n",
                "5",
                "`return` carries a value, but this body returns nothing",
            ),
            (
                "locus D { params { n: Int = 0; } fn bump() { return self.n + 1; } }\n\
                 fn main() { let d = D { }; d.bump(); }\n",
                "self.n + 1",
                "`return` carries a value, but this body returns nothing",
            ),
            (
                "fn note(n: Int) { if n > 0 { return n; } println(n); }\n\
                 fn main() { note(1); }\n",
                "n",
                "`return` carries a value, but this body returns nothing",
            ),
        ],
        "unit_return_value",
    );
    assert_built("fn main() { println(\"x\"); return 3; }\n", "hale_cb_main_exit_code");
}

/// A topic whose payload is `BytesView`, the raw-frame path, used where
/// no ring carries raw frames: subscribed or sent with no binding
/// ("no serializer synthesized for bus payload type `BytesView`" / "bus
/// send payload must be a user-type"), or bound to a native `shm_ring`
/// with no `layout:` ("only single-segment struct payloads are
/// supported"). Bound to a `layout:` ring, it builds.
#[test]
fn a_raw_frame_topic_off_a_layout_ring_is_refused_by_the_check_not_the_build() {
    let program = |main_body: &str| {
        format!(
            "topic Recs {{ payload: BytesView; }}\n\
             ring_layout ForeignRing {{\n\
             magic 0x52494E47464D5431;\n\
             version 1 at 8 : u32;\n\
             buffer_size at 12 : u32;\n\
             data_at 128;\n\
             cursor published {{ at 64; repr atomic_u64; load acquire; unit bytes; }}\n\
             framing byte_records {{ len_prefix u32; align 8; pad_sentinel 0xFFFFFFFF; }}\n\
             overflow lap_detect;\n\
             }}\n\
             locus Reader {{ params {{ n: Int = 0; }} bus {{ subscribe Recs as on_rec; }} fn on_rec(v: BytesView) {{ self.n = self.n + 1; }} }}\n\
             main locus App {{ params {{ r: Reader = Reader {{ }}; }} {main_body} }}\n\
             fn main() {{ App {{ }}; }}\n"
        )
    };
    let unbound = program("");
    let native = program("bindings { Recs: shm_ring(\"/hale.cb.recs.native\", on_overflow: drop); }");
    let sent = "topic Recs { payload: BytesView; }\n\
                locus Feed { bus { publish Recs; } run() { let b = std::bytes::BytesBuilder { initial_cap: 64 }; b.append_u8(2); Recs <- b.view(); } }\n\
                fn main() { Feed { }; }\n";
    assert_refused_by_the_check(
        &[
            (&unbound, "subscribe Recs as on_rec;", "subscribe `Recs`: topic `Recs` declares `payload: BytesView`, the raw-frame path"),
            (&native, "Recs: shm_ring(\"/hale.cb.recs.native\", on_overflow: drop);", "`Recs: shm_ring(...)`: topic `Recs` declares `payload: BytesView`"),
            (sent, "Recs <- b.view();", "`Recs <- ...`: topic `Recs` declares `payload: BytesView`, the raw-frame path"),
        ],
        "raw_frame_topic",
    );
    assert_built(
        &program("bindings { Recs: shm_ring(\"/hale.cb.recs.layout\", on_overflow: drop, layout: ForeignRing); }"),
        "hale_cb_raw_frame_layout_ring",
    );
}

/// The 2026-10-12 friction sweep's shapes, each held to the sweep's own
/// verdict: what the check now accepts builds, and what it refuses is
/// refused by the check, never first by the build. In particular an
/// interface-typed parameter's field read passed the check (the local
/// typed `Unknown`) and was refused by codegen ("field access `.v` on
/// non-record type Interface"), the divergence this file exists for.
///
/// Plain string literals, for the reason given above.
#[test]
fn the_friction_sweep_shapes_agree() {
    let builds = [
        // F.43: the framework vocabulary names fields, params, locals, fns.
        ("type Job { run: Int; epoch: Int; restart: Bool = false; }\n\
          fn run(capacity: Int) -> Int { let where = capacity + 1; return where; }\n\
          locus W { params { tier: Int = 1; } fn restart() -> Int { return self.tier; } }\n\
          fn main() { let j = Job { run: 1, epoch: 2 }; let w = W { }; println(run(j.run + j.epoch) + w.restart()); }\n",
         "hale_cb_fs_words"),
        // `or discard` drops a value-bearing fallible as a statement.
        ("type Oops { why: String = \"\"; }\n\
          fn parse(s: String) -> Int fallible(Oops) { if s == \"bad\" { fail Oops { why: s }; } return 7; }\n\
          fn main() { parse(\"bad\") or discard; parse(\"ok\") or discard; }\n",
         "hale_cb_fs_discard"),
        // A literal const takes the literal's type; a trailing comma.
        ("const LIMIT = 100;\nfn add(a: Int, b: Int) -> Int { return a + b; }\n\
          fn main() { println(add(LIMIT, 2,)); }\n",
         "hale_cb_fs_const"),
        // `@unbounded` before a mode.
        ("@form(vec)\nlocus V { capacity { heap items of String; } }\n\
          locus L { params { v: V = V { }; n: Int = 3; }\n\
          @unbounded\n mode bulk() { let mut i = 0; while i < self.n { self.v.push(\"x\"); i = i + 1; } }\n\
          run() { self.bulk(); } }\n\
          fn main() { let l = L { }; }\n",
         "hale_cb_fs_mode"),
        // An interface parameter's methods are callable.
        ("interface T { fn x() -> Int; }\n\
          locus A { params { v: Int = 1; } fn x() -> Int { return self.v; } }\n\
          fn f(c: T) -> Int { let d = c; return d.x() + c.x(); }\n\
          fn main() { let a = A { }; println(f(a)); }\n",
         "hale_cb_fs_iface_ok"),
    ];
    for (src, tag) in builds {
        assert_eq!(sweep_verdict(src, tag), Verdict::Built, "{}", src);
    }
    let refused_by_check = [
        "type Oops { why: String = \"\"; }\n\
         fn parse(s: String) -> Int fallible(Oops) { return 7; }\n\
         fn main() { let x = parse(\"1\") or discard; println(x); }\n",
        "interface T { fn x() -> Int; }\n\
         locus A { params { v: Int = 1; } fn x() -> Int { return self.v; } }\n\
         fn f(c: T) -> Int { return c.v; }\n\
         fn main() { let a = A { }; println(f(a)); }\n",
        "interface T { fn x() -> Int; }\n\
         locus A { params { v: Int = 1; } fn x() -> Int { return self.v; } }\n\
         locus H { fn m(c: T) -> Int { return c.y(); } }\n\
         fn main() { let a = A { }; let h = H { }; println(h.m(a)); }\n",
        "locus F { params { ps: [Decimal; 2] = [1.5d, 2.0d]; } run { println(self.ps.sum(0.0d)); } }\n\
         fn main() { let f = F { }; }\n",
    ];
    for src in refused_by_check {
        assert_eq!(
            sweep_verdict(src, "hale_cb_fs_refused"),
            Verdict::Skipped("the checker rejects it"),
            "the check refuses it, so the build is never reached:\n{}",
            src
        );
    }
}

/// `bounded[Time; N]` checked clean and the build refused its element
/// type ("element type Time not supported"): a `Time` is the i64 a
/// `Duration` is, so it lowers as a scalar element, in a type's field and
/// in a locus's params. Behaviour: tests/hale/bounded_time_test.hl.
///
/// Plain string literals, for the reason given above.
#[test]
fn a_bounded_of_time_checks_and_builds() {
    let builds = [
        ("type W { seen: bounded[Time; 3]; }\n\
          fn main() { let mut w = W { }; push(w.seen, `2026-05-08T12:00:00Z`) or { }; println(count(w.seen)); }\n",
         "hale_cb_bounded_time_field"),
        ("locus L {\n\
          params { marks: bounded[Time; 2]; }\n\
          run() { push(self.marks, `2026-05-08T12:00:00Z`) or { }; for x in self.marks { println(x); } }\n\
          }\n\
          fn main() { L { }; }\n",
         "hale_cb_bounded_time_param"),
    ];
    for (src, tag) in builds {
        assert_eq!(sweep_verdict(src, tag), Verdict::Built, "{}", src);
    }
}

/// An expression that produces no value, put where a value is taken by
/// its type rather than checked against a declared one: a `let`, a
/// tuple or array element, an operand, a generic fn's argument. `let
/// full = push(self.marks, t) or { };` checked clean and the build
/// refused it ("`or` expression in value position has Unit success
/// type"), as did `let u = g();` for a fn returning nothing ("fn `g`
/// returns no value but is used in expression position"). There is no
/// `()` value to bind or compute with, so the check refuses each at the
/// expression; the same call written as a statement builds.
///
/// Plain string literals, for the reason given above.
#[test]
fn a_unit_value_in_a_value_position_is_refused_by_the_check_not_the_build() {
    let marks = |stmt: &str| {
        format!(
            "locus L {{\n\
             params {{ marks: bounded[Time; 2]; }}\n\
             run() {{ let t = `2026-05-08T12:00:00Z`; {stmt} println(count(self.marks)); }}\n\
             }}\n\
             fn main() {{ L {{ }}; }}\n"
        )
    };
    let or_block = marks("let full = push(self.marks, t) or { };");
    let or_discard = marks("let full = push(self.marks, t) or discard;");
    assert_refused_by_the_check(
        &[
            (&or_block, "push(self.marks, t) or { }", "let `full` needs a value, and this expression produces none"),
            (&or_discard, "push(self.marks, t) or discard", "let `full` needs a value, and this expression produces none"),
            ("fn g() { println(\"g\"); }\nfn main() { let u = g(); }\n", "g()", "let `u` needs a value, and this expression produces none"),
            (
                "fn main() { let c = true; let x = if c { println(\"a\"); } else { println(\"b\"); }; }\n",
                "if c { println(\"a\"); } else { println(\"b\"); }",
                "let `x` needs a value, and this expression produces none",
            ),
            ("fn g() { println(\"g\"); }\nfn main() { let (a, b) = (g(), 1); println(b); }\n", "g()", "a tuple element needs a value, and this expression produces none"),
            ("fn g() { println(\"g\"); }\nfn main() { let a = [g()]; }\n", "g()", "an array element needs a value, and this expression produces none"),
            ("fn g() { println(\"g\"); }\nfn main() { let n = -g(); }\n", "g()", "the operand of a unary operator needs a value, and this expression produces none"),
            (
                "fn g() { println(\"g\"); }\nfn id<T>(x: T) -> T { return x; }\nfn main() { id(g()); }\n",
                "g()",
                "an argument to generic fn `id` needs a value, and this expression produces none",
            ),
        ],
        "unit_value",
    );
    assert_built(&marks("push(self.marks, t) or { };"), "hale_cb_unit_or_block_statement");
}

/// An operator over operands it does not apply to: the check held the
/// two sides to one type and asked nothing more, so `true < false`, two
/// structs compared with `==`, `b"a" + b"b"`, an enum ordered or `!` of
/// an `Int` checked clean and the build refused each ("binop Lt on
/// Bool", "binop Eq on TypeRef(..)", "unop Not on Int"). The table the
/// check now holds is the one lowering implements (spec/semantics.md §
/// "Operators and their operand types").
///
/// Plain string literals, for the reason given above.
#[test]
fn an_operator_over_operands_it_does_not_apply_to_is_refused_by_the_check_not_the_build() {
    let bin = |decl: &str, ty: &str, v: &str, op: &str| {
        format!("{decl}fn main() {{ let x: {ty} = {v}; let y: {ty} = {v}; let e = x {op} y; println(\"ok\"); }}\n")
    };
    let p = "type P { a: Int; }\n";
    let c = "type Color = enum { Red, Green };\n";
    let cases = [
        (bin("", "Bool", "true", "<"), "x < y", "`<` does not apply to `Bool`"),
        (bin(p, "P", "P { a: 1 }", "=="), "x == y", "`==` does not apply to a struct (`P`)"),
        (bin("", "(Int, Int)", "(1, 2)", "=="), "x == y", "`==` does not apply to a tuple"),
        (bin("", "Bytes", "b\"a\"", "+"), "x + y", "`+` does not apply to a byte string"),
        (bin(c, "Color", "Color::Red", "<"), "x < y", "`<` does not apply to `Color`"),
        (bin("", "Float", "1.5", "%"), "x % y", "`%` does not apply to `Float`"),
        (bin("", "String", "\"a\"", "-"), "x - y", "`-` does not apply to `String`"),
        (bin("", "Int", "1", "&&"), "x && y", "`&&` does not apply to `Int`"),
        ("fn main() { let x = 1; let e = !x; println(\"ok\"); }\n".to_string(), "!x", "`!` does not apply to `Int`"),
        (
            "fn main() { let mut x = true; x += false; println(\"ok\"); }\n".to_string(),
            "x += false;",
            "`+` does not apply to `Bool`",
        ),
    ];
    let cases: Vec<(&str, &str, &str)> = cases.iter().map(|(s, a, h)| (s.as_str(), *a, *h)).collect();
    assert_refused_by_the_check(&cases, "operator");
    for (i, src) in [
        bin(c, "Color", "Color::Red", "!="),
        bin("", "String", "\"a\"", "<"),
        bin("", "Decimal", "1.5d", "%"),
        bin("", "Bool", "true", "&&"),
    ]
    .iter()
    .enumerate()
    {
        assert_built(src, &format!("hale_cb_operator_ok_{i}"));
    }
}

/// The places that took a value's type on trust: the values an enum
/// variant carries, a literal in a pattern, a `bounded` buffer's element
/// type, a generic fn's type argument, a hashmap's key field, a topic
/// payload's fields. Each checked clean and the build refused the
/// program ("`Wr::One` arg 0: expected type Int, got String", "match arm
/// pattern type String doesn't match scrutinee type Int", "bounded[T;
/// N]: element type Tuple(..) not supported", "generic type argument
/// `Money` not supported", "key type Bool unsupported", "bus payload
/// field `x: Tuple(..)`"). The neighbours that lower are built, a
/// payload-free enum on the wire among them (its tag, since 2026-10-13).
///
/// Plain string literals, for the reason given above.
#[test]
fn a_value_where_its_place_cannot_hold_it_is_refused_by_the_check_not_the_build() {
    assert_refused_by_the_check(
        &[
            (
                "type Wr = enum { One(Int), No };\nfn main() { let w = Wr::One(\"s\"); println(\"ok\"); }\n",
                "\"s\"",
                "`Wr::One` value 0: expected `Int`, got `String`",
            ),
            (
                "type Wr = enum { One(Int), No };\nfn main() { let w = Wr::One(1, 2); println(\"ok\"); }\n",
                "Wr::One(1, 2)",
                "`Wr::One` carries 1 value, and this passes 2",
            ),
            (
                "fn main() { let x = 3; match x { \"a\" -> { println(\"a\"); }, _ -> { println(\"o\"); }, } }\n",
                "\"a\"",
                "this pattern is of type `String`, and the value matched is of type `Int`",
            ),
            (
                "fn main() { let x = (1, \"s\"); match x { (1, 2) -> { println(\"a\"); }, _ -> { println(\"o\"); }, } }\n",
                "2",
                "this pattern is of type `Int`, and the value matched is of type `String`",
            ),
            (
                "type W { xs: bounded[(Int, Int); 2]; }\nfn main() { let w = W { }; println(count(w.xs)); }\n",
                "bounded[(Int, Int); 2]",
                "`bounded[(Int, Int); N]`: a tuple",
            ),
            (
                "type Shape = enum { Dot, Circle(Int) };\ntype W { xs: bounded[Shape; 2]; }\n\
                 fn main() { let w = W { }; println(count(w.xs)); }\n",
                "bounded[Shape; 2]",
                "`bounded[Shape; N]`: `Shape` is an enum with payloads",
            ),
            (
                "unit cent;\ntype Money = quantity Int in cent;\nfn id<G>(x: G) -> G { return x; }\n\
                 fn main() { let m: Money = 3cent; let y = id(m); println(\"ok\"); }\n",
                "id",
                "generic fn `id`: `G` would be `Money`, a quantity",
            ),
            (
                "type Row { k: Bool; n: Int = 0; }\n@form(hashmap)\nlocus M { capacity { pool rows of Row indexed_by k; } }\n\
                 fn main() { let m = M { }; println(\"ok\"); }\n",
                "k",
                "@form(hashmap) key `Row.k` is a `Bool`",
            ),
            (
                "type Shape = enum { Dot, Circle(Int) };\ntype Msg { x: Shape; n: Int = 0; }\n\
                 topic Ev { payload: Msg; subject: \"ev.x\"; }\n\
                 locus Sub { bus { subscribe Ev as on_ev; } fn on_ev(m: Msg) { println(m.n); } }\n\
                 fn main() { let s = Sub { }; }\n",
                "subscribe Ev as on_ev;",
                "subscribe `Ev`: topic `Ev` declares `payload: Msg`, whose field `x` is an enum with payloads (`Shape`)",
            ),
            (
                "type Msg { x: (Int, Int); n: Int = 0; }\n\
                 topic Ev { payload: Msg; subject: \"ev.x\"; }\n\
                 locus Sub { bus { subscribe Ev as on_ev; } fn on_ev(m: Msg) { println(m.n); } }\n\
                 fn main() { let s = Sub { }; }\n",
                "subscribe Ev as on_ev;",
                "subscribe `Ev`: topic `Ev` declares `payload: Msg`, whose field `x` is a tuple",
            ),
        ],
        "place",
    );
    for (i, src) in [
        "type Fee = enum { Flat(Decimal<2>), Waived };\nfn main() { let f = Fee::Flat(1.25d); println(\"ok\"); }\n",
        "fn main() { let d = 60s; match d { 1min -> { println(\"m\"); }, _ -> { println(\"o\"); }, } }\n",
        "type Color = enum { Red, Green };\ntype W { xs: bounded[Color; 2]; }\n\
         fn main() { let mut w = W { }; push(w.xs, Color::Green) or { }; println(count(w.xs)); }\n",
        "type OrderId = distinct Int;\nfn id<G>(x: G) -> G { return x; }\nfn main() { let o = id(OrderId(7)); println(Int(o)); }\n",
        "type Color = enum { Red, Green };\n\
         locus Q { params { x: Color = Color::Red; } fn put(v: Color) { self.x = v; } }\n\
         fn main() { let q = Q { }; q.put(Color::Green); println(\"ok\"); }\n",
        "type Oops { why: String = \"\"; }\n\
         fn parse(s: String) -> Int fallible(Oops) { if s == \"bad\" { fail Oops { why: s }; } return 7; }\n\
         fn main() { parse(\"bad\") or { println(err.why); }; parse(\"bad\") or 0; }\n",
        "type Status = enum { Up, Down };\ntype Msg { key: Status = Status::Up; n: Int = 0; }\n\
         topic Ev { payload: Msg; subject: \"ev.x\"; keyed_by key; }\n\
         locus Sub { bus { subscribe Ev as on_ev; } fn on_ev(m: Msg) { println(m.n); } }\n\
         main locus App { params { s: Sub = Sub { }; } bus { publish Ev; } run() { Ev <- Msg { key: Status::Down }; } }\n\
         fn main() { App { }; }\n",
    ]
    .iter()
    .enumerate()
    {
        assert_built(src, &format!("hale_cb_place_ok_{i}"));
    }
}
