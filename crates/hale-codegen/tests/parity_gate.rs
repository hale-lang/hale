//! GH #1949: an adversarial check/build parity gate. Every program
//! `hale check` accepts must also build — including the programs nobody
//! wrote yet.
//!
//! `corpus_check_build_agreement` holds the repository's own programs to
//! that, and its divergence baseline has been empty since 2026-10-14. But
//! those programs steer around the gaps their authors knew about: probes
//! written as ordinary user code found 7 of 14 check-clean programs that
//! `hale build` refused. This file goes looking for the rest. It
//! GENERATES check-clean programs by mutating the corpus — the example
//! fixtures, `tests/hale`, the programs embedded in Rust tests, and the
//! `hale` snippets of the book and the spec — and builds each one the
//! check accepts.
//!
//! ## What a mutation is
//!
//! `parity/mutators.rs` holds the mutators (its header says how to add
//! one). Each edits a program's text at the spans the parser recorded:
//!
//! - fn tail shapes: `return e;` to a tail and back, a tail `if`/`match`/
//!   block, an early return inside a loop, a return through a `let`;
//! - generics: a value through `fn __pg_id<T>(x: T) -> T`, through a
//!   generic record `__PgBox<T>` a generic fn builds, a free fn's
//!   parameter made a type parameter;
//! - seeds: a declaration moved to another file of the seed, or to a seed
//!   of its own imported under an alias;
//! - element types: a probe exercising `bounded[T; N]` fields and
//!   parameters, `[T; N]` arrays, a payload enum, a generic record, a fn
//!   value, `@form(vec)` and `@form(hashmap)` over `T` drawn from Int,
//!   String, Decimal, U256, I256, U64, a payload enum, a record and the
//!   program's own records and enums; a locus with no form whose own
//!   `items` / `entries` param a `for` walks;
//! - call shapes: a method as a free fn, a call through a fn value, a
//!   forwarding wrapper, interface dispatch;
//! - fallible plumbing: an `or` clause given another disposition (`or
//!   raise`, `or fail err`, `or discard`, a default), a call routed through
//!   a fallible wrapper and addressed each of those ways;
//! - the entry point: `fn main` dropped from a seed with a `main locus`
//!   of its own, which nothing then starts (GH #2108), or its body moved
//!   to a helper the new `fn main` calls.
//!
//! A candidate is a seed program with a chain of one or two mutations.
//! Which ones is a draw over the cross product: every (seed, mutator)
//! pair is drawn or not on a key of its own — the seed's TEXT (FNV-1a),
//! the mutator's NAME and the salt — and every (seed, first, second)
//! triple likewise, at a lower rate; the mutation a drawn pair makes runs
//! on an RNG seeded by the same key. So a candidate is the same on every
//! machine, does not move when an unrelated program joins the corpus,
//! and does not move when a MUTATOR joins `MUTATORS`: the new one adds
//! its own candidates and deals no one else's again
//! (`a_new_mutator_moves_no_other_candidate`). A drawn pair whose
//! mutator finds no site yields nothing rather than passing the draw to
//! another mutator.
//!
//! ## The verdict
//!
//! Each candidate is checked as `hale check <seed dir>` checks it (the
//! editor's configuration: the check plus the build rules) and, when the
//! check reports no error, built as `hale build` builds it (a host build,
//! lowering gated on the check). A refusal, or a panic in lowering, is a
//! DIVERGENCE. One is the mutation's only when the unmutated seed builds:
//! a seed that already diverges is `corpus_check_build_agreement`'s.
//!
//! A divergence is keyed by its SIGNATURE: the first line of the refusal
//! with numbers and quoted names taken out (`gate::signature`), so one
//! cause reached through many programs is one entry. Each report carries
//! the seed, the mutation chain, the refusal, and the program
//! delta-debugged down to the lines the divergence needs
//! (`parity/minimize.rs`).
//!
//! One refusal is not a divergence: a loop of a `@vectorize` fn the
//! build's optimizer did not vectorize on this target (GH #2134, F.62).
//! `hale check` refuses what no target vectorizes; what is left is a
//! target-dependent build assertion by design (spec/semantics.md §
//! Vectorized loops), so the gate counts it apart
//! (`Verdict::TargetAssertion`, `target_assertions` in the run's
//! report) and never lists it.
//!
//! ## The ratchet
//!
//! `tests/fixtures/parity/divergences.txt` lists the divergences known
//! today, one per line: `#<issue> | <reproducer> | <signature>`. Each has
//! a minimized reproducer beside it (`<name>.repro`: its files, each
//! under a `// == <path> ==` line). The gate fails
//!
//! - on a divergence whose signature is not listed (a new gap: fix it,
//!   or refuse it at check with a span; listing it is for a gap that
//!   needs its own change, with an issue filed); and
//! - when a listed reproducer no longer diverges
//!   ([`listed_divergences_still_reproduce`], in the ordinary suite):
//!   the gap is closed, so its line and reproducer go. The list can only
//!   shrink.
//!
//! To list a divergence: file an issue (the reproducer, what the check
//! says, what the build says), save the minimized program the failure
//! printed as `tests/fixtures/parity/<name>.repro`, and add `#<issue> |
//! <name>.repro | <signature>` with the signature the failure printed
//! after `===`. The two entries tagged `#1942` are gaps PR #1962 closes;
//! whichever of the two changes lands second deletes them.
//!
//! ## Running it
//!
//! ```sh
//! # the sample (the ordinary codegen suite, seconds) and the ratchet's
//! # reproducers
//! cargo test --release -p hale-codegen --test parity_gate
//! # the sweep (CI's sanitizer job; every seed, the sweep's draw)
//! cargo test --release -p hale-codegen --test parity_gate -- --ignored --nocapture sweep_
//! ```
//!
//! Knobs (all read by the test, never by codegen):
//!
//! - `HALE_PARITY_SEEDS` caps the sweep's seeds; `HALE_PARITY_SINGLE`
//!   and `HALE_PARITY_CHAIN` set its draw, in parts per thousand of the
//!   (seed, mutator) pairs and the (seed, first, second) triples
//!   (`DRAW`; 1000 and 0 try every mutator once on every seed);
//! - `HALE_PARITY_SALT=<n>` draws another set of candidates, to explore
//!   (the gate's own runs use 0), and `HALE_PARITY_ONLY=<mutator>`
//!   restricts a run to one mutator;
//! - `HALE_CORPUS_SWEEP_THREADS` is the worker count;
//! - `HALE_PARITY_DUMP=<dir>` writes every divergent candidate there as a
//!   `.repro`, and `minimize_a_reproducer` (`HALE_PARITY_REPRO=<file>`)
//!   minimizes one;
//! - `HALE_PARITY_MIN_BUDGET` caps the minimizer's builds per divergence
//!   (0 reports the unminimized program);
//! - `HALE_PARITY_STATS=1` prints what each mutator produced (applied,
//!   checked clean, built, diverged).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "parity/gate.rs"]
mod gate;
#[path = "parity/minimize.rs"]
mod minimize;
#[path = "parity/mutators.rs"]
mod mutators;

use gate::{Files, Verdict};
use mutators::{Rng, MUTATORS};

// --- seeds -----------------------------------------------------------

/// A program the mutators start from.
#[derive(Clone)]
struct Seed {
    origin: String,
    source: String,
}

/// Whether a program can seed candidates: one file (an import's other
/// half is not here), no `@ffi` host side, a native target, and an
/// entry point a build starts at.
fn seedable(source: &str) -> Option<String> {
    if source.contains("import \"") || source.contains("@ffi(") || source.contains("target wasm") {
        return None;
    }
    let program = hale_syntax::parse_source(source).ok()?;
    let has_fn_main = program
        .items
        .iter()
        .any(|i| matches!(i, hale_syntax::ast::TopDecl::Fn(f) if f.name.name == "main"));
    let has_main_locus = program
        .items
        .iter()
        .any(|i| matches!(i, hale_syntax::ast::TopDecl::Locus(l) if l.is_main));
    if has_fn_main {
        Some(source.to_string())
    } else if has_main_locus {
        // a library application seed: started by an importer
        None
    } else {
        // GH #829's synthetic entry point (corpus_check_build_agreement)
        Some(format!("{}\nfn main() {{ }}\n", source.trim_end()))
    }
}

fn repo() -> PathBuf {
    hale_corpus::repo_root()
}

/// The `tests/hale` programs: one file each, the `*_lib` directories
/// left out (they are imported halves).
fn tests_hale() -> Vec<Seed> {
    let dir = repo().join("tests/hale");
    hale_corpus::source_hl(&dir)
        .into_iter()
        .filter(|p| p.parent() == Some(dir.as_path()))
        .filter_map(|p| {
            let source = std::fs::read_to_string(&p).ok()?;
            Some(Seed { origin: format!("tests/hale/{}", p.file_name()?.to_string_lossy()), source })
        })
        .collect()
}

/// The `hale` code blocks of the book and the spec that are whole
/// programs (`docs_snippets.rs`'s extraction, minus `fragment` blocks).
fn doc_snippets() -> Vec<Seed> {
    let mut out = Vec::new();
    for root in ["docs/src", "spec"] {
        let dir = repo().join(root);
        for md in hale_corpus::source_files(&dir).into_iter().filter(|p| p.extension().is_some_and(|e| e == "md")) {
            let Ok(text) = std::fs::read_to_string(&md) else { continue };
            let rel = md.strip_prefix(repo()).unwrap_or(&md).display().to_string();
            let mut body: Option<(usize, String)> = None;
            for (i, line) in text.lines().enumerate() {
                let t = line.trim_start();
                match &mut body {
                    None => {
                        if let Some(info) = t.strip_prefix("```") {
                            let info = info.trim();
                            if (info == "hale" || info.starts_with("hale,")) && !info.contains("fragment") {
                                body = Some((i + 1, String::new()));
                            }
                        }
                    }
                    Some((start, b)) => {
                        if t.starts_with("```") {
                            out.push(Seed { origin: format!("{rel}:{start}"), source: std::mem::take(b) });
                            body = None;
                        } else {
                            b.push_str(line);
                            b.push('\n');
                        }
                    }
                }
            }
        }
    }
    out
}

/// Prepare and deduplicate: seedable programs, by text, in a stable
/// order (the text's hash, so a new program does not reorder the rest).
fn prepare(raw: Vec<Seed>) -> Vec<Seed> {
    let mut seen = BTreeSet::new();
    let mut out: Vec<Seed> = raw
        .into_iter()
        .filter(|s| !s.origin.contains("hale-stdlib/") && !s.origin.contains("runtime/stdlib"))
        .filter_map(|s| Some(Seed { source: seedable(&s.source)?, origin: s.origin }))
        .filter(|s| seen.insert(s.source.clone()))
        .collect();
    out.sort_by_key(|s| (mutators::fnv(&s.source), s.origin.clone()));
    out
}

/// The sample's seeds: the example fixtures and `tests/hale` — files on
/// disk, which change less often than the embedded programs, so a pull
/// request meets the gate's verdict on its own change.
fn sample_seeds() -> Vec<Seed> {
    let examples = repo().join("crates/hale-codegen/tests/fixtures/examples");
    let mut raw: Vec<Seed> = hale_corpus::source_hl(&examples)
        .into_iter()
        .filter_map(|p| {
            let source = std::fs::read_to_string(&p).ok()?;
            let origin = p.strip_prefix(repo()).unwrap_or(&p).display().to_string();
            Some(Seed { origin, source })
        })
        .collect();
    raw.extend(tests_hale());
    prepare(raw)
}

/// Every seed: the whole corpus (fixtures and embedded programs),
/// `tests/hale`, and the docs' and spec's snippets.
fn all_seeds() -> Vec<Seed> {
    let mut raw: Vec<Seed> = hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok())
        .into_iter()
        .map(|p| Seed { origin: p.origin, source: p.source })
        .collect();
    raw.extend(tests_hale());
    raw.extend(doc_snippets());
    prepare(raw)
}

// --- candidates ------------------------------------------------------

#[derive(Clone)]
struct Candidate {
    seed: usize,
    chain: Vec<String>,
    mutators: Vec<&'static str>,
    files: Files,
}

fn seed_files(seed: &Seed) -> Files {
    std::iter::once((mutators::MAIN.to_string(), seed.source.clone())).collect()
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.parse().ok()
}

fn salt() -> u64 {
    std::env::var("HALE_PARITY_SALT").ok().and_then(|v| v.parse().ok()).unwrap_or(0)
}

fn only() -> Option<String> {
    std::env::var("HALE_PARITY_ONLY").ok().filter(|s| !s.is_empty())
}

/// How much of the cross product a run draws, in parts per thousand: each
/// (seed, mutator) pair is a single mutation with probability
/// `single`/1000, and each (seed, first, second) triple a chain of two
/// with probability `chain`/1000.
#[derive(Clone, Copy)]
struct Draw {
    single: u64,
    chain: u64,
}

/// One 64-bit key from several: SplitMix64 over each word in turn, so
/// every word moves every bit of the result.
fn key(words: &[u64]) -> u64 {
    words.iter().fold(0x243F_6A88_85A3_08D3, |h, w| Rng::new(h ^ w).next_u64())
}

/// Whether a pair or triple whose key is `k` is in the draw.
fn drawn(k: u64, per_mille: u64) -> bool {
    // the high bits: `Rng::new(k)` below starts from the low ones
    (k >> 32) % 1000 < per_mille
}

/// A seed's candidates. Every (seed, mutator) pair and every (seed,
/// first, second) triple is drawn or not on its OWN key — the seed's
/// text, the mutators' names and the salt — and the mutation it draws
/// runs on an RNG seeded by that key. Nothing about a candidate depends
/// on which other mutators exist or in what order `MUTATORS` lists them:
/// adding a mutator adds its own candidates and moves no one else's
/// (`a_new_mutator_moves_no_other_candidate` holds that). A drawn pair
/// whose mutator finds no site yields nothing — it does not fall through
/// to another mutator, which is what would couple them.
fn candidates_of(index: usize, seed: &Seed, pool: &[&mutators::Mutator], draw: Draw) -> Vec<Candidate> {
    let base = key(&[mutators::fnv(&seed.source), salt()]);
    let start = seed_files(seed);
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut push = |files: Files, chain: Vec<String>, names: Vec<&'static str>| {
        if files != start && seen.insert(files.clone()) {
            out.push(Candidate { seed: index, chain, mutators: names, files });
        }
    };
    for m in pool {
        let k = key(&[base, mutators::fnv(m.name)]);
        if !drawn(k, draw.single) {
            continue;
        }
        if let Some((files, desc)) = (m.apply)(&start, &mut Rng::new(k)) {
            push(files, vec![desc], vec![m.name]);
        }
    }
    for first in pool {
        for second in pool {
            let k = key(&[base, mutators::fnv(first.name), mutators::fnv(second.name)]);
            if !drawn(k, draw.chain) {
                continue;
            }
            let mut rng = Rng::new(k);
            let Some((files, a)) = (first.apply)(&start, &mut rng) else { continue };
            let Some((files, b)) = (second.apply)(&files, &mut rng) else { continue };
            push(files, vec![a, b], vec![first.name, second.name]);
        }
    }
    out
}

/// The mutators a run draws from: every one, or `HALE_PARITY_ONLY`'s.
fn pool() -> Vec<&'static mutators::Mutator> {
    match only() {
        Some(name) => MUTATORS.iter().filter(|m| m.name == name).collect(),
        None => MUTATORS.iter().collect(),
    }
}

// --- the run ---------------------------------------------------------

fn threads() -> usize {
    env_usize("HALE_CORPUS_SWEEP_THREADS")
        .filter(|n| *n > 0)
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get().min(8)).unwrap_or(4))
}

/// Run `f` over `items` on the worker pool, results in input order.
fn pooled<T: Sync, R: Send>(items: &[T], f: impl Fn(usize, &T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let workers = threads().min(items.len().max(1));
    let mut all: Vec<(usize, R)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                let (next, f) = (&next, &f);
                scope.spawn(move || {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        mine.push((i, f(i, item)));
                    }
                    mine
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e))).collect()
    });
    all.sort_by_key(|(i, _)| *i);
    all.into_iter().map(|(_, r)| r).collect()
}

/// A divergence the run found, with what reproduces it.
struct Found {
    signature: String,
    message: String,
    seed: String,
    chain: Vec<String>,
    files: Files,
    /// How many candidates reached it.
    hits: usize,
    /// The mutators that reached it (each candidate's first).
    via: BTreeSet<&'static str>,
}

#[derive(Default)]
struct Stats {
    applied: usize,
    clean: usize,
    built: usize,
    diverged: usize,
}

struct Report {
    candidates: usize,
    clean: usize,
    built: usize,
    /// GH #2134: candidates the build refused only by a target-dependent
    /// `@vectorize` assertion ([`Verdict::TargetAssertion`]).
    target_assertions: usize,
    /// By signature, in signature order.
    found: BTreeMap<String, Found>,
    /// Divergences the unmutated seed already has (the agreement
    /// sweep's), by signature.
    seed_owned: BTreeSet<String>,
    stats: BTreeMap<&'static str, Stats>,
}

fn run(seeds: &[Seed], draw: Draw) -> Report {
    let pool = pool();
    let candidates: Vec<Candidate> =
        seeds.iter().enumerate().flat_map(|(i, s)| candidates_of(i, s, &pool, draw)).collect();
    let verdicts: Vec<Verdict> = pooled(&candidates, |i, c| gate::verdict(&c.files, &format!("hale_parity_{i}")));
    let mut report = Report {
        candidates: candidates.len(),
        clean: 0,
        built: 0,
        target_assertions: 0,
        found: BTreeMap::new(),
        seed_owned: BTreeSet::new(),
        stats: BTreeMap::new(),
    };
    // A seed's own verdict, asked only of a seed one of its candidates
    // diverged from.
    let divergent_seeds: Vec<usize> = candidates
        .iter()
        .zip(&verdicts)
        .filter(|(_, v)| matches!(v, Verdict::Diverges(_)))
        .map(|(c, _)| c.seed)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let own = pooled(&divergent_seeds, |_, s| gate::verdict(&seed_files(&seeds[*s]), &format!("hale_parity_seed_{s}")));
    let seed_verdicts: BTreeMap<usize, Verdict> = divergent_seeds.iter().copied().zip(own).collect();
    for (c, v) in candidates.iter().zip(&verdicts) {
        let st = report.stats.entry(c.mutators[0]).or_default();
        st.applied += 1;
        match v {
            Verdict::CheckRejects => {}
            // GH #2134: a target-dependent build assertion, counted apart.
            Verdict::TargetAssertion(_) => {
                st.clean += 1;
                report.clean += 1;
                report.target_assertions += 1;
            }
            Verdict::Built => {
                st.clean += 1;
                st.built += 1;
                report.clean += 1;
                report.built += 1;
            }
            Verdict::Diverges(msg) => {
                st.clean += 1;
                report.clean += 1;
                let sig = gate::signature(msg);
                if let Ok(dir) = std::env::var("HALE_PARITY_DUMP") {
                    let name = format!("{:016x}.repro", mutators::fnv(&render_repro(&c.files)));
                    let head = format!("// {}\n// seed: {}\n// mutations: {}\n", sig, seeds[c.seed].origin, c.chain.join(" > "));
                    let _ = std::fs::create_dir_all(&dir);
                    let _ = std::fs::write(Path::new(&dir).join(name), head + &render_repro(&c.files));
                }
                if let Some(Verdict::Diverges(own)) = seed_verdicts.get(&c.seed) {
                    if gate::signature(own) == sig {
                        report.seed_owned.insert(sig);
                        continue;
                    }
                }
                st.diverged += 1;
                report
                    .found
                    .entry(sig.clone())
                    .and_modify(|f| {
                        f.hits += 1;
                        f.via.extend(c.mutators.iter().copied());
                    })
                    .or_insert_with(|| Found {
                        signature: sig,
                        message: msg.clone(),
                        seed: seeds[c.seed].origin.clone(),
                        chain: c.chain.clone(),
                        files: c.files.clone(),
                        hits: 1,
                        via: c.mutators.iter().copied().collect(),
                    });
            }
        }
    }
    report
}

// --- the ratchet -----------------------------------------------------

fn parity_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/parity")
}

/// One line of `divergences.txt`.
struct Listed {
    issue: String,
    repro: String,
    signature: String,
}

fn listed() -> Vec<Listed> {
    let text = std::fs::read_to_string(parity_dir().join("divergences.txt")).expect("tests/fixtures/parity/divergences.txt");
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .map(|l| {
            let parts: Vec<&str> = l.splitn(3, " | ").collect();
            assert_eq!(parts.len(), 3, "a divergences.txt line is `#<issue> | <reproducer> | <signature>`: {l:?}");
            assert!(parts[0].starts_with('#'), "a listed divergence names its issue, `#<number>`: {l:?}");
            Listed { issue: parts[0].to_string(), repro: parts[1].to_string(), signature: parts[2].to_string() }
        })
        .collect()
}

/// A listed reproducer's files.
fn read_repro(name: &str) -> Files {
    read_repro_at(&parity_dir().join(name))
}

/// A reproducer's files: each under a `// == <path> ==` line (lines
/// before the first are comments: what it reproduces).
fn read_repro_at(path: &Path) -> Files {
    let name = path.display();
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reproducer {name}: {e}"));
    let mut files = Files::new();
    let mut cur: Option<String> = None;
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("// == ").and_then(|l| l.strip_suffix(" ==")) {
            cur = Some(path.to_string());
            files.insert(path.to_string(), String::new());
        } else if let Some(p) = &cur {
            let f = files.get_mut(p).unwrap();
            f.push_str(line);
            f.push('\n');
        }
    }
    assert!(files.contains_key(mutators::MAIN), "reproducer {name} has no `// == {} ==` section", mutators::MAIN);
    files
}

fn render_repro(files: &Files) -> String {
    let mut out = String::new();
    for (path, src) in files {
        out.push_str(&format!("// == {path} ==\n{}", src.trim_end()));
        out.push('\n');
    }
    out
}

/// The minimized program for a divergence: lines removed while the
/// candidate still checks clean and still diverges with this signature.
fn minimized(files: &Files, signature: &str, budget: usize) -> Files {
    let n = AtomicUsize::new(0);
    let still = |f: &Files| match gate::verdict(f, &format!("hale_parity_min_{}", n.fetch_add(1, Ordering::Relaxed))) {
        Verdict::Diverges(m) => gate::signature(&m) == signature,
        _ => false,
    };
    minimize::minimize(files, &still, budget).0
}

/// Hold a run's divergences to the list: every one must be listed.
fn judge(report: &Report, minimize_budget: usize) -> Result<(), String> {
    let known: BTreeSet<String> = listed().into_iter().map(|l| l.signature).collect();
    let new: Vec<&Found> = report.found.values().filter(|f| !known.contains(&f.signature)).collect();
    if new.is_empty() {
        return Ok(());
    }
    let mut msg = format!(
        "\n{} NEW check/build divergence(s): `hale check` accepts a program `hale build` refuses. Fix the \
         build, or refuse the program at check with a span; a gap that needs its own change is listed in \
         tests/fixtures/parity/divergences.txt with its issue and the reproducer below (as `<name>.repro`).\n",
        new.len()
    );
    // minimize in parallel, one divergence per worker
    let budget = env_usize("HALE_PARITY_MIN_BUDGET").unwrap_or(minimize_budget);
    let small = pooled(&new, |_, f| if budget == 0 { f.files.clone() } else { minimized(&f.files, &f.signature, budget) });
    for (f, small) in new.iter().zip(small) {
        msg.push_str(&format!(
            "\n=== {}\n  seed:      {}\n  mutations: {}\n  reached by {} candidate(s)\n  refusal:   {}\n  minimized:\n{}\n",
            f.signature,
            f.seed,
            f.chain.join(" > "),
            f.hits,
            f.message.lines().next().unwrap_or(""),
            render_repro(&small).lines().map(|l| format!("    {l}")).collect::<Vec<_>>().join("\n")
        ));
    }
    Err(msg)
}

fn print_summary(what: &str, report: &Report, took: std::time::Duration) {
    eprintln!(
        "{what}: {} candidates, {} checked clean, {} built, {} target-dependent `@vectorize` refusal(s), \
         {} divergence signature(s), {} seed-owned, in {:.1?}",
        report.candidates,
        report.clean,
        report.built,
        report.target_assertions,
        report.found.len(),
        report.seed_owned.len(),
        took
    );
    for f in report.found.values() {
        eprintln!("  [{}x] {}\n        first: {} : {}\n        via: {:?}", f.hits, f.signature, f.seed, f.chain.join(" > "), f.via);
    }
    if std::env::var("HALE_PARITY_STATS").as_deref() == Ok("1") {
        eprintln!("  {:<20} {:>8} {:>8} {:>8} {:>8}", "mutator", "applied", "clean", "built", "diverged");
        for (name, s) in &report.stats {
            eprintln!("  {:<20} {:>8} {:>8} {:>8} {:>8}", name, s.applied, s.clean, s.built, s.diverged);
        }
    }
}

// --- the tests -------------------------------------------------------

/// The sample's seeds.
const SAMPLE_SEEDS: usize = 16;

/// The draw the sample and the sweep make: about 30% of the (seed,
/// mutator) pairs and 1.5% of the (seed, first, second) triples, which
/// over the whole corpus is about as many candidates (two singles to a
/// chain) as the sanitizer job has had time for. A sweep run takes
/// `HALE_PARITY_SINGLE` / `HALE_PARITY_CHAIN` (parts per thousand) over it.
const DRAW: Draw = Draw { single: 300, chain: 15 };

fn sweep_draw() -> Draw {
    Draw {
        single: env_usize("HALE_PARITY_SINGLE").map_or(DRAW.single, |n| n as u64),
        chain: env_usize("HALE_PARITY_CHAIN").map_or(DRAW.chain, |n| n as u64),
    }
}

/// The sample: a fixed, deterministic slice of the on-disk corpus,
/// mutated and built — seconds, in the ordinary codegen suite.
#[test]
fn sample_of_mutated_programs_builds() {
    let t = Instant::now();
    let seeds: Vec<Seed> = sample_seeds().into_iter().take(SAMPLE_SEEDS).collect();
    assert!(seeds.len() == SAMPLE_SEEDS, "only {} sample seeds: the corpus walk is broken", seeds.len());
    let report = run(&seeds, DRAW);
    print_summary("parity sample", &report, t.elapsed());
    assert!(report.clean > 0, "no sample candidate checked clean: the mutators are broken, not the compiler");
    if let Err(msg) = judge(&report, 150) {
        panic!("{msg}");
    }
}

/// The ratchet's other half: every listed divergence still diverges —
/// its reproducer checks clean, and the build refuses it with the listed
/// signature. When a fix closes one, this fails until its line and its
/// reproducer are deleted, so the list can only shrink.
#[test]
fn listed_divergences_still_reproduce() {
    let entries = listed();
    let mut wrong = Vec::new();
    let mut repros = BTreeSet::new();
    for (i, l) in entries.iter().enumerate() {
        repros.insert(l.repro.clone());
        let files = read_repro(&l.repro);
        match gate::verdict(&files, &format!("hale_parity_listed_{i}")) {
            Verdict::Diverges(m) if gate::signature(&m) == l.signature => {}
            Verdict::Diverges(m) => wrong.push(format!(
                "{} ({}): diverges, but as `{}`, not the listed `{}`",
                l.repro,
                l.issue,
                gate::signature(&m),
                l.signature
            )),
            Verdict::TargetAssertion(m) => wrong.push(format!(
                "{} ({}): a target-dependent `@vectorize` refusal, not the listed divergence: {m}",
                l.repro, l.issue
            )),
            Verdict::Built => wrong.push(format!(
                "{} ({}): BUILDS now — the gap is closed; delete its line from divergences.txt and the reproducer",
                l.repro, l.issue
            )),
            Verdict::CheckRejects => wrong.push(format!(
                "{} ({}): the check refuses it now — if the check refuses it with a span, the divergence is closed; \
                 delete its line and the reproducer",
                l.repro, l.issue
            )),
        }
    }
    // every reproducer on disk is listed: one left behind is a fixed gap
    // whose line went and whose program did not
    let on_disk: BTreeSet<String> = std::fs::read_dir(parity_dir())
        .expect("tests/fixtures/parity")
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .filter(|n| n.ends_with(".repro"))
        .collect();
    for stray in on_disk.difference(&repros) {
        wrong.push(format!("{stray}: a reproducer no line of divergences.txt names"));
    }
    assert!(wrong.is_empty(), "the parity ratchet:\n  {}", wrong.join("\n  "));
}

/// Every mutator reaches a check-clean program somewhere in the sample's
/// seeds: one that only ever writes programs the check refuses tests
/// nothing, and would pass forever. Checks only; nothing is built.
#[test]
fn mutators_reach_check_clean_programs() {
    let seeds = sample_seeds();
    let names: Vec<&'static str> = MUTATORS.iter().map(|m| m.name).collect();
    let reached = pooled(&names, |_, name| {
        // Each mutator over the seeds in order, until one candidate checks clean.
        for (si, seed) in seeds.iter().enumerate() {
            let m = MUTATORS.iter().find(|m| m.name == *name).unwrap();
            for k in 0..2u64 {
                let mut rng = Rng::new(mutators::fnv(&seed.source) ^ k);
                if let Some((files, _)) = (m.apply)(&seed_files(seed), &mut rng) {
                    if std::panic::catch_unwind(|| gate::check_clean(&files)).unwrap_or(false) {
                        return Some(si);
                    }
                }
            }
        }
        None
    });
    let dead: Vec<&&str> = names.iter().zip(&reached).filter(|(_, r)| r.is_none()).map(|(n, _)| n).collect();
    assert!(dead.is_empty(), "mutators that produced no check-clean program over {} seeds: {dead:?}", seeds.len());
}

/// GH #2134 (F.62): a check-clean program whose `@vectorize` loop the
/// build's optimizer does not vectorize is the target's answer, counted
/// as a target assertion, never a divergence.
#[test]
fn a_vectorize_refusal_is_a_target_assertion_not_a_divergence() {
    let src = "@vectorize\nfn until(xs: [Int; 64], limit: Int) -> Int {\n    let mut s = 0;\n    let mut i = 0;\n    \
               while s < limit {\n        s = s + xs[i % 64];\n        i = i + 1;\n    }\n    return i;\n}\n\n\
               fn main() {\n    println(until([3; 64], 500));\n}\n";
    let files: Files = std::iter::once(("main/main.hl".to_string(), src.to_string())).collect();
    match gate::verdict(&files, "pg_vectorize") {
        Verdict::TargetAssertion(m) => assert!(m.contains("this loop does not vectorize on"), "{m}"),
        other => panic!("expected a target assertion, got {other:?}"),
    }
}

/// GH #2108: `entry_shape`'s two shapes over a seed whose `main locus` its
/// `fn main` starts. Dropping `fn main` leaves a `main locus` nothing
/// starts, which the build refuses at the locus's name: the check refuses
/// it too, so the candidate is the check's to reject and never a
/// divergence. Moving the body into a helper keeps a program that builds.
#[test]
fn entry_shape_drops_main_into_a_check_refusal_and_keeps_the_helper_built() {
    let src = "main locus App {\n    run() {\n        println(\"hi\");\n    }\n}\n\nfn main() {\n    App { };\n}\n";
    let start: Files = std::iter::once((mutators::MAIN.to_string(), src.to_string())).collect();
    let m = MUTATORS.iter().find(|m| m.name == "entry_shape").expect("the mutator is listed");
    let mut seen = BTreeMap::new();
    for k in 0..64u64 {
        if let Some((files, desc)) = (m.apply)(&start, &mut Rng::new(k)) {
            seen.entry(desc).or_insert(files);
        }
    }
    let dropped = seen.get("entry_shape(drop)").expect("a draw drops `fn main`");
    assert!(!dropped[mutators::MAIN].contains("fn main"), "{}", dropped[mutators::MAIN]);
    assert_eq!(gate::verdict(dropped, "pg_entry_drop"), Verdict::CheckRejects);
    let helper = seen.get("entry_shape(helper)").expect("a draw moves the body to a helper");
    assert_eq!(gate::verdict(helper, "pg_entry_helper"), Verdict::Built);
}

/// The minimizer keeps what the predicate needs and drops the rest.
#[test]
fn the_minimizer_shrinks_to_what_the_predicate_needs() {
    let src = "fn a() -> Int {\n    return 1;\n}\n\nfn b() -> Int {\n    return 2;\n}\n\nfn main() {\n    let x = a();\n    let y = b();\n}\n";
    let files: Files = std::iter::once((mutators::MAIN.to_string(), src.to_string())).collect();
    let still = |f: &Files| f[mutators::MAIN].contains("return 2;") && f[mutators::MAIN].contains("fn main");
    let (small, asked) = minimize::minimize(&files, &still, 500);
    let out = &small[mutators::MAIN];
    assert!(out.contains("return 2;") && out.contains("fn main"), "{out}");
    assert!(!out.contains("return 1;"), "the minimizer left a line the predicate does not need:\n{out}");
    assert!(asked < 100, "asked {asked} times for a 12-line program");
}

/// The draw is per mutator: taking any one mutator out of the pool (as
/// adding one puts it in) or reordering the pool leaves every other
/// mutator's candidates as they were. Before this held, adding a mutator
/// re-dealt every seed's mutations, and the gate's coverage moved with
/// the length of `MUTATORS` (a new mutator surfaced 8 old gaps in other
/// mutators' shapes, and dropping it hid them again).
#[test]
fn a_new_mutator_moves_no_other_candidate() {
    let seeds: Vec<Seed> = sample_seeds().into_iter().take(SAMPLE_SEEDS).collect();
    let full: Vec<&mutators::Mutator> = MUTATORS.iter().collect();
    let programs = |pool: &[&mutators::Mutator], skip: Option<&str>| -> BTreeSet<Files> {
        seeds
            .iter()
            .enumerate()
            .flat_map(|(i, s)| candidates_of(i, s, pool, DRAW))
            .filter(|c| skip.is_none_or(|x| !c.mutators.contains(&x)))
            .map(|c| c.files)
            .collect()
    };
    let everything = programs(&full, None);
    assert!(!everything.is_empty(), "the sample draws no candidate");
    let mut reversed = full.clone();
    reversed.reverse();
    assert_eq!(programs(&reversed, None), everything, "reordering MUTATORS changed the draw");
    for m in &full {
        let without: Vec<&mutators::Mutator> = full.iter().copied().filter(|x| x.name != m.name).collect();
        let rest = programs(&without, None);
        assert!(rest.is_subset(&everything), "taking `{}` out of the pool drew a program the full pool does not", m.name);
        assert!(
            programs(&full, Some(m.name)).is_subset(&rest),
            "taking `{}` out of the pool moved another mutator's candidates",
            m.name
        );
    }
}

/// The signature leaves out what a mutation varies.
#[test]
fn a_signature_drops_names_and_numbers() {
    assert_eq!(
        gate::signature("main.hl:12:5: cannot lower `__pg_id` for `Foo` at 3\nmore"),
        gate::signature("main.hl:40:1: cannot lower `bar` for `Baz` at 17"),
    );
    assert_eq!(
        gate::signature("`or` substitute type mismatch: expected std::process::ProcessOutput, got Bool"),
        gate::signature("`or` substitute type mismatch: expected Rec, got Decimal"),
    );
    assert_eq!(
        gate::signature("call to `x`: none in scope — did you mean `y`? (in the standard library, core.hl:3:1)"),
        gate::signature("call to `z`: none in scope"),
    );
    assert_ne!(gate::signature("cannot lower a `x`"), gate::signature("cannot emit a `x`"));
}

/// The sweep: every seed of the corpus, more candidates each. CI's
/// sanitizer job runs it beside the agreement sweep.
#[test]
#[ignore = "builds thousands of mutated programs; run explicitly (CI's sanitizer job)"]
fn sweep_of_mutated_programs_builds() {
    let t = Instant::now();
    let mut seeds = all_seeds();
    if let Some(n) = env_usize("HALE_PARITY_SEEDS") {
        seeds.truncate(n);
    }
    let report = run(&seeds, sweep_draw());
    print_summary(&format!("parity sweep over {} seeds", seeds.len()), &report, t.elapsed());
    assert!(report.clean > 200, "only {} candidates checked clean: the sweep is broken, not the compiler", report.clean);
    if let Err(msg) = judge(&report, 300) {
        panic!("{msg}");
    }
}


/// A developer's tool, not a gate: minimize one `.repro` (a dumped
/// candidate, `HALE_PARITY_DUMP`, or a listed reproducer) under the
/// divergence it shows now, and print the result.
///
/// ```sh
/// HALE_PARITY_REPRO=path/to/x.repro cargo test --release -p hale-codegen \
///     --test parity_gate -- minimize_a_reproducer --ignored --nocapture
/// ```
#[test]
#[ignore = "a tool: set HALE_PARITY_REPRO to a .repro file"]
fn minimize_a_reproducer() {
    let Ok(path) = std::env::var("HALE_PARITY_REPRO") else {
        eprintln!("set HALE_PARITY_REPRO to a .repro file");
        return;
    };
    let files = read_repro_at(Path::new(&path));
    let Verdict::Diverges(msg) = gate::verdict(&files, "hale_parity_tool") else {
        panic!("{path} does not diverge now (it builds, or the check refuses it)");
    };
    let sig = gate::signature(&msg);
    let small = minimized(&files, &sig, env_usize("HALE_PARITY_MIN_BUDGET").unwrap_or(400));
    eprintln!("refusal:   {}\nsignature: {sig}\n{}", msg.lines().next().unwrap_or(""), render_repro(&small));
}
