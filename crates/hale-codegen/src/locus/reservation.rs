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
//! - **Claimants** are the nodes whose registrations run while the root
//!   is built: the root, a cooperative param child, recursively, and an
//!   anchor (a pinned or pool-placed field), which registers on the
//!   instantiating thread once its own thread (or its pool's worker) has
//!   built its params — and the nodes of those params, which register in
//!   that init while the instantiating thread waits for it; the init
//!   reads the root's scope through its argument block. A subscription
//!   registers through the bus (no `shm_ring` binding) to a locus the
//!   plan owes readiness; keyed ones take part, their key applied when
//!   they claim.
//! - **Publishers** are every node: an anchor publishes from its
//!   instantiation on, a replicated field's instances too.
//!
//! A subject is reserved only where some publisher may publish before
//! some claimant registers: a cooperative node publishes from its birth,
//! after its subtree is built, an off-thread one from its anchor's
//! instantiation; a claimant registers before its params where it does so
//! early (its unkeyed subscriptions, line 6 of #1533), else just before
//! its birth. A tree whose subjects need none emits nothing, so a program
//! with no such tree pays nothing.
//!
//! An anchor nested in another anchor's params builds its own params in
//! an init of that init: those nodes, and a replicated field's
//! (`replicas = K`) instances, are not claimants; that is the remainder.

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
    /// 0 in the root's function; 1 in the params init of an anchor the
    /// root's function builds (its thread's, or its pool worker's),
    /// which reads `scope` through the init's argument block. Deeper
    /// inits take part as publishers only.
    pub(crate) depth: u8,
    /// The root site's entry-block slot holding `scope` while it is open
    /// (`Cx::reserve_exit_slots`).
    pub(crate) slot: PointerValue<'ctx>,
}

/// How a param's literal takes part in its parent's tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TreeChild {
    /// Built inline, in its parent's function: a node of the tree.
    Cooperative,
    /// A pinned or pool-placed anchor: registers in its parent's
    /// function, and its params are built in an init its own thread (or
    /// its pool's worker) runs while the instantiating thread waits.
    Anchor,
    /// A replicated field's instances: publish only.
    Replicated,
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
                return Some((name, TreeChild::Replicated));
            }
            if let Some(class) = self.deployment.main_placement_map.get(fname) {
                if matches!(class, ScheduleClass::Pinned(_)) {
                    return Some((name, TreeChild::Anchor));
                }
            }
            if self.deployment.main_cooperative_pools.get(fname).is_some_and(|p| p != "main") {
                return Some((name, TreeChild::Anchor));
            }
        }
        if matches!(info.schedule_class, ScheduleClass::Pinned(_)) {
            return Some((name, TreeChild::Anchor));
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

    /// `fn_depth`: the init depth of the function this node's literal is
    /// lowered in (0, the root's). `off_from`: inside a replicated
    /// field's subtree, the position its replicas start from.
    #[allow(clippy::too_many_arguments)]
    fn walk_tree(
        &self,
        locus: &str,
        inits: &[StructInit],
        how: TreeChild,
        fn_depth: u32,
        off_from: Option<u32>,
        depth: u32,
        w: &mut Walk,
    ) {
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
        let child_depth = if how == TreeChild::Anchor { fn_depth + 1 } else { fn_depth };
        for (child, child_how, child_inits) in &children {
            let child_off = match (off_from, child_how) {
                (Some(f), _) => Some(f),
                (None, TreeChild::Replicated) => Some(w.pos),
                (None, _) => None,
            };
            self.walk_tree(child, child_inits, *child_how, child_depth, child_off, depth + 1, w);
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
        if off_from.is_some() || fn_depth > 1 || !self.owes_readiness(locus) {
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
    pub(crate) fn static_tree_plan(&self, root: &str, inits: &[StructInit], root_how: TreeChild) -> Vec<ReservePlan> {
        let mut w = Walk::default();
        self.walk_tree(root, inits, root_how, 0, None, 0, &mut w);
        let mut plans = Vec::new();
        for (subject, claims) in &w.claims {
            let needed = w.publishes.iter().any(|p| {
                pattern_matches(&p.pattern, subject)
                    && claims.iter().any(|c| c.node != p.node && p.from < c.reg)
            });
            if !needed {
                continue;
            }
            // What the reservation retains: each claimant's own bound
            // applies at its claim, so it keeps the first cells any
            // drop_new or unbounded claimant (or the topic's `on_full:
            // fail`) keeps, and the last any drop_old claimant keeps.
            let (keep_first, keep_last, note) = match self.full_fail_subjects.get(subject) {
                Some(&cap) => (cap, 0, 3),
                None => {
                    let mut first: Option<i64> = None;
                    let mut last = 0i64;
                    let mut unbounded = false;
                    for c in claims {
                        match self.sub_bounds.get(&(c.locus.clone(), subject.clone())).copied() {
                            Some((cap, 2)) => last = last.max(cap),
                            Some((cap, _)) => first = Some(first.map_or(cap, |f| f.max(cap))),
                            None => unbounded = true,
                        }
                    }
                    if unbounded {
                        (-1, last, 1)
                    } else {
                        (first.unwrap_or(0), last, 0)
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
                keep_first,
                keep_last,
                note,
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
                &[ptr_t.into(), ptr_t.into(), i64_t.into(), ptr_t.into(), i32_t.into(), i64_t.into(), i64_t.into(), i32_t.into(), i32_t.into()],
                false,
            ),
            "lotus_bus_reserve_claim" => void_t.fn_type(&[ptr_t.into(), ptr_t.into(), ptr_t.into()], false),
            _ => void_t.fn_type(&[ptr_t.into()], false),
        };
        self.module.add_function(name, ty, None)
    }

    /// The root's reservations, before its params are built: the scope
    /// its nodes claim against, or `None` when the tree needs none.
    pub(crate) fn emit_reservations(
        &mut self,
        root: &str,
        inits: &[StructInit],
        root_how: TreeChild,
    ) -> Result<Option<ReserveCtx<'ctx>>, CodegenError> {
        let plans = self.static_tree_plan(root, inits, root_how);
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
                        i64_t.const_int(plan.keep_first as u64, true).into(),
                        i64_t.const_int(plan.keep_last as u64, true).into(),
                        i32_t.const_int(u64::from(plan.note), false).into(),
                        i32_t.const_int(u64::from(plan.key_is_str), false).into(),
                    ],
                    "bus.reserve",
                )
                .map_err(e)?;
            subjects.insert(plan.subject.clone());
        }
        // The site's slot, NULL on every path until here: an exit out of
        // the construction finds the scope in it and closes it.
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let entry = func.get_first_basic_block().expect("a function has an entry block");
        let here = self.builder.get_insert_block();
        match entry.get_first_instruction() {
            Some(first) => self.builder.position_before(&first),
            None => self.builder.position_at_end(entry),
        }
        let slot = self.builder.build_alloca(ptr_t, "bus.reserve.slot").map_err(e)?;
        self.builder.build_store(slot, ptr_t.const_null()).map_err(e)?;
        if let Some(bb) = here {
            self.builder.position_at_end(bb);
        }
        if let Some(loc) = self.di_current_loc {
            self.builder.set_current_debug_location(loc);
        }
        self.builder.build_store(slot, scope).map_err(e)?;
        self.reserve_exit_slots.push((func, slot));
        Ok(Some(ReserveCtx { scope, func, subjects: Rc::new(subjects), depth: 0, slot }))
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
        let null = self.context.ptr_type(AddressSpace::default()).const_null();
        self.builder.build_store(ctx.slot, null).map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// An exit of this function: a root site whose slot still holds its
    /// scope was left part-way (an initializer's propagated failure), and
    /// its scope closes here. A completed construction's slot is NULL,
    /// which the close ignores.
    pub(crate) fn emit_reserve_scopes_abandoned(&mut self) -> Result<(), CodegenError> {
        let here = self.builder.get_insert_block().and_then(|b| b.get_parent());
        let slots: Vec<PointerValue<'ctx>> = self
            .reserve_exit_slots
            .iter()
            .filter(|(f, _)| Some(*f) == here)
            .map(|(_, s)| *s)
            .collect();
        if slots.is_empty() {
            return Ok(());
        }
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let end = self.reserve_fn("lotus_bus_reserve_end");
        for slot in slots.iter().rev() {
            let scope = self.builder.build_load(ptr_t, *slot, "bus.reserve.left").map_err(e)?;
            self.builder.build_call(end, &[scope.into()], "bus.reserve.abandoned").map_err(e)?;
            self.builder.build_store(*slot, ptr_t.const_null()).map_err(e)?;
        }
        Ok(())
    }
}

/// One subject's reservation.
#[derive(Clone, Debug)]
pub(crate) struct ReservePlan {
    pub(crate) subject: String,
    pub(crate) count: i64,
    pub(crate) payload: String,
    /// The first cells it retains (a drop_new, unbounded or `on_full:
    /// fail` claimant's); -1 for a queue's worth.
    pub(crate) keep_first: i64,
    /// The last cells it retains after them (a drop_old claimant's).
    pub(crate) keep_last: i64,
    /// Whether shedding past `keep_first` is said: 1 a claimant is
    /// unbounded, 3 the topic is `on_full: fail`.
    pub(crate) note: u8,
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
