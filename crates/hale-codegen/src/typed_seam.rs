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

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// The checker's type of `e`, an expression of the body being lowered,
    /// as a layout: `None` outside every body, where the checker recorded
    /// no row (or two answers), or for a type with no layout.
    pub(crate) fn ty_at(&self, e: &Expr) -> Option<CodegenTy> {
        let body = self.current_body?;
        let args = self.specialization_args();
        let path = self.default_evaluation.as_slice();
        let ty = match (args, path.is_empty()) {
            (None, true) => self.expr_types.at(body, e)?,
            // a monomorph's body outside every default: its own walk's
            // row, else the template's (a site whose type names no
            // parameter)
            (Some(_), true) => self.typed.scoped_expr(body, args, path, e).or_else(|| self.expr_types.at(body, e))?,
            // a default evaluated here: the row of this evaluation
            (_, false) => self.typed.scoped_expr(body, args, path, e)?,
        };
        self.layout_of(ty)
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

