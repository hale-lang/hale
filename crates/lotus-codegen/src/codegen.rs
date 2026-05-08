//! AST → LLVM IR → object file → executable, for the
//! milestone-0 subset.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
    TargetTriple,
};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, PointerValue};
use inkwell::{AddressSpace, OptimizationLevel};

use lotus_syntax::ast::*;

/// Compile-time tag for a value's type. Mirrors a small subset
/// of `lotus_types::Ty`; we don't pull the full type system in
/// because codegen only needs to discriminate the lowered
/// shapes (Int/Float/Bool are scalar; String is ptr).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LotusType {
    Int,
    Float,
    Bool,
    String,
}

#[derive(Debug)]
pub enum CodegenError {
    Unsupported(String),
    LlvmInit(String),
    LlvmEmit(String),
    Link(String),
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodegenError::Unsupported(s) => write!(f, "unsupported in codegen v0: {}", s),
            CodegenError::LlvmInit(s) => write!(f, "LLVM init failed: {}", s),
            CodegenError::LlvmEmit(s) => write!(f, "LLVM emit failed: {}", s),
            CodegenError::Link(s) => write!(f, "link failed: {}", s),
        }
    }
}

impl std::error::Error for CodegenError {}

/// Compile `program` to an executable at `output_path`. Uses
/// `clang` to link the object file produced by LLVM.
pub fn build_executable(
    program: &Program,
    output_path: &Path,
) -> Result<(), CodegenError> {
    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| CodegenError::LlvmInit(e.to_string()))?;

    let context = Context::create();
    let module = context.create_module("lotus_main");
    let builder = context.create_builder();

    let mut cx = Cx {
        context: &context,
        module,
        builder,
        program,
    };

    cx.declare_builtins();
    cx.lower_program()?;

    // Emit object file, then link via clang.
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple)
        .map_err(|e| CodegenError::LlvmInit(e.to_string()))?;
    let machine = target
        .create_target_machine(
            &triple,
            "generic",
            "",
            OptimizationLevel::Default,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| {
            CodegenError::LlvmInit("could not create target machine".to_string())
        })?;

    let obj_path: PathBuf = output_path.with_extension("o");
    machine
        .write_to_file(&cx.module, FileType::Object, &obj_path)
        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

    let status = Command::new("clang")
        .arg(&obj_path)
        .arg("-o")
        .arg(output_path)
        .status()
        .map_err(|e| CodegenError::Link(format!("clang invocation: {}", e)))?;
    let _ = std::fs::remove_file(&obj_path);
    if !status.success() {
        return Err(CodegenError::Link(format!(
            "clang exited with {}",
            status
        )));
    }
    Ok(())
}

struct Cx<'ctx, 'p> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: inkwell::builder::Builder<'ctx>,
    program: &'p Program,
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    fn declare_builtins(&self) {
        // declare i32 @printf(ptr, ...)
        let i32_t = self.context.i32_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let printf_ty = i32_t.fn_type(&[ptr_t.into()], true);
        self.module.add_function("printf", printf_ty, None);
    }

    fn lower_program(&mut self) -> Result<(), CodegenError> {
        // Locate fn main.
        let main_decl = self
            .program
            .items
            .iter()
            .find_map(|item| match item {
                TopDecl::Fn(f) if f.name.name == "main" => Some(f),
                _ => None,
            })
            .ok_or_else(|| {
                CodegenError::Unsupported("program has no `fn main()`".to_string())
            })?;

        // Define the C entry point: i32 @main()
        let i32_t = self.context.i32_type();
        let main_ty = i32_t.fn_type(&[], false);
        let main_fn = self.module.add_function("main", main_ty, None);
        let entry = self.context.append_basic_block(main_fn, "entry");
        self.builder.position_at_end(entry);

        let mut scope = Scope::default();
        for stmt in &main_decl.body.stmts {
            self.lower_stmt(stmt, &mut scope)?;
        }

        // return 0;
        let zero = i32_t.const_int(0, false);
        self.builder
            .build_return(Some(&zero))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    fn lower_stmt(
        &mut self,
        stmt: &Stmt,
        scope: &mut Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        match stmt {
            Stmt::Expr(Expr::Struct { path, inits, .. }) => {
                if path.segments.len() != 1 {
                    return Err(CodegenError::Unsupported(format!(
                        "qualified-name locus literal `{}`",
                        path.segments
                            .iter()
                            .map(|s| s.name.as_str())
                            .collect::<Vec<_>>()
                            .join("::")
                    )));
                }
                let name = &path.segments[0].name;
                let locus_decl = self
                    .program
                    .items
                    .iter()
                    .find_map(|item| match item {
                        TopDecl::Locus(l) if &l.name.name == name => Some(l.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "no locus `{}` declared",
                            name
                        ))
                    })?;
                self.lower_locus_birth(&locus_decl, inits)?;
                Ok(())
            }
            Stmt::Expr(Expr::Call { callee, args, .. }) => {
                let name = match callee.as_ref() {
                    Expr::Ident(i) => i.name.as_str(),
                    _ => {
                        return Err(CodegenError::Unsupported(
                            "non-identifier callee".to_string(),
                        ))
                    }
                };
                self.lower_print_call(name, args, scope, &BTreeMap::new())
            }
            Stmt::Let { name, value, .. } => {
                let (val, ty) = self.lower_expr(value, scope, &BTreeMap::new())?;
                let alloca = self.alloca_for(ty, &name.name)?;
                self.builder
                    .build_store(alloca, val)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                scope.locals.insert(name.name.clone(), (alloca, ty));
                Ok(())
            }
            Stmt::Assign { target, op, value, .. } => {
                // v0 codegen: only bare-local assignment. `self.X =`
                // and field/index lvalues require the locus-as-struct
                // ABI which lands later in phase 3.
                if target.head.name == "self" || !target.tail.is_empty() {
                    return Err(CodegenError::Unsupported(
                        "assignment target other than a local variable"
                            .to_string(),
                    ));
                }
                let (alloca, slot_ty) = scope
                    .locals
                    .get(&target.head.name)
                    .copied()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "assignment to unbound `{}`",
                            target.head.name
                        ))
                    })?;
                let (rhs, rhs_ty) =
                    self.lower_expr(value, scope, &BTreeMap::new())?;
                let new_val = if matches!(op, AssignOp::Eq) {
                    if rhs_ty != slot_ty {
                        return Err(CodegenError::Unsupported(format!(
                            "type mismatch in assignment: slot {:?} vs rhs {:?}",
                            slot_ty, rhs_ty
                        )));
                    }
                    rhs
                } else {
                    let bin_op = match op {
                        AssignOp::PlusEq => BinOp::Add,
                        AssignOp::MinusEq => BinOp::Sub,
                        AssignOp::StarEq => BinOp::Mul,
                        AssignOp::SlashEq => BinOp::Div,
                        AssignOp::PercentEq => BinOp::Mod,
                        other => {
                            return Err(CodegenError::Unsupported(format!(
                                "compound assignment {:?}",
                                other
                            )));
                        }
                    };
                    let llvm_ty = self.llvm_basic_type(slot_ty);
                    let cur = self
                        .builder
                        .build_load(llvm_ty, alloca, &target.head.name)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let (v, _) = self.lower_binop(bin_op, cur, rhs, slot_ty)?;
                    v
                };
                self.builder
                    .build_store(alloca, new_val)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok(())
            }
            Stmt::Expr(_) => Err(CodegenError::Unsupported(
                "expression statement other than locus literal or builtin call"
                    .to_string(),
            )),
            _ => Err(CodegenError::Unsupported(format!(
                "statement form {:?}",
                std::mem::discriminant(stmt)
            ))),
        }
    }

    fn alloca_for(
        &self,
        ty: LotusType,
        name: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        match ty {
            LotusType::Int => self
                .builder
                .build_alloca(self.context.i64_type(), name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string())),
            LotusType::Float => self
                .builder
                .build_alloca(self.context.f64_type(), name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string())),
            LotusType::Bool => self
                .builder
                .build_alloca(self.context.bool_type(), name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string())),
            LotusType::String => self
                .builder
                .build_alloca(self.context.ptr_type(AddressSpace::default()), name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string())),
        }
    }

    fn lower_expr(
        &mut self,
        e: &Expr,
        scope: &Scope<'ctx>,
        self_state: &BTreeMap<String, ParamValue>,
    ) -> Result<(BasicValueEnum<'ctx>, LotusType), CodegenError> {
        match e {
            Expr::Literal(Literal::Int(n), _) => {
                let v = self.context.i64_type().const_int(*n as u64, true);
                Ok((v.into(), LotusType::Int))
            }
            Expr::Literal(Literal::Float(f), _) => {
                let v = self.context.f64_type().const_float(*f);
                Ok((v.into(), LotusType::Float))
            }
            Expr::Literal(Literal::Bool(b), _) => {
                let v = self.context.bool_type().const_int(*b as u64, false);
                Ok((v.into(), LotusType::Bool))
            }
            Expr::Literal(Literal::String(s), _) => {
                let p = self.global_string(s);
                Ok((p.into(), LotusType::String))
            }
            Expr::Ident(id) => {
                let (alloca, ty) = scope.locals.get(&id.name).ok_or_else(|| {
                    CodegenError::Unsupported(format!(
                        "unknown identifier `{}`",
                        id.name
                    ))
                })?;
                let llvm_ty = self.llvm_basic_type(*ty);
                let loaded = self
                    .builder
                    .build_load(llvm_ty, *alloca, &id.name)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((loaded, *ty))
            }
            Expr::Field { receiver, name, .. }
                if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
            {
                let value = self_state.get(&name.name).ok_or_else(|| {
                    CodegenError::Unsupported(format!(
                        "self.{}: param not in compile-time state",
                        name.name
                    ))
                })?;
                Ok(self.const_param(value))
            }
            Expr::Binary { op, left, right, span: _ } => {
                let (lv, lt) = self.lower_expr(left, scope, self_state)?;
                let (rv, rt) = self.lower_expr(right, scope, self_state)?;
                if lt != rt {
                    return Err(CodegenError::Unsupported(format!(
                        "binary op operands of mixed types {:?} and {:?}",
                        lt, rt
                    )));
                }
                self.lower_binop(*op, lv, rv, lt)
            }
            Expr::Unary { op, operand, .. } => {
                let (v, t) = self.lower_expr(operand, scope, self_state)?;
                self.lower_unop(*op, v, t)
            }
            _ => Err(CodegenError::Unsupported(format!(
                "expression form {:?}",
                std::mem::discriminant(e)
            ))),
        }
    }

    fn const_param(
        &mut self,
        v: &ParamValue,
    ) -> (BasicValueEnum<'ctx>, LotusType) {
        match v {
            ParamValue::Int(n) => (
                self.context.i64_type().const_int(*n as u64, true).into(),
                LotusType::Int,
            ),
            ParamValue::Float(f) => (
                self.context.f64_type().const_float(*f).into(),
                LotusType::Float,
            ),
            ParamValue::Bool(b) => (
                self.context.bool_type().const_int(*b as u64, false).into(),
                LotusType::Bool,
            ),
            ParamValue::String(s) => (self.global_string(s).into(), LotusType::String),
        }
    }

    fn llvm_basic_type(
        &self,
        t: LotusType,
    ) -> inkwell::types::BasicTypeEnum<'ctx> {
        match t {
            LotusType::Int => self.context.i64_type().into(),
            LotusType::Float => self.context.f64_type().into(),
            LotusType::Bool => self.context.bool_type().into(),
            LotusType::String => self.context.ptr_type(AddressSpace::default()).into(),
        }
    }

    fn lower_binop(
        &mut self,
        op: BinOp,
        lv: BasicValueEnum<'ctx>,
        rv: BasicValueEnum<'ctx>,
        ty: LotusType,
    ) -> Result<(BasicValueEnum<'ctx>, LotusType), CodegenError> {
        use inkwell::IntPredicate as IP;
        use inkwell::FloatPredicate as FP;
        match (op, ty) {
            (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod, LotusType::Int) => {
                let l = lv.into_int_value();
                let r = rv.into_int_value();
                let v = match op {
                    BinOp::Add => self.builder.build_int_add(l, r, "add"),
                    BinOp::Sub => self.builder.build_int_sub(l, r, "sub"),
                    BinOp::Mul => self.builder.build_int_mul(l, r, "mul"),
                    BinOp::Div => self.builder.build_int_signed_div(l, r, "sdiv"),
                    BinOp::Mod => self.builder.build_int_signed_rem(l, r, "srem"),
                    _ => unreachable!(),
                };
                let v = v.map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v.into(), LotusType::Int))
            }
            (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div, LotusType::Float) => {
                let l = lv.into_float_value();
                let r = rv.into_float_value();
                let v = match op {
                    BinOp::Add => self.builder.build_float_add(l, r, "fadd"),
                    BinOp::Sub => self.builder.build_float_sub(l, r, "fsub"),
                    BinOp::Mul => self.builder.build_float_mul(l, r, "fmul"),
                    BinOp::Div => self.builder.build_float_div(l, r, "fdiv"),
                    _ => unreachable!(),
                };
                let v = v.map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v.into(), LotusType::Float))
            }
            (BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq,
                LotusType::Int) =>
            {
                let l = lv.into_int_value();
                let r = rv.into_int_value();
                let pred = match op {
                    BinOp::Eq => IP::EQ,
                    BinOp::NotEq => IP::NE,
                    BinOp::Lt => IP::SLT,
                    BinOp::Gt => IP::SGT,
                    BinOp::LtEq => IP::SLE,
                    BinOp::GtEq => IP::SGE,
                    _ => unreachable!(),
                };
                let v = self
                    .builder
                    .build_int_compare(pred, l, r, "icmp")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v.into(), LotusType::Bool))
            }
            (BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq,
                LotusType::Float) =>
            {
                let l = lv.into_float_value();
                let r = rv.into_float_value();
                let pred = match op {
                    BinOp::Eq => FP::OEQ,
                    BinOp::NotEq => FP::ONE,
                    BinOp::Lt => FP::OLT,
                    BinOp::Gt => FP::OGT,
                    BinOp::LtEq => FP::OLE,
                    BinOp::GtEq => FP::OGE,
                    _ => unreachable!(),
                };
                let v = self
                    .builder
                    .build_float_compare(pred, l, r, "fcmp")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v.into(), LotusType::Bool))
            }
            (BinOp::And, LotusType::Bool) => {
                let v = self
                    .builder
                    .build_and(lv.into_int_value(), rv.into_int_value(), "and")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v.into(), LotusType::Bool))
            }
            (BinOp::Or, LotusType::Bool) => {
                let v = self
                    .builder
                    .build_or(lv.into_int_value(), rv.into_int_value(), "or")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v.into(), LotusType::Bool))
            }
            _ => Err(CodegenError::Unsupported(format!(
                "binop {:?} on {:?}",
                op, ty
            ))),
        }
    }

    fn lower_unop(
        &mut self,
        op: UnaryOp,
        v: BasicValueEnum<'ctx>,
        ty: LotusType,
    ) -> Result<(BasicValueEnum<'ctx>, LotusType), CodegenError> {
        match (op, ty) {
            (UnaryOp::Neg, LotusType::Int) => {
                let zero = self.context.i64_type().const_int(0, true);
                let r = self
                    .builder
                    .build_int_sub(zero, v.into_int_value(), "neg")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((r.into(), LotusType::Int))
            }
            (UnaryOp::Neg, LotusType::Float) => {
                let r = self
                    .builder
                    .build_float_neg(v.into_float_value(), "fneg")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((r.into(), LotusType::Float))
            }
            (UnaryOp::Not, LotusType::Bool) => {
                let r = self
                    .builder
                    .build_not(v.into_int_value(), "not")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((r.into(), LotusType::Bool))
            }
            _ => Err(CodegenError::Unsupported(format!(
                "unop {:?} on {:?}",
                op, ty
            ))),
        }
    }

    /// Lower a `print` / `println` call. Used from both the
    /// fn-body context (where args may reference local
    /// bindings) and the birth-body context (where args may
    /// reference compile-time-known self.X params).
    fn lower_print_call(
        &mut self,
        name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
        self_state: &BTreeMap<String, ParamValue>,
    ) -> Result<(), CodegenError> {
        if name != "println" && name != "print" {
            return Err(CodegenError::Unsupported(format!("builtin `{}`", name)));
        }
        let mut format = String::new();
        let mut printf_args: Vec<BasicMetadataValueEnum> =
            Vec::with_capacity(args.len() + 1);
        // Reserve format-string slot at index 0; filled in
        // after we know the format.
        printf_args.push(BasicMetadataValueEnum::PointerValue(
            self.context.ptr_type(AddressSpace::default()).const_null(),
        ));
        for a in args {
            // String literals splice into the format directly
            // (no value pushed); everything else lowers to an
            // LLVM value.
            if let Expr::Literal(Literal::String(s), _) = a {
                format.push_str(&escape_format(s));
                continue;
            }
            let (val, ty) = self.lower_expr(a, scope, self_state)?;
            match ty {
                LotusType::Int => {
                    format.push_str("%lld");
                    printf_args.push(BasicMetadataValueEnum::IntValue(val.into_int_value()));
                }
                LotusType::Float => {
                    format.push_str("%g");
                    printf_args
                        .push(BasicMetadataValueEnum::FloatValue(val.into_float_value()));
                }
                LotusType::Bool => {
                    // No printf %b; widen to a string at format
                    // time via a select on the lowered i1.
                    let true_s = self.global_string("true");
                    let false_s = self.global_string("false");
                    let chosen = self
                        .builder
                        .build_select(val.into_int_value(), true_s, false_s, "boolstr")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    format.push_str("%s");
                    printf_args.push(BasicMetadataValueEnum::PointerValue(
                        chosen.into_pointer_value(),
                    ));
                }
                LotusType::String => {
                    format.push_str("%s");
                    printf_args.push(BasicMetadataValueEnum::PointerValue(
                        val.into_pointer_value(),
                    ));
                }
            }
        }
        if name == "println" {
            format.push('\n');
        }
        let fmt_ptr = self.global_string(&format);
        printf_args[0] = BasicMetadataValueEnum::PointerValue(fmt_ptr);
        let printf = self
            .module
            .get_function("printf")
            .expect("printf declared");
        self.builder
            .build_call(printf, &printf_args, "printf_call")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// Lower `LocusName { ... };` for a locus that has only
    /// `params` and `birth()`. The locus's state is a flat
    /// scope of scalars (string / Int / Float / Bool); birth()
    /// runs inline against them. Beyond birth(), the rest of
    /// the lifecycle is unimplemented in this milestone.
    fn lower_locus_birth(
        &mut self,
        locus: &LocusDecl,
        inits: &[StructInit],
    ) -> Result<(), CodegenError> {
        let mut state: std::collections::BTreeMap<String, ParamValue> =
            std::collections::BTreeMap::new();
        for member in &locus.members {
            if let LocusMember::Params(pb) = member {
                for p in &pb.params {
                    if let ParamInit::Value(e) = &p.init {
                        state.insert(p.name.name.clone(), param_value(e)?);
                    }
                }
            }
        }
        for init in inits {
            state.insert(init.name.name.clone(), param_value(&init.value)?);
        }

        // Lower birth() body.
        let birth = locus.members.iter().find_map(|m| match m {
            LocusMember::Lifecycle(lc) if matches!(lc.kind, LifecycleKind::Birth) => {
                Some(lc.body.clone())
            }
            _ => None,
        });
        let Some(body) = birth else {
            return Ok(());
        };
        for stmt in &body.stmts {
            self.lower_birth_stmt(stmt, &state)?;
        }
        Ok(())
    }

    fn lower_birth_stmt(
        &mut self,
        stmt: &Stmt,
        state: &BTreeMap<String, ParamValue>,
    ) -> Result<(), CodegenError> {
        match stmt {
            Stmt::Expr(Expr::Call { callee, args, .. }) => {
                let name = match callee.as_ref() {
                    Expr::Ident(i) => i.name.as_str(),
                    _ => {
                        return Err(CodegenError::Unsupported(
                            "non-identifier callee".to_string(),
                        ))
                    }
                };
                self.lower_print_call(name, args, &Scope::default(), state)
            }
            _ => Err(CodegenError::Unsupported(
                "birth-body statement other than println / print".to_string(),
            )),
        }
    }

    fn global_string(&mut self, s: &str) -> PointerValue<'ctx> {
        let g = self
            .builder
            .build_global_string_ptr(s, "s")
            .expect("build_global_string_ptr");
        g.as_pointer_value()
    }
}

#[derive(Default)]
struct Scope<'ctx> {
    locals: BTreeMap<String, (PointerValue<'ctx>, LotusType)>,
}

#[derive(Debug, Clone)]
enum ParamValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

fn param_value(e: &Expr) -> Result<ParamValue, CodegenError> {
    match e {
        Expr::Literal(Literal::String(s), _) => Ok(ParamValue::String(s.clone())),
        Expr::Literal(Literal::Int(n), _) => Ok(ParamValue::Int(*n)),
        Expr::Literal(Literal::Float(f), _) => Ok(ParamValue::Float(*f)),
        Expr::Literal(Literal::Bool(b), _) => Ok(ParamValue::Bool(*b)),
        _ => Err(CodegenError::Unsupported(
            "param initializer must be a literal in milestone-1 codegen".to_string(),
        )),
    }
}

fn escape_format(s: &str) -> String {
    s.replace('%', "%%")
}

/// LLVM produces architecture-specific triples; expose a way
/// to override for cross-compilation tests later.
pub fn host_triple() -> TargetTriple {
    TargetMachine::get_default_triple()
}
