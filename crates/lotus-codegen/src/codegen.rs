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
use inkwell::types::StructType;
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, FunctionValue, PointerValue,
};
use inkwell::{AddressSpace, OptimizationLevel};

use lotus_syntax::ast::*;

/// Compile-time tag for a value's type. Mirrors a small subset
/// of `lotus_types::Ty`; we don't pull the full type system in
/// because codegen only needs to discriminate the lowered
/// shapes (Int/Float/Bool/Duration are scalar i64/f64/i1/i64;
/// String is a ptr to a NUL-terminated byte array; LocusRef is
/// a ptr to a locus's struct, name-tagged so field access
/// resolves to the right `getelementptr`).
///
/// `Duration` is logically an i64 nanosecond count, distinct from
/// `Int` so type-driven dispatch (e.g. `time::sleep` accepts only
/// Duration) stays correct at the codegen layer.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LotusType {
    Int,
    Float,
    Bool,
    String,
    Duration,
    /// 64-bit decimal value. v0 codegen stores this as `f64` (same
    /// hack the interpreter uses — `parse_decimal` calls
    /// `s.parse::<f64>()`). Distinct from `Float` at the type level
    /// so type-checking stays strict; same LLVM lowering. A real
    /// fixed-point or arbitrary-precision representation lands
    /// later when Decimal precision actually matters.
    Decimal,
    /// Wall-clock instant. v0 codegen stores this as a pointer to
    /// the literal's source-spelling string (same hack the
    /// interpreter uses). Real `time_t` / `i64`-since-epoch
    /// lowering lands later. Distinct from `String` at the type
    /// level so the typechecker keeps `Time` and `String` apart.
    Time,
    /// Pointer to a locus's struct. The string carries the locus
    /// name; the layout + field map live in `Cx.user_loci`. Used
    /// for the child param of `accept(g: ChildLocus)`.
    LocusRef(String),
    /// Pointer to a user-defined type's struct (`type T { ... }`).
    /// The string carries the type name; the layout + field map
    /// live in `Cx.user_types`. Used for type-literal expressions
    /// like `Greeting { text: "hi", ... }` and for field access on
    /// values bound from those literals.
    TypeRef(String),
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
        current_user_fn_ret: None,
        current_self: None,
        loops: Vec::new(),
        user_fns: BTreeMap::new(),
        user_loci: BTreeMap::new(),
        user_types: BTreeMap::new(),
        bus_state: None,
        deferred_dissolves: Vec::new(),
        in_main: false,
        current_arena_override: None,
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

    // Drop the lotus runtime C source next to the object file so
    // clang compiles + links it into the same binary. The C
    // source is bundled into the codegen crate via include_str!,
    // so the lotus binary is self-contained — no separate
    // runtime install needed. Name is keyed off the object path
    // so parallel `cargo test` invocations don't race on a
    // shared filename in `/tmp`.
    let runtime_c_path = obj_path.with_extension("arena.c");
    std::fs::write(&runtime_c_path, RUNTIME_C_SOURCE)
        .map_err(|e| CodegenError::Link(format!("write runtime C: {}", e)))?;

    let status = Command::new("clang")
        .arg(&obj_path)
        .arg(&runtime_c_path)
        .arg("-O2")
        .arg("-o")
        .arg(output_path)
        .status()
        .map_err(|e| CodegenError::Link(format!("clang invocation: {}", e)))?;
    let _ = std::fs::remove_file(&obj_path);
    let _ = std::fs::remove_file(&runtime_c_path);
    if !status.success() {
        return Err(CodegenError::Link(format!(
            "clang exited with {}",
            status
        )));
    }
    Ok(())
}

/// The lotus runtime C source, bundled at compile time so the
/// codegen path is self-contained. Defines `lotus_arena_create`,
/// `lotus_arena_alloc`, `lotus_arena_destroy` — replacements for
/// libc malloc/free that respect the framework's region model
/// (m19 introduces the substrate; m20+ wires it to locus
/// lifetimes).
const RUNTIME_C_SOURCE: &str = include_str!("../runtime/lotus_arena.c");

struct Cx<'ctx, 'p> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: inkwell::builder::Builder<'ctx>,
    program: &'p Program,
    /// Set while lowering a function's body so that `if` / `while`
    /// can `append_basic_block` onto it.
    current_fn: Option<FunctionValue<'ctx>>,
    /// Return type of the user-defined fn currently being lowered,
    /// for typechecking `return` statements at codegen time. Outer
    /// `None` means we're in `main` (the C entry point) or outside
    /// any user fn — `return` is rejected there. Inner `None` means
    /// the user fn has no return type (void return).
    current_user_fn_ret: Option<Option<LotusType>>,
    /// Set while lowering a locus lifecycle method body so that
    /// `self.X` reads/writes lower to GEP+load/store on the struct
    /// pointer passed as the method's first arg.
    current_self: Option<SelfCx<'ctx>>,
    /// Stack of enclosing loops so `break` / `continue` can find
    /// their target blocks.
    loops: Vec<LoopFrame<'ctx>>,
    /// User-defined fns indexed by name. Filled in pass 1 of
    /// `lower_program` so call sites can refer to fns declared
    /// later in the same file.
    user_fns: BTreeMap<String, FnSig<'ctx>>,
    /// User-defined loci indexed by name. Filled in pass A of
    /// `lower_program`; carries the LLVM struct type for the
    /// locus's params + the lifecycle methods compiled against it.
    user_loci: BTreeMap<String, LocusInfo<'ctx>>,
    /// User-defined `type` declarations indexed by name. Filled
    /// in pass A0 of `lower_program`; carries the LLVM struct
    /// type and field map for plain data records (no methods).
    /// Used for type literals like `Point { x: 3, y: 4 }`.
    user_types: BTreeMap<String, TypeInfo<'ctx>>,
    /// Bus state generated when any locus declares a subscribe.
    /// `entries` is a fixed-size array global of `(subject_ptr,
    /// self_ptr, handler_ptr)` triples; `count` tracks how many
    /// have been registered at runtime. None means no subscribes
    /// in the program — `<-` becomes a no-op.
    bus_state: Option<BusState<'ctx>>,
    /// Stack of "deferred-dissolve" frames: each enclosing fn
    /// body / lifecycle method body opens one. Long-lived loci
    /// (any locus with a `bus subscribe` declaration) instantiated
    /// inside that body are pushed here instead of dissolving
    /// immediately, then drained + dissolved in reverse order at
    /// scope exit so they outlive synchronous publishes.
    deferred_dissolves: Vec<Vec<(PointerValue<'ctx>, String)>>,
    /// True while lowering the body of `main`. `return` is treated
    /// as an exit-code return (truncated to i32) when this is set,
    /// rather than the user-fn `current_user_fn_ret` path.
    in_main: bool,
    /// When set, `arena_alloc` routes through this arena pointer
    /// instead of `current_self`'s arena field or the program
    /// global. Used during locus-instantiation field init so
    /// composite literals (`TradeKernel { ... }`) used as
    /// default-init values land in the *new* locus's arena
    /// rather than the parent's. Restored after the field-init
    /// loop completes.
    current_arena_override: Option<PointerValue<'ctx>>,
}

#[derive(Debug, Clone, Copy)]
struct BusState<'ctx> {
    /// `[N x { ptr, ptr, ptr }]` global, all-zero-initialized.
    entries: inkwell::values::GlobalValue<'ctx>,
    /// `i64` global, initialized to 0; tracks current entry count.
    count: inkwell::values::GlobalValue<'ctx>,
    /// Capacity baked into `entries` array.
    capacity: u64,
    /// `void (ptr subject, ptr payload)` — the per-program dispatch
    /// fn body emitted once after pass A.
    dispatch_fn: FunctionValue<'ctx>,
}

#[derive(Debug, Clone)]
struct FnSig<'ctx> {
    func: FunctionValue<'ctx>,
    params: Vec<LotusType>,
    /// `None` = void (no return type in the lotus declaration).
    ret: Option<LotusType>,
}

/// Compiled locus type. Lifecycle methods take `self_ptr` as their
/// first arg; field access in their bodies lowers to GEPs against
/// `struct_ty` using the index from `fields`.
#[derive(Debug, Clone)]
struct LocusInfo<'ctx> {
    struct_ty: StructType<'ctx>,
    /// Field name → (index in struct, field type).
    fields: BTreeMap<String, (u32, LotusType)>,
    /// Field initializers in declaration order. Each entry is
    /// (name, default_init). Overrides at instantiation sites
    /// replace the default for that field. Default-init can be a
    /// pre-resolved literal (so simple defaults stay cheap) OR a
    /// deferred AST expression evaluated at the instantiation
    /// site (for composite literals like
    /// `current_kernel: TradeKernel = TradeKernel { ... }` where
    /// the default isn't a scalar literal).
    defaults: Vec<(String, DefaultInit)>,
    /// Lifecycle method LLVM functions, keyed by lifecycle name
    /// ("birth", "accept", "run"). drain / dissolve wait on the
    /// scheduler + recovery work.
    methods: BTreeMap<&'static str, FunctionValue<'ctx>>,
    /// For loci that declare `accept(child: ChildLocus)`, the
    /// child param's (binding name, child locus name). None for
    /// loci without an accept method. Used by both accept body
    /// lowering and child-instantiation sites (which must call
    /// parent.accept before child.birth, per F.7).
    accept_param: Option<(String, String)>,
    /// User-defined `fn` members on the locus (called as bus
    /// handlers, mode dispatchers, etc.). Each entry is
    /// (method name → LLVM function value). Methods take
    /// `self_ptr` plus their declared params.
    user_methods: BTreeMap<String, FunctionValue<'ctx>>,
    /// Each `bus subscribe "S" as h ...` declaration on this
    /// locus: (subject_literal, handler_method_name). At
    /// instantiation time, registration emits a triple
    /// (subject_str, self_ptr, handler_fn_ptr) into the global
    /// bus table.
    subscriptions: Vec<(String, String)>,
    /// Closure declarations on this locus that fire at the
    /// dissolve epoch (the default). v0 codegen lowers ONLY
    /// dissolve-epoch closures; tick / duration / birth / explicit
    /// epochs are typechecked but rejected at lowering time. Each
    /// element is a `(name, ClosureAssertion)` pair carried over
    /// from the AST so the synthetic `__closures` fn body can
    /// re-lower the assertion expressions.
    closures: Vec<(String, ClosureAssertion)>,
    /// Synthetic `<Locus>.__closures(self_ptr, parent_self_or_null,
    /// on_failure_fn_or_null)` fn that evaluates every
    /// dissolve-epoch closure. None when `closures` is empty —
    /// saves the indirect call when the locus has nothing to
    /// check. Called between drain() and dissolve() per F.4 + F.9.
    closures_fn: Option<FunctionValue<'ctx>>,
    /// `on_failure(child: ChildL, err: ClosureViolation)` handler
    /// declared on this locus, if any. Each parent has at most
    /// one handler (per FailureDecl AST shape); the handler's
    /// first param's type names the single child locus it accepts.
    /// Stored as (child_locus_name, llvm_fn). When a child of
    /// matching type fails its closure, the runtime routes the
    /// violation to this fn instead of dprintf+exit.
    failure_handler: Option<(String, FunctionValue<'ctx>)>,
    /// When this locus declares `accept(child: T)`, every accept
    /// dispatch appends the child's self_ptr to a built-in
    /// fixed-cap array embedded in the locus struct so
    /// `for child in self.children { ... }` can iterate. Indexes
    /// of the synthetic array + counter fields if the locus
    /// declares accept. None for accept-less loci.
    children_field_idx: Option<u32>,
    /// Index of the `i64 child_count` field. Always paired with
    /// `children_field_idx`.
    child_count_field_idx: Option<u32>,
    /// Index of the synthetic `__arena: ptr` field carrying this
    /// locus's `lotus_arena_t*`. Always 0 — the arena field is
    /// the first slot in every locus struct so bus dispatch can
    /// GEP-load it from a type-erased self pointer (m20: bus
    /// payload copy semantics need the subscriber's arena, and
    /// the dispatch fn doesn't know which locus type its
    /// self_ptr came from).
    arena_field_idx: u32,
}

/// Maximum number of children any locus struct's built-in
/// `children` array can hold. v0 codegen uses a fixed cap to
/// avoid resize / heap dance; trellis-grade loci typically have
/// O(few-dozen) coordinatees, and 04-modes' AggregatorL only
/// instantiates 3.
const CHILDREN_CAP: u32 = 16;

/// One locus param's default-initializer. Either pre-resolved
/// (the common case — scalar literal) or deferred to the
/// instantiation site (composite literal like
/// `TradeKernel { ... }` whose evaluation needs the codegen
/// builder).
#[derive(Debug, Clone)]
enum DefaultInit {
    Const(ParamValue),
    Expr(Expr),
}

/// Compiled user `type` (a plain data record). No methods, no
/// lifecycle; just the struct type + field map. Field access
/// lowers to GEP+load; instantiation lowers to alloca + per-field
/// store + return-the-pointer.
#[derive(Debug, Clone)]
struct TypeInfo<'ctx> {
    struct_ty: StructType<'ctx>,
    /// Field name → (index in struct, field type).
    fields: BTreeMap<String, (u32, LotusType)>,
    /// Field declaration order; needed because a struct literal
    /// might list fields in a different order than the type
    /// declaration, and field stores still go to the right
    /// indexed slot.
    field_order: Vec<String>,
}

/// Carried on `Cx` while lowering a lifecycle method body so
/// `self.X` reads/writes resolve to GEPs against the LLVM struct,
/// and so child instantiations inside the body can look up the
/// parent's accept method.
#[derive(Debug, Clone)]
struct SelfCx<'ctx> {
    locus_name: String,
    struct_ty: StructType<'ctx>,
    self_ptr: PointerValue<'ctx>,
    fields: BTreeMap<String, (u32, LotusType)>,
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
        let arena_create_ty = ptr_t.fn_type(&[], false);
        self.module
            .add_function("lotus_arena_create", arena_create_ty, None);
        let arena_alloc_ty =
            ptr_t.fn_type(&[ptr_t.into(), i64_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_arena_alloc", arena_alloc_ty, None);
        let arena_destroy_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_arena_destroy", arena_destroy_ty, None);

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

        // declare i32 @fflush(ptr)
        //
        // Used by bubble() right before the dprintf-to-stderr so
        // any pending stdout output (from prior println calls in
        // an on_failure handler) flushes BEFORE the violation
        // report writes to fd 2, matching the interpreter's
        // observable output order.
        let fflush_ty = i32_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function("fflush", fflush_ty, None);

        // declare ptr @memcpy(ptr dest, ptr src, i64 n)
        //
        // Used by `bus_dispatch` to copy the publisher's payload
        // into a fresh allocation in the subscriber's arena before
        // invoking the handler — per spec/memory.md "A typed
        // message crossing a locus boundary is a copy, not a
        // pointer." Standard libc surface; we don't use LLVM's
        // intrinsic memcpy because clang lowers it through the
        // libc symbol anyway and a normal call is easier to
        // reason about.
        let memcpy_ty =
            ptr_t.fn_type(&[ptr_t.into(), ptr_t.into(), i64_t.into()], false);
        self.module.add_function("memcpy", memcpy_ty, None);
    }

    /// LLVM struct type for one entry in the bus subscription
    /// table: `{ ptr subject, ptr self, ptr handler }`. With LLVM
    /// 18 opaque pointers the per-element type only matters for
    /// allocation + GEP indexing.
    fn bus_entry_type(&self) -> inkwell::types::StructType<'ctx> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(&[ptr_t.into(), ptr_t.into(), ptr_t.into()], false)
    }

    /// Emit the bus subscription table + counter + dispatch fn.
    /// Called once per module after we know the total subscription
    /// count. Capacity is fixed at compile time — every subscribe
    /// declaration in source contributes one slot, and registration
    /// at locus instantiation just bumps the counter to fill it.
    fn init_bus_state(&mut self, capacity: u64) -> Result<(), CodegenError> {
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let void_t = self.context.void_type();
        let entry_ty = self.bus_entry_type();
        let table_ty = entry_ty.array_type(capacity as u32);

        let entries_global = self.module.add_global(table_ty, None, "bus.entries");
        entries_global.set_initializer(&table_ty.const_zero());
        entries_global.set_linkage(inkwell::module::Linkage::Internal);

        let count_global = self.module.add_global(i64_t, None, "bus.count");
        count_global.set_initializer(&i64_t.const_int(0, false));
        count_global.set_linkage(inkwell::module::Linkage::Internal);

        // void @bus_dispatch(ptr %subject, ptr %payload, i64 %size):
        //   for (i = 0; i < bus.count; i++)
        //     if (strcmp(bus.entries[i].subject, %subject) == 0)
        //       sub_self  = bus.entries[i].self
        //       sub_arena = load (sub_self + 0)             ; arena field is slot 0
        //       copy      = lotus_arena_alloc(sub_arena, size, 8)
        //       memcpy(copy, %payload, %size)
        //       bus.entries[i].handler(sub_self, copy)
        //
        // Per spec/memory.md: "A typed message crossing a locus
        // boundary is a copy, not a pointer." The publisher owns
        // the original payload (in its arena); the subscriber gets
        // a copy in its own arena, freed when the subscriber
        // dissolves. This decouples publisher / subscriber
        // lifetimes — exactly what trellis-demo's
        // `self.current_kernel = msg` pattern needs to be safe.
        let dispatch_ty = void_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), i64_t.into()],
            false,
        );
        let dispatch_fn = self
            .module
            .add_function("lotus.bus_dispatch", dispatch_ty, None);
        let entry_bb = self.context.append_basic_block(dispatch_fn, "entry");
        let header_bb =
            self.context.append_basic_block(dispatch_fn, "loop.header");
        let check_bb =
            self.context.append_basic_block(dispatch_fn, "loop.check");
        let call_bb =
            self.context.append_basic_block(dispatch_fn, "loop.call");
        let inc_bb = self.context.append_basic_block(dispatch_fn, "loop.inc");
        let done_bb = self.context.append_basic_block(dispatch_fn, "done");

        self.builder.position_at_end(entry_bb);
        let i_slot = self
            .builder
            .build_alloca(i64_t, "i.slot")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(i_slot, i64_t.const_int(0, false))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(header_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(header_bb);
        let count = self
            .builder
            .build_load(i64_t, count_global.as_pointer_value(), "count")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let i = self
            .builder
            .build_load(i64_t, i_slot, "i")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let in_range = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::ULT,
                i,
                count,
                "in.range",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(in_range, check_bb, done_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // check_bb: load entry[i].subject; strcmp; branch
        self.builder.position_at_end(check_bb);
        let subj_param = dispatch_fn
            .get_nth_param(0)
            .expect("subject param")
            .into_pointer_value();
        let subj_slot_ptr = unsafe {
            self.builder
                .build_gep(
                    table_ty,
                    entries_global.as_pointer_value(),
                    &[i64_t.const_int(0, false), i, i32_t.const_int(0, false)],
                    "entry.subject.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        let subj_loaded = self
            .builder
            .build_load(ptr_t, subj_slot_ptr, "entry.subject")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        let strcmp_fn = self
            .module
            .get_function("strcmp")
            .expect("strcmp declared in declare_builtins");
        let cmp = self
            .builder
            .build_call(
                strcmp_fn,
                &[subj_loaded.into(), subj_param.into()],
                "subj.cmp",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let cmp_int = cmp
            .try_as_basic_value()
            .left()
            .expect("strcmp returns i32")
            .into_int_value();
        let is_match = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                cmp_int,
                i32_t.const_int(0, false),
                "is.match",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(is_match, call_bb, inc_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // call_bb: load self + handler, call handler(self, payload)
        self.builder.position_at_end(call_bb);
        let self_slot_ptr = unsafe {
            self.builder
                .build_gep(
                    table_ty,
                    entries_global.as_pointer_value(),
                    &[i64_t.const_int(0, false), i, i32_t.const_int(1, false)],
                    "entry.self.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        let entry_self = self
            .builder
            .build_load(ptr_t, self_slot_ptr, "entry.self")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        let handler_slot_ptr = unsafe {
            self.builder
                .build_gep(
                    table_ty,
                    entries_global.as_pointer_value(),
                    &[i64_t.const_int(0, false), i, i32_t.const_int(2, false)],
                    "entry.handler.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        let handler = self
            .builder
            .build_load(ptr_t, handler_slot_ptr, "entry.handler")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        let payload_param = dispatch_fn
            .get_nth_param(1)
            .expect("payload param")
            .into_pointer_value();
        let size_param = dispatch_fn
            .get_nth_param(2)
            .expect("size param")
            .into_int_value();

        // Copy payload into the subscriber's arena. The arena
        // field is at struct offset 0 on every locus type — so
        // we can pull it out of the type-erased self_ptr without
        // knowing which locus we're talking to. Allocate `size`
        // bytes there, memcpy from the publisher's pointer, pass
        // the COPY to the handler.
        let sub_arena = self
            .builder
            .build_load(ptr_t, entry_self, "sub.__arena")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        let arena_alloc_fn = self
            .module
            .get_function("lotus_arena_alloc")
            .expect("lotus_arena_alloc declared");
        let align = i64_t.const_int(8, false);
        let copy_raw = self
            .builder
            .build_call(
                arena_alloc_fn,
                &[sub_arena.into(), size_param.into(), align.into()],
                "payload.copy",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("arena_alloc returns ptr");
        let copy_ptr = copy_raw.into_pointer_value();
        let memcpy_fn = self
            .module
            .get_function("memcpy")
            .expect("memcpy declared");
        self.builder
            .build_call(
                memcpy_fn,
                &[copy_ptr.into(), payload_param.into(), size_param.into()],
                "payload.memcpy",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let handler_callee_ty =
            void_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.builder
            .build_indirect_call(
                handler_callee_ty,
                handler,
                &[entry_self.into(), copy_ptr.into()],
                "handler.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(inc_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // inc_bb: i++; branch to header
        self.builder.position_at_end(inc_bb);
        let i_now = self
            .builder
            .build_load(i64_t, i_slot, "i.now")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let i_next = self
            .builder
            .build_int_add(i_now, i64_t.const_int(1, false), "i.next")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(i_slot, i_next)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(header_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // done_bb: ret
        self.builder.position_at_end(done_bb);
        self.builder
            .build_return(None)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.bus_state = Some(BusState {
            entries: entries_global,
            count: count_global,
            capacity,
            dispatch_fn,
        });
        Ok(())
    }

    /// Resolve the (parent_self, on_failure_fn) pair for a child
    /// of `child_locus_name` whose closure may fail at dissolve.
    /// Reads `current_self` (set while we're in the parent's
    /// lifecycle body) and that parent's `failure_handler`. If
    /// the parent declares an on_failure that takes this child
    /// type, returns the parent's self_ptr + the handler fn ptr.
    /// Otherwise returns (null, null) — the closure-fail path
    /// will fall back to the v0 dprintf+exit report.
    fn resolve_failure_route(
        &self,
        child_locus_name: &str,
    ) -> (PointerValue<'ctx>, PointerValue<'ctx>) {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let null_ptr = ptr_t.const_null();
        let Some(cs) = self.current_self.as_ref() else {
            return (null_ptr, null_ptr);
        };
        let Some(parent_info) = self.user_loci.get(&cs.locus_name) else {
            return (null_ptr, null_ptr);
        };
        let Some((expected_child, handler_fn)) =
            parent_info.failure_handler.as_ref()
        else {
            return (null_ptr, null_ptr);
        };
        if expected_child != child_locus_name {
            return (null_ptr, null_ptr);
        }
        (
            cs.self_ptr,
            handler_fn.as_global_value().as_pointer_value(),
        )
    }

    /// Push a fresh deferred-dissolve frame onto the stack. Each
    /// fn body / lifecycle method body opens one at entry and
    /// flushes at exit so long-lived loci instantiated inside it
    /// outlive synchronous publishes within the same body.
    fn push_dissolve_frame(&mut self) {
        self.deferred_dissolves.push(Vec::new());
    }

    /// Pop the top deferred-dissolve frame and emit its drain →
    /// dissolve calls in reverse instantiation order. Called just
    /// before the body's final `ret` so the alloca slots are still
    /// live when their drain/dissolve methods read self.X.
    fn flush_dissolve_frame(&mut self) -> Result<(), CodegenError> {
        let frame = self
            .deferred_dissolves
            .pop()
            .expect("flush without matching push");
        for (self_ptr, locus_name) in frame.into_iter().rev() {
            let info = self
                .user_loci
                .get(&locus_name)
                .cloned()
                .expect("deferred locus declared");
            // drain → __closures → dissolve, mirroring the
            // ephemeral-locus ordering. Long-lived loci dissolve
            // here at scope-exit; the cascade itself ran each
            // descendant's closures during the descendant's own
            // ephemeral-dissolve / scope-exit.
            if let Some(drain_fn) = info.methods.get("drain") {
                self.builder
                    .build_call(
                        *drain_fn,
                        &[self_ptr.into()],
                        &format!("{}.drain.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            if let Some(closures_fn) = info.closures_fn {
                let (parent_self, handler_ptr) =
                    self.resolve_failure_route(&locus_name);
                self.builder
                    .build_call(
                        closures_fn,
                        &[
                            self_ptr.into(),
                            parent_self.into(),
                            handler_ptr.into(),
                        ],
                        &format!("{}.__closures.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            if let Some(dissolve_fn) = info.methods.get("dissolve") {
                self.builder
                    .build_call(
                        *dissolve_fn,
                        &[self_ptr.into()],
                        &format!("{}.dissolve.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            // Long-lived locus arena released at scope exit, after
            // its dissolve has run. Symmetric with the ephemeral
            // path in lower_locus_instantiation.
            self.emit_locus_arena_destroy(&info, self_ptr, &locus_name)?;
        }
        Ok(())
    }

    /// Emit a single subscription registration:
    ///   bus.entries[bus.count] = { subject_str, self_ptr, handler_fn }
    ///   bus.count += 1
    /// Called once per `bus subscribe` declaration when its locus
    /// is instantiated.
    fn emit_bus_register(
        &mut self,
        subject: &str,
        self_ptr: PointerValue<'ctx>,
        handler_fn: FunctionValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let bus = self
            .bus_state
            .expect("subscriptions registered ⇒ bus_state initialized");
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let entry_ty = self.bus_entry_type();
        let table_ty = entry_ty.array_type(bus.capacity as u32);

        let count = self
            .builder
            .build_load(i64_t, bus.count.as_pointer_value(), "bus.count.cur")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let subj_str = self.global_string(subject);
        let handler_ptr = handler_fn.as_global_value().as_pointer_value();

        // entries[count].subject = subj_str
        let subj_slot = unsafe {
            self.builder
                .build_gep(
                    table_ty,
                    bus.entries.as_pointer_value(),
                    &[i64_t.const_int(0, false), count, i32_t.const_int(0, false)],
                    "reg.subject.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        self.builder
            .build_store(subj_slot, subj_str)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // entries[count].self = self_ptr
        let self_slot = unsafe {
            self.builder
                .build_gep(
                    table_ty,
                    bus.entries.as_pointer_value(),
                    &[i64_t.const_int(0, false), count, i32_t.const_int(1, false)],
                    "reg.self.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        self.builder
            .build_store(self_slot, self_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // entries[count].handler = handler_ptr
        let handler_slot = unsafe {
            self.builder
                .build_gep(
                    table_ty,
                    bus.entries.as_pointer_value(),
                    &[i64_t.const_int(0, false), count, i32_t.const_int(2, false)],
                    "reg.handler.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        self.builder
            .build_store(handler_slot, handler_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // bus.count = count + 1
        let next = self
            .builder
            .build_int_add(count, i64_t.const_int(1, false), "bus.count.next")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(bus.count.as_pointer_value(), next)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
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
                TopDecl::Fn(f) if f.name.name == "main" => Some(f.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                CodegenError::Unsupported("program has no `fn main()`".to_string())
            })?;

        // Pre-pass: register the built-in `ClosureViolation` type.
        // The interpreter exposes this as a Value::Struct with
        // fields { locus, closure, left, right, tolerance, diff };
        // codegen v0 only carries `locus` and `closure` (both
        // String) since the dynamic-typed left/right/diff fields
        // would need polymorphic record support. on_failure
        // handlers can therefore read err.locus and err.closure.
        self.declare_builtin_closure_violation_type();

        // Pass A0: declare every user-defined `type` so locus
        // params, fn signatures, and struct literals can reference
        // them by name regardless of source order. Plain data
        // records — no methods, no lifecycle.
        let type_decls: Vec<TypeDecl> = self
            .program
            .items
            .iter()
            .filter_map(|item| match item {
                TopDecl::Type(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        for t in &type_decls {
            self.declare_user_type(t)?;
        }

        // Pass A: declare each user-defined locus. Split in two so
        // accept's child-locus param can resolve regardless of the
        // declaration order in source:
        //   A1: every locus's struct type + field layout
        //   A2: every locus's lifecycle method signatures
        let locus_decls: Vec<LocusDecl> = self
            .program
            .items
            .iter()
            .filter_map(|item| match item {
                TopDecl::Locus(l) => Some(l.clone()),
                _ => None,
            })
            .collect();
        for l in &locus_decls {
            self.declare_locus_struct(l)?;
        }
        for l in &locus_decls {
            self.declare_locus_methods(l)?;
        }

        // After A2: if any locus declared a `bus subscribe`,
        // emit the bus globals + the linear-scan dispatch fn.
        // The dispatch fn body is generated before any call site
        // can need it; the globals' capacity is baked from the
        // total subscription count across all loci.
        let total_subs: u64 = self
            .user_loci
            .values()
            .map(|info| info.subscriptions.len() as u64)
            .sum();
        if total_subs > 0 {
            self.init_bus_state(total_subs)?;
        }

        // Pass B: declare every user-defined function so call sites
        // can refer to fns declared later in the file.
        let user_fn_decls: Vec<FnDecl> = self
            .program
            .items
            .iter()
            .filter_map(|item| match item {
                TopDecl::Fn(f) if f.name.name != "main" => Some(f.clone()),
                _ => None,
            })
            .collect();
        for f in &user_fn_decls {
            self.declare_user_fn(f)?;
        }

        // Pass C: lower lifecycle method bodies (birth, run, ...).
        for l in &locus_decls {
            self.lower_locus_method_bodies(l)?;
        }

        // Pass D: lower bodies of user-defined fns.
        for f in &user_fn_decls {
            self.lower_user_fn_body(f)?;
        }

        // Pass 3: the C entry point — i32 @main(). Always void-arg
        // and i32-return at the LLVM ABI level, regardless of what
        // the user wrote (lotus's "main returns Int = exit code"
        // semantics map onto this; explicit `return` from main is
        // not yet implemented in codegen).
        let i32_t = self.context.i32_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let main_ty = i32_t.fn_type(&[], false);
        let main_fn = self.module.add_function("main", main_ty, None);
        let entry = self.context.append_basic_block(main_fn, "entry");
        self.builder.position_at_end(entry);
        self.current_fn = Some(main_fn);
        self.current_user_fn_ret = None;
        self.current_self = None;
        self.in_main = true;
        self.push_dissolve_frame();

        // Prelude: spin up the program-wide arena. Every
        // `arena_alloc` call site loads `@lotus.arena.global`, so
        // this store has to happen before any user code runs.
        let arena_create = self
            .module
            .get_function("lotus_arena_create")
            .expect("lotus_arena_create declared");
        let arena_global = self
            .module
            .get_global("lotus.arena.global")
            .expect("arena global declared");
        let arena_ptr = self
            .builder
            .build_call(arena_create, &[], "arena.init")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("arena_create returns ptr");
        self.builder
            .build_store(arena_global.as_pointer_value(), arena_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let mut scope = Scope::default();
        let end = self.lower_block(&main_decl.body, &mut scope)?;

        // Only emit `ret 0` if the body fell through. If it ended
        // in a terminator (e.g. an unreachable `if` whose branches
        // both `break`/`return`), the trailing block is already
        // closed and writing more IR is unsound.
        if end == BlockEnd::Open {
            self.flush_dissolve_frame()?;
            // Tear down the arena before exit. exit(0) via `ret`
            // would drop the chunk linked list either way (process
            // exit reclaims everything), but going through
            // lotus_arena_destroy keeps this path equivalent to
            // the early-return path emitted in `lower_return` when
            // a user `return n;` from main runs.
            self.emit_arena_destroy()?;
            let zero = i32_t.const_int(0, false);
            self.builder
                .build_return(Some(&zero))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        } else {
            // Body terminated unconditionally — drop the frame
            // without emitting the dissolve calls. Any deferred
            // dissolves are unreachable.
            let _ = self.deferred_dissolves.pop();
        }
        let _ = ptr_t;
        self.in_main = false;
        self.current_fn = None;
        Ok(())
    }

    /// Emit a call to `lotus_arena_destroy(@lotus.arena.global)`.
    /// Used at every main-exit point so the arena tears down
    /// cleanly. (Matters most for tooling — e.g. valgrind /
    /// LeakSanitizer don't see chunks as leaked when the process
    /// returns; the OS reclaims either way.)
    fn emit_arena_destroy(&mut self) -> Result<(), CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let arena_global = self
            .module
            .get_global("lotus.arena.global")
            .expect("arena global declared");
        let arena_ptr = self
            .builder
            .build_load(ptr_t, arena_global.as_pointer_value(), "arena.destroy.cur")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let destroy = self
            .module
            .get_function("lotus_arena_destroy")
            .expect("lotus_arena_destroy declared");
        self.builder
            .build_call(destroy, &[arena_ptr.into()], "arena.destroy.call")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
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

    /// Map a `TypeExpr` to the codegen's `LotusType`. Scalar
    /// primitives + bare locus type names are supported; arrays /
    /// tuples / generics wait.
    fn type_expr_to_lotus(
        &self,
        t: &TypeExpr,
    ) -> Result<LotusType, CodegenError> {
        match t {
            TypeExpr::Primitive(p, _) => match p {
                PrimType::Int => Ok(LotusType::Int),
                PrimType::Float => Ok(LotusType::Float),
                PrimType::Bool => Ok(LotusType::Bool),
                PrimType::String => Ok(LotusType::String),
                PrimType::Duration => Ok(LotusType::Duration),
                PrimType::Decimal => Ok(LotusType::Decimal),
                PrimType::Time => Ok(LotusType::Time),
                other => Err(CodegenError::Unsupported(format!(
                    "type primitive `{:?}` in signature",
                    other
                ))),
            },
            TypeExpr::Named { path, generic_args, .. }
                if generic_args.is_empty() && path.segments.len() == 1 =>
            {
                let name = &path.segments[0].name;
                if self.user_loci.contains_key(name) {
                    Ok(LotusType::LocusRef(name.clone()))
                } else if self.user_types.contains_key(name) {
                    Ok(LotusType::TypeRef(name.clone()))
                } else {
                    Err(CodegenError::Unsupported(format!(
                        "unknown type name `{}` in signature",
                        name
                    )))
                }
            }
            other => Err(CodegenError::Unsupported(format!(
                "type form {:?} in signature",
                std::mem::discriminant(other)
            ))),
        }
    }

    /// Register the built-in `ClosureViolation` record so closure-
    /// failure handlers can take it as their second param. v0
    /// fields: `locus: String`, `closure: String`, `diff: Int`.
    /// The polymorphic `left / right / tolerance` fields the
    /// interpreter exposes wait on polymorphic record support.
    /// `diff` is i64; for Int/Duration closures it carries `left -
    /// right` directly; for Float/Decimal closures it carries 0
    /// (the value isn't a useful signed Int there anyway).
    fn declare_builtin_closure_violation_type(&mut self) {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i64_t = self.context.i64_type();
        let mut fields: BTreeMap<String, (u32, LotusType)> = BTreeMap::new();
        fields.insert("locus".into(), (0, LotusType::String));
        fields.insert("closure".into(), (1, LotusType::String));
        fields.insert("diff".into(), (2, LotusType::Int));
        let field_order = vec![
            "locus".to_string(),
            "closure".to_string(),
            "diff".to_string(),
        ];
        let llvm_field_tys: Vec<inkwell::types::BasicTypeEnum> =
            vec![ptr_t.into(), ptr_t.into(), i64_t.into()];
        let struct_ty = self
            .context
            .opaque_struct_type("type.ClosureViolation");
        struct_ty.set_body(&llvm_field_tys, false);
        self.user_types.insert(
            "ClosureViolation".to_string(),
            TypeInfo {
                struct_ty,
                fields,
                field_order,
            },
        );
    }

    /// Pass A0: declare a user `type` decl as an LLVM struct type.
    /// Aliases and enums are not yet lowered — only struct bodies.
    /// No defaults are expected (the language requires struct
    /// literals to provide every field at the call site).
    fn declare_user_type(&mut self, t: &TypeDecl) -> Result<(), CodegenError> {
        if !t.generics.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "generic type `{}`",
                t.name.name
            )));
        }
        let struct_fields = match &t.body {
            TypeDeclBody::Struct(fs) => fs,
            TypeDeclBody::Alias(_) => {
                return Err(CodegenError::Unsupported(format!(
                    "type alias `{}`: codegen v0 only lowers struct types",
                    t.name.name
                )));
            }
            TypeDeclBody::Enum(_) => {
                return Err(CodegenError::Unsupported(format!(
                    "enum type `{}`: codegen v0 only lowers struct types",
                    t.name.name
                )));
            }
        };

        let mut fields: BTreeMap<String, (u32, LotusType)> = BTreeMap::new();
        let mut field_order: Vec<String> = Vec::new();
        let mut llvm_field_tys: Vec<inkwell::types::BasicTypeEnum> =
            Vec::new();
        for (idx, f) in struct_fields.iter().enumerate() {
            let ft = self.type_expr_to_lotus(&f.ty)?;
            llvm_field_tys.push(self.llvm_basic_type(&ft));
            fields.insert(f.name.name.clone(), (idx as u32, ft));
            field_order.push(f.name.name.clone());
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
            },
        );
        Ok(())
    }

    /// Pass A: declare a locus's struct type + lifecycle method
    /// signatures. Body lowering happens later (pass C).
    ///
    /// Each lifecycle method takes the struct pointer as its first
    /// arg and returns void. accept additionally takes the child
    /// pointer.
    /// Pass A1: register a locus's struct type + field layout. Done
    /// before any method signatures (pass A2) so accept methods can
    /// reference any locus's struct type, regardless of declaration
    /// order in source.
    fn declare_locus_struct(
        &mut self,
        l: &LocusDecl,
    ) -> Result<(), CodegenError> {
        if !l.annotations.is_empty() {
            // tier / projection annotations are framework metadata,
            // not ABI — fine to ignore for codegen.
        }

        // Each locus param must have either a literal default or
        // a typed default expression evaluable at instantiation
        // time. Scalar literals lock in `DefaultInit::Const` so
        // const_param can build them directly; non-literal defaults
        // (like `current_kernel: TradeKernel = TradeKernel { ... }`)
        // get `DefaultInit::Expr` and are evaluated at the
        // instantiation site through lower_expr. Type ascription is
        // REQUIRED for non-literal defaults (we don't infer a type
        // from an arbitrary expression here — the AST resolver
        // doesn't run in codegen v0).
        let mut fields: BTreeMap<String, (u32, LotusType)> = BTreeMap::new();
        let mut defaults: Vec<(String, DefaultInit)> = Vec::new();
        let mut llvm_field_tys: Vec<inkwell::types::BasicTypeEnum> =
            Vec::new();

        // Synthetic `__arena: ptr` is *always* the first field
        // (index 0). m20+ allocations on behalf of a locus go to
        // this arena; bus dispatch's payload-copy step pulls it
        // out of the subscriber's self_ptr at runtime via a
        // fixed-offset GEP. Keeping it at idx 0 means the dispatch
        // fn doesn't need to know the subscriber's specific locus
        // type to find the arena.
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        llvm_field_tys.push(ptr_t.into());
        let arena_field_idx: u32 = 0;
        let mut idx: u32 = 1;

        for member in &l.members {
            if let LocusMember::Params(pb) = member {
                for p in &pb.params {
                    let default_expr = match &p.init {
                        ParamInit::Value(e) => e,
                        ParamInit::Inferred => {
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` param `{}`: codegen requires a \
                                 default value (literal or typed expression)",
                                l.name.name, p.name.name
                            )));
                        }
                    };
                    // Try to lock in as a literal Const first; fall
                    // back to deferred Expr if that fails.
                    let (default, default_ty): (DefaultInit, LotusType) =
                        match param_value(default_expr) {
                            Ok(pv) => {
                                let ty = match &pv {
                                    ParamValue::Int(_) => LotusType::Int,
                                    ParamValue::Float(_) => LotusType::Float,
                                    ParamValue::Bool(_) => LotusType::Bool,
                                    ParamValue::String(_) => LotusType::String,
                                    ParamValue::Duration(_) => {
                                        LotusType::Duration
                                    }
                                    ParamValue::Decimal(_) => LotusType::Decimal,
                                    ParamValue::Time(_) => LotusType::Time,
                                };
                                (DefaultInit::Const(pv), ty)
                            }
                            Err(_) => {
                                // Non-literal default → require an
                                // explicit type ascription so we
                                // know the field's LLVM shape
                                // without evaluating the default.
                                let ascribed = p.ty.as_ref().ok_or_else(|| {
                                    CodegenError::Unsupported(format!(
                                        "locus `{}` param `{}`: non-literal \
                                         default requires a type ascription",
                                        l.name.name, p.name.name
                                    ))
                                })?;
                                let ty = self.type_expr_to_lotus(ascribed)?;
                                (DefaultInit::Expr(default_expr.clone()), ty)
                            }
                        };
                    if let Some(ascribed) = &p.ty {
                        let asc_ty = self.type_expr_to_lotus(ascribed)?;
                        if asc_ty != default_ty {
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` param `{}`: declared {:?}, \
                                 default {:?}",
                                l.name.name, p.name.name, asc_ty, default_ty
                            )));
                        }
                    }
                    fields.insert(
                        p.name.name.clone(),
                        (idx, default_ty.clone()),
                    );
                    defaults.push((p.name.name.clone(), default));
                    llvm_field_tys.push(self.llvm_basic_type(&default_ty));
                    idx += 1;
                }
            }
        }

        // If this locus declares accept, append a synthetic
        // children array + counter at the end of the struct so
        // each accept dispatch can record the child's self_ptr
        // for `for child in self.children { ... }` iteration.
        let has_accept = l.members.iter().any(|m| {
            matches!(m, LocusMember::Lifecycle(lc)
                if matches!(lc.kind, LifecycleKind::Accept))
        });
        let (children_field_idx, child_count_field_idx) = if has_accept {
            let i64_t = self.context.i64_type();
            let arr_ty = ptr_t.array_type(CHILDREN_CAP);
            let arr_idx = idx;
            llvm_field_tys.push(arr_ty.into());
            idx += 1;
            let cnt_idx = idx;
            llvm_field_tys.push(i64_t.into());
            idx += 1;
            (Some(arr_idx), Some(cnt_idx))
        } else {
            (None, None)
        };
        let _ = idx;

        let struct_ty = self
            .context
            .opaque_struct_type(&format!("locus.{}", l.name.name));
        struct_ty.set_body(&llvm_field_tys, false);

        self.user_loci.insert(
            l.name.name.clone(),
            LocusInfo {
                struct_ty,
                fields,
                defaults,
                methods: BTreeMap::new(),
                accept_param: None,
                user_methods: BTreeMap::new(),
                subscriptions: Vec::new(),
                closures: Vec::new(),
                closures_fn: None,
                failure_handler: None,
                children_field_idx,
                child_count_field_idx,
                arena_field_idx,
            },
        );
        Ok(())
    }

    /// Pass A2: declare each lifecycle method's LLVM function
    /// signature. Runs after every locus's struct type exists, so
    /// accept's child-locus param can resolve regardless of
    /// declaration order.
    ///
    /// Accepted lifecycle methods (codegen v0):
    /// - `birth(self_ptr)` — runs after instantiation fills fields
    /// - `accept(parent_self_ptr, child_ptr)` — runs once per child,
    ///   before that child's own `birth` (per F.7)
    /// - `run(self_ptr)` — runs after `birth`
    /// - `drain(self_ptr)` — runs after `run`, before `dissolve`,
    ///   after the body's child loci have already finished their
    ///   own drain/dissolve sequence (F.4 depth-first cascade)
    /// - `dissolve(self_ptr)` — runs last, before the alloca dies
    fn declare_locus_methods(
        &mut self,
        l: &LocusDecl,
    ) -> Result<(), CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let void_t = self.context.void_type();
        let mut methods: BTreeMap<&'static str, FunctionValue<'ctx>> =
            BTreeMap::new();
        let mut accept_param: Option<(String, String)> = None;
        let mut user_methods: BTreeMap<String, FunctionValue<'ctx>> =
            BTreeMap::new();
        let mut subscriptions: Vec<(String, String)> = Vec::new();
        let mut closures: Vec<(String, ClosureAssertion)> = Vec::new();
        let mut failure_handler: Option<(String, FunctionValue<'ctx>)> = None;
        for member in &l.members {
            match member {
                LocusMember::Params(_) | LocusMember::Contract(_) => {
                    // Params handled in pass A1; contracts are a
                    // typecheck-only feature with no codegen ABI.
                }
                LocusMember::Lifecycle(lc) => {
                    if lc.ret.is_some() {
                        return Err(CodegenError::Unsupported(format!(
                            "locus `{}` lifecycle `{:?}` declares a return \
                             type; only void is supported in v0",
                            l.name.name, lc.kind
                        )));
                    }
                    match lc.kind {
                        LifecycleKind::Birth
                        | LifecycleKind::Run
                        | LifecycleKind::Drain
                        | LifecycleKind::Dissolve => {
                            let kind: &'static str = match lc.kind {
                                LifecycleKind::Birth => "birth",
                                LifecycleKind::Run => "run",
                                LifecycleKind::Drain => "drain",
                                LifecycleKind::Dissolve => "dissolve",
                                _ => unreachable!(),
                            };
                            if !lc.params.is_empty() {
                                return Err(CodegenError::Unsupported(format!(
                                    "locus `{}` lifecycle `{}` declares \
                                     params; only the implicit self is \
                                     supported",
                                    l.name.name, kind
                                )));
                            }
                            let fn_ty =
                                void_t.fn_type(&[ptr_t.into()], false);
                            let func = self.module.add_function(
                                &format!("{}.{}", l.name.name, kind),
                                fn_ty,
                                None,
                            );
                            methods.insert(kind, func);
                        }
                        LifecycleKind::Accept => {
                            if lc.params.len() != 1 {
                                return Err(CodegenError::Unsupported(format!(
                                    "locus `{}` accept() must take exactly \
                                     one child param, got {}",
                                    l.name.name,
                                    lc.params.len()
                                )));
                            }
                            let p = &lc.params[0];
                            let child_ty = self.type_expr_to_lotus(&p.ty)?;
                            let child_locus = match &child_ty {
                                LotusType::LocusRef(name) => name.clone(),
                                other => {
                                    return Err(CodegenError::Unsupported(
                                        format!(
                                            "locus `{}` accept() param must \
                                             be a locus type; got {:?}",
                                            l.name.name, other
                                        ),
                                    ));
                                }
                            };
                            let fn_ty = void_t
                                .fn_type(&[ptr_t.into(), ptr_t.into()], false);
                            let func = self.module.add_function(
                                &format!("{}.accept", l.name.name),
                                fn_ty,
                                None,
                            );
                            methods.insert("accept", func);
                            accept_param =
                                Some((p.name.name.clone(), child_locus));
                        }
                    }
                }
                LocusMember::Bus(bb) => {
                    // Collect subscribe declarations; publish is
                    // typecheck-only (the `<-` operator does the
                    // emit at codegen). Subject must be a literal
                    // string at compile time.
                    for bm in &bb.members {
                        match bm {
                            BusMember::Subscribe { subject, handler, .. } => {
                                subscriptions
                                    .push((subject.clone(), handler.name.clone()));
                            }
                            BusMember::Publish { .. } => {
                                // No-op at codegen; type info
                                // already enforced by typechecker.
                            }
                        }
                    }
                }
                LocusMember::Fn(fd) => {
                    // Locus user-fn: declare as
                    // `<Locus>.<name>(self_ptr, ...args)`. Body
                    // lowered in pass C.
                    if !fd.generics.is_empty() {
                        return Err(CodegenError::Unsupported(format!(
                            "locus `{}` method `{}`: generics not lowered",
                            l.name.name, fd.name.name
                        )));
                    }
                    let mut llvm_param_tys: Vec<inkwell::types::BasicMetadataTypeEnum> =
                        Vec::with_capacity(fd.params.len() + 1);
                    llvm_param_tys.push(ptr_t.into());
                    for p in &fd.params {
                        if p.default.is_some() {
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` method `{}` param `{}` default \
                                 values not yet lowered",
                                l.name.name, fd.name.name, p.name.name
                            )));
                        }
                        let lt = self.type_expr_to_lotus(&p.ty)?;
                        llvm_param_tys.push(self.llvm_basic_type(&lt).into());
                    }
                    let fn_ty = match &fd.ret {
                        None => void_t.fn_type(&llvm_param_tys, false),
                        Some(t) => {
                            let rt = self.type_expr_to_lotus(t)?;
                            match rt {
                                LotusType::Int | LotusType::Duration => self
                                    .context
                                    .i64_type()
                                    .fn_type(&llvm_param_tys, false),
                                LotusType::Float | LotusType::Decimal => self
                                    .context
                                    .f64_type()
                                    .fn_type(&llvm_param_tys, false),
                                LotusType::Bool => self
                                    .context
                                    .bool_type()
                                    .fn_type(&llvm_param_tys, false),
                                LotusType::String
                                | LotusType::Time
                                | LotusType::LocusRef(_)
                                | LotusType::TypeRef(_) => self
                                    .context
                                    .ptr_type(AddressSpace::default())
                                    .fn_type(&llvm_param_tys, false),
                            }
                        }
                    };
                    let func = self.module.add_function(
                        &format!("{}.{}", l.name.name, fd.name.name),
                        fn_ty,
                        None,
                    );
                    user_methods.insert(fd.name.name.clone(), func);
                }
                LocusMember::Closure(c) => {
                    // Reject non-default-epoch closures at codegen
                    // v0; only the dissolve-epoch closures lower to
                    // a synthetic __closures fn fired between drain
                    // and dissolve. Tick / duration / birth /
                    // explicit epochs need the runtime epoch
                    // engine.
                    let mut explicit_epoch = false;
                    for clause in &c.clauses {
                        match clause {
                            ClosureClause::Epoch(EpochSpec::Dissolve) => {
                                explicit_epoch = true;
                            }
                            ClosureClause::Epoch(_) => {
                                return Err(CodegenError::Unsupported(
                                    format!(
                                        "closure `{}` on `{}`: only \
                                         dissolve-epoch closures are \
                                         lowered in codegen v0",
                                        c.name.name, l.name.name
                                    ),
                                ));
                            }
                            ClosureClause::PersistsThrough(_)
                            | ClosureClause::ResetsOn(_) => {
                                // Recovery-event hooks; relevant
                                // when accumulators land. No effect
                                // on the v0 single-shot path.
                            }
                        }
                    }
                    let _ = explicit_epoch;
                    closures.push((c.name.name.clone(), c.assertion.clone()));
                }
                LocusMember::Failure(fd) => {
                    // on_failure(child: ChildL, err: ClosureViolation)
                    // is a handler closures route to when an
                    // unabsorbed violation reaches the parent.
                    if fd.params.len() != 2 {
                        return Err(CodegenError::Unsupported(format!(
                            "locus `{}` on_failure must take exactly two \
                             params (child + err), got {}",
                            l.name.name,
                            fd.params.len()
                        )));
                    }
                    let child_ty = self.type_expr_to_lotus(&fd.params[0].ty)?;
                    let child_locus_name = match &child_ty {
                        LotusType::LocusRef(n) => n.clone(),
                        other => {
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` on_failure first param must be \
                                 a locus type; got {:?}",
                                l.name.name, other
                            )));
                        }
                    };
                    let err_ty = self.type_expr_to_lotus(&fd.params[1].ty)?;
                    if err_ty != LotusType::TypeRef("ClosureViolation".into())
                    {
                        return Err(CodegenError::Unsupported(format!(
                            "locus `{}` on_failure second param must be \
                             ClosureViolation; got {:?}",
                            l.name.name, err_ty
                        )));
                    }
                    // Sig: void(parent_self, child_self, violation)
                    let fn_ty = void_t.fn_type(
                        &[ptr_t.into(), ptr_t.into(), ptr_t.into()],
                        false,
                    );
                    let func = self.module.add_function(
                        &format!("{}.on_failure", l.name.name),
                        fn_ty,
                        None,
                    );
                    failure_handler = Some((child_locus_name, func));
                }
                LocusMember::Mode(md) => {
                    // Modes lower as locus methods named after
                    // the mode kind (bulk / harmonic /
                    // resolution). They share the locus's struct
                    // (per F.5: mode projections share the
                    // locus's arena). Callable via self.bulk()
                    // through the existing self.method() path.
                    let mode_name = match md.kind {
                        ModeKind::Bulk => "bulk",
                        ModeKind::Harmonic => "harmonic",
                        ModeKind::Resolution => "resolution",
                    };
                    let mut llvm_param_tys: Vec<inkwell::types::BasicMetadataTypeEnum> =
                        Vec::with_capacity(md.params.len() + 1);
                    llvm_param_tys.push(ptr_t.into());
                    for p in &md.params {
                        if p.default.is_some() {
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` mode `{}` param `{}` defaults \
                                 not yet lowered",
                                l.name.name, mode_name, p.name.name
                            )));
                        }
                        let lt = self.type_expr_to_lotus(&p.ty)?;
                        llvm_param_tys.push(self.llvm_basic_type(&lt).into());
                    }
                    let fn_ty = match &md.ret {
                        None => void_t.fn_type(&llvm_param_tys, false),
                        Some(t) => {
                            let rt = self.type_expr_to_lotus(t)?;
                            match rt {
                                LotusType::Int | LotusType::Duration => self
                                    .context
                                    .i64_type()
                                    .fn_type(&llvm_param_tys, false),
                                LotusType::Float | LotusType::Decimal => self
                                    .context
                                    .f64_type()
                                    .fn_type(&llvm_param_tys, false),
                                LotusType::Bool => self
                                    .context
                                    .bool_type()
                                    .fn_type(&llvm_param_tys, false),
                                LotusType::String
                                | LotusType::Time
                                | LotusType::LocusRef(_)
                                | LotusType::TypeRef(_) => self
                                    .context
                                    .ptr_type(AddressSpace::default())
                                    .fn_type(&llvm_param_tys, false),
                            }
                        }
                    };
                    let func = self.module.add_function(
                        &format!("{}.{}", l.name.name, mode_name),
                        fn_ty,
                        None,
                    );
                    user_methods.insert(mode_name.to_string(), func);
                }
                LocusMember::Const(_)
                | LocusMember::Type(_) => {
                    return Err(CodegenError::Unsupported(format!(
                        "locus `{}` member kind not yet lowered to codegen",
                        l.name.name
                    )));
                }
            }
        }

        // If this locus has any dissolve-epoch closures, declare
        // its synthetic __closures fn here so call sites in
        // lower_locus_instantiation / flush_dissolve_frame can
        // resolve it. Sig: (self_ptr, parent_self_or_null,
        // on_failure_fn_or_null) — call sites pass the parent's
        // self + on_failure fn ptr if the parent has a matching
        // handler, else null/null. Body lowered in pass C.
        let closures_fn = if !closures.is_empty() {
            let fn_ty = void_t.fn_type(
                &[ptr_t.into(), ptr_t.into(), ptr_t.into()],
                false,
            );
            Some(self.module.add_function(
                &format!("{}.__closures", l.name.name),
                fn_ty,
                None,
            ))
        } else {
            None
        };

        // Stash the methods + accept_param onto the existing
        // LocusInfo.
        let info = self
            .user_loci
            .get_mut(&l.name.name)
            .expect("locus struct declared in pass A1");
        info.methods = methods;
        info.accept_param = accept_param;
        info.user_methods = user_methods;
        info.subscriptions = subscriptions;
        info.closures = closures;
        info.closures_fn = closures_fn;
        info.failure_handler = failure_handler;
        Ok(())
    }

    /// Pass C: lower each declared lifecycle method body. For birth
    /// and run, the method's only arg is `self_ptr`. For accept,
    /// the second arg is the child pointer, bound as a `LocusRef`
    /// local under the param's declared name.
    fn lower_locus_method_bodies(
        &mut self,
        l: &LocusDecl,
    ) -> Result<(), CodegenError> {
        let info = self
            .user_loci
            .get(&l.name.name)
            .cloned()
            .expect("locus declared in pass A");
        for member in &l.members {
            if let LocusMember::Lifecycle(lc) = member {
                let kind: &'static str = match lc.kind {
                    LifecycleKind::Birth => "birth",
                    LifecycleKind::Run => "run",
                    LifecycleKind::Accept => "accept",
                    LifecycleKind::Drain => "drain",
                    LifecycleKind::Dissolve => "dissolve",
                };
                let func = *info
                    .methods
                    .get(kind)
                    .expect("method declared in pass A2");
                let entry = self.context.append_basic_block(func, "entry");
                self.builder.position_at_end(entry);
                let self_ptr = func
                    .get_nth_param(0)
                    .expect("self_ptr param")
                    .into_pointer_value();
                self.current_fn = Some(func);
                self.current_user_fn_ret = None;
                self.current_self = Some(SelfCx {
                    locus_name: l.name.name.clone(),
                    struct_ty: info.struct_ty,
                    self_ptr,
                    fields: info.fields.clone(),
                });
                self.loops.clear();
                self.push_dissolve_frame();

                let mut scope = Scope::default();

                // accept gets the child pointer as its second arg;
                // bind it under the source-level param name as a
                // LocusRef local so `g.X` lowers to GEP+load.
                if kind == "accept" {
                    let (param_name, child_locus) = info
                        .accept_param
                        .as_ref()
                        .expect("accept declared with accept_param");
                    let child_ptr = func
                        .get_nth_param(1)
                        .expect("child_ptr param")
                        .into_pointer_value();
                    // Stash through an alloca'd ptr slot so the
                    // existing Ident-resolution path works without
                    // special-casing fn args.
                    let slot = self
                        .builder
                        .build_alloca(
                            self.context.ptr_type(AddressSpace::default()),
                            param_name,
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(slot, child_ptr)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    scope.locals.insert(
                        param_name.clone(),
                        (slot, LotusType::LocusRef(child_locus.clone())),
                    );
                }

                let end = self.lower_block(&lc.body, &mut scope)?;
                if end == BlockEnd::Open {
                    self.flush_dissolve_frame()?;
                    self.builder
                        .build_return(None)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                } else {
                    let _ = self.deferred_dissolves.pop();
                }

                self.current_fn = None;
                self.current_self = None;
            }
        }

        // on_failure(child: ChildL, err: ClosureViolation) body.
        // LLVM sig: void(parent_self, child_self, violation_ptr).
        // Inside the body: bind the child param as a LocusRef
        // local (so c.field GEPs into child struct) and the err
        // param as a TypeRef("ClosureViolation") local (so
        // err.locus / err.closure GEP into the violation struct).
        if let (Some(failure_decl), Some((child_locus_name, ff))) =
            (l.members.iter().find_map(|m| match m {
                LocusMember::Failure(fd) => Some(fd),
                _ => None,
            }), info.failure_handler.as_ref())
        {
            let child_locus_name = child_locus_name.clone();
            let ff = *ff;
            let entry = self.context.append_basic_block(ff, "entry");
            self.builder.position_at_end(entry);
            let parent_self = ff
                .get_nth_param(0)
                .expect("parent_self param")
                .into_pointer_value();
            let child_self = ff
                .get_nth_param(1)
                .expect("child_self param")
                .into_pointer_value();
            let viol_ptr = ff
                .get_nth_param(2)
                .expect("violation param")
                .into_pointer_value();
            self.current_fn = Some(ff);
            self.current_user_fn_ret = None;
            self.current_self = Some(SelfCx {
                locus_name: l.name.name.clone(),
                struct_ty: info.struct_ty,
                self_ptr: parent_self,
                fields: info.fields.clone(),
            });
            self.loops.clear();
            self.push_dissolve_frame();

            let mut scope = Scope::default();
            // Bind c (the child) and err (the violation) as
            // alloca'd-pointer locals so the existing Ident
            // resolution path works.
            let child_param_name = failure_decl.params[0].name.name.clone();
            let err_param_name = failure_decl.params[1].name.name.clone();
            let ptr_t = self.context.ptr_type(AddressSpace::default());
            let child_slot = self
                .builder
                .build_alloca(ptr_t, &child_param_name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(child_slot, child_self)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            scope.locals.insert(
                child_param_name,
                (child_slot, LotusType::LocusRef(child_locus_name.clone())),
            );
            let err_slot = self
                .builder
                .build_alloca(ptr_t, &err_param_name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(err_slot, viol_ptr)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            scope.locals.insert(
                err_param_name,
                (err_slot, LotusType::TypeRef("ClosureViolation".into())),
            );

            let end = self.lower_block(&failure_decl.body, &mut scope)?;
            if end == BlockEnd::Open {
                self.flush_dissolve_frame()?;
                self.builder
                    .build_return(None)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            } else {
                let _ = self.deferred_dissolves.pop();
            }

            self.current_fn = None;
            self.current_self = None;
        }

        // Synthetic __closures fn: evaluate every dissolve-epoch
        // closure assertion in declaration order. Each assertion
        // computes |left - right| <= tolerance; on fail, write a
        // ClosureViolation report to stderr (fd 2 via dprintf) and
        // exit non-zero. Pass paths flow through silently.
        if !info.closures.is_empty() {
            let func = info
                .closures_fn
                .expect("closures non-empty implies closures_fn declared");
            let entry = self.context.append_basic_block(func, "entry");
            self.builder.position_at_end(entry);
            let self_ptr = func
                .get_nth_param(0)
                .expect("self_ptr param")
                .into_pointer_value();
            // arg 1: parent_self_or_null; arg 2: on_failure fn ptr
            // or null. Both nullable — call sites that have no
            // matching parent handler pass null/null and the fail
            // path falls back to dprintf+exit.
            let parent_self_arg = func
                .get_nth_param(1)
                .expect("parent_self_or_null param")
                .into_pointer_value();
            let parent_handler_arg = func
                .get_nth_param(2)
                .expect("on_failure_or_null param")
                .into_pointer_value();
            self.current_fn = Some(func);
            self.current_user_fn_ret = None;
            self.current_self = Some(SelfCx {
                locus_name: l.name.name.clone(),
                struct_ty: info.struct_ty,
                self_ptr,
                fields: info.fields.clone(),
            });
            self.loops.clear();
            self.push_dissolve_frame();

            for (cname, assertion) in &info.closures {
                self.lower_closure_check(
                    &l.name.name,
                    cname,
                    assertion,
                    parent_self_arg,
                    parent_handler_arg,
                )?;
            }

            self.flush_dissolve_frame()?;
            self.builder
                .build_return(None)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.current_fn = None;
            self.current_self = None;
        }

        // Locus user-fns (`fn` members): same body lowering as
        // lifecycle methods, but with their declared param list
        // (after self_ptr) bound as locals + their declared
        // return type tracked.
        for member in &l.members {
            if let LocusMember::Fn(fd) = member {
                let func = *info
                    .user_methods
                    .get(&fd.name.name)
                    .expect("locus fn declared in pass A2");
                let entry = self.context.append_basic_block(func, "entry");
                self.builder.position_at_end(entry);
                let self_ptr = func
                    .get_nth_param(0)
                    .expect("self_ptr param")
                    .into_pointer_value();
                self.current_fn = Some(func);
                let ret_ty = match &fd.ret {
                    None => None,
                    Some(t) => Some(self.type_expr_to_lotus(t)?),
                };
                self.current_user_fn_ret = Some(ret_ty.clone());
                self.current_self = Some(SelfCx {
                    locus_name: l.name.name.clone(),
                    struct_ty: info.struct_ty,
                    self_ptr,
                    fields: info.fields.clone(),
                });
                self.loops.clear();
                self.push_dissolve_frame();

                let mut scope = Scope::default();
                for (i, p) in fd.params.iter().enumerate() {
                    let lt = self.type_expr_to_lotus(&p.ty)?;
                    let alloca = self.alloca_for(&lt, &p.name.name)?;
                    let v = func
                        .get_nth_param((i + 1) as u32)
                        .expect("locus method arg index in range");
                    self.builder
                        .build_store(alloca, v)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    scope.locals.insert(p.name.name.clone(), (alloca, lt));
                }

                let end = self.lower_block(&fd.body, &mut scope)?;
                if end == BlockEnd::Open {
                    self.flush_dissolve_frame()?;
                    match ret_ty {
                        None => {
                            self.builder
                                .build_return(None)
                                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        }
                        Some(_) => {
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` method `{}` falls through without \
                                 returning a value",
                                l.name.name, fd.name.name
                            )));
                        }
                    }
                } else {
                    let _ = self.deferred_dissolves.pop();
                }

                self.current_fn = None;
                self.current_user_fn_ret = None;
                self.current_self = None;
            }
        }

        // Mode bodies — same lowering as Fn members, with the
        // synthetic method name (bulk / harmonic / resolution).
        for member in &l.members {
            if let LocusMember::Mode(md) = member {
                let mode_name = match md.kind {
                    ModeKind::Bulk => "bulk",
                    ModeKind::Harmonic => "harmonic",
                    ModeKind::Resolution => "resolution",
                };
                let func = *info
                    .user_methods
                    .get(mode_name)
                    .expect("mode declared in pass A2");
                let entry = self.context.append_basic_block(func, "entry");
                self.builder.position_at_end(entry);
                let self_ptr = func
                    .get_nth_param(0)
                    .expect("self_ptr param")
                    .into_pointer_value();
                self.current_fn = Some(func);
                let ret_ty = match &md.ret {
                    None => None,
                    Some(t) => Some(self.type_expr_to_lotus(t)?),
                };
                self.current_user_fn_ret = Some(ret_ty.clone());
                self.current_self = Some(SelfCx {
                    locus_name: l.name.name.clone(),
                    struct_ty: info.struct_ty,
                    self_ptr,
                    fields: info.fields.clone(),
                });
                self.loops.clear();
                self.push_dissolve_frame();

                let mut scope = Scope::default();
                for (i, p) in md.params.iter().enumerate() {
                    let lt = self.type_expr_to_lotus(&p.ty)?;
                    let alloca = self.alloca_for(&lt, &p.name.name)?;
                    let v = func
                        .get_nth_param((i + 1) as u32)
                        .expect("mode arg index in range");
                    self.builder
                        .build_store(alloca, v)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    scope.locals.insert(p.name.name.clone(), (alloca, lt));
                }

                let end = self.lower_block(&md.body, &mut scope)?;
                if end == BlockEnd::Open {
                    self.flush_dissolve_frame()?;
                    match ret_ty {
                        None => {
                            self.builder
                                .build_return(None)
                                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        }
                        Some(_) => {
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` mode `{}` falls through without \
                                 returning a value",
                                l.name.name, mode_name
                            )));
                        }
                    }
                } else {
                    let _ = self.deferred_dissolves.pop();
                }

                self.current_fn = None;
                self.current_user_fn_ret = None;
                self.current_self = None;
            }
        }
        Ok(())
    }

    /// Declare a user-defined fn's LLVM function value and signature.
    /// Body lowering happens in a separate pass so calls in pass 2
    /// can resolve to fns declared anywhere in the program.
    fn declare_user_fn(&mut self, f: &FnDecl) -> Result<(), CodegenError> {
        if !f.generics.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "generic fn `{}`",
                f.name.name
            )));
        }
        let mut param_tys = Vec::with_capacity(f.params.len());
        let mut llvm_param_tys: Vec<inkwell::types::BasicMetadataTypeEnum> =
            Vec::with_capacity(f.params.len());
        for p in &f.params {
            if p.default.is_some() {
                return Err(CodegenError::Unsupported(format!(
                    "fn `{}` param `{}` default values not yet lowered",
                    f.name.name, p.name.name
                )));
            }
            let lt = self.type_expr_to_lotus(&p.ty)?;
            llvm_param_tys.push(self.llvm_basic_type(&lt).into());
            param_tys.push(lt);
        }
        let ret_ty = match &f.ret {
            Some(t) => Some(self.type_expr_to_lotus(t)?),
            None => None,
        };
        let fn_ty = match &ret_ty {
            Some(LotusType::Int) | Some(LotusType::Duration) => self
                .context
                .i64_type()
                .fn_type(&llvm_param_tys, false),
            Some(LotusType::Float) | Some(LotusType::Decimal) => {
                self.context.f64_type().fn_type(&llvm_param_tys, false)
            }
            Some(LotusType::Bool) => {
                self.context.bool_type().fn_type(&llvm_param_tys, false)
            }
            Some(LotusType::String)
            | Some(LotusType::Time)
            | Some(LotusType::LocusRef(_))
            | Some(LotusType::TypeRef(_)) => self
                .context
                .ptr_type(AddressSpace::default())
                .fn_type(&llvm_param_tys, false),
            None => self
                .context
                .void_type()
                .fn_type(&llvm_param_tys, false),
        };
        let func = self.module.add_function(&f.name.name, fn_ty, None);
        self.user_fns.insert(
            f.name.name.clone(),
            FnSig {
                func,
                params: param_tys,
                ret: ret_ty,
            },
        );
        Ok(())
    }

    /// Lower a user fn's body. Each declared param is materialized
    /// as an alloca in the entry block so reads through `Ident`
    /// see the value-stored slot exactly the way `let`-bindings do.
    fn lower_user_fn_body(&mut self, f: &FnDecl) -> Result<(), CodegenError> {
        let sig = self
            .user_fns
            .get(&f.name.name)
            .cloned()
            .expect("fn declared in pass 1");
        let func = sig.func;
        let entry = self.context.append_basic_block(func, "entry");
        self.builder.position_at_end(entry);
        self.current_fn = Some(func);
        self.current_user_fn_ret = Some(sig.ret.clone());
        self.current_self = None;
        self.loops.clear();
        self.push_dissolve_frame();

        let mut scope = Scope::default();
        for (i, p) in f.params.iter().enumerate() {
            let lt = sig.params[i].clone();
            let alloca = self.alloca_for(&lt, &p.name.name)?;
            let v = func
                .get_nth_param(i as u32)
                .expect("param index in range");
            self.builder
                .build_store(alloca, v)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            scope.locals.insert(p.name.name.clone(), (alloca, lt));
        }

        let end = self.lower_block(&f.body, &mut scope)?;
        if end == BlockEnd::Open {
            self.flush_dissolve_frame()?;
            match &sig.ret {
                None => {
                    self.builder
                        .build_return(None)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Some(_) => {
                    return Err(CodegenError::Unsupported(format!(
                        "fn `{}` falls through without returning a value",
                        f.name.name
                    )));
                }
            }
        } else {
            let _ = self.deferred_dissolves.pop();
        }

        self.current_fn = None;
        self.current_user_fn_ret = None;
        Ok(())
    }

    /// Emit a call to a user-defined fn. Returns the lowered value
    /// + type when the fn has a return type, or `None` for void
    /// fns. Used from both expression-position and statement-position
    /// call sites.
    fn lower_user_fn_call(
        &mut self,
        name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<Option<(BasicValueEnum<'ctx>, LotusType)>, CodegenError> {
        let sig = self
            .user_fns
            .get(name)
            .cloned()
            .ok_or_else(|| {
                CodegenError::Unsupported(format!("call to unknown fn `{}`", name))
            })?;
        if args.len() != sig.params.len() {
            return Err(CodegenError::Unsupported(format!(
                "fn `{}` expects {} args, got {}",
                name,
                sig.params.len(),
                args.len()
            )));
        }
        let mut llvm_args: Vec<BasicMetadataValueEnum> =
            Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            let (v, ty) = self.lower_expr(a, scope)?;
            if ty != sig.params[i] {
                return Err(CodegenError::Unsupported(format!(
                    "fn `{}` arg {} type mismatch: expected {:?}, got {:?}",
                    name, i, sig.params[i], ty
                )));
            }
            llvm_args.push(v.into());
        }
        let call = self
            .builder
            .build_call(sig.func, &llvm_args, &format!("{}.call", name))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        match sig.ret {
            None => Ok(None),
            Some(lt) => {
                let v = call
                    .try_as_basic_value()
                    .left()
                    .expect("non-void fn returns a basic value");
                Ok(Some((v, lt)))
            }
        }
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
                        "qualified-name struct literal `{}`",
                        path.segments
                            .iter()
                            .map(|s| s.name.as_str())
                            .collect::<Vec<_>>()
                            .join("::")
                    )));
                }
                let name = path.segments[0].name.as_str();
                if self.user_loci.contains_key(name) {
                    let _ = self.lower_locus_instantiation(name, inits, scope)?;
                } else if self.user_types.contains_key(name) {
                    // Statement-position type literal: build it,
                    // discard the pointer. Useful for side-effect-
                    // free expressions like `Foo {};` (rare but legal).
                    let _ = self.lower_user_type_instantiation(name, inits, scope)?;
                } else {
                    return Err(CodegenError::Unsupported(format!(
                        "struct literal `{}`: no locus or type by that name",
                        name
                    )));
                }
                Ok(BlockEnd::Open)
            }
            Stmt::Expr(Expr::Call { callee, args, .. }) => {
                match callee.as_ref() {
                    Expr::Ident(i) => {
                        let name = i.name.as_str();
                        if name == "bubble" {
                            // bubble() ends the block — propagate
                            // Terminated up so the lower_block walker
                            // stops emitting IR after this stmt.
                            return self.lower_bubble_call(args, scope);
                        } else if self.user_fns.contains_key(name) {
                            // Discard return value; statement-position
                            // call.
                            let _ = self.lower_user_fn_call(name, args, scope)?;
                        } else {
                            self.lower_print_call(name, args, scope)?;
                        }
                    }
                    Expr::Path(qn) => {
                        self.lower_path_call(qn, args, scope)?;
                    }
                    Expr::Field { receiver, name, .. }
                        if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
                    {
                        let _ = self.lower_self_method_call(
                            &name.name,
                            args,
                            scope,
                        )?;
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
                let (val, ty) = self.lower_expr(value, scope)?;
                let alloca = self.alloca_for(&ty, &name.name)?;
                self.builder
                    .build_store(alloca, val)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                scope.locals.insert(name.name.clone(), (alloca, ty));
                Ok(BlockEnd::Open)
            }
            Stmt::Assign { target, op, value, .. } => {
                // Resolve the target into a (slot_ptr, slot_ty)
                // pair. Bare locals come from the scope; `self.X`
                // GEPs into the current method's self-struct.
                let (slot_ptr, slot_ty, slot_name) = if target.head.name
                    == "self"
                {
                    if target.tail.len() != 1 {
                        return Err(CodegenError::Unsupported(format!(
                            "assignment target `self.{}` with {} segment(s) \
                             not yet supported",
                            target
                                .tail
                                .iter()
                                .filter_map(|s| match s {
                                    LValueSeg::Field(i) => Some(i.name.as_str()),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("."),
                            target.tail.len()
                        )));
                    }
                    let field_name = match &target.tail[0] {
                        LValueSeg::Field(i) => i.name.clone(),
                        LValueSeg::Index(_) => {
                            return Err(CodegenError::Unsupported(
                                "indexed self assignment".to_string(),
                            ));
                        }
                    };
                    let cs = self.current_self.as_ref().cloned().ok_or_else(
                        || {
                            CodegenError::Unsupported(
                                "`self.X =` outside a locus method".to_string(),
                            )
                        },
                    )?;
                    let (idx, ty) = cs.fields.get(&field_name).cloned().ok_or_else(
                        || {
                            CodegenError::Unsupported(format!(
                                "no field `{}` on locus self",
                                field_name
                            ))
                        },
                    )?;
                    let ptr = self
                        .builder
                        .build_struct_gep(
                            cs.struct_ty,
                            cs.self_ptr,
                            idx,
                            &format!("self.{}.ptr", field_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    (ptr, ty, format!("self.{}", field_name))
                } else if target.tail.is_empty() {
                    let (alloca, ty) = scope
                        .locals
                        .get(&target.head.name)
                        .cloned()
                        .ok_or_else(|| {
                            CodegenError::Unsupported(format!(
                                "assignment to unbound `{}`",
                                target.head.name
                            ))
                        })?;
                    (alloca, ty, target.head.name.clone())
                } else {
                    return Err(CodegenError::Unsupported(
                        "non-self field/index assignment target".to_string(),
                    ));
                };

                let (rhs, rhs_ty) = self.lower_expr(value, scope)?;
                let new_val = if matches!(op, AssignOp::Eq) {
                    if rhs_ty != slot_ty {
                        return Err(CodegenError::Unsupported(format!(
                            "type mismatch in assignment to `{}`: \
                             slot {:?} vs rhs {:?}",
                            slot_name, slot_ty, rhs_ty
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
                    let llvm_ty = self.llvm_basic_type(&slot_ty);
                    let cur = self
                        .builder
                        .build_load(llvm_ty, slot_ptr, &slot_name)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let (v, _) = self.lower_binop(bin_op, cur, rhs, &slot_ty)?;
                    v
                };
                self.builder
                    .build_store(slot_ptr, new_val)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok(BlockEnd::Open)
            }
            Stmt::If(if_stmt) => self.lower_if(if_stmt, scope),
            Stmt::While { cond, body, .. } => {
                self.lower_while(cond, body, scope)
            }
            Stmt::For { name, iter, body, .. } => {
                self.lower_for(name, iter, body, scope)
            }
            Stmt::Break(_) => self.lower_break(),
            Stmt::Continue(_) => self.lower_continue(),
            Stmt::Block(b) => self.lower_block(b, scope),
            Stmt::Return(expr_opt, _) => self.lower_return(expr_opt.as_ref(), scope),
            Stmt::Recovery { op, args, modifier, .. } => {
                if modifier.is_some() {
                    return Err(CodegenError::Unsupported(
                        "recovery modifier (for/until) not lowered".into(),
                    ));
                }
                match op {
                    RecoveryOp::Bubble => self.lower_bubble_call(args, scope),
                    other => Err(CodegenError::Unsupported(format!(
                        "recovery op {:?} not lowered",
                        other
                    ))),
                }
            }
            Stmt::Send { subject, value, .. } => {
                self.lower_send(subject, value, scope)?;
                Ok(BlockEnd::Open)
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

    fn lower_if(
        &mut self,
        ifs: &IfStmt,
        scope: &mut Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        let (cond_v, cond_ty) =
            self.lower_expr(&ifs.cond, scope)?;
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
        let (cond_v, cond_ty) = self.lower_expr(cond, scope)?;
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

    /// Lower `for X in iter { body }`. v0 codegen recognizes only
    /// `iter == self.children`: the iter is a fixed-cap array of
    /// child-locus pointers paired with an i64 counter, both
    /// embedded in the current self struct. Each iteration loads
    /// `self.children[i]` as a `LocusRef(child_type)` local named
    /// `X` so `X.field` resolves through the existing GEP path.
    fn lower_for(
        &mut self,
        var_name: &Ident,
        iter: &Expr,
        body: &Block,
        scope: &mut Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        // Pattern-match on `self.children` as the iter. Other
        // iterators (arrays, ranges, etc.) need a richer
        // collection ABI — out of v0 scope.
        let is_self_children = matches!(iter, Expr::Field { receiver, name, .. }
            if matches!(receiver.as_ref(), Expr::KwSelf(_))
                && name.name == "children");
        if !is_self_children {
            return Err(CodegenError::Unsupported(format!(
                "for-loop iterator `{:?}`: codegen v0 only supports \
                 `for X in self.children`",
                std::mem::discriminant(iter)
            )));
        }
        let cs = self.current_self.as_ref().cloned().ok_or_else(|| {
            CodegenError::Unsupported(
                "self.children outside a locus method".into(),
            )
        })?;
        let info = self
            .user_loci
            .get(&cs.locus_name)
            .cloned()
            .expect("current_self points to a declared locus");
        let arr_idx = info.children_field_idx.ok_or_else(|| {
            CodegenError::Unsupported(format!(
                "locus `{}` has no children (no accept declared)",
                cs.locus_name
            ))
        })?;
        let cnt_idx = info.child_count_field_idx.expect("paired");
        let child_locus = info
            .accept_param
            .as_ref()
            .map(|(_, ln)| ln.clone())
            .expect("children_field_idx implies accept declared");

        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let arr_ty = ptr_t.array_type(CHILDREN_CAP);

        let func = self
            .current_fn
            .expect("current_fn set while lowering a for");
        let header_bb = self.context.append_basic_block(func, "for.cond");
        let body_bb = self.context.append_basic_block(func, "for.body");
        let inc_bb = self.context.append_basic_block(func, "for.inc");
        let exit_bb = self.context.append_basic_block(func, "for.end");

        // i = 0 (alloca'd so the body can break/continue cleanly)
        let i_slot = self
            .builder
            .build_alloca(i64_t, "for.i.slot")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(i_slot, i64_t.const_int(0, false))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(header_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // header: load i, load count, branch on i < count
        self.builder.position_at_end(header_bb);
        let cnt_ptr = self
            .builder
            .build_struct_gep(
                cs.struct_ty,
                cs.self_ptr,
                cnt_idx,
                "for.count.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let count = self
            .builder
            .build_load(i64_t, cnt_ptr, "for.count")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let i = self
            .builder
            .build_load(i64_t, i_slot, "for.i")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let in_range = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::ULT,
                i,
                count,
                "for.in.range",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(in_range, body_bb, exit_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // body: load self.children[i] as a LocusRef local named
        // var_name; lower body; jump to inc.
        self.builder.position_at_end(body_bb);
        let arr_ptr = self
            .builder
            .build_struct_gep(
                cs.struct_ty,
                cs.self_ptr,
                arr_idx,
                "for.children.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let slot_ptr = unsafe {
            self.builder
                .build_gep(
                    arr_ty,
                    arr_ptr,
                    &[i32_t.const_int(0, false), i],
                    "for.slot.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        let child_ptr = self
            .builder
            .build_load(ptr_t, slot_ptr, "for.child")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Stash through an alloca so the existing Ident-resolution
        // path works. The local's type is LocusRef(child_locus) so
        // `child.value` GEPs through the right struct.
        let local_slot = self
            .builder
            .build_alloca(ptr_t, &var_name.name)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(local_slot, child_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let prev = scope.locals.insert(
            var_name.name.clone(),
            (local_slot, LotusType::LocusRef(child_locus)),
        );
        self.loops.push(LoopFrame {
            continue_bb: inc_bb,
            break_bb: exit_bb,
        });
        let body_end = self.lower_block(body, scope)?;
        self.loops.pop();
        if body_end == BlockEnd::Open {
            self.builder
                .build_unconditional_branch(inc_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        // Restore prior binding for `var_name` (if any) so the
        // for-loop's local doesn't leak past the loop body.
        if let Some(prev) = prev {
            scope.locals.insert(var_name.name.clone(), prev);
        } else {
            scope.locals.remove(&var_name.name);
        }

        // inc: i = i + 1; jump to header
        self.builder.position_at_end(inc_bb);
        let i_now = self
            .builder
            .build_load(i64_t, i_slot, "for.i.now")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let i_next = self
            .builder
            .build_int_add(i_now, i64_t.const_int(1, false), "for.i.next")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(i_slot, i_next)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(header_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

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

    fn lower_return(
        &mut self,
        expr: Option<&Expr>,
        scope: &Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        if self.in_main {
            // Return from main maps to the C entry point's i32
            // exit code. A bare `return;` exits 0; `return n;`
            // truncates the i64 to i32 to match the declared
            // ABI. main itself returns void at the lotus level
            // OR Int (per spec/runtime.md), but at LLVM we always
            // emit i32. Flush the dissolve frame first so any
            // long-lived loci wind down before the process exits,
            // then tear down the arena.
            self.flush_dissolve_frame()?;
            self.emit_arena_destroy()?;
            // Re-open an empty frame so the post-flush bookkeeping
            // (popped in lower_program) stays balanced.
            self.push_dissolve_frame();
            let i32_t = self.context.i32_type();
            let code = match expr {
                None => i32_t.const_int(0, false),
                Some(e) => {
                    let (v, ty) = self.lower_expr(e, scope)?;
                    if ty != LotusType::Int {
                        return Err(CodegenError::Unsupported(format!(
                            "`return` from main must carry Int (exit code); \
                             got {:?}",
                            ty
                        )));
                    }
                    self.builder
                        .build_int_truncate(
                            v.into_int_value(),
                            i32_t,
                            "exit.code",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                }
            };
            self.builder
                .build_return(Some(&code))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            return Ok(BlockEnd::Terminated);
        }
        let ret_ty = self.current_user_fn_ret.clone().ok_or_else(|| {
            CodegenError::Unsupported(
                "`return` outside a user fn".to_string(),
            )
        })?;
        match (expr, ret_ty) {
            (None, None) => {
                self.builder
                    .build_return(None)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            (Some(e), Some(declared_ty)) => {
                let (v, got_ty) = self.lower_expr(e, scope)?;
                if got_ty != declared_ty {
                    return Err(CodegenError::Unsupported(format!(
                        "return type mismatch: declared {:?}, got {:?}",
                        declared_ty, got_ty
                    )));
                }
                self.builder
                    .build_return(Some(&v))
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            (None, Some(declared)) => {
                return Err(CodegenError::Unsupported(format!(
                    "fn declared to return {:?} but `return;` carries no value",
                    declared
                )));
            }
            (Some(_), None) => {
                return Err(CodegenError::Unsupported(
                    "fn declared with no return type but `return e;` \
                     carries a value"
                        .to_string(),
                ));
            }
        }
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
        ty: &LotusType,
        name: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        match ty {
            LotusType::Int | LotusType::Duration => self
                .builder
                .build_alloca(self.context.i64_type(), name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string())),
            LotusType::Float | LotusType::Decimal => self
                .builder
                .build_alloca(self.context.f64_type(), name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string())),
            LotusType::Bool => self
                .builder
                .build_alloca(self.context.bool_type(), name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string())),
            LotusType::String
            | LotusType::Time
            | LotusType::LocusRef(_)
            | LotusType::TypeRef(_) => self
                .builder
                .build_alloca(self.context.ptr_type(AddressSpace::default()), name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string())),
        }
    }

    fn lower_expr(
        &mut self,
        e: &Expr,
        scope: &Scope<'ctx>,
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
            Expr::Literal(Literal::Decimal(s), _) => {
                // v0 codegen mirrors the interpreter's
                // `parse_decimal`: strip optional `d` suffix, parse
                // as f64. Real fixed-point lands later when Decimal
                // precision actually matters for trellis production.
                let stripped = s.strip_suffix('d').unwrap_or(s);
                let f = stripped.parse::<f64>().map_err(|e| {
                    CodegenError::Unsupported(format!(
                        "Decimal literal `{}` failed to parse: {}",
                        s, e
                    ))
                })?;
                let v = self.context.f64_type().const_float(f);
                Ok((v.into(), LotusType::Decimal))
            }
            Expr::Literal(Literal::Time(s), _) => {
                // v0 codegen mirrors the interpreter: store the
                // source spelling as a NUL-terminated global. Real
                // i64-since-epoch arithmetic lands later.
                let p = self.global_string(s);
                Ok((p.into(), LotusType::Time))
            }
            Expr::Ident(id) => {
                let (alloca, ty) = scope
                    .locals
                    .get(&id.name)
                    .cloned()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "unknown identifier `{}`",
                            id.name
                        ))
                    })?;
                let llvm_ty = self.llvm_basic_type(&ty);
                let loaded = self
                    .builder
                    .build_load(llvm_ty, alloca, &id.name)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((loaded, ty))
            }
            Expr::Field { receiver, name, .. }
                if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
            {
                let cs = self.current_self.as_ref().cloned().ok_or_else(
                    || {
                        CodegenError::Unsupported(format!(
                            "self.{} read outside a locus method",
                            name.name
                        ))
                    },
                )?;
                let (idx, ty) = cs.fields.get(&name.name).cloned().ok_or_else(
                    || {
                        CodegenError::Unsupported(format!(
                            "no field `{}` on locus self",
                            name.name
                        ))
                    },
                )?;
                let ptr = self
                    .builder
                    .build_struct_gep(
                        cs.struct_ty,
                        cs.self_ptr,
                        idx,
                        &format!("self.{}.ptr", name.name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let llvm_ty = self.llvm_basic_type(&ty);
                let val = self
                    .builder
                    .build_load(llvm_ty, ptr, &format!("self.{}", name.name))
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((val, ty))
            }
            Expr::Field { receiver, name, .. } => {
                // Generalized field access: lower the receiver as
                // an expression. If it's a LocusRef or TypeRef
                // pointer, GEP+load. This supports `g.X`, `g.x.y`,
                // `self.kernel.multiplier`, expressions returning
                // a TypeRef value, etc.
                let (recv_val, recv_ty) = self.lower_expr(receiver, scope)?;
                let (struct_ty, fields, ref_kind) = match &recv_ty {
                    LotusType::LocusRef(n) => {
                        let info = self
                            .user_loci
                            .get(n)
                            .cloned()
                            .expect("LocusRef points to a declared locus");
                        (info.struct_ty, info.fields, format!("locus `{}`", n))
                    }
                    LotusType::TypeRef(n) => {
                        let info = self
                            .user_types
                            .get(n)
                            .cloned()
                            .expect("TypeRef points to a declared type");
                        (info.struct_ty, info.fields, format!("type `{}`", n))
                    }
                    other => {
                        return Err(CodegenError::Unsupported(format!(
                            "field access `.{}` on non-record type {:?}",
                            name.name, other
                        )));
                    }
                };
                let (idx, field_ty) = fields
                    .get(&name.name)
                    .cloned()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "no field `{}` on {}",
                            name.name, ref_kind
                        ))
                    })?;
                let recv_ptr = recv_val.into_pointer_value();
                let field_ptr = self
                    .builder
                    .build_struct_gep(
                        struct_ty,
                        recv_ptr,
                        idx,
                        &format!("field.{}.ptr", name.name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let llvm_ty = self.llvm_basic_type(&field_ty);
                let val = self
                    .builder
                    .build_load(
                        llvm_ty,
                        field_ptr,
                        &format!("field.{}", name.name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((val, field_ty))
            }
            Expr::Binary { op, left, right, span: _ } => {
                let (lv, lt) = self.lower_expr(left, scope)?;
                let (rv, rt) = self.lower_expr(right, scope)?;
                if lt != rt {
                    return Err(CodegenError::Unsupported(format!(
                        "binary op operands of mixed types {:?} and {:?}",
                        lt, rt
                    )));
                }
                self.lower_binop(*op, lv, rv, &lt)
            }
            Expr::Unary { op, operand, .. } => {
                let (v, t) = self.lower_expr(operand, scope)?;
                self.lower_unop(*op, v, &t)
            }
            Expr::Call { callee, args, .. } => match callee.as_ref() {
                Expr::Ident(i) if self.user_fns.contains_key(&i.name) => {
                    let result =
                        self.lower_user_fn_call(i.name.as_str(), args, scope)?;
                    result.ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "fn `{}` returns no value but is used in \
                             expression position",
                            i.name
                        ))
                    })
                }
                Expr::Path(qn) => self.lower_path_call_expr(qn, args),
                Expr::Field { receiver, name, .. }
                    if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
                {
                    let result =
                        self.lower_self_method_call(&name.name, args, scope)?;
                    result.ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "self.{} returns no value but is used in \
                             expression position",
                            name.name
                        ))
                    })
                }
                _ => Err(CodegenError::Unsupported(format!(
                    "non-user-fn call in expression position: {:?}",
                    std::mem::discriminant(callee.as_ref())
                ))),
            },
            Expr::Struct { path, inits, .. }
                if path.segments.len() == 1
                    && self.user_loci.contains_key(&path.segments[0].name) =>
            {
                // Locus literal in expression position: instantiate
                // and return the self_ptr typed as LocusRef. The
                // caller (a let-binding, etc.) keeps the locus
                // alive for the duration of the binding's scope.
                // Ephemeral semantics still apply — the struct is
                // alloca'd on the current frame, and drain/dissolve
                // fired at the end of lower_locus_instantiation
                // for ephemeral loci. The pointer remains valid
                // because the alloca itself outlives statement
                // boundaries; field reads through the locus's
                // expose'd contract still work for the rest of
                // the body.
                let name = path.segments[0].name.clone();
                let ptr =
                    self.lower_locus_instantiation(&name, inits, scope)?;
                Ok((ptr.into(), LotusType::LocusRef(name)))
            }
            Expr::Struct { path, inits, .. }
                if path.segments.len() == 1
                    && self.user_types.contains_key(&path.segments[0].name) =>
            {
                let name = path.segments[0].name.clone();
                let ptr = self.lower_user_type_instantiation(&name, inits, scope)?;
                Ok((ptr.into(), LotusType::TypeRef(name)))
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
            ParamValue::Decimal(f) => (
                self.context.f64_type().const_float(*f).into(),
                LotusType::Decimal,
            ),
            ParamValue::Time(s) => {
                (self.global_string(s).into(), LotusType::Time)
            }
        }
    }

    fn llvm_basic_type(
        &self,
        t: &LotusType,
    ) -> inkwell::types::BasicTypeEnum<'ctx> {
        match t {
            LotusType::Int | LotusType::Duration => {
                self.context.i64_type().into()
            }
            LotusType::Float | LotusType::Decimal => {
                self.context.f64_type().into()
            }
            LotusType::Bool => self.context.bool_type().into(),
            LotusType::String
            | LotusType::Time
            | LotusType::LocusRef(_)
            | LotusType::TypeRef(_) => {
                self.context.ptr_type(AddressSpace::default()).into()
            }
        }
    }

    fn lower_binop(
        &mut self,
        op: BinOp,
        lv: BasicValueEnum<'ctx>,
        rv: BasicValueEnum<'ctx>,
        ty: &LotusType,
    ) -> Result<(BasicValueEnum<'ctx>, LotusType), CodegenError> {
        use inkwell::IntPredicate as IP;
        use inkwell::FloatPredicate as FP;
        let ty_owned = ty.clone();
        match (op, ty_owned) {
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
            // Duration arithmetic — add/sub produce Duration. Mul
            // / div / mod don't have natural Duration semantics
            // (multiply by a scalar would, but we don't have
            // scalar-by-Duration overloads yet).
            (BinOp::Add | BinOp::Sub, LotusType::Duration) => {
                let l = lv.into_int_value();
                let r = rv.into_int_value();
                let v = match op {
                    BinOp::Add => self.builder.build_int_add(l, r, "dadd"),
                    BinOp::Sub => self.builder.build_int_sub(l, r, "dsub"),
                    _ => unreachable!(),
                };
                let v = v.map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v.into(), LotusType::Duration))
            }
            (BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq,
                LotusType::Duration) =>
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
                    .build_int_compare(pred, l, r, "dcmp")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v.into(), LotusType::Bool))
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
            (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div, LotusType::Decimal) => {
                let l = lv.into_float_value();
                let r = rv.into_float_value();
                let v = match op {
                    BinOp::Add => self.builder.build_float_add(l, r, "decadd"),
                    BinOp::Sub => self.builder.build_float_sub(l, r, "decsub"),
                    BinOp::Mul => self.builder.build_float_mul(l, r, "decmul"),
                    BinOp::Div => self.builder.build_float_div(l, r, "decdiv"),
                    _ => unreachable!(),
                };
                let v = v.map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v.into(), LotusType::Decimal))
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
                LotusType::Float | LotusType::Decimal) =>
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
        ty: &LotusType,
    ) -> Result<(BasicValueEnum<'ctx>, LotusType), CodegenError> {
        let ty_owned = ty.clone();
        match (op, ty_owned) {
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
            (UnaryOp::Neg, LotusType::Decimal) => {
                let r = self
                    .builder
                    .build_float_neg(v.into_float_value(), "decneg")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((r.into(), LotusType::Decimal))
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

    /// Lower a `print` / `println` call. Args resolve through the
    /// current scope and (when set) the current `self` struct, so
    /// the same lowering serves both ordinary fn bodies and
    /// lifecycle-method bodies.
    fn lower_print_call(
        &mut self,
        name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
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
            let (val, ty) = self.lower_expr(a, scope)?;
            match &ty {
                LotusType::Int => {
                    format.push_str("%lld");
                    printf_args.push(BasicMetadataValueEnum::IntValue(val.into_int_value()));
                }
                LotusType::Float | LotusType::Decimal => {
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
                LotusType::String | LotusType::Time => {
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
                LotusType::LocusRef(name) => {
                    return Err(CodegenError::Unsupported(format!(
                        "println of a locus value (LocusRef `{}`) — \
                         lotus has no Display protocol yet",
                        name
                    )));
                }
                LotusType::TypeRef(name) => {
                    return Err(CodegenError::Unsupported(format!(
                        "println of a type value (TypeRef `{}`) — \
                         print individual fields instead",
                        name
                    )));
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
    /// Statement-position path call. Currently dispatches to the
    /// few stdlib paths codegen recognizes; everything else errors.
    fn lower_path_call(
        &mut self,
        qn: &QualifiedName,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        let segs: Vec<&str> =
            qn.segments.iter().map(|s| s.name.as_str()).collect();
        match segs.as_slice() {
            ["time", "sleep"] => self.lower_time_sleep(args, scope),
            ["time", "monotonic"] => {
                // statement-position: just discard the returned value
                let _ = self.lower_time_monotonic(args)?;
                Ok(())
            }
            _ => Err(CodegenError::Unsupported(format!(
                "path call `{}`",
                segs.join("::")
            ))),
        }
    }

    /// Expression-position path call — must return a value.
    fn lower_path_call_expr(
        &mut self,
        qn: &QualifiedName,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, LotusType), CodegenError> {
        let segs: Vec<&str> =
            qn.segments.iter().map(|s| s.name.as_str()).collect();
        match segs.as_slice() {
            ["time", "monotonic"] => self.lower_time_monotonic(args),
            _ => Err(CodegenError::Unsupported(format!(
                "path call `{}` in expression position",
                segs.join("::")
            ))),
        }
    }

    /// Lower `time::monotonic()` to `clock_gettime(CLOCK_MONOTONIC,
    /// &ts)` followed by `ts.tv_sec * 1_000_000_000 + ts.tv_nsec`.
    /// Result is a `Duration` (i64 nanoseconds since an
    /// unspecified reference).
    fn lower_time_monotonic(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, LotusType), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "time::monotonic takes 0 arguments, got {}",
                args.len()
            )));
        }
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let ts_t = self.timespec_type();

        let ts = self
            .builder
            .build_alloca(ts_t, "ts")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let cgt = self
            .module
            .get_function("clock_gettime")
            .expect("clock_gettime declared");
        // CLOCK_MONOTONIC = 1 on Linux.
        let clock_id = i32_t.const_int(1, false);
        self.builder
            .build_call(cgt, &[clock_id.into(), ts.into()], "cgt.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Ignore the return value best-effort; CLOCK_MONOTONIC
        // shouldn't fail. tv_sec * 1e9 + tv_nsec.
        let sec_ptr = self
            .builder
            .build_struct_gep(ts_t, ts, 0, "ts.sec.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let nsec_ptr = self
            .builder
            .build_struct_gep(ts_t, ts, 1, "ts.nsec.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let sec = self
            .builder
            .build_load(i64_t, sec_ptr, "ts.sec")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let nsec = self
            .builder
            .build_load(i64_t, nsec_ptr, "ts.nsec")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let billion = i64_t.const_int(1_000_000_000, false);
        let sec_ns = self
            .builder
            .build_int_mul(sec, billion, "sec.ns")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let total = self
            .builder
            .build_int_add(sec_ns, nsec, "now.ns")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((total.into(), LotusType::Duration))
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
    ) -> Result<(), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "time::sleep takes 1 argument, got {}",
                args.len()
            )));
        }
        let (val, ty) = self.lower_expr(&args[0], scope)?;
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

    /// Statement-level locus instantiation `T { f: v, ... };`.
    /// Allocates a struct on the caller's stack, fills its fields
    /// (defaults overridden by the call site), then calls birth()
    /// and run() if present. The locus is ephemeral: when the
    /// surrounding fn returns the alloca is reclaimed. Long-lived
    /// loci wait on the cooperative scheduler + region allocator.
    fn lower_locus_instantiation(
        &mut self,
        locus_name: &str,
        inits: &[StructInit],
        scope: &Scope<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let info = self
            .user_loci
            .get(locus_name)
            .cloned()
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "no locus `{}` declared",
                    locus_name
                ))
            })?;

        // Build a name → override-expr map for the call site.
        let overrides: BTreeMap<&str, &Expr> = inits
            .iter()
            .map(|i| (i.name.name.as_str(), &i.value))
            .collect();

        let self_ptr = self
            .builder
            .build_alloca(info.struct_ty, &format!("{}.self", locus_name))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // First — initialize the synthetic `__arena` field
        // (struct slot 0) with a fresh arena. Allocations made
        // on behalf of this locus during the rest of
        // instantiation (composite-literal defaults / overrides)
        // and during its lifecycle method bodies will route
        // through `arena_alloc`, which prefers `current_self`'s
        // arena field over the program global.
        let arena_create = self
            .module
            .get_function("lotus_arena_create")
            .expect("lotus_arena_create declared");
        let new_arena = self
            .builder
            .build_call(arena_create, &[], &format!("{}.arena", locus_name))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("arena_create returns ptr");
        let arena_field = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.arena_field_idx,
                &format!("{}.__arena.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(arena_field, new_arena)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Initialize each field. Overrides go through lower_expr in
        // the caller's scope so any expression — not just literals —
        // can be passed. Defaults are either pre-resolved scalar
        // literals (DefaultInit::Const → const_param) or deferred
        // expressions (DefaultInit::Expr → lower_expr) that may
        // construct composite values like `TradeKernel { ... }` at
        // the instantiation site.
        //
        // While evaluating field defaults / overrides, allocations
        // created by composite literals (the only kind that allocs
        // at this point) should land in THE NEW LOCUS'S arena —
        // they're effectively part of its initial state. We achieve
        // that by setting `current_arena_override` to the new
        // arena ptr; arena_alloc's lookup prefers an override over
        // both `current_self` (the parent, here) and the program
        // global.
        let prev_arena_override = self.current_arena_override;
        self.current_arena_override = Some(new_arena.into_pointer_value());
        for (fname, default) in info.defaults.iter() {
            let (val, val_ty) = if let Some(expr) = overrides.get(fname.as_str())
            {
                self.lower_expr(expr, scope)?
            } else {
                match default {
                    DefaultInit::Const(pv) => self.const_param(pv),
                    DefaultInit::Expr(e) => self.lower_expr(e, scope)?,
                }
            };
            let (slot_idx, declared_ty) = info
                .fields
                .get(fname)
                .cloned()
                .expect("field declared by declare_locus_struct");
            if val_ty != declared_ty {
                return Err(CodegenError::Unsupported(format!(
                    "locus `{}` field `{}` type mismatch: declared {:?}, \
                     got {:?}",
                    locus_name, fname, declared_ty, val_ty
                )));
            }
            let field_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    slot_idx,
                    &format!("{}.{}.ptr", locus_name, fname),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(field_ptr, val)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        self.current_arena_override = prev_arena_override;

        // Zero-init the synthetic child_count field if this locus
        // declares accept. The children array slots are written
        // on accept dispatch; only the counter must start at 0.
        if let Some(cnt_idx) = info.child_count_field_idx {
            let cnt_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    cnt_idx,
                    &format!("{}.child_count.ptr", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let zero = self.context.i64_type().const_int(0, false);
            self.builder
                .build_store(cnt_ptr, zero)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }

        // F.7 ordering: if we're inside a parent locus's lifecycle
        // method AND the parent has an accept(child: ThisLocus) that
        // matches our type, call parent.accept(parent_self, child)
        // BEFORE this child's own birth. This is how
        // `02-parent-child` wires the coordinator's accept callback
        // to each greeter instantiation in run().
        //
        // Additionally, when the parent's children array exists
        // (accept declared), append the child's self_ptr to it +
        // bump child_count so `for child in self.children { ... }`
        // can iterate later.
        if let Some(parent_self) = self.current_self.clone() {
            let parent_info = self
                .user_loci
                .get(&parent_self.locus_name)
                .cloned()
                .expect("current_self points to a declared locus");
            if let Some((_, expected_child)) = &parent_info.accept_param {
                if expected_child == locus_name {
                    let accept_fn = parent_info
                        .methods
                        .get("accept")
                        .copied()
                        .expect("accept_param implies accept method");
                    self.builder
                        .build_call(
                            accept_fn,
                            &[
                                parent_self.self_ptr.into(),
                                self_ptr.into(),
                            ],
                            &format!("{}.accept.call", parent_self.locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    // Append child_self → parent.children[child_count++]
                    if let (Some(arr_idx), Some(cnt_idx)) = (
                        parent_info.children_field_idx,
                        parent_info.child_count_field_idx,
                    ) {
                        let i64_t = self.context.i64_type();
                        let cnt_ptr = self
                            .builder
                            .build_struct_gep(
                                parent_info.struct_ty,
                                parent_self.self_ptr,
                                cnt_idx,
                                "child.count.ptr",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        let cur = self
                            .builder
                            .build_load(i64_t, cnt_ptr, "child.count")
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                            .into_int_value();
                        let arr_ptr = self
                            .builder
                            .build_struct_gep(
                                parent_info.struct_ty,
                                parent_self.self_ptr,
                                arr_idx,
                                "children.ptr",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        let i32_t = self.context.i32_type();
                        let ptr_t = self
                            .context
                            .ptr_type(AddressSpace::default());
                        let arr_ty = ptr_t.array_type(CHILDREN_CAP);
                        let slot = unsafe {
                            self.builder
                                .build_gep(
                                    arr_ty,
                                    arr_ptr,
                                    &[i32_t.const_int(0, false), cur],
                                    "child.slot.ptr",
                                )
                                .map_err(|e| {
                                    CodegenError::LlvmEmit(e.to_string())
                                })?
                        };
                        self.builder
                            .build_store(slot, self_ptr)
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        let one = i64_t.const_int(1, false);
                        let next = self
                            .builder
                            .build_int_add(cur, one, "child.count.next")
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        self.builder
                            .build_store(cnt_ptr, next)
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    }
                }
            }
        }

        // Bus subscription registration runs BEFORE birth so a
        // locus's own birth() can publish on subjects it
        // subscribes to (rare but legal). For each declared
        // `bus subscribe "S" as h ...`: append (S, self_ptr,
        // <Locus>.h) into the global bus table.
        for (subject, handler_name) in &info.subscriptions {
            let handler_fn = info
                .user_methods
                .get(handler_name)
                .copied()
                .ok_or_else(|| {
                    CodegenError::Unsupported(format!(
                        "locus `{}` subscribes to `{}` with handler `{}` \
                         but no such method declared",
                        locus_name, subject, handler_name
                    ))
                })?;
            self.emit_bus_register(subject, self_ptr, handler_fn)?;
        }

        // Fire birth → run in order. drain → dissolve are deferred
        // if this locus is long-lived (has any bus subscribe), so
        // it can keep receiving published events until its
        // enclosing scope ends. Otherwise (ephemeral), all four
        // fire immediately like before.
        //
        // F.4 depth-first cascade: any child loci instantiated
        // inside this locus's run() body have already gone
        // through their full birth → run → drain → dissolve
        // sequence (each via this same lowering, recursively)
        // before run() returns. Long-lived loci defer drain →
        // dissolve to scope end via `deferred_dissolves`; the
        // cascade still fires depth-first when those scope-exit
        // calls run.
        let is_long_lived = !info.subscriptions.is_empty();
        for kind in &["birth", "run"] {
            if let Some(method) = info.methods.get(kind) {
                self.builder
                    .build_call(
                        *method,
                        &[self_ptr.into()],
                        &format!("{}.{}.call", locus_name, kind),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
        }
        if !is_long_lived {
            // drain → __closures → dissolve. Mirrors the
            // interpreter ordering in eval.rs::dissolve_locus:
            // drain body fires first, then dissolve-epoch closures
            // are evaluated, then the user's dissolve() body.
            if let Some(drain_fn) = info.methods.get("drain") {
                self.builder
                    .build_call(
                        *drain_fn,
                        &[self_ptr.into()],
                        &format!("{}.drain.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            if let Some(closures_fn) = info.closures_fn {
                let (parent_self, handler_ptr) =
                    self.resolve_failure_route(&locus_name);
                self.builder
                    .build_call(
                        closures_fn,
                        &[
                            self_ptr.into(),
                            parent_self.into(),
                            handler_ptr.into(),
                        ],
                        &format!("{}.__closures.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            if let Some(dissolve_fn) = info.methods.get("dissolve") {
                self.builder
                    .build_call(
                        *dissolve_fn,
                        &[self_ptr.into()],
                        &format!("{}.dissolve.call", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            // Wholesale-free the locus's arena. Per spec/memory.md:
            // "When the locus dissolves, the region is freed
            // wholesale." Anything allocated for this locus —
            // composite-default literals, ClosureViolations, bus
            // payload copies it received — goes here.
            self.emit_locus_arena_destroy(&info, self_ptr, locus_name)?;
        } else if let Some(top) = self.deferred_dissolves.last_mut() {
            top.push((self_ptr, locus_name.to_string()));
        } else {
            // Should be unreachable: every fn body / lifecycle
            // body opens a frame in lower_program/method body
            // setup. If we hit this, the locus instantiation is
            // outside any tracked scope and won't get cleaned up.
            return Err(CodegenError::Unsupported(format!(
                "long-lived locus `{}` instantiated outside any tracked \
                 scope (no deferred-dissolve frame)",
                locus_name
            )));
        }

        Ok(self_ptr)
    }

    /// Lower a `self.method(args)` call. Resolves the method on the
    /// current locus's `user_methods` table and emits a call with
    /// `self_ptr` prepended. Returns the lowered value + type when
    /// the method has a return type, or `None` for void methods.
    fn lower_self_method_call(
        &mut self,
        method_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<Option<(BasicValueEnum<'ctx>, LotusType)>, CodegenError> {
        let cs = self.current_self.as_ref().cloned().ok_or_else(|| {
            CodegenError::Unsupported(format!(
                "self.{}(...) outside a locus method",
                method_name
            ))
        })?;
        let info = self
            .user_loci
            .get(&cs.locus_name)
            .cloned()
            .expect("current_self points to a declared locus");
        let func = info
            .user_methods
            .get(method_name)
            .copied()
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "locus `{}` has no method `{}`",
                    cs.locus_name, method_name
                ))
            })?;
        // Find the source-level decl so we can read param / ret
        // types. Methods come from either LocusMember::Fn (named
        // user fns) or LocusMember::Mode (synthetic name from
        // ModeKind). We extract a uniform (params, ret) tuple
        // from whichever matches.
        struct MethodSig {
            params: Vec<Param>,
            ret: Option<TypeExpr>,
        }
        let sig: MethodSig = self
            .program
            .items
            .iter()
            .find_map(|item| match item {
                TopDecl::Locus(l) if l.name.name == cs.locus_name => l
                    .members
                    .iter()
                    .find_map(|m| match m {
                        LocusMember::Fn(fd) if fd.name.name == method_name => {
                            Some(MethodSig {
                                params: fd.params.clone(),
                                ret: fd.ret.clone(),
                            })
                        }
                        LocusMember::Mode(md) => {
                            let n = match md.kind {
                                ModeKind::Bulk => "bulk",
                                ModeKind::Harmonic => "harmonic",
                                ModeKind::Resolution => "resolution",
                            };
                            if n == method_name {
                                Some(MethodSig {
                                    params: md.params.clone(),
                                    ret: md.ret.clone(),
                                })
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }),
                _ => None,
            })
            .expect("method declaration was visited in pass A2");
        if args.len() != sig.params.len() {
            return Err(CodegenError::Unsupported(format!(
                "self.{}: expected {} args, got {}",
                method_name,
                sig.params.len(),
                args.len()
            )));
        }
        let mut llvm_args: Vec<BasicMetadataValueEnum> =
            Vec::with_capacity(args.len() + 1);
        llvm_args.push(cs.self_ptr.into());
        for (i, a) in args.iter().enumerate() {
            let (v, ty) = self.lower_expr(a, scope)?;
            let want = self.type_expr_to_lotus(&sig.params[i].ty)?;
            if ty != want {
                return Err(CodegenError::Unsupported(format!(
                    "self.{} arg {} type mismatch: expected {:?}, got {:?}",
                    method_name, i, want, ty
                )));
            }
            llvm_args.push(v.into());
        }
        let call = self
            .builder
            .build_call(
                func,
                &llvm_args,
                &format!("self.{}.call", method_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        match &sig.ret {
            None => Ok(None),
            Some(t) => {
                let rt = self.type_expr_to_lotus(t)?;
                let v = call
                    .try_as_basic_value()
                    .left()
                    .expect("non-void method returns a basic value");
                Ok(Some((v, rt)))
            }
        }
    }

    /// Lower one closure assertion `left ~~ right within tol` as
    /// the body of the synthetic `<Locus>.__closures` fn:
    ///
    /// ```text
    ///   diff = abs(left - right)
    ///   pass = diff <= tolerance
    ///   if pass: continue
    ///   else:
    ///     dprintf(2, "ClosureViolation: locus `L` closure `C` failed at dissolve\n")
    ///     exit(1)
    /// ```
    ///
    /// Operand types must match each other AND the tolerance type.
    /// v0 supports Int / Duration / Float / Decimal closures.
    /// String / Bool / record-typed closures are rejected (would
    /// need a domain-specific approx-equal operator anyway).
    fn lower_closure_check(
        &mut self,
        locus_name: &str,
        closure_name: &str,
        ass: &ClosureAssertion,
        parent_self_or_null: PointerValue<'ctx>,
        on_failure_or_null: PointerValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let scope = Scope::default();
        let (lv, lt) = self.lower_expr(&ass.left, &scope)?;
        let (rv, rt) = self.lower_expr(&ass.right, &scope)?;
        if lt != rt {
            return Err(CodegenError::Unsupported(format!(
                "closure `{}` on `{}`: left/right types differ ({:?} vs {:?})",
                closure_name, locus_name, lt, rt
            )));
        }
        let (tv, tt) = self.lower_expr(&ass.tolerance, &scope)?;
        if tt != lt {
            return Err(CodegenError::Unsupported(format!(
                "closure `{}` on `{}`: tolerance type differs ({:?} vs operand {:?})",
                closure_name, locus_name, tt, lt
            )));
        }

        // Track the signed-i64 diff for Int/Duration closures so
        // we can populate ClosureViolation.diff at routing time.
        // For Float/Decimal closures, diff is f64 and we store 0
        // in the violation (the interpreter exposes a polymorphic
        // diff there, which v0 codegen's static struct can't
        // express).
        let mut int_diff: Option<inkwell::values::IntValue<'ctx>> = None;

        let pass = match &lt {
            LotusType::Int | LotusType::Duration => {
                let l = lv.into_int_value();
                let r = rv.into_int_value();
                let t = tv.into_int_value();
                let diff = self
                    .builder
                    .build_int_sub(l, r, "diff")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                int_diff = Some(diff);
                let zero = self.context.i64_type().const_int(0, false);
                let neg = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::SLT,
                        diff,
                        zero,
                        "diff.neg",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let neg_diff = self
                    .builder
                    .build_int_sub(zero, diff, "diff.neg.val")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let abs = self
                    .builder
                    .build_select(neg, neg_diff, diff, "abs")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                self.builder
                    .build_int_compare(
                        inkwell::IntPredicate::SLE,
                        abs,
                        t,
                        "closure.pass",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            }
            LotusType::Float | LotusType::Decimal => {
                let l = lv.into_float_value();
                let r = rv.into_float_value();
                let t = tv.into_float_value();
                let diff = self
                    .builder
                    .build_float_sub(l, r, "fdiff")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let neg_diff = self
                    .builder
                    .build_float_neg(diff, "fdiff.neg")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let zero = self.context.f64_type().const_float(0.0);
                let is_neg = self
                    .builder
                    .build_float_compare(
                        inkwell::FloatPredicate::OLT,
                        diff,
                        zero,
                        "fdiff.neg",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let abs = self
                    .builder
                    .build_select(is_neg, neg_diff, diff, "fabs")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_float_value();
                self.builder
                    .build_float_compare(
                        inkwell::FloatPredicate::OLE,
                        abs,
                        t,
                        "closure.pass",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            }
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "closure `{}` on `{}`: ~~ not defined for {:?}",
                    closure_name, locus_name, other
                )));
            }
        };

        let func = self
            .current_fn
            .expect("current_fn set in __closures body");
        let cont_bb = self
            .context
            .append_basic_block(func, "closure.cont");
        let fail_bb = self
            .context
            .append_basic_block(func, "closure.fail");
        self.builder
            .build_conditional_branch(pass, cont_bb, fail_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // fail_bb: route to parent's on_failure if non-null,
        // else fall back to dprintf+exit.
        self.builder.position_at_end(fail_bb);
        let route_bb = self
            .context
            .append_basic_block(func, "closure.fail.route");
        let bare_bb = self
            .context
            .append_basic_block(func, "closure.fail.bare");
        let post_bb = self
            .context
            .append_basic_block(func, "closure.fail.post");
        let null_check = self
            .builder
            .build_is_not_null(on_failure_or_null, "has.handler")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(null_check, route_bb, bare_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // route_bb: build a ClosureViolation, call parent's
        // on_failure(parent_self, child_self, violation). If the
        // handler returns (absorb), continue; if it bubbles, the
        // bubble path inside the handler exits the program before
        // returning. Either way we just branch to post_bb.
        self.builder.position_at_end(route_bb);
        let viol_info = self
            .user_types
            .get("ClosureViolation")
            .cloned()
            .expect("ClosureViolation declared at startup");
        let size = viol_info
            .struct_ty
            .size_of()
            .expect("violation struct has known size");
        let viol_ptr = self.arena_alloc(size, "viol.alloc")?;
        let locus_str = self.global_string(locus_name);
        let closure_str = self.global_string(closure_name);
        let f0 = self
            .builder
            .build_struct_gep(
                viol_info.struct_ty,
                viol_ptr,
                0,
                "viol.locus.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(f0, locus_str)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f1 = self
            .builder
            .build_struct_gep(
                viol_info.struct_ty,
                viol_ptr,
                1,
                "viol.closure.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(f1, closure_str)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f2 = self
            .builder
            .build_struct_gep(
                viol_info.struct_ty,
                viol_ptr,
                2,
                "viol.diff.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let i64_t = self.context.i64_type();
        let diff_val = int_diff.unwrap_or_else(|| i64_t.const_int(0, false));
        self.builder
            .build_store(f2, diff_val)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // self.current_self.self_ptr is the failing locus's self —
        // pass it as the child_self arg.
        let child_self = self
            .current_self
            .as_ref()
            .expect("__closures runs with current_self set")
            .self_ptr;
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let void_t = self.context.void_type();
        let handler_callee_ty = void_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), ptr_t.into()],
            false,
        );
        self.builder
            .build_indirect_call(
                handler_callee_ty,
                on_failure_or_null,
                &[
                    parent_self_or_null.into(),
                    child_self.into(),
                    viol_ptr.into(),
                ],
                "on_failure.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(post_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // bare_bb: no handler — emit the v0 fallback report and
        // exit(1).
        self.builder.position_at_end(bare_bb);
        let msg = format!(
            "ClosureViolation: locus `{}` closure `{}` failed at dissolve\n",
            locus_name, closure_name
        );
        let msg_ptr = self.global_string(&msg);
        let dprintf_fn = self
            .module
            .get_function("dprintf")
            .expect("dprintf declared in declare_builtins");
        let i32_t = self.context.i32_type();
        let stderr_fd = i32_t.const_int(2, false);
        self.builder
            .build_call(
                dprintf_fn,
                &[stderr_fd.into(), msg_ptr.into()],
                "closure.dprintf",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let exit_fn = self
            .module
            .get_function("exit")
            .expect("exit declared in declare_builtins");
        self.builder
            .build_call(
                exit_fn,
                &[i32_t.const_int(1, false).into()],
                "closure.exit",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unreachable()
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // post_bb: parent absorbed → continue with next closure.
        self.builder.position_at_end(post_bb);
        self.builder
            .build_unconditional_branch(cont_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // cont_bb: continue with next closure (or fall off body).
        self.builder.position_at_end(cont_bb);
        Ok(())
    }

    /// Lower `bubble(err);` — the F.9 re-raise primitive. Inside
    /// an `on_failure` handler body, `bubble(err)` reports the
    /// violation to stderr and exits the process non-zero. v0
    /// codegen prints a fixed "ClosureViolation: bubbled" message;
    /// preserving the original violation's locus/closure fields
    /// would require reading them off the err pointer, which works
    /// but isn't required to ship 03c.
    fn lower_bubble_call(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "bubble() takes exactly one argument, got {}",
                args.len()
            )));
        }
        // Read err.locus + err.closure off the violation, format
        // the standard violation message, dprintf to stderr, exit.
        let (val, ty) = self.lower_expr(&args[0], scope)?;
        if ty != LotusType::TypeRef("ClosureViolation".into()) {
            return Err(CodegenError::Unsupported(format!(
                "bubble() requires a ClosureViolation; got {:?}",
                ty
            )));
        }
        let viol_info = self
            .user_types
            .get("ClosureViolation")
            .cloned()
            .expect("ClosureViolation declared at startup");
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let viol_ptr = val.into_pointer_value();
        let locus_field = self
            .builder
            .build_struct_gep(
                viol_info.struct_ty,
                viol_ptr,
                0,
                "bubble.locus.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let locus_str = self
            .builder
            .build_load(ptr_t, locus_field, "bubble.locus")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let closure_field = self
            .builder
            .build_struct_gep(
                viol_info.struct_ty,
                viol_ptr,
                1,
                "bubble.closure.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let closure_str = self
            .builder
            .build_load(ptr_t, closure_field, "bubble.closure")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Flush all stdio (NULL = all open streams) so any pending
        // stdout output from a preceding println in this on_failure
        // body lands before our stderr message.
        let fflush_fn = self
            .module
            .get_function("fflush")
            .expect("fflush declared");
        self.builder
            .build_call(
                fflush_fn,
                &[ptr_t.const_null().into()],
                "bubble.fflush",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let fmt = self.global_string(
            "runtime error: ClosureViolation: locus `%s` closure `%s` failed at dissolve\n",
        );
        let dprintf_fn = self
            .module
            .get_function("dprintf")
            .expect("dprintf declared");
        let i32_t = self.context.i32_type();
        self.builder
            .build_call(
                dprintf_fn,
                &[
                    i32_t.const_int(2, false).into(),
                    fmt.into(),
                    locus_str.into(),
                    closure_str.into(),
                ],
                "bubble.dprintf",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let exit_fn = self
            .module
            .get_function("exit")
            .expect("exit declared");
        self.builder
            .build_call(
                exit_fn,
                &[i32_t.const_int(1, false).into()],
                "bubble.exit",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // bubble doesn't return; emit unreachable to close the
        // current bb so subsequent stmts produce no IR.
        self.builder
            .build_unreachable()
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(BlockEnd::Terminated)
    }

    /// Lower a `subject <- payload;` statement to a call into the
    /// generated `lotus.bus_dispatch` fn. Subject must be a String
    /// literal (or evaluate to a String pointer); payload must be
    /// a TypeRef value (a pointer to a user-type struct). The
    /// dispatch fn linear-scans the global subscription table and
    /// invokes each matching handler with `(self_ptr, payload_ptr)`.
    fn lower_send(
        &mut self,
        subject: &Expr,
        value: &Expr,
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        let bus = self.bus_state.ok_or_else(|| {
            CodegenError::Unsupported(
                "bus send `<-` used but no `bus subscribe` declared in \
                 program — nothing to dispatch to"
                    .to_string(),
            )
        })?;
        let (subj_val, subj_ty) = self.lower_expr(subject, scope)?;
        if subj_ty != LotusType::String {
            return Err(CodegenError::Unsupported(format!(
                "bus send subject must be String; got {:?}",
                subj_ty
            )));
        }
        let (payload_val, payload_ty) = self.lower_expr(value, scope)?;
        let payload_type_name = match &payload_ty {
            LotusType::TypeRef(name) => name.clone(),
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "bus send payload must be a user-type value; got {:?}",
                    other
                )));
            }
        };
        let payload_info = self
            .user_types
            .get(&payload_type_name)
            .cloned()
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "bus payload type `{}` not declared",
                    payload_type_name
                ))
            })?;
        let payload_size = payload_info
            .struct_ty
            .size_of()
            .expect("payload struct has known size");
        self.builder
            .build_call(
                bus.dispatch_fn,
                &[subj_val.into(), payload_val.into(), payload_size.into()],
                "bus.dispatch.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// Lower a user-type struct literal `T { f: v, ... }` to an
    /// alloca + per-field store, returning the pointer. Every
    /// field declared on `T` must be supplied by the literal —
    /// type literals don't have defaults at codegen v0 (only
    /// locus params do). Bus payloads will use this same lowering
    /// in m12 once `<-` dispatch lands.
    fn lower_user_type_instantiation(
        &mut self,
        type_name: &str,
        inits: &[StructInit],
        scope: &Scope<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let info = self
            .user_types
            .get(type_name)
            .cloned()
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "no type `{}` declared",
                    type_name
                ))
            })?;
        let by_name: BTreeMap<&str, &Expr> = inits
            .iter()
            .map(|i| (i.name.name.as_str(), &i.value))
            .collect();
        for fname in &info.field_order {
            if !by_name.contains_key(fname.as_str()) {
                return Err(CodegenError::Unsupported(format!(
                    "type `{}` literal missing field `{}` (no defaults at \
                     codegen v0)",
                    type_name, fname
                )));
            }
        }

        // Allocate from the lotus arena so the struct outlives the
        // current stack frame. Bus payloads (publisher's frame
        // returns before subscribers finish reading), composite
        // locus param defaults (the default-init expr runs in
        // lower_locus_instantiation, but the resulting pointer is
        // stored on the locus and read later) — both need
        // long-lived storage. m19's arena holds them for the
        // lifetime of the program; m20 will scope to the
        // enclosing locus.
        let size = info
            .struct_ty
            .size_of()
            .expect("user struct has known size");
        let self_ptr = self.arena_alloc(size, &format!("{}.alloc", type_name))?;
        for fname in &info.field_order {
            let expr = by_name
                .get(fname.as_str())
                .copied()
                .expect("field presence checked above");
            let (val, val_ty) = self.lower_expr(expr, scope)?;
            let (idx, declared_ty) = info
                .fields
                .get(fname)
                .cloned()
                .expect("field declared by declare_user_type");
            if val_ty != declared_ty {
                return Err(CodegenError::Unsupported(format!(
                    "type `{}` field `{}` type mismatch: declared {:?}, \
                     got {:?}",
                    type_name, fname, declared_ty, val_ty
                )));
            }
            let field_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    idx,
                    &format!("{}.{}.ptr", type_name, fname),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(field_ptr, val)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        Ok(self_ptr)
    }

    fn global_string(&mut self, s: &str) -> PointerValue<'ctx> {
        let g = self
            .builder
            .build_global_string_ptr(s, "s")
            .expect("build_global_string_ptr");
        g.as_pointer_value()
    }

    /// Allocate `size` bytes through the lotus region allocator.
    /// 8-byte alignment, matching the natural alignment for every
    /// scalar lotus type. The returned ptr is alive until the
    /// arena it came from is destroyed.
    ///
    /// Arena selection (m20):
    /// 1. `current_arena_override` — set during locus-instantiation
    ///    field init so composite-literal defaults land in the new
    ///    locus's arena.
    /// 2. `current_self`'s arena field — when we're in a lifecycle
    ///    method body, the locus's own arena. The arena field is
    ///    always struct slot 0 (`arena_field_idx`).
    /// 3. `@lotus.arena.global` — the program-wide arena (used in
    ///    main and free fns, where there's no enclosing locus).
    fn arena_alloc(
        &mut self,
        size: inkwell::values::IntValue<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let arena_ptr = self.current_arena_ptr()?;
        let i64_t = self.context.i64_type();
        let alloc_fn = self
            .module
            .get_function("lotus_arena_alloc")
            .expect("lotus_arena_alloc declared");
        let align = i64_t.const_int(8, false);
        let raw = self
            .builder
            .build_call(
                alloc_fn,
                &[arena_ptr.into(), size.into(), align.into()],
                name,
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_arena_alloc returns ptr");
        Ok(raw.into_pointer_value())
    }

    /// The arena pointer to allocate from at the current builder
    /// position. See `arena_alloc` for the priority order.
    fn current_arena_ptr(
        &mut self,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        if let Some(p) = self.current_arena_override {
            return Ok(p);
        }
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        if let Some(cs) = self.current_self.clone() {
            let info = self
                .user_loci
                .get(&cs.locus_name)
                .cloned()
                .expect("current_self points to a declared locus");
            let arena_field_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    cs.self_ptr,
                    info.arena_field_idx,
                    "self.__arena.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let arena_ptr = self
                .builder
                .build_load(ptr_t, arena_field_ptr, "self.__arena")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            return Ok(arena_ptr.into_pointer_value());
        }
        let arena_global = self
            .module
            .get_global("lotus.arena.global")
            .expect("arena global declared");
        let arena_ptr = self
            .builder
            .build_load(ptr_t, arena_global.as_pointer_value(), "arena.cur")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(arena_ptr.into_pointer_value())
    }

    /// Emit `lotus_arena_destroy(<load self_ptr->__arena>)` for a
    /// just-dissolved locus. Used in both the ephemeral-locus
    /// dissolve path (lower_locus_instantiation) and the deferred-
    /// dissolve flush at body exit. Safe to call after the
    /// dissolve method body has run; the arena is the LAST piece
    /// of the locus's state to go.
    fn emit_locus_arena_destroy(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        locus_name: &str,
    ) -> Result<(), CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let arena_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.arena_field_idx,
                &format!("{}.__arena.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let arena = self
            .builder
            .build_load(ptr_t, arena_field_ptr, &format!("{}.__arena", locus_name))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let destroy = self
            .module
            .get_function("lotus_arena_destroy")
            .expect("lotus_arena_destroy declared");
        self.builder
            .build_call(destroy, &[arena.into()], &format!("{}.arena.destroy", locus_name))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
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
    Decimal(f64),
    /// Time literal stored as its source spelling.
    Time(String),
}

fn param_value(e: &Expr) -> Result<ParamValue, CodegenError> {
    match e {
        Expr::Literal(Literal::String(s), _) => Ok(ParamValue::String(s.clone())),
        Expr::Literal(Literal::Int(n), _) => Ok(ParamValue::Int(*n)),
        Expr::Literal(Literal::Float(f), _) => Ok(ParamValue::Float(*f)),
        Expr::Literal(Literal::Bool(b), _) => Ok(ParamValue::Bool(*b)),
        Expr::Literal(Literal::Duration(ns), _) => Ok(ParamValue::Duration(*ns)),
        Expr::Literal(Literal::Decimal(s), _) => {
            let stripped = s.strip_suffix('d').unwrap_or(s);
            let f = stripped.parse::<f64>().map_err(|e| {
                CodegenError::Unsupported(format!(
                    "Decimal literal `{}` failed to parse: {}",
                    s, e
                ))
            })?;
            Ok(ParamValue::Decimal(f))
        }
        Expr::Literal(Literal::Time(s), _) => Ok(ParamValue::Time(s.clone())),
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
