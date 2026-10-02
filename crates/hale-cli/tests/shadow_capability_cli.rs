//! The capability matrix's shadow, CLI half (F.40 phase 3, P3 1 of 3):
//! the refusals the command line spells for itself, run beside the
//! cells that state them.
//!
//! - the invocations (`parse_exec_build_options`): `hale run`,
//!   `hale replay` and a recording run (`hale run` under
//!   `LOTUS_OBS_RECORD`), on the host, a musl triple and wasm32,
//!   against the `Run`, `Replay` and `Record` cells, wording included.
//!   The host's are observed by doing them: the run exits 0, the
//!   recording is written, the replay of it exits 0.
//! - `--wrap-main` (`verbs/build.rs`'s string match): `hale build
//!   --wrap-main` with no target, a musl triple and each wasm32
//!   spelling, against `EntryInversion(WrapMain)`. The wasm32 builds
//!   need clang and wasm-ld, and are left out, named, when one is
//!   missing.
//!
//! The checker's rows are shadowed in
//! `crates/hale-types/tests/shadow_capability.rs`, codegen's in
//! `crates/hale-codegen/tests/shadow_capability_lowering.rs`.
//!
//! The gate: every divergence is classified in
//! `fixtures/shadow_capability_cli.txt`. Regenerate with
//! `HALE_SHADOW_REGEN=1`, then classify by hand.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use hale_graph::shadow::{gate_message, parse_fixture, Report};
use hale_types::capability::{
    derive_capability_matrix, Capability, CapabilityMatrix, Invocation, InvocationVerdict, Inversion, TargetClass,
};
use hale_types::target::TargetSpec;

#[path = "support/vault.rs"]
mod vault;

const PROGRAM: &str = "fn main() {\n    println(\"ran\");\n}\n";
const MUSL: &str = "x86_64-unknown-linux-musl";

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/shadow_capability_cli.txt")
}

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_shadow_capability_cli_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("scratch dir");
    d
}

fn tool(name: &str) -> bool {
    [name.to_string(), format!("{name}-18")]
        .iter()
        .any(|c| Command::new(c).arg("--version").output().is_ok())
}

/// A refusal's text as the CLI prints it: its first line, without the
/// `error: ` the build verb puts before it.
fn refusal(out: &Output) -> String {
    let err = String::from_utf8_lossy(&out.stderr);
    let first = err.lines().next().unwrap_or("").trim();
    first.strip_prefix("error: ").unwrap_or(first).to_string()
}

/// `allowed` when the command did what it was asked, its refusal
/// otherwise.
fn outcome(out: &Output, did: bool) -> String {
    if out.status.success() && did {
        "allowed".to_string()
    } else {
        refusal(out)
    }
}

struct Shadow {
    m: CapabilityMatrix,
    reports: BTreeMap<(&'static str, &'static str), Report>,
    refused: BTreeMap<(&'static str, &'static str), usize>,
}

impl Shadow {
    fn compare(&mut self, row: &'static str, class: TargetClass, key: &str, old: String, new: String) {
        if old == new && old != "allowed" && old != "lower" {
            *self.refused.entry((row, class.name())).or_default() += 1;
        }
        let tag = format!("{} {row}", class.name());
        let id = format!("crates/hale-cli/tests/shadow_capability_cli.rs#{}", key.replace(' ', "-"));
        self.reports
            .entry((row, class.name()))
            .or_insert_with(|| Report::new(&format!("target_capability · {row} · {}", class.name())))
            .compare_rows(
                &id,
                &[(key.to_string(), old)],
                &[(key.to_string(), new)],
                |k| Some(format!("{tag} {k}")),
                |k| Some(format!("{tag} {k}")),
                |_| Vec::new(),
                |_| Vec::new(),
            );
    }

    fn invocation_fact(&self, class: TargetClass, inv: Invocation, holes: &[(&str, &str)]) -> String {
        let cell = self.m.invocation(class, inv).expect("every invocation has a row");
        match &cell.verdict {
            InvocationVerdict::Allowed => "allowed".to_string(),
            InvocationVerdict::Refused(r) => r.render(&cell.witness, holes),
        }
    }

    fn behaviour_fact(&self, class: TargetClass, cap: Capability) -> String {
        let cell = self.m.behaviour(class, cap).expect("every capability has a row");
        match cell.refusal() {
            None => "lower".to_string(),
            Some(r) => r.render(&cell.witness, &[]),
        }
    }
}

fn hale(args: &[&str], record: Option<&Path>) -> Output {
    let mut c = vault::hale();
    c.args(args);
    if let Some(rec) = record {
        c.env("LOTUS_OBS_RECORD", rec);
    }
    c.output().expect("run hale")
}

#[test]
fn every_cli_refusal_agrees_with_its_cell_or_is_classified() {
    let dir = scratch();
    let prog = dir.join("prog.hl");
    std::fs::write(&prog, PROGRAM).expect("write program");
    let p = prog.to_str().unwrap();
    let mut shadow = Shadow { m: derive_capability_matrix(), reports: BTreeMap::new(), refused: BTreeMap::new() };
    assert_eq!(
        TargetClass::of(&TargetSpec::host()),
        Some(TargetClass::PosixAsync),
        "the host column is the one the CLI runs"
    );

    // ---- the host: each invocation, done.
    let out = hale(&["run", p], None);
    let ran = String::from_utf8_lossy(&out.stdout).contains("ran");
    let fact = shadow.invocation_fact(TargetClass::PosixAsync, Invocation::Run, &[]);
    shadow.compare("invocations", TargetClass::PosixAsync, "Run", outcome(&out, ran), fact);
    let rec = dir.join("prog.halerec");
    let out = hale(&["run", p], Some(&rec));
    let fact = shadow.invocation_fact(TargetClass::PosixAsync, Invocation::Record, &[]);
    shadow.compare("invocations", TargetClass::PosixAsync, "Record", outcome(&out, rec.is_file()), fact);
    // `println` is a live effect, which replay refuses by default: an
    // effect policy, not the invocation's cell.
    let out = hale(&["replay", "--allow-live-effects", rec.to_str().unwrap(), p], None);
    let fact = shadow.invocation_fact(TargetClass::PosixAsync, Invocation::Replay, &[]);
    shadow.compare("invocations", TargetClass::PosixAsync, "Replay", outcome(&out, true), fact);

    // ---- musl and wasm32: each invocation, refused before anything
    // is built.
    for (class, target) in [(TargetClass::PosixNoAsync, MUSL), (TargetClass::Wasm32, "wasm32")] {
        let rec = dir.join(format!("{target}.halerec"));
        let cases: [(Invocation, &str, Output); 3] = [
            (Invocation::Run, "run", hale(&["run", "--target", target, p], None)),
            (Invocation::Record, "run", hale(&["run", "--target", target, p], Some(&rec))),
            (Invocation::Replay, "replay", hale(&["replay", "--target", target, rec.to_str().unwrap(), p], None)),
        ];
        for (inv, cmd, out) in cases {
            let fact = shadow.invocation_fact(class, inv, &[("cmd", cmd), ("triple", target)]);
            shadow.compare("invocations", class, inv.name(), outcome(&out, false), fact);
        }
    }

    // ---- `--wrap-main`, on each column and each wasm32 spelling.
    let wrap = Capability::EntryInversion(Inversion::WrapMain);
    let mut cases: Vec<(TargetClass, Vec<&str>, &str)> = vec![
        (TargetClass::PosixAsync, vec![], "no --target"),
        (TargetClass::PosixAsync, vec!["--target", "native"], "--target native"),
        (TargetClass::PosixNoAsync, vec!["--target", MUSL], "--target musl"),
    ];
    let wasm_tools = tool("clang") && tool("wasm-ld");
    if wasm_tools {
        for spelling in ["wasm32", "wasm", "wasm32-unknown-unknown"] {
            cases.push((TargetClass::Wasm32, vec!["--target", spelling], spelling));
        }
    } else {
        eprintln!("SKIP: the wasm32 `--wrap-main` builds need clang and wasm-ld, and one is missing");
    }
    for (class, target, label) in cases {
        let out_path = dir.join(format!("wrapped-{}", label.replace([' ', '-'], "_")));
        let out_path = if class == TargetClass::Wasm32 { out_path.with_extension("wasm") } else { out_path };
        let mut args = vec!["build"];
        args.extend(target.iter().copied());
        let o = out_path.to_str().unwrap();
        args.extend(["--wrap-main", "-o", o, p]);
        let out = hale(&args, None);
        let observed = if out.status.success() && out_path.is_file() { "lower".to_string() } else { refusal(&out) };
        let fact = shadow.behaviour_fact(class, wrap);
        shadow.compare("--wrap-main", class, &format!("WrapMain {label}"), observed, fact);
    }
    let _ = std::fs::remove_dir_all(&dir);

    let mut merged = Report::new("target_capability");
    let mut summary = String::new();
    for ((row, class), r) in &shadow.reports {
        let refused = shadow.refused.get(&(*row, *class)).copied().unwrap_or(0);
        summary.push_str(&format!(
            "  {row} × {class}: {} rows ({refused} refused by both), {} divergent\n",
            r.rows_compared,
            r.divergences.len()
        ));
        merged.programs += r.programs;
        merged.rows_compared += r.rows_compared;
        merged.divergences.extend(r.divergences.iter().cloned());
    }
    eprintln!("shadow target_capability (CLI):\n{summary}");

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
        gate_message(&merged, &unexplained, &stale, "crates/hale-cli/tests/fixtures/shadow_capability_cli.txt")
    );
}
