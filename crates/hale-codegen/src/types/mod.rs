//! Type-system codegen: type-expression lowering, user type / enum
//! declaration, generic monomorphization, F.30 view coercions, F.20
//! interface dispatch. Round 5 of the codegen model-org refactor.
//!
//! These lift as inherent `impl<'ctx, 'p> Cx<'ctx, 'p>` blocks rather
//! than a trait extension — inherent impls merge across files in
//! Rust, so no `use` import is needed at call sites.

use std::collections::BTreeMap;

use hale_syntax::ast::{
    Expr, Ident, Literal, PrimType, QualifiedName, StructInit, TopDecl,
    TypeDecl, TypeDeclBody, TypeExpr,
};
use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::AddressSpace;

use crate::codegen::{
    codegen_ty_size_bytes, literal_to_view_coerces, CodegenError,
    CodegenTy, Cx, EnumInfo, EnumVariantInfo, Scope, TypeInfo,
};

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// Map a `TypeExpr` to the codegen's `CodegenTy`. Scalar
    /// primitives + bare locus type names are supported; arrays /
    /// tuples / generics wait.
    pub(crate) fn type_expr_to_codegen_ty(
        &self,
        t: &TypeExpr,
    ) -> Result<CodegenTy, CodegenError> {
        match t {
            TypeExpr::Primitive(p, _) => match p {
                PrimType::Int => Ok(CodegenTy::Int),
                PrimType::Float => Ok(CodegenTy::Float),
                PrimType::Bool => Ok(CodegenTy::Bool),
                PrimType::String => Ok(CodegenTy::String),
                PrimType::Duration => Ok(CodegenTy::Duration),
                PrimType::Decimal => Ok(CodegenTy::Decimal),
                PrimType::Time => Ok(CodegenTy::Time),
                PrimType::Bytes => Ok(CodegenTy::Bytes),
                PrimType::BytesView => Ok(CodegenTy::BytesView),
                PrimType::StringView => Ok(CodegenTy::StringView),
                other => Err(CodegenError::Unsupported(format!(
                    "type primitive `{:?}` in signature",
                    other
                ))),
            },
            TypeExpr::Named { path, generic_args, .. }
                if generic_args.is_empty() && path.segments.len() == 1 =>
            {
                let name = &path.segments[0].name;
                if self.user_loci.contains_key(name)
                    || self.pending_locus_names.contains(name)
                {
                    // B10: `pending_locus_names` lets a locus that
                    // hasn't been fully declared yet still resolve
                    // as a LocusRef target. The opaque struct body
                    // gets populated when the referenced locus's
                    // own `declare_locus_struct` runs later in the
                    // same pass.
                    Ok(CodegenTy::LocusRef(name.clone()))
                } else if self.user_enums.contains_key(name) {
                    // Enums resolve to Enum(name) — BEFORE the
                    // user_types/pending_type_names branch. An enum
                    // name is also inserted into `pending_type_names`
                    // by the forward-ref pre-pass, so if that branch
                    // ran first a declared enum used in a type
                    // annotation / param / return / match-scrutinee
                    // would mis-resolve to TypeRef (generic
                    // record-by-pointer), while enum *construction*
                    // yields Enum — the representation mismatch made
                    // the enum machinery unreachable from annotated
                    // values (no-payload print, payload match, etc.).
                    // user_enums is populated by the time lowering
                    // resolves these, so checking it first is the fix.
                    Ok(CodegenTy::Enum(name.clone()))
                } else if self.user_types.contains_key(name)
                    || self.pending_type_names.contains(name)
                {
                    // iris F.10 (2026-05-24): consult
                    // `pending_type_names` as a forward-ref
                    // fallback for single-segment named paths.
                    // Pre-fix this branch only checked
                    // `user_types`, so a struct field referencing
                    // a sibling type declared later in
                    // `self.program.items` order errored with
                    // "unknown type name in signature". The
                    // multi-segment branch below already did
                    // this; bringing the single-segment branch
                    // in line closes the case where
                    // `apply_qualified_path_renames` collapses
                    // `lib::Name` to a single mangled segment
                    // that points at a not-yet-registered
                    // imported-lib type decl.
                    Ok(CodegenTy::TypeRef(name.clone()))
                } else if self.user_interfaces.contains(name) {
                    // F.20 Phase B: interface type in signature
                    // position. Lowered as a fat pointer (data +
                    // vtable); coercion from a concrete locus is
                    // built at the call site, dispatch through
                    // the vtable is emitted at the method-call site.
                    Ok(CodegenTy::Interface(name.clone()))
                } else if name == "LocusRef" || name == "TypeRef" {
                    // B16 / G14: `LocusRef` / `TypeRef` are
                    // codegen-internal kinds, not user-spellable
                    // types. Spell the locus or type by name
                    // directly — there's no separate "reference"
                    // type in v1.
                    Err(CodegenError::Unsupported(format!(
                        "type `{}` is not user-spellable in v1; \
                         spell the locus or type by name directly \
                         (e.g. `params {{ x: MyLocus; }}` for a \
                         borrowed locus reference)",
                        name
                    )))
                } else {
                    Err(CodegenError::Unsupported(format!(
                        "unknown type name `{}` in signature",
                        name
                    )))
                }
            }
            // m83: path-qualified stdlib type — `std::io::tcp::Stream`
            // in a fn signature / locus param. Same path → mangled
            // name table the struct-literal lowering uses; resolves
            // to the bundled-stdlib locus's LocusRef. Without this,
            // `fn(std::io::tcp::Stream)` would parse but fail in
            // type_expr_to_codegen_ty when building the FnPtr's arg
            // tys. Generic-arg paths over stdlib types are not
            // a v0 concern (no generic stdlib loci yet).
            TypeExpr::Named { path, generic_args, .. }
                if generic_args.is_empty() && path.segments.len() > 1 =>
            {
                let segs: Vec<&str> = path
                    .segments
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect();
                let mangled_owned = self.mangled_for_path(&segs).ok_or_else(|| {
                    CodegenError::Unsupported(format!(
                        "qualified type `{}` not in stdlib path-renames table",
                        segs.join("::")
                    ))
                })?;
                let mangled: &str = &mangled_owned;
                if self.user_loci.contains_key(mangled)
                    || self.pending_locus_names.contains(mangled)
                {
                    Ok(CodegenTy::LocusRef(mangled.to_string()))
                } else if self.user_enums.contains_key(mangled) {
                    // Path-qualified enum (e.g. a cross-seed
                    // `lib::Result`). Checked before the
                    // user_types/pending_type_names branch for the
                    // same reason as the single-segment case — enum
                    // names also land in pending_type_names, and an
                    // enum must resolve to Enum(name) so its value
                    // representation matches construction.
                    Ok(CodegenTy::Enum(mangled.to_string()))
                } else if self.user_types.contains_key(mangled)
                    || self.pending_type_names.contains(mangled)
                {
                    // m84: path-qualified stdlib `type` records.
                    // `std::http::Request` in a fn signature
                    // resolves to TypeRef("__StdHttpRequest").
                    // B8: pending_type_names lets cross-decl
                    // references resolve regardless of source
                    // order (user `type Ctx { req: std::http::
                    // Request; }` works even when the stdlib
                    // type is declared after the user type).
                    Ok(CodegenTy::TypeRef(mangled.to_string()))
                } else if self.user_interfaces.contains(mangled) {
                    // F.20 Phase B + Sink-migration follow-up:
                    // path-qualified stdlib interface — e.g.
                    // `std::text::Sink` in a fn signature resolves
                    // to Interface("__StdTextSink"). Mirrors the
                    // unqualified-name branch above; the lookup
                    // table maps the user-facing path to the
                    // mangled interface name and codegen treats it
                    // as a fat-pointer-typed slot.
                    Ok(CodegenTy::Interface(mangled.to_string()))
                } else {
                    Err(CodegenError::Unsupported(format!(
                        "qualified type `{}` (mangled `{}`) declared in stdlib \
                         path-renames table but not registered in user_loci, \
                         user_types, or user_interfaces yet — sequencing issue: \
                         type_expr_to_codegen_ty called before pass A0/A1 \
                         populated this name",
                        segs.join("::"),
                        mangled,
                    )))
                }
            }
            // m61: generic instantiation in type position. Mangle
            // (template, args) to a flat name and look it up; the
            // monomorphization pass in lower_program (A0b) will
            // have synthesized + declared the mangled struct
            // before any concrete decl that references it tries
            // to resolve. If lookup fails, the generic was used
            // somewhere the discovery pass missed (m61b territory)
            // — error clearly.
            TypeExpr::Named { path, generic_args, .. }
                if !generic_args.is_empty() && path.segments.len() == 1 =>
            {
                let mangled = Self::mangle_generic_name(
                    &path.segments[0].name,
                    generic_args,
                )?;
                if self.user_types.contains_key(&mangled) {
                    Ok(CodegenTy::TypeRef(mangled))
                } else if self.user_enums.contains_key(&mangled) {
                    Ok(CodegenTy::Enum(mangled))
                } else if self.user_loci.contains_key(&mangled) {
                    // m63: generic locus instantiation —
                    // resolves to a LocusRef pointing at the
                    // synthesized concrete locus.
                    Ok(CodegenTy::LocusRef(mangled))
                } else {
                    Err(CodegenError::Unsupported(format!(
                        "generic instantiation `{}` not synthesized — \
                         discovery missed the use site; reachable from \
                         {:?}",
                        mangled, path.segments[0].name
                    )))
                }
            }
            TypeExpr::Array { elem, size, .. } => {
                let elem_ty = self.type_expr_to_codegen_ty(elem)?;
                let n = match size {
                    Some(Expr::Literal(Literal::Int(n), _)) if *n > 0 => *n as u64,
                    Some(_) => {
                        return Err(CodegenError::Unsupported(
                            "array size must be a positive integer literal in v0"
                                .into(),
                        ));
                    }
                    None => {
                        return Err(CodegenError::Unsupported(
                            "unsized arrays not supported in v0; use [T; N]".into(),
                        ));
                    }
                };
                Ok(CodegenTy::Array(Box::new(elem_ty), n))
            }
            TypeExpr::Tuple(parts, _) => {
                if parts.len() < 2 {
                    return Err(CodegenError::Unsupported(format!(
                        "tuple type must have at least 2 elements; got {}",
                        parts.len()
                    )));
                }
                let mut elem_tys = Vec::with_capacity(parts.len());
                for p in parts {
                    elem_tys.push(self.type_expr_to_codegen_ty(p)?);
                }
                Ok(CodegenTy::Tuple(elem_tys))
            }
            TypeExpr::Function { params, ret, .. } => {
                let mut arg_tys = Vec::with_capacity(params.len());
                for p in params {
                    arg_tys.push(self.type_expr_to_codegen_ty(p)?);
                }
                let ret_ty = match ret {
                    Some(r) => Some(Box::new(self.type_expr_to_codegen_ty(r)?)),
                    None => None,
                };
                Ok(CodegenTy::FnPtr {
                    args: arg_tys,
                    ret: ret_ty,
                })
            }
            other => Err(CodegenError::Unsupported(format!(
                "type form {:?} in signature",
                std::mem::discriminant(other)
            ))),
        }
    }

    /// Pass A0: declare a user `type` decl as an LLVM struct type.
    /// Aliases and enums are not yet lowered — only struct bodies.
    /// No defaults are expected (the language requires struct
    /// literals to provide every field at the call site).
    pub(crate) fn declare_user_type(&mut self, t: &TypeDecl) -> Result<(), CodegenError> {
        if !t.generics.is_empty() {
            // m61: generic templates are not lowered directly. The
            // monomorphization pass (lower_program A0b) walks every
            // generic-arg use site, synthesizes a concrete TypeDecl
            // per (template, args) tuple with the generic params
            // substituted, and calls declare_user_type on the
            // synthesized non-generic decl. Skipping here lets the
            // template's source declaration coexist with its
            // synthesized instantiations without producing a
            // template-shaped LLVM struct (which would have no
            // sensible field types because T isn't a real type).
            return Ok(());
        }
        let struct_fields = match &t.body {
            TypeDeclBody::Struct(fs) => fs,
            TypeDeclBody::Alias(_) => {
                return Err(CodegenError::Unsupported(format!(
                    "type alias `{}`: codegen v0 only lowers struct types",
                    t.name.name
                )));
            }
            TypeDeclBody::Enum(variants) => {
                // m47 + payloads: register the enum's variants
                // and compute the storage layout. Each variant's
                // payload field types resolve via type_expr_to_codegen_ty
                // (so nested struct / enum types are valid). For
                // payload-bearing variants we measure the per-
                // variant payload size (sum of field sizes,
                // 8-byte-aligned per field) and pick the maximum
                // as the byte-array body size in the unified
                // enum storage struct.
                let mut variant_infos: Vec<EnumVariantInfo> = Vec::new();
                let mut has_payload = false;
                let mut max_bytes: u64 = 0;
                for v in variants {
                    let mut field_tys: Vec<CodegenTy> = Vec::new();
                    let mut bytes: u64 = 0;
                    for f in &v.fields {
                        let lt = self.type_expr_to_codegen_ty(f)?;
                        bytes += codegen_ty_size_bytes(self.context, &lt);
                        field_tys.push(lt);
                    }
                    if !field_tys.is_empty() {
                        has_payload = true;
                    }
                    if bytes > max_bytes {
                        max_bytes = bytes;
                    }
                    variant_infos.push(EnumVariantInfo {
                        name: v.name.name.clone(),
                        field_tys,
                    });
                }
                self.user_enums.insert(
                    t.name.name.clone(),
                    EnumInfo {
                        variants: variant_infos,
                        has_payload,
                        payload_bytes: max_bytes,
                    },
                );
                return Ok(());
            }
        };

        let mut fields: BTreeMap<String, (u32, CodegenTy)> = BTreeMap::new();
        let mut field_order: Vec<String> = Vec::new();
        let mut defaults: BTreeMap<String, Expr> = BTreeMap::new();
        let mut llvm_field_tys: Vec<inkwell::types::BasicTypeEnum> =
            Vec::new();
        for (idx, f) in struct_fields.iter().enumerate() {
            let ft = self.type_expr_to_codegen_ty(&f.ty)?;
            llvm_field_tys.push(self.llvm_basic_type(&ft));
            fields.insert(f.name.name.clone(), (idx as u32, ft));
            field_order.push(f.name.name.clone());
            if let Some(d) = &f.default {
                defaults.insert(f.name.name.clone(), d.clone());
            }
        }
        let struct_ty = self
            .context
            .opaque_struct_type(&format!("type.{}", t.name.name));
        struct_ty.set_body(&llvm_field_tys, false);

        self.user_types.insert(
            t.name.name.clone(),
            TypeInfo {
                struct_ty,
                fields,
                field_order,
                defaults,
            },
        );
        Ok(())
    }

    /// m61b: when an `Expr::Struct { path: bare, ... }` is being
    /// lowered against an expected generic-instantiation type
    /// (e.g., `Box<Int>` from a let ascription or fn param), and
    /// `bare` matches the template name, return a rewritten path
    /// pointing at the mangled monomorph (`Box_Int`) so the rest
    /// of struct-literal lowering finds it in `user_types`.
    /// Returns `None` when no rewrite applies — caller falls back
    /// to the original path. Idempotent: if `bare` is already a
    /// mangled name in `user_types`, the rewrite is skipped.
    /// m67: like resolve_generic_struct_path but takes a target
    /// CodegenTy (the declared type at the use site — return slot
    /// or struct field) instead of a TypeExpr ascription. Used at
    /// return statements (where the target is the fn's declared
    /// return CodegenTy) and struct field initializers (where the
    /// target is the field's declared CodegenTy). The caller has
    /// already converted the source-position TypeExpr through
    /// `type_expr_to_codegen_ty`, so we get the mangled name directly.
    pub(crate) fn resolve_generic_struct_path_for_codegen_ty(
        &self,
        bare: &QualifiedName,
        target: &CodegenTy,
    ) -> Option<QualifiedName> {
        if bare.segments.len() != 1 {
            return None;
        }
        let bare_name = &bare.segments[0].name;
        // Already concrete — leave it alone.
        if self.user_types.contains_key(bare_name)
            || self.user_loci.contains_key(bare_name)
        {
            return None;
        }
        let target_name = match target {
            CodegenTy::TypeRef(n) => n,
            CodegenTy::LocusRef(n) => n,
            CodegenTy::Enum(n) => n,
            _ => return None,
        };
        // The target must be a mangled monomorph of the bare name:
        // it has to start with `<bare>_` and exist in user_types or
        // user_loci. The underscore separator is what mangle_name
        // emits between template name and each arg.
        let prefix = format!("{}_", bare_name);
        if !target_name.starts_with(&prefix) {
            return None;
        }
        if !self.user_types.contains_key(target_name)
            && !self.user_loci.contains_key(target_name)
        {
            return None;
        }
        Some(QualifiedName {
            segments: vec![Ident {
                name: target_name.clone(),
                span: bare.segments[0].span,
            }],
            span: bare.span,
        })
    }

    pub(crate) fn resolve_generic_struct_path(
        &self,
        bare: &QualifiedName,
        expected: &TypeExpr,
    ) -> Option<QualifiedName> {
        if bare.segments.len() != 1 {
            return None;
        }
        let bare_name = &bare.segments[0].name;
        // Already concrete (mangled or not) — leave it alone.
        // m63: also accept user_loci as concrete since generic
        // locus instantiations land there too.
        if self.user_types.contains_key(bare_name)
            || self.user_loci.contains_key(bare_name)
        {
            return None;
        }
        let (ty_name, generic_args) = match expected {
            TypeExpr::Named { path, generic_args, .. }
                if path.segments.len() == 1
                    && !generic_args.is_empty() =>
            {
                (&path.segments[0].name, generic_args)
            }
            _ => return None,
        };
        if bare_name != ty_name {
            return None;
        }
        let mangled =
            Self::mangle_generic_name(bare_name, generic_args).ok()?;
        // m63: extend lookup to user_loci so generic locus
        // instantiations resolve via let-ascription bare-name
        // construction the same way generic structs do.
        if !self.user_types.contains_key(&mangled)
            && !self.user_loci.contains_key(&mangled)
        {
            return None;
        }
        Some(QualifiedName {
            segments: vec![Ident {
                name: mangled,
                span: bare.segments[0].span,
            }],
            span: bare.span,
        })
    }

    /// F.30b (5b) (2026-05-20): wrap a String / Bytes literal
    /// value in a view struct for storage-site default coercion.
    /// Only fires when the initializer is a String / Bytes
    /// literal — those live in the global string table at
    /// program-lifetime, so the view's `builder` field is NULL
    /// (lotus_*_view_data skips the epoch check on that branch).
    /// The unpack at the eventual read site is a one-load
    /// pass-through to the literal's static pointer.
    pub(crate) fn wrap_literal_as_view(
        &mut self,
        value: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let f = self
            .module
            .get_function("lotus_view_from_static_data")
            .expect("lotus_view_from_static_data declared");
        let call = self
            .builder
            .build_call(f, &[value.into()], "view.from_static")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // F.30b view-ABI compaction: helper returns the 16-byte
        // view struct by value; no arena allocation.
        Ok(call
            .try_as_basic_value()
            .left()
            .expect("lotus_view_from_static_data returns view struct"))
    }

    /// F.30b: unpack a view's underlying data pointer for a
    /// read-position consumer. For a `BytesView` value calls
    /// `lotus_bytes_view_data` (returns the Bytes-shaped data ptr,
    /// recomputed as `b->buf - 8`); for `StringView` calls
    /// `lotus_str_view_data` (returns the NUL-terminated C-string
    /// `b->buf`). Both helpers compare the view's stamped epoch
    /// against the source builder's live mutation_epoch and panic
    /// on mismatch — catching "builder mutated between view() and
    /// read" misuse at the read site.
    /// Non-view types pass through unchanged. Call this at every
    /// codegen site where a value of declared type "could be a
    /// view or its non-view sibling" is about to flow into a C
    /// primitive that expects the non-view layout.
    pub(crate) fn unpack_view_if_needed(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &CodegenTy,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let helper_name = match ty {
            CodegenTy::BytesView => "lotus_bytes_view_data",
            CodegenTy::StringView => "lotus_str_view_data",
            _ => return Ok(value),
        };
        let f = self
            .module
            .get_function(helper_name)
            .unwrap_or_else(|| panic!("{} declared", helper_name));
        // F.30b view-ABI compaction: `value` is the view struct
        // passed by value. The helper checks the staleness epoch
        // (when the static sentinel isn't set) and returns the
        // underlying data ptr.
        let call = self
            .builder
            .build_call(f, &[value.into()], "view.unpack")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(call
            .try_as_basic_value()
            .left()
            .expect("view-unpack helper returns ptr"))
    }

    pub(crate) fn llvm_basic_type(
        &self,
        t: &CodegenTy,
    ) -> inkwell::types::BasicTypeEnum<'ctx> {
        match t {
            CodegenTy::Int | CodegenTy::Duration => {
                self.context.i64_type().into()
            }
            CodegenTy::FnPtr { .. } => {
                // m80: function pointers store as raw `ptr`. The
                // FunctionType for indirect calls is synthesized
                // from the FnPtr's args/ret at the call site, not
                // baked into the storage layout.
                self.context.ptr_type(AddressSpace::default()).into()
            }
            CodegenTy::Float => self.context.f64_type().into(),
            CodegenTy::Decimal => {
                // m48: Decimal is an i128 mantissa with implicit
                // scale 9 (mantissa × 10^-9). Distinct from Float
                // at the type level so type-checking stays strict
                // AND distinct at the LLVM level so arithmetic
                // goes through integer ops with the scale-9
                // adjustment in mul/div, not f64.
                self.context.i128_type().into()
            }
            CodegenTy::Bool => self.context.bool_type().into(),
            CodegenTy::Enum(name) => {
                // m47 + payloads: no-payload enums stay as i32
                // tags (value semantics, no allocation). Once a
                // variant carries a payload, the whole enum
                // becomes a pointer to the unified storage
                // struct `{ i32 tag, [N x i8] body }`. The
                // representation switch is per-enum, not
                // per-variant, so the LLVM type is uniform
                // across construction / pattern-match / arg-pass
                // sites for any given enum.
                match self.user_enums.get(name) {
                    Some(info) if info.has_payload => self
                        .context
                        .ptr_type(AddressSpace::default())
                        .into(),
                    _ => self.context.i32_type().into(),
                }
            }
            CodegenTy::BytesView | CodegenTy::StringView => {
                // F.30b view ABI: views are 16-byte by-value
                // structs `{void *src, int64_t epoch}`. The same
                // type is used at field/storage sites and as the
                // return type of view-producing helpers.
                self.view_struct_ty().into()
            }
            CodegenTy::String
            | CodegenTy::Bytes
            | CodegenTy::Time
            | CodegenTy::LocusRef(_)
            | CodegenTy::TypeRef(_)
            | CodegenTy::Array(_, _)
            | CodegenTy::Tuple(_)
            | CodegenTy::Interface(_)
            | CodegenTy::Cell(_, _) => {
                self.context.ptr_type(AddressSpace::default()).into()
            }
        }
    }

    /// LLVM `[N x T]` for the element type + size of an Array
    /// CodegenTy. Used at array-literal allocation time + at GEP
    /// time when indexing or iterating.
    pub(crate) fn llvm_array_storage_type(
        &self,
        elem: &CodegenTy,
        n: u64,
    ) -> inkwell::types::ArrayType<'ctx> {
        match self.llvm_basic_type(elem) {
            inkwell::types::BasicTypeEnum::IntType(t) => t.array_type(n as u32),
            inkwell::types::BasicTypeEnum::FloatType(t) => t.array_type(n as u32),
            inkwell::types::BasicTypeEnum::PointerType(t) => t.array_type(n as u32),
            inkwell::types::BasicTypeEnum::StructType(t) => t.array_type(n as u32),
            inkwell::types::BasicTypeEnum::ArrayType(t) => t.array_type(n as u32),
            inkwell::types::BasicTypeEnum::VectorType(t) => t.array_type(n as u32),
        }
    }

    /// Shared field-initialization pass for a user-type struct
    /// literal. Writes each field's evaluated init expression into
    /// the matching slot of `dest_ptr`. Used by:
    ///   - `lower_user_type_instantiation` (arena-allocates storage),
    ///   - `lower_send`'s ephemeral-payload fast path (stack-allocs
    ///     storage in the entry block).
    ///
    /// Field-level deep-copy is OFF by default. Downstream consumers
    /// that need arena-anchored fields (hashmap.set, vec.push,
    /// locus-field self.X store, free-fn return epilogue) run their
    /// own `emit_cross_arena_store_deep_copy` /
    /// `anchor_struct_fields_in_place` against the populated struct;
    /// pushing the deep-copy down to this layer defeats the
    /// downstream's same-arena skip (the `e = store.get(); store.
    /// set(MetricEntry { key: e.key, ... })` pattern from the pond
    /// metrics workload — measured as MetricMap chunk growth in
    /// the residency dump when the deep-copy was unconditional).
    ///
    /// One exception: when `current_arena_override` is active —
    /// which only happens during the Stage 1 return-arena routing
    /// in `lower_return` for a method-with-scratch returning an
    /// aggregate — the outer struct is being placed in caller_arena
    /// rather than the method's scratch, and the boundary deep-copy
    /// will be skipped by the same-arena check. If a heap field
    /// initializer aliases a value in some OTHER arena (the calling
    /// method's scratch, self.__arena, etc.) the stored pointer
    /// would dangle after that arena's destroy. Under override we
    /// emit `emit_cross_arena_store_deep_copy_ptr` per heap field;
    /// the helper's same-arena skip makes it a no-op when the field
    /// is already in caller_arena (typical case — sub-literals also
    /// lowered under the same override), and falls through to
    /// recursive deep-copy when it's not (the alias-safe path for
    /// literals like `BookSignalSnapshot { buys: let_bound_value }`).
    pub(crate) fn populate_user_type_fields(
        &mut self,
        type_name: &str,
        info: &TypeInfo<'ctx>,
        inits: &[StructInit],
        dest_ptr: PointerValue<'ctx>,
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        let by_name: BTreeMap<&str, &Expr> = inits
            .iter()
            .map(|i| (i.name.name.as_str(), &i.value))
            .collect();
        // 2026-05-16 — fields with declared defaults may be omitted
        // from the literal; the default expression evaluates in the
        // caller's scope at instantiation time. Mirrors the
        // locus-param default behavior.
        for fname in &info.field_order {
            if !by_name.contains_key(fname.as_str())
                && !info.defaults.contains_key(fname.as_str())
            {
                return Err(CodegenError::Unsupported(format!(
                    "type `{}` literal missing field `{}`",
                    type_name, fname
                )));
            }
        }
        for fname in &info.field_order {
            let expr: &Expr = match by_name.get(fname.as_str()).copied() {
                Some(e) => e,
                None => info
                    .defaults
                    .get(fname.as_str())
                    .expect("default presence checked above"),
            };
            let (idx, declared_ty) = info
                .fields
                .get(fname)
                .cloned()
                .expect("field declared by declare_user_type");
            // m67: rewrite a bare-name struct literal in field-
            // init position to its mangled monomorph using the
            // field's declared CodegenTy as the target.
            let rewritten;
            let expr_to_lower: &Expr = match expr {
                Expr::Struct { path, inits, span } => {
                    match self
                        .resolve_generic_struct_path_for_codegen_ty(
                            path,
                            &declared_ty,
                        )
                    {
                        Some(new_path) => {
                            rewritten = Expr::Struct {
                                path: new_path,
                                inits: inits.clone(),
                                span: *span,
                            };
                            &rewritten
                        }
                        None => expr,
                    }
                }
                _ => expr,
            };
            let (val, val_ty) = self.lower_expr(expr_to_lower, scope)?;
            // B13 / G30: F.23 Int → Float widening in user-type
            // field-init position. Matches the call-site Int→Float
            // coercion in `lower_user_fn_call`.
            let (val, val_ty) =
                if declared_ty == CodegenTy::Float && val_ty == CodegenTy::Int {
                    let w = self.coerce_to_float(
                        val,
                        &val_ty,
                        &format!("type `{}` field `{}`", type_name, fname),
                    )?;
                    (w.into(), CodegenTy::Float)
                } else {
                    (val, val_ty)
                };
            // 2026-05-18 — locus → interface coercion at user-type
            // field-init. Mirrors the same coercion in
            // `lower_locus_literal_init` (locus-literal field-init,
            // ~line 27616) and `lower_user_fn_call` (call-site arg).
            // Without this, `type T { f: Iface; }` would refuse a
            // concrete locus literal even though the typechecker
            // accepts it via `check_structural_impl` (~check.rs 2895).
            let (val, val_ty) = if let (
                CodegenTy::Interface(iface),
                CodegenTy::LocusRef(l),
            ) = (&declared_ty, &val_ty)
            {
                let fat = self.coerce_to_interface(
                    val.into_pointer_value(),
                    l,
                    iface,
                )?;
                (fat.into(), declared_ty.clone())
            } else if literal_to_view_coerces(
                &val_ty,
                &declared_ty,
                expr_to_lower,
            ) {
                // F.30b (5b): wrap a String/Bytes literal in a
                // view struct so a `text: StringView = ""` or
                // `body: BytesView = b""` field declaration
                // type-checks. The wrapped view carries the
                // static-epoch sentinel; lotus_*_view_data sees
                // it and returns `src` directly (the underlying
                // data is program-lifetime — no epoch check).
                let wrapped = self.wrap_literal_as_view(val)?;
                (wrapped, declared_ty.clone())
            } else {
                (val, val_ty)
            };
            if val_ty != declared_ty {
                return Err(CodegenError::Unsupported(format!(
                    "type `{}` field `{}` type mismatch: declared {:?}, \
                     got {:?}",
                    type_name, fname, declared_ty, val_ty
                )));
            }
            // Conditional field-level deep-copy: only when override
            // is active (Stage 1 return-arena routing). Routes each
            // heap field through the same-arena skip wrapper so
            // alias initializers anchor in override arena while
            // already-anchored or fresh-allocated-under-override
            // fields pass through unchanged. See fn doc for the
            // rationale on why this is OFF by default.
            let val = if let Some(arena) = self.current_arena_override {
                let needs_copy = val.is_pointer_value()
                    && matches!(
                        &declared_ty,
                        CodegenTy::String
                            | CodegenTy::Bytes
                            | CodegenTy::TypeRef(_)
                            | CodegenTy::Tuple(_)
                            | CodegenTy::Array(_, _)
                            | CodegenTy::Enum(_)
                    );
                if needs_copy {
                    self.emit_cross_arena_store_deep_copy_ptr(
                        val,
                        &declared_ty,
                        arena,
                        &format!("{}.{}.fieldinit", type_name, fname),
                    )?
                } else {
                    val
                }
            } else {
                val
            };
            let field_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    dest_ptr,
                    idx,
                    &format!("{}.{}.ptr", type_name, fname),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(field_ptr, val)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        Ok(())
    }

    /// m47-payloads: storage struct for a payload-bearing enum:
    /// `{ i32 tag, [N x i8] body }` where N is `payload_bytes`.
    /// The struct is the same for every variant of the enum;
    /// per-variant payloads sit inside the `body` byte array,
    /// re-interpreted by the variant's field types at access
    /// time. Caller must check `has_payload` before calling.
    pub(crate) fn enum_storage_struct(
        &self,
        info: &EnumInfo,
    ) -> inkwell::types::StructType<'ctx> {
        let i32_t = self.context.i32_type();
        let body_t = self
            .context
            .i8_type()
            .array_type(info.payload_bytes as u32);
        self.context
            .struct_type(&[i32_t.into(), body_t.into()], false)
    }

    /// G20 / F.20 Phase B follow-up: does `locus_name` cover every
    /// method declared by `iface_name` by name? Method-name-only
    /// (signature compatibility is the typechecker's job). Used by
    /// the m90 return-routing extension to decide whether a fresh
    /// locus instantiation inside an `-> Interface(I)` fn body
    /// could plausibly be the returned value and therefore needs
    /// program-lifetime allocation.
    pub(crate) fn locus_satisfies_interface(
        &self,
        locus_name: &str,
        iface_name: &str,
    ) -> bool {
        let iface_methods: Vec<&str> = match self
            .program
            .items
            .iter()
            .find_map(|item| match item {
                TopDecl::Interface(i) if i.name.name == iface_name => {
                    Some(i.methods.iter().map(|m| m.name.name.as_str()).collect())
                }
                _ => None,
            }) {
            Some(v) => v,
            None => return false,
        };
        let info = match self.user_loci.get(locus_name) {
            Some(i) => i,
            None => return false,
        };
        iface_methods
            .iter()
            .all(|m| info.user_methods.contains_key(*m))
    }

    /// F.20 Phase B: build a fat-pointer interface value from a
    /// concrete locus pointer. Allocates a 16-byte `{data, vtable}`
    /// struct in the current arena, stores the locus pointer at
    /// slot 0 and the per-(locus, iface) vtable global address at
    /// slot 1, and returns a pointer to the struct. The returned
    /// pointer is the LLVM-level representation of a value whose
    /// CodegenTy is `Interface(iface_name)`.
    pub(crate) fn coerce_to_interface(
        &mut self,
        locus_val: PointerValue<'ctx>,
        locus_name: &str,
        iface_name: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let vtable = self.ensure_vtable(locus_name, iface_name)?;
        let i64_t = self.context.i64_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let fat_struct_ty = self
            .context
            .struct_type(&[ptr_t.into(), ptr_t.into()], false);
        let size_val = i64_t.const_int(16, false);
        let fat_ptr =
            self.arena_alloc(size_val, &format!("iface.{}.fat", iface_name))?;
        let data_slot = self
            .builder
            .build_struct_gep(fat_struct_ty, fat_ptr, 0, "iface.data.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(data_slot, locus_val)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let vtable_slot = self
            .builder
            .build_struct_gep(fat_struct_ty, fat_ptr, 1, "iface.vtable.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(vtable_slot, vtable.as_pointer_value())
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(fat_ptr)
    }

    /// F.30b view-ABI compaction: 16-byte view value passed/returned
    /// by value. `{void *src, int64_t epoch}` — `src` is either the
    /// builder pointer (real view) or the static data pointer (when
    /// epoch == LOTUS_VIEW_EPOCH_STATIC = -1, the static sentinel).
    /// Layout matches `lotus_view_t` in lotus_arena.c; SysV AMD64
    /// returns both eightbytes in `rax`/`rdx`. Same struct shape is
    /// used for BytesView and StringView — the C-side helpers
    /// (`lotus_bytes_view_data` / `lotus_str_view_data`) read the
    /// Bytes-shape (`buf - 8`) or C-string shape (`buf`) at unpack
    /// time, so the codegen-visible type is uniform.
    pub(crate) fn view_struct_ty(&self) -> inkwell::types::StructType<'ctx> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i64_t = self.context.i64_type();
        self.context.struct_type(&[ptr_t.into(), i64_t.into()], false)
    }

    /// Bus-arena reclaim (2026-05-21): heap types where a
    /// `self.X = expr` store from inside a method body must
    /// deep-copy `expr` into `self.__arena` so the stored pointer
    /// outlives the method-scratch destroy. Scalars / views /
    /// LocusRefs are pass-through (views alias their source;
    /// loci use their own arena routing). Enum is conservative
    /// — we let `emit_return_value_deep_copy` decide based on
    /// payload presence.
    pub(crate) fn ty_needs_self_field_deep_copy(ty: &CodegenTy) -> bool {
        match ty {
            CodegenTy::Int
            | CodegenTy::Float
            | CodegenTy::Bool
            | CodegenTy::Decimal
            | CodegenTy::Time
            | CodegenTy::Duration
            | CodegenTy::FnPtr { .. }
            | CodegenTy::LocusRef(_)
            | CodegenTy::BytesView
            | CodegenTy::StringView
            | CodegenTy::Cell(_, _) => false,
            CodegenTy::String
            | CodegenTy::Bytes
            | CodegenTy::TypeRef(_)
            | CodegenTy::Tuple(_)
            | CodegenTy::Array(_, _)
            | CodegenTy::Interface(_)
            | CodegenTy::Enum(_) => true,
        }
    }

}
