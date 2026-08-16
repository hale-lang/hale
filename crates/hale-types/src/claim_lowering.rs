//! GH #476 Change 4 — lowering every law surface to `ClaimIr`.
//!
//! `lower_claims(bundle, model)` produces the typed law table for
//! one application: claims-block forms (world tier, adopted
//! constitution clauses, library-tier blocks) through the SAME
//! clause enumeration the evaluator walks
//! (`claims::enumerate_clauses` — one authority, extracted so two
//! walks cannot drift), plus the annotation surfaces
//! (`@effects` / `@no_panic` / `@budget` / `@phase_effects` /
//! `@effects(depends:)`). Fleet plan rows lower in `hale-cli`
//! (`fleet::lower_plan_claims`) — their targets are plan-level
//! names until Change 7's `FleetModel`.
//!
//! **Lowering only** (the epic's Change-4 scope): the old
//! evaluators stay active and authoritative; nothing consumes
//! these rows yet. The lowering is TOTAL over parseable programs —
//! an unresolvable reference lowers with `id: None` (the raw and
//! display spellings are kept), which is the residue Change 5's
//! `invalid` verdict consumes; it never panics and never drops a
//! law on the floor.
//!
//! `@effects(is: {…})` (carries) deliberately does NOT lower: it
//! is a classification FACT, not an obligation — the model already
//! records it as `labels` rows, and a judgment never evaluates it.

use std::collections::BTreeMap;

use hale_model::{
    ApplicationModel, ClaimIr, ClaimIrTable, ClaimOrigin, ClaimRow,
    CountCmpIr, EffectClassRef, GrantIr, GroupRef, NameRef,
    ProvenanceId, QuantDimIr, SetIr, TopicIrRef,
};
use hale_syntax::ast::{
    ClaimForm, ClaimSet, CountCmp, EffectAssert, EffectClass,
    LocusMember, Program, QuantDim, TopDecl, TopicRef,
};

use crate::symbol::Bundle;

/// Lower every law surface of one bundle against its derived model.
pub fn lower_claims(
    bundle: &Bundle<'_>,
    model: &ApplicationModel,
) -> ClaimIrTable {
    let programs: Vec<&Program> =
        bundle.programs.values().copied().collect();

    // ---- display mapping (author spelling) ----
    let demangle: BTreeMap<&str, String> = bundle
        .import_renames
        .iter()
        .map(|(segs, mangled)| (mangled.as_str(), segs.join("::")))
        .collect();
    let display_of = |raw: &str| -> String {
        demangle.get(raw).cloned().unwrap_or_else(|| raw.to_string())
    };
    let name_ref = |raw: &str| NameRef {
        raw: raw.to_string(),
        display: display_of(raw),
    };

    // ---- resolution maps out of the model ----
    let e = &model.entities;
    let group_id: BTreeMap<&str, hale_model::GroupId> = e
        .groups
        .iter()
        .enumerate()
        .map(|(i, g)| (g.name.as_str(), hale_model::GroupId(i as u32)))
        .collect();
    let topic_id: BTreeMap<&str, hale_model::TopicId> = e
        .topics
        .iter()
        .enumerate()
        .map(|(i, t)| (t.name.as_str(), hale_model::TopicId(i as u32)))
        .collect();
    let fn_id: BTreeMap<&str, hale_model::FunctionId> = e
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| {
            (f.name.as_str(), hale_model::FunctionId(i as u32))
        })
        .collect();
    let locus_id: BTreeMap<&str, hale_model::LocusDeclId> = e
        .loci
        .iter()
        .enumerate()
        .map(|(i, l)| {
            (l.name.as_str(), hale_model::LocusDeclId(i as u32))
        })
        .collect();
    let phase_id: BTreeMap<&str, hale_model::PhaseId> = e
        .phases
        .iter()
        .enumerate()
        .map(|(i, p)| (p.name.as_str(), hale_model::PhaseId(i as u32)))
        .collect();
    let seed_id: BTreeMap<&str, hale_model::SeedId> = e
        .seeds
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name.as_str(), hale_model::SeedId(i as u32)))
        .collect();
    let subject_id: BTreeMap<&str, hale_model::SubjectId> = e
        .subjects
        .iter()
        .enumerate()
        .map(|(i, s)| {
            (s.pattern.as_str(), hale_model::SubjectId(i as u32))
        })
        .collect();

    // ---- ref lowerers ----
    let group_ref = |n: &str| GroupRef {
        group: group_id.get(n).copied(),
        name: name_ref(n),
    };
    // A topic ref: post-mangle it is a single canonical segment; an
    // unmangled 2-segment `alias::T` resolves through the rename
    // table to the same canonical symbol.
    let topic_ref = |t: &TopicRef| -> TopicIrRef {
        let joined = t.display();
        let raw = if t.segments.len() == 2 {
            bundle
                .import_renames
                .iter()
                .find(|(segs, _)| {
                    segs.len() == 2
                        && segs[0] == t.segments[0].name
                        && segs[1] == t.segments[1].name
                })
                .map(|(_, m)| m.clone())
                .unwrap_or(joined.clone())
        } else {
            joined.clone()
        };
        TopicIrRef {
            topic: topic_id.get(raw.as_str()).copied(),
            name: NameRef {
                display: display_of(&raw),
                raw,
            },
        }
    };
    let effect_names = crate::effects::effect_names_of(&programs);
    let class_ref = |c: &EffectClass| EffectClassRef {
        name: match c {
            EffectClass::User(i) => effect_names
                .get(*i as usize)
                .cloned()
                .unwrap_or_else(|| format!("<user:{}>", i)),
            other => other.as_str().to_string(),
        },
    };
    let set_ir = |s: &ClaimSet| match s {
        ClaimSet::Group(g) => SetIr::Group(group_ref(&g.name)),
        ClaimSet::Effects { name, .. } => {
            SetIr::EffectCarriers(EffectClassRef { name: name.clone() })
        }
    };
    let cmp_ir = |c: CountCmp| match c {
        CountCmp::Eq => CountCmpIr::Eq,
        CountCmp::Le => CountCmpIr::Le,
        CountCmp::Ge => CountCmpIr::Ge,
    };

    // ---- provenance (same resolution rule as the model builder) ----
    let mut table = ClaimIrTable::default();
    for sf in &bundle.sources {
        table.provenance.sources.push(
            hale_model::provenance::SourceUnit {
                path: sf.path.clone(),
                digest: u64::from_str_radix(&sf.digest, 16)
                    .unwrap_or(0),
            },
        );
    }
    let sources = bundle.sources.clone();
    let loc = move |pos: u32| -> (i64, u32) {
        match sources
            .iter()
            .filter(|f| {
                pos >= f.base && pos < f.base.saturating_add(f.len + 1)
            })
            .max_by_key(|f| f.base)
        {
            Some(f) => (f.id as i64, pos - f.base),
            None => (-1, pos),
        }
    };
    let intern = |records: &mut Vec<hale_model::Provenance>,
                      span: hale_syntax::Span|
     -> ProvenanceId {
        let s = span.start.as_usize() as u32;
        let (src, ls) = loc(s);
        let (_, le) = loc(span.end.as_usize() as u32);
        let id = ProvenanceId(records.len() as u32);
        records.push(if src >= 0 {
            hale_model::Provenance::Source {
                source: hale_model::SourceId(src as u32),
                span: (ls, le.max(ls)),
            }
        } else {
            hale_model::Provenance::Synthetic {
                origin: "unplaceable span".to_string(),
            }
        });
        id
    };

    // ---- 1. claims-block forms, via the evaluator's enumeration ----
    let universe =
        crate::claims::enumerate_clauses(&programs, &bundle.import_renames);
    let mut rows: Vec<(String, ClaimOrigin, ClaimIr, hale_syntax::Span)> =
        Vec::new();
    for c in &universe.claims {
        let origin = if let Some(k) = universe.origins.get(&c.name.name)
        {
            ClaimOrigin::Constitution { name: k.clone() }
        } else if let Some(alias) =
            universe.library.get(&c.name.name)
        {
            ClaimOrigin::Library {
                alias: alias.clone(),
            }
        } else {
            ClaimOrigin::Main
        };
        let law = match &c.form {
            ClaimForm::ForbidReaches {
                src,
                dst,
                via_calls,
                via_bus,
                during,
                avoiding,
            } => ClaimIr::ForbidReaches {
                src: set_ir(src),
                dst: set_ir(dst),
                via_calls: *via_calls,
                via_bus: *via_bus,
                during: during.as_ref().map(|p| {
                    (
                        phase_id.get(p.name.as_str()).copied(),
                        p.name.clone(),
                    )
                }),
                avoiding: avoiding
                    .as_ref()
                    .map(|a| group_ref(&a.name)),
            },
            ClaimForm::OnlyEdges { src, dst, grants } => {
                ClaimIr::OnlyEdges {
                    src: group_ref(&src.name),
                    dst: group_ref(&dst.name),
                    grants: grants
                        .iter()
                        .map(|g| GrantIr {
                            publish: g.publish,
                            topic: topic_ref(&g.topic),
                        })
                        .collect(),
                }
            }
            ClaimForm::Bound {
                class_name,
                limit,
                from,
                ..
            } => ClaimIr::Bound {
                class: EffectClassRef {
                    name: class_name.clone(),
                },
                limit: *limit,
                from: group_ref(&from.name),
            },
            ClaimForm::Require {
                publishers,
                group,
                topic,
            } => ClaimIr::RequireEndpoint {
                publishers: *publishers,
                group: group_ref(&group.name),
                topic: topic_ref(topic),
            },
            ClaimForm::RequireSealed { group } => {
                ClaimIr::RequireSealed {
                    group: group_ref(&group.name),
                }
            }
            ClaimForm::RequireAttributed { class_name } => {
                ClaimIr::RequireAttributed {
                    class: EffectClassRef {
                        name: class_name.name.clone(),
                    },
                }
            }
            ClaimForm::Cover { alias, group } => ClaimIr::Cover {
                seed: (
                    seed_id.get(alias.name.as_str()).copied(),
                    alias.name.clone(),
                ),
                group: group_ref(&group.name),
            },
            ClaimForm::Count {
                publishers,
                topic,
                cmp,
                n,
            } => ClaimIr::Count {
                publishers: *publishers,
                topic: topic_ref(topic),
                cmp: cmp_ir(*cmp),
                n: *n,
            },
        };
        rows.push((c.name.name.clone(), origin, law, c.span));
    }

    // ---- 2. annotation surfaces, in declaration order ----
    let fn_at = |raw: String| {
        let id = fn_id.get(raw.as_str()).copied();
        (id, name_ref(&raw))
    };
    let subj_at = |s: &str| {
        (subject_id.get(s).copied(), name_ref(s))
    };
    struct AnnCtx<'x> {
        rows: &'x mut Vec<(
            String,
            ClaimOrigin,
            ClaimIr,
            hale_syntax::Span,
        )>,
    }
    let lower_fn_anns =
        |ctx: &mut AnnCtx,
         raw: String,
         f: &hale_syntax::ast::FnDecl| {
            for a in &f.effects {
                let at = fn_at(raw.clone());
                let law = match a {
                    EffectAssert::Forbid(cs) => ClaimIr::EffectForbid {
                        at,
                        classes: cs.iter().map(&class_ref).collect(),
                    },
                    EffectAssert::PublishSet(subjects) => {
                        ClaimIr::EffectPublishSet {
                            at,
                            subjects: subjects
                                .iter()
                                .map(|s| subj_at(s))
                                .collect(),
                        }
                    }
                    EffectAssert::Causes(cs) => ClaimIr::EffectCauses {
                        at,
                        classes: cs.iter().map(&class_ref).collect(),
                    },
                    EffectAssert::Only(cs) => ClaimIr::EffectOnly {
                        at,
                        classes: cs.iter().map(&class_ref).collect(),
                    },
                    EffectAssert::NoPanic => ClaimIr::NoPanic { at },
                    // Classification fact, not a law — the model's
                    // `labels` rows already carry it.
                    EffectAssert::Carries(_) => continue,
                };
                ctx.rows.push((
                    raw.clone(),
                    ClaimOrigin::Annotation,
                    law,
                    f.name.span,
                ));
            }
            if let Some(n) = f.budget {
                ctx.rows.push((
                    raw.clone(),
                    ClaimOrigin::Annotation,
                    ClaimIr::AllocBudget {
                        at: fn_at(raw.clone()),
                        per_call: n,
                    },
                    f.name.span,
                ));
            }
            for (dim, limit) in &f.quantities {
                ctx.rows.push((
                    raw.clone(),
                    ClaimOrigin::Annotation,
                    ClaimIr::QuantBudget {
                        at: fn_at(raw.clone()),
                        dim: match dim {
                            QuantDim::StackBytes => {
                                QuantDimIr::StackBytes
                            }
                            QuantDim::BlockPoints => {
                                QuantDimIr::BlockPoints
                            }
                            QuantDim::Publish => QuantDimIr::Publish,
                            QuantDim::Fanout => QuantDimIr::Fanout,
                            QuantDim::UserClass(i) => {
                                QuantDimIr::UserClass(class_ref(
                                    &EffectClass::User(*i),
                                ))
                            }
                        },
                        limit: *limit,
                    },
                    f.name.span,
                ));
            }
        };
    fn walk_decls<'a>(
        items: &'a [TopDecl],
        out: &mut Vec<AnnSite<'a>>,
    ) {
        for item in items {
            match item {
                TopDecl::Fn(f) => out.push(AnnSite::Free(f)),
                TopDecl::Locus(l) => out.push(AnnSite::Locus(l)),
                TopDecl::Module(m) => walk_decls(&m.items, out),
                _ => {}
            }
        }
    }
    enum AnnSite<'a> {
        Free(&'a hale_syntax::ast::FnDecl),
        Locus(&'a hale_syntax::ast::LocusDecl),
    }
    let mut sites = Vec::new();
    for p in &programs {
        walk_decls(&p.items, &mut sites);
    }
    {
        let mut ctx = AnnCtx { rows: &mut rows };
        for site in &sites {
            match site {
                AnnSite::Free(f) => {
                    lower_fn_anns(&mut ctx, f.name.name.clone(), f)
                }
                AnnSite::Locus(l) => {
                    let lname = l.name.name.clone();
                    if let Some(pe) = &l.phase_effects {
                        ctx.rows.push((
                            lname.clone(),
                            ClaimOrigin::Annotation,
                            ClaimIr::PhaseEffects {
                                locus: (
                                    locus_id
                                        .get(lname.as_str())
                                        .copied(),
                                    name_ref(&lname),
                                ),
                                phases: pe
                                    .phases
                                    .iter()
                                    .map(|(ph, cs)| {
                                        (
                                            ph.clone(),
                                            cs.iter()
                                                .map(&class_ref)
                                                .collect(),
                                        )
                                    })
                                    .collect(),
                            },
                            pe.span,
                        ));
                    }
                    if let Some(ds) = &l.depends {
                        ctx.rows.push((
                            lname.clone(),
                            ClaimOrigin::Annotation,
                            ClaimIr::DependsSet {
                                locus: (
                                    locus_id
                                        .get(lname.as_str())
                                        .copied(),
                                    name_ref(&lname),
                                ),
                                subjects: ds
                                    .subjects
                                    .iter()
                                    .map(|s| subj_at(s))
                                    .collect(),
                            },
                            ds.span,
                        ));
                    }
                    for m in &l.members {
                        if let LocusMember::Fn(f) = m {
                            lower_fn_anns(
                                &mut ctx,
                                format!("{}::{}", lname, f.name.name),
                                f,
                            );
                        }
                    }
                }
            }
        }
    }

    // ---- finalize: authored ordinals + interned provenance ----
    for (i, (name, origin, law, span)) in rows.into_iter().enumerate()
    {
        let pid = intern(&mut table.provenance.records, span);
        table.rows.push(ClaimRow {
            ordinal: i as u32,
            name,
            origin,
            law,
            provenance: pid,
        });
    }
    table
}
