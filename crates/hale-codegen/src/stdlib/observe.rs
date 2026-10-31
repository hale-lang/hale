//! `std::observe::*` path-call lowering (GH #1592): the process-wide
//! series table a `std::observe::Scrape` param scrapes.
//!
//! Every call lowers to its runtime function only in a program whose
//! main locus holds the backend that serves it: the series table's in
//! one holding a `std::observe::Scrape` param ([`Cx::observes_metrics`]),
//! the event ring's (GH #1592 S4) in one holding a `std::observe::Otlp`
//! ([`Cx::observes_events`]). In any other program it lowers to its
//! answer when nothing is registered — `-1` for an id, `0`, `0.0` or
//! `""` — and calls nothing, so a `std::metrics::Registry` there
//! registers nothing and pays nothing.

use hale_syntax::ast::Expr;
use hale_syntax::ast::TopDecl;
use hale_types::stdlib_surface::IntrinsicId;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum};

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

/// One argument's kind at the C boundary.
#[derive(Clone, Copy)]
enum Arg {
    Int,
    Float,
    Str,
}

/// What the call answers.
#[derive(Clone, Copy, PartialEq)]
enum Ret {
    /// An id: `-1` when nothing is registered.
    Id,
    Int,
    Float,
    /// A string the runtime writes into the caller's arena.
    Str,
}

/// The `observe` kind a call serves, and its runtime function's shape.
fn shape(id: IntrinsicId) -> Option<(&'static str, &'static str, &'static [Arg], Ret)> {
    use IntrinsicId as Id;
    let events = match id {
        Id::ObserveEventsTakeRaw => Some(("lotus_observe_events_take", &[Arg::Int][..], Ret::Int)),
        Id::ObserveEventStrRaw => Some(("lotus_observe_event_str", &[Arg::Int, Arg::Int][..], Ret::Str)),
        Id::ObserveEventIntRaw => Some(("lotus_observe_event_int", &[Arg::Int, Arg::Int][..], Ret::Int)),
        Id::ObserveEventsDroppedRaw => Some(("lotus_observe_events_dropped", &[Arg::Int][..], Ret::Int)),
        _ => None,
    };
    if let Some((c, a, r)) = events {
        return Some(("events", c, a, r));
    }
    let (c, a, r) = metrics_shape(id)?;
    Some(("metrics", c, a, r))
}

fn metrics_shape(id: IntrinsicId) -> Option<(&'static str, &'static [Arg], Ret)> {
    use IntrinsicId as Id;
    Some(match id {
        Id::ObserveRegistryRaw => ("lotus_observe_registry", &[Arg::Str], Ret::Id),
        Id::ObserveDropRegistryRaw => ("lotus_observe_registry_drop", &[Arg::Int], Ret::Int),
        Id::ObserveSeriesRaw => (
            "lotus_observe_series",
            &[Arg::Int, Arg::Str, Arg::Str, Arg::Int, Arg::Str],
            Ret::Id,
        ),
        Id::ObserveAddRaw => ("lotus_observe_add", &[Arg::Int, Arg::Float], Ret::Int),
        Id::ObserveSetRaw => ("lotus_observe_set", &[Arg::Int, Arg::Float], Ret::Int),
        Id::ObserveObserveRaw => ("lotus_observe_hist", &[Arg::Int, Arg::Float], Ret::Int),
        Id::ObserveSnapshotRaw => ("lotus_observe_snapshot", &[], Ret::Int),
        Id::ObserveSnapStrRaw => ("lotus_observe_snap_str", &[Arg::Int, Arg::Int], Ret::Str),
        Id::ObserveSnapNumRaw => ("lotus_observe_snap_num", &[Arg::Int, Arg::Int], Ret::Float),
        Id::ObserveSnapIntRaw => ("lotus_observe_snap_int", &[Arg::Int, Arg::Int], Ret::Int),
        _ => return None,
    })
}

/// Whether `items` hold a main locus whose params hold a `metrics`
/// backend (`std::observe::Scrape`, recognized by its type:
/// `hale_types::observe_expand`).
pub(crate) fn declares_observe_metrics(items: &[TopDecl]) -> bool {
    hale_types::observe_expand::holds_backend(items, "metrics")
}

pub(crate) trait ObserveStdlib<'ctx> {
    fn lower_std_observe(
        &mut self,
        id: IntrinsicId,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;
}

impl<'ctx, 'p> ObserveStdlib<'ctx> for Cx<'ctx, 'p> {
    fn lower_std_observe(
        &mut self,
        id: IntrinsicId,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let e = |e: inkwell::builder::BuilderError| CodegenError::LlvmEmit(e.to_string());
        let (kind, cname, kinds, ret) =
            shape(id).ok_or_else(|| CodegenError::Unsupported(format!("{id:?} is not a std::observe call")))?;
        if args.len() != kinds.len() {
            return Err(CodegenError::Unsupported(format!(
                "{cname} takes {} args, got {}",
                kinds.len(),
                args.len()
            )));
        }
        let i64_t = self.context.i64_type();
        let declared = if kind == "events" { self.observes_events() } else { self.observes_metrics() };
        if !declared {
            // No backend for the kind: the answer with nothing
            // registered, and no call. The arguments are not lowered
            // either: every one is a plain read (a param, a literal, a
            // label set's field) with nothing to run.
            return Ok(match ret {
                Ret::Id => (i64_t.const_int(-1i64 as u64, true).into(), CodegenTy::Int),
                Ret::Int => (i64_t.const_zero().into(), CodegenTy::Int),
                Ret::Float => (self.context.f64_type().const_zero().into(), CodegenTy::Float),
                Ret::Str => (self.global_string("").into(), CodegenTy::String),
            });
        }
        let mut vals: Vec<BasicMetadataValueEnum<'ctx>> = Vec::new();
        if ret == Ret::Str {
            vals.push(self.current_arena_ptr()?.into());
        }
        for (a, k) in args.iter().zip(kinds) {
            let (v, ty) = self.lower_expr(a, scope)?;
            let v = match (k, &ty) {
                (Arg::Int, CodegenTy::Int) => v,
                (Arg::Float, CodegenTy::Float) => v,
                (Arg::Str, CodegenTy::String | CodegenTy::StringView) => self.unpack_view_if_needed(v, &ty)?,
                _ => {
                    return Err(CodegenError::Unsupported(format!("{cname}: an argument of type {ty:?}")));
                }
            };
            vals.push(v.into());
        }
        let f = self.module.get_function(cname).unwrap_or_else(|| panic!("{cname} declared"));
        let v = self
            .builder
            .build_call(f, &vals, "observe.ret")
            .map_err(e)?
            .try_as_basic_value()
            .left()
            .expect("returns a value");
        Ok((
            v,
            match ret {
                Ret::Id | Ret::Int => CodegenTy::Int,
                Ret::Float => CodegenTy::Float,
                Ret::Str => CodegenTy::String,
            },
        ))
    }
}
