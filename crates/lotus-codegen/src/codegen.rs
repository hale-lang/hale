//! AST → LLVM IR → object file → executable, for the
//! milestone-0 subset.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use inkwell::basic_block::BasicBlock;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
    TargetTriple,
};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, FunctionValue, PointerValue,
};
use inkwell::{AddressSpace, OptimizationLevel};

use lotus_syntax::ast::*;

/// Compile-time tag for a value's type. Mirrors a small subset
/// of `lotus_types::Ty`; we don't pull the full type system in
/// because codegen only needs to discriminate the lowered
/// shapes (Int/Float/Bool/Duration are scalar i64/f64/i1/i64;
/// String is a ptr to a NUL-terminated byte array).
///
/// `Duration` is logically an i64 nanosecond count, distinct from
/// `Int` so type-driven dispatch (e.g. `time::sleep` accepts only
/// Duration) stays correct at the codegen layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LotusType {
    Int,
    Float,
    Bool,
    String,
    Duration,
}

/// Did the last lowered statement leave the current basic block
/// open for further IR (Open) or close it with a terminator like
/// `br` / `ret` / `unreachable` (Terminated)? Statements after a
/// Terminated must not emit further IR into the same block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockEnd {
    Open,
    Terminated,
}

#[derive(Debug, Clone, Copy)]
struct LoopFrame<'ctx> {
    continue_bb: BasicBlock<'ctx>,
    break_bb: BasicBlock<'ctx>,
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
        current_fn: None,
        loops: Vec::new(),
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
    /// Set while lowering a function's body so that `if` / `while`
    /// can `append_basic_block` onto it.
    current_fn: Option<FunctionValue<'ctx>>,
    /// Stack of enclosing loops so `break` / `continue` can find
    /// their target blocks.
    loops: Vec<LoopFrame<'ctx>>,
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    fn declare_builtins(&self) {
        // declare i32 @printf(ptr, ...)
        let i32_t = self.context.i32_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let printf_ty = i32_t.fn_type(&[ptr_t.into()], true);
        self.module.add_function("printf", printf_ty, None);

        // declare i32 @clock_nanosleep(i32, i32, ptr, ptr)
        //
        // Backing primitive for `time::sleep` on the monotonic
        // clock. CLOCK_MONOTONIC means NTP / wall-clock adjustments
        // cannot warp scheduling; EINTR retry uses `rem` so signals
        // do not shorten the total sleep. CLOCK_REALTIME is reserved
        // for `time::now()` (wall-clock observation) and never used
        // for scheduling.
        let clock_nanosleep_ty =
            i32_t.fn_type(&[i32_t.into(), i32_t.into(), ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("clock_nanosleep", clock_nanosleep_ty, None);
    }

    /// LLVM struct type matching `struct timespec` on Linux x86_64
    /// (`{ time_t tv_sec; long tv_nsec }` ≡ `{ i64, i64 }`).
    /// 32-bit / non-Linux ABIs would need a different layout; we
    /// only target 64-bit Linux for now.
    fn timespec_type(&self) -> inkwell::types::StructType<'ctx> {
        let i64_t = self.context.i64_type();
        self.context.struct_type(&[i64_t.into(), i64_t.into()], false)
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
        self.current_fn = Some(main_fn);

        let mut scope = Scope::default();
        let end = self.lower_block(&main_decl.body, &mut scope)?;

        // Only emit `ret 0` if the body fell through. If it ended
        // in a terminator (e.g. an unreachable `if` whose branches
        // both `break`/`return`), the trailing block is already
        // closed and writing more IR is unsound.
        if end == BlockEnd::Open {
            let zero = i32_t.const_int(0, false);
            self.builder
                .build_return(Some(&zero))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        self.current_fn = None;
        Ok(())
    }

    fn lower_block(
        &mut self,
        block: &Block,
        scope: &mut Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        for stmt in &block.stmts {
            match self.lower_stmt(stmt, scope)? {
                BlockEnd::Open => continue,
                BlockEnd::Terminated => return Ok(BlockEnd::Terminated),
            }
        }
        Ok(BlockEnd::Open)
    }

    fn lower_stmt(
        &mut self,
        stmt: &Stmt,
        scope: &mut Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
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
                Ok(BlockEnd::Open)
            }
            Stmt::Expr(Expr::Call { callee, args, .. }) => {
                match callee.as_ref() {
                    Expr::Ident(i) => {
                        self.lower_print_call(
                            i.name.as_str(),
                            args,
                            scope,
                            &BTreeMap::new(),
                        )?;
                    }
                    Expr::Path(qn) => {
                        self.lower_path_call(qn, args, scope)?;
                    }
                    _ => {
                        return Err(CodegenError::Unsupported(
                            "non-identifier callee".to_string(),
                        ));
                    }
                }
                Ok(BlockEnd::Open)
            }
            Stmt::Let { name, value, .. } => {
                let (val, ty) = self.lower_expr(value, scope, &BTreeMap::new())?;
                let alloca = self.alloca_for(ty, &name.name)?;
                self.builder
                    .build_store(alloca, val)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                scope.locals.insert(name.name.clone(), (alloca, ty));
                Ok(BlockEnd::Open)
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
                Ok(BlockEnd::Open)
            }
            Stmt::If(if_stmt) => self.lower_if(if_stmt, scope),
            Stmt::While { cond, body, .. } => {
                self.lower_while(cond, body, scope)
            }
            Stmt::Break(_) => self.lower_break(),
            Stmt::Continue(_) => self.lower_continue(),
            Stmt::Block(b) => self.lower_block(b, scope),
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

    fn lower_if(
        &mut self,
        ifs: &IfStmt,
        scope: &mut Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        let (cond_v, cond_ty) =
            self.lower_expr(&ifs.cond, scope, &BTreeMap::new())?;
        if cond_ty != LotusType::Bool {
            return Err(CodegenError::Unsupported(format!(
                "if condition must be Bool; got {:?}",
                cond_ty
            )));
        }
        let func = self
            .current_fn
            .expect("current_fn set while lowering an if");
        let then_bb = self.context.append_basic_block(func, "then");
        let else_bb = self.context.append_basic_block(func, "else");
        let merge_bb = self.context.append_basic_block(func, "ifend");

        self.builder
            .build_conditional_branch(cond_v.into_int_value(), then_bb, else_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // then-branch
        self.builder.position_at_end(then_bb);
        let then_end = self.lower_block(&ifs.then_block, scope)?;
        if then_end == BlockEnd::Open {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }

        // else-branch (or empty fall-through if absent)
        self.builder.position_at_end(else_bb);
        let else_end = match &ifs.else_block {
            None => BlockEnd::Open,
            Some(eb) => match eb.as_ref() {
                ElseBranch::Else(b) => self.lower_block(b, scope)?,
                ElseBranch::ElseIf(nested) => self.lower_if(nested, scope)?,
            },
        };
        if else_end == BlockEnd::Open {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }

        // If both arms terminated, merge_bb has no predecessors —
        // close it with `unreachable` so the function is well-formed,
        // and report the if itself as Terminated so callers don't
        // emit further code into it.
        if then_end == BlockEnd::Terminated
            && else_end == BlockEnd::Terminated
        {
            self.builder.position_at_end(merge_bb);
            self.builder
                .build_unreachable()
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            return Ok(BlockEnd::Terminated);
        }

        self.builder.position_at_end(merge_bb);
        Ok(BlockEnd::Open)
    }

    fn lower_while(
        &mut self,
        cond: &Expr,
        body: &Block,
        scope: &mut Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        let func = self
            .current_fn
            .expect("current_fn set while lowering a while");
        let header_bb = self.context.append_basic_block(func, "while.cond");
        let body_bb = self.context.append_basic_block(func, "while.body");
        let exit_bb = self.context.append_basic_block(func, "while.end");

        self.builder
            .build_unconditional_branch(header_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // header: evaluate cond and branch
        self.builder.position_at_end(header_bb);
        let (cond_v, cond_ty) = self.lower_expr(cond, scope, &BTreeMap::new())?;
        if cond_ty != LotusType::Bool {
            return Err(CodegenError::Unsupported(format!(
                "while condition must be Bool; got {:?}",
                cond_ty
            )));
        }
        self.builder
            .build_conditional_branch(cond_v.into_int_value(), body_bb, exit_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // body
        self.builder.position_at_end(body_bb);
        self.loops.push(LoopFrame {
            continue_bb: header_bb,
            break_bb: exit_bb,
        });
        let body_end = self.lower_block(body, scope)?;
        self.loops.pop();
        if body_end == BlockEnd::Open {
            self.builder
                .build_unconditional_branch(header_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }

        // The exit is reachable from the header (cond=false) and
        // any `break`s inside the body, so it's always Open.
        self.builder.position_at_end(exit_bb);
        Ok(BlockEnd::Open)
    }

    fn lower_break(&mut self) -> Result<BlockEnd, CodegenError> {
        let frame = self.loops.last().copied().ok_or_else(|| {
            CodegenError::Unsupported("`break` outside a loop".to_string())
        })?;
        self.builder
            .build_unconditional_branch(frame.break_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(BlockEnd::Terminated)
    }

    fn lower_continue(&mut self) -> Result<BlockEnd, CodegenError> {
        let frame = self.loops.last().copied().ok_or_else(|| {
            CodegenError::Unsupported(
                "`continue` outside a loop".to_string(),
            )
        })?;
        self.builder
            .build_unconditional_branch(frame.continue_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(BlockEnd::Terminated)
    }

    fn alloca_for(
        &self,
        ty: LotusType,
        name: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        match ty {
            LotusType::Int | LotusType::Duration => self
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
            Expr::Literal(Literal::Duration(ns), _) => {
                // Duration literals are i64 nanoseconds at the
                // lowered level; tracked as Duration so callers
                // like `time::sleep` enforce the typed contract.
                let v = self.context.i64_type().const_int(*ns as u64, true);
                Ok((v.into(), LotusType::Duration))
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
            ParamValue::Duration(ns) => (
                self.context.i64_type().const_int(*ns as u64, true).into(),
                LotusType::Duration,
            ),
        }
    }

    fn llvm_basic_type(
        &self,
        t: LotusType,
    ) -> inkwell::types::BasicTypeEnum<'ctx> {
        match t {
            LotusType::Int | LotusType::Duration => {
                self.context.i64_type().into()
            }
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
                LotusType::Duration => {
                    // Match the interpreter's `<ns>ns` rendering so
                    // both paths produce identical stdout.
                    format.push_str("%lldns");
                    printf_args.push(BasicMetadataValueEnum::IntValue(
                        val.into_int_value(),
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

    /// Dispatch a `path::ident(...)` call. Currently only
    /// `time::sleep` is recognized; other namespaced calls land in
    /// the stdlib lowering when those features arrive.
    fn lower_path_call(
        &mut self,
        qn: &QualifiedName,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        let segs: Vec<&str> =
            qn.segments.iter().map(|s| s.name.as_str()).collect();
        match segs.as_slice() {
            ["time", "sleep"] => {
                self.lower_time_sleep(args, scope, &BTreeMap::new())
            }
            _ => Err(CodegenError::Unsupported(format!(
                "path call `{}`",
                segs.join("::")
            ))),
        }
    }

    /// Lower `time::sleep(duration)` to a monotonic-clock,
    /// EINTR-retrying `clock_nanosleep` call. The lowered IR is:
    ///
    /// ```text
    ///   sec = ns / 1_000_000_000
    ///   nsec = ns % 1_000_000_000
    ///   req.tv_sec  = sec
    ///   req.tv_nsec = nsec
    ///   while clock_nanosleep(CLOCK_MONOTONIC, 0, &req, &rem) == EINTR {
    ///       req = rem;   // resume from the remaining time
    ///   }
    /// ```
    ///
    /// `CLOCK_MONOTONIC` is hardcoded to 1 (Linux); flags = 0 means
    /// the request is relative (`TIMER_ABSTIME` would make it a
    /// deadline). Any non-EINTR error exits the loop best-effort —
    /// we don't crash the program over a clock failure.
    fn lower_time_sleep(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        self_state: &BTreeMap<String, ParamValue>,
    ) -> Result<(), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "time::sleep takes 1 argument, got {}",
                args.len()
            )));
        }
        let (val, ty) = self.lower_expr(&args[0], scope, self_state)?;
        if ty != LotusType::Duration {
            return Err(CodegenError::Unsupported(format!(
                "time::sleep expects Duration, got {:?}",
                ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let ts_t = self.timespec_type();
        let ns = val.into_int_value();
        let billion = i64_t.const_int(1_000_000_000, false);

        let sec = self
            .builder
            .build_int_signed_div(ns, billion, "ts.sec")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let nsec = self
            .builder
            .build_int_signed_rem(ns, billion, "ts.nsec")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let req = self
            .builder
            .build_alloca(ts_t, "req")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let rem = self
            .builder
            .build_alloca(ts_t, "rem")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let req_sec_ptr = self
            .builder
            .build_struct_gep(ts_t, req, 0, "req.sec.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let req_nsec_ptr = self
            .builder
            .build_struct_gep(ts_t, req, 1, "req.nsec.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(req_sec_ptr, sec)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(req_nsec_ptr, nsec)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let func = self
            .current_fn
            .expect("current_fn set while lowering time::sleep");
        let loop_bb = self.context.append_basic_block(func, "sleep.loop");
        let retry_bb = self.context.append_basic_block(func, "sleep.retry");
        let done_bb = self.context.append_basic_block(func, "sleep.done");

        self.builder
            .build_unconditional_branch(loop_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // loop_bb: call clock_nanosleep, branch on EINTR vs done
        self.builder.position_at_end(loop_bb);
        let cns = self
            .module
            .get_function("clock_nanosleep")
            .expect("clock_nanosleep declared");
        // CLOCK_MONOTONIC = 1, flags = 0
        let clock_id = i32_t.const_int(1, false);
        let flags = i32_t.const_int(0, false);
        let call_result = self
            .builder
            .build_call(
                cns,
                &[
                    clock_id.into(),
                    flags.into(),
                    req.into(),
                    rem.into(),
                ],
                "cns.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_int = call_result
            .try_as_basic_value()
            .left()
            .expect("clock_nanosleep returns i32")
            .into_int_value();
        // EINTR == 4 on Linux. Everything else (including success=0)
        // exits the loop.
        let eintr = i32_t.const_int(4, false);
        let is_eintr = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, ret_int, eintr, "is.eintr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(is_eintr, retry_bb, done_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // retry_bb: copy rem → req, jump back into the loop
        self.builder.position_at_end(retry_bb);
        let rem_sec_ptr = self
            .builder
            .build_struct_gep(ts_t, rem, 0, "rem.sec.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let rem_nsec_ptr = self
            .builder
            .build_struct_gep(ts_t, rem, 1, "rem.nsec.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let rem_sec = self
            .builder
            .build_load(i64_t, rem_sec_ptr, "rem.sec")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let rem_nsec = self
            .builder
            .build_load(i64_t, rem_nsec_ptr, "rem.nsec")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(req_sec_ptr, rem_sec)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(req_nsec_ptr, rem_nsec)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(loop_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(done_bb);
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
    Duration(i64),
}

fn param_value(e: &Expr) -> Result<ParamValue, CodegenError> {
    match e {
        Expr::Literal(Literal::String(s), _) => Ok(ParamValue::String(s.clone())),
        Expr::Literal(Literal::Int(n), _) => Ok(ParamValue::Int(*n)),
        Expr::Literal(Literal::Float(f), _) => Ok(ParamValue::Float(*f)),
        Expr::Literal(Literal::Bool(b), _) => Ok(ParamValue::Bool(*b)),
        Expr::Literal(Literal::Duration(ns), _) => Ok(ParamValue::Duration(*ns)),
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
