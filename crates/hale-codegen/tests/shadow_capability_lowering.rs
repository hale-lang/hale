//! The capability matrix's shadow, lowering half (F.40 phase 3, P3 1
//! of 3): the codegen legacy rows run beside their cells.
//!
//! - `is_wasm` and `lotus_replay_start_ingress` (the obligations): each
//!   program is built for the host and for wasm32 with its IR dumped,
//!   and each obligation's runtime calls are looked up in both modules.
//!   Where the host module makes one, wasm32's answer is observed (made:
//!   `Emit`; not made: `Omit`) and held to the obligation's wasm32 cell.
//!   A module-level observation: it sees which spines a target emits a
//!   call in only through whether any does.
//! - `link_wasm`: the `[ffi] link` refusal, by building with a link
//!   library on both targets, against `LinkLibrary`; and the export
//!   list, read from each wasm module's export section, against
//!   `ExportSurface`'s lowering data and the program's `@export` set.
//! - the entry inversion (`has_exports`, the `@export locus` `run()`
//!   refusal): each program's build outcome on both targets against
//!   `EntryInversion`.
//! - the thread behaviours, which no legacy row states (design §2.5):
//!   each program's build outcome on both targets against
//!   `PinnedThreads`, `PoolThreads` and `RemoteTransport(Adapter)`. On
//!   wasm32 the first and last are a late refusal today, wasm-ld's
//!   `pthread_join` signature mismatch.
//! - `@ffi("js")`: each declaring program's build outcome on both
//!   targets against `ForeignAbi(Js)`; a native build that calls one
//!   fails at the link (design §2.6, T5).
//!
//! A build that fails for a cause none of these rows compares fails the
//! test: every refusal a program meets is some cell's.
//! - the site inventory: every line of codegen that reads the target's
//!   wasm-ness or `has_async_io` is listed with what it decides (a
//!   backend choice, which is no cell; the link; a behaviour; or the
//!   obligations it emits), so a new site fails until it is classified.
//!
//! The programs: the design's wasm set (§2.9) as the corpus reaches it,
//! `wasm-flower` and the playground; a sample of the corpus by the
//! features that reach each obligation's sites; and probes for the
//! refusals no program in the tree makes.
//!
//! The gate: every divergence is classified in
//! `fixtures/shadow_capability_lowering.txt`. Regenerate with
//! `HALE_SHADOW_REGEN=1`, then classify by hand. Skipped, naming the
//! missing tool, when clang or wasm-ld is absent.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use hale_codegen::{build_executable_with_options, BuildOptions, CodegenError, CompileTarget};
use hale_graph::shadow::{gate_message, parse_fixture, program_id, Report};
use hale_syntax::ast::{flat_decls, LocusMember, PlacementSpec, Program, TopDecl, TransportSpec};
use hale_types::capability::{
    derive_capability_matrix, Abi, Capability, CapabilityMatrix, Inversion, Obligation, TargetClass, Transport,
    WASM_FIXED_EXPORTS,
};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/wasm_module.rs"]
mod wasm_module;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/shadow_capability_lowering.txt")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tool(name: &str) -> bool {
    [name.to_string(), format!("{name}-18")]
        .iter()
        .any(|c| Command::new(c).arg("--version").output().is_ok())
}

/// The runtime calls (or the global, for the drain observer) by which
/// a module shows an obligation emitted.
fn symbols(o: Obligation) -> &'static [&'static str] {
    match o {
        Obligation::ReplayIngress => &["lotus_replay_start_ingress"],
        Obligation::ObservationIdentity => &["lotus_obs_eager_init", "lotus_obs_topic_shape", "lotus_obs_exec_digest_set"],
        Obligation::SignalInstall => &["lotus_io_init", "lotus_drain_signals_install"],
        Obligation::DrainObserver => &["lotus.observes_drain."],
        Obligation::DrainTerm => &["lotus_process_draining_flag"],
        Obligation::BindingConfig => &["lotus_bus_load_config", "lotus_bus_register_key_extractor"],
        Obligation::PoolJoin => &["lotus_coop_pool_shutdown_all"],
        Obligation::WaitAbort => &["lotus_bus_wait_abort_all"],
        Obligation::IngressQuiesce => &["lotus_bus_ingress_quiesce"],
    }
}

/// Whether the module uses the symbol: a line that names it and is
/// neither its declaration nor an external global's.
fn uses(ir: &str, sym: &str) -> bool {
    ir.lines().any(|l| {
        let t = l.trim_start();
        t.contains(sym) && !t.starts_with("declare ") && !(t.starts_with('@') && t.contains("= external"))
    })
}

/// One build: its pre-optimization IR, the module's exports (wasm32),
/// or the refusal codegen returned.
struct Built {
    ir: String,
    exports: Option<Vec<String>>,
    err: Option<CodegenError>,
    /// What the import backstop said of a wasm32 module that built.
    backstop: Option<String>,
}

fn build(program: &Program, target: CompileTarget, link: &[&str], name: &str, origin: &str) -> Built {
    let bin = harness::unique_bin(name);
    let out = if target == CompileTarget::Wasm32 { bin.with_extension("wasm") } else { bin.clone() };
    let ll = bin.with_extension("ll");
    let options = BuildOptions {
        target,
        dump_ir: Some(ll.clone()),
        link_libs: link.iter().map(|s| s.to_string()).collect(),
        ..build_opts::options()
    };
    let err = build_executable_with_options(program, &out, &[], &options).err();
    let ir = std::fs::read_to_string(&ll).unwrap_or_default();
    let built_wasm = target == CompileTarget::Wasm32 && err.is_none();
    let exports = built_wasm.then(|| std::fs::read(&out).ok().and_then(|b| wasm_module::exports(&b))).flatten();
    let backstop = if built_wasm { wasm_module::backstop(origin, program, &out).err() } else { None };
    for p in [&out, &ll, &out.with_extension("mjs")] {
        let _ = std::fs::remove_file(p);
    }
    Built { ir, exports, err, backstop }
}

/// A codegen refusal's own text: what a cell's wording is held to.
fn refusal_text(e: &CodegenError) -> String {
    match e {
        CodegenError::Unsupported(s) | CodegenError::UnsupportedAt(s, _) | CodegenError::Link(s) => s.clone(),
        other => other.to_string(),
    }
}

const WASM_ORIGINS: &[&str] = &[
    "crates/hale-types/tests/wasm_target_gating.rs",
    "crates/hale-codegen/tests/wasm_target.rs",
    "crates/hale-cli/tests/wasm_package_csrc.rs",
    "crates/hale-cli/tests/target_model.rs",
    "crates/hale-cli/tests/build_output_path.rs",
    "crates/hale-cli/tests/wasm_link_is_quiet.rs",
    "crates/hale-cli/tests/check_arg_parsing.rs",
    "crates/hale-syntax/tests/wrap_main.rs",
];

/// The features that reach each obligation's sites, and the most
/// programs per feature the sample takes.
const FEATURES: &[&str] = &[
    "main locus",
    "cooperative(pool",
    "self.draining",
    "restart",
    "std::time::sleep",
    "or wait",
    "bindings {",
    "pinned",
];
const PER_FEATURE: usize = 3;

const EXPORT_ONLY_PROBE: &str = "@export fn probe_answer() -> Int {\n    return 42;\n}\n";
const EXPORTED_RUN_PROBE: &str = "@export locus Probe {\n    params {\n        n: Int = 0;\n    }\n\n    fn bump() -> Int {\n        return self.n + 1;\n    }\n\n    fn run() {\n    }\n}\n\nfn main() {\n}\n";
const LINK_PROBE: &str = "fn main() {\n    println(\"linked\");\n}\n";

fn programs() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let corpus = hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok());
    for p in &corpus {
        if WASM_ORIGINS.iter().any(|o| p.origin.starts_with(o)) {
            out.push((p.origin.clone(), p.source.clone()));
        }
    }
    let mut disk = vec![repo_root().join("iris/examples/wasm-flower/flower.hl"), repo_root().join("play/ui.hl")];
    if let Ok(entries) = std::fs::read_dir(repo_root().join("play/examples")) {
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "hl")).collect();
        files.sort();
        disk.extend(files);
    }
    for path in disk {
        if let Ok(src) = std::fs::read_to_string(&path) {
            let origin = path.strip_prefix(repo_root()).unwrap_or(&path).display().to_string();
            out.push((origin, src));
        }
    }
    for feature in FEATURES {
        let mut taken = 0;
        for p in &corpus {
            if taken == PER_FEATURE {
                break;
            }
            if p.source.contains(feature) && p.source.contains("fn main") && !out.iter().any(|(_, s)| *s == p.source) {
                out.push((p.origin.clone(), p.source.clone()));
                taken += 1;
            }
        }
    }
    for (name, src) in [("export-only", EXPORT_ONLY_PROBE), ("exported-locus-run", EXPORTED_RUN_PROBE)] {
        out.push((format!("crates/hale-codegen/tests/shadow_capability_lowering.rs#{name}-probe"), src.to_string()));
    }
    let mut seen = BTreeSet::new();
    out.retain(|(_, s)| seen.insert(s.clone()));
    out
}

/// The program's `@export` set: its `@export fn`s, and for an `@export
/// locus` `_hale_start` and its methods that are not fallible.
fn export_set(p: &Program) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for item in flat_decls(&p.items) {
        match item {
            TopDecl::Fn(f) if f.export => {
                out.insert(f.name.name.clone());
            }
            TopDecl::Locus(l) if l.export => {
                out.insert("_hale_start".to_string());
                for m in &l.members {
                    if let LocusMember::Fn(f) = m {
                        if f.fallible.is_none() {
                            out.insert(f.name.name.clone());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn inversion(p: &Program) -> Option<(Inversion, String)> {
    let has_main = flat_decls(&p.items).any(|i| matches!(i, TopDecl::Fn(f) if f.name.name == "main"));
    for item in flat_decls(&p.items) {
        if let TopDecl::Locus(l) = item {
            let runs = l.members.iter().any(|m| matches!(m, LocusMember::Fn(f) if f.name.name == "run"));
            if l.export && runs {
                return Some((Inversion::ExportedLocusRun, l.name.name.clone()));
            }
        }
    }
    let exports = flat_decls(&p.items).any(|i| matches!(i, TopDecl::Fn(f) if f.export) || matches!(i, TopDecl::Locus(l) if l.export));
    (exports && !has_main).then(|| (Inversion::ExportOnly, String::new()))
}

/// The thread and binding behaviours a program asks for, as written: a
/// `pinned` entry or an adapter binding asks for `PinnedThreads` (an
/// adapter's instance runs on its own thread), an adapter binding for
/// `RemoteTransport(Adapter)`, a pool other than `main` for
/// `PoolThreads`. A spelling walk, for the shadow only: P3 2 of 3's use
/// producer derives them from the resolved graph.
fn thread_uses(p: &Program) -> BTreeSet<Capability> {
    let mut out = BTreeSet::new();
    for item in flat_decls(&p.items) {
        let TopDecl::Locus(l) = item else { continue };
        for m in &l.members {
            match m {
                LocusMember::Placement(b) => {
                    for e in &b.entries {
                        match &e.spec {
                            PlacementSpec::Pinned { .. } => {
                                out.insert(Capability::PinnedThreads);
                            }
                            PlacementSpec::Cooperative { pool: Some(pool), .. } if pool.name != "main" => {
                                out.insert(Capability::PoolThreads);
                            }
                            _ => {}
                        }
                    }
                }
                LocusMember::Bindings(b) => {
                    for e in &b.entries {
                        if matches!(e.transport, TransportSpec::Adapter { .. }) {
                            out.insert(Capability::PinnedThreads);
                            out.insert(Capability::RemoteTransport(Transport::Adapter));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// What a build says about the thread behaviours: `lower` when it
/// built; the refusal's text when it failed at the link and the module
/// calls `pthread_join` (the mismatch wasm-ld refuses: the cause is
/// the call, which is only in the IR, since wasm-ld's own error goes to
/// stderr); the refusal's text and what the module lacks otherwise.
fn thread_outcome(b: &Built) -> String {
    match &b.err {
        None => "lower".to_string(),
        Some(e @ CodegenError::Link(_)) if uses(&b.ir, "@pthread_join(") => refusal_text(e),
        Some(e) => format!("{} (and the module calls no pthread_join)", refusal_text(e)),
    }
}

struct Shadow {
    m: CapabilityMatrix,
    reports: BTreeMap<(&'static str, &'static str), Report>,
    failed: Vec<String>,
    /// Rows both producers answer with a refusal, per (row, column).
    refused: BTreeMap<(&'static str, &'static str), usize>,
    /// Programs whose build failed for a cause no compared row names.
    unattributed: Vec<String>,
    /// Programs whose wasm32 build the admission refused before lowering.
    admission_refused: Vec<String>,
}

impl Shadow {
    fn compare(&mut self, row: &'static str, class: TargetClass, id: &str, old: Vec<(String, String)>, new: Vec<(String, String)>) {
        if old.is_empty() && new.is_empty() {
            return;
        }
        let refusals = old
            .iter()
            .filter(|(k, v)| !matches!(v.as_str(), "lower" | "Emit" | "Omit" | "exported") && new.iter().any(|(nk, nv)| nk == k && nv == v))
            .count();
        *self.refused.entry((row, class.name())).or_default() += refusals;
        let tag = format!("{} {row}", class.name());
        self.reports
            .entry((row, class.name()))
            .or_insert_with(|| Report::new(&format!("target_capability · {row} · {}", class.name())))
            .compare_rows(id, &old, &new, |k| Some(format!("{tag} {k}")), |k| Some(format!("{tag} {k}")), |_| Vec::new(), |_| Vec::new());
    }

    fn behaviour_fact(&self, class: TargetClass, cap: Capability, holes: &[(&str, &str)]) -> String {
        let cell = self.m.behaviour(class, cap).expect("every capability has a row");
        match cell.refusal() {
            None => "lower".to_string(),
            Some(r) => r.render(&cell.witness, holes),
        }
    }

    fn one(&mut self, origin: &str, src: &str, host: &Built, wasm: &Built, program: &Program) {
        let id = program_id(origin, src);
        // A build the admission refused (P3 2 of 3: a harness build reads
        // the cells before lowering) stops before anything a row below
        // observes, unless the refusal is that row's own.
        let admitted = |b: &Built| !matches!(b.err, Some(CodegenError::CapabilityRefused(..)));
        // The entry inversion: the build's outcome on each target.
        if let Some((form, locus)) = inversion(program) {
            for (class, built) in [(TargetClass::PosixAsync, host), (TargetClass::Wasm32, wasm)] {
                let old = match &built.err {
                    None => "lower".to_string(),
                    Some(e) => refusal_text(e),
                };
                let new = self.behaviour_fact(class, Capability::EntryInversion(form), &[("locus", &locus)]);
                if !admitted(built) && old != new {
                    continue;
                }
                let key = format!("EntryInversion({})", form.name());
                self.compare("entry inversion", class, &id, vec![(key.clone(), old)], vec![(key, new)]);
            }
        }
        // The thread behaviours: the build's outcome on each target, for
        // each one the program asks for. A pool is observed only where no
        // owned thread is asked for too, since a refusal of the latter
        // would hide the former's answer.
        let wanted = thread_uses(program);
        for (class, built) in [(TargetClass::PosixAsync, host), (TargetClass::Wasm32, wasm)] {
            if !admitted(built) {
                continue;
            }
            let observed = thread_outcome(built);
            let mut old = Vec::new();
            let mut new = Vec::new();
            for cap in &wanted {
                if *cap == Capability::PoolThreads && wanted.contains(&Capability::PinnedThreads) {
                    continue;
                }
                old.push((cap.label(), observed.clone()));
                new.push((cap.label(), self.behaviour_fact(class, *cap, &[])));
            }
            self.compare("threads and bindings", class, &id, old, new);
        }
        // `@ffi("js")`: the build's outcome on each target, against
        // `ForeignAbi(Js)`. A native build declares the fn as an
        // external symbol, so a call to it fails at the native link.
        let js: Vec<String> = flat_decls(&program.items)
            .filter_map(|i| match i {
                TopDecl::Fn(f) if f.ffi.as_ref().is_some_and(|a| a.abi == "js") => Some(f.name.name.clone()),
                _ => None,
            })
            .collect();
        let calls_js = |b: &Built| js.iter().any(|n| uses(&b.ir, &format!("@{n}(")));
        let form = inversion(program).map(|(f, _)| f);
        if !js.is_empty() {
            for (class, built) in [(TargetClass::PosixAsync, host), (TargetClass::Wasm32, wasm)] {
                // An export-only program's native build stops at its
                // entry, which the entry inversion's row compares, and
                // hides this answer.
                if built.err.is_some() && !calls_js(built) && (form.is_some() || !admitted(built)) {
                    continue;
                }
                let old = match &built.err {
                    None => "lower".to_string(),
                    Some(e) if calls_js(built) => refusal_text(e),
                    Some(e) => format!("{} (and the module calls no `@ffi(\"js\")` fn)", refusal_text(e)),
                };
                let new = self.behaviour_fact(
                    class,
                    Capability::ForeignAbi(Abi::Js),
                    // This shadow compares the native capability class;
                    // its fixture is shared by Linux and macOS hosts.
                    &[("fn", &js[0]), ("selector", "the native target")],
                );
                self.compare("foreign abi", class, &id, vec![("ForeignAbi(Js)".to_string(), old)], vec![("ForeignAbi(Js)".to_string(), new)]);
            }
        }
        // Every failed build is one a row above compares, or the
        // admission's: the entry inversion's refusal, the late
        // `pthread_join` refusal of an owned thread, the native link of an
        // `@ffi("js")` call, or a use whose cell is `Reject`. Any other is
        // a cause the cells do not name.
        let host_explained = host.err.is_none() || form.is_some() || calls_js(host) || !admitted(host);
        let wasm_explained = wasm.err.is_none()
            || form.is_some()
            || !admitted(wasm)
            || (wanted.contains(&Capability::PinnedThreads) && uses(&wasm.ir, "@pthread_join("));
        if !admitted(wasm) {
            self.admission_refused.push(origin.to_string());
        }
        if !(host_explained && wasm_explained) {
            self.unattributed.push(origin.to_string());
        }
        let (Some(_), Some(_)) = (host.err.as_ref().map_or(Some(()), |_| None), wasm.err.as_ref().map_or(Some(()), |_| None)) else {
            self.failed.push(format!(
                "{origin}: {}",
                [("host", &host.err), ("wasm32", &wasm.err)]
                    .iter()
                    .filter_map(|(t, e)| e.as_ref().map(|e| format!("{t}: {}", refusal_text(e).lines().next().unwrap_or(""))))
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
            return;
        };
        // The obligations, wherever the host module makes the call.
        let mut old = Vec::new();
        let mut new = Vec::new();
        let mut replay_old = Vec::new();
        let mut replay_new = Vec::new();
        for o in Obligation::ALL {
            if !symbols(o).iter().any(|s| uses(&host.ir, s)) {
                continue;
            }
            let observed = if symbols(o).iter().any(|s| uses(&wasm.ir, s)) { "Emit" } else { "Omit" };
            let cell = self.m.obligation(TargetClass::Wasm32, o).expect("every obligation has a row");
            let stated = if cell.emits() { "Emit" } else { "Omit" };
            let host_cell = self.m.obligation(TargetClass::PosixAsync, o).expect("every obligation has a row");
            let key = o.name();
            let (o_rows, n_rows) = if o == Obligation::ReplayIngress { (&mut replay_old, &mut replay_new) } else { (&mut old, &mut new) };
            o_rows.push((format!("{key} on wasm32"), observed.to_string()));
            n_rows.push((format!("{key} on wasm32"), stated.to_string()));
            o_rows.push((format!("{key} on the host"), "Emit".to_string()));
            n_rows.push((format!("{key} on the host"), if host_cell.emits() { "Emit" } else { "Omit" }.to_string()));
        }
        self.compare("is_wasm", TargetClass::Wasm32, &id, old, new);
        self.compare("lotus_replay_start_ingress", TargetClass::Wasm32, &id, replay_old, replay_new);
        // The export list, against the cell's fixed exports and the
        // program's `@export` set.
        if let Some(actual) = &wasm.exports {
            let set = export_set(program);
            let defines_main = wasm.ir.lines().any(|l| l.starts_with("define") && l.contains("@main("));
            let mut old = Vec::new();
            let mut new = Vec::new();
            for name in actual {
                old.push((name.clone(), "exported".to_string()));
            }
            for e in WASM_FIXED_EXPORTS {
                let present = !e.if_defined || e.name == "memory" || (e.name == "main" && defines_main);
                if present {
                    new.push((e.name.to_string(), "exported".to_string()));
                }
            }
            for name in &set {
                if actual.contains(name) || !new.iter().any(|(n, _)| n == name) {
                    new.push((name.clone(), "exported".to_string()));
                }
            }
            new.sort();
            new.dedup();
            self.compare("link_wasm (exports)", TargetClass::Wasm32, &id, old, new);
        }
    }
}

#[test]
fn every_legacy_lowering_row_agrees_with_its_cell_or_is_classified() {
    if !(tool("clang") && tool("wasm-ld")) {
        eprintln!("SKIP: the wasm32 builds need clang and wasm-ld, and one is missing");
        return;
    }
    let all = programs();
    let parsed: Vec<(String, String, Program)> = all
        .into_iter()
        .filter_map(|(o, s)| hale_syntax::parse_source(&s).ok().map(|p| (o, s, p)))
        .collect();
    // Build on both targets, a few programs at a time.
    let built: Vec<(Built, Built)> = std::thread::scope(|scope| {
        let chunks: Vec<_> = parsed
            .chunks(parsed.len().div_ceil(8).max(1))
            .enumerate()
            .map(|(c, chunk)| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .enumerate()
                        .map(|(i, (o, _, p))| {
                            let name = format!("hale_shadow_cap_{c}_{i}");
                            (build(p, CompileTarget::Native, &[], &name, o), build(p, CompileTarget::Wasm32, &[], &name, o))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        chunks.into_iter().flat_map(|h| h.join().expect("a build thread")).collect()
    });
    let mut shadow = Shadow {
        m: derive_capability_matrix(),
        reports: BTreeMap::new(),
        failed: Vec::new(),
        refused: BTreeMap::new(),
        unattributed: Vec::new(),
        admission_refused: Vec::new(),
    };
    for ((origin, src, program), (host, wasm)) in parsed.iter().zip(&built) {
        shadow.one(origin, src, host, wasm, program);
    }
    let backstop: Vec<&String> = built.iter().filter_map(|(_, w)| w.backstop.as_ref()).collect();
    assert!(backstop.is_empty(), "the import backstop:\n{}", backstop.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"));

    // `[ffi] link`: one program, with a link library, on each target.
    let link_probe = hale_syntax::parse_source(LINK_PROBE).unwrap();
    let id = program_id("crates/hale-codegen/tests/shadow_capability_lowering.rs#link-probe", LINK_PROBE);
    for (class, target) in [(TargetClass::PosixAsync, CompileTarget::Native), (TargetClass::Wasm32, CompileTarget::Wasm32)] {
        let b = build(&link_probe, target, &["m"], "hale_shadow_cap_link", "shadow_capability_lowering#link-probe");
        let old = b.err.as_ref().map(refusal_text).unwrap_or_else(|| "lower".to_string());
        let new = shadow.behaviour_fact(class, Capability::LinkLibrary, &[("libs", "[\"m\"]")]);
        shadow.compare("link_wasm (refusal)", class, &id, vec![("[ffi] link".to_string(), old)], vec![("[ffi] link".to_string(), new)]);
    }

    let mut merged = Report::new("target_capability");
    let mut summary = String::new();
    for ((row, class), r) in &shadow.reports {
        let refused = shadow.refused.get(&(*row, *class)).copied().unwrap_or(0);
        summary.push_str(&format!(
            "  {row} × {class}: {} programs, {} rows ({refused} refused by both), {} divergent\n",
            r.programs,
            r.rows_compared,
            r.divergences.len()
        ));
        merged.programs += r.programs;
        merged.rows_compared += r.rows_compared;
        merged.divergences.extend(r.divergences.iter().cloned());
    }
    eprintln!(
        "shadow target_capability (lowering): {} programs, {} built on both targets\n{summary}not built on both (no obligation or export rows):\n  {}\nrefused by the admission on wasm32:\n  {}",
        parsed.len(),
        parsed.len() - shadow.failed.len(),
        shadow.failed.join("\n  "),
        shadow.admission_refused.join("\n  ")
    );
    // 18 since T2 and T3 (P3 2 of 3): the sample is chosen by the
    // features a spine reads (pools, `pinned`, `sleep`, bindings), and
    // wasm32 now refuses those at the check (it was 23, the rest built
    // never to run).
    assert!(parsed.len() - shadow.failed.len() >= 15, "the lowering shadow built too few programs on both targets");
    assert!(
        shadow.unattributed.is_empty(),
        "a build failed for a cause no cell names (not the entry inversion, an owned thread's \
         `pthread_join` or an `@ffi(\"js\")` call):\n  {}",
        shadow.unattributed.join("\n  ")
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
        gate_message(&merged, &unexplained, &stale, "crates/hale-codegen/tests/fixtures/shadow_capability_lowering.txt")
    );
}

/// What a line of codegen that reads the target's wasm-ness decides.
/// Since P3 3 of 3 only an emission choice and the link path do: every
/// behaviour and obligation is a read of the lowering view's cells
/// ([`CELL_READS`]).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Reads {
    /// An emission choice: a `TargetSpec` query, never a cell.
    Backend,
    /// The choice of link path; the export list inside it is
    /// `ExportSurface`'s lowering data.
    Link,
}

const CG: &str = "crates/hale-codegen/src/codegen.rs";
const INST: &str = "crates/hale-codegen/src/locus/instantiation.rs";

/// Every line of codegen that reads the target's wasm-ness or its
/// `has_async_io`, by the line and the first line of code after it.
const SITES: &[(&str, &str, Reads)] = &[
    (CG, "let is_wasm = target_spec.is_wasm(); ⏎ let is_foreign = options.target.is_foreign();", Reads::Backend),
    (CG, "if is_wasm { ⏎ Target::initialize_webassembly(&InitializationConfig::default());", Reads::Backend),
    (CG, "let triple = if is_wasm { ⏎ TargetTriple::create(\"wasm32-unknown-unknown\")", Reads::Backend),
    (CG, "let (cpu, features): (String, String) = if is_wasm { ⏎ (\"generic\".to_string(), String::new())", Reads::Backend),
    (CG, "let opt_level = if is_wasm { ⏎ OptimizationLevel::Default", Reads::Backend),
    (CG, "lifecycle_trace: options.lifecycle_trace && !is_wasm && !is_foreign, ⏎ lc_spine: \"Instantiation\",", Reads::Backend),
    (CG, "if !is_wasm { ⏎ if let Some(dbg) = &options.debug {", Reads::Backend),
    (CG, "let lto_kind = if is_wasm || is_foreign || sanitized { ⏎ LtoMode::Off", Reads::Backend),
    (CG, "if !is_wasm { ⏎ let cpu_attr = cx.context.create_string_attribute(\"target-cpu\", &cpu);", Reads::Backend),
    (CG, "if !is_wasm { ⏎ let mut f = cx.module.get_first_function();", Reads::Backend),
    (CG, "let pass_pipeline = if is_wasm { ⏎ \"default<O2>\"", Reads::Backend),
    (CG, "if is_wasm { ⏎ let Some(hale_types::capability::Lowering::Exports(fixed)) =", Reads::Link),
    (CG, "if !target_spec.is_wasm() && !target_spec.is_macos() { ⏎ match options.target_cpu {", Reads::Backend),
];

/// Every read of the lowering view's cells in codegen: the file, the
/// behaviour or obligation read, and how many lines read it. A new read
/// fails here until it is listed.
const CELL_READS: &[(&str, &str, usize)] = &[
    (CG, "Capability::LinkLibrary", 1),
    (CG, "Capability::ExportSurface", 2),
    (CG, "Capability::EntryInversion(Inversion::ExportOnly)", 1),
    (CG, "Capability::AsyncIoPool", 1),
    (CG, "Capability::ForeignAbi(Abi::Js)", 1),
    (CG, "Obligation::SignalInstall", 2),
    // The identity setters and eager init in the prelude, and the
    // observation probes' gate (`obs_live_check`, P3 T7).
    (CG, "Obligation::ObservationIdentity", 3),
    (CG, "Obligation::BindingConfig", 2),
    (CG, "Obligation::DrainObserver", 1),
    // The five spines' heads and the frame teardown's wait-abort, all
    // through the plan's steps in `emit_teardown_obligations`.
    (CG, "Obligation::IngressQuiesce", 1),
    (CG, "Obligation::PoolJoin", 1),
    (CG, "Obligation::WaitAbort", 1),
    (INST, "Obligation::ReplayIngress", 1),
    (INST, "Capability::PoolThreads", 1),
    ("crates/hale-codegen/src/locus/dissolve.rs", "Obligation::DrainTerm", 1),
    ("crates/hale-codegen/src/locus/restart.rs", "Obligation::DrainTerm", 1),
    ("crates/hale-codegen/src/stdlib/time.rs", "Obligation::DrainTerm", 1),
];

fn expected_count(_file: &str, _print: &str) -> usize {
    1
}

/// The behaviour or obligation a line reads off the cells, as
/// `Capability::…` or `Obligation::…` with the crate path dropped.
fn cell_read(line: &str) -> Option<String> {
    let t = line.trim();
    if t.starts_with("//") {
        return None;
    }
    let reads = ["cells.emits(", "cells.behaviour(", "cells.lowering("].iter().any(|p| t.contains(p))
        || [".emits(", ".behaviour(", ".lowering("].iter().any(|p| t.starts_with(p));
    if !reads {
        return None;
    }
    let at = t.find("Obligation::").or_else(|| t.find("Capability::"))?;
    let mut depth = 0i32;
    let mut out = String::new();
    for c in t[at..].chars() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => break,
            ')' => depth -= 1,
            c if c.is_alphanumeric() || c == '_' || c == ':' => {}
            _ => break,
        }
        out.push(c);
    }
    Some(out.replace("hale_types::capability::", ""))
}
/// The fingerprint of a site: its trimmed line and the first line of
/// code after it.
fn fingerprints(path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim();
        if t.starts_with("//") || !(t.contains("is_wasm") || t.contains("has_async_io()")) {
            continue;
        }
        let next = lines[i + 1..]
            .iter()
            .map(|l| l.trim())
            .find(|l| !l.is_empty() && !l.starts_with("//"))
            .unwrap_or("");
        out.push(format!("{t} ⏎ {next}"));
    }
    out
}

/// Every site that reads the target is classified, and every
/// classified site exists: a new `is_wasm` read fails here until it
/// says which cell (or which backend choice) it is.
#[test]
fn every_site_that_reads_the_target_is_classified() {
    let src = repo_root().join("crates/hale-codegen/src");
    let mut files = Vec::new();
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    walk(&src, &mut files);
    files.sort();
    let mut found: BTreeMap<(String, String), usize> = BTreeMap::new();
    for f in &files {
        let rel = f.strip_prefix(repo_root()).unwrap().display().to_string().replace("crates/hale-codegen/../../", "");
        let rel = rel.trim_start_matches("./").to_string();
        for print in fingerprints(f) {
            *found.entry((rel.clone(), print)).or_default() += 1;
        }
    }
    let listed: BTreeMap<(String, String), usize> =
        SITES.iter().map(|(f, p, _)| ((f.to_string(), p.to_string()), expected_count(f, p))).collect();
    let unlisted: Vec<String> = found
        .iter()
        .filter(|(k, n)| listed.get(*k) != Some(*n))
        .map(|((f, p), n)| format!("{f}: {p} (×{n})"))
        .collect();
    let missing: Vec<String> = listed
        .iter()
        .filter(|(k, n)| found.get(*k) != Some(*n))
        .map(|((f, p), n)| format!("{f}: {p} (×{n})"))
        .collect();
    assert!(
        unlisted.is_empty() && missing.is_empty(),
        "sites that read the target and are not classified:\n  {}\nclassified sites not found:\n  {}",
        unlisted.join("\n  "),
        missing.join("\n  ")
    );
    assert!(SITES.iter().all(|(_, _, r)| matches!(r, Reads::Backend | Reads::Link)));
    // Every behaviour and obligation is read off the cells, each read
    // listed; every obligation is read somewhere.
    let mut reads: BTreeMap<(String, String), usize> = BTreeMap::new();
    for f in &files {
        let rel = f.strip_prefix(repo_root()).unwrap().display().to_string().replace("crates/hale-codegen/../../", "");
        let text = std::fs::read_to_string(f).unwrap();
        for l in text.lines() {
            if let Some(r) = cell_read(l) {
                *reads.entry((rel.clone(), r)).or_default() += 1;
            }
        }
    }
    let listed: BTreeMap<(String, String), usize> =
        CELL_READS.iter().map(|(f, r, n)| ((f.to_string(), r.to_string()), *n)).collect();
    assert_eq!(reads, listed, "the cell reads in codegen and the listed ones differ");
    let read: BTreeSet<&str> = CELL_READS.iter().map(|(_, r, _)| *r).collect();
    for o in Obligation::ALL {
        assert!(read.contains(format!("Obligation::{}", o.name()).as_str()), "no site reads {}", o.name());
    }
}
