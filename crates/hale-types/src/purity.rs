//! F.36 Slice 1 (2026-05-28): per-method purity inference.
//!
//! Computes whether each locus method and free fn in a bundle is
//! *pure* — safely callable from any thread without coordination.
//! A method is pure iff its body, transitively, performs no:
//!
//! - writes to `self.<field>` (state mutation observable across calls)
//! - bus publishes (`subject <- value`) — side effects on the bus
//! - closure violations (`violate NAME`) — effects on parent
//! - calls to impure stdlib fns: those whose row in the stdlib
//!   surface table carries `syscall`, `block` or `publish`
//!   (`println`, `time::sleep`, `std::io::fs::*`, sockets, process)
//! - calls to user-defined methods / free fns that are themselves
//!   not pure (transitively)
//!
//! Pure methods may read `self.<field>`, compute with locals
//! (including `let mut`, since locals don't escape), allocate from
//! arenas, and call other pure methods.
//!
//! The result is a derived property, not a user-declared one — no
//! `pure` keyword. The compiler stores per-method purity in a
//! [`PurityMap`] keyed by [`PurityKey`]; downstream assertion sites
//! (notably F.36 Slice 2's `codec(L)` binding) consult this map to
//! verify that callbacks the substrate dispatches from arbitrary
//! threads are stateless.
//!
//! Source state actions come from the shared allocation walk. Inference
//! follows its resolved calls, default evaluations and implicit lifecycle
//! and parent entries. Missing execution or confinement coverage is
//! `Uncertified`; codec bindings require `Pure`.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{LValue, LValueSeg, Program};
use hale_syntax::Span;

use crate::resolve::TopScope;
use crate::stdlib_surface::EffectSet;

/// Per-method purity result. [`Purity::Impure`] carries the first
/// impurity found in the shared source account — the diagnostic anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purity {
    /// No impurity found in the method body (transitively).
    Pure,
    /// Required execution or receiver-confinement facts are missing.
    Uncertified { reason: String, span: Span },
    /// Body has at least one impurity; `reason` names the first
    /// one selected from the source account. Subsequent impurities exist but
    /// aren't tracked (the first is the natural diagnostic site).
    Impure(Impurity),
}

/// The first impurity found in a method body. Used to anchor a
/// diagnostic at the offending source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Impurity {
    /// `self.X = ...` (or compound assign). `field_chain` is the
    /// dotted path from `self` to the assigned location.
    SelfFieldWrite { field_chain: String, span: Span },
    /// `SUBJECT <- VALUE;` bus send. Side effect on the bus.
    BusSend { subject_repr: String, span: Span },
    /// `violate NAME [with PAYLOAD];` — escalates a structural
    /// failure through the parent's `on_failure`.
    Violate { closure_name: String, span: Span },
    /// Call to a stdlib fn the effect table marks as having
    /// observable effects (println, file write, sleep, ...).
    ImpureStdlibCall { fn_name: String, span: Span },
    /// Construction can mutate the accepting parent's child tracking.
    ParentAdmission {
        owner: String,
        child: String,
        span: Span,
    },
    /// Call to a user-defined fn / method that the inference
    /// pass marks impure.
    ImpureCalleeCall { callee_name: String, span: Span },
}

/// Identifies a fn for purity lookup. R1 (2026-07-29): this was a
/// byte-identical duplicate of `alloc_summary::FnKey` — the walkers
/// are converging on one key type (see `crate::callgraph`), so
/// purity now shares it. `free_fn` / `method` constructors come
/// with it.
pub use crate::alloc_summary::FnKey as PurityKey;

/// Bundle-wide purity result keyed by [`PurityKey`].
pub type PurityMap = BTreeMap<PurityKey, Purity>;

/// Primitive source actions captured by the allocation family's existing walk.
/// These rows are shared with implicit execution; purity does not resurvey AST.
#[derive(Debug, Clone)]
pub struct PuritySite {
    pub action: StateAction,
    pub span: Span,
}
#[derive(Debug, Clone)]
pub enum StateAction {
    SelfWrite(String),
    LocusWrite(String),
    Yield,
    Terminate,
    Reperspective,
    SharedWrite,
    ParentAdmission { owner: String, child: String },
}

const IMPURE_CLASSES: EffectSet = EffectSet::SYSCALL
    .union(EffectSet::BLOCK)
    .union(EffectSet::PUBLISH);
const IMPURE_BARE_BUILTINS: &[&str] = &["println", "print", "eprintln", "eprint"];
pub fn impure_bare_builtins() -> &'static [&'static str] {
    IMPURE_BARE_BUILTINS
}

/// Compatibility source entry. Contextual parent inputs are not supplied by
/// bare programs, so a relevant construction remains uncertified. Production
/// reads the snapshot's shared execution summary through `infer_from_summary`.
pub fn infer_purity_for_bundle(programs: &[&Program], _scope: &TopScope) -> PurityMap {
    let bundle = crate::symbol::Bundle::new(
        programs
            .iter()
            .enumerate()
            .map(|(i, p)| (format!("purity-{i}"), *p))
            .collect(),
    );
    let summary = crate::alloc_summary::derive_alloc_summary(&bundle);
    infer_from_summary(&summary, &crate::effects::ffi_names(programs))
}

/// One fixpoint over the same resolved call and implicit evaluation rows that
/// effects use. Known impurity wins over unrelated missing coverage. Pure is
/// published only after every reached dependency has settled as pure.
pub fn infer_from_summary(
    summary: &crate::alloc_summary::AllocSummary,
    ffi: &BTreeSet<String>,
) -> PurityMap {
    use crate::alloc_summary::{Callee, EffectSiteKind};
    let mut own = PurityMap::new();
    let mut dependencies = BTreeMap::<PurityKey, Vec<(PurityKey, Span)>>::new();
    for (key, f) in &summary.fns {
        let mut impurity = None;
        let mut gap = None;
        for site in &f.purity_sites {
            let known = match &site.action {
                StateAction::SelfWrite(field) => Some(Impurity::SelfFieldWrite {
                    field_chain: field.clone(),
                    span: site.span,
                }),
                StateAction::LocusWrite(field) => {
                    gap.get_or_insert((
                        format!("mutation of `{field}` needs a complete receiver-confinement account"),
                        site.span,
                    ));
                    None
                }
                StateAction::ParentAdmission { owner, child } => Some(Impurity::ParentAdmission {
                    owner: owner.clone(),
                    child: child.clone(),
                    span: site.span,
                }),
                action => Some(Impurity::ImpureStdlibCall {
                    fn_name: match action {
                        StateAction::Yield => "yield",
                        StateAction::Terminate => "terminate",
                        StateAction::Reperspective => "reperspective",
                        StateAction::SharedWrite => "shared-memory write",
                        _ => unreachable!(),
                    }
                    .into(),
                    span: site.span,
                }),
            };
            if impurity.is_none() {
                impurity = known;
            }
        }
        if let Some((closure, span)) = f.violates.first() {
            impurity.get_or_insert(Impurity::Violate {
                closure_name: closure.clone(),
                span: *span,
            });
        }
        for site in &f.effect_sites {
            match &site.kind {
                EffectSiteKind::Publish(subject) => {
                    impurity.get_or_insert(Impurity::BusSend {
                        subject_repr: subject
                            .as_ref()
                            .map(|s| s.text.clone())
                            .unwrap_or_else(|| "<computed>".into()),
                        span: site.span,
                    });
                }
                EffectSiteKind::Recovery(op) => {
                    impurity.get_or_insert(Impurity::ImpureStdlibCall {
                        fn_name: format!("{op:?}"),
                        span: site.span,
                    });
                }
                EffectSiteKind::Spawn(_) => {}
                _ => {
                    gap.get_or_insert(("implicit execution has unresolved admission, scheduling, frame or failure context".into(),site.span));
                }
            }
        }
        if ffi.contains(&key.fn_name) && key.locus.is_none() {
            impurity.get_or_insert(Impurity::ImpureStdlibCall {
                fn_name: key.display(),
                span: Span::new(0, 0),
            });
        }
        let deps = dependencies.entry(key.clone()).or_default();
        for edge in &f.calls {
            match &edge.callee {
                Callee::Resolved(to) if to.locus.is_none() && ffi.contains(&to.fn_name) => {
                    impurity.get_or_insert(Impurity::ImpureStdlibCall {
                        fn_name: to.display(),
                        span: edge.span,
                    });
                }
                Callee::Resolved(to) if summary.fns.contains_key(to) => {
                    deps.push((to.clone(), edge.span))
                }
                Callee::Resolved(to) => {
                    gap.get_or_insert((
                        format!("the executable body of `{}` is unavailable", to.display()),
                        edge.span,
                    ));
                }
                Callee::Unresolved(_) if edge.via_interface.is_some() && !edge.indirect => {} // uninhabited interface
                Callee::Unresolved(name) => {
                    if f.fn_params.contains(name) || edge.indirect || edge.opaque_method_call() {
                        gap.get_or_insert((
                            format!("the callee `{name}` has no closed purity account"),
                            edge.span,
                        ));
                    } else if edge
                        .authored
                        .is_some_and(|site| f.value_conversions.contains(&site))
                    {
                        // A source scalar conversion, with operand execution
                        // already represented by this body's own rows.
                    } else if !edge.receiver_present
                        && IMPURE_BARE_BUILTINS.contains(&name.as_str())
                    {
                        impurity.get_or_insert(Impurity::ImpureStdlibCall {
                            fn_name: name.clone(),
                            span: edge.span,
                        });
                    } else if let Some(builtin) = crate::builtin_sigs::bare_builtin_sig(name)
                        .filter(|_| !edge.receiver_present)
                    {
                        if builtin.effects.is_unclassified() {
                            gap.get_or_insert((
                                format!("the value builtin `{name}` has no purity classification"),
                                edge.span,
                            ));
                        } else if builtin.effects.0 & IMPURE_CLASSES.0 != 0 {
                            impurity.get_or_insert(Impurity::ImpureStdlibCall {
                                fn_name: name.clone(),
                                span: edge.span,
                            });
                        }
                    } else if let Some(e) =
                        crate::stdlib_surface::effects_for(&name.split("::").collect::<Vec<_>>())
                    {
                        if e.is_unclassified() || edge.indirect || edge.opaque_method_call() {
                            gap.get_or_insert((
                                format!("the purity of `{name}` is not classified"),
                                edge.span,
                            ));
                        } else if e.0 & IMPURE_CLASSES.0 != 0 {
                            impurity.get_or_insert(Impurity::ImpureStdlibCall {
                                fn_name: name.clone(),
                                span: edge.span,
                            });
                        }
                    } else {
                        gap.get_or_insert((
                            format!("the callee `{name}` has no closed purity account"),
                            edge.span,
                        ));
                    }
                }
            }
        }
        for edge in f.effect_evaluations() {
            deps.push((edge.target.clone(), edge.span));
        }
        if !summary.parent_execution_supplied
            && key.locus.is_some()
            && summary.protocol_hooks.values().flatten().any(|h| {
                !h.synthesized && h.stage == crate::protocol_execution::ProtocolStage::Accept
            })
            && !f.constructions.is_empty()
        {
            gap.get_or_insert((
                "the contextual parent state account has not been supplied".into(),
                f.constructions[0].span,
            ));
        }
        if let Some(known) = impurity {
            own.insert(key.clone(), Purity::Impure(known));
        } else if let Some((reason, span)) = gap {
            own.insert(key.clone(), Purity::Uncertified { reason, span });
        }
    }
    let mut result = PurityMap::new();
    loop {
        let mut changed = false;
        for key in summary.fns.keys() {
            let mut state = own.get(key).cloned();
            let mut pending = false;
            if !matches!(state, Some(Purity::Impure(_))) {
                for (to, span) in &dependencies[key] {
                    match result.get(to) {
                        Some(Purity::Impure(_)) => {
                            state = Some(Purity::Impure(Impurity::ImpureCalleeCall {
                                callee_name: to.display(),
                                span: *span,
                            }));
                            break;
                        }
                        Some(Purity::Uncertified { .. }) => {
                            if state.is_none() {
                                state = Some(Purity::Uncertified {
                                    reason: format!("purity of `{}` is uncertified", to.display()),
                                    span: *span,
                                });
                            }
                        }
                        Some(Purity::Pure) => {}
                        None if summary.fns.contains_key(to) => pending = true,
                        None => {
                            state = Some(Purity::Uncertified {
                                reason: format!("implicit body `{}` is unavailable", to.display()),
                                span: *span,
                            });
                        }
                    }
                }
                if state.is_none() && !pending {
                    state = Some(Purity::Pure);
                }
            }
            if let Some(state) = state {
                if result.get(key) != Some(&state) {
                    result.insert(key.clone(), state);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    // Unresolved SCCs have no positive purity proof. This is coverage, not a
    // fabricated known impurity. Forward dependents receive the same refusal.
    for (key, f) in &summary.fns {
        result
            .entry(key.clone())
            .or_insert_with(|| Purity::Uncertified {
                reason: "recursive purity dependencies do not have a closed proof".into(),
                span: f.calls.first().map(|c| c.span).unwrap_or(Span::new(0, 0)),
            });
    }
    result
}

pub(crate) fn format_lvalue_chain(lv: &LValue) -> String {
    let mut s = lv.head.name.clone();
    for seg in &lv.tail {
        match seg {
            LValueSeg::Field(id) => {
                s.push('.');
                s.push_str(&id.name);
            }
            LValueSeg::Index(_) => s.push_str("[…]"),
        }
    }
    s
}
#[cfg(test)]
mod tests {
    use super::*;
    use hale_syntax::parse_source;

    fn purity_of(src: &str, key: PurityKey) -> Purity {
        let program = parse_source(src).expect("parse");
        let scope = crate::resolve::TopScope::default();
        let map = infer_purity_for_bundle(&[&program], &scope);
        map.get(&key).cloned().unwrap_or_else(|| {
            panic!(
                "no purity entry for {:?}; map = {:?}",
                key,
                map.keys().collect::<Vec<_>>()
            )
        })
    }

    #[test]
    fn pure_free_fn_with_arithmetic_is_pure() {
        let p = purity_of(
            "fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { }",
            PurityKey::free_fn(None, "add"),
        );
        assert!(matches!(p, Purity::Pure), "expected pure, got {:?}", p);
    }

    #[test]
    fn fn_calling_println_is_impure() {
        let p = purity_of(
            "fn shout(s: String) { println(s); } fn main() { }",
            PurityKey::free_fn(None, "shout"),
        );
        match p {
            Purity::Impure(Impurity::ImpureStdlibCall { fn_name, .. }) => {
                assert_eq!(fn_name, "println", "unexpected callee");
            }
            other => panic!("expected ImpureStdlibCall(println), got {:?}", other),
        }
    }

    /// The surface table is the producer: `std::io::tcp::connect` is
    /// `syscall | block` there, and was missing from the hand list
    /// this replaced, so a fn calling it was inferred pure.
    #[test]
    fn fn_calling_a_syscall_row_is_impure() {
        let p = purity_of(
            r#"fn dial() -> Int { return std::io::tcp::connect("127.0.0.1", 9) or -1; } fn main() { }"#,
            PurityKey::free_fn(None, "dial"),
        );
        match p {
            Purity::Impure(Impurity::ImpureStdlibCall { fn_name, .. }) => {
                assert_eq!(fn_name, "std::io::tcp::connect");
            }
            other => panic!("expected ImpureStdlibCall(tcp::connect), got {:?}", other),
        }
    }

    /// Reading the clock is `time` alone: not an impure class.
    #[test]
    fn fn_reading_the_clock_is_pure() {
        let p = purity_of(
            "fn stamp() -> Int { return std::time::now(); } fn main() { }",
            PurityKey::free_fn(None, "stamp"),
        );
        assert!(matches!(p, Purity::Pure), "expected pure, got {:?}", p);
    }

    #[test]
    fn method_writing_self_field_is_impure() {
        let src = r#"
            locus C {
                params { n: Int = 0; }
                fn bump() {
                    self.n = self.n + 1;
                }
            }
            fn main() { }
        "#;
        let p = purity_of(src, PurityKey::method(None, "C", "bump"));
        match p {
            Purity::Impure(Impurity::SelfFieldWrite { field_chain, .. }) => {
                assert_eq!(field_chain, "self.n");
            }
            other => panic!("expected SelfFieldWrite, got {:?}", other),
        }
    }

    #[test]
    fn method_reading_self_field_is_pure() {
        let src = r#"
            locus C {
                params { n: Int = 0; }
                fn get() -> Int {
                    return self.n;
                }
            }
            fn main() { }
        "#;
        let p = purity_of(src, PurityKey::method(None, "C", "get"));
        assert!(matches!(p, Purity::Pure), "expected pure, got {:?}", p);
    }

    #[test]
    fn transitive_impurity_via_callee_propagates() {
        let src = r#"
            fn does_io(s: String) { println(s); }
            fn wrapper(s: String) { does_io(s); }
            fn main() { }
        "#;
        let p = purity_of(src, PurityKey::free_fn(None, "wrapper"));
        match p {
            Purity::Impure(Impurity::ImpureCalleeCall { callee_name, .. }) => {
                assert_eq!(callee_name, "does_io");
            }
            other => panic!("expected ImpureCalleeCall(does_io), got {:?}", other),
        }
    }
}
