//! The loaded-seed harness agrees with the bare-program harness (F.40
//! phase 4, T1).
//!
//! A codegen test used to build through a bare-program adapter in
//! codegen that handed a parsed `Program` to `Snapshot::from_program`: a
//! pipeline of its own beside the one every verb runs, which loads a
//! seed. That adapter is gone (T2); `build_opts::build_program` is the
//! same two steps, `Snapshot::from_program` then `build_resolved`, for the
//! tests whose subject is a `Program` they made. `build_opts::build_source`
//! builds from the text instead, as a
//! verb builds a seed of one file: an overlay buffer at a virtual
//! `main.hl`, `Snapshot::load`, the harness's configuration (lowering not
//! gated on the check), the lowering view, `build_resolved`.
//!
//! Before any test moves onto it, this holds the two paths to each other
//! over every program the corpus carries (`hale_corpus::all`: the `.hl`
//! fixtures and every program a test embeds): the same pre-optimization
//! IR, and a program one path refuses is refused by the other with the
//! same error. A program that does not parse is refused by the parser on
//! the bare path (the test's own `parse_source`, before the build) and
//! by the load on the other: the same diagnostics, with the same spans
//! (one file loads at base 0), carried in the load-failure
//! error.
//!
//! The IR is compared verbatim. Neither path stamps an identity into it:
//! the snapshot keys differ (a loaded seed's digests its path and text, a
//! bare program's is the handoff's ordinal), but a harness build carries
//! no execution digest or model hash (`BuildOptions::exec_digest`,
//! `model_hash`), so no `lotus_obs_exec_digest_set` constant is emitted
//! and none needs masking. The one mask is in a panic's message (see
//! [`without_addresses`]).
//!
//! A program that `import`s a sibling seed is not a program of one file:
//! the bare path builds it with no rename table and the seed path looks
//! for the sibling in a directory that holds nothing. Those are counted
//! and named in the report, never compared (the multi-file and import
//! sites move with their own scratch directories, T2).
//!
//! The default run is a deterministic sample (every `SAMPLE_STRIDE`-th
//! program); `HALE_HARNESS_AGREEMENT=full` compares every program, the
//! way `ownership_matrix` reads `HALE_MATRIX`.

use std::sync::atomic::{AtomicUsize, Ordering};

use hale_codegen::{BuildOptions, CodegenError};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The sample's stride over the corpus, in corpus order: about 120 of
/// its 2,400 programs, some 10 s.
const SAMPLE_STRIDE: usize = 20;

/// What one path made of a program.
#[derive(Debug, PartialEq)]
enum Outcome {
    /// It parsed and built: the pre-optimization IR.
    Built(String),
    /// The parser refused it: each diagnostic's message and span.
    Unparsed(Vec<(String, hale_syntax::Span)>),
    /// The build refused it: the error, and the IR when lowering got as
    /// far as writing it (a link failure).
    Refused(String, Option<String>),
    /// The build panicked: the panic's message. A compiler panic is a
    /// defect of its own; here it is one more outcome both paths must
    /// share, and it must not take the other programs' verdicts with it.
    Panicked(String),
}

/// Run one path's build, a panic caught as its outcome.
fn caught(build: impl FnOnce() -> Outcome) -> Outcome {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(build)).unwrap_or_else(|p| {
        let text = p
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        Outcome::Panicked(without_addresses(&text))
    })
}

/// A panic message with the heap addresses it prints masked. inkwell's
/// "expected the IntValue variant" panic prints the offending LLVM value,
/// `address: 0x…` included: where that build's allocator put the value,
/// which no two builds share, so the same panic of the same program
/// never prints the same text twice. The address is masked; everything
/// else the message says (the value's name, its instruction, its type)
/// is compared.
fn without_addresses(text: &str) -> String {
    const AT: &str = "address: 0x";
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(AT) {
        out.push_str(&rest[..i + AT.len()]);
        out.push('…');
        rest = rest[i + AT.len()..].trim_start_matches(|c: char| c.is_ascii_hexdigit());
    }
    out.push_str(rest);
    out
}

/// The options both paths build with: the test default, the IR dumped
/// beside the binary, and the dev profile, which shortens the LLVM work
/// after the dump without touching what is dumped (both paths share it).
fn options(ll: &std::path::Path) -> BuildOptions {
    BuildOptions { dump_ir: Some(ll.to_path_buf()), dev_profile: true, ..build_opts::options() }
}

fn finish(result: Result<(), CodegenError>, bin: &std::path::Path, ll: &std::path::Path) -> Outcome {
    let ir = std::fs::read_to_string(ll).ok();
    let _ = std::fs::remove_file(ll);
    let _ = std::fs::remove_file(bin);
    match result {
        Ok(()) => Outcome::Built(ir.expect("BuildOptions::dump_ir should have written the .ll")),
        Err(e) => Outcome::Refused(format!("{:?}", e), ir),
    }
}

/// The bare-program path: the parsed `Program` handed to `Snapshot::from_program`.
fn bare(source: &str, bin: &std::path::Path) -> Outcome {
    let program = match hale_syntax::parse_source(source) {
        Ok(p) => p,
        Err(diags) => return Outcome::Unparsed(diags.into_iter().map(|d| (d.message, d.span)).collect()),
    };
    let ll = bin.with_extension("ll");
    let result = build_opts::build_program(&program, bin, &[], &options(&ll));
    finish(result, bin, &ll)
}

/// The loaded-seed path. A load that fails on a parse is read back
/// through `load_seed` for its diagnostics, after checking the helper
/// refused it with the load's rendering, as the bare path maps a load
/// failure.
fn seeded(source: &str, bin: &std::path::Path) -> Outcome {
    let ll = bin.with_extension("ll");
    let options = options(&ll);
    let result = build_opts::build_source(source, bin, &options);
    if let Err(hale_frontend::snapshot::LoadError::Load(f)) =
        build_opts::load_seed(source, build_opts::harness_config(&options))
    {
        if f.io.is_empty() && !f.diags.is_empty() {
            assert!(
                matches!(&result, Err(CodegenError::Unsupported(t)) if *t == f.text()),
                "a program that does not parse is refused with the load's rendering, got {result:?}"
            );
            let _ = std::fs::remove_file(&ll);
            return Outcome::Unparsed(f.diags.into_iter().map(|d| (d.message, d.span)).collect());
        }
    }
    finish(result, bin, &ll)
}

/// The first line on which two IR texts differ, both sides.
fn first_difference(a: &str, b: &str) -> String {
    let (mut la, mut lb) = (a.lines(), b.lines());
    let mut n = 0;
    loop {
        n += 1;
        match (la.next(), lb.next()) {
            (Some(x), Some(y)) if x == y => continue,
            (None, None) => return "no line differs".to_string(),
            (x, y) => return format!("line {n}: bare `{}` / seed `{}`", x.unwrap_or("<end>"), y.unwrap_or("<end>")),
        }
    }
}

/// How the two outcomes of one program compare.
#[derive(Debug)]
enum Agreement {
    Identical,
    UnparsedAlike,
    RefusedAlike,
    /// Both paths' builds panicked with one message: reported by name.
    PanickedAlike(String),
    NotOneFile,
    Differs(String),
}

fn agreement(source: &str, tag: &str) -> Agreement {
    if source.contains("import \"") {
        return Agreement::NotOneFile;
    }
    // One path at a time, both to the same binary path, so nothing a
    // build derives from its output path can differ between them.
    let bin = harness::unique_bin(tag);
    let a = caught(|| bare(source, &bin));
    let b = caught(|| seeded(source, &bin));
    // A panic leaves what the build had written.
    let _ = std::fs::remove_file(bin.with_extension("ll"));
    let _ = std::fs::remove_file(&bin);
    match (a, b) {
        (Outcome::Built(x), Outcome::Built(y)) if x == y => Agreement::Identical,
        (Outcome::Built(x), Outcome::Built(y)) => Agreement::Differs(format!("IR, {}", first_difference(&x, &y))),
        (Outcome::Unparsed(x), Outcome::Unparsed(y)) if x == y => Agreement::UnparsedAlike,
        (Outcome::Refused(x, xi), Outcome::Refused(y, yi)) if x == y && xi == yi => Agreement::RefusedAlike,
        (Outcome::Panicked(x), Outcome::Panicked(y)) if x == y => Agreement::PanickedAlike(x),
        (a, b) => Agreement::Differs(format!("bare {} / seed {}", summary(&a), summary(&b))),
    }
}

fn summary(o: &Outcome) -> String {
    match o {
        Outcome::Built(_) => "built".to_string(),
        Outcome::Unparsed(d) => format!("unparsed {:?}", d.first()),
        Outcome::Refused(e, ir) => format!("refused `{}`{}", e, if ir.is_some() { " (IR written)" } else { "" }),
        Outcome::Panicked(m) => format!("panicked `{m}`"),
    }
}

fn threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get().min(8)).unwrap_or(4)
}

#[test]
fn the_loaded_seed_harness_builds_what_the_bare_program_harness_builds() {
    let full = std::env::var("HALE_HARNESS_AGREEMENT").as_deref() == Ok("full");
    let programs: Vec<hale_corpus::Program> = hale_corpus::all()
        .into_iter()
        .enumerate()
        .filter(|(i, _)| full || i % SAMPLE_STRIDE == 0)
        .map(|(_, p)| p)
        .collect();
    let next = AtomicUsize::new(0);
    let programs = &programs;
    let mut verdicts: Vec<(usize, Agreement)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads())
            .map(|_| {
                let next = &next;
                scope.spawn(move || {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(p) = programs.get(i) else { break };
                        mine.push((i, agreement(&p.source, &format!("seed_agree_{i}"))));
                    }
                    mine
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    });
    verdicts.sort_by_key(|(i, _)| *i);

    let (mut identical, mut unparsed, mut refused) = (0usize, 0usize, 0usize);
    let (mut panicked, mut not_one_file, mut differing) = (Vec::new(), Vec::new(), Vec::new());
    for (i, v) in verdicts {
        let origin = &programs[i].origin;
        match v {
            Agreement::Identical => identical += 1,
            Agreement::UnparsedAlike => unparsed += 1,
            Agreement::RefusedAlike => refused += 1,
            Agreement::PanickedAlike(m) => panicked.push(format!("{origin}: {m}")),
            Agreement::NotOneFile => not_one_file.push(origin.clone()),
            Agreement::Differs(d) => differing.push(format!("{origin}: {d}")),
        }
    }
    eprintln!(
        "loaded-seed agreement ({}): {} programs, {} identical, {} unparsed alike, {} refused alike by the build, \
         {} panicked alike, {} differing, {} importing a sibling seed (not compared)",
        if full { "full" } else { "sample" },
        programs.len(),
        identical,
        unparsed,
        refused,
        panicked.len(),
        differing.len(),
        not_one_file.len()
    );
    for o in &panicked {
        eprintln!("  both builds panic: {o}");
    }
    for o in &not_one_file {
        eprintln!("  imports a sibling seed: {o}");
    }
    assert!(identical > 0, "nothing was compared: the corpus or the sample is empty");
    assert!(differing.is_empty(), "the two harnesses disagree on {} program(s):\n{}", differing.len(), differing.join("\n"));
}

/// A program that does not parse, pinned without the corpus: the load
/// refuses it with the parser's own diagnostics, and the helper with the
/// load's located rendering. (A plain literal, not a raw string, so the
/// corpus harvester does not take it in.)
#[test]
fn a_program_that_does_not_parse_is_refused_with_the_parsers_diagnostics() {
    let source = "fn main() {\n    let x = ;\n}\n";
    let bin = harness::unique_bin("unparsed");
    let a = bare(source, &bin);
    let b = seeded(source, &bin);
    assert!(matches!(&a, Outcome::Unparsed(d) if !d.is_empty()), "{a:?}");
    assert_eq!(a, b);
    let err = build_opts::build_source(source, &bin, &build_opts::options()).unwrap_err();
    assert!(
        matches!(&err, CodegenError::Unsupported(t) if t.contains("main.hl:2:")),
        "the refusal names the line in the virtual main.hl: {err:?}"
    );
}