//! CI harness for compiler-to-Lean evidence, through the shared frontend. This
//! links no LLVM and uses the same producer as the check command's export.
use std::path::Path;

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: reachability_proofs <seed> <report.json> <proof.lean>".into());
    }
    let snapshot = Snapshot::load(
        Path::new(&args[0]),
        LoadMode::WholeSeed,
        &Disk,
        Config::check(true, false),
    )
    .map_err(|error| match error {
        hale_frontend::snapshot::LoadError::Refused(message) => message,
        hale_frontend::snapshot::LoadError::Load(failure) => failure
            .diags
            .iter()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
            .join("; "),
    })?;
    let model = snapshot
        .demand_model()
        .map_err(|error| format!("no model: {error:?}"))?;
    let selection = snapshot
        .demand_law_selection()
        .map_err(|error| format!("no laws: {error:?}"))?;
    let bundle = snapshot.bundle();
    let table = hale_types::claim_lowering::lower_claims_over(&bundle, model, selection);
    let bases = bundle.sources.iter().map(|s| s.base).collect::<Vec<_>>();
    let proof = hale_types::reachability_proof::ProofBundle::derive(&table, model, &bases)?;
    let report = proof.canonical_json();
    let lean = proof.lean_source()?;
    proof.check_canonical(&report)?;
    proof.check_canonical_lean(&lean)?;
    std::fs::write(&args[1], report).map_err(|e| e.to_string())?;
    std::fs::write(&args[2], lean).map_err(|e| e.to_string())?;
    Ok(())
}
