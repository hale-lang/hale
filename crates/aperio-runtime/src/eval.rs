//! Tree-walking interpreter — runs a parsed-and-typed program.
//!
//! v0 scope: enough to run hello-world (literals, builtin
//! call, locus birth + state) through 04-modes (mode methods,
//! self.children, while/for, struct literals). Bus and
//! perspectives are not yet executable — the interpreter
//! errors at first reference to those features.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use aperio_syntax::ast::*;

use crate::builtins;
use crate::bus::BusRouter;
use crate::env::Env;
use crate::value::{DecimalVal, FnRef, LocusHandle, SlotState, Value};

/// A non-local control-flow signal raised by `return`,
/// `break`, `continue`, `bubble(err)`, or a runtime error.
pub enum Signal {
    Return(Value),
    Break,
    Continue,
    /// `bubble(v)` raised inside an `on_failure` handler.
    /// The runtime catches it at the failure-routing layer
    /// and either re-raises to the next parent's handler or
    /// surfaces the wrapped value as the program's failure
    /// (process exits non-zero).
    Bubble(Value),
    /// v1.x-VIOLATE (F.27): a `violate NAME;` statement fired
    /// and was successfully routed to the parent's on_failure
    /// (parent absorbed). Signals divergence of the enclosing
    /// block so subsequent statements don't run, but does not
    /// abort the program — the locus's `draining` flag is set
    /// and the caller frame catches at the next dispatch
    /// boundary (handler exit, lifecycle transition, run-loop
    /// iteration). Caught + cleared by exec_block on the way
    /// out so it propagates only as far as the locus method
    /// body it was raised in.
    Violate,
    /// `std::process::exit(code)` — unwind to `run_main`, which
    /// maps the carried i32 to the program's exit code. Bypasses
    /// the normal dissolve cascade (matches libc exit semantics:
    /// the process terminates immediately).
    Exit(i32),
    Error(String),
}

impl From<String> for Signal {
    fn from(s: String) -> Self {
        Signal::Error(s)
    }
}

pub fn run_program(program: &Program) -> Result<i32, String> {
    let mut program_owned = program.clone();
    aperio_syntax::desugar::desugar_intra_locus_topics(&mut program_owned);
    aperio_syntax::desugar::desugar_topics(&mut program_owned);
    let mut interp = Interpreter::new();
    interp.load_program(&program_owned);
    interp.run_main()
}

pub fn run_bundle(programs: &[&Program]) -> Result<i32, String> {
    let mut interp = Interpreter::new();
    let owned: Vec<Program> = programs
        .iter()
        .map(|p| {
            let mut clone = (*p).clone();
            aperio_syntax::desugar::desugar_intra_locus_topics(&mut clone);
            aperio_syntax::desugar::desugar_topics(&mut clone);
            clone
        })
        .collect();
    for p in &owned {
        interp.load_program(p);
    }
    interp.run_main()
}

/// Run a bundle with a custom bus configuration. Used by tests
/// that exercise specific transport choices (ring buffer) on
/// otherwise-default sources.
pub fn run_bundle_with_bus(
    programs: &[&Program],
    bus_config: Vec<(String, crate::bus::TransportKind)>,
) -> Result<i32, String> {
    let mut interp = Interpreter::new();
    for (subject, kind) in bus_config {
        interp.bus.with_transport(subject, kind);
    }
    for p in programs {
        interp.load_program(p);
    }
    interp.run_main()
}

struct Interpreter {
    env: Env,
    /// Top-level decls indexed by name. Fns and consts are
    /// resolved through env; loci and types live here so we
    /// can instantiate / reference them.
    loci: BTreeMap<String, Rc<LocusDecl>>,
    types: BTreeMap<String, Rc<TypeDecl>>,
    /// Stack of `self` handles — the innermost is the locus
    /// whose lifecycle/method we're executing.
    self_stack: Vec<LocusHandle>,
    /// Stack of "current parent" — the locus that should
    /// receive any unbound child instantiations as anonymous
    /// children. None at top level (main's implicit locus).
    parent_stack: Vec<LocusHandle>,
    /// In-memory bus router. Subscribers register on locus
    /// instantiation; `<-` dispatches through it.
    bus: BusRouter,
    /// Long-lived loci instantiated at program top level (no
    /// enclosing parent on the stack at instantiation time).
    /// Their closures fire at program end via the dissolve
    /// cascade — the F.4 / F.9 endpoint of the audit graph.
    top_level_loci: Vec<LocusHandle>,
    /// m46: closure-accumulator substitution context. Set by
    /// `evaluate_closure` right before lowering the assertion
    /// expressions; cleared after. When set, `eval_expr`'s
    /// `Expr::Sum` arm reads from the next accumulator slot
    /// instead of computing an array reduction. Slots are
    /// occurrence-ordered (left expr's sums first, then
    /// right's, then tolerance's) and the index advances per
    /// `sum(...)` encountered during lowering.
    accumulator_ctx: Option<AccumulatorEvalCtx>,
}

/// m46: per-closure accumulator substitution context for the
/// interpreter. Held in `Interpreter::accumulator_ctx` while a
/// closure assertion is being evaluated.
#[derive(Debug, Clone)]
struct AccumulatorEvalCtx {
    handle: LocusHandle,
    closure_name: String,
    next_idx: std::cell::Cell<usize>,
}

impl Interpreter {
    fn new() -> Self {
        let env = Env::new();
        builtins::install_builtins(&env);
        Interpreter {
            env,
            loci: BTreeMap::new(),
            types: BTreeMap::new(),
            self_stack: Vec::new(),
            parent_stack: Vec::new(),
            bus: BusRouter::new(),
            top_level_loci: Vec::new(),
            accumulator_ctx: None,
        }
    }

    fn load_program(&mut self, program: &Program) {
        for item in &program.items {
            self.load_top_decl(item);
        }
    }

    fn load_top_decl(&mut self, item: &TopDecl) {
        match item {
            TopDecl::Fn(f) => {
                self.env.define(
                    f.name.name.clone(),
                    Value::Fn(FnRef {
                        decl: Rc::new(f.clone()),
                        bound_self: None,
                    }),
                );
            }
            TopDecl::Locus(l) => {
                self.loci
                    .insert(l.name.name.clone(), Rc::new(l.clone()));
            }
            TopDecl::Type(t) => {
                self.types
                    .insert(t.name.name.clone(), Rc::new(t.clone()));
            }
            TopDecl::Const(_) => {
                // Milestone v0: const decls aren't evaluated in the
                // top-level pass; deferred.
            }
            TopDecl::Module(m) => {
                for item in &m.items {
                    self.load_top_decl(item);
                }
            }
            TopDecl::Perspective(_) => {
                // Perspectives don't run; only their stable_when /
                // serialize_as are observed by the bus router. v0
                // doesn't run that path.
            }
            TopDecl::Interface(_) => {
                // Interfaces are pure type-level — method
                // signatures only, no bodies. The interpreter
                // handles interface dispatch lazily at call time
                // by looking the method up on the receiver locus.
            }
            TopDecl::Topic(_) => {
                // Topic declarations carry only the payload type,
                // which the typechecker consumed already. The
                // desugaring pass (run before the interpreter
                // loads the program) rewrites every topic
                // reference into its literal-subject equivalent,
                // so the interpreter sees no topic-specific
                // semantics at this stage.
            }
        }
    }

    fn run_main(&mut self) -> Result<i32, String> {
        let main = match self.env.lookup("main") {
            Some(Value::Fn(f)) => f,
            _ => return Err("no `fn main()` defined".to_string()),
        };
        let main_result = self.call_fn(&main, &[]);
        // `return n` from main (or `main()` returning Int) maps to
        // a process exit code per spec/runtime.md. Bare return /
        // fall-through exits 0.
        let mut explicit_exit_code: Option<i32> = None;
        let main_signal = match main_result {
            Ok(crate::value::Value::Int(n)) => {
                explicit_exit_code = Some(n as i32);
                None
            }
            Ok(_) => None,
            Err(Signal::Return(crate::value::Value::Int(n))) => {
                explicit_exit_code = Some(n as i32);
                None
            }
            Err(Signal::Return(_)) => None,
            // std::process::exit(n): immediate-exit semantics. The
            // dissolve cascade below is skipped so the process
            // terminates with `n` regardless of in-flight loci —
            // mirrors libc exit(3). Return early.
            Err(Signal::Exit(n)) => {
                return Ok(n);
            }
            Err(s) => Some(s),
        };

        // Program-end dissolve cascade. Iterate in reverse-
        // instantiation order (last registered, first
        // dissolved) — depth-first cleanup of the audit graph,
        // F.4 + F.9. Any unabsorbed closure violation becomes
        // a non-zero exit; main's own error (if any) takes
        // precedence.
        let mut dissolve_signal: Option<Signal> = None;
        let to_dissolve = std::mem::take(&mut self.top_level_loci);
        for handle in to_dissolve.into_iter().rev() {
            if let Err(sig) = self.dissolve_locus(handle, None) {
                dissolve_signal.get_or_insert(sig);
            }
        }

        match main_signal.or(dissolve_signal) {
            None => Ok(explicit_exit_code.unwrap_or(0)),
            Some(Signal::Error(s)) => Err(s),
            Some(_) => Err("unexpected control-flow signal at program end".to_string()),
        }
    }

    fn call_fn(&mut self, f: &FnRef, args: &[Value]) -> Result<Value, Signal> {
        // Defaulted params can be omitted by callers — we fill
        // the tail from the param's default expr. Non-defaulted
        // params must all be provided. Defaults must form a
        // suffix; the parser permits any order but the
        // typechecker should reject a non-defaulted param after
        // a defaulted one (validated here as a runtime guard
        // until the typecheck rule lands).
        if args.len() > f.decl.params.len() {
            return Err(Signal::Error(format!(
                "fn `{}` called with {} args, expected at most {}",
                f.decl.name.name,
                args.len(),
                f.decl.params.len()
            )));
        }
        for (i, p) in f.decl.params.iter().enumerate() {
            if i >= args.len() && p.default.is_none() {
                return Err(Signal::Error(format!(
                    "fn `{}` called with {} args; param `{}` has no \
                     default and was not provided",
                    f.decl.name.name,
                    args.len(),
                    p.name.name
                )));
            }
        }
        self.env.push();
        // 3a fix: if this FnRef carries a bound receiver (i.e. it
        // came from `locus_value.method`), push it onto self_stack
        // so the body's `self.X` reads / writes resolve to the
        // captured locus. Without this the interpreter and codegen
        // paths diverge — codegen passes self_ptr as the first arg
        // at every method call, but the interpreter relied on an
        // ambient self_stack that's empty when the call originates
        // in a free fn.
        let pushed_self = if let Some(handle) = f.bound_self.clone() {
            self.self_stack.push(handle);
            true
        } else {
            false
        };
        for (i, param) in f.decl.params.iter().enumerate() {
            let v = if i < args.len() {
                args[i].clone()
            } else {
                // Default was checked above to exist.
                let default_expr = param.default.as_ref().unwrap();
                self.eval_expr(default_expr)?
            };
            self.env.define(&param.name.name, v);
        }
        let result = self.exec_block(&f.decl.body);
        self.env.pop();
        if pushed_self {
            self.self_stack.pop();
        }
        match result {
            Ok(()) => Ok(Value::Unit),
            Err(Signal::Return(v)) => Ok(v),
            // v1.x-VIOLATE (F.27): a `violate NAME;` inside the
            // method body diverged the body. The closure has
            // already been routed to the parent's on_failure via
            // deliver_violation; from the call site's perspective
            // the call returned with the locus in `draining`
            // state. Surface as Unit so caller code keeps
            // composing (the canonical pattern then uses
            // `if !self.draining { ... }` to suppress
            // downstream effects).
            Err(Signal::Violate) => Ok(Value::Unit),
            Err(other) => Err(other),
        }
    }

    fn exec_block(&mut self, block: &Block) -> Result<(), Signal> {
        // Snapshot the parent stack height so unbound children
        // started inside this block can be drained at exit.
        let parent_marker = self.parent_stack.len();
        // Anonymous-child collector for this block (when no
        // locus is currently active as parent).
        let _children_at_block_start = self.locus_anon_marker();

        self.env.push();
        let result = (|| -> Result<(), Signal> {
            for stmt in &block.stmts {
                self.exec_stmt(stmt)?;
            }
            // Stmt-context block: the trailing expression (if any) is
            // evaluated for side effects; its value is discarded.
            // Block-as-expression contexts call eval_block_as_expr
            // instead.
            if let Some(tail) = &block.tail {
                let _ = self.eval_expr(tail)?;
            }
            Ok(())
        })();
        self.env.pop();

        // Drain anonymous children started during this block.
        // (v0: nothing to do — they ran synchronously inside
        // their birth() / run() invocation.)
        let _ = parent_marker;

        result
    }

    fn locus_anon_marker(&self) -> usize {
        self.self_stack.len()
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> Result<(), Signal> {
        match stmt {
            Stmt::Let { name, value, .. } => {
                let v = self.eval_expr(value)?;
                self.env.define(&name.name, v);
                Ok(())
            }
            Stmt::LetTuple { names, value, .. } => {
                let v = self.eval_expr(value)?;
                let parts = match v {
                    Value::Tuple(parts) => parts,
                    other => {
                        return Err(Signal::Error(format!(
                            "let-tuple destructure: rhs is {}, not a tuple",
                            other.type_name()
                        )));
                    }
                };
                if parts.len() != names.len() {
                    return Err(Signal::Error(format!(
                        "let-tuple destructure: expected {} elements, got {}",
                        names.len(),
                        parts.len()
                    )));
                }
                for (n, val) in names.iter().zip(parts.into_iter()) {
                    self.env.define(&n.name, val);
                }
                Ok(())
            }
            Stmt::Assign { target, op, value, .. } => {
                let v = self.eval_expr(value)?;
                self.assign_lvalue(target, *op, v)
            }
            Stmt::Send { subject, value, .. } => {
                let payload = self.eval_expr(value)?;
                let subject_str = match self.eval_expr(subject)? {
                    Value::String(s) => s,
                    other => {
                        return Err(Signal::Error(format!(
                            "bus send subject must be String; got {}",
                            other.type_name()
                        )));
                    }
                };
                self.dispatch_bus(&subject_str, payload)
            }
            Stmt::If(if_stmt) => self.exec_if(if_stmt),
            Stmt::Match(m) => self.exec_match(m),
            Stmt::For { name, iter, body, .. } => {
                // Two iterator shapes: a Range expression
                // (`lo..hi` / `lo..=hi`) integrates as a counted
                // loop; everything else evaluates to a Value and
                // must be iterable (Array for now). Range is
                // handled here without going through eval_expr
                // because eval_expr rejects ranges in non-iterator
                // position.
                if let Expr::Range { lo, hi, inclusive, span } = iter {
                    let lo_v = self.eval_expr(lo)?;
                    let hi_v = self.eval_expr(hi)?;
                    let lo_i = match lo_v {
                        Value::Int(n) => n,
                        other => {
                            return Err(Signal::Error(format!(
                                "for: range bound must be Int, got {} \
                                 (at {:?})",
                                other.type_name(),
                                span
                            )));
                        }
                    };
                    let hi_i = match hi_v {
                        Value::Int(n) => n,
                        other => {
                            return Err(Signal::Error(format!(
                                "for: range bound must be Int, got {} \
                                 (at {:?})",
                                other.type_name(),
                                span
                            )));
                        }
                    };
                    let last = if *inclusive { hi_i } else { hi_i - 1 };
                    let mut i = lo_i;
                    while i <= last {
                        self.env.push();
                        self.env.define(&name.name, Value::Int(i));
                        let r = self.exec_block(body);
                        self.env.pop();
                        match r {
                            Ok(()) => {}
                            Err(Signal::Continue) => {
                                i += 1;
                                continue;
                            }
                            Err(Signal::Break) => break,
                            Err(other) => return Err(other),
                        }
                        i += 1;
                    }
                    return Ok(());
                }
                let iter_val = self.eval_expr(iter)?;
                let items: Vec<Value> = match iter_val {
                    Value::Array(a) => a.borrow().clone(),
                    other => {
                        return Err(Signal::Error(format!(
                            "for: cannot iterate {}",
                            other.type_name()
                        )));
                    }
                };
                for item in items {
                    self.env.push();
                    self.env.define(&name.name, item);
                    let r = self.exec_block(body);
                    self.env.pop();
                    match r {
                        Ok(()) => {}
                        Err(Signal::Continue) => continue,
                        Err(Signal::Break) => break,
                        Err(other) => return Err(other),
                    }
                }
                Ok(())
            }
            Stmt::While { cond, body, .. } => loop {
                let c = self.eval_expr(cond)?;
                if !c.truthy() {
                    return Ok(());
                }
                match self.exec_block(body) {
                    Ok(()) => {}
                    Err(Signal::Continue) => continue,
                    Err(Signal::Break) => return Ok(()),
                    Err(other) => return Err(other),
                }
            },
            Stmt::Return(expr, _) => {
                let v = match expr {
                    Some(e) => self.eval_expr(e)?,
                    None => Value::Unit,
                };
                Err(Signal::Return(v))
            }
            Stmt::Break(_) => Err(Signal::Break),
            Stmt::Continue(_) => Err(Signal::Continue),
            Stmt::Fail { value, .. } => {
                // v1.x-FORM-1 PR7: `fail <expr>;` evaluates the
                // payload and exits the fallible fn body via the
                // error path. We model this as a `Return` of a
                // `Value::FallibleErr(payload)` — the call site
                // sees the FallibleErr value and the immediate
                // caller's `or` disposition handles it.
                let payload = self.eval_expr(value)?;
                Err(Signal::Return(Value::FallibleErr(Box::new(payload))))
            }
            Stmt::Yield(_) => {
                // Interpreter is single-threaded with synchronous
                // bus dispatch — there's no pending-cell queue to
                // drain. Codegen lowers `yield` to
                // `lotus_bus_queue_drain` where the substrate
                // actually has cells; here it's a no-op (semantic
                // is preserved: the program continues; nothing
                // pending was missed because nothing was deferred).
                Ok(())
            }
            Stmt::Block(b) => self.exec_block(b),
            Stmt::Recovery { op, args, .. } => {
                let mut arg_vs = Vec::with_capacity(args.len());
                for a in args {
                    arg_vs.push(self.eval_expr(a)?);
                }
                match op {
                    RecoveryOp::Bubble => {
                        let payload = arg_vs.into_iter().next().unwrap_or(Value::Nil);
                        Err(Signal::Bubble(payload))
                    }
                    // m40: restart(c) bumps c.restart_count by 1
                    // unconditionally — the post-on_failure
                    // dispatch in `instantiate_locus` checks
                    // pre/post values + the cap (2 attempts) to
                    // decide whether to re-run birth() + birth-
                    // epoch closures. We don't gate the bump
                    // here so the cap is observable: a third
                    // restart() raises count to 3 (>2) and the
                    // re-run is skipped.
                    RecoveryOp::Restart => {
                        let target = arg_vs.into_iter().next().ok_or_else(|| {
                            Signal::Error(
                                "restart() takes one locus argument".into(),
                            )
                        })?;
                        match target {
                            Value::Locus(handle) => {
                                let cur = handle.restart_count.get();
                                handle.restart_count.set(cur + 1);
                                self.reset_accumulators_for_event(
                                    &handle, "restart",
                                );
                                Ok(())
                            }
                            other => Err(Signal::Error(format!(
                                "restart() expects a locus argument; got {}",
                                other.type_name()
                            ))),
                        }
                    }
                    // m41: quarantine(c) sets a sticky flag on the
                    // target locus. lower_locus_instantiation
                    // / instantiate_locus check the flag before
                    // entering run(); drain / dissolve still
                    // fire as cleanup. Bus-dispatch gating waits
                    // on m41b.
                    RecoveryOp::Quarantine => {
                        let target = arg_vs.into_iter().next().ok_or_else(|| {
                            Signal::Error(
                                "quarantine() takes one locus argument".into(),
                            )
                        })?;
                        match target {
                            Value::Locus(handle) => {
                                handle.quarantined.set(true);
                                self.reset_accumulators_for_event(
                                    &handle, "quarantine",
                                );
                                Ok(())
                            }
                            other => Err(Signal::Error(format!(
                                "quarantine() expects a locus argument; got {}",
                                other.type_name()
                            ))),
                        }
                    }
                    // m45: restart_in_place(c) is restart(c) +
                    // a flag that tells the rerun loop to zero
                    // user fields back to declared defaults
                    // before invoking birth(). Cap-2 budget on
                    // restart_count is shared with
                    // RecoveryOp::Restart so both variants
                    // collectively use at most 2 attempts per
                    // locus lifetime.
                    RecoveryOp::RestartInPlace => {
                        let target = arg_vs.into_iter().next().ok_or_else(|| {
                            Signal::Error(
                                "restart_in_place() takes one locus argument"
                                    .into(),
                            )
                        })?;
                        match target {
                            Value::Locus(handle) => {
                                let cur = handle.restart_count.get();
                                handle.restart_count.set(cur + 1);
                                handle.restart_in_place_pending.set(true);
                                self.reset_accumulators_for_event(
                                    &handle, "restart_in_place",
                                );
                                Ok(())
                            }
                            other => Err(Signal::Error(format!(
                                "restart_in_place() expects a locus argument; \
                                 got {}",
                                other.type_name()
                            ))),
                        }
                    }
                    // drain / dissolve / reorganize: parsed for
                    // surface completeness; full semantics land
                    // with later milestones.
                    _ => Ok(()),
                }
            }
            Stmt::Expr(e) => {
                let _ = self.eval_expr(e)?;
                Ok(())
            }
            // v1.x-VIOLATE (F.27): inline structural failure.
            // Snapshot the captures fields from self, synthesize a
            // ClosureViolation value, set the locus's draining
            // flag, and route to the parent's on_failure via the
            // existing closure-violation pathway.
            Stmt::Violate { name, payload, span: _ } => {
                let handle = self.self_stack.last().cloned().ok_or_else(|| {
                    Signal::Error(format!(
                        "`violate {}`: no enclosing locus on self_stack",
                        name.name
                    ))
                })?;
                let closure_decl = handle
                    .decl
                    .members
                    .iter()
                    .find_map(|m| match m {
                        aperio_syntax::ast::LocusMember::Closure(c)
                            if c.name.name == name.name =>
                        {
                            Some(c.clone())
                        }
                        _ => None,
                    })
                    .ok_or_else(|| {
                        Signal::Error(format!(
                            "`violate {}`: locus `{}` has no closure named `{}`",
                            name.name, handle.name, name.name,
                        ))
                    })?;
                let is_inline = closure_decl.clauses.iter().any(|c| matches!(
                    c,
                    aperio_syntax::ast::ClosureClause::Epoch(
                        aperio_syntax::ast::EpochSpec::Inline
                    )
                ));
                if !is_inline {
                    return Err(Signal::Error(format!(
                        "`violate {}`: closure `{}` is not `epoch inline`",
                        name.name, name.name,
                    )));
                }
                let captures: Vec<String> = closure_decl
                    .clauses
                    .iter()
                    .flat_map(|c| match c {
                        aperio_syntax::ast::ClosureClause::Captures(names) => {
                            names.iter().map(|n| n.name.clone()).collect::<Vec<_>>()
                        }
                        _ => Vec::new(),
                    })
                    .collect();

                let payload_val = match payload {
                    Some(p) => Some(self.eval_expr(p)?),
                    None => None,
                };

                let mut fields: BTreeMap<String, Value> = BTreeMap::new();
                fields.insert("locus".into(), Value::String(handle.name.clone()));
                fields.insert("closure".into(), Value::String(name.name.clone()));
                let state = handle.state.borrow();
                for cap in &captures {
                    let v = state
                        .get(cap)
                        .cloned()
                        .unwrap_or(Value::Nil);
                    fields.insert(cap.clone(), v);
                }
                drop(state);
                if let Some(v) = payload_val {
                    fields.insert("payload".into(), v);
                }
                let violation = Value::Struct {
                    name: "ClosureViolation".to_string(),
                    fields: Rc::new(RefCell::new(fields)),
                };

                handle.draining.set(true);
                let parent = handle.parent.borrow().clone();
                self.deliver_violation(handle.clone(), parent.as_ref(), violation)?;
                // Synthesize a divergent control-flow signal so
                // subsequent statements in the surrounding block
                // don't run. The error is structural-shaped per
                // F.9; reusing Signal::Error matches the existing
                // closure-violation cascade for uncaught cases
                // (parent absorbs → Ok(()) → caller treats
                // statement as having diverged via the natural
                // unwinding). Since deliver_violation returns
                // Ok(()) when the parent absorbed, we need an
                // explicit divergence marker — use a fresh Signal
                // tagged as ViolateDiverge so dissolve cleanup
                // distinguishes it from a hard error path.
                Err(Signal::Violate)
            }
        }
    }

    fn exec_if(&mut self, stmt: &IfStmt) -> Result<(), Signal> {
        let cond = self.eval_expr(&stmt.cond)?;
        if cond.truthy() {
            self.exec_block(&stmt.then_block)
        } else if let Some(else_branch) = &stmt.else_block {
            match else_branch.as_ref() {
                ElseBranch::Else(b) => self.exec_block(b),
                ElseBranch::ElseIf(s) => self.exec_if(s),
            }
        } else {
            Ok(())
        }
    }

    /// Block-as-expression: run stmts then return the trailing
    /// expression's value. If the block has no trailing expression,
    /// returns `Value::Unit`.
    fn eval_block_as_expr(&mut self, block: &Block) -> Result<Value, Signal> {
        self.env.push();
        let result = (|| -> Result<Value, Signal> {
            for stmt in &block.stmts {
                self.exec_stmt(stmt)?;
            }
            match &block.tail {
                Some(tail) => self.eval_expr(tail),
                None => Ok(Value::Unit),
            }
        })();
        self.env.pop();
        result
    }

    /// If-as-expression: cond gates which arm runs; the chosen arm's
    /// trailing expression value is the result.
    fn eval_if_as_expr(&mut self, stmt: &IfStmt) -> Result<Value, Signal> {
        let cond = self.eval_expr(&stmt.cond)?;
        if cond.truthy() {
            self.eval_block_as_expr(&stmt.then_block)
        } else if let Some(else_branch) = &stmt.else_block {
            match else_branch.as_ref() {
                ElseBranch::Else(b) => self.eval_block_as_expr(b),
                ElseBranch::ElseIf(s) => self.eval_if_as_expr(s),
            }
        } else {
            Ok(Value::Unit)
        }
    }

    fn exec_match(&mut self, stmt: &MatchStmt) -> Result<(), Signal> {
        let scrutinee = self.eval_expr(&stmt.scrutinee)?;
        for arm in &stmt.arms {
            let mut bindings: BTreeMap<String, Value> = BTreeMap::new();
            if !pattern_match(&arm.pattern, &scrutinee, &mut bindings) {
                continue;
            }
            self.env.push();
            for (name, value) in &bindings {
                self.env.define(name, value.clone());
            }
            // Evaluate guard, if any.
            if let Some(guard) = &arm.guard {
                let g = self.eval_expr(guard)?;
                if !g.truthy() {
                    self.env.pop();
                    continue;
                }
            }
            let result = match &arm.body {
                MatchArmBody::Expr(e) => {
                    self.eval_expr(e).map(|_| ())
                }
                MatchArmBody::Block(b) => self.exec_block(b),
            };
            self.env.pop();
            return result;
        }
        // No arm matched. v0 cut: silently no-op (Rust would
        // panic on non-exhaustive; lotus's match is statement-
        // shape so falling through is the natural choice).
        Ok(())
    }

    fn assign_lvalue(
        &mut self,
        target: &LValue,
        op: AssignOp,
        rhs: Value,
    ) -> Result<(), Signal> {
        // self.field = ...  on the innermost locus on the stack
        if target.head.name == "self" {
            let handle = match self.self_stack.last() {
                Some(h) => h.clone(),
                None => {
                    return Err(Signal::Error(
                        "`self` referenced outside a locus body".to_string(),
                    ))
                }
            };
            return self.assign_through_segments(
                Value::Locus(handle),
                &target.tail,
                op,
                rhs,
            );
        }

        // local variable
        if target.tail.is_empty() {
            let new_val = if op == AssignOp::Eq {
                rhs
            } else {
                let cur = self
                    .env
                    .lookup(&target.head.name)
                    .ok_or_else(|| Signal::Error(format!(
                        "assignment to unknown variable `{}`",
                        target.head.name
                    )))?;
                self.compound_assign(&cur, op, &rhs)?
            };
            if !self.env.assign(&target.head.name, new_val) {
                return Err(Signal::Error(format!(
                    "assignment to unbound variable `{}`",
                    target.head.name
                )));
            }
            return Ok(());
        }

        // local.field.path = ... — load then descend via segments
        let head = self
            .env
            .lookup(&target.head.name)
            .ok_or_else(|| Signal::Error(format!(
                "assignment through unknown variable `{}`",
                target.head.name
            )))?;
        self.assign_through_segments(head, &target.tail, op, rhs)
    }

    fn assign_through_segments(
        &mut self,
        head: Value,
        segs: &[LValueSeg],
        op: AssignOp,
        rhs: Value,
    ) -> Result<(), Signal> {
        // Walk down to the parent of the final seg, then mutate.
        let mut cur = head;
        if segs.is_empty() {
            return Err(Signal::Error("invalid lvalue: no segments".to_string()));
        }
        for seg in &segs[..segs.len() - 1] {
            cur = self.descend(cur, seg)?;
        }
        let last = &segs[segs.len() - 1];
        match (cur, last) {
            (Value::Struct { fields, .. }, LValueSeg::Field(name)) => {
                let mut fb = fields.borrow_mut();
                let new_val = if op == AssignOp::Eq {
                    rhs
                } else {
                    let cur = fb
                        .get(&name.name)
                        .cloned()
                        .ok_or_else(|| Signal::Error(format!(
                            "field `{}` not found",
                            name.name
                        )))?;
                    self.compound_assign(&cur, op, &rhs)?
                };
                fb.insert(name.name.clone(), new_val);
                Ok(())
            }
            (Value::Locus(handle), LValueSeg::Field(name)) => {
                let mut fb = handle.state.borrow_mut();
                let new_val = if op == AssignOp::Eq {
                    rhs
                } else {
                    let cur = fb
                        .get(&name.name)
                        .cloned()
                        .ok_or_else(|| Signal::Error(format!(
                            "self.{}: field not found",
                            name.name
                        )))?;
                    self.compound_assign(&cur, op, &rhs)?
                };
                fb.insert(name.name.clone(), new_val);
                Ok(())
            }
            (Value::Array(a), LValueSeg::Index(idx_expr)) => {
                let idx = self.eval_expr(idx_expr)?;
                let i = match idx {
                    Value::Int(n) if n >= 0 => n as usize,
                    other => {
                        return Err(Signal::Error(format!(
                            "array index must be non-negative Int; got {}",
                            other.type_name()
                        )))
                    }
                };
                let mut ab = a.borrow_mut();
                if i >= ab.len() {
                    return Err(Signal::Error(format!(
                        "array index {} out of bounds (len {})",
                        i,
                        ab.len()
                    )));
                }
                let new_val = if op == AssignOp::Eq {
                    rhs
                } else {
                    let cur = ab[i].clone();
                    self.compound_assign(&cur, op, &rhs)?
                };
                ab[i] = new_val;
                Ok(())
            }
            (Value::Cell { cell, .. }, LValueSeg::Field(name)) => {
                // F.22 v1.x-2: `cell.field = v` on a struct cell.
                // The cell's inner Value should be a Value::Struct
                // (default-constructed at acquire/alloc when the
                // slot's elem_ty is a user struct). Mutate the
                // struct's fields map directly.
                let cell_val = cell.borrow().clone();
                match cell_val {
                    Value::Struct { fields, .. } => {
                        let new_val = if op == AssignOp::Eq {
                            rhs
                        } else {
                            let cur = fields
                                .borrow()
                                .get(&name.name)
                                .cloned()
                                .ok_or_else(|| {
                                    Signal::Error(format!(
                                        "cell field `{}` not found",
                                        name.name
                                    ))
                                })?;
                            self.compound_assign(&cur, op, &rhs)?
                        };
                        fields
                            .borrow_mut()
                            .insert(name.name.clone(), new_val);
                        Ok(())
                    }
                    _ => Err(Signal::Error(format!(
                        "cell.{} write: cell does not hold a struct \
                         (primitive-cell field IO is not supported at v1)",
                        name.name
                    ))),
                }
            }
            (other, _) => Err(Signal::Error(format!(
                "cannot assign through {}",
                other.type_name()
            ))),
        }
    }

    fn descend(&mut self, cur: Value, seg: &LValueSeg) -> Result<Value, Signal> {
        match (cur, seg) {
            (Value::Struct { fields, .. }, LValueSeg::Field(name)) => fields
                .borrow()
                .get(&name.name)
                .cloned()
                .ok_or_else(|| Signal::Error(format!("field `{}` not found", name.name))),
            (Value::Cell { cell, .. }, LValueSeg::Field(name)) => {
                // F.22 v1.x-2: descend into a struct cell for nested
                // assignment paths (`outer_cell.field.subfield = v`).
                // The cell's inner Value must be a Value::Struct.
                let inner = cell.borrow().clone();
                match inner {
                    Value::Struct { fields, .. } => fields
                        .borrow()
                        .get(&name.name)
                        .cloned()
                        .ok_or_else(|| {
                            Signal::Error(format!(
                                "cell field `{}` not found",
                                name.name
                            ))
                        }),
                    _ => Err(Signal::Error(format!(
                        "cell.{} descent: cell does not hold a struct",
                        name.name
                    ))),
                }
            }
            (Value::Locus(handle), LValueSeg::Field(name)) => handle
                .state
                .borrow()
                .get(&name.name)
                .cloned()
                .ok_or_else(|| Signal::Error(format!("self.{}: field not found", name.name))),
            (Value::Array(a), LValueSeg::Index(idx_expr)) => {
                let idx = self.eval_expr(idx_expr)?;
                let i = match idx {
                    Value::Int(n) if n >= 0 => n as usize,
                    other => {
                        return Err(Signal::Error(format!(
                            "array index must be non-negative Int; got {}",
                            other.type_name()
                        )))
                    }
                };
                a.borrow()
                    .get(i)
                    .cloned()
                    .ok_or_else(|| Signal::Error(format!("array index {} out of bounds", i)))
            }
            (other, _) => Err(Signal::Error(format!(
                "cannot descend into {}",
                other.type_name()
            ))),
        }
    }

    fn compound_assign(
        &mut self,
        cur: &Value,
        op: AssignOp,
        rhs: &Value,
    ) -> Result<Value, Signal> {
        let bin = match op {
            AssignOp::Eq => return Ok(rhs.clone()),
            AssignOp::PlusEq => BinOp::Add,
            AssignOp::MinusEq => BinOp::Sub,
            AssignOp::StarEq => BinOp::Mul,
            AssignOp::SlashEq => BinOp::Div,
            AssignOp::PercentEq => BinOp::Mod,
            AssignOp::AmpEq => BinOp::BitAnd,
            AssignOp::PipeEq => BinOp::BitOr,
            AssignOp::CaretEq => BinOp::BitXor,
        };
        eval_binop(bin, cur, rhs).map_err(Signal::Error)
    }

    fn eval_expr(&mut self, expr: &Expr) -> Result<Value, Signal> {
        match expr {
            Expr::Literal(lit, _) => Ok(eval_literal(lit)),
            Expr::Ident(id) => self.env.lookup(&id.name).ok_or_else(|| {
                Signal::Error(format!("unknown identifier `{}`", id.name))
            }),
            Expr::KwSelf(_) => match self.self_stack.last() {
                Some(h) => Ok(Value::Locus(h.clone())),
                None => Err(Signal::Error(
                    "`self` referenced outside a locus body".to_string(),
                )),
            },
            Expr::Path(qname) => {
                let segs: Vec<&str> = qname.segments.iter().map(|i| i.name.as_str()).collect();
                if let Some(v) = builtins::resolve_path(&segs) {
                    return Ok(v);
                }
                if let [single] = segs.as_slice() {
                    if let Some(v) = self.env.lookup(single) {
                        return Ok(v);
                    }
                }
                // m47 + payloads: 2-segment path may be an enum
                // variant construction (`EnumName::VariantName`).
                // Path-form (no parens) is the no-payload case;
                // payload-bearing variants reach this arm only
                // if their declared field count is zero, in
                // which case the construction has no args. With
                // args, the parser produces Expr::Call with this
                // Path as the callee — handled in the Call arm.
                if let [enum_name, variant_name] = segs.as_slice() {
                    if let Some(t) = self.types.get(*enum_name) {
                        if let TypeDeclBody::Enum(variants) = &t.body {
                            if let Some(v) = variants
                                .iter()
                                .find(|v| v.name.name == *variant_name)
                            {
                                if !v.fields.is_empty() {
                                    return Err(Signal::Error(format!(
                                        "{}::{} expects {} arg(s); use `{}::{}(...)`",
                                        enum_name,
                                        variant_name,
                                        v.fields.len(),
                                        enum_name,
                                        variant_name,
                                    )));
                                }
                                return Ok(Value::EnumVariant {
                                    enum_name: (*enum_name).to_string(),
                                    variant_name: (*variant_name).to_string(),
                                    payload: Vec::new(),
                                });
                            }
                            return Err(Signal::Error(format!(
                                "enum `{}` has no variant `{}`",
                                enum_name, variant_name
                            )));
                        }
                    }
                }
                Err(Signal::Error(format!(
                    "unresolved path `{}`",
                    qname
                        .segments
                        .iter()
                        .map(|i| i.name.as_str())
                        .collect::<Vec<_>>()
                        .join("::")
                )))
            }
            Expr::Path2 { receiver, name, .. } => {
                let segs = path_segments(receiver, name);
                if let Some(segs) = segs {
                    let segs_ref: Vec<&str> = segs.iter().map(|s| s.as_str()).collect();
                    if let Some(v) = builtins::resolve_path(&segs_ref) {
                        return Ok(v);
                    }
                }
                Err(Signal::Error(
                    "::-paths to user code not yet implemented in v0 interpreter".to_string(),
                ))
            }
            Expr::Binary { op, left, right, .. } => {
                let l = self.eval_expr(left)?;
                let r = self.eval_expr(right)?;
                eval_binop(*op, &l, &r).map_err(Signal::Error)
            }
            Expr::Unary { op, operand, .. } => {
                let v = self.eval_expr(operand)?;
                eval_unop(*op, &v).map_err(Signal::Error)
            }
            Expr::Call { callee, args, .. } => {
                // m47-payloads: detect enum variant construction
                // with args — `EnumName::Variant(arg0, ...)`. The
                // callee's Path is a 2-segment qualified name; if
                // the first segment names a declared enum and the
                // second matches one of its variants, evaluate
                // each arg and assemble Value::EnumVariant with
                // the payload.
                if let Expr::Path(qn) = callee.as_ref() {
                    if qn.segments.len() == 2 {
                        let enum_name = &qn.segments[0].name;
                        let variant_name = &qn.segments[1].name;
                        if let Some(t) = self.types.get(enum_name) {
                            if let TypeDeclBody::Enum(variants) = &t.body {
                                if let Some(v) = variants
                                    .iter()
                                    .find(|v| v.name.name == *variant_name)
                                {
                                    if v.fields.len() != args.len() {
                                        return Err(Signal::Error(format!(
                                            "{}::{} expects {} arg(s), got {}",
                                            enum_name,
                                            variant_name,
                                            v.fields.len(),
                                            args.len()
                                        )));
                                    }
                                    let mut payload: Vec<Value> =
                                        Vec::with_capacity(args.len());
                                    for a in args {
                                        payload.push(self.eval_expr(a)?);
                                    }
                                    return Ok(Value::EnumVariant {
                                        enum_name: enum_name.clone(),
                                        variant_name: variant_name.clone(),
                                        payload,
                                    });
                                }
                            }
                        }
                    }
                }
                // m46-vocab: count() / mean(x) accumulator
                // builtins. Inside a closure assertion (ctx set),
                // each routes to the next accumulator slot:
                // count returns Value::Int; mean returns Float
                // computed from the slot's Tuple([sum, count]).
                // Outside a closure ctx, both fall through to
                // the generic ident-resolution path (which will
                // error since neither is a declared fn).
                if let Expr::Ident(i) = callee.as_ref() {
                    if self.accumulator_ctx.is_some() {
                        if (i.name == "count" && args.is_empty())
                            || (i.name == "mean" && args.len() == 1)
                        {
                            return self.read_next_accumulator_slot(&i.name);
                        }
                    }
                }
                // std::process::exit(n) — intercept before the
                // generic stdlib-path dispatch so we can raise
                // the dedicated Signal::Exit rather than going
                // through a Builtin (whose return signature has
                // no divergence channel). `n` evaluates as an
                // Int; truncation to i32 mirrors the codegen
                // ABI.
                if let Expr::Path(qn) = callee.as_ref() {
                    let segs: Vec<&str> = qn
                        .segments
                        .iter()
                        .map(|i| i.name.as_str())
                        .collect();
                    if segs.as_slice() == ["std", "process", "exit"] {
                        if args.len() != 1 {
                            return Err(Signal::Error(format!(
                                "std::process::exit takes 1 arg (code), got {}",
                                args.len()
                            )));
                        }
                        let v = self.eval_expr(&args[0])?;
                        let code = match v {
                            Value::Int(n) => n as i32,
                            other => {
                                return Err(Signal::Error(format!(
                                    "std::process::exit: code must be Int, got {}",
                                    other.type_name()
                                )))
                            }
                        };
                        return Err(Signal::Exit(code));
                    }
                }
                // m44: intercept `check_closures()` before
                // ident resolution — it's a substrate
                // primitive (like `quarantine` / `restart`)
                // that fires explicit-epoch closures on the
                // current self. Returns Unit so it fits in
                // both Stmt::Expr and (less idiomatic) Expr
                // position.
                if let Expr::Ident(i) = callee.as_ref() {
                    if i.name == "check_closures" {
                        if !args.is_empty() {
                            return Err(Signal::Error(format!(
                                "check_closures() takes 0 arguments, got {}",
                                args.len()
                            )));
                        }
                        let handle =
                            self.self_stack.last().cloned().ok_or_else(|| {
                                Signal::Error(
                                    "check_closures() must be called from \
                                     inside a locus body"
                                        .into(),
                                )
                            })?;
                        // m44: the locus's actual parent was
                        // captured at instantiation time on
                        // handle.parent; parent_stack during a
                        // bus handler / lifecycle body has self
                        // overlaid on top so the local
                        // parent_stack reading is wrong here.
                        let parent = handle.parent.borrow().clone();
                        self.fire_explicit_closures(handle, parent)?;
                        return Ok(Value::Unit);
                    }
                }
                // F.22: `self.<slot>.<method>(args)` routes to
                // the slot's acquire / release / alloc / free
                // rather than ordinary locus-method dispatch.
                // Detected before the receiver is evaluated
                // because slots have no value-level
                // representation outside the method-call path
                // — eval_expr on `self.<slot>` would error
                // through read_field's "no field" branch.
                if let Some(result) =
                    self.try_eval_capacity_slot_call(callee, args)?
                {
                    return Ok(result);
                }
                // v1.x-FORM-1 PR7: `<vec-locus>.<form-method>(args)`
                // intercepts before normal method dispatch when
                // the receiver is a `@form(vec)` locus and the
                // method name is one of the synthesized vec
                // methods (push / get / pop / len / is_empty).
                if let Some(result) =
                    self.try_eval_form_vec_call(callee, args)?
                {
                    return Ok(result);
                }
                // v1.x-FORM-4 PR6: parallel dispatch for
                // `<hashmap-locus>.<form-method>(args)`.
                if let Some(result) =
                    self.try_eval_form_hashmap_call(callee, args)?
                {
                    return Ok(result);
                }
                // v1.x-FORM-5: parallel dispatch for
                // `<ring-buffer-locus>.<form-method>(args)`.
                if let Some(result) =
                    self.try_eval_form_ring_buffer_call(callee, args)?
                {
                    return Ok(result);
                }
                let callee_v = self.eval_expr(callee)?;
                let mut arg_vs = Vec::with_capacity(args.len());
                for a in args {
                    arg_vs.push(self.eval_expr(a)?);
                }
                self.invoke(&callee_v, &arg_vs)
            }
            Expr::Field { receiver, name, .. } => {
                let r = self.eval_expr(receiver)?;
                self.read_field(&r, &name.name)
            }
            Expr::Index { receiver, index, .. } => {
                // m36: range-indexed receiver does string slicing
                // (interpreter parity with codegen). Today only
                // String supports slicing — Array slicing would
                // need a length-aware slice value the v0
                // representation doesn't carry. Inclusive range
                // bumps `hi` by 1 to match exclusive-clamp logic.
                if let Expr::Range { lo, hi, inclusive, .. } = index.as_ref() {
                    let r = self.eval_expr(receiver)?;
                    let s = match &r {
                        Value::String(s) => s.clone(),
                        other => {
                            return Err(Signal::Error(format!(
                                "range slicing only supported on String; \
                                 got {}",
                                other.type_name()
                            )));
                        }
                    };
                    let lo_v = self.eval_expr(lo)?;
                    let hi_v = self.eval_expr(hi)?;
                    let lo_i = match lo_v {
                        Value::Int(n) => n,
                        other => {
                            return Err(Signal::Error(format!(
                                "string slice lo bound must be Int; got {}",
                                other.type_name()
                            )));
                        }
                    };
                    let hi_i = match hi_v {
                        Value::Int(n) => n,
                        other => {
                            return Err(Signal::Error(format!(
                                "string slice hi bound must be Int; got {}",
                                other.type_name()
                            )));
                        }
                    };
                    let n = s.len() as i64;
                    let lo_c = lo_i.max(0).min(n);
                    let hi_excl = if *inclusive { hi_i + 1 } else { hi_i };
                    let hi_c = hi_excl.max(lo_c).min(n);
                    return Ok(Value::String(
                        s[lo_c as usize..hi_c as usize].to_string(),
                    ));
                }
                let r = self.eval_expr(receiver)?;
                let i = self.eval_expr(index)?;
                read_index(&r, &i).map_err(Signal::Error)
            }
            Expr::Tuple(parts, _) => {
                let mut vs = Vec::with_capacity(parts.len());
                for p in parts {
                    vs.push(self.eval_expr(p)?);
                }
                Ok(Value::Tuple(vs))
            }
            Expr::Array(parts, _) => {
                let mut vs = Vec::with_capacity(parts.len());
                for p in parts {
                    vs.push(self.eval_expr(p)?);
                }
                Ok(Value::Array(Rc::new(RefCell::new(vs))))
            }
            Expr::ArrayRepeat { val, count, .. } => {
                // `[val; N]` — interpreter evaluates val once,
                // clones the resulting Value N times into the
                // backing Vec. Cheaper than running eval N times
                // for the same RHS and matches the codegen path's
                // single-eval semantics.
                let v = self.eval_expr(val)?;
                let n = *count as usize;
                let mut vs = Vec::with_capacity(n);
                for _ in 0..n {
                    vs.push(v.clone());
                }
                Ok(Value::Array(Rc::new(RefCell::new(vs))))
            }
            Expr::Struct { path, inits, .. } => self.eval_struct_or_locus(path, inits),
            Expr::Block(b) => self.eval_block_as_expr(b),
            Expr::If(s) => self.eval_if_as_expr(s),
            Expr::Match(m) => {
                self.exec_match(m)?;
                Ok(Value::Unit)
            }
            Expr::Sum(inner, _) => {
                // m46: when an accumulator-eval ctx is active
                // (we're inside a closure assertion's
                // left/right/tolerance), `sum(...)` reads from
                // the next accumulator slot — sample-update
                // already ran. Outside a closure assertion,
                // fall through to the existing array-reduction
                // semantic (`sum([1,2,3])` → 6).
                if let Some(ctx) = self.accumulator_ctx.clone() {
                    let idx = ctx.next_idx.get();
                    ctx.next_idx.set(idx + 1);
                    let map = ctx.handle.accumulators.borrow();
                    let slots = map.get(&ctx.closure_name).ok_or_else(|| {
                        Signal::Error(format!(
                            "internal: closure `{}` accumulator slots missing \
                             at substitution time",
                            ctx.closure_name
                        ))
                    })?;
                    let v = slots.get(idx).cloned().ok_or_else(|| {
                        Signal::Error(format!(
                            "internal: closure `{}` accumulator slot {} \
                             out of range (have {})",
                            ctx.closure_name,
                            idx,
                            slots.len()
                        ))
                    })?;
                    return Ok(v);
                }
                let v = self.eval_expr(inner)?;
                reduction(&v, BinOp::Add).map_err(Signal::Error)
            }
            Expr::Prod(inner, _) => {
                let v = self.eval_expr(inner)?;
                reduction(&v, BinOp::Mul).map_err(Signal::Error)
            }
            Expr::Approx { left, right, tolerance, .. } => {
                let l = self.eval_expr(left)?;
                let r = self.eval_expr(right)?;
                let t = self.eval_expr(tolerance)?;
                approx(&l, &r, &t).map_err(Signal::Error)
            }
            Expr::Range { .. } => Err(Signal::Error(
                "range expressions are only valid as a for-loop \
                 iterator (e.g., `for i in 0..n`)"
                    .into(),
            )),
            Expr::Or { inner, disposition, .. } => {
                // v1.x-FORM-1 PR7: evaluate the inner. If it
                // produced a Value::FallibleErr, apply the
                // disposition; otherwise the inner succeeded
                // and we pass its value through unchanged.
                let inner_v = self.eval_expr(inner)?;
                match inner_v {
                    Value::FallibleErr(payload) => match disposition {
                        OrDisposition::Raise(_) => {
                            // `or raise` → closure violation
                            // routed through the existing
                            // bubble / on_failure machinery.
                            Err(Signal::Bubble(*payload))
                        }
                        OrDisposition::Discard(_) => {
                            // `or discard` — swallow the err,
                            // produce Unit. Typecheck has already
                            // ensured the call's success type is
                            // Unit so the Unit substitute is the
                            // right shape for the surrounding
                            // context.
                            let _ = payload;
                            Ok(Value::Unit)
                        }
                        OrDisposition::Substitute(rhs) => {
                            // Bind `err` to the payload in scope
                            // and evaluate the substitute body.
                            self.env.push();
                            self.env.define("err", *payload);
                            let result = self.eval_expr(rhs);
                            self.env.pop();
                            result
                        }
                    },
                    // Inner produced a regular value — pass
                    // through. (Typecheck rejects bare `or` on
                    // non-fallible expressions in PR2, so we
                    // should rarely reach this with a "useless"
                    // or — but the no-op behavior is harmless
                    // either way.)
                    other => Ok(other),
                }
            }
        }
    }

    fn read_field(&mut self, v: &Value, name: &str) -> Result<Value, Signal> {
        match v {
            // Numeric tuple field access: `t.0`, `t.1`. The
            // parser stores the digit string as the field name.
            Value::Tuple(parts) => {
                if let Ok(i) = name.parse::<usize>() {
                    if i < parts.len() {
                        return Ok(parts[i].clone());
                    }
                    return Err(Signal::Error(format!(
                        "tuple field index {} out of range (arity {})",
                        i,
                        parts.len()
                    )));
                }
                Err(Signal::Error(format!(
                    "tuple field access expects a numeric index; got `.{}`",
                    name
                )))
            }
            Value::Struct { fields, .. } => fields
                .borrow()
                .get(name)
                .cloned()
                .ok_or_else(|| Signal::Error(format!("no field `{}`", name))),
            Value::Cell { cell, .. } => {
                // F.22 v1.x-2: `cell.field` reads through to the
                // cell's inner Value::Struct. Primitive cells
                // (inner == Nil at v1) error with a clear message.
                let inner = cell.borrow().clone();
                match inner {
                    Value::Struct { fields, .. } => fields
                        .borrow()
                        .get(name)
                        .cloned()
                        .ok_or_else(|| Signal::Error(format!(
                            "no field `{}` on cell",
                            name
                        ))),
                    _ => Err(Signal::Error(format!(
                        "cell.{} read: cell does not hold a struct \
                         (primitive-cell field IO is not supported at v1)",
                        name
                    ))),
                }
            }
            Value::Locus(handle) => {
                if name == "children" {
                    let arr: Vec<Value> = handle
                        .children
                        .borrow()
                        .iter()
                        .map(|c| Value::Locus(c.clone()))
                        .collect();
                    return Ok(Value::Array(Rc::new(RefCell::new(arr))));
                }
                // v1.x-VIOLATE (F.27): synthetic Bool flag set by
                // `violate NAME;` and read as `self.draining`.
                if name == "draining" {
                    return Ok(Value::Bool(handle.draining.get()));
                }
                if name == "k_max" {
                    // F.1: k_max = B / [(1-phi)c + phi*sigma].
                    // Computed from current B/c/sigma/phi state
                    // values — the params are mutable so the
                    // bound floats. Missing params default to
                    // sensible neutral values that surface the
                    // configuration error rather than NaN.
                    let state = handle.state.borrow();
                    let b = numeric(state.get("B"))
                        .ok_or_else(|| Signal::Error(format!(
                            "locus `{}`: self.k_max requires param `B`",
                            handle.name
                        )))?;
                    let c = numeric(state.get("c"))
                        .ok_or_else(|| Signal::Error(format!(
                            "locus `{}`: self.k_max requires param `c`",
                            handle.name
                        )))?;
                    let sigma = numeric(state.get("sigma"))
                        .ok_or_else(|| Signal::Error(format!(
                            "locus `{}`: self.k_max requires param `sigma`",
                            handle.name
                        )))?;
                    let phi = numeric(state.get("phi"))
                        .ok_or_else(|| Signal::Error(format!(
                            "locus `{}`: self.k_max requires param `phi`",
                            handle.name
                        )))?;
                    let denom = (1.0 - phi) * c + phi * sigma;
                    if denom == 0.0 {
                        return Err(Signal::Error(format!(
                            "locus `{}`: k_max denominator is zero",
                            handle.name
                        )));
                    }
                    return Ok(Value::Float(b / denom));
                }
                if let Some(v) = handle.state.borrow().get(name).cloned() {
                    return Ok(v);
                }
                // Try a method (mode or fn member) named `name`.
                if let Some(method) = lookup_method(&handle.decl, name) {
                    // 3a fix: bind the receiver onto the FnRef so
                    // `call_fn` pushes it onto `self_stack` before
                    // evaluating the body. Without this, every
                    // `self.X` inside the method errors with
                    // "self referenced outside a locus body" when
                    // the call came from a free fn (no ambient
                    // self).
                    return Ok(Value::Fn(FnRef {
                        decl: Rc::new(method),
                        bound_self: Some(handle.clone()),
                    }));
                }
                Err(Signal::Error(format!(
                    "locus `{}` has no field or method `{}`",
                    handle.name, name
                )))
            }
            _ => Err(Signal::Error(format!(
                "cannot read field on {}",
                v.type_name()
            ))),
        }
    }

    fn invoke(&mut self, callee: &Value, args: &[Value]) -> Result<Value, Signal> {
        match callee {
            Value::Builtin(b) => (b.func)(args).map_err(Signal::Error),
            Value::Fn(f) => self.call_fn(f, args),
            other => Err(Signal::Error(format!(
                "cannot call {}",
                other.type_name()
            ))),
        }
    }

    /// F.22 slot dispatch. Matches the AST shape
    /// `Field { receiver: Field { receiver: KwSelf, name: slot_name },
    /// name: method_name }` and, if `slot_name` is a declared slot
    /// on the current self's locus, routes the call directly to
    /// the slot's acquire / release / alloc / free.
    ///
    /// Returns Ok(None) when the callee shape doesn't match a
    /// slot call — caller falls through to ordinary dispatch.
    /// Returns Ok(Some(value)) on success (Value::Cell from
    /// acquire/alloc; Value::Unit from release/free).
    /// v1.x-FORM-1 PR7: dispatch `<vec-locus>.push(...)` /
    /// `.get(...)` / `.pop()` / `.len()` / `.is_empty()` against
    /// the synthesized form-vec storage when the receiver is a
    /// `@form(vec)` locus.
    ///
    /// Returns `Ok(None)` when the call doesn't match this
    /// pattern — caller falls through to ordinary dispatch.
    /// Returns `Ok(Some(value))` on success (including the
    /// fallible flavor where `get` / `pop` may return
    /// `Value::FallibleErr` on out-of-bounds / empty).
    fn try_eval_form_vec_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
    ) -> Result<Option<Value>, Signal> {
        let (receiver_expr, method_name) = match callee {
            Expr::Field { receiver, name, .. } => (receiver, name.name.clone()),
            _ => return Ok(None),
        };
        if !matches!(
            method_name.as_str(),
            "push" | "get" | "set" | "pop" | "len" | "is_empty"
            | "sort" | "sort_by" | "sort_desc_by"
        ) {
            return Ok(None);
        }
        // Evaluate the receiver and check it's a @form(vec) locus.
        let recv_v = self.eval_expr(receiver_expr)?;
        let handle = match recv_v {
            Value::Locus(h) => h,
            _ => return Ok(None),
        };
        let is_form_vec = handle
            .decl
            .form
            .as_ref()
            .map(|f| f.name.name == "vec")
            .unwrap_or(false);
        if !is_form_vec {
            return Ok(None);
        }
        // Find the (single) vec-state slot. Shape verification
        // (PR3a) guarantees exactly one heap slot on a valid
        // @form(vec); we take the first vec-state slot we find
        // and surface a clear error if storage is malformed.
        let items_rc = {
            let slots = handle.slots.borrow();
            slots
                .iter()
                .find_map(|(_, st)| match st {
                    SlotState::Vec { items } => Some(items.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    Signal::Error(format!(
                        "@form(vec) locus `{}` has no vec-state slot \
                         (form-shape verification should have caught this)",
                        handle.name
                    ))
                })?
        };
        // Evaluate arguments before any mutation.
        let mut arg_vs = Vec::with_capacity(args.len());
        for a in args {
            arg_vs.push(self.eval_expr(a)?);
        }
        match method_name.as_str() {
            "push" => {
                if arg_vs.len() != 1 {
                    return Err(Signal::Error(format!(
                        "@form(vec).push expects 1 arg, got {}",
                        arg_vs.len()
                    )));
                }
                items_rc.borrow_mut().push(arg_vs.into_iter().next().unwrap());
                Ok(Some(Value::Unit))
            }
            "get" => {
                if arg_vs.len() != 1 {
                    return Err(Signal::Error(format!(
                        "@form(vec).get expects 1 arg, got {}",
                        arg_vs.len()
                    )));
                }
                let i = match &arg_vs[0] {
                    Value::Int(n) => *n,
                    other => {
                        return Err(Signal::Error(format!(
                            "@form(vec).get index must be Int; got {}",
                            other.type_name()
                        )));
                    }
                };
                let items = items_rc.borrow();
                if i < 0 || (i as usize) >= items.len() {
                    let len = items.len() as i64;
                    drop(items);
                    Ok(Some(Value::FallibleErr(Box::new(
                        index_error_value("out_of_bounds", i, len),
                    ))))
                } else {
                    Ok(Some(items[i as usize].clone()))
                }
            }
            "set" => {
                if arg_vs.len() != 2 {
                    return Err(Signal::Error(format!(
                        "@form(vec).set expects 2 args (idx, value), got {}",
                        arg_vs.len()
                    )));
                }
                let mut iter = arg_vs.into_iter();
                let idx_v = iter.next().unwrap();
                let new_v = iter.next().unwrap();
                let i = match idx_v {
                    Value::Int(n) => n,
                    other => {
                        return Err(Signal::Error(format!(
                            "@form(vec).set index must be Int; got {}",
                            other.type_name()
                        )));
                    }
                };
                let mut items = items_rc.borrow_mut();
                if i < 0 || (i as usize) >= items.len() {
                    let len = items.len() as i64;
                    drop(items);
                    Ok(Some(Value::FallibleErr(Box::new(
                        index_error_value("out_of_bounds", i, len),
                    ))))
                } else {
                    items[i as usize] = new_v;
                    Ok(Some(Value::Unit))
                }
            }
            "pop" => {
                if !arg_vs.is_empty() {
                    return Err(Signal::Error(format!(
                        "@form(vec).pop expects 0 args, got {}",
                        arg_vs.len()
                    )));
                }
                let mut items = items_rc.borrow_mut();
                match items.pop() {
                    Some(v) => Ok(Some(v)),
                    None => Ok(Some(Value::FallibleErr(Box::new(
                        index_error_value("empty", 0, 0),
                    )))),
                }
            }
            "len" => {
                if !arg_vs.is_empty() {
                    return Err(Signal::Error(format!(
                        "@form(vec).len expects 0 args, got {}",
                        arg_vs.len()
                    )));
                }
                Ok(Some(Value::Int(items_rc.borrow().len() as i64)))
            }
            "is_empty" => {
                if !arg_vs.is_empty() {
                    return Err(Signal::Error(format!(
                        "@form(vec).is_empty expects 0 args, got {}",
                        arg_vs.len()
                    )));
                }
                Ok(Some(Value::Bool(items_rc.borrow().is_empty())))
            }
            "sort" => {
                if !arg_vs.is_empty() {
                    return Err(Signal::Error(format!(
                        "@form(vec).sort expects 0 args, got {}",
                        arg_vs.len()
                    )));
                }
                let mut items = items_rc.borrow_mut();
                let mismatch = items.iter().find_map(|v| match v {
                    Value::Int(_) | Value::Float(_) | Value::String(_) => None,
                    other => Some(other.type_name().to_string()),
                });
                if let Some(t) = mismatch {
                    return Err(Signal::Error(format!(
                        "@form(vec).sort: cell type must be Int, Float, or \
                         String; got element of type {}. Use sort_by(cmp) \
                         for other cell types.",
                        t
                    )));
                }
                items.sort_by(|a, b| match (a, b) {
                    (Value::Int(x), Value::Int(y)) => x.cmp(y),
                    (Value::Float(x), Value::Float(y)) => {
                        x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
                    }
                    (Value::String(x), Value::String(y)) => x.cmp(y),
                    _ => std::cmp::Ordering::Equal,
                });
                Ok(Some(Value::Unit))
            }
            "sort_by" | "sort_desc_by" => {
                let reverse = method_name == "sort_desc_by";
                if arg_vs.len() != 1 {
                    return Err(Signal::Error(format!(
                        "@form(vec).{} expects 1 arg (cmp), got {}",
                        method_name,
                        arg_vs.len()
                    )));
                }
                let cmp_fn = match arg_vs.into_iter().next().unwrap() {
                    Value::Fn(f) => f,
                    other => {
                        return Err(Signal::Error(format!(
                            "@form(vec).{}: cmp must be a fn-pointer, got {}",
                            method_name,
                            other.type_name()
                        )));
                    }
                };
                // Take a working copy so we can release the borrow
                // while invoking user code (which may re-borrow).
                let mut working = items_rc.borrow().clone();
                let len = working.len();
                let mut err_slot: Option<Signal> = None;
                // Simple insertion sort — small N typical; keeps the
                // call surface to call_fn without re-entering the
                // borrow_mut held by Rust's sort_by closure.
                // cmp(a, b) == true means "a should come before b".
                // For sort_desc_by, we swap arg order so the same
                // user predicate yields the reverse ordering.
                for i in 1..len {
                    let mut j = i;
                    while j > 0 {
                        // Ask: should working[j] come before working[j-1]?
                        let (a, b) = if reverse {
                            (working[j - 1].clone(), working[j].clone())
                        } else {
                            (working[j].clone(), working[j - 1].clone())
                        };
                        let res = match self.call_fn(&cmp_fn, &[a, b]) {
                            Ok(v) => v,
                            Err(s) => {
                                err_slot = Some(s);
                                break;
                            }
                        };
                        let goes_before = matches!(res, Value::Bool(true));
                        if !goes_before {
                            break;
                        }
                        working.swap(j - 1, j);
                        j -= 1;
                    }
                    if err_slot.is_some() {
                        break;
                    }
                }
                if let Some(s) = err_slot {
                    return Err(s);
                }
                *items_rc.borrow_mut() = working;
                Ok(Some(Value::Unit))
            }
            _ => unreachable!("filtered at the top of the fn"),
        }
    }

    /// v1.x-FORM-4 PR6: parallel to `try_eval_form_vec_call` for
    /// `@form(hashmap)` synthesized methods. Returns `Ok(None)`
    /// when the receiver isn't a hashmap-form locus or the method
    /// isn't one of the synth names — caller falls through. The
    /// fallible methods (`get`, `remove`) return
    /// `Value::FallibleErr(Box::new(key_error_value("missing_key")))`
    /// when the key is absent; the immediate caller's `or`
    /// disposition unwraps.
    fn try_eval_form_hashmap_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
    ) -> Result<Option<Value>, Signal> {
        let (receiver_expr, method_name) = match callee {
            Expr::Field { receiver, name, .. } => (receiver, name.name.clone()),
            _ => return Ok(None),
        };
        if !matches!(
            method_name.as_str(),
            "set" | "get" | "has" | "remove" | "len" | "is_empty"
            | "key_at" | "entry_at" | "bump"
        ) {
            return Ok(None);
        }
        let recv_v = self.eval_expr(receiver_expr)?;
        let handle = match recv_v {
            Value::Locus(h) => h,
            _ => return Ok(None),
        };
        let is_form_hashmap = handle
            .decl
            .form
            .as_ref()
            .map(|f| f.name.name == "hashmap")
            .unwrap_or(false);
        if !is_form_hashmap {
            return Ok(None);
        }
        let (entries_rc, indexed_by_field) = {
            let slots = handle.slots.borrow();
            slots
                .iter()
                .find_map(|(_, st)| match st {
                    SlotState::Hashmap {
                        entries,
                        indexed_by_field,
                    } => Some((entries.clone(), indexed_by_field.clone())),
                    _ => None,
                })
                .ok_or_else(|| {
                    Signal::Error(format!(
                        "@form(hashmap) locus `{}` has no hashmap-state \
                         slot (form-shape verification should have caught \
                         this)",
                        handle.name
                    ))
                })?
        };

        let mut arg_vs = Vec::with_capacity(args.len());
        for a in args {
            arg_vs.push(self.eval_expr(a)?);
        }

        match method_name.as_str() {
            "set" => {
                if arg_vs.len() != 1 {
                    return Err(Signal::Error(format!(
                        "@form(hashmap).set expects 1 arg, got {}",
                        arg_vs.len()
                    )));
                }
                let value = arg_vs.into_iter().next().unwrap();
                let key = extract_indexed_field(&value, &indexed_by_field)
                    .ok_or_else(|| {
                        Signal::Error(format!(
                            "@form(hashmap).set: value missing indexed-by \
                             field `{}` (typecheck should have caught this)",
                            indexed_by_field
                        ))
                    })?;
                let mut entries = entries_rc.borrow_mut();
                if let Some(slot) =
                    entries.iter_mut().find(|(k, _)| values_equal(k, &key))
                {
                    slot.1 = value;
                } else {
                    entries.push((key, value));
                }
                Ok(Some(Value::Unit))
            }
            "get" => {
                if arg_vs.len() != 1 {
                    return Err(Signal::Error(format!(
                        "@form(hashmap).get expects 1 arg, got {}",
                        arg_vs.len()
                    )));
                }
                let key = arg_vs.into_iter().next().unwrap();
                let entries = entries_rc.borrow();
                match entries.iter().find(|(k, _)| values_equal(k, &key)) {
                    Some((_, v)) => Ok(Some(v.clone())),
                    None => Ok(Some(Value::FallibleErr(Box::new(
                        key_error_value("missing_key"),
                    )))),
                }
            }
            "has" => {
                if arg_vs.len() != 1 {
                    return Err(Signal::Error(format!(
                        "@form(hashmap).has expects 1 arg, got {}",
                        arg_vs.len()
                    )));
                }
                let key = arg_vs.into_iter().next().unwrap();
                let entries = entries_rc.borrow();
                let found = entries
                    .iter()
                    .any(|(k, _)| values_equal(k, &key));
                Ok(Some(Value::Bool(found)))
            }
            "remove" => {
                if arg_vs.len() != 1 {
                    return Err(Signal::Error(format!(
                        "@form(hashmap).remove expects 1 arg, got {}",
                        arg_vs.len()
                    )));
                }
                let key = arg_vs.into_iter().next().unwrap();
                let mut entries = entries_rc.borrow_mut();
                if let Some(pos) =
                    entries.iter().position(|(k, _)| values_equal(k, &key))
                {
                    entries.remove(pos);
                    Ok(Some(Value::Unit))
                } else {
                    Ok(Some(Value::FallibleErr(Box::new(
                        key_error_value("missing_key"),
                    ))))
                }
            }
            "len" => {
                if !arg_vs.is_empty() {
                    return Err(Signal::Error(format!(
                        "@form(hashmap).len expects 0 args, got {}",
                        arg_vs.len()
                    )));
                }
                Ok(Some(Value::Int(entries_rc.borrow().len() as i64)))
            }
            "is_empty" => {
                if !arg_vs.is_empty() {
                    return Err(Signal::Error(format!(
                        "@form(hashmap).is_empty expects 0 args, got {}",
                        arg_vs.len()
                    )));
                }
                Ok(Some(Value::Bool(entries_rc.borrow().is_empty())))
            }
            "key_at" => {
                if arg_vs.len() != 1 {
                    return Err(Signal::Error(format!(
                        "@form(hashmap).key_at expects 1 arg (index), got {}",
                        arg_vs.len()
                    )));
                }
                let i = match &arg_vs[0] {
                    Value::Int(n) => *n,
                    other => {
                        return Err(Signal::Error(format!(
                            "@form(hashmap).key_at index must be Int; got {}",
                            other.type_name()
                        )));
                    }
                };
                let entries = entries_rc.borrow();
                if i < 0 || (i as usize) >= entries.len() {
                    let len = entries.len() as i64;
                    drop(entries);
                    Ok(Some(Value::FallibleErr(Box::new(
                        index_error_value("out_of_bounds", i, len),
                    ))))
                } else {
                    Ok(Some(entries[i as usize].0.clone()))
                }
            }
            "entry_at" => {
                if arg_vs.len() != 1 {
                    return Err(Signal::Error(format!(
                        "@form(hashmap).entry_at expects 1 arg (index), got {}",
                        arg_vs.len()
                    )));
                }
                let i = match &arg_vs[0] {
                    Value::Int(n) => *n,
                    other => {
                        return Err(Signal::Error(format!(
                            "@form(hashmap).entry_at index must be Int; got {}",
                            other.type_name()
                        )));
                    }
                };
                let entries = entries_rc.borrow();
                if i < 0 || (i as usize) >= entries.len() {
                    let len = entries.len() as i64;
                    drop(entries);
                    Ok(Some(Value::FallibleErr(Box::new(
                        index_error_value("out_of_bounds", i, len),
                    ))))
                } else {
                    Ok(Some(entries[i as usize].1.clone()))
                }
            }
            "bump" => {
                if arg_vs.len() != 1 {
                    return Err(Signal::Error(format!(
                        "@form(hashmap).bump expects 1 arg (key), got {}",
                        arg_vs.len()
                    )));
                }
                let key = arg_vs.into_iter().next().unwrap();
                // Convention: cell must have exactly two fields —
                // the indexed-by key + one Int counter. Find the
                // counter field by scanning the existing entries'
                // value shape, OR by walking the locus decl's
                // capacity slot. We use the value shape because
                // entries_rc may already contain a sample.
                let mut entries = entries_rc.borrow_mut();
                // Find counter field by examining the cell type
                // structure. Pull from the receiver's decl.
                let counter_name = self.find_hashmap_counter_field(
                    &handle,
                    &indexed_by_field,
                ).map_err(Signal::Error)?;

                if let Some(slot) = entries
                    .iter_mut()
                    .find(|(k, _)| values_equal(k, &key))
                {
                    // Increment existing.
                    if let Value::Struct { fields, .. } = &slot.1 {
                        let mut f = fields.borrow_mut();
                        let cur = f.get(&counter_name).cloned()
                            .unwrap_or(Value::Int(0));
                        let next = match cur {
                            Value::Int(n) => Value::Int(n + 1),
                            _ => return Err(Signal::Error(format!(
                                "@form(hashmap).bump: counter field `{}` \
                                 is not Int",
                                counter_name
                            ))),
                        };
                        f.insert(counter_name.clone(), next);
                    } else {
                        return Err(Signal::Error(
                            "@form(hashmap).bump: stored value is not a \
                             struct".to_string(),
                        ));
                    }
                } else {
                    // Init at count = 1. Build a fresh struct
                    // with key and counter populated.
                    let new_value = self.build_bump_initial_struct(
                        &handle,
                        &indexed_by_field,
                        &counter_name,
                        key.clone(),
                    ).map_err(Signal::Error)?;
                    entries.push((key, new_value));
                }
                Ok(Some(Value::Unit))
            }
            _ => unreachable!("filtered at the top of the fn"),
        }
    }

    /// v1.x-FORM-5: parallel to `try_eval_form_vec_call` for
    /// `@form(ring_buffer)` synthesized methods. `push` returns
    /// Bool (false = full); `pop` is fallible with `EmptyError`.
    fn try_eval_form_ring_buffer_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
    ) -> Result<Option<Value>, Signal> {
        let (receiver_expr, method_name) = match callee {
            Expr::Field { receiver, name, .. } => (receiver, name.name.clone()),
            _ => return Ok(None),
        };
        if !matches!(
            method_name.as_str(),
            "push" | "pop" | "len" | "is_full"
        ) {
            return Ok(None);
        }
        let recv_v = self.eval_expr(receiver_expr)?;
        let handle = match recv_v {
            Value::Locus(h) => h,
            _ => return Ok(None),
        };
        let is_form_rb = handle
            .decl
            .form
            .as_ref()
            .map(|f| f.name.name == "ring_buffer")
            .unwrap_or(false);
        if !is_form_rb {
            return Ok(None);
        }
        let (cap, items_rc) = {
            let slots = handle.slots.borrow();
            slots
                .iter()
                .find_map(|(_, st)| match st {
                    SlotState::RingBuffer { cap, items } => {
                        Some((*cap, items.clone()))
                    }
                    _ => None,
                })
                .ok_or_else(|| {
                    Signal::Error(format!(
                        "@form(ring_buffer) locus `{}` has no ring_buffer-state \
                         slot (form-shape verification should have caught this)",
                        handle.name
                    ))
                })?
        };
        let mut arg_vs = Vec::with_capacity(args.len());
        for a in args {
            arg_vs.push(self.eval_expr(a)?);
        }
        match method_name.as_str() {
            "push" => {
                if arg_vs.len() != 1 {
                    return Err(Signal::Error(format!(
                        "@form(ring_buffer).push expects 1 arg, got {}",
                        arg_vs.len()
                    )));
                }
                let mut items = items_rc.borrow_mut();
                if items.len() >= cap {
                    Ok(Some(Value::Bool(false)))
                } else {
                    items.push_back(arg_vs.into_iter().next().unwrap());
                    Ok(Some(Value::Bool(true)))
                }
            }
            "pop" => {
                if !arg_vs.is_empty() {
                    return Err(Signal::Error(format!(
                        "@form(ring_buffer).pop expects 0 args, got {}",
                        arg_vs.len()
                    )));
                }
                let mut items = items_rc.borrow_mut();
                match items.pop_front() {
                    Some(v) => Ok(Some(v)),
                    None => Ok(Some(Value::FallibleErr(Box::new(
                        empty_error_value("empty"),
                    )))),
                }
            }
            "len" => {
                if !arg_vs.is_empty() {
                    return Err(Signal::Error(format!(
                        "@form(ring_buffer).len expects 0 args, got {}",
                        arg_vs.len()
                    )));
                }
                Ok(Some(Value::Int(items_rc.borrow().len() as i64)))
            }
            "is_full" => {
                if !arg_vs.is_empty() {
                    return Err(Signal::Error(format!(
                        "@form(ring_buffer).is_full expects 0 args, got {}",
                        arg_vs.len()
                    )));
                }
                Ok(Some(Value::Bool(items_rc.borrow().len() >= cap)))
            }
            _ => unreachable!("filtered at the top of the fn"),
        }
    }

    fn try_eval_capacity_slot_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
    ) -> Result<Option<Value>, Signal> {
        let (slot_name, method_name) = match callee {
            Expr::Field { receiver, name: method, .. } => {
                match receiver.as_ref() {
                    Expr::Field { receiver: inner, name: slot, .. }
                        if matches!(
                            inner.as_ref(),
                            Expr::KwSelf(_)
                        ) =>
                    {
                        (slot.name.clone(), method.name.clone())
                    }
                    _ => return Ok(None),
                }
            }
            _ => return Ok(None),
        };
        let handle = match self.self_stack.last().cloned() {
            Some(h) => h,
            None => return Ok(None),
        };
        let (slot_kind, elem_ty_name) = {
            let slots = handle.slots.borrow();
            match slots.get(&slot_name) {
                Some(SlotState::Pool { elem_ty_name, .. }) => {
                    (CapacitySlotKind::Pool, elem_ty_name.clone())
                }
                Some(SlotState::Heap { elem_ty_name, .. }) => {
                    (CapacitySlotKind::Heap, elem_ty_name.clone())
                }
                // v1.x-FORM-1 PR7: vec slots don't expose the
                // F.22 capacity-method surface (acquire/release/
                // alloc/free) — they use the form-method
                // dispatch path (push/get/pop/len/is_empty)
                // routed elsewhere.
                Some(SlotState::Vec { .. }) => return Ok(None),
                // v1.x-FORM-4 PR6: hashmap slots likewise route
                // through their own form-method path
                // (set/get/has/remove/len/is_empty).
                Some(SlotState::Hashmap { .. }) => return Ok(None),
                // v1.x-FORM-5: ring_buffer slots route through
                // their form-method path (push/pop/len/is_full).
                Some(SlotState::RingBuffer { .. }) => return Ok(None),
                None => return Ok(None),
            }
        };
        // For struct-cell slots, a freshly-created cell gets a
        // default-instantiated Value::Struct so subsequent
        // `cell.field` reads/writes work. Primitive cells (Int /
        // Float / ...) start as Value::Nil — field access on
        // them errors at use-site.
        let fresh_cell = || -> Result<Rc<RefCell<Value>>, Signal> {
            let v = match &elem_ty_name {
                Some(tn) => {
                    let t = self.types.get(tn).cloned().ok_or_else(|| {
                        Signal::Error(format!(
                            "F.22 cell elem type `{}` not declared",
                            tn
                        ))
                    })?;
                    match &t.body {
                        TypeDeclBody::Struct(fields) => {
                            let mut field_vals: BTreeMap<String, Value> =
                                BTreeMap::new();
                            for f in fields {
                                field_vals
                                    .insert(f.name.name.clone(), Value::Nil);
                            }
                            Value::Struct {
                                name: tn.clone(),
                                fields: Rc::new(RefCell::new(field_vals)),
                            }
                        }
                        _ => Value::Nil,
                    }
                }
                None => Value::Nil,
            };
            Ok(Rc::new(RefCell::new(v)))
        };
        // Slot exists; from here, mismatches are hard errors.
        match (slot_kind, method_name.as_str()) {
            (CapacitySlotKind::Pool, "acquire") => {
                if !args.is_empty() {
                    return Err(Signal::Error(format!(
                        "pool slot `{}`.acquire takes no args, got {}",
                        slot_name, args.len()
                    )));
                }
                let popped = {
                    let mut slots = handle.slots.borrow_mut();
                    match slots.get_mut(&slot_name) {
                        Some(SlotState::Pool { free, .. }) => free.pop(),
                        _ => unreachable!("checked above"),
                    }
                };
                let cell = match popped {
                    Some(c) => c,
                    None => fresh_cell()?,
                };
                return Ok(Some(Value::Cell {
                    slot_locus: handle.name.clone(),
                    slot_name,
                    cell,
                }));
            }
            (CapacitySlotKind::Pool, "release") => {
                if args.len() != 1 {
                    return Err(Signal::Error(format!(
                        "pool slot `{}`.release takes 1 cell arg, got {}",
                        slot_name, args.len()
                    )));
                }
                let cell_val = self.eval_expr(&args[0])?;
                let cell_rc = match cell_val {
                    Value::Cell { cell, slot_locus, slot_name: origin_slot } => {
                        // v1.x-5: enforce slot-of-origin at runtime.
                        if slot_locus != handle.name
                            || origin_slot != slot_name
                        {
                            return Err(Signal::Error(format!(
                                "pool slot `{}.{}`.release: cell originated \
                                 from `{}.{}` — cells can only be released \
                                 into the slot they came from",
                                handle.name,
                                slot_name,
                                slot_locus,
                                origin_slot
                            )));
                        }
                        cell
                    }
                    other => {
                        return Err(Signal::Error(format!(
                            "pool slot `{}`.release expects a cell, got {}",
                            slot_name,
                            other.type_name()
                        )));
                    }
                };
                let mut slots = handle.slots.borrow_mut();
                match slots.get_mut(&slot_name) {
                    Some(SlotState::Pool { free, .. }) => {
                        free.push(cell_rc)
                    }
                    _ => unreachable!("checked above"),
                }
                Ok(Some(Value::Unit))
            }
            (CapacitySlotKind::Heap, "alloc") => {
                if !args.is_empty() {
                    return Err(Signal::Error(format!(
                        "heap slot `{}`.alloc takes no args, got {}",
                        slot_name, args.len()
                    )));
                }
                let cell = fresh_cell()?;
                let mut slots = handle.slots.borrow_mut();
                match slots.get_mut(&slot_name) {
                    Some(SlotState::Heap { live, .. }) => {
                        live.push(cell.clone());
                    }
                    _ => unreachable!("checked above"),
                }
                Ok(Some(Value::Cell {
                    slot_locus: handle.name.clone(),
                    slot_name,
                    cell,
                }))
            }
            (CapacitySlotKind::Heap, "free") => {
                if args.len() != 1 {
                    return Err(Signal::Error(format!(
                        "heap slot `{}`.free takes 1 cell arg, got {}",
                        slot_name, args.len()
                    )));
                }
                let cell_val = self.eval_expr(&args[0])?;
                let cell_rc = match cell_val {
                    Value::Cell { cell, slot_locus, slot_name: origin_slot } => {
                        // v1.x-5: enforce slot-of-origin at runtime.
                        if slot_locus != handle.name
                            || origin_slot != slot_name
                        {
                            return Err(Signal::Error(format!(
                                "heap slot `{}.{}`.free: cell originated \
                                 from `{}.{}` — cells can only be released \
                                 into the slot they came from",
                                handle.name,
                                slot_name,
                                slot_locus,
                                origin_slot
                            )));
                        }
                        cell
                    }
                    other => {
                        return Err(Signal::Error(format!(
                            "heap slot `{}`.free expects a cell, got {}",
                            slot_name,
                            other.type_name()
                        )));
                    }
                };
                let mut slots = handle.slots.borrow_mut();
                match slots.get_mut(&slot_name) {
                    Some(SlotState::Heap { live, .. }) => {
                        live.retain(|c| !Rc::ptr_eq(c, &cell_rc));
                    }
                    _ => unreachable!("checked above"),
                }
                Ok(Some(Value::Unit))
            }
            (CapacitySlotKind::Pool, other) => Err(Signal::Error(format!(
                "pool slot `{}`: method `{}` not available — use \
                 `acquire()` / `release(c)`",
                slot_name, other
            ))),
            (CapacitySlotKind::Heap, other) => Err(Signal::Error(format!(
                "heap slot `{}`: method `{}` not available — use \
                 `alloc()` / `free(c)`",
                slot_name, other
            ))),
        }
    }

    /// Struct or locus instantiation. Disambiguated by name:
    /// if the name is in `self.loci`, it's a locus instantiation
    /// (allocate state, run birth(), then if the locus has a
    /// `run` lifecycle, run it synchronously — interpreter v0
    /// has no scheduler).
    fn eval_struct_or_locus(
        &mut self,
        path: &QualifiedName,
        inits: &[StructInit],
    ) -> Result<Value, Signal> {
        if path.segments.len() != 1 {
            return Err(Signal::Error(
                "qualified-name struct/locus literals not yet implemented".to_string(),
            ));
        }
        let name = &path.segments[0].name;
        if let Some(decl) = self.loci.get(name).cloned() {
            return self.instantiate_locus(decl, inits);
        }
        if let Some(t) = self.types.get(name).cloned() {
            return self.instantiate_struct(&t, inits);
        }
        Err(Signal::Error(format!(
            "no locus or struct named `{}`",
            name
        )))
    }

    fn instantiate_struct(
        &mut self,
        decl: &TypeDecl,
        inits: &[StructInit],
    ) -> Result<Value, Signal> {
        let fields = match &decl.body {
            TypeDeclBody::Struct(fields) => fields,
            _ => {
                return Err(Signal::Error(format!(
                    "type `{}` is not a struct; cannot use {{...}} literal",
                    decl.name.name
                )))
            }
        };
        let mut out: BTreeMap<String, Value> = BTreeMap::new();
        // First populate defaults; then overwrite with explicit inits.
        for f in fields {
            if let Some(default) = &f.default {
                let v = self.eval_expr(default)?;
                out.insert(f.name.name.clone(), v);
            }
        }
        for init in inits {
            let v = self.eval_expr(&init.value)?;
            out.insert(init.name.name.clone(), v);
        }
        // Verify all required fields are present.
        for f in fields {
            if !out.contains_key(&f.name.name) {
                return Err(Signal::Error(format!(
                    "type `{}`: missing field `{}`",
                    decl.name.name, f.name.name
                )));
            }
        }
        Ok(Value::Struct {
            name: decl.name.name.clone(),
            fields: Rc::new(RefCell::new(out)),
        })
    }

    fn instantiate_locus(
        &mut self,
        decl: Rc<LocusDecl>,
        inits: &[StructInit],
    ) -> Result<Value, Signal> {
        let mut state: BTreeMap<String, Value> = BTreeMap::new();

        // Apply param defaults first.
        for member in &decl.members {
            if let LocusMember::Params(pb) = member {
                for p in &pb.params {
                    if let ParamInit::Value(e) = &p.init {
                        let v = self.eval_expr(e)?;
                        state.insert(p.name.name.clone(), v);
                    }
                }
            }
        }
        // Then explicit overrides.
        for init in inits {
            let v = self.eval_expr(&init.value)?;
            state.insert(init.name.name.clone(), v);
        }

        // m43: count duration-epoch closures and seed the
        // last-fire timestamps to monotonic-now so the first
        // fire happens after `N` elapses since instantiation
        // (not immediately).
        let now_ns = monotonic_ns_now();
        let duration_count = decl
            .members
            .iter()
            .filter(|m| match m {
                LocusMember::Closure(c) => closure_fires_at_duration(c),
                _ => false,
            })
            .count();
        // m44: capture parent at instantiation so primitives
        // like check_closures() — called from inside the
        // locus's body where parent_stack is overlaid with
        // self — can route violations to the right handler.
        let parent_at_birth = self.parent_stack.last().cloned();

        // F.22: initialize capacity slots in declaration order.
        // Each slot starts empty (Pool: empty free-list; Heap:
        // empty live set). Cells are created on demand at
        // acquire / alloc time. The interpreter doesn't match the
        // C-side chunked-grow ramp — it just allocates one Rc per
        // cell and lets Rust drop them when the slot map is
        // dropped at dissolve.
        let mut slots: BTreeMap<String, SlotState> = BTreeMap::new();
        for member in &decl.members {
            if let LocusMember::Capacity(cb) = member {
                for slot in &cb.slots {
                    // Identify the elem_ty name for struct-cell
                    // slots so acquire/alloc can default-construct
                    // a Value::Struct for field IO. Primitives
                    // (Int / Float / ...) carry None and don't
                    // support field access at v1.
                    let elem_ty_name = match &slot.elem_ty {
                        TypeExpr::Named { path, .. }
                            if path.segments.len() == 1 =>
                        {
                            let name = &path.segments[0].name;
                            if self.types.contains_key(name) {
                                Some(name.clone())
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    // v1.x-FORM-1 PR7: when the enclosing locus
                    // is `@form(vec)`, the single heap slot
                    // becomes a contiguous vec backed by a
                    // Rust Vec<Value>. Synthesized methods
                    // (push/get/pop/len/is_empty) dispatch via
                    // the form-method path in eval.rs.
                    let is_form_vec = decl
                        .form
                        .as_ref()
                        .map(|f| f.name.name == "vec")
                        .unwrap_or(false);
                    // v1.x-FORM-4 PR6: when the locus is
                    // `@form(hashmap)`, the single pool slot
                    // becomes a keyed entry table. Synthesized
                    // methods (set/get/has/remove/len/is_empty)
                    // dispatch via `try_eval_form_hashmap_call`.
                    let is_form_hashmap = decl
                        .form
                        .as_ref()
                        .map(|f| f.name.name == "hashmap")
                        .unwrap_or(false);
                    // v1.x-FORM-5: when the locus carries
                    // `@form(ring_buffer, cap = N)`, the single
                    // pool slot becomes a bounded FIFO with the
                    // user-specified cap. Synthesized methods
                    // (push/pop/len/is_full) dispatch via
                    // `try_eval_form_ring_buffer_call`.
                    let is_form_ring_buffer = decl
                        .form
                        .as_ref()
                        .map(|f| f.name.name == "ring_buffer")
                        .unwrap_or(false);
                    let st = if is_form_vec
                        && matches!(slot.kind, CapacitySlotKind::Heap)
                    {
                        SlotState::Vec {
                            items: Rc::new(RefCell::new(Vec::new())),
                        }
                    } else if is_form_hashmap
                        && matches!(slot.kind, CapacitySlotKind::Pool)
                    {
                        let indexed_by_field = slot
                            .indexed_by
                            .as_ref()
                            .map(|i| i.name.clone())
                            .unwrap_or_default();
                        SlotState::Hashmap {
                            indexed_by_field,
                            entries: Rc::new(RefCell::new(Vec::new())),
                        }
                    } else if is_form_ring_buffer
                        && matches!(slot.kind, CapacitySlotKind::Pool)
                    {
                        let cap = decl
                            .form
                            .as_ref()
                            .and_then(|f| {
                                f.args.iter().find(|a| a.name.name == "cap")
                            })
                            .and_then(|a| match &a.value {
                                Expr::Literal(Literal::Int(n), _)
                                    if *n > 0 =>
                                {
                                    Some(*n as usize)
                                }
                                _ => None,
                            })
                            .unwrap_or(0);
                        SlotState::RingBuffer {
                            cap,
                            items: Rc::new(RefCell::new(
                                std::collections::VecDeque::with_capacity(cap),
                            )),
                        }
                    } else {
                        match slot.kind {
                            CapacitySlotKind::Pool => SlotState::Pool {
                                elem_ty_name,
                                free: Vec::new(),
                            },
                            CapacitySlotKind::Heap => SlotState::Heap {
                                elem_ty_name,
                                live: Vec::new(),
                            },
                        }
                    };
                    slots.insert(slot.name.name.clone(), st);
                }
            }
        }

        let handle = LocusHandle {
            name: decl.name.name.clone(),
            state: Rc::new(RefCell::new(state)),
            children: Rc::new(RefCell::new(Vec::new())),
            decl: decl.clone(),
            dissolved: Rc::new(std::cell::Cell::new(false)),
            restart_count: Rc::new(std::cell::Cell::new(0)),
            quarantined: Rc::new(std::cell::Cell::new(false)),
            draining: Rc::new(std::cell::Cell::new(false)),
            duration_last_fire: Rc::new(RefCell::new(vec![now_ns; duration_count])),
            parent: Rc::new(RefCell::new(parent_at_birth)),
            restart_in_place_pending: Rc::new(std::cell::Cell::new(false)),
            // m46: accumulators lazy-init at first sample. Empty
            // map starts off; `update_accumulators_for_closure`
            // creates per-closure entries on demand using the
            // sample's runtime type to choose the zero. Avoids a
            // separate type-from-AST inference pass for the
            // interpreter (codegen's pass already validates types).
            accumulators: Rc::new(RefCell::new(BTreeMap::new())),
            slots: Rc::new(RefCell::new(slots)),
        };

        // Register every bus subscription on the router. m42:
        // capture the locus's parent at subscribe time so
        // tick-epoch closures fired after a bus handler can
        // route violations to the correct on_failure handler.
        let subscribe_parent = self.parent_stack.last().cloned();
        for member in &decl.members {
            if let LocusMember::Bus(bb) = member {
                for bm in &bb.members {
                    if let BusMember::Subscribe { subject, handler, .. } = bm {
                        self.bus.subscribe(
                            subject.canonical().to_string(),
                            handle.clone(),
                            handler.name.clone(),
                            subscribe_parent.clone(),
                        );
                    }
                }
            }
        }

        // Attach to enclosing parent (if any) before birth, per
        // F.7. v0: we don't yet route through accept(); we
        // simply register on the parent.
        if let Some(parent) = self.parent_stack.last() {
            parent.children.borrow_mut().push(handle.clone());
            // Run accept() on the parent if declared.
            if let Some(accept_decl) = lookup_lifecycle(&parent.decl, LifecycleKind::Accept) {
                self.run_lifecycle(parent.clone(), &accept_decl, &[Value::Locus(handle.clone())])?;
            }
        }

        // Run birth() + birth-epoch closures, with m40 restart
        // re-runs if the parent's on_failure body called
        // restart(self). The re-run is a depth-bounded loop
        // rather than recursion: we capture restart_count before
        // each on_failure call, and after deliver_violation
        // returns we check whether the count was bumped within
        // the cap (2). Bounded loop count = bounded recursion;
        // each iteration re-runs birth() + the same closure
        // sequence on the same handle.
        let birth_closures: Vec<ClosureDecl> = handle
            .decl
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Closure(c) if closure_fires_at_birth(c) => {
                    Some(c.clone())
                }
                _ => None,
            })
            .collect();

        loop {
            // m45: if a previous iteration's on_failure body
            // called restart_in_place(self), reset user fields
            // to declared defaults before invoking birth().
            // Clears the flag so a subsequent plain restart()
            // doesn't accidentally repeat the zero pass.
            if handle.restart_in_place_pending.get() {
                handle.restart_in_place_pending.set(false);
                let mut state = handle.state.borrow_mut();
                for member in &decl.members {
                    if let LocusMember::Params(pb) = member {
                        for p in &pb.params {
                            if let ParamInit::Value(e) = &p.init {
                                drop(state);
                                let v = self.eval_expr(e)?;
                                state = handle.state.borrow_mut();
                                state.insert(p.name.name.clone(), v);
                            }
                        }
                    }
                }
            }
            // birth() runs at the top of every attempt — first
            // attempt is the natural birth; subsequent attempts
            // are restart-driven re-runs of the same lifecycle.
            if let Some(birth_decl) =
                lookup_lifecycle(&decl, LifecycleKind::Birth)
            {
                self.run_lifecycle(handle.clone(), &birth_decl, &[])?;
            }

            // Evaluate every birth-epoch closure. On failure,
            // deliver_violation invokes the parent's on_failure
            // (if any). We capture pre-/post-restart_count
            // around each delivery so we can detect a restart()
            // call in the handler body.
            let mut should_rerun = false;
            for closure in &birth_closures {
                match self.evaluate_closure(handle.clone(), closure)? {
                    ClosureOutcome::Pass => {}
                    ClosureOutcome::Violation(v) => {
                        let parent = self.parent_stack.last().cloned();
                        let pre = handle.restart_count.get();
                        self.deliver_violation(
                            handle.clone(),
                            parent.as_ref(),
                            v,
                        )?;
                        let post = handle.restart_count.get();
                        // Cap of 2 attempts per locus lifetime.
                        // Bumped + within cap → re-run birth +
                        // closures; otherwise keep iterating
                        // through remaining closures (parent
                        // already absorbed via on_failure
                        // returning, or deliver_violation would
                        // have raised Signal::Error).
                        if post > pre && post <= 2 {
                            should_rerun = true;
                            break;
                        }
                    }
                }
            }

            if !should_rerun {
                break;
            }
        }

        // If this locus has a run() lifecycle, run it
        // synchronously (no scheduler in v0). After run() returns
        // the locus is treated as drained.
        // m41: skip run() if a parent's on_failure quarantined
        // this locus during the birth-closure check above.
        if !handle.quarantined.get() {
            if let Some(run_decl) =
                lookup_lifecycle(&decl, LifecycleKind::Run)
            {
                self.run_lifecycle(handle.clone(), &run_decl, &[])?;
                // m42: tick fires after run() returns — run()
                // is a substrate cell just like a bus handler.
                // Look up the parent at instantiation time
                // (parent_stack still holds it; the lifecycle
                // method's push has been popped by run_lifecycle).
                let parent = self.parent_stack.last().cloned();
                self.fire_tick_closures(handle.clone(), parent.clone())?;
                // m43: duration shares the cell-boundary
                // cadence; fires only when declared `duration N`
                // has elapsed.
                self.fire_duration_closures(handle.clone(), parent)?;
            }
        }

        // Dissolve discipline (F.9): a locus is "ephemeral" if
        // it has no run() and no bus subscribe declarations —
        // it dissolves immediately at end of instantiation,
        // firing any epoch=dissolve closures. Long-lived loci
        // (run, or subscribed) register for program-end
        // dissolve when instantiated at top level (no parent
        // on the stack); when instantiated inside a parent
        // they're owned by that parent and dissolve via its
        // child cascade.
        if is_ephemeral_locus(&decl) {
            let parent = self.parent_stack.last().cloned();
            self.dissolve_locus(handle.clone(), parent)?;
            // Ephemeral handle stays in parent.children so
            // `for child in self.children` (and other reads)
            // continue to observe its post-dissolve state. The
            // dissolved flag prevents the parent's later cascade
            // from re-firing drain/dissolve.
        } else if self.parent_stack.is_empty() {
            self.top_level_loci.push(handle.clone());
        }

        Ok(Value::Locus(handle))
    }

    fn run_lifecycle(
        &mut self,
        handle: LocusHandle,
        decl: &LifecycleDecl,
        args: &[Value],
    ) -> Result<(), Signal> {
        if args.len() != decl.params.len() {
            return Err(Signal::Error(format!(
                "lifecycle method called with {} args, expected {}",
                args.len(),
                decl.params.len()
            )));
        }
        self.self_stack.push(handle.clone());
        // Lifecycle methods aren't implicit-locus (F.6); but they
        // ARE implicit-parent for child instantiations inside.
        self.parent_stack.push(handle.clone());
        self.env.push();
        for (p, a) in decl.params.iter().zip(args.iter()) {
            self.env.define(&p.name.name, a.clone());
        }
        let result = self.exec_block(&decl.body);
        self.env.pop();
        self.parent_stack.pop();
        self.self_stack.pop();
        match result {
            // v1.x-VIOLATE (F.27): violate-divergence is a clean
            // method-body exit; the closure has already been
            // routed to parent's on_failure.
            Ok(()) | Err(Signal::Return(_)) | Err(Signal::Violate) => Ok(()),
            Err(other) => Err(other),
        }
    }

    /// Run a locus's bound `fn` member as a handler invocation.
    /// Used by the bus router when dispatching subscribed
    /// messages: the handler runs *as the locus*, so self_stack
    /// and parent_stack are pushed for the call.
    fn run_handler(
        &mut self,
        handle: LocusHandle,
        handler_name: &str,
        arg: Value,
    ) -> Result<(), Signal> {
        let fn_decl = lookup_method(&handle.decl, handler_name).ok_or_else(|| {
            Signal::Error(format!(
                "bus dispatch: locus `{}` has no handler `{}`",
                handle.name, handler_name
            ))
        })?;
        if fn_decl.params.len() != 1 {
            return Err(Signal::Error(format!(
                "bus handler `{}` must take exactly one parameter; got {}",
                handler_name,
                fn_decl.params.len()
            )));
        }
        self.self_stack.push(handle.clone());
        self.parent_stack.push(handle.clone());
        self.env.push();
        self.env.define(&fn_decl.params[0].name.name, arg);
        let result = self.exec_block(&fn_decl.body);
        self.env.pop();
        self.parent_stack.pop();
        self.self_stack.pop();
        match result {
            // v1.x-VIOLATE (F.27): violate-divergence is a clean
            // method-body exit; the closure has already been
            // routed to parent's on_failure.
            Ok(()) | Err(Signal::Return(_)) | Err(Signal::Violate) => Ok(()),
            Err(other) => Err(other),
        }
    }

    /// Drain a locus through the dissolve discipline (F.4 +
    /// F.9): drain children first depth-first, evaluate every
    /// closure whose epoch is dissolve, run the dissolve()
    /// lifecycle if defined. A failing closure produces a
    /// ClosureViolation that is delivered to the parent's
    /// `on_failure(child, err)` handler if one is defined.
    /// If the parent absorbs (handler returns without raising),
    /// the dissolution is treated as a collapse. If no parent
    /// or no parent-handler, the violation is reported on
    /// stderr and the dissolve completes.
    fn dissolve_locus(
        &mut self,
        handle: LocusHandle,
        parent: Option<LocusHandle>,
    ) -> Result<(), Signal> {
        // Idempotency: ephemeral loci dissolve once at end of
        // instantiation. The parent's later cascade walks the
        // same children list; without this guard each child
        // would dissolve twice. The handle keeps living as long
        // as something holds an Rc — `for child in self.children`
        // and any other reads still see its (post-dissolve) state.
        if handle.dissolved.get() {
            return Ok(());
        }
        handle.dissolved.set(true);

        // Depth-first child drain (per F.4): every child is
        // dissolved with `handle` as their parent so violations
        // route to the locally-correct on_failure.
        let children: Vec<LocusHandle> = handle.children.borrow().clone();
        for child in children {
            self.dissolve_locus(child, Some(handle.clone()))?;
        }

        // F.4: after the child cascade (which IS drain's
        // recursive structure), invoke the locus's own drain()
        // body if declared. Default is no-op. drain runs before
        // closure evaluation and dissolve() so user-level cleanup
        // can happen while the locus's state is still observable.
        if let Some(drain_decl) = lookup_lifecycle(&handle.decl, LifecycleKind::Drain) {
            self.run_lifecycle(handle.clone(), &drain_decl, &[])?;
        }

        // Evaluate every closure declared on this locus whose
        // epoch is dissolve (the default).
        let closures: Vec<ClosureDecl> = handle
            .decl
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Closure(c) if closure_fires_at_dissolve(c) => Some(c.clone()),
                _ => None,
            })
            .collect();

        let mut violations: Vec<Value> = Vec::new();
        for closure in &closures {
            match self.evaluate_closure(handle.clone(), closure)? {
                ClosureOutcome::Pass => {}
                ClosureOutcome::Violation(v) => violations.push(v),
            }
        }

        // Run dissolve() lifecycle if defined (independent of
        // closure evaluation; this is the locus's own cleanup).
        if let Some(dissolve_decl) = lookup_lifecycle(&handle.decl, LifecycleKind::Dissolve) {
            self.run_lifecycle(handle.clone(), &dissolve_decl, &[])?;
        }

        // Route violations to parent's on_failure if defined.
        for violation in violations {
            self.deliver_violation(handle.clone(), parent.as_ref(), violation)?;
        }

        Ok(())
    }

    /// Deliver a ClosureViolation to the parent's on_failure
    /// handler. Three outcomes per F.9:
    ///   - parent absorbs (handler returns) → Ok(()) collapse
    ///   - parent bubbles (`bubble(err)` raised) → Signal::Error
    ///     wrapping a formatted violation; the program exits
    ///     non-zero
    ///   - no parent or no handler → Signal::Error directly
    fn deliver_violation(
        &mut self,
        child: LocusHandle,
        parent: Option<&LocusHandle>,
        violation: Value,
    ) -> Result<(), Signal> {
        if let Some(parent) = parent {
            if let Some(failure_decl) = lookup_failure(&parent.decl) {
                if failure_decl.params.len() != 2 {
                    return Err(Signal::Error(format!(
                        "on_failure on locus `{}` must take 2 params (child, err); \
                         got {}",
                        parent.name,
                        failure_decl.params.len()
                    )));
                }
                return self.run_failure(
                    parent.clone(),
                    &failure_decl,
                    Value::Locus(child),
                    violation,
                );
            }
        }
        Err(Signal::Error(format_violation(&violation)))
    }

    fn run_failure(
        &mut self,
        handle: LocusHandle,
        decl: &FailureDecl,
        child_handle: Value,
        violation: Value,
    ) -> Result<(), Signal> {
        self.self_stack.push(handle.clone());
        self.parent_stack.push(handle);
        self.env.push();
        self.env
            .define(&decl.params[0].name.name, child_handle);
        self.env.define(&decl.params[1].name.name, violation);
        let result = self.exec_block(&decl.body);
        self.env.pop();
        self.parent_stack.pop();
        self.self_stack.pop();
        match result {
            Ok(()) | Err(Signal::Return(_)) => Ok(()),
            Err(Signal::Bubble(v)) => Err(Signal::Error(format_violation(&v))),
            Err(other) => Err(other),
        }
    }

    /// m46-vocab: read the next accumulator slot in the current
    /// `accumulator_ctx` and return its substituted value. Called
    /// from `eval_expr`'s Call arm for `count()` and `mean(x)`.
    /// `Expr::Sum` has its own arm that does the equivalent for
    /// Sum-kind slots. Advances `next_idx` so a following call in
    /// the same assertion lands on the next slot.
    ///
    /// count: slot is Value::Int → return as-is.
    /// mean: slot is Value::Tuple([sum, count]) → coerce both
    /// to f64 and divide; return Value::Float.
    fn read_next_accumulator_slot(
        &mut self,
        builtin: &str,
    ) -> Result<Value, Signal> {
        let ctx = self.accumulator_ctx.clone().ok_or_else(|| {
            Signal::Error(
                "internal: read_next_accumulator_slot called without ctx".into(),
            )
        })?;
        let idx = ctx.next_idx.get();
        ctx.next_idx.set(idx + 1);
        let map = ctx.handle.accumulators.borrow();
        let slots = map.get(&ctx.closure_name).ok_or_else(|| {
            Signal::Error(format!(
                "internal: closure `{}` accumulator slots missing at \
                 substitution time",
                ctx.closure_name
            ))
        })?;
        let slot = slots.get(idx).cloned().ok_or_else(|| {
            Signal::Error(format!(
                "internal: closure `{}` accumulator slot {} out of range \
                 (have {})",
                ctx.closure_name,
                idx,
                slots.len()
            ))
        })?;
        match builtin {
            "count" => Ok(slot),
            "mean" => match slot {
                Value::Tuple(parts) if parts.len() == 2 => {
                    let sum_f = numeric_to_f64(&parts[0]).ok_or_else(|| {
                        Signal::Error(
                            "internal: mean slot's sum is not numeric".into(),
                        )
                    })?;
                    let count = match &parts[1] {
                        Value::Int(n) => *n as f64,
                        _ => {
                            return Err(Signal::Error(
                                "internal: mean slot's count is not Int".into(),
                            ));
                        }
                    };
                    if count == 0.0 {
                        // Should be unreachable — sample-update
                        // bumps count BEFORE substitution.
                        return Err(Signal::Error(
                            "internal: mean accumulator has zero count at \
                             substitution time"
                                .into(),
                        ));
                    }
                    Ok(Value::Float(sum_f / count))
                }
                other => Err(Signal::Error(format!(
                    "internal: mean slot is not a 2-tuple, got {:?}",
                    other
                ))),
            },
            other => Err(Signal::Error(format!(
                "internal: read_next_accumulator_slot unknown builtin `{}`",
                other
            ))),
        }
    }

    /// m46 / m46-vocab: evaluate each accumulator's inner
    /// expression (if any) in `handle`'s scope and update the
    /// matching slot in `handle.accumulators[closure_name]`. Sum
    /// adds the inner sample to a running-sum Value. Count bumps
    /// a running-count Int. Mean updates a Value::Tuple([sum,
    /// count]) — sum += inner, count += 1. Lazy-initializes the
    /// slot list on first fire. Self is already on the
    /// `self_stack` when we get here (pushed by
    /// `evaluate_closure`).
    fn update_closure_accumulators(
        &mut self,
        handle: &LocusHandle,
        closure_name: &str,
        accs: &[(AccKind, Option<Expr>)],
    ) -> Result<(), Signal> {
        // Evaluate each inner expr first (no borrow on accumulators).
        // Count slots have None and don't evaluate anything.
        let mut samples: Vec<Option<Value>> = Vec::with_capacity(accs.len());
        for (_, inner_opt) in accs {
            match inner_opt {
                Some(inner) => samples.push(Some(self.eval_expr(inner)?)),
                None => samples.push(None),
            }
        }
        // Now grab the accumulators map and update.
        let mut map = handle.accumulators.borrow_mut();
        let slots = map
            .entry(closure_name.to_string())
            .or_insert_with(Vec::new);
        // Pad with kind-appropriate zeros.
        while slots.len() < accs.len() {
            let i = slots.len();
            let zero = match accs[i].0 {
                AccKind::Sum => match &samples[i] {
                    Some(s) => zero_value_of_same_type(s),
                    None => Value::Int(0),
                },
                AccKind::Count => Value::Int(0),
                AccKind::Mean => {
                    let sum_zero = match &samples[i] {
                        Some(s) => zero_value_of_same_type(s),
                        None => Value::Float(0.0),
                    };
                    Value::Tuple(vec![sum_zero, Value::Int(0)])
                }
            };
            slots.push(zero);
        }
        // Update each slot per kind.
        for (slot, ((kind, _), sample_opt)) in
            slots.iter_mut().zip(accs.iter().zip(samples.into_iter()))
        {
            match kind {
                AccKind::Sum => {
                    let sample = sample_opt.expect("sum has inner");
                    *slot = eval_binop(BinOp::Add, slot, &sample)
                        .map_err(Signal::Error)?;
                }
                AccKind::Count => {
                    *slot = eval_binop(
                        BinOp::Add, slot, &Value::Int(1),
                    )
                    .map_err(Signal::Error)?;
                }
                AccKind::Mean => {
                    let sample = sample_opt.expect("mean has inner");
                    if let Value::Tuple(parts) = slot {
                        if parts.len() != 2 {
                            return Err(Signal::Error(
                                "internal: mean accumulator slot has wrong arity"
                                    .into(),
                            ));
                        }
                        parts[0] = eval_binop(BinOp::Add, &parts[0], &sample)
                            .map_err(Signal::Error)?;
                        parts[1] = eval_binop(
                            BinOp::Add, &parts[1], &Value::Int(1),
                        )
                        .map_err(Signal::Error)?;
                    } else {
                        return Err(Signal::Error(
                            "internal: mean slot is not a Tuple".into(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// m46: zero each closure's accumulators on `handle` whose
    /// `persists_through(...)` clause does not name `event`.
    /// Default = reset; opting into preservation requires the
    /// explicit clause. Called from restart / restart_in_place /
    /// quarantine recovery dispatch.
    fn reset_accumulators_for_event(
        &mut self,
        handle: &LocusHandle,
        event: &str,
    ) {
        let closures: Vec<ClosureDecl> = handle
            .decl
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Closure(c) => Some(c.clone()),
                _ => None,
            })
            .collect();
        let mut map = handle.accumulators.borrow_mut();
        for c in &closures {
            let persists = c.clauses.iter().any(|cl| {
                if let ClosureClause::PersistsThrough(events) = cl {
                    events.iter().any(|e| e.name == event)
                } else {
                    false
                }
            });
            if persists {
                continue;
            }
            if let Some(slots) = map.get_mut(&c.name.name) {
                for slot in slots.iter_mut() {
                    *slot = zero_value_of_same_type(slot);
                }
            }
        }
    }

    /// Evaluate one closure assertion. Returns
    /// [`ClosureOutcome::Pass`] when the band holds, or
    /// [`ClosureOutcome::Violation`] carrying a structured
    /// `ClosureViolation` value the runtime can route to a
    /// parent's on_failure handler.
    fn evaluate_closure(
        &mut self,
        handle: LocusHandle,
        closure: &ClosureDecl,
    ) -> Result<ClosureOutcome, Signal> {
        // v1.x-VIOLATE (F.27): assertion-less inline closures
        // don't auto-evaluate. Callers (fire_tick_closures,
        // dissolve, etc.) filter them out by epoch, so reaching
        // here without an assertion is a contract bug.
        let Some(assertion) = closure.assertion.as_ref() else {
            return Err(Signal::Error(format!(
                "evaluate_closure called on assertion-less closure `{}`",
                closure.name.name,
            )));
        };
        self.self_stack.push(handle.clone());
        self.env.push();

        // m46 / m46-vocab: sample-update each accumulator slot
        // for this closure BEFORE evaluating the assertion. Slot
        // list comes from AST shape — `sum(x)` / `count()` /
        // `mean(x)` in left/right/tolerance, in occurrence order.
        // The assertion's substitutions then read the post-update
        // values.
        let accs = collect_accumulators_in_assertion(assertion);
        if !accs.is_empty() {
            self.update_closure_accumulators(
                &handle,
                &closure.name.name,
                &accs,
            )?;
            self.accumulator_ctx = Some(AccumulatorEvalCtx {
                handle: handle.clone(),
                closure_name: closure.name.name.clone(),
                next_idx: std::cell::Cell::new(0),
            });
        }

        let result: Result<(Value, Value, Value), Signal> = (|| {
            let lt = self.eval_expr(&assertion.left)?;
            let rt = self.eval_expr(&assertion.right)?;
            let tol = self.eval_expr(&assertion.tolerance)?;
            Ok((lt, rt, tol))
        })();

        self.accumulator_ctx = None;
        self.env.pop();
        self.self_stack.pop();

        let (lt, rt, tol) = result?;
        let passes = approx_pass(&lt, &rt, &tol).map_err(Signal::Error)?;
        if passes {
            return Ok(ClosureOutcome::Pass);
        }

        let mut fields: BTreeMap<String, Value> = BTreeMap::new();
        fields.insert("locus".into(), Value::String(handle.name.clone()));
        fields.insert("closure".into(), Value::String(closure.name.name.clone()));
        fields.insert("left".into(), lt.clone());
        fields.insert("right".into(), rt.clone());
        fields.insert("tolerance".into(), tol);
        fields.insert("diff".into(), diff_value(&lt, &rt));
        let violation = Value::Struct {
            name: "ClosureViolation".to_string(),
            fields: Rc::new(RefCell::new(fields)),
        };
        Ok(ClosureOutcome::Violation(violation))
    }

    /// m42: evaluate every tick-epoch closure on `handle` and
    /// route violations through the given parent's
    /// `on_failure` (if any). Called after each bus handler
    /// invocation on this locus AND after run() returns.
    /// Skips quarantined loci — once stop-trying is set,
    /// tick-epoch checks are pointless (the locus's state is
    /// frozen for substrate-purposes anyway).
    fn fire_tick_closures(
        &mut self,
        handle: LocusHandle,
        parent: Option<LocusHandle>,
    ) -> Result<(), Signal> {
        if handle.quarantined.get() {
            return Ok(());
        }
        let tick_closures: Vec<ClosureDecl> = handle
            .decl
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Closure(c) if closure_fires_at_tick(c) => {
                    Some(c.clone())
                }
                _ => None,
            })
            .collect();
        for closure in &tick_closures {
            match self.evaluate_closure(handle.clone(), closure)? {
                ClosureOutcome::Pass => {}
                ClosureOutcome::Violation(v) => {
                    self.deliver_violation(
                        handle.clone(),
                        parent.as_ref(),
                        v,
                    )?;
                    // If on_failure called quarantine(self), the
                    // remaining tick closures don't need to fire
                    // this round — quarantined loci are silenced
                    // until process exit.
                    if handle.quarantined.get() {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }

    /// m44: evaluate every explicit-epoch closure on `handle`
    /// and route violations through the parent's `on_failure`.
    /// Called by the `check_closures();` builtin from inside a
    /// locus body — never automatically. Skipped when the
    /// locus is quarantined.
    fn fire_explicit_closures(
        &mut self,
        handle: LocusHandle,
        parent: Option<LocusHandle>,
    ) -> Result<(), Signal> {
        if handle.quarantined.get() {
            return Ok(());
        }
        let explicit_closures: Vec<ClosureDecl> = handle
            .decl
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Closure(c) if closure_fires_at_explicit(c) => {
                    Some(c.clone())
                }
                _ => None,
            })
            .collect();
        for closure in &explicit_closures {
            match self.evaluate_closure(handle.clone(), closure)? {
                ClosureOutcome::Pass => {}
                ClosureOutcome::Violation(v) => {
                    self.deliver_violation(
                        handle.clone(),
                        parent.as_ref(),
                        v,
                    )?;
                    if handle.quarantined.get() {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }

    /// m43: evaluate every duration-epoch closure on `handle`
    /// gated on monotonic-elapsed-since-last-fire >= the
    /// closure's declared duration expression. Called at the
    /// same sites as fire_tick_closures (after each bus
    /// handler, after run() returns) so duration shares the
    /// cell-boundary cadence — a duration closure fires at
    /// most once per cell, but only if N has elapsed.
    /// On fire, last_fire is updated to monotonic-now BEFORE
    /// the assertion runs so a violation routed to on_failure
    /// (which can take arbitrary time) doesn't reset the
    /// interval clock.
    fn fire_duration_closures(
        &mut self,
        handle: LocusHandle,
        parent: Option<LocusHandle>,
    ) -> Result<(), Signal> {
        if handle.quarantined.get() {
            return Ok(());
        }
        let duration_closures: Vec<(usize, ClosureDecl)> = handle
            .decl
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Closure(c) if closure_fires_at_duration(c) => {
                    Some(c.clone())
                }
                _ => None,
            })
            .enumerate()
            .collect();
        for (idx, closure) in &duration_closures {
            let duration_expr = duration_expr_for(closure)
                .expect("duration_closures filter ⇒ duration epoch");
            // Evaluate the duration expression in the locus's
            // own self_stack (so `self.poll_interval` etc work).
            self.self_stack.push(handle.clone());
            let dur_result = self.eval_expr(duration_expr);
            self.self_stack.pop();
            let duration_ns = match dur_result? {
                Value::Duration(ns) => ns,
                Value::Int(n) => n,
                other => {
                    return Err(Signal::Error(format!(
                        "duration epoch on `{}.{}`: duration expression \
                         must evaluate to Duration or Int (ns); got {}",
                        handle.name,
                        closure.name.name,
                        other.type_name()
                    )));
                }
            };
            let now = monotonic_ns_now();
            let last = handle.duration_last_fire.borrow()[*idx];
            if now - last < duration_ns {
                continue;
            }
            handle.duration_last_fire.borrow_mut()[*idx] = now;
            match self.evaluate_closure(handle.clone(), closure)? {
                ClosureOutcome::Pass => {}
                ClosureOutcome::Violation(v) => {
                    self.deliver_violation(
                        handle.clone(),
                        parent.as_ref(),
                        v,
                    )?;
                    if handle.quarantined.get() {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }

    /// Publish a payload on a subject, then drain all pending
    /// deliveries until none remain. The drain loop catches
    /// re-entrant publishes from inside handlers — a handler
    /// that calls `<-` puts more deliveries in the queue, and
    /// we keep draining until quiescence.
    fn dispatch_bus(&mut self, subject: &str, payload: Value) -> Result<(), Signal> {
        self.bus.publish(subject, payload);
        loop {
            let batch = self.bus.drain_all();
            if batch.is_empty() {
                break;
            }
            for delivery in batch {
                // m41b: skip quarantined subscribers — the
                // recovery primitive's "stop trying" signal
                // extends to bus dispatch, so a quarantined
                // locus stops receiving messages.
                // m46-followup: skip dissolved subscribers too —
                // a locus that has already run dissolve() is
                // logically gone; firing handlers on it would
                // touch post-dissolve state. Mirrors the codegen
                // path's deregister-on-dissolve via
                // `lotus_bus_quarantine_self` from
                // emit_locus_arena_destroy.
                if delivery.subscription.locus.quarantined.get()
                    || delivery.subscription.locus.dissolved.get()
                {
                    continue;
                }
                let sub_locus = delivery.subscription.locus.clone();
                let sub_parent = delivery.subscription.parent.clone();
                self.run_handler(
                    sub_locus.clone(),
                    &delivery.subscription.handler,
                    delivery.payload,
                )?;
                // m42: tick fires after each substrate cell on
                // this locus. A bus handler IS one substrate
                // cell — fire tick-epoch closures here so any
                // invariant violated by the handler's state
                // change reaches the parent's on_failure
                // before the next cell starts.
                self.fire_tick_closures(sub_locus.clone(), sub_parent.clone())?;
                // m43: duration-epoch closures share the cell-
                // boundary cadence with tick but fire only when
                // their declared `duration N` has elapsed since
                // last fire.
                self.fire_duration_closures(sub_locus, sub_parent)?;
            }
        }
        Ok(())
    }

    /// 2026-05-16: find the counter field name for
    /// `@form(hashmap).bump`. Convention: cell type has exactly
    /// two fields — the indexed-by key + one Int field. Returns
    /// the Int field's name. Walks the locus's capacity slot to
    /// find the cell type, then `self.types` for the field list.
    fn find_hashmap_counter_field(
        &self,
        handle: &crate::value::LocusHandle,
        indexed_by: &str,
    ) -> Result<String, String> {
        use aperio_syntax::ast::{LocusMember, PrimType, TypeExpr};
        let cell_type_name = handle
            .decl
            .members
            .iter()
            .find_map(|m| match m {
                LocusMember::Capacity(cb) => cb.slots.first().and_then(|s| {
                    if let TypeExpr::Named { path, .. } = &s.elem_ty {
                        path.segments.last().map(|seg| seg.name.clone())
                    } else {
                        None
                    }
                }),
                _ => None,
            })
            .ok_or_else(|| {
                "@form(hashmap).bump: locus has no capacity-slot cell type"
                    .to_string()
            })?;
        let cell_decl = self.types.get(&cell_type_name).ok_or_else(|| {
            format!(
                "@form(hashmap).bump: cell type `{}` not registered",
                cell_type_name
            )
        })?;
        use aperio_syntax::ast::TypeDeclBody;
        let struct_fields = match &cell_decl.body {
            TypeDeclBody::Struct(fs) => fs,
            _ => {
                return Err(format!(
                    "@form(hashmap).bump: cell `{}` is not a struct",
                    cell_type_name
                ))
            }
        };
        let mut int_fields: Vec<String> = Vec::new();
        let mut extras: Vec<String> = Vec::new();
        for f in struct_fields {
            if f.name.name == indexed_by {
                continue;
            }
            match &f.ty {
                TypeExpr::Primitive(PrimType::Int, _) => {
                    int_fields.push(f.name.name.clone());
                }
                _ => extras.push(f.name.name.clone()),
            }
        }
        if int_fields.len() == 1 && extras.is_empty() {
            Ok(int_fields.into_iter().next().unwrap())
        } else {
            Err(format!(
                "@form(hashmap).bump: cell `{}` must have exactly two \
                 fields — the indexed-by key (`{}`) and one Int counter. \
                 Use the explicit has/get/set pattern for richer cells.",
                cell_type_name, indexed_by
            ))
        }
    }

    /// 2026-05-16: synthesize the initial entry value for the
    /// init branch of `@form(hashmap).bump(k)`. Sets the key
    /// field to `k` and the counter field to 1.
    fn build_bump_initial_struct(
        &self,
        handle: &crate::value::LocusHandle,
        indexed_by: &str,
        counter_name: &str,
        key: Value,
    ) -> Result<Value, String> {
        use aperio_syntax::ast::{LocusMember, TypeExpr};
        let cell_type_name = handle
            .decl
            .members
            .iter()
            .find_map(|m| match m {
                LocusMember::Capacity(cb) => cb.slots.first().and_then(|s| {
                    if let TypeExpr::Named { path, .. } = &s.elem_ty {
                        path.segments.last().map(|seg| seg.name.clone())
                    } else {
                        None
                    }
                }),
                _ => None,
            })
            .ok_or_else(|| {
                "@form(hashmap).bump: locus has no capacity-slot cell type"
                    .to_string()
            })?;
        let mut fields = std::collections::BTreeMap::new();
        fields.insert(indexed_by.to_string(), key);
        fields.insert(counter_name.to_string(), Value::Int(1));
        Ok(Value::Struct {
            name: cell_type_name,
            fields: std::rc::Rc::new(std::cell::RefCell::new(fields)),
        })
    }
}

/// Outcome of evaluating one closure assertion at its epoch.
enum ClosureOutcome {
    Pass,
    Violation(Value),
}

/// m46-vocab kind tag for accumulator slots in the interpreter.
/// Mirrors the codegen `AccumulatorKind` but with interpreter-side
/// state shape: Sum holds a single Value, Count holds Value::Int,
/// Mean holds Value::Tuple([sum_value, count_value]).
#[derive(Debug, Clone, Copy, PartialEq)]
enum AccKind {
    Sum,
    Count,
    Mean,
}

/// m46 / m46-vocab: walk a closure assertion's left + right +
/// tolerance in declaration order, collect every accumulator
/// builtin call as `(kind, optional_inner_expr)`. Three forms:
/// `Expr::Sum(inner)`, `count()`, `mean(arg)`. Mirrors codegen's
/// `collect_sum_calls` but operates on the runtime crate's view
/// of the AST (kept duplicated since the helper is small).
fn collect_accumulators_in_assertion(
    ass: &ClosureAssertion,
) -> Vec<(AccKind, Option<Expr>)> {
    let mut out = Vec::new();
    walk_collect_accumulators(&ass.left, &mut out);
    walk_collect_accumulators(&ass.right, &mut out);
    walk_collect_accumulators(&ass.tolerance, &mut out);
    out
}

fn walk_collect_accumulators(
    e: &Expr,
    out: &mut Vec<(AccKind, Option<Expr>)>,
) {
    match e {
        Expr::Sum(inner, _) => out.push((AccKind::Sum, Some((**inner).clone()))),
        Expr::Call { callee, args, .. } => {
            if let Expr::Ident(id) = callee.as_ref() {
                if id.name == "count" && args.is_empty() {
                    out.push((AccKind::Count, None));
                    return;
                }
                if id.name == "mean" && args.len() == 1 {
                    out.push((AccKind::Mean, Some(args[0].clone())));
                    return;
                }
            }
            walk_collect_accumulators(callee, out);
            for a in args {
                walk_collect_accumulators(a, out);
            }
        }
        Expr::Binary { left, right, .. } => {
            walk_collect_accumulators(left, out);
            walk_collect_accumulators(right, out);
        }
        Expr::Unary { operand, .. } => walk_collect_accumulators(operand, out),
        Expr::Field { receiver, .. } => walk_collect_accumulators(receiver, out),
        Expr::Index { receiver, index, .. } => {
            walk_collect_accumulators(receiver, out);
            walk_collect_accumulators(index, out);
        }
        _ => {}
    }
}

/// m46-vocab: coerce a numeric Value to f64. Used when computing
/// `mean = sum / count` at substitution time — the numerator can
/// be Int / Float / Decimal / Duration depending on the inner
/// expr's type.
fn numeric_to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Int(n) => Some(*n as f64),
        Value::Float(f) => Some(*f),
        Value::Duration(ns) => Some(*ns as f64),
        Value::Decimal(d) => Some(d.to_f64()),
        _ => None,
    }
}

/// m46: produce a zero value of the same numeric type as `v`.
/// Used at lazy accumulator init (first sample) and at recovery
/// reset. Decimal stores as a string, so the "zero" is the
/// canonical "0d" literal so subsequent `eval_binop(Add, ...)`
/// produces a syntactically-clean Decimal sum.
fn zero_value_of_same_type(v: &Value) -> Value {
    match v {
        Value::Int(_) => Value::Int(0),
        Value::Float(_) => Value::Float(0.0),
        Value::Decimal(_) => Value::Decimal(DecimalVal::zero()),
        Value::Duration(_) => Value::Duration(0),
        // Anything non-numeric falls through to Int(0); the
        // codegen-side type check rejects this case at struct-
        // decl time, so the interpreter only sees it for
        // closures that bypassed type validation.
        _ => Value::Int(0),
    }
}

/// Try to match a pattern against a value. On success, populate
/// `bindings` with any names the pattern binds and return true.
/// On failure, return false (and the bindings map's content is
/// not meaningful — the caller discards it).
fn pattern_match(
    pat: &Pattern,
    val: &Value,
    bindings: &mut BTreeMap<String, Value>,
) -> bool {
    match pat {
        Pattern::Wildcard(_) => true,
        Pattern::Binding(ident) => {
            bindings.insert(ident.name.clone(), val.clone());
            true
        }
        Pattern::Literal(lit, _) => literal_matches(lit, val),
        Pattern::Tuple(parts, _) => match val {
            Value::Tuple(vs) if vs.len() == parts.len() => parts
                .iter()
                .zip(vs.iter())
                .all(|(p, v)| pattern_match(p, v, bindings)),
            _ => false,
        },
        Pattern::Constructor { path, args, .. } => {
            // m47 + payloads:
            //   - 1-segment path matches struct values by name
            //     (no args supported there yet).
            //   - 2-segment path `EnumName::VariantName` matches
            //     enum variant values; if args are present they
            //     bind / wildcard the payload fields in
            //     declaration order. v0.1 sub-patterns are
            //     Wildcard / Binding only.
            let segs: Vec<&str> =
                path.segments.iter().map(|s| s.name.as_str()).collect();
            match (segs.as_slice(), val) {
                ([single], Value::Struct { name, .. }) if args.is_empty() => {
                    *single == name
                }
                (
                    [enum_name, variant_name],
                    Value::EnumVariant {
                        enum_name: en,
                        variant_name: vn,
                        payload,
                    },
                ) => {
                    if !(*enum_name == en && *variant_name == vn) {
                        return false;
                    }
                    if args.is_empty() {
                        return true;
                    }
                    if args.len() != payload.len() {
                        return false;
                    }
                    for (sub, val) in args.iter().zip(payload.iter()) {
                        match sub {
                            Pattern::Wildcard(_) => {}
                            Pattern::Binding(ident) => {
                                bindings
                                    .insert(ident.name.clone(), val.clone());
                            }
                            Pattern::Literal(lit, _) => {
                                if !literal_matches(lit, val) {
                                    return false;
                                }
                            }
                            _ => return false,
                        }
                    }
                    true
                }
                _ => false,
            }
        }
    }
}

fn literal_matches(lit: &Literal, val: &Value) -> bool {
    match (lit, val) {
        (Literal::Int(a), Value::Int(b)) => a == b,
        (Literal::Float(a), Value::Float(b)) => a == b,
        (Literal::Decimal(a), Value::Decimal(b)) => {
            DecimalVal::parse(a).map(|p| DecimalVal::eq(p, *b)).unwrap_or(false)
        }
        (Literal::Duration(a), Value::Duration(b)) => a == b,
        (Literal::String(a), Value::String(b)) => a == b,
        (Literal::Bool(a), Value::Bool(b)) => a == b,
        (Literal::Nil, Value::Nil) => true,
        _ => false,
    }
}

fn format_violation(v: &Value) -> String {
    if let Value::Struct { fields, .. } = v {
        let f = fields.borrow();
        let locus = f.get("locus").map(|v| v.display()).unwrap_or_default();
        let closure = f.get("closure").map(|v| v.display()).unwrap_or_default();
        let lt = f.get("left").map(|v| v.display()).unwrap_or_default();
        let rt = f.get("right").map(|v| v.display()).unwrap_or_default();
        let tol = f.get("tolerance").map(|v| v.display()).unwrap_or_default();
        return format!(
            "ClosureViolation: locus `{}` closure `{}` failed at dissolve: \
             {} ~~ {} within {}",
            locus, closure, lt, rt, tol
        );
    }
    format!("bubble: {}", v.display())
}

fn numeric(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Int(n) => Some(*n as f64),
        Value::Float(f) => Some(*f),
        Value::Decimal(d) => Some(d.to_f64()),
        _ => None,
    }
}

fn lookup_failure(decl: &LocusDecl) -> Option<FailureDecl> {
    decl.members.iter().find_map(|m| match m {
        LocusMember::Failure(fd) => Some(fd.clone()),
        _ => None,
    })
}

fn diff_value(l: &Value, r: &Value) -> Value {
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => Value::Int(a - b),
        (Value::Float(a), Value::Float(b)) => Value::Float(a - b),
        (Value::Decimal(a), Value::Decimal(b)) => {
            Value::Decimal(DecimalVal::sub(*a, *b))
        }
        _ => Value::Nil,
    }
}

fn is_ephemeral_locus(decl: &LocusDecl) -> bool {
    // Ephemeral: no run lifecycle, no bus subscribe declarations.
    // (Long-lived loci stay registered; v0 doesn't yet have
    // program-end dissolve so their closures never fire.)
    let has_run = decl
        .members
        .iter()
        .any(|m| matches!(m, LocusMember::Lifecycle(lc) if matches!(lc.kind, LifecycleKind::Run)));
    let has_subscribe = decl.members.iter().any(|m| match m {
        LocusMember::Bus(bb) => bb
            .members
            .iter()
            .any(|bm| matches!(bm, BusMember::Subscribe { .. })),
        _ => false,
    });
    !has_run && !has_subscribe
}

fn closure_fires_at_dissolve(c: &ClosureDecl) -> bool {
    // Closure fires at dissolve if either no epoch clause was
    // given (default per spec) or the explicit epoch is
    // EpochSpec::Dissolve.
    let mut has_epoch = false;
    for clause in &c.clauses {
        if let ClosureClause::Epoch(spec) = clause {
            has_epoch = true;
            if matches!(spec, EpochSpec::Dissolve) {
                return true;
            }
        }
    }
    !has_epoch
}

fn closure_fires_at_birth(c: &ClosureDecl) -> bool {
    // m39: closure fires at birth iff an explicit
    // EpochSpec::Birth clause was declared. Default closures
    // (no epoch clause) stay dissolve-only — m39 doesn't
    // change pre-existing behavior, only adds the birth-epoch
    // path on top.
    c.clauses.iter().any(|clause| {
        matches!(clause, ClosureClause::Epoch(EpochSpec::Birth))
    })
}

fn closure_fires_at_tick(c: &ClosureDecl) -> bool {
    // m42: closure fires at tick iff an explicit
    // EpochSpec::Tick clause was declared. Tick is the
    // "after every substrate cell" epoch — fires after
    // each bus handler invocation on the locus, and after
    // run() returns. Defaults stay dissolve-only.
    c.clauses.iter().any(|clause| {
        matches!(clause, ClosureClause::Epoch(EpochSpec::Tick))
    })
}

fn closure_fires_at_duration(c: &ClosureDecl) -> bool {
    // m43: closure fires at duration iff an explicit
    // `epoch duration <expr>;` clause was declared. The
    // expression is evaluated at fire-check time (so it
    // can reference self.X), gating the assertion on
    // monotonic-elapsed-since-last-fire >= duration.
    c.clauses.iter().any(|clause| {
        matches!(clause, ClosureClause::Epoch(EpochSpec::Duration(_)))
    })
}

fn closure_fires_at_explicit(c: &ClosureDecl) -> bool {
    // m44: closure fires at explicit iff an explicit
    // `epoch explicit;` clause was declared. Fires only
    // when the user calls `check_closures();` from inside
    // the locus's body — never automatically.
    c.clauses.iter().any(|clause| {
        matches!(clause, ClosureClause::Epoch(EpochSpec::Explicit))
    })
}

fn duration_expr_for(c: &ClosureDecl) -> Option<&Expr> {
    c.clauses.iter().find_map(|clause| match clause {
        ClosureClause::Epoch(EpochSpec::Duration(e)) => Some(e),
        _ => None,
    })
}

fn monotonic_ns_now() -> i64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    let _ = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as i64)
        .saturating_mul(1_000_000_000)
        .saturating_add(ts.tv_nsec as i64)
}

fn approx_pass(l: &Value, r: &Value, tol: &Value) -> Result<bool, String> {
    // Decimal: compare via exact i128 mantissa arithmetic.
    if let (Value::Decimal(a), Value::Decimal(b), Value::Decimal(t)) =
        (l, r, tol)
    {
        let diff = DecimalVal::sub(*a, *b);
        let abs = DecimalVal {
            mantissa: diff.mantissa.abs(),
            scale: diff.scale,
        };
        return Ok(DecimalVal::cmp(abs, *t) != std::cmp::Ordering::Greater);
    }
    // Duration: compare via i64 ns.
    if let (Value::Duration(a), Value::Duration(b), Value::Duration(t)) =
        (l, r, tol)
    {
        return Ok((*a - *b).abs() <= *t);
    }
    let (la, ra, ta) = match (l, r, tol) {
        (Value::Int(a), Value::Int(b), Value::Int(t)) => (*a as f64, *b as f64, *t as f64),
        (Value::Int(a), Value::Int(b), Value::Float(t)) => (*a as f64, *b as f64, *t),
        (Value::Float(a), Value::Float(b), Value::Int(t)) => (*a, *b, *t as f64),
        (Value::Float(a), Value::Float(b), Value::Float(t)) => (*a, *b, *t),
        _ => {
            return Err(format!(
                "closure assertion: numeric operands required; got {} ~~ {} within {}",
                l.type_name(),
                r.type_name(),
                tol.type_name()
            ))
        }
    };
    Ok((la - ra).abs() <= ta)
}

fn lookup_lifecycle(decl: &LocusDecl, kind: LifecycleKind) -> Option<LifecycleDecl> {
    decl.members.iter().find_map(|m| match m {
        LocusMember::Lifecycle(lc) if lc.kind == kind => Some(lc.clone()),
        _ => None,
    })
}

fn lookup_method(decl: &LocusDecl, name: &str) -> Option<FnDecl> {
    // Try free fn members first.
    for m in &decl.members {
        if let LocusMember::Fn(f) = m {
            if f.name.name == name {
                return Some(f.clone());
            }
        }
    }
    // Then mode declarations: bulk / harmonic / resolution can be
    // invoked as methods.
    for m in &decl.members {
        if let LocusMember::Mode(md) = m {
            let mname = match md.kind {
                ModeKind::Bulk => "bulk",
                ModeKind::Harmonic => "harmonic",
                ModeKind::Resolution => "resolution",
            };
            if mname == name {
                return Some(FnDecl {
                    name: Ident {
                        name: mname.to_string(),
                        span: md.span,
                    },
                    generics: Vec::new(),
                    params: md.params.clone(),
                    ret: md.ret.clone(),
                    fallible: None,
                    body: md.body.clone(),
                    span: md.span,
                });
            }
        }
    }
    None
}

fn path_segments(recv: &Expr, name: &Ident) -> Option<Vec<String>> {
    match recv {
        Expr::Ident(id) => Some(vec![id.name.clone(), name.name.clone()]),
        Expr::Path(qn) => {
            let mut v: Vec<String> =
                qn.segments.iter().map(|i| i.name.clone()).collect();
            v.push(name.name.clone());
            Some(v)
        }
        Expr::Path2 { receiver, name: inner, .. } => {
            let mut v = path_segments(receiver, inner)?;
            v.push(name.name.clone());
            Some(v)
        }
        _ => None,
    }
}

/// %g-equivalent Float formatter: 6 fractional digits, trailing
/// zeros + dangling `.` stripped. Mirrors codegen's
/// `printf("%g", f)` so interpreter and codegen Float output
/// agree byte-for-byte. Pre-m48 this was named `fmt_decimal`
/// because Decimal was f64-backed too; with Decimal now exact
/// the function only formats Floats.
pub(crate) fn fmt_float(f: f64) -> String {
    let s = format!("{:.6}", f);
    if s.contains('.') {
        let trimmed = s.trim_end_matches('0').trim_end_matches('.');
        trimmed.to_string()
    } else {
        s
    }
}

fn eval_literal(lit: &Literal) -> Value {
    match lit {
        Literal::Int(n) => Value::Int(*n),
        Literal::Float(f) => Value::Float(*f),
        Literal::Decimal(s) => Value::Decimal(
            DecimalVal::parse(s).unwrap_or(DecimalVal::zero()),
        ),
        Literal::String(s) => Value::String(s.clone()),
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Nil => Value::Nil,
        Literal::Duration(ns) => Value::Duration(*ns),
        Literal::Time(s) => Value::Time(s.clone()),
        Literal::Bytes(b) => Value::Bytes(b.clone()),
    }
}

fn eval_binop(op: BinOp, l: &Value, r: &Value) -> Result<Value, String> {
    use BinOp::*;
    match (op, l, r) {
        (Add, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
        (Sub, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a - b)),
        (Mul, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a * b)),
        (Div, Value::Int(_), Value::Int(0)) => Err("integer division by zero".to_string()),
        (Div, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a / b)),
        (Mod, Value::Int(_), Value::Int(0)) => Err("integer modulo by zero".to_string()),
        (Mod, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a % b)),
        (Add, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
        (Sub, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
        (Mul, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
        (Div, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
        (Add, Value::Decimal(a), Value::Decimal(b)) => {
            Ok(Value::Decimal(DecimalVal::add(*a, *b)))
        }
        (Sub, Value::Decimal(a), Value::Decimal(b)) => {
            Ok(Value::Decimal(DecimalVal::sub(*a, *b)))
        }
        (Mul, Value::Decimal(a), Value::Decimal(b)) => {
            Ok(Value::Decimal(DecimalVal::mul(*a, *b)))
        }
        (Div, Value::Decimal(a), Value::Decimal(b)) => {
            DecimalVal::div(*a, *b).map(Value::Decimal)
        }
        (Mod, Value::Decimal(a), Value::Decimal(b)) => {
            if b.mantissa == 0 {
                return Err("decimal modulo by zero".to_string());
            }
            // Align both mantissas to a common scale, then
            // integer-mod the aligned mantissas. Result keeps
            // the shared scale — same recipe as `add` / `sub`.
            let scale = a.scale.max(b.scale);
            let am = a.mantissa * 10i128.pow(scale - a.scale);
            let bm = b.mantissa * 10i128.pow(scale - b.scale);
            Ok(Value::Decimal(DecimalVal {
                mantissa: am % bm,
                scale,
            }))
        }
        (Lt | Gt | LtEq | GtEq, Value::Decimal(a), Value::Decimal(b)) => {
            let ord = DecimalVal::cmp(*a, *b);
            Ok(Value::Bool(match op {
                Lt => ord == std::cmp::Ordering::Less,
                Gt => ord == std::cmp::Ordering::Greater,
                LtEq => ord != std::cmp::Ordering::Greater,
                GtEq => ord != std::cmp::Ordering::Less,
                _ => unreachable!(),
            }))
        }
        (Add, Value::String(a), Value::String(b)) => Ok(Value::String(format!("{}{}", a, b))),
        // Duration arithmetic. Add / sub produce Duration; Lt/Gt
        // are useful for timeout-style comparisons.
        (Add, Value::Duration(a), Value::Duration(b)) => {
            Ok(Value::Duration(a + b))
        }
        (Sub, Value::Duration(a), Value::Duration(b)) => {
            Ok(Value::Duration(a - b))
        }
        (Lt, Value::Duration(a), Value::Duration(b)) => Ok(Value::Bool(a < b)),
        (Gt, Value::Duration(a), Value::Duration(b)) => Ok(Value::Bool(a > b)),
        (LtEq, Value::Duration(a), Value::Duration(b)) => Ok(Value::Bool(a <= b)),
        (GtEq, Value::Duration(a), Value::Duration(b)) => Ok(Value::Bool(a >= b)),
        (Eq, a, b) => Ok(Value::Bool(values_equal(a, b))),
        (NotEq, a, b) => Ok(Value::Bool(!values_equal(a, b))),
        (Lt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
        (Gt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a > b)),
        (LtEq, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a <= b)),
        (GtEq, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a >= b)),
        (Lt, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a < b)),
        (Gt, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a > b)),
        (LtEq, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a <= b)),
        (GtEq, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a >= b)),
        // Lexicographic on String — codegen uses strcmp; the
        // interpreter uses Rust's standard String ordering, which
        // is byte-level identical for ASCII and produces the same
        // observable order for UTF-8 (well-formed input).
        (Lt, Value::String(a), Value::String(b)) => Ok(Value::Bool(a < b)),
        (Gt, Value::String(a), Value::String(b)) => Ok(Value::Bool(a > b)),
        (LtEq, Value::String(a), Value::String(b)) => Ok(Value::Bool(a <= b)),
        (GtEq, Value::String(a), Value::String(b)) => Ok(Value::Bool(a >= b)),
        (And, a, b) => Ok(Value::Bool(a.truthy() && b.truthy())),
        (Or, a, b) => Ok(Value::Bool(a.truthy() || b.truthy())),
        (BitAnd, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a & b)),
        (BitOr, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a | b)),
        (BitXor, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a ^ b)),
        (Shl, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a << b)),
        (Shr, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a >> b)),
        _ => Err(format!(
            "binop {:?}: unsupported operand types {} and {}",
            op,
            l.type_name(),
            r.type_name()
        )),
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Float(a), Value::Float(b)) => a == b,
        (Value::Decimal(a), Value::Decimal(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Duration(a), Value::Duration(b)) => a == b,
        (Value::Time(a), Value::Time(b)) => a == b,
        (
            Value::EnumVariant {
                enum_name: ea,
                variant_name: va,
                payload: pa,
            },
            Value::EnumVariant {
                enum_name: eb,
                variant_name: vb,
                payload: pb,
            },
        ) => {
            // Tag identity first; then deep-equal payloads.
            // Codegen v0.1 only compares tags (no payload eq);
            // interpreter going further is fine — programs that
            // need parity should match-bind the payload and
            // compare fields explicitly.
            ea == eb
                && va == vb
                && pa.len() == pb.len()
                && pa.iter().zip(pb.iter()).all(|(x, y)| values_equal(x, y))
        }
        (Value::Nil, Value::Nil) => true,
        (Value::Unit, Value::Unit) => true,
        _ => false,
    }
}

fn eval_unop(op: UnaryOp, v: &Value) -> Result<Value, String> {
    match (op, v) {
        (UnaryOp::Neg, Value::Int(n)) => Ok(Value::Int(-n)),
        (UnaryOp::Neg, Value::Float(f)) => Ok(Value::Float(-f)),
        (UnaryOp::Neg, Value::Decimal(d)) => Ok(Value::Decimal(d.neg())),
        (UnaryOp::Not, v) => Ok(Value::Bool(!v.truthy())),
        (UnaryOp::BitNot, Value::Int(n)) => Ok(Value::Int(!n)),
        _ => Err(format!(
            "unop {:?}: unsupported operand type {}",
            op,
            v.type_name()
        )),
    }
}

fn read_index(receiver: &Value, index: &Value) -> Result<Value, String> {
    match (receiver, index) {
        (Value::Array(a), Value::Int(i)) if *i >= 0 => {
            let i = *i as usize;
            a.borrow()
                .get(i)
                .cloned()
                .ok_or_else(|| format!("array index {} out of bounds", i))
        }
        _ => Err(format!(
            "cannot index {} with {}",
            receiver.type_name(),
            index.type_name()
        )),
    }
}

fn reduction(v: &Value, op: BinOp) -> Result<Value, String> {
    let arr = match v {
        Value::Array(a) => a.borrow().clone(),
        other => return Err(format!("sum/prod expects an Array, got {}", other.type_name())),
    };
    let init = match op {
        BinOp::Add => Value::Int(0),
        BinOp::Mul => Value::Int(1),
        _ => unreachable!(),
    };
    let mut acc = init;
    for item in arr {
        acc = eval_binop(op, &acc, &item)?;
    }
    Ok(acc)
}

fn approx(l: &Value, r: &Value, tol: &Value) -> Result<Value, String> {
    approx_pass(l, r, tol).map(Value::Bool)
}

/// v1.x-FORM-1 PR7: construct an `IndexError` value matching the
/// stdlib type synthesized by `inject_form_stdlib_types`. Used by
/// `@form(vec).get` / `.pop` to surface a typed payload when the
/// operation falls outside its valid index / non-empty contract.
fn index_error_value(kind: &str, index: i64, len: i64) -> Value {
    let mut fields: BTreeMap<String, Value> = BTreeMap::new();
    fields.insert("kind".to_string(), Value::String(kind.to_string()));
    fields.insert("index".to_string(), Value::Int(index));
    fields.insert("len".to_string(), Value::Int(len));
    Value::Struct {
        name: "IndexError".to_string(),
        fields: Rc::new(RefCell::new(fields)),
    }
}

/// v1.x-FORM-4 PR6: construct a `KeyError` value matching the
/// stdlib type synthesized alongside `IndexError`. v1's KeyError
/// is minimal — just a `kind: String` tag. Used by
/// `@form(hashmap).get` / `.remove` to surface a typed payload
/// on missing-key access.
fn key_error_value(kind: &str) -> Value {
    let mut fields: BTreeMap<String, Value> = BTreeMap::new();
    fields.insert("kind".to_string(), Value::String(kind.to_string()));
    Value::Struct {
        name: "KeyError".to_string(),
        fields: Rc::new(RefCell::new(fields)),
    }
}

/// v1.x-FORM-5: construct an `EmptyError` value for
/// `@form(ring_buffer).pop()` on an empty buffer. Same minimal
/// `kind: String` shape as KeyError.
fn empty_error_value(kind: &str) -> Value {
    let mut fields: BTreeMap<String, Value> = BTreeMap::new();
    fields.insert("kind".to_string(), Value::String(kind.to_string()));
    Value::Struct {
        name: "EmptyError".to_string(),
        fields: Rc::new(RefCell::new(fields)),
    }
}

/// Construct an `IoError` value for the `std::io::fs::*` /
/// `std::io::tcp::*` fallible path-calls. Same three-field
/// shape (`kind`, `errno`, `path`) as the codegen-side struct
/// — agents read all three uniformly.
pub(crate) fn io_error_value(kind: &str, errno: i64, path: &str) -> Value {
    let mut fields: BTreeMap<String, Value> = BTreeMap::new();
    fields.insert("kind".to_string(), Value::String(kind.to_string()));
    fields.insert("errno".to_string(), Value::Int(errno));
    fields.insert("path".to_string(), Value::String(path.to_string()));
    Value::Struct {
        name: "IoError".to_string(),
        fields: Rc::new(RefCell::new(fields)),
    }
}

/// v1.x-FORM-4 PR6: extract the indexed-by field from a struct
/// value at `set` call time. Returns `None` when the value isn't
/// a struct or doesn't carry the named field — both of which
/// indicate a typecheck escape (PR2 verifies the field exists on
/// the cell type at locus declaration time).
fn extract_indexed_field(value: &Value, field_name: &str) -> Option<Value> {
    match value {
        Value::Struct { fields, .. } => {
            fields.borrow().get(field_name).cloned()
        }
        _ => None,
    }
}


