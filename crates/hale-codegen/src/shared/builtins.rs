//! C-runtime + libc + lotus extern declarations. Round 7a of the
//! codegen model-org refactor.
//!
//! Lifted as an inherent `impl<'ctx, 'p> Cx<'ctx, 'p>` block on Cx —
//! call sites need no `use` import. The body is one giant
//! `module.add_function(...)` per primitive; the whole thing runs
//! at start of Pass A.

use inkwell::AddressSpace;

use crate::codegen::{Cx, SOCKOPT_NAMES};

impl<'ctx, 'p> Cx<'ctx, 'p> {
    pub(crate) fn declare_builtins(&self) {
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

        // declare i32 @clock_gettime(i32, ptr)
        //
        // Backing primitive for `time::monotonic()` (and, when it
        // lands, `time::now()`). Same MONOTONIC vs REALTIME
        // discipline as `clock_nanosleep`.
        let clock_gettime_ty =
            i32_t.fn_type(&[i32_t.into(), ptr_t.into()], false);
        self.module
            .add_function("clock_gettime", clock_gettime_ty, None);

        // declare i32 @strcmp(ptr, ptr)
        //
        // Used by `@bus_dispatch` to match subscription subjects
        // against the publish subject. Subjects are NUL-terminated
        // global strings so the standard libc primitive applies.
        let strcmp_ty = i32_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module.add_function("strcmp", strcmp_ty, None);

        // declare i32 @dprintf(i32 fd, ptr fmt, ...)
        //
        // POSIX libc; lets the closure-violation report go to fd 2
        // (stderr) without needing a `stderr` global. Variadic.
        let dprintf_ty =
            i32_t.fn_type(&[i32_t.into(), ptr_t.into()], true);
        self.module.add_function("dprintf", dprintf_ty, None);

        // declare void @exit(i32) noreturn
        //
        // Used by closure-violation handler to abort with a non-zero
        // exit status when an unabsorbed closure fail at dissolve.
        // Mirrors the interpreter's "runtime error: ClosureViolation"
        // path, which exits non-zero too.
        let void_t = self.context.void_type();
        let exit_ty = void_t.fn_type(&[i32_t.into()], false);
        let exit_fn = self.module.add_function("exit", exit_ty, None);
        // No `noreturn` attr in inkwell stable; the unreachable we
        // emit after the call is enough for LLVM to optimize.
        let _ = exit_fn;

        // declare ptr @lotus_arena_create()
        // declare ptr @lotus_arena_alloc(ptr arena, i64 size, i64 align)
        // declare void @lotus_arena_destroy(ptr arena)
        //
        // The lotus region allocator (v0 substrate). Replaces libc
        // malloc as the backing store for type literals (bus
        // payloads, composite locus param defaults) and synthesized
        // ClosureViolation records. v0 wires a single program-wide
        // arena initialized at the top of main and destroyed at
        // exit; m20 will refine to per-locus arenas matching
        // spec/memory.md "A locus owns a region."
        //
        // Backed by libc malloc internally — the C source for the
        // arena lives in `runtime/lotus_arena.c` and is compiled +
        // linked alongside the generated object file. From LLVM IR
        // we just see the C-ABI surface.
        let i64_t = self.context.i64_type();
        // Target-pointer-width int for C `size_t` / `ssize_t` params and
        // returns — i64 native, i32 wasm32. Any lotus_* runtime symbol whose
        // C signature uses `size_t`/`ssize_t` MUST declare that slot with
        // this, or the wasm `call` signature mismatches the definition and
        // traps (the bus-codec / @form-collection bug class, 69925dc et al).
        // `int64_t`/`uint64_t` params stay i64; `int` stays i32.
        let usize_t = self.usize_type();
        let arena_create_ty = ptr_t.fn_type(&[], false);
        let arena_create_fn = self
            .module
            .add_function("lotus_arena_create", arena_create_ty, None);
        // declare ptr @lotus_arena_create_labeled(ptr label)
        // 2026-05-22 PM: codegen-side entry point that stashes a
        // human-readable label on the residency registry entry
        // (LOTUS_ARENA_RESIDENCY=1 dump). Called at locus
        // instantiation with the locus name as a global string
        // literal so the residency snapshot identifies which
        // locus owns which arena without needing to resolve
        // construction backtraces.
        let arena_create_labeled_ty =
            ptr_t.fn_type(&[ptr_t.into()], false);
        let arena_create_labeled_fn = self.module.add_function(
            "lotus_arena_create_labeled",
            arena_create_labeled_ty,
            None,
        );
        // F.32-3 (2026-05-25): sized variant. Emitted at the
        // Fresh-strategy locus instantiation when the locus is
        // placed on a non-`main` cooperative pool — codegen
        // computes a per-pool chunk-size hint from the loci-per-
        // pool count and passes it as the second arg. C side
        // clamps to [4096, env-default] and rounds; out-of-range
        // hints fall back to the env default silently.
        // `initial_chunk_bytes` is size_t (i32 wasm32).
        let arena_create_labeled_sized_ty =
            ptr_t.fn_type(&[ptr_t.into(), usize_t.into()], false);
        let arena_create_labeled_sized_fn = self.module.add_function(
            "lotus_arena_create_labeled_sized",
            arena_create_labeled_sized_ty,
            None,
        );
        // m22: chunked-class parent calls this when accepting a
        // child; the child arena registers a slot index in the
        // parent so destroy can free-list it for reuse.
        let subregion_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        let subregion_fn = self
            .module
            .add_function("lotus_arena_create_subregion", subregion_ty, None);
        let arena_alloc_ty =
            ptr_t.fn_type(&[ptr_t.into(), i64_t.into(), i64_t.into()], false);
        let arena_alloc_fn = self
            .module
            .add_function("lotus_arena_alloc", arena_alloc_ty, None);
        // 2026-07-01 aliasing metadata, stage 1. The allocator family
        // returns FRESH memory — no other live pointer refers to the
        // returned block (bump allocation never re-hands live bytes;
        // chunk-pool and child-struct-freelist reuse only recycles
        // blocks whose previous owners are dead, same contract as
        // malloc reusing freed memory). `noalias` on the return is
        // the same guarantee C's malloc carries, and it lets LLVM
        // treat every allocation as a distinct object: store-to-load
        // forwarding and dead-store elimination across struct-literal
        // init sequences, payload copies, and sret writes stop being
        // blocked by "might alias the destination". The C runtime
        // never unwinds (`nounwind`) and these entry points always
        // return (`willreturn` — the cap path returns NULL rather
        // than aborting). No `memory(...)` mask: the diagnostic env
        // paths (residency logging, cap dprintf) read/write freely,
        // so the default conservative mask stays.
        {
            use inkwell::attributes::{Attribute, AttributeLoc};
            let noalias_kind = Attribute::get_named_enum_kind_id("noalias");
            let nounwind_kind = Attribute::get_named_enum_kind_id("nounwind");
            let willreturn_kind =
                Attribute::get_named_enum_kind_id("willreturn");
            let noalias = self.context.create_enum_attribute(noalias_kind, 0);
            let nounwind =
                self.context.create_enum_attribute(nounwind_kind, 0);
            let willreturn =
                self.context.create_enum_attribute(willreturn_kind, 0);
            for f in [
                arena_create_fn,
                arena_create_labeled_fn,
                arena_create_labeled_sized_fn,
                subregion_fn,
                arena_alloc_fn,
            ] {
                f.add_attribute(AttributeLoc::Return, noalias);
                f.add_attribute(AttributeLoc::Function, nounwind);
                f.add_attribute(AttributeLoc::Function, willreturn);
            }
        }
        let arena_destroy_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_arena_destroy", arena_destroy_ty, None);
        // declare i32 @lotus_arena_contains_ptr(ptr arena, ptr p)
        // Returns 1 if `p` is inside one of `arena`'s chunks. Used
        // by the codegen's same-arena skip at cross-arena store
        // boundaries (hashmap.set / vec.set / vec.push /
        // ring_buffer.push) to pass-through values that already
        // live in the destination arena — the dominant cost on the
        // read-modify-write pattern (get an entry, mutate locally,
        // put it back) where the value's heap fields are already
        // anchored in the receiver locus's __arena.
        let arena_contains_ty =
            self.context.i32_type()
                .fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module.add_function(
            "lotus_arena_contains_ptr",
            arena_contains_ty,
            None,
        );

        // v1.x-3: recognition projection class — bitmap-tracked
        // fixed_cell pool and bump-allocated shared_slab pool.
        // Each acquire returns a `lotus_arena_t*` so child body
        // code stays projection-class-agnostic per F.22; release
        // routes through the matching pool primitive instead of
        // `lotus_arena_destroy`.
        //
        // declare ptr  @lotus_recpool_fixed_create(i64 cap, i64 bytes)
        // declare ptr  @lotus_recpool_fixed_acquire(ptr pool)
        // declare void @lotus_recpool_fixed_release(ptr pool, ptr arena)
        // declare void @lotus_recpool_fixed_destroy(ptr pool)
        // declare ptr  @lotus_recpool_slab_create(i64 cap, i64 bytes)
        // declare ptr  @lotus_recpool_slab_acquire(ptr pool)
        // declare void @lotus_recpool_slab_release(ptr pool, ptr arena)
        // declare void @lotus_recpool_slab_destroy(ptr pool)
        // recpool_*_create(size_t cap_count, size_t cell_bytes/slab_bytes).
        let recpool_create_ty =
            ptr_t.fn_type(&[usize_t.into(), usize_t.into()], false);
        let recpool_acquire_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        let recpool_release_ty =
            void_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        let recpool_destroy_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_recpool_fixed_create", recpool_create_ty, None);
        self.module
            .add_function("lotus_recpool_fixed_acquire", recpool_acquire_ty, None);
        self.module
            .add_function("lotus_recpool_fixed_release", recpool_release_ty, None);
        self.module
            .add_function("lotus_recpool_fixed_destroy", recpool_destroy_ty, None);
        self.module
            .add_function("lotus_recpool_slab_create", recpool_create_ty, None);
        self.module
            .add_function("lotus_recpool_slab_acquire", recpool_acquire_ty, None);
        self.module
            .add_function("lotus_recpool_slab_release", recpool_release_ty, None);
        self.module
            .add_function("lotus_recpool_slab_destroy", recpool_destroy_ty, None);

        // v1.x-FORM-2 PR6: root-locus value-error panic. Called
        // when an `or raise` propagates past every enclosing
        // fallible(E) frame — the value error has escaped the
        // implicit main locus's body. Today the runtime fn
        // dprintf+exit(1)s; architecturally it's the seat for a
        // future routing-through-main-locus-on_failure extension,
        // hence the typename arg + opaque payload ptr/size.
        let root_panic_ty = void_t.fn_type(
            &[ptr_t.into(), i64_t.into(), ptr_t.into()],
            false,
        );
        self.module
            .add_function("lotus_root_panic", root_panic_ty, None);

        // F.22 capacity-tuple substrate primitives.
        //
        // declare ptr  @lotus_pool_create(i64 cell_size, i64 cell_align)
        // declare ptr  @lotus_pool_acquire(ptr pool)
        // declare void @lotus_pool_release(ptr pool, ptr cell)
        // declare void @lotus_pool_destroy(ptr pool)
        // declare ptr  @lotus_heap_create(i64 cell_size, i64 cell_align)
        // declare ptr  @lotus_heap_alloc(ptr heap)
        // declare void @lotus_heap_free(ptr heap, ptr cell)
        // declare void @lotus_heap_destroy(ptr heap)
        //
        // Pool of T: fixed-size cell recycling. acquire returns a
        // cell pointer; release puts it back on the free-list.
        // Heap of T: individually-freed cells; destroy frees all
        // still-live cells wholesale. Both type-erased at the C
        // ABI — codegen passes cell_size and cell_align (both `size_t`,
        // hence usize_t — i32 wasm32) at create time, computed from T's
        // struct layout.
        let pool_create_ty =
            ptr_t.fn_type(&[usize_t.into(), usize_t.into()], false);
        self.module
            .add_function("lotus_pool_create", pool_create_ty, None);
        let pool_acquire_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_pool_acquire", pool_acquire_ty, None);
        let pool_release_ty =
            void_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_pool_release", pool_release_ty, None);
        let pool_destroy_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_pool_destroy", pool_destroy_ty, None);
        let heap_create_ty =
            ptr_t.fn_type(&[usize_t.into(), usize_t.into()], false);
        self.module
            .add_function("lotus_heap_create", heap_create_ty, None);
        let heap_alloc_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_heap_alloc", heap_alloc_ty, None);
        let heap_free_ty =
            void_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_heap_free", heap_free_ty, None);
        let heap_destroy_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_heap_destroy", heap_destroy_ty, None);

        // v1.x-FORM-2: @form(vec) C runtime primitives. The vec lives
        // inline as { i64 cap, i64 len, ptr buf } in the locus struct;
        // lotus_vec_* operate on a pointer to that field.
        //
        // declare void  @lotus_vec_init(ptr vec)
        // declare void  @lotus_vec_push(ptr vec, i64 elem_size, ptr elem)
        // declare i32   @lotus_vec_get(ptr vec, i64 elem_size, i64 i, ptr out)
        // declare i32   @lotus_vec_pop(ptr vec, i64 elem_size, ptr out)
        // declare i64   @lotus_vec_len(ptr vec)
        // declare i32   @lotus_vec_is_empty(ptr vec)
        // declare void  @lotus_vec_destroy(ptr vec)
        //
        // get / pop / is_empty return i32 (1 = OK, 0 = err) at the C
        // boundary. The fallible methods (get, pop) invert this at
        // codegen time to match Hale's i1 (true = err) ABI.
        let i32_t = self.context.i32_type();
        // The collection primitives' `size_t` params (elem_size / key_size
        // / value_size / fixed_cap) use `usize_t` (declared above); call
        // sites narrow the size value with `size_to_usize` (no-op native).
        let vec_init_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_vec_init", vec_init_ty, None);
        let vec_push_ty =
            void_t.fn_type(&[ptr_t.into(), usize_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_vec_push", vec_push_ty, None);
        let vec_get_ty = i32_t.fn_type(
            &[ptr_t.into(), usize_t.into(), i64_t.into(), ptr_t.into()],
            false,
        );
        self.module.add_function("lotus_vec_get", vec_get_ty, None);
        // declare i32 @lotus_vec_set(ptr vec, size_t elem_size, i64 i, ptr elem)
        // Same C ABI as lotus_vec_get, opposite direction: caller
        // hands a pointer to the new element. Returns 1=OK, 0=OOB.
        let vec_set_ty = i32_t.fn_type(
            &[ptr_t.into(), usize_t.into(), i64_t.into(), ptr_t.into()],
            false,
        );
        self.module.add_function("lotus_vec_set", vec_set_ty, None);
        let vec_pop_ty =
            i32_t.fn_type(&[ptr_t.into(), usize_t.into(), ptr_t.into()], false);
        self.module.add_function("lotus_vec_pop", vec_pop_ty, None);
        let vec_len_ty = i64_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function("lotus_vec_len", vec_len_ty, None);
        let vec_is_empty_ty = i32_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_vec_is_empty", vec_is_empty_ty, None);
        let vec_destroy_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_vec_destroy", vec_destroy_ty, None);
        // declare void @lotus_vec_sort_int(ptr vec)
        // declare void @lotus_vec_sort_float(ptr vec)
        // declare void @lotus_vec_sort_string(ptr vec)
        // declare void @lotus_vec_sort_by(ptr vec, i64 elem_size,
        //                                  ptr trampoline_fn, ptr cookie)
        // Primitive sorts use qsort with typed comparators baked
        // into the C runtime; sort_by takes a per-T trampoline
        // synthesized at codegen + a cookie carrying the user's
        // comparator fn-pointer + caller arena + reverse flag.
        let vec_sort_prim_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_vec_sort_int", vec_sort_prim_ty, None);
        self.module
            .add_function("lotus_vec_sort_float", vec_sort_prim_ty, None);
        self.module
            .add_function("lotus_vec_sort_string", vec_sort_prim_ty, None);
        let vec_sort_by_ty = void_t.fn_type(
            &[ptr_t.into(), usize_t.into(), ptr_t.into(), ptr_t.into()],
            false,
        );
        self.module
            .add_function("lotus_vec_sort_by", vec_sort_by_ty, None);

        // v1.x-FORM-4: @form(hashmap) C runtime primitives. The
        // hashmap lives inline as { i64 cap, i64 len, i64 key_size,
        // i64 value_size, i32 key_type_tag, ptr slots } in the
        // locus struct; lotus_hashmap_* operate on a pointer to
        // that field. key_size / value_size / key_type_tag are
        // baked in at init time (per-call sites pass raw key/value
        // pointers without per-call size args, unlike vec).
        //
        // declare void    @lotus_hashmap_init(ptr m, i64 key_size, i64 value_size, i32 key_type_tag)
        // declare void    @lotus_hashmap_set(ptr m, ptr key, ptr value)
        // declare i32     @lotus_hashmap_get(ptr m, ptr key, ptr out_value)
        // declare i32     @lotus_hashmap_has(ptr m, ptr key)
        // declare i32     @lotus_hashmap_remove(ptr m, ptr key)
        // declare i64     @lotus_hashmap_len(ptr m)
        // declare i32     @lotus_hashmap_is_empty(ptr m)
        // declare void    @lotus_hashmap_destroy(ptr m)
        //
        // get / has / remove / is_empty return i32 (1 = OK / true,
        // 0 = missing / false) at the C boundary. The fallible
        // methods (get, remove) invert this at codegen time to
        // match Hale's i1 (true = err) ABI.
        let hashmap_init_ty = void_t.fn_type(
            &[ptr_t.into(), usize_t.into(), usize_t.into(), i32_t.into()],
            false,
        );
        self.module
            .add_function("lotus_hashmap_init", hashmap_init_ty, None);
        // F.32-1α (2026-05-24): sync = serialized variant.
        // Same signature as the plain init; differs only in
        // that it pthread_mutex_init's the per-map mutex.
        self.module
            .add_function("lotus_hashmap_init_serialized", hashmap_init_ty, None);
        // F.32-1β2 (2026-05-25): sync = striped variant. Cell
        // stride is cache-padded; a per-map pthread_rwlock_t
        // gates grow exclusion; entry points use cell-level
        // CAS for slot claim.
        self.module
            .add_function("lotus_hashmap_init_striped", hashmap_init_ty, None);
        // F.32-1γ-v1 (2026-05-25): sync = lockfree variant.
        // Takes an extra `fixed_cap` i64 arg (user-declared
        // via `cap = N` on the form annotation; no grow).
        let hashmap_init_lockfree_ty = void_t.fn_type(
            &[ptr_t.into(), usize_t.into(), usize_t.into(), i32_t.into(), usize_t.into()],
            false,
        );
        self.module
            .add_function("lotus_hashmap_init_lockfree", hashmap_init_lockfree_ty, None);
        let hashmap_set_ty =
            void_t.fn_type(&[ptr_t.into(), ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_hashmap_set", hashmap_set_ty, None);
        let hashmap_get_ty =
            i32_t.fn_type(&[ptr_t.into(), ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_hashmap_get", hashmap_get_ty, None);
        let hashmap_has_ty =
            i32_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_hashmap_has", hashmap_has_ty, None);
        let hashmap_remove_ty =
            i32_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_hashmap_remove", hashmap_remove_ty, None);
        let hashmap_len_ty = i64_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_hashmap_len", hashmap_len_ty, None);
        let hashmap_is_empty_ty = i32_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_hashmap_is_empty", hashmap_is_empty_ty, None);
        let hashmap_destroy_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_hashmap_destroy", hashmap_destroy_ty, None);
        // declare i32 @lotus_hashmap_key_at(ptr m, i64 i, ptr out_key)
        // declare i32 @lotus_hashmap_value_at(ptr m, i64 i, ptr out_value)
        // Hash-table-order iteration (added 2026-05-16). Same i32
        // return shape as get/remove — 1=OK, 0=out-of-range — so
        // codegen wraps in the standard fallible(IndexError) shape.
        let hashmap_key_at_ty =
            i32_t.fn_type(&[ptr_t.into(), i64_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_hashmap_key_at", hashmap_key_at_ty, None);
        let hashmap_value_at_ty =
            i32_t.fn_type(&[ptr_t.into(), i64_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_hashmap_value_at", hashmap_value_at_ty, None);
        // declare void @lotus_text_tokenize_words_into(ptr target_vec, ptr src, ptr arena, i32 lowercase)
        let tokenize_words_ty = void_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), ptr_t.into(), i32_t.into()],
            false,
        );
        self.module
            .add_function("lotus_text_tokenize_words_into", tokenize_words_ty, None);

        // @form(ring_buffer): fixed-capacity FIFO. The `cap` is
        // baked in at init from the form annotation arg; methods
        // operate on the same inline struct pattern as vec /
        // hashmap. push returns i32 (1 = pushed, 0 = full) at the
        // C boundary; pop returns i32 (1 = popped, 0 = empty) and
        // writes the popped element bytes through an out-pointer.
        //
        // declare void @lotus_ring_buffer_init(ptr rb, i64 cap, i64 elem_size)
        // declare i32  @lotus_ring_buffer_push(ptr rb, ptr src)
        // declare i32  @lotus_ring_buffer_pop(ptr rb, ptr out)
        // declare i64  @lotus_ring_buffer_len(ptr rb)
        // declare i32  @lotus_ring_buffer_is_full(ptr rb)
        // declare void @lotus_ring_buffer_destroy(ptr rb)
        // `cap` and `elem_size` are both `size_t` in the C runtime —
        // target-pointer-width (i32 wasm32 / i64 native).
        let rb_init_ty =
            void_t.fn_type(&[ptr_t.into(), usize_t.into(), usize_t.into()], false);
        self.module
            .add_function("lotus_ring_buffer_init", rb_init_ty, None);
        let rb_push_ty = i32_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_ring_buffer_push", rb_push_ty, None);
        let rb_pop_ty = i32_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_ring_buffer_pop", rb_pop_ty, None);
        let rb_len_ty = i64_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_ring_buffer_len", rb_len_ty, None);
        let rb_is_full_ty = i32_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_ring_buffer_is_full", rb_is_full_ty, None);
        let rb_destroy_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_ring_buffer_destroy", rb_destroy_ty, None);

        // m36: string runtime helpers. Each takes a `ptr` for the
        // destination arena (where the result lives) plus the
        // operands; results are NUL-terminated buffers owned by
        // the caller's arena. `lotus_str_eq` returns i32 0/1 we
        // truncate to i1; `lotus_str_len` returns i64 directly.
        // declare ptr @lotus_str_concat(ptr arena, ptr a, ptr b)
        // declare i32 @lotus_str_eq(ptr a, ptr b)
        // declare i64 @lotus_str_len(ptr s)
        // declare ptr @lotus_str_slice(ptr arena, ptr s, i64 lo, i64 hi)
        let str_concat_ty =
            ptr_t.fn_type(&[ptr_t.into(), ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_str_concat", str_concat_ty, None);
        // m49: deep-copy String into the destination arena. Used at
        // free-fn return boundaries — the body's per-call subregion
        // is about to be destroyed, so any String the body returns
        // gets cloned into the caller's arena first.
        // declare ptr @lotus_str_clone(ptr arena, ptr s)
        let str_clone_ty =
            ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_str_clone", str_clone_ty, None);
        // declare ptr @lotus_str_assign_in_place(ptr arena, ptr old, ptr new)
        // declare ptr @lotus_bytes_assign_in_place(ptr arena, ptr old, ptr new)
        // — used at the `self.X = String|Bytes` field-assign site to
        // reuse the old buffer when new fits. Closes the leak
        // class for per-update heap-field reassignment without
        // changing the String/Bytes ABI. See lotus_arena.c for the
        // structural details.
        let str_assign_inplace_ty = ptr_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), ptr_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_str_assign_in_place",
            str_assign_inplace_ty,
            None,
        );
        self.module.add_function(
            "lotus_bytes_assign_in_place",
            str_assign_inplace_ty,
            None,
        );
        // F.30: deep-copy Bytes blob (length-prefixed) into a
        // destination arena. Companion to lotus_str_clone for
        // BytesView → Bytes upgrades via `std::bytes::clone`.
        // declare ptr @lotus_bytes_clone(ptr arena, ptr src)
        let bytes_clone_ty =
            ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_bytes_clone", bytes_clone_ty, None);
        let i32_t_local = self.context.i32_type();
        let str_eq_ty =
            i32_t_local.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module.add_function("lotus_str_eq", str_eq_ty, None);
        let str_len_ty = i64_t.fn_type(&[ptr_t.into()], false);
        let str_len_fn = self
            .module
            .add_function("lotus_str_len", str_len_ty, None);
        let str_slice_ty = ptr_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), i64_t.into(), i64_t.into()],
            false,
        );
        self.module
            .add_function("lotus_str_slice", str_slice_ty, None);

        // m37: to_string runtime helpers. Each renders one
        // primitive into a fresh arena-owned NUL-terminated
        // buffer using the same format println does.
        // declare ptr @lotus_str_from_int(ptr arena, i64 n)
        // declare ptr @lotus_str_from_float(ptr arena, double f)
        // declare ptr @lotus_str_from_duration(ptr arena, i64 ns)
        let f64_t = self.context.f64_type();
        let str_from_int_ty =
            ptr_t.fn_type(&[ptr_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_str_from_int", str_from_int_ty, None);
        let str_from_float_ty =
            ptr_t.fn_type(&[ptr_t.into(), f64_t.into()], false);
        self.module
            .add_function("lotus_str_from_float", str_from_float_ty, None);
        let str_from_dur_ty =
            ptr_t.fn_type(&[ptr_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_str_from_duration", str_from_dur_ty, None);

        // m38: starts_with / contains string predicates.
        // declare i32 @lotus_str_starts_with(ptr s, ptr prefix)
        // declare i32 @lotus_str_contains(ptr s, ptr sub)
        let str_predicate_ty =
            i32_t_local.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module.add_function(
            "lotus_str_starts_with",
            str_predicate_ty,
            None,
        );
        self.module
            .add_function("lotus_str_contains", str_predicate_ty, None);

        // m84: byte index of substring (or -1 if not found).
        // declare i64 @lotus_str_index_of(ptr s, ptr sub)
        let str_index_of_ty =
            i64_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_str_index_of", str_index_of_ty, None);

        // m89: Bytes value primitives.
        // declare ptr @lotus_bytes_create(ptr arena, i64 len)
        // declare i64 @lotus_bytes_len(ptr b)
        // declare ptr @lotus_bytes_data(ptr b)
        let bytes_create_ty =
            ptr_t.fn_type(&[ptr_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_bytes_create", bytes_create_ty, None);
        let bytes_len_ty = i64_t.fn_type(&[ptr_t.into()], false);
        let bytes_len_fn = self
            .module
            .add_function("lotus_bytes_len", bytes_len_ty, None);
        let bytes_data_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        let bytes_data_fn = self
            .module
            .add_function("lotus_bytes_data", bytes_data_ty, None);
        // B2 / G5: bytes-literal helper.
        // declare ptr @lotus_bytes_from_buf(ptr arena, ptr src, i64 len)
        let bytes_from_buf_ty =
            ptr_t.fn_type(&[ptr_t.into(), ptr_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_bytes_from_buf", bytes_from_buf_ty, None);

        // m89: file/socket I/O on Bytes.
        // declare ptr @lotus_fs_read_bytes(ptr arena, ptr path)
        // declare ptr @lotus_fs_read_bytes_global(ptr path)
        // declare i32 @lotus_tcp_send_bytes(i32 fd, ptr bytes)
        let fs_read_bytes_ty =
            ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_fs_read_bytes", fs_read_bytes_ty, None);
        let fs_read_bytes_global_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_fs_read_bytes_global",
            fs_read_bytes_global_ty,
            None,
        );

        let tcp_send_bytes_ty = i32_t_local.fn_type(
            &[self.context.i32_type().into(), ptr_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_tcp_send_bytes",
            tcp_send_bytes_ty,
            None,
        );

        // C4 (pond/crypto follow-up): CSPRNG random bytes via
        // `getrandom(2)` with `/dev/urandom` fallback. Returns a
        // Bytes pointer in the bus payload arena (NULL on error,
        // length-0 blob when caller asks for n <= 0).
        // declare ptr @lotus_os_getrandom(i64 n)
        let os_getrandom_ty = ptr_t.fn_type(&[i64_t.into()], false);
        self.module
            .add_function("lotus_os_getrandom", os_getrandom_ty, None);

        // C2 (pond/subprocess + pond/agent/sandbox): subprocess
        // primitives. `std::process::run` is the sync 80% case;
        // `spawn` / `wait` / `kill` / `pipe_*` are the async
        // lifecycle. All argv inputs are newline-separated Strings
        // (argv[0]\nargv[1]\n...) — Hale's static array surface
        // can't express dynamic command-lines ergonomically; the
        // newline shape mirrors cli.hl's argv_keys convention.
        // declare i32 @lotus_process_run(ptr argv_blob,
        //                                ptr out_code,
        //                                ptr out_signal,
        //                                ptr out_stdout,
        //                                ptr out_stderr)
        let process_run_ty = i32_t.fn_type(
            &[
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
            ],
            false,
        );
        self.module
            .add_function("lotus_process_run", process_run_ty, None);
        // declare i32 @lotus_process_spawn(ptr argv_blob,
        //                                  ptr out_pid,
        //                                  ptr out_stdin_fd,
        //                                  ptr out_stdout_fd,
        //                                  ptr out_stderr_fd)
        let process_spawn_ty = i32_t.fn_type(
            &[
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
            ],
            false,
        );
        self.module
            .add_function("lotus_process_spawn", process_spawn_ty, None);
        // declare i32 @lotus_process_wait(i32 pid,
        //                                 ptr out_code,
        //                                 ptr out_signal)
        let process_wait_ty = i32_t.fn_type(
            &[i32_t.into(), ptr_t.into(), ptr_t.into()],
            false,
        );
        self.module
            .add_function("lotus_process_wait", process_wait_ty, None);
        // declare i32 @lotus_process_kill_escalate(i32 pid)
        let process_kill_ty = i32_t.fn_type(&[i32_t.into()], false);
        self.module.add_function(
            "lotus_process_kill_escalate",
            process_kill_ty,
            None,
        );
        // declare ptr @lotus_process_pipe_read_nonblocking(i32 fd)
        let process_pipe_read_ty = ptr_t.fn_type(&[i32_t.into()], false);
        self.module.add_function(
            "lotus_process_pipe_read_nonblocking",
            process_pipe_read_ty,
            None,
        );
        // declare i64 @lotus_process_pipe_write(i32 fd, ptr str)
        let process_pipe_write_ty =
            i64_t.fn_type(&[i32_t.into(), ptr_t.into()], false);
        self.module.add_function(
            "lotus_process_pipe_write",
            process_pipe_write_ty,
            None,
        );

        // m48: render a Decimal (i128 mantissa, implicit scale 9)
        // into a NUL-terminated string buffer.
        // declare void @lotus_decimal_to_string(i64 hi, i64 lo, ptr buf)
        let dec_to_str_ty = self.context.void_type().fn_type(
            &[i64_t.into(), i64_t.into(), ptr_t.into()],
            false,
        );
        self.module
            .add_function("lotus_decimal_to_string", dec_to_str_ty, None);
        // declare ptr @lotus_str_from_decimal(ptr arena, i64 hi, i64 lo)
        let dec_str_arena_ty =
            ptr_t.fn_type(&[ptr_t.into(), i64_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_str_from_decimal", dec_str_arena_ty, None);
        // declare double @lotus_decimal_to_float(i64 hi, i64 lo)
        // 2026-05-21: direct i128 → f64 conversion at scale 9.
        // Backs std::decimal::to_float(d), killing the ASCII
        // round-trip in downstream hot paths (DecimalFloat.to_float
        // used to format and re-parse).
        let f64_t = self.context.f64_type();
        let dec_to_float_ty =
            f64_t.fn_type(&[i64_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_decimal_to_float", dec_to_float_ty, None);

        // The single program-wide arena pointer. Initialized in
        // the prelude of main; consulted by every arena-allocated
        // user-type literal and ClosureViolation. m20 makes this
        // a per-locus pointer carried on the locus struct;
        // m21 plumbs the right one through bus dispatch.
        let arena_global =
            self.module
                .add_global(ptr_t, None, "lotus.arena.global");
        arena_global.set_initializer(&ptr_t.const_null());
        arena_global.set_linkage(inkwell::module::Linkage::Internal);

        // m26 + m28b stage 1: cooperative scheduler — bus dispatch queue.
        // declare ptr  @lotus_bus_queue_create()
        // declare void @lotus_bus_queue_enqueue(ptr q, ptr handler, ptr self,
        //                                       ptr payload_src, i64 payload_size)
        // declare void @lotus_bus_queue_drain(ptr q)
        // declare void @lotus_bus_queue_destroy(ptr q)
        //
        // m28b stage 1 changed enqueue's signature: cells now carry
        // an INLINE payload buffer (memcpy'd from payload_src). The
        // subscriber-arena copy moves to drain time so that cross-
        // thread cells don't write into another thread's arena.
        let bus_queue_create_ty = ptr_t.fn_type(&[], false);
        self.module.add_function(
            "lotus_bus_queue_create",
            bus_queue_create_ty,
            None,
        );
        let bus_queue_enqueue_ty = void_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), ptr_t.into(), ptr_t.into(), i64_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_bus_queue_enqueue",
            bus_queue_enqueue_ty,
            None,
        );
        let bus_queue_drain_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_bus_queue_drain",
            bus_queue_drain_ty,
            None,
        );
        let bus_queue_destroy_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_bus_queue_destroy",
            bus_queue_destroy_ty,
            None,
        );

        // Pure-cooperative fast path: when the program never spawns
        // a pinned thread, the bus queue's mutex is dead weight
        // (~20-40ns/event uncontended). The C runtime checks
        // `g_bus_has_pinned` per enqueue/drain; this entry point
        // sets the flag. Codegen emits one call at program startup
        // when any locus in the program is `: schedule pinned`.
        let bus_mark_pinned_ty = void_t.fn_type(&[], false);
        self.module
            .add_function("lotus_bus_mark_pinned", bus_mark_pinned_ty, None);

        // m28b stage 2: per-pinned-locus mailbox surface.
        // declare ptr  @lotus_mailbox_create()
        // declare void @lotus_mailbox_post(ptr mb, ptr handler, ptr self,
        //                                  ptr payload_src, i64 payload_size)
        // declare i32  @lotus_mailbox_drain_one(ptr mb)
        // declare void @lotus_mailbox_shutdown(ptr mb)
        // declare void @lotus_mailbox_destroy(ptr mb)
        let mailbox_create_ty = ptr_t.fn_type(&[], false);
        self.module
            .add_function("lotus_mailbox_create", mailbox_create_ty, None);
        let mailbox_post_ty = void_t.fn_type(
            &[
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                i64_t.into(),
            ],
            false,
        );
        self.module
            .add_function("lotus_mailbox_post", mailbox_post_ty, None);
        let mailbox_drain_one_ty = i32_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_mailbox_drain_one",
            mailbox_drain_one_ty,
            None,
        );
        let mailbox_shutdown_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_mailbox_shutdown",
            mailbox_shutdown_ty,
            None,
        );
        let mailbox_destroy_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_mailbox_destroy", mailbox_destroy_ty, None);
        // 2026-05-23: coop→pinned mid-program drain.
        // declare void @lotus_mailbox_set_current(ptr mb)
        // declare ptr  @lotus_mailbox_get_current()
        // declare void @lotus_mailbox_drain_pending(ptr mb)
        let mailbox_set_current_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_mailbox_set_current",
            mailbox_set_current_ty,
            None,
        );
        let mailbox_get_current_ty = ptr_t.fn_type(&[], false);
        self.module.add_function(
            "lotus_mailbox_get_current",
            mailbox_get_current_ty,
            None,
        );
        let mailbox_drain_pending_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_mailbox_drain_pending",
            mailbox_drain_pending_ty,
            None,
        );

        // m45-followup: process-wide bus router living in the C
        // runtime. Replaces the per-program LLVM-side
        // {bus.entries, bus.count, lotus.bus_dispatch} triple
        // with a heap-grown dynamic vec; capacity is no longer
        // a compile-time-fixed multiple of the declared
        // subscription count.
        // declare void @lotus_bus_register(ptr subject, ptr self,
        //                                  ptr handler, ptr mailbox,
        //                                  ptr deserialize_fn)
        // declare void @lotus_bus_dispatch(ptr queue, ptr subject,
        //                                  ptr struct_payload, i64 struct_size,
        //                                  ptr serialize_fn)
        // declare void @lotus_bus_quarantine_self(ptr self)
        // declare void @lotus_bus_router_destroy()
        // m60: lotus_bus_register grows a 5th arg, the per-subject
        // deserialize fn ptr. The reader thread (m59) needs it to
        // decode wire-format bytes into a struct before invoking
        // the handler. Cooperative-only programs that never receive
        // bytes from the cross-process bus still pass it (it's
        // unused at runtime); kept unconditional to keep the ABI
        // stable across config-set vs config-not-set runs.
        // m70: lotus_bus_dispatch grows a 5th arg, the per-subject
        // serialize fn ptr. Local dispatch enqueues struct bytes
        // (the in-memory layout the publisher built); remote fanout
        // serializes those bytes via the supplied fn into the wire
        // format the reader thread will deserialize. Splitting
        // local-vs-remote here keeps the per-field wire format
        // (variable-width Strings) confined to the cross-process
        // path; local subscribers continue to receive struct bytes
        // exactly as before m70.
        let bus_register_ty = void_t.fn_type(
            &[
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
            ],
            false,
        );
        self.module
            .add_function("lotus_bus_register", bus_register_ty, None);
        // F.31 Phase 4: pool-aware register.
        // declare void @lotus_bus_register_with_pool(ptr subject, ptr self,
        //   ptr handler, ptr mailbox, ptr deserialize, ptr coop_pool)
        let bus_register_with_pool_ty = void_t.fn_type(
            &[
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
            ],
            false,
        );
        self.module.add_function(
            "lotus_bus_register_with_pool",
            bus_register_with_pool_ty,
            None,
        );
        // Phase 3 (2026-05-25, spec/semantics.md § "Phase 3:
        // routing keys"). Register a subscriber with a routing-
        // key filter. Same signature as
        // lotus_bus_register_with_pool plus three trailing args:
        //   key_filter_kind: i8 (0 = no filter, 1 = specific key,
        //                        2 = catch-unmatched fallback)
        //   key_lo: i64
        //   key_hi: i64
        // declare void @lotus_bus_register_keyed(ptr, ptr, ptr, ptr,
        //     ptr, ptr, i8, i64, i64)
        let i8_t = self.context.i8_type();
        let bus_register_keyed_ty = void_t.fn_type(
            &[
                ptr_t.into(),     // subject
                ptr_t.into(),     // self_ptr
                ptr_t.into(),     // handler
                ptr_t.into(),     // mailbox
                ptr_t.into(),     // deserialize
                ptr_t.into(),     // coop_pool
                i8_t.into(),      // key_filter_kind
                i64_t.into(),     // key_lo
                i64_t.into(),     // key_hi
            ],
            false,
        );
        self.module.add_function(
            "lotus_bus_register_keyed",
            bus_register_keyed_ty,
            None,
        );
        // Phase 3 keyed dispatch entry point. Same first 5 args as
        // lotus_bus_dispatch plus (key_lo, key_hi).
        let bus_dispatch_keyed_ty = void_t.fn_type(
            &[
                ptr_t.into(),     // queue
                ptr_t.into(),     // subject
                ptr_t.into(),     // struct_payload
                i64_t.into(),     // struct_size
                ptr_t.into(),     // serialize_fn
                i64_t.into(),     // key_lo
                i64_t.into(),     // key_hi
            ],
            false,
        );
        self.module.add_function(
            "lotus_bus_dispatch_keyed",
            bus_dispatch_keyed_ty,
            None,
        );
        // Phase 3 fail policy (2026-05-25): same signature as
        // _keyed but returns i32 (1 = matched, 0 = no specific
        // match). Codegen for `K <- value or raise` branches on
        // the return value to call lotus_root_panic.
        let bus_dispatch_keyed_fallible_ty = i32_t.fn_type(
            &[
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                i64_t.into(),
                ptr_t.into(),
                i64_t.into(),
                i64_t.into(),
            ],
            false,
        );
        self.module.add_function(
            "lotus_bus_dispatch_keyed_fallible",
            bus_dispatch_keyed_fallible_ty,
            None,
        );
        // Flat-payload serialize-skip siblings (2026-06-28): same
        // signatures as the _keyed / _keyed_fallible entries above.
        // Codegen routes here ONLY when the payload type is proven
        // flat (transitively pointer-free POD) via bus_payload_is_flat;
        // the runtime does a verbatim local fanout and serializes once
        // for remote only. serialize_fn is still passed (remote half
        // needs it). See lotus_arena.c lotus_bus_dispatch_flat.
        self.module.add_function(
            "lotus_bus_dispatch_keyed_flat",
            bus_dispatch_keyed_ty,
            None,
        );
        self.module.add_function(
            "lotus_bus_dispatch_keyed_fallible_flat",
            bus_dispatch_keyed_fallible_ty,
            None,
        );
        // F.31 Phase 4: cooperative-pool worker surface.
        // declare ptr  @lotus_coop_pool_register(ptr name)
        // declare ptr  @lotus_coop_pool_lookup(ptr name)
        // declare void @lotus_coop_pool_start_all()
        // declare void @lotus_coop_pool_shutdown_all()
        // declare void @lotus_coop_pool_destroy_all()
        let coop_pool_register_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_coop_pool_register",
            coop_pool_register_ty,
            None,
        );
        let coop_pool_lookup_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_coop_pool_lookup",
            coop_pool_lookup_ty,
            None,
        );
        // Pool-inheritance fix (2026-05-29): the pool whose worker
        // thread is currently on-CPU (NULL on main). Lets codegen
        // route a child instantiated inside a method body running
        // on a pool worker to that pool, when no static placement
        // name is known.
        // declare ptr @lotus_coop_pool_current()
        let coop_pool_current_ty = ptr_t.fn_type(&[], false);
        self.module.add_function(
            "lotus_coop_pool_current",
            coop_pool_current_ty,
            None,
        );
        // declare void @lotus_coop_pool_post(ptr pool, ptr handler,
        //     ptr self_ptr, ptr payload_src, i64 payload_size)
        let coop_pool_post_ty = void_t.fn_type(
            &[
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                i64_t.into(),
            ],
            false,
        );
        self.module.add_function(
            "lotus_coop_pool_post",
            coop_pool_post_ty,
            None,
        );
        let coop_pool_void_ty = void_t.fn_type(&[], false);
        self.module.add_function(
            "lotus_coop_pool_start_all",
            coop_pool_void_ty,
            None,
        );
        self.module.add_function(
            "lotus_coop_pool_shutdown_all",
            coop_pool_void_ty,
            None,
        );
        self.module.add_function(
            "lotus_coop_pool_destroy_all",
            coop_pool_void_ty,
            None,
        );
        // F.35 Slice 2: opt a pool into async_io mode (per-pool epoll
        // + ucontext coroutine dispatch). Called from the prelude
        // right after lotus_coop_pool_register for pools whose
        // placement entries declare `where async_io`.
        // declare i32 @lotus_coop_pool_enable_async_io(ptr pool)
        let coop_pool_enable_async_ty =
            self.context.i32_type().fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_coop_pool_enable_async_io",
            coop_pool_enable_async_ty,
            None,
        );
        let bus_dispatch_ty = void_t.fn_type(
            &[
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                i64_t.into(),
                ptr_t.into(),
            ],
            false,
        );
        self.module
            .add_function("lotus_bus_dispatch", bus_dispatch_ty, None);
        // Flat-payload serialize-skip sibling (2026-06-28): same
        // signature as lotus_bus_dispatch. See the keyed siblings above
        // and lotus_arena.c lotus_bus_dispatch_flat.
        self.module
            .add_function("lotus_bus_dispatch_flat", bus_dispatch_ty, None);
        // Static-devirt (build #1b). lotus_bus_register_static appends
        // the subscriber's g_bus_entries index into the per-subject
        // bucket `id` (in addition to the normal dynamic register —
        // the dynamic path stays source of truth). Same trailing args
        // as lotus_bus_register_keyed, prefixed with the i32 subject id.
        // declare void @lotus_bus_register_static(i32 id, ptr subject,
        //   ptr self, ptr handler, ptr mailbox, ptr deserialize,
        //   ptr coop_pool, i8 kind, i64 key_lo, i64 key_hi)
        let bus_register_static_ty = void_t.fn_type(
            &[
                i32_t.into(),     // subject id
                ptr_t.into(),     // subject
                ptr_t.into(),     // self_ptr
                ptr_t.into(),     // handler
                ptr_t.into(),     // mailbox
                ptr_t.into(),     // deserialize
                ptr_t.into(),     // coop_pool
                i8_t.into(),      // key_filter_kind
                i64_t.into(),     // key_lo
                i64_t.into(),     // key_hi
            ],
            false,
        );
        self.module.add_function(
            "lotus_bus_register_static",
            bus_register_static_ty,
            None,
        );
        // lotus_bus_dispatch_static reads bucket `id` directly (no
        // scan, no strcmp) and routes the publish identically to the
        // dynamic path. `flat` selects the verbatim-vs-wire fanout
        // (matching lotus_bus_dispatch_flat vs lotus_bus_dispatch);
        // `no_pinned` selects the no-acquire-load cooperative enqueue
        // when the program has no pinned/cross-pool placement (#3).
        // declare void @lotus_bus_dispatch_static(ptr queue, i32 id,
        //   ptr subject, ptr payload, i64 size, ptr serialize_fn,
        //   i32 flat, i32 no_pinned)
        let bus_dispatch_static_ty = void_t.fn_type(
            &[
                ptr_t.into(),     // queue
                i32_t.into(),     // subject id
                ptr_t.into(),     // subject
                ptr_t.into(),     // struct_payload
                i64_t.into(),     // struct_size
                ptr_t.into(),     // serialize_fn
                i32_t.into(),     // flat
                i32_t.into(),     // no_pinned
            ],
            false,
        );
        self.module.add_function(
            "lotus_bus_dispatch_static",
            bus_dispatch_static_ty,
            None,
        );
        // Direct-call devirt (build #1b slice-2). For an eligible
        // subject whose every subscriber is same-thread AND whose every
        // handler is provably QUIET AND whose payload is flat, codegen
        // replaces the deferred enqueue with this SYNCHRONOUS direct
        // call: it iterates the per-subject bucket `id` and calls each
        // subscriber's handler(self, payload) inline (skipping
        // quarantined / keyed / off-thread entries). No queue, no
        // serialize_fn, no flat/no_pinned flags — a flat payload is
        // passed straight through by pointer.
        // declare void @lotus_bus_dispatch_static_direct(i32 id,
        //   ptr subject, ptr payload, i64 size)
        let bus_dispatch_static_direct_ty = void_t.fn_type(
            &[
                i32_t.into(), // subject id
                ptr_t.into(), // subject
                ptr_t.into(), // payload
                i64_t.into(), // size
            ],
            false,
        );
        self.module.add_function(
            "lotus_bus_dispatch_static_direct",
            bus_dispatch_static_direct_ty,
            None,
        );
        // Direct-call-INLINE accessors (build #1b slice-3). For a
        // single-subscriber-handler direct subject, codegen bakes the
        // handler as a DIRECT call and walks the per-subject bucket
        // itself through these two LAYOUT-SAFE, HOISTABLE accessors —
        // instead of the indirect `e->handler` call inside the non-
        // inlined `lotus_bus_dispatch_static_direct` helper. Both are
        // marked `pure` (memory(read) + nounwind + willreturn) so LLVM
        // hoists the loop-invariant count/self-ptr loads out of a
        // publish loop and the baked handler inlines into the loop body.
        // declare i64 @lotus_bus_static_direct_count(i32 id)
        // declare ptr @lotus_bus_static_direct_selfptr(i32 id, i64 k)
        let direct_count_ty = i64_t.fn_type(&[i32_t.into()], false);
        let direct_count_fn = self.module.add_function(
            "lotus_bus_static_direct_count",
            direct_count_ty,
            None,
        );
        let direct_selfptr_ty =
            ptr_t.fn_type(&[i32_t.into(), i64_t.into()], false);
        let direct_selfptr_fn = self.module.add_function(
            "lotus_bus_static_direct_selfptr",
            direct_selfptr_ty,
            None,
        );
        // `pure` ⇒ memory(read) (encoded ArgMem|InaccessibleMem|Other =
        // Ref = 0b010101 = 21 per llvm/Support/ModRef.h) + nounwind +
        // willreturn. memory(read) (NOT memory(none)) keeps a quarantine
        // store elsewhere ordered correctly w.r.t. these reads while
        // still letting LICM hoist them when `id` is loop-invariant.
        {
            use inkwell::attributes::{Attribute, AttributeLoc};
            let mem_kind = Attribute::get_named_enum_kind_id("memory");
            let nounwind_kind = Attribute::get_named_enum_kind_id("nounwind");
            let willreturn_kind =
                Attribute::get_named_enum_kind_id("willreturn");
            let mem_read = self.context.create_enum_attribute(mem_kind, 21);
            let nounwind =
                self.context.create_enum_attribute(nounwind_kind, 0);
            let willreturn =
                self.context.create_enum_attribute(willreturn_kind, 0);
            for f in [direct_count_fn, direct_selfptr_fn] {
                f.add_attribute(AttributeLoc::Function, mem_read);
                f.add_attribute(AttributeLoc::Function, nounwind);
                f.add_attribute(AttributeLoc::Function, willreturn);
            }
        }
        let bus_quarantine_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_bus_quarantine_self",
            bus_quarantine_ty,
            None,
        );
        let bus_router_destroy_ty = void_t.fn_type(&[], false);
        self.module.add_function(
            "lotus_bus_router_destroy",
            bus_router_destroy_ty,
            None,
        );

        // m58: deployment-config subject binding. Codegen emits
        // a single call in main's prelude:
        //   lotus_bus_load_config(getenv("LOTUS_BUS_CONFIG"));
        // The C-runtime fn no-ops when path is NULL, so binaries
        // run without LOTUS_BUS_CONFIG set behave exactly as
        // pre-m58. Source-level lotus stays transport-agnostic
        // per notes/open-questions #8 — the binding lives entirely
        // in the deployment-config file.
        // declare void @lotus_bus_load_config(ptr path)
        // declare ptr  @getenv(ptr name)
        let bus_load_cfg_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_bus_load_config",
            bus_load_cfg_ty,
            None,
        );
        let getenv_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function("getenv", getenv_ty, None);

        // v1.x topic bindings: codegen emits one call per
        // `bindings { Topic: <transport> : role; }` entry in the
        // main locus, before lotus_bus_load_config so an env
        // override can layer on top of the static program shape.
        // declare void @lotus_bus_register_remote(ptr subject, ptr url, i32 role)
        let i32_t = self.context.i32_type();
        let bus_register_remote_ty = void_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), i32_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_bus_register_remote",
            bus_register_remote_ty,
            None,
        );

        // Wave B (bus-transport redesign): adapter-binding
        // registration. The runtime stores the (self, send_fn)
        // pair in lotus_bus_remote_entry_t's adapter slot and
        // dispatches outbound payloads through the fn pointer.
        // declare void @lotus_bus_register_remote_adapter(ptr subject, ptr self, ptr send_fn)
        let bus_register_adapter_ty = void_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), ptr_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_bus_register_remote_adapter",
            bus_register_adapter_ty,
            None,
        );

        // F.36 Slice 3 (2026-05-28): codec-binding registration.
        // declare void @lotus_bus_register_codec(ptr subject, ptr self,
        //   ptr encode_fn, ptr decode_fn)
        let bus_register_codec_ty = void_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), ptr_t.into(), ptr_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_bus_register_codec",
            bus_register_codec_ty,
            None,
        );

        // Form K4c + K7 (2026-05-20): shm_ring binding registration +
        // publish-side dispatch. K7 adds the overflow_policy i32
        // discriminator (matches ast::ShmRingOverflow::runtime_tag).
        // declare void @lotus_bus_register_shm_ring(ptr subject, i64 slot_size, i64 slot_count, ptr shm_name, i32 overflow_policy)
        let bus_register_shm_ring_ty = void_t.fn_type(
            &[
                ptr_t.into(),
                i64_t.into(),
                i64_t.into(),
                ptr_t.into(),
                i32_t.into(),
            ],
            false,
        );
        self.module.add_function(
            "lotus_bus_register_shm_ring",
            bus_register_shm_ring_ty,
            None,
        );
        // declare i32 @lotus_bus_publish_shm_ring(ptr subject, ptr value, i64 value_size)
        let bus_publish_shm_ring_ty = i32_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), i64_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_bus_publish_shm_ring",
            bus_publish_shm_ring_ty,
            None,
        );

        // Form K6b (2026-05-20): subscriber registration. Emitted
        // at subscriber locus birth alongside the existing
        // `lotus_bus_register` call. Opens the SHM ring (or
        // attaches), spawns a reader thread, and registers the
        // (self_ptr, handler_fn) pair for dispatch.
        // declare void @lotus_bus_register_subscriber_shm_ring(
        //   ptr subject, i64 slot_size, i64 slot_count,
        //   ptr shm_name, ptr self_ptr, ptr handler_fn)
        let bus_register_sub_shm_ring_ty = void_t.fn_type(
            &[
                ptr_t.into(),
                i64_t.into(),
                i64_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
            ],
            false,
        );
        self.module.add_function(
            "lotus_bus_register_subscriber_shm_ring",
            bus_register_sub_shm_ring_ty,
            None,
        );

        // Drain<T> batch consumer (2026-06-26): same signature as the
        // per-record register above, but spawns a BATCH reader thread
        // that calls the handler ONCE per available batch with a
        // `Drain<T>` handle (`{ void* ring, i64 start, i64 end }`)
        // instead of once per record. The handler loops over the
        // batch inline (`for t in feed`), reading each record
        // zero-copy via lotus_shm_ring_read_slot.
        // declare void @lotus_bus_register_subscriber_shm_ring_batch(
        //   ptr subject, i64 slot_size, i64 slot_count,
        //   ptr shm_name, ptr self_ptr, ptr batch_handler_fn)
        self.module.add_function(
            "lotus_bus_register_subscriber_shm_ring_batch",
            bus_register_sub_shm_ring_ty,
            None,
        );

        // Drain<T> for-loop primitive: read a pointer to the ring slot
        // at `seqno` (1-based). Returns NULL if the seqno is stale /
        // uncommitted; the batch for-loop skips those. `seqno` is a
        // uint64_t (i64, NOT size_t) — it is a logical sequence
        // number, native + wasm32 alike.
        // declare ptr @lotus_shm_ring_read_slot(ptr ring, i64 seqno)
        let shm_ring_read_slot_ty =
            ptr_t.fn_type(&[ptr_t.into(), i64_t.into()], false);
        self.module.add_function(
            "lotus_shm_ring_read_slot",
            shm_ring_read_slot_ty,
            None,
        );

        // Proposal B (2026-06-06): foreign-layout subscriber. Like
        // the native register above, but the ring shape is described
        // by a `ring_layout` rather than the LRSRNG1 header — so it
        // takes a flat 16-entry uint64 descriptor (built from the
        // resolved layout in `emit_bus_register_shm_ring`) instead of
        // slot_size/slot_count, and attaches read-only.
        // declare void @lotus_bus_register_subscriber_shm_ring_layout(
        //   ptr subject, ptr shm_name, ptr desc_words,
        //   ptr self_ptr, ptr handler_fn)
        let bus_register_sub_shm_ring_layout_ty = void_t.fn_type(
            &[
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
                ptr_t.into(),
            ],
            false,
        );
        self.module.add_function(
            "lotus_bus_register_subscriber_shm_ring_layout",
            bus_register_sub_shm_ring_layout_ty,
            None,
        );

        // Proposal B M3a (2026-06-06): foreign-layout PRODUCER. The
        // prelude emits a register for each layout-bound topic the
        // bundle publishes (creates the ring); publish sites route
        // through publish_shm_ring_layout (frames one byte_records
        // record + advances the cursor).
        // declare void @lotus_bus_register_shm_ring_layout(
        //   ptr subject, ptr shm_name, ptr desc_words, i64 capacity)
        let bus_register_shm_ring_layout_ty = void_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), ptr_t.into(), i64_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_bus_register_shm_ring_layout",
            bus_register_shm_ring_layout_ty,
            None,
        );
        // declare i32 @lotus_bus_publish_shm_ring_layout(
        //   ptr subject, ptr value, i64 value_size)
        let bus_publish_shm_ring_layout_ty = i32_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), i64_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_bus_publish_shm_ring_layout",
            bus_publish_shm_ring_layout_ty,
            None,
        );

        // m105: adapter-driven inbound dispatch. Hands wire bytes
        // through the subject's registered deserialize fn into the
        // local handler set. Backs the `std::bus::__local_dispatch`
        // primitive that adapter `run` loops invoke when they
        // receive a message from their transport.
        // declare void @lotus_bus_dispatch_wire(ptr subject, ptr wire_bytes, i64 wire_size)
        let bus_dispatch_wire_ty = void_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), i64_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_bus_dispatch_wire",
            bus_dispatch_wire_ty,
            None,
        );

        // m59: subscriber-side reader threads need access to the
        // cooperative bus queue to dispatch incoming bytes into
        // the local handler set. The codegen-emitted main prelude
        // publishes the queue pointer to the C runtime via
        // lotus_bus_set_queue right after lotus_bus_queue_create
        // succeeds; the reader thread uses it to call
        // lotus_bus_local_dispatch on each recv. Setter form
        // (rather than passing the queue through register_remote)
        // keeps register_remote's signature stable across
        // milestones and matches the pattern of bus_dispatch
        // taking the queue as an explicit parameter.
        // declare void @lotus_bus_set_queue(ptr queue)
        let bus_set_queue_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_bus_set_queue",
            bus_set_queue_ty,
            None,
        );

        // 2026-05-29: growable accept'd-children tracker. Replaces
        // the fixed __children[16] inline array whose unchecked
        // append corrupted adjacent struct memory past 16 accepts.
        // declare void @lotus_children_push(ptr buf, ptr count, ptr cap, ptr child)
        // (buf/count/cap are addresses of the parent struct's
        //  __children / __child_count / __child_cap fields)
        let children_push_ty = void_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), ptr_t.into(), ptr_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_children_push",
            children_push_ty,
            None,
        );
        // declare void @lotus_children_free(ptr buf)
        let children_free_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_children_free",
            children_free_ty,
            None,
        );
        // 2026-06-01: declare void @lotus_children_remove(ptr buf,
        // ptr count, ptr child) — swap-removes a reclaimed accept'd
        // child from its parent's tracker so iterating parents don't
        // deref a freed child. buf is the LOADED __children value
        // (void**), count is the ADDRESS of __child_count.
        let children_remove_ty = void_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), ptr_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_children_remove",
            children_remove_ty,
            None,
        );
        // 2026-07-01: accept'd-child struct recycling. Instantiation
        // of an owner-arena child (accept'd / bubbled) allocates via
        // the recycler; the teardown path pushes the dead struct
        // back. Keeps a churn daemon's owner arena at O(peak-alive)
        // children instead of O(total-ever) — the F.3 contract.
        // declare ptr @lotus_child_struct_alloc(ptr owner_arena, i64 size, i64 align)
        let child_struct_alloc_ty = ptr_t.fn_type(
            &[ptr_t.into(), i64_t.into(), i64_t.into()],
            false,
        );
        let child_struct_alloc_fn = self.module.add_function(
            "lotus_child_struct_alloc",
            child_struct_alloc_ty,
            None,
        );
        // declare void @lotus_child_struct_release(ptr owner_self, ptr child, i64 size)
        // (owner_self is the OWNER's locus struct; the runtime derefs
        //  its slot-0 __arena field itself, so this call site doesn't
        //  need the owner's concrete struct type.)
        let child_struct_release_ty = void_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), i64_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_child_struct_release",
            child_struct_release_ty,
            None,
        );

        // m70: lazy global payload arena for cross-process String
        // byte storage. The synthesized __deserialize_T body calls
        // this when decoding a length-prefixed String — allocates
        // a buffer that survives the reader-thread → dispatch →
        // handler chain (the per-locus arena isn't accessible at
        // deserialize time because the subscriber identity isn't
        // known yet; one subject can have multiple subscribers).
        // declare ptr @lotus_bus_payload_arena_alloc(i64 size, i64 align)
        let bus_payload_alloc_ty =
            ptr_t.fn_type(&[i64_t.into(), i64_t.into()], false);
        let bus_payload_alloc_fn = self.module.add_function(
            "lotus_bus_payload_arena_alloc",
            bus_payload_alloc_ty,
            None,
        );
        // 2026-07-01 aliasing metadata, stage 1 (continued — see the
        // arena-family block above for the soundness argument).
        // Fresh-memory allocators get a `noalias` return;
        // `lotus_child_struct_alloc` recycles only blocks whose
        // previous owners are dead (children_remove'd + latch-gated
        // release), the same contract malloc has when reusing freed
        // memory. The three accessors are pure reads audited against
        // runtime/lotus_arena.c: `lotus_str_len` = strlen,
        // `lotus_bytes_len` = one 8-byte load, `lotus_bytes_data` =
        // pointer arithmetic only — memory(read) (mask 21, same
        // encoding as the v0.9.0 static-dispatch block) + nounwind +
        // willreturn lets LICM hoist length reads out of loops and
        // CSE repeated calls.
        {
            use inkwell::attributes::{Attribute, AttributeLoc};
            let noalias_kind = Attribute::get_named_enum_kind_id("noalias");
            let nounwind_kind = Attribute::get_named_enum_kind_id("nounwind");
            let willreturn_kind =
                Attribute::get_named_enum_kind_id("willreturn");
            let mem_kind = Attribute::get_named_enum_kind_id("memory");
            let noalias = self.context.create_enum_attribute(noalias_kind, 0);
            let nounwind =
                self.context.create_enum_attribute(nounwind_kind, 0);
            let willreturn =
                self.context.create_enum_attribute(willreturn_kind, 0);
            let mem_read = self.context.create_enum_attribute(mem_kind, 21);
            for f in [bus_payload_alloc_fn, child_struct_alloc_fn] {
                f.add_attribute(AttributeLoc::Return, noalias);
                f.add_attribute(AttributeLoc::Function, nounwind);
                f.add_attribute(AttributeLoc::Function, willreturn);
            }
            for f in [str_len_fn, bytes_len_fn, bytes_data_fn] {
                f.add_attribute(AttributeLoc::Function, mem_read);
                f.add_attribute(AttributeLoc::Function, nounwind);
                f.add_attribute(AttributeLoc::Function, willreturn);
            }
        }

        // m28c: optional CPU-core affinity. Pinned loci that
        // declare `: schedule pinned(core = N)` emit a call to
        // this helper right after pthread_create — it wraps
        // pthread_setaffinity_np behind a stable signature so
        // codegen doesn't need to know the cpu_set_t layout.
        // declare void @lotus_set_core_affinity(i64 tid, i32 core)
        let set_aff_ty =
            void_t.fn_type(&[i64_t.into(), i32_t.into()], false);
        self.module
            .add_function("lotus_set_core_affinity", set_aff_ty, None);

        // The program-wide bus queue pointer. Initialized in
        // main's prelude alongside the arena; drained at
        // strategic points (before each deferred-dissolve flush)
        // so cooperative subscribers run their handlers before
        // they themselves dissolve. Destroyed at main exit.
        let bus_queue_global =
            self.module
                .add_global(ptr_t, None, "lotus.bus_queue.global");
        bus_queue_global.set_initializer(&ptr_t.const_null());
        bus_queue_global.set_linkage(inkwell::module::Linkage::Internal);

        // m27: pthread externs for pinned-class loci.
        // declare i32 @pthread_create(ptr thread, ptr attr, ptr start, ptr arg)
        // declare i32 @pthread_join(i64 thread, ptr retval)
        // pthread_t is `unsigned long` on Linux x86-64 — i64.
        // (If lotus ever targets a platform with a different
        // pthread_t representation, this hardcoded width will
        // need to grow into a target-specific selector.)
        let pthread_create_ty = i32_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), ptr_t.into(), ptr_t.into()],
            false,
        );
        self.module
            .add_function("pthread_create", pthread_create_ty, None);
        let pthread_join_ty =
            i32_t.fn_type(&[i64_t.into(), ptr_t.into()], false);
        self.module
            .add_function("pthread_join", pthread_join_ty, None);

        // m28a: per-locus thread_main is synthesized at the
        // pthread_create call site (no C-side adapter). Each
        // pinned locus gets its own `__pinned_main_<LocusName>`
        // function whose signature matches pthread's start-routine
        // contract directly: ptr (ptr).

        // declare i32 @fflush(ptr)
        //
        // Used by bubble() right before the dprintf-to-stderr so
        // any pending stdout output (from prior println calls in
        // an on_failure handler) flushes BEFORE the violation
        // report writes to fd 2, matching the interpreter's
        // observable output order.
        let fflush_ty = i32_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function("fflush", fflush_ty, None);

        // declare ptr @memcpy(ptr dest, ptr src, size_t n)
        //
        // Used by `bus_dispatch` to copy the publisher's payload
        // into a fresh allocation in the subscriber's arena before
        // invoking the handler — per spec/memory.md "A typed
        // message crossing a locus boundary is a copy, not a
        // pointer." Standard libc surface; we don't use LLVM's
        // intrinsic memcpy because clang lowers it through the
        // libc symbol anyway and a normal call is easier to
        // reason about. `n` is `size_t` — target-pointer-width
        // (i64 native / i32 wasm32) so the call matches the libc ABI.
        let memcpy_ty =
            ptr_t.fn_type(&[ptr_t.into(), ptr_t.into(), self.usize_type().into()], false);
        self.module.add_function("memcpy", memcpy_ty, None);

        // ---- Phase 1 stdlib builtins (m71+) ----
        //
        // Functions reached via the magic `std::*` path. Each
        // backing libc primitive is declared here; the per-symbol
        // lowering lives in the stdlib section near
        // `lower_std_process_pid`. Adding a new stdlib function
        // means: declare its libc backer here, add a match arm in
        // `lower_stdlib_path_call_expr` (or the stmt sibling), and
        // implement one `lower_std_*` method.

        // declare i32 @getpid(void)  — POSIX, backs std::process::pid()
        let getpid_ty = i32_t.fn_type(&[], false);
        self.module.add_function("getpid", getpid_ty, None);
        // declare i64 @lotus_process_rss_bytes(void)
        // 2026-05-21: peak resident-set size in bytes via
        // getrusage(RUSAGE_SELF). Observability primitive — lets
        // Hale code measure its own memory pressure without
        // needing the read_file-of-/proc/self/statm path
        // (synthesized files report st_size=0 so read_file's
        // fstat-based sizing returns empty; separate fix).
        let rss_bytes_ty = i64_t.fn_type(&[], false);
        self.module
            .add_function("lotus_process_rss_bytes", rss_bytes_ty, None);
        // pond P4 stage 1: terminal/raw-IO primitives.
        // declare i64 @lotus_term_is_tty(i64 fd)  — isatty(fd) -> 0/1
        let term_is_tty_ty = i64_t.fn_type(&[i64_t.into()], false);
        self.module
            .add_function("lotus_term_is_tty", term_is_tty_ty, None);
        // declare i64 @lotus_term_write_stdout(ptr s) — fflush + raw
        // write(1, s, strlen(s)); bytes written, -1 on error.
        let term_write_ty = i64_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_term_write_stdout", term_write_ty, None);
        // declare i64 @lotus_term_raw_enable(void) / @lotus_term_raw_disable(void)
        // — termios raw-mode toggle backing std::term::RawMode (1 ok / 0 fail).
        let term_raw_ty = i64_t.fn_type(&[], false);
        self.module
            .add_function("lotus_term_raw_enable", term_raw_ty, None);
        self.module
            .add_function("lotus_term_raw_disable", term_raw_ty, None);
        // declare i64 @lotus_term_size_packed(void)  — (cols<<16)|rows / 0
        self.module
            .add_function("lotus_term_size_packed", i64_t.fn_type(&[], false), None);
        // declare i64 @lotus_term_read_byte(i64 timeout_ms) — byte / -1 / -2
        self.module.add_function(
            "lotus_term_read_byte",
            i64_t.fn_type(&[i64_t.into()], false),
            None,
        );
        // declare void @lotus_arena_residency_dump_fd(i32 fd)
        // 2026-05-22 PM: writes per-arena residency snapshot
        // (bytes / chunks / construction backtrace) to the given
        // fd. No-op when LOTUS_ARENA_RESIDENCY is unset. The
        // Hale surface `std::process::dump_arena_residency()`
        // calls this with stderr's fd; downstream daemons wire
        // it into their checkpoint hook so locus arenas are
        // still alive when the snapshot is taken (atexit fires
        // post-dissolve and would report an empty set).
        let residency_dump_ty =
            void_t.fn_type(&[i32_t.into()], false);
        self.module.add_function(
            "lotus_arena_residency_dump_fd",
            residency_dump_ty,
            None,
        );
        // F.35 Slice 4: per-pool residency dump (parked-coro count +
        // pending cell-queue depth per cooperative pool). Surfaced
        // as `std::process::dump_pool_residency()` for ops embedding
        // in heartbeat ticks. Same shape as the arena residency
        // dump above — fd-parameterized so the same primitive can
        // back stderr ops dumps and per-route diagnostic captures.
        // declare void @lotus_coop_pool_dump_parked_counts(i32 fd)
        let pool_dump_ty =
            void_t.fn_type(&[i32_t.into()], false);
        self.module.add_function(
            "lotus_coop_pool_dump_parked_counts",
            pool_dump_ty,
            None,
        );

        // m73b: TCP primitives reachable from Hale source via
        // the `std::io::tcp::__*` magic-path calls. lotus_tcp_t
        // (the bus's "blocking-accept-of-one" struct adapter
        // from m72) stays for transport tests; these split-
        // shape fd-level primitives are what stdlib loci
        // call in their lifecycle bodies.

        // declare i32 @lotus_tcp_listen_socket(ptr host, i16 port)
        // bind + listen, returns listen_fd (>=0) or -1.
        let i16_t = self.context.i16_type();
        let tcp_listen_ty =
            i32_t.fn_type(&[ptr_t.into(), i16_t.into()], false);
        self.module
            .add_function("lotus_tcp_listen_socket", tcp_listen_ty, None);

        // declare i32 @lotus_tcp_accept_one(i32 listen_fd)
        // accept, returns conn_fd (>=0) or -1.
        let tcp_accept_ty = i32_t.fn_type(&[i32_t.into()], false);
        self.module
            .add_function("lotus_tcp_accept_one", tcp_accept_ty, None);

        // declare i32 @lotus_tcp_connect(ptr host, i16 port)
        // socket + connect with retry, returns conn_fd or -1.
        let tcp_connect_ty =
            i32_t.fn_type(&[ptr_t.into(), i16_t.into()], false);
        self.module
            .add_function("lotus_tcp_connect", tcp_connect_ty, None);

        // declare i32 @lotus_tcp_close_fd(i32 fd)
        // close, returns 0 or -1.
        let tcp_close_ty = i32_t.fn_type(&[i32_t.into()], false);
        self.module
            .add_function("lotus_tcp_close_fd", tcp_close_ty, None);
        // declare i32 @lotus_tcp_shutdown_listen_socket(i32 fd)
        // C-iii (2026-05-21): shutdown(SHUT_RDWR) on the listen
        // fd to interrupt a blocking accept() from any thread.
        // The fd stays open; dissolve() does the actual close.
        // Returns the shutdown() syscall result (0 / -1).
        let tcp_shutdown_listen_ty =
            i32_t.fn_type(&[i32_t.into()], false);
        self.module.add_function(
            "lotus_tcp_shutdown_listen_socket",
            tcp_shutdown_listen_ty,
            None,
        );
        // declare i32 @lotus_tcp_set_recv_timeout_ns(i32 fd, i64 ns)
        // SO_RCVTIMEO on the socket. On a main-thread blocking
        // accept() this bounds the wait (accept returns -1 after
        // `ns`), letting a server give up an idle listen instead of
        // blocking forever. ns <= 0 clears the timeout.
        let tcp_set_timeout_ty =
            i32_t.fn_type(&[i32_t.into(), i64_t.into()], false);
        self.module.add_function(
            "lotus_tcp_set_recv_timeout_ns",
            tcp_set_timeout_ty,
            None,
        );

        // TLS substrate (std::io::tls::*). Client-only at v1:
        // a handshaked connection identified by an opaque int
        // handle. Bodies live in `runtime/lotus_tls.c`; link
        // adds `-lssl -lcrypto` for system OpenSSL.
        //
        // declare i32 @lotus_tls_connect(ptr host, i16 port)
        let tls_connect_ty =
            i32_t.fn_type(&[ptr_t.into(), i16_t.into()], false);
        self.module
            .add_function("lotus_tls_connect", tls_connect_ty, None);
        // declare i32 @lotus_tls_send_bytes(i32 handle, ptr bytes)
        let tls_send_bytes_ty =
            i32_t.fn_type(&[i32_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_tls_send_bytes", tls_send_bytes_ty, None);
        // declare ptr @lotus_tls_recv_bytes(i32 handle, i32 max_bytes)
        let tls_recv_bytes_ty =
            ptr_t.fn_type(&[i32_t.into(), i32_t.into()], false);
        self.module
            .add_function("lotus_tls_recv_bytes", tls_recv_bytes_ty, None);
        // declare i32 @lotus_tls_close(i32 handle)
        let tls_close_ty = i32_t.fn_type(&[i32_t.into()], false);
        self.module
            .add_function("lotus_tls_close", tls_close_ty, None);

        // Raw UDP substrate (std::io::udp::*). Datagram socket;
        // preserves per-message boundaries from the kernel, no
        // delivery guarantee. NOT a bus transport (the bus's
        // atomic-delivery contract isn't satisfied by UDP).
        //
        // declare i32 @lotus_udp_bind(ptr host, i16 port)
        let udp_bind_ty =
            i32_t.fn_type(&[ptr_t.into(), i16_t.into()], false);
        self.module
            .add_function("lotus_udp_bind", udp_bind_ty, None);
        // declare i32 @lotus_udp_sendto_str(i32 fd, ptr host, i16 port, ptr msg)
        let udp_sendto_str_ty = i32_t.fn_type(
            &[i32_t.into(), ptr_t.into(), i16_t.into(), ptr_t.into()],
            false,
        );
        self.module
            .add_function("lotus_udp_sendto_str", udp_sendto_str_ty, None);
        // declare ptr @lotus_udp_recv_bytes_global(i32 fd, i32 max_bytes)
        let udp_recv_bytes_ty =
            ptr_t.fn_type(&[i32_t.into(), i32_t.into()], false);
        self.module.add_function(
            "lotus_udp_recv_bytes_global",
            udp_recv_bytes_ty,
            None,
        );
        // declare i32 @lotus_udp_close(i32 fd)
        let udp_close_ty = i32_t.fn_type(&[i32_t.into()], false);
        self.module
            .add_function("lotus_udp_close", udp_close_ty, None);

        // 2026-05-26: UDP multicast surface (P1) + transparent
        // setsockopt pass-through (P2). All take an fd as first
        // arg; named knobs map to single setsockopt calls. The
        // setsockopt_int / getsockopt_int / setsockopt_bool
        // pass-throughs accept level + name from
        // std::io::sockopt's named Int constants.
        // declare i32 @lotus_udp_join_group(i32 fd, ptr group, ptr iface)
        let udp_group_ty = i32_t.fn_type(
            &[i32_t.into(), ptr_t.into(), ptr_t.into()],
            false,
        );
        self.module
            .add_function("lotus_udp_join_group", udp_group_ty, None);
        self.module
            .add_function("lotus_udp_leave_group", udp_group_ty, None);
        // declare i32 @lotus_udp_set_multicast_ttl(i32 fd, i32 ttl)
        let udp_set_int_ty = i32_t.fn_type(
            &[i32_t.into(), i32_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_udp_set_multicast_ttl",
            udp_set_int_ty,
            None,
        );
        self.module.add_function(
            "lotus_udp_set_multicast_loop",
            udp_set_int_ty,
            None,
        );
        // declare i32 @lotus_udp_set_multicast_iface(i32 fd, ptr addr)
        let udp_set_iface_ty = i32_t.fn_type(
            &[i32_t.into(), ptr_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_udp_set_multicast_iface",
            udp_set_iface_ty,
            None,
        );
        // declare i32 @lotus_udp_setsockopt_int(i32 fd, i32 level, i32 name, i32 value)
        let udp_setsockopt_ty = i32_t.fn_type(
            &[i32_t.into(), i32_t.into(), i32_t.into(), i32_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_udp_setsockopt_int",
            udp_setsockopt_ty,
            None,
        );
        self.module.add_function(
            "lotus_udp_setsockopt_bool",
            udp_setsockopt_ty,
            None,
        );
        // declare i32 @lotus_udp_getsockopt_int(i32 fd, i32 level, i32 name)
        let udp_getsockopt_ty = i32_t.fn_type(
            &[i32_t.into(), i32_t.into(), i32_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_udp_getsockopt_int",
            udp_getsockopt_ty,
            None,
        );
        // declare i32 @lotus_tcp_set_nodelay(i32 fd, i32 on) — the
        // std::io::tcp::set_nodelay convenience (TCP_NODELAY).
        let tcp_set_nodelay_ty =
            i32_t.fn_type(&[i32_t.into(), i32_t.into()], false);
        self.module.add_function(
            "lotus_tcp_set_nodelay",
            tcp_set_nodelay_ty,
            None,
        );
        // std::diag::* gate-counter readers (#7). Both return i64; -1
        // when the wrap shim is absent (sanitizer builds).
        // declare i64 @lotus_diag_heap_alloc_count()
        self.module.add_function(
            "lotus_diag_heap_alloc_count",
            i64_t.fn_type(&[], false),
            None,
        );
        // declare i64 @lotus_diag_syscall_count(ptr name)
        self.module.add_function(
            "lotus_diag_syscall_count",
            i64_t.fn_type(&[ptr_t.into()], false),
            None,
        );
        // #5 follow-on: std::shm::last_record_* getters — the decoded
        // in-band header fields of the most recent foreign-ring record
        // dispatched on this thread (0 if the field isn't declared).
        for g in ["seq", "kernel_ns", "user_ns"] {
            self.module.add_function(
                &format!("lotus_shm_last_record_{}", g),
                i64_t.fn_type(&[], false),
                None,
            );
        }

        // std::io::sockopt::* named-constant getters. Each is a
        // zero-arg int-returning fn that returns the platform's
        // numeric value of the corresponding setsockopt
        // level / name. Dispatched via path-call lowering below
        // (under `std::io::sockopt::<NAME>`).
        let sockopt_getter_ty = i32_t.fn_type(&[], false);
        for name in SOCKOPT_NAMES {
            self.module.add_function(
                &format!("lotus_sockopt_{}", name),
                sockopt_getter_ty,
                None,
            );
        }

        // 2026-05-26 — UDP P4 (recv_with_source + timeouts).
        // declare ptr @lotus_udp_recv_bytes_with_source(i32 fd, i32 max_bytes)
        let udp_recv_src_ty =
            ptr_t.fn_type(&[i32_t.into(), i32_t.into()], false);
        self.module.add_function(
            "lotus_udp_recv_bytes_with_source",
            udp_recv_src_ty,
            None,
        );
        // declare ptr @lotus_udp_last_source_host()
        let udp_last_host_ty = ptr_t.fn_type(&[], false);
        self.module.add_function(
            "lotus_udp_last_source_host",
            udp_last_host_ty,
            None,
        );
        // declare i64 @lotus_udp_last_source_port()
        let udp_last_port_ty = i64_t.fn_type(&[], false);
        self.module.add_function(
            "lotus_udp_last_source_port",
            udp_last_port_ty,
            None,
        );
        // declare i32 @lotus_udp_set_recv_timeout_ns(i32 fd, i64 ns)
        let udp_set_timeout_ty = i32_t.fn_type(
            &[i32_t.into(), i64_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_udp_set_recv_timeout_ns",
            udp_set_timeout_ty,
            None,
        );
        self.module.add_function(
            "lotus_udp_set_send_timeout_ns",
            udp_set_timeout_ty,
            None,
        );
        // 2026-05-27 — TCP siblings of the udp timeout
        // primitives. Same C-side helper (sock_set_timeout_ns)
        // under the hood; the udp_set_timeout_ty fn signature
        // (`i32 (i32 fd, i64 ns) -> i32`) is identical.
        self.module.add_function(
            "lotus_tcp_set_recv_timeout_ns",
            udp_set_timeout_ty,
            None,
        );
        self.module.add_function(
            "lotus_tcp_set_send_timeout_ns",
            udp_set_timeout_ty,
            None,
        );
        // TLS siblings (WsClient liveness fix). Same `i32 (i32, i64)`
        // signature; the first arg is a TLS *handle* (the C side resolves it
        // to the connection's underlying socket fd) rather than a raw fd.
        self.module.add_function(
            "lotus_tls_set_recv_timeout_ns",
            udp_set_timeout_ty,
            None,
        );
        self.module.add_function(
            "lotus_tls_set_send_timeout_ns",
            udp_set_timeout_ty,
            None,
        );

        // Held-open file substrate (std::io::file::File). Mirrors
        // the lotus_tcp_* split shape — primitives hand a raw fd
        // back to the Hale-side locus, which stashes it on
        // self.fd and runs lotus_file_close in its dissolve().
        //
        // declare i32 @lotus_file_open(ptr path, ptr mode_str)
        let file_open_ty =
            i32_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_file_open", file_open_ty, None);
        // declare i32 @lotus_file_close(i32 fd)
        let file_close_ty = i32_t.fn_type(&[i32_t.into()], false);
        self.module
            .add_function("lotus_file_close", file_close_ty, None);
        // declare ptr @lotus_file_read_line_global(i32 fd)
        // Returns String alloc'd in the bus payload arena, or
        // NULL on EOF / error (caller distinguishes via errno).
        let file_read_line_ty = ptr_t.fn_type(&[i32_t.into()], false);
        self.module
            .add_function("lotus_file_read_line_global", file_read_line_ty, None);
        // declare i32 @lotus_file_at_eof(i32 fd)
        let file_at_eof_ty = i32_t.fn_type(&[i32_t.into()], false);
        self.module
            .add_function("lotus_file_at_eof", file_at_eof_ty, None);
        // declare i32 @lotus_file_seek(i32 fd, i64 offset)
        let file_seek_ty =
            i32_t.fn_type(&[i32_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_file_seek", file_seek_ty, None);
        // declare i32 @lotus_file_write_all(i32 fd, ptr buf, i64 len)
        let file_write_all_ty = i32_t.fn_type(
            &[i32_t.into(), ptr_t.into(), i64_t.into()],
            false,
        );
        self.module
            .add_function("lotus_file_write_all", file_write_all_ty, None);

        // m81: send / recv on a connected TCP fd, String-shaped.
        // declare i32 @lotus_tcp_send_str(i32 fd, ptr msg)
        let tcp_send_str_ty =
            i32_t.fn_type(&[i32_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_tcp_send_str", tcp_send_str_ty, None);
        // declare ptr @lotus_tcp_recv_str(i32 fd, i32 max_bytes)
        let tcp_recv_str_ty =
            ptr_t.fn_type(&[i32_t.into(), i32_t.into()], false);
        self.module
            .add_function("lotus_tcp_recv_str", tcp_recv_str_ty, None);
        // Phase 2g: binary-safe TCP recv. Mirrors recv_str's signature
        // (fd, max_bytes) but returns a Bytes blob (length-prefix +
        // body), so embedded NUL bytes survive.
        // declare ptr @lotus_tcp_recv_bytes(i32 fd, i32 max_bytes)
        let tcp_recv_bytes_ty =
            ptr_t.fn_type(&[i32_t.into(), i32_t.into()], false);
        self.module
            .add_function("lotus_tcp_recv_bytes", tcp_recv_bytes_ty, None);
        // Phase 1: caller-provided destination — recv into a
        // builder, zero allocation in g_bus_payload_arena.
        // declare i64 @lotus_tcp_recv_into(i32 fd, ptr builder, i64 max_bytes)
        let tcp_recv_into_ty = i64_t.fn_type(
            &[i32_t.into(), ptr_t.into(), i64_t.into()],
            false,
        );
        self.module
            .add_function("lotus_tcp_recv_into", tcp_recv_into_ty, None);
        // 2026-06-13 — recv_stamped (#1): same signature as recv_into,
        // plus the SO_TIMESTAMPNS setter and the thread-local stamp
        // getters.
        // declare i64 @lotus_tcp_recv_stamped(i32 fd, ptr builder, i64 max_bytes)
        self.module
            .add_function("lotus_tcp_recv_stamped", tcp_recv_into_ty, None);
        // declare i32 @lotus_tcp_set_rx_timestamps(i32 fd, i32 on)
        self.module.add_function(
            "lotus_tcp_set_rx_timestamps",
            i32_t.fn_type(&[i32_t.into(), i32_t.into()], false),
            None,
        );
        // declare i64 @lotus_tcp_last_recv_kernel_ns()
        self.module.add_function(
            "lotus_tcp_last_recv_kernel_ns",
            i64_t.fn_type(&[], false),
            None,
        );
        // declare i64 @lotus_tcp_last_recv_user_ns()
        self.module.add_function(
            "lotus_tcp_last_recv_user_ns",
            i64_t.fn_type(&[], false),
            None,
        );
        // declare i64 @lotus_tls_recv_into(i32 handle, ptr builder, i64 max_bytes)
        let tls_recv_into_ty = i64_t.fn_type(
            &[i32_t.into(), ptr_t.into(), i64_t.into()],
            false,
        );
        self.module
            .add_function("lotus_tls_recv_into", tls_recv_into_ty, None);
        // 2026-06-14 — TLS fast-path siblings (handle-addressed): nodelay,
        // SO_TIMESTAMPNS opt-in, recv_stamped + the stamp getters.
        self.module
            .add_function("lotus_tls_recv_stamped_into", tls_recv_into_ty, None);
        let tls_bool_setter_ty =
            i32_t.fn_type(&[i32_t.into(), i32_t.into()], false);
        self.module.add_function("lotus_tls_set_nodelay", tls_bool_setter_ty, None);
        self.module.add_function("lotus_tls_set_rx_timestamps", tls_bool_setter_ty, None);
        self.module.add_function(
            "lotus_tls_last_recv_kernel_ns", i64_t.fn_type(&[], false), None);
        self.module.add_function(
            "lotus_tls_last_recv_user_ns", i64_t.fn_type(&[], false), None);
        // declare i64 @lotus_udp_recv_into(i32 fd, ptr builder, i64 max_bytes)
        let udp_recv_into_ty = i64_t.fn_type(
            &[i32_t.into(), ptr_t.into(), i64_t.into()],
            false,
        );
        self.module
            .add_function("lotus_udp_recv_into", udp_recv_into_ty, None);
        // Phase 2g: cross-shape converters anchored in the global
        // payload arena so the result outlives the call site.
        // declare ptr @lotus_str_from_bytes(ptr bytes)
        let str_from_bytes_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_str_from_bytes", str_from_bytes_ty, None);
        // declare ptr @lotus_bytes_from_str(ptr str)
        let bytes_from_str_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_bytes_from_str", bytes_from_str_ty, None);
        // declare i64 @lotus_bytes_at(ptr bytes, i64 i)
        let bytes_at_ty =
            i64_t.fn_type(&[ptr_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_bytes_at", bytes_at_ty, None);
        // 2026-06-13 — std::bytes word-scan + masked-XOR primitives (#4).
        // declare i64 @lotus_bytes_find_byte(ptr b, i64 off, i64 needle)
        self.module.add_function(
            "lotus_bytes_find_byte",
            i64_t.fn_type(&[ptr_t.into(), i64_t.into(), i64_t.into()], false),
            None,
        );
        // declare i32 @lotus_bytes_builder_xor_mask_into(ptr handle, ptr src, i64 key)
        self.module.add_function(
            "lotus_bytes_builder_xor_mask_into",
            i32_t.fn_type(&[ptr_t.into(), ptr_t.into(), i64_t.into()], false),
            None,
        );
        // declare i64 @lotus_bytes_read_uint(ptr b, i64 off, i32 width,
        //   i32 is_signed, i32 big_endian, ptr oob) — binary-pack reader.
        let i32_bru = self.context.i32_type();
        let bytes_read_uint_ty = i64_t.fn_type(
            &[
                ptr_t.into(),
                i64_t.into(),
                i32_bru.into(),
                i32_bru.into(),
                i32_bru.into(),
                ptr_t.into(),
            ],
            false,
        );
        self.module.add_function(
            "lotus_bytes_read_uint",
            bytes_read_uint_ty,
            None,
        );
        // #3 (MirrorRing) — RAW {ptr,len} read siblings + the mirror ring.
        // declare i64 @lotus_bytes_read_uint_raw(ptr base, i64 len, i64 off,
        //   i32 width, i32 is_signed, i32 big_endian, ptr oob)
        self.module.add_function(
            "lotus_bytes_read_uint_raw",
            i64_t.fn_type(
                &[
                    ptr_t.into(), i64_t.into(), i64_t.into(),
                    i32_bru.into(), i32_bru.into(), i32_bru.into(),
                    ptr_t.into(),
                ],
                false,
            ),
            None,
        );
        // declare i64 @lotus_bytes_at_raw(ptr base, i64 len, i64 i)
        self.module.add_function(
            "lotus_bytes_at_raw",
            i64_t.fn_type(&[ptr_t.into(), i64_t.into(), i64_t.into()], false),
            None,
        );
        // declare i64 @lotus_bytes_find_byte_raw(ptr base, i64 len, i64 off, i64 needle)
        self.module.add_function(
            "lotus_bytes_find_byte_raw",
            i64_t.fn_type(
                &[ptr_t.into(), i64_t.into(), i64_t.into(), i64_t.into()],
                false,
            ),
            None,
        );
        // MirrorRing runtime: new/free/readable/writable/commit/consume/
        // len/capacity + recv-into-mirror. readable/writable return the
        // {ptr,len} view struct (a BytesMut).
        let mr_view_ty = self.view_struct_ty();
        self.module.add_function(
            "lotus_mirror_ring_new",
            ptr_t.fn_type(&[i64_t.into()], false),
            None,
        );
        self.module.add_function(
            "lotus_mirror_ring_free",
            self.context.void_type().fn_type(&[ptr_t.into()], false),
            None,
        );
        self.module.add_function(
            "lotus_mirror_ring_readable",
            mr_view_ty.fn_type(&[ptr_t.into()], false),
            None,
        );
        self.module.add_function(
            "lotus_mirror_ring_writable",
            mr_view_ty.fn_type(&[ptr_t.into()], false),
            None,
        );
        self.module.add_function(
            "lotus_mirror_ring_commit",
            self.context.void_type().fn_type(&[ptr_t.into(), i64_t.into()], false),
            None,
        );
        self.module.add_function(
            "lotus_mirror_ring_consume",
            self.context.void_type().fn_type(&[ptr_t.into(), i64_t.into()], false),
            None,
        );
        self.module.add_function(
            "lotus_mirror_ring_len",
            i64_t.fn_type(&[ptr_t.into()], false),
            None,
        );
        self.module.add_function(
            "lotus_mirror_ring_capacity",
            i64_t.fn_type(&[ptr_t.into()], false),
            None,
        );
        self.module.add_function(
            "lotus_tcp_recv_into_mirror",
            i64_t.fn_type(&[i32_bru.into(), ptr_t.into(), i64_t.into()], false),
            None,
        );
        // declare void @lotus_bytes_write_uint(ptr base, i64 cap, i64 off,
        //   i32 width, i64 val, i32 big_endian, ptr oob) — binary-pack
        // writer into a raw region (A1 zero-copy ring write).
        let bytes_write_uint_ty = self.context.void_type().fn_type(
            &[
                ptr_t.into(),
                i64_t.into(),
                i64_t.into(),
                i32_bru.into(),
                i64_t.into(),
                i32_bru.into(),
                ptr_t.into(),
            ],
            false,
        );
        self.module.add_function(
            "lotus_bytes_write_uint",
            bytes_write_uint_ty,
            None,
        );
        // JSON Tier-3 Level-A SIMD scan primitives:
        // i64 @lotus_json_next_*(ptr s, i64 from, i64 len).
        let json_scan_ty =
            i64_t.fn_type(&[ptr_t.into(), i64_t.into(), i64_t.into()], false);
        for name in [
            "lotus_json_next_struct_or_quote",
            "lotus_json_next_quote_or_bs",
            "lotus_json_next_non_ws",
        ] {
            self.module.add_function(name, json_scan_ty, None);
        }
        // A1 zero-copy ring write: reserve a slot, commit a length.
        // declare ptr @lotus_bus_reserve_shm_ring_layout(ptr subject, i64 max)
        let reserve_ty = ptr_t.fn_type(&[ptr_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_bus_reserve_shm_ring_layout", reserve_ty, None);
        // declare i32 @lotus_bus_commit_shm_ring_layout(ptr subject, i64 len)
        let commit_ty = i32_bru.fn_type(&[ptr_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_bus_commit_shm_ring_layout", commit_ty, None);
        // declare ptr @lotus_bytes_slice(ptr bytes, i64 lo, i64 hi)
        let bytes_slice_ty = ptr_t.fn_type(
            &[ptr_t.into(), i64_t.into(), i64_t.into()],
            false,
        );
        self.module
            .add_function("lotus_bytes_slice", bytes_slice_ty, None);
        // ws-echo: outbound construction primitives.
        // declare ptr @lotus_bytes_from_int(i64 v)
        let bytes_from_int_ty = ptr_t.fn_type(&[i64_t.into()], false);
        self.module
            .add_function("lotus_bytes_from_int", bytes_from_int_ty, None);
        // declare ptr @lotus_bytes_concat(ptr a, ptr b)
        let bytes_concat_ty =
            ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_bytes_concat", bytes_concat_ty, None);
        // ws-echo: SHA-1 + base64 for the WebSocket handshake.
        // declare ptr @lotus_crypto_sha1(ptr bytes)
        let sha1_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_crypto_sha1", sha1_ty, None);
        // C3 (pond follow-up): SHA-256 + HMAC-SHA256.
        // declare ptr @lotus_crypto_sha256(ptr bytes)
        let sha256_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_crypto_sha256", sha256_ty, None);
        // declare ptr @lotus_crypto_hmac_sha256(ptr key, ptr msg)
        let hmac_sha256_ty =
            ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_crypto_hmac_sha256", hmac_sha256_ty, None);
        // 2026-06-25 (fathom Kraken/Gate.io): SHA-512 + HMAC-SHA512.
        // declare ptr @lotus_crypto_sha512(ptr bytes)
        let sha512_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_crypto_sha512", sha512_ty, None);
        // declare ptr @lotus_crypto_hmac_sha512(ptr key, ptr msg)
        let hmac_sha512_ty =
            ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_crypto_hmac_sha512", hmac_sha512_ty, None);
        // 2026-05-27: declare i64 @lotus_crypto_crc32(ptr bytes)
        let crc32_ty = i64_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_crypto_crc32", crc32_ty, None);
        // fathom handoff (2026-06-02): ECDSA P-256 / ES256 (OpenSSL,
        // bodies in lotus_tls.c).
        // declare ptr @lotus_crypto_ecdsa_p256_sign(ptr key, ptr msg)
        let ecdsa_sign_ty =
            ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_crypto_ecdsa_p256_sign", ecdsa_sign_ty, None);
        // declare ptr @lotus_crypto_ecdsa_p256_sign_or_null(ptr key,
        // ptr msg) — NULL on failure, backs the fallible(CryptoError)
        // lowering (the bare call uses the empty-bytes form above).
        self.module.add_function(
            "lotus_crypto_ecdsa_p256_sign_or_null",
            ecdsa_sign_ty,
            None,
        );
        // declare i64 @lotus_crypto_ecdsa_p256_verify(ptr pub, ptr msg, ptr sig)
        let ecdsa_verify_ty =
            i64_t.fn_type(&[ptr_t.into(), ptr_t.into(), ptr_t.into()], false);
        self.module.add_function(
            "lotus_crypto_ecdsa_p256_verify",
            ecdsa_verify_ty,
            None,
        );
        // declare ptr @lotus_text_base64_encode(ptr bytes)
        let b64_encode_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_text_base64_encode", b64_encode_ty, None);
        // v1.x-16: declare ptr @lotus_text_base64_decode(ptr s)
        let b64_decode_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_text_base64_decode", b64_decode_ty, None);
        // declare ptr @lotus_text_base64url_encode(ptr bytes) — RFC
        // 4648 §5 URL-safe, unpadded (JWT/JWS, OAuth, webhooks).
        let b64url_encode_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_text_base64url_encode", b64url_encode_ty, None);
        // ws-echo: cheap RNG (xorshift64*) for nonces / jitter.
        let void_t = self.context.void_type();
        let rand_seed_ty = void_t.fn_type(&[], false);
        self.module
            .add_function("lotus_rand_seed_from_time", rand_seed_ty, None);
        let rand_next_ty = i64_t.fn_type(&[i64_t.into()], false);
        self.module
            .add_function("lotus_rand_next_int", rand_next_ty, None);

        // C7 (pond follow-up): wall-clock seconds since the Unix
        // epoch. Backs `std::time::now() -> Int`. Thin wrapper
        // around clock_gettime(CLOCK_REALTIME, &ts) in
        // lotus_arena.c — kept as a named C primitive (rather than
        // inlining like `time::monotonic` does) so the spec's
        // `lotus_time_now_seconds` symbol is observable from the
        // linked object.
        // declare i64 @lotus_time_now_seconds()
        let time_now_seconds_ty = i64_t.fn_type(&[], false);
        self.module.add_function(
            "lotus_time_now_seconds",
            time_now_seconds_ty,
            None,
        );
        // declare ptr @lotus_time_from_unix(i64 n)
        // Returns a NUL-terminated ISO 8601 UTC string in the
        // caller arena, the runtime representation of a Time value.
        let time_from_unix_ty = ptr_t.fn_type(&[i64_t.into()], false);
        self.module.add_function(
            "lotus_time_from_unix",
            time_from_unix_ty,
            None,
        );

        // Phase 2e: list_dir index API. count + at over the
        // cached newline-blob; both share the global payload arena.
        // declare i64 @lotus_fs_list_dir_count(ptr path)
        let list_dir_count_ty = i64_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_fs_list_dir_count", list_dir_count_ty, None);
        // declare ptr @lotus_fs_list_dir_at(ptr path, i64 idx)
        let list_dir_at_ty =
            ptr_t.fn_type(&[ptr_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_fs_list_dir_at", list_dir_at_ty, None);

        // m75: filesystem primitives reachable from Hale source
        // via the `std::io::fs::*` magic-path calls. The C-level
        // surface is in lotus_arena.c (m74 ship); these
        // declarations let codegen emit calls into them.

        // IoError synthesis helpers (used by the fallible-fs/tcp
        // dispatcher in `try_lower_fallible_stdlib_path_call`).
        // declare i32 @lotus_get_errno(void)
        let get_errno_ty = i32_t.fn_type(&[], false);
        self.module
            .add_function("lotus_get_errno", get_errno_ty, None);
        // declare ptr @lotus_io_error_kind(i32 errno_val)
        let io_kind_ty = ptr_t.fn_type(&[i32_t.into()], false);
        self.module
            .add_function("lotus_io_error_kind", io_kind_ty, None);

        // declare i64 @lotus_fs_read_file(ptr path, ptr out_buf, i64 out_cap)
        // returns bytes read (>=0) or -1.
        let fs_read_ty =
            i64_t.fn_type(&[ptr_t.into(), ptr_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_fs_read_file", fs_read_ty, None);
        // declare ptr @lotus_fs_read_file_growing(ptr arena, ptr path)
        // 2026-05-21: size-tolerant variant for synthesized files
        // (/proc/*, /sys/*, FIFO pipes) where fstat returns 0.
        // Returns a NUL-terminated arena-allocated String or NULL
        // on error. read_file's fallible codegen path routes
        // through this so /proc/self/statm-style reads return
        // the real bytes instead of an empty string.
        let fs_read_growing_ty =
            ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module.add_function(
            "lotus_fs_read_file_growing",
            fs_read_growing_ty,
            None,
        );

        // declare i32 @lotus_fs_write_file(ptr path, ptr buf, i64 len)
        // returns 0 or -1.
        let fs_write_ty =
            i32_t.fn_type(&[ptr_t.into(), ptr_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_fs_write_file", fs_write_ty, None);

        // declare i32 @lotus_fs_write_file_append(ptr path, ptr buf, i64 len)
        // ergonomics arc — returns 0 or -1; opens with O_APPEND
        // instead of O_TRUNC. Companion to write_file.
        let fs_write_append_ty =
            i32_t.fn_type(&[ptr_t.into(), ptr_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_fs_write_file_append", fs_write_append_ty, None);

        // declare i32 @lotus_fs_mkdir(ptr path)
        // ergonomics arc — returns 0 on success, -1 on error
        // (errno set; EEXIST if dir already exists). Single-level
        // only; not recursive.
        let fs_mkdir_ty = i32_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_fs_mkdir", fs_mkdir_ty, None);

        // C9 (pond/logfmt rotation):
        // declare i32 @lotus_fs_rename(ptr src, ptr dst)
        // Returns 0 on success, -1 on error (errno set).
        let fs_rename_ty =
            i32_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_fs_rename", fs_rename_ty, None);

        // C9 (pond/logfmt rotation):
        // declare i32 @lotus_fs_unlink(ptr path)
        // Returns 0 on success, -1 on error (errno set).
        let fs_unlink_ty = i32_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_fs_unlink", fs_unlink_ty, None);

        // C9 (pond/agent/sandbox):
        // declare ptr @lotus_fs_mktemp(ptr prefix, ptr suffix)
        // mkstemps(3) wrapper. Returns an arena-anchored path
        // string on success, NULL on error (errno set). Same
        // String-fallible shape as lotus_fs_read_file.
        let fs_mktemp_ty =
            ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_fs_mktemp", fs_mktemp_ty, None);

        // declare i64 @lotus_fs_file_size(ptr path)
        // returns size or -1.
        let fs_size_ty = i64_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_fs_file_size", fs_size_ty, None);

        // declare i32 @lotus_fs_file_exists(ptr path)
        // returns 0 or 1.
        let fs_exists_ty = i32_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_fs_file_exists", fs_exists_ty, None);

        // declare ptr @lotus_fs_extension_global(ptr path)
        // returns the basename's last-dot suffix (".go", ".md"),
        // or the empty string when there is no extension. Result
        // lives in the lazy global payload arena.
        let fs_extension_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_fs_extension_global", fs_extension_ty, None);

        // declare ptr @lotus_stdin_read_line()
        // Reads a single line from stdin; trailing newline (and
        // optional CR) stripped. Returns the empty-string sentinel
        // on EOF / IO error. Result lives in the lazy global
        // payload arena (pointer-stable for the program's life).
        // Paired with lotus_stdin_read_line_status for callers
        // that need to distinguish empty-line from EOF.
        let stdin_read_line_ty = ptr_t.fn_type(&[], false);
        self.module
            .add_function("lotus_stdin_read_line", stdin_read_line_ty, None);

        // declare i32 @lotus_stdin_read_line_status()
        // Returns status of the most recent lotus_stdin_read_line:
        //   0 = success, -1 = EOF, -2 = IO error, -3 = OOM.
        let stdin_status_ty = i32_t.fn_type(&[], false);
        self.module
            .add_function("lotus_stdin_read_line_status", stdin_status_ty, None);

        // declare ptr @lotus_bus_payload_arena_alloc(i64 size, i64 align)
        // m70 lazy global arena for cross-call buffer ownership.
        // read_file uses this to allocate the returned String
        // since the buffer must outlive the call frame.
        let arena_alloc_ty =
            ptr_t.fn_type(&[i64_t.into(), i64_t.into()], false);
        self.module.add_function(
            "lotus_bus_payload_arena_alloc",
            arena_alloc_ty,
            None,
        );

        // declare i64 @lotus_str_len(ptr s)
        // (already declared earlier in this fn for String ops; re-
        // declaration via add_function would be a duplicate symbol
        // so we skip — codegen reuses the existing one.)

        // m77: env / argv primitives. Codegen emits a call to
        // lotus_env_init in main's prelude that captures argc/
        // argv into static globals; the std::env::* path calls
        // then reach them via the get-style accessors below.

        // declare void @lotus_env_init(i32 argc, ptr argv)
        let env_init_ty =
            self.context.void_type().fn_type(&[i32_t.into(), ptr_t.into()], false);
        self.module
            .add_function("lotus_env_init", env_init_ty, None);

        // 2026-05-17 — declare void @lotus_io_init(void). Called
        // from main's prelude to switch stdout to line-buffered
        // so `println` flushes on `\n` regardless of whether
        // stdout is a TTY or pipe. Fixes the silent-drop bug
        // where `println("READY"); accept_block();` hung pipe
        // consumers because the "READY" sat in libc's buffer.
        let io_init_ty = self.context.void_type().fn_type(&[], false);
        self.module
            .add_function("lotus_io_init", io_init_ty, None);

        // declare i32 @lotus_env_args_count(void)
        let env_args_count_ty = i32_t.fn_type(&[], false);
        self.module
            .add_function("lotus_env_args_count", env_args_count_ty, None);

        // declare ptr @lotus_env_arg(i32 i)
        let env_arg_ty = ptr_t.fn_type(&[i32_t.into()], false);
        self.module.add_function("lotus_env_arg", env_arg_ty, None);

        // declare ptr @lotus_env_var(ptr name)
        let env_var_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function("lotus_env_var", env_var_ty, None);

        // declare i32 @lotus_env_var_exists(ptr name)
        let env_var_exists_ty = i32_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_env_var_exists",
            env_var_exists_ty,
            None,
        );

        // m78: minimal string-parsing primitives.
        // declare i64 @lotus_str_parse_int(ptr s)
        let parse_int_ty = i64_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_str_parse_int", parse_int_ty, None);
        // declare i32 @lotus_str_can_parse_int(ptr s)
        let can_parse_ty = i32_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_str_can_parse_int",
            can_parse_ty,
            None,
        );
        // v1.x-16: declare double @lotus_str_parse_float(ptr s)
        let parse_float_ty =
            self.context.f64_type().fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_str_parse_float", parse_float_ty, None);
        // declare i32 @lotus_str_can_parse_float(ptr s)
        let can_parse_float_ty = i32_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_str_can_parse_float",
            can_parse_float_ty,
            None,
        );
        // declare void @lotus_str_parse_decimal(ptr s, ptr out_hi, ptr out_lo)
        // The i128 mantissa is returned as two i64 halves via out
        // pointers — matches the lotus_decimal_to_string convention
        // (the LLVM/C ABI for __int128 is awkward to wire uniformly).
        let parse_decimal_ty = self.context.void_type().fn_type(
            &[ptr_t.into(), ptr_t.into(), ptr_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_str_parse_decimal",
            parse_decimal_ty,
            None,
        );
        // declare i32 @lotus_str_can_parse_decimal(ptr s)
        let can_parse_decimal_ty = i32_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_str_can_parse_decimal",
            can_parse_decimal_ty,
            None,
        );

        // 2026-05-26: direct byte-access on a String. The
        // safe variant strlens internally; the _unchecked
        // variant is for tight scan loops where the bound is
        // externally known (caller has called len(s) once).
        let str_byte_at_ty = i64_t.fn_type(
            &[ptr_t.into(), i64_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_str_byte_at",
            str_byte_at_ty,
            None,
        );
        self.module.add_function(
            "lotus_str_byte_at_unchecked",
            str_byte_at_ty,
            None,
        );

        // 2026-05-26: range-bounded variants of the str parsers
        // for the allocation-free JSON-walk path. Each takes
        // (json: ptr, start: i64, end_exclusive: i64) plus the
        // existing out / target args.
        // declare i32 @lotus_str_range_eq(ptr s, i64 start, i64 end, ptr t)
        let range_eq_ty = i32_t.fn_type(
            &[ptr_t.into(), i64_t.into(), i64_t.into(), ptr_t.into()],
            false,
        );
        self.module
            .add_function("lotus_str_range_eq", range_eq_ty, None);
        // declare i64 @lotus_str_parse_int_range(ptr s, i64 start, i64 end)
        let parse_int_range_ty = i64_t.fn_type(
            &[ptr_t.into(), i64_t.into(), i64_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_str_parse_int_range",
            parse_int_range_ty,
            None,
        );
        // declare i32 @lotus_str_can_parse_int_range(ptr s, i64 start, i64 end)
        let can_parse_int_range_ty = i32_t.fn_type(
            &[ptr_t.into(), i64_t.into(), i64_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_str_can_parse_int_range",
            can_parse_int_range_ty,
            None,
        );
        // declare void @lotus_str_parse_decimal_range(ptr s, i64 start, i64 end, ptr out_hi, ptr out_lo)
        let parse_decimal_range_ty = self.context.void_type().fn_type(
            &[
                ptr_t.into(),
                i64_t.into(),
                i64_t.into(),
                ptr_t.into(),
                ptr_t.into(),
            ],
            false,
        );
        self.module.add_function(
            "lotus_str_parse_decimal_range",
            parse_decimal_range_ty,
            None,
        );
        // declare i32 @lotus_str_can_parse_decimal_range(ptr s, i64 start, i64 end)
        let can_parse_decimal_range_ty = i32_t.fn_type(
            &[ptr_t.into(), i64_t.into(), i64_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_str_can_parse_decimal_range",
            can_parse_decimal_range_ty,
            None,
        );

        // v1.x: ASCII case folding primitives.
        // declare ptr @lotus_str_lower(ptr s)
        let case_fold_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_str_lower", case_fold_ty, None);
        // declare ptr @lotus_str_upper(ptr s)
        self.module
            .add_function("lotus_str_upper", case_fold_ty, None);
        // declare ptr @lotus_str_substring(ptr s, i64 lo, i64 hi)
        let str_substring_ty = ptr_t.fn_type(
            &[ptr_t.into(), i64_t.into(), i64_t.into()],
            false,
        );
        self.module
            .add_function("lotus_str_substring", str_substring_ty, None);

        // declare ptr @lotus_str_trim(ptr s)
        self.module
            .add_function("lotus_str_trim", case_fold_ty, None);
        // declare ptr @lotus_str_replace(ptr s, ptr needle, ptr rep)
        let replace_ty = ptr_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), ptr_t.into()],
            false,
        );
        self.module
            .add_function("lotus_str_replace", replace_ty, None);
        // declare ptr @lotus_str_repeat(ptr s, i64 n)
        let repeat_ty = ptr_t.fn_type(&[ptr_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_str_repeat", repeat_ty, None);
        // declare ptr @lotus_str_pad_left(ptr s, i64 width, ptr pad)
        let pad_ty = ptr_t.fn_type(
            &[ptr_t.into(), i64_t.into(), ptr_t.into()],
            false,
        );
        self.module
            .add_function("lotus_str_pad_left", pad_ty, None);
        // declare ptr @lotus_str_pad_right(ptr s, i64 width, ptr pad)
        self.module
            .add_function("lotus_str_pad_right", pad_ty, None);

        // v1.x-15: string-builder primitive. Doubling realloc-backed
        // buffer that turns O(N²) accumulation into amortized O(N).
        // Handle is a `ptr` (carried as Bytes in the Hale surface).
        // declare ptr @lotus_str_builder_new(void)
        let sb_new_ty = ptr_t.fn_type(&[], false);
        self.module
            .add_function("lotus_str_builder_new", sb_new_ty, None);
        // declare void @lotus_str_builder_append(ptr handle, ptr s)
        let sb_append_ty = self
            .context
            .void_type()
            .fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module.add_function(
            "lotus_str_builder_append",
            sb_append_ty,
            None,
        );
        // declare i64 @lotus_str_builder_len(ptr handle)
        let sb_len_ty = i64_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_str_builder_len", sb_len_ty, None);
        // declare ptr @lotus_str_builder_finish(ptr handle)
        let sb_finish_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_str_builder_finish",
            sb_finish_ty,
            None,
        );

        // C10 (pond follow-up): binary-safe builder. Same shape as
        // the str-builder above but the chunk arg is a Bytes blob
        // (read via the `[i64 len]` prefix so embedded NULs survive)
        // and finish() returns a Bytes blob (no trailing NUL).
        // Handle stays a `ptr` carried as Bytes in the Hale
        // surface, matching the str-builder ergonomic.
        // declare ptr @lotus_bytes_builder_new(i64 initial_cap)
        let bb_new_ty = ptr_t.fn_type(&[i64_t.into()], false);
        self.module
            .add_function("lotus_bytes_builder_new", bb_new_ty, None);
        // declare i64 @lotus_bytes_builder_append(ptr handle, ptr chunk)
        // 2026-05-19: was `void` — now returns 1=ok / 0=fail so the
        // BytesBuilder locus's `append` method can `violate
        // alloc_failed` on realloc-NULL (F.27 routing).
        let bb_append_ty = i64_t
            .fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module.add_function(
            "lotus_bytes_builder_append",
            bb_append_ty,
            None,
        );
        // declare i64 @lotus_bytes_builder_append_str(ptr handle, ptr str)
        // — append a Hale String's bytes (NUL-terminated) in one
        // strlen + memcpy. Same (ptr, ptr) shape as __append.
        self.module.add_function(
            "lotus_bytes_builder_append_str",
            bb_append_ty,
            None,
        );
        // declare i64 @lotus_bytes_builder_append_scalar(ptr handle,
        //   i64 value, i32 width, i32 big_endian) — binary-pack writer.
        let i32_bbas = self.context.i32_type();
        let bb_append_scalar_ty = i64_t.fn_type(
            &[ptr_t.into(), i64_t.into(), i32_bbas.into(), i32_bbas.into()],
            false,
        );
        self.module.add_function(
            "lotus_bytes_builder_append_scalar",
            bb_append_scalar_ty,
            None,
        );
        // declare i64 @lotus_bytes_builder_append_pad(ptr handle, i64 to_align)
        let bb_append_pad_ty =
            i64_t.fn_type(&[ptr_t.into(), i64_t.into()], false);
        self.module.add_function(
            "lotus_bytes_builder_append_pad",
            bb_append_pad_ty,
            None,
        );
        // declare i64 @lotus_bytes_builder_len(ptr handle)
        let bb_len_ty = i64_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_bytes_builder_len", bb_len_ty, None);
        // declare ptr @lotus_bytes_builder_finish(ptr handle)
        let bb_finish_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_bytes_builder_finish",
            bb_finish_ty,
            None,
        );
        // Phase-0 in-place ops for long-lived recv-loop accumulators
        // (pond/websocket per-frame Bytes-allocation leak).
        // declare void @lotus_bytes_builder_shift_front(ptr handle, i64 n)
        let bb_shift_ty = self
            .context
            .void_type()
            .fn_type(&[ptr_t.into(), i64_t.into()], false);
        self.module.add_function(
            "lotus_bytes_builder_shift_front",
            bb_shift_ty,
            None,
        );
        // declare void @lotus_bytes_builder_clear(ptr handle)
        let bb_clear_ty =
            self.context.void_type().fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_bytes_builder_clear",
            bb_clear_ty,
            None,
        );
        // declare ptr @lotus_bytes_builder_snapshot(ptr handle)
        let bb_snap_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_bytes_builder_snapshot",
            bb_snap_ty,
            None,
        );
        // declare void @lotus_bytes_builder_free(ptr handle)
        let bb_free_ty =
            self.context.void_type().fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_bytes_builder_free",
            bb_free_ty,
            None,
        );
        // F.30b view-ABI compaction (2026-05-22 PM): the view is
        // a 16-byte `{void *src, int64_t epoch}` struct passed and
        // returned by value. Both eightbytes are INTEGER class per
        // SysV AMD64, so returns land in {rax, rdx} and by-value
        // args land in two arg registers — no memory traffic, no
        // arena_alloc per view() call (the dominant residual
        // chunk-allocation trigger pre-rework).
        let view_struct_ty = self.view_struct_ty();
        // declare {ptr, i64} @lotus_bytes_builder_view(ptr handle)
        let bb_view_ty = view_struct_ty.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_bytes_builder_view",
            bb_view_ty,
            None,
        );
        // declare {ptr, i64} @lotus_bytes_builder_text_view(ptr handle)
        let bb_text_view_ty = view_struct_ty.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_bytes_builder_text_view",
            bb_text_view_ty,
            None,
        );
        // View-unpack helpers — take a view by value, return the
        // underlying data pointer (Bytes-shaped for bytes_view_data,
        // NUL-terminated C string for str_view_data). The helper
        // compares the view's epoch against the source builder's
        // live mutation_epoch and panics on mismatch; the static
        // sentinel `epoch == -1` skips the check.
        // declare ptr @lotus_bytes_view_data({ptr, i64} view)
        // declare ptr @lotus_str_view_data({ptr, i64} view)
        let view_data_ty = ptr_t.fn_type(&[view_struct_ty.into()], false);
        self.module.add_function(
            "lotus_bytes_view_data",
            view_data_ty,
            None,
        );
        self.module.add_function(
            "lotus_str_view_data",
            view_data_ty,
            None,
        );
        // F.30b (5b): wrap a static-data ptr (String/Bytes literal)
        // in a view struct with the static-epoch sentinel. The
        // unpack helper skips the epoch check and returns `src`
        // directly. No arena allocation — the view materializes in
        // SSA registers and lands in the caller's storage slot.
        // declare {ptr, i64} @lotus_view_from_static_data(ptr data)
        let view_static_ty = view_struct_ty.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_view_from_static_data",
            view_static_ty,
            None,
        );
        // declare i64 @lotus_bytes_builder_append_slice(ptr handle, ptr src_blob, i64 lo, i64 hi)
        // Phase-3 Site 1 (2026-05-19): copy src[lo..hi) directly
        // into the builder's tail. Returns 1=ok / 0=fail (null
        // handle, out-of-range, realloc NULL). Hale-side
        // wrapper routes 0 through `violate alloc_failed` per
        // F.27. Eliminates the slice+append pair's intermediate
        // Bytes wrapper that otherwise lands in g_bus_payload_arena.
        let bb_append_slice_ty = i64_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), i64_t.into(), i64_t.into()],
            false,
        );
        self.module.add_function(
            "lotus_bytes_builder_append_slice",
            bb_append_slice_ty,
            None,
        );
        // declare void @lotus_set_caller_arena(ptr arena)
        // Phase-3 (2026-05-19) __caller_arena threading: stdlib
        // primitives that previously allocated into the program-
        // lifetime g_bus_payload_arena now read a thread-local
        // arena pointer instead. The codegen emits a setter call
        // before each user-callable primitive invocation, passing
        // `current_arena_ptr()` from the calling context (locus
        // method's self arena / free fn's __caller_arena /
        // main's program arena). Primitives fall back to the
        // capped g_bus_payload_arena when TLS is null (interpreter
        // or non-Hale C entry). See lotus_arena.c
        // lotus_caller_arena_or_global for the read side.
        let void_t = self.context.void_type();
        let set_caller_arena_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_set_caller_arena",
            set_caller_arena_ty,
            None,
        );
        // declare ptr @lotus_caller_arena_or_global()
        // Bus-arena reclaim (2026-05-21): the read side of the
        // TLS caller-arena pair. Locus-method epilogues call this
        // to find the deep-copy destination for heap return
        // values — the same arena the caller set via
        // `lotus_set_caller_arena` just before the call. Falls
        // back to the capped g_bus_payload_arena (lazy global)
        // when TLS is null so degenerate entry paths don't
        // deref a NULL arena.
        let caller_arena_or_global_ty = ptr_t.fn_type(&[], false);
        self.module.add_function(
            "lotus_caller_arena_or_global",
            caller_arena_or_global_ty,
            None,
        );
        // declare i64 @lotus_bytes_is_alloc_fail(ptr blob)
        // F.27 discriminator: 1 iff blob is the alloc-fail
        // sentinel returned from BytesBuilder snapshot()/finish()
        // failure paths. Used by the locus method bodies to detect
        // payload-arena alloc failure and route through
        // `violate alloc_failed` before returning. Success paths
        // always emit a fresh arena-allocated blob (even for
        // len=0) via lotus_bytes_create, so the sentinel is
        // unambiguous.
        let bb_is_alloc_fail_ty = i64_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_bytes_is_alloc_fail",
            bb_is_alloc_fail_ty,
            None,
        );

        // m96: tree-sitter substrate. extern "C" symbols defined
        // in `runtime/lotus_treesitter.rs` (compiled into the
        // sibling `hale-ts-shim` staticlib). The link step
        // adds `libhale_ts_shim.a` so these references resolve.
        // All handles are i64 (1-based; 0 = absent / failure).
        // String returns land in the lazy global payload arena.
        let i64_handle_ty = i64_t;
        // declare i64 @lotus_ts_parse_go(ptr src)
        let ts_parse_ty = i64_handle_ty.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_ts_parse_go", ts_parse_ty, None);
        // declare i64 @lotus_ts_root_node(i64 tree_id)
        let ts_root_ty = i64_handle_ty.fn_type(&[i64_t.into()], false);
        self.module
            .add_function("lotus_ts_root_node", ts_root_ty, None);
        // declare ptr @lotus_ts_node_kind(i64 node_id)
        let ts_kind_ty = ptr_t.fn_type(&[i64_t.into()], false);
        self.module
            .add_function("lotus_ts_node_kind", ts_kind_ty, None);
        // declare i64 @lotus_ts_node_child_count(i64 node_id)
        let ts_count_ty = i64_t.fn_type(&[i64_t.into()], false);
        self.module
            .add_function("lotus_ts_node_child_count", ts_count_ty, None);
        // declare i64 @lotus_ts_node_named_child_count(i64 node_id)
        self.module
            .add_function("lotus_ts_node_named_child_count", ts_count_ty, None);
        // declare i64 @lotus_ts_node_child(i64 node_id, i64 idx)
        let ts_child_ty =
            i64_t.fn_type(&[i64_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_ts_node_child", ts_child_ty, None);
        // declare i64 @lotus_ts_node_named_child(i64 node_id, i64 idx)
        self.module
            .add_function("lotus_ts_node_named_child", ts_child_ty, None);
        // declare i64 @lotus_ts_node_start_byte(i64 node_id)
        self.module
            .add_function("lotus_ts_node_start_byte", ts_count_ty, None);
        // declare i64 @lotus_ts_node_end_byte(i64 node_id)
        self.module
            .add_function("lotus_ts_node_end_byte", ts_count_ty, None);
        // declare ptr @lotus_ts_node_text(i64 node_id)
        self.module
            .add_function("lotus_ts_node_text", ts_kind_ty, None);
        // declare i64 @lotus_ts_node_is_named(i64 node_id)
        self.module
            .add_function("lotus_ts_node_is_named", ts_count_ty, None);

        // libm Float primitives — std::math::{sqrt, exp, log, floor,
        // ceil} (single-arg) and std::math::pow (two-arg). Each is a
        // straight pass-through to libm; the link line already pulls
        // libm transitively via libc on Linux. Resolves
        // notes/hale-friction.md 2026-05-10 float-surface-gaps
        // (the `std::math` sub-bullet). v0 cut is the six fns above;
        // sin/cos/tan/atan2 etc. come in a follow-up when a workload
        // surfaces the need.
        let f64_t = self.context.f64_type();
        let math_unary_ty = f64_t.fn_type(&[f64_t.into()], false);
        let math_binary_ty = f64_t.fn_type(&[f64_t.into(), f64_t.into()], false);
        self.module.add_function("sqrt", math_unary_ty, None);
        self.module.add_function("exp", math_unary_ty, None);
        self.module.add_function("log", math_unary_ty, None);
        self.module.add_function("floor", math_unary_ty, None);
        self.module.add_function("ceil", math_unary_ty, None);
        self.module.add_function("pow", math_binary_ty, None);
        // iris F.9 (2026-05-23): trig surface for spatial code
        // (lotus_viz, polar→cartesian, animation phase). Each
        // is a direct libm extern, mirroring sqrt / exp / log
        // shape. Float-typed at the Hale surface.
        self.module.add_function("sin", math_unary_ty, None);
        self.module.add_function("cos", math_unary_ty, None);
        self.module.add_function("tan", math_unary_ty, None);
        self.module.add_function("asin", math_unary_ty, None);
        self.module.add_function("acos", math_unary_ty, None);
        self.module.add_function("atan", math_unary_ty, None);
        self.module.add_function("atan2", math_binary_ty, None);

        // C8 (pond follow-up): IEEE 754 surface — tanh + NaN /
        // inf sentinels + `is_nan` predicate. `tanh` resolves
        // through a direct LLVM extern (mirroring sqrt / exp /
        // log / floor / ceil / pow above) rather than a C-runtime
        // wrapper — that keeps libm an *on-demand* dependency
        // (any binary that doesn't actually call tanh from
        // user code stays free of the unresolved libm reference,
        // which matters for the test helper binaries that link
        // lotus_arena.c without `-lm`). `nan` / `inf` / `is_nan`
        // are wrapped as `lotus_math_*` C primitives because they
        // don't reference libm at all — they use `<math.h>`
        // compile-time macros + the `f != f` test. `nan` / `inf`
        // are nullary; `is_nan` returns i32 (0/1), truncated to
        // i1 at the call site mirroring the file_exists pattern.
        // Resolves the pond/ml/neural hand-rolled tanh + pond/
        // math/matrix synthesised NaN sentinels.
        // declare double @tanh(double)
        self.module.add_function("tanh", math_unary_ty, None);
        // declare double @lotus_math_nan()
        // declare double @lotus_math_inf()
        let math_nullary_f64_ty = f64_t.fn_type(&[], false);
        self.module
            .add_function("lotus_math_nan", math_nullary_f64_ty, None);
        self.module
            .add_function("lotus_math_inf", math_nullary_f64_ty, None);
        // declare i32 @lotus_math_is_nan(double)
        let i32_t = self.context.i32_type();
        let math_is_nan_ty = i32_t.fn_type(&[f64_t.into()], false);
        self.module
            .add_function("lotus_math_is_nan", math_is_nan_ty, None);
    }

}
