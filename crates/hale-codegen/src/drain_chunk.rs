//! The drain's stop on a counted loop, checked once a chunk (GH #2089).
//!
//! GH #1858 ends every loop of `fn main` and of a `run()` that does not
//! read `draining` between two iterations: each loop's back-edge is a
//! latch, one monotonic load of `lotus_process_draining_flag` and a
//! branch (`Cx::push_loop_frame`). On a tight counted loop that load is
//! what LLVM cannot move: an atomic load inside the loop keeps the loop
//! vectorizer away, and a reduction that ran at AVX-512 width ran scalar
//! (`loop_overhead` 1.86 → 19.9 ms, `form_vec_get` 13 → 42 µs).
//!
//! A counted loop — `for i in a..b`, or `while i < n { …; i = i + 1; }`
//! with `i` stepped only there and `n` never written — is lowered here as
//! two loops when its body is a *leaf*: no call (an LLVM intrinsic or a
//! `noreturn` failure path aside) and no loop of its own. The outer loop
//! takes the latch once per chunk; the inner one runs the chunk with no
//! latch at all:
//!
//! ```text
//!   chunk:  end = (i < hi && hi - i > K) ? i + K : hi
//!   inner:  i < end ? body : chunk_exit        // the loop LLVM vectorizes
//!   chunk_exit: i < hi ? latch : exit
//!   latch:  stop ? stop_end : chunk            // the #1858 check
//! ```
//!
//! The inner loop has one exit and a trip count known on entry (`end -
//! i`), the shape LLVM 18's vectorizer takes. A chunk is
//! [`DRAIN_CHUNK`] iterations, so a stop is seen at most `DRAIN_CHUNK - 1`
//! iterations later than before: iterations of a body that calls nothing
//! and loops on nothing, so straight-line work (`loop_overhead`'s chunk
//! of 4096 XORs is about 1 µs; a body of a few hundred instructions is a
//! few hundred µs). A body that calls or loops keeps the per-iteration
//! latch: its iterations have no bound, so a chunk of them could hold a
//! stop for as long as they take.
//!
//! The decision is made from the body's IR, after the body is lowered:
//! its blocks are scanned for calls, and the loops it pushed are counted.
//! The header and back-edge are wired afterwards, chunked or not.

use std::collections::HashSet;

use inkwell::basic_block::BasicBlock;
use inkwell::values::{InstructionOpcode, IntValue, PointerValue};
use inkwell::IntPredicate;

use hale_syntax::ast::{AssignOp, BinOp, Block, ElseBranch, Expr, Ident, IfStmt, LValueSeg, Literal, OrDisposition, Stmt};

use crate::codegen::{BlockEnd, CodegenError, CodegenTy, Cx, LoopFrame, Scope};

/// Iterations of a leaf counted loop between two looks at the drain flag.
pub(crate) const DRAIN_CHUNK: u64 = 4096;

fn llvm(e: inkwell::builder::BuilderError) -> CodegenError {
    CodegenError::LlvmEmit(e.to_string())
}

/// `while i < n { …; i = i + 1; }`: the counted shape, its two names.
pub(crate) struct CountedWhile<'a> {
    var: &'a Ident,
    bound: &'a Expr,
}

impl<'a> CountedWhile<'a> {
    /// The counted shape, or `None`: the condition `i < n` with `n` a name
    /// or an integer literal, and the body's last statement the plain step
    /// `i = i + 1`. The rest of the body must be read whole and found to
    /// leave `i` and `n` alone: the chunked loop computes its chunk's end
    /// from `n` once and counts steps, so a write of `n` it did not see
    /// would run the loop past its bound, and a way round that skipped the
    /// step (a `continue`, a write of `i`) could keep a stop out for good.
    /// So the walk is a whitelist ([`stmt_keeps`], [`expr_keeps`]): a form
    /// it does not know — a `match` (an `if let` and a `let … else` desugar
    /// to one), a `continue`, a loop, a closure — refuses the shape, and the
    /// loop keeps the latch on every back-edge.
    pub(crate) fn of(cond: &'a Expr, body: &'a Block) -> Option<Self> {
        let Expr::Binary { op: BinOp::Lt, left, right, .. } = cond else { return None };
        let Expr::Ident(var) = left.as_ref() else { return None };
        let bound_name = match right.as_ref() {
            Expr::Ident(n) if n.name != var.name => Some(n.name.as_str()),
            Expr::Literal(Literal::Int(_), _) => None,
            _ => return None,
        };
        if body.tail.is_some() {
            return None;
        }
        let (last, rest) = body.stmts.split_last()?;
        let Stmt::Assign { target, op: AssignOp::Eq, value, .. } = last else { return None };
        let steps = target.head.name == var.name
            && target.tail.is_empty()
            && matches!(value, Expr::Binary { op: BinOp::Add, left, right, .. }
                if matches!(left.as_ref(), Expr::Ident(l) if l.name == var.name)
                    && matches!(right.as_ref(), Expr::Literal(Literal::Int(1), _)));
        if !steps {
            return None;
        }
        let names: Vec<&str> = std::iter::once(var.name.as_str()).chain(bound_name).collect();
        rest.iter().all(|s| stmt_keeps(s, &names)).then_some(CountedWhile { var, bound: right })
    }
}

/// `s` is a statement form the walk reads whole, and it neither writes nor
/// binds any of `names` and never goes round the loop early. Anything else
/// is refused.
fn stmt_keeps(s: &Stmt, names: &[&str]) -> bool {
    match s {
        Stmt::Let { name, value, .. } => !names.contains(&name.name.as_str()) && expr_keeps(value, names),
        Stmt::Assign { target, value, .. } => {
            !names.contains(&target.head.name.as_str())
                && target.tail.iter().all(|seg| match seg {
                    LValueSeg::Field(_) => true,
                    LValueSeg::Index(e) => expr_keeps(e, names),
                })
                && expr_keeps(value, names)
        }
        Stmt::If(i) => if_keeps(i, names),
        Stmt::Block(b) => block_keeps(b, names),
        Stmt::Expr(e) => expr_keeps(e, names),
        Stmt::Return(e, _) => e.as_ref().map_or(true, |e| expr_keeps(e, names)),
        Stmt::Break(_) => true,
        _ => false,
    }
}

fn block_keeps(b: &Block, names: &[&str]) -> bool {
    b.stmts.iter().all(|s| stmt_keeps(s, names)) && b.tail.as_deref().map_or(true, |e| expr_keeps(e, names))
}

fn if_keeps(i: &IfStmt, names: &[&str]) -> bool {
    expr_keeps(&i.cond, names)
        && block_keeps(&i.then_block, names)
        && match i.else_block.as_deref() {
            None => true,
            Some(ElseBranch::Else(b)) => block_keeps(b, names),
            Some(ElseBranch::ElseIf(inner)) => if_keeps(inner, names),
        }
}

/// `e` is an expression form the walk reads whole, every statement inside
/// it (a block's, an `if`'s) kept as [`stmt_keeps`] keeps one. A call
/// cannot write a caller's `Int` local, so its arguments are only read.
fn expr_keeps(e: &Expr, names: &[&str]) -> bool {
    match e {
        Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => true,
        Expr::Binary { left, right, .. } => expr_keeps(left, names) && expr_keeps(right, names),
        Expr::Unary { operand, .. } => expr_keeps(operand, names),
        Expr::Call { callee, args, .. } => expr_keeps(callee, names) && args.iter().all(|a| expr_keeps(a, names)),
        Expr::Field { receiver, .. } => expr_keeps(receiver, names),
        Expr::Index { receiver, index, .. } => expr_keeps(receiver, names) && expr_keeps(index, names),
        Expr::Tuple(parts, _) | Expr::Array(parts, _) => parts.iter().all(|p| expr_keeps(p, names)),
        Expr::Struct { inits, .. } => inits.iter().all(|i| expr_keeps(&i.value, names)),
        Expr::Block(b) => block_keeps(b, names),
        Expr::If(i) => if_keeps(i, names),
        Expr::Range { lo, hi, .. } => expr_keeps(lo, names) && expr_keeps(hi, names),
        Expr::Or { inner, disposition, .. } => {
            expr_keeps(inner, names)
                && match disposition {
                    OrDisposition::Substitute(sub) => expr_keeps(sub, names),
                    OrDisposition::Raise(_) | OrDisposition::Discard(_) => true,
                    _ => false,
                }
        }
        _ => false,
    }
}

/// The blocks a terminator may branch to, a branch on a constant
/// condition taking only the edge it takes.
fn live_successors(bb: BasicBlock<'_>) -> Vec<BasicBlock<'_>> {
    let Some(term) = bb.get_terminator() else { return Vec::new() };
    if term.get_opcode() == InstructionOpcode::Br && term.get_num_operands() == 3 {
        if let Some(inkwell::values::BasicValueEnum::IntValue(c)) = term.get_operand(0).and_then(|o| o.left()) {
            if let Some(v) = c.get_zero_extended_constant() {
                // a conditional br's operands: cond, else, then
                let taken = if v != 0 { 2 } else { 1 };
                return term.get_operand(taken).and_then(|o| o.right()).into_iter().collect();
            }
        }
    }
    (0..term.get_num_operands()).filter_map(|k| term.get_operand(k).and_then(|o| o.right())).collect()
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// Whether loops lowered here take the drain's stop (GH #1858).
    pub(crate) fn backedge_stops_here(&self) -> bool {
        matches!((self.backedge_stop_fn, self.current_fn), (Some(stop), Some(cur)) if stop == cur)
    }

    /// The #1858 latch in a block of its own: one monotonic load of the
    /// drain flag, and a cold branch to a park point and `brk` when a stop
    /// is up, else to `cont`. The builder's position is kept.
    pub(crate) fn build_drain_latch(
        &mut self,
        cont: BasicBlock<'ctx>,
        brk: BasicBlock<'ctx>,
    ) -> Result<BasicBlock<'ctx>, CodegenError> {
        let cur = self.current_fn.expect("a loop is lowered inside a fn");
        let here = self.builder.get_insert_block().expect("a loop is lowered inside a block");
        let latch = self.context.append_basic_block(cur, "loop.drain_stop");
        self.builder.position_at_end(latch);
        let flag = self.emit_process_draining_load("loop.drain_stop.flag")?;
        let i64_t = self.context.i64_type();
        // The runtime's LOTUS_DRAIN_STOPS: SIGNAL (1) | TEST (4).
        let stops = self.builder.build_and(flag, i64_t.const_int(1 | 4, false), "loop.drain_stop.bits").map_err(llvm)?;
        let up = self
            .builder
            .build_int_compare(IntPredicate::NE, stops, i64_t.const_zero(), "loop.drain_stop.up")
            .map_err(llvm)?;
        let stop_bb = self.context.append_basic_block(cur, "loop.drain_stop.end");
        let br = self.builder.build_conditional_branch(up, stop_bb, cont).map_err(llvm)?;
        self.builder.position_at_end(stop_bb);
        self.emit_unwind_poll()?;
        self.builder.build_unconditional_branch(brk).map_err(llvm)?;
        let weights = self.context.metadata_node(&[
            self.context.metadata_string("branch_weights").into(),
            self.context.i32_type().const_int(1, false).into(),
            self.context.i32_type().const_int(1 << 20, false).into(),
        ]);
        let kind = self.context.get_kind_id("prof");
        br.set_metadata(weights, kind).map_err(|err| CodegenError::LlvmEmit(err.to_string()))?;
        self.builder.position_at_end(here);
        Ok(latch)
    }

    /// Whether the blocks a loop body lowered to are a leaf: the body
    /// pushed no loop of its own, and no block of it that runs and can
    /// still lead to the next iteration calls anything but an LLVM
    /// intrinsic or a `noreturn` fn. A block behind a branch on a constant
    /// never runs (a `.get` whose bounds check was eliminated keeps its
    /// error path behind `br i1 false`), and a block that cannot reach the
    /// back-edge leaves the loop (a raise, a panic, a `return`): a call in
    /// either cannot hold a stop back.
    fn body_is_leaf(
        &self,
        before: &HashSet<BasicBlock<'ctx>>,
        entry: BasicBlock<'ctx>,
        back: BasicBlock<'ctx>,
        loops_before: u64,
    ) -> bool {
        if self.loops_lowered != loops_before {
            return false;
        }
        let in_body = |bb: &BasicBlock<'ctx>| *bb == entry || !before.contains(bb);
        // the blocks that run, from the body's entry
        let mut live: Vec<BasicBlock<'ctx>> = Vec::new();
        let mut work = vec![entry];
        while let Some(bb) = work.pop() {
            if !in_body(&bb) || live.contains(&bb) {
                continue;
            }
            live.push(bb);
            work.extend(live_successors(bb));
        }
        // of those, the ones that can still reach the back-edge
        let mut reaches: HashSet<BasicBlock<'ctx>> = HashSet::new();
        loop {
            let grew = live
                .iter()
                .filter(|bb| !reaches.contains(*bb))
                .filter(|bb| live_successors(**bb).iter().any(|s| *s == back || reaches.contains(s)))
                .copied()
                .collect::<Vec<_>>();
            if grew.is_empty() {
                break;
            }
            reaches.extend(grew);
        }
        let noreturn = inkwell::attributes::Attribute::get_named_enum_kind_id("noreturn");
        for bb in reaches {
            let mut i = bb.get_first_instruction();
            while let Some(inst) = i {
                i = inst.get_next_instruction();
                if inst.get_opcode() != InstructionOpcode::Call {
                    continue;
                }
                let n = inst.get_num_operands();
                let Some(inkwell::values::BasicValueEnum::PointerValue(callee)) =
                    n.checked_sub(1).and_then(|last| inst.get_operand(last)).and_then(|o| o.left())
                else {
                    return false;
                };
                let name = callee.get_name().to_string_lossy().into_owned();
                if name.starts_with("llvm.") {
                    continue;
                }
                let Some(f) = self.module.get_function(&name) else { return false };
                if f.get_enum_attribute(inkwell::attributes::AttributeLoc::Function, noreturn).is_none() {
                    return false;
                }
            }
        }
        true
    }

    /// The chunk's end: `i + K` when `[i, hi)` holds more than K, else `hi`.
    fn chunk_end(&mut self, i: IntValue<'ctx>, hi: IntValue<'ctx>) -> Result<IntValue<'ctx>, CodegenError> {
        let i64_t = self.context.i64_type();
        let k = i64_t.const_int(DRAIN_CHUNK, false);
        let in_range = self.builder.build_int_compare(IntPredicate::SLT, i, hi, "chunk.in").map_err(llvm)?;
        // i < hi, so hi - i, read unsigned, is the exact distance
        let left = self.builder.build_int_sub(hi, i, "chunk.left").map_err(llvm)?;
        let more = self.builder.build_int_compare(IntPredicate::UGT, left, k, "chunk.more").map_err(llvm)?;
        let take = self.builder.build_and(in_range, more, "chunk.take").map_err(llvm)?;
        let stepped = self.builder.build_int_add(i, k, "chunk.stepped").map_err(llvm)?;
        Ok(self.builder.build_select(take, stepped, hi, "chunk.end").map_err(llvm)?.into_int_value())
    }

    /// Lower one counted loop around `lower_body`, chunked when the body
    /// turns out a leaf. `i` and `hi` read the counter and the bound,
    /// `step` is the back-edge's own work (the range's increment; nothing
    /// for a `while`, whose body steps), positioned in the block it gets.
    #[allow(clippy::too_many_arguments)]
    fn lower_counted_loop(
        &mut self,
        prefix: &str,
        read_i: &mut dyn FnMut(&mut Self) -> Result<IntValue<'ctx>, CodegenError>,
        read_hi: &mut dyn FnMut(&mut Self, &mut Scope<'ctx>) -> Result<IntValue<'ctx>, CodegenError>,
        enter_body: &mut dyn FnMut(&mut Self, &mut Scope<'ctx>) -> Result<(), CodegenError>,
        body: &Block,
        step: &mut dyn FnMut(&mut Self) -> Result<(), CodegenError>,
        leave_body: &mut dyn FnMut(&mut Self, &mut Scope<'ctx>),
        scope: &mut Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        let func = self.current_fn.expect("a counted loop is lowered inside a fn");
        let i64_t = self.context.i64_type();
        let end_slot: PointerValue<'ctx> = self.alloca_in_entry(i64_t.into(), &format!("{prefix}.chunk.end"))?;
        let enter_bb = self.context.append_basic_block(func, &format!("{prefix}.enter"));
        let header_bb = self.context.append_basic_block(func, &format!("{prefix}.cond"));
        let body_bb = self.context.append_basic_block(func, &format!("{prefix}.body"));
        let back_bb = self.context.append_basic_block(func, &format!("{prefix}.back"));
        let exit_bb = self.context.append_basic_block(func, &format!("{prefix}.end"));
        self.builder.build_unconditional_branch(enter_bb).map_err(llvm)?;

        let before: HashSet<BasicBlock<'ctx>> = func.get_basic_blocks().into_iter().collect();
        let loops_before = self.loops_lowered;
        self.builder.position_at_end(body_bb);
        enter_body(self, scope)?;
        self.loops.push(LoopFrame::new(back_bb, exit_bb, true));
        let body_res = self.lower_loop_body(body, scope);
        self.loops.pop();
        leave_body(self, scope);
        let body_end = body_res?;
        if body_end == BlockEnd::Open {
            self.builder.build_unconditional_branch(back_bb).map_err(llvm)?;
        }
        let leaf = self.body_is_leaf(&before, body_bb, back_bb, loops_before);
        self.loops_lowered += 1;

        if leaf {
            let chunk_bb = self.context.append_basic_block(func, &format!("{prefix}.chunk"));
            let chunk_exit_bb = self.context.append_basic_block(func, &format!("{prefix}.chunk.exit"));
            let latch = self.build_drain_latch(chunk_bb, exit_bb)?;
            self.builder.position_at_end(enter_bb);
            self.builder.build_unconditional_branch(chunk_bb).map_err(llvm)?;
            // chunk: where this chunk ends
            self.builder.position_at_end(chunk_bb);
            let i = read_i(self)?;
            let hi = read_hi(self, scope)?;
            let end = self.chunk_end(i, hi)?;
            self.builder.build_store(end_slot, end).map_err(llvm)?;
            self.builder.build_unconditional_branch(header_bb).map_err(llvm)?;
            // inner header: within the chunk
            self.builder.position_at_end(header_bb);
            let i = read_i(self)?;
            let end = self.builder.build_load(i64_t, end_slot, "chunk.end.now").map_err(llvm)?.into_int_value();
            let go = self.builder.build_int_compare(IntPredicate::SLT, i, end, "chunk.go").map_err(llvm)?;
            self.builder.build_conditional_branch(go, body_bb, chunk_exit_bb).map_err(llvm)?;
            // chunk exit: the loop's own condition, then the latch
            self.builder.position_at_end(chunk_exit_bb);
            let i = read_i(self)?;
            let hi = read_hi(self, scope)?;
            let more = self.builder.build_int_compare(IntPredicate::SLT, i, hi, "chunk.more.iters").map_err(llvm)?;
            self.builder.build_conditional_branch(more, latch, exit_bb).map_err(llvm)?;
            // back-edge: step, then the inner header, no latch
            self.builder.position_at_end(back_bb);
            step(self)?;
            self.builder.build_unconditional_branch(header_bb).map_err(llvm)?;
        } else {
            let latch = self.build_drain_latch(header_bb, exit_bb)?;
            self.builder.position_at_end(enter_bb);
            self.builder.build_unconditional_branch(header_bb).map_err(llvm)?;
            self.builder.position_at_end(header_bb);
            let i = read_i(self)?;
            let hi = read_hi(self, scope)?;
            let go = self.builder.build_int_compare(IntPredicate::SLT, i, hi, "counted.go").map_err(llvm)?;
            self.builder.build_conditional_branch(go, body_bb, exit_bb).map_err(llvm)?;
            self.builder.position_at_end(back_bb);
            step(self)?;
            self.builder.build_unconditional_branch(latch).map_err(llvm)?;
        }
        self.builder.position_at_end(exit_bb);
        Ok(BlockEnd::Open)
    }

    /// `for var in lo..hi` in a frame whose loops take the drain's stop:
    /// a counted loop ([`Self::lower_counted_loop`]).
    pub(crate) fn lower_for_range_chunked(
        &mut self,
        var_name: &Ident,
        lo: IntValue<'ctx>,
        hi: IntValue<'ctx>,
        body: &Block,
        scope: &mut Scope<'ctx>,
        bce_pushed: bool,
    ) -> Result<BlockEnd, CodegenError> {
        let i64_t = self.context.i64_type();
        let i_slot = self.builder.build_alloca(i64_t, "for.range.i.slot").map_err(llvm)?;
        self.builder.build_store(i_slot, lo).map_err(llvm)?;
        let hi_slot = self.builder.build_alloca(i64_t, "for.range.hi.slot").map_err(llvm)?;
        self.builder.build_store(hi_slot, hi).map_err(llvm)?;
        let prev = std::cell::RefCell::new(None);
        let res = self.lower_counted_loop(
            "for.range",
            &mut |cx| Ok(cx.builder.build_load(i64_t, i_slot, "for.range.i").map_err(llvm)?.into_int_value()),
            &mut |cx, _| Ok(cx.builder.build_load(i64_t, hi_slot, "for.range.hi").map_err(llvm)?.into_int_value()),
            &mut |cx, scope| {
                let i = cx.builder.build_load(i64_t, i_slot, "for.range.i").map_err(llvm)?;
                let local = cx.alloca_for(&CodegenTy::Int, &var_name.name)?;
                cx.builder.build_store(local, i).map_err(llvm)?;
                *prev.borrow_mut() = Some(scope.locals.insert(var_name.name.clone(), (local, CodegenTy::Int)));
                Ok(())
            },
            body,
            &mut |cx| {
                let now = cx.builder.build_load(i64_t, i_slot, "for.range.i.now").map_err(llvm)?.into_int_value();
                let next = cx.builder.build_int_add(now, i64_t.const_int(1, false), "for.range.i.next").map_err(llvm)?;
                cx.builder.build_store(i_slot, next).map_err(llvm)?;
                Ok(())
            },
            &mut |cx, scope| {
                if bce_pushed {
                    cx.bce_loops.pop();
                }
                match prev.borrow_mut().take() {
                    Some(Some(p)) => {
                        scope.locals.insert(var_name.name.clone(), p);
                    }
                    _ => {
                        scope.locals.remove(&var_name.name);
                    }
                }
            },
            scope,
        );
        res
    }

    /// `while i < n { …; i = i + 1; }` in a frame whose loops take the
    /// drain's stop, `i` an `Int` local: a counted loop
    /// ([`Self::lower_counted_loop`]). `None` when `i` is not a local
    /// `Int` here, for the caller to lower the loop as it is.
    pub(crate) fn lower_while_counted(
        &mut self,
        counted: &CountedWhile<'_>,
        body: &Block,
        scope: &mut Scope<'ctx>,
    ) -> Result<Option<BlockEnd>, CodegenError> {
        let Some((i_slot, CodegenTy::Int)) = scope.locals.get(&counted.var.name).cloned() else { return Ok(None) };
        if let Expr::Ident(n) = counted.bound {
            if !matches!(scope.locals.get(&n.name), Some((_, CodegenTy::Int))) {
                return Ok(None);
            }
        }
        let i64_t = self.context.i64_type();
        let bound = counted.bound;
        let end = self.lower_counted_loop(
            "while.counted",
            &mut |cx| Ok(cx.builder.build_load(i64_t, i_slot, "while.counted.i").map_err(llvm)?.into_int_value()),
            &mut |cx, scope| Ok(cx.lower_expr(bound, scope)?.0.into_int_value()),
            &mut |_, _| Ok(()),
            body,
            &mut |_| Ok(()),
            &mut |_, _| {},
            scope,
        )?;
        Ok(Some(end))
    }
}
