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

use lotus_syntax::ast::*;

use crate::builtins;
use crate::bus::BusRouter;
use crate::env::Env;
use crate::value::{FnRef, LocusHandle, Value};

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
    Error(String),
}

impl From<String> for Signal {
    fn from(s: String) -> Self {
        Signal::Error(s)
    }
}

pub fn run_program(program: &Program) -> Result<i32, String> {
    let mut interp = Interpreter::new();
    interp.load_program(program);
    interp.run_main()
}

pub fn run_bundle(programs: &[&Program]) -> Result<i32, String> {
    let mut interp = Interpreter::new();
    for p in programs {
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
        if args.len() != f.decl.params.len() {
            return Err(Signal::Error(format!(
                "fn `{}` called with {} args, expected {}",
                f.decl.name.name,
                args.len(),
                f.decl.params.len()
            )));
        }
        self.env.push();
        for (param, arg) in f.decl.params.iter().zip(args.iter()) {
            self.env.define(&param.name.name, arg.clone());
        }
        let result = self.exec_block(&f.decl.body);
        self.env.pop();
        match result {
            Ok(()) => Ok(Value::Unit),
            Err(Signal::Return(v)) => Ok(v),
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
                    // restart / restart_in_place / drain / dissolve /
                    // quarantine / reorganize: parsed for surface
                    // completeness; full semantics land with the
                    // scheduler + region allocator.
                    _ => Ok(()),
                }
            }
            Stmt::Expr(e) => {
                let _ = self.eval_expr(e)?;
                Ok(())
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
            Expr::Struct { path, inits, .. } => self.eval_struct_or_locus(path, inits),
            Expr::Block(b) => {
                self.exec_block(b)?;
                Ok(Value::Unit)
            }
            Expr::If(s) => {
                self.exec_if(s)?;
                Ok(Value::Unit)
            }
            Expr::Match(m) => {
                self.exec_match(m)?;
                Ok(Value::Unit)
            }
            Expr::Sum(inner, _) => {
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
        }
    }

    fn read_field(&mut self, v: &Value, name: &str) -> Result<Value, Signal> {
        match v {
            Value::Struct { fields, .. } => fields
                .borrow()
                .get(name)
                .cloned()
                .ok_or_else(|| Signal::Error(format!("no field `{}`", name))),
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
                    return Ok(Value::Fn(FnRef {
                        decl: Rc::new(method),
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

        let handle = LocusHandle {
            name: decl.name.name.clone(),
            state: Rc::new(RefCell::new(state)),
            children: Rc::new(RefCell::new(Vec::new())),
            decl: decl.clone(),
            dissolved: Rc::new(std::cell::Cell::new(false)),
        };

        // Register every bus subscription on the router.
        for member in &decl.members {
            if let LocusMember::Bus(bb) = member {
                for bm in &bb.members {
                    if let BusMember::Subscribe { subject, handler, .. } = bm {
                        self.bus.subscribe(
                            subject.clone(),
                            handle.clone(),
                            handler.name.clone(),
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

        // Run birth().
        if let Some(birth_decl) = lookup_lifecycle(&decl, LifecycleKind::Birth) {
            self.run_lifecycle(handle.clone(), &birth_decl, &[])?;
        }

        // If this locus has a run() lifecycle, run it
        // synchronously (no scheduler in v0). After run() returns
        // the locus is treated as drained.
        if let Some(run_decl) = lookup_lifecycle(&decl, LifecycleKind::Run) {
            self.run_lifecycle(handle.clone(), &run_decl, &[])?;
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
            Ok(()) | Err(Signal::Return(_)) => Ok(()),
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
            Ok(()) | Err(Signal::Return(_)) => Ok(()),
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
        self.self_stack.push(handle.clone());
        self.env.push();

        let result: Result<(Value, Value, Value), Signal> = (|| {
            let lt = self.eval_expr(&closure.assertion.left)?;
            let rt = self.eval_expr(&closure.assertion.right)?;
            let tol = self.eval_expr(&closure.assertion.tolerance)?;
            Ok((lt, rt, tol))
        })();

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
                self.run_handler(
                    delivery.subscription.locus,
                    &delivery.subscription.handler,
                    delivery.payload,
                )?;
            }
        }
        Ok(())
    }
}

/// Outcome of evaluating one closure assertion at its epoch.
enum ClosureOutcome {
    Pass,
    Violation(Value),
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
            // v0: empty-args constructor matches a struct
            // value by struct name (last path segment).
            // Non-empty args require enum-variant support
            // and aren't wired in v0.
            let last = match path.segments.last() {
                Some(s) => &s.name,
                None => return false,
            };
            if !args.is_empty() {
                return false;
            }
            match val {
                Value::Struct { name, .. } => name == last,
                _ => false,
            }
        }
    }
}

fn literal_matches(lit: &Literal, val: &Value) -> bool {
    match (lit, val) {
        (Literal::Int(a), Value::Int(b)) => a == b,
        (Literal::Float(a), Value::Float(b)) => a == b,
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
        Value::Decimal(s) => parse_decimal(s),
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
            let af = parse_decimal(a).unwrap_or(0.0);
            let bf = parse_decimal(b).unwrap_or(0.0);
            Value::Decimal(fmt_decimal(af - bf))
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

fn approx_pass(l: &Value, r: &Value, tol: &Value) -> Result<bool, String> {
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

fn eval_literal(lit: &Literal) -> Value {
    match lit {
        Literal::Int(n) => Value::Int(*n),
        Literal::Float(f) => Value::Float(*f),
        Literal::Decimal(s) => Value::Decimal(s.clone()),
        Literal::String(s) => Value::String(s.clone()),
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Nil => Value::Nil,
        Literal::Duration(ns) => Value::Duration(*ns),
        Literal::Time(s) => Value::Time(s.clone()),
        Literal::Bytes(b) => Value::Bytes(b.clone()),
    }
}

fn parse_decimal(s: &str) -> Option<f64> {
    // Strip a trailing `d` if the source spelling carried it.
    let s = s.strip_suffix('d').unwrap_or(s);
    s.parse::<f64>().ok()
}

fn fmt_decimal(f: f64) -> String {
    // v0: format without the `d` suffix — the suffix is part
    // of literal syntax, not the value's printed form. Matches
    // how Decimal literals are stored by the lexer (`1.0`, not
    // `1.0d`). Precision is not yet shopspring/decimal-grade;
    // milestone 3 swaps the internal store.
    format!("{}", f)
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
            let af = parse_decimal(a)
                .ok_or_else(|| format!("decimal parse failed: `{}`", a))?;
            let bf = parse_decimal(b)
                .ok_or_else(|| format!("decimal parse failed: `{}`", b))?;
            Ok(Value::Decimal(fmt_decimal(af + bf)))
        }
        (Sub, Value::Decimal(a), Value::Decimal(b)) => {
            let af = parse_decimal(a)
                .ok_or_else(|| format!("decimal parse failed: `{}`", a))?;
            let bf = parse_decimal(b)
                .ok_or_else(|| format!("decimal parse failed: `{}`", b))?;
            Ok(Value::Decimal(fmt_decimal(af - bf)))
        }
        (Mul, Value::Decimal(a), Value::Decimal(b)) => {
            let af = parse_decimal(a)
                .ok_or_else(|| format!("decimal parse failed: `{}`", a))?;
            let bf = parse_decimal(b)
                .ok_or_else(|| format!("decimal parse failed: `{}`", b))?;
            Ok(Value::Decimal(fmt_decimal(af * bf)))
        }
        (Div, Value::Decimal(a), Value::Decimal(b)) => {
            let af = parse_decimal(a)
                .ok_or_else(|| format!("decimal parse failed: `{}`", a))?;
            let bf = parse_decimal(b)
                .ok_or_else(|| format!("decimal parse failed: `{}`", b))?;
            if bf == 0.0 {
                return Err("decimal division by zero".to_string());
            }
            Ok(Value::Decimal(fmt_decimal(af / bf)))
        }
        (Lt | Gt | LtEq | GtEq, Value::Decimal(a), Value::Decimal(b)) => {
            let af = parse_decimal(a)
                .ok_or_else(|| format!("decimal parse failed: `{}`", a))?;
            let bf = parse_decimal(b)
                .ok_or_else(|| format!("decimal parse failed: `{}`", b))?;
            Ok(Value::Bool(match op {
                Lt => af < bf,
                Gt => af > bf,
                LtEq => af <= bf,
                GtEq => af >= bf,
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
        (Value::Nil, Value::Nil) => true,
        (Value::Unit, Value::Unit) => true,
        _ => false,
    }
}

fn eval_unop(op: UnaryOp, v: &Value) -> Result<Value, String> {
    match (op, v) {
        (UnaryOp::Neg, Value::Int(n)) => Ok(Value::Int(-n)),
        (UnaryOp::Neg, Value::Float(f)) => Ok(Value::Float(-f)),
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
    let (la, ra, ta) = match (l, r, tol) {
        (Value::Int(a), Value::Int(b), Value::Int(t)) => (*a as f64, *b as f64, *t as f64),
        (Value::Int(a), Value::Int(b), Value::Float(t)) => (*a as f64, *b as f64, *t),
        (Value::Float(a), Value::Float(b), Value::Int(t)) => (*a, *b, *t as f64),
        (Value::Float(a), Value::Float(b), Value::Float(t)) => (*a, *b, *t),
        _ => {
            return Err(format!(
                "~~ expects numeric operands; got {} ~~ {} within {}",
                l.type_name(),
                r.type_name(),
                tol.type_name()
            ))
        }
    };
    Ok(Value::Bool((la - ra).abs() <= ta))
}
