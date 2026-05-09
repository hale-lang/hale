//! Runtime values for the tree-walking interpreter.
//!
//! v0 cut: simple owned values. No interning, no arenas. Each
//! locus instance carries its own state map; struct values
//! carry their fields. Not optimized — the interpreter is the
//! "is the language semantically real" check, not the
//! production execution path.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use lotus_syntax::ast::{FnDecl, LocusDecl, ModeKind};

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    /// Decimal stored as its source spelling for v0 — proper
    /// decimal arithmetic comes when we wire shopspring-shape
    /// FFI in milestone 3.
    Decimal(String),
    String(String),
    Bool(bool),
    Duration(i64),
    Time(String),
    Bytes(Vec<u8>),
    Nil,
    Unit,
    Array(Rc<RefCell<Vec<Value>>>),
    Tuple(Vec<Value>),
    Struct {
        name: String,
        fields: Rc<RefCell<BTreeMap<String, Value>>>,
    },
    /// A live locus instance. Holds its state and a shared
    /// reference to its declaration so methods can be invoked
    /// off the handle.
    Locus(LocusHandle),
    /// A user-defined free `fn`.
    Fn(FnRef),
    /// Builtin function (println, print, etc.).
    Builtin(BuiltinRef),
}

#[derive(Debug, Clone)]
pub struct LocusHandle {
    pub name: String,
    pub state: Rc<RefCell<BTreeMap<String, Value>>>,
    pub children: Rc<RefCell<Vec<LocusHandle>>>,
    pub decl: Rc<LocusDecl>,
    /// Tracks whether `dissolve_locus` has run on this handle.
    /// Ephemeral loci dissolve immediately at end of instantiation
    /// (sets the flag), and the parent's later cascade then skips
    /// already-dissolved children. The handle itself stays in
    /// `parent.children` so `for child in self.children` can still
    /// observe the (post-dissolve) state — the locus's `state`
    /// Rc is shared, so reads remain valid even after dissolution.
    pub dissolved: Rc<std::cell::Cell<bool>>,
    /// m40: restart attempt counter — bumped by the
    /// `restart(child)` recovery primitive inside an
    /// `on_failure` body. The post-on_failure dispatch in
    /// `instantiate_locus` reads it to decide whether to re-run
    /// `birth()` + birth-epoch closures. Cap is 2 attempts per
    /// locus lifetime (v0 default); past the cap, restart is a
    /// no-op and the violation falls through to the parent's
    /// collapse path.
    pub restart_count: Rc<std::cell::Cell<i64>>,
    /// m41: sticky quarantine flag set by the
    /// `quarantine(child)` recovery primitive. Once set,
    /// `run()` is skipped on this locus; drain / dissolve
    /// still fire (cleanup is unconditional). Bus-dispatch
    /// gating waits on m41b.
    pub quarantined: Rc<std::cell::Cell<bool>>,
    /// m45: signals the next restart re-run should reset
    /// user fields to declared defaults before invoking
    /// birth(). Set by `restart_in_place(c)`; cleared by
    /// the rerun branch in `instantiate_locus` after the
    /// re-init pass runs. Both restart and restart_in_place
    /// share the cap-2 budget on `restart_count`; this
    /// flag only changes whether the re-run preserves
    /// state or factory-resets it.
    pub restart_in_place_pending: Rc<std::cell::Cell<bool>>,
    /// m43: per-duration-closure last-fire timestamps in
    /// monotonic nanoseconds. Vec is parallel to the locus's
    /// declared duration-epoch closures (in declaration
    /// order). Initialized to time::monotonic() at
    /// instantiation so the first fire happens after `N`
    /// has elapsed since birth, not immediately.
    pub duration_last_fire: Rc<RefCell<Vec<i64>>>,
    /// m44: the locus's parent at instantiation time
    /// (`parent_stack.last()` at the moment the LocusHandle
    /// was built). Stored so primitives like
    /// `check_closures();` — called from inside the locus's
    /// body where parent_stack has been pushed with self —
    /// can route violations to the correct on_failure
    /// handler. None for top-level loci. Creates an Rc
    /// cycle (parent.children references child, child.parent
    /// references parent) but the cycle only matters at
    /// process exit; v0 accepts the leak.
    pub parent: Rc<RefCell<Option<LocusHandle>>>,
    /// m46: per-closure accumulator state. Keyed by closure name;
    /// the inner Vec parallels the closure's `sum(...)` calls in
    /// occurrence order (left expr's sums first, then right's,
    /// then tolerance's). Initialized at instantiation. Each
    /// epoch fire samples-and-updates each slot before the
    /// closure's assertion is evaluated, so `sum(...)` references
    /// in the assertion read the post-update running total.
    /// Cleared on recovery events (restart / restart_in_place /
    /// quarantine) unless the closure's `persists_through(...)`
    /// clause names the event.
    pub accumulators: Rc<RefCell<BTreeMap<String, Vec<Value>>>>,
}

#[derive(Debug, Clone)]
pub struct FnRef {
    pub decl: Rc<FnDecl>,
}

#[derive(Clone)]
pub struct BuiltinRef {
    pub name: &'static str,
    pub func: Rc<dyn Fn(&[Value]) -> Result<Value, String>>,
}

impl std::fmt::Debug for BuiltinRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuiltinRef").field("name", &self.name).finish()
    }
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "Int",
            Value::Float(_) => "Float",
            Value::Decimal(_) => "Decimal",
            Value::String(_) => "String",
            Value::Bool(_) => "Bool",
            Value::Duration(_) => "Duration",
            Value::Time(_) => "Time",
            Value::Bytes(_) => "Bytes",
            Value::Nil => "Nil",
            Value::Unit => "()",
            Value::Array(_) => "Array",
            Value::Tuple(_) => "Tuple",
            Value::Struct { .. } => "Struct",
            Value::Locus(_) => "Locus",
            Value::Fn(_) => "Fn",
            Value::Builtin(_) => "Builtin",
        }
    }

    pub fn display(&self) -> String {
        match self {
            Value::Int(n) => n.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Decimal(s) => s.clone(),
            Value::String(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Duration(ns) => format!("{}ns", ns),
            Value::Time(s) => s.clone(),
            Value::Bytes(b) => format!("<{} bytes>", b.len()),
            Value::Nil => "nil".to_string(),
            Value::Unit => "()".to_string(),
            Value::Array(a) => {
                let items: Vec<String> = a.borrow().iter().map(|v| v.display()).collect();
                format!("[{}]", items.join(", "))
            }
            Value::Tuple(t) => {
                let items: Vec<String> = t.iter().map(|v| v.display()).collect();
                format!("({})", items.join(", "))
            }
            Value::Struct { name, fields } => {
                let parts: Vec<String> = fields
                    .borrow()
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v.display()))
                    .collect();
                format!("{} {{ {} }}", name, parts.join(", "))
            }
            Value::Locus(h) => format!("<locus {}>", h.name),
            Value::Fn(_) => "<fn>".to_string(),
            Value::Builtin(b) => format!("<builtin {}>", b.name),
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Nil => false,
            _ => true,
        }
    }
}

/// A duration literal in source carries its raw integer with
/// the multiplier baked in (per the lexer). We keep a small
/// helper here so the eval layer can talk about it.
pub fn duration_ns(value: i64) -> Value {
    Value::Duration(value)
}

pub fn mode_kind_name(k: ModeKind) -> &'static str {
    match k {
        ModeKind::Bulk => "bulk",
        ModeKind::Harmonic => "harmonic",
        ModeKind::Resolution => "resolution",
    }
}
