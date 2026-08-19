//! GH #476 Change 5e — the certificate-evidence sidecar.
//!
//! `derive_certificate_evidence` runs the certificate engines (the
//! one analysis authority — the grouped report is the same pass
//! `hale check` consumes) and keys each certificate's outcome +
//! diagnostics BY THE ClaimIr ORDINAL it answers. The sidecar
//! lives OUTSIDE the model (a model must not carry a cached prior
//! judgment of itself), and carries the model's `TopologyShapeV1`
//! so a judgment structurally refuses stale evidence.

use std::collections::BTreeMap;

use hale_model::{
    ApplicationModel, CertificateEvidence, ClaimIr, ClaimIrTable,
    EvidenceRow, EvidenceTable, Provenance, ProvenanceId, VerdictIr,
};

use crate::symbol::Bundle;

/// The EVIDENCE-ENGINE SEMANTICS VERSION (review round 4). The
/// package version does not change per commit, so it cannot
/// identify the analysis: two builds can share `CARGO_PKG_VERSION`
/// while differing in effect/witness traversal, allocation-summary
/// behavior, stdlib classification, renaming, or certificate
/// grouping and diagnostic rules.
///
/// CONTRACT: bump this constant in the SAME change as any
/// result-affecting modification to the certificate engines or
/// their inputs — `effects.rs` (grouping, strata, wording),
/// `alloc_summary.rs` / `callgraph.rs` (traversal), `claims.rs`
/// (clause enumeration the lowering shares), or the producer /
/// judgment in this module. The static registries that ARE
/// data (stdlib surface classification, path renames, stdlib
/// source) are hashed in directly, so drifting them does not rely
/// on anyone remembering this constant.
/// v2 (review round 6): cyclic-class certificates now judge
/// Invalid instead of replaying a vacuous Holds, and an undeclared
/// user-class `@budget` dimension judges Invalid instead of
/// Uncertified — evidence produced under v1 semantics must not be
/// replayed by a v2 judgment (or vice versa).
pub const ANALYSIS_SEMANTICS_VERSION: u32 = 2;

/// Digest of the certificate engines' inputs OUTSIDE the model:
/// the analysis-semantics version above, the Hale-source stdlib
/// the walks absorb, the stdlib-surface classification registry,
/// the path-rename table, and the compiler version.
/// `TopologyShapeV1` cannot cover these (review rounds 3–4); a
/// judgment recomputes this and refuses evidence produced by a
/// different analysis.
pub fn analysis_inputs_digest() -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    };
    eat(&ANALYSIS_SEMANTICS_VERSION.to_le_bytes());
    eat(hale_stdlib::AP_SOURCE.as_bytes());
    eat(env!("CARGO_PKG_VERSION").as_bytes());
    for (segs, mangled) in hale_stdlib::PATH_RENAMES {
        for s in *segs {
            eat(s.as_bytes());
            eat(b"\x1f");
        }
        eat(mangled.as_bytes());
        eat(b"\x1e");
    }
    for surface in crate::stdlib_surface::SURFACES {
        for s in surface.ns {
            eat(s.as_bytes());
            eat(b"\x1f");
        }
        for f in surface.fns {
            eat(f.name.as_bytes());
            eat(&f.effects.0.to_le_bytes());
        }
        for p in surface.open_prefixes {
            eat(p.as_bytes());
            eat(b"\x1f");
        }
        eat(b"\x1e");
    }
    h
}

/// Derive the sidecar for one bundle's lowered law table.
pub fn derive_certificate_evidence(
    bundle: &Bundle<'_>,
    table: &ClaimIrTable,
    model: &ApplicationModel,
) -> EvidenceTable {
    let programs: Vec<&hale_syntax::ast::Program> =
        bundle.programs.values().copied().collect();
    let (_flat, groups) = crate::effects::effect_report_grouped(
        &programs,
        &bundle.import_renames,
    );
    let mut out = EvidenceTable {
        model_shape: crate::topology_projection::project_shape_hash(
            model,
        ),
        law_digest: table.semantic_digest(),
        inputs_digest: analysis_inputs_digest(),
        ..EvidenceTable::default()
    };
    for sf in &bundle.sources {
        out.provenance.sources.push(
            hale_model::provenance::SourceUnit {
                path: sf.path.clone(),
                digest: sf.digest.clone(),
            },
        );
    }
    let sources = bundle.sources.clone();
    let loc = move |pos: u32| -> Option<(u32, u32)> {
        sources
            .iter()
            .filter(|f| {
                pos >= f.base && pos < f.base.saturating_add(f.len + 1)
            })
            .max_by_key(|f| f.base)
            .map(|f| (f.id, pos - f.base))
    };
    // Evidence multimap (subject display, form) → group indices,
    // consumed in generation order — the ONE place string matching
    // happens; rows key by ordinal from here on.
    let mut by_key: BTreeMap<(String, String), Vec<usize>> =
        BTreeMap::new();
    let demangled: Vec<(String, String)> = groups
        .iter()
        .map(|(row, _)| {
            (
                crate::stdlib_bodies::demangle_str(
                    &row.subject,
                    &bundle.import_renames,
                ),
                crate::stdlib_bodies::demangle_str(
                    &row.form,
                    &bundle.import_renames,
                ),
            )
        })
        .collect();
    for (i, key) in demangled.iter().enumerate() {
        by_key.entry(key.clone()).or_default().push(i);
    }
    let mut cursor: BTreeMap<(String, String), usize> =
        BTreeMap::new();
    for row in &table.rows {
        let forms = row.certificate_forms();
        if forms.is_empty() {
            continue;
        }
        let subject = match &row.law {
            ClaimIr::EffectForbid { at, .. }
            | ClaimIr::EffectOnly { at, .. }
            | ClaimIr::EffectPublishSet { at, .. }
            | ClaimIr::NoPanic { at } => at.0,
            _ => None,
        };
        let mut certs: Vec<CertificateEvidence> = Vec::new();
        for key in forms {
            let idx = by_key.get(&key).and_then(|list| {
                let c = cursor.entry(key.clone()).or_insert(0);
                let i = list.get(*c).copied();
                *c += 1;
                i
            });
            let Some(i) = idx else { continue };
            let (cert, ds) = &groups[i];
            let mut diags_out: Vec<(String, ProvenanceId)> =
                Vec::new();
            // The origin flag is authoritative: the emitters tag
            // each diagnostic from the witness step's owning fn
            // (stdlib parses at base 0, so a stdlib span cannot be
            // told from a user span numerically).
            let (mut only_diags, flags): (Vec<_>, Vec<bool>) =
                ds.iter().cloned().unzip();
            crate::stdlib_bodies::demangle_imports(
                &mut only_diags,
                &bundle.import_renames,
            );
            for (d, foreign) in
                only_diags.into_iter().zip(flags)
            {
                let s0 = d.span.start.as_usize() as u32;
                let e0 = d.span.end.as_usize() as u32;
                let pid = ProvenanceId(
                    out.provenance.records.len() as u32,
                );
                let user = if foreign { None } else { loc(s0) };
                match user {
                    Some((src, local)) => {
                        out.provenance.records.push(
                            Provenance::Source {
                                source: hale_model::SourceId(src),
                                span: (
                                    local,
                                    local + e0.saturating_sub(s0),
                                ),
                            },
                        );
                    }
                    None => {
                        // Stdlib parse space (or a sourceless test
                        // bundle) — preserve the span verbatim,
                        // normalized non-inverted.
                        out.provenance.records.push(
                            Provenance::ForeignSpan {
                                span: (s0, e0.max(s0)),
                            },
                        );
                    }
                }
                diags_out.push((d.message, pid));
            }
            certs.push(CertificateEvidence {
                form: demangled[i].1.clone(),
                result: match cert.result {
                    crate::verdict::Verdict::Holds => {
                        VerdictIr::Holds
                    }
                    crate::verdict::Verdict::Violated => {
                        VerdictIr::Violated
                    }
                    crate::verdict::Verdict::Uncertified => {
                        VerdictIr::Uncertified
                    }
                    crate::verdict::Verdict::Invalid => {
                        VerdictIr::Invalid
                    }
                },
                diags: diags_out,
            });
        }
        out.rows.push(EvidenceRow {
            ordinal: row.ordinal,
            subject,
            certs,
        });
    }
    out
}
