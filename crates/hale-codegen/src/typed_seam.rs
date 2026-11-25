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


use crate::codegen::{CodegenError, CodegenTy, Cx};

/// An interface `accept(c: I)` / `release(c: I)` monomorph being lowered
/// (`Cx::current_accept_interface`): the interface `I`, the locus it is
/// lowered for, and the parameter that holds the child, which the body
/// reads as that locus while the checker typed it `I` (GH #2111).
#[derive(Debug, Clone)]
pub(crate) struct AcceptMonomorph {
    pub(crate) iface: String,
    pub(crate) child: String,
    pub(crate) param: String,
}

/// What a call that hands back an aggregate of loci asks of the checker
/// ([`Cx::ty_required`]): its type, whose loci its owner tears down.
pub(crate) const AGGREGATE_CALL: &str = "the checker's type of this call, which hands back an aggregate of loci";

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// The checker's type of `e`, an expression of the body being lowered,
    /// as a layout. Lowering types no value itself (GH #2111): with no row
    /// (none recorded, two answers, `Unknown`, a type with no layout, or
    /// lowered outside every body) it answers `None` so that the caller's
    /// lowering can go on, and the build is refused when lowering ends, at
    /// `e` ([`Cx::refuse_missing_rows`]): a compiler defect, never a guess.
    pub(crate) fn ty_at(&self, e: &Expr) -> Option<CodegenTy> {
        self.ty_required(e, &format!("the checker's type of this {}", kind_name(e)))
    }

    /// [`Cx::ty_at`], naming what lowering asked (`what`) where the row is
    /// missing.
    pub(crate) fn ty_required(&self, e: &Expr, what: &str) -> Option<CodegenTy> {
        let t = self.row_layout(e).ok();
        if t.is_none() {
            self.required_misses.borrow_mut().push((what.to_string(), e.span(), self.lowering_stdlib_body()));
        }
        t
    }

    /// The checker's type of `e`, refused at `e` where there is none: the
    /// type of a value lowering hands back, which it takes from the row.
    pub(crate) fn row_of(&self, e: &Expr) -> Result<CodegenTy, CodegenError> {
        self.row_layout(e).map_err(|_| {
            CodegenError::missing_row("expression_typing", format!("the checker's type of this {}", kind_name(e)), None)
                .placed(e.span(), self.lowering_stdlib_body())
        })
    }

    /// GH #2111: the value `e` lowered to, of `built`, handed back as the
    /// checker's type of it, which lowering takes from the row. A value of
    /// another type is a compiler defect, refused at `e`: lowering and the
    /// checker disagree on what `e` is. A row of `()` (an expression that
    /// gives no value, built as statements) is no type, and the value is
    /// none either.
    pub(crate) fn typed_by_row(
        &self,
        e: &Expr,
        (v, built): (inkwell::values::BasicValueEnum<'ctx>, CodegenTy),
    ) -> Result<(inkwell::values::BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if self.row_is_unit(e) {
            return Ok((v, built));
        }
        let row = self.row_of(e)?;
        if row != built {
            return Err(CodegenError::Invalid(format!(
                "the checker types this {} `{row}`, and lowering built `{built}`: a compiler defect",
                kind_name(e)
            ))
            .placed(e.span(), self.lowering_stdlib_body()));
        }
        Ok((v, row))
    }

    /// The element type and capacity of the receiver `args[0]` when the
    /// checker types it a `bounded[T; N]`; `None` for a receiver of another
    /// type, or none. A projection of the receiver's row (GH #2111): until
    /// the checker's rows reached lowering, lowering typed the receiver
    /// again with a walk of its own over a subset of the grammar, which
    /// grew a shape at a time as programs reached the build untyped
    /// (downstream handoffs, 2026-11-06 and 2026-11-14).
    pub(crate) fn bounded_recv_spec(&self, args: &[Expr]) -> Option<(CodegenTy, u64)> {
        match self.ty_required(args.first()?, "the checker's type of this `bounded` call's receiver")? {
            CodegenTy::Bounded(elem, cap) => Some((*elem, cap)),
            _ => None,
        }
    }

    /// Whether the checker types `e` `()`: a block the check accepted in
    /// a value's place only because it never ends (`f() or { return x; }`,
    /// `block_always_diverges`), so lowering builds it as statements and
    /// asks it for no value. A row of `()` has no layout, so [`Cx::ty_at`]
    /// answers no such site.
    pub(crate) fn row_is_unit(&self, e: &Expr) -> bool {
        matches!(self.checker_row(e), Ok(Ty::Unit))
    }

    /// [`Cx::ty_at`]'s answer, or why there is none.
    fn row_layout(&self, e: &Expr) -> Result<CodegenTy, &'static str> {
        // a quantity literal is built as the count the check converted it
        // to, its conversion's target
        if let Some(t) = self.quantity_literal_target(e) {
            return Ok(t);
        }
        let ty = self.checker_row(e)?;
        // an interface `accept` / `release` monomorph reads its child as the
        // locus it is lowered for, where the checker typed the interface
        if let (Some(m), Expr::Ident(id), Ty::Named(n)) = (&self.current_accept_interface, e, ty) {
            if id.name == m.param && *n == m.iface {
                return self.layout_of(&Ty::Named(m.child.clone())).ok_or("no layout");
            }
        }
        self.layout_of(ty).ok_or("no layout")
    }

    /// The checker's row of `e`, by the body, the monomorph's arguments and
    /// the default being evaluated, or why there is none.
    fn checker_row(&self, e: &Expr) -> Result<&Ty, &'static str> {
        let body = self.current_body.ok_or("no body")?;
        let args = self.specialization_args();
        let path = self.default_evaluation.as_slice();
        let ty = match (args, path.is_empty()) {
            (None, true) => self.expr_types.at(body, e),
            // a monomorph's body outside every default: its own walk's
            // row, else the template's (a site whose type names no
            // parameter)
            (Some(_), true) => self.scoped_row_of(body, args, path, e).or_else(|| self.expr_types.at(body, e)),
            // a default evaluated here: the row of this evaluation
            (_, false) => self.scoped_row_of(body, args, path, e),
        };
        let Some(ty) = ty else {
            let typed_unknown = path.is_empty() && self.expr_types.recorded(body, e).is_some();
            return Err(if typed_unknown { "unknown" } else { "no row" });
        };
        Ok(ty)
    }

    /// The row of `e` on the walk of `args` and `path`: a program body's
    /// ([`hale_types::typed_bodies::TypedBodies::scoped_expr`]), else a
    /// stdlib body's, which the stdlib's own walk typed
    /// ([`hale_types::typed_bodies::ExprTypes::scoped_at`]).
    fn scoped_row_of(&self, body: hale_syntax::ast::NodeId, args: Option<&[Ty]>, path: &[u32], e: &Expr) -> Option<&Ty> {
        self.typed.scoped_expr(body, args, path, e).or_else(|| self.expr_types.scoped_at(body, args, path, e))
    }

    /// A path to an imported seed's fn or a Hale-bodied stdlib fn, used
    /// as a value (GH #1082): the fn pointer of `mangled`, the fn the path
    /// names, typed by the path's row. It used to be lowered as a name
    /// built at the path, which has no row of its own (GH #2111).
    pub(crate) fn path_fn_value(&self, path: &Expr, mangled: &str) -> Result<(inkwell::values::BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let sig = self
            .user_fns
            .get(mangled)
            .ok_or_else(|| CodegenError::Unsupported(format!("unknown identifier `{mangled}`")))?;
        let ty = self.ty_at(path).ok_or_else(|| {
            CodegenError::missing_row("expression_typing", "the checker's type of this fn value", Some(path.span()))
        })?;
        Ok((sig.func.as_global_value().as_pointer_value().into(), ty))
    }

    /// GH #2111: the type a quantity literal is built as: its value
    /// conversion's target (U3: the checker converted it into the
    /// denomination it flows into, `100ms` into a `Millis` an `Int`, `Time(
    /// 5us)`'s literal a `Time`). `None` for another expression, or a
    /// literal with no row.
    pub(crate) fn quantity_literal_target(&self, e: &Expr) -> Option<CodegenTy> {
        let Expr::Literal(hale_syntax::ast::Literal::Quantity { .. }, span) = e else { return None };
        let row = self.row(hale_types::typed_bodies::SiteKind::value(*span))?;
        row.count?;
        Some(self.unit_repr(&row.to))
    }

    /// A top-level constant's value, read at `e` ([`Cx::const_param`]): an
    /// integer is its declared type's, at that type's width (`const SMALL:
    /// U64 = 7`), as the checker's row of the read says; lowering typed
    /// every integer constant an `Int`.
    pub(crate) fn const_value(&mut self, e: &Expr, pv: &crate::codegen::ParamValue) -> (inkwell::values::BasicValueEnum<'ctx>, CodegenTy) {
        match (pv, self.ty_at(e)) {
            (crate::codegen::ParamValue::Int(n), Some(t @ (CodegenTy::U64 | CodegenTy::Narrow(_)))) => {
                let bits = t.machine_int().map_or(64, |(b, _)| b);
                (self.context.custom_width_int_type(bits).const_int(*n as u64, true).into(), t)
            }
            _ => self.const_param(pv),
        }
    }

    /// GH #2111, at the end of lowering: a value lowered with no row of the
    /// checker's ([`Cx::ty_at`]) is a compiler defect, refused at the first
    /// one's site.
    pub(crate) fn refuse_missing_rows(&self) -> Result<(), CodegenError> {
        match self.required_misses.borrow().first() {
            Some((what, at, stdlib)) => {
                Err(CodegenError::missing_row("expression_typing", what, None).placed(*at, *stdlib))
            }
            None => Ok(()),
        }
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
        // read as `ty_at` reads an expression's row: a monomorph's own
        // walk (else the template's), a default's evaluation's
        let row = self.current_body.and_then(|body| {
            let args = self.specialization_args();
            let path = self.default_evaluation.as_slice();
            match (args, path.is_empty()) {
                (None, true) => self.expr_types.place_at(body, key),
                (Some(_), true) => self.typed.scoped_row(body, args, path, key).or_else(|| self.expr_types.place_at(body, key)),
                (_, false) => self.typed.scoped_row(body, args, path, key),
            }
        });
        row.and_then(|t| self.layout_of(t))
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
        // a locus's param defaults are its own text, lowered where it is
        // built: their specialization is the built locus's
        // (`params_init_self`), or none for a locus that is no monomorph,
        // whatever the enclosing fn's or method's is
        if let (Some(built), true) = (&self.params_init_self, self.in_params_default) {
            let m = self.typed.monomorphs().named(&built.locus_name)?;
            return (m.kind == hale_types::typed_bodies::TemplateKind::Locus).then_some(m.args.as_slice());
        }
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
        // A quantity at a denomination it does not declare (`Money in
        // 1/10000 cent`, `Duration in s`) is a type the checker
        // synthesizes and no declaration spells: it is the integer it
        // counts.
        if let Ty::Named(n) = t {
            if n.contains(" in ") {
                let int = self.unit_int(t)?;
                return self.type_expr_to_codegen_ty(&TypeExpr::Primitive(int, hale_syntax::Span::new(0, 0))).ok();
            }
        }
        // A capacity slot's cell is a type no declaration spells: the cell
        // lowering builds, of its element, from its locus and slot (F.22)
        if let Ty::Cell { elem, locus, slot } = t {
            return Some(CodegenTy::Cell(Box::new(self.layout_of(elem)?), Some((locus.clone(), slot.clone()))));
        }
        // A batch handler's `Drain<T>` is spelled as its monomorph
        // (`Drain_T`), which no declaration declares: the run of `T`s
        // lowering hands the handler
        if let Ty::Named(n) = t {
            if let Some(elem) = n.strip_prefix("Drain_").filter(|e| self.user_types.contains_key(*e)) {
                let span = hale_syntax::Span::new(0, 0);
                let drain = TypeExpr::Named {
                    path: QualifiedName { segments: vec![Ident::new("Drain".to_string(), span)], span },
                    generic_args: vec![spelled(&Ty::Named(elem.to_string()))?],
                    span,
                };
                return self.type_expr_to_codegen_ty(&drain).ok();
            }
        }
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
        Ty::Projection(..) | Ty::Cell { .. } | Ty::Unknown => return None,
    })
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
