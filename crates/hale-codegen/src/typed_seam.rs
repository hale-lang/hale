//! GH #2111 (phase 1): the seam through which lowering takes a value's
//! type from the checker instead of typing it again.
//!
//! The checker types every expression of every body; the lowering view
//! carries its answers keyed by body (`hale_types::typed_bodies::ExprTypes`,
//! [`hale_types::resolved::LoweringView::expr_types`]). [`Cx::ty_at`] reads
//! the row of an expression of the body being lowered (`current_body`,
//! entered by `Cx::in_body`), in a monomorph the checker's walk of that
//! monomorph and inside a default the walk of the evaluation being lowered
//! (`TypedBodies::scoped_expr`), and lays the type out as a `CodegenTy`
//! ([`Cx::layout_of`]) through the one function that lays out a written
//! type (`type_expr_to_codegen_ty`), so a value's layout and a declared
//! one never differ. `CodegenTy` stays the layout type; what it is a
//! layout of is the checker's answer.
//!
//! A family of constructs moves onto the seam one at a time (the survey
//! on GH #2111); a site with no row answers `None`, which its family
//! reads as it read its own walk's `None` until then.

use hale_syntax::ast::{Expr, Ident, QualifiedName, TypeExpr};
use hale_types::ty::Ty;

use hale_types::typed_bodies::{expr_kind, expr_name};

use crate::codegen::{CodegenError, CodegenTy, Cx, MissingRow};

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// The checker's type of `e`, an expression of the body being lowered,
    /// as a layout: `None` outside every body, where the checker recorded
    /// no row (or two answers), or for a type with no layout.
    pub(crate) fn ty_at(&self, e: &Expr) -> Option<CodegenTy> {
        match self.row_layout(e) {
            Ok(t) => Some(t),
            Err(reason) => {
                if self.seam_strict {
                    self.note_missing_row(e, reason);
                }
                None
            }
        }
    }

    /// [`Cx::ty_at`] at a site that takes another path where the checker
    /// left no row: one of the seam's fallbacks (GH #2111), counted by
    /// `seam_strict_baseline`. A site moves to `ty_at` when it refuses a
    /// missing row instead.
    pub(crate) fn ty_or_fallback(&self, e: &Expr) -> Option<CodegenTy> {
        self.ty_at(e)
    }

    /// One of the seam's fallbacks (GH #2111): lowering types `e` itself
    /// below this call, the checker's row or none. Under strict mode a
    /// missing row is noted here; otherwise it does nothing. Every call
    /// site is counted by `seam_strict_baseline`, and goes when its
    /// lowering takes `e`'s type from the row.
    pub(crate) fn seam_fallback(&self, e: &Expr) {
        if self.seam_strict {
            let _ = self.ty_at(e);
        }
    }

    /// [`Cx::ty_at`]'s answer, or why there is none (a [`MissingRow`]'s
    /// reason).
    fn row_layout(&self, e: &Expr) -> Result<CodegenTy, &'static str> {
        let body = self.current_body.ok_or("no body")?;
        let args = self.specialization_args();
        let path = self.default_evaluation.as_slice();
        let ty = match (args, path.is_empty()) {
            (None, true) => self.expr_types.at(body, e),
            // a monomorph's body outside every default: its own walk's
            // row, else the template's (a site whose type names no
            // parameter)
            (Some(_), true) => self.typed.scoped_expr(body, args, path, e).or_else(|| self.expr_types.at(body, e)),
            // a default evaluated here: the row of this evaluation
            (_, false) => self.typed.scoped_expr(body, args, path, e),
        };
        let Some(ty) = ty else {
            let typed_unknown = path.is_empty() && self.expr_types.recorded(body, e).is_some();
            return Err(if typed_unknown { "unknown" } else { "no row" });
        };
        self.layout_of(ty).ok_or("no layout")
    }

    /// GH #2111 strict mode: `e` is lowered without the checker's type.
    fn note_missing_row(&self, e: &Expr, reason: &'static str) {
        let sp = e.span();
        let stdlib = self.lowering_stdlib_body();
        let key = (sp.start.0, sp.end.0, expr_kind(e), expr_name(e), stdlib);
        self.seam_misses.borrow_mut().entry(key).or_insert_with(|| MissingRow {
            family: family_of(e),
            kind: kind_name(e),
            stdlib,
            reason,
            span: sp,
            in_fn: self
                .current_fn
                .map(|f| f.get_name().to_string_lossy().into_owned())
                .unwrap_or_default(),
        });
    }

    /// GH #2111 strict mode, at the end of lowering: every expression it
    /// typed without the checker's row, refused together.
    pub(crate) fn refuse_missing_rows(&self) -> Result<(), CodegenError> {
        if !self.seam_strict {
            return Ok(());
        }
        let rows: Vec<MissingRow> = self.seam_misses.borrow().values().cloned().collect();
        if rows.is_empty() {
            return Ok(());
        }
        Err(CodegenError::MissingRows(rows))
    }

    /// GH #2111: the checker's type of the assignment target `target`, a
    /// place of the body being lowered: its head's place, or its last
    /// segment's (a field by its name, an element by its index). The
    /// checker types every assignment's target, so a target with no row,
    /// or with a type that has no layout, is refused at its span, a
    /// compiler defect rather than a guess.
    pub(crate) fn place_ty(&self, target: &hale_syntax::ast::LValue) -> Result<CodegenTy, CodegenError> {
        let key = match target.tail.last() {
            None => hale_types::typed_bodies::place_name_key(&target.head),
            Some(hale_syntax::ast::LValueSeg::Field(f)) => hale_types::typed_bodies::place_name_key(f),
            Some(hale_syntax::ast::LValueSeg::Index(ix)) => hale_types::typed_bodies::place_index_key(ix),
        };
        self.current_body
            .and_then(|body| self.expr_types.place_at(body, key))
            .and_then(|t| self.layout_of(t))
            .ok_or_else(|| {
                CodegenError::missing_row("expression_typing", "the checker's type of this assignment's target", Some(target.span))
            })
    }

    /// The checker's type of the call `call`, the value its lowering
    /// hands back, where lowering has no signature of its own to read
    /// (a bare builtin's result: `len`, `to_string`, `min` over its
    /// operands). A call with no row is refused at its span, a compiler
    /// defect rather than a guess (GH #2111).
    pub(crate) fn call_result(&self, call: &Expr) -> Result<CodegenTy, CodegenError> {
        self.ty_at(call).ok_or_else(|| {
            CodegenError::missing_row("expression_typing", "the checker's type of this call", Some(call.span()))
        })
    }

    /// The type arguments of the monomorph being lowered: a generic fn's
    /// specialization (`current_specialization`), else the generic
    /// locus's monomorph the current `self` is. The checker walks each
    /// monomorph's body under its arguments, and lowering reads that walk.
    fn specialization_args(&self) -> Option<&'p [Ty]> {
        if let Some((_, args)) = &self.current_specialization {
            // the args live as long as the specialization being lowered;
            // read them through the monomorph table, which lives as long
            // as the lowering view
            if let Some(m) = self.typed.monomorphs().rows().iter().find(|m| &m.args == args) {
                return Some(&m.args);
            }
        }
        let cs = self.current_self.as_ref()?;
        let m = self.typed.monomorphs().named(&cs.locus_name)?;
        (m.kind == hale_types::typed_bodies::TemplateKind::Locus).then_some(m.args.as_slice())
    }

    /// The layout of the checker's type `t`: the layout of the type
    /// expression that spells it (`type_expr_to_codegen_ty`). A fallible
    /// value is laid out as its success type, which is what an `or`
    /// leaves; `Unknown` and a projection have none here.
    pub(crate) fn layout_of(&self, t: &Ty) -> Option<CodegenTy> {
        let te = spelled(t)?;
        self.type_expr_to_codegen_ty(&te).ok()
    }
}

/// The type expression that spells `t`, as a declaration would write it.
fn spelled(t: &Ty) -> Option<TypeExpr> {
    let span = hale_syntax::Span::new(0, 0);
    Some(match t {
        Ty::Prim(p) => TypeExpr::Primitive(*p, span),
        Ty::Named(n) => TypeExpr::Named {
            path: QualifiedName { segments: vec![Ident::new(n.clone(), span)], span },
            generic_args: Vec::new(),
            span,
        },
        Ty::Array(e, n) => TypeExpr::Array {
            elem: Box::new(spelled(e)?),
            size: n.map(|n| Expr::Literal(hale_syntax::ast::Literal::Int(n as i64), span)),
            span,
        },
        Ty::Bounded(e, cap) => TypeExpr::Bounded { elem: Box::new(spelled(e)?), cap: *cap, span },
        Ty::Tuple(ps) => TypeExpr::Tuple(ps.iter().map(spelled).collect::<Option<_>>()?, span),
        Ty::Function { params, ret } => TypeExpr::Function {
            params: params.iter().map(spelled).collect::<Option<_>>()?,
            ret: match ret.as_ref() {
                Ty::Unit => None,
                r => Some(Box::new(spelled(r)?)),
            },
            span,
        },
        Ty::Fallible { success, .. } => spelled(success)?,
        Ty::Unit => TypeExpr::Tuple(Vec::new(), span),
        Ty::Projection(..) | Ty::Unknown => return None,
    })
}

/// The survey's family of an expression lowering types (GH #2111):
/// what a [`MissingRow`] is counted under.
fn family_of(e: &Expr) -> u8 {
    match e {
        Expr::Call { callee, .. } => match callee.as_ref() {
            Expr::Path(q) if q.segments.first().is_some_and(|s| s.name == "std") => 9,
            _ => 2,
        },
        Expr::Path(_) | Expr::Path2 { .. } | Expr::Field { .. } | Expr::Index { .. } | Expr::KwSelf(_) => 3,
        Expr::Ident(_) => 4,
        Expr::Struct { .. } | Expr::Array(..) | Expr::ArrayRepeat { .. } | Expr::Tuple(..) => 5,
        Expr::If(_) | Expr::Match(_) | Expr::Block(_) | Expr::Or { .. } => 6,
        Expr::Literal(..)
        | Expr::Binary { .. }
        | Expr::Unary { .. }
        | Expr::Range { .. }
        | Expr::Sum(..)
        | Expr::Prod(..)
        | Expr::Approx { .. } => 7,
    }
}

/// An expression's kind, by name.
fn kind_name(e: &Expr) -> &'static str {
    match e {
        Expr::Literal(..) => "literal",
        Expr::Ident(_) => "ident",
        Expr::Path(_) => "path",
        Expr::KwSelf(_) => "self",
        Expr::Binary { .. } => "binary",
        Expr::Unary { .. } => "unary",
        Expr::Call { .. } => "call",
        Expr::Field { .. } => "field",
        Expr::Path2 { .. } => "path2",
        Expr::Index { .. } => "index",
        Expr::Tuple(..) => "tuple",
        Expr::Array(..) => "array",
        Expr::Struct { .. } => "struct",
        Expr::Block(_) => "block",
        Expr::If(_) => "if",
        Expr::Match(_) => "match",
        Expr::Sum(..) => "sum",
        Expr::Prod(..) => "prod",
        Expr::Approx { .. } => "approx",
        Expr::Range { .. } => "range",
        Expr::ArrayRepeat { .. } => "array_repeat",
        Expr::Or { .. } => "or",
    }
}
