//! AST → LLVM IR → object file → executable, for the
//! milestone-0 subset.

use std::path::{Path, PathBuf};
use std::process::Command;

use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
    TargetTriple,
};
use inkwell::values::{BasicMetadataValueEnum, FunctionValue, PointerValue};
use inkwell::{AddressSpace, OptimizationLevel};

use lotus_syntax::ast::*;

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

        // Lower the body. The only statement shape currently
        // handled at top level is locus instantiation as an
        // expression statement: `LocusName { ... };`.
        for stmt in &main_decl.body.stmts {
            self.lower_stmt(stmt, main_fn)?;
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
        _fn_val: FunctionValue<'ctx>,
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
            Stmt::Expr(_) => Err(CodegenError::Unsupported(
                "expression statement other than locus literal".to_string(),
            )),
            _ => Err(CodegenError::Unsupported(format!(
                "statement form {:?}",
                std::mem::discriminant(stmt)
            ))),
        }
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
        state: &std::collections::BTreeMap<String, ParamValue>,
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
                if name != "println" && name != "print" {
                    return Err(CodegenError::Unsupported(format!(
                        "builtin `{}`",
                        name
                    )));
                }
                // Compose a single printf format + arg list.
                // Each argument contributes a format fragment
                // (with `%` escaped in literal text) and
                // optionally a value to plug in.
                let mut format = String::new();
                let mut printf_args: Vec<BasicMetadataValueEnum> =
                    Vec::with_capacity(args.len() + 1);
                // Reserve the format-string slot at index 0.
                printf_args.push(BasicMetadataValueEnum::PointerValue(
                    self.context.ptr_type(AddressSpace::default()).const_null(),
                ));
                for a in args {
                    let (fmt, value) = self.resolve_arg(a, state)?;
                    format.push_str(&fmt);
                    if let Some(v) = value {
                        printf_args.push(v);
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

    /// Lower one println argument. Returns:
    /// - `String`: the format-string fragment to splice in
    ///   (literal text with `%` escaped, or a printf format
    ///   specifier like `%s` / `%lld` / `%g`).
    /// - `Option<BasicMetadataValueEnum>`: the LLVM value to
    ///   pass to printf, if the fragment was a specifier;
    ///   `None` if the argument was a literal text fragment.
    fn resolve_arg(
        &mut self,
        e: &Expr,
        state: &std::collections::BTreeMap<String, ParamValue>,
    ) -> Result<(String, Option<BasicMetadataValueEnum<'ctx>>), CodegenError> {
        match e {
            Expr::Literal(Literal::String(s), _) => Ok((escape_format(s), None)),
            Expr::Literal(Literal::Int(n), _) => {
                let v = self.context.i64_type().const_int(*n as u64, true);
                Ok(("%lld".to_string(), Some(BasicMetadataValueEnum::IntValue(v))))
            }
            Expr::Literal(Literal::Float(f), _) => {
                let v = self.context.f64_type().const_float(*f);
                Ok(("%g".to_string(), Some(BasicMetadataValueEnum::FloatValue(v))))
            }
            Expr::Literal(Literal::Bool(b), _) => {
                let s = if *b { "true" } else { "false" };
                Ok((s.to_string(), None))
            }
            Expr::Field { receiver, name, .. } if matches!(receiver.as_ref(), Expr::KwSelf(_)) => {
                let value = state.get(&name.name).ok_or_else(|| {
                    CodegenError::Unsupported(format!(
                        "self.{}: param not found in compile-time state",
                        name.name
                    ))
                })?;
                match value {
                    ParamValue::String(s) => {
                        let ptr = self.global_string(s);
                        Ok((
                            "%s".to_string(),
                            Some(BasicMetadataValueEnum::PointerValue(ptr)),
                        ))
                    }
                    ParamValue::Int(n) => {
                        let v = self.context.i64_type().const_int(*n as u64, true);
                        Ok((
                            "%lld".to_string(),
                            Some(BasicMetadataValueEnum::IntValue(v)),
                        ))
                    }
                    ParamValue::Float(f) => {
                        let v = self.context.f64_type().const_float(*f);
                        Ok((
                            "%g".to_string(),
                            Some(BasicMetadataValueEnum::FloatValue(v)),
                        ))
                    }
                    ParamValue::Bool(b) => {
                        Ok(((if *b { "true" } else { "false" }).to_string(), None))
                    }
                }
            }
            _ => Err(CodegenError::Unsupported(
                "println argument is neither literal nor self.<param>".to_string(),
            )),
        }
    }
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
