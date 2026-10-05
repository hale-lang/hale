//! The capability matrix's shadow, checker half (F.40 phase 3, P3 1 of
//! 3): every legacy row the checker holds, run beside the cell that
//! replaces it, over the corpus, `tests/hale`, the DNA seeds, the
//! `wasm-flower` example and the playground, on the glibc, musl and
//! wasm32 columns.
//!
//! The rows, and how each legacy answer is observed:
//!
//! - the stdlib call sites (the `wasm_unavailable_stdlib` table's, which
//!   the admission law replaced in P3 2 of 3): every `std::` call site,
//!   checked under wasm32 (a `target wasm { }` declaration added where
//!   the program has none), against the `StdNamespace` cell of the
//!   path's namespace, wording included. On the POSIX columns every call
//!   lowers, which the host check observes.
//! - `wasm_target` (the trigger): whether the gate fires, observed by a
//!   probe call appended to the program, checked with no `--target` and
//!   with `--target wasm32`, against the effective-target row (T1(b)).
//! - `TargetSpec` (`has_async_io`): every `where async_io` placement
//!   entry, checked with each column's spec, against `AsyncIoPool`,
//!   wording included.
//! - `ffi_type_unportable`: every `@ffi` and `@export` parameter and
//!   return, against `FfiType` of the resolved type's class.
//!
//! The codegen rows (`link_wasm`, `is_wasm`, `lotus_replay_start_ingress`)
//! are shadowed in `crates/hale-codegen/tests/shadow_capability_lowering.rs`,
//! the CLI's invocation and `--wrap-main` refusals in
//! `crates/hale-cli/tests/shadow_capability_cli.rs`.
//!
//! The gate: every divergence is classified in
//! `fixtures/shadow_capability.txt`. Regenerate with
//! `HALE_SHADOW_REGEN=1`, then classify by hand.

#[path = "support/entries.rs"]
mod entries;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hale_graph::shadow::{gate_message, parse_fixture, program_id, Report};
use hale_syntax::ast::{
    flat_decls, Ident, LocusMember, PlacementConstraint, Program, TargetDecl, TopDecl,
};
use hale_syntax::sites::{for_each_site, SiteKind};
use hale_syntax::Span;
use hale_types::capability::{
    derive_capability_matrix, target_row, Abi, Capability, CapabilityMatrix, ConfiguredTarget,
    FfiTypeClass, TargetClass,
};
use hale_types::target::TargetSpec;
use hale_types::Bundle;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/shadow_capability.txt")
}

/// The test files and examples §2.9 of the design lists as the tree's
/// wasm programs.
const WASM_ORIGINS: &[&str] = &[
    "crates/hale-types/tests/wasm_target_gating.rs",
    "crates/hale-codegen/tests/wasm_target.rs",
    "crates/hale-cli/tests/wasm_package_csrc.rs",
    "crates/hale-cli/tests/target_model.rs",
    "crates/hale-cli/tests/build_output_path.rs",
    "crates/hale-cli/tests/wasm_link_is_quiet.rs",
    "crates/hale-cli/tests/check_arg_parsing.rs",
    "crates/hale-syntax/tests/wrap_main.rs",
    "iris/examples/wasm-flower/",
    "play/",
];

/// The corpus, plus the on-disk programs it does not walk.
fn programs() -> Vec<hale_corpus::Program> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<hale_corpus::Program>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                walk(&p, root, out);
            } else if p.extension().is_some_and(|e| e == "hl") {
                if let Ok(source) = std::fs::read_to_string(&p) {
                    let origin = p.strip_prefix(root).unwrap_or(&p).display().to_string();
                    out.push(hale_corpus::Program { origin, source });
                }
            }
        }
    }
    let root = hale_corpus::repo_root();
    let mut all = hale_corpus::all();
    for rel in ["tests/hale", "dna", "iris/examples/wasm-flower", "play"] {
        walk(&root.join(rel), &root, &mut all);
    }
    // No program in the tree spells an FFI type the predicate refuses,
    // so the FFI leg carries its own: one per refused class with a
    // spelling, in each direction and position.
    for (class, ty) in [
        ("decimal", "Decimal"),
        ("uint", "Uint"),
        ("bounded", "bounded[Int; 4]"),
        ("array", "[Int; 4]"),
        ("tuple", "(Int, Int)"),
        ("function", "fn(Int) -> Int"),
        ("unit", "()"),
    ] {
        all.push(hale_corpus::Program {
            origin: format!("crates/hale-types/tests/shadow_capability.rs#ffi-probe-{class}"),
            source: format!(
                "@ffi(\"c\") fn probe_import(x: {ty}) -> {ty};\n\n@export fn probe_export(x: {ty}) -> {ty} {{\n    return x;\n}}\n\nfn main() {{\n}}\n"
            ),
        });
    }
    let mut seen = BTreeSet::new();
    all.retain(|p| seen.insert(p.source.clone()));
    all
}

fn spec(name: &str) -> TargetSpec {
    TargetSpec::parse(name).expect("a known target")
}

/// What the checker says about one program under one target, the way
/// `check_program` prepares it (the desugar sequence, then the mint),
/// with the bundle's target set from the spec as a build sets it.
struct Checked {
    diags: Vec<hale_syntax::error::Diag>,
    known: hale_types::resolve::TopScope,
    /// The program as the checker saw it, after the desugar sequence
    /// (which drops a declaration's `-> ()`, among others).
    program: Program,
}

/// The configured target a column's check runs under: the host named
/// by nothing (a declaration then selects wasm32, as `hale check`
/// without `--target`), or the triple `--target` names.
fn configured(target: &TargetSpec, explicit: bool) -> ConfiguredTarget {
    if explicit {
        ConfiguredTarget { name: target.triple.to_string(), spec: *target, explicit: true }
    } else {
        ConfiguredTarget::host()
    }
}

fn check(program: &Program, target: &ConfiguredTarget) -> Option<Checked> {
    let mut p = program.clone();
    hale_types::desugar_sequence::desugar_before_check(
        &mut [&mut p],
        &hale_types::desugar_sequence::Sequence { import_renames: &[], api: None, api_roles: None },
    )
    .ok()?;
    let ids = hale_types::snapshot::mint([("", &mut p)], &[]);
    let mut programs = BTreeMap::new();
    programs.insert(String::new(), &p);
    let mut bundle = Bundle::new(programs);
    bundle.snapshot = ids;
    bundle.target = target.clone();
    let diags = entries::check_bundle_opts_whole_program(&bundle, false);
    let (known, _) = hale_types::resolve::build_top_scope(&bundle);
    drop(bundle);
    Some(Checked { diags, known, program: p })
}

fn declares_wasm(p: &Program) -> bool {
    p.items.iter().any(|it| matches!(it, TopDecl::Target(t) if matches!(t.name.name.as_str(), "wasm" | "browser_js")))
}

/// The program with the gate's trigger added: a top-level
/// `target wasm { }`, at the end so no span moves.
fn with_declaration(p: &Program, at: usize) -> Program {
    let mut p = p.clone();
    let span = Span::new(at, at);
    p.items.push(TopDecl::Target(TargetDecl {
        name: Ident::new("wasm", span),
        capabilities: Vec::new(),
        span,
        synthesized: false,
    }));
    p
}

/// The probe the trigger leg appends after the source: one call into a
/// namespace only wasm32 refuses. Parsed with the program, so every
/// site of the program keeps its offset and the probe's lie past it.
const PROBE: &str = "\n\nfn __capability_probe() {\n    let _ = std::process::pid();\n}\n";

/// The `std::` path a call site spells: the callee's text in the
/// source, `None` for any other callee.
fn std_call_path(src: &str, at: usize) -> Option<String> {
    let rest = src.get(at..)?;
    if !rest.starts_with("std::") {
        return None;
    }
    let path: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == ':').collect();
    Some(path.trim_end_matches(':').to_string())
}

/// The namespace a `std::` path lies in: the longest namespace the
/// matrix has a row for that prefixes it. A spelling match, for the
/// shadow only: P3 2 of 3's use producer reads resolved identities.
fn namespace_of(m: &CapabilityMatrix, path: &str) -> Option<&'static str> {
    let segs: Vec<&str> = path.split("::").skip(1).collect();
    m.behaviours
        .iter()
        .filter_map(|r| match r.capability {
            Capability::StdNamespace(ns) => Some(ns),
            _ => None,
        })
        .filter(|ns| {
            let n: Vec<&str> = ns.split("::").collect();
            segs.len() > n.len() && segs[..n.len()] == n[..]
        })
        .max_by_key(|ns| ns.split("::").count())
}

const GATE: &str = "is unavailable under `target wasm`";
/// The gate's refusal under either selector (the declaration, or
/// `--target wasm32`).
const GATES: &str = "is unavailable under `";

/// One (row, column) tally: rows compared, and divergences.
#[derive(Default)]
struct Tally {
    rows: usize,
    divergent: usize,
}

struct Shadow {
    m: CapabilityMatrix,
    reports: BTreeMap<(&'static str, &'static str), Report>,
    /// Rows both producers answer with a refusal, per (row, column).
    refused: BTreeMap<(&'static str, &'static str), usize>,
    /// The design's wasm programs the walk reached, per origin.
    wasm_programs: BTreeMap<&'static str, usize>,
}

impl Shadow {
    fn report(&mut self, row: &'static str, class: TargetClass) -> &mut Report {
        self.reports
            .entry((row, class.name()))
            .or_insert_with(|| Report::new(&format!("target_capability · {row} · {}", class.name())))
    }

    fn compare(&mut self, row: &'static str, class: TargetClass, id: &str, old: Vec<(String, String)>, new: Vec<(String, String)>) {
        if old.is_empty() && new.is_empty() {
            return;
        }
        let refusals = old
            .iter()
            .filter(|(k, v)| !matches!(v.as_str(), "lower" | "portable" | "off") && new.iter().any(|(nk, nv)| nk == k && nv == v))
            .count();
        *self.refused.entry((row, class.name())).or_default() += refusals;
        let tag = format!("{} {row}", class.name());
        self.report(row, class).compare_rows(
            id,
            &old,
            &new,
            |k| Some(format!("{tag} {k}")),
            |k| Some(format!("{tag} {k}")),
            |_| Vec::new(),
            |_| Vec::new(),
        );
    }

    fn one(&mut self, origin: &str, src: &str) {
        let Ok(program) = hale_syntax::parse_source(src) else { return };
        if let Some(o) = WASM_ORIGINS.iter().find(|o| origin.starts_with(*o)) {
            *self.wasm_programs.entry(o).or_default() += 1;
        }
        let id = program_id(origin, src);
        let end = src.len() + 1;
        let declared = declares_wasm(&program);

        // The use sites, read once from the program as written.
        let mut calls: Vec<(usize, String)> = Vec::new();
        for_each_site(&program, &mut |kind, span, _| {
            if kind == SiteKind::Call {
                if let Some(path) = std_call_path(src, span.start.0 as usize) {
                    calls.push((span.start.0 as usize, path));
                }
            }
        });
        // One row per site: the walk reaches a call the parser shares
        // between two parents (an f-string's interpolation) twice.
        calls.sort();
        calls.dedup();
        let mut async_io: Vec<(String, usize)> = Vec::new();
        for item in flat_decls(&program.items) {
            if let TopDecl::Locus(l) = item {
                for m in &l.members {
                    if let LocusMember::Placement(b) = m {
                        for e in &b.entries {
                            for c in &e.constraints {
                                if c.kind == PlacementConstraint::AsyncIo {
                                    async_io.push((e.field.name.clone(), c.span.start.0 as usize));
                                }
                            }
                        }
                    }
                }
            }
        }

        // ---- the host: the program as written, plus the probe, checked
        // as `hale check` checks it with no `--target`.
        let host_spec = spec("x86_64-unknown-linux-gnu");
        let wasm_spec = spec("wasm32");
        let probed = hale_syntax::parse_source(&format!("{src}{PROBE}")).ok();
        let Some(host) = check(probed.as_ref().unwrap_or(&program), &configured(&host_spec, false)) else { return };
        if let Some(probed) = &probed {
            // The gate reads the effective target: the host fallback, a
            // declaration over it, or `--target wasm32`.
            self.trigger(&id, &program, &host, end, &configured(&host_spec, false));
            if let Some(on_wasm) = check(probed, &configured(&wasm_spec, true)) {
                self.trigger(&id, &program, &on_wasm, end, &configured(&wasm_spec, true));
            }
        }
        if !declared {
            self.table(TargetClass::PosixAsync, &id, &host, &calls, src.len());
        }
        self.ffi(&id, &host);

        // ---- wasm32: the table under its trigger, worded as the
        // declaration words it.
        let triggered = if declared { program.clone() } else { with_declaration(&program, end) };
        let on_wasm = check(&triggered, &configured(&wasm_spec, true));
        if let Some(on_wasm) = &on_wasm {
            self.table(TargetClass::Wasm32, &id, on_wasm, &calls, src.len());
        }

        // ---- `where async_io`, on each column's spec.
        if !async_io.is_empty() {
            self.async_io(TargetClass::PosixAsync, &id, &host, &async_io);
            if let Some(musl) = check(&program, &configured(&spec("x86_64-unknown-linux-musl"), true)) {
                self.async_io(TargetClass::PosixNoAsync, &id, &musl, &async_io);
            }
            if let Some(on_wasm) = &on_wasm {
                self.async_io(TargetClass::Wasm32, &id, on_wasm, &async_io);
            }
        }
    }

    /// The stdlib gate's call-site refusals against `StdNamespace`, per
    /// call site: the table `wasm_unavailable_stdlib` kept until the
    /// admission law replaced it (P3 2 of 3), held to the cell its path
    /// names. A refusal with a witness chain is a use the table never
    /// saw (a construction, a handle's method, a crossing), not a call
    /// site's.
    fn table(&mut self, class: TargetClass, id: &str, checked: &Checked, calls: &[(usize, String)], src_len: usize) {
        let mut old = Vec::new();
        let mut new = Vec::new();
        let refused: BTreeMap<usize, &str> = checked
            .diags
            .iter()
            .filter(|d| d.message.contains(GATE) && !d.message.contains(" — witness: "))
            .map(|d| (d.span.start.0 as usize, d.message.as_str()))
            .collect();
        for (at, path) in calls {
            let key = format!("{path}@{at}");
            old.push((key.clone(), refused.get(at).map(|m| m.to_string()).unwrap_or_else(|| "lower".to_string())));
            new.push((key, self.std_fact(class, path)));
        }
        // A refusal at a site the walk did not see. Past the source, it
        // is in code the desugar sequence generated (the `api` pass's
        // binding calls `std::http`), whose spans index no text: the
        // refusal names the path, and the cell answers for it. Only the
        // generated calls the gate refuses are visible this way.
        for (at, msg) in &refused {
            if calls.iter().any(|(c, _)| c == at) {
                continue;
            }
            let path = msg.split('`').nth(1).unwrap_or("");
            if *at < src_len || !path.starts_with("std::") {
                old.push((format!("(unwalked)@{at}"), msg.to_string()));
                continue;
            }
            let key = format!("{path}@{at} (generated)");
            old.push((key.clone(), msg.to_string()));
            new.push((key, self.std_fact(class, path)));
        }
        self.compare("stdlib call sites", class, id, old, new);
    }

    /// The cell's answer for one `std::` call path.
    fn std_fact(&self, class: TargetClass, path: &str) -> String {
        match namespace_of(&self.m, path) {
            None => "lower".to_string(),
            Some(ns) => {
                let cell = self.m.behaviour(class, Capability::StdNamespace(ns)).expect("every namespace has a row");
                match cell.refusal() {
                    None => "lower".to_string(),
                    Some(r) => r.render(&cell.witness, &[("path", &path["std::".len()..]), ("selector", "`target wasm`")]),
                }
            }
        }
    }

    /// `wasm_target` (the gate's trigger, which reads the effective
    /// target) against the effective-target row.
    fn trigger(&mut self, id: &str, program: &Program, checked: &Checked, end: usize, configured: &ConfiguredTarget) {
        let fired = checked.diags.iter().any(|d| d.message.contains(GATES) && d.span.start.0 as usize >= end);
        let old = vec![("stdlib gate".to_string(), if fired { "wasm32" } else { "off" }.to_string())];
        let mut programs = BTreeMap::new();
        programs.insert(String::new(), program);
        let mut bundle = Bundle::new(programs);
        bundle.target = configured.clone();
        let row = target_row(&bundle);
        let gate = if row.is_wasm32() { "wasm32" } else { "off" };
        let new = vec![("stdlib gate".to_string(), gate.to_string())];
        let class = TargetClass::of(&configured.spec).expect("a column");
        self.compare("wasm_target", class, id, old, new);
    }

    /// `TargetSpec::has_async_io` against `AsyncIoPool`, per entry.
    fn async_io(&mut self, class: TargetClass, id: &str, checked: &Checked, entries: &[(String, usize)]) {
        let cell = self.m.behaviour(class, Capability::AsyncIoPool).expect("AsyncIoPool has a row");
        let mut old = Vec::new();
        let mut new = Vec::new();
        for (field, at) in entries {
            let key = format!("placement `{field}`@{at}");
            let prefix = format!("placement entry `{field}`: `async_io` pools aren't supported on");
            let o = checked
                .diags
                .iter()
                .find(|d| d.span.start.0 as usize == *at && d.message.starts_with(&prefix))
                .map(|d| d.message.clone())
                .unwrap_or_else(|| "lower".to_string());
            old.push((key.clone(), o));
            let n = match cell.refusal() {
                None => "lower".to_string(),
                Some(r) => r.render(&cell.witness, &[("field", field)]),
            };
            new.push((key, n));
        }
        self.compare("TargetSpec::has_async_io", class, id, old, new);
    }

    /// The checker's FFI type diagnostics against `FfiType`, per
    /// signature position (the row keeps the name of the predicate it
    /// replaced, `ffi_type_unportable`, which P3 3 of 3 deleted: the
    /// checker now reads these cells). The answer reads no target, so
    /// the host's check is its answer on every column; each column's own
    /// cell is held to it.
    fn ffi(&mut self, id: &str, host: &Checked) {
        let mut old = Vec::new();
        let mut new: Vec<(String, FfiTypeClass, Abi)> = Vec::new();
        for item in flat_decls(&host.program.items) {
            let TopDecl::Fn(f) = item else { continue };
            let (attr, abi) = match (&f.ffi, f.export) {
                (Some(a), _) => ("@ffi", Abi::of(&a.abi)),
                (None, true) => ("@export", Some(Abi::C)),
                (None, false) => continue,
            };
            let Some(abi) = abi else { continue };
            let mut positions: Vec<(String, &hale_syntax::ast::TypeExpr, String)> = f
                .params
                .iter()
                .map(|p| {
                    (
                        format!("{attr} `{}` param `{}`", f.name.name, p.name.name),
                        &p.ty,
                        format!("`{attr}` fn `{}` parameter `{}` has type", f.name.name, p.name.name),
                    )
                })
                .collect();
            if let Some(r) = &f.ret {
                positions.push((format!("{attr} `{}` return", f.name.name), r, format!("`{attr}` fn `{}` return type", f.name.name)));
            }
            for (key, te, prefix) in positions {
                let at = te.span().start.0 as usize;
                let key = format!("{key}@{at}");
                let o = host
                    .diags
                    .iter()
                    .find(|d| d.span.start.0 as usize == at && d.message.starts_with(&prefix))
                    .and_then(|d| d.message.split_once(" — ").map(|(_, r)| r.to_string()))
                    .map(|r| {
                        r.trim_end_matches(" (the export is a C-ABI symbol; only the FFI-portable set crosses)")
                            .to_string()
                    })
                    .unwrap_or_else(|| "portable".to_string());
                old.push((key.clone(), o));
                let ty = hale_types::resolve::resolve_type_expr(te, &host.known.names);
                new.push((key, FfiTypeClass::of(&ty), abi));
            }
        }
        for class in TargetClass::ALL {
            let new: Vec<(String, String)> = new
                .iter()
                .map(|(key, t, abi)| {
                    let cell = self.m.behaviour(class, Capability::FfiType(*t, *abi)).expect("every FFI type class has a row");
                    let fact = match cell.refusal() {
                        None => "portable".to_string(),
                        Some(r) => r.render(&cell.witness, &[]),
                    };
                    (key.clone(), fact)
                })
                .collect();
            self.compare("ffi_type_unportable", class, id, old.clone(), new);
        }
    }
}

/// `TargetSpec::has_async_io` per known target against `AsyncIoPool`:
/// the target model's answer, before any program reads it. They agree
/// on the POSIX columns; on wasm32 the runtime has the backend over
/// imports the loader stubs, and the cell refuses it (T2): the one
/// classified difference, which closed the `TargetSpec` legacy row.
#[test]
fn has_async_io_agrees_with_the_async_io_cell() {
    let m = derive_capability_matrix();
    for t in TargetSpec::known() {
        let Some(class) = TargetClass::of(&t) else { continue };
        let cell = m.behaviour(class, Capability::AsyncIoPool).unwrap();
        let expected = t.has_async_io() && class != TargetClass::Wasm32;
        assert_eq!(expected, cell.is_lower(), "{}: has_async_io vs AsyncIoPool × {}", t.triple, class.name());
    }
}

#[test]
fn every_legacy_checker_row_agrees_with_its_cell_or_is_classified() {
    let mut shadow = Shadow {
        m: derive_capability_matrix(),
        reports: BTreeMap::new(),
        refused: BTreeMap::new(),
        wasm_programs: BTreeMap::new(),
    };
    let all = programs();
    for p in &all {
        shadow.one(&p.origin, &p.source);
    }
    assert!(all.len() > 1000, "the program walk is vacuous ({} programs)", all.len());

    let mut merged = Report::new("target_capability");
    let mut summary = String::new();
    let mut tallies: BTreeMap<(&str, &str), Tally> = BTreeMap::new();
    for ((row, class), r) in &shadow.reports {
        let t = tallies.entry((row, class)).or_default();
        t.rows += r.rows_compared;
        t.divergent += r.divergences.len();
        merged.programs += r.programs;
        merged.rows_compared += r.rows_compared;
        merged.divergences.extend(r.divergences.iter().cloned());
    }
    for ((row, class), t) in &tallies {
        let refused = shadow.refused.get(&(*row, *class)).copied().unwrap_or(0);
        summary.push_str(&format!(
            "  {row} × {class}: {} rows ({refused} refused by both), {} divergent\n",
            t.rows, t.divergent
        ));
    }
    let wasm: Vec<String> = shadow.wasm_programs.iter().map(|(o, n)| format!("{o} {n}")).collect();
    eprintln!(
        "shadow target_capability (checker): {} programs; the design's wasm set: {}\n{summary}",
        all.len(),
        wasm.join(", ")
    );

    let path = fixture_path();
    let existing = std::fs::read_to_string(&path)
        .ok()
        .map(|t| parse_fixture(&t).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        .unwrap_or_default();
    if std::env::var("HALE_SHADOW_REGEN").as_deref() == Ok("1") {
        std::fs::write(&path, merged.render_fixture(&existing)).expect("write fixture");
        eprintln!("{}", merged.render());
        return;
    }
    let (unexplained, stale) = merged.explain(&existing);
    assert!(
        unexplained.is_empty() && stale.is_empty(),
        "{}",
        gate_message(&merged, &unexplained, &stale, "crates/hale-types/tests/fixtures/shadow_capability.txt")
    );
}
