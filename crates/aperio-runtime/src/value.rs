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

use aperio_syntax::ast::{FnDecl, LocusDecl, ModeKind};

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    /// m48: exact fixed-point Decimal — i128 mantissa with a
    /// per-value scale (number of fractional digits). Replaces
    /// the v0 string-spelling representation; arithmetic now
    /// operates on the mantissa directly with no f64 round-trip.
    /// Source spelling round-trips as long as no operation
    /// expands scale past what `display` can render.
    Decimal(DecimalVal),
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
    /// m47 + payloads: enum variant value. The variant carries
    /// its payload field values in declaration order; an empty
    /// `payload` Vec covers the no-payload case. Codegen
    /// represents this as either an i32 tag (no-payload-only
    /// enums) or a pointer to `{i32, [N x i8]}` storage; the
    /// interpreter keeps the pair of names + the payload list
    /// so pattern-binding fields and identity comparisons work
    /// without consulting a separate enum-decl registry walk.
    EnumVariant {
        enum_name: String,
        variant_name: String,
        payload: Vec<Value>,
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
            Value::EnumVariant { .. } => "EnumVariant",
            Value::Locus(_) => "Locus",
            Value::Fn(_) => "Fn",
            Value::Builtin(_) => "Builtin",
        }
    }

    pub fn display(&self) -> String {
        match self {
            Value::Int(n) => n.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Decimal(d) => d.display(),
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
            Value::EnumVariant { enum_name, variant_name, payload } => {
                if payload.is_empty() {
                    format!("{}::{}", enum_name, variant_name)
                } else {
                    let parts: Vec<String> =
                        payload.iter().map(|v| v.display()).collect();
                    format!("{}::{}({})", enum_name, variant_name, parts.join(", "))
                }
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

/// m48: exact fixed-point decimal value. `mantissa × 10^-scale`.
/// Per-value scale (rather than a fixed program-wide scale) so
/// source spelling round-trips: `1.50d` stores as
/// (mantissa=150, scale=2) and prints back as `1.50` (display
/// strips trailing zeros to match the existing %g-equivalent
/// shape, but the underlying scale preserves whatever precision
/// arithmetic produced).
///
/// Overflow at v0.1 is unchecked — same policy as Int. i128 is
/// wide enough for currency / engineering work where both
/// operands stay under ~10^19; a workload that hits a real
/// limit will tell us before we add saturating / checked
/// variants.
#[derive(Debug, Clone, Copy)]
pub struct DecimalVal {
    pub mantissa: i128,
    pub scale: u32,
}

impl DecimalVal {
    pub fn zero() -> Self {
        Self { mantissa: 0, scale: 0 }
    }

    pub fn from_i64(n: i64) -> Self {
        Self { mantissa: n as i128, scale: 0 }
    }

    /// Parse a decimal numeric literal. Accepts an optional
    /// trailing `d` (the decimal-suffix marker the lexer carries
    /// in some paths) and an optional leading sign.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.strip_suffix('d').unwrap_or(s);
        let (sign, rest) = match s.as_bytes().first() {
            Some(b'-') => (-1i128, &s[1..]),
            Some(b'+') => (1i128, &s[1..]),
            _ => (1i128, s),
        };
        if rest.is_empty() {
            return None;
        }
        let mut mantissa: i128 = 0;
        let mut scale: u32 = 0;
        let mut seen_dot = false;
        let mut seen_digit = false;
        for c in rest.chars() {
            match c {
                '0'..='9' => {
                    seen_digit = true;
                    mantissa = mantissa.checked_mul(10)?;
                    mantissa = mantissa.checked_add((c as u8 - b'0') as i128)?;
                    if seen_dot {
                        scale = scale.checked_add(1)?;
                    }
                }
                '.' if !seen_dot => {
                    seen_dot = true;
                }
                '_' => {}
                _ => return None,
            }
        }
        if !seen_digit {
            return None;
        }
        Some(Self { mantissa: sign * mantissa, scale })
    }

    /// Renders the value with the natural number of fractional
    /// digits (matches stored `scale`), trimming trailing zeros
    /// and a dangling decimal point. Negative values keep the
    /// minus sign on the integer part.
    pub fn display(&self) -> String {
        if self.scale == 0 {
            return self.mantissa.to_string();
        }
        let neg = self.mantissa < 0;
        let abs = self.mantissa.unsigned_abs();
        let pow = 10u128.pow(self.scale);
        let int_part = abs / pow;
        let frac_part = abs % pow;
        let frac_str = format!("{:0width$}", frac_part, width = self.scale as usize);
        let trimmed = frac_str.trim_end_matches('0');
        let mut out = String::new();
        if neg {
            out.push('-');
        }
        out.push_str(&int_part.to_string());
        if !trimmed.is_empty() {
            out.push('.');
            out.push_str(trimmed);
        }
        out
    }

    pub fn to_f64(&self) -> f64 {
        (self.mantissa as f64) / 10f64.powi(self.scale as i32)
    }

    /// Align two decimals to a common scale (the larger of the
    /// two). Used by add/sub/cmp. Mantissa scaling is unchecked —
    /// if the smaller-scale side already has a near-i128 mantissa,
    /// shifting may overflow.
    fn align(a: Self, b: Self) -> (i128, i128, u32) {
        let scale = a.scale.max(b.scale);
        let a_m = a.mantissa * 10i128.pow(scale - a.scale);
        let b_m = b.mantissa * 10i128.pow(scale - b.scale);
        (a_m, b_m, scale)
    }

    pub fn add(a: Self, b: Self) -> Self {
        let (am, bm, scale) = Self::align(a, b);
        Self { mantissa: am + bm, scale }
    }

    pub fn sub(a: Self, b: Self) -> Self {
        let (am, bm, scale) = Self::align(a, b);
        Self { mantissa: am - bm, scale }
    }

    pub fn mul(a: Self, b: Self) -> Self {
        Self {
            mantissa: a.mantissa * b.mantissa,
            scale: a.scale + b.scale,
        }
    }

    /// Division. Result scale is the larger of the inputs (with a
    /// 9-digit minimum so `1d / 3d` keeps useful precision rather
    /// than truncating to 0). Truncates toward zero — same as i64.
    pub fn div(a: Self, b: Self) -> Result<Self, String> {
        if b.mantissa == 0 {
            return Err("decimal division by zero".to_string());
        }
        let target_scale = a.scale.max(b.scale).max(9);
        // Want result_mantissa = round((a.m * 10^(target+b.scale-a.scale)) / b.m)
        // i.e., scale numerator so the implicit scale of the
        // quotient equals target_scale.
        let extra = target_scale + b.scale - a.scale;
        let scaled_num = a.mantissa * 10i128.pow(extra);
        Ok(Self {
            mantissa: scaled_num / b.mantissa,
            scale: target_scale,
        })
    }

    pub fn neg(self) -> Self {
        Self { mantissa: -self.mantissa, scale: self.scale }
    }

    pub fn cmp(a: Self, b: Self) -> std::cmp::Ordering {
        let (am, bm, _) = Self::align(a, b);
        am.cmp(&bm)
    }

    pub fn eq(a: Self, b: Self) -> bool {
        Self::cmp(a, b) == std::cmp::Ordering::Equal
    }
}

impl PartialEq for DecimalVal {
    fn eq(&self, other: &Self) -> bool {
        DecimalVal::eq(*self, *other)
    }
}

impl Eq for DecimalVal {}
