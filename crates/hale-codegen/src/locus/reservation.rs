//! Reserving a static tree's subscriptions before its first birth
//! (#1534, decision line 6).
//!
//! A static tree is a root literal and the locus literals its params
//! build, recursively: the root, and every param whose override or
//! default is a locus literal. Each node registers its subscriptions as
//! it is built, so what an earlier node's `birth()` or `run()` publishes
//! to a later one would reach no subscription of it. At the root, before
//! its params are built, the tree is enumerated here, and every subject
//! one node subscribes to and another may publish before that subscriber
//! has registered is reserved (`lotus_bus_reserve`): the runtime holds
//! what is published to it, and each of the tree's registrations of it
//! claims its copy (`lotus_bus_reserve_claim`) and gets it once its birth
//! completes, like anything else published during it. The root's birth
//! closes the scope (`lotus_bus_reserve_end`).
//!
//! Which nodes take part:
//!
//! - **Claimants** are the nodes whose registrations run on the
//!   constructing thread, in the root's function: the root, a cooperative
//!   param child, recursively, and a pinned anchor (it registers on the
//!   instantiating thread, after its thread initialized its params). A
//!   subscription registers through the bus (no `shm_ring` binding) to a
//!   locus the plan owes readiness; keyed ones take part, their key
//!   applied when they claim.
//! - **Publishers** are every node, the subtrees off the constructing
//!   thread included (a pinned anchor's params, a pool-placed field and
//!   its params): those publish from the anchor's instantiation on.
//!
//! A subject is reserved only where some publisher may publish before
//! some claimant registers: a cooperative node publishes from its birth,
//! after its subtree is built, an off-thread one from its anchor's
//! instantiation; a claimant registers before its params where it does so
//! early (its unkeyed subscriptions, line 6 of #1533), else just before
//! its birth. A tree whose subjects need none emits nothing, so a program
//! with no such tree pays nothing.
//!
//! The subtrees a pinned or pool-placed field builds on its own thread
//! register there: they are not claimants of the root's reservations
//! (their own literals are roots of the trees they build). That, and a
//! replicated field (`replicas = K`), are the remainder.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use hale_syntax::ast::{Expr, ScheduleClass, StructInit};
use inkwell::values::{FunctionValue, PointerValue};
use inkwell::AddressSpace;

use crate::codegen::{CodegenError, CodegenTy, Cx, DefaultInit};

/// The reservations of one root literal, as its tree's nodes see them
/// while they are lowered: the runtime scope, the function it lives in
/// (a claim emitted anywhere else is not this tree's), and the reserved
/// subjects.
#[derive(Clone, Debug)]
pub(crate) struct ReserveCtx<'ctx> {
    pub(crate) scope: PointerValue<'ctx>,
    pub(crate) func: FunctionValue<'ctx>,
    pub(crate) subjects: Rc<BTreeSet<String>>,
}

/// How a param's literal takes part in its parent's tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TreeChild {
    /// Built inline on the constructing thread: a node of the tree.
    Cooperative,
    /// A pinned anchor: registers on the constructing thread, builds
    /// its params on its own.
    PinnedAnchor,
    /// Placed on a worker pool, or replicated: publishes only.
    OffThread,
}

struct Claim {
    node: usize,
    reg: u32,
    locus: String,
    payload: String,
}

struct Publish {
    node: usize,
    from: u32,
    pattern: String,
}

#[derive(Default)]
struct Walk {
    pos: u32,
    nodes: usize,
    claims: BTreeMap<String, Vec<Claim>>,
    publishes: Vec<Publish>,
}

/// `hale_model::wildcard_match`'s rule (and the runtime's): a trailing
/// `**` matches its root and everything under it.
fn pattern_matches(pattern: &str, subject: &str) -> bool {
    if pattern == subject {
        return true;
    }
    match pattern.strip_suffix("**") {
        Some("") => true,
        Some(pre) => match pre.strip_suffix('.') {
            Some(root) => subject == root || (subject.starts_with(pre) && subject.len() > pre.len()),
            None => false,
        },
        None => false,
    }
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// The locus a param's value literal builds, when it is one, and how
    /// it takes part in `parent`'s tree. `is_main`: `parent` is the entry
    /// locus, whose fields its `placement { }` places. The same answer
    /// the params loop reads to mark the child a node of the tree.
    pub(crate) fn static_tree_child(
        &self,
        is_main: bool,
        fname: &str,
        value: &Expr,
        field_ty: Option<&CodegenTy>,
    ) -> Option<(String, TreeChild)> {
        let Expr::Struct { path, .. } = value else { return None };
        let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
        let mut name = if segs.len() == 1 { segs[0].to_string() } else { self.mangled_for_path(&segs)? };
        if let Some(ty) = field_ty {
            if let Some(q) = self.resolve_generic_struct_path_for_codegen_ty(path, ty) {
                name = q.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::");
            }
        }
        let info = self.user_loci.get(&name)?;
        if is_main {
            if self.deployment.main_placement_replicas.contains_key(fname) {
                return Some((name, TreeChild::OffThread));
            }
            if let Some(class) = self.deployment.main_placement_map.get(fname) {
                if matches!(class, ScheduleClass::Pinned(_)) {
                    return Some((name, TreeChild::PinnedAnchor));
                }
            }
            if self.deployment.main_cooperative_pools.get(fname).is_some_and(|p| p != "main") {
                return Some((name, TreeChild::OffThread));
            }
        }
        if matches!(info.schedule_class, ScheduleClass::Pinned(_)) {
            return Some((name, TreeChild::PinnedAnchor));
        }
        Some((name, TreeChild::Cooperative))
    }

    /// The literals `locus`'s params build, built with `inits`: each
    /// field's override or `Expr` default that is a locus literal, with
    /// the child's own inits, in the params loop's order.
    fn static_tree_children(&self, locus: &str, inits: &[StructInit]) -> Vec<(String, TreeChild, Vec<StructInit>)> {
        let Some(info) = self.user_loci.get(locus) else { return Vec::new() };
        let is_main = self.is_entry_locus(locus);
        let mut out = Vec::new();
        for (fname, default) in &info.defaults {
            let value = match inits.iter().find(|i| i.name.name == *fname) {
                Some(i) => &i.value,
                None => match default {
                    DefaultInit::Expr(e) => e,
                    _ => continue,
                },
            };
            let field_ty = info.fields.get(fname).map(|(_, t)| t);
            if let Some((child, how)) = self.static_tree_child(is_main, fname, value, field_ty) {
                let Expr::Struct { inits: child_inits, .. } = value else { continue };
                out.push((child, how, child_inits.clone()));
            }
        }
        out
    }

    /// Whether `locus`'s unkeyed bus subscriptions register before its
    /// params (#1533), as the instantiation decides it for a cooperative
    /// node of a tree lowered under the current method.
    fn registers_early(&self, locus: &str) -> bool {
        let Some(info) = self.user_loci.get(locus) else { return false };
        let accepted_or_bubbled = self.current_self.as_ref().is_some_and(|cs| {
            let accepts = match self.specialized_accepts.get(&cs.locus_name) {
                Some(a) => a.contains(locus),
                None => self.ownership_accepts.get(&cs.locus_name).is_some_and(|a| a.contains(locus)),
            };
            let key = (cs.locus_name.clone(), locus.to_string());
            accepts
                || self.ownership_bubble_plan.contains_key(&key)
                || self.ownership_bubble_nonsingleton_plan.contains_key(&key)
                || self.ownership_bubble_mixed_plan.contains_key(&key)
        });
        !matches!(info.schedule_class, ScheduleClass::Pinned(_))
            && self.owes_readiness(locus)
            && !accepted_or_bubbled
            && info
                .subscriptions
                .iter()
                .any(|(s, _, _, k)| k.is_none() && !self.shm_ring_subjects.contains_key(s))
    }

    fn walk_tree(&self, locus: &str, inits: &[StructInit], how: TreeChild, off_from: Option<u32>, depth: u32, w: &mut Walk) {
        // The lowering laws refuse a cycle through param defaults before
        // lowering; the bound is a belt.
        if depth > 64 {
            return;
        }
        let Some(info) = self.user_loci.get(locus) else { return };
        let node = w.nodes;
        w.nodes += 1;
        let pre = w.pos;
        w.pos += 1;
        let children = self.static_tree_children(locus, inits);
        let on_thread = off_from.is_none() && how != TreeChild::OffThread;
        for (child, child_how, child_inits) in &children {
            let child_off = match (off_from, how) {
                (Some(f), _) => Some(f),
                (None, TreeChild::Cooperative) => None,
                (None, _) => Some(pre),
            };
            self.walk_tree(child, child_inits, *child_how, child_off, depth + 1, w);
        }
        let post = w.pos;
        w.pos += 1;
        let from = match (off_from, how) {
            (Some(f), _) => f,
            (None, TreeChild::Cooperative) => post,
            (None, _) => pre,
        };
        for pattern in &info.publish_patterns {
            w.publishes.push(Publish { node, from, pattern: pattern.clone() });
        }
        if !on_thread || !self.owes_readiness(locus) {
            return;
        }
        let early = how == TreeChild::Cooperative && self.registers_early(locus);
        for (subject, _, payload, key) in &info.subscriptions {
            if self.shm_ring_subjects.contains_key(subject) {
                continue;
            }
            let reg = if early && key.is_none() { pre } else { post };
            w.claims.entry(subject.clone()).or_default().push(Claim {
                node,
                reg,
                locus: locus.to_string(),
                payload: payload.clone(),
            });
        }
    }

    /// The reservations the tree rooted at `root` (built with `inits`)
    /// needs, as (subject, claims, payload type, bound, policy, String
    /// key). Empty when none does.
    pub(crate) fn static_tree_plan(&self, root: &str, inits: &[StructInit]) -> Vec<ReservePlan> {
        let mut w = Walk::default();
        self.walk_tree(root, inits, TreeChild::Cooperative, None, 0, &mut w);
        let mut plans = Vec::new();
        for (subject, claims) in &w.claims {
            let needed = w.publishes.iter().any(|p| {
                pattern_matches(&p.pattern, subject)
                    && claims.iter().any(|c| c.node != p.node && p.from < c.reg)
            });
            if !needed {
                continue;
            }
            // The topic's bound: `bounded(N); on_full: fail;`, else the
            // most generous of the claimants' own, when every one bounds
            // it (each still sheds by its own once the cells reach it).
            let (cap, policy) = match self.full_fail_subjects.get(subject) {
                Some(&cap) => (cap, 3),
                None => {
                    let bounds: Vec<(i64, u8)> = claims
                        .iter()
                        .filter_map(|c| self.sub_bounds.get(&(c.locus.clone(), subject.clone())).copied())
                        .collect();
                    if bounds.len() == claims.len() {
                        bounds.into_iter().max_by_key(|(cap, _)| *cap).map(|(c, p)| (c, i64::from(p))).unwrap_or((0, 0))
                    } else {
                        (0, 0)
                    }
                }
            };
            let key_is_str = self.routing_key_subjects.get(subject).is_some_and(|k| {
                self.user_types
                    .get(&k.payload_type_name)
                    .and_then(|t| t.fields.get(&k.keyed_by_field))
                    .is_some_and(|(_, ty)| matches!(ty, CodegenTy::String))
            });
            plans.push(ReservePlan {
                subject: subject.clone(),
                count: claims.len() as i64,
                payload: claims[0].payload.clone(),
                cap,
                policy,
                key_is_str,
            });
        }
        plans
    }

    fn reserve_fn(&mut self, name: &str) -> FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function(name) {
            return f;
        }
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i64_t = self.context.i64_type();
        let i32_t = self.context.i32_type();
        let void_t = self.context.void_type();
        let ty = match name {
            "lotus_bus_reserve_begin" => ptr_t.fn_type(&[], false),
            "lotus_bus_reserve" => void_t.fn_type(
                &[ptr_t.into(), ptr_t.into(), i64_t.into(), ptr_t.into(), i32_t.into(), i64_t.into(), i64_t.into(), i32_t.into()],
                false,
            ),
            "lotus_bus_reserve_claim" => void_t.fn_type(&[ptr_t.into(), ptr_t.into(), ptr_t.into()], false),
            _ => void_t.fn_type(&[ptr_t.into()], false),
        };
        self.module.add_function(name, ty, None)
    }

    /// The root's reservations, before its params are built: the scope
    /// its nodes claim against, or `None` when the tree needs none.
    pub(crate) fn emit_reservations(&mut self, root: &str, inits: &[StructInit]) -> Result<Option<ReserveCtx<'ctx>>, CodegenError> {
        let plans = self.static_tree_plan(root, inits);
        if plans.is_empty() {
            return Ok(None);
        }
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let func = self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_parent())
            .expect("an instantiation is inside a function");
        let begin = self.reserve_fn("lotus_bus_reserve_begin");
        let scope = self
            .builder
            .build_call(begin, &[], "bus.reserve.scope")
            .map_err(e)?
            .try_as_basic_value()
            .left()
            .expect("lotus_bus_reserve_begin returns ptr")
            .into_pointer_value();
        let reserve = self.reserve_fn("lotus_bus_reserve");
        let i64_t = self.context.i64_type();
        let i32_t = self.context.i32_type();
        let mut subjects = BTreeSet::new();
        for plan in &plans {
            let subj = self.global_string(&plan.subject);
            let deserialize = match self.codec_thunks.get(&plan.subject) {
                Some(t) => t.decode.as_global_value().as_pointer_value(),
                None => match self.serializers.get(&plan.payload) {
                    Some(s) => s.deserialize.as_global_value().as_pointer_value(),
                    None => continue,
                },
            };
            let static_id = self.bus_devirt_ids.get(&plan.subject).copied().unwrap_or(u32::MAX);
            self.builder
                .build_call(
                    reserve,
                    &[
                        scope.into(),
                        subj.into(),
                        i64_t.const_int(plan.count as u64, false).into(),
                        deserialize.into(),
                        i32_t.const_int(u64::from(static_id), false).into(),
                        i64_t.const_int(plan.cap as u64, true).into(),
                        i64_t.const_int(plan.policy as u64, false).into(),
                        i32_t.const_int(u64::from(plan.key_is_str), false).into(),
                    ],
                    "bus.reserve",
                )
                .map_err(e)?;
            subjects.insert(plan.subject.clone());
        }
        Ok(Some(ReserveCtx { scope, func, subjects: Rc::new(subjects) }))
    }

    /// A node's registration of `subject` claims what the tree's
    /// reservation of it holds.
    pub(crate) fn emit_reserve_claim(
        &mut self,
        ctx: &ReserveCtx<'ctx>,
        subject: &str,
        self_ptr: PointerValue<'ctx>,
    ) -> Result<(), CodegenError> {
        if !ctx.subjects.contains(subject) {
            return Ok(());
        }
        let here = self.builder.get_insert_block().and_then(|b| b.get_parent());
        if here != Some(ctx.func) {
            return Ok(());
        }
        let claim = self.reserve_fn("lotus_bus_reserve_claim");
        let subj = self.global_string(subject);
        self.builder
            .build_call(claim, &[ctx.scope.into(), subj.into(), self_ptr.into()], "bus.reserve.claim")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// The root's birth is next: the scope closes.
    pub(crate) fn emit_reserve_end(&mut self, ctx: &ReserveCtx<'ctx>) -> Result<(), CodegenError> {
        let end = self.reserve_fn("lotus_bus_reserve_end");
        self.builder
            .build_call(end, &[ctx.scope.into()], "bus.reserve.end")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }
}

/// One subject's reservation.
#[derive(Clone, Debug)]
pub(crate) struct ReservePlan {
    pub(crate) subject: String,
    pub(crate) count: i64,
    pub(crate) payload: String,
    pub(crate) cap: i64,
    pub(crate) policy: i64,
    pub(crate) key_is_str: bool,
}

#[cfg(test)]
mod tests {
    use super::pattern_matches;

    #[test]
    fn wildcard_rule_matches_the_runtime() {
        assert!(pattern_matches("x.ping", "x.ping"));
        assert!(pattern_matches("**", "anything"));
        assert!(pattern_matches("log.app.**", "log.app"));
        assert!(pattern_matches("log.app.**", "log.app.db"));
        assert!(!pattern_matches("log.app.**", "log.apple"));
        assert!(!pattern_matches("log**", "log.x"));
    }
}
