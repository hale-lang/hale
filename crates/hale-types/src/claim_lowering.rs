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
//! these rows yet. The lowering is TOTAL over parseable programs
//! and never drops law-shaped source: an unresolvable reference
//! lowers with `id: None` (raw and display spellings kept, its own
//! provenance), and law-SELECTION invalidity the enumeration
//! reports (unknown/cyclic constitutions, illegal library-tier
//! `adopt`, duplicates, name collisions) becomes structured
//! [`LoweringIssue`]s on the table — an IR-only evaluator observes
//! the invalidity, never "no law" (review round 15).
//!
//! Reference doctrine (review round 15): when an id RESOLVES, the
//! reference's name/display are taken from the model entity — one
//! construction, so an evaluator using the id and a renderer using
//! the name cannot describe different laws (`validate` enforces the
//! agreement). Unresolved references keep the source spelling.
//!
//! `@effects(is: {…})` (carries) deliberately does NOT lower: it
//! is a classification FACT, not an obligation — the model already
//! records it as `labels` rows, and a judgment never evaluates it.

use std::collections::BTreeMap;

use hale_model::{
    ApplicationModel, ClaimIr, ClaimIrTable, ClaimOrigin, ClaimRow,
    CountCmpIr, EffectClassRef, GrantIr, GroupRef, LoweringIssue,
    NameRef, PhaseIrRef, Provenance, ProvenanceId, QuantDimIr,
    SeedIrRef, BusSelector, SetIr, TopicIrRef,
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
        demangle.get(raw).cloned().unwrap_or_else(|| n_to_owned(raw))
    };
    fn n_to_owned(s: &str) -> String {
        s.to_string()
    }

    // ---- resolution maps out of the model ----
    let e = &model.entities;
    let group_id: BTreeMap<&str, u32> = e
        .groups
        .iter()
        .enumerate()
        .map(|(i, g)| (g.name.as_str(), i as u32))
        .collect();
    let topic_id: BTreeMap<&str, u32> = e
        .topics
        .iter()
        .enumerate()
        .map(|(i, t)| (t.name.as_str(), i as u32))
        .collect();
    let fn_id: BTreeMap<&str, u32> = e
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name.as_str(), i as u32))
        .collect();
    let locus_id: BTreeMap<&str, u32> = e
        .loci
        .iter()
        .enumerate()
        .map(|(i, l)| (l.name.as_str(), i as u32))
        .collect();
    let phase_id: BTreeMap<&str, u32> = e
        .phases
        .iter()
        .enumerate()
        .map(|(i, p)| (p.name.as_str(), i as u32))
        .collect();
    let seed_id: BTreeMap<&str, u32> = e
        .seeds
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name.as_str(), i as u32))
        .collect();
    let class_id: BTreeMap<&str, u32> = e
        .effect_classes
        .iter()
        .enumerate()
        .map(|(i, c)| (c.name.as_str(), i as u32))
        .collect();

    // ---- provenance (same resolution rule as the model builder) ----
    let mut table = ClaimIrTable::default();
    for sf in &bundle.sources {
        table.provenance.sources.push(
            hale_model::provenance::SourceUnit {
                path: sf.path.clone(),
                digest: sf.digest.clone(),
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
    let intern = |records: &mut Vec<Provenance>,
                  span: hale_syntax::Span|
     -> ProvenanceId {
        let s = span.start.as_usize() as u32;
        let (src, ls) = loc(s);
        let (_, le) = loc(span.end.as_usize() as u32);
        let id = ProvenanceId(records.len() as u32);
        records.push(if src >= 0 {
            Provenance::Source {
                source: hale_model::SourceId(src as u32),
                span: (ls, le.max(ls)),
            }
        } else {
            Provenance::Synthetic {
                origin: "unplaceable span".to_string(),
            }
        });
        id
    };

    // ---- ref lowerers (entity-sourced names when resolved) ----
    let group_ref = |recs: &mut Vec<Provenance>,
                     n: &str,
                     span: hale_syntax::Span|
     -> GroupRef {
        let pid = intern(recs, span);
        match group_id.get(n) {
            Some(i) => {
                let row = &e.groups[*i as usize];
                GroupRef {
                    group: Some(hale_model::GroupId(*i)),
                    name: NameRef {
                        raw: row.name.clone(),
                        display: row.display.clone(),
                    },
                    provenance: pid,
                }
            }
            None => GroupRef {
                group: None,
                name: NameRef {
                    raw: n.to_string(),
                    display: display_of(n),
                },
                provenance: pid,
            },
        }
    };
    // A topic name: post-mangle a single canonical segment; an
    // unmangled `alias::T` resolves through the rename table.
    let topic_raw = |joined: &str, segments: &[&str]| -> String {
        if segments.len() == 2 {
            bundle
                .import_renames
                .iter()
                .find(|(segs, _)| {
                    segs.len() == 2
                        && segs[0] == segments[0]
                        && segs[1] == segments[1]
                })
                .map(|(_, m)| m.clone())
                .unwrap_or_else(|| joined.to_string())
        } else {
            joined.to_string()
        }
    };
    let topic_ref_parts = |recs: &mut Vec<Provenance>,
                           joined: &str,
                           segments: &[&str],
                           span: hale_syntax::Span|
     -> TopicIrRef {
        let pid = intern(recs, span);
        let raw = topic_raw(joined, segments);
        match topic_id.get(raw.as_str()) {
            Some(i) => {
                let row = &e.topics[*i as usize];
                TopicIrRef {
                    topic: Some(hale_model::TopicId(*i)),
                    name: NameRef {
                        raw: row.name.clone(),
                        display: row.display.clone(),
                    },
                    provenance: pid,
                }
            }
            None => TopicIrRef {
                topic: None,
                name: NameRef {
                    display: display_of(&raw),
                    raw,
                },
                provenance: pid,
            },
        }
    };
    let topic_ref = |recs: &mut Vec<Provenance>,
                     t: &TopicRef|
     -> TopicIrRef {
        let segs: Vec<&str> =
            t.segments.iter().map(|s| s.name.as_str()).collect();
        topic_ref_parts(recs, &t.display(), &segs, t.span)
    };
    // The ONE bus selector for annotation entries (rounds 16-18):
    // EVERY spelling — identifier, alias path, string literal —
    // gets its candidate sets from `effects::topic_ref_matches`
    // DIRECTLY (exact string equality first, trailing-name second).
    // The parser collapses literals and paths to plain strings, so
    // `"audit::log"` cannot be told from `alias::Topic`
    // syntactically — and the evaluator never tells them apart
    // either: a qualified path also tail-matches every same-tailed
    // import. One rule, shared, no guessing from string contents.
    let bus_selector = |recs: &mut Vec<Provenance>,
                        s: &str,
                        span: hale_syntax::Span|
     -> BusSelector {
        let mut topics: Vec<hale_model::TopicId> = e
            .topics
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                crate::effects::topic_ref_matches(s, &t.name)
            })
            .map(|(i, _)| hale_model::TopicId(i as u32))
            .collect();
        topics.sort();
        topics.dedup();
        let mut subjects: Vec<hale_model::SubjectId> = e
            .subjects
            .iter()
            .enumerate()
            .filter(|(_, su)| {
                crate::effects::topic_ref_matches(s, &su.pattern)
            })
            .map(|(i, _)| hale_model::SubjectId(i as u32))
            .collect();
        subjects.sort();
        subjects.dedup();
        BusSelector {
            name: s.to_string(),
            topics,
            subjects,
            provenance: intern(recs, span),
        }
    };
    let effect_names = crate::effects::effect_names_of(&programs);
    let class_ref_named = |recs: &mut Vec<Provenance>,
                           name: &str,
                           span: hale_syntax::Span|
     -> EffectClassRef {
        let pid = intern(recs, span);
        let builtin = hale_model::is_builtin_effect_class(name);
        EffectClassRef {
            class: if builtin {
                None
            } else {
                class_id
                    .get(name)
                    .map(|i| hale_model::EffectClassId(*i))
            },
            builtin,
            name: name.to_string(),
            provenance: pid,
        }
    };
    let class_ref = |recs: &mut Vec<Provenance>,
                     c: &EffectClass,
                     span: hale_syntax::Span|
     -> EffectClassRef {
        let name = match c {
            EffectClass::User(i) => effect_names
                .get(*i as usize)
                .cloned()
                .unwrap_or_else(|| format!("<user:{}>", i)),
            other => other.as_str().to_string(),
        };
        class_ref_named(recs, &name, span)
    };
    let fn_at = |raw: &str| -> (Option<hale_model::FunctionId>, NameRef) {
        match fn_id.get(raw) {
            Some(i) => {
                let row = &e.functions[*i as usize];
                (
                    Some(hale_model::FunctionId(*i)),
                    NameRef {
                        raw: row.name.clone(),
                        display: row.display.clone(),
                    },
                )
            }
            None => (
                None,
                NameRef {
                    raw: raw.to_string(),
                    display: display_of(raw),
                },
            ),
        }
    };
    let locus_at =
        |raw: &str| -> (Option<hale_model::LocusDeclId>, NameRef) {
            match locus_id.get(raw) {
                Some(i) => {
                    let row = &e.loci[*i as usize];
                    (
                        Some(hale_model::LocusDeclId(*i)),
                        NameRef {
                            raw: row.name.clone(),
                            display: row.display.clone(),
                        },
                    )
                }
                None => (
                    None,
                    NameRef {
                        raw: raw.to_string(),
                        display: display_of(raw),
                    },
                ),
            }
        };
    let cmp_ir = |c: CountCmp| match c {
        CountCmp::Eq => CountCmpIr::Eq,
        CountCmp::Le => CountCmpIr::Le,
        CountCmp::Ge => CountCmpIr::Ge,
    };

    // ---- 1. claims-block forms, via the evaluator's enumeration ----
    let universe = crate::claims::enumerate_clauses(
        &programs,
        &bundle.import_renames,
    );
    // Law-SELECTION invalidity becomes structured issues — never
    // silently dropped (review round 15).
    for d in &universe.diags {
        let pid = intern(&mut table.provenance.records, d.span);
        table.issues.push(LoweringIssue {
            message: d.message.clone(),
            provenance: pid,
            // Law SELECTION: which laws exist at all (group
            // resolution, constitution adoption, name collisions).
            // No family owns it, and no other engine reports it.
            family: None,
        });
    }
    let recs = &mut table.provenance.records;
    let mut rows: Vec<(String, ClaimOrigin, ClaimIr, hale_syntax::Span)> =
        Vec::new();
    for c in &universe.claims {
        let origin = if let Some(k) = universe.origins.get(&c.name.name)
        {
            ClaimOrigin::Constitution { name: k.clone() }
        } else if let Some(alias) = universe.library.get(&c.name.name)
        {
            ClaimOrigin::Library {
                alias: alias.clone(),
            }
        } else {
            ClaimOrigin::Main
        };
        let set_ir = |recs: &mut Vec<Provenance>, s: &ClaimSet| match s
        {
            ClaimSet::Group(g) => {
                SetIr::Group(group_ref(recs, &g.name, g.span))
            }
            ClaimSet::Effects { name, span, .. } => {
                SetIr::EffectCarriers(class_ref_named(
                    recs, name, *span,
                ))
            }
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
                src: set_ir(recs, src),
                dst: set_ir(recs, dst),
                via_calls: *via_calls,
                via_bus: *via_bus,
                during: during.as_ref().map(|p| PhaseIrRef {
                    phase: phase_id
                        .get(p.name.as_str())
                        .map(|i| hale_model::PhaseId(*i)),
                    name: p.name.clone(),
                    provenance: intern(recs, p.span),
                }),
                avoiding: avoiding
                    .as_ref()
                    .map(|a| group_ref(recs, &a.name, a.span)),
            },
            ClaimForm::OnlyEdges { src, dst, grants } => {
                ClaimIr::OnlyEdges {
                    src: group_ref(recs, &src.name, src.span),
                    dst: group_ref(recs, &dst.name, dst.span),
                    grants: grants
                        .iter()
                        .map(|g| GrantIr {
                            publish: g.publish,
                            topic: topic_ref(recs, &g.topic),
                        })
                        .collect(),
                }
            }
            ClaimForm::Bound {
                class_name,
                class_span,
                limit,
                from,
                ..
            } => ClaimIr::Bound {
                class: class_ref_named(recs, class_name, *class_span),
                limit: *limit,
                from: group_ref(recs, &from.name, from.span),
            },
            ClaimForm::Require {
                publishers,
                group,
                topic,
            } => ClaimIr::RequireEndpoint {
                publishers: *publishers,
                group: group_ref(recs, &group.name, group.span),
                topic: topic_ref(recs, topic),
            },
            ClaimForm::RequireSealed { group } => {
                ClaimIr::RequireSealed {
                    group: group_ref(recs, &group.name, group.span),
                }
            }
            ClaimForm::RequireAttributed { class_name } => {
                ClaimIr::RequireAttributed {
                    class: class_ref_named(
                        recs,
                        &class_name.name,
                        class_name.span,
                    ),
                }
            }
            ClaimForm::Cover { alias, group } => ClaimIr::Cover {
                seed: SeedIrRef {
                    seed: seed_id
                        .get(alias.name.as_str())
                        .map(|i| hale_model::SeedId(*i)),
                    name: alias.name.clone(),
                    provenance: intern(recs, alias.span),
                },
                group: group_ref(recs, &group.name, group.span),
            },
            ClaimForm::Count {
                publishers,
                topic,
                cmp,
                n,
            } => ClaimIr::Count {
                publishers: *publishers,
                topic: topic_ref(recs, topic),
                cmp: cmp_ir(*cmp),
                n: *n,
            },
        };
        // Anchor at the claim NAME, exactly where the evaluator
        // anchors its diagnostics (review: 5a span parity).
        rows.push((c.name.name.clone(), origin, law, c.name.span));
    }

    // ---- 2. annotation surfaces, in declaration order ----
    enum AnnSite<'a> {
        Free(&'a hale_syntax::ast::FnDecl),
        Locus(&'a hale_syntax::ast::LocusDecl),
    }
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
    let mut sites = Vec::new();
    for p in &programs {
        walk_decls(&p.items, &mut sites);
    }
    let mut ann_issues: Vec<(String, hale_syntax::Span)> = Vec::new();
    let effect_names = crate::effects::effect_names_of(&programs);
    let declared_classes = crate::effects::declared_of(&programs);
    let lower_fn_anns = |recs: &mut Vec<Provenance>,
                         rows: &mut Vec<(
        String,
        ClaimOrigin,
        ClaimIr,
        hale_syntax::Span,
    )>,
                         issues_out: &mut Vec<(
        String,
        hale_syntax::Span,
    )>,
                         raw: &str,
                         f: &hale_syntax::ast::FnDecl| {
        // The undeclared-class validation (#345) — the evaluator's
        // pass 1 over this root's Forbid/Causes/Carries lists, with
        // its per-ROOT dedup across the three surfaces. It lives in
        // the LOWERING because `is:` (carries) produces no ClaimIr
        // row at all — validated here or nowhere — and because one
        // authority must own the dedup: a root writing
        // `is: {money}, none: {money}` gets ONE diagnostic. The
        // judgment keeps only the verdict consequence (a row
        // asserting about an undeclared class judges Invalid).
        let mut seen_undeclared: Vec<u16> = Vec::new();
        // The subject renders as the RESOLVED display spelling —
        // for an imported fn or method, `raw` is the post-merge
        // canonical symbol (`__lib_…`), which appears nowhere in
        // the author's source (review round 2).
        let subj_display = fn_at(raw).1.display;
        for a in &f.effects {
            let cs: &[EffectClass] = match a {
                EffectAssert::Forbid(cs)
                | EffectAssert::Causes(cs)
                | EffectAssert::Carries(cs) => cs,
                _ => &[],
            };
            for c in cs {
                let EffectClass::User(i) = c else { continue };
                if declared_classes.contains(i)
                    || seen_undeclared.contains(i)
                {
                    continue;
                }
                seen_undeclared.push(*i);
                let bad = effect_names
                    .get(*i as usize)
                    .cloned()
                    .unwrap_or_default();
                let mut near: Vec<&String> = effect_names
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| {
                        declared_classes.contains(&(*j as u16))
                    })
                    .map(|(_, n)| n)
                    .filter(|n| crate::effects::close(n, &bad))
                    .collect();
                near.sort();
                let hint = match near.first() {
                    Some(n) => format!(" Did you mean `{}`?", n),
                    None => String::new(),
                };
                issues_out.push((
                    format!(
                        "`{}` asserts about effect class `{}`, \
                         which is never declared. Add `effect {};` \
                         at the top level.{}",
                        subj_display, bad, bad, hint
                    ),
                    f.name.span,
                ));
            }
        }
        for a in &f.effects {
            let at = fn_at(raw);
            let law = match a {
                EffectAssert::Forbid(cs) => ClaimIr::EffectForbid {
                    at,
                    classes: cs
                        .iter()
                        .map(|c| class_ref(recs, c, f.name.span))
                        .collect(),
                },
                // TOPIC selectors (review rounds 15/16): a
                // qualified/canonical spelling is exact; an
                // UNQUALIFIED spelling follows the evaluator's
                // documented cross-seed rule — a library author
                // cannot know the consumer's alias, so the name
                // matches merged topics by TRAILING name, possibly
                // several.
                EffectAssert::PublishSet(entries) => {
                    ClaimIr::EffectPublishSet {
                        at,
                        entries: entries
                            .iter()
                            .map(|t| {
                                bus_selector(recs, t, f.name.span)
                            })
                            .collect(),
                    }
                }
                EffectAssert::Causes(cs) => ClaimIr::EffectCauses {
                    at,
                    classes: cs
                        .iter()
                        .map(|c| class_ref(recs, c, f.name.span))
                        .collect(),
                },
                EffectAssert::Only(cs) => ClaimIr::EffectOnly {
                    at,
                    classes: cs
                        .iter()
                        .map(|c| class_ref(recs, c, f.name.span))
                        .collect(),
                },
                EffectAssert::NoPanic => ClaimIr::NoPanic { at },
                // Classification fact, not a law — the model's
                // `labels` rows already carry it.
                EffectAssert::Carries(_) => continue,
            };
            rows.push((
                raw.to_string(),
                ClaimOrigin::Annotation,
                law,
                f.name.span,
            ));
        }
        if let Some(n) = f.budget {
            rows.push((
                raw.to_string(),
                ClaimOrigin::Annotation,
                ClaimIr::AllocBudget {
                    at: fn_at(raw),
                    per_call: n,
                },
                f.name.span,
            ));
        }
        for (dim, limit) in &f.quantities {
            // The quantitative evaluator refuses an undeclared
            // user-class dimension with its own wording; the
            // diagnostic is retained here as a lowering issue
            // (review round 6), and the judgment carries the
            // verdict consequence (Invalid).
            if let QuantDim::UserClass(i) = dim {
                if !declared_classes.contains(i) {
                    let bad = effect_names
                        .get(*i as usize)
                        .cloned()
                        .unwrap_or_default();
                    let mut near: Vec<&String> = effect_names
                        .iter()
                        .enumerate()
                        .filter(|(j, _)| {
                            declared_classes.contains(&(*j as u16))
                        })
                        .map(|(_, n)| n)
                        .filter(|n| crate::effects::close(n, &bad))
                        .collect();
                    near.sort();
                    let hint = match near.first() {
                        Some(n) => format!(" Did you mean `{}`?", n),
                        None => String::new(),
                    };
                    issues_out.push((
                        format!(
                            "`{}` budgets effect class `{}`, which is \
                             never declared. Add `effect {};` at the \
                             top level.{}",
                            subj_display, bad, bad, hint
                        ),
                        f.name.span,
                    ));
                }
            }
            rows.push((
                raw.to_string(),
                ClaimOrigin::Annotation,
                ClaimIr::QuantBudget {
                    at: fn_at(raw),
                    dim: match dim {
                        QuantDim::StackBytes => QuantDimIr::StackBytes,
                        QuantDim::BlockPoints => {
                            QuantDimIr::BlockPoints
                        }
                        QuantDim::Publish => QuantDimIr::Publish,
                        QuantDim::Fanout => QuantDimIr::Fanout,
                        QuantDim::UserClass(i) => {
                            QuantDimIr::UserClass(class_ref(
                                recs,
                                &EffectClass::User(*i),
                                f.name.span,
                            ))
                        }
                    },
                    limit: *limit,
                },
                f.name.span,
            ));
        }
    };
    for site in &sites {
        match site {
            AnnSite::Free(f) => lower_fn_anns(
                recs,
                &mut rows,
                &mut ann_issues,
                &f.name.name,
                f,
            ),
            AnnSite::Locus(l) => {
                let lname = &l.name.name;
                if let Some(pe) = &l.phase_effects {
                    let phases = pe
                        .phases
                        .iter()
                        .map(|(ph, cs)| {
                            (
                                ph.clone(),
                                cs.iter()
                                    .map(|c| {
                                        class_ref(recs, c, pe.span)
                                    })
                                    .collect(),
                            )
                        })
                        .collect();
                    rows.push((
                        lname.clone(),
                        ClaimOrigin::Annotation,
                        ClaimIr::PhaseEffects {
                            locus: locus_at(lname),
                            phases,
                        },
                        pe.span,
                    ));
                }
                if let Some(ds) = &l.depends {
                    rows.push((
                        lname.clone(),
                        ClaimOrigin::Annotation,
                        ClaimIr::DependsSet {
                            locus: locus_at(lname),
                            entries: ds
                                .subjects
                                .iter()
                                .map(|s| {
                                    bus_selector(recs, s, ds.span)
                                })
                                .collect(),
                        },
                        ds.span,
                    ));
                }
                for m in &l.members {
                    if let LocusMember::Fn(f) = m {
                        let raw =
                            format!("{}::{}", lname, f.name.name);
                        lower_fn_anns(
                            recs,
                            &mut rows,
                            &mut ann_issues,
                            &raw,
                            f,
                        );
                    }
                }
            }
        }
    }

    for (message, span) in ann_issues {
        let pid = intern(recs, span);
        table.issues.push(LoweringIssue {
            message,
            provenance: pid,
            // The ANNOTATION surface — `@effects` / `@budget` /
            // `@no_*`. The effects engine reports these in check.
            family: Some(hale_model::JudgmentFamily::Certificate),
        });
    }

    // ---- finalize: authored ordinals + row provenance ----
    for (i, (name, origin, law, span)) in rows.into_iter().enumerate()
    {
        let pid = intern(recs, span);
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
