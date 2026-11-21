//! The gate's verdict on one candidate: `hale check` over the seed, then
//! `hale build` over the same seed, both through the snapshot the verbs
//! load (`Snapshot::load` over an overlay, `LoadMode::WholeSeed`).
//!
//! A candidate is a set of files laid out under [`ROOT`]: the main seed's
//! files under `main/`, and an imported seed's under `main/pglib/` (the main seed
//! spells it `import "./pglib" as <alias>;`, as `tests/hale`
//! lays its libraries out). Nothing is written to disk; the overlay holds the text.

use std::collections::BTreeMap;
use std::path::PathBuf;

use hale_codegen::CodegenError;
use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, LoadError, Snapshot};
use hale_frontend::source::Overlay;

use crate::build_opts;
use crate::harness;

/// Where a candidate's files live in the overlay. Fixed, like
/// `build_opts::SEED_ENTRY`: nothing is written there.
pub const ROOT: &str = "/hale-parity-seed";

/// A candidate program: its files by path relative to [`ROOT`]
/// (`main/main.hl` always, `main/<other>.hl` for a decl moved within the
/// seed, `main/pglib/lib.hl` for an imported seed).
pub type Files = BTreeMap<String, String>;

/// What the gate saw.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// The check reported an error (or could not run): not the gate's
    /// business — a mutation that breaks the program is discarded.
    CheckRejects,
    /// Checked clean and built.
    Built,
    /// Checked clean and the build refused it, or lowering panicked. The
    /// rendered error.
    Diverges(String),
    /// GH #2134 (F.63): checked clean, and the build refused a loop of a
    /// `@vectorize` fn its optimizer did not vectorize on this target.
    /// That refusal is a target-dependent build assertion, not a gap
    /// between check and build: `hale check` refuses what no target
    /// vectorizes, and what is left (an in-order `Float` reduction, the
    /// cost model, the vector width) is decided by the target. It is
    /// counted apart and never listed as a divergence (spec/semantics.md
    /// § Vectorized loops). The rendered error.
    TargetAssertion(String),
}

fn overlay(files: &Files) -> BTreeMap<PathBuf, String> {
    files.iter().map(|(p, s)| (PathBuf::from(ROOT).join(p), s.clone())).collect()
}

fn entry() -> PathBuf {
    PathBuf::from(ROOT).join("main")
}

/// `hale check <seed dir>`'s verdict: the editor's configuration is the
/// CLI's report (the check plus the build rules it runs beside it).
pub fn check_clean(files: &Files) -> bool {
    let buffers = overlay(files);
    let Ok(snap) = Snapshot::load(&entry(), LoadMode::WholeSeed, &Overlay::new(&buffers), Config::editor()) else {
        return false;
    };
    match snap.demand_check() {
        Ok(checked) => !checked.diags.iter().any(|d| d.is_error()),
        Err(_) => false,
    }
}

/// `hale build <seed dir>`: a build's configuration for the host (the
/// harness's, with lowering gated on the check as the verb gates it),
/// then codegen. `Err` is the build's refusal, rendered.
pub fn build(files: &Files, tag: &str) -> Result<(), String> {
    build_err(files, tag).map_err(|e| match e {
        BuildErr::Codegen(e) => render(&e),
        BuildErr::Before(msg) => msg,
    })
}

/// Why [`build_err`] did not build: a codegen error, or a refusal before
/// codegen (the load, the lowering view), rendered.
pub enum BuildErr {
    Codegen(CodegenError),
    Before(String),
}

/// [`build`], with codegen's error as it was raised.
pub fn build_err(files: &Files, tag: &str) -> Result<(), BuildErr> {
    let options = build_opts::options();
    let mut config = build_opts::harness_config(&options);
    config.check_gates_lowering = true;
    let buffers = overlay(files);
    let snap = match Snapshot::load(&entry(), LoadMode::WholeSeed, &Overlay::new(&buffers), config) {
        Ok(s) => s,
        Err(LoadError::Refused(msg)) => return Err(BuildErr::Before(format!("load refused: {msg}"))),
        Err(LoadError::Load(f)) => return Err(BuildErr::Before(format!("load failed: {}", f.text()))),
    };
    let view = match snap.demand_lowering() {
        Ok(v) => v,
        Err(b) => {
            let why = b.refused.clone().unwrap_or_else(|| {
                b.because.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
            });
            return Err(BuildErr::Before(format!("lowering view blocked ({}): {why}", b.family)));
        }
    };
    let bin = harness::unique_bin(tag);
    let out = hale_codegen::build_resolved(view, &bin, &options);
    let _ = std::fs::remove_file(&bin);
    out.map_err(BuildErr::Codegen)
}

fn render(e: &CodegenError) -> String {
    e.to_string()
}

/// The whole verdict. A panic in the check or the build is caught and is
/// a divergence when the check had passed (a panic in the check itself is
/// the check rejecting, loudly — not this gate's subject).
pub fn verdict(files: &Files, tag: &str) -> Verdict {
    let checked = std::panic::catch_unwind(|| check_clean(files)).unwrap_or(false);
    if !checked {
        return Verdict::CheckRejects;
    }
    match std::panic::catch_unwind(|| build_err(files, tag)) {
        Ok(Ok(())) => Verdict::Built,
        Ok(Err(BuildErr::Codegen(e @ CodegenError::NotVectorized(..)))) => Verdict::TargetAssertion(render(&e)),
        Ok(Err(BuildErr::Codegen(e))) => Verdict::Diverges(render(&e)),
        Ok(Err(BuildErr::Before(msg))) => Verdict::Diverges(msg),
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "a panic with no message".to_string());
            Verdict::Diverges(format!("panicked: {msg}"))
        }
    }
}

/// A divergence's cause, as the ratchet keys it: the message with what a
/// mutation varies taken out, so one cause reached through two programs
/// (or two names, or two types) is one entry:
///
/// - only the first line (a refusal's later lines are context);
/// - a trailing location the refusal was placed at (`(in the standard
///   library, core.hl:N:N)`) and a `— did you mean …?` hint;
/// - names between backticks or quotes, written `_`;
/// - numbers, written `#`;
/// - type-like words (capitalised, or a `::` path), written `T`.
pub fn signature(msg: &str) -> String {
    let mut first = msg.lines().next().unwrap_or("").trim().to_string();
    if let Some(i) = first.find(" (in the standard library") {
        first.truncate(i);
    }
    if let Some(i) = first.find(" — did you mean") {
        first.truncate(i);
    }
    let mut out = String::new();
    let mut chars = first.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '`' | '"' => {
                for d in chars.by_ref() {
                    if d == c {
                        break;
                    }
                }
                out.push(c);
                out.push('_');
                out.push(c);
            }
            d if d.is_ascii_digit() => {
                while chars.peek().is_some_and(|d| d.is_ascii_digit()) {
                    chars.next();
                }
                out.push('#');
            }
            _ => out.push(c),
        }
    }
    let out = out.replace(ROOT, "");
    let words: Vec<&str> = out.split_whitespace().collect();
    words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let core = w.trim_matches(|c: char| !c.is_alphanumeric() && c != ':' && c != '_');
            let typeish = core.contains("::") || core.chars().next().is_some_and(|c| c.is_ascii_uppercase());
            // a sentence's first word is capitalised for the sentence
            let starts_sentence = i == 0 || words[i - 1].ends_with(['.', '!', '?']);
            if typeish && core.len() > 1 && !starts_sentence {
                w.replace(core, "T")
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
