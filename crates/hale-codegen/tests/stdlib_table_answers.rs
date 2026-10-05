//! The stdlib table's answers, pinned (F.40 phase 4, S2).
//!
//! `hale_types::stdlib_surface` answers the checker, the effects
//! analysis, the documentation and lowering through a handful of
//! functions: `lookup`, `is_locus_path`, `effects_for`, `signature_for`,
//! `parks_on_async_io`, `holds_cooperative_worker`, `unknown_fn_error`
//! and `suggest`, plus the namespace listing the catalogue, the LSP and
//! the stdlib digest walk. This test renders every one of those answers,
//! for every path the table, the renames and the dispatchers name (and a
//! few misspellings), into one text held equal to a committed golden
//! file. It was written before the two tables (`SURFACES` and `SIGS`)
//! were folded into one row per function, as the fold's oracle: the
//! golden did not move across the fold, and it stays as the table's pin.
//!
//! A deliberate change to an answer regenerates it:
//!
//! ```sh
//! cargo test --release -p hale-codegen --test stdlib_data stdlib_table_answers:: \
//!     --config 'env.HALE_REGEN_STDLIB_TABLE="1"'
//! ```

use std::collections::BTreeSet;
use std::fmt::Write as _;

use hale_types::stdlib_surface::{self as surf, EffectSet, SigTy};

const GOLDEN: &str = "tests/fixtures/stdlib_table_answers.txt";

/// Paths no table names, each probing one way an answer is decided: a
/// near miss in a tabled namespace, an unknown namespace, a builtin's
/// spelling, a path without its `std`, the bare builtins, and paths too
/// short or too long to be a function.
const PROBES: &[&str] = &[
    "std",
    "std::io",
    "std::io::fs",
    "std::io::fs::read_fil",
    "std::io::fs::read_file::extra",
    "std::io::tcp::conect",
    "std::io::file::clos",
    "std::time::slep",
    "std::str::lenght",
    "std::str::len",
    "std::str::parse_in",
    "std::json::parse",
    "std::mth::sqrt",
    "std::totally::fake",
    "std::bytes::read_u17_le",
    "std::bytes::builder::__apend",
    "std::bus::__binding_fai",
    "std::test::__note_pas",
    "std::io::sockopt::SO_REUSEADR",
    "std::x",
    "env::args_count",
    "println",
    "print",
    "eprintln",
    "eprint",
    "len",
];

fn sig_ty(t: &SigTy) -> String {
    match t {
        SigTy::Int => "Int".into(),
        SigTy::Uint => "Uint".into(),
        SigTy::Float => "Float".into(),
        SigTy::Bool => "Bool".into(),
        SigTy::Str => "Str".into(),
        SigTy::Bytes => "Bytes".into(),
        SigTy::BytesMut => "BytesMut".into(),
        SigTy::Decimal => "Decimal".into(),
        SigTy::Duration => "Duration".into(),
        SigTy::Time => "Time".into(),
        SigTy::Unit => "Unit".into(),
        SigTy::Any => "Any".into(),
        SigTy::Named(n) => format!("Named({n})"),
    }
}

fn effects(e: EffectSet) -> String {
    if e.is_unclassified() {
        return "UNCLASSIFIED".into();
    }
    const NAMES: &[(EffectSet, &str)] = &[
        (EffectSet::SYSCALL, "syscall"),
        (EffectSet::BLOCK, "block"),
        (EffectSet::PUBLISH, "publish"),
        (EffectSet::TIME, "time"),
        (EffectSet::ENTROPY, "entropy"),
        (EffectSet::ENV, "env"),
        (EffectSet::ALLOC, "alloc"),
        (EffectSet::SECRET_USE, "secret_use"),
    ];
    let known = NAMES.iter().fold(0, |acc, (s, _)| acc | s.0);
    let mut parts: Vec<String> =
        NAMES.iter().filter(|(s, _)| e.contains(*s)).map(|(_, n)| n.to_string()).collect();
    if e.0 & !known != 0 {
        parts.push(format!("{:#x}", e.0 & !known));
    }
    if parts.is_empty() {
        "pure".into()
    } else {
        parts.join("|")
    }
}

/// Every answer the table gives for one path, on one line.
fn answers(path: &str) -> String {
    let segs: Vec<&str> = path.split("::").collect();
    let mut line = path.to_string();
    let found = surf::lookup(&segs);
    match found {
        Some((s, idx)) => write!(line, " | lookup={}@{}", s.ns.join("::"), idx).unwrap(),
        None => line.push_str(" | lookup=-"),
    }
    write!(line, " | locus={}", surf::is_locus_path(&segs)).unwrap();
    match surf::effects_for(&segs) {
        Some(e) => write!(line, " | effects={}", effects(e)).unwrap(),
        None => line.push_str(" | effects=-"),
    }
    match surf::signature_for(&segs) {
        Some(sig) => {
            let params: Vec<String> = sig.params.iter().map(sig_ty).collect();
            write!(
                line,
                " | sig=({}) -> {}{} | ret_ty={}",
                params.join(", "),
                sig_ty(&sig.ret),
                sig.fallible.map(|e| format!(" ! {e}")).unwrap_or_default(),
                sig.ret_ty().display(),
            )
            .unwrap();
            match sig.or_types() {
                Some((ok, err)) => write!(line, " | or=({}, {})", ok.display(), err.display()).unwrap(),
                None => line.push_str(" | or=-"),
            }
        }
        None => line.push_str(" | sig=-"),
    }
    write!(
        line,
        " | parks={} | holds={}",
        surf::parks_on_async_io(&segs),
        surf::holds_cooperative_worker(&segs)
    )
    .unwrap();
    match surf::unknown_fn_error(&segs) {
        Some(m) => write!(line, " | unknown={m}").unwrap(),
        None => line.push_str(" | unknown=-"),
    }
    if let Some((s, idx)) = found {
        match surf::suggest(s, segs[idx]) {
            Some(n) => write!(line, " | suggest={n}").unwrap(),
            None => line.push_str(" | suggest=-"),
        }
    }
    line
}

/// Every path a table, the renames, the dispatchers or the probes name.
fn paths() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for s in surf::SURFACES {
        for f in s.fns {
            out.insert(format!("std::{}::{}", s.ns.join("::"), f.name));
        }
    }
    for s in surf::SIGS {
        out.insert(s.display_path());
    }
    for p in surf::LOCUS_PATHS {
        out.insert(p.join("::"));
    }
    for (p, _) in surf::BUILTIN_SPELLINGS {
        out.insert(p.join("::"));
    }
    for p in surf::ASYNC_IO_PARKING.iter().chain(surf::COOPERATIVE_YIELDING_BLOCK_LEAVES) {
        out.insert(p.join("::"));
    }
    for (p, _) in hale_stdlib::PATH_RENAMES {
        if p.first() == Some(&"std") {
            out.insert(p.join("::"));
        }
    }
    for s in crate::stdlib_dispatch_coverage::scrape() {
        out.extend(s.paths.keys().cloned());
    }
    out.extend(PROBES.iter().map(|p| p.to_string()));
    out
}

/// The namespace listing, in the table's order: what the catalogue, the
/// LSP's completion and the stdlib digest iterate.
fn listing() -> String {
    let mut out = String::new();
    for s in surf::SURFACES {
        writeln!(out, "ns std::{} open_prefixes={:?}", s.ns.join("::"), s.open_prefixes).unwrap();
        for f in s.fns {
            writeln!(out, "  {} {}", f.name, effects(f.effects)).unwrap();
        }
    }
    out
}

fn render() -> String {
    let mut out = String::from("# The stdlib table's answers. Generated by stdlib_table_answers.rs.\n\n## listing\n");
    out.push_str(&listing());
    out.push_str("\n## answers\n");
    for p in paths() {
        out.push_str(&answers(&p));
        out.push('\n');
    }
    out
}

#[test]
fn the_stdlib_tables_answers_are_pinned() {
    let rendered = render();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN);
    if std::env::var("HALE_REGEN_STDLIB_TABLE").as_deref() == Ok("1") {
        std::fs::write(&path, &rendered).expect("write the golden");
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    if rendered == expected {
        return;
    }
    let exp: BTreeSet<&str> = expected.lines().collect();
    let cur: BTreeSet<&str> = rendered.lines().collect();
    let gone: Vec<&&str> = exp.difference(&cur).collect();
    let new: Vec<&&str> = cur.difference(&exp).collect();
    panic!(
        "the stdlib table's answers moved ({} lines gone, {} new):\n\ngone:\n{:#?}\n\nnew:\n{:#?}\n\n\
         An intended change regenerates {GOLDEN} with HALE_REGEN_STDLIB_TABLE=1.",
        gone.len(),
        new.len(),
        gone,
        new
    );
}

/// The pin is only an oracle if it reaches the table: every row of it,
/// and paths every answer can give.
#[test]
fn the_pin_is_not_vacuous() {
    let r = render();
    let lines = r.split("\n## answers\n").nth(1).expect("an answers section").lines().count();
    assert!(lines > 500, "only {lines} paths rendered");
    for needle in [
        "| sig=(",
        "| or=(",
        "| effects=syscall|block",
        "| parks=true",
        "| holds=true",
        "| unknown=unknown stdlib function",
        "| unknown=unknown stdlib namespace",
        "| suggest=",
        "| locus=true",
    ] {
        assert!(r.contains(needle), "no answer renders `{needle}`");
    }
}
