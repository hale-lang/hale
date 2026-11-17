//! Known String lengths (GH #1947, part 2).
//!
//! A `String` is a pointer to NUL-terminated bytes with no stored length
//! (spec/types.md), so `len(s)` is a `strlen`, and so is the clamp of a
//! slice `s[lo..hi]`, of `std::str::substring`, the bound of
//! `std::str::index_of_from` and of `cp_at` / `cp_size`. A loop that
//! measures a String it does not change re-walks the text on every
//! iteration: the byte loop `while i < len(s) { s[i..(i + 1)] … }` is
//! quadratic.
//!
//! A local or parameter of type `String` that its scope measures gets a
//! **length cache** beside its slot: an entry-block `{ ptr, i64 }` holding
//! a value the local held and that value's length. A measure of the local
//! loads the local and compares it to the cached pointer: equal, the
//! cached length is the answer; not, it is a `strlen` and the cache is
//! refilled. So the length is measured once per value the local holds,
//! not once per use.
//!
//! Sound because a String's bytes never change while a name holds it
//! (value semantics), with one exception this module follows: the
//! appending assignment `s = s + …` writes into the local's own buffer
//! and keeps its pointer (`crate::string_append`). Each write of the
//! local updates its cache:
//!
//! - its `let` and a parameter's binding empty it (a `let` in a loop body
//!   writes the same slot on every iteration, and a value of the last
//!   iteration's arena may sit at the same address);
//! - a plain assignment empties it;
//! - an append fills it with the accumulator's length, which the append
//!   already knows.
//!
//! The pointer compare is the second guard: a write of the slot this
//! module did not see stores another pointer, and the next measure
//! misses. No representation changes: a String is still a `char *` to
//! FFI, the bus wire and the runtime, a String reached any other way (a
//! field, an element, a call's result) is measured as before, and the
//! length is `strlen`'s, so an embedded NUL still ends the String.

use inkwell::module::Linkage;
use inkwell::types::StructType;
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::AddressSpace;

use hale_syntax::ast::*;
use hale_types::expr_walk::{visit_expr, visit_stmt, Visited};

use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

fn llvm_err(e: inkwell::builder::BuilderError) -> CodegenError {
    CodegenError::LlvmEmit(e.to_string())
}

/// The internal fn every cached measure calls, inlined at every site.
const LEN_CACHED_FN: &str = "lotus.str_len_cached";

/// The `std::str` functions whose first argument is a String they
/// measure, and that lower to a known-length sibling when it has a cache.
const MEASURING_STD_STR: [&str; 4] = ["substring", "index_of_from", "cp_at", "cp_size"];

/// Whether `e` measures the String named `name`: `len(name)`, a range
/// slice of `name`, or a [`MEASURING_STD_STR`] call with `name` first.
fn measures(e: &Expr, name: &str) -> bool {
    let is_name = |a: &Expr| matches!(a, Expr::Ident(i) if i.name == name);
    match e {
        Expr::Call { callee, args, .. } => {
            let Some(first) = args.first() else { return false };
            if !is_name(first) {
                return false;
            }
            match callee.as_ref() {
                Expr::Ident(i) => i.name == "len",
                Expr::Path(q) => q.segments.last().is_some_and(|s| MEASURING_STD_STR.contains(&s.name.as_str())),
                _ => false,
            }
        }
        Expr::Index { receiver, index, .. } => is_name(receiver) && matches!(index.as_ref(), Expr::Range { .. }),
        _ => false,
    }
}

/// Whether the statements (and tail) measure the String named `name`
/// anywhere, nested blocks included. A shadowing binding of the name is
/// not told apart: a cache nothing reads costs an alloca and a store.
pub(crate) fn measures_local(stmts: &[Stmt], tail: Option<&Expr>, name: &str) -> bool {
    let mut found = false;
    let mut f = |v: Visited<'_>| {
        if let Visited::Expr(e) = v {
            if !found && measures(e, name) {
                found = true;
            }
        }
    };
    for s in stmts {
        visit_stmt(s, &mut f);
    }
    if let Some(t) = tail {
        visit_expr(t, &mut f);
    }
    found
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// The cache's type: `{ ptr value, i64 len }`.
    fn str_len_cache_type(&self) -> StructType<'ctx> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(&[ptr_t.into(), self.context.i64_type().into()], false)
    }

    /// Give the String local in `slot` a length cache (once per slot),
    /// and empty it here, where the local is bound.
    fn bind_str_len_cache(&mut self, slot: PointerValue<'ctx>, name: &str) -> Result<(), CodegenError> {
        let cache = match self.str_len_caches.get(&slot) {
            Some(c) => *c,
            None => {
                let ty = self.str_len_cache_type().into();
                let c = self.alloca_in_entry(ty, &format!("{name}.len.cache"))?;
                self.str_len_caches.insert(slot, c);
                c
            }
        };
        self.empty_str_len_cache(cache)
    }

    fn empty_str_len_cache(&mut self, cache: PointerValue<'ctx>) -> Result<(), CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        self.builder.build_store(cache, ptr_t.const_null()).map_err(llvm_err)?;
        Ok(())
    }

    /// `block.stmts[i]` just lowered: a `let` of a String that the rest of
    /// its block measures gets a length cache, emptied at the `let`.
    pub(crate) fn note_string_len_cache(
        &mut self,
        block: &Block,
        i: usize,
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        let Stmt::Let { name, .. } = &block.stmts[i] else { return Ok(()) };
        let Some((slot, CodegenTy::String)) = scope.locals.get(&name.name).cloned() else { return Ok(()) };
        if self.current_fn.is_none() || !measures_local(&block.stmts[i + 1..], block.tail.as_deref(), &name.name) {
            return Ok(());
        }
        self.bind_str_len_cache(slot, &name.name)
    }

    /// The parameters of a fn just bound into `scope`: each String one
    /// `body` measures gets a length cache, emptied at the fn's entry.
    pub(crate) fn note_param_len_caches(
        &mut self,
        params: &[Param],
        body: &Block,
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        if self.current_fn.is_none() {
            return Ok(());
        }
        for p in params {
            let Some((slot, CodegenTy::String)) = scope.locals.get(&p.name.name).cloned() else { continue };
            if measures_local(&body.stmts, body.tail.as_deref(), &p.name.name) {
                self.bind_str_len_cache(slot, &p.name.name)?;
            }
        }
        Ok(())
    }

    /// The local in `slot` was just assigned a value whose length is not
    /// known here: its cache, if it has one, is emptied.
    pub(crate) fn forget_str_len(&mut self, slot: PointerValue<'ctx>) -> Result<(), CodegenError> {
        match self.str_len_caches.get(&slot).copied() {
            Some(cache) => self.empty_str_len_cache(cache),
            None => Ok(()),
        }
    }

    /// The local in `slot` was just assigned `value`, of length `len`.
    pub(crate) fn know_str_len(
        &mut self,
        slot: PointerValue<'ctx>,
        value: BasicValueEnum<'ctx>,
        len: IntValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let Some(cache) = self.str_len_caches.get(&slot).copied() else { return Ok(()) };
        let ty = self.str_len_cache_type();
        self.builder.build_store(cache, value).map_err(llvm_err)?;
        let len_ptr = self.builder.build_struct_gep(ty, cache, 1, "str.len.cache.len").map_err(llvm_err)?;
        self.builder.build_store(len_ptr, len).map_err(llvm_err)?;
        Ok(())
    }

    /// Whether the slot in `slot` has a length cache.
    pub(crate) fn has_str_len_cache(&self, slot: PointerValue<'ctx>) -> bool {
        self.str_len_caches.contains_key(&slot)
    }

    /// The length cache of the String `e` names, when `e` is a local or
    /// parameter that has one.
    pub(crate) fn str_len_cache_of(&self, e: &Expr, scope: &Scope<'ctx>) -> Option<PointerValue<'ctx>> {
        let Expr::Ident(id) = e else { return None };
        let (slot, ty) = scope.locals.get(&id.name)?;
        if *ty != CodegenTy::String {
            return None;
        }
        self.str_len_caches.get(slot).copied()
    }

    /// The length of `s`, a value just loaded from the local whose cache
    /// is `cache`: the cached length when the cache holds `s`, otherwise
    /// `lotus_str_len(s)`, which then fills the cache.
    pub(crate) fn emit_cached_str_len(
        &mut self,
        s: PointerValue<'ctx>,
        cache: PointerValue<'ctx>,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let f = self.str_len_cached_fn();
        let n = self
            .builder
            .build_call(f, &[s.into(), cache.into()], "str.len")
            .map_err(llvm_err)?
            .try_as_basic_value()
            .left()
            .expect("lotus.str_len_cached returns i64")
            .into_int_value();
        Ok(n)
    }

    /// The internal fn behind [`Self::emit_cached_str_len`], defined once
    /// per module. It is a few instructions and marked `inlinehint`, so the
    /// inliner puts it at each call and the cache (an alloca no call then
    /// takes) is promoted to registers. Not `alwaysinline`: that mark is
    /// seam fusion's alone (GH #1530, `inline_new_hop_calls`), and
    /// `seam_fusion.rs` holds a program with no seam to carrying none.
    ///
    /// ```text
    /// define internal i64 @lotus.str_len_cached(ptr %s, ptr %c) inlinehint {
    ///   if (%s == null) return 0;
    ///   if (%c.value == %s) return %c.len;
    ///   %n = lotus_str_len(%s); %c = { %s, %n }; return %n;
    /// }
    /// ```
    ///
    /// A NULL `s` (only an `@ffi` fn can return one) is 0 long: the
    /// known-length siblings this answers for (`lotus_str_range_copy`,
    /// `lotus_str_range_index_of`, `lotus_str_cp_*_n`) treat it as empty,
    /// as the functions they replace did, and the cache's empty state
    /// (a NULL value) is never a hit.
    fn str_len_cached_fn(&mut self) -> FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function(LEN_CACHED_FN) {
            return f;
        }
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i64_t = self.context.i64_type();
        let f = self.module.add_function(
            LEN_CACHED_FN,
            i64_t.fn_type(&[ptr_t.into(), ptr_t.into()], false),
            Some(Linkage::Internal),
        );
        for attr in ["inlinehint", "nounwind"] {
            let kind = inkwell::attributes::Attribute::get_named_enum_kind_id(attr);
            f.add_attribute(inkwell::attributes::AttributeLoc::Function, self.context.create_enum_attribute(kind, 0));
        }
        let ty = self.str_len_cache_type();
        let strlen = self.module.get_function("lotus_str_len").expect("lotus_str_len declared");
        // A builder of its own, which never had a debug location: none of
        // the fn being lowered reaches this one, and the fn has no
        // subprogram (so no `di_begin_function`, which resets the shared
        // builder's location for a program fn's entry block).
        let b = self.context.create_builder();
        let entry = self.context.append_basic_block(f, "start");
        let null = self.context.append_basic_block(f, "null");
        let check = self.context.append_basic_block(f, "check");
        let hit = self.context.append_basic_block(f, "hit");
        let miss = self.context.append_basic_block(f, "miss");
        let s = f.get_nth_param(0).expect("s").into_pointer_value();
        let c = f.get_nth_param(1).expect("cache").into_pointer_value();
        let emit = || -> Result<(), inkwell::builder::BuilderError> {
            b.position_at_end(entry);
            let is_null = b.build_is_null(s, "s.null")?;
            b.build_conditional_branch(is_null, null, check)?;
            b.position_at_end(null);
            b.build_return(Some(&i64_t.const_zero()))?;
            b.position_at_end(check);
            let held = b.build_load(ptr_t, c, "cache.value")?.into_pointer_value();
            let same = b.build_int_compare(inkwell::IntPredicate::EQ, held, s, "cache.hit")?;
            b.build_conditional_branch(same, hit, miss)?;
            b.position_at_end(hit);
            let len_ptr = b.build_struct_gep(ty, c, 1, "cache.len.ptr")?;
            let n = b.build_load(i64_t, len_ptr, "cache.len")?;
            b.build_return(Some(&n))?;
            b.position_at_end(miss);
            let n = b
                .build_call(strlen, &[s.into()], "len")?
                .try_as_basic_value()
                .left()
                .expect("lotus_str_len returns i64");
            b.build_store(c, s)?;
            let len_ptr = b.build_struct_gep(ty, c, 1, "cache.len.ptr")?;
            b.build_store(len_ptr, n)?;
            b.build_return(Some(&n))?;
            Ok(())
        };
        emit().expect("lotus.str_len_cached emits");
        f
    }
}
