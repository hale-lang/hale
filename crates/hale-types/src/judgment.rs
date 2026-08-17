//! GH #476 Changes 5a–5e — the judgment engine over
//! `ClaimIr` × `ApplicationModel`.
//!
//! Family-by-family migration of the claim evaluators onto the
//! canonical model. Each family lands with DIAGNOSTICS PARITY —
//! the same public spelling, spans, and related notes the
//! authoritative evaluator emits, held byte-equal by a permanent
//! corpus differential — plus negative controls proving the engine
//! reads the model relations it claims to (dropping a family's
//! rows must change its verdicts). The old evaluators in
//! `claims.rs` stay live and authoritative until Change 9 removes
//! the duplicate authorities.
//!
//! 5a: reachability (`forbid reaches`) + holes. The walk reuses
//! `model_graph::search` — the same engine both existing tiers
//! share — with `FunctionId` vertices over model rows: `calls`
//! (Direct/Interface + ViaStdlib), the publish × subscribe
//! composition per subject, `member_of` ∩ the summary universe for
//! group projection, `phase_of` for `during`, and typed holes
//! (`IndirectCall` / `UntypedReceiver` / `ComputedSubject`) for
//! the fail-closed edges.

use hale_model::{ApplicationModel, ClaimIrTable};
use hale_syntax::Diag;

use crate::verdict::Verdict;

/// One judged law row: the ClaimIr ordinal, the verdict, and the
/// diagnostics — byte-compatible with the authoritative
/// evaluator's for the migrated family.
#[derive(Debug)]
pub struct Judged {
    pub ordinal: u32,
    pub verdict: Verdict,
    pub diags: Vec<Diag>,
}

/// Judge the 5a family (`forbid reaches`) of one lowered law table
/// against its model. `source_bases[id]` is the bundle-global base
/// offset of provenance source `id` (the caller derives it from
/// `Bundle::sources`), used to reconstruct the evaluator's
/// bundle-global diagnostic spans from the model's source-local
/// provenance.
pub fn judge_forbid_reaches(
    _table: &ClaimIrTable,
    _model: &ApplicationModel,
    _source_bases: &[u32],
) -> Vec<Judged> {
    // Implemented in Change 5a (this branch): see the module docs
    // for the migration contract.
    todo!("5a: reachability judgment")
}
