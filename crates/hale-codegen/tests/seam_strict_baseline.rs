//! GH #2111 (phase 1): lowering takes every value's type from the checker.
//!
//! Lowering reads a value's type from the checker through one seam
//! (`Cx::ty_at`, `Cx::typed_by_row`): every expression `lower_expr`
//! lowers hands back the checker's row of it, and a value of another type
//! is a compiler defect, refused at its span. There is no other path: a
//! value with no row (none recorded, `Unknown`, a type with no layout,
//! lowered outside every body) is a located internal error, the build
//! refused when lowering ends (`Cx::refuse_missing_rows`). Phase 1 ended
//! in that deletion: strict mode was the only mode left, its knob
//! (`BuildOptions::seam_strict`) went, and with it every fallback site.
//!
//! This test holds two things:
//!
//! - the survey's static measures of how much typing lowering still does
//!   itself, read from codegen's source and held to
//!   `fixtures/seam_strict_baseline.txt`: a measure that moves the wrong way
//!   fails (a change that has to add lowering, a new type or intrinsic,
//!   regenerates the file with a one-line reason in its PR body), and the
//!   fallback sites are held at none;
//! - every corpus example, every `tests/hale` program, every `dna/tests`
//!   fixture (found as `hale test` finds them) and every program embedded in
//!   a Rust test that `corpus_check_build_agreement`'s sweep builds lowers
//!   with every value typed by its row: none is refused for a missing row
//!   or a disagreement.
//!
//! ```text
//! cargo test --release -p hale-codegen --test seam_strict_baseline \
//!     --config 'env.HALE_SEAM_BASELINE_REGEN="1"'
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "../../hale-types/tests/support/entries.rs"]
mod entries;

use hale_codegen::{BuildOptions, CodegenError};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("the repository root")
}

fn baseline_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/seam_strict_baseline.txt")
}

/// A program the burn-down builds: a seed on disk, by its entry, or a
/// program embedded in a Rust test, parsed.
enum Source {
    Entry(PathBuf),
    Embedded(hale_syntax::ast::Program),
}

/// Every program the burn-down covers, by its name in the report: the
/// seeds on disk ([`seeds`]) and the programs embedded in the Rust tests
/// that `corpus_check_build_agreement`'s sweep builds ([`embedded`]).
fn programs() -> Vec<(String, Source)> {
    let mut out: Vec<(String, Source)> = seeds().into_iter().map(|(n, p)| (n, Source::Entry(p))).collect();
    out.extend(embedded().into_iter().map(|(n, p)| (n, Source::Embedded(p))));
    out
}

/// The programs embedded in the Rust tests (`hale_corpus`) that
/// `corpus_check_build_agreement`'s sweep builds, as it takes them: each
/// one the checker accepts, with `fn main() { }` added where it has
/// none; not one that imports a sibling seed (the harvester holds one
/// seed at a time), declares an `@ffi` host import (whose host side is
/// the test's) or selects wasm32. A stdlib module is no program: the
/// check refuses it, as it declares the stdlib's names a second time
/// (spec/semantics.md § One top-level scope). They reach code no seed on
/// disk does: stdlib bodies, synthesized defaults, generated handlers.
fn embedded() -> Vec<(String, hale_syntax::ast::Program)> {
    let mut out = Vec::new();
    for p in hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok()) {
        if p.source.contains("import \"") || p.source.contains("@ffi(") {
            continue;
        }
        let Ok(mut program) = hale_syntax::parse_source(&p.source) else { continue };
        if entries::check_program(&program).iter().any(|d| d.is_error()) {
            continue;
        }
        let mut bundle = BTreeMap::new();
        bundle.insert(String::new(), &program);
        if hale_types::capability::target_row(&hale_types::Bundle::new(bundle)).is_wasm32() {
            continue;
        }
        if !program.items.iter().any(|i| matches!(i, hale_syntax::ast::TopDecl::Fn(f) if f.name.name == "main")) {
            let stub = hale_syntax::parse_source("fn main() { }\n").expect("the synthetic entry point parses");
            program.items.extend(stub.items);
        }
        out.push((p.origin, program));
    }
    out
}

/// The seeds on disk the burn-down covers: (its name in the report, its
/// entry). A corpus example is its directory, a seed of every file in
/// it; the others are a file and the imports it follows.
fn seeds() -> Vec<(String, PathBuf)> {
    let root = repo_root();
    let mut out = Vec::new();
    let mut dir = |rel: &str, pick: &dyn Fn(&Path) -> bool| {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(root.join(rel))
            .unwrap_or_else(|e| panic!("{rel}: {e}"))
            .map(|e| e.expect("a directory entry").path())
            .filter(|p| pick(p))
            .collect();
        entries.sort();
        for p in entries {
            let name = p.strip_prefix(&root).expect("under the root").display().to_string();
            out.push((name, p));
        }
    };
    dir("crates/hale-codegen/tests/fixtures/examples", &|p| p.join("main.hl").is_file());
    for rel in ["tests/hale", "dna/tests"] {
        let mut files = Vec::new();
        test_files(&root.join(rel), &mut files);
        for p in files {
            let name = p.strip_prefix(&root).expect("under the root").display().to_string();
            out.push((name, p));
        }
    }
    out
}

/// Every `*_test.hl` under `dir`, as `hale test <dir>` finds them
/// (`collect_test_files`): depth-first in sorted order, skipping `vendor/`
/// and dot-directories.
fn test_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.expect("a directory entry").path())
        .collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.is_dir() {
            if name != "vendor" && !name.starts_with('.') {
                test_files(&p, out);
            }
        } else if name.ends_with("_test.hl") {
            out.push(p);
        }
    }
}

/// Whether `e` refuses a value whose type lowering could not take from
/// the checker: a missing row, or a value of another type.
fn untyped(e: &CodegenError) -> bool {
    let msg = e.to_string();
    msg.contains("required `expression_typing` row") || msg.contains("and lowering built")
}

/// Lower `source` as `hale build` loads it, lowering only: `Ok` when every
/// value has its row, or the refusal. An embedded program lowering
/// refuses for another reason is `corpus_check_build_agreement`'s, not
/// this test's: `Ok`.
fn lowers_typed(source: &Source, options: &BuildOptions) -> Result<(), String> {
    use hale_frontend::frontend::LoadMode;
    use hale_frontend::snapshot::{LoadError, Snapshot};
    let out = harness::unique_bin("seam_rows");
    let r = match source {
        Source::Embedded(program) => match build_opts::build_program(program, &out, &[], options) {
            Err(e) if !untyped(&e) => Ok(()),
            other => other.map_err(|e| e.to_string()),
        },
        Source::Entry(entry) => {
            match Snapshot::load(entry, LoadMode::WholeSeed, &hale_frontend::source::Disk, build_opts::harness_config(options)) {
                Ok(snap) => match snap.demand_lowering() {
                    Ok(view) => hale_codegen::build_resolved(view, &out, options).map_err(|e| e.to_string()),
                    Err(b) => Err(format!("{:?}", b.refused)),
                },
                Err(LoadError::Refused(msg)) => Err(msg),
                Err(LoadError::Load(f)) => Err(f.text()),
            }
        }
    };
    let _ = std::fs::remove_file(&out);
    r
}

fn codegen_sources() -> BTreeMap<PathBuf, String> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for e in std::fs::read_dir(dir).expect("the source directory") {
            let p = e.expect("an entry").path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.insert(p.clone(), std::fs::read_to_string(&p).expect("a source file"));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out
}

/// The survey's measures of how much typing lowering does itself (GH
/// #2111), from the source, each with the direction it may move:
///
/// - `static typers`: fns whose signature names a `CodegenTy` and no IR
///   value (they type an expression without lowering it), outside the
///   seam (`typed_seam.rs`, which reads the checker's answer);
/// - `value+type lowerings`: fns that hand back an IR value and its
///   `CodegenTy`;
/// - `CodegenTy built in an expression`: lines that build a `CodegenTy`
///   variant, not match one;
/// - `seam fallbacks (static)`: the sites that took lowering's own path
///   where the checker's row was missing (`Cx::seam_fallback`), which phase
///   1 deleted: held at 0;
///
/// all of which may only go down; and `seam reads`, the reads of the
/// checker's type outside the seam itself (`Cx::ty_at`, `Cx::row_of`,
/// `Cx::typed_by_row`, `Cx::call_result`, `Cx::ty_required`,
/// `Cx::place_ty`), which may only go up.
fn static_measures() -> Vec<(&'static str, usize, bool)> {
    let sources = codegen_sources();
    let (mut typers, mut lowerings, mut built, mut fallbacks, mut reads) = (0, 0, 0, 0, 0);
    for (path, text) in &sources {
        let seam = path.ends_with("typed_seam.rs");
        // a fn's signature: from `fn name(` to its body or `;`
        let bytes = text.as_bytes();
        let mut i = 0;
        while let Some(at) = text[i..].find("fn ") {
            let start = i + at;
            i = start + 3;
            if start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
                continue;
            }
            let rest = &text[i..];
            let name_len = rest.bytes().take_while(|b| b.is_ascii_alphanumeric() || *b == b'_').count();
            if name_len == 0 {
                continue;
            }
            let mut j = i + name_len;
            if text[j..].starts_with('<') {
                let Some(close) = text[j..].find('>') else { continue };
                j += close + 1;
            }
            if !text[j..].starts_with('(') {
                continue;
            }
            let mut depth = 0usize;
            let mut k = j;
            for (off, c) in text[j..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            k = j + off + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let end = [text[k..].find('{'), text[k..].find(';')].into_iter().flatten().min().map_or(text.len(), |e| k + e);
            let ret = &text[k..end];
            // the seam's own fns read the checker's answer: no typer
            if !seam && ret.contains("CodegenTy") {
                if ["BasicValueEnum", "PointerValue", "IntValue"].iter().any(|v| ret.contains(v)) {
                    lowerings += 1;
                } else {
                    typers += 1;
                }
            }
        }
        for line in text.lines() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            let builds = code.match_indices("CodegenTy::").any(|(at, _)| code[at + 11..].starts_with(|c: char| c.is_ascii_uppercase()));
            let matches = code.contains("=>")
                || code.contains("matches!")
                || code.contains("if let")
                || code.contains("while let")
                || code.contains("let Some(CodegenTy")
                || code.contains("let CodegenTy");
            if builds && !matches {
                built += 1;
            }
            if !seam {
                fallbacks += code.matches(".seam_fallback(").count();
                reads += [".ty_at(", ".row_of(", ".typed_by_row(", ".call_result(", ".ty_required(", ".place_ty(", ".row_is_unit("].iter().map(|r| code.matches(r).count()).sum::<usize>();
            }
        }
    }
    vec![
        ("static typers", typers, false),
        ("value+type lowerings", lowerings, false),
        ("CodegenTy built in an expression", built, false),
        ("seam fallbacks (static)", fallbacks, false),
        ("seam reads", reads, true),
    ]
}


fn render(measures: &[(&'static str, usize, bool)]) -> String {
    let mut s = String::from(
        "# Generated by `seam_strict_baseline.rs` under HALE_SEAM_BASELINE_REGEN=1 (GH #2111).\n\
         # The survey's measures of crates/hale-codegen/src: `seam reads` may only go up, every\n\
         # other measure only down, and `seam fallbacks (static)` is held at 0. Phase 1 ended with\n\
         # every value's type the checker's: no fallback, and a missing row a located internal error.\n",
    );
    for (name, n, _) in measures {
        s.push_str(&format!("measure\t{name}\t{n}\n"));
    }
    s
}

/// A baseline's measures by name.
fn parse(text: &str) -> BTreeMap<String, usize> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let ["measure", name, n] = f.as_slice() else { panic!("not a baseline line: {l:?}") };
            (name.to_string(), n.parse().expect("a count"))
        })
        .collect()
}

#[test]
fn lowering_types_no_more_than_the_baseline() {
    let measures = static_measures();
    let fallbacks = measures.iter().find(|(n, _, _)| *n == "seam fallbacks (static)").map(|(_, n, _)| *n);
    assert_eq!(fallbacks, Some(0), "a site takes lowering's own type where the checker's row is missing (GH #2111)");
    let now = render(&measures);
    let path = baseline_path();
    if std::env::var_os("HALE_SEAM_BASELINE_REGEN").is_some() {
        std::fs::write(&path, &now).expect("write the baseline");
        return;
    }
    let was = parse(&std::fs::read_to_string(&path).unwrap_or_default());
    let mut worse = Vec::new();
    let mut moved = Vec::new();
    for (name, n, up_is_better) in &measures {
        let a = was.get(*name).copied().unwrap_or(0);
        if a == *n {
            continue;
        }
        let line = format!("{name}: {a} -> {n}");
        if (*n > a) == *up_is_better {
            moved.push(line);
        } else {
            worse.push(line);
        }
    }
    let regen = "cargo test --release -p hale-codegen --test seam_strict_baseline \
                 --config 'env.HALE_SEAM_BASELINE_REGEN=\"1\"'";
    assert!(
        worse.is_empty(),
        "lowering types more itself than the baseline holds (GH #2111): record the type in the checker \
         and read it through the seam instead (a typer deleted by inlining it into its callers moves the \
         lowering numbers too). A change that has to add lowering (a new type, a new intrinsic) \
         regenerates the baseline and says why in one line of its PR body:\n{}\n\n{regen}",
        worse.join("\n")
    );
    if !moved.is_empty() {
        eprintln!("the measures moved (GH #2111); regenerate the baseline in the change that moved them:\n{}\n\n{regen}", moved.join("\n"));
    }
}

/// Stack for a build worker: the compiler's passes recurse (the parser,
/// the check, lowering), and a spawned thread's default (2 MiB) is well
/// under the main thread a build otherwise runs on; the dna fixtures'
/// largest programs overflowed it on macOS. As `hale test`'s workers
/// (`TEST_WORKER_STACK`): a virtual reservation, committed as used.
const WORKER_STACK: usize = 256 << 20;

#[test]
fn every_program_s_values_have_rows() {
    // lowering's verdict is all this reads: the build stops there (a
    // program lowering accepts would otherwise be optimized and linked)
    let options = BuildOptions { lower_only: true, ..build_opts::options() };
    let programs = programs();
    let seeds = programs.iter().filter(|(_, p)| matches!(p, Source::Entry(_))).count();
    assert!(seeds > 400, "the corpus, tests/hale and dna/tests: found {seeds}");
    assert!(programs.len() - seeds > 1000, "the embedded programs: found {}", programs.len() - seeds);

    let failed: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
    std::thread::scope(|s| {
        for _ in 0..workers {
            std::thread::Builder::new()
                .name("seam-rows-worker".to_string())
                .stack_size(WORKER_STACK)
                .spawn_scoped(s, || loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((name, source)) = programs.get(i) else { break };
                    if let Err(e) = lowers_typed(source, &options) {
                        failed.lock().expect("the failures").push(format!("{name}: {e}"));
                    }
                })
                .expect("spawn a build worker");
        }
    });
    let failed = failed.into_inner().expect("the failures");
    assert!(failed.is_empty(), "programs lowering did not take every value's type from the checker:\n{}", failed.join("\n"));
}

/// A literal in a tuple's sub-pattern is lowered as a literal of the
/// tuple's part it is matched against: `(1, 1min)` against an `(Int,
/// Duration)` is an `Int` and a `Duration`, and its rows say so. The check
/// typed a whole pattern's literal only.
#[test]
fn a_sub_pattern_s_literals_have_rows() {
    let options = build_opts::options();
    let src = "fn f(t: (Int, Duration)) -> Int { return match t { (1, 1min) -> 1, _ -> 0 }; }\n\
               fn main() { println(f((1, 60s))); }\n";
    let out = harness::unique_bin("seam_strict_sub_pattern");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "1\n");
}

/// Values lowering lowers that the check left untyped, or never visited:
/// a restart bound, a shm write's binder, an unascribed param's array of
/// loci, a generic record's array-of-instantiation field, the unprefixed
/// `time::` calls, a generic locus's field of a fn type named by an alias
/// called, the `local_context()` a direct send passes, and a batch
/// handler's `Drain<T>` and its elements. Each has its row.
#[test]
fn what_the_check_left_untyped_has_rows() {
    let options = BuildOptions { lower_only: true, ..build_opts::options() };
    let programs = [
        ("restart bound", r#"locus Worker {
    params { attempts: Int = 0; target: Int = 99; }
    closure reached { self.attempts ~~ self.target within 0; epoch birth; }
    birth() { self.attempts = self.attempts + 1; }
    run() { println("RUN"); }
}
locus Coordinator {
    params { max: Int = 2; }
    on_failure(c: Worker, err: ClosureViolation) {
        let k = self.max + 7;
        println("fail", k);
        restart(c) for self.max + 1;
    }
    run() { Worker { target: 99 }; }
}
fn main() { let c = Coordinator { }; }
"#),
        ("unascribed param", r#"locus C {
    params { name: String = "c"; }
    fn bye() { println(f"C.bye {self.name}"); }
}
fn close(c: C) { c.bye(); }
locus P {
    params { cs = [C { name: "a" }, C { name: "b" }]; n = 3; }
    run() { std::time::sleep(20ms); }
    dissolve() { for c in self.cs { c.bye(); } close(self.cs[0]); println(self.n + 1); }
}
main locus App {
    params { q: P = P { }; }
    run() { std::time::sleep(100ms); }
}
fn main() { App { }; }
"#),
        ("array of an instantiation", r#"
type In {
    k: Int = 0;
    label: String = "";
}

type Holder<T> {
    items: [T; 2];
}

type Shelf<T> {
    rows: [Holder<T>; 2];
}

type Wrap {
    h: Holder<In>;
    tag: String = "";
}

type Box<T> {
    v: T;
}

fn main() {
    let a: Holder<In> = Holder { items: [In { k: 1, label: "a" }, In { k: 2, label: "b" }] };
    let b: Holder<In> = Holder { items: [In { k: 3, label: "c" }, In { k: 4, label: "d" }] };
    let s: Shelf<In> = Shelf { rows: [a, b] };
    println(s.rows[1].items[0].label, s.rows[1].items[0].k);
    let w = Wrap { h: s.rows[0], tag: "w" };
    let x: Box<Wrap> = Box { v: w };
    println(x.v.tag, x.v.h.items[1].label);
}
"#),
        ("unprefixed time", r#"fn main() {
    let t0 = time::monotonic();
    let t1 = std::time::monotonic();
    let e = t1 - t0;
    if e > 20ms { println("ok"); }
}
"#),
        ("generic fn-typed field called", r#"type Callback = fn(H) -> Int;
locus Hook<T> { params { cb: T; } fn fire(h: H) -> Int { return self.cb(h); } }
fn reset(h: H) -> Int { return 7; }
locus H {
    params { n: Int = 1; }
    fn generic() -> Int {
        let hk: Hook<Callback> = Hook { cb: reset };
        return hk.fire(self);
    }
}
fn main() { let h = H { }; println(h.generic()); }
"#),
        ("context of a direct send", r#"type Ping { n: Int; }
topic Pings { payload: Ping; subject: "solo.ping"; }
locus Solo {
    bus { subscribe Pings as on_ping; publish Pings; }
    fn on_ping(p: Ping, ctx: std::api::Context) { println(ctx.via + " " + to_string(p.n)); }
    run() { Pings <- Ping { n: 3 }; }
}
fn main() { Solo { }; }
"#),
        ("batch handler's Drain", r#"
        type Tick { px: Int; sz: Int; }
        topic Quotes { payload: Tick; subject: "quotes-x"; }

        locus Agg {
            params { total: Int = 0; }
            bus { subscribe Quotes as on_quotes; }
            fn on_quotes(feed: Drain<Tick>) {
                for t in feed {
                    self.total = self.total + t.px;
                }
                println("agg total=", self.total);
            }
        }

        locus Feed {
            bus { publish Quotes; }
            birth() {
                Quotes <- Tick { px: 10, sz: 1 };
                Quotes <- Tick { px: 20, sz: 2 };
                Quotes <- Tick { px: 30, sz: 3 };
            }
        }

        main locus App {
            bindings {
                Quotes: shm_ring("/hale_scratch_drain", slot_count: 1024, on_overflow: drop) where zero_copy;
            }
        }

        fn main() {
            App { };
            Agg { };
            Feed { };
            // Give the batch reader thread time to receive + dispatch
            // the published batch before the process exits.
            time::sleep(500ms);
        }
    "#),
        ("shm write binder", r#"
        ring_layout ForeignRing {
            magic 0x52494E47464D5431;
            version 1 at 8 : u32;
            buffer_size at 12 : u32;
            data_at 128;
            cursor published { at 64; repr atomic_u64; load acquire; unit bytes; }
            framing byte_records { len_prefix u32; align 8; pad_sentinel 0xFFFFFFFF; }
            overflow lap_detect;
        }
        topic Recs { payload: BytesView; }
        locus Sub {
            bus { subscribe Recs as on_rec; }
            fn on_rec(v: BytesView) {
                let kind = std::bytes::read_i64_le(v, 0) or -1;
                let a = std::bytes::read_i64_le(v, 8) or -1;
                if kind == 2 {
                    let b = std::bytes::read_i64_le(v, 16) or -1;
                    println("rec kind=2 a=", to_string(a), " b=", to_string(b));
                } else {
                    println("rec kind=1 a=", to_string(a));
                }
            }
        }
        locus Producer {
            bus { publish Recs; }
            birth() {
                Recs.write(24) { w =>
                    std::bytes::write_i64_le(w, 0, 1) or raise;
                    std::bytes::write_i64_le(w, 8, 100) or raise;
                    16
                };
                Recs.write(24) { w =>
                    std::bytes::write_i64_le(w, 0, 2) or raise;
                    std::bytes::write_i64_le(w, 8, 200) or raise;
                    std::bytes::write_i64_le(w, 16, 201) or raise;
                    24
                };
            }
        }
        main locus App {
            bindings {
                Recs: shm_ring("/hale_scratch", on_overflow: drop,
                               layout: ForeignRing, buffer_size: 4096);
            }
        }
        fn main() {
            App { };
            Sub { };
            std::time::sleep(50ms);
            Producer { };
            std::time::sleep(500ms);
        }
    "#),
    ];
    for (what, src) in programs {
        let out = harness::unique_bin("seam_strict_untyped");
        build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("{what}: it lowers: {e}"));
        let _ = std::fs::remove_file(&out);
    }
}

/// A locus literal in a record's field default (`type Box { k: Int =
/// n(App { }); }`, built by `Box { }`): the locus's own param defaults
/// (`w: Worker = Worker { }`) are its text, typed in its body on no
/// evaluation path, and lowering read them on the field's evaluation path,
/// where they have no row.
#[test]
fn a_locus_default_in_a_field_default_reads_its_own_rows() {
    let options = BuildOptions { lower_only: true, ..build_opts::options() };
    // a plain string, not a harvested program: the parity sweep makes `n`
    // generic, and a generic call in a field default has no row there yet
    let src = "locus Worker { run() { print(\"worker\"); } }\n\
               main locus App {\n\
                   params { w: Worker = Worker { }; }\n\
                   run() { print(\"app\"); }\n\
               }\n\
               fn n(a: App) -> Int { return 1; }\n\
               type Box { k: Int = n(App { }); }\n\
               fn main() { for i in 0..3 { let b = Box { }; } }\n";
    let out = harness::unique_bin("seam_strict_locus_default_in_a_default");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it lowers: {e}"));
    let _ = std::fs::remove_file(&out);
}

/// What the parity sweep's mutations reach, with no row: a generic fn
/// whose declared return is an interface (`fn make<T>(w: T) -> Greeter`),
/// whose call the checker typed `Unknown` (an interface is no type of the
/// checker's own; a non-generic fn's return reads as the interface it
/// declares, and a generic one's now does too); and a hashmap form over a
/// record another file declares, whose key the form's synthesized methods
/// read before that record was in scope, so `m.key_at(i) or raise` was
/// typed `Unknown`. Plain strings, not harvested programs, so the sweep
/// mutates neither further.
#[test]
fn what_the_parity_sweep_reaches_has_rows() {
    let options = BuildOptions { lower_only: true, ..build_opts::options() };
    let src = "interface Greeter { fn greet() -> String; }\n\
               locus Hi { fn greet() -> String { return \"hi\"; } }\n\
               locus Hey { fn greet() -> String { return \"hey\"; } }\n\
               fn make<T>(which: T) -> Greeter { if which == 0 { return Hi { }; } return Hey { }; }\n\
               fn main() { let b = make(1); println(b.greet()); }\n";
    let out = harness::unique_bin("seam_strict_generic_interface_return");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("a generic fn's interface return: it lowers: {e}"));
    let dir = harness::unique_dir("seam_strict_hashmap_import");
    std::fs::create_dir_all(dir.join("lib")).expect("the seed's directory");
    std::fs::write(
        dir.join("main.hl"),
        "import \"./lib\" as lib { WC };\n\
         @form(hashmap)\n\
         locus CM { capacity { pool entries of WC indexed_by word; } }\n\
         fn walk() { let m = CM { }; let mut i = 0; while i < m.len() { let k = m.key_at(i) or raise; i = i + 1; } }\n\
         fn main() { walk(); }\n",
    )
    .expect("main.hl");
    std::fs::write(dir.join("lib/lib.hl"), "type WC { word: String; count: Int; }\n").expect("lib.hl");
    let out = harness::unique_bin("seam_strict_hashmap_import");
    build_opts::build_seed_dir(&dir, &out, &options).unwrap_or_else(|e| panic!("an imported hashmap cell: it lowers: {e}"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A substitute block that leaves (`f() or { return x; }`) is no value:
/// the checker types it `()` and accepts it beside the call's value only
/// because it never ends. Lowering built it as a value, with a
/// placeholder it typed `Int` itself, and strict mode counted it a block
/// with no layout (836 of the corpus's sites, the RPC decoders' bails
/// most of them). It is built as statements now, at every success type.
#[test]
fn a_substitute_that_leaves_asks_no_value() {
    let options = build_opts::options();
    let src = "type Pt { x: Int; }\n\
               fn word(i: Int) -> String fallible(String) { if i < 0 { fail \"neg\"; } \"w\" }\n\
               fn point(i: Int) -> Pt fallible(String) { if i < 0 { fail \"neg\"; } Pt { x: i } }\n\
               fn count(i: Int) -> Int fallible(String) { if i < 0 { fail \"neg\"; } i }\n\
               fn first(i: Int) -> String {\n\
                   let s = word(i) or { return \"left at word\"; };\n\
                   let p = point(i) or { return \"left at point: \" + err; };\n\
                   let n = count(i) or { return \"left at count\"; };\n\
                   s + to_string(p.x + n)\n\
               }\n\
               fn main() { println(first(2)); println(first(-1)); }\n";
    let out = harness::unique_bin("seam_strict_leaving_substitute");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "w4\nleft at word\n");
}

/// What names no type of its own takes the place's, and the checker's row
/// says so: an empty literal (`[]`) at a `bounded` or array place, a
/// generic record's literal spelled with the template's name at its
/// monomorph's place (a fixed-width argument's too, `Box<U64>`), and a
/// quantity at a denomination no declaration names (`Duration in s`),
/// which is the integer it counts. Each was a row lowering had no layout
/// for.
#[test]
fn a_place_decides_what_names_no_type_of_its_own() {
    let options = build_opts::options();
    let src = "type Box<T> { value: T; }\n\
               fn main() {\n\
                   let mut tags: bounded[String; 4] = [];\n\
                   push(tags, \"a\") or raise;\n\
                   let none: [Int; 0] = [];\n\
                   let b: Box<U64> = Box { value: 7 };\n\
                   let i: Box<Int> = Box { value: 8 };\n\
                   let secs = (3s + 500ms).in(s) or floor;\n\
                   println(count(tags) + len(none));\n\
                   println(b.value);\n\
                   println(i.value);\n\
                   println(secs);\n\
               }\n";
    let out = harness::unique_bin("seam_strict_place_decides");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "1\n7\n8\n3s\n");
}

/// An element chain's nodes have rows of their own. The desugar builds
/// them before the check, but every one at the chain's span, and a row's
/// key told a literal or a block only by its span: the chain's counter
/// `0` and its flag `true`, and its `{ …; n }` and its `or { break; }`,
/// were one key with two answers, so `Unknown`. A literal is keyed by its
/// class too, and a block by whether it gives a value
/// (`hale_types::typed_bodies::expr_name`).
#[test]
fn a_chain_s_nodes_have_rows_of_their_own() {
    let options = build_opts::options();
    let src = "fn main() {\n\
                   let xs = [1, 2, 3, 4];\n\
                   println(xs.filter(it > 1).count());\n\
                   println(xs.map(it * 2).sum());\n\
                   println(xs.any(it > 3));\n\
                   let f = xs.filter(it > 2).first() or 0;\n\
                   println(f);\n\
               }\n";
    let out = harness::unique_bin("seam_strict_chain_rows");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "3\n20\ntrue\n3\n");
}

/// An element chain's operators have rows of their own. Its loop's counter
/// step `ctr + 1` (an `Int`), its bound `ctr >= lim` (a `Bool`) and a
/// sum's `acc + fin` (the sum's type) are all at the chain's span, and a
/// row's key told an operator only by its span: one key with three
/// answers, so `Unknown`. An operator is keyed by itself and its operands'
/// kinds too (`hale_types::typed_bodies::expr_name`).
#[test]
fn a_chain_s_operators_have_rows_of_their_own() {
    let options = build_opts::options();
    let src = "fn main() {\n\
                   let v = [1, 2, 3, 4, 5, 6, 7, 8];\n\
                   println(v.skip(1).take(3).sum());\n\
                   println(v.filter(it % 2 == 0).skip(1).take(2).sum());\n\
                   let f = [1.5, 2.5];\n\
                   println(f.map(it).sum(0.0));\n\
                   println(v.filter(it > 6).max() or 0);\n\
               }\n";
    let out = harness::unique_bin("seam_strict_chain_operators");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "9\n10\n4\n8\n");
}

/// A loop over a form's `entries` / `items`, or over `self.children`, is
/// lowered by its receiver's row. The iterand itself is a form's
/// pseudo-field, which the checker gives no type (it binds the loop
/// variable to the form's cell), or the children, whose type has no
/// layout. Lowering asked the iterand's row whether it was a `bounded`
/// before it looked for either shape, so each loop was a missing row.
#[test]
fn a_loop_over_a_form_or_the_children_asks_its_receiver_s_row() {
    let options = build_opts::options();
    let src = "type Cell { id: Int; n: Int; }\n\
               @form(hashmap)\n\
               locus Cells { capacity { pool cells of Cell indexed_by id; } }\n\
               @form(vec)\n\
               locus CellList { capacity { heap cells of Cell; } }\n\
               locus Leaf { params { value: Int = 0; } contract { expose value: Int; } }\n\
               locus Sum {\n\
                   contract { consume value: Int; }\n\
                   accept(c: Leaf) { }\n\
                   fn total() -> Int {\n\
                       let mut t = 0;\n\
                       for child in self.children { t = t + child.value; }\n\
                       return t;\n\
                   }\n\
                   run() {\n\
                       let _a = Leaf { value: 10 };\n\
                       let _b = Leaf { value: 20 };\n\
                       println(self.total());\n\
                   }\n\
               }\n\
               fn main() {\n\
                   let m = Cells { };\n\
                   m.set(Cell { id: 1, n: 3 });\n\
                   m.set(Cell { id: 2, n: 4 });\n\
                   let mut a = 0;\n\
                   for e in m.entries { a = a + e.n; }\n\
                   println(a);\n\
                   let v = CellList { };\n\
                   v.push(Cell { id: 1, n: 5 });\n\
                   v.push(Cell { id: 2, n: 6 });\n\
                   let mut b = 0;\n\
                   for x in v.items { b = b + x.n; }\n\
                   println(b);\n\
                   Sum { };\n\
               }\n";
    let out = harness::unique_bin("seam_strict_form_loops");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "7\n11\n30\n");
}

/// A path to a fn used as a value (an imported seed's, `apply(lib::add3)`,
/// or a Hale-bodied stdlib fn's, `apply(std::url::encode)`, GH #1082) is
/// the fn pointer of the fn it names, typed by the path's row. Lowering
/// built a name at the path and lowered that instead, and a built name
/// has no row.
#[test]
fn a_path_to_a_fn_used_as_a_value_is_typed_by_its_row() {
    let options = build_opts::options();
    let src = "fn apply(f: fn (String) -> String, s: String) -> String { return f(s); }\n\
               fn main() {\n\
                   println(apply(std::url::encode, \"a b\"));\n\
                   let f = std::url::encode;\n\
                   println(f(\"c/d\"));\n\
               }\n";
    let out = harness::unique_bin("seam_strict_fn_values");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "a%20b\nc%2Fd\n");
}

/// What lowering builds where an error is raised has rows: an `or raise` a
/// `@raise fn` converts is built as `or fail conv(err)` at the `raise`, and
/// a fallible handler's implicit `or raise` as `handler(err) or raise` at
/// the handler. The check decides both (`judge_raise`, GH #1951) and now
/// types what is built there (`ViewTypes::note_built`): the conversion's
/// call and its `err`, and the handler's `or`.
#[test]
fn what_lowering_builds_where_an_error_is_raised_has_rows() {
    let options = build_opts::options();
    let src = "type ParseError2 { at: Int; text: String; }\n\
               type FetchError { url: String; }\n\
               type AppError = ParseError2 | FetchError;\n\
               type Report { code: Int; detail: String; }\n\
               fn parse(s: String) -> Int fallible(ParseError2) {\n\
                   if s == \"bad\" { fail ParseError2 { at: 3, text: s }; }\n\
                   return len(s);\n\
               }\n\
               fn fetch(url: String) -> String fallible(FetchError) {\n\
                   if url == \"\" { fail FetchError { url: \"<none>\" }; }\n\
                   return url;\n\
               }\n\
               fn reparse(e: FetchError) -> String fallible(ParseError2) {\n\
                   fail ParseError2 { at: 0, text: \"retry \" + e.url };\n\
               }\n\
               @raise fn report_from_parse(e: ParseError2) -> Report {\n\
                   return Report { code: 400 + e.at, detail: \"parse: \" + e.text };\n\
               }\n\
               fn checked(s: String) -> Int fallible(Report) {\n\
                   let n = parse(s) or raise;\n\
                   return n * 10;\n\
               }\n\
               fn refetch(url: String) -> String fallible(Report) {\n\
                   let body = fetch(url) or reparse(err);\n\
                   return body;\n\
               }\n\
               fn load_again(url: String) -> String fallible(AppError) {\n\
                   let body = fetch(url) or reparse(err);\n\
                   return body;\n\
               }\n\
               fn main() {\n\
                   println(checked(\"abcd\") or -1);\n\
                   println(checked(\"bad\") or err.code);\n\
                   println(refetch(\"\") or err.detail);\n\
                   println(load_again(\"x\") or \"e\");\n\
               }\n";
    let out = harness::unique_bin("seam_strict_raise_rows");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "40\n403\nparse: retry <none>\nx\n");
}

/// What the program's prelude lowers for the main locus's `bindings { }`
/// has rows, in that locus's body: an adapter's literal and its
/// initializers (the checker checks the literal as an expression of a free
/// fn, and keeps its rows for the prelude), and a `unix(path)` binding's
/// transport literal (typed from the binding's row). The prelude was lowered
/// in no body at all, so each was a missing row.
#[test]
fn what_the_prelude_lowers_for_the_bindings_has_rows() {
    let options = build_opts::options();
    let src = "type Note { n: Int = 0; }\n\
               type Wire { subject: String = \"\"; data: Bytes = b\"\"; }\n\
               topic Lost { payload: Note; subject: \"lost\"; }\n\
               topic Beat { payload: Note; subject: \"beat\"; }\n\
               topic Q { payload: Wire; subject: \"q\"; }\n\
               locus Fwd {\n\
                   params { tag: String = \"\"; }\n\
                   bus { publish Q; }\n\
                   fn send(subject: String, bytes: Bytes) { Q <- Wire { subject: self.tag + subject, data: bytes }; }\n\
               }\n\
               main locus App {\n\
                   bindings {\n\
                       Lost: Fwd { tag: \"t-\" + \"x\" };\n\
                       Beat: unix(\"/tmp/hale-seam-prelude.sock\", role: connect);\n\
                   }\n\
                   bus { publish Lost; publish Beat; }\n\
                   run() { }\n\
               }\n\
               fn main() { App { }; }\n";
    let out = harness::unique_bin("seam_strict_prelude_rows");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    build_opts::build_source(src, &out, &build_opts::options()).expect("it builds");
    let _ = std::fs::remove_file(&out);
}

/// A literal lowering lowers outside an expression of the program has a
/// row: a match arm's literal pattern (`1min` against a `Duration`), which
/// the check holds to the scrutinee's type and so types as it, and a
/// closure's `epoch duration(20ms)`, which the check typed but the rows'
/// visitor never visited. A `U64` pattern past `Int` is built at the
/// scrutinee's type, as an `Int` one already was.
#[test]
fn a_pattern_s_and_an_epoch_s_literals_have_rows() {
    let options = build_opts::options();
    let src = "locus Beat {\n\
                   params { n: Int = 1; }\n\
                   closure tick {\n\
                       self.n ~~ 1 within 0;\n\
                       epoch duration(20ms);\n\
                   }\n\
                   run() { }\n\
               }\n\
               fn kind(d: Duration) -> Int {\n\
                   return match d { 1min -> 1, _ -> 0 };\n\
               }\n\
               fn big(u: U64) -> Int {\n\
                   return match u { 18446744073709551615 -> 2, _ -> 0 };\n\
               }\n\
               fn main() {\n\
                   println(kind(60s));\n\
                   println(big(U64::max()));\n\
                   Beat { };\n\
               }\n";
    let out = harness::unique_bin("seam_strict_pattern_literals");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "1\n2\n");
}

/// An element chain's value blocks and `or`s have rows of their own. A
/// chain builds them at its span, and a key told a block only by whether it
/// gives a value and an `or` not at all: `max(it.qty)` gives its element
/// from one value block and compares its key from another, and
/// `reverse_into(r)` has an `or` beside another. A value block is keyed by
/// its tail too, and an `or` by what it handles
/// (`hale_types::typed_bodies::expr_name`).
#[test]
fn a_chain_s_blocks_and_ors_have_rows_of_their_own() {
    let options = build_opts::options();
    let src = "type Line { sku: String = \"\"; qty: Int = 0; }\n\
               type Ledger { entries: bounded[Line; 4]; }\n\
               @form(vec)\n\
               locus Nums { capacity { heap items of Int; } }\n\
               fn main() {\n\
                   let l = Ledger { };\n\
                   push(l.entries, Line { sku: \"ab\", qty: 2 }) or discard;\n\
                   push(l.entries, Line { sku: \"cd\", qty: 7 }) or discard;\n\
                   let top = l.entries.max(it.qty) or Line { };\n\
                   println(top.sku);\n\
                   let v = Nums { };\n\
                   v.push(1);\n\
                   v.push(2);\n\
                   v.push(3);\n\
                   v.push(4);\n\
                   let r = Nums { };\n\
                   v.take(3).reverse_into(r);\n\
                   println(r.get(0) or -1);\n\
               }\n";
    let out = harness::unique_bin("seam_strict_chain_blocks");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "cd\n3\n");
}

/// A `.Variant` pattern's binder of a generic enum's monomorph has a row:
/// `.Some(x)` against a `Value<Int>` (spelled so, through an alias, or in a
/// generic body's `Int` specialization) binds `x` an `Int`. The check found
/// a variant's values only in a declared enum, so it bound `x` `Unknown`.
#[test]
fn a_variant_binder_of_a_monomorph_has_a_row() {
    let options = build_opts::options();
    let src = "type Value<T> = enum { Some(T), None };\n\
               type IntValue = Value<Int>;\n\
               fn unwrap_or(v: Value<Int>, d: Int) -> Int {\n\
                   return match v { .Some(x) -> x, .None -> d };\n\
               }\n\
               fn aliased(v: IntValue) -> Int {\n\
                   if let .Some(x) = v { x } else { -7 }\n\
               }\n\
               fn get<T>(v: Value<T>, d: T) -> T {\n\
                   return match v { .Some(x) -> x, .None -> d };\n\
               }\n\
               fn main() {\n\
                   println(unwrap_or(Value_Int::Some(4), 0));\n\
                   println(aliased(Value_Int::None));\n\
                   println(get(Value_Int::Some(11), 0));\n\
               }\n";
    let out = harness::unique_bin("seam_strict_variant_binders");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "4\n-7\n11\n");
}

/// A `match` whose every arm leaves, used where a value goes (a `let`'s
/// initializer, an `or` block's trailing `match`), gives no value: the
/// check records it `()`, the row lowering reads as "builds no value", as
/// it reads an `or { return x; }` block's. It recorded it `Unknown`, which
/// lowering read as a missing row; the check still holds it `Unknown`,
/// legal where a diverging expression is.
#[test]
fn a_match_whose_every_arm_leaves_gives_no_value() {
    let options = build_opts::options();
    let src = "fn risky(n: Int) -> Int fallible(String) { if n > 0 { fail \"pos\"; } return n; }\n\
               fn all_arms(n: Int) -> Int {\n\
                   let v = match n {\n\
                       0 -> { return 10; },\n\
                       _ -> { match n { 1 -> { return 11; }, _ -> { return 12; }, } },\n\
                   };\n\
                   return v;\n\
               }\n\
               fn or_leaving(n: Int) -> Int {\n\
                   let v = risky(n) or { match err { \"pos\" -> { return 100; }, _ -> { return 200; }, } };\n\
                   return v + 1;\n\
               }\n\
               fn main() { println(all_arms(1)); println(or_leaving(1)); println(or_leaving(0)); }\n";
    let out = harness::unique_bin("seam_strict_leaving_match");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "11\n100\n1\n");
}

/// A generic locus's param default is read in the built monomorph's
/// specialization. `wrapped: Box<T> = Box { value: 7 }` is lowered where a
/// `Holder<Int>` is built, as `Box_Int`; the check typed it so in
/// `Holder_Int`'s walk, but lowering asked the template's walk (whose row is
/// the template `Box`, with no layout): the specialization it read was the
/// enclosing method's, not the built locus's.
#[test]
fn a_generic_locus_s_param_default_reads_its_monomorph_s_rows() {
    let options = build_opts::options();
    let src = "type Box<T> { value: T = 0; }\n\
               locus Holder<T> { params { wrapped: Box<T> = Box { value: 7 }; } }\n\
               fn main() { let h: Holder<Int> = Holder { }; println(h.wrapped.value); }\n";
    let out = harness::unique_bin("seam_strict_template_default");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "7\n");
}

/// The built locus's specialization wins over the enclosing generic fn's:
/// `make<U>` building a `Holder<Int>` lowers its `Box { value: 7 }` default
/// as `Holder_Int`'s, not with `make_String`'s own arguments.
#[test]
fn a_param_default_built_in_a_generic_fn_reads_the_built_locus_s_rows() {
    let options = build_opts::options();
    let src = "type Box<T> { value: T = 0; }\n\
               locus Holder<T> { params { wrapped: Box<T> = Box { value: 7 }; } }\n\
               fn make<U>(x: U) -> Int { let h: Holder<Int> = Holder { }; return h.wrapped.value; }\n\
               fn main() { println(make(\"hello\")); }\n";
    let out = harness::unique_bin("seam_strict_template_default_in_generic_fn");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "7\n");
}

/// A send's subject that is a value the send reads (`subj <- …`, the
/// logger's `"log." + path` built at run time) has a row. The check looked
/// a subject up as a topic or a wire-subject literal only, and never typed
/// one that is neither, which lowering reads as a `String`.
#[test]
fn a_send_s_read_subject_has_a_row() {
    let options = build_opts::options();
    let src = "locus Sink {\n\
                   params { seen: Int = 0; }\n\
                   bus { subscribe \"log.**\" as on_event of type std::log::LogEvent; }\n\
                   fn on_event(e: std::log::LogEvent) { self.seen = self.seen + 1; }\n\
               }\n\
               main locus App {\n\
                   params {\n\
                       sink: Sink = Sink { };\n\
                       log: std::log::Logger = std::log::Logger { name: \"app\", min_severity: 2 };\n\
                   }\n\
                   run() { self.log.info(\"hello\"); }\n\
               }\n\
               fn main() { App { }; }\n";
    let out = harness::unique_bin("seam_strict_send_subject");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    build_opts::build_source(src, &out, &build_opts::options()).expect("it builds");
    let _ = std::fs::remove_file(&out);
}

/// Each exposure's generated param has rows of its own. The API expansion
/// builds an exposure's param from text, `… rows: __RpcSurface_<id> { } …`,
/// and parsed every exposure's at one range: two exposures' surface
/// literals were at one span of the main locus's body, one row key with two
/// types (`__RpcSurface_3` and `__RpcSurface_4`), `Unknown`. Each exposure's
/// is parsed at a range of its own. `tests/hale/api/witness_test.hl` serves
/// several surfaces from one main locus.
#[test]
fn each_exposure_s_generated_param_has_rows_of_its_own() {
    let options = BuildOptions { lower_only: true, ..build_opts::options() };
    let entry = repo_root().join("tests/hale/api/witness_test.hl");
    lowers_typed(&Source::Entry(entry), &options).unwrap_or_else(|e| panic!("it lowers: {e}"));
}

/// A capacity slot's cell has a type: `self.<slot>.acquire()` from a pool
/// and `alloc()` from a heap give a cell of the slot's element, from this
/// locus and slot, as lowering builds it, and a cell of a record reads and
/// writes its element's fields. The check typed the call `Unknown` and the
/// local bound to it with it.
#[test]
fn a_slot_s_cell_has_a_type() {
    let options = build_opts::options();
    let src = "type R { a: Int = 0; }\n\
               locus Slots {\n\
                   capacity { pool entries of Int; heap cells of R; }\n\
                   run() {\n\
                       let c = self.entries.acquire();\n\
                       self.entries.release(c);\n\
                       let r = self.cells.alloc();\n\
                       r.a = 3;\n\
                       println(r.a + 1);\n\
                       self.cells.free(r);\n\
                   }\n\
               }\n\
               fn main() { Slots { }; }\n";
    let out = harness::unique_bin("seam_strict_slot_cells");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "4\n");
}

/// A send the intra-locus rewrite turns into a direct call has rows for
/// the nodes the call is made of. The rewrite is lowering's, after the
/// check, and builds `self.<field>.<handler>(value)` (a child subscribes)
/// or `self.<handler>(value)` (the publisher does) at the send's span: its
/// `self` and its field had no row, and lowering typed them itself. They
/// are typed from the scope's declarations, as the checker types them
/// (`hale_types::typed_bodies::ExprTypes::add_direct_sends`).
#[test]
fn a_direct_send_s_nodes_have_rows() {
    let options = build_opts::options();
    let src = "type Ping { seq: Int = 0; }\n\
               topic Tick { payload: Ping; subject: \"direct.tick\"; }\n\
               topic Beat { payload: Ping; subject: \"direct.beat\"; }\n\
               locus Sub {\n\
                   params { acc: Int = 0; }\n\
                   bus { subscribe Tick as on_tick; }\n\
                   fn on_tick(p: Ping) { self.acc = self.acc + p.seq; }\n\
                   fn sum() -> Int { return self.acc; }\n\
               }\n\
               locus Pub {\n\
                   params { sub: Sub = Sub { }; n: Int = 0; }\n\
                   bus { publish Tick; publish Beat; subscribe Beat as on_beat; }\n\
                   fn on_beat(p: Ping) { self.n = self.n + p.seq; }\n\
                   fn go() {\n\
                       Tick <- Ping { seq: 3 };\n\
                       Beat <- Ping { seq: 4 };\n\
                   }\n\
                   fn total() -> Int { return self.sub.sum() + self.n; }\n\
               }\n\
               main locus App { params { pub: Pub = Pub { }; } }\n\
               fn main() {\n\
                   let app = App { };\n\
                   app.pub.go();\n\
                   println(app.pub.total());\n\
               }\n";
    let out = harness::unique_bin("seam_strict_direct_send_rows");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "7\n");
}

/// A call of a method beside a param of its name, and of a generic enum's
/// monomorph's variant spelled by its mangled name, have rows: the check
/// typed both calls `Unknown` (the first's callee as the param, the
/// second's as no enum it knew), and lowering typed them itself.
#[test]
fn a_method_and_a_monomorph_s_variant_have_call_rows() {
    let options = build_opts::options();
    let src = "type Value<T> = enum { Some(T), None };\n\
               locus Sub {\n\
                   params { hits: Int = 4; }\n\
                   fn hits() -> Int { return self.hits + 1; }\n\
               }\n\
               locus Pub {\n\
                   params { sub: Sub = Sub { }; }\n\
                   fn total() -> Int { return self.sub.hits(); }\n\
               }\n\
               fn main() {\n\
                   let p = Pub { };\n\
                   println(p.total());\n\
                   let a: Value<Int> = Value_Int::Some(1);\n\
                   println(a == Value_Int::Some(1));\n\
               }\n";
    let out = harness::unique_bin("seam_strict_call_rows");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "5\ntrue\n");
}

/// A default left to a literal has its rows where the literal evaluates
/// it: in a stdlib body (`std::metrics`' counter builds its entry with
/// `__StdMetricsEntry { }`), and in a program's body inside a stdlib
/// default (`std::api::Context { }` leaves `caller`, whose default is a
/// `std::api::Principal { }`), where the evaluation path names the
/// stdlib's literal by the id the checker walked it under.
#[test]
fn a_stdlib_default_has_rows_where_it_is_evaluated() {
    let options = build_opts::options();
    let src = "fn main() {\n\
                   let reg = std::metrics::Registry { namespace: \"app\" };\n\
                   let hits = std::metrics::counter(reg, \"hits\", std::metrics::labels_one(\"r\", \"a\"));\n\
                   hits.inc();\n\
                   let c = std::api::Context { role: \"r\" };\n\
                   println(c.caller.uid);\n\
                   println(c.caller.mode);\n\
               }\n";
    let out = harness::unique_bin("seam_strict_stdlib_defaults");
    build_opts::build_source(src, &out, &options).unwrap_or_else(|e| panic!("it builds: {e}"));
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "-1\nlocal\n");
}


