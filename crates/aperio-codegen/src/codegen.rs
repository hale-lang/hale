//! AST → LLVM IR → object file → executable, for the
//! milestone-0 subset.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use inkwell::basic_block::BasicBlock;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
    TargetTriple,
};
use inkwell::types::{BasicType, StructType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, FunctionValue, PointerValue,
};
use inkwell::{AddressSpace, OptimizationLevel};

use aperio_syntax::ast::*;

/// Compile-time tag for a value's type. Mirrors a small subset
/// of `aperio_types::Ty`; we don't pull the full type system in
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
enum CodegenTy {
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
    /// m89: opaque byte buffer. Distinct from `String` because
    /// String is NUL-terminated (binary content with embedded 0
    /// bytes truncates). Bytes carries an explicit length, so
    /// it's the right type for binary file I/O, raw HTTP bodies,
    /// images, etc.
    ///
    /// Representation: a single `ptr` value at the LLVM level,
    /// pointing to an arena-allocated blob laid out as
    /// `[i64 len][u8 data[len]]`. Same single-pointer ABI as
    /// String — fits return-by-value through the m49 calling
    /// convention without struct-typed return shenanigans.
    /// Length is read by dereferencing the prefix; the data
    /// pointer is the blob plus 8 bytes.
    Bytes,
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
    /// m47: enum value. v0.1 stores the enum as a single i32 tag
    /// (variant index in declaration order); no-payload-only
    /// variants — payload-bearing variants need tagged-union
    /// storage with payload bytes and aren't shipped yet. The
    /// string carries the enum name; variant names + tag table
    /// live in `Cx.user_enums`.
    Enum(String),
    /// Fixed-size array `[T; N]`. Represented at runtime as a
    /// pointer to an LLVM `[N x T]` value living in the enclosing
    /// arena (allocated at literal-creation time, freed wholesale
    /// with the locus). v0 is fixed-size only; growable arrays
    /// would need a length field + reallocation policy that the
    /// region allocator's bump-list shape doesn't fit cleanly.
    Array(Box<CodegenTy>, u64),
    /// Tuple `(T1, T2, ...)`. Anonymous heterogeneous record;
    /// arity-fixed at compile time, no field names. Lowered as
    /// a pointer to an arena-allocated anonymous LLVM struct.
    /// The component types live in the Vec, in declaration
    /// order, and access happens through numeric field syntax
    /// (`t.0`, `t.1`, ...) or via tuple-destructuring let /
    /// match patterns.
    Tuple(Vec<CodegenTy>),
    /// m80: function pointer. `fn(T1, T2) -> R` or `fn(T1, T2)`
    /// for void-returning. At LLVM level, stored as a `ptr` (raw
    /// function pointer); calls go through `build_indirect_call`
    /// with an `inkwell::FunctionType` synthesized from this
    /// CodegenTy's args + ret. Used by stdlib loci that take a
    /// user-supplied callback (m82's Listener.on_connection); also
    /// available for general user-code callback patterns.
    FnPtr {
        args: Vec<CodegenTy>,
        ret: Option<Box<CodegenTy>>,
    },
    /// F.20 Phase B: interface value. The string carries the
    /// interface name; method order + signatures live in
    /// `Cx.user_interfaces`. Lowered as a `ptr` at the LLVM
    /// level pointing to an arena-allocated fat-pointer struct
    /// `{ i8* data, i8* vtable }`. `data` is the underlying
    /// locus pointer (single-pointer LocusRef ABI); `vtable`
    /// is a per-(locus, interface) static global of fn pointers
    /// indexed by interface-method declaration order. Built at
    /// the call site where a locus flows into an interface slot;
    /// dispatch loads vtable[i] and indirect-calls with data
    /// as the implicit self arg.
    Interface(String),
    /// F.22 capacity-slot cell handle. `acquire()` / `alloc()`
    /// return one of these; `release(cell)` / `free(cell)`
    /// accept one. The boxed `CodegenTy` is the cell's element
    /// type (T from `pool X of T` / `heap Y of T`). At LLVM
    /// level, a `ptr` to T's struct layout.
    ///
    /// v1.x-2: struct cells expose `cell.field` read/write.
    /// v1.x-5: the second field carries slot origin
    /// `(locus_name, slot_name)` so release/free can reject
    /// cells released into a different slot of the same shape.
    /// `None` reserved for future generic-Cell<T> positions
    /// (e.g. fn args that take any cell); v1 always sets it.
    Cell(Box<CodegenTy>, Option<(String, String)>),
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

    // m73a: parse the bundled stdlib source and merge its decls
    // into the user program before lowering. Stdlib loci land in
    // `user_loci` alongside user-declared loci with no special
    // casing in the lowering passes; collision with user names is
    // prevented by the `__Std*` mangled prefix on bundled decls.
    let stdlib_program = aperio_syntax::parse_source(STDLIB_AP_SOURCE)
        .map_err(|diags| {
            let summary = diags
                .iter()
                .map(|d| format!("{:?}", d))
                .collect::<Vec<_>>()
                .join("; ");
            CodegenError::Unsupported(format!("stdlib parse: {}", summary))
        })?;
    // MOA substrate (moa::* path prefix) — same parse-and-merge as
    // stdlib, with its own source constant and path-rename table.
    // Bundled into every binary alongside stdlib. See `moa/MOA.md`
    // for the architectural pattern these types support.
    let moa_program = aperio_syntax::parse_source(MOA_AP_SOURCE)
        .map_err(|diags| {
            let summary = diags
                .iter()
                .map(|d| format!("{:?}", d))
                .collect::<Vec<_>>()
                .join("; ");
            CodegenError::Unsupported(format!("moa parse: {}", summary))
        })?;
    let mut merged = program.clone();
    merged.items.extend(stdlib_program.items);
    merged.items.extend(moa_program.items);

    let context = Context::create();
    let module = context.create_module("lotus_main");
    let builder = context.create_builder();

    let mut cx = Cx {
        context: &context,
        module,
        builder,
        program: &merged,
        current_fn: None,
        current_user_fn_ret: None,
        current_self: None,
        loops: Vec::new(),
        user_fns: BTreeMap::new(),
        user_loci: BTreeMap::new(),
        user_types: BTreeMap::new(),
        user_enums: BTreeMap::new(),
        user_interfaces: BTreeSet::new(),
        bus_state: None,
        deferred_dissolves: Vec::new(),
        in_main: false,
        current_arena_override: None,
        current_user_fn_caller_arena: None,
        current_user_fn_arena: None,
        current_user_fn_exit_bb: None,
        current_user_fn_ret_alloca: None,
        accumulator_ctx: None,
        serializers: BTreeMap::new(),
        generic_fn_templates: BTreeMap::new(),
        generic_locus_templates: BTreeMap::new(),
        defer_next_locus_dissolve: false,
        vtables: BTreeMap::new(),
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
    if std::env::var("LOTUS_DUMP_IR").is_ok() {
        let ir_path = output_path.with_extension("ll");
        let _ = cx.module.print_to_file(&ir_path);
    }
    machine
        .write_to_file(&cx.module, FileType::Object, &obj_path)
        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

    // Drop the lotus runtime C source next to the object file so
    // clang compiles + links it into the same binary. The C
    // source is bundled into the codegen crate via include_str!,
    // so the Aperio binary is self-contained — no separate
    // runtime install needed. Name is keyed off the object path
    // so parallel `cargo test` invocations don't race on a
    // shared filename in `/tmp`.
    let runtime_c_path = obj_path.with_extension("arena.c");
    std::fs::write(&runtime_c_path, RUNTIME_C_SOURCE)
        .map_err(|e| CodegenError::Link(format!("write runtime C: {}", e)))?;

    // m96: locate the tree-sitter shim staticlib produced by the
    // sibling `aperio-ts-shim` workspace crate. We don't try to
    // build it here (cargo handles that when the workspace is
    // built); we just check both `release/` and `debug/` profile
    // dirs and pass whichever exists. Linking unconditionally
    // means a program that doesn't use `std::ts` still pulls in
    // the shim's symbols; LLVM/clang's GC will drop unreferenced
    // ones. For ~28 MB of grammar tables that's a non-trivial
    // size cost but acceptable for v0; if it becomes painful, a
    // future flag can gate the link on `std::ts` actually being
    // referenced by the user program.
    let ts_shim_path = locate_ts_shim_staticlib();
    let mut clang = Command::new("clang");
    clang
        .arg(&obj_path)
        .arg(&runtime_c_path)
        .arg("-O2")
        // m27: pinned-class loci spawn pthreads via the
        // pthread_create / pthread_join externs declared in
        // codegen. Linker needs -lpthread to satisfy them. Link
        // unconditionally — the dependency is small and trying
        // to gate it on "did this program declare any pinned
        // loci?" would entangle the codegen pass with the link
        // step. Cost: one extra dynamic dependency in the
        // resulting binary.
        .arg("-lpthread");
    if let Some(p) = ts_shim_path.as_ref() {
        clang.arg(p);
        // Rust staticlibs depend on libdl + libm via libstd.
        // Adding these unconditionally is harmless when no
        // staticlib symbols are actually pulled in.
        clang.arg("-ldl").arg("-lm");
    }
    let status = clang
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

/// m96: find `libaperio_ts_shim.a`, the staticlib produced by the
/// sibling `aperio-ts-shim` workspace crate. Returns `None` if the
/// staticlib hasn't been built yet — the user-program link will
/// then succeed only if the program doesn't actually call any
/// `std::ts::*` primitive (the externs would resolve to undefined
/// at link time and clang would error). For the dogfood phase the
/// workspace target dir is right next to this crate; an installed
/// `aperio` binary from `cargo install` would need a packaged
/// substrate, which is a follow-up.
///
/// Lookup order: `APERIO_TS_SHIM_A` env var (explicit override),
/// then `target/release/`, then `target/debug/` relative to the
/// codegen crate's manifest dir.
fn locate_ts_shim_staticlib() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("APERIO_TS_SHIM_A") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    // CARGO_MANIFEST_DIR at build time of aperio-codegen is
    // `<workspace>/crates/aperio-codegen`. Workspace root is two
    // dirs up.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest.parent()?.parent()?;
    for profile in ["release", "debug"] {
        let p = workspace
            .join("target")
            .join(profile)
            .join("libaperio_ts_shim.a");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Bundled Aperio source for the stdlib. m73a established the
/// concat-with-user-source mechanism: the parsed stdlib `Program`
/// has its `items` appended to the user's `Program.items` before
/// `lower_program` runs, so each stdlib locus sits in `user_loci`
/// exactly like user-declared loci. Path-qualified references
/// (`std::io::tcp::Listener`) are rewritten at struct-literal
/// codegen sites to the mangled locus names declared in this
/// source via the `STDLIB_PATH_RENAMES` table below.
///
/// m93 split the single stdlib.ap into one file per domain.
/// Order matters: pass A1 walks loci in source order and resolves
/// each locus's param types as it goes. Listener references
/// Stream, so io_tcp.ap (which declares both, Stream first) lands
/// before http.ap (which references Stream in fn signatures).
/// core.ap lands first because text.ap depends on its
/// __replace_all / __html_escape helpers. test.ap is standalone
/// and could go anywhere — it ends up last by convention.
const STDLIB_AP_SOURCE: &str = concat!(
    include_str!("../runtime/stdlib/core.ap"),
    "\n",
    include_str!("../runtime/stdlib/io_tcp.ap"),
    "\n",
    include_str!("../runtime/stdlib/http.ap"),
    "\n",
    include_str!("../runtime/stdlib/text.ap"),
    "\n",
    include_str!("../runtime/stdlib/test.ap"),
    "\n",
    include_str!("../runtime/stdlib/log.ap"),
    "\n",
    // m96: tree-sitter substrate. Standalone — references only
    // path-call primitives (`std::ts::*`) plus core builtins
    // (`println`, while, assignment), so order is flexible.
    // Lands last by convention.
    include_str!("../runtime/stdlib/ts.ap"),
    "\n",
    // Post-m102: language-agnostic AST query interface.
    // Wraps `std::ts::*` + per-language node-kind strings
    // behind a single `Lang` locus. Depends on `std::ts::*`
    // path calls and `std::str::index_of`; both are path
    // calls that resolve at codegen time, so source-order
    // dependency on ts.ap is just stylistic — the
    // path-call resolution is independent of bundle order.
    include_str!("../runtime/stdlib/lang.ap"),
    "\n",
    // Corpus-extraction pass: cross-cutting helpers lifted from
    // the apps/ tree into the std seed so they stop being
    // hand-rolled per consumer. Each is a namespace lotus
    // (empty params, methods only). Order between these is
    // flexible — they reference only path-call primitives.
    include_str!("../runtime/stdlib/iter.ap"),
    "\n",
    // tagged.ap depends on iter.ap (Lines is used internally in
    // every Accumulator method), so it must land after.
    include_str!("../runtime/stdlib/tagged.ap"),
    "\n",
    // name.ap is independent — only uses std::str::index_of.
    include_str!("../runtime/stdlib/name.ap"),
    "\n",
    // json.ap depends on iter.ap for build_array's line walk.
    include_str!("../runtime/stdlib/json.ap"),
    "\n",
    // yaml.ap depends on iter.ap for Reader's line walks.
    // Mirrors json.ap's shape (Builder is a namespace lotus
    // returning Strings). Used by the codebase-onboarder's
    // skeleton + render stages.
    include_str!("../runtime/stdlib/yaml.ap"),
    "\n",
    // cli.ap is independent — only uses std::str::index_of,
    // std::str::parse_int / can_parse_int, and std::env::*
    // path calls. Lands after the other corpus helpers by
    // convention.
    include_str!("../runtime/stdlib/cli.ap"),
    "\n",
    // source.ap depends on iter.ap (Lines for entry iteration)
    // and references std::lang::Lang in its on_file fn-pointer
    // type — lang.ap must be declared before this file's parse
    // pass A1 resolves param types. lang.ap lands at line ~385
    // above, so source.ap lands here without issue.
    include_str!("../runtime/stdlib/source.ap"),
);

/// Maps each user-facing stdlib path (locus OR type) to the
/// mangled name declared in `STDLIB_AP_SOURCE`. The mangled
/// prefix (`__StdIo...`, `__StdHttp...`) makes collision with
/// user-declared identifiers impossible at v0. Each entry is
/// `&[&"std", ...]` → flat string. Whether the resolved name
/// refers to a locus or a type is determined downstream by
/// looking it up in `user_loci` / `user_types` — this table
/// is just the path → name mapping. Keep sorted by path for
/// review.
const STDLIB_PATH_RENAMES: &[(&[&str], &str)] = &[
    (&["std", "cli", "Resolver"], "__StdCliResolver"),
    (&["std", "http", "Request"], "__StdHttpRequest"),
    (&["std", "http", "Response"], "__StdHttpResponse"),
    (&["std", "io", "tcp", "Listener"], "__StdIoTcpListener"),
    (&["std", "io", "tcp", "Stream"], "__StdIoTcpStream"),
    (&["std", "iter", "Lines"], "__StdIterLines"),
    (&["std", "json", "Builder"], "__StdJsonBuilder"),
    (&["std", "lang", "Lang"], "__StdLangLang"),
    (&["std", "lang", "Morpheme"], "__StdLangMorpheme"),
    (&["std", "log", "LogEvent"], "__StdLogEvent"),
    (&["std", "log", "Logger"], "__StdLogLogger"),
    (&["std", "log", "StdoutSink"], "__StdLogStdoutSink"),
    (&["std", "name", "Convention"], "__StdNameConvention"),
    (&["std", "source", "Walk"], "__StdSourceWalk"),
    (&["std", "tagged", "Accumulator"], "__StdTaggedAccumulator"),
    (&["std", "text", "Sink"], "__StdTextSink"),
    (&["std", "text", "StdoutSink"], "__StdTextStdoutSink"),
    (&["std", "text", "StringSink"], "__StdTextStringSink"),
    (&["std", "text", "FileSink"], "__StdTextFileSink"),
    (&["std", "yaml", "Builder"], "__StdYamlBuilder"),
    (&["std", "yaml", "Reader"], "__StdYamlReader"),
];

/// MOA substrate source — bundled into every binary, parses + merges
/// alongside `STDLIB_AP_SOURCE`. Resolves under the `moa::*` path
/// prefix via `MOA_PATH_RENAMES`. Lives in `/moa/` at the repo root,
/// not in `crates/aperio-codegen/runtime/stdlib/`, because it is
/// conceptually one layer below the language and above stdlib — the
/// architectural substrate apps build on. See `moa/README.md`.
const MOA_AP_SOURCE: &str = concat!(
    include_str!("../../../moa/types.ap"),
);

/// Maps each user-facing `moa::*` path to the mangled name declared
/// in `MOA_AP_SOURCE`. Same shape as `STDLIB_PATH_RENAMES` — flat
/// after the namespace prefix (`moa::RuntimeEvent`, not
/// `moa::types::RuntimeEvent`) per the stdlib precedent. Keep sorted
/// by path for review.
const MOA_PATH_RENAMES: &[(&[&str], &str)] = &[
    (&["moa", "BraidId"], "__MoaBraidId"),
    (&["moa", "LocusId"], "__MoaLocusId"),
    (&["moa", "RuntimeEvent"], "__MoaRuntimeEvent"),
    (&["moa", "Tick"], "__MoaTick"),
];

/// Look up the mangled name for a bundled-substrate path (`std::*`
/// stdlib or `moa::*` substrate). Returns `None` when the path isn't
/// recognized; callers then surface the path-as-typed in their error
/// message. Dispatches by first segment so the two path-rename tables
/// stay independent; behavior for `std::*` is unchanged from the
/// pre-moa world.
fn stdlib_mangled_for_path(segs: &[&str]) -> Option<&'static str> {
    let table: &[(&[&str], &str)] = match segs.first() {
        Some(&"std") => STDLIB_PATH_RENAMES,
        Some(&"moa") => MOA_PATH_RENAMES,
        _ => return None,
    };
    table
        .iter()
        .find(|(p, _)| *p == segs)
        .map(|(_, name)| *name)
}


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
    current_user_fn_ret: Option<Option<CodegenTy>>,
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
    /// m47: user-defined enum declarations indexed by name. Each
    /// entry carries the variant-name → tag-index map. v0.1
    /// supports no-payload-only enums; an enum value is an i32
    /// holding the variant's tag.
    user_enums: BTreeMap<String, EnumInfo>,
    /// F.20: user-declared interface names. Phase A registers
    /// them here so `type_expr_to_codegen_ty` can resolve an
    /// interface-typed signature slot to `CodegenTy::Interface`.
    /// Phase B reads method order + signatures lazily from the
    /// AST when synthesizing a per-(locus, interface) vtable
    /// (the typechecker already enforces structural impl, so
    /// codegen just needs the layout, not a re-verified table).
    user_interfaces: BTreeSet<String>,
    /// Bus state generated when any locus declares a subscribe.
    /// `Some` iff the program contains at least one `bus subscribe`
    /// declaration. Bus storage itself lives in the C runtime
    /// (m45-followup); this is just a presence flag so a `<-` send
    /// in a program without subscribers errors clearly.
    bus_state: Option<BusState>,
    /// Stack of "deferred-dissolve" frames: each enclosing fn
    /// body / lifecycle method body opens one. Long-lived loci
    /// (any locus with a `bus subscribe` declaration) instantiated
    /// inside that body are pushed here instead of dissolving
    /// immediately, then drained + dissolved in reverse order at
    /// scope exit so they outlive synchronous publishes.
    /// Each entry is `(self_ptr, locus_name, thread_id_alloca)`.
    /// If `thread_id_alloca` is Some, the locus is pinned (m27)
    /// and the flush emits a `pthread_join(load thread_id_alloca)`
    /// before the rest of the dissolve sequence; pthread_join
    /// blocks until the pinned thread's run() returns. Cooperative
    /// long-lived loci have None.
    deferred_dissolves:
        Vec<Vec<(PointerValue<'ctx>, String, Option<PointerValue<'ctx>>)>>,
    /// True while lowering the body of `main`. `return` is treated
    /// as an exit-code return (truncated to i32) when this is set,
    /// rather than the user-fn `current_user_fn_ret` path.
    in_main: bool,
    /// When set, `arena_alloc` routes through this arena pointer
    /// instead of `current_self`'s arena field or the program
    /// global. Used during locus-instantiation field init so
    /// composite literals (`Kernel { ... }`) used as
    /// default-init values land in the *new* locus's arena
    /// rather than the parent's. Restored after the field-init
    /// loop completes.
    current_arena_override: Option<PointerValue<'ctx>>,
    /// m49: free-fn implicit-locus arenas. Set during a non-main
    /// free fn body lowering. `caller_arena_alloca` holds the
    /// implicit `__caller_arena: ptr` first param (the arena that
    /// owns the call site). `fn_arena_alloca` holds the per-call
    /// subregion of caller_arena that the fn body's allocations
    /// route through (fallback in `current_arena_ptr`). `exit_bb`
    /// is the unified return-epilogue block: every `return` stores
    /// its value to `ret_alloca` (if non-void) and br's here; the
    /// epilogue deep-copies the value into caller_arena, destroys
    /// the subregion, and emits `build_return`. Refactor avoids
    /// duplicating destroy/copy at every return site.
    current_user_fn_caller_arena: Option<PointerValue<'ctx>>,
    current_user_fn_arena: Option<PointerValue<'ctx>>,
    current_user_fn_exit_bb: Option<inkwell::basic_block::BasicBlock<'ctx>>,
    current_user_fn_ret_alloca: Option<PointerValue<'ctx>>,
    /// m46: closure-accumulator substitution context. Set by
    /// `lower_closure_check` right before lowering the assertion
    /// expressions; cleared after. When set, `lower_expr`'s Call
    /// match arm intercepts `sum(...)` calls and emits a load
    /// from the next accumulator slot (per `next_idx`) instead
    /// of doing the call. Slot order matches `collect_sum_calls`
    /// order in declare_locus_struct, so the Nth `sum` encountered
    /// during lowering corresponds to the Nth slot.
    accumulator_ctx: Option<AccumulatorCtx<'ctx>>,
    /// m60: per-payload-type serializer / deserializer fns,
    /// keyed by the type's name (e.g., "Ping" or "Greeting").
    /// Synthesized in lower_program after pass A2 once user_types
    /// + user_enums are populated; bodies are identity (memcpy)
    /// at v0.1, so observable bytes are unchanged from the
    /// pre-m60 raw-struct path. The functions exist so codegen
    /// call sites (`<-` send + bus subscribe register) can route
    /// payloads through a per-type encode/decode pair instead of
    /// memcpy'ing struct bytes inline — a future wire-format
    /// milestone replaces the bodies without touching call
    /// sites. Per notes/open-questions #10, the receiver's
    /// arena gets a fresh copy of the payload struct: the
    /// deserializer's job is to reconstruct that struct from
    /// whatever bytes the wire delivered.
    serializers: BTreeMap<String, SerializerPair<'ctx>>,
    /// m62: generic free fn templates indexed by name.
    /// Populated in lower_program from FnDecls whose
    /// `generics: Vec<GenericParam>` is non-empty. Call sites
    /// look here to find templates and trigger on-demand
    /// instantiation: lower_call_expr infers concrete type args
    /// from the actual arg CodegenTys, mangles, and synthesizes
    /// + lowers a per-instantiation specialized fn body. The
    /// resulting specialized FunctionValue lands in `user_fns`
    /// keyed by mangled name, so subsequent calls with the same
    /// type args resolve directly.
    generic_fn_templates: BTreeMap<String, FnDecl>,
    /// m63: generic locus templates indexed by name. Populated
    /// from LocusDecls with non-empty generics. Unlike generic
    /// fns (which synthesize on-demand at call sites), generic
    /// loci synthesize upfront from discovery: every TypeExpr
    /// reference to `Cache<Int, String>` triggers synthesis of
    /// a `Cache_Int_String` LocusDecl, and the synthesized
    /// decl flows through the standard A1/A2/C locus passes
    /// alongside user-written decls. Loci are typically
    /// instantiated via struct-literal syntax + bare-name
    /// resolution (`let c: Cache<Int, String> = Cache { ... };`),
    /// matching the m61b/m61c pattern for generic structs.
    generic_locus_templates: BTreeMap<String, LocusDecl>,
    /// m82: locus-all-the-way-down lifecycle. Set true by
    /// `Stmt::Let` lowering immediately before evaluating the RHS
    /// when that RHS is a locus struct literal. Consumed (via
    /// `std::mem::take`) by `lower_locus_instantiation`, which
    /// then routes the locus into the enclosing fn's
    /// `deferred_dissolves` frame instead of dissolving eagerly
    /// at the end of the struct-literal expression. The decoupling
    /// the user-visible binding (`s`) holds the handle; the locus
    /// instance lives until its binding's scope ends. Cleared
    /// between any two RHS lowerings; nested instantiations inside
    /// the same RHS evaluation (e.g. `Outer { inner: Inner{...} }`)
    /// only consume it for the outermost call, leaving inner
    /// instantiations on the eager path. Statement-position locus
    /// literals (`Stream { ... };`) are unaffected — the flag
    /// only fires from `Stmt::Let`.
    defer_next_locus_dissolve: bool,
    /// F.20 Phase B: per-(locus, interface) vtable globals.
    /// Synthesized lazily by `ensure_vtable` the first time a
    /// given locus is coerced to a given interface. Layout is
    /// `[N x ptr]` where N is the interface's method count and
    /// element i is the locus's method matching interface
    /// method i (by declaration order). The symbol name is
    /// `__vt.<locus>.<iface>`. Stored as a `GlobalValue` so the
    /// fat-pointer build site can take its address without a
    /// re-emit.
    vtables: BTreeMap<(String, String), inkwell::values::GlobalValue<'ctx>>,
}

/// m60: paired serialize / deserialize fns synthesized per bus
/// payload type. v0.1 wire format is identity (struct bytes
/// passed through memcpy) so a publisher and subscriber on the
/// same arch + same compiler version stay byte-compatible. The
/// shape is what matters: codegen routes payloads through these
/// hooks so a future wire-format change drops in by replacing
/// the function bodies, not the call sites.
#[derive(Debug, Clone, Copy)]
struct SerializerPair<'ctx> {
    /// `i64 @__serialize_T(ptr src, ptr dst, i64 cap)` — copies
    /// the struct at `src` (size known statically per T) into
    /// `dst` (caller-provided buffer of `cap` bytes), returning
    /// the number of bytes written. Identity body at v0.1 just
    /// memcpys `sizeof(T)` bytes. cap is reserved for the future
    /// wire-format pass.
    serialize: FunctionValue<'ctx>,
    /// `i64 @__deserialize_T(ptr src, i64 n, ptr dst, i64 cap)` —
    /// reads `n` wire bytes from `src` and reconstructs a struct
    /// of type T at `dst` (caller-provided buffer of `cap` bytes),
    /// returning the struct's natural size on success or -1 on
    /// error. Identity body at v0.1 memcpys `sizeof(T)` bytes
    /// (which equals `n` because the wire format is identity).
    deserialize: FunctionValue<'ctx>,
}

/// m46: substitution context for `sum(...)` lowering inside a
/// closure assertion. See `Codegen::accumulator_ctx`.
#[derive(Debug, Clone)]
struct AccumulatorCtx<'ctx> {
    slots: Vec<AccumulatorSlot>,
    next_idx: usize,
    self_ptr: PointerValue<'ctx>,
    struct_ty: inkwell::types::StructType<'ctx>,
}

/// Marker that the program contains at least one `bus subscribe`
/// declaration, so dispatch + register call sites can fail clearly
/// when a `<-` send is emitted in a program without subscribers.
/// Storage moved to the C runtime (m45-followup), so all the
/// LLVM-side handles the prior `BusState` carried are gone.
#[derive(Debug, Clone, Copy)]
struct BusState;

/// m47 (enums): per-enum metadata. Variants in declaration order;
/// each variant's i32 tag is its index in this list. m47-payloads
/// extends to payload-bearing variants: each variant carries its
/// own field types, and the enum has a unified storage struct
/// `{ i32 tag, [max x i8] body }` whose body is sized to the
/// largest variant's payload bytes. No-payload-only enums keep
/// the value-semantics i32 representation; once any variant
/// carries a payload, the whole enum switches to pointer
/// semantics (the byte-array body needs heap-or-arena storage).
#[derive(Debug, Clone)]
struct EnumInfo {
    variants: Vec<EnumVariantInfo>,
    /// True iff at least one variant has fields. Drives the
    /// representation switch: i32-tag-only when false, struct
    /// pointer when true.
    has_payload: bool,
    /// Payload bytes reserved in the enum's storage struct's
    /// `[N x i8]` body (max over all variants). Zero when
    /// has_payload is false (no body field at all).
    payload_bytes: u64,
}

#[derive(Debug, Clone)]
struct EnumVariantInfo {
    name: String,
    /// Field types in declaration order. Empty for no-payload.
    field_tys: Vec<CodegenTy>,
}

/// m46 / m46-vocab: which accumulator form a slot implements.
///
/// - **Sum**: one slot of the inner expr's type. Sample = +inner.
///   Substitute = load slot. Output type = inner's type.
/// - **Count**: one i64 slot. Sample = +1 (no inner expr).
///   Substitute = load slot. Output type = Int.
/// - **Mean**: two slots — one for the running sum (inner's type)
///   and one i64 for the count. Sample = sum += inner; count += 1.
///   Substitute = sum / count cast to Float. Output type = Float
///   always (avoids Int/Float-mean coercion edge cases; means are
///   inherently real-valued).
#[derive(Debug, Clone, Copy, PartialEq)]
enum AccumulatorKind {
    Sum,
    Count,
    Mean,
}

/// m46 / m46-vocab: one slot per accumulator call detected in a
/// closure's assertion. Each slot owns one or two struct fields
/// on the locus and accumulates state across epoch fires.
///
/// `kind` selects sample / substitute behavior.
/// `inner_expr` is the argument expression for Sum / Mean — the
/// thing being summed. None for Count.
/// `ty` is the OUTPUT type of the substitute load (Int for Count;
/// inner's type for Sum; Float for Mean). Drives the assertion's
/// left/right type unification.
/// `inner_ty` is the type of `inner_expr` itself — what gets
/// stored in the running-sum slot for Sum / Mean. Equal to `ty`
/// for Sum; differs from `ty` for Mean (sum slot stores inner's
/// type, output is Float). Unused for Count.
/// `field_idx` is the slot's primary struct field — sum slot for
/// Sum / Mean, count slot for Count.
/// `field_idx_2` is the second struct field — count slot for
/// Mean. None for Sum / Count.
#[derive(Debug, Clone)]
struct AccumulatorSlot {
    kind: AccumulatorKind,
    inner_expr: Option<Expr>,
    ty: CodegenTy,
    inner_ty: CodegenTy,
    field_idx: u32,
    field_idx_2: Option<u32>,
}

#[derive(Debug, Clone)]
struct FnSig<'ctx> {
    func: FunctionValue<'ctx>,
    params: Vec<CodegenTy>,
    /// Per-param default-value expression. Same length as
    /// `params`. `None` = required arg; `Some(expr)` = caller can
    /// omit and the default will be evaluated at the call site.
    /// Defaults must form a suffix (typecheck-validated at fn
    /// declaration), so callers omit a contiguous tail.
    defaults: Vec<Option<Expr>>,
    /// `None` = void (no return type in the Aperio declaration).
    ret: Option<CodegenTy>,
}

/// Compiled locus type. Lifecycle methods take `self_ptr` as their
/// first arg; field access in their bodies lowers to GEPs against
/// `struct_ty` using the index from `fields`.
#[derive(Debug, Clone)]
struct LocusInfo<'ctx> {
    struct_ty: StructType<'ctx>,
    /// Field name → (index in struct, field type).
    fields: BTreeMap<String, (u32, CodegenTy)>,
    /// Field initializers in declaration order. Each entry is
    /// (name, default_init). Overrides at instantiation sites
    /// replace the default for that field. Default-init can be a
    /// pre-resolved literal (so simple defaults stay cheap) OR a
    /// deferred AST expression evaluated at the instantiation
    /// site (for composite literals like
    /// `current_kernel: Kernel = Kernel { ... }` where
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
    /// locus: (subject_literal, handler_method_name,
    /// payload_type_name). At instantiation time, registration
    /// emits a 5-tuple (subject_str, self_ptr, handler_fn_ptr,
    /// mailbox_or_null, deserialize_fn_ptr) into the global
    /// bus table — the deserialize fn is looked up via the
    /// payload type name in `cx.serializers` (m60).
    subscriptions: Vec<(String, String, String)>,
    /// Closure declarations on this locus. All five epochs lower
    /// (Birth m39, Dissolve, Tick m42, Duration m43, Explicit m44).
    /// Each element is `(name, assertion, epoch)` carried over from
    /// the AST so the synthetic closure-eval fns can re-lower the
    /// assertion expressions and the synthesis pass can partition
    /// by epoch.
    closures: Vec<(String, ClosureAssertion, EpochSpec)>,
    /// m46 (closure accumulators): per-closure accumulator slots.
    /// Each `sum(expr)` call detected in a closure assertion's
    /// left/right/tolerance produces one slot. The slots are
    /// occurrence-ordered (left expr's sums first, then right's,
    /// then tolerance's) so the lowering pass can match each
    /// `sum(...)` in the lowered assertion to the right slot via
    /// a counter. Slot fields live on the locus struct after
    /// the user fields, before the synthetic flags.
    accumulators_per_closure: BTreeMap<String, Vec<AccumulatorSlot>>,
    /// m46: per-closure list of recovery-event names listed in
    /// `persists_through(...)`. Default is reset (zero the
    /// accumulators on the event); a name in this list opts that
    /// closure's accumulators out of reset for that event.
    /// Recognized event names: `restart`, `restart_in_place`,
    /// `quarantine`, `dissolve`. (`replace` from the spec example
    /// awaits perspective hot-load.)
    persists_through_per_closure: BTreeMap<String, Vec<String>>,
    /// Synthetic `<Locus>.__birth_closures(self_ptr,
    /// parent_self_or_null, on_failure_fn_or_null)` fn that
    /// evaluates every birth-epoch closure right after birth()
    /// returns. None when no birth-epoch closures exist.
    birth_closures_fn: Option<FunctionValue<'ctx>>,
    /// Synthetic `<Locus>.__dissolve_closures(self_ptr,
    /// parent_self_or_null, on_failure_fn_or_null)` fn that
    /// evaluates every dissolve-epoch closure between drain()
    /// and dissolve() per F.4 + F.9. None when no dissolve-epoch
    /// closures exist. (Pre-m39 spelling: `closures_fn`. Renamed
    /// at m39 when birth-epoch shipped so each epoch's fn has
    /// an unambiguous slot.)
    dissolve_closures_fn: Option<FunctionValue<'ctx>>,
    /// m42: Synthetic `<Locus>.__tick_closures(self_ptr,
    /// parent_self_or_null, on_failure_fn_or_null)` fn that
    /// evaluates every tick-epoch closure. Called after
    /// `run()` returns and after each bus handler invocation.
    /// None when no tick-epoch closures exist.
    tick_closures_fn: Option<FunctionValue<'ctx>>,
    /// m42: Synthetic `<Locus>.__tick_closures_wrapper(self_ptr)`
    /// thunk that loads `__parent_self` + `__parent_on_failure`
    /// from the struct and tail-calls `__tick_closures`. Used
    /// from bus-handler thunks (which only have `self`,
    /// `payload` in scope, not parent context). Always paired
    /// with `tick_closures_fn`.
    tick_wrapper_fn: Option<FunctionValue<'ctx>>,
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
    /// m40: index of the synthetic `__restart_count: i64` field.
    /// Always present (zero-init at instantiation). The
    /// `restart(child)` recovery primitive bumps it; the
    /// post-on_failure dispatch check inside synthesized
    /// `__birth_closures` reads it to decide whether to re-run
    /// birth() + birth-epoch closures. Cap is 2 attempts per
    /// locus lifetime (v0 default).
    restart_count_field_idx: u32,
    /// m41: index of the synthetic `__quarantined: i64` flag.
    /// Always present (zero-init at instantiation). The
    /// `quarantine(child)` recovery primitive sets it to 1; the
    /// lifecycle dispatch in `lower_locus_instantiation` reads
    /// it after birth() + __birth_closures and skips `run()`
    /// if set. Drain / dissolve still fire unconditionally.
    quarantined_field_idx: u32,
    /// m45: index of the synthetic `__restart_in_place_pending:
    /// i64` flag. 0 = next re-run is a regular `restart` (state
    /// preserved); 1 = next re-run is `restart_in_place` (user
    /// fields zeroed back to declared defaults before birth).
    /// Set by `restart_in_place(c)`; cleared by the rerun
    /// branch in `lower_closure_check` after the re-init pass
    /// runs. Both restart variants share the cap-2 budget on
    /// `__restart_count` — the kind flag only changes whether
    /// the re-run preserves state.
    restart_in_place_pending_field_idx: u32,
    /// m42: index of the synthetic `__parent_self: ptr` field.
    /// Set at instantiation time to the resolved parent
    /// self_ptr (from `resolve_failure_route`), or null if no
    /// parent has a matching on_failure handler. Read by
    /// `__tick_closures_wrapper` so tick-epoch fires can
    /// route violations to the parent without a static call
    /// site (bus drains don't have one).
    parent_self_field_idx: u32,
    /// m42: index of the synthetic `__parent_on_failure: ptr`
    /// field. Paired with `parent_self_field_idx`. Holds the
    /// parent's `on_failure` fn ptr (or null). Read by the
    /// tick wrapper to decide whether to absorb-or-stderr.
    parent_on_failure_field_idx: u32,
    /// m43: indices of the synthetic
    /// `__duration_last_fire_<i>: i64` fields, parallel to the
    /// locus's declared duration-epoch closures (in declaration
    /// order). Empty when the locus has no duration closures.
    /// Each field holds monotonic-ns of the closure's last
    /// fire; instantiation seeds it with time::monotonic() so
    /// the first fire happens after `N` elapses (not
    /// immediately). The synthesized `__duration_closures`
    /// fn loads each, compares now - last >= N, and fires
    /// when so.
    duration_last_fire_field_idxs: Vec<u32>,
    /// m43: synthetic `<Locus>.__duration_closures(self,
    /// parent_self_or_null, on_failure_or_null)` fn. None when
    /// the locus has no duration closures. Called at the same
    /// sites tick fires (after each bus handler, after run()
    /// returns) — duration shares the cell-boundary cadence
    /// but gates each closure on elapsed-since-last-fire.
    duration_closures_fn: Option<FunctionValue<'ctx>>,
    /// m43-followup: 1-arg adapter mirroring `tick_wrapper_fn`
    /// for duration. `<Locus>.__duration_closures_wrapper(self_ptr)`
    /// loads the struct's `__parent_self` + `__parent_on_failure`
    /// fields and tail-calls the 3-arg `__duration_closures`.
    /// Used from the pinned thread's post-`run()` path, where
    /// `resolve_failure_route` can't see the right `current_self`
    /// (we're off the main thread). Always paired with
    /// `duration_closures_fn`. Closes the documented v0 limit
    /// where pinned post-run() didn't fire duration.
    duration_wrapper_fn: Option<FunctionValue<'ctx>>,
    /// m44: synthetic `<Locus>.__explicit_closures(self,
    /// parent_self_or_null, on_failure_or_null)` fn. Called
    /// only by the `check_closures();` builtin from inside a
    /// locus method body — fires every explicit-epoch
    /// closure on the current self. Where birth/tick/duration
    /// fire automatically at substrate-cell boundaries, the
    /// explicit epoch is user-triggered: useful for "audit at
    /// this checkpoint" patterns where the locus author knows
    /// when an invariant should hold.
    explicit_closures_fn: Option<FunctionValue<'ctx>>,
    /// Index of the synthetic `__mailbox: ptr` field carrying this
    /// locus's `lotus_mailbox_t*`. Only set for pinned-class loci
    /// that declare `bus subscribe` — those need a per-locus
    /// mailbox so cross-thread publishers can post cells without
    /// touching the pinned thread's arena directly. None for every
    /// other locus (cooperative loci route through the global
    /// queue; pinned loci without subscriptions don't need a
    /// mailbox at all). m28b stage 2.
    mailbox_field_idx: Option<u32>,
    /// Per-spec projection class. Resolved at declare-locus-struct
    /// time from the `LocusAnnotation::Projection` annotation, or
    /// (per spec/memory.md) defaults to chunked if the locus
    /// declares accept, rich otherwise. Used at instantiation
    /// time to pick the parent's sub-region strategy: a
    /// chunked-class parent's accepted children call
    /// `lotus_arena_create_subregion(parent_arena)` instead of
    /// `lotus_arena_create()` (m22).
    projection_class: ProjectionClass,
    /// Per-locus execution strategy (m25). Resolved at declare-
    /// locus-struct time from the `LocusAnnotation::Schedule`
    /// annotation, or defaults to `Cooperative`. m25 only stores
    /// it — no runtime semantics yet (the runtime today is
    /// effectively greedy-everywhere via synchronous nested
    /// dispatch). m26 will branch on this to either deferred
    /// dispatch (cooperative) or sync (greedy); m27 spawns
    /// dedicated threads for pinned loci.
    #[allow(dead_code)]
    schedule_class: ScheduleClass,
    /// F.22 capacity-tuple slots declared on this locus. Order is
    /// declaration order: init runs in this order at instantiation
    /// (after slot 0 / arena is set); destroy runs in reverse at
    /// dissolve (before slot 0 / arena destroy). Each entry's
    /// `struct_field_idx` points at the `__slot_<name>: ptr` field
    /// appended to the locus struct layout.
    capacity_slots: Vec<CapacitySlotLayout>,
}

/// F.22 slot record carried on every LocusInfo. v1 surface:
/// records name, kind, cell type, and the struct slot where the
/// allocator pointer lives. Task #17 will widen this to participate
/// in `self.X.acquire()` / `self.X.alloc()` method dispatch.
#[derive(Debug, Clone)]
struct CapacitySlotLayout {
    name: String,
    kind: CapacitySlotKind,
    /// Cell element type (T from `pool/heap X of T`). Used at
    /// instantiation time to derive cell_size for the
    /// `lotus_*_create(size, align)` call via inkwell's
    /// `size_of()` on the LLVM type.
    elem_ty: CodegenTy,
    /// Index of the `__slot_<name>: ptr` field in the locus
    /// struct's LLVM body. Used at instantiation (store the
    /// create()-returned ptr here) and at dissolve (load the
    /// ptr and call destroy()).
    struct_field_idx: u32,
}

/// Maximum number of children any locus struct's built-in
/// `children` array can hold. v0 codegen uses a fixed cap to
/// avoid resize / heap dance; production-grade loci typically have
/// O(few-dozen) coordinatees, and 04-modes' AggregatorL only
/// instantiates 3.
const CHILDREN_CAP: u32 = 16;

/// One locus param's default-initializer. Either pre-resolved
/// (the common case — scalar literal) or deferred to the
/// instantiation site (composite literal like
/// `Kernel { ... }` whose evaluation needs the codegen
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
    fields: BTreeMap<String, (u32, CodegenTy)>,
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
    fields: BTreeMap<String, (u32, CodegenTy)>,
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
        // m22: chunked-class parent calls this when accepting a
        // child; the child arena registers a slot index in the
        // parent so destroy can free-list it for reuse.
        let subregion_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_arena_create_subregion", subregion_ty, None);
        let arena_alloc_ty =
            ptr_t.fn_type(&[ptr_t.into(), i64_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_arena_alloc", arena_alloc_ty, None);
        let arena_destroy_ty = void_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_arena_destroy", arena_destroy_ty, None);

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
        // ABI — codegen passes cell_size and cell_align as i64
        // params at create time, computed from T's struct layout.
        let pool_create_ty =
            ptr_t.fn_type(&[i64_t.into(), i64_t.into()], false);
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
            ptr_t.fn_type(&[i64_t.into(), i64_t.into()], false);
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
        let i32_t_local = self.context.i32_type();
        let str_eq_ty =
            i32_t_local.fn_type(&[ptr_t.into(), ptr_t.into()], false);
        self.module.add_function("lotus_str_eq", str_eq_ty, None);
        let str_len_ty = i64_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function("lotus_str_len", str_len_ty, None);
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
        self.module
            .add_function("lotus_bytes_len", bytes_len_ty, None);
        let bytes_data_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_bytes_data", bytes_data_ty, None);

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

        // m90: directory enumeration (newline-separated entries
        // as a String). Lifetime via the global payload arena.
        // declare ptr @lotus_fs_list_dir_global(ptr path)
        let fs_list_dir_global_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module.add_function(
            "lotus_fs_list_dir_global",
            fs_list_dir_global_ty,
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
        self.module.add_function(
            "lotus_bus_payload_arena_alloc",
            bus_payload_alloc_ty,
            None,
        );

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

        // m73b: TCP primitives reachable from Aperio source via
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
        // declare ptr @lotus_text_base64_encode(ptr bytes)
        let b64_encode_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_text_base64_encode", b64_encode_ty, None);
        // v1.x-16: declare ptr @lotus_text_base64_decode(ptr s)
        let b64_decode_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_text_base64_decode", b64_decode_ty, None);
        // ws-echo: cheap RNG (xorshift64*) for nonces / jitter.
        let void_t = self.context.void_type();
        let rand_seed_ty = void_t.fn_type(&[], false);
        self.module
            .add_function("lotus_rand_seed_from_time", rand_seed_ty, None);
        let rand_next_ty = i64_t.fn_type(&[i64_t.into()], false);
        self.module
            .add_function("lotus_rand_next_int", rand_next_ty, None);

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

        // Phase 2f: errno-style status for read_file. Returns 0
        // on success or the platform errno on failure.
        // declare i32 @lotus_fs_read_file_status(ptr path)
        let read_file_status_ty = i32_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function(
                "lotus_fs_read_file_status",
                read_file_status_ty,
                None,
            );

        // m75: filesystem primitives reachable from Aperio source
        // via the `std::io::fs::*` magic-path calls. The C-level
        // surface is in lotus_arena.c (m74 ship); these
        // declarations let codegen emit calls into them.

        // declare i64 @lotus_fs_read_file(ptr path, ptr out_buf, i64 out_cap)
        // returns bytes read (>=0) or -1.
        let fs_read_ty =
            i64_t.fn_type(&[ptr_t.into(), ptr_t.into(), i64_t.into()], false);
        self.module
            .add_function("lotus_fs_read_file", fs_read_ty, None);

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

        // v1.x: ASCII case folding primitives.
        // declare ptr @lotus_str_lower(ptr s)
        let case_fold_ty = ptr_t.fn_type(&[ptr_t.into()], false);
        self.module
            .add_function("lotus_str_lower", case_fold_ty, None);
        // declare ptr @lotus_str_upper(ptr s)
        self.module
            .add_function("lotus_str_upper", case_fold_ty, None);
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
        // Handle is a `ptr` (carried as Bytes in the Aperio surface).
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

        // m96: tree-sitter substrate. extern "C" symbols defined
        // in `runtime/lotus_treesitter.rs` (compiled into the
        // sibling `aperio-ts-shim` staticlib). The link step
        // adds `libaperio_ts_shim.a` so these references resolve.
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
        // notes/aperio-friction.md 2026-05-10 float-surface-gaps
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
    }

    /// Mark the program as containing at least one `bus subscribe`
    /// declaration. m45-followup migrated bus storage out of LLVM
    /// (a fixed-cap `[N x { ptr, ptr, ptr, ptr }]` global plus a
    /// linear-scan dispatch fn) into the C runtime
    /// (`lotus_bus_register` / `_dispatch` / `_quarantine_self`),
    /// so the only remaining LLVM-side state is this presence
    /// marker. Without it, a stray `<-` in a program with no
    /// subscribers would silently call the C-runtime dispatch on
    /// an empty table; with it, `lower_send` errors out at compile
    /// time, preserving the prior diagnostic.
    fn init_bus_state(&mut self) -> Result<(), CodegenError> {
        self.bus_state = Some(BusState);
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
    /// Emit a call to drain the cooperative-scheduler bus queue.
    /// Pops every enqueued (handler, self, payload) cell and
    /// invokes the handler. Handlers may enqueue more cells —
    /// the drain loop in the C runtime continues until the
    /// queue is empty at pop time. Called at the start of every
    /// `flush_dissolve_frame` so cooperative subscribers process
    /// pending cells BEFORE they themselves dissolve.
    fn emit_bus_drain(&mut self) -> Result<(), CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let queue_global = self
            .module
            .get_global("lotus.bus_queue.global")
            .expect("bus queue global declared");
        let queue_ptr = self
            .builder
            .build_load(
                ptr_t,
                queue_global.as_pointer_value(),
                "queue.cur",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let drain_fn = self
            .module
            .get_function("lotus_bus_queue_drain")
            .expect("lotus_bus_queue_drain declared");
        self.builder
            .build_call(
                drain_fn,
                &[queue_ptr.into()],
                "bus.queue.drain",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// Emit a call to destroy the bus queue. Used at every
    /// main-exit point so the queue tears down cleanly. m52:
    /// `flush_dissolve_frame_kind` now drains the queue after
    /// each dissolve in the loop, so cells enqueued by a
    /// dissolve method get dispatched to still-alive
    /// subscribers before those subscribers themselves
    /// dissolve. By the time we reach this destroy call, the
    /// queue is empty in well-formed programs (any residual
    /// cells would target dissolved subscribers, which the
    /// deregister-on-dissolve invariant prevents from being
    /// enqueued in the first place).
    /// m45-followup: also tears down the C-runtime bus router's
    /// entries vec so the heap allocation is freed alongside the
    /// queue's. Bus state lives entirely in the C runtime now.
    fn emit_bus_queue_destroy(&mut self) -> Result<(), CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let queue_global = self
            .module
            .get_global("lotus.bus_queue.global")
            .expect("bus queue global declared");
        let queue_ptr = self
            .builder
            .build_load(
                ptr_t,
                queue_global.as_pointer_value(),
                "queue.destroy.cur",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let destroy_fn = self
            .module
            .get_function("lotus_bus_queue_destroy")
            .expect("lotus_bus_queue_destroy declared");
        self.builder
            .build_call(
                destroy_fn,
                &[queue_ptr.into()],
                "bus.queue.destroy.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let router_destroy_fn = self
            .module
            .get_function("lotus_bus_router_destroy")
            .expect("lotus_bus_router_destroy declared");
        self.builder
            .build_call(
                router_destroy_fn,
                &[],
                "bus.router.destroy.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    fn flush_dissolve_frame(&mut self) -> Result<(), CodegenError> {
        self.flush_dissolve_frame_kind(true)
    }

    /// m46-vocab follow-up: variant of `flush_dissolve_frame` that
    /// flushes the deferred-dissolve frame WITHOUT draining the
    /// cooperative queue. Used by on_failure bodies (and any
    /// other context that runs synchronously inside a substrate
    /// cell) — the outer cell's flush owns the drain. A drain
    /// here would recursively pull queued cells into the current
    /// tick's call stack, breaking ordering: every closure check
    /// after this body returns would observe the post-recursion
    /// accumulator state instead of the at-this-fire state.
    /// Same principle as the Tick/Explicit closure-eval bodies
    /// (which already pop the frame manually and skip the drain).
    fn flush_dissolve_frame_kind(
        &mut self,
        drain_queue: bool,
    ) -> Result<(), CodegenError> {
        // m26: drain the bus queue BEFORE dissolves fire, so
        // every cooperative subscriber gets to process pending
        // cells while it's still alive. Handlers may publish
        // more events; the C-side drain loop keeps popping
        // until the queue is empty at pop time. Anything
        // enqueued during the dissolves below is leaked (v0
        // limitation; realistic programs don't publish
        // during dissolve).
        if drain_queue {
            self.emit_bus_drain()?;
        }
        let frame = self
            .deferred_dissolves
            .pop()
            .expect("flush without matching push");
        for (self_ptr, locus_name, thread_id_alloca) in frame.into_iter().rev() {
            let info = self
                .user_loci
                .get(&locus_name)
                .cloned()
                .expect("deferred locus declared");
            // Skip entries whose `let X = SomeLocus { };` never
            // executed on this control-flow path. The
            // lower_locus_instantiation hoist (entry-block alloca
            // with NULL-init arena field) gives us a reliable
            // sentinel: if the arena field reads NULL here, the
            // let-statement was bypassed (e.g., by an earlier
            // `return`), and there is nothing to dissolve. Skip
            // the entire entry — drain, dissolve, and
            // arena_destroy all would dereference uninitialized
            // state otherwise.
            //
            // For pinned entries the same sentinel applies: the
            // pthread was never created, so pthread_join would
            // block forever on a garbage TID.
            let func = self
                .current_fn
                .expect("flush called outside a fn body");
            let ptr_t = self.context.ptr_type(AddressSpace::default());
            let arena_check_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    info.arena_field_idx,
                    &format!("{}.dissolve.arena.gep", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let arena_check = self
                .builder
                .build_load(ptr_t, arena_check_ptr, &format!("{}.dissolve.arena", locus_name))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_pointer_value();
            let is_null = self
                .builder
                .build_is_null(
                    arena_check,
                    &format!("{}.dissolve.skip", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let skip_bb = self
                .context
                .append_basic_block(func, &format!("{}.dissolve.skip", locus_name));
            let process_bb = self
                .context
                .append_basic_block(func, &format!("{}.dissolve.process", locus_name));
            let after_bb = self
                .context
                .append_basic_block(func, &format!("{}.dissolve.after", locus_name));
            self.builder
                .build_conditional_branch(is_null, skip_bb, process_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder.position_at_end(skip_bb);
            self.builder
                .build_unconditional_branch(after_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder.position_at_end(process_bb);
            // m28a + m28b: pinned loci — pthread_join blocks until
            // the pinned thread's full lifecycle (birth → run →
            // mailbox loop (if subscriptions) → drain → dissolve)
            // has finished. The main thread's only remaining work
            // for a pinned entry is signaling shutdown to the
            // mailbox (so the pinned thread breaks out of its
            // mailbox loop), the join, and the arena_destroy;
            // drain / closures / dissolve are SKIPPED on the main
            // side because they already ran on the pinned thread.
            let is_pinned_entry = thread_id_alloca.is_some();
            if let Some(tid_slot) = thread_id_alloca {
                let i64_t = self.context.i64_type();
                let ptr_t = self.context.ptr_type(AddressSpace::default());
                // m28b: signal mailbox shutdown if the pinned
                // locus has one. The shutdown call wakes any
                // thread blocked in lotus_mailbox_drain_one's
                // condvar wait, with shutdown=1, so it returns 0
                // and the pinned thread proceeds to drain/dissolve.
                if let Some(mb_idx) = info.mailbox_field_idx {
                    let mb_slot = self
                        .builder
                        .build_struct_gep(
                            info.struct_ty,
                            self_ptr,
                            mb_idx,
                            &format!("{}.__mailbox.flush", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let mb = self
                        .builder
                        .build_load(ptr_t, mb_slot, "mailbox.shutdown.load")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_pointer_value();
                    let shutdown_fn = self
                        .module
                        .get_function("lotus_mailbox_shutdown")
                        .expect("lotus_mailbox_shutdown declared");
                    self.builder
                        .build_call(
                            shutdown_fn,
                            &[mb.into()],
                            &format!("{}.mailbox.shutdown", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                let tid = self
                    .builder
                    .build_load(i64_t, tid_slot, "pinned.tid")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let null_retval = ptr_t.const_null();
                let join_fn = self
                    .module
                    .get_function("pthread_join")
                    .expect("pthread_join declared");
                self.builder
                    .build_call(
                        join_fn,
                        &[tid.into(), null_retval.into()],
                        &format!("{}.pthread_join", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                // After join, destroy the mailbox.
                if let Some(mb_idx) = info.mailbox_field_idx {
                    let mb_slot = self
                        .builder
                        .build_struct_gep(
                            info.struct_ty,
                            self_ptr,
                            mb_idx,
                            &format!("{}.__mailbox.destroy", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let mb = self
                        .builder
                        .build_load(ptr_t, mb_slot, "mailbox.destroy.load")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_pointer_value();
                    let destroy_fn = self
                        .module
                        .get_function("lotus_mailbox_destroy")
                        .expect("lotus_mailbox_destroy declared");
                    self.builder
                        .build_call(
                            destroy_fn,
                            &[mb.into()],
                            &format!("{}.mailbox.destroy.call", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
            }
            if !is_pinned_entry {
                // Cooperative long-lived: drain → __closures →
                // dissolve, mirroring the ephemeral-locus ordering.
                // The cascade itself ran each descendant's closures
                // during the descendant's own ephemeral-dissolve /
                // scope-exit.
                if let Some(drain_fn) = info.methods.get("drain") {
                    self.builder
                        .build_call(
                            *drain_fn,
                            &[self_ptr.into()],
                            &format!("{}.drain.call", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                if let Some(closures_fn) = info.dissolve_closures_fn {
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
                            &format!(
                                "{}.__dissolve_closures.call",
                                locus_name
                            ),
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
            }
            // Arena released at scope exit, after the pinned thread
            // is joined or after the cooperative drain/dissolve has
            // run. Symmetric with the ephemeral path in
            // lower_locus_instantiation.
            self.emit_locus_arena_destroy(&info, self_ptr, &locus_name)?;
            // m52: drain again after each dissolve. The dissolve
            // method may publish — those cells enqueue for
            // still-alive subscribers later in the reverse-iter
            // order. Without this in-loop drain they'd sit until
            // emit_bus_queue_destroy, by which point all
            // subscribers have dissolved and the cells are
            // leaked (use-after-free if dispatched). Drain here
            // dispatches them while their targets are still
            // alive; the deregister-on-dissolve invariant
            // (m45-followup-2) means cells never target the
            // just-dissolved locus. The drain loop in the
            // C-runtime keeps popping until the queue is empty
            // at pop time, so chain-reactions where a fired
            // handler publishes more get caught in the same
            // drain pass.
            if drain_queue {
                self.emit_bus_drain()?;
            }
            // Close the per-entry process_bb by branching to
            // after_bb so both skip_bb and process_bb converge
            // before the loop body advances to the next entry.
            self.builder
                .build_unconditional_branch(after_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder.position_at_end(after_bb);
        }
        Ok(())
    }

    /// m70: synthesize `__serialize_T` / `__deserialize_T` for a
    /// bus payload type with per-field wire encoding.
    ///
    /// Wire format (per F-design "compile-time agreement, no
    /// runtime negotiation"; versioning is open-question #13's
    /// `serialize_as TypeV1` future, NOT in m70):
    /// - Field order = declared order. No padding on wire.
    /// - Int / Float / Bool / Time / Duration: 8 bytes,
    ///   little-endian (matches in-memory layout on x86_64).
    /// - Decimal: 16 bytes (matches in-memory layout).
    /// - String: 8-byte i64 length-prefix LE, then N UTF-8
    ///   bytes (no NUL on wire). Decode allocates len+1 from
    ///   the lazy global payload arena and writes a NUL
    ///   terminator so existing C-side `strlen`/`strcpy` ops
    ///   still work on the deserialized struct.
    /// - Other field types (nested struct, enum, array): error
    ///   at synthesis time — defer to a future polish.
    ///
    /// Enum payload types: keep memcpy-shape (no per-field walk
    /// inside variants for v0.1). Enum-with-String fields
    /// errors at synthesis time. Non-String enums round-trip
    /// fine because their in-memory layout has no pointers.
    ///
    /// Saves and restores the builder position so this can run
    /// between passes that use the builder (notably between A2
    /// and the body-lowering passes C/D).
    fn synthesize_serializer(
        &mut self,
        type_name: &str,
    ) -> Result<(), CodegenError> {
        if self.serializers.contains_key(type_name) {
            return Ok(());     /* already synthesized */
        }
        let i64_t = self.context.i64_type();
        let i32_t = self.context.i32_type();
        let i8_t = self.context.i8_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());

        // Decide the synthesis strategy. Two shapes:
        //   - Struct payload (struct_layout = Some): per-field
        //     walk (m70 wire format).
        //   - Enum payload (struct_layout = None, enum_size =
        //     Some): memcpy of the enum storage struct, after
        //     verifying no variant carries a String (would
        //     corrupt cross-process).
        let struct_layout: Option<(
            StructType<'ctx>,
            Vec<String>,
            BTreeMap<String, (u32, CodegenTy)>,
        )>;
        let enum_size: Option<inkwell::values::IntValue<'ctx>>;
        if let Some(info) = self.user_types.get(type_name).cloned() {
            struct_layout =
                Some((info.struct_ty, info.field_order, info.fields));
            enum_size = None;
        } else if let Some(info) = self.user_enums.get(type_name).cloned() {
            if !info.has_payload {
                return Err(CodegenError::Unsupported(format!(
                    "bus payload `{}` is a no-payload enum; wrap in a \
                     struct or add a variant payload",
                    type_name
                )));
            }
            // m70: refuse enum-with-String for cross-process —
            // per-variant per-field serialization is post-v1.
            for v in &info.variants {
                for ft in &v.field_tys {
                    if matches!(ft, CodegenTy::String) {
                        return Err(CodegenError::Unsupported(format!(
                            "bus payload `{}` variant `{}` has a String \
                             field; cross-process String inside an enum \
                             variant is post-v1 (m70 supports String only \
                             at the top-level struct)",
                            type_name, v.name
                        )));
                    }
                }
            }
            let size = self
                .enum_storage_struct(&info)
                .size_of()
                .expect("enum storage struct has known size");
            struct_layout = None;
            enum_size = Some(size);
        } else {
            return Err(CodegenError::Unsupported(format!(
                "synthesize_serializer: type `{}` not declared",
                type_name
            )));
        }
        let _ = i32_t;
        let _ = i8_t;

        let saved_block = self.builder.get_insert_block();

        // i64 @__serialize_T(ptr src, ptr dst, i64 cap)
        let ser_ty = i64_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), i64_t.into()],
            false,
        );
        let ser_fn = self.module.add_function(
            &format!("__serialize_{}", type_name),
            ser_ty,
            None,
        );
        let ser_entry = self.context.append_basic_block(ser_fn, "entry");
        self.builder.position_at_end(ser_entry);
        let ser_src = ser_fn
            .get_nth_param(0)
            .expect("ser src arg")
            .into_pointer_value();
        let ser_dst = ser_fn
            .get_nth_param(1)
            .expect("ser dst arg")
            .into_pointer_value();
        let _ = ser_fn.get_nth_param(2); // cap, ignored at v0.1

        let total_written: inkwell::values::IntValue<'ctx> =
            if let Some((struct_ty, field_order, fields)) = &struct_layout
            {
                self.emit_per_field_serialize(
                    ser_src,
                    ser_dst,
                    *struct_ty,
                    field_order,
                    fields,
                )?
            } else {
                let size_iv = enum_size.expect("enum size present");
                self.emit_memcpy_call(
                    ser_dst,
                    ser_src,
                    size_iv,
                    "ser.memcpy",
                )?;
                size_iv
            };
        self.builder
            .build_return(Some(&total_written))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // i64 @__deserialize_T(ptr src, i64 n, ptr dst, i64 cap)
        let de_ty = i64_t.fn_type(
            &[ptr_t.into(), i64_t.into(), ptr_t.into(), i64_t.into()],
            false,
        );
        let de_fn = self.module.add_function(
            &format!("__deserialize_{}", type_name),
            de_ty,
            None,
        );
        let de_entry = self.context.append_basic_block(de_fn, "entry");
        self.builder.position_at_end(de_entry);
        let de_src = de_fn
            .get_nth_param(0)
            .expect("de src arg")
            .into_pointer_value();
        let _ = de_fn.get_nth_param(1); // n, ignored
        let de_dst = de_fn
            .get_nth_param(2)
            .expect("de dst arg")
            .into_pointer_value();
        let _ = de_fn.get_nth_param(3); // cap, ignored

        let de_struct_size: inkwell::values::IntValue<'ctx> =
            if let Some((struct_ty, field_order, fields)) = &struct_layout
            {
                self.emit_per_field_deserialize(
                    de_src,
                    de_dst,
                    *struct_ty,
                    field_order,
                    fields,
                )?
            } else {
                let size_iv = enum_size.expect("enum size present");
                self.emit_memcpy_call(
                    de_dst,
                    de_src,
                    size_iv,
                    "de.memcpy",
                )?;
                size_iv
            };
        self.builder
            .build_return(Some(&de_struct_size))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        if let Some(b) = saved_block {
            self.builder.position_at_end(b);
        }

        self.serializers.insert(
            type_name.to_string(),
            SerializerPair { serialize: ser_fn, deserialize: de_fn },
        );
        Ok(())
    }

    /// m70: emit a memcpy(dst, src, n) call without consuming
    /// the result. Centralizes the symbol lookup so callers
    /// don't repeat the `get_function("memcpy").expect(...)`
    /// boilerplate.
    fn emit_memcpy_call(
        &self,
        dst: PointerValue<'ctx>,
        src: PointerValue<'ctx>,
        n: inkwell::values::IntValue<'ctx>,
        name: &str,
    ) -> Result<(), CodegenError> {
        let memcpy_fn = self
            .module
            .get_function("memcpy")
            .expect("memcpy declared");
        self.builder
            .build_call(
                memcpy_fn,
                &[dst.into(), src.into(), n.into()],
                name,
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// m70: emit IR for the body of `__serialize_T` for a struct
    /// payload, walking fields in declared order. Returns the
    /// total bytes-written value (i64) for the fn's return.
    fn emit_per_field_serialize(
        &mut self,
        src: PointerValue<'ctx>,
        dst: PointerValue<'ctx>,
        struct_ty: StructType<'ctx>,
        field_order: &[String],
        fields: &BTreeMap<String, (u32, CodegenTy)>,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        let i64_t = self.context.i64_type();
        let i8_t = self.context.i8_type();
        let cursor_alloca = self
            .builder
            .build_alloca(i64_t, "ser.cursor")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(cursor_alloca, i64_t.const_int(0, false))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        for fname in field_order {
            let (idx, field_ty) = fields
                .get(fname)
                .cloned()
                .expect("field declared in field_order also present in fields");
            let src_field_ptr = self
                .builder
                .build_struct_gep(struct_ty, src, idx, "ser.field.ptr")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let cursor_iv = self
                .builder
                .build_load(i64_t, cursor_alloca, "ser.cursor.load")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_int_value();
            let dst_at_cursor = unsafe {
                self.builder
                    .build_gep(i8_t, dst, &[cursor_iv], "ser.dst.cursor")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            };

            match &field_ty {
                CodegenTy::String => {
                    // Wire: i64 LE length + N bytes (no NUL).
                    let str_ptr = self
                        .builder
                        .build_load(
                            self.context.ptr_type(AddressSpace::default()),
                            src_field_ptr,
                            "ser.str.ptr",
                        )
                        .map_err(|e| {
                            CodegenError::LlvmEmit(e.to_string())
                        })?
                        .into_pointer_value();
                    let str_len = self.emit_str_len_call(str_ptr)?;
                    // Write 8-byte length prefix.
                    let len_alloca = self
                        .builder
                        .build_alloca(i64_t, "ser.str.len")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(len_alloca, str_len)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.emit_memcpy_call(
                        dst_at_cursor,
                        len_alloca,
                        i64_t.const_int(8, false),
                        "ser.str.memcpy.len",
                    )?;
                    let after_len = self
                        .builder
                        .build_int_add(
                            cursor_iv,
                            i64_t.const_int(8, false),
                            "ser.cursor.after.len",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let dst_after_len = unsafe {
                        self.builder
                            .build_gep(
                                i8_t,
                                dst,
                                &[after_len],
                                "ser.dst.after.len",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    };
                    self.emit_memcpy_call(
                        dst_after_len,
                        str_ptr,
                        str_len,
                        "ser.str.memcpy.bytes",
                    )?;
                    let after_bytes = self
                        .builder
                        .build_int_add(
                            after_len,
                            str_len,
                            "ser.cursor.after.bytes",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(cursor_alloca, after_bytes)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                CodegenTy::Int
                | CodegenTy::Float
                | CodegenTy::Bool
                | CodegenTy::Time
                | CodegenTy::Duration
                | CodegenTy::Decimal => {
                    let nbytes = codegen_ty_size_bytes(self.context, &field_ty);
                    let nbytes_iv = i64_t.const_int(nbytes, false);
                    self.emit_memcpy_call(
                        dst_at_cursor,
                        src_field_ptr,
                        nbytes_iv,
                        "ser.fixed.memcpy",
                    )?;
                    let after = self
                        .builder
                        .build_int_add(
                            cursor_iv,
                            nbytes_iv,
                            "ser.cursor.after",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(cursor_alloca, after)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                CodegenTy::TypeRef(nested_name) => {
                    // Nested user-struct: recurse on its field
                    // layout. The slot at `src_field_ptr` holds a
                    // pointer to the nested storage (TypeRef
                    // values are heap-allocated structs); load
                    // it, then walk the nested fields starting at
                    // the current dst cursor.
                    let nested_info = self
                        .user_types
                        .get(nested_name.as_str())
                        .cloned()
                        .ok_or_else(|| {
                            CodegenError::Unsupported(format!(
                                "bus payload field `{}: {}` — nested \
                                 type not declared",
                                fname, nested_name
                            ))
                        })?;
                    let nested_src = self
                        .builder
                        .build_load(
                            self.context.ptr_type(AddressSpace::default()),
                            src_field_ptr,
                            "ser.nested.load",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_pointer_value();
                    let nested_written = self.emit_per_field_serialize(
                        nested_src,
                        dst_at_cursor,
                        nested_info.struct_ty,
                        &nested_info.field_order,
                        &nested_info.fields,
                    )?;
                    let after = self
                        .builder
                        .build_int_add(
                            cursor_iv,
                            nested_written,
                            "ser.cursor.after.nested",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(cursor_alloca, after)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                other => {
                    return Err(CodegenError::Unsupported(format!(
                        "bus payload field `{}: {:?}` — m70 wire format \
                         supports primitives, String, and nested structs \
                         (whose leaves are primitives/String); arrays / \
                         tuples / enums cross-process are post-v1 polish",
                        fname, other
                    )));
                }
            }
        }

        let total = self
            .builder
            .build_load(i64_t, cursor_alloca, "ser.cursor.final")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        Ok(total)
    }

    /// m70: emit IR for the body of `__deserialize_T` for a
    /// struct payload, walking fields in declared order.
    /// Returns the in-memory struct size (the dst contains a
    /// concrete struct, regardless of how much wire was
    /// consumed).
    fn emit_per_field_deserialize(
        &mut self,
        src: PointerValue<'ctx>,
        dst: PointerValue<'ctx>,
        struct_ty: StructType<'ctx>,
        field_order: &[String],
        fields: &BTreeMap<String, (u32, CodegenTy)>,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        let i64_t = self.context.i64_type();
        let i8_t = self.context.i8_type();
        let cursor_alloca = self
            .builder
            .build_alloca(i64_t, "de.cursor")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(cursor_alloca, i64_t.const_int(0, false))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        for fname in field_order {
            let (idx, field_ty) = fields
                .get(fname)
                .cloned()
                .expect("field declared in field_order also present in fields");
            let dst_field_ptr = self
                .builder
                .build_struct_gep(struct_ty, dst, idx, "de.field.ptr")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let cursor_iv = self
                .builder
                .build_load(i64_t, cursor_alloca, "de.cursor.load")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_int_value();
            let src_at_cursor = unsafe {
                self.builder
                    .build_gep(i8_t, src, &[cursor_iv], "de.src.cursor")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            };

            match &field_ty {
                CodegenTy::String => {
                    // Read 8-byte length prefix.
                    let len_alloca = self
                        .builder
                        .build_alloca(i64_t, "de.str.len.alloca")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.emit_memcpy_call(
                        len_alloca,
                        src_at_cursor,
                        i64_t.const_int(8, false),
                        "de.str.memcpy.len",
                    )?;
                    let str_len = self
                        .builder
                        .build_load(i64_t, len_alloca, "de.str.len")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_int_value();
                    let after_len = self
                        .builder
                        .build_int_add(
                            cursor_iv,
                            i64_t.const_int(8, false),
                            "de.cursor.after.len",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let src_after_len = unsafe {
                        self.builder
                            .build_gep(
                                i8_t,
                                src,
                                &[after_len],
                                "de.src.after.len",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    };
                    // Allocate len+1 from the lazy global payload
                    // arena. The +1 is for the C-side NUL
                    // terminator so existing strlen / strcpy /
                    // string-printing code works on the
                    // deserialized struct.
                    let alloc_size = self
                        .builder
                        .build_int_add(
                            str_len,
                            i64_t.const_int(1, false),
                            "de.str.alloc.size",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let alloc_fn = self
                        .module
                        .get_function("lotus_bus_payload_arena_alloc")
                        .expect("lotus_bus_payload_arena_alloc declared");
                    let buf = self
                        .builder
                        .build_call(
                            alloc_fn,
                            &[
                                alloc_size.into(),
                                i64_t.const_int(1, false).into(),
                            ],
                            "de.str.alloc",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .try_as_basic_value()
                        .left()
                        .expect("payload arena alloc returns ptr")
                        .into_pointer_value();
                    self.emit_memcpy_call(
                        buf,
                        src_after_len,
                        str_len,
                        "de.str.memcpy.bytes",
                    )?;
                    // Write trailing NUL: buf[str_len] = 0.
                    let nul_slot = unsafe {
                        self.builder
                            .build_gep(
                                i8_t,
                                buf,
                                &[str_len],
                                "de.str.nul.ptr",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    };
                    self.builder
                        .build_store(nul_slot, i8_t.const_int(0, false))
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    // Store buf pointer into dst struct's String
                    // field slot.
                    self.builder
                        .build_store(dst_field_ptr, buf)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let after_bytes = self
                        .builder
                        .build_int_add(
                            after_len,
                            str_len,
                            "de.cursor.after.bytes",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(cursor_alloca, after_bytes)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                CodegenTy::Int
                | CodegenTy::Float
                | CodegenTy::Bool
                | CodegenTy::Time
                | CodegenTy::Duration
                | CodegenTy::Decimal => {
                    let nbytes = codegen_ty_size_bytes(self.context, &field_ty);
                    let nbytes_iv = i64_t.const_int(nbytes, false);
                    self.emit_memcpy_call(
                        dst_field_ptr,
                        src_at_cursor,
                        nbytes_iv,
                        "de.fixed.memcpy",
                    )?;
                    let after = self
                        .builder
                        .build_int_add(
                            cursor_iv,
                            nbytes_iv,
                            "de.cursor.after",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(cursor_alloca, after)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                CodegenTy::TypeRef(nested_name) => {
                    // Nested user-struct: allocate a fresh nested
                    // storage in the payload arena, recurse to
                    // deserialize its fields, and store the new
                    // pointer into dst's slot.
                    let nested_info = self
                        .user_types
                        .get(nested_name.as_str())
                        .cloned()
                        .ok_or_else(|| {
                            CodegenError::Unsupported(format!(
                                "bus payload field `{}: {}` — nested \
                                 type not declared",
                                fname, nested_name
                            ))
                        })?;
                    let nested_size = nested_info
                        .struct_ty
                        .size_of()
                        .expect("nested struct ty has known size");
                    let alloc_fn = self
                        .module
                        .get_function("lotus_bus_payload_arena_alloc")
                        .expect("lotus_bus_payload_arena_alloc declared");
                    let nested_dst = self
                        .builder
                        .build_call(
                            alloc_fn,
                            &[
                                nested_size.into(),
                                i64_t.const_int(8, false).into(),
                            ],
                            "de.nested.alloc",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .try_as_basic_value()
                        .left()
                        .expect("payload arena alloc returns ptr")
                        .into_pointer_value();
                    let nested_consumed = self.emit_per_field_deserialize_size(
                        src_at_cursor,
                        nested_dst,
                        nested_info.struct_ty,
                        &nested_info.field_order,
                        &nested_info.fields,
                    )?;
                    self.builder
                        .build_store(dst_field_ptr, nested_dst)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let after = self
                        .builder
                        .build_int_add(
                            cursor_iv,
                            nested_consumed,
                            "de.cursor.after.nested",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(cursor_alloca, after)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                other => {
                    return Err(CodegenError::Unsupported(format!(
                        "bus payload field `{}: {:?}` — m70 wire format \
                         supports primitives, String, and nested structs \
                         (whose leaves are primitives/String)",
                        fname, other
                    )));
                }
            }
        }

        let struct_size = struct_ty
            .size_of()
            .expect("payload struct has known size");
        Ok(struct_size)
    }

    /// Variant of `emit_per_field_deserialize` that returns the
    /// number of *wire bytes* consumed rather than the in-memory
    /// struct size. Needed by the nested-struct recursion in
    /// `emit_per_field_deserialize` so the caller can advance its
    /// wire cursor by the consumed amount. Same body, different
    /// return.
    fn emit_per_field_deserialize_size(
        &mut self,
        src: PointerValue<'ctx>,
        dst: PointerValue<'ctx>,
        struct_ty: StructType<'ctx>,
        field_order: &[String],
        fields: &BTreeMap<String, (u32, CodegenTy)>,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        let i64_t = self.context.i64_type();
        let i8_t = self.context.i8_type();
        let cursor_alloca = self
            .builder
            .build_alloca(i64_t, "de.nested.cursor")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(cursor_alloca, i64_t.const_int(0, false))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        for fname in field_order {
            let (idx, field_ty) = fields
                .get(fname)
                .cloned()
                .expect("field declared in field_order also present in fields");
            let dst_field_ptr = self
                .builder
                .build_struct_gep(struct_ty, dst, idx, "de.nested.field.ptr")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let cursor_iv = self
                .builder
                .build_load(i64_t, cursor_alloca, "de.nested.cursor.load")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_int_value();
            let src_at_cursor = unsafe {
                self.builder
                    .build_gep(i8_t, src, &[cursor_iv], "de.nested.src.cursor")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            };

            match &field_ty {
                CodegenTy::String => {
                    let len_alloca = self
                        .builder
                        .build_alloca(i64_t, "de.nested.str.len.alloca")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.emit_memcpy_call(
                        len_alloca,
                        src_at_cursor,
                        i64_t.const_int(8, false),
                        "de.nested.str.memcpy.len",
                    )?;
                    let str_len = self
                        .builder
                        .build_load(i64_t, len_alloca, "de.nested.str.len")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_int_value();
                    let after_len = self
                        .builder
                        .build_int_add(
                            cursor_iv,
                            i64_t.const_int(8, false),
                            "de.nested.cursor.after.len",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let src_after_len = unsafe {
                        self.builder
                            .build_gep(
                                i8_t,
                                src,
                                &[after_len],
                                "de.nested.src.after.len",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    };
                    let alloc_size = self
                        .builder
                        .build_int_add(
                            str_len,
                            i64_t.const_int(1, false),
                            "de.nested.str.alloc.size",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let alloc_fn = self
                        .module
                        .get_function("lotus_bus_payload_arena_alloc")
                        .expect("lotus_bus_payload_arena_alloc declared");
                    let buf = self
                        .builder
                        .build_call(
                            alloc_fn,
                            &[
                                alloc_size.into(),
                                i64_t.const_int(1, false).into(),
                            ],
                            "de.nested.str.alloc",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .try_as_basic_value()
                        .left()
                        .expect("payload arena alloc returns ptr")
                        .into_pointer_value();
                    self.emit_memcpy_call(
                        buf,
                        src_after_len,
                        str_len,
                        "de.nested.str.memcpy.bytes",
                    )?;
                    let nul_slot = unsafe {
                        self.builder
                            .build_gep(
                                i8_t,
                                buf,
                                &[str_len],
                                "de.nested.str.nul.ptr",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    };
                    self.builder
                        .build_store(nul_slot, i8_t.const_int(0, false))
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(dst_field_ptr, buf)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let after_bytes = self
                        .builder
                        .build_int_add(
                            after_len,
                            str_len,
                            "de.nested.cursor.after.bytes",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(cursor_alloca, after_bytes)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                CodegenTy::Int
                | CodegenTy::Float
                | CodegenTy::Bool
                | CodegenTy::Time
                | CodegenTy::Duration
                | CodegenTy::Decimal => {
                    let nbytes = codegen_ty_size_bytes(self.context, &field_ty);
                    let nbytes_iv = i64_t.const_int(nbytes, false);
                    self.emit_memcpy_call(
                        dst_field_ptr,
                        src_at_cursor,
                        nbytes_iv,
                        "de.nested.fixed.memcpy",
                    )?;
                    let after = self
                        .builder
                        .build_int_add(
                            cursor_iv,
                            nbytes_iv,
                            "de.nested.cursor.after",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(cursor_alloca, after)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                CodegenTy::TypeRef(nested_name) => {
                    let nested_info = self
                        .user_types
                        .get(nested_name.as_str())
                        .cloned()
                        .ok_or_else(|| {
                            CodegenError::Unsupported(format!(
                                "bus payload field `{}: {}` — nested \
                                 type not declared",
                                fname, nested_name
                            ))
                        })?;
                    let nested_size = nested_info
                        .struct_ty
                        .size_of()
                        .expect("nested struct ty has known size");
                    let alloc_fn = self
                        .module
                        .get_function("lotus_bus_payload_arena_alloc")
                        .expect("lotus_bus_payload_arena_alloc declared");
                    let nested_dst = self
                        .builder
                        .build_call(
                            alloc_fn,
                            &[
                                nested_size.into(),
                                i64_t.const_int(8, false).into(),
                            ],
                            "de.nested.deep.alloc",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .try_as_basic_value()
                        .left()
                        .expect("payload arena alloc returns ptr")
                        .into_pointer_value();
                    let nested_consumed = self.emit_per_field_deserialize_size(
                        src_at_cursor,
                        nested_dst,
                        nested_info.struct_ty,
                        &nested_info.field_order,
                        &nested_info.fields,
                    )?;
                    self.builder
                        .build_store(dst_field_ptr, nested_dst)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let after = self
                        .builder
                        .build_int_add(
                            cursor_iv,
                            nested_consumed,
                            "de.nested.cursor.after.nested",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(cursor_alloca, after)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                other => {
                    return Err(CodegenError::Unsupported(format!(
                        "bus payload field `{}: {:?}` — m70 wire format \
                         supports primitives, String, and nested structs",
                        fname, other
                    )));
                }
            }
        }

        let total = self
            .builder
            .build_load(i64_t, cursor_alloca, "de.nested.cursor.final")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        Ok(total)
    }

    /// m70: emit a call to `lotus_str_len(s)` and return the
    /// resulting i64. Centralizes the symbol lookup.
    fn emit_str_len_call(
        &self,
        s: PointerValue<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        let str_len_fn = self
            .module
            .get_function("lotus_str_len")
            .expect("lotus_str_len declared");
        let n = self
            .builder
            .build_call(str_len_fn, &[s.into()], "ser.str.len.call")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_str_len returns i64")
            .into_int_value();
        Ok(n)
    }

    /// Emit a single subscription registration as one call to
    /// `lotus_bus_register(subject, self, handler, mailbox,
    /// deserialize_fn)`. The C runtime owns the entries vec and
    /// grows it on demand, so there's no compile-time-fixed
    /// capacity ceiling. `mailbox_or_null` is `Some(mb_ptr)` for
    /// pinned subscribers (cells route to that locus's mailbox)
    /// and `None` for cooperative subscribers (cells route to
    /// the global queue). m60: `payload_type` names the type
    /// declared in the matching `bus subscribe "..." of type T`
    /// — used to look up `__deserialize_T` so the reader thread
    /// (m59) can decode wire-format bytes into a struct before
    /// dispatching to the handler.
    fn emit_bus_register(
        &mut self,
        subject: &str,
        self_ptr: PointerValue<'ctx>,
        handler_fn: FunctionValue<'ctx>,
        mailbox_or_null: Option<PointerValue<'ctx>>,
        payload_type: &str,
    ) -> Result<(), CodegenError> {
        let _ = self
            .bus_state
            .expect("subscriptions registered ⇒ bus_state initialized");
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let subj_str = self.global_string(subject);
        let handler_ptr = handler_fn.as_global_value().as_pointer_value();
        let mailbox_val = mailbox_or_null.unwrap_or_else(|| ptr_t.const_null());
        let deserialize_ptr = self
            .serializers
            .get(payload_type)
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "no serializer synthesized for bus payload type `{}` — \
                     m60 should have created one in pass A3",
                    payload_type
                ))
            })?
            .deserialize
            .as_global_value()
            .as_pointer_value();
        let register_fn = self
            .module
            .get_function("lotus_bus_register")
            .expect("lotus_bus_register declared in declare_builtins");
        self.builder
            .build_call(
                register_fn,
                &[
                    subj_str.into(),
                    self_ptr.into(),
                    handler_ptr.into(),
                    mailbox_val.into(),
                    deserialize_ptr.into(),
                ],
                "bus.register.call",
            )
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

        // F.20: register interface declarations by name. The
        // codegen layer uses this in two places: signature lowering
        // (`Interface(name)` in CodegenTy) and Phase-B vtable
        // synthesis (lazy lookup of method order from the AST).
        for item in &self.program.items {
            if let TopDecl::Interface(i) = item {
                self.user_interfaces.insert(i.name.name.clone());
            }
        }

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

        // m61 / m61b: discover generic instantiations referenced
        // anywhere in the program, synthesize a concrete
        // (mangled-name) decl per unique (template, args), and
        // declare those FIRST so non-generic decls that reference
        // generic types resolve cleanly when their bodies are
        // walked. m61b broadens the walk from struct fields only
        // to: fn signatures, locus signatures (params, lifecycle,
        // mode, fn, bus payloads), let ascriptions in fn bodies.
        // Recurses into nested args so `Box<Pair<Int,String>>`
        // would synthesize both `Pair_Int_String` and
        // `Box_Pair_Int_String` (parser `>>` ambiguity is a
        // separate gap; the codegen substrate is ready).
        let mut generic_type_decls: BTreeMap<String, TypeDecl> = type_decls
            .iter()
            .filter(|t| !t.generics.is_empty())
            .map(|t| (t.name.name.clone(), t.clone()))
            .collect();
        // m65: inject built-in stdlib generics (Result, Option) so
        // programs can reference them without an explicit `type`
        // declaration. User-written `type Result<...>` /
        // `type Option<...>` decls take precedence (existing entry
        // kept) — this keeps the door open for stdlib customization
        // in a future tooling milestone but doesn't fight the user
        // today.
        for builtin in Self::builtin_generic_type_decls() {
            generic_type_decls
                .entry(builtin.name.name.clone())
                .or_insert(builtin);
        }
        // m63: collect generic locus templates from the program.
        // Loci with non-empty `generics` get registered here so
        // discovery can route their TypeExpr uses through the
        // same walker as generic types.
        let raw_locus_decls: Vec<LocusDecl> = self
            .program
            .items
            .iter()
            .filter_map(|item| match item {
                TopDecl::Locus(l) => Some(l.clone()),
                _ => None,
            })
            .collect();
        let generic_locus_decls: BTreeMap<String, LocusDecl> = raw_locus_decls
            .iter()
            .filter(|l| !l.generics.is_empty())
            .map(|l| (l.name.name.clone(), l.clone()))
            .collect();
        let generic_names: BTreeSet<String> = generic_type_decls
            .keys()
            .chain(generic_locus_decls.keys())
            .cloned()
            .collect();
        let mut seen_mangles: BTreeSet<String> = BTreeSet::new();
        let mut requests: Vec<(String, Vec<TypeExpr>)> = Vec::new();
        Self::collect_generic_uses_in_program(
            self.program,
            &generic_names,
            &mut seen_mangles,
            &mut requests,
        )?;
        // m63: process requests as a queue — synthesizing one
        // instantiation may surface NEW generic uses inside its
        // substituted body (e.g., `Holder<Int>` instantiates
        // a body containing `Box<Int>` which itself needs
        // synthesizing). The queue closes when discovery on the
        // most recent synthesis adds nothing new.
        // m67: split the generic-instantiation pass into two
        // phases so a synthesized type can reference another
        // synthesized type regardless of queue order. Phase 1
        // synthesizes everything (no declarations) while
        // continuing to discover nested generic uses; phase 2
        // declares synthesized types in dependency order via a
        // retry loop that defers any decl whose field-type
        // resolution would fail because a dep isn't declared
        // yet. The pre-m67 BFS-then-declare path worked for
        // linear chains (Outer → Pair_Int → Box_Int) only when
        // declared in reverse queue order; for fan-in cases
        // where two outer types share a nested generic, the
        // queue order doesn't yield a valid topological sort.
        let mut synthesized_types: Vec<TypeDecl> = Vec::new();
        let mut synthesized_loci: Vec<LocusDecl> = Vec::new();
        let mut next_idx = 0usize;
        while next_idx < requests.len() {
            let (template_name, args) = requests[next_idx].clone();
            next_idx += 1;
            if let Some(template) = generic_type_decls.get(&template_name) {
                let synthesized =
                    Self::synthesize_generic_instantiation(
                        template, &args,
                    )?;
                // Walk the synthesized decl's body for nested
                // generic uses; deps go in the queue and get
                // synthesized in subsequent loop iterations.
                if let TypeDeclBody::Struct(fields) = &synthesized.body {
                    for f in fields {
                        Self::collect_generic_uses(
                            &f.ty,
                            &generic_names,
                            &mut seen_mangles,
                            &mut requests,
                        )?;
                    }
                } else if let TypeDeclBody::Enum(variants) =
                    &synthesized.body
                {
                    for v in variants {
                        for f in &v.fields {
                            Self::collect_generic_uses(
                                f,
                                &generic_names,
                                &mut seen_mangles,
                                &mut requests,
                            )?;
                        }
                    }
                }
                synthesized_types.push(synthesized);
            } else if let Some(template) =
                generic_locus_decls.get(&template_name)
            {
                let mangled =
                    Self::mangle_generic_name(&template_name, &args)?;
                let synthesized =
                    Self::synthesize_generic_locus_instantiation(
                        template, &args, &mangled,
                    )?;
                // Walk synthesized locus's substituted member
                // type positions for nested generic uses.
                for member in &synthesized.members {
                    Self::collect_in_locus_member(
                        member,
                        &generic_names,
                        &mut seen_mangles,
                        &mut requests,
                    )?;
                }
                synthesized_loci.push(synthesized);
            }
        }
        // Phase 2: declare synthesized types in dependency order.
        // Each iteration, attempt every still-pending decl; keep
        // those whose declarations succeed and retry the rest.
        // Lack of progress in a full pass means a cycle (which
        // shouldn't happen for value-shaped generic types — they
        // can't directly contain themselves) and surfaces a
        // clear error.
        let mut pending = synthesized_types;
        while !pending.is_empty() {
            let mut next_pending: Vec<TypeDecl> = Vec::new();
            let mut progress = false;
            // Snapshot user_types/user_enums state before the
            // pass so a partial registration in declare_user_type
            // (struct opaque type creation, enum tag table) can
            // be detected and retried cleanly. In practice
            // declare_user_type either fully succeeds (registers
            // in user_types/user_enums) or returns Err before
            // any partial state lands; the retry loop relies on
            // that contract.
            for syn in pending.drain(..) {
                let mangled = syn.name.name.clone();
                match self.declare_user_type(&syn) {
                    Ok(()) => {
                        progress = true;
                    }
                    Err(CodegenError::Unsupported(msg))
                        if msg.contains("not synthesized")
                            || msg.contains("unknown type name") =>
                    {
                        // Defer: a referenced dep isn't declared
                        // yet. Keep for retry.
                        let _ = mangled;
                        next_pending.push(syn);
                    }
                    Err(e) => return Err(e),
                }
            }
            if !progress {
                let names: Vec<String> = next_pending
                    .iter()
                    .map(|t| t.name.name.clone())
                    .collect();
                return Err(CodegenError::Unsupported(format!(
                    "generic-type dependency cycle or unresolvable \
                     dep among synthesized monomorphs: {:?}",
                    names
                )));
            }
            pending = next_pending;
        }
        // Register generic locus templates so user_loci lookups
        // and bare-name-resolution paths can find them.
        for l in &raw_locus_decls {
            if !l.generics.is_empty() {
                self.generic_locus_templates
                    .insert(l.name.name.clone(), l.clone());
            }
        }

        // Now declare concrete user-written non-generic decls.
        // The generic templates themselves are skipped inside
        // declare_user_type (m61: generic decls produce no LLVM
        // type directly; their instantiations were declared
        // above).
        for t in &type_decls {
            self.declare_user_type(t)?;
        }

        // Pass A: declare each user-defined locus. Split in two so
        // accept's child-locus param can resolve regardless of the
        // declaration order in source:
        //   A1: every locus's struct type + field layout
        //   A2: every locus's lifecycle method signatures
        // m63: append synthesized locus instantiations (from
        // the m63 monomorphization pass above) to the
        // user-written locus list so they flow through the
        // standard A1 / A2 / C passes alongside non-generic
        // decls. Synthesized loci have empty `generics`, so the
        // declare_/lower_ passes treat them as regular concrete
        // loci with the mangled names.
        let mut locus_decls: Vec<LocusDecl> = raw_locus_decls.clone();
        locus_decls.extend(synthesized_loci);
        for l in &locus_decls {
            self.declare_locus_struct(l)?;
        }
        for l in &locus_decls {
            self.declare_locus_methods(l)?;
        }

        // After A2: if any locus declared a `bus subscribe`, mark
        // the program as bus-active. m45-followup: bus storage
        // moved to the C runtime, so this is a presence flag now —
        // there's no LLVM-side table to size, no compile-time cap
        // to budget, and no dispatch-fn body to emit. `lower_send`
        // checks the flag so a stray `<-` in a subscriber-less
        // program errors at compile time (preserving the prior
        // diagnostic) rather than calling `lotus_bus_dispatch` on
        // an empty C-runtime table at runtime.
        let decl_subs: u64 = self
            .user_loci
            .values()
            .map(|info| info.subscriptions.len() as u64)
            .sum();
        if decl_subs > 0 {
            self.init_bus_state()?;
        }

        // Pass A3 (m60): synthesize per-payload-type serializer
        // and deserializer fns. Walks every locus's bus declarations
        // and pulls the type names from both Subscribe (carried in
        // info.subscriptions[].2) and Publish (carried in the AST
        // directly, since publish doesn't go through info). Dedupe
        // by name. Bodies are identity at v0.1; the shape is what
        // matters — call sites in lower_send + emit_bus_register
        // route payloads through these hooks instead of memcpy'ing
        // struct bytes inline, so a future wire-format milestone
        // drops in by replacing the bodies, not the call sites.
        let mut payload_types: BTreeSet<String> = BTreeSet::new();
        for info in self.user_loci.values() {
            for (_, _, payload_type) in &info.subscriptions {
                payload_types.insert(payload_type.clone());
            }
        }
        for l in &locus_decls {
            for member in &l.members {
                if let LocusMember::Bus(bb) = member {
                    for bm in &bb.members {
                        if let BusMember::Publish { ty, .. } = bm {
                            if let Ok(lt) = self.type_expr_to_codegen_ty(ty) {
                                match lt {
                                    CodegenTy::TypeRef(n) => {
                                        payload_types.insert(n);
                                    }
                                    CodegenTy::Enum(n) => {
                                        payload_types.insert(n);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }
        for type_name in &payload_types {
            self.synthesize_serializer(type_name)?;
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

        // m62: register every generic fn template so
        // lower_call_expr can find them at call sites and
        // synthesize per-instantiation specialized fns on
        // demand. Templates themselves were skipped by
        // declare_user_fn (they emit no LLVM IR until pinned).
        for f in &user_fn_decls {
            if !f.generics.is_empty() {
                self.generic_fn_templates
                    .insert(f.name.name.clone(), f.clone());
            }
        }

        // Pass C: lower lifecycle method bodies (birth, run, ...).
        for l in &locus_decls {
            self.lower_locus_method_bodies(l)?;
        }

        // Pass D: lower bodies of user-defined fns.
        for f in &user_fn_decls {
            self.lower_user_fn_body(f)?;
        }

        // Pass 3: the C entry point — i32 @main(i32 argc, ptr argv).
        // m77 lifted the signature from `i32 @main()` to capture
        // argc/argv so std::env::args_count / arg can reach them.
        // The pre-m77 zero-arg signature still works for any
        // platform that doesn't actually pass them (cargo test
        // harness calls main(...) via Command::new which always
        // does), and the call into lotus_env_init in the prelude
        // below stashes the values for stdlib retrieval.
        let i32_t = self.context.i32_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let main_ty = i32_t.fn_type(&[i32_t.into(), ptr_t.into()], false);
        let main_fn = self.module.add_function("main", main_ty, None);
        let entry = self.context.append_basic_block(main_fn, "entry");
        self.builder.position_at_end(entry);
        self.current_fn = Some(main_fn);
        self.current_user_fn_ret = None;
        self.current_self = None;
        self.in_main = true;
        self.push_dissolve_frame();

        // m77: pull argc/argv off main's params and hand them to
        // the C-runtime stash so std::env::args_count / arg /
        // var / var_exists can reach them. Must run before any
        // user code in main().
        let argc_param = main_fn
            .get_nth_param(0)
            .expect("main argc param")
            .into_int_value();
        let argv_param = main_fn
            .get_nth_param(1)
            .expect("main argv param")
            .into_pointer_value();
        let env_init = self
            .module
            .get_function("lotus_env_init")
            .expect("lotus_env_init declared");
        self.builder
            .build_call(
                env_init,
                &[argc_param.into(), argv_param.into()],
                "",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

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

        // m26: spin up the cooperative-scheduler bus queue.
        // Bus dispatch enqueues here at publish time; the drain
        // loop pops cells at scope-exit points (currently before
        // deferred-dissolve flush). Even programs with no bus
        // subscribes get a queue allocated — costs ~80 bytes;
        // not worth the conditional-emit complexity to skip it.
        let queue_create = self
            .module
            .get_function("lotus_bus_queue_create")
            .expect("lotus_bus_queue_create declared");
        let queue_ptr = self
            .builder
            .build_call(queue_create, &[], "bus.queue.init")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("queue_create returns ptr");
        let queue_global = self
            .module
            .get_global("lotus.bus_queue.global")
            .expect("bus queue global declared");
        self.builder
            .build_store(queue_global.as_pointer_value(), queue_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // m59: publish the queue pointer to the C runtime so
        // subscriber-side reader threads (spawned by
        // lotus_bus_register_remote when a config entry's role
        // is `listen`) can dispatch recv'd bytes into the local
        // handler set via lotus_bus_local_dispatch. Has to happen
        // BEFORE lotus_bus_load_config below so that any reader
        // threads spawned by load_config see the queue pointer
        // rather than a NULL.
        let set_queue_fn = self
            .module
            .get_function("lotus_bus_set_queue")
            .expect("lotus_bus_set_queue declared");
        self.builder
            .build_call(
                set_queue_fn,
                &[queue_ptr.into()],
                "bus.set_queue",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // m58: load the deployment-config map (subject -> transport
        // URL + role) from the path in $LOTUS_BUS_CONFIG. Emitted
        // unconditionally — programs without the env var set hit
        // the C-runtime's `if (!path) return` early-out and pay one
        // syscall + one branch at startup. Programs with it set
        // get their cross-process bus routes opened before any
        // user code runs (so `<- "subj" | ...` calls reach remote
        // subscribers from the very first publish).
        let load_cfg_fn = self
            .module
            .get_function("lotus_bus_load_config")
            .expect("lotus_bus_load_config declared");
        let getenv_fn = self
            .module
            .get_function("getenv")
            .expect("getenv declared");
        let env_var_name = self
            .builder
            .build_global_string_ptr("LOTUS_BUS_CONFIG", "lotus.bus_config.envname")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        let cfg_path_ptr = self
            .builder
            .build_call(
                getenv_fn,
                &[env_var_name.into()],
                "lotus.bus_config.path",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("getenv returns ptr");
        self.builder
            .build_call(
                load_cfg_fn,
                &[cfg_path_ptr.into()],
                "lotus.bus_config.load",
            )
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
            self.emit_bus_queue_destroy()?;
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
        // Stmt-context: the trailing expression (if any) is evaluated
        // for side effects; its value is discarded. Callers that want
        // the value (Expr::If / Expr::Block lowering) use
        // lower_block_as_expr instead.
        if let Some(tail) = &block.tail {
            let _ = self.lower_expr(tail, scope)?;
        }
        Ok(BlockEnd::Open)
    }

    /// Lower a block in expression-context: lower its statements for
    /// side effects, then lower the trailing expression and return its
    /// value. Errors if the block has no trailing expression. Returns
    /// `(value, ty, BlockEnd)` where BlockEnd reports whether either
    /// the statements terminated control flow (in which case `value`
    /// is a poison undef and callers should branch to merge based on
    /// the reported BlockEnd).
    fn lower_block_as_expr(
        &mut self,
        block: &Block,
        scope: &mut Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy, BlockEnd), CodegenError> {
        for stmt in &block.stmts {
            match self.lower_stmt(stmt, scope)? {
                BlockEnd::Open => continue,
                BlockEnd::Terminated => {
                    // Statements terminated control flow before reaching
                    // the tail. Synthesize a placeholder value so the
                    // caller can build the phi shape; it will not be
                    // selected at runtime because the branch is closed.
                    let i32_ty = self.context.i32_type();
                    let undef = i32_ty.get_undef();
                    return Ok((
                        undef.into(),
                        CodegenTy::Int,
                        BlockEnd::Terminated,
                    ));
                }
            }
        }
        match &block.tail {
            Some(tail) => {
                let (v, ty) = self.lower_expr(tail, scope)?;
                Ok((v, ty, BlockEnd::Open))
            }
            None => Err(CodegenError::Unsupported(
                "block used as expression has no trailing expression"
                    .to_string(),
            )),
        }
    }

    /// Map a `TypeExpr` to the codegen's `CodegenTy`. Scalar
    /// primitives + bare locus type names are supported; arrays /
    /// tuples / generics wait.
    fn type_expr_to_codegen_ty(
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
                    Ok(CodegenTy::LocusRef(name.clone()))
                } else if self.user_types.contains_key(name) {
                    Ok(CodegenTy::TypeRef(name.clone()))
                } else if self.user_enums.contains_key(name) {
                    Ok(CodegenTy::Enum(name.clone()))
                } else if self.user_interfaces.contains(name) {
                    // F.20 Phase B: interface type in signature
                    // position. Lowered as a fat pointer (data +
                    // vtable); coercion from a concrete locus is
                    // built at the call site, dispatch through
                    // the vtable is emitted at the method-call site.
                    Ok(CodegenTy::Interface(name.clone()))
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
                let mangled = stdlib_mangled_for_path(&segs).ok_or_else(|| {
                    CodegenError::Unsupported(format!(
                        "qualified type `{}` not in stdlib path-renames table",
                        segs.join("::")
                    ))
                })?;
                if self.user_loci.contains_key(mangled) {
                    Ok(CodegenTy::LocusRef(mangled.to_string()))
                } else if self.user_types.contains_key(mangled) {
                    // m84: path-qualified stdlib `type` records.
                    // `std::http::Request` in a fn signature
                    // resolves to TypeRef("__StdHttpRequest").
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
        let mut fields: BTreeMap<String, (u32, CodegenTy)> = BTreeMap::new();
        fields.insert("locus".into(), (0, CodegenTy::String));
        fields.insert("closure".into(), (1, CodegenTy::String));
        fields.insert("diff".into(), (2, CodegenTy::Int));
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

    /// m65: synthesize the built-in stdlib generic enums
    /// `Result<T, E>` and `Option<T>` as TypeDecls so programs
    /// can reference them without an explicit `type` declaration.
    /// They flow through the same m61c generic-enum
    /// monomorphization path as user-declared generics: discovery
    /// walks call sites and field types, synthesis produces e.g.
    /// `Result_Int_String` / `Option_Int`, and the path-call
    /// constructor (`Result_Int_String::Ok(...)`) lights up
    /// automatically. Returned with synthetic spans (0,0); they
    /// never surface in diagnostics because templates with
    /// non-empty generics aren't lowered directly — only their
    /// concrete instantiations are.
    fn builtin_generic_type_decls() -> Vec<TypeDecl> {
        let span = aperio_syntax::span::Span::new(0, 0);
        let mk_ident = |s: &str| Ident {
            name: s.to_string(),
            span,
        };
        let mk_t = |name: &str| TypeExpr::Named {
            path: QualifiedName {
                segments: vec![mk_ident(name)],
                span,
            },
            generic_args: Vec::new(),
            span,
        };
        let mk_param = |name: &str| GenericParam {
            name: mk_ident(name),
            bound: None,
            span,
        };
        let result_decl = TypeDecl {
            name: mk_ident("Result"),
            generics: vec![mk_param("T"), mk_param("E")],
            body: TypeDeclBody::Enum(vec![
                EnumVariant {
                    name: mk_ident("Ok"),
                    fields: vec![mk_t("T")],
                    span,
                },
                EnumVariant {
                    name: mk_ident("Err"),
                    fields: vec![mk_t("E")],
                    span,
                },
            ]),
            span,
        };
        let option_decl = TypeDecl {
            name: mk_ident("Option"),
            generics: vec![mk_param("T")],
            body: TypeDeclBody::Enum(vec![
                EnumVariant {
                    name: mk_ident("Some"),
                    fields: vec![mk_t("T")],
                    span,
                },
                EnumVariant {
                    name: mk_ident("None"),
                    fields: Vec::new(),
                    span,
                },
            ]),
            span,
        };
        vec![result_decl, option_decl]
    }

    /// Pass A0: declare a user `type` decl as an LLVM struct type.
    /// Aliases and enums are not yet lowered — only struct bodies.
    /// No defaults are expected (the language requires struct
    /// literals to provide every field at the call site).
    fn declare_user_type(&mut self, t: &TypeDecl) -> Result<(), CodegenError> {
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
        let mut llvm_field_tys: Vec<inkwell::types::BasicTypeEnum> =
            Vec::new();
        for (idx, f) in struct_fields.iter().enumerate() {
            let ft = self.type_expr_to_codegen_ty(&f.ty)?;
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

    /// m64: check that each generic param's declared bound (if
    /// any) is satisfied by the corresponding type arg. v0.1
    /// recognizes only the `Numeric` bound, which admits Int /
    /// Float / Decimal / Duration. Unknown bound names error
    /// clearly so future bounds (e.g., `Eq`, `Ord`,
    /// `ProjectionClass`) surface a gap rather than passing
    /// silently. Called from each synthesize_generic_* helper
    /// before substitution so the check fires on the same code
    /// path that produces the mangled name.
    fn check_generic_bounds(
        template_kind: &str,
        template_name: &str,
        params: &[GenericParam],
        args: &[TypeExpr],
    ) -> Result<(), CodegenError> {
        for (gp, arg) in params.iter().zip(args.iter()) {
            let bound = match &gp.bound {
                Some(b) => b,
                None => continue,
            };
            let bound_name = match bound {
                TypeExpr::Named { path, generic_args, .. }
                    if path.segments.len() == 1
                        && generic_args.is_empty() =>
                {
                    path.segments[0].name.as_str()
                }
                _ => {
                    return Err(CodegenError::Unsupported(format!(
                        "{} `{}`: bound on `{}` is not a simple \
                         named bound (only simple names like \
                         `Numeric` recognized at v0.1)",
                        template_kind, template_name, gp.name.name
                    )));
                }
            };
            match bound_name {
                "Numeric" => {
                    let ok = matches!(
                        arg,
                        TypeExpr::Primitive(
                            PrimType::Int
                                | PrimType::Float
                                | PrimType::Decimal
                                | PrimType::Duration,
                            _,
                        )
                    );
                    if !ok {
                        return Err(CodegenError::Unsupported(format!(
                            "{} `{}`: type arg for `{}` must satisfy \
                             `Numeric` bound (Int / Float / Decimal / \
                             Duration); got non-numeric",
                            template_kind, template_name, gp.name.name
                        )));
                    }
                }
                other => {
                    return Err(CodegenError::Unsupported(format!(
                        "{} `{}`: unrecognized bound `{}` on `{}` \
                         (only `Numeric` recognized at v0.1)",
                        template_kind, template_name, other, gp.name.name
                    )));
                }
            }
        }
        Ok(())
    }

    /// m61: produce the mangled name for a generic instantiation.
    /// `Box<Int>` → `"Box_Int"`, `Pair<Int, String>` →
    /// `"Pair_Int_String"`. Recurses into nested generics so
    /// `Box<Pair<Int, String>>` → `"Box_Pair_Int_String"`. Each
    /// arg must be a primitive or a non-generic user type at this
    /// milestone (or itself a generic instantiation, which mangles
    /// recursively).
    fn mangle_generic_name(
        template: &str,
        args: &[TypeExpr],
    ) -> Result<String, CodegenError> {
        let mut tokens: Vec<String> = Vec::with_capacity(args.len());
        for a in args {
            tokens.push(Self::type_expr_mangle_token(a)?);
        }
        Ok(format!("{}_{}", template, tokens.join("_")))
    }

    /// m61: produce a single-token mangle for one generic arg.
    /// Primitives use their canonical name (`Int`, `String`,
    /// ...); a non-generic Named ref uses the bare name; a
    /// generic ref recurses through `mangle_generic_name`.
    fn type_expr_mangle_token(t: &TypeExpr) -> Result<String, CodegenError> {
        match t {
            TypeExpr::Primitive(p, _) => match p {
                PrimType::Int => Ok("Int".into()),
                PrimType::Float => Ok("Float".into()),
                PrimType::Bool => Ok("Bool".into()),
                PrimType::String => Ok("String".into()),
                PrimType::Duration => Ok("Duration".into()),
                PrimType::Decimal => Ok("Decimal".into()),
                PrimType::Time => Ok("Time".into()),
                other => Err(CodegenError::Unsupported(format!(
                    "primitive `{:?}` as generic arg (m61 v0.1)",
                    other
                ))),
            },
            TypeExpr::Named { path, generic_args, .. }
                if path.segments.len() == 1 =>
            {
                if generic_args.is_empty() {
                    Ok(path.segments[0].name.clone())
                } else {
                    Self::mangle_generic_name(
                        &path.segments[0].name,
                        generic_args,
                    )
                }
            }
            other => Err(CodegenError::Unsupported(format!(
                "type form `{:?}` as generic arg (m61 v0.1 only \
                 supports primitives + named types)",
                other
            ))),
        }
    }

    /// m61: substitute generic param refs (`T`, `U`, ...) inside a
    /// `TypeExpr` per the supplied substitution map. Recurses into
    /// generic args, array elems, tuple parts, etc., so a body
    /// like `[T; 4]` substitutes to `[Int; 4]` correctly.
    fn substitute_type_expr(
        expr: &TypeExpr,
        subst: &BTreeMap<String, TypeExpr>,
    ) -> TypeExpr {
        match expr {
            TypeExpr::Named { path, generic_args, span }
                if path.segments.len() == 1
                    && generic_args.is_empty()
                    && subst.contains_key(&path.segments[0].name) =>
            {
                subst[&path.segments[0].name].clone()
            }
            TypeExpr::Named { path, generic_args, span } => TypeExpr::Named {
                path: path.clone(),
                generic_args: generic_args
                    .iter()
                    .map(|a| Self::substitute_type_expr(a, subst))
                    .collect(),
                span: span.clone(),
            },
            TypeExpr::Array { elem, size, span } => TypeExpr::Array {
                elem: Box::new(Self::substitute_type_expr(elem, subst)),
                size: size.clone(),
                span: span.clone(),
            },
            TypeExpr::Tuple(parts, span) => TypeExpr::Tuple(
                parts
                    .iter()
                    .map(|p| Self::substitute_type_expr(p, subst))
                    .collect(),
                span.clone(),
            ),
            TypeExpr::Projection { class, inner, span } => {
                TypeExpr::Projection {
                    class: *class,
                    inner: Box::new(Self::substitute_type_expr(inner, subst)),
                    span: span.clone(),
                }
            }
            other => other.clone(),
        }
    }

    /// m63: synthesize a concrete (non-generic) LocusDecl from
    /// a generic template + a tuple of resolved type args. The
    /// substitution walks every TypeExpr-bearing position in
    /// the locus's members:
    /// - Params block (ParamDecl.ty)
    /// - Bus block (Subscribe.ty + Publish.ty)
    /// - Lifecycle decls (params + ret + body let-ascriptions)
    /// - Fn methods (params + ret + body let-ascriptions)
    /// - Const decls (ty)
    /// - Nested Type decls (struct fields + enum variant fields)
    ///
    /// v0.1 limits: Mode, Failure, Closure, Contract members
    /// pass through unchanged. Generic loci using those
    /// surfaces would need m63b. The body walk for lifecycle /
    /// fn methods only substitutes let / let-tuple ascriptions
    /// (matching the m62 fn-body shallow substitution).
    fn synthesize_generic_locus_instantiation(
        template: &LocusDecl,
        type_args: &[TypeExpr],
        mangled_name: &str,
    ) -> Result<LocusDecl, CodegenError> {
        if template.generics.len() != type_args.len() {
            return Err(CodegenError::Unsupported(format!(
                "generic locus `{}`: expected {} type args, got {}",
                template.name.name,
                template.generics.len(),
                type_args.len()
            )));
        }
        Self::check_generic_bounds(
            "generic locus",
            &template.name.name,
            &template.generics,
            type_args,
        )?;
        let mut subst: BTreeMap<String, TypeExpr> = BTreeMap::new();
        for (gp, arg) in template.generics.iter().zip(type_args.iter()) {
            subst.insert(gp.name.name.clone(), arg.clone());
        }
        let new_members: Vec<LocusMember> = template
            .members
            .iter()
            .map(|m| Self::substitute_locus_member(m, &subst))
            .collect();
        Ok(LocusDecl {
            name: Ident {
                name: mangled_name.to_string(),
                span: template.name.span.clone(),
            },
            generics: Vec::new(),
            annotations: template.annotations.clone(),
            form: template.form.clone(),
            members: new_members,
            span: template.span.clone(),
        })
    }

    fn substitute_locus_member(
        member: &LocusMember,
        subst: &BTreeMap<String, TypeExpr>,
    ) -> LocusMember {
        match member {
            LocusMember::Params(pb) => LocusMember::Params(ParamsBlock {
                params: pb
                    .params
                    .iter()
                    .map(|p| ParamDecl {
                        name: p.name.clone(),
                        ty: p.ty.as_ref().map(|t| {
                            Self::substitute_type_expr(t, subst)
                        }),
                        init: p.init.clone(),
                        span: p.span.clone(),
                    })
                    .collect(),
                span: pb.span.clone(),
            }),
            LocusMember::Bus(bb) => LocusMember::Bus(BusBlock {
                members: bb
                    .members
                    .iter()
                    .map(|bm| match bm {
                        BusMember::Subscribe { subject, handler, ty, span } => {
                            BusMember::Subscribe {
                                subject: subject.clone(),
                                handler: handler.clone(),
                                ty: ty.as_ref().map(|t| {
                                    Self::substitute_type_expr(t, subst)
                                }),
                                span: span.clone(),
                            }
                        }
                        BusMember::Publish { subject, ty, alias, span } => {
                            BusMember::Publish {
                                subject: subject.clone(),
                                ty: Self::substitute_type_expr(ty, subst),
                                alias: alias.clone(),
                                span: span.clone(),
                            }
                        }
                    })
                    .collect(),
                span: bb.span.clone(),
            }),
            LocusMember::Lifecycle(lc) => LocusMember::Lifecycle(
                LifecycleDecl {
                    kind: lc.kind,
                    params: lc
                        .params
                        .iter()
                        .map(|p| Param {
                            name: p.name.clone(),
                            ty: Self::substitute_type_expr(&p.ty, subst),
                            default: p.default.clone(),
                            span: p.span.clone(),
                        })
                        .collect(),
                    ret: lc
                        .ret
                        .as_ref()
                        .map(|t| Self::substitute_type_expr(t, subst)),
                    body: Self::substitute_block_type_ascriptions(
                        &lc.body, subst,
                    ),
                    span: lc.span.clone(),
                },
            ),
            LocusMember::Fn(fd) => LocusMember::Fn(FnDecl {
                name: fd.name.clone(),
                generics: fd.generics.clone(),
                params: fd
                    .params
                    .iter()
                    .map(|p| Param {
                        name: p.name.clone(),
                        ty: Self::substitute_type_expr(&p.ty, subst),
                        default: p.default.clone(),
                        span: p.span.clone(),
                    })
                    .collect(),
                ret: fd
                    .ret
                    .as_ref()
                    .map(|t| Self::substitute_type_expr(t, subst)),
                fallible: fd
                    .fallible
                    .as_ref()
                    .map(|t| Self::substitute_type_expr(t, subst)),
                body: Self::substitute_block_type_ascriptions(
                    &fd.body, subst,
                ),
                span: fd.span.clone(),
            }),
            LocusMember::Const(c) => LocusMember::Const(ConstDecl {
                name: c.name.clone(),
                ty: Self::substitute_type_expr(&c.ty, subst),
                value: c.value.clone(),
                span: c.span.clone(),
            }),
            // Mode, Failure, Closure, Contract, Type pass through
            // unchanged at v0.1; m63b can extend them when a
            // workload exercises generic loci that use those
            // surfaces.
            other => other.clone(),
        }
    }

    /// m62: convert a CodegenTy back to a TypeExpr for the
    /// generic-fn inference path. The resulting TypeExpr is used
    /// to mangle the instantiation name and to substitute into
    /// the template's body — both purely structural operations,
    /// so the synthetic spans are fine.
    fn codegen_ty_to_type_expr(
        t: &CodegenTy,
    ) -> Result<TypeExpr, CodegenError> {
        // Synthetic span for the synthesized TypeExpr — these
        // never surface in user-visible diagnostics because m62
        // structural ops only inspect shape, not source location.
        let span = aperio_syntax::span::Span::new(0, 0);
        match t {
            CodegenTy::Int => Ok(TypeExpr::Primitive(PrimType::Int, span)),
            CodegenTy::Float => {
                Ok(TypeExpr::Primitive(PrimType::Float, span))
            }
            CodegenTy::Bool => Ok(TypeExpr::Primitive(PrimType::Bool, span)),
            CodegenTy::String => {
                Ok(TypeExpr::Primitive(PrimType::String, span))
            }
            CodegenTy::Duration => {
                Ok(TypeExpr::Primitive(PrimType::Duration, span))
            }
            CodegenTy::Decimal => {
                Ok(TypeExpr::Primitive(PrimType::Decimal, span))
            }
            CodegenTy::Time => Ok(TypeExpr::Primitive(PrimType::Time, span)),
            CodegenTy::TypeRef(name) | CodegenTy::Enum(name) => {
                Ok(TypeExpr::Named {
                    path: QualifiedName {
                        segments: vec![Ident {
                            name: name.clone(),
                            span,
                        }],
                        span,
                    },
                    generic_args: Vec::new(),
                    span,
                })
            }
            other => Err(CodegenError::Unsupported(format!(
                "codegen_ty_to_type_expr: form `{:?}` not supported \
                 (m62 v0.1 limits inference to primitives + named \
                 types as generic args)",
                other
            ))),
        }
    }

    /// m62: structurally walk a declared TypeExpr against an
    /// actual CodegenTy, recording bindings for any generic
    /// param refs. `params` names which idents in the TypeExpr
    /// represent generic params (vs. concrete user types).
    /// Errors if a param binds to multiple distinct types
    /// (inconsistent inference).
    fn unify_generic_param_bindings(
        declared: &TypeExpr,
        actual: &CodegenTy,
        params: &BTreeSet<String>,
        bindings: &mut BTreeMap<String, TypeExpr>,
    ) -> Result<(), CodegenError> {
        // Generic-param ref: bind to actual.
        if let TypeExpr::Named {
            path, generic_args, ..
        } = declared
        {
            if path.segments.len() == 1
                && generic_args.is_empty()
                && params.contains(&path.segments[0].name)
            {
                let bound = Self::codegen_ty_to_type_expr(actual)?;
                let name = &path.segments[0].name;
                if let Some(prior) = bindings.get(name) {
                    if prior != &bound {
                        return Err(CodegenError::Unsupported(format!(
                            "generic param `{}` inferred as both \
                             `{:?}` and `{:?}` from call site",
                            name, prior, bound
                        )));
                    }
                } else {
                    bindings.insert(name.clone(), bound);
                }
                return Ok(());
            }
        }
        // Otherwise structural recurse where shapes match.
        match (declared, actual) {
            (TypeExpr::Array { elem, .. }, CodegenTy::Array(a_elem, _)) => {
                Self::unify_generic_param_bindings(
                    elem, a_elem, params, bindings,
                )
            }
            (TypeExpr::Tuple(parts, _), CodegenTy::Tuple(a_parts))
                if parts.len() == a_parts.len() =>
            {
                for (p, a) in parts.iter().zip(a_parts) {
                    Self::unify_generic_param_bindings(
                        p, a, params, bindings,
                    )?;
                }
                Ok(())
            }
            // Concrete-vs-concrete shapes: nothing to bind.
            // Mismatches don't error here — the typechecker (or
            // the call site type check after substitution) will
            // surface them.
            _ => Ok(()),
        }
    }

    /// m62: infer the concrete type-args tuple for a generic fn
    /// call by unifying each declared param TypeExpr against the
    /// actual arg CodegenTy. Returns the args in the same order
    /// as the template's `generics: Vec<GenericParam>`.
    fn infer_generic_fn_args(
        template: &FnDecl,
        actual_arg_tys: &[CodegenTy],
    ) -> Result<Vec<TypeExpr>, CodegenError> {
        let visible_args = template.params.len().min(actual_arg_tys.len());
        let generic_param_names: BTreeSet<String> = template
            .generics
            .iter()
            .map(|g| g.name.name.clone())
            .collect();
        let mut bindings: BTreeMap<String, TypeExpr> = BTreeMap::new();
        for (p, actual_ty) in template
            .params
            .iter()
            .zip(actual_arg_tys.iter())
            .take(visible_args)
        {
            Self::unify_generic_param_bindings(
                &p.ty,
                actual_ty,
                &generic_param_names,
                &mut bindings,
            )?;
        }
        let mut args: Vec<TypeExpr> = Vec::new();
        for gp in &template.generics {
            match bindings.get(&gp.name.name) {
                Some(t) => args.push(t.clone()),
                None => {
                    return Err(CodegenError::Unsupported(format!(
                        "generic fn `{}`: could not infer param `{}` \
                         from call site (m62 v0.1 requires every \
                         generic param to appear in an arg position \
                         that pins it)",
                        template.name.name, gp.name.name
                    )));
                }
            }
        }
        Ok(args)
    }

    /// m62: synthesize a concrete (non-generic) FnDecl from a
    /// generic template + a tuple of resolved type args. The
    /// template's params, return type, and body type-ascriptions
    /// (let / let-tuple) are walked through `substitute_type_expr`.
    /// Body expression-level generic-typed sites that aren't
    /// covered (e.g., struct literals using a generic type
    /// without a let ascription) flow through as-is and rely on
    /// the m61b/m61c surface resolution at lowering time.
    fn synthesize_generic_fn_instantiation(
        template: &FnDecl,
        type_args: &[TypeExpr],
        mangled_name: &str,
    ) -> Result<FnDecl, CodegenError> {
        if template.generics.len() != type_args.len() {
            return Err(CodegenError::Unsupported(format!(
                "generic fn `{}`: expected {} type args, got {}",
                template.name.name,
                template.generics.len(),
                type_args.len()
            )));
        }
        Self::check_generic_bounds(
            "generic fn",
            &template.name.name,
            &template.generics,
            type_args,
        )?;
        let mut subst: BTreeMap<String, TypeExpr> = BTreeMap::new();
        for (gp, arg) in template.generics.iter().zip(type_args.iter()) {
            subst.insert(gp.name.name.clone(), arg.clone());
        }
        let new_params: Vec<Param> = template
            .params
            .iter()
            .map(|p| Param {
                name: p.name.clone(),
                ty: Self::substitute_type_expr(&p.ty, &subst),
                default: p.default.clone(),
                span: p.span.clone(),
            })
            .collect();
        let new_ret = template
            .ret
            .as_ref()
            .map(|t| Self::substitute_type_expr(t, &subst));
        let new_body = Self::substitute_block_type_ascriptions(
            &template.body,
            &subst,
        );
        let new_fallible = template
            .fallible
            .as_ref()
            .map(|t| Self::substitute_type_expr(t, &subst));
        Ok(FnDecl {
            name: Ident {
                name: mangled_name.to_string(),
                span: template.name.span.clone(),
            },
            generics: Vec::new(),
            params: new_params,
            ret: new_ret,
            fallible: new_fallible,
            body: new_body,
            span: template.span.clone(),
        })
    }

    /// m62: shallow walk of a block to substitute generic-param
    /// refs in any let/let-tuple type ascriptions. Other
    /// type-position uses inside the body (cast targets, nested
    /// fn decl signatures, etc.) are not walked — generic fn
    /// bodies that need those aren't expected at v0.1 and would
    /// surface as type errors after substitution leaves them
    /// pointing at unbound names.
    fn substitute_block_type_ascriptions(
        block: &Block,
        subst: &BTreeMap<String, TypeExpr>,
    ) -> Block {
        let new_stmts = block
            .stmts
            .iter()
            .map(|s| Self::substitute_stmt_type_ascriptions(s, subst))
            .collect();
        Block {
            stmts: new_stmts,
            tail: block.tail.clone(),
            span: block.span.clone(),
        }
    }

    fn substitute_stmt_type_ascriptions(
        stmt: &Stmt,
        subst: &BTreeMap<String, TypeExpr>,
    ) -> Stmt {
        match stmt {
            Stmt::Let { is_mut, name, ty, value, span } => Stmt::Let {
                is_mut: *is_mut,
                name: name.clone(),
                ty: ty
                    .as_ref()
                    .map(|t| Self::substitute_type_expr(t, subst)),
                value: value.clone(),
                span: span.clone(),
            },
            Stmt::LetTuple { is_mut, names, ty, value, span } => {
                Stmt::LetTuple {
                    is_mut: *is_mut,
                    names: names.clone(),
                    ty: ty
                        .as_ref()
                        .map(|t| Self::substitute_type_expr(t, subst)),
                    value: value.clone(),
                    span: span.clone(),
                }
            }
            other => other.clone(),
        }
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
    fn resolve_generic_struct_path_for_codegen_ty(
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

    fn resolve_generic_struct_path(
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

    /// m61b: comprehensive walk for generic instantiations
    /// across the entire program. Calls `collect_generic_uses` at
    /// every TypeExpr-bearing site we care about: type decl
    /// fields, fn / locus signatures, bus payload types, let
    /// ascriptions in fn bodies. The walk is purely structural —
    /// the typechecker has already validated names — and runs
    /// before any synthesis so the synthesized instantiations
    /// land in user_types before any concrete decl's bodies are
    /// resolved.
    fn collect_generic_uses_in_program(
        program: &Program,
        generic_names: &BTreeSet<String>,
        seen: &mut BTreeSet<String>,
        requests: &mut Vec<(String, Vec<TypeExpr>)>,
    ) -> Result<(), CodegenError> {
        for item in &program.items {
            match item {
                TopDecl::Type(t) if t.generics.is_empty() => {
                    match &t.body {
                        TypeDeclBody::Struct(fields) => {
                            for f in fields {
                                Self::collect_generic_uses(
                                    &f.ty,
                                    generic_names,
                                    seen,
                                    requests,
                                )?;
                            }
                        }
                        // m61c: enum variant field types can
                        // reference generic instantiations
                        // (e.g., a non-generic enum variant
                        // carrying a Box<Int> payload).
                        TypeDeclBody::Enum(variants) => {
                            for v in variants {
                                for f in &v.fields {
                                    Self::collect_generic_uses(
                                        f,
                                        generic_names,
                                        seen,
                                        requests,
                                    )?;
                                }
                            }
                        }
                        TypeDeclBody::Alias(_) => {}
                    }
                }
                TopDecl::Type(_) => {
                    /* generic template — its own body's `T`
                     * references aren't instantiations. */
                }
                TopDecl::Fn(f) => {
                    Self::collect_in_fn_decl(
                        f,
                        generic_names,
                        seen,
                        requests,
                    )?;
                }
                TopDecl::Locus(l) if l.generics.is_empty() => {
                    for member in &l.members {
                        Self::collect_in_locus_member(
                            member,
                            generic_names,
                            seen,
                            requests,
                        )?;
                    }
                }
                TopDecl::Locus(_) => {
                    /* m63: generic locus template — its members'
                     * type positions still mention the generic
                     * params (e.g. `Box<T>`) which can't unify
                     * against any actual concrete type yet.
                     * Synthesis later walks the substituted
                     * version. */
                }
                TopDecl::Const(c) => {
                    Self::collect_generic_uses(
                        &c.ty,
                        generic_names,
                        seen,
                        requests,
                    )?;
                }
                TopDecl::Perspective(_) | TopDecl::Module(_) => {
                    /* Perspective / Module type-bearing positions
                     * could grow into this when those features
                     * land in v0.1 codegen. */
                }
                TopDecl::Interface(_) => {
                    /* Interface declarations have no body to walk
                     * for generic uses. Method signatures are
                     * captured separately during the impl-check
                     * pass. */
                }
            }
        }
        Ok(())
    }

    fn collect_in_fn_decl(
        f: &FnDecl,
        generic_names: &BTreeSet<String>,
        seen: &mut BTreeSet<String>,
        requests: &mut Vec<(String, Vec<TypeExpr>)>,
    ) -> Result<(), CodegenError> {
        for p in &f.params {
            Self::collect_generic_uses(&p.ty, generic_names, seen, requests)?;
        }
        if let Some(ret) = &f.ret {
            Self::collect_generic_uses(ret, generic_names, seen, requests)?;
        }
        Self::collect_in_block(&f.body, generic_names, seen, requests)?;
        Ok(())
    }

    fn collect_in_locus_member(
        member: &LocusMember,
        generic_names: &BTreeSet<String>,
        seen: &mut BTreeSet<String>,
        requests: &mut Vec<(String, Vec<TypeExpr>)>,
    ) -> Result<(), CodegenError> {
        match member {
            LocusMember::Params(pb) => {
                for p in &pb.params {
                    if let Some(t) = &p.ty {
                        Self::collect_generic_uses(
                            t,
                            generic_names,
                            seen,
                            requests,
                        )?;
                    }
                }
            }
            LocusMember::Bus(bb) => {
                for bm in &bb.members {
                    match bm {
                        BusMember::Subscribe { ty: Some(t), .. } => {
                            Self::collect_generic_uses(
                                t,
                                generic_names,
                                seen,
                                requests,
                            )?;
                        }
                        BusMember::Publish { ty, .. } => {
                            Self::collect_generic_uses(
                                ty,
                                generic_names,
                                seen,
                                requests,
                            )?;
                        }
                        BusMember::Subscribe { ty: None, .. } => {}
                    }
                }
            }
            LocusMember::Lifecycle(lc) => {
                for p in &lc.params {
                    Self::collect_generic_uses(
                        &p.ty,
                        generic_names,
                        seen,
                        requests,
                    )?;
                }
                if let Some(r) = &lc.ret {
                    Self::collect_generic_uses(
                        r,
                        generic_names,
                        seen,
                        requests,
                    )?;
                }
                Self::collect_in_block(
                    &lc.body,
                    generic_names,
                    seen,
                    requests,
                )?;
            }
            LocusMember::Fn(fd) => {
                Self::collect_in_fn_decl(
                    fd,
                    generic_names,
                    seen,
                    requests,
                )?;
            }
            LocusMember::Mode(_) => {
                /* Modes' body / params walk would mirror Fn; not
                 * yet wired. m61c. */
            }
            LocusMember::Const(c) => {
                Self::collect_generic_uses(
                    &c.ty,
                    generic_names,
                    seen,
                    requests,
                )?;
            }
            LocusMember::Type(t) if t.generics.is_empty() => {
                match &t.body {
                    TypeDeclBody::Struct(fields) => {
                        for f in fields {
                            Self::collect_generic_uses(
                                &f.ty,
                                generic_names,
                                seen,
                                requests,
                            )?;
                        }
                    }
                    TypeDeclBody::Enum(variants) => {
                        for v in variants {
                            for f in &v.fields {
                                Self::collect_generic_uses(
                                    f,
                                    generic_names,
                                    seen,
                                    requests,
                                )?;
                            }
                        }
                    }
                    TypeDeclBody::Alias(_) => {}
                }
            }
            LocusMember::Contract(_)
            | LocusMember::Failure(_)
            | LocusMember::Closure(_)
            | LocusMember::Type(_) => {}
            LocusMember::Capacity(_) => {
                // F.22 slot cell types are concrete in v1; no
                // generic-template use sites. Future generic Pool/
                // Heap (`pool entries of T;` with locus generic
                // T) would walk slot.elem_ty here.
            }
        }
        Ok(())
    }

    fn collect_in_block(
        block: &Block,
        generic_names: &BTreeSet<String>,
        seen: &mut BTreeSet<String>,
        requests: &mut Vec<(String, Vec<TypeExpr>)>,
    ) -> Result<(), CodegenError> {
        for stmt in &block.stmts {
            Self::collect_in_stmt(stmt, generic_names, seen, requests)?;
        }
        Ok(())
    }

    fn collect_in_stmt(
        stmt: &Stmt,
        generic_names: &BTreeSet<String>,
        seen: &mut BTreeSet<String>,
        requests: &mut Vec<(String, Vec<TypeExpr>)>,
    ) -> Result<(), CodegenError> {
        match stmt {
            Stmt::Let { ty: Some(t), .. } => {
                Self::collect_generic_uses(t, generic_names, seen, requests)?;
            }
            Stmt::LetTuple { ty: Some(t), .. } => {
                Self::collect_generic_uses(t, generic_names, seen, requests)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// m61: walk a TypeExpr looking for generic instantiations
    /// that reference one of the known generic templates. Each
    /// unique (template_name, args) tuple is recorded in
    /// `requests` (deduped via the mangled name in `seen`), with
    /// recursion into the args themselves so nested generics like
    /// `Box<Pair<Int, String>>` register `Pair_Int_String` first
    /// then `Box_Pair_Int_String`. Discovery is purely structural
    /// — the typechecker has already validated names.
    fn collect_generic_uses(
        t: &TypeExpr,
        generic_names: &BTreeSet<String>,
        seen: &mut BTreeSet<String>,
        requests: &mut Vec<(String, Vec<TypeExpr>)>,
    ) -> Result<(), CodegenError> {
        match t {
            TypeExpr::Named { path, generic_args, .. }
                if path.segments.len() == 1
                    && !generic_args.is_empty()
                    && generic_names
                        .contains(&path.segments[0].name) =>
            {
                // Recurse into args first so nested instantiations
                // are discovered (and end up in requests order
                // before the outer one — which is what
                // declare_user_type needs to resolve them).
                for a in generic_args {
                    Self::collect_generic_uses(
                        a,
                        generic_names,
                        seen,
                        requests,
                    )?;
                }
                let mangled = Self::mangle_generic_name(
                    &path.segments[0].name,
                    generic_args,
                )?;
                if seen.insert(mangled) {
                    requests.push((
                        path.segments[0].name.clone(),
                        generic_args.clone(),
                    ));
                }
            }
            TypeExpr::Named { generic_args, .. } => {
                /* non-generic Named ref (or unknown): still
                 * recurse into any args in case they themselves
                 * use a known generic template. */
                for a in generic_args {
                    Self::collect_generic_uses(
                        a,
                        generic_names,
                        seen,
                        requests,
                    )?;
                }
            }
            TypeExpr::Array { elem, .. } => Self::collect_generic_uses(
                elem,
                generic_names,
                seen,
                requests,
            )?,
            TypeExpr::Tuple(parts, _) => {
                for p in parts {
                    Self::collect_generic_uses(
                        p,
                        generic_names,
                        seen,
                        requests,
                    )?;
                }
            }
            TypeExpr::Projection { inner, .. } => Self::collect_generic_uses(
                inner,
                generic_names,
                seen,
                requests,
            )?,
            TypeExpr::Primitive(_, _) | TypeExpr::Function { .. } => {}
        }
        Ok(())
    }

    /// m61: synthesize a concrete TypeDecl from a generic template
    /// + a tuple of concrete type args. The synthesized decl has
    /// the mangled name (e.g., `Box_Int`), no generics, and a body
    /// where every reference to a generic param is substituted
    /// with the corresponding arg. Caller hands the result to
    /// `declare_user_type` like any other non-generic decl.
    fn synthesize_generic_instantiation(
        template: &TypeDecl,
        args: &[TypeExpr],
    ) -> Result<TypeDecl, CodegenError> {
        if template.generics.len() != args.len() {
            return Err(CodegenError::Unsupported(format!(
                "generic type `{}` expects {} args, got {}",
                template.name.name,
                template.generics.len(),
                args.len()
            )));
        }
        // m64: enforce declared bounds (e.g., `T: Numeric`)
        // before substitution.
        Self::check_generic_bounds(
            "generic type",
            &template.name.name,
            &template.generics,
            args,
        )?;
        let mangled = Self::mangle_generic_name(&template.name.name, args)?;

        let mut subst: BTreeMap<String, TypeExpr> = BTreeMap::new();
        for (gp, arg) in template.generics.iter().zip(args.iter()) {
            subst.insert(gp.name.name.clone(), arg.clone());
        }

        // Walk the body, substituting generic param refs.
        let new_body = match &template.body {
            TypeDeclBody::Struct(fields) => {
                let new_fields: Vec<StructField> = fields
                    .iter()
                    .map(|f| StructField {
                        name: f.name.clone(),
                        ty: Self::substitute_type_expr(&f.ty, &subst),
                        default: f.default.clone(),
                        span: f.span.clone(),
                    })
                    .collect();
                TypeDeclBody::Struct(new_fields)
            }
            TypeDeclBody::Alias(_) => {
                return Err(CodegenError::Unsupported(format!(
                    "generic alias `{}` (m61c v0.1 supports struct \
                     and enum templates only)",
                    template.name.name
                )));
            }
            TypeDeclBody::Enum(variants) => {
                // m61c: substitute generic params throughout each
                // variant's field TypeExprs. Variant names stay as
                // declared (e.g., `Result_Int_String::Ok` keeps
                // `Ok` as the variant name) — the enum's mangled
                // outer name + variant lookup by string handles
                // namespacing.
                let new_variants: Vec<EnumVariant> = variants
                    .iter()
                    .map(|v| EnumVariant {
                        name: v.name.clone(),
                        fields: v
                            .fields
                            .iter()
                            .map(|f| {
                                Self::substitute_type_expr(f, &subst)
                            })
                            .collect(),
                        span: v.span.clone(),
                    })
                    .collect();
                TypeDeclBody::Enum(new_variants)
            }
        };

        Ok(TypeDecl {
            name: Ident {
                name: mangled,
                span: template.name.span.clone(),
            },
            generics: Vec::new(),
            body: new_body,
            span: template.span.clone(),
        })
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
        if !l.generics.is_empty() {
            // m63: generic locus templates emit nothing at decl
            // time. Per-instantiation specialized LocusDecls get
            // synthesized in the m63 monomorphization pass and
            // flow through this same fn under their mangled
            // name.
            return Ok(());
        }
        // Resolve projection class: explicit annotation wins;
        // otherwise default per spec/memory.md (chunked if
        // accept declared, rich otherwise — recognition is
        // explicit-only since N≈100-500 is too aggressive for
        // implicit choice).
        let has_accept = l.members.iter().any(|m| {
            matches!(m, LocusMember::Lifecycle(lc)
                if matches!(lc.kind, LifecycleKind::Accept))
        });
        let projection_class: ProjectionClass = l
            .annotations
            .iter()
            .find_map(|a| match a {
                LocusAnnotation::Projection(pc) => Some(*pc),
                _ => None,
            })
            .unwrap_or(if has_accept {
                ProjectionClass::Chunked
            } else {
                ProjectionClass::Rich
            });

        // m25: schedule class. Default cooperative — even though
        // current codegen runs everything synchronously (which is
        // structurally greedy), cooperative is the spec's default
        // and the natural target for m26. Users who want today's
        // sync-everywhere semantics LOCKED IN as their design
        // choice (rather than incidental) can write
        // `: schedule greedy` explicitly.
        let schedule_class: ScheduleClass = l
            .annotations
            .iter()
            .find_map(|a| match a {
                LocusAnnotation::Schedule(sc) => Some(*sc),
                _ => None,
            })
            .unwrap_or(ScheduleClass::Cooperative);

        // Each locus param must have either a literal default or
        // a typed default expression evaluable at instantiation
        // time. Scalar literals lock in `DefaultInit::Const` so
        // const_param can build them directly; non-literal defaults
        // (like `current_kernel: Kernel = Kernel { ... }`)
        // get `DefaultInit::Expr` and are evaluated at the
        // instantiation site through lower_expr. Type ascription is
        // REQUIRED for non-literal defaults (we don't infer a type
        // from an arbitrary expression here — the AST resolver
        // doesn't run in codegen v0).
        let mut fields: BTreeMap<String, (u32, CodegenTy)> = BTreeMap::new();
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
                    let (default, default_ty): (DefaultInit, CodegenTy) =
                        match param_value(default_expr) {
                            Ok(pv) => {
                                let ty = match &pv {
                                    ParamValue::Int(_) => CodegenTy::Int,
                                    ParamValue::Float(_) => CodegenTy::Float,
                                    ParamValue::Bool(_) => CodegenTy::Bool,
                                    ParamValue::String(_) => CodegenTy::String,
                                    ParamValue::Duration(_) => {
                                        CodegenTy::Duration
                                    }
                                    ParamValue::Decimal(_) => CodegenTy::Decimal,
                                    ParamValue::Time(_) => CodegenTy::Time,
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
                                let ty = self.type_expr_to_codegen_ty(ascribed)?;
                                // m61c: bare-name struct literal in
                                // a typed param default rewrites to
                                // the mangled monomorph at decl
                                // time, so the deferred lower_expr
                                // at instantiation sees the right
                                // path. Mirrors the let-ascription
                                // hook from m61b.
                                let stored_default =
                                    match default_expr {
                                        Expr::Struct {
                                            path,
                                            inits,
                                            span,
                                        } => match self
                                            .resolve_generic_struct_path(
                                                path, ascribed,
                                            ) {
                                            Some(new_path) => {
                                                Expr::Struct {
                                                    path: new_path,
                                                    inits: inits.clone(),
                                                    span: *span,
                                                }
                                            }
                                            None => default_expr.clone(),
                                        },
                                        _ => default_expr.clone(),
                                    };
                                (DefaultInit::Expr(stored_default), ty)
                            }
                        };
                    if let Some(ascribed) = &p.ty {
                        let asc_ty = self.type_expr_to_codegen_ty(ascribed)?;
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

        // m28b stage 2: pinned-class loci that declare bus
        // subscriptions get a synthetic `__mailbox: ptr` field.
        // The mailbox is allocated at instantiation and stored in
        // this slot so all three sites that need it can reach it
        // via self_ptr: subscribe registration (main thread),
        // synthesized thread_main's mailbox loop (pinned thread),
        // and the deferred-dissolve flush (main thread, signals
        // shutdown before pthread_join). Cooperative loci and
        // pinned loci without subscriptions don't need this.
        let has_subscribe = matches!(schedule_class, ScheduleClass::Pinned(_))
            && l.members.iter().any(|m| match m {
                LocusMember::Bus(b) => b.members.iter().any(|bm| {
                    matches!(bm, BusMember::Subscribe { .. })
                }),
                _ => false,
            });
        let mailbox_field_idx = if has_subscribe {
            let i = idx;
            llvm_field_tys.push(ptr_t.into());
            idx += 1;
            Some(i)
        } else {
            None
        };

        // m40: synthetic `__restart_count: i64` field, always
        // appended to every locus struct. Zero-initialized at
        // instantiation; bumped by the `restart(child)` recovery
        // primitive when the parent's on_failure handler asks
        // for a retry. The default cap is 2 attempts per locus
        // lifetime — past that, restart() no-ops and the
        // violation falls through to the parent's collapse path.
        // Always-present so the runtime check after on_failure
        // doesn't need to branch on whether the locus opted in.
        let i64_t_struct = self.context.i64_type();
        let restart_count_field_idx = idx;
        llvm_field_tys.push(i64_t_struct.into());
        idx += 1;
        // m41: synthetic `__quarantined: i64` flag, always
        // appended to every locus struct. Zero-initialized at
        // instantiation; set to 1 by the `quarantine(child)`
        // recovery primitive. The post-`__birth_closures`
        // dispatch in lower_locus_instantiation reads it and
        // skips `run()` if set. Drain / dissolve still fire
        // (those are cleanup, unconditional). Bus dispatch
        // gating waits on a C-runtime change with a fixed-offset
        // load — for now, quarantined loci still receive bus
        // messages but don't enter run().
        let quarantined_field_idx = idx;
        llvm_field_tys.push(i64_t_struct.into());
        idx += 1;
        // m45: synthetic __restart_in_place_pending flag —
        // distinguishes a pending `restart_in_place` re-run
        // (zero fields first) from a plain `restart` re-run
        // (state preserved). Zero-init at instantiation.
        let restart_in_place_pending_field_idx = idx;
        llvm_field_tys.push(i64_t_struct.into());
        idx += 1;
        // m42: synthetic `__parent_self: ptr` and
        // `__parent_on_failure: ptr` fields. Always present
        // (uniform struct shape — the alternative would be
        // conditional layout that complicates bus dispatch's
        // type-erased self_ptr access). Set at instantiation
        // time from `resolve_failure_route`; read by
        // `__tick_closures_wrapper` when firing tick-epoch
        // closures from a non-static call site (the bus
        // drain). Cost: 16 bytes of overhead per locus
        // instance — negligible vs. the closure-routing
        // capability they unlock.
        let ptr_field_t = self.context.ptr_type(AddressSpace::default());
        let parent_self_field_idx = idx;
        llvm_field_tys.push(ptr_field_t.into());
        idx += 1;
        let parent_on_failure_field_idx = idx;
        llvm_field_tys.push(ptr_field_t.into());
        idx += 1;
        // m43: append one i64 __duration_last_fire field per
        // duration-epoch closure on this locus (in declaration
        // order). Init at instantiation to time::monotonic()
        // so the first fire happens after `N` elapses.
        let mut duration_last_fire_field_idxs: Vec<u32> = Vec::new();
        for member in &l.members {
            if let LocusMember::Closure(c) = member {
                let is_duration = c.clauses.iter().any(|cl| {
                    matches!(cl, ClosureClause::Epoch(EpochSpec::Duration(_)))
                });
                if is_duration {
                    duration_last_fire_field_idxs.push(idx);
                    llvm_field_tys.push(i64_t_struct.into());
                    idx += 1;
                }
            }
        }

        // m46: closure accumulators. For each `sum(expr)` call
        // detected in a closure's assertion (left/right/tolerance,
        // in that order), append one struct field of `expr`'s type.
        // v0 restricts inner exprs to `self.X` reads — type comes
        // straight from the locus's params. Anything else errors
        // with a clear message at struct-decl time. Per-closure
        // persists_through clauses are also stashed here for the
        // recovery-reset gating.
        let mut accumulators_per_closure: BTreeMap<
            String,
            Vec<AccumulatorSlot>,
        > = BTreeMap::new();
        let mut persists_through_per_closure: BTreeMap<String, Vec<String>> =
            BTreeMap::new();
        for member in &l.members {
            let LocusMember::Closure(c) = member else {
                continue;
            };
            let mut accs: Vec<(AccumulatorKind, Option<Expr>)> = Vec::new();
            collect_sum_calls(&c.assertion.left, &mut accs);
            collect_sum_calls(&c.assertion.right, &mut accs);
            collect_sum_calls(&c.assertion.tolerance, &mut accs);
            let mut slots: Vec<AccumulatorSlot> = Vec::new();
            for (kind, inner_opt) in accs {
                match kind {
                    AccumulatorKind::Sum => {
                        let inner = inner_opt.expect("sum carries inner");
                        let inner_ty = infer_accumulator_inner_type(
                            &l.name.name,
                            &c.name.name,
                            &inner,
                            &fields,
                        )?;
                        let llvm_ty: inkwell::types::BasicTypeEnum =
                            self.llvm_basic_type(&inner_ty);
                        llvm_field_tys.push(llvm_ty);
                        let slot_idx = idx;
                        idx += 1;
                        slots.push(AccumulatorSlot {
                            kind: AccumulatorKind::Sum,
                            inner_expr: Some(inner),
                            ty: inner_ty.clone(),
                            inner_ty,
                            field_idx: slot_idx,
                            field_idx_2: None,
                        });
                    }
                    AccumulatorKind::Count => {
                        // One i64 slot. Inner expr = none; output = Int.
                        llvm_field_tys.push(i64_t_struct.into());
                        let slot_idx = idx;
                        idx += 1;
                        slots.push(AccumulatorSlot {
                            kind: AccumulatorKind::Count,
                            inner_expr: None,
                            ty: CodegenTy::Int,
                            inner_ty: CodegenTy::Int,
                            field_idx: slot_idx,
                            field_idx_2: None,
                        });
                    }
                    AccumulatorKind::Mean => {
                        // Two slots: running sum (inner's type) +
                        // count (i64). Output is always Float.
                        let inner = inner_opt.expect("mean carries inner");
                        let inner_ty = infer_accumulator_inner_type(
                            &l.name.name,
                            &c.name.name,
                            &inner,
                            &fields,
                        )?;
                        let llvm_inner: inkwell::types::BasicTypeEnum =
                            self.llvm_basic_type(&inner_ty);
                        llvm_field_tys.push(llvm_inner);
                        let sum_idx = idx;
                        idx += 1;
                        llvm_field_tys.push(i64_t_struct.into());
                        let count_idx = idx;
                        idx += 1;
                        slots.push(AccumulatorSlot {
                            kind: AccumulatorKind::Mean,
                            inner_expr: Some(inner),
                            ty: CodegenTy::Float,
                            inner_ty,
                            field_idx: sum_idx,
                            field_idx_2: Some(count_idx),
                        });
                    }
                }
            }
            if !slots.is_empty() {
                accumulators_per_closure
                    .insert(c.name.name.clone(), slots);
            }
            let mut persists: Vec<String> = Vec::new();
            for clause in &c.clauses {
                if let ClosureClause::PersistsThrough(events) = clause {
                    for ev in events {
                        persists.push(ev.name.clone());
                    }
                }
            }
            if !persists.is_empty() {
                persists_through_per_closure
                    .insert(c.name.name.clone(), persists);
            }
        }

        // F.22 capacity slots: walk every `capacity { ... }` block
        // on this locus and append one `__slot_<name>: ptr` field
        // per declared slot. Order is declaration order across all
        // capacity blocks (concatenated). The slot's allocator
        // pointer (lotus_pool_t* or lotus_heap_t*) lives in this
        // field; per-slot create/destroy live in lower_locus_
        // instantiation and emit_locus_arena_destroy.
        //
        // Restriction 1 (locus cell rejection) is also enforced
        // here at codegen — typecheck duplicates the check for
        // better diagnostics, but routing the rejection through
        // codegen catches the case where typecheck is bypassed
        // (e.g. internal-test paths) AND grounds the error in
        // the same CodegenTy world the rest of codegen reasons
        // in.
        let mut capacity_slots: Vec<CapacitySlotLayout> = Vec::new();
        let mut seen_slot_names: BTreeSet<String> = BTreeSet::new();
        for member in &l.members {
            let LocusMember::Capacity(cb) = member else {
                continue;
            };
            for slot in &cb.slots {
                if !seen_slot_names.insert(slot.name.name.clone()) {
                    return Err(CodegenError::Unsupported(format!(
                        "locus `{}`: duplicate capacity slot `{}`",
                        l.name.name, slot.name.name
                    )));
                }
                let elem_ty = self.type_expr_to_codegen_ty(&slot.elem_ty)?;
                if matches!(elem_ty, CodegenTy::LocusRef(_)) {
                    return Err(CodegenError::Unsupported(format!(
                        "locus `{}`: capacity slot `{}` cell type is a \
                         locus reference; F.22 restriction 1 rejects \
                         locus-typed cells — route locus membership \
                         through `accept(c: ...)` instead",
                        l.name.name, slot.name.name
                    )));
                }
                // F.22 v1.x-4: typecheck recognizes the
                // `as_parent_for ChildL` clause; the runtime
                // mechanic (passing parent's allocator to the
                // child at accept-time + skipping destroy on
                // borrowed slots) is the v1.x-4b followup.
                // Reject explicitly at codegen so users don't
                // get silent miscompilation.
                if let Some(child) = &slot.as_parent_for {
                    return Err(CodegenError::Unsupported(format!(
                        "locus `{}`: capacity slot `{} as_parent_for {}` \
                         parsed and type-checks, but the runtime mechanic \
                         (slot parent-override at accept-time) is the \
                         v1.x-4b followup. Remove the `as_parent_for` \
                         clause until v1.x-4b lands; the slot will own \
                         its own allocator.",
                        l.name.name, slot.name.name, child.name
                    )));
                }
                let slot_field_idx = idx;
                llvm_field_tys.push(ptr_t.into());
                idx += 1;
                capacity_slots.push(CapacitySlotLayout {
                    name: slot.name.name.clone(),
                    kind: slot.kind,
                    elem_ty,
                    struct_field_idx: slot_field_idx,
                });
            }
        }
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
                accumulators_per_closure,
                persists_through_per_closure,
                birth_closures_fn: None,
                dissolve_closures_fn: None,
                tick_closures_fn: None,
                tick_wrapper_fn: None,
                duration_closures_fn: None,
                duration_wrapper_fn: None,
                duration_last_fire_field_idxs,
                explicit_closures_fn: None,
                failure_handler: None,
                children_field_idx,
                child_count_field_idx,
                arena_field_idx,
                restart_count_field_idx,
                quarantined_field_idx,
                restart_in_place_pending_field_idx,
                parent_self_field_idx,
                parent_on_failure_field_idx,
                mailbox_field_idx,
                projection_class,
                schedule_class,
                capacity_slots,
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
        if !l.generics.is_empty() {
            // m63: see declare_locus_struct — generic templates
            // skip method declaration too.
            return Ok(());
        }
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let void_t = self.context.void_type();
        let mut methods: BTreeMap<&'static str, FunctionValue<'ctx>> =
            BTreeMap::new();
        let mut accept_param: Option<(String, String)> = None;
        let mut user_methods: BTreeMap<String, FunctionValue<'ctx>> =
            BTreeMap::new();
        let mut subscriptions: Vec<(String, String, String)> = Vec::new();
        let mut closures: Vec<(String, ClosureAssertion, EpochSpec)> =
            Vec::new();
        let mut failure_handler: Option<(String, FunctionValue<'ctx>)> = None;

        // Pre-collect bus-handler method names so we can reject
        // defaults on them: bus dispatch is a fixed (self, payload)
        // C-runtime call that can't materialize default values for
        // extra params. Defaults on non-handler methods (called via
        // `self.method(...)`) work fine — m33 lifts that gate.
        let bus_handler_names: std::collections::BTreeSet<String> = l
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Bus(bb) => Some(bb.members.iter().filter_map(|bm| {
                    match bm {
                        BusMember::Subscribe { handler, .. } => Some(handler.name.clone()),
                        _ => None,
                    }
                })),
                _ => None,
            })
            .flatten()
            .collect();

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
                            let child_ty = self.type_expr_to_codegen_ty(&p.ty)?;
                            let child_locus = match &child_ty {
                                CodegenTy::LocusRef(name) => name.clone(),
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
                    //
                    // m60: each subscription also carries the
                    // payload type's name so registration can
                    // look up the matching __deserialize_T fn in
                    // cx.serializers. The `of type T` clause is
                    // optional in the AST but every example
                    // declares it; if it's missing we fall back
                    // to extracting the type from the handler's
                    // first param signature later in pass A2.
                    for bm in &bb.members {
                        match bm {
                            BusMember::Subscribe { subject, handler, ty, .. } => {
                                let payload_type_name = ty
                                    .as_ref()
                                    .and_then(|t| {
                                        self.type_expr_to_codegen_ty(t).ok()
                                    })
                                    .and_then(|lt| match lt {
                                        CodegenTy::TypeRef(n) => Some(n),
                                        CodegenTy::Enum(n) => Some(n),
                                        _ => None,
                                    })
                                    .ok_or_else(|| {
                                        CodegenError::Unsupported(format!(
                                            "locus `{}` subscribe `{}`: \
                                             missing or unsupported \
                                             payload type (m60 requires \
                                             a TypeRef or has-payload \
                                             Enum)",
                                            l.name.name, subject
                                        ))
                                    })?;
                                subscriptions.push((
                                    subject.clone(),
                                    handler.name.clone(),
                                    payload_type_name,
                                ));
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
                    let is_bus_handler =
                        bus_handler_names.contains(&fd.name.name);
                    let mut seen_default = false;
                    for p in &fd.params {
                        if p.default.is_some() {
                            if is_bus_handler {
                                return Err(CodegenError::Unsupported(format!(
                                    "locus `{}` method `{}`: bus-subscribed \
                                     handlers can't have default param values \
                                     (bus dispatch is fixed-arity self+payload)",
                                    l.name.name, fd.name.name
                                )));
                            }
                            seen_default = true;
                        } else if seen_default {
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` method `{}`: required param `{}` \
                                 follows a defaulted param; defaults must form \
                                 a suffix",
                                l.name.name, fd.name.name, p.name.name
                            )));
                        }
                        let lt = self.type_expr_to_codegen_ty(&p.ty)?;
                        llvm_param_tys.push(self.llvm_basic_type(&lt).into());
                    }
                    let fn_ty = match &fd.ret {
                        None => void_t.fn_type(&llvm_param_tys, false),
                        Some(t) => {
                            let rt = self.type_expr_to_codegen_ty(t)?;
                            match rt {
                                CodegenTy::Int | CodegenTy::Duration => self
                                    .context
                                    .i64_type()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::Float => self
                                    .context
                                    .f64_type()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::Decimal => self
                                    .context
                                    .i128_type()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::Bool => self
                                    .context
                                    .bool_type()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::Enum(name) => {
                                    if self
                                        .user_enums
                                        .get(name.as_str())
                                        .map(|i| i.has_payload)
                                        .unwrap_or(false)
                                    {
                                        self.context
                                            .ptr_type(AddressSpace::default())
                                            .fn_type(&llvm_param_tys, false)
                                    } else {
                                        self.context
                                            .i32_type()
                                            .fn_type(&llvm_param_tys, false)
                                    }
                                }
                                CodegenTy::String
                                | CodegenTy::Bytes
                                | CodegenTy::Time
                                | CodegenTy::LocusRef(_)
                                | CodegenTy::TypeRef(_)
                                | CodegenTy::Array(_, _)
                                | CodegenTy::Tuple(_)
                                | CodegenTy::FnPtr { .. }
                                | CodegenTy::Interface(_)
                                | CodegenTy::Cell(_, _) => self
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
                    // m39 + m42 + m43 + m44: all five closure
                    // epochs now lower. Default (no epoch
                    // clause) = Dissolve, matching pre-m39
                    // semantics.
                    let mut epoch = EpochSpec::Dissolve;
                    for clause in &c.clauses {
                        match clause {
                            ClosureClause::Epoch(spec) => {
                                epoch = spec.clone();
                            }
                            ClosureClause::PersistsThrough(_)
                            | ClosureClause::ResetsOn(_) => {
                                // Recovery-event hooks; relevant
                                // when accumulators land. No effect
                                // on the v0 single-shot path.
                            }
                        }
                    }
                    closures.push((
                        c.name.name.clone(),
                        c.assertion.clone(),
                        epoch,
                    ));
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
                    let child_ty = self.type_expr_to_codegen_ty(&fd.params[0].ty)?;
                    let child_locus_name = match &child_ty {
                        CodegenTy::LocusRef(n) => n.clone(),
                        other => {
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` on_failure first param must be \
                                 a locus type; got {:?}",
                                l.name.name, other
                            )));
                        }
                    };
                    let err_ty = self.type_expr_to_codegen_ty(&fd.params[1].ty)?;
                    if err_ty != CodegenTy::TypeRef("ClosureViolation".into())
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
                    // m54: mode params accept defaults under the
                    // same suffix-only rule as locus fn methods.
                    // lower_self_method_call already handles the
                    // call-site fill-in (it dispatches uniformly
                    // on Fn / Mode via the program-walk that
                    // returns a MethodSig), so the only thing
                    // this declare-side block needs is the
                    // ordering check.
                    let mut seen_default = false;
                    for p in &md.params {
                        if p.default.is_some() {
                            seen_default = true;
                        } else if seen_default {
                            return Err(CodegenError::Unsupported(format!(
                                "locus `{}` mode `{}`: required param \
                                 `{}` follows a defaulted param; defaults \
                                 must form a suffix",
                                l.name.name, mode_name, p.name.name
                            )));
                        }
                        let lt = self.type_expr_to_codegen_ty(&p.ty)?;
                        llvm_param_tys.push(self.llvm_basic_type(&lt).into());
                    }
                    let fn_ty = match &md.ret {
                        None => void_t.fn_type(&llvm_param_tys, false),
                        Some(t) => {
                            let rt = self.type_expr_to_codegen_ty(t)?;
                            match rt {
                                CodegenTy::Int | CodegenTy::Duration => self
                                    .context
                                    .i64_type()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::Float => self
                                    .context
                                    .f64_type()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::Decimal => self
                                    .context
                                    .i128_type()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::Bool => self
                                    .context
                                    .bool_type()
                                    .fn_type(&llvm_param_tys, false),
                                CodegenTy::Enum(name) => {
                                    if self
                                        .user_enums
                                        .get(name.as_str())
                                        .map(|i| i.has_payload)
                                        .unwrap_or(false)
                                    {
                                        self.context
                                            .ptr_type(AddressSpace::default())
                                            .fn_type(&llvm_param_tys, false)
                                    } else {
                                        self.context
                                            .i32_type()
                                            .fn_type(&llvm_param_tys, false)
                                    }
                                }
                                CodegenTy::String
                                | CodegenTy::Bytes
                                | CodegenTy::Time
                                | CodegenTy::LocusRef(_)
                                | CodegenTy::TypeRef(_)
                                | CodegenTy::Array(_, _)
                                | CodegenTy::Tuple(_)
                                | CodegenTy::FnPtr { .. }
                                | CodegenTy::Interface(_)
                                | CodegenTy::Cell(_, _) => self
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
                LocusMember::Capacity(_) => {
                    // F.22 slots have no method-decl phase — the
                    // struct layout, slot init, and slot destroy
                    // are already wired in declare_locus_struct,
                    // lower_locus_instantiation, and
                    // emit_locus_arena_destroy. The user-facing
                    // `self.X.acquire()` dispatch lands in #17.
                }
            }
        }

        // m39: declare per-epoch synthetic eval fns. Each has the
        // same signature — `(self_ptr, parent_self_or_null,
        // on_failure_fn_or_null)` — but lowers a different subset
        // of the closure list. Call sites pass the parent's self +
        // on_failure fn ptr if the parent has a matching handler,
        // else null/null. Bodies lowered in pass C.
        let has_birth = closures
            .iter()
            .any(|(_, _, ep)| matches!(ep, EpochSpec::Birth));
        let has_dissolve = closures
            .iter()
            .any(|(_, _, ep)| matches!(ep, EpochSpec::Dissolve));
        let has_tick = closures
            .iter()
            .any(|(_, _, ep)| matches!(ep, EpochSpec::Tick));
        let has_duration = closures
            .iter()
            .any(|(_, _, ep)| matches!(ep, EpochSpec::Duration(_)));
        let has_explicit = closures
            .iter()
            .any(|(_, _, ep)| matches!(ep, EpochSpec::Explicit));
        let make_eval_fn = |name: &str| {
            let fn_ty = void_t.fn_type(
                &[ptr_t.into(), ptr_t.into(), ptr_t.into()],
                false,
            );
            self.module.add_function(name, fn_ty, None)
        };
        let birth_closures_fn = if has_birth {
            Some(make_eval_fn(&format!(
                "{}.__birth_closures",
                l.name.name
            )))
        } else {
            None
        };
        let dissolve_closures_fn = if has_dissolve {
            Some(make_eval_fn(&format!(
                "{}.__dissolve_closures",
                l.name.name
            )))
        } else {
            None
        };
        // m42: tick_closures_fn has the standard 3-arg shape
        // (self, parent, on_failure); tick_wrapper_fn is a
        // 1-arg adapter `(self) -> void` that loads parent +
        // on_failure from the struct's __parent_self /
        // __parent_on_failure fields and tail-calls the
        // 3-arg fn. The wrapper is what bus-handler thunks
        // call (they only have self in scope).
        let tick_closures_fn = if has_tick {
            Some(make_eval_fn(&format!(
                "{}.__tick_closures",
                l.name.name
            )))
        } else {
            None
        };
        let tick_wrapper_fn = if has_tick {
            let wrapper_ty = void_t.fn_type(&[ptr_t.into()], false);
            Some(self.module.add_function(
                &format!("{}.__tick_closures_wrapper", l.name.name),
                wrapper_ty,
                None,
            ))
        } else {
            None
        };
        // m43: __duration_closures has the standard 3-arg
        // shape. Bodies do their own gating on
        // monotonic-elapsed-since-last-fire per closure;
        // shared with tick at the lifecycle call sites
        // (post-handler, post-run).
        let duration_closures_fn = if has_duration {
            Some(make_eval_fn(&format!(
                "{}.__duration_closures",
                l.name.name
            )))
        } else {
            None
        };
        // m43-followup: 1-arg wrapper adapter, same shape as
        // tick_wrapper_fn. Needed for the pinned post-run() path,
        // where the calling context is the pinned thread (no
        // `current_self`), so the 3-arg fn's parent args can't be
        // resolved at the call site — they have to come from the
        // struct fields baked at instantiation time.
        let duration_wrapper_fn = if has_duration {
            let wrapper_ty = void_t.fn_type(&[ptr_t.into()], false);
            Some(self.module.add_function(
                &format!("{}.__duration_closures_wrapper", l.name.name),
                wrapper_ty,
                None,
            ))
        } else {
            None
        };
        // m44: __explicit_closures has the same 3-arg shape.
        // Called only by the `check_closures();` builtin —
        // user-triggered audit at a chosen checkpoint.
        let explicit_closures_fn = if has_explicit {
            Some(make_eval_fn(&format!(
                "{}.__explicit_closures",
                l.name.name
            )))
        } else {
            None
        };
        // m42: tick-call placement. An earlier draft wrapped
        // each subscribed handler with a post-call thunk;
        // that broke order because the handler's own tail
        // `bus_queue_drain` (m26) recursively processed
        // queued cells before the thunk's tick step ran.
        // Final design inlines the tick call into the
        // subscribed user-fn body just before its tail
        // drain — see Pass C's user-fn body lowering.

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
        info.birth_closures_fn = birth_closures_fn;
        info.dissolve_closures_fn = dissolve_closures_fn;
        info.tick_closures_fn = tick_closures_fn;
        info.tick_wrapper_fn = tick_wrapper_fn;
        info.duration_closures_fn = duration_closures_fn;
        info.duration_wrapper_fn = duration_wrapper_fn;
        info.explicit_closures_fn = explicit_closures_fn;
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
        if !l.generics.is_empty() {
            // m63: generic templates have no method bodies to
            // lower until pinned by an instantiation site.
            return Ok(());
        }
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
                        (slot, CodegenTy::LocusRef(child_locus.clone())),
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
                (child_slot, CodegenTy::LocusRef(child_locus_name.clone())),
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
                (err_slot, CodegenTy::TypeRef("ClosureViolation".into())),
            );

            let end = self.lower_block(&failure_decl.body, &mut scope)?;
            if end == BlockEnd::Open {
                // m46-vocab follow-up: on_failure runs
                // synchronously inside an outer substrate cell
                // (the closure-eval body that detected the
                // violation). Don't drain the bus queue here —
                // the outer cell owns that. A recursive drain
                // would pull queued cells mid-tick, advancing
                // accumulator state across "this fire's"
                // boundary. See `flush_dissolve_frame_kind`.
                self.flush_dissolve_frame_kind(false)?;
                self.builder
                    .build_return(None)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            } else {
                let _ = self.deferred_dissolves.pop();
            }

            self.current_fn = None;
            self.current_self = None;
        }

        // Synthetic __birth_closures + __dissolve_closures fns:
        // each evaluates every closure assertion in its epoch in
        // declaration order. Each assertion computes |left -
        // right| <= tolerance; on fail, write a ClosureViolation
        // report to stderr (fd 2 via dprintf) and exit non-zero,
        // OR route to the parent's on_failure if the call site
        // passed a non-null handler. Pass paths flow through
        // silently. Same body shape per epoch — only the closure
        // subset differs — so we use a small helper closure.
        for epoch in [
            EpochSpec::Birth,
            EpochSpec::Dissolve,
            EpochSpec::Tick,
            EpochSpec::Explicit,
        ]
        .iter()
        {
            let fn_slot = match epoch {
                EpochSpec::Birth => info.birth_closures_fn,
                EpochSpec::Dissolve => info.dissolve_closures_fn,
                EpochSpec::Tick => info.tick_closures_fn,
                EpochSpec::Explicit => info.explicit_closures_fn,
                _ => None,
            };
            let func = match fn_slot {
                Some(f) => f,
                None => continue,
            };
            let entry = self.context.append_basic_block(func, "entry");
            self.builder.position_at_end(entry);
            let self_ptr = func
                .get_nth_param(0)
                .expect("self_ptr param")
                .into_pointer_value();
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

            for (cname, assertion, c_epoch) in &info.closures {
                if c_epoch != epoch {
                    continue;
                }
                self.lower_closure_check(
                    &l.name.name,
                    cname,
                    assertion,
                    parent_self_arg,
                    parent_handler_arg,
                    c_epoch.clone(),
                )?;
            }

            // m42 + m44: tick AND explicit fire inside / from
            // contexts where re-entering the bus queue would
            // be wrong. Tick fires from the cooperative drain
            // (recursive drain would pull every remaining cell
            // into one tick's call stack). Explicit fires at
            // a user-chosen checkpoint inside the locus's
            // body — the surrounding body's normal
            // flush_dissolve_frame at scope exit will handle
            // any drain at the right time. So both pop the
            // frame manually and skip the drain. For
            // Birth + Dissolve the drain is historically OK
            // (those run outside the drain context).
            if matches!(epoch, EpochSpec::Tick | EpochSpec::Explicit) {
                let _ = self.deferred_dissolves.pop();
            } else {
                self.flush_dissolve_frame()?;
            }
            self.builder
                .build_return(None)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.current_fn = None;
            self.current_self = None;
        }

        // m43: __duration_closures body. Each duration-epoch
        // closure gates on monotonic-elapsed-since-last-fire
        // before evaluating the assertion. last_fire is
        // updated to monotonic-now BEFORE the assertion runs
        // so a routed-and-absorbed violation in on_failure
        // doesn't reset the interval clock. Per-closure last-
        // fire fields parallel info.duration_last_fire_field_idxs
        // in declaration order.
        if let Some(duration_fn) = info.duration_closures_fn {
            let entry =
                self.context.append_basic_block(duration_fn, "entry");
            self.builder.position_at_end(entry);
            let self_ptr = duration_fn
                .get_nth_param(0)
                .expect("self_ptr param")
                .into_pointer_value();
            let parent_self_arg = duration_fn
                .get_nth_param(1)
                .expect("parent_self_or_null param")
                .into_pointer_value();
            let parent_handler_arg = duration_fn
                .get_nth_param(2)
                .expect("on_failure_or_null param")
                .into_pointer_value();
            self.current_fn = Some(duration_fn);
            self.current_user_fn_ret = None;
            self.current_self = Some(SelfCx {
                locus_name: l.name.name.clone(),
                struct_ty: info.struct_ty,
                self_ptr,
                fields: info.fields.clone(),
            });
            self.loops.clear();
            self.push_dissolve_frame();

            let i64_t = self.context.i64_type();
            let mut duration_idx: usize = 0;
            // Snapshot info.closures to a local (we'll be
            // emitting nested LLVM that may otherwise alias
            // through self.user_loci while we work).
            let closures_snapshot = info.closures.clone();
            for (cname, assertion, c_epoch) in &closures_snapshot {
                let duration_expr = match c_epoch {
                    EpochSpec::Duration(e) => e.clone(),
                    _ => continue,
                };
                let last_field_idx = info
                    .duration_last_fire_field_idxs[duration_idx];
                duration_idx += 1;

                // last = load __duration_last_fire_<i>
                let last_slot = self
                    .builder
                    .build_struct_gep(
                        info.struct_ty,
                        self_ptr,
                        last_field_idx,
                        &format!(
                            "{}.duration[{}].last.ptr",
                            l.name.name, cname
                        ),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let last = self
                    .builder
                    .build_load(i64_t, last_slot, "duration.last")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                // now = time::monotonic() (i64 ns)
                let (now_v, _) =
                    self.lower_time_monotonic(&[])?;
                let now = now_v.into_int_value();
                // Evaluate the duration expression in self-scope
                // — same approach as closure assertions, so it
                // can reference self.X (e.g.
                // `duration(self.poll_interval)`).
                let scope = Scope::default();
                let (dur_v, _) =
                    self.lower_expr(&duration_expr, &scope)?;
                let dur_n = dur_v.into_int_value();
                let elapsed = self
                    .builder
                    .build_int_sub(now, last, "duration.elapsed")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let should_fire = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::SGE,
                        elapsed,
                        dur_n,
                        "duration.should_fire",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let fire_bb = self.context.append_basic_block(
                    duration_fn,
                    &format!("duration.{}.fire", cname),
                );
                let skip_bb = self.context.append_basic_block(
                    duration_fn,
                    &format!("duration.{}.skip", cname),
                );
                self.builder
                    .build_conditional_branch(should_fire, fire_bb, skip_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                // fire_bb: store now -> last_fire, then run
                // the assertion check (which routes to
                // on_failure on violation).
                self.builder.position_at_end(fire_bb);
                self.builder
                    .build_store(last_slot, now)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.lower_closure_check(
                    &l.name.name,
                    cname,
                    assertion,
                    parent_self_arg,
                    parent_handler_arg,
                    c_epoch.clone(),
                )?;
                self.builder
                    .build_unconditional_branch(skip_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

                self.builder.position_at_end(skip_bb);
            }

            // Same flush-skip rationale as tick: duration
            // fires inside the cooperative drain loop, so we
            // can't recursively re-enter the queue here.
            let _ = self.deferred_dissolves.pop();
            self.builder
                .build_return(None)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.current_fn = None;
            self.current_self = None;
        }

        // m42: tick_wrapper body. The bus drain loop calls
        // tick_wrapper(self) after each handler returns; the
        // wrapper loads the parent fields baked onto the struct
        // at instantiation time and forwards to the 3-arg
        // __tick_closures fn. This indirection lets us route
        // tick violations through the same parent on_failure
        // handler the birth/dissolve epochs use, without
        // changing the bus drain loop's signature.
        // m43-followup: duration uses the same shape so the
        // pinned post-run path has a 1-arg call site that can
        // route violations off-main-thread.
        let wrapper_pairs = [
            (info.tick_wrapper_fn, info.tick_closures_fn, "tick"),
            (
                info.duration_wrapper_fn,
                info.duration_closures_fn,
                "duration",
            ),
        ];
        for (wrapper_opt, eval_opt, tag) in wrapper_pairs {
            let (Some(wrapper_fn), Some(eval_fn)) = (wrapper_opt, eval_opt)
            else {
                continue;
            };
            let entry = self.context.append_basic_block(wrapper_fn, "entry");
            self.builder.position_at_end(entry);
            let self_ptr = wrapper_fn
                .get_nth_param(0)
                .expect("self_ptr param")
                .into_pointer_value();
            let ptr_t = self.context.ptr_type(AddressSpace::default());
            let parent_self_slot = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    info.parent_self_field_idx,
                    "parent_self.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let parent_self = self
                .builder
                .build_load(ptr_t, parent_self_slot, "parent_self")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_pointer_value();
            let parent_handler_slot = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    info.parent_on_failure_field_idx,
                    "parent_handler.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let parent_handler = self
                .builder
                .build_load(
                    ptr_t,
                    parent_handler_slot,
                    "parent_handler",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_pointer_value();
            self.builder
                .build_call(
                    eval_fn,
                    &[
                        self_ptr.into(),
                        parent_self.into(),
                        parent_handler.into(),
                    ],
                    &format!("{}.closures.call", tag),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_return(None)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
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
                    Some(t) => Some(self.type_expr_to_codegen_ty(t)?),
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
                    let lt = self.type_expr_to_codegen_ty(&p.ty)?;
                    let alloca = self.alloca_for(&lt, &p.name.name)?;
                    let v = func
                        .get_nth_param((i + 1) as u32)
                        .expect("locus method arg index in range");
                    self.builder
                        .build_store(alloca, v)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    scope.locals.insert(p.name.name.clone(), (alloca, lt));
                }

                // m42: gate subscribed bus-handler bodies on the
                // __quarantined flag at entry. m41b (m45-followup-2
                // form) nulls subjects in the C-runtime entries
                // vec so future publishes skip a quarantined
                // subscriber, but cells enqueued before quarantine
                // remain in the queue and would otherwise still
                // fire. This entry gate matches the interpreter's
                // `delivery.subscription.locus.quarantined`
                // check in dispatch_bus, so already-queued
                // deliveries observe the stop-trying signal.
                let is_subscribed_handler = info
                    .subscriptions
                    .iter()
                    .any(|(_, h, _)| h == &fd.name.name);
                if is_subscribed_handler {
                    let i64_t = self.context.i64_type();
                    let q_slot = self
                        .builder
                        .build_struct_gep(
                            info.struct_ty,
                            self_ptr,
                            info.quarantined_field_idx,
                            "handler.quarantined.ptr",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let q_val = self
                        .builder
                        .build_load(i64_t, q_slot, "handler.quarantined")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_int_value();
                    let is_q = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::NE,
                            q_val,
                            i64_t.const_int(0, false),
                            "handler.is_quarantined",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let skip_bb = self
                        .context
                        .append_basic_block(func, "handler.skip");
                    let body_bb = self
                        .context
                        .append_basic_block(func, "handler.body");
                    self.builder
                        .build_conditional_branch(is_q, skip_bb, body_bb)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder.position_at_end(skip_bb);
                    self.builder
                        .build_return(None)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder.position_at_end(body_bb);
                }

                let end = self.lower_block(&fd.body, &mut scope)?;
                if end == BlockEnd::Open {
                    // m42: if this user fn is a registered bus
                    // handler AND the locus has tick closures,
                    // fire __tick_closures HERE — after the
                    // body's effects but BEFORE the tail
                    // bus_queue_drain. The tail drain (m26)
                    // would otherwise recursively process the
                    // next queued cell first, and tick would
                    // see the next cell's state instead of
                    // this handler's. Tick is the natural
                    // "between substrate cells" point and
                    // belongs inline with the cell's body
                    // termination, ahead of any cooperative
                    // yield.
                    let is_subscribed_handler = info
                        .subscriptions
                        .iter()
                        .any(|(_, h, _)| h == &fd.name.name);
                    if is_subscribed_handler {
                        // Load parent_self and parent_handler
                        // once; both tick and duration fns
                        // need them and the loads are pure
                        // GEP+load.
                        let parent_self_slot = self
                            .builder
                            .build_struct_gep(
                                info.struct_ty,
                                self_ptr,
                                info.parent_self_field_idx,
                                "epoch.parent_self.ptr",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        let parent_self_v = self
                            .builder
                            .build_load(
                                self.context.ptr_type(AddressSpace::default()),
                                parent_self_slot,
                                "epoch.parent_self",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                            .into_pointer_value();
                        let parent_handler_slot = self
                            .builder
                            .build_struct_gep(
                                info.struct_ty,
                                self_ptr,
                                info.parent_on_failure_field_idx,
                                "epoch.parent_handler.ptr",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        let parent_handler_v = self
                            .builder
                            .build_load(
                                self.context.ptr_type(AddressSpace::default()),
                                parent_handler_slot,
                                "epoch.parent_handler",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                            .into_pointer_value();
                        if let Some(tick_fn) = info.tick_closures_fn {
                            self.builder
                                .build_call(
                                    tick_fn,
                                    &[
                                        self_ptr.into(),
                                        parent_self_v.into(),
                                        parent_handler_v.into(),
                                    ],
                                    &format!(
                                        "{}.{}.tick.post_handler.call",
                                        l.name.name, fd.name.name
                                    ),
                                )
                                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        }
                        // m43: duration shares the cell-boundary
                        // cadence with tick — same call site,
                        // each closure self-gates on elapsed
                        // time inside the synthesized fn.
                        if let Some(duration_fn) =
                            info.duration_closures_fn
                        {
                            self.builder
                                .build_call(
                                    duration_fn,
                                    &[
                                        self_ptr.into(),
                                        parent_self_v.into(),
                                        parent_handler_v.into(),
                                    ],
                                    &format!(
                                        "{}.{}.duration.post_handler.call",
                                        l.name.name, fd.name.name
                                    ),
                                )
                                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        }
                    }
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
                    Some(t) => Some(self.type_expr_to_codegen_ty(t)?),
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
                    let lt = self.type_expr_to_codegen_ty(&p.ty)?;
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
            // m62: generic templates declare nothing directly.
            // Per-instantiation specialized fns get declared
            // on-demand from `lower_call_expr` once arg types
            // are known and inference can pin the type args.
            return Ok(());
        }
        let mut param_tys = Vec::with_capacity(f.params.len());
        let ptr_t_for_caller_arena =
            self.context.ptr_type(AddressSpace::default());
        let mut llvm_param_tys: Vec<inkwell::types::BasicMetadataTypeEnum> =
            Vec::with_capacity(f.params.len() + 1);
        // m49: implicit `__caller_arena: ptr` first param. Caller
        // passes their `current_arena_ptr()`; callee opens a
        // subregion of it at body entry. `params` / `defaults` /
        // user-visible arity stays unchanged — this is purely an
        // ABI-level implicit prefix.
        llvm_param_tys.push(ptr_t_for_caller_arena.into());
        let mut defaults: Vec<Option<Expr>> = Vec::with_capacity(f.params.len());
        let mut seen_default = false;
        for p in &f.params {
            if p.default.is_some() {
                seen_default = true;
            } else if seen_default {
                // Suffix-only rule: a non-defaulted param can't
                // follow a defaulted one. Otherwise the
                // omit-trailing-args ABI breaks down — the caller
                // can't tell which positional slot they're filling.
                return Err(CodegenError::Unsupported(format!(
                    "fn `{}`: required param `{}` follows a defaulted \
                     param; defaults must form a suffix",
                    f.name.name, p.name.name
                )));
            }
            let lt = self.type_expr_to_codegen_ty(&p.ty)?;
            llvm_param_tys.push(self.llvm_basic_type(&lt).into());
            param_tys.push(lt);
            defaults.push(p.default.clone());
        }
        let ret_ty = match &f.ret {
            Some(t) => Some(self.type_expr_to_codegen_ty(t)?),
            None => None,
        };
        let fn_ty = match &ret_ty {
            Some(CodegenTy::Int) | Some(CodegenTy::Duration) => self
                .context
                .i64_type()
                .fn_type(&llvm_param_tys, false),
            Some(CodegenTy::Float) => {
                self.context.f64_type().fn_type(&llvm_param_tys, false)
            }
            Some(CodegenTy::Decimal) => {
                self.context.i128_type().fn_type(&llvm_param_tys, false)
            }
            Some(CodegenTy::Bool) => {
                self.context.bool_type().fn_type(&llvm_param_tys, false)
            }
            Some(CodegenTy::Enum(name)) => {
                if self
                    .user_enums
                    .get(name.as_str())
                    .map(|i| i.has_payload)
                    .unwrap_or(false)
                {
                    self.context
                        .ptr_type(AddressSpace::default())
                        .fn_type(&llvm_param_tys, false)
                } else {
                    self.context.i32_type().fn_type(&llvm_param_tys, false)
                }
            }
            Some(CodegenTy::String)
            | Some(CodegenTy::Bytes)
            | Some(CodegenTy::Time)
            | Some(CodegenTy::LocusRef(_))
            | Some(CodegenTy::TypeRef(_))
            | Some(CodegenTy::Array(_, _))
            | Some(CodegenTy::Tuple(_))
            | Some(CodegenTy::FnPtr { .. })
            | Some(CodegenTy::Interface(_))
            | Some(CodegenTy::Cell(_, _)) => self
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
                defaults,
                ret: ret_ty,
            },
        );
        Ok(())
    }

    /// Lower a user fn's body. Each declared param is materialized
    /// as an alloca in the entry block so reads through `Ident`
    /// see the value-stored slot exactly the way `let`-bindings do.
    ///
    /// m49: every non-main free fn has an implicit `__caller_arena:
    /// ptr` first param at the LLVM ABI. The body opens a subregion
    /// of that arena so its allocations route through a per-call
    /// region that gets wholesale-freed at fn return. Heap-typed
    /// return values are deep-copied into `__caller_arena` before
    /// the subregion is destroyed. All `return` statements branch
    /// to a unified `fn.exit` epilogue block to avoid duplicating
    /// the destroy + copy at every return site.
    fn lower_user_fn_body(&mut self, f: &FnDecl) -> Result<(), CodegenError> {
        if !f.generics.is_empty() {
            // m62: generic templates have no body to lower until
            // call sites pin the type args. lower_call_expr
            // synthesizes + lowers per-instantiation bodies on-
            // demand.
            return Ok(());
        }
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

        let ptr_t = self.context.ptr_type(AddressSpace::default());

        // m49: implicit __caller_arena param materializes into a
        // local alloca first, before any user code runs. The
        // alloca is what the deep-copy epilogue reads back —
        // reading the LLVM param directly would also work since
        // params are SSA values that live for the whole fn, but
        // going through an alloca keeps the IR shape uniform with
        // every other binding the body sees.
        let caller_arena_alloca = self
            .builder
            .build_alloca(ptr_t, "__caller_arena.slot")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let caller_arena_param = func
            .get_nth_param(0)
            .expect("implicit __caller_arena param at slot 0");
        self.builder
            .build_store(caller_arena_alloca, caller_arena_param)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // m49: open a subregion of caller_arena. The body's
        // allocations route through this. Stored in an alloca so
        // `current_arena_ptr` can re-load it when subregion-only
        // routing is required (the alloca survives across blocks).
        let subregion_create = self
            .module
            .get_function("lotus_arena_create_subregion")
            .expect("lotus_arena_create_subregion declared");
        let fn_arena_ptr = self
            .builder
            .build_call(
                subregion_create,
                &[caller_arena_param.into()],
                "fn.arena.create",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("subregion_create returns ptr");
        let fn_arena_alloca = self
            .builder
            .build_alloca(ptr_t, "fn.arena.slot")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(fn_arena_alloca, fn_arena_ptr)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // m49: ret-value alloca (only for typed returns) + unified
        // fn.exit block. Every `return e;` stores into ret_alloca
        // and br's to fn.exit; void returns just br. The exit
        // epilogue runs the deep-copy + arena_destroy + ret.
        let ret_alloca = match &sig.ret {
            None => None,
            Some(ret_ty) => {
                let alloca = self.alloca_for(ret_ty, "fn.ret.slot")?;
                Some(alloca)
            }
        };
        let exit_bb = self.context.append_basic_block(func, "fn.exit");

        self.current_user_fn_caller_arena = Some(caller_arena_alloca);
        self.current_user_fn_arena = Some(fn_arena_alloca);
        self.current_user_fn_exit_bb = Some(exit_bb);
        self.current_user_fn_ret_alloca = ret_alloca;

        let mut scope = Scope::default();
        for (i, p) in f.params.iter().enumerate() {
            let lt = sig.params[i].clone();
            let alloca = self.alloca_for(&lt, &p.name.name)?;
            // Declared params are at LLVM slot i+1 (slot 0 is the
            // implicit __caller_arena).
            let v = func
                .get_nth_param(i as u32 + 1)
                .expect("param index in range");
            self.builder
                .build_store(alloca, v)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            scope.locals.insert(p.name.name.clone(), (alloca, lt));
        }

        let end = self.lower_block(&f.body, &mut scope)?;
        if end == BlockEnd::Open {
            match &sig.ret {
                None => {
                    // Void fall-through: br to exit; epilogue
                    // flushes deferred-dissolves + builds
                    // `ret void`. m53: flush moved into exit so
                    // both fall-through and explicit-return
                    // paths share a single drain + dissolve
                    // sequence.
                    self.builder
                        .build_unconditional_branch(exit_bb)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Some(_) => {
                    return Err(CodegenError::Unsupported(format!(
                        "fn `{}` falls through without returning a value",
                        f.name.name
                    )));
                }
            }
        }
        // For terminated bodies (a `return` or other terminator
        // inside the body), the Stmt::Return path already br'd to
        // exit_bb. The frame stays on the deferred_dissolves
        // stack and gets flushed by emit_fn_exit_epilogue below.

        // m49: emit the fn.exit epilogue. Position there, flush
        // the deferred-dissolves frame (drains the bus queue +
        // dissolves any loci bound in the fn body — m53), do the
        // deep-copy of ret_alloca → caller_arena (only for heap
        // types), destroy the subregion, build_return.
        self.emit_fn_exit_epilogue(&sig)?;

        self.current_user_fn_caller_arena = None;
        self.current_user_fn_arena = None;
        self.current_user_fn_exit_bb = None;
        self.current_user_fn_ret_alloca = None;
        self.current_fn = None;
        self.current_user_fn_ret = None;
        Ok(())
    }

    /// m49: emit the unified return epilogue at `fn.exit` for the
    /// fn currently being lowered. Positioned at the exit block
    /// on entry; on return, the block is closed by `build_return`.
    /// For typed-return fns, `ret_alloca` is read, deep-copied
    /// into `caller_arena`, then returned. For void fns, just
    /// destroy the subregion and `ret void`.
    fn emit_fn_exit_epilogue(
        &mut self,
        sig: &FnSig<'ctx>,
    ) -> Result<(), CodegenError> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let exit_bb = self
            .current_user_fn_exit_bb
            .expect("exit_bb set during fn body lowering");
        self.builder.position_at_end(exit_bb);

        let caller_arena_alloca = self
            .current_user_fn_caller_arena
            .expect("caller_arena_alloca set during fn body lowering");
        let fn_arena_alloca = self
            .current_user_fn_arena
            .expect("fn_arena_alloca set during fn body lowering");

        // Deep-copy BEFORE flush_dissolve_frame. The return value
        // can point into a let-bound sub-locus's arena (e.g. when
        // a body returns `someLocus.method(...)` directly — the
        // method allocates its concat result in its own arena);
        // flushing first frees those arenas, leaving ret_alloca
        // dangling for the str_clone read. The copy lands in
        // caller_arena, which is the parent caller's region and
        // stays alive across this fn's flush + arena_destroy
        // below. flush_dissolve_frame is a downstream operation:
        // by the time we reach it, the caller-visible value is
        // already safe in caller_arena, so dissolves can free
        // sub-locus arenas without touching what we just copied.
        let copied_ret: Option<BasicValueEnum<'ctx>> = match &sig.ret {
            None => None,
            Some(ret_ty) => {
                let ret_alloca = self
                    .current_user_fn_ret_alloca
                    .expect("ret_alloca set when ret type is Some");
                let llvm_ret_ty = self.llvm_basic_type(ret_ty);
                let raw_ret = self
                    .builder
                    .build_load(llvm_ret_ty, ret_alloca, "fn.ret.load")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let dest_arena = self
                    .builder
                    .build_load(ptr_t, caller_arena_alloca, "caller_arena.load")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_pointer_value();
                let copied = self.emit_return_value_deep_copy(
                    raw_ret, ret_ty, dest_arena,
                )?;
                Some(copied)
            }
        };

        // m53: free-fn handle-rooting. Flush the deferred-
        // dissolves frame opened at fn entry so any long-lived
        // loci bound in the fn body (e.g. `let _w = Watcher { };`
        // where Watcher subscribes to a bus subject) get drained
        // + dissolved before fn return. Pre-m53 the typed-return
        // path silently popped the frame without flushing,
        // leaking those handles past fn return. Per spec/memory
        // §"Free `fn` functions": "the function returns when:
        // body's last statement completes, AND all children of
        // the implicit locus have dissolved." flush_dissolve_frame
        // realizes the second clause at the codegen substrate.
        // The bus drain inside the flush dispatches any cells
        // published during the body (or by birth() of the loci
        // we're about to dissolve) before they go away. Same
        // entry point as locus-method scope-exit so the
        // semantics is uniform across all fn flavors. Runs after
        // the return-value deep-copy so freeing sub-locus arenas
        // here can't strand a String the caller is about to read.
        self.flush_dissolve_frame()?;

        // Destroy the per-call subregion AFTER the deep-copy so the
        // copy reads from valid memory. The subregion's chunk list
        // gets wholesale-freed; nothing in caller_arena is touched.
        let arena_destroy = self
            .module
            .get_function("lotus_arena_destroy")
            .expect("lotus_arena_destroy declared");
        let fn_arena_loaded = self
            .builder
            .build_load(ptr_t, fn_arena_alloca, "fn.arena.load")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_call(
                arena_destroy,
                &[fn_arena_loaded.into()],
                "fn.arena.destroy",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        match copied_ret {
            None => {
                self.builder
                    .build_return(None)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            Some(v) => {
                self.builder
                    .build_return(Some(&v))
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
        }
        Ok(())
    }

    /// m49: deep-copy a fn's return value from the per-call
    /// subregion into the caller's arena. Recursive on the lotus
    /// type structure. Value types (Int/Float/Bool/Decimal-i128/
    /// Time/Duration/no-payload-Enum) are pure SSA — identity copy.
    /// String calls `lotus_str_clone(dest, src)`. Tuple allocates
    /// a fresh storage struct in `dest_arena` and recursively
    /// copies each field. Other heap-typed returns (Array,
    /// TypeRef, has-payload-Enum) reject for v0.1 — none currently
    /// appear as free-fn returns; ship as a follow-up when a
    /// workload demands.
    fn emit_return_value_deep_copy(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &CodegenTy,
        dest_arena: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match ty {
            CodegenTy::Int
            | CodegenTy::Float
            | CodegenTy::Bool
            | CodegenTy::Decimal
            | CodegenTy::Time
            | CodegenTy::Duration
            | CodegenTy::FnPtr { .. } => Ok(value),
            CodegenTy::Enum(name) => {
                let info = self
                    .user_enums
                    .get(name.as_str())
                    .cloned();
                match info {
                    Some(info) if info.has_payload => {
                        // m51: per-variant switch + recursive
                        // payload deep-copy. See
                        // emit_enum_payload_deep_copy.
                        self.emit_enum_payload_deep_copy(
                            &info,
                            value.into_pointer_value(),
                            dest_arena,
                        )
                    }
                    _ => Ok(value),
                }
            }
            CodegenTy::String => {
                let f = self
                    .module
                    .get_function("lotus_str_clone")
                    .expect("lotus_str_clone declared");
                let res = self
                    .builder
                    .build_call(
                        f,
                        &[dest_arena.into(), value.into_pointer_value().into()],
                        "fn.ret.str.clone",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_str_clone returns ptr");
                Ok(res)
            }
            CodegenTy::Bytes => {
                // m89: Bytes returned from a fn is shipped through
                // the same lazy-global-payload arena that
                // read_bytes uses, so the returned pointer is
                // already program-lifetime-safe. No deep-copy
                // into dest_arena needed (and we'd lose the
                // length prefix if we tried to use lotus_str_clone,
                // which strlens). Pass the value through as-is —
                // future m89 follow-up: a `lotus_bytes_clone`
                // primitive that copies len + body into
                // dest_arena, for callers that genuinely want
                // the payload moved.
                Ok(value)
            }
            CodegenTy::Tuple(elem_tys) => {
                // Allocate a fresh tuple-storage struct in
                // dest_arena, then recursively deep-copy each
                // element. Layout matches Expr::Tuple lowering so
                // tup.0 / tup.1 reads work identically on the
                // returned tuple.
                let storage_ty = self.llvm_tuple_storage_type(elem_tys);
                let bytes = storage_ty
                    .size_of()
                    .expect("tuple storage type has known size");
                let alloc_fn = self
                    .module
                    .get_function("lotus_arena_alloc")
                    .expect("lotus_arena_alloc declared");
                let i64_t = self.context.i64_type();
                let new_tup = self
                    .builder
                    .build_call(
                        alloc_fn,
                        &[
                            dest_arena.into(),
                            bytes.into(),
                            i64_t.const_int(8, false).into(),
                        ],
                        "fn.ret.tuple.alloc",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("arena_alloc returns ptr")
                    .into_pointer_value();
                let i32_t = self.context.i32_type();
                let src_ptr = value.into_pointer_value();
                for (i, elem_ty) in elem_tys.iter().enumerate() {
                    let src_slot = unsafe {
                        self.builder
                            .build_gep(
                                storage_ty,
                                src_ptr,
                                &[
                                    i32_t.const_int(0, false),
                                    i32_t.const_int(i as u64, false),
                                ],
                                &format!("fn.ret.tup.src.slot{}", i),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?
                    };
                    let llvm_elem_ty = self.llvm_basic_type(elem_ty);
                    let elem_val = self
                        .builder
                        .build_load(
                            llvm_elem_ty,
                            src_slot,
                            &format!("fn.ret.tup.src.load{}", i),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let copied = self.emit_return_value_deep_copy(
                        elem_val, elem_ty, dest_arena,
                    )?;
                    let dst_slot = unsafe {
                        self.builder
                            .build_gep(
                                storage_ty,
                                new_tup,
                                &[
                                    i32_t.const_int(0, false),
                                    i32_t.const_int(i as u64, false),
                                ],
                                &format!("fn.ret.tup.dst.slot{}", i),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?
                    };
                    self.builder
                        .build_store(dst_slot, copied)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Ok(new_tup.into())
            }
            CodegenTy::Array(elem_ty, n) => {
                // m51: deep-copy a fixed-size array. Allocate
                // `[n x llvm(elem)]` in dest_arena, GEP each slot
                // in the source, recurse on the element value, and
                // store into the destination slot. Layout matches
                // the array-literal allocation path so callers see
                // the returned array's slot loads identically.
                let arr_ty =
                    self.llvm_array_storage_type(elem_ty, *n);
                let bytes = arr_ty
                    .size_of()
                    .expect("array storage type has known size");
                let alloc_fn = self
                    .module
                    .get_function("lotus_arena_alloc")
                    .expect("lotus_arena_alloc declared");
                let i64_t = self.context.i64_type();
                let new_arr = self
                    .builder
                    .build_call(
                        alloc_fn,
                        &[
                            dest_arena.into(),
                            bytes.into(),
                            i64_t.const_int(8, false).into(),
                        ],
                        "fn.ret.arr.alloc",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("arena_alloc returns ptr")
                    .into_pointer_value();
                let i32_t = self.context.i32_type();
                let src_ptr = value.into_pointer_value();
                let llvm_elem_ty = self.llvm_basic_type(elem_ty);
                for i in 0..*n {
                    let src_slot = unsafe {
                        self.builder
                            .build_gep(
                                arr_ty,
                                src_ptr,
                                &[
                                    i32_t.const_int(0, false),
                                    i32_t.const_int(i, false),
                                ],
                                &format!("fn.ret.arr.src.slot{}", i),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?
                    };
                    let elem_val = self
                        .builder
                        .build_load(
                            llvm_elem_ty,
                            src_slot,
                            &format!("fn.ret.arr.src.load{}", i),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let copied = self.emit_return_value_deep_copy(
                        elem_val, elem_ty, dest_arena,
                    )?;
                    let dst_slot = unsafe {
                        self.builder
                            .build_gep(
                                arr_ty,
                                new_arr,
                                &[
                                    i32_t.const_int(0, false),
                                    i32_t.const_int(i, false),
                                ],
                                &format!("fn.ret.arr.dst.slot{}", i),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?
                    };
                    self.builder
                        .build_store(dst_slot, copied)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Ok(new_arr.into())
            }
            CodegenTy::TypeRef(name) => {
                // m51: deep-copy a user-defined struct. Allocate a
                // fresh struct in dest_arena, walk each declared
                // field by its struct slot index, recursively copy
                // the loaded value, and store into the
                // destination. Field order matches the original
                // declaration via TypeInfo.field_order.
                let info = self
                    .user_types
                    .get(name.as_str())
                    .cloned()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "free-fn return of unknown type `{}`",
                            name
                        ))
                    })?;
                let struct_ty = info.struct_ty;
                let bytes = struct_ty
                    .size_of()
                    .expect("user-type struct has known size");
                let alloc_fn = self
                    .module
                    .get_function("lotus_arena_alloc")
                    .expect("lotus_arena_alloc declared");
                let i64_t = self.context.i64_type();
                let new_struct = self
                    .builder
                    .build_call(
                        alloc_fn,
                        &[
                            dest_arena.into(),
                            bytes.into(),
                            i64_t.const_int(8, false).into(),
                        ],
                        "fn.ret.struct.alloc",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("arena_alloc returns ptr")
                    .into_pointer_value();
                let src_ptr = value.into_pointer_value();
                for fname in &info.field_order {
                    let (idx, fty) = info
                        .fields
                        .get(fname)
                        .cloned()
                        .expect("field_order lists declared fields");
                    let src_slot = self
                        .builder
                        .build_struct_gep(
                            struct_ty,
                            src_ptr,
                            idx,
                            &format!("fn.ret.struct.src.{}", fname),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let llvm_field_ty = self.llvm_basic_type(&fty);
                    let field_val = self
                        .builder
                        .build_load(
                            llvm_field_ty,
                            src_slot,
                            &format!("fn.ret.struct.load.{}", fname),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let copied = self.emit_return_value_deep_copy(
                        field_val, &fty, dest_arena,
                    )?;
                    let dst_slot = self
                        .builder
                        .build_struct_gep(
                            struct_ty,
                            new_struct,
                            idx,
                            &format!("fn.ret.struct.dst.{}", fname),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(dst_slot, copied)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Ok(new_struct.into())
            }
            CodegenTy::LocusRef(_) => Err(CodegenError::Unsupported(format!(
                "free-fn return of {:?}: locus references shouldn't \
                 cross arena boundaries — pass via bus instead",
                ty
            ))),
            CodegenTy::Interface(_) => Err(CodegenError::Unsupported(format!(
                "free-fn return of {:?}: interface values shouldn't \
                 cross arena boundaries at v0 — the data pointer \
                 inside the fat pointer would dangle. Interface return \
                 deep-copy is a Phase B follow-up; for now, take an \
                 interface arg and dispatch from the caller's frame.",
                ty
            ))),
            CodegenTy::Cell(_, _) => Err(CodegenError::Unsupported(format!(
                "free-fn return of {:?}: F.22 capacity-slot cells \
                 can't cross fn boundaries — the cell's lifetime is \
                 the locus's slot, and the caller's frame doesn't \
                 know which slot to release/free into. Round-trip \
                 cells inside the locus body instead.",
                ty
            ))),
        }
    }

    /// m51: switch-on-tag deep-copy for a has-payload enum return
    /// value. We pre-load the tag, then dispatch through a switch
    /// where each case alloc's a fresh storage struct in
    /// dest_arena, deep-copies the variant's payload fields via
    /// load_enum_payload_fields + recursive emit_return_value_deep_copy,
    /// and writes them back via lower_enum_variant_alloc. The new
    /// pointers PHI-join into a single returned ptr value.
    fn emit_enum_payload_deep_copy(
        &mut self,
        info: &EnumInfo,
        src_ptr: PointerValue<'ctx>,
        dest_arena: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let func = self
            .current_fn
            .expect("enum deep-copy emitted inside a fn");
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i32_t = self.context.i32_type();
        let tag = self.load_enum_tag(info, src_ptr)?;
        let entry_bb = self
            .builder
            .get_insert_block()
            .expect("builder positioned");
        let cont_bb = self.context.append_basic_block(func, "enum.dc.cont");
        let default_bb = self.context.append_basic_block(func, "enum.dc.default");
        let mut variant_blocks: Vec<(
            inkwell::values::IntValue<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
            BasicValueEnum<'ctx>,
        )> = Vec::new();
        // Set up per-variant blocks first; switch wires after.
        for (i, _) in info.variants.iter().enumerate() {
            let bb = self
                .context
                .append_basic_block(func, &format!("enum.dc.v{}", i));
            self.builder.position_at_end(bb);
            // Push the caller_arena_override so payload allocations
            // for this variant happen in dest_arena, not the fn
            // subregion. Wait — actually, we need the *new enum
            // struct* to land in dest_arena via lower_enum_variant_alloc,
            // which calls arena_alloc through current_arena_ptr.
            // Override current_arena_override for this stretch.
            let prev_override = self.current_arena_override;
            self.current_arena_override = Some(dest_arena);
            // Load the variant's payload fields from src_ptr, then
            // deep-copy each one into dest_arena.
            let raw_fields = self.load_enum_payload_fields(info, src_ptr, i)?;
            let mut copied_fields: Vec<(BasicValueEnum<'ctx>, CodegenTy)> =
                Vec::with_capacity(raw_fields.len());
            for (val, fty) in raw_fields {
                let copied = self.emit_return_value_deep_copy(
                    val, &fty, dest_arena,
                )?;
                copied_fields.push((copied, fty));
            }
            // Allocate the new enum value in dest_arena (the
            // override routes lower_enum_variant_alloc's
            // arena_alloc there).
            let new_ptr =
                self.lower_enum_variant_alloc(info, i as u32, &copied_fields)?;
            self.current_arena_override = prev_override;
            self.builder
                .build_unconditional_branch(cont_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            variant_blocks.push((
                i32_t.const_int(i as u64, false),
                bb,
                new_ptr.into(),
            ));
        }
        // Default block: should be unreachable (tag is always one
        // of the declared variants). Fall through with a null ptr
        // PHI value to keep IR well-formed.
        self.builder.position_at_end(default_bb);
        self.builder
            .build_unconditional_branch(cont_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Wire the switch from entry_bb.
        self.builder.position_at_end(entry_bb);
        let cases: Vec<(
            inkwell::values::IntValue<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
        )> = variant_blocks
            .iter()
            .map(|(c, bb, _)| (*c, *bb))
            .collect();
        self.builder
            .build_switch(tag, default_bb, &cases)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // PHI in cont.
        self.builder.position_at_end(cont_bb);
        let phi = self
            .builder
            .build_phi(ptr_t, "enum.dc.phi")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let mut incoming: Vec<(
            &dyn inkwell::values::BasicValue<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
        )> = Vec::new();
        for (_, bb, val) in &variant_blocks {
            incoming.push((val, *bb));
        }
        let null = ptr_t.const_null();
        incoming.push((&null, default_bb));
        phi.add_incoming(&incoming);
        Ok(phi.as_basic_value())
    }

    /// Emit a call to a user-defined fn. Returns the lowered value
    /// + type when the fn has a return type, or `None` for void
    /// fns. Used from both expression-position and statement-position
    /// call sites.
    /// m62: lower a call to a generic free fn. Lowers each arg
    /// once (so side effects fire at most once), infers concrete
    /// type args from the resulting CodegenTys, mangles, and —
    /// if this instantiation hasn't been seen before — synthesizes
    /// + lowers a specialized fn body (saving and restoring
    /// builder state so the surrounding caller's IR isn't
    /// disturbed). Then emits a manual `build_call` using the
    /// already-lowered arg values + the implicit `__caller_arena`.
    fn lower_generic_fn_call(
        &mut self,
        name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<Option<(BasicValueEnum<'ctx>, CodegenTy)>, CodegenError> {
        let mut arg_pairs: Vec<(BasicValueEnum<'ctx>, CodegenTy)> =
            Vec::with_capacity(args.len());
        for a in args {
            arg_pairs.push(self.lower_expr(a, scope)?);
        }
        let template = self
            .generic_fn_templates
            .get(name)
            .cloned()
            .expect("caller verified template exists");
        if arg_pairs.len() != template.params.len() {
            return Err(CodegenError::Unsupported(format!(
                "generic fn `{}` expects {} args, got {} (m62 v0.1 \
                 doesn't fill defaults on generic templates yet)",
                name,
                template.params.len(),
                arg_pairs.len()
            )));
        }
        let arg_tys: Vec<CodegenTy> =
            arg_pairs.iter().map(|(_, t)| t.clone()).collect();
        let inferred = Self::infer_generic_fn_args(&template, &arg_tys)?;
        let mangled = Self::mangle_generic_name(name, &inferred)?;

        // Synthesize + lower the specialized fn if we haven't
        // seen this instantiation before.
        if !self.user_fns.contains_key(&mangled) {
            let synth = Self::synthesize_generic_fn_instantiation(
                &template, &inferred, &mangled,
            )?;
            // Save builder state — synthesizing a fn switches
            // the insertion point to the new fn's entry block,
            // and lower_user_fn_body resets all the
            // current_user_fn_* fields. Critically also
            // save/restore `in_main`: synthesis fires while
            // lowering main's body, but the synthesized fn
            // itself isn't main and `return x` inside its body
            // must hit the typed-return path, not the exit-code
            // path. Same for `current_self` and the loop stack.
            let saved_block = self.builder.get_insert_block();
            let saved_current_fn = self.current_fn;
            let saved_user_fn_ret = self.current_user_fn_ret.clone();
            let saved_caller_arena =
                self.current_user_fn_caller_arena;
            let saved_fn_arena = self.current_user_fn_arena;
            let saved_exit_bb = self.current_user_fn_exit_bb;
            let saved_ret_alloca = self.current_user_fn_ret_alloca;
            let saved_in_main = self.in_main;
            let saved_current_self = self.current_self.clone();
            let saved_loops = std::mem::take(&mut self.loops);

            self.in_main = false;
            self.current_self = None;

            self.declare_user_fn(&synth)?;
            self.lower_user_fn_body(&synth)?;

            // Restore caller-side state.
            if let Some(b) = saved_block {
                self.builder.position_at_end(b);
            }
            self.current_fn = saved_current_fn;
            self.current_user_fn_ret = saved_user_fn_ret;
            self.current_user_fn_caller_arena = saved_caller_arena;
            self.current_user_fn_arena = saved_fn_arena;
            self.current_user_fn_exit_bb = saved_exit_bb;
            self.current_user_fn_ret_alloca = saved_ret_alloca;
            self.in_main = saved_in_main;
            self.current_self = saved_current_self;
            self.loops = saved_loops;
        }

        // Emit the call manually using the pre-lowered args (so
        // we don't lower them twice). Mirrors lower_user_fn_call's
        // call-emit tail but without the inline arg lowering.
        let sig = self
            .user_fns
            .get(&mangled)
            .cloned()
            .expect("specialized fn was just declared");
        let caller_arena = self.current_arena_ptr()?;
        let mut llvm_args: Vec<BasicMetadataValueEnum> =
            Vec::with_capacity(sig.params.len() + 1);
        llvm_args.push(caller_arena.into());
        for (i, (val, ty)) in arg_pairs.iter().enumerate() {
            if ty != &sig.params[i] {
                return Err(CodegenError::Unsupported(format!(
                    "generic fn `{}` arg {} type mismatch after \
                     monomorphization: expected {:?}, got {:?}",
                    mangled, i, sig.params[i], ty
                )));
            }
            llvm_args.push((*val).into());
        }
        let call = self
            .builder
            .build_call(
                sig.func,
                &llvm_args,
                &format!("{}.call", mangled),
            )
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

    fn lower_user_fn_call(
        &mut self,
        name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<Option<(BasicValueEnum<'ctx>, CodegenTy)>, CodegenError> {
        let sig = self
            .user_fns
            .get(name)
            .cloned()
            .ok_or_else(|| {
                CodegenError::Unsupported(format!("call to unknown fn `{}`", name))
            })?;
        if args.len() > sig.params.len() {
            return Err(CodegenError::Unsupported(format!(
                "fn `{}` expects at most {} args, got {}",
                name,
                sig.params.len(),
                args.len()
            )));
        }
        // Verify each missing positional slot has a default.
        for (i, default) in sig.defaults.iter().enumerate() {
            if i >= args.len() && default.is_none() {
                return Err(CodegenError::Unsupported(format!(
                    "fn `{}`: required param at position {} not \
                     provided (only {} args given)",
                    name,
                    i,
                    args.len()
                )));
            }
        }
        // m49: prepend the caller's current arena as the implicit
        // `__caller_arena` first arg. The callee opens a subregion
        // of this at body entry; its body's allocations route
        // through that subregion, and any heap-typed return value
        // gets deep-copied back into this arena before the
        // subregion is destroyed. Captured here BEFORE we lower
        // the user-visible args because lowering them is allowed
        // to allocate (e.g. building a string) and we want the
        // arena snapshot to be the call site's arena, not whatever
        // intermediate state the arg-lowering walks into.
        let caller_arena_at_call = self.current_arena_ptr()?;
        let mut llvm_args: Vec<BasicMetadataValueEnum> =
            Vec::with_capacity(sig.params.len() + 1);
        llvm_args.push(caller_arena_at_call.into());
        for i in 0..sig.params.len() {
            let (v, ty) = if i < args.len() {
                self.lower_expr(&args[i], scope)?
            } else {
                // Default expressions evaluate at the call site.
                // For const/literal defaults that's a constant; for
                // arbitrary expressions they execute in the caller's
                // scope (matching the interpreter's semantics).
                let default = sig.defaults[i].as_ref().expect("checked above");
                self.lower_expr(default, scope)?
            };
            // F.20 Phase B: implicit locus → interface coercion. If
            // the param is an Interface and the arg is a LocusRef
            // (typechecker already verified the structural impl),
            // build the fat pointer at the call site. Other type
            // mismatches still error — except Int → Float, which
            // widens via sitofp (resolves notes/aperio-friction.md
            // 2026-05-10 float-surface-gaps).
            let v = if let (CodegenTy::Interface(iface), CodegenTy::LocusRef(l)) =
                (&sig.params[i], &ty)
            {
                let fat = self.coerce_to_interface(
                    v.into_pointer_value(),
                    l,
                    iface,
                )?;
                fat.into()
            } else if sig.params[i] == CodegenTy::Float && ty == CodegenTy::Int {
                let widened = self.coerce_to_float(
                    v,
                    &ty,
                    &format!("fn `{}` arg {}", name, i),
                )?;
                widened.into()
            } else if ty != sig.params[i] {
                return Err(CodegenError::Unsupported(format!(
                    "fn `{}` arg {} type mismatch: expected {:?}, got {:?}",
                    name, i, sig.params[i], ty
                )));
            } else {
                v
            };
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
                // m73a: rewrite recognized `std::*` paths to the
                // mangled stdlib locus name declared in
                // STDLIB_AP_SOURCE. Unknown qualified paths still
                // error below, just with the better
                // "no locus or type by that name" message.
                let segs: Vec<&str> = path
                    .segments
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect();
                let resolved_name: String = if path.segments.len() > 1 {
                    match stdlib_mangled_for_path(&segs) {
                        Some(mangled) => mangled.to_string(),
                        None => {
                            return Err(CodegenError::Unsupported(format!(
                                "qualified-name struct literal `{}`",
                                segs.join("::")
                            )));
                        }
                    }
                } else {
                    path.segments[0].name.clone()
                };
                let name = resolved_name.as_str();
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
                        } else if name == "check_closures" {
                            // m44: explicit-epoch closure check
                            // surface. `check_closures();` from
                            // inside a locus body fires every
                            // explicit-epoch closure on the
                            // current self, routing violations
                            // through the locus's parent
                            // on_failure (matching the
                            // birth/dissolve/tick paths).
                            self.lower_check_closures_call(args)?;
                        } else if self.user_fns.contains_key(name) {
                            // Discard return value; statement-position
                            // call.
                            let _ = self.lower_user_fn_call(name, args, scope)?;
                        } else if self
                            .generic_fn_templates
                            .contains_key(name)
                        {
                            // m62: generic fn at statement
                            // position — synthesize on-demand,
                            // discard return value.
                            let _ = self
                                .lower_generic_fn_call(name, args, scope)?;
                        } else if let Some((slot_ptr, CodegenTy::FnPtr {
                            args: arg_tys,
                            ret: ret_ty,
                        })) = scope.locals.get(name).cloned()
                        {
                            // m83: local-variable fn-pointer call.
                            // `on_conn(s)` inside a fn whose param
                            // is `on_conn: fn(Stream)`. Load the
                            // pointer value from its slot and
                            // indirect-call through the shared
                            // m80 helper. Statement position:
                            // discard any return value.
                            let ptr_t = self
                                .context
                                .ptr_type(AddressSpace::default());
                            let fn_value_ptr = self
                                .builder
                                .build_load(ptr_t, slot_ptr, name)
                                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                                .into_pointer_value();
                            let _ = self.emit_fnptr_indirect_call(
                                fn_value_ptr,
                                &arg_tys,
                                ret_ty.as_deref(),
                                args,
                                scope,
                                name,
                            )?;
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
                    Expr::Field { receiver, name, .. } => {
                        // m81: external method call —
                        // `obj.method(args)` where obj is a
                        // LocusRef value (let-bound, fn param,
                        // accepted child).
                        let _ = self.lower_external_method_call(
                            receiver,
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
            Stmt::Let { name, ty: ascribed, value, .. } => {
                // m61b: when a let has both a generic-typed
                // ascription and a bare-name struct literal as
                // its value, rewrite the literal's path to the
                // mangled name. So `let b: Box<Int> = Box { value: 42 };`
                // works without requiring the user to spell
                // Box_Int.
                let rewritten;
                let value_to_lower: &Expr = match (ascribed.as_ref(), value)
                {
                    (
                        Some(asc),
                        Expr::Struct {
                            path,
                            inits,
                            span: sspan,
                        },
                    ) => match self
                        .resolve_generic_struct_path(path, asc)
                    {
                        Some(new_path) => {
                            rewritten = Expr::Struct {
                                path: new_path,
                                inits: inits.clone(),
                                span: *sspan,
                            };
                            &rewritten
                        }
                        None => value,
                    },
                    _ => value,
                };
                // m82: if the RHS is a locus struct literal, signal
                // `lower_locus_instantiation` to defer the locus's
                // dissolve to the enclosing fn's scope-exit flush
                // instead of firing eagerly at the end of the
                // struct-literal expression. The binding is the
                // user-visible handle; the locus instance lives
                // until that handle goes out of scope. Set
                // immediately before lowering the RHS — consumed
                // (via `std::mem::take`) by the outermost
                // instantiation, leaving any nested locus literals
                // inside the RHS on the eager path. Cleared after
                // lowering regardless of whether the flag was
                // consumed (defensive — guards against the RHS
                // bailing out before reaching an instantiation).
                if self.expr_is_locus_literal(value_to_lower) {
                    self.defer_next_locus_dissolve = true;
                }
                let lower_result = self.lower_expr(value_to_lower, scope);
                self.defer_next_locus_dissolve = false;
                let (mut val, mut ty) = lower_result?;
                // Int → Float widening at a Float-ascribed let
                // binding. `let nf: Float = self.n;` where `n: Int`
                // is the canonical case. Resolves
                // notes/aperio-friction.md 2026-05-10
                // float-surface-gaps (sub-bullet 1). Other
                // ascription/RHS mismatches stay at the existing
                // mismatch behavior — this is a one-way widening
                // only, not a general coercion surface.
                if let Some(asc) = ascribed.as_ref() {
                    if let Ok(asc_ty) = self.type_expr_to_codegen_ty(asc) {
                        if asc_ty == CodegenTy::Float && ty == CodegenTy::Int {
                            let widened = self.coerce_to_float(
                                val,
                                &ty,
                                &format!("let {}: Float", name.name),
                            )?;
                            val = widened.into();
                            ty = CodegenTy::Float;
                        }
                    }
                }
                let alloca = self.alloca_for(&ty, &name.name)?;
                self.builder
                    .build_store(alloca, val)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                scope.locals.insert(name.name.clone(), (alloca, ty));
                Ok(BlockEnd::Open)
            }
            Stmt::LetTuple { names, value, .. } => {
                let (tup_val, tup_ty) = self.lower_expr(value, scope)?;
                let elem_tys = match &tup_ty {
                    CodegenTy::Tuple(ts) => ts.clone(),
                    other => {
                        return Err(CodegenError::Unsupported(format!(
                            "let-tuple destructure expects a tuple rhs, \
                             got {:?}",
                            other
                        )));
                    }
                };
                if elem_tys.len() != names.len() {
                    return Err(CodegenError::Unsupported(format!(
                        "let-tuple destructure: expected {} elements, \
                         got {}",
                        names.len(),
                        elem_tys.len()
                    )));
                }
                let storage_ty = self.llvm_tuple_storage_type(&elem_tys);
                let tup_ptr = tup_val.into_pointer_value();
                let i32_t = self.context.i32_type();
                for (i, (n, et)) in
                    names.iter().zip(elem_tys.iter()).enumerate()
                {
                    let slot = unsafe {
                        self.builder
                            .build_gep(
                                storage_ty,
                                tup_ptr,
                                &[
                                    i32_t.const_int(0, false),
                                    i32_t.const_int(i as u64, false),
                                ],
                                &format!("tup.{}.ptr", i),
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    };
                    let elem_llvm = self.llvm_basic_type(et);
                    let loaded = self
                        .builder
                        .build_load(elem_llvm, slot, &format!("tup.{}", i))
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let alloca = self.alloca_for(et, &n.name)?;
                    self.builder
                        .build_store(alloca, loaded)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    scope.locals.insert(n.name.clone(), (alloca, et.clone()));
                }
                Ok(BlockEnd::Open)
            }
            Stmt::Assign { target, op, value, .. } => {
                // Resolve the target into a (slot_ptr, slot_ty)
                // pair. Bare locals come from the scope; `self.X`
                // GEPs into the current method's self-struct.
                let (slot_ptr, slot_ty, slot_name) = if target.head.name
                    == "self"
                {
                    // market-book: `self.<arrayField>[i] = v`.
                    // Two-segment self-target (Field, Index) that
                    // mirrors the let-bound `arr[i] = v` path:
                    // GEP into the self struct's field slot,
                    // load the array pointer, GEP at the
                    // computed index, store. Without this branch
                    // every BookL ladder helper would have to
                    // copy-out / mutate-locally / write-back the
                    // entire fixed-cap array per single-slot
                    // update — quadratic on ladder size.
                    if target.tail.len() == 2 {
                        if let (
                            LValueSeg::Field(fname_ident),
                            LValueSeg::Index(idx_expr),
                        ) = (&target.tail[0], &target.tail[1])
                        {
                            let cs = self
                                .current_self
                                .as_ref()
                                .cloned()
                                .ok_or_else(|| {
                                    CodegenError::Unsupported(
                                        "`self.X[i] =` outside a locus method"
                                            .to_string(),
                                    )
                                })?;
                            let (field_idx, field_ty) = cs
                                .fields
                                .get(&fname_ident.name)
                                .cloned()
                                .ok_or_else(|| {
                                    CodegenError::Unsupported(format!(
                                        "no field `{}` on locus self",
                                        fname_ident.name
                                    ))
                                })?;
                            let (elem_ty, n) = match field_ty {
                                CodegenTy::Array(elem, n) => (*elem, n),
                                other => {
                                    return Err(CodegenError::Unsupported(format!(
                                        "indexed assignment to non-array \
                                         self field `{}` (type {:?})",
                                        fname_ident.name, other
                                    )));
                                }
                            };
                            let field_slot_ptr = self
                                .builder
                                .build_struct_gep(
                                    cs.struct_ty,
                                    cs.self_ptr,
                                    field_idx,
                                    &format!("self.{}.idx_ptr", fname_ident.name),
                                )
                                .map_err(|e| {
                                    CodegenError::LlvmEmit(e.to_string())
                                })?;
                            let ptr_t = self
                                .context
                                .ptr_type(AddressSpace::default());
                            let arr_ptr = self
                                .builder
                                .build_load(
                                    ptr_t,
                                    field_slot_ptr,
                                    &format!(
                                        "self.{}.arr",
                                        fname_ident.name
                                    ),
                                )
                                .map_err(|e| {
                                    CodegenError::LlvmEmit(e.to_string())
                                })?
                                .into_pointer_value();
                            let (idx_val, idx_ty) =
                                self.lower_expr(idx_expr, scope)?;
                            if idx_ty != CodegenTy::Int {
                                return Err(CodegenError::Unsupported(format!(
                                    "array index must be Int, got {:?}",
                                    idx_ty
                                )));
                            }
                            let i32_t = self.context.i32_type();
                            let storage_ty =
                                self.llvm_array_storage_type(&elem_ty, n);
                            let slot_ptr = unsafe {
                                self.builder
                                    .build_gep(
                                        storage_ty,
                                        arr_ptr,
                                        &[
                                            i32_t.const_int(0, false),
                                            idx_val.into_int_value(),
                                        ],
                                        &format!(
                                            "self.{}.slot",
                                            fname_ident.name
                                        ),
                                    )
                                    .map_err(|e| {
                                        CodegenError::LlvmEmit(e.to_string())
                                    })?
                            };
                            let (rhs, rhs_ty) =
                                self.lower_expr(value, scope)?;
                            if rhs_ty != elem_ty {
                                return Err(CodegenError::Unsupported(format!(
                                    "type mismatch in `self.{}[idx] = ...`: \
                                     slot {:?} vs rhs {:?}",
                                    fname_ident.name, elem_ty, rhs_ty
                                )));
                            }
                            if !matches!(op, AssignOp::Eq) {
                                return Err(CodegenError::Unsupported(format!(
                                    "compound assignment `{:?}` on \
                                     `self.{}[idx]` is not supported; \
                                     use `=` only",
                                    op, fname_ident.name
                                )));
                            }
                            self.builder
                                .build_store(slot_ptr, rhs)
                                .map_err(|e| {
                                    CodegenError::LlvmEmit(e.to_string())
                                })?;
                            return Ok(BlockEnd::Open);
                        }
                    }
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
                } else if target.tail.len() == 1
                    && matches!(target.tail[0], LValueSeg::Field(_))
                    && matches!(
                        scope.locals.get(&target.head.name).map(|(_, t)| t),
                        Some(CodegenTy::Cell(inner, _))
                            if matches!(inner.as_ref(), CodegenTy::TypeRef(_))
                    )
                {
                    // F.22 v1.x-2: `cell.field = v` on a struct-cell
                    // local. Load the cell pointer, GEP into the
                    // struct's field, store. Same shape as TypeRef
                    // field assignment.
                    let fname = match &target.tail[0] {
                        LValueSeg::Field(i) => i.name.clone(),
                        _ => unreachable!(),
                    };
                    let (alloca, cell_ty) = scope
                        .locals
                        .get(&target.head.name)
                        .cloned()
                        .expect("matched above");
                    let elem_ty_name = match &cell_ty {
                        CodegenTy::Cell(inner, _) => match inner.as_ref() {
                            CodegenTy::TypeRef(n) => n.clone(),
                            _ => unreachable!("matched above"),
                        },
                        _ => unreachable!("matched above"),
                    };
                    let info = self
                        .user_types
                        .get(&elem_ty_name)
                        .cloned()
                        .ok_or_else(|| {
                            CodegenError::Unsupported(format!(
                                "cell-field assign: type `{}` not declared",
                                elem_ty_name
                            ))
                        })?;
                    let (field_idx, field_ty) = info
                        .fields
                        .get(&fname)
                        .cloned()
                        .ok_or_else(|| {
                            CodegenError::Unsupported(format!(
                                "no field `{}` on type `{}`",
                                fname, elem_ty_name
                            ))
                        })?;
                    let ptr_t = self
                        .context
                        .ptr_type(AddressSpace::default());
                    let cell_ptr = self
                        .builder
                        .build_load(ptr_t, alloca, &target.head.name)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_pointer_value();
                    let field_ptr = self
                        .builder
                        .build_struct_gep(
                            info.struct_ty,
                            cell_ptr,
                            field_idx,
                            &format!(
                                "{}.{}.ptr",
                                target.head.name, fname
                            ),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    (
                        field_ptr,
                        field_ty,
                        format!("{}.{}", target.head.name, fname),
                    )
                } else if target.tail.len() == 1
                    && matches!(target.tail[0], LValueSeg::Index(_))
                {
                    // `arr[i] = v` for a local array. Look up the
                    // local, load the array ptr, GEP at index, store.
                    let idx_expr = match &target.tail[0] {
                        LValueSeg::Index(e) => e,
                        _ => unreachable!(),
                    };
                    let (alloca, ty) = scope
                        .locals
                        .get(&target.head.name)
                        .cloned()
                        .ok_or_else(|| {
                            CodegenError::Unsupported(format!(
                                "indexed assignment to unbound `{}`",
                                target.head.name
                            ))
                        })?;
                    let (elem_ty, n) = match ty {
                        CodegenTy::Array(elem, n) => (*elem, n),
                        other => {
                            return Err(CodegenError::Unsupported(format!(
                                "indexed assignment to non-array local \
                                 `{}` (type {:?})",
                                target.head.name, other
                            )));
                        }
                    };
                    let ptr_t = self.context.ptr_type(AddressSpace::default());
                    let arr_ptr = self
                        .builder
                        .build_load(ptr_t, alloca, &target.head.name)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_pointer_value();
                    let (idx_val, idx_ty) = self.lower_expr(idx_expr, scope)?;
                    if idx_ty != CodegenTy::Int {
                        return Err(CodegenError::Unsupported(format!(
                            "array index must be Int, got {:?}",
                            idx_ty
                        )));
                    }
                    let i32_t = self.context.i32_type();
                    let storage_ty = self.llvm_array_storage_type(&elem_ty, n);
                    let slot_ptr = unsafe {
                        self.builder
                            .build_gep(
                                storage_ty,
                                arr_ptr,
                                &[
                                    i32_t.const_int(0, false),
                                    idx_val.into_int_value(),
                                ],
                                "array.assign.slot",
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    };
                    (
                        slot_ptr,
                        elem_ty,
                        format!("{}[idx]", target.head.name),
                    )
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
            Stmt::Yield(_) => {
                // m26b: explicit yield → drain the bus queue at
                // this point. Per spec/runtime.md cooperative
                // yield points: "explicit `yield` (rare, for
                // long-running computations)." Use case: a
                // long-internal-loop body where you want pending
                // bus cells to fire mid-body rather than waiting
                // for the body's normal scope-exit drain.
                self.emit_bus_drain()?;
                Ok(BlockEnd::Open)
            }
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
                    RecoveryOp::Restart => self.lower_restart_call(args, scope),
                    RecoveryOp::RestartInPlace => {
                        self.lower_restart_in_place_call(args, scope)
                    }
                    RecoveryOp::Quarantine => {
                        self.lower_quarantine_call(args, scope)
                    }
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
            Stmt::Match(m) => self.lower_match_stmt(m, scope),
            Stmt::Fail { .. } => Err(CodegenError::Unsupported(
                "Stmt::Fail not yet lowered (v1.x-FORM-1 PR1 is parser-only; \
                 codegen ships in PR6)"
                    .into(),
            )),
            Stmt::Expr(_) => Err(CodegenError::Unsupported(
                "expression statement other than locus literal or builtin call"
                    .to_string(),
            )),
        }
    }

    fn lower_if(
        &mut self,
        ifs: &IfStmt,
        scope: &mut Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        let (cond_v, cond_ty) =
            self.lower_expr(&ifs.cond, scope)?;
        if cond_ty != CodegenTy::Bool {
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
        if cond_ty != CodegenTy::Bool {
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

    /// m36: lower a `len(x)` builtin call. v0 supports two
    /// argument shapes: a String (calls `lotus_str_len` for an
    /// O(strlen) length count) and a fixed-size Array (compile-
    /// time N from `CodegenTy::Array(_, N)`). Returns Int.
    /// Tuples / TypeRef receivers are deliberately rejected —
    /// no use case asks for tuple-arity at runtime, and structs
    /// have a fixed field set known at the type level.
    fn lower_len_builtin(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "`len` expects exactly 1 argument, got {}",
                args.len()
            )));
        }
        let (v, ty) = self.lower_expr(&args[0], scope)?;
        match ty {
            CodegenTy::String => {
                let len_fn = self
                    .module
                    .get_function("lotus_str_len")
                    .expect("lotus_str_len declared");
                let val = self
                    .builder
                    .build_call(
                        len_fn,
                        &[v.into_pointer_value().into()],
                        "str.len",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_str_len returns i64");
                Ok((val, CodegenTy::Int))
            }
            CodegenTy::Bytes => {
                // m89: Bytes carries an explicit length prefix —
                // not strlen, since binary data may have embedded
                // NULs. lotus_bytes_len reads the i64 at offset 0.
                let len_fn = self
                    .module
                    .get_function("lotus_bytes_len")
                    .expect("lotus_bytes_len declared");
                let val = self
                    .builder
                    .build_call(
                        len_fn,
                        &[v.into_pointer_value().into()],
                        "bytes.len",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_bytes_len returns i64");
                Ok((val, CodegenTy::Int))
            }
            CodegenTy::Array(_, n) => {
                let val = self.context.i64_type().const_int(n, true);
                Ok((val.into(), CodegenTy::Int))
            }
            other => Err(CodegenError::Unsupported(format!(
                "`len` not supported for argument type {:?}",
                other
            ))),
        }
    }

    /// v1.x-11: lower `Int(x)` — explicit Float → Int narrowing.
    /// Float arg lowers via `fptosi` (round-toward-zero); Int arg
    /// is the identity. Decimal narrowing currently uses the same
    /// fptosi after reading the cell, so Decimal → Int also works.
    /// Other types are rejected so silent narrowing doesn't sneak
    /// in through inference.
    fn lower_int_cast_builtin(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "`Int` cast expects exactly 1 argument, got {}",
                args.len()
            )));
        }
        let (v, ty) = self.lower_expr(&args[0], scope)?;
        match ty {
            CodegenTy::Int => Ok((v, CodegenTy::Int)),
            CodegenTy::Float => {
                let res = self
                    .builder
                    .build_float_to_signed_int(
                        v.into_float_value(),
                        self.context.i64_type(),
                        "Int.cast",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((res.into(), CodegenTy::Int))
            }
            other => Err(CodegenError::Unsupported(format!(
                "`Int(...)` cast not supported for argument type {:?} \
                 (only Float → Int narrowing and Int identity are \
                 supported in v1)",
                other
            ))),
        }
    }

    /// m38: lower min(a, b) / max(a, b) / abs(x). All work
    /// across the four numeric types (Int / Duration via
    /// signed integer ops, Float / Decimal via float ops).
    /// abs takes 1 arg; min/max take 2.
    fn lower_math_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let arity = if name == "abs" { 1 } else { 2 };
        if args.len() != arity {
            return Err(CodegenError::Unsupported(format!(
                "`{}` expects exactly {} argument(s), got {}",
                name,
                arity,
                args.len()
            )));
        }
        let (av, at) = self.lower_expr(&args[0], scope)?;
        if name == "abs" {
            return self.lower_abs(av, &at);
        }
        let (bv, bt) = self.lower_expr(&args[1], scope)?;
        if at != bt {
            return Err(CodegenError::Unsupported(format!(
                "`{}`: operand types must match; got {:?} and {:?}",
                name, at, bt
            )));
        }
        match at {
            CodegenTy::Int | CodegenTy::Duration | CodegenTy::Decimal => {
                let pred = match name {
                    "min" => inkwell::IntPredicate::SLT,
                    "max" => inkwell::IntPredicate::SGT,
                    _ => unreachable!(),
                };
                let cmp = self
                    .builder
                    .build_int_compare(
                        pred,
                        av.into_int_value(),
                        bv.into_int_value(),
                        &format!("{}.cmp", name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let v = self
                    .builder
                    .build_select(cmp, av, bv, &format!("{}.sel", name))
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v, at))
            }
            CodegenTy::Float => {
                let pred = match name {
                    "min" => inkwell::FloatPredicate::OLT,
                    "max" => inkwell::FloatPredicate::OGT,
                    _ => unreachable!(),
                };
                let cmp = self
                    .builder
                    .build_float_compare(
                        pred,
                        av.into_float_value(),
                        bv.into_float_value(),
                        &format!("{}.fcmp", name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let v = self
                    .builder
                    .build_select(cmp, av, bv, &format!("{}.sel", name))
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v, at))
            }
            other => Err(CodegenError::Unsupported(format!(
                "`{}` not supported for type {:?}",
                name, other
            ))),
        }
    }

    fn lower_abs(
        &mut self,
        v: BasicValueEnum<'ctx>,
        ty: &CodegenTy,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        match ty {
            CodegenTy::Int | CodegenTy::Duration | CodegenTy::Decimal => {
                // m48: Decimal lives in i128 so the int-abs path
                // works for it directly — same neg + select shape
                // as Int / Duration, just with the i128 zero
                // constant instead of i64.
                let zero: inkwell::values::IntValue<'ctx> =
                    if matches!(ty, CodegenTy::Decimal) {
                        i128_const(self.context, 0)
                    } else {
                        self.context.i64_type().const_int(0, true)
                    };
                let iv = v.into_int_value();
                let neg = self
                    .builder
                    .build_int_sub(zero, iv, "abs.neg")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let is_neg = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::SLT,
                        iv,
                        zero,
                        "abs.is_neg",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let chosen = self
                    .builder
                    .build_select(is_neg, neg.into(), v, "abs.sel")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((chosen, ty.clone()))
            }
            CodegenTy::Float => {
                let fv = v.into_float_value();
                let neg = self
                    .builder
                    .build_float_neg(fv, "abs.fneg")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let zero = self.context.f64_type().const_float(0.0);
                let is_neg = self
                    .builder
                    .build_float_compare(
                        inkwell::FloatPredicate::OLT,
                        fv,
                        zero,
                        "abs.is_neg",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let chosen = self
                    .builder
                    .build_select(is_neg, neg.into(), v, "abs.sel")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((chosen, ty.clone()))
            }
            other => Err(CodegenError::Unsupported(format!(
                "`abs` not supported for type {:?}",
                other
            ))),
        }
    }

    /// m38: lower starts_with(s, prefix) / contains(s, sub).
    /// Both take two String args, return Bool. C runtime helpers
    /// return i32 0/1 which we truncate to i1 by comparing to 1.
    fn lower_str_predicate_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "`{}` expects exactly 2 arguments, got {}",
                name,
                args.len()
            )));
        }
        let (sv, st) = self.lower_expr(&args[0], scope)?;
        let (pv, pt) = self.lower_expr(&args[1], scope)?;
        if !matches!(st, CodegenTy::String) || !matches!(pt, CodegenTy::String) {
            return Err(CodegenError::Unsupported(format!(
                "`{}` expects two String args; got {:?} and {:?}",
                name, st, pt
            )));
        }
        let runtime_fn_name = format!("lotus_str_{}", name);
        let f = self
            .module
            .get_function(&runtime_fn_name)
            .expect("string predicate runtime fn declared");
        let raw = self
            .builder
            .build_call(
                f,
                &[
                    sv.into_pointer_value().into(),
                    pv.into_pointer_value().into(),
                ],
                &format!("str.{}", name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("string predicate returns i32");
        let one = self.context.i32_type().const_int(1, false);
        let v = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                raw.into_int_value(),
                one,
                &format!("str.{}.bool", name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((v.into(), CodegenTy::Bool))
    }

    /// m37: lower a `to_string(x)` builtin call. Routes by the
    /// argument's CodegenTy to the right snprintf-backed runtime
    /// helper; result is a fresh arena-owned NUL-terminated
    /// buffer formatted exactly like `println` would render the
    /// same value, so a value written via to_string + concat
    /// reads identical to the same value passed to println.
    /// String passes through (so `to_string` is the identity on
    /// String — handy in generic-feeling helper fns).
    /// Whether `value_to_string` can render a value of this type
    /// inline. Used by the `String + <printable>` auto-coercion
    /// in lower_expr's BinOp::Add branch — types not in this set
    /// fall back to the existing mixed-type error.
    fn value_to_string_supports(ty: &CodegenTy) -> bool {
        matches!(
            ty,
            CodegenTy::String
                | CodegenTy::Int
                | CodegenTy::Bool
                | CodegenTy::Float
                | CodegenTy::Decimal
                | CodegenTy::Duration
                | CodegenTy::Time
                | CodegenTy::Enum(_)
        )
    }

    fn lower_to_string_builtin(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "`to_string` expects exactly 1 argument, got {}",
                args.len()
            )));
        }
        let (v, ty) = self.lower_expr(&args[0], scope)?;
        let res = self.value_to_string(v, &ty)?;
        Ok((res, CodegenTy::String))
    }

    /// m47-payloads-followup: convert any single value to a
    /// String pointer, mirroring the interpreter's
    /// Value::display semantics. Extracted from
    /// lower_to_string_builtin so the has-payload enum branch can
    /// recurse on each payload field's render. For has-payload
    /// enums this emits a switch on the variant tag with
    /// per-variant inline string assembly via lotus_str_concat.
    fn value_to_string(
        &mut self,
        v: BasicValueEnum<'ctx>,
        ty: &CodegenTy,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let ty = ty.clone();
        match ty {
            CodegenTy::String => Ok(v),
            CodegenTy::Int => {
                let arena_ptr = self.current_arena_ptr()?;
                let f = self
                    .module
                    .get_function("lotus_str_from_int")
                    .expect("lotus_str_from_int declared");
                let res = self
                    .builder
                    .build_call(
                        f,
                        &[arena_ptr.into(), v.into_int_value().into()],
                        "to_string.int",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_str_from_int returns ptr");
                Ok(res)
            }
            CodegenTy::Duration => {
                let arena_ptr = self.current_arena_ptr()?;
                let f = self
                    .module
                    .get_function("lotus_str_from_duration")
                    .expect("lotus_str_from_duration declared");
                let res = self
                    .builder
                    .build_call(
                        f,
                        &[arena_ptr.into(), v.into_int_value().into()],
                        "to_string.dur",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_str_from_duration returns ptr");
                Ok(res)
            }
            CodegenTy::Float => {
                let arena_ptr = self.current_arena_ptr()?;
                let f = self
                    .module
                    .get_function("lotus_str_from_float")
                    .expect("lotus_str_from_float declared");
                let res = self
                    .builder
                    .build_call(
                        f,
                        &[arena_ptr.into(), v.into_float_value().into()],
                        "to_string.float",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_str_from_float returns ptr");
                Ok(res)
            }
            CodegenTy::Decimal => {
                let arena_ptr = self.current_arena_ptr()?;
                let i128_v = v.into_int_value();
                let i64_t = self.context.i64_type();
                let lo = self
                    .builder
                    .build_int_truncate(i128_v, i64_t, "ts_dec_lo")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let shift = self.context.i128_type().const_int(64, false);
                let hi_wide = self
                    .builder
                    .build_right_shift(i128_v, shift, true, "ts_dec_hi_w")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let hi = self
                    .builder
                    .build_int_truncate(hi_wide, i64_t, "ts_dec_hi")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let f = self
                    .module
                    .get_function("lotus_str_from_decimal")
                    .expect("lotus_str_from_decimal declared");
                let res = self
                    .builder
                    .build_call(
                        f,
                        &[arena_ptr.into(), hi.into(), lo.into()],
                        "to_string.decimal",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_str_from_decimal returns ptr");
                Ok(res)
            }
            CodegenTy::Bool => {
                let true_ptr = self.global_string("true");
                let false_ptr = self.global_string("false");
                let res = self
                    .builder
                    .build_select(
                        v.into_int_value(),
                        true_ptr,
                        false_ptr,
                        "to_string.bool",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok(res)
            }
            CodegenTy::Enum(enum_name) => {
                let info = self
                    .user_enums
                    .get(&enum_name)
                    .cloned()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "to_string: unknown enum `{}`",
                            enum_name
                        ))
                    })?;
                if !info.has_payload {
                    // No-payload enum: simple names-array lookup
                    // — same as a Bool select but indexed by tag.
                    let names_g = self.enum_names_array(&enum_name)?;
                    let array_ty =
                        names_g.get_value_type().into_array_type();
                    let i32_t = self.context.i32_type();
                    let zero = i32_t.const_int(0, false);
                    let elem_ptr = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                array_ty,
                                names_g.as_pointer_value(),
                                &[zero, v.into_int_value()],
                                "ts.enum.name.ptr",
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?
                    };
                    let ptr_t =
                        self.context.ptr_type(AddressSpace::default());
                    let label = self
                        .builder
                        .build_load(ptr_t, elem_ptr, "ts.enum.name")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    return Ok(label);
                }
                self.lower_enum_with_payload_to_string(&info, &enum_name, v)
            }
            other => Err(CodegenError::Unsupported(format!(
                "`to_string` not supported for type {:?}",
                other
            ))),
        }
    }

    /// m47-payloads-followup: render a has-payload enum value to
    /// a String. Per-variant block builds the rendering via
    /// lotus_str_concat; results join in a PHI. Matches the
    /// interpreter's Value::display: no-payload variant of a
    /// has-payload enum → "EnumName::V"; with payload →
    /// "EnumName::V(p0, p1, ...)".
    fn lower_enum_with_payload_to_string(
        &mut self,
        info: &EnumInfo,
        enum_name: &str,
        v: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let func = self
            .current_fn
            .expect("value_to_string inside a function body");
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i32_t = self.context.i32_type();

        // Entry block is whatever the caller is in. Load the tag
        // there, then jump to a dispatch block that switches.
        let entry_bb = self
            .builder
            .get_insert_block()
            .expect("builder positioned at entry");
        let enum_ptr = v.into_pointer_value();
        let tag = self.load_enum_tag(info, enum_ptr)?;
        let dispatch_bb =
            self.context.append_basic_block(func, "enum.ts.dispatch");
        let cont_bb = self.context.append_basic_block(func, "enum.ts.cont");
        let default_bb =
            self.context.append_basic_block(func, "enum.ts.default");

        // entry → dispatch
        self.builder.position_at_end(entry_bb);
        self.builder
            .build_unconditional_branch(dispatch_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Build per-variant blocks first so we can collect the
        // (tag, block, rendered) tuples; the switch in dispatch_bb
        // wires to them.
        let mut variant_blocks: Vec<(
            inkwell::values::IntValue<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
            BasicValueEnum<'ctx>,
        )> = Vec::new();
        for (idx, vinfo) in info.variants.iter().enumerate() {
            let vinfo = vinfo.clone();
            let case_bb = self
                .context
                .append_basic_block(func, &format!("enum.ts.v{}", idx));
            self.builder.position_at_end(case_bb);
            let mut acc =
                self.global_string(&format!("{}::{}", enum_name, vinfo.name));
            if !vinfo.field_tys.is_empty() {
                let open_paren = self.global_string("(");
                acc = self
                    .str_concat(acc.into(), open_paren.into())?
                    .into_pointer_value();
                let fields =
                    self.load_enum_payload_fields(info, enum_ptr, idx)?;
                for (j, (fv, fty)) in fields.iter().enumerate() {
                    if j > 0 {
                        let comma = self.global_string(", ");
                        acc = self
                            .str_concat(acc.into(), comma.into())?
                            .into_pointer_value();
                    }
                    let rendered = self.value_to_string(*fv, fty)?;
                    acc = self.str_concat(acc.into(), rendered)?.into_pointer_value();
                }
                let close = self.global_string(")");
                acc = self
                    .str_concat(acc.into(), close.into())?
                    .into_pointer_value();
            }
            self.builder
                .build_unconditional_branch(cont_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let final_bb = self.builder.get_insert_block().unwrap();
            variant_blocks.push((
                i32_t.const_int(idx as u64, false),
                final_bb,
                acc.into(),
            ));
        }

        // Default: unreachable for a well-typed program; use an
        // empty string so the PHI is well-defined.
        self.builder.position_at_end(default_bb);
        let default_acc = self.global_string("");
        self.builder
            .build_unconditional_branch(cont_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Dispatch: switch on the tag.
        self.builder.position_at_end(dispatch_bb);
        let cases: Vec<(
            inkwell::values::IntValue<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
        )> = variant_blocks
            .iter()
            .map(|(c, bb, _)| (*c, *bb))
            .collect();
        self.builder
            .build_switch(tag, default_bb, &cases)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // PHI in cont.
        self.builder.position_at_end(cont_bb);
        let phi = self
            .builder
            .build_phi(ptr_t, "enum.ts.phi")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let mut incoming: Vec<(
            &dyn inkwell::values::BasicValue<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
        )> = Vec::new();
        for (_, bb, acc) in &variant_blocks {
            incoming.push((acc, *bb));
        }
        incoming.push((&default_acc, default_bb));
        phi.add_incoming(&incoming);
        Ok(phi.as_basic_value())
    }

    /// Inline lotus_str_concat call. Caller's arena owns the
    /// result.
    fn str_concat(
        &mut self,
        a: BasicValueEnum<'ctx>,
        b: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let arena_ptr = self.current_arena_ptr()?;
        let f = self
            .module
            .get_function("lotus_str_concat")
            .expect("lotus_str_concat declared");
        let res = self
            .builder
            .build_call(
                f,
                &[
                    arena_ptr.into(),
                    a.into_pointer_value().into(),
                    b.into_pointer_value().into(),
                ],
                "str.concat",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_str_concat returns ptr");
        Ok(res)
    }

    /// Build the equality comparison used by Literal patterns
    /// (top-level and tuple sub-pattern). Int / Duration / Bool
    /// use integer EQ; Float / Decimal use ordered float EQ. Other
    /// types (String, Time, LocusRef, TypeRef, Array, Tuple) are
    /// not first-class match-on-literal targets in v0; the typecheck
    /// ahead of codegen rejects them already.
    fn lower_match_eq_cmp(
        &mut self,
        scrut_val: BasicValueEnum<'ctx>,
        lit_val: BasicValueEnum<'ctx>,
        ty: &CodegenTy,
        name: &str,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        match ty {
            CodegenTy::Int | CodegenTy::Duration | CodegenTy::Bool | CodegenTy::Decimal => self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::EQ,
                    scrut_val.into_int_value(),
                    lit_val.into_int_value(),
                    name,
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string())),
            CodegenTy::Float => self
                .builder
                .build_float_compare(
                    inkwell::FloatPredicate::OEQ,
                    scrut_val.into_float_value(),
                    lit_val.into_float_value(),
                    name,
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string())),
            // m47-payloads-followup: String equality goes through
            // the lotus_str_eq runtime helper. Used both by match
            // arms with String literals and by enum-deep-eq when
            // a payload field is String-typed.
            CodegenTy::String | CodegenTy::Time => {
                let eq_fn = self
                    .module
                    .get_function("lotus_str_eq")
                    .expect("lotus_str_eq declared");
                let raw = self
                    .builder
                    .build_call(
                        eq_fn,
                        &[
                            scrut_val.into_pointer_value().into(),
                            lit_val.into_pointer_value().into(),
                        ],
                        name,
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_str_eq returns i32");
                let one = self.context.i32_type().const_int(1, false);
                self.builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        raw.into_int_value(),
                        one,
                        &format!("{}.bool", name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))
            }
            other => Err(CodegenError::Unsupported(format!(
                "match on {:?} not supported in v0",
                other
            ))),
        }
    }

    /// Lower `match scrutinee { arms... }`. v0 patterns:
    /// - `Literal` (Int / Bool / Duration / Decimal / Float / String) —
    ///   compare scrutinee against literal value, branch.
    /// - `Wildcard` — unconditional fallthrough into arm body.
    /// - `Binding(x)` — unconditional fallthrough; binds the
    ///   scrutinee value as a local named `x` for the arm body.
    ///
    /// Tuple / Constructor patterns land later; codegen rejects
    /// them today. F.18 exhaustiveness is checked upstream by the
    /// typechecker, so a non-matching scrutinee at runtime falls
    /// through with no behavior (interpreter behaves the same —
    /// match is statement-shape, not a typed expression that must
    /// produce a value).
    ///
    /// Match arm guards (`pattern if cond -> body`) lower as: the
    /// pattern test routes to a guard-check block (or directly to
    /// body if no guard); the guard-check block installs any
    /// binding the pattern declared, evaluates the guard, and
    /// cond-branches to body (true) or next-arm (false). The
    /// binding is visible to the guard expression — that's the
    /// whole point.
    fn lower_match_stmt(
        &mut self,
        m: &MatchStmt,
        scope: &mut Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        let (scrutinee_val, scrutinee_ty) = self.lower_expr(&m.scrutinee, scope)?;
        let func = self.current_fn.expect("match outside function");
        let after_bb = self.context.append_basic_block(func, "match.after");

        let mut all_terminated = true;

        for (i, arm) in m.arms.iter().enumerate() {
            let body_bb = self
                .context
                .append_basic_block(func, &format!("match.arm{}.body", i));
            let next_bb = self
                .context
                .append_basic_block(func, &format!("match.arm{}.next", i));
            // For guarded arms, insert a guard_bb between pattern
            // test and body; binding install lives there so the
            // guard expression can reference it. For non-guarded
            // arms, binding install happens at body_bb (existing
            // behavior preserved exactly).
            let has_guard = arm.guard.is_some();
            let guard_bb = if has_guard {
                Some(
                    self.context
                        .append_basic_block(func, &format!("match.arm{}.guard", i)),
                )
            } else {
                None
            };
            let pattern_target = guard_bb.unwrap_or(body_bb);

            // Track bindings the arm body needs to see. Each entry
            // is (name, value-to-store, type). For the m24/m29
            // surface (Wildcard / Binding / Literal) this list has
            // at most one entry; m35 tuple patterns introduce
            // arbitrary sub-bindings, one per sub-pattern's
            // Binding sub-pattern. Entries are applied at
            // pattern_target's start (guard_bb if guarded,
            // else body_bb).
            let mut bindings: Vec<(String, BasicValueEnum<'ctx>, CodegenTy)> =
                Vec::new();

            match &arm.pattern {
                Pattern::Wildcard(_) => {
                    self.builder
                        .build_unconditional_branch(pattern_target)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Pattern::Binding(ident) => {
                    bindings.push((
                        ident.name.clone(),
                        scrutinee_val,
                        scrutinee_ty.clone(),
                    ));
                    self.builder
                        .build_unconditional_branch(pattern_target)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Pattern::Literal(lit, span) => {
                    let (lit_val, lit_ty) =
                        self.lower_expr(&Expr::Literal(lit.clone(), *span), scope)?;
                    if lit_ty != scrutinee_ty {
                        return Err(CodegenError::Unsupported(format!(
                            "match arm pattern type {:?} doesn't match \
                             scrutinee type {:?}",
                            lit_ty, scrutinee_ty
                        )));
                    }
                    let cond = self.lower_match_eq_cmp(
                        scrutinee_val,
                        lit_val,
                        &lit_ty,
                        &format!("match.arm{}.cmp", i),
                    )?;
                    self.builder
                        .build_conditional_branch(cond, pattern_target, next_bb)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Pattern::Tuple(sub_patterns, _) => {
                    // m35: destructure scrutinee as a tuple, then
                    // apply each sub-pattern. v0 supports flat
                    // sub-patterns: Wildcard / Binding / Literal.
                    // Nested tuple patterns aren't needed for the
                    // shapes the language exposes today.
                    let elem_tys = match &scrutinee_ty {
                        CodegenTy::Tuple(ts) => ts.clone(),
                        other => {
                            return Err(CodegenError::Unsupported(format!(
                                "tuple pattern against non-tuple \
                                 scrutinee {:?}",
                                other
                            )));
                        }
                    };
                    if elem_tys.len() != sub_patterns.len() {
                        return Err(CodegenError::Unsupported(format!(
                            "tuple pattern arity {} != scrutinee tuple \
                             arity {}",
                            sub_patterns.len(),
                            elem_tys.len()
                        )));
                    }
                    let storage_ty = self.llvm_tuple_storage_type(&elem_tys);
                    let i32_t = self.context.i32_type();
                    let scrut_ptr = scrutinee_val.into_pointer_value();
                    let bool_t = self.context.bool_type();
                    let mut acc_cond: inkwell::values::IntValue<'ctx> =
                        bool_t.const_int(1, false);
                    for (j, sub) in sub_patterns.iter().enumerate() {
                        let slot = unsafe {
                            self.builder
                                .build_gep(
                                    storage_ty,
                                    scrut_ptr,
                                    &[
                                        i32_t.const_int(0, false),
                                        i32_t.const_int(j as u64, false),
                                    ],
                                    &format!("match.arm{}.tup.{}.ptr", i, j),
                                )
                                .map_err(|e| {
                                    CodegenError::LlvmEmit(e.to_string())
                                })?
                        };
                        let elem_llvm = self.llvm_basic_type(&elem_tys[j]);
                        let elem_val = self
                            .builder
                            .build_load(
                                elem_llvm,
                                slot,
                                &format!("match.arm{}.tup.{}", i, j),
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        match sub {
                            Pattern::Wildcard(_) => {}
                            Pattern::Binding(ident) => {
                                bindings.push((
                                    ident.name.clone(),
                                    elem_val,
                                    elem_tys[j].clone(),
                                ));
                            }
                            Pattern::Literal(lit, span) => {
                                let (lit_val, lit_ty) = self.lower_expr(
                                    &Expr::Literal(lit.clone(), *span),
                                    scope,
                                )?;
                                if lit_ty != elem_tys[j] {
                                    return Err(CodegenError::Unsupported(
                                        format!(
                                            "tuple sub-pattern at index {} \
                                             type {:?} doesn't match field \
                                             type {:?}",
                                            j, lit_ty, elem_tys[j]
                                        ),
                                    ));
                                }
                                let sub_cond = self.lower_match_eq_cmp(
                                    elem_val,
                                    lit_val,
                                    &lit_ty,
                                    &format!(
                                        "match.arm{}.tup.{}.cmp",
                                        i, j
                                    ),
                                )?;
                                acc_cond = self
                                    .builder
                                    .build_and(
                                        acc_cond,
                                        sub_cond,
                                        &format!(
                                            "match.arm{}.tup.{}.acc",
                                            i, j
                                        ),
                                    )
                                    .map_err(|e| {
                                        CodegenError::LlvmEmit(e.to_string())
                                    })?;
                            }
                            Pattern::Tuple(_, _)
                            | Pattern::Constructor { .. } => {
                                return Err(CodegenError::Unsupported(
                                    "nested tuple / constructor sub-pattern \
                                     not yet lowered"
                                        .into(),
                                ));
                            }
                        }
                    }
                    self.builder
                        .build_conditional_branch(acc_cond, pattern_target, next_bb)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Pattern::Constructor { path, args, .. } => {
                    // m47 + payloads: enum constructor pattern.
                    // Path must be `EnumName::VariantName`. Compare
                    // the scrutinee's tag to the variant's tag; if
                    // the variant has a payload AND the pattern
                    // carries args (Wildcard or Binding sub-
                    // patterns), bind each payload field to its
                    // local name. v0.1 sub-patterns are
                    // Wildcard / Binding only — nested literal
                    // / tuple / further constructor sub-patterns
                    // aren't lowered (parser doesn't even produce
                    // them with the current grammar).
                    if path.segments.len() != 2 {
                        return Err(CodegenError::Unsupported(format!(
                            "constructor pattern path must be \
                             `Enum::Variant` (got {} segments)",
                            path.segments.len()
                        )));
                    }
                    let enum_name = path.segments[0].name.clone();
                    let variant_name = &path.segments[1].name;
                    let info = self
                        .user_enums
                        .get(&enum_name)
                        .cloned()
                        .ok_or_else(|| {
                            CodegenError::Unsupported(format!(
                                "constructor pattern: unknown enum `{}`",
                                enum_name
                            ))
                        })?;
                    let variant_idx = info
                        .variants
                        .iter()
                        .position(|v| v.name == *variant_name)
                        .ok_or_else(|| {
                            CodegenError::Unsupported(format!(
                                "enum `{}` has no variant `{}`",
                                enum_name, variant_name
                            ))
                        })?;
                    if !matches!(scrutinee_ty, CodegenTy::Enum(ref n) if n == &enum_name)
                    {
                        return Err(CodegenError::Unsupported(format!(
                            "constructor pattern `{}::{}` against \
                             scrutinee of type {:?}",
                            enum_name, variant_name, scrutinee_ty
                        )));
                    }
                    let variant = &info.variants[variant_idx];
                    if !args.is_empty() && args.len() != variant.field_tys.len()
                    {
                        return Err(CodegenError::Unsupported(format!(
                            "constructor pattern `{}::{}` has {} arg(s); \
                             variant declares {}",
                            enum_name,
                            variant_name,
                            args.len(),
                            variant.field_tys.len()
                        )));
                    }
                    let i32_t = self.context.i32_type();
                    let tag_val = i32_t.const_int(variant_idx as u64, false);
                    // For has-payload enums the scrutinee value is
                    // a pointer; load the tag through it. For
                    // pure no-payload enums it's an i32 directly.
                    let actual_tag: inkwell::values::IntValue<'ctx> = if info
                        .has_payload
                    {
                        let enum_ptr = scrutinee_val.into_pointer_value();
                        self.load_enum_tag(&info, enum_ptr)?
                    } else {
                        scrutinee_val.into_int_value()
                    };
                    let cond = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            actual_tag,
                            tag_val,
                            &format!(
                                "match.arm{}.enum.{}.cmp",
                                i, variant_name
                            ),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    // If pattern carries arg sub-patterns, load
                    // payload fields. Wildcard skips, Binding
                    // pushes a binding, Literal AND-extends the
                    // tag-eq cond with a field-vs-literal compare
                    // (so `Event::Tick(0) -> ...` only matches
                    // when the payload's first field equals 0).
                    // Other sub-pattern forms (Tuple / nested
                    // Constructor) aren't lowered at v0.1.
                    let mut acc_cond = cond;
                    if !args.is_empty() {
                        let enum_ptr = scrutinee_val.into_pointer_value();
                        let fields = self
                            .load_enum_payload_fields(&info, enum_ptr, variant_idx)?;
                        for (j, sub) in args.iter().enumerate() {
                            match sub {
                                Pattern::Wildcard(_) => {}
                                Pattern::Binding(ident) => {
                                    let (val, ty) = fields[j].clone();
                                    bindings.push((ident.name.clone(), val, ty));
                                }
                                Pattern::Literal(lit, span) => {
                                    let (lit_val, lit_ty) = self.lower_expr(
                                        &Expr::Literal(lit.clone(), *span),
                                        scope,
                                    )?;
                                    let (field_val, field_ty) = fields[j].clone();
                                    if lit_ty != field_ty {
                                        return Err(CodegenError::Unsupported(
                                            format!(
                                                "constructor pattern arg {} \
                                                 literal type {:?} doesn't \
                                                 match payload field type {:?}",
                                                j, lit_ty, field_ty
                                            ),
                                        ));
                                    }
                                    let sub_cond = self.lower_match_eq_cmp(
                                        field_val,
                                        lit_val,
                                        &field_ty,
                                        &format!(
                                            "match.arm{}.enum.{}.field{}.cmp",
                                            i, variant_name, j
                                        ),
                                    )?;
                                    acc_cond = self
                                        .builder
                                        .build_and(
                                            acc_cond,
                                            sub_cond,
                                            &format!(
                                                "match.arm{}.enum.{}.acc",
                                                i, variant_name
                                            ),
                                        )
                                        .map_err(|e| {
                                            CodegenError::LlvmEmit(e.to_string())
                                        })?;
                                }
                                _ => {
                                    return Err(CodegenError::Unsupported(
                                        "constructor pattern arg must be a \
                                         binding, `_`, or literal at v0.1"
                                            .into(),
                                    ));
                                }
                            }
                        }
                    }
                    self.builder
                        .build_conditional_branch(acc_cond, pattern_target, next_bb)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
            }

            // Helper closure semantics inlined: install the
            // bindings list (alloca, store, scope.insert) and
            // return the saved priors so we can restore on arm
            // exit.
            let install_bindings = |this: &mut Self,
                                    scope: &mut Scope<'ctx>,
                                    bindings: &[(
                String,
                BasicValueEnum<'ctx>,
                CodegenTy,
            )]|
             -> Result<
                Vec<(String, Option<(PointerValue<'ctx>, CodegenTy)>)>,
                CodegenError,
            > {
                let mut out = Vec::with_capacity(bindings.len());
                for (bname, bval, bty) in bindings {
                    let alloca = this.alloca_for(bty, bname)?;
                    this.builder
                        .build_store(alloca, *bval)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let prior = scope.locals.insert(
                        bname.clone(),
                        (alloca, bty.clone()),
                    );
                    out.push((bname.clone(), prior));
                }
                Ok(out)
            };

            let saved: Vec<(String, Option<(PointerValue<'ctx>, CodegenTy)>)>;
            if let (Some(gbb), Some(guard_expr)) = (guard_bb, arm.guard.as_ref()) {
                // Guard-check path: install binding so the guard
                // can see it, evaluate the guard, cond-branch.
                self.builder.position_at_end(gbb);
                saved = install_bindings(self, scope, &bindings)?;
                let (gv, gty) = self.lower_expr(guard_expr, scope)?;
                if gty != CodegenTy::Bool {
                    return Err(CodegenError::Unsupported(format!(
                        "match arm guard must have type Bool, got {:?}",
                        gty
                    )));
                }
                self.builder
                    .build_conditional_branch(
                        gv.into_int_value(),
                        body_bb,
                        next_bb,
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder.position_at_end(body_bb);
            } else {
                self.builder.position_at_end(body_bb);
                saved = install_bindings(self, scope, &bindings)?;
            }

            let body_end = match &arm.body {
                MatchArmBody::Expr(e) => {
                    // Statement-position match: arm body is treated
                    // like a Stmt::Expr (value discarded). Route
                    // call exprs through `lower_stmt` so calls to
                    // builtins (println) and void-returning user
                    // fns are handled by the existing dispatch
                    // table; other expressions go through
                    // `lower_expr` and have their value dropped.
                    if matches!(e, Expr::Call { .. }) {
                        let s = Stmt::Expr(e.clone());
                        self.lower_stmt(&s, scope)?
                    } else {
                        let _ = self.lower_expr(e, scope)?;
                        BlockEnd::Open
                    }
                }
                MatchArmBody::Block(b) => self.lower_block(b, scope)?,
            };

            // Restore previous bindings (if any were shadowed) in
            // reverse order so a Pattern that introduced two
            // bindings of the same name (rare but legal) restores
            // to the original outer binding.
            for (bname, prior) in saved.into_iter().rev() {
                match prior {
                    Some(p) => {
                        scope.locals.insert(bname, p);
                    }
                    None => {
                        scope.locals.remove(&bname);
                    }
                }
            }

            if body_end == BlockEnd::Open {
                self.builder
                    .build_unconditional_branch(after_bb)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                all_terminated = false;
            }

            // Continue lowering at next_bb (where the next arm's
            // test will go, or — after the loop — the fallthrough).
            self.builder.position_at_end(next_bb);
        }

        // Fallthrough: no arm matched. v0 mirrors the interpreter
        // (silent no-op) — F.18 exhaustiveness is enforced at
        // typecheck, so this path is unreachable for well-typed
        // programs.
        self.builder
            .build_unconditional_branch(after_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // The fallthrough adds a path to after_bb, so the block
        // is reachable even if every arm body terminated.
        let _ = all_terminated;
        self.builder.position_at_end(after_bb);
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
        // Three iterator shapes today: `self.children` (built-in
        // fixed-cap array on accept-declaring loci),
        // `lo..hi` / `lo..=hi` integer ranges (counted loop), and
        // arbitrary expressions of CodegenTy::Array.
        let is_self_children = matches!(iter, Expr::Field { receiver, name, .. }
            if matches!(receiver.as_ref(), Expr::KwSelf(_))
                && name.name == "children");
        if let Expr::Range { lo, hi, inclusive, .. } = iter {
            return self.lower_for_range(var_name, lo, hi, *inclusive, body, scope);
        }
        if !is_self_children {
            return self.lower_for_array(var_name, iter, body, scope);
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
            (local_slot, CodegenTy::LocusRef(child_locus)),
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

    /// `for i in lo..hi` (exclusive) or `lo..=hi` (inclusive)
    /// integer range. Lowers as a counted loop: i = lo; while
    /// i < hi (or <=); body; i = i + 1. Both bounds are
    /// evaluated once at loop entry — modifying lo/hi inside the
    /// body has no effect on the iteration count.
    fn lower_for_range(
        &mut self,
        var_name: &Ident,
        lo: &Expr,
        hi: &Expr,
        inclusive: bool,
        body: &Block,
        scope: &mut Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        let (lo_val, lo_ty) = self.lower_expr(lo, scope)?;
        let (hi_val, hi_ty) = self.lower_expr(hi, scope)?;
        if lo_ty != CodegenTy::Int || hi_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "for-range bounds must be Int (got {:?}..{:?})",
                lo_ty, hi_ty
            )));
        }

        let i64_t = self.context.i64_type();
        let func = self
            .current_fn
            .expect("current_fn set while lowering a for");
        let header_bb = self.context.append_basic_block(func, "for.range.cond");
        let body_bb = self.context.append_basic_block(func, "for.range.body");
        let inc_bb = self.context.append_basic_block(func, "for.range.inc");
        let exit_bb = self.context.append_basic_block(func, "for.range.end");

        let i_slot = self
            .builder
            .build_alloca(i64_t, "for.range.i.slot")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(i_slot, lo_val.into_int_value())
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Stash the upper bound in a slot too so the user binding
        // for `i` doesn't share a name with our loop's bookkeeping.
        let hi_slot = self
            .builder
            .build_alloca(i64_t, "for.range.hi.slot")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(hi_slot, hi_val.into_int_value())
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(header_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(header_bb);
        let i = self
            .builder
            .build_load(i64_t, i_slot, "for.range.i")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let hi_v = self
            .builder
            .build_load(i64_t, hi_slot, "for.range.hi")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let pred = if inclusive {
            inkwell::IntPredicate::SLE
        } else {
            inkwell::IntPredicate::SLT
        };
        let in_range = self
            .builder
            .build_int_compare(pred, i, hi_v, "for.range.in")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(in_range, body_bb, exit_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let local_slot = self.alloca_for(&CodegenTy::Int, &var_name.name)?;
        self.builder
            .build_store(local_slot, i)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let prev = scope
            .locals
            .insert(var_name.name.clone(), (local_slot, CodegenTy::Int));
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
        if let Some(prev) = prev {
            scope.locals.insert(var_name.name.clone(), prev);
        } else {
            scope.locals.remove(&var_name.name);
        }

        self.builder.position_at_end(inc_bb);
        let i_now = self
            .builder
            .build_load(i64_t, i_slot, "for.range.i.now")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let i_next = self
            .builder
            .build_int_add(
                i_now,
                i64_t.const_int(1, false),
                "for.range.i.next",
            )
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

    /// `for x in arr` over a CodegenTy::Array iterator. Iterates
    /// 0..N where N is the array's static size, GEPs each slot,
    /// loads it, binds it as `x` for the body.
    fn lower_for_array(
        &mut self,
        var_name: &Ident,
        iter: &Expr,
        body: &Block,
        scope: &mut Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        let (arr_val, arr_ty) = self.lower_expr(iter, scope)?;
        let (elem_ty, n) = match arr_ty {
            CodegenTy::Array(elem, n) => (*elem, n),
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "for-loop iterator must be an array (got {:?})",
                    other
                )));
            }
        };
        let arr_ptr = arr_val.into_pointer_value();

        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let storage_ty = self.llvm_array_storage_type(&elem_ty, n);
        let elem_llvm = self.llvm_basic_type(&elem_ty);

        let func = self
            .current_fn
            .expect("current_fn set while lowering a for");
        let header_bb = self.context.append_basic_block(func, "for.arr.cond");
        let body_bb = self.context.append_basic_block(func, "for.arr.body");
        let inc_bb = self.context.append_basic_block(func, "for.arr.inc");
        let exit_bb = self.context.append_basic_block(func, "for.arr.end");

        let i_slot = self
            .builder
            .build_alloca(i64_t, "for.arr.i.slot")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(i_slot, i64_t.const_int(0, false))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(header_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(header_bb);
        let i = self
            .builder
            .build_load(i64_t, i_slot, "for.arr.i")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let in_range = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::ULT,
                i,
                i64_t.const_int(n, false),
                "for.arr.in.range",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(in_range, body_bb, exit_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let slot_ptr = unsafe {
            self.builder
                .build_gep(
                    storage_ty,
                    arr_ptr,
                    &[i32_t.const_int(0, false), i],
                    "for.arr.slot.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        let elem_val = self
            .builder
            .build_load(elem_llvm, slot_ptr, "for.arr.elem")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let local_slot = self.alloca_for(&elem_ty, &var_name.name)?;
        self.builder
            .build_store(local_slot, elem_val)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let prev = scope.locals.insert(
            var_name.name.clone(),
            (local_slot, elem_ty.clone()),
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
        if let Some(prev) = prev {
            scope.locals.insert(var_name.name.clone(), prev);
        } else {
            scope.locals.remove(&var_name.name);
        }

        self.builder.position_at_end(inc_bb);
        let i_now = self
            .builder
            .build_load(i64_t, i_slot, "for.arr.i.now")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let i_next = self
            .builder
            .build_int_add(
                i_now,
                i64_t.const_int(1, false),
                "for.arr.i.next",
            )
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
            self.emit_bus_queue_destroy()?;
            // Re-open an empty frame so the post-flush bookkeeping
            // (popped in lower_program) stays balanced.
            self.push_dissolve_frame();
            let i32_t = self.context.i32_type();
            let code = match expr {
                None => i32_t.const_int(0, false),
                Some(e) => {
                    let (v, ty) = self.lower_expr(e, scope)?;
                    if ty != CodegenTy::Int {
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
        // m49: when lowering a *free fn* body, `return` stores
        // into the fn's ret_alloca (if typed) and br's to fn.exit
        // so the unified epilogue runs (deep-copy + subregion
        // destroy). Direct `build_return` here would skip both.
        // Lifecycle methods (mode, run, accept, ...) set
        // `current_user_fn_ret` but not `current_user_fn_exit_bb`
        // — they don't own a per-call subregion, so `return` from
        // them stays a direct build_return as before.
        let in_free_fn = self.current_user_fn_exit_bb.is_some();
        match (expr, ret_ty) {
            (None, None) => {
                if in_free_fn {
                    let exit_bb = self.current_user_fn_exit_bb.unwrap();
                    self.builder
                        .build_unconditional_branch(exit_bb)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                } else {
                    self.builder
                        .build_return(None)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
            }
            (Some(e), Some(declared_ty)) => {
                // m67: rewrite a bare-name struct literal in return
                // position to its mangled monomorph using the fn's
                // declared return type as the target.
                let rewritten;
                let e_to_lower: &Expr = match e {
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
                            None => e,
                        }
                    }
                    _ => e,
                };
                let (v, got_ty) = self.lower_expr(e_to_lower, scope)?;
                if got_ty != declared_ty {
                    return Err(CodegenError::Unsupported(format!(
                        "return type mismatch: declared {:?}, got {:?}",
                        declared_ty, got_ty
                    )));
                }
                if in_free_fn {
                    let ret_alloca = self
                        .current_user_fn_ret_alloca
                        .expect("ret_alloca set when ret type is Some in free fn");
                    self.builder
                        .build_store(ret_alloca, v)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let exit_bb = self.current_user_fn_exit_bb.unwrap();
                    self.builder
                        .build_unconditional_branch(exit_bb)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                } else {
                    self.builder
                        .build_return(Some(&v))
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
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
        ty: &CodegenTy,
        name: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        match ty {
            CodegenTy::Int | CodegenTy::Duration => self
                .builder
                .build_alloca(self.context.i64_type(), name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string())),
            CodegenTy::FnPtr { .. } => self
                .builder
                .build_alloca(
                    self.context.ptr_type(AddressSpace::default()),
                    name,
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string())),
            CodegenTy::Float => self
                .builder
                .build_alloca(self.context.f64_type(), name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string())),
            CodegenTy::Decimal => self
                .builder
                .build_alloca(self.context.i128_type(), name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string())),
            CodegenTy::Bool => self
                .builder
                .build_alloca(self.context.bool_type(), name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string())),
            CodegenTy::Enum(en) => {
                let payload = self
                    .user_enums
                    .get(en.as_str())
                    .map(|i| i.has_payload)
                    .unwrap_or(false);
                if payload {
                    self.builder
                        .build_alloca(self.context.ptr_type(AddressSpace::default()), name)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))
                } else {
                    self.builder
                        .build_alloca(self.context.i32_type(), name)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))
                }
            }
            CodegenTy::String
            | CodegenTy::Bytes
            | CodegenTy::Time
            | CodegenTy::LocusRef(_)
            | CodegenTy::TypeRef(_)
            | CodegenTy::Array(_, _)
            | CodegenTy::Tuple(_)
            | CodegenTy::Interface(_)
            | CodegenTy::Cell(_, _) => self
                .builder
                .build_alloca(self.context.ptr_type(AddressSpace::default()), name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string())),
        }
    }

    fn lower_expr(
        &mut self,
        e: &Expr,
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        match e {
            // m46: `sum(expr)` inside a closure assertion is the
            // accumulator load (sample-update already ran);
            // outside a closure assertion it is rejected (the
            // batch array-reduction `sum(arr)` is interpreter-
            // only at v0 — codegen never supported it).
            Expr::Sum(_, _) => {
                if self.accumulator_ctx.is_some() {
                    self.lower_accumulator_load()
                } else {
                    Err(CodegenError::Unsupported(
                        "`sum(...)` outside a closure assertion is not \
                         supported in codegen v0".into(),
                    ))
                }
            }
            Expr::Literal(Literal::Int(n), _) => {
                let v = self.context.i64_type().const_int(*n as u64, true);
                Ok((v.into(), CodegenTy::Int))
            }
            Expr::Literal(Literal::Float(f), _) => {
                let v = self.context.f64_type().const_float(*f);
                Ok((v.into(), CodegenTy::Float))
            }
            Expr::Literal(Literal::Bool(b), _) => {
                let v = self.context.bool_type().const_int(*b as u64, false);
                Ok((v.into(), CodegenTy::Bool))
            }
            Expr::Literal(Literal::String(s), _) => {
                let p = self.global_string(s);
                Ok((p.into(), CodegenTy::String))
            }
            Expr::Literal(Literal::Duration(ns), _) => {
                // Duration literals are i64 nanoseconds at the
                // lowered level; tracked as Duration so callers
                // like `time::sleep` enforce the typed contract.
                let v = self.context.i64_type().const_int(*ns as u64, true);
                Ok((v.into(), CodegenTy::Duration))
            }
            Expr::Literal(Literal::Decimal(s), _) => {
                // m48: lower Decimal literals to i128 mantissa
                // with fixed scale 9 (mantissa × 10^-9). Per-value
                // scale lives only in the interpreter; codegen
                // picks one fixed scale so add/sub stay i128 ops
                // without mantissa alignment, and mul/div compose
                // via single division by 10^9. Source spelling
                // round-trips through the runtime print helper
                // because trailing zeros are trimmed at display.
                let mantissa = parse_decimal_to_i128_scale9(s).ok_or_else(|| {
                    CodegenError::Unsupported(format!(
                        "Decimal literal `{}` failed to parse",
                        s
                    ))
                })?;
                let v = i128_const(self.context, mantissa);
                Ok((v.into(), CodegenTy::Decimal))
            }
            Expr::Literal(Literal::Time(s), _) => {
                // v0 codegen mirrors the interpreter: store the
                // source spelling as a NUL-terminated global. Real
                // i64-since-epoch arithmetic lands later.
                let p = self.global_string(s);
                Ok((p.into(), CodegenTy::Time))
            }
            Expr::Path(qn) => {
                // m47 + payloads: enum variant construction
                // `EnumName::Variant`. For pure no-payload enums
                // the value is just the i32 tag. For
                // payload-bearing enums (any variant has fields),
                // a no-payload variant still allocates the
                // unified `{i32, [N x i8]}` struct in the current
                // arena and stores the tag — matching the
                // representation a payload variant would
                // produce, so callers don't have to care which
                // variant they're holding.
                if qn.segments.len() == 2 {
                    let enum_name = qn.segments[0].name.clone();
                    let variant_name = &qn.segments[1].name;
                    if let Some(info) = self.user_enums.get(&enum_name).cloned() {
                        let tag = info
                            .variants
                            .iter()
                            .position(|v| v.name == *variant_name)
                            .ok_or_else(|| {
                                CodegenError::Unsupported(format!(
                                    "enum `{}` has no variant `{}`",
                                    enum_name, variant_name
                                ))
                            })?;
                        if info.has_payload {
                            let v = self.lower_enum_variant_alloc(
                                &info,
                                tag as u32,
                                &[],
                            )?;
                            return Ok((v.into(), CodegenTy::Enum(enum_name)));
                        }
                        let v = self
                            .context
                            .i32_type()
                            .const_int(tag as u64, false);
                        return Ok((v.into(), CodegenTy::Enum(enum_name)));
                    }
                }
                Err(CodegenError::Unsupported(format!(
                    "unresolved path `{}`",
                    qn.segments
                        .iter()
                        .map(|s| s.name.clone())
                        .collect::<Vec<_>>()
                        .join("::")
                )))
            }
            Expr::Ident(id) => {
                // m89-fix: locals shadow globals. Check scope
                // first; only fall back to user_fns (treating the
                // identifier as a fn-pointer value, m80) when no
                // local of that name exists. The reverse order
                // would let stdlib fns whose params happen to
                // share a name with a user-declared fn (e.g. a
                // user fn `b` and a stdlib param `b: Bytes`)
                // resolve the param to the global fn — exactly
                // the silent shadowing bug locals are meant to
                // prevent.
                if let Some((alloca, ty)) =
                    scope.locals.get(&id.name).cloned()
                {
                    let llvm_ty = self.llvm_basic_type(&ty);
                    let loaded = self
                        .builder
                        .build_load(llvm_ty, alloca, &id.name)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    return Ok((loaded, ty));
                }
                // m80: a bare identifier in expression position
                // can be a user function name used as a value.
                // The Expr::Call path that handles `handler(...)`
                // checks user_fns separately, so this only fires
                // when handler appears NOT as a callee.
                if let Some(sig) = self.user_fns.get(&id.name).cloned() {
                    let fn_ptr = sig
                        .func
                        .as_global_value()
                        .as_pointer_value();
                    let fn_ty = CodegenTy::FnPtr {
                        args: sig.params.clone(),
                        ret: sig.ret.clone().map(Box::new),
                    };
                    return Ok((fn_ptr.into(), fn_ty));
                }
                Err(CodegenError::Unsupported(format!(
                    "unknown identifier `{}`",
                    id.name
                )))
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
                // 3b: F.16 built-in `self.k_max`. Computed from
                // the locus's B / c / sigma / phi params on every
                // read so the bound floats with mutable params.
                // Formula: `k_max = B / [(1-phi)c + phi*sigma]`.
                // The interpreter computes the same expression in
                // `read_field` for Value::Locus; codegen lowers it
                // here so `aperio build` matches `aperio run` for
                // capacity-cascade demos. Int params are widened
                // to Float before the arithmetic; phi must already
                // be Float.
                if name.name == "k_max" {
                    let load_field = |this: &mut Self, fname: &str| {
                        let (fidx, fty) = cs
                            .fields
                            .get(fname)
                            .cloned()
                            .ok_or_else(|| {
                                CodegenError::Unsupported(format!(
                                    "self.k_max requires param `{}` on locus `{}`",
                                    fname, cs.locus_name
                                ))
                            })?;
                        let ptr = this
                            .builder
                            .build_struct_gep(
                                cs.struct_ty,
                                cs.self_ptr,
                                fidx,
                                &format!("self.{}.k_max.ptr", fname),
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        let llvm_ty = this.llvm_basic_type(&fty);
                        let val = this
                            .builder
                            .build_load(
                                llvm_ty,
                                ptr,
                                &format!("self.{}.k_max", fname),
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        Ok::<_, CodegenError>((val, fty))
                    };
                    let (b_v, b_ty) = load_field(self, "B")?;
                    let (c_v, c_ty) = load_field(self, "c")?;
                    let (sigma_v, sigma_ty) = load_field(self, "sigma")?;
                    let (phi_v, phi_ty) = load_field(self, "phi")?;
                    let b_f = self.coerce_to_float(b_v, &b_ty, "self.k_max.B")?;
                    let c_f = self.coerce_to_float(c_v, &c_ty, "self.k_max.c")?;
                    let sigma_f = self.coerce_to_float(
                        sigma_v,
                        &sigma_ty,
                        "self.k_max.sigma",
                    )?;
                    let phi_f = match phi_ty {
                        CodegenTy::Float => phi_v.into_float_value(),
                        other => {
                            return Err(CodegenError::Unsupported(format!(
                                "self.k_max requires param `phi` of type \
                                 Float, got {:?}",
                                other
                            )));
                        }
                    };
                    let f64_t = self.context.f64_type();
                    let one = f64_t.const_float(1.0);
                    let one_minus_phi = self
                        .builder
                        .build_float_sub(one, phi_f, "k_max.1mphi")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let term_left = self
                        .builder
                        .build_float_mul(one_minus_phi, c_f, "k_max.term_left")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let term_right = self
                        .builder
                        .build_float_mul(phi_f, sigma_f, "k_max.term_right")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let denom = self
                        .builder
                        .build_float_add(
                            term_left,
                            term_right,
                            "k_max.denom",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let k_max = self
                        .builder
                        .build_float_div(b_f, denom, "k_max")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    return Ok((k_max.into(), CodegenTy::Float));
                }
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
                // m35: numeric tuple field access (`t.0`, `t.1`).
                // The parser stores the digit string as the field
                // name; if the receiver is a tuple, GEP+load.
                if let CodegenTy::Tuple(elems) = &recv_ty {
                    let i = name.name.parse::<usize>().map_err(|_| {
                        CodegenError::Unsupported(format!(
                            "tuple field access expects a numeric index; \
                             got `.{}`",
                            name.name
                        ))
                    })?;
                    if i >= elems.len() {
                        return Err(CodegenError::Unsupported(format!(
                            "tuple field index {} out of range (arity {})",
                            i,
                            elems.len()
                        )));
                    }
                    let storage_ty = self.llvm_tuple_storage_type(elems);
                    let i32_t = self.context.i32_type();
                    let recv_ptr = recv_val.into_pointer_value();
                    let slot = unsafe {
                        self.builder
                            .build_gep(
                                storage_ty,
                                recv_ptr,
                                &[
                                    i32_t.const_int(0, false),
                                    i32_t.const_int(i as u64, false),
                                ],
                                &format!("tup.{}.ptr", i),
                            )
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    };
                    let elem_llvm = self.llvm_basic_type(&elems[i]);
                    let val = self
                        .builder
                        .build_load(elem_llvm, slot, &format!("tup.{}", i))
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    return Ok((val, elems[i].clone()));
                }
                let (struct_ty, fields, ref_kind) = match &recv_ty {
                    CodegenTy::LocusRef(n) => {
                        let info = self
                            .user_loci
                            .get(n)
                            .cloned()
                            .expect("LocusRef points to a declared locus");
                        (info.struct_ty, info.fields, format!("locus `{}`", n))
                    }
                    CodegenTy::TypeRef(n) => {
                        let info = self
                            .user_types
                            .get(n)
                            .cloned()
                            .expect("TypeRef points to a declared type");
                        (info.struct_ty, info.fields, format!("type `{}`", n))
                    }
                    // F.22 v1.x-2: `cell.field` reads a struct
                    // cell's field. Cell<T> is a *T at LLVM
                    // level, so GEP + load works identically to
                    // TypeRef(T) field access. Primitive cells
                    // (Cell<Int> etc.) don't have addressable
                    // fields — those reject below with a
                    // focused message.
                    CodegenTy::Cell(inner, _) => match inner.as_ref() {
                        CodegenTy::TypeRef(n) => {
                            let info = self
                                .user_types
                                .get(n)
                                .cloned()
                                .expect("Cell<TypeRef> points to declared type");
                            (
                                info.struct_ty,
                                info.fields,
                                format!("cell of type `{}`", n),
                            )
                        }
                        other => {
                            return Err(CodegenError::Unsupported(format!(
                                "field access on Cell<{:?}>: only struct cells \
                                 expose fields at v1; primitive-cell content \
                                 access is a v1.x follow-up",
                                other
                            )));
                        }
                    },
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
                // Ergonomics arc: `String + <printable>` and
                // `<printable> + String` auto-coerce the non-String
                // side via value_to_string. Resolves the apps/tcp-echo
                // friction "String + Int rejected even though
                // println('p=', n) works." Other mixed-type binops
                // remain errors.
                if *op == BinOp::Add && lt != rt {
                    if lt == CodegenTy::String && Self::value_to_string_supports(&rt) {
                        let coerced = self.value_to_string(rv, &rt)?;
                        return self.lower_binop(*op, lv, coerced, &CodegenTy::String);
                    }
                    if rt == CodegenTy::String && Self::value_to_string_supports(&lt) {
                        let coerced = self.value_to_string(lv, &lt)?;
                        return self.lower_binop(*op, coerced, rv, &CodegenTy::String);
                    }
                }
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
                // m46-vocab: count() / mean(x) accumulator builtins
                // — when an accumulator-eval ctx is active, route
                // to the next slot. count() takes 0 args; mean(x)
                // takes one. Outside a closure assertion, both fall
                // through to the generic user-fn path (which will
                // error since neither is declared).
                Expr::Ident(i)
                    if i.name == "count"
                        && args.is_empty()
                        && self.accumulator_ctx.is_some() =>
                {
                    self.lower_accumulator_load()
                }
                Expr::Ident(i)
                    if i.name == "mean"
                        && args.len() == 1
                        && self.accumulator_ctx.is_some() =>
                {
                    self.lower_accumulator_load()
                }
                Expr::Ident(i) if i.name == "len" => {
                    self.lower_len_builtin(args, scope)
                }
                Expr::Ident(i) if i.name == "to_string" => {
                    self.lower_to_string_builtin(args, scope)
                }
                Expr::Ident(i) if i.name == "Int" => {
                    // v1.x-11: explicit Float → Int narrowing.
                    self.lower_int_cast_builtin(args, scope)
                }
                Expr::Ident(i)
                    if matches!(
                        i.name.as_str(),
                        "min" | "max" | "abs"
                    ) =>
                {
                    self.lower_math_builtin(&i.name, args, scope)
                }
                Expr::Ident(i)
                    if matches!(
                        i.name.as_str(),
                        "starts_with" | "contains"
                    ) =>
                {
                    self.lower_str_predicate_builtin(&i.name, args, scope)
                }
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
                Expr::Ident(i)
                    if self.generic_fn_templates.contains_key(&i.name) =>
                {
                    let result =
                        self.lower_generic_fn_call(&i.name, args, scope)?;
                    result.ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "generic fn `{}` returns no value but is \
                             used in expression position",
                            i.name
                        ))
                    })
                }
                // m83: local-variable fn-pointer call in expression
                // position. Mirrors the statement-position arm above
                // but keeps the call result. A non-value-returning
                // FnPtr in expression position is an error (you can't
                // bind void to a let).
                Expr::Ident(i)
                    if matches!(
                        scope.locals.get(&i.name).map(|(_, t)| t),
                        Some(CodegenTy::FnPtr { .. })
                    ) =>
                {
                    let (slot_ptr, fn_ty) = scope
                        .locals
                        .get(&i.name)
                        .cloned()
                        .expect("matched FnPtr above");
                    let (arg_tys, ret_ty) = match fn_ty {
                        CodegenTy::FnPtr { args, ret } => (args, ret),
                        _ => unreachable!(),
                    };
                    let ptr_t = self
                        .context
                        .ptr_type(AddressSpace::default());
                    let fn_value_ptr = self
                        .builder
                        .build_load(ptr_t, slot_ptr, &i.name)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .into_pointer_value();
                    let result = self.emit_fnptr_indirect_call(
                        fn_value_ptr,
                        &arg_tys,
                        ret_ty.as_deref(),
                        args,
                        scope,
                        &i.name,
                    )?;
                    result.ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "fn-pointer `{}` returns no value but is used \
                             in expression position",
                            i.name
                        ))
                    })
                }
                Expr::Path(qn) => self.lower_path_call_expr(qn, args, scope),
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
                Expr::Field { receiver, name, .. } => {
                    // m81: external method call in expression
                    // position. Same shape as the stmt arm but
                    // requires the method to return a value.
                    let result = self.lower_external_method_call(
                        receiver, &name.name, args, scope,
                    )?;
                    result.ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "method `{}` returns no value but is used in \
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
                Ok((ptr.into(), CodegenTy::LocusRef(name)))
            }
            // m81 + m84: path-qualified stdlib literal in expression
            // position. `std::io::tcp::Stream { conn_fd }` resolves
            // to a locus → LocusRef; `std::http::Request { ... }`
            // resolves to a `type` (record) → TypeRef. The mangled-
            // name lookup is shared; the dispatch (locus vs type)
            // follows whichever map the mangled name lives in.
            Expr::Struct { path, inits, .. }
                if path.segments.len() > 1 => {
                let segs: Vec<&str> = path
                    .segments
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect();
                let mangled = stdlib_mangled_for_path(&segs).ok_or_else(|| {
                    CodegenError::Unsupported(format!(
                        "qualified-name struct literal `{}` in expression position",
                        segs.join("::")
                    ))
                })?;
                if self.user_loci.contains_key(mangled) {
                    let ptr = self.lower_locus_instantiation(mangled, inits, scope)?;
                    Ok((ptr.into(), CodegenTy::LocusRef(mangled.to_string())))
                } else if self.user_types.contains_key(mangled) {
                    let ptr = self.lower_user_type_instantiation(mangled, inits, scope)?;
                    Ok((ptr.into(), CodegenTy::TypeRef(mangled.to_string())))
                } else {
                    Err(CodegenError::Unsupported(format!(
                        "stdlib path `{}` (mangled `{}`) not found in \
                         user_loci or user_types",
                        segs.join("::"),
                        mangled
                    )))
                }
            }
            Expr::Struct { path, inits, .. }
                if path.segments.len() == 1
                    && self.user_types.contains_key(&path.segments[0].name) =>
            {
                let name = path.segments[0].name.clone();
                let ptr = self.lower_user_type_instantiation(&name, inits, scope)?;
                Ok((ptr.into(), CodegenTy::TypeRef(name)))
            }
            Expr::Array(parts, _) => {
                // Lower an array literal `[a, b, c]` into an arena
                // allocation of size N * sizeof(elem). Element types
                // come from the first element; subsequent elements
                // are typechecked already so we only verify the
                // first dictates the type. Empty arrays would need
                // an ascription to know the element type — reject
                // them in v0.
                if parts.is_empty() {
                    return Err(CodegenError::Unsupported(
                        "empty array literal needs a type ascription \
                         (not yet supported in v0)"
                            .into(),
                    ));
                }
                let mut elem_vals: Vec<BasicValueEnum<'ctx>> =
                    Vec::with_capacity(parts.len());
                let mut elem_ty: Option<CodegenTy> = None;
                for p in parts {
                    let (v, t) = self.lower_expr(p, scope)?;
                    if let Some(prev) = &elem_ty {
                        if prev != &t {
                            return Err(CodegenError::Unsupported(format!(
                                "array literal mixes element types \
                                 ({:?} and {:?})",
                                prev, t
                            )));
                        }
                    } else {
                        elem_ty = Some(t);
                    }
                    elem_vals.push(v);
                }
                let elem_ty = elem_ty.expect("non-empty array has a type");
                let n = elem_vals.len() as u64;
                let i32_t = self.context.i32_type();
                let arr_ty = self.llvm_array_storage_type(&elem_ty, n);
                let bytes = arr_ty
                    .size_of()
                    .expect("array storage type has known size");
                let arr_ptr =
                    self.arena_alloc(bytes, "array.literal.alloc")?;
                for (i, v) in elem_vals.iter().enumerate() {
                    let slot = unsafe {
                        self.builder
                            .build_gep(
                                arr_ty,
                                arr_ptr,
                                &[
                                    i32_t.const_int(0, false),
                                    i32_t.const_int(i as u64, false),
                                ],
                                &format!("array.lit.slot{}", i),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?
                    };
                    self.builder
                        .build_store(slot, *v)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Ok((arr_ptr.into(), CodegenTy::Array(Box::new(elem_ty), n)))
            }
            Expr::ArrayRepeat { val, count, .. } => {
                // `[val; N]` — evaluate val once, fill N slots
                // with the result. Same arena-allocation pattern
                // as Expr::Array; the difference is just the
                // single source value broadcast across the slots.
                // Parser already enforced `count` is a non-negative
                // Int literal.
                let n = *count;
                if n == 0 {
                    return Err(CodegenError::Unsupported(
                        "array-repeat with zero count is not yet \
                         supported in v0 (empty arrays need a type \
                         ascription mechanism)".into(),
                    ));
                }
                let (v, elem_ty) = self.lower_expr(val, scope)?;
                let i32_t = self.context.i32_type();
                let arr_ty = self.llvm_array_storage_type(&elem_ty, n);
                let bytes = arr_ty
                    .size_of()
                    .expect("array storage type has known size");
                let arr_ptr =
                    self.arena_alloc(bytes, "array.repeat.alloc")?;
                for i in 0..n {
                    let slot = unsafe {
                        self.builder
                            .build_gep(
                                arr_ty,
                                arr_ptr,
                                &[
                                    i32_t.const_int(0, false),
                                    i32_t.const_int(i, false),
                                ],
                                &format!("array.rep.slot{}", i),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?
                    };
                    self.builder
                        .build_store(slot, v)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Ok((arr_ptr.into(), CodegenTy::Array(Box::new(elem_ty), n)))
            }
            Expr::Tuple(parts, _) => {
                // m35: tuple literal `(a, b, ...)`. Lower each
                // component, allocate an anonymous struct in the
                // arena (sized to fit the components), and store
                // each component at its slot. Typecheck rejects
                // 0- and 1-element tuples upstream; we still
                // guard here defensively.
                if parts.len() < 2 {
                    return Err(CodegenError::Unsupported(format!(
                        "tuple literal must have at least 2 elements; \
                         got {}",
                        parts.len()
                    )));
                }
                let mut elem_vals: Vec<BasicValueEnum<'ctx>> =
                    Vec::with_capacity(parts.len());
                let mut elem_tys: Vec<CodegenTy> =
                    Vec::with_capacity(parts.len());
                for p in parts {
                    let (v, t) = self.lower_expr(p, scope)?;
                    elem_vals.push(v);
                    elem_tys.push(t);
                }
                let storage_ty = self.llvm_tuple_storage_type(&elem_tys);
                let bytes = storage_ty
                    .size_of()
                    .expect("tuple storage type has known size");
                let tup_ptr =
                    self.arena_alloc(bytes, "tuple.literal.alloc")?;
                let i32_t = self.context.i32_type();
                for (i, v) in elem_vals.iter().enumerate() {
                    let slot = unsafe {
                        self.builder
                            .build_gep(
                                storage_ty,
                                tup_ptr,
                                &[
                                    i32_t.const_int(0, false),
                                    i32_t.const_int(i as u64, false),
                                ],
                                &format!("tuple.lit.slot{}", i),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?
                    };
                    self.builder
                        .build_store(slot, *v)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
                Ok((tup_ptr.into(), CodegenTy::Tuple(elem_tys)))
            }
            Expr::Index { receiver, index, .. } => {
                // m36: range-indexed receivers do slicing, not
                // single-element indexing. Today only String
                // supports slicing — arrays return a fixed-size
                // sub-array would need a length field the v0
                // representation doesn't carry. Inclusive range
                // (`s[lo..=hi]`) maps to lotus_str_slice with hi+1
                // since the helper takes exclusive `hi`.
                if let Expr::Range { lo, hi, inclusive, .. } = index.as_ref() {
                    let (recv_val, recv_ty) = self.lower_expr(receiver, scope)?;
                    if !matches!(recv_ty, CodegenTy::String) {
                        return Err(CodegenError::Unsupported(format!(
                            "range slicing only supported on String in v0, \
                             not {:?}",
                            recv_ty
                        )));
                    }
                    let (lo_v, lo_t) = self.lower_expr(lo, scope)?;
                    let (hi_v, hi_t) = self.lower_expr(hi, scope)?;
                    if lo_t != CodegenTy::Int || hi_t != CodegenTy::Int {
                        return Err(CodegenError::Unsupported(format!(
                            "string slice bounds must be Int; got \
                             {:?}..{:?}",
                            lo_t, hi_t
                        )));
                    }
                    let hi_final = if *inclusive {
                        let one = self.context.i64_type().const_int(1, true);
                        self.builder
                            .build_int_add(hi_v.into_int_value(), one, "hi.incl")
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    } else {
                        hi_v.into_int_value()
                    };
                    let arena_ptr = self.current_arena_ptr()?;
                    let slice_fn = self
                        .module
                        .get_function("lotus_str_slice")
                        .expect("lotus_str_slice declared");
                    let v = self
                        .builder
                        .build_call(
                            slice_fn,
                            &[
                                arena_ptr.into(),
                                recv_val.into_pointer_value().into(),
                                lo_v.into_int_value().into(),
                                hi_final.into(),
                            ],
                            "str.slice",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .try_as_basic_value()
                        .left()
                        .expect("lotus_str_slice returns ptr");
                    return Ok((v, CodegenTy::String));
                }
                let (recv_val, recv_ty) = self.lower_expr(receiver, scope)?;
                let (idx_val, idx_ty) = self.lower_expr(index, scope)?;
                if idx_ty != CodegenTy::Int {
                    return Err(CodegenError::Unsupported(format!(
                        "array index must be Int, got {:?}",
                        idx_ty
                    )));
                }
                match recv_ty {
                    CodegenTy::Array(elem_ty, n) => {
                        let i32_t = self.context.i32_type();
                        let arr_ty = self.llvm_array_storage_type(&elem_ty, n);
                        let recv_ptr = recv_val.into_pointer_value();
                        let slot = unsafe {
                            self.builder
                                .build_gep(
                                    arr_ty,
                                    recv_ptr,
                                    &[
                                        i32_t.const_int(0, false),
                                        idx_val.into_int_value(),
                                    ],
                                    "array.index.slot",
                                )
                                .map_err(|e| {
                                    CodegenError::LlvmEmit(e.to_string())
                                })?
                        };
                        let elem_llvm = self.llvm_basic_type(&elem_ty);
                        let loaded = self
                            .builder
                            .build_load(elem_llvm, slot, "array.index.load")
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        Ok((loaded, *elem_ty))
                    }
                    other => Err(CodegenError::Unsupported(format!(
                        "indexing a non-array value (type {:?})",
                        other
                    ))),
                }
            }
            Expr::If(ifs) => self.lower_if_expr(ifs, scope),
            Expr::Block(block) => {
                let mut inner = Scope {
                    locals: scope.locals.clone(),
                };
                let (v, ty, _end) = self.lower_block_as_expr(block, &mut inner)?;
                Ok((v, ty))
            }
            _ => Err(CodegenError::Unsupported(format!(
                "expression form {:?}",
                std::mem::discriminant(e)
            ))),
        }
    }

    /// Lower an `if`-as-expression. The then- and else- blocks must
    /// each have a trailing expression; the values are phi-merged at
    /// the join point and the phi is the if-expression's value. An
    /// if without an else (e.g. `if cond { 1 }`) is rejected — there
    /// is no value to merge on the missing branch.
    fn lower_if_expr(
        &mut self,
        ifs: &IfStmt,
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let (cond_v, cond_ty) = self.lower_expr(&ifs.cond, scope)?;
        if cond_ty != CodegenTy::Bool {
            return Err(CodegenError::Unsupported(format!(
                "if condition must be Bool; got {:?}",
                cond_ty
            )));
        }
        let func = self
            .current_fn
            .expect("current_fn set while lowering an if-expression");
        let then_bb = self.context.append_basic_block(func, "ifx.then");
        let else_bb = self.context.append_basic_block(func, "ifx.else");
        let merge_bb = self.context.append_basic_block(func, "ifx.end");
        self.builder
            .build_conditional_branch(cond_v.into_int_value(), then_bb, else_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // then-branch: lower then-block as expr, capture incoming bb
        // for the phi (lowering may have created intermediate bbs).
        self.builder.position_at_end(then_bb);
        let mut then_scope = Scope {
            locals: scope.locals.clone(),
        };
        let (then_v, then_ty, then_end) =
            self.lower_block_as_expr(&ifs.then_block, &mut then_scope)?;
        let then_incoming = self.builder.get_insert_block().unwrap_or(then_bb);
        if then_end == BlockEnd::Open {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }

        // else-branch: an if-as-expression requires an else with a
        // trailing expression; lower it through the same expr helper.
        self.builder.position_at_end(else_bb);
        let mut else_scope = Scope {
            locals: scope.locals.clone(),
        };
        let (else_v, else_ty, else_end) = match &ifs.else_block {
            Some(eb) => match eb.as_ref() {
                ElseBranch::Else(b) => self.lower_block_as_expr(b, &mut else_scope)?,
                ElseBranch::ElseIf(nested) => {
                    let (v, ty) = self.lower_if_expr(nested, &else_scope)?;
                    (v, ty, BlockEnd::Open)
                }
            },
            None => {
                return Err(CodegenError::Unsupported(
                    "if-as-expression requires an else branch with a \
                     trailing expression"
                        .to_string(),
                ));
            }
        };
        let else_incoming = self.builder.get_insert_block().unwrap_or(else_bb);
        if else_end == BlockEnd::Open {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }

        if then_ty != else_ty
            && then_end == BlockEnd::Open
            && else_end == BlockEnd::Open
        {
            return Err(CodegenError::Unsupported(format!(
                "if-expression arms have mismatched types: then={:?}, else={:?}",
                then_ty, else_ty
            )));
        }

        // Both arms terminated — value path is unreachable. Build a
        // placeholder phi shape so callers can keep going, but mark
        // the merge block as unreachable.
        if then_end == BlockEnd::Terminated && else_end == BlockEnd::Terminated {
            self.builder.position_at_end(merge_bb);
            let undef = self.context.i32_type().get_undef();
            self.builder
                .build_unreachable()
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            return Ok((undef.into(), then_ty));
        }

        self.builder.position_at_end(merge_bb);
        let result_ty = if then_end == BlockEnd::Open {
            then_ty
        } else {
            else_ty
        };
        let llvm_ty = self.llvm_basic_type(&result_ty);
        let phi = self
            .builder
            .build_phi(llvm_ty, "ifx.phi")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        if then_end == BlockEnd::Open {
            phi.add_incoming(&[(&then_v, then_incoming)]);
        }
        if else_end == BlockEnd::Open {
            phi.add_incoming(&[(&else_v, else_incoming)]);
        }
        Ok((phi.as_basic_value(), result_ty))
    }

    fn const_param(
        &mut self,
        v: &ParamValue,
    ) -> (BasicValueEnum<'ctx>, CodegenTy) {
        match v {
            ParamValue::Int(n) => (
                self.context.i64_type().const_int(*n as u64, true).into(),
                CodegenTy::Int,
            ),
            ParamValue::Float(f) => (
                self.context.f64_type().const_float(*f).into(),
                CodegenTy::Float,
            ),
            ParamValue::Bool(b) => (
                self.context.bool_type().const_int(*b as u64, false).into(),
                CodegenTy::Bool,
            ),
            ParamValue::String(s) => (self.global_string(s).into(), CodegenTy::String),
            ParamValue::Duration(ns) => (
                self.context.i64_type().const_int(*ns as u64, true).into(),
                CodegenTy::Duration,
            ),
            ParamValue::Decimal(m) => (
                i128_const(self.context, *m).into(),
                CodegenTy::Decimal,
            ),
            ParamValue::Time(s) => {
                (self.global_string(s).into(), CodegenTy::Time)
            }
        }
    }

    fn llvm_basic_type(
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

    /// Anonymous LLVM struct for a tuple's storage layout. Used
    /// at tuple-literal allocation time + at GEP time when
    /// reading a numeric tuple field, destructuring in a let,
    /// or matching a tuple pattern. Element types come from
    /// `llvm_basic_type` so nested tuples / arrays / records are
    /// stored as pointers (matching the rest of the CodegenTy
    /// representation).
    fn llvm_tuple_storage_type(
        &self,
        elems: &[CodegenTy],
    ) -> inkwell::types::StructType<'ctx> {
        let field_tys: Vec<inkwell::types::BasicTypeEnum<'ctx>> = elems
            .iter()
            .map(|t| self.llvm_basic_type(t))
            .collect();
        self.context.struct_type(&field_tys, false)
    }

    /// LLVM `[N x T]` for the element type + size of an Array
    /// CodegenTy. Used at array-literal allocation time + at GEP
    /// time when indexing or iterating.
    fn llvm_array_storage_type(
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

    fn lower_binop(
        &mut self,
        op: BinOp,
        lv: BasicValueEnum<'ctx>,
        rv: BasicValueEnum<'ctx>,
        ty: &CodegenTy,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        use inkwell::IntPredicate as IP;
        use inkwell::FloatPredicate as FP;
        let ty_owned = ty.clone();
        match (op, ty_owned) {
            (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod, CodegenTy::Int) => {
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
                Ok((v.into(), CodegenTy::Int))
            }
            // ws-echo: bitwise ops on Int. Parser+typechecker
            // already accept these; interpreter handles them in
            // eval_binop. Wiring `&`, `|`, `^`, `<<`, `>>` here
            // closes the parity gap so WebSocket frame parsing
            // and similar packed-bit work doesn't have to fall
            // back on the arithmetic-emulation workaround
            // (`b & 0x80` → `b >= 128`, XOR via per-bit loop).
            // `>>` defaults to logical right shift — matches the
            // interpreter's `i64 >> i64` semantics for the
            // protocol-bit use case; an arithmetic variant can
            // ship as a stdlib fn if a workload needs it.
            (BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr,
                CodegenTy::Int) =>
            {
                let l = lv.into_int_value();
                let r = rv.into_int_value();
                let v = match op {
                    BinOp::BitAnd => self.builder.build_and(l, r, "band"),
                    BinOp::BitOr => self.builder.build_or(l, r, "bor"),
                    BinOp::BitXor => self.builder.build_xor(l, r, "bxor"),
                    BinOp::Shl => self.builder.build_left_shift(l, r, "bshl"),
                    BinOp::Shr => self.builder.build_right_shift(l, r, false, "bshr"),
                    _ => unreachable!(),
                };
                let v = v.map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v.into(), CodegenTy::Int))
            }
            // Duration arithmetic — add/sub produce Duration. Mul
            // / div / mod don't have natural Duration semantics
            // (multiply by a scalar would, but we don't have
            // scalar-by-Duration overloads yet).
            (BinOp::Add | BinOp::Sub, CodegenTy::Duration) => {
                let l = lv.into_int_value();
                let r = rv.into_int_value();
                let v = match op {
                    BinOp::Add => self.builder.build_int_add(l, r, "dadd"),
                    BinOp::Sub => self.builder.build_int_sub(l, r, "dsub"),
                    _ => unreachable!(),
                };
                let v = v.map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v.into(), CodegenTy::Duration))
            }
            (BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq,
                CodegenTy::Duration) =>
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
                Ok((v.into(), CodegenTy::Bool))
            }
            (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div, CodegenTy::Float) => {
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
                Ok((v.into(), CodegenTy::Float))
            }
            (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod, CodegenTy::Decimal) => {
                // m48: Decimal arithmetic on i128 mantissa with
                // implicit scale 9. Add/Sub/Mod: direct i128 ops
                // (the mantissas already share the implicit
                // scale, so a%b at scale 9 is just `a_m %
                // b_m` reinterpreted at scale 9). Mul: (a × b)
                // is mantissa scale 18; divide by 10^9 to bring
                // back to scale 9. Div: scale a's mantissa up by
                // 10^9 first so (a × 10^9) / b lands at scale 9.
                // Mul and div risk i128 overflow on the
                // intermediate product when operands exceed
                // ~10^19; v0.1 accepts the same wrap-around
                // policy as Int multiplication.
                let l = lv.into_int_value();
                let r = rv.into_int_value();
                let pow9 = i128_const(self.context, 1_000_000_000i128);
                let v = match op {
                    BinOp::Add => self.builder.build_int_add(l, r, "decadd"),
                    BinOp::Sub => self.builder.build_int_sub(l, r, "decsub"),
                    BinOp::Mod => self.builder.build_int_signed_rem(l, r, "decmod"),
                    BinOp::Mul => {
                        let prod = self
                            .builder
                            .build_int_mul(l, r, "decmul_raw")
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        self.builder.build_int_signed_div(prod, pow9, "decmul")
                    }
                    BinOp::Div => {
                        let scaled = self
                            .builder
                            .build_int_mul(l, pow9, "decdiv_scale")
                            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                        self.builder.build_int_signed_div(scaled, r, "decdiv")
                    }
                    _ => unreachable!(),
                };
                let v = v.map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v.into(), CodegenTy::Decimal))
            }
            (BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq,
                CodegenTy::Int) =>
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
                Ok((v.into(), CodegenTy::Bool))
            }
            (BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq,
                CodegenTy::Decimal) =>
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
                    .build_int_compare(pred, l, r, "deccmp")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v.into(), CodegenTy::Bool))
            }
            (BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq,
                CodegenTy::Float) =>
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
                Ok((v.into(), CodegenTy::Bool))
            }
            // m47-followup + payloads: enum equality.
            //   - No-payload enums: values are i32 tags; integer
            //     compare directly.
            //   - Has-payload enums: deep equality — tag match
            //     AND per-variant per-field equality. Switch on
            //     the tag, AND each variant's field comparisons
            //     into a per-block result, PHI back at the join.
            //     `!=` is the negation of the eq result.
            // Ord-style operators (<, >, <=, >=) aren't supported
            // — declaration order isn't a meaningful ordering.
            (BinOp::Eq | BinOp::NotEq, CodegenTy::Enum(name)) => {
                let info = self
                    .user_enums
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "enum eq on unknown type `{}`",
                            name
                        ))
                    })?;
                let eq_val: inkwell::values::IntValue<'ctx> = if info.has_payload
                {
                    self.lower_enum_deep_eq(
                        &info,
                        lv.into_pointer_value(),
                        rv.into_pointer_value(),
                    )?
                } else {
                    self.builder
                        .build_int_compare(
                            IP::EQ,
                            lv.into_int_value(),
                            rv.into_int_value(),
                            "enumcmp",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                };
                let v = match op {
                    BinOp::Eq => eq_val,
                    BinOp::NotEq => self
                        .builder
                        .build_not(eq_val, "enumneq")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?,
                    _ => unreachable!(),
                };
                Ok((v.into(), CodegenTy::Bool))
            }
            (BinOp::And, CodegenTy::Bool) => {
                let v = self
                    .builder
                    .build_and(lv.into_int_value(), rv.into_int_value(), "and")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v.into(), CodegenTy::Bool))
            }
            (BinOp::Or, CodegenTy::Bool) => {
                let v = self
                    .builder
                    .build_or(lv.into_int_value(), rv.into_int_value(), "or")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((v.into(), CodegenTy::Bool))
            }
            // m36: String concatenation. The result lives in the
            // current arena (caller's locus or program-wide); the
            // C runtime helper memcpy's both operands into a fresh
            // NUL-terminated buffer.
            (BinOp::Add, CodegenTy::String) => {
                let arena_ptr = self.current_arena_ptr()?;
                let concat_fn = self
                    .module
                    .get_function("lotus_str_concat")
                    .expect("lotus_str_concat declared");
                let v = self
                    .builder
                    .build_call(
                        concat_fn,
                        &[
                            arena_ptr.into(),
                            lv.into_pointer_value().into(),
                            rv.into_pointer_value().into(),
                        ],
                        "str.concat",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_str_concat returns ptr");
                Ok((v, CodegenTy::String))
            }
            // m36: String equality / inequality via strcmp wrapper.
            // The C helper returns i32 0/1; we truncate to i1.
            (BinOp::Eq | BinOp::NotEq, CodegenTy::String) => {
                let eq_fn = self
                    .module
                    .get_function("lotus_str_eq")
                    .expect("lotus_str_eq declared");
                let raw = self
                    .builder
                    .build_call(
                        eq_fn,
                        &[
                            lv.into_pointer_value().into(),
                            rv.into_pointer_value().into(),
                        ],
                        "str.eq",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_str_eq returns i32");
                let one = self.context.i32_type().const_int(1, false);
                let is_eq = self
                    .builder
                    .build_int_compare(
                        IP::EQ,
                        raw.into_int_value(),
                        one,
                        "str.eq.bool",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let result = if matches!(op, BinOp::NotEq) {
                    self.builder
                        .build_not(is_eq, "str.neq")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                } else {
                    is_eq
                };
                Ok((result.into(), CodegenTy::Bool))
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
        ty: &CodegenTy,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let ty_owned = ty.clone();
        match (op, ty_owned) {
            (UnaryOp::Neg, CodegenTy::Int) => {
                let zero = self.context.i64_type().const_int(0, true);
                let r = self
                    .builder
                    .build_int_sub(zero, v.into_int_value(), "neg")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((r.into(), CodegenTy::Int))
            }
            (UnaryOp::Neg, CodegenTy::Float) => {
                let r = self
                    .builder
                    .build_float_neg(v.into_float_value(), "fneg")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((r.into(), CodegenTy::Float))
            }
            (UnaryOp::Neg, CodegenTy::Decimal) => {
                // m48: Decimal lives in i128; negate via subtract
                // from i128 zero, mirroring Int's neg lowering.
                let zero = i128_const(self.context, 0);
                let r = self
                    .builder
                    .build_int_sub(zero, v.into_int_value(), "decneg")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((r.into(), CodegenTy::Decimal))
            }
            (UnaryOp::Not, CodegenTy::Bool) => {
                let r = self
                    .builder
                    .build_not(v.into_int_value(), "not")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((r.into(), CodegenTy::Bool))
            }
            _ => Err(CodegenError::Unsupported(format!(
                "unop {:?} on {:?}",
                op, ty
            ))),
        }
    }

    /// Lower a `print` / `println` / `eprint` / `eprintln` call.
    /// Args resolve through the current scope and (when set) the
    /// current `self` struct, so the same lowering serves both
    /// ordinary fn bodies and lifecycle-method bodies. The `e`-
    /// prefixed variants route to stderr via fprintf; the bare
    /// variants stay on stdout via printf.
    fn lower_print_call(
        &mut self,
        name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        if name != "println" && name != "print"
            && name != "eprintln" && name != "eprint"
        {
            return Err(CodegenError::Unsupported(format!("builtin `{}`", name)));
        }
        let to_stderr = name == "eprintln" || name == "eprint";
        let with_newline = name == "println" || name == "eprintln";
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
                CodegenTy::Int => {
                    format.push_str("%lld");
                    printf_args.push(BasicMetadataValueEnum::IntValue(val.into_int_value()));
                }
                CodegenTy::Float => {
                    format.push_str("%g");
                    printf_args
                        .push(BasicMetadataValueEnum::FloatValue(val.into_float_value()));
                }
                CodegenTy::Decimal => {
                    // m48: render the i128 mantissa via the
                    // C runtime helper into a stack buffer, then
                    // splice the buffer in as %s. Splitting i128
                    // → (hi, lo) via lshr/trunc keeps the FFI
                    // call ABI-portable (passing __int128
                    // directly relies on the platform's i128
                    // calling convention, which inkwell doesn't
                    // model uniformly).
                    let i128_v = val.into_int_value();
                    let i64_t = self.context.i64_type();
                    let lo = self
                        .builder
                        .build_int_truncate(i128_v, i64_t, "dec_lo")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let shift = self
                        .context
                        .i128_type()
                        .const_int(64, false);
                    let hi_wide = self
                        .builder
                        .build_right_shift(i128_v, shift, true, "dec_hi_wide")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let hi = self
                        .builder
                        .build_int_truncate(hi_wide, i64_t, "dec_hi")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let buf_ty = self.context.i8_type().array_type(64);
                    let buf = self
                        .builder
                        .build_alloca(buf_ty, "dec_buf")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    let dec_to_str = self
                        .module
                        .get_function("lotus_decimal_to_string")
                        .ok_or_else(|| {
                            CodegenError::LlvmEmit(
                                "lotus_decimal_to_string undeclared".into(),
                            )
                        })?;
                    self.builder
                        .build_call(
                            dec_to_str,
                            &[hi.into(), lo.into(), buf.into()],
                            "dec_render",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    format.push_str("%s");
                    printf_args.push(BasicMetadataValueEnum::PointerValue(buf));
                }
                CodegenTy::Bool => {
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
                CodegenTy::String | CodegenTy::Time => {
                    format.push_str("%s");
                    printf_args.push(BasicMetadataValueEnum::PointerValue(
                        val.into_pointer_value(),
                    ));
                }
                CodegenTy::Duration => {
                    // Match the interpreter's `<ns>ns` rendering so
                    // both paths produce identical stdout.
                    format.push_str("%lldns");
                    printf_args.push(BasicMetadataValueEnum::IntValue(
                        val.into_int_value(),
                    ));
                }
                CodegenTy::LocusRef(name) => {
                    return Err(CodegenError::Unsupported(format!(
                        "println of a locus value (LocusRef `{}`) — \
                         lotus has no Display protocol yet",
                        name
                    )));
                }
                CodegenTy::TypeRef(name) => {
                    return Err(CodegenError::Unsupported(format!(
                        "println of a type value (TypeRef `{}`) — \
                         print individual fields instead",
                        name
                    )));
                }
                CodegenTy::Array(_, _) => {
                    return Err(CodegenError::Unsupported(
                        "println of an array — print individual \
                         elements via indexing or iteration".into(),
                    ));
                }
                CodegenTy::Tuple(_) => {
                    return Err(CodegenError::Unsupported(
                        "println of a tuple — print individual \
                         components via .0 / .1 / let-destructure".into(),
                    ));
                }
                CodegenTy::FnPtr { .. } => {
                    return Err(CodegenError::Unsupported(
                        "println of a function pointer — function \
                         values have no surface representation".into(),
                    ));
                }
                CodegenTy::Bytes => {
                    // m89: print Bytes as `<bytes len=N>` so it's
                    // identifiable in logs without dumping
                    // potentially-binary content. Users who want the
                    // body should write a hex / base64 helper —
                    // direct stringification would be lossy for
                    // binary data and confusing for ASCII data
                    // (NUL-terminated assumptions break).
                    let len_fn = self
                        .module
                        .get_function("lotus_bytes_len")
                        .expect("lotus_bytes_len declared");
                    let len_v = self
                        .builder
                        .build_call(
                            len_fn,
                            &[val.into_pointer_value().into()],
                            "println.bytes.len",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .try_as_basic_value()
                        .left()
                        .expect("lotus_bytes_len returns i64")
                        .into_int_value();
                    format.push_str("<bytes len=%lld>");
                    printf_args.push(BasicMetadataValueEnum::IntValue(len_v));
                }
                CodegenTy::Enum(enum_name) => {
                    // m47-followup + payloads: render the enum
                    // value via the shared value_to_string path —
                    // for no-payload enums it returns a pointer
                    // into the names-array global; for has-payload
                    // enums it builds the rendering inline (per-
                    // variant switch + per-field render). Splice
                    // the resulting char* in as %s.
                    let _ = enum_name;
                    let s = self.value_to_string(val, &ty)?;
                    format.push_str("%s");
                    printf_args.push(BasicMetadataValueEnum::PointerValue(
                        s.into_pointer_value(),
                    ));
                }
                CodegenTy::Interface(name) => {
                    return Err(CodegenError::Unsupported(format!(
                        "println of an interface value (`{}`) — \
                         interface values have no surface \
                         representation; call a method on it instead",
                        name
                    )));
                }
                CodegenTy::Cell(inner, _) => {
                    return Err(CodegenError::Unsupported(format!(
                        "println of an F.22 capacity-slot cell \
                         (Cell<{:?}>) — cells are opaque round-trip \
                         handles at v1, not printable values. \
                         Release/free the cell and println a \
                         specific value instead.",
                        inner
                    )));
                }
            }
        }
        if with_newline {
            format.push('\n');
        }
        let fmt_ptr = self.global_string(&format);
        printf_args[0] = BasicMetadataValueEnum::PointerValue(fmt_ptr);
        if to_stderr {
            // dprintf(2, fmt, args...) — write directly to fd 2
            // (stderr) without depending on the libc `stderr` FILE*
            // global, which is a macro-expanded function-call on
            // some libcs and an extern global on others. dprintf is
            // already declared for the closure-violation report and
            // is the cheapest cross-libc path.
            let i32_t = self.context.i32_type();
            let fd_two = i32_t.const_int(2, false);
            let mut dprintf_args: Vec<BasicMetadataValueEnum> =
                Vec::with_capacity(printf_args.len() + 1);
            dprintf_args.push(BasicMetadataValueEnum::IntValue(fd_two));
            dprintf_args.extend(printf_args.iter().cloned());
            let dprintf = self
                .module
                .get_function("dprintf")
                .expect("dprintf declared");
            self.builder
                .build_call(dprintf, &dprintf_args, "dprintf_call")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        } else {
            let printf = self
                .module
                .get_function("printf")
                .expect("printf declared");
            self.builder
                .build_call(printf, &printf_args, "printf_call")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
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
        // m71: std::* paths route through the stdlib lowering. The
        // dispatcher returns Some(_) iff it recognized the path; an
        // unknown std::* path errors with the same shape as the rest
        // of this match's catch-all.
        if segs.first() == Some(&"std") {
            return self.lower_stdlib_path_call(&segs, args, scope);
        }
        match segs.as_slice() {
            ["time", "sleep"] => self.lower_time_sleep(args, scope),
            ["time", "monotonic"] => {
                // statement-position: just discard the returned value
                let _ = self.lower_time_monotonic(args)?;
                Ok(())
            }
            // m47-payloads: enum-variant construction at stmt
            // position — value is discarded but allocation +
            // stores still need to run for side effects (e.g.
            // an arena-bumping construction the user expects to
            // observe via a closure or bus). Delegate to the
            // expression form, drop the result.
            [enum_name, _variant]
                if self.user_enums.contains_key(*enum_name) =>
            {
                let _ = self.lower_path_call_expr(qn, args, scope)?;
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
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let segs: Vec<&str> =
            qn.segments.iter().map(|s| s.name.as_str()).collect();
        if segs.first() == Some(&"std") {
            return self.lower_stdlib_path_call_expr(&segs, args, scope);
        }
        match segs.as_slice() {
            ["time", "monotonic"] => self.lower_time_monotonic(args),
            // m47-payloads: enum-variant construction with
            // arguments. `EnumName::Variant(arg0, arg1, ...)`.
            // Validate the arity + each arg type matches the
            // variant's declared field types, lower each arg,
            // then allocate the storage struct in the current
            // arena and pack the payload bytes.
            [enum_name, variant_name]
                if self.user_enums.contains_key(*enum_name) =>
            {
                let info = self
                    .user_enums
                    .get(*enum_name)
                    .cloned()
                    .expect("user_enums.get under contains_key");
                let variant_idx = info
                    .variants
                    .iter()
                    .position(|v| v.name == *variant_name)
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "enum `{}` has no variant `{}`",
                            enum_name, variant_name
                        ))
                    })?;
                let variant = info.variants[variant_idx].clone();
                if args.len() != variant.field_tys.len() {
                    return Err(CodegenError::Unsupported(format!(
                        "{}::{} expects {} arg(s), got {}",
                        enum_name,
                        variant_name,
                        variant.field_tys.len(),
                        args.len()
                    )));
                }
                let mut field_vals: Vec<(BasicValueEnum<'ctx>, CodegenTy)> =
                    Vec::with_capacity(args.len());
                for (j, a) in args.iter().enumerate() {
                    let (v, ty) = self.lower_expr(a, scope)?;
                    if ty != variant.field_tys[j] {
                        return Err(CodegenError::Unsupported(format!(
                            "{}::{} arg {}: expected type {:?}, got {:?}",
                            enum_name, variant_name, j, variant.field_tys[j], ty
                        )));
                    }
                    field_vals.push((v, ty));
                }
                let ptr = self.lower_enum_variant_alloc(
                    &info,
                    variant_idx as u32,
                    &field_vals,
                )?;
                Ok((ptr.into(), CodegenTy::Enum(enum_name.to_string())))
            }
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
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
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
        Ok((total.into(), CodegenTy::Duration))
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
        if ty != CodegenTy::Duration {
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

    // ============================================================
    // Phase 1 stdlib lowering (m71+).
    //
    // `.ap` source references stdlib symbols by fully-qualified
    // path: `std::io::tcp::listen(8080)`, `std::io::fs::read_file
    // (path)`, `std::process::pid()`. The parser tokenizes `::`
    // already and the type checker punts namespaced paths to
    // `Ty::Unknown`; codegen resolves them here.
    //
    // No general module system in Phase 1 — `std::*` is the only
    // recognized prefix, matched against a hardcoded namespace
    // dispatcher. Adding a function means: declare its libc backer
    // in `declare_builtins` (Phase 1 stdlib section), add a match
    // arm here, and implement one `lower_std_*` method.
    // ============================================================

    /// Statement-position dispatcher for `std::*` paths. The leading
    /// `"std"` segment is included in `segs` for symmetry with the
    /// expression-position dispatcher.
    fn lower_stdlib_path_call(
        &mut self,
        segs: &[&str],
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        match segs {
            // Statement-position calls that have a useful return
            // value still go through the expression form; we drop
            // the result.
            ["std", "process", "pid"] => {
                let _ = self.lower_std_process_pid(args)?;
                Ok(())
            }
            ["std", "io", "tcp", "__listen_socket"] => {
                let _ = self.lower_std_io_tcp_listen_socket(args, scope)?;
                Ok(())
            }
            ["std", "io", "tcp", "__accept_one"] => {
                let _ = self.lower_std_io_tcp_accept_one(args, scope)?;
                Ok(())
            }
            ["std", "io", "tcp", "__connect"] => {
                let _ = self.lower_std_io_tcp_connect(args, scope)?;
                Ok(())
            }
            ["std", "io", "tcp", "__close_fd"] => {
                let _ = self.lower_std_io_tcp_close_fd(args, scope)?;
                Ok(())
            }
            ["std", "io", "tcp", "__send"] => {
                let _ = self.lower_std_io_tcp_send(args, scope)?;
                Ok(())
            }
            ["std", "io", "tcp", "__recv"] => {
                let _ = self.lower_std_io_tcp_recv(args, scope)?;
                Ok(())
            }
            ["std", "io", "fs", "read_bytes"] => {
                let _ = self.lower_std_io_fs_read_bytes(args, scope)?;
                Ok(())
            }
            ["std", "io", "fs", "list_dir"] => {
                let _ = self.lower_std_io_fs_list_dir(args, scope)?;
                Ok(())
            }
            ["std", "io", "tcp", "__send_bytes"] => {
                let _ = self.lower_std_io_tcp_send_bytes(args, scope)?;
                Ok(())
            }
            // Phase 2g: binary-safe TCP recv + Bytes/String surface.
            // Statement position is unusual for these (the values
            // are normally bound), but we wire them so a discarded
            // call doesn't error.
            ["std", "io", "tcp", "__recv_bytes"] => {
                let _ = self.lower_std_io_tcp_recv_bytes(args, scope)?;
                Ok(())
            }
            ["std", "str", "from_bytes"] => {
                let _ = self.lower_std_str_from_bytes(args, scope)?;
                Ok(())
            }
            ["std", "bytes", "from_string"] => {
                let _ = self.lower_std_bytes_from_string(args, scope)?;
                Ok(())
            }
            ["std", "bytes", "at"] => {
                let _ = self.lower_std_bytes_at(args, scope)?;
                Ok(())
            }
            ["std", "bytes", "slice"] => {
                let _ = self.lower_std_bytes_slice(args, scope)?;
                Ok(())
            }
            ["std", "bytes", "from_int"] => {
                let _ = self.lower_std_bytes_from_int(args, scope)?;
                Ok(())
            }
            ["std", "bytes", "concat"] => {
                let _ = self.lower_std_bytes_concat(args, scope)?;
                Ok(())
            }
            ["std", "crypto", "sha1"] => {
                let _ = self.lower_std_crypto_sha1(args, scope)?;
                Ok(())
            }
            ["std", "text", "base64", "encode"] => {
                let _ = self.lower_std_text_base64_encode(args, scope)?;
                Ok(())
            }
            ["std", "text", "base64", "decode"] => {
                let _ = self.lower_std_text_base64_decode(args, scope)?;
                Ok(())
            }
            ["std", "rand", "seed_from_time"] => {
                self.lower_std_rand_seed_from_time(args)?;
                Ok(())
            }
            ["std", "rand", "next_int"] => {
                let _ = self.lower_std_rand_next_int(args, scope)?;
                Ok(())
            }
            // Phase 2e: list_dir index API.
            ["std", "io", "fs", "list_dir_count"] => {
                let _ = self.lower_std_io_fs_list_dir_count(args, scope)?;
                Ok(())
            }
            ["std", "io", "fs", "list_dir_at"] => {
                let _ = self.lower_std_io_fs_list_dir_at(args, scope)?;
                Ok(())
            }
            // Phase 2f: read_file errno status.
            ["std", "io", "fs", "read_file_status"] => {
                let _ = self.lower_std_io_fs_read_file_status(args, scope)?;
                Ok(())
            }
            ["std", "io", "fs", "read_file"] => {
                let _ = self.lower_std_io_fs_read_file(args, scope)?;
                Ok(())
            }
            ["std", "io", "fs", "write_file"] => {
                let _ = self.lower_std_io_fs_write_file(args, scope)?;
                Ok(())
            }
            ["std", "io", "fs", "write_file_append"] => {
                let _ = self.lower_std_io_fs_write_file_append(args, scope)?;
                Ok(())
            }
            ["std", "io", "fs", "mkdir"] => {
                let _ = self.lower_std_io_fs_mkdir(args, scope)?;
                Ok(())
            }
            ["std", "io", "fs", "file_size"] => {
                let _ = self.lower_std_io_fs_file_size(args, scope)?;
                Ok(())
            }
            ["std", "io", "fs", "file_exists"] => {
                let _ = self.lower_std_io_fs_file_exists(args, scope)?;
                Ok(())
            }
            ["std", "io", "fs", "extension"] => {
                let _ = self.lower_std_io_fs_extension(args, scope)?;
                Ok(())
            }
            ["std", "env", "args_count"] => {
                let _ = self.lower_std_env_args_count(args)?;
                Ok(())
            }
            ["std", "env", "arg"] => {
                let _ = self.lower_std_env_arg(args, scope)?;
                Ok(())
            }
            ["std", "env", "var"] => {
                let _ = self.lower_std_env_var(args, scope)?;
                Ok(())
            }
            ["std", "env", "var_exists"] => {
                let _ = self.lower_std_env_var_exists(args, scope)?;
                Ok(())
            }
            ["std", "str", "index_of"] => {
                let _ = self.lower_std_str_index_of(args, scope)?;
                Ok(())
            }
            ["std", "str", "parse_int"] => {
                let _ = self.lower_std_str_parse_int(args, scope)?;
                Ok(())
            }
            ["std", "str", "parse_float"] => {
                let _ = self.lower_std_str_parse_float(args, scope)?;
                Ok(())
            }
            ["std", "str", "can_parse_float"] => {
                let _ = self.lower_std_str_can_parse_float(args, scope)?;
                Ok(())
            }
            ["std", "str", "lower"] => {
                let _ = self.lower_std_str_case_fold(args, scope, "lower")?;
                Ok(())
            }
            ["std", "str", "upper"] => {
                let _ = self.lower_std_str_case_fold(args, scope, "upper")?;
                Ok(())
            }
            ["std", "str", "trim"] => {
                let _ = self.lower_std_str_case_fold(args, scope, "trim")?;
                Ok(())
            }
            ["std", "str", "replace"] => {
                let _ = self.lower_std_str_replace(args, scope)?;
                Ok(())
            }
            ["std", "str", "repeat"] => {
                let _ = self.lower_std_str_repeat(args, scope)?;
                Ok(())
            }
            ["std", "str", "pad_left"] => {
                let _ = self.lower_std_str_pad(args, scope, "pad_left")?;
                Ok(())
            }
            ["std", "str", "pad_right"] => {
                let _ = self.lower_std_str_pad(args, scope, "pad_right")?;
                Ok(())
            }
            // v1.x-15: string-builder primitive.
            ["std", "str", "builder_new"] => {
                let _ = self.lower_std_str_builder_new(args)?;
                Ok(())
            }
            ["std", "str", "builder_append"] => {
                self.lower_std_str_builder_append(args, scope)
            }
            ["std", "str", "builder_len"] => {
                let _ = self.lower_std_str_builder_len(args, scope)?;
                Ok(())
            }
            ["std", "str", "builder_finish"] => {
                let _ = self.lower_std_str_builder_finish(args, scope)?;
                Ok(())
            }
            // m84: parse_request also reachable in statement
            // position (rare — usually you keep the result), but
            // wire it for completeness so `std::http::parse_request(raw);`
            // doesn't error.
            ["std", "http", "parse_request"] => {
                let _ = self.lower_user_fn_call(
                    "__parse_http_request",
                    args,
                    scope,
                )?;
                Ok(())
            }
            // m85: void-returning response writer. Routes to the
            // bare-name stdlib fn `__write_http_response`.
            ["std", "http", "write_response"] => {
                let _ = self.lower_user_fn_call(
                    "__write_http_response",
                    args,
                    scope,
                )?;
                Ok(())
            }
            // m87: std::test::* assertion primitives. Each is a
            // void-returning stdlib fn that prints diagnostic
            // + exits 1 on failure, no-op on success. Users
            // write tests as ordinary Aperio binaries that
            // exit 0 on pass.
            // m91: markdown → HTML (statement position rare, but
            // wired for completeness). The expression-position arm
            // below is the canonical use.
            ["std", "text", "md_to_html"] => {
                let _ = self.lower_user_fn_call(
                    "__md_to_html",
                    args,
                    scope,
                )?;
                Ok(())
            }
            ["std", "test", "assert"] => {
                let _ = self.lower_user_fn_call(
                    "__test_assert",
                    args,
                    scope,
                )?;
                Ok(())
            }
            ["std", "test", "assert_eq_int"] => {
                let _ = self.lower_user_fn_call(
                    "__test_assert_eq_int",
                    args,
                    scope,
                )?;
                Ok(())
            }
            ["std", "test", "assert_eq_str"] => {
                let _ = self.lower_user_fn_call(
                    "__test_assert_eq_str",
                    args,
                    scope,
                )?;
                Ok(())
            }
            ["std", "str", "can_parse_int"] => {
                let _ = self.lower_std_str_can_parse_int(args, scope)?;
                Ok(())
            }
            // m96: std::ts::* tree-sitter substrate. All routes
            // also have expression-position arms below; dropping
            // the result here is fine for parse-and-discard
            // patterns (which are unusual but legal).
            ["std", "ts", "parse_go"] => {
                let _ = self.lower_std_ts_parse_go(args, scope)?;
                Ok(())
            }
            ["std", "ts", "root_node"] => {
                let _ = self.lower_std_ts_int1_to_int(
                    "lotus_ts_root_node",
                    args,
                    scope,
                    "std::ts::root_node",
                )?;
                Ok(())
            }
            ["std", "ts", "node_kind"] => {
                let _ = self.lower_std_ts_int1_to_string(
                    "lotus_ts_node_kind",
                    args,
                    scope,
                    "std::ts::node_kind",
                )?;
                Ok(())
            }
            ["std", "ts", "node_text"] => {
                let _ = self.lower_std_ts_int1_to_string(
                    "lotus_ts_node_text",
                    args,
                    scope,
                    "std::ts::node_text",
                )?;
                Ok(())
            }
            ["std", "ts", "node_child_count"] => {
                let _ = self.lower_std_ts_int1_to_int(
                    "lotus_ts_node_child_count",
                    args,
                    scope,
                    "std::ts::node_child_count",
                )?;
                Ok(())
            }
            ["std", "ts", "node_named_child_count"] => {
                let _ = self.lower_std_ts_int1_to_int(
                    "lotus_ts_node_named_child_count",
                    args,
                    scope,
                    "std::ts::node_named_child_count",
                )?;
                Ok(())
            }
            ["std", "ts", "node_child"] => {
                let _ = self.lower_std_ts_int2_to_int(
                    "lotus_ts_node_child",
                    args,
                    scope,
                    "std::ts::node_child",
                )?;
                Ok(())
            }
            ["std", "ts", "node_named_child"] => {
                let _ = self.lower_std_ts_int2_to_int(
                    "lotus_ts_node_named_child",
                    args,
                    scope,
                    "std::ts::node_named_child",
                )?;
                Ok(())
            }
            ["std", "ts", "node_start_byte"] => {
                let _ = self.lower_std_ts_int1_to_int(
                    "lotus_ts_node_start_byte",
                    args,
                    scope,
                    "std::ts::node_start_byte",
                )?;
                Ok(())
            }
            ["std", "ts", "node_end_byte"] => {
                let _ = self.lower_std_ts_int1_to_int(
                    "lotus_ts_node_end_byte",
                    args,
                    scope,
                    "std::ts::node_end_byte",
                )?;
                Ok(())
            }
            ["std", "ts", "node_is_named"] => {
                let _ = self.lower_std_ts_int1_to_int(
                    "lotus_ts_node_is_named",
                    args,
                    scope,
                    "std::ts::node_is_named",
                )?;
                Ok(())
            }
            // m79: std::time::* aliases. The legacy `time::*`
            // dispatcher above still works; these route to the
            // same lower_time_* implementations under the
            // canonical `std::*` namespace.
            ["std", "time", "sleep"] => self.lower_time_sleep(args, scope),
            ["std", "time", "monotonic"] => {
                let _ = self.lower_time_monotonic(args)?;
                Ok(())
            }
            // m79: std::process::exit. Calls libc exit() with the
            // user-supplied code, then emits unreachable + a fresh
            // basic block so subsequent statements (dead but
            // syntactically permitted) have somewhere to lower
            // into.
            ["std", "process", "exit"] => {
                self.lower_std_process_exit(args, scope)
            }
            _ => Err(CodegenError::Unsupported(format!(
                "stdlib path `{}` — not implemented",
                segs.join("::")
            ))),
        }
    }

    /// Expression-position dispatcher for `std::*` paths.
    fn lower_stdlib_path_call_expr(
        &mut self,
        segs: &[&str],
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        match segs {
            ["std", "process", "pid"] => self.lower_std_process_pid(args),
            ["std", "io", "tcp", "__listen_socket"] => {
                self.lower_std_io_tcp_listen_socket(args, scope)
            }
            ["std", "io", "tcp", "__accept_one"] => {
                self.lower_std_io_tcp_accept_one(args, scope)
            }
            ["std", "io", "tcp", "__connect"] => {
                self.lower_std_io_tcp_connect(args, scope)
            }
            ["std", "io", "tcp", "__close_fd"] => {
                self.lower_std_io_tcp_close_fd(args, scope)
            }
            ["std", "io", "tcp", "__send"] => {
                self.lower_std_io_tcp_send(args, scope)
            }
            ["std", "io", "tcp", "__recv"] => {
                self.lower_std_io_tcp_recv(args, scope)
            }
            ["std", "io", "fs", "read_bytes"] => {
                self.lower_std_io_fs_read_bytes(args, scope)
            }
            ["std", "io", "fs", "list_dir"] => {
                self.lower_std_io_fs_list_dir(args, scope)
            }
            ["std", "io", "tcp", "__send_bytes"] => {
                self.lower_std_io_tcp_send_bytes(args, scope)
            }
            // Phase 2g: binary-safe TCP recv + Bytes/String surface.
            ["std", "io", "tcp", "__recv_bytes"] => {
                self.lower_std_io_tcp_recv_bytes(args, scope)
            }
            ["std", "str", "from_bytes"] => {
                self.lower_std_str_from_bytes(args, scope)
            }
            ["std", "bytes", "from_string"] => {
                self.lower_std_bytes_from_string(args, scope)
            }
            ["std", "bytes", "at"] => {
                self.lower_std_bytes_at(args, scope)
            }
            ["std", "bytes", "slice"] => {
                self.lower_std_bytes_slice(args, scope)
            }
            ["std", "bytes", "from_int"] => {
                self.lower_std_bytes_from_int(args, scope)
            }
            ["std", "bytes", "concat"] => {
                self.lower_std_bytes_concat(args, scope)
            }
            ["std", "crypto", "sha1"] => {
                self.lower_std_crypto_sha1(args, scope)
            }
            ["std", "text", "base64", "encode"] => {
                self.lower_std_text_base64_encode(args, scope)
            }
            ["std", "text", "base64", "decode"] => {
                self.lower_std_text_base64_decode(args, scope)
            }
            ["std", "rand", "next_int"] => {
                self.lower_std_rand_next_int(args, scope)
            }
            // Phase 2e: list_dir index API.
            ["std", "io", "fs", "list_dir_count"] => {
                self.lower_std_io_fs_list_dir_count(args, scope)
            }
            ["std", "io", "fs", "list_dir_at"] => {
                self.lower_std_io_fs_list_dir_at(args, scope)
            }
            // Phase 2f: read_file errno status.
            ["std", "io", "fs", "read_file_status"] => {
                self.lower_std_io_fs_read_file_status(args, scope)
            }
            ["std", "io", "fs", "read_file"] => {
                self.lower_std_io_fs_read_file(args, scope)
            }
            ["std", "io", "fs", "write_file"] => {
                self.lower_std_io_fs_write_file(args, scope)
            }
            ["std", "io", "fs", "write_file_append"] => {
                self.lower_std_io_fs_write_file_append(args, scope)
            }
            ["std", "io", "fs", "mkdir"] => {
                self.lower_std_io_fs_mkdir(args, scope)
            }
            ["std", "io", "fs", "file_size"] => {
                self.lower_std_io_fs_file_size(args, scope)
            }
            ["std", "io", "fs", "file_exists"] => {
                self.lower_std_io_fs_file_exists(args, scope)
            }
            ["std", "io", "fs", "extension"] => {
                self.lower_std_io_fs_extension(args, scope)
            }
            ["std", "env", "args_count"] => self.lower_std_env_args_count(args),
            ["std", "env", "arg"] => self.lower_std_env_arg(args, scope),
            ["std", "env", "var"] => self.lower_std_env_var(args, scope),
            ["std", "env", "var_exists"] => {
                self.lower_std_env_var_exists(args, scope)
            }
            ["std", "str", "index_of"] => {
                self.lower_std_str_index_of(args, scope)
            }
            ["std", "str", "parse_int"] => {
                self.lower_std_str_parse_int(args, scope)
            }
            ["std", "str", "parse_float"] => {
                self.lower_std_str_parse_float(args, scope)
            }
            ["std", "str", "can_parse_float"] => {
                self.lower_std_str_can_parse_float(args, scope)
            }
            ["std", "str", "lower"] => {
                self.lower_std_str_case_fold(args, scope, "lower")
            }
            ["std", "str", "upper"] => {
                self.lower_std_str_case_fold(args, scope, "upper")
            }
            ["std", "str", "trim"] => {
                self.lower_std_str_case_fold(args, scope, "trim")
            }
            ["std", "str", "replace"] => {
                self.lower_std_str_replace(args, scope)
            }
            ["std", "str", "repeat"] => {
                self.lower_std_str_repeat(args, scope)
            }
            ["std", "str", "pad_left"] => {
                self.lower_std_str_pad(args, scope, "pad_left")
            }
            ["std", "str", "pad_right"] => {
                self.lower_std_str_pad(args, scope, "pad_right")
            }
            ["std", "str", "builder_new"] => {
                self.lower_std_str_builder_new(args)
            }
            ["std", "str", "builder_len"] => {
                self.lower_std_str_builder_len(args, scope)
            }
            ["std", "str", "builder_finish"] => {
                self.lower_std_str_builder_finish(args, scope)
            }
            // m84: std::http::parse_request(raw: String) -> Request.
            // Implementation lives in stdlib.ap as the bare-name
            // free fn `__parse_http_request`. The path-call form is
            // the user-facing API; routing here keeps the stdlib's
            // private fn names hidden behind the std:: namespace.
            ["std", "http", "parse_request"] => {
                let result = self.lower_user_fn_call(
                    "__parse_http_request",
                    args,
                    scope,
                )?;
                result.ok_or_else(|| {
                    CodegenError::Unsupported(
                        "std::http::parse_request returns Request but \
                         called in a position that expects no value"
                            .to_string(),
                    )
                })
            }
            // ws-echo: per-request header lookup. Delegates to
            // the stdlib-internal `__http_request_header` fn.
            ["std", "http", "header"] => {
                let result = self.lower_user_fn_call(
                    "__http_request_header",
                    args,
                    scope,
                )?;
                result.ok_or_else(|| {
                    CodegenError::Unsupported(
                        "std::http::header returns String but called \
                         in a position that expects no value"
                            .to_string(),
                    )
                })
            }
            // m91: markdown → HTML.
            ["std", "text", "md_to_html"] => {
                let result = self.lower_user_fn_call(
                    "__md_to_html",
                    args,
                    scope,
                )?;
                result.ok_or_else(|| {
                    CodegenError::Unsupported(
                        "std::text::md_to_html returns String but \
                         called in a position that expects no value"
                            .to_string(),
                    )
                })
            }
            ["std", "str", "can_parse_int"] => {
                self.lower_std_str_can_parse_int(args, scope)
            }
            // m96: std::ts::* tree-sitter substrate (expression
            // position). See sibling arms in
            // `lower_stdlib_path_call` for shape rationale.
            ["std", "ts", "parse_go"] => self.lower_std_ts_parse_go(args, scope),
            ["std", "ts", "root_node"] => self.lower_std_ts_int1_to_int(
                "lotus_ts_root_node",
                args,
                scope,
                "std::ts::root_node",
            ),
            ["std", "ts", "node_kind"] => self.lower_std_ts_int1_to_string(
                "lotus_ts_node_kind",
                args,
                scope,
                "std::ts::node_kind",
            ),
            ["std", "ts", "node_text"] => self.lower_std_ts_int1_to_string(
                "lotus_ts_node_text",
                args,
                scope,
                "std::ts::node_text",
            ),
            ["std", "ts", "node_child_count"] => self.lower_std_ts_int1_to_int(
                "lotus_ts_node_child_count",
                args,
                scope,
                "std::ts::node_child_count",
            ),
            ["std", "ts", "node_named_child_count"] => self
                .lower_std_ts_int1_to_int(
                    "lotus_ts_node_named_child_count",
                    args,
                    scope,
                    "std::ts::node_named_child_count",
                ),
            ["std", "ts", "node_child"] => self.lower_std_ts_int2_to_int(
                "lotus_ts_node_child",
                args,
                scope,
                "std::ts::node_child",
            ),
            ["std", "ts", "node_named_child"] => self.lower_std_ts_int2_to_int(
                "lotus_ts_node_named_child",
                args,
                scope,
                "std::ts::node_named_child",
            ),
            ["std", "ts", "node_start_byte"] => self.lower_std_ts_int1_to_int(
                "lotus_ts_node_start_byte",
                args,
                scope,
                "std::ts::node_start_byte",
            ),
            ["std", "ts", "node_end_byte"] => self.lower_std_ts_int1_to_int(
                "lotus_ts_node_end_byte",
                args,
                scope,
                "std::ts::node_end_byte",
            ),
            ["std", "ts", "node_is_named"] => self.lower_std_ts_int1_to_int(
                "lotus_ts_node_is_named",
                args,
                scope,
                "std::ts::node_is_named",
            ),
            // m79: std::time::monotonic alias for expression position.
            // sleep is statement-only; trying to use it in an
            // expression falls through to the catch-all error.
            ["std", "time", "monotonic"] => self.lower_time_monotonic(args),
            // std::math::* libm Float primitives. Resolves
            // notes/aperio-friction.md 2026-05-10 float-surface-gaps
            // (the `std::math` sub-bullet). v0 cut: unary
            // sqrt/exp/log/floor/ceil + binary pow. Each lowers to
            // a libm call through the extern decl in
            // declare_builtins; arg type-checked as Float.
            ["std", "math", "sqrt"] => {
                self.lower_std_math_unary("sqrt", args, scope)
            }
            ["std", "math", "exp"] => {
                self.lower_std_math_unary("exp", args, scope)
            }
            ["std", "math", "log"] => {
                self.lower_std_math_unary("log", args, scope)
            }
            ["std", "math", "floor"] => {
                self.lower_std_math_unary("floor", args, scope)
            }
            ["std", "math", "ceil"] => {
                self.lower_std_math_unary("ceil", args, scope)
            }
            ["std", "math", "pow"] => {
                self.lower_std_math_binary("pow", args, scope)
            }
            _ => Err(CodegenError::Unsupported(format!(
                "stdlib path `{}` in expression position — not implemented",
                segs.join("::")
            ))),
        }
    }

    /// std::math::<sqrt|exp|log|floor|ceil> — single Float arg →
    /// Float result. Routes to the libm extern declared in
    /// declare_builtins. Int args coerce to Float via sitofp at
    /// the call site (same Int→Float widening this commit adds).
    fn lower_std_math_unary(
        &mut self,
        libm_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::math::{} takes 1 argument, got {}",
                libm_name,
                args.len()
            )));
        }
        let (v, ty) = self.lower_expr(&args[0], scope)?;
        let f = self.coerce_to_float(v, &ty, &format!("std::math::{}", libm_name))?;
        let func = self
            .module
            .get_function(libm_name)
            .expect("libm fn declared in declare_builtins");
        let call = self
            .builder
            .build_call(
                func,
                &[f.into()],
                &format!("std::math::{}.call", libm_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let result = call
            .try_as_basic_value()
            .left()
            .expect("libm unary returns f64");
        Ok((result, CodegenTy::Float))
    }

    /// std::math::pow — two Float args → Float result. Same
    /// libm pass-through pattern as the unary helper. Int args
    /// coerce.
    fn lower_std_math_binary(
        &mut self,
        libm_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::math::{} takes 2 arguments, got {}",
                libm_name,
                args.len()
            )));
        }
        let (a_val, a_ty) = self.lower_expr(&args[0], scope)?;
        let a_f = self.coerce_to_float(a_val, &a_ty, &format!("std::math::{} arg 0", libm_name))?;
        let (b_val, b_ty) = self.lower_expr(&args[1], scope)?;
        let b_f = self.coerce_to_float(b_val, &b_ty, &format!("std::math::{} arg 1", libm_name))?;
        let func = self
            .module
            .get_function(libm_name)
            .expect("libm fn declared in declare_builtins");
        let call = self
            .builder
            .build_call(
                func,
                &[a_f.into(), b_f.into()],
                &format!("std::math::{}.call", libm_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let result = call
            .try_as_basic_value()
            .left()
            .expect("libm binary returns f64");
        Ok((result, CodegenTy::Float))
    }

    /// Widen an Int value to Float, or pass through if it's
    /// already Float. Used at Float-typed slot boundaries
    /// (std::math fn args, let-binding ascription, fn-arg sites)
    /// to admit Int literals / Int-typed expressions in Float
    /// position. Resolves notes/aperio-friction.md 2026-05-10
    /// float-surface-gaps (the `Int → Float coercion` sub-bullet).
    /// Float → Int narrowing remains explicit (no implicit
    /// truncation); Decimal → Float and other lossy mixes also
    /// stay rejected.
    fn coerce_to_float(
        &self,
        v: BasicValueEnum<'ctx>,
        ty: &CodegenTy,
        callee_label: &str,
    ) -> Result<inkwell::values::FloatValue<'ctx>, CodegenError> {
        match ty {
            CodegenTy::Float => Ok(v.into_float_value()),
            CodegenTy::Int => {
                let f64_t = self.context.f64_type();
                self.builder
                    .build_signed_int_to_float(
                        v.into_int_value(),
                        f64_t,
                        "int.to.float",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))
            }
            other => Err(CodegenError::Unsupported(format!(
                "{}: expected Float (or Int via widening), got {:?}",
                callee_label, other
            ))),
        }
    }

    /// Lower `std::process::exit(code: Int)` to libc `exit()`.
    /// Statement-position only; the block becomes terminated
    /// after the call, so we open a fresh basic block to land
    /// any subsequent (dead) lowering into. Matches the
    /// closure-violation handler's exit pattern.
    fn lower_std_process_exit(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::process::exit takes 1 arg (code), got {}",
                args.len()
            )));
        }
        let (code_val, code_ty) = self.lower_expr(&args[0], scope)?;
        if code_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::process::exit: code must be Int, got {:?}",
                code_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let code_i32 = self
            .builder
            .build_int_truncate(code_val.into_int_value(), i32_t, "exit.code.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let exit_fn = self
            .module
            .get_function("exit")
            .expect("exit declared in declare_builtins");
        self.builder
            .build_call(exit_fn, &[code_i32.into()], "exit.call")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unreachable()
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Fresh dead block so any post-exit() statements have
        // somewhere to lower into without violating LLVM's
        // single-terminator-per-block rule.
        let func = self
            .current_fn
            .expect("current_fn set while lowering std::process::exit");
        let after = self.context.append_basic_block(func, "after.exit");
        self.builder.position_at_end(after);
        Ok(())
    }

    /// Lower `std::process::pid() -> Int` to `getpid()`. POSIX
    /// returns `pid_t` (i32 on Linux); Aperio `Int` is i64, so we
    /// sign-extend. m71 ships this as the proof symbol that the
    /// magic-`std::*`-path resolver works end-to-end; the same
    /// pattern (declare libc fn → match arm → one `lower_std_*`
    /// method) extends to every Phase 1 stdlib function.
    fn lower_std_process_pid(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::process::pid takes 0 arguments, got {}",
                args.len()
            )));
        }
        let i64_t = self.context.i64_type();
        let getpid = self
            .module
            .get_function("getpid")
            .expect("getpid declared");
        let call = self
            .builder
            .build_call(getpid, &[], "getpid.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let pid_i32 = call
            .try_as_basic_value()
            .left()
            .expect("getpid returns i32")
            .into_int_value();
        let pid_i64 = self
            .builder
            .build_int_s_extend(pid_i32, i64_t, "pid.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((pid_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::tcp::__listen_socket(host: String,
    /// port: Int) -> Int`. host is passed through as the
    /// NUL-terminated string pointer Aperio uses for String
    /// values; port is i64 truncated to i16 (port range fits).
    /// Returns the listen_fd as Int, sign-extended from i32.
    /// Stdlib loci call this from birth() and stash the result
    /// on self.
    fn lower_std_io_tcp_listen_socket(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__listen_socket takes 2 args (host, port), got {}",
                args.len()
            )));
        }
        let (host_val, host_ty) = self.lower_expr(&args[0], scope)?;
        if host_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__listen_socket: host must be String, got {:?}",
                host_ty
            )));
        }
        let (port_val, port_ty) = self.lower_expr(&args[1], scope)?;
        if port_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__listen_socket: port must be Int, got {:?}",
                port_ty
            )));
        }
        let i16_t = self.context.i16_type();
        let i64_t = self.context.i64_type();
        let port_i16 = self
            .builder
            .build_int_truncate(port_val.into_int_value(), i16_t, "port.i16")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tcp_listen_socket")
            .expect("lotus_tcp_listen_socket declared");
        let call = self
            .builder
            .build_call(f, &[host_val.into(), port_i16.into()], "listen.fd")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let fd_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let fd_i64 = self
            .builder
            .build_int_s_extend(fd_i32, i64_t, "fd.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((fd_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::tcp::__connect(host: String, port: Int)
    /// -> Int`. Returns conn_fd (or -1 on error) as Int.
    fn lower_std_io_tcp_connect(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__connect takes 2 args (host, port), got {}",
                args.len()
            )));
        }
        let (host_val, host_ty) = self.lower_expr(&args[0], scope)?;
        if host_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__connect: host must be String, got {:?}",
                host_ty
            )));
        }
        let (port_val, port_ty) = self.lower_expr(&args[1], scope)?;
        if port_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__connect: port must be Int, got {:?}",
                port_ty
            )));
        }
        let i16_t = self.context.i16_type();
        let i64_t = self.context.i64_type();
        let port_i16 = self
            .builder
            .build_int_truncate(port_val.into_int_value(), i16_t, "port.i16")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tcp_connect")
            .expect("lotus_tcp_connect declared");
        let call = self
            .builder
            .build_call(f, &[host_val.into(), port_i16.into()], "connect.fd")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let fd_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let fd_i64 = self
            .builder
            .build_int_s_extend(fd_i32, i64_t, "connfd.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((fd_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::tcp::__accept_one(listen_fd: Int) -> Int`.
    /// Returns the conn_fd (or -1 on error) as Int.
    fn lower_std_io_tcp_accept_one(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__accept_one takes 1 arg (listen_fd), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__accept_one: listen_fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "lfd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tcp_accept_one")
            .expect("lotus_tcp_accept_one declared");
        let call = self
            .builder
            .build_call(f, &[fd_i32.into()], "accept.fd")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let conn_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let conn_i64 = self
            .builder
            .build_int_s_extend(conn_i32, i64_t, "conn.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((conn_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::str::parse_int(s: String) -> Int`. Atoi-ish:
    /// returns 0 on parse failure or empty input. Disambiguate
    /// via `std::str::can_parse_int` if needed. Strict trailing-
    /// char check — "42abc" rejects, returns 0.
    fn lower_std_str_parse_int(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::parse_int takes 1 arg (s), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if s_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::str::parse_int: s must be String, got {:?}",
                s_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_str_parse_int")
            .expect("lotus_str_parse_int declared");
        let call = self
            .builder
            .build_call(f, &[s_val.into()], "parse.int.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((v, CodegenTy::Int))
    }

    /// Lower `std::str::index_of(s: String, sub: String) -> Int`.
    /// Returns the byte index of the first occurrence of `sub` in
    /// `s`, or -1 when `sub` doesn't appear. Empty needle returns
    /// 0 by convention. Wraps `lotus_str_index_of` directly. m84:
    /// the substring-search primitive HTTP request parsing leans
    /// on (find ` ` between method and path, `\r\n` to bound the
    /// request line).
    fn lower_std_str_index_of(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::index_of takes 2 args (s, sub), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if s_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::str::index_of: s must be String, got {:?}",
                s_ty
            )));
        }
        let (sub_val, sub_ty) = self.lower_expr(&args[1], scope)?;
        if sub_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::str::index_of: sub must be String, got {:?}",
                sub_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_str_index_of")
            .expect("lotus_str_index_of declared");
        let call = self
            .builder
            .build_call(
                f,
                &[s_val.into(), sub_val.into()],
                "str.index_of.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((v, CodegenTy::Int))
    }

    /// Lower `std::str::can_parse_int(s: String) -> Bool`.
    fn lower_std_str_can_parse_int(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::can_parse_int takes 1 arg (s), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if s_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::str::can_parse_int: s must be String, got {:?}",
                s_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let f = self
            .module
            .get_function("lotus_str_can_parse_int")
            .expect("lotus_str_can_parse_int declared");
        let call = self
            .builder
            .build_call(f, &[s_val.into()], "can.parse.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_bool = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret_i32,
                i32_t.const_zero(),
                "can.parse.bool",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_bool.into(), CodegenTy::Bool))
    }

    /// v1.x-16: `std::str::parse_float(s: String) -> Float`. Strict
    /// trailing-NUL parse; empty / non-numeric / partial-tail inputs
    /// return 0.0. Disambiguate via `std::str::can_parse_float`.
    fn lower_std_str_parse_float(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::parse_float takes 1 arg (s), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if s_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::str::parse_float: s must be String, got {:?}",
                s_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_str_parse_float")
            .expect("lotus_str_parse_float declared");
        let call = self
            .builder
            .build_call(f, &[s_val.into()], "parse.float.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("returns f64");
        Ok((v, CodegenTy::Float))
    }

    /// v1.x: `std::str::lower(s)` / `std::str::upper(s)` (ASCII
    /// case folding) and `std::str::trim(s)` (whitespace strip).
    /// All take one String, return a new String in the bus
    /// payload arena.
    fn lower_std_str_case_fold(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        which: &str,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::{} takes 1 arg (s), got {}",
                which,
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if s_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::str::{}: s must be String, got {:?}",
                which, s_ty
            )));
        }
        let extern_name = match which {
            "lower" => "lotus_str_lower",
            "upper" => "lotus_str_upper",
            "trim"  => "lotus_str_trim",
            _ => unreachable!(),
        };
        let f = self
            .module
            .get_function(extern_name)
            .expect("string fold/strip extern declared");
        let call = self
            .builder
            .build_call(f, &[s_val.into()], &format!("str.{}.ret", which))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((v, CodegenTy::String))
    }

    /// v1.x: `std::str::repeat(s, n) -> String`. Concatenates `s`
    /// with itself n times. n <= 0 returns empty.
    fn lower_std_str_repeat(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::repeat takes 2 args (s, n), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if s_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::str::repeat: s must be String, got {:?}",
                s_ty
            )));
        }
        let (n_val, n_ty) = self.lower_expr(&args[1], scope)?;
        if n_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::str::repeat: n must be Int, got {:?}",
                n_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_str_repeat")
            .expect("lotus_str_repeat declared");
        let call = self
            .builder
            .build_call(f, &[s_val.into(), n_val.into()], "str.repeat.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call.try_as_basic_value().left().expect("returns ptr");
        Ok((v, CodegenTy::String))
    }

    /// v1.x: `std::str::pad_left(s, width, pad) -> String` and
    /// `pad_right`. `pad` is a single-char String; only the first
    /// byte is used. If s is already >= width, returns s unchanged.
    fn lower_std_str_pad(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        which: &str,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::{} takes 3 args (s, width, pad), got {}",
                which,
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        let (w_val, w_ty) = self.lower_expr(&args[1], scope)?;
        let (p_val, p_ty) = self.lower_expr(&args[2], scope)?;
        if s_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::str::{}: s must be String, got {:?}",
                which, s_ty
            )));
        }
        if w_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::str::{}: width must be Int, got {:?}",
                which, w_ty
            )));
        }
        if p_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::str::{}: pad must be String, got {:?}",
                which, p_ty
            )));
        }
        let extern_name = match which {
            "pad_left" => "lotus_str_pad_left",
            "pad_right" => "lotus_str_pad_right",
            _ => unreachable!(),
        };
        let f = self
            .module
            .get_function(extern_name)
            .expect("pad extern declared");
        let call = self
            .builder
            .build_call(
                f,
                &[s_val.into(), w_val.into(), p_val.into()],
                &format!("str.{}.ret", which),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call.try_as_basic_value().left().expect("returns ptr");
        Ok((v, CodegenTy::String))
    }

    /// v1.x: `std::str::replace(s, needle, replacement) -> String`.
    /// Naive O(n*m) scan; greedy-forward (each match advances by
    /// needle_len, not 1). Empty needle is a no-op (avoids the
    /// infinite-replace footgun).
    fn lower_std_str_replace(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::replace takes 3 args (s, needle, replacement), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        let (n_val, n_ty) = self.lower_expr(&args[1], scope)?;
        let (r_val, r_ty) = self.lower_expr(&args[2], scope)?;
        for (label, ty) in &[
            ("s", &s_ty),
            ("needle", &n_ty),
            ("replacement", &r_ty),
        ] {
            if **ty != CodegenTy::String {
                return Err(CodegenError::Unsupported(format!(
                    "std::str::replace: {} must be String, got {:?}",
                    label, ty
                )));
            }
        }
        let f = self
            .module
            .get_function("lotus_str_replace")
            .expect("lotus_str_replace declared");
        let call = self
            .builder
            .build_call(
                f,
                &[s_val.into(), n_val.into(), r_val.into()],
                "str.replace.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((v, CodegenTy::String))
    }

    /// v1.x-15: `std::str::builder_new() -> Bytes`. Allocates a
    /// doubling-realloc-backed buffer; Bytes is the carrier type
    /// (opaque — users shouldn't index into it, only pass through
    /// to the other builder_* fns). Resolves the
    /// reader-list_item-quadratic-concat friction by turning N
    /// append calls into amortized O(N) total cost.
    fn lower_std_str_builder_new(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::str::builder_new takes 0 args, got {}",
                args.len()
            )));
        }
        let f = self
            .module
            .get_function("lotus_str_builder_new")
            .expect("lotus_str_builder_new declared");
        let call = self
            .builder
            .build_call(f, &[], "sb.new.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// v1.x-15: `std::str::builder_append(b: Bytes, s: String)`.
    /// Void-returning; statement-position only.
    fn lower_std_str_builder_append(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::builder_append takes 2 args (b, s), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if b_ty != CodegenTy::Bytes {
            return Err(CodegenError::Unsupported(format!(
                "std::str::builder_append: builder must be Bytes \
                 (from builder_new), got {:?}",
                b_ty
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[1], scope)?;
        if s_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::str::builder_append: s must be String, got {:?}",
                s_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_str_builder_append")
            .expect("lotus_str_builder_append declared");
        self.builder
            .build_call(f, &[b_val.into(), s_val.into()], "sb.append")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// v1.x-15: `std::str::builder_len(b: Bytes) -> Int`. Inspect
    /// the running length without materializing the final String.
    fn lower_std_str_builder_len(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::builder_len takes 1 arg (b), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if b_ty != CodegenTy::Bytes {
            return Err(CodegenError::Unsupported(format!(
                "std::str::builder_len: builder must be Bytes, got {:?}",
                b_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_str_builder_len")
            .expect("lotus_str_builder_len declared");
        let call = self
            .builder
            .build_call(f, &[b_val.into()], "sb.len.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((v, CodegenTy::Int))
    }

    /// v1.x-15: `std::str::builder_finish(b: Bytes) -> String`.
    /// Materializes the accumulated string in the bus payload
    /// arena (lives for the rest of the program) and frees the
    /// builder. The Bytes handle must NOT be reused after finish.
    fn lower_std_str_builder_finish(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::builder_finish takes 1 arg (b), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if b_ty != CodegenTy::Bytes {
            return Err(CodegenError::Unsupported(format!(
                "std::str::builder_finish: builder must be Bytes, got {:?}",
                b_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_str_builder_finish")
            .expect("lotus_str_builder_finish declared");
        let call = self
            .builder
            .build_call(f, &[b_val.into()], "sb.finish.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::String))
    }

    /// v1.x-16: `std::str::can_parse_float(s: String) -> Bool`.
    fn lower_std_str_can_parse_float(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::can_parse_float takes 1 arg (s), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if s_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::str::can_parse_float: s must be String, got {:?}",
                s_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let f = self
            .module
            .get_function("lotus_str_can_parse_float")
            .expect("lotus_str_can_parse_float declared");
        let call = self
            .builder
            .build_call(f, &[s_val.into()], "can.parse.float.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_bool = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret_i32,
                i32_t.const_zero(),
                "can.parse.float.bool",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_bool.into(), CodegenTy::Bool))
    }

    // ---- m96: std::ts (tree-sitter) lowering helpers ----
    //
    // The path-call dispatch arms (in `lower_stdlib_path_call`
    // and `_expr`) route `std::ts::*` to these helpers. Each one
    // is a thin wrapper over a `lotus_ts_*` extern declared in
    // `declare_builtins`. Tree and node handles are i64 — 1-based
    // with 0 as the "absent / failed" sentinel so the Aperio side
    // can branch on zero without wrestling with Option types.

    /// Lower `std::ts::parse_go(src: String) -> Int`. Returns a
    /// tree handle (>=1) on success or 0 on failure.
    fn lower_std_ts_parse_go(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::ts::parse_go takes 1 arg (src), got {}",
                args.len()
            )));
        }
        let (src_val, src_ty) = self.lower_expr(&args[0], scope)?;
        if src_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::ts::parse_go: src must be String, got {:?}",
                src_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_ts_parse_go")
            .expect("lotus_ts_parse_go declared");
        let call = self
            .builder
            .build_call(f, &[src_val.into()], "ts.parse.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((v, CodegenTy::Int))
    }

    /// Single-Int-arg → Int helper. Used by `root_node`, child-
    /// count, named-child-count, start_byte, end_byte, is_named.
    /// Picks the extern by name; threads the arg through unchanged.
    fn lower_std_ts_int1_to_int(
        &mut self,
        extern_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
        path: &str,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "{} takes 1 arg, got {}",
                path,
                args.len()
            )));
        }
        let (v, ty) = self.lower_expr(&args[0], scope)?;
        if ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "{}: arg must be Int, got {:?}",
                path, ty
            )));
        }
        let f = self
            .module
            .get_function(extern_name)
            .unwrap_or_else(|| panic!("{} declared", extern_name));
        let call = self
            .builder
            .build_call(f, &[v.into()], "ts.i1.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let r = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((r, CodegenTy::Int))
    }

    /// Two-Int-arg → Int helper. Used by `node_child` and
    /// `node_named_child`.
    fn lower_std_ts_int2_to_int(
        &mut self,
        extern_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
        path: &str,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "{} takes 2 args, got {}",
                path,
                args.len()
            )));
        }
        let (a, at) = self.lower_expr(&args[0], scope)?;
        if at != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "{}: arg 0 must be Int, got {:?}",
                path, at
            )));
        }
        let (b, bt) = self.lower_expr(&args[1], scope)?;
        if bt != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "{}: arg 1 must be Int, got {:?}",
                path, bt
            )));
        }
        let f = self
            .module
            .get_function(extern_name)
            .unwrap_or_else(|| panic!("{} declared", extern_name));
        let call = self
            .builder
            .build_call(f, &[a.into(), b.into()], "ts.i2.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let r = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((r, CodegenTy::Int))
    }

    /// Single-Int-arg → String helper. Used by `node_kind` and
    /// `node_text`. The returned pointer is owned by the lazy
    /// global payload arena (lifetime = program), so the caller
    /// can stash it anywhere without lifetime concerns — same
    /// shape as `std::io::fs::read_file`'s String return.
    fn lower_std_ts_int1_to_string(
        &mut self,
        extern_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
        path: &str,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "{} takes 1 arg, got {}",
                path,
                args.len()
            )));
        }
        let (v, ty) = self.lower_expr(&args[0], scope)?;
        if ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "{}: arg must be Int, got {:?}",
                path, ty
            )));
        }
        let f = self
            .module
            .get_function(extern_name)
            .unwrap_or_else(|| panic!("{} declared", extern_name));
        let call = self
            .builder
            .build_call(f, &[v.into()], "ts.s.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let r = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((r, CodegenTy::String))
    }

    /// Lower `std::env::args_count() -> Int`. Returns argc as
    /// captured in main's prelude (m77 codegen change).
    fn lower_std_env_args_count(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::env::args_count takes 0 args, got {}",
                args.len()
            )));
        }
        let i64_t = self.context.i64_type();
        let f = self
            .module
            .get_function("lotus_env_args_count")
            .expect("lotus_env_args_count declared");
        let call = self
            .builder
            .build_call(f, &[], "argc.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let raw = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ext = self
            .builder
            .build_int_s_extend(raw, i64_t, "argc.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ext.into(), CodegenTy::Int))
    }

    /// Lower `std::env::arg(i: Int) -> String`. Returns argv[i]
    /// for valid i; out-of-range indices return the empty
    /// String (the C runtime's stable g_empty_str). Negative i
    /// also returns empty rather than UB.
    fn lower_std_env_arg(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::env::arg takes 1 arg (index), got {}",
                args.len()
            )));
        }
        let (i_val, i_ty) = self.lower_expr(&args[0], scope)?;
        if i_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::env::arg: index must be Int, got {:?}",
                i_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i_i32 = self
            .builder
            .build_int_truncate(i_val.into_int_value(), i32_t, "arg.i.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_env_arg")
            .expect("lotus_env_arg declared");
        let call = self
            .builder
            .build_call(f, &[i_i32.into()], "arg.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::String))
    }

    /// Lower `std::env::var(name: String) -> String`. Returns the
    /// env value or empty String for unset vars. Use
    /// `std::env::var_exists` to disambiguate.
    fn lower_std_env_var(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::env::var takes 1 arg (name), got {}",
                args.len()
            )));
        }
        let (name_val, name_ty) = self.lower_expr(&args[0], scope)?;
        if name_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::env::var: name must be String, got {:?}",
                name_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_env_var")
            .expect("lotus_env_var declared");
        let call = self
            .builder
            .build_call(f, &[name_val.into()], "var.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::String))
    }

    /// Lower `std::env::var_exists(name: String) -> Bool`.
    fn lower_std_env_var_exists(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::env::var_exists takes 1 arg (name), got {}",
                args.len()
            )));
        }
        let (name_val, name_ty) = self.lower_expr(&args[0], scope)?;
        if name_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::env::var_exists: name must be String, got {:?}",
                name_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let f = self
            .module
            .get_function("lotus_env_var_exists")
            .expect("lotus_env_var_exists declared");
        let call = self
            .builder
            .build_call(f, &[name_val.into()], "var_exists.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_bool = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret_i32,
                i32_t.const_zero(),
                "var.exists.bool",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_bool.into(), CodegenTy::Bool))
    }

    /// m89: Lower `std::io::fs::read_bytes(path: String) -> Bytes`.
    /// Routes to `lotus_fs_read_bytes_global` so the resulting
    /// Bytes blob lives in the lazy global payload arena (same
    /// lifetime story as read_file's String). Embedded NUL bytes
    /// are preserved because Bytes carries an explicit length
    /// prefix — the reason this exists alongside read_file.
    fn lower_std_io_fs_read_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::read_bytes takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if path_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::read_bytes: path must be String, got {:?}",
                path_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_fs_read_bytes_global")
            .expect("lotus_fs_read_bytes_global declared");
        let call = self
            .builder
            .build_call(f, &[path_val.into()], "fs.read_bytes.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("lotus_fs_read_bytes_global returns ptr");
        Ok((v, CodegenTy::Bytes))
    }

    /// m90: Lower `std::io::fs::list_dir(path: String) -> String`.
    /// Returns a newline-separated String of entry names (skipping
    /// `.` and `..`). Empty string on error / missing dir / empty
    /// dir — callers distinguish via `len(result) == 0`.
    fn lower_std_io_fs_list_dir(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if path_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir: path must be String, got {:?}",
                path_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_fs_list_dir_global")
            .expect("lotus_fs_list_dir_global declared");
        let call = self
            .builder
            .build_call(f, &[path_val.into()], "fs.list_dir.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("lotus_fs_list_dir_global returns ptr");
        Ok((v, CodegenTy::String))
    }

    /// m89: Lower `std::io::tcp::__send_bytes(fd: Int, b: Bytes) -> Int`.
    /// Wraps `lotus_tcp_send_bytes` — same shape as `__send` but
    /// uses the explicit Bytes length, no NUL truncation. Returns
    /// the i32-cast-to-i64 result (0 on success, -1 on error)
    /// so user code can branch on it.
    fn lower_std_io_tcp_send_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__send_bytes takes 2 args (fd, bytes), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__send_bytes: fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[1], scope)?;
        if b_ty != CodegenTy::Bytes {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__send_bytes: bytes must be Bytes, got {:?}",
                b_ty
            )));
        }
        // Truncate fd's Int (i64) → i32 to match the C ABI.
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(
                fd_val.into_int_value(),
                i32_t,
                "fd.i32",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tcp_send_bytes")
            .expect("lotus_tcp_send_bytes declared");
        let call = self
            .builder
            .build_call(
                f,
                &[fd_i32.into(), b_val.into()],
                "tcp.send_bytes.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        // Sign-extend i32 → i64 so the result fits the Int contract.
        let i64_t = self.context.i64_type();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "send_bytes.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::fs::read_file(path: String) -> String`.
    /// Two-phase: stat the file to learn its size, allocate a
    /// (size+1)-byte buffer in the lazy global payload arena
    /// (so the resulting String outlives the call frame), then
    /// read into it and NUL-terminate at the actual bytes-read
    /// offset. If the file is missing or unreadable, both
    /// file_size and read_file return -1; we clamp to 0 and
    /// hand back an empty String. Callers that need to
    /// distinguish "empty file" from "missing file" use
    /// `std::io::fs::file_exists` first.
    fn lower_std_io_fs_read_file(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::read_file takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if path_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::read_file: path must be String, got {:?}",
                path_ty
            )));
        }
        let i8_t = self.context.i8_type();
        let i64_t = self.context.i64_type();
        let zero64 = i64_t.const_zero();
        let one64 = i64_t.const_int(1, false);

        // 1. Get the file size.
        let size_fn = self
            .module
            .get_function("lotus_fs_file_size")
            .expect("lotus_fs_file_size declared");
        let size_call = self
            .builder
            .build_call(size_fn, &[path_val.into()], "fs.size")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let raw_size = size_call
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();

        // 2. Clamp negative size to 0 so the alloc/read paths
        //    proceed without a separate error branch.
        let is_neg = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SLT,
                raw_size,
                zero64,
                "size.neg",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let safe_size = self
            .builder
            .build_select(is_neg, zero64, raw_size, "size.safe")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let alloc_size = self
            .builder
            .build_int_add(safe_size, one64, "alloc.size")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // 3. Allocate (size+1) bytes in the lazy global arena.
        let alloc_fn = self
            .module
            .get_function("lotus_bus_payload_arena_alloc")
            .expect("lotus_bus_payload_arena_alloc declared");
        let buf_call = self
            .builder
            .build_call(
                alloc_fn,
                &[alloc_size.into(), one64.into()],
                "fs.buf",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let buf_ptr = buf_call
            .try_as_basic_value()
            .left()
            .expect("returns ptr")
            .into_pointer_value();

        // 4. Read into the buffer.
        let read_fn = self
            .module
            .get_function("lotus_fs_read_file")
            .expect("lotus_fs_read_file declared");
        let read_call = self
            .builder
            .build_call(
                read_fn,
                &[path_val.into(), buf_ptr.into(), safe_size.into()],
                "fs.read",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let raw_n = read_call
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();

        // 5. Clamp bytes-read to 0 for negative returns.
        let n_neg = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SLT,
                raw_n,
                zero64,
                "n.neg",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let safe_n = self
            .builder
            .build_select(n_neg, zero64, raw_n, "n.safe")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();

        // 6. NUL-terminate at offset safe_n.
        let nul_ptr = unsafe {
            self.builder
                .build_in_bounds_gep(i8_t, buf_ptr, &[safe_n], "nul.ptr")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        self.builder
            .build_store(nul_ptr, i8_t.const_zero())
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        Ok((buf_ptr.into(), CodegenTy::String))
    }

    /// Lower `std::io::fs::write_file(path: String, content:
    /// String) -> Int`. Returns 0 on success, -1 on error.
    /// Truncates any existing file. Length is computed from
    /// the content's String pointer via lotus_str_len (Aperio
    /// Strings are NUL-terminated in memory).
    fn lower_std_io_fs_write_file(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::write_file takes 2 args (path, content), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if path_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::write_file: path must be String, got {:?}",
                path_ty
            )));
        }
        let (content_val, content_ty) = self.lower_expr(&args[1], scope)?;
        if content_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::write_file: content must be String, got {:?}",
                content_ty
            )));
        }
        let i64_t = self.context.i64_type();

        // strlen(content) → i64 length on the wire.
        let len_fn = self
            .module
            .get_function("lotus_str_len")
            .expect("lotus_str_len declared");
        let len_call = self
            .builder
            .build_call(len_fn, &[content_val.into()], "wf.len")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let len_i64 = len_call
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();

        let write_fn = self
            .module
            .get_function("lotus_fs_write_file")
            .expect("lotus_fs_write_file declared");
        let write_call = self
            .builder
            .build_call(
                write_fn,
                &[path_val.into(), content_val.into(), len_i64.into()],
                "fs.write.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = write_call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "wf.ret.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::fs::write_file_append(path, content) -> Int`.
    /// Same shape as write_file but opens the file with O_APPEND
    /// instead of O_TRUNC. Returns 0 on success, -1 on error.
    /// Resolves the apps/log-router friction "no append primitive
    /// forces buffer-everything-then-flush at dissolve."
    fn lower_std_io_fs_write_file_append(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::write_file_append takes 2 args (path, content), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if path_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::write_file_append: path must be String, got {:?}",
                path_ty
            )));
        }
        let (content_val, content_ty) = self.lower_expr(&args[1], scope)?;
        if content_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::write_file_append: content must be String, got {:?}",
                content_ty
            )));
        }
        let i64_t = self.context.i64_type();
        let len_fn = self
            .module
            .get_function("lotus_str_len")
            .expect("lotus_str_len declared");
        let len_call = self
            .builder
            .build_call(len_fn, &[content_val.into()], "wfa.len")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let len_i64 = len_call
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();
        let f = self
            .module
            .get_function("lotus_fs_write_file_append")
            .expect("lotus_fs_write_file_append declared");
        let call = self
            .builder
            .build_call(
                f,
                &[path_val.into(), content_val.into(), len_i64.into()],
                "fs.wfa.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "wfa.ret.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::fs::mkdir(path: String) -> Int`. Single-
    /// level only (NOT recursive). Returns 0 on success, -1 on
    /// error (errno set; EEXIST if the dir already exists).
    /// Resolves the apps/ssg friction "no mkdir / create_dir
    /// forces shell-out via README precondition."
    fn lower_std_io_fs_mkdir(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::mkdir takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if path_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::mkdir: path must be String, got {:?}",
                path_ty
            )));
        }
        let i64_t = self.context.i64_type();
        let f = self
            .module
            .get_function("lotus_fs_mkdir")
            .expect("lotus_fs_mkdir declared");
        let call = self
            .builder
            .build_call(f, &[path_val.into()], "fs.mkdir.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "mkdir.ret.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::fs::file_size(path: String) -> Int`.
    /// Returns the byte size or -1 on error.
    fn lower_std_io_fs_file_size(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::file_size takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if path_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::file_size: path must be String, got {:?}",
                path_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_fs_file_size")
            .expect("lotus_fs_file_size declared");
        let call = self
            .builder
            .build_call(f, &[path_val.into()], "fs.size.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let size_i64 = call
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();
        Ok((size_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::fs::file_exists(path: String) -> Bool`.
    /// Returns true if the path exists, false otherwise.
    fn lower_std_io_fs_file_exists(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::file_exists takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if path_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::file_exists: path must be String, got {:?}",
                path_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i1_t = self.context.bool_type();
        let f = self
            .module
            .get_function("lotus_fs_file_exists")
            .expect("lotus_fs_file_exists declared");
        let call = self
            .builder
            .build_call(f, &[path_val.into()], "fs.exists.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        // Truncate i32 0/1 to i1 for Aperio Bool.
        let ret_bool = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret_i32,
                i32_t.const_zero(),
                "exists.bool",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let _ = i1_t; // silence unused warning if any
        Ok((ret_bool.into(), CodegenTy::Bool))
    }

    /// Lower `std::io::fs::extension(path: String) -> String`.
    /// Returns the basename's last-dot suffix including the
    /// leading dot (".go", ".md"), or the empty string when
    /// there is no extension. Result lives in the global
    /// payload arena (same lifetime as list_dir / read_file).
    fn lower_std_io_fs_extension(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::extension takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if path_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::extension: path must be String, got {:?}",
                path_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_fs_extension_global")
            .expect("lotus_fs_extension_global declared");
        let call = self
            .builder
            .build_call(f, &[path_val.into()], "fs.extension.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("lotus_fs_extension_global returns ptr");
        Ok((v, CodegenTy::String))
    }

    /// Lower `std::io::tcp::__send(fd: Int, msg: String) -> Int`.
    /// Writes msg's bytes (strlen-determined length) to fd.
    /// Returns 0 on success, -1 on error.
    fn lower_std_io_tcp_send(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__send takes 2 args (fd, msg), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__send: fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let (msg_val, msg_ty) = self.lower_expr(&args[1], scope)?;
        if msg_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__send: msg must be String, got {:?}",
                msg_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "send.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tcp_send_str")
            .expect("lotus_tcp_send_str declared");
        let call = self
            .builder
            .build_call(f, &[fd_i32.into(), msg_val.into()], "send.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "send.ret.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::tcp::__recv(fd: Int, max_bytes: Int) ->
    /// String`. Returns up to max_bytes from fd as a String
    /// (allocated in the lazy global arena, stable for program
    /// lifetime). Empty String on EOF or error.
    fn lower_std_io_tcp_recv(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__recv takes 2 args (fd, max_bytes), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__recv: fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let (max_val, max_ty) = self.lower_expr(&args[1], scope)?;
        if max_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__recv: max_bytes must be Int, got {:?}",
                max_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "recv.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let max_i32 = self
            .builder
            .build_int_truncate(max_val.into_int_value(), i32_t, "recv.max.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tcp_recv_str")
            .expect("lotus_tcp_recv_str declared");
        let call = self
            .builder
            .build_call(f, &[fd_i32.into(), max_i32.into()], "recv.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::String))
    }

    /// Phase 2g: lower `std::io::tcp::__recv_bytes(fd: Int,
    /// max_bytes: Int) -> Bytes`. Mirrors `__recv` but returns
    /// a length-prefixed Bytes blob anchored in the global
    /// payload arena, so embedded NUL bytes survive intact.
    /// Empty Bytes (length 0) on EOF, fd errors, or cap <= 0.
    fn lower_std_io_tcp_recv_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__recv_bytes takes 2 args (fd, max_bytes), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__recv_bytes: fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let (max_val, max_ty) = self.lower_expr(&args[1], scope)?;
        if max_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__recv_bytes: max_bytes must be Int, got {:?}",
                max_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "recvb.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let max_i32 = self
            .builder
            .build_int_truncate(
                max_val.into_int_value(),
                i32_t,
                "recvb.max.i32",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tcp_recv_bytes")
            .expect("lotus_tcp_recv_bytes declared");
        let call = self
            .builder
            .build_call(f, &[fd_i32.into(), max_i32.into()], "recvb.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// Phase 2g: lower `std::str::from_bytes(b: Bytes) -> String`.
    /// Allocates a (len+1)-byte buffer in the global payload arena,
    /// memcpys the Bytes body, NUL-terminates. Embedded NULs in the
    /// source persist in the buffer but the strlen-based String view
    /// will truncate at the first — by design (callers who need
    /// NUL-safe handling stay in Bytes).
    fn lower_std_str_from_bytes(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::str::from_bytes takes 1 arg (b), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if b_ty != CodegenTy::Bytes {
            return Err(CodegenError::Unsupported(format!(
                "std::str::from_bytes: b must be Bytes, got {:?}",
                b_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_str_from_bytes")
            .expect("lotus_str_from_bytes declared");
        let call = self
            .builder
            .build_call(f, &[b_val.into()], "str_from_bytes.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::String))
    }

    /// Phase 2g: lower `std::bytes::from_string(s: String) -> Bytes`.
    /// strlen the source, allocate a Bytes blob of that length in
    /// the global payload arena, memcpy the body. Symmetric inverse
    /// of std::str::from_bytes.
    fn lower_std_bytes_from_string(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::from_string takes 1 arg (s), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if s_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::from_string: s must be String, got {:?}",
                s_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_bytes_from_str")
            .expect("lotus_bytes_from_str declared");
        let call = self
            .builder
            .build_call(f, &[s_val.into()], "bytes_from_str.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// Phase 2g: lower `std::bytes::at(b: Bytes, i: Int) -> Int`.
    /// Byte-as-Int accessor — returns the i-th byte's unsigned
    /// value (0..255) sign-extended into i64. Returns -1 if i is
    /// out of range. Pairs with std::bytes::slice and std::bytes::
    /// from_string for binary protocol parsing.
    fn lower_std_bytes_at(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::at takes 2 args (b, i), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if b_ty != CodegenTy::Bytes {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::at: b must be Bytes, got {:?}",
                b_ty
            )));
        }
        let (i_val, i_ty) = self.lower_expr(&args[1], scope)?;
        if i_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::at: i must be Int, got {:?}",
                i_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_bytes_at")
            .expect("lotus_bytes_at declared");
        let call = self
            .builder
            .build_call(f, &[b_val.into(), i_val.into()], "bytes_at.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((ret, CodegenTy::Int))
    }

    /// Phase 2g: lower `std::bytes::slice(b: Bytes, lo: Int, hi: Int)
    /// -> Bytes`. Half-open range [lo, hi); out-of-range bounds
    /// clamp; hi <= lo yields an empty Bytes. The result is a copy
    /// (not a view) so it composes with deep-copy-shaped lifetime
    /// conventions.
    fn lower_std_bytes_slice(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::slice takes 3 args (b, lo, hi), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if b_ty != CodegenTy::Bytes {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::slice: b must be Bytes, got {:?}",
                b_ty
            )));
        }
        let (lo_val, lo_ty) = self.lower_expr(&args[1], scope)?;
        if lo_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::slice: lo must be Int, got {:?}",
                lo_ty
            )));
        }
        let (hi_val, hi_ty) = self.lower_expr(&args[2], scope)?;
        if hi_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::slice: hi must be Int, got {:?}",
                hi_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_bytes_slice")
            .expect("lotus_bytes_slice declared");
        let call = self
            .builder
            .build_call(
                f,
                &[b_val.into(), lo_val.into(), hi_val.into()],
                "bytes_slice.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// ws-echo `bytes-construction-from-ints`: lower
    /// `std::bytes::from_int(v: Int) -> Bytes`. Builds a single-
    /// byte Bytes blob from the low 8 bits of `v`. Anchored in
    /// the program-lifetime payload arena, so the returned
    /// pointer matches recv_bytes / bytes_slice lifetime
    /// conventions and can flow through bus payloads without
    /// extra copying.
    fn lower_std_bytes_from_int(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::from_int takes 1 arg (v), got {}",
                args.len()
            )));
        }
        let (v_val, v_ty) = self.lower_expr(&args[0], scope)?;
        if v_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::from_int: v must be Int, got {:?}",
                v_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_bytes_from_int")
            .expect("lotus_bytes_from_int declared");
        let call = self
            .builder
            .build_call(f, &[v_val.into()], "bytes_from_int.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// ws-echo `bytes-construction-from-ints`: lower
    /// `std::bytes::concat(a: Bytes, b: Bytes) -> Bytes`.
    /// Returns a fresh Bytes containing `a` followed by `b`,
    /// allocated in the program-lifetime payload arena. With
    /// `from_int`, recursive concat composes any outbound
    /// byte sequence (WebSocket frame headers, length prefixes,
    /// custom binary protocols).
    fn lower_std_bytes_concat(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::concat takes 2 args (a, b), got {}",
                args.len()
            )));
        }
        let (a_val, a_ty) = self.lower_expr(&args[0], scope)?;
        if a_ty != CodegenTy::Bytes {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::concat: a must be Bytes, got {:?}",
                a_ty
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[1], scope)?;
        if b_ty != CodegenTy::Bytes {
            return Err(CodegenError::Unsupported(format!(
                "std::bytes::concat: b must be Bytes, got {:?}",
                b_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_bytes_concat")
            .expect("lotus_bytes_concat declared");
        let call = self
            .builder
            .build_call(
                f,
                &[a_val.into(), b_val.into()],
                "bytes_concat.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// ws-echo `sha1-base64-missing`: lower
    /// `std::crypto::sha1(b: Bytes) -> Bytes`. Returns a 20-byte
    /// digest. Stand-alone implementation in the C runtime per
    /// RFC 3174 — no OpenSSL dependency. Anchored in the
    /// program-lifetime payload arena.
    fn lower_std_crypto_sha1(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::sha1 takes 1 arg (b), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if b_ty != CodegenTy::Bytes {
            return Err(CodegenError::Unsupported(format!(
                "std::crypto::sha1: b must be Bytes, got {:?}",
                b_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_crypto_sha1")
            .expect("lotus_crypto_sha1 declared");
        let call = self
            .builder
            .build_call(f, &[b_val.into()], "sha1.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// ws-echo `sha1-base64-missing`: lower
    /// `std::text::base64::encode(b: Bytes) -> String`. Standard
    /// alphabet, `=` padding to multiple of 4. Anchored in the
    /// payload arena.
    fn lower_std_text_base64_encode(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::text::base64::encode takes 1 arg (b), got {}",
                args.len()
            )));
        }
        let (b_val, b_ty) = self.lower_expr(&args[0], scope)?;
        if b_ty != CodegenTy::Bytes {
            return Err(CodegenError::Unsupported(format!(
                "std::text::base64::encode: b must be Bytes, got {:?}",
                b_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_text_base64_encode")
            .expect("lotus_text_base64_encode declared");
        let call = self
            .builder
            .build_call(f, &[b_val.into()], "b64.encode.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::String))
    }

    /// v1.x-16: lower
    /// `std::text::base64::decode(s: String) -> Bytes`. Standard
    /// alphabet, padding tolerated, whitespace ignored. Returns
    /// the empty Bytes blob on parse failure (non-alphabet char,
    /// wrong length, too much padding). Anchored in the payload
    /// arena.
    fn lower_std_text_base64_decode(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::text::base64::decode takes 1 arg (s), got {}",
                args.len()
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[0], scope)?;
        if s_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::text::base64::decode: s must be String, got {:?}",
                s_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_text_base64_decode")
            .expect("lotus_text_base64_decode declared");
        let call = self
            .builder
            .build_call(f, &[s_val.into()], "b64.decode.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::Bytes))
    }

    /// ws-echo `random-seed-missing`: lower
    /// `std::rand::seed_from_time()` — re-seed the shared xorshift64*
    /// state from CLOCK_MONOTONIC. Library-internal use only; not
    /// cryptographically secure. Statement-position only.
    fn lower_std_rand_seed_from_time(
        &mut self,
        args: &[Expr],
    ) -> Result<(), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::rand::seed_from_time takes 0 args, got {}",
                args.len()
            )));
        }
        let f = self
            .module
            .get_function("lotus_rand_seed_from_time")
            .expect("lotus_rand_seed_from_time declared");
        self.builder
            .build_call(f, &[], "rand.seed")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

    /// ws-echo `random-seed-missing`: lower
    /// `std::rand::next_int(max: Int) -> Int` — uniform-ish int in
    /// [0, max). max <= 0 returns 0. Auto-seeds from monotonic
    /// time on first call so callers that forget the explicit
    /// seed still get distinct values per process run.
    fn lower_std_rand_next_int(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::rand::next_int takes 1 arg (max), got {}",
                args.len()
            )));
        }
        let (max_val, max_ty) = self.lower_expr(&args[0], scope)?;
        if max_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::rand::next_int: max must be Int, got {:?}",
                max_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_rand_next_int")
            .expect("lotus_rand_next_int declared");
        let call = self
            .builder
            .build_call(f, &[max_val.into()], "rand.next.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((v, CodegenTy::Int))
    }

    /// Phase 2e: lower `std::io::fs::list_dir_count(path: String)
    /// -> Int`. Returns the number of entries in `path` (skipping
    /// `.` / `..`), 0 on error or empty directory. Shares the
    /// global payload arena cache with `list_dir_at` so the
    /// directory read amortises across both calls.
    fn lower_std_io_fs_list_dir_count(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir_count takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if path_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir_count: path must be String, got {:?}",
                path_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_fs_list_dir_count")
            .expect("lotus_fs_list_dir_count declared");
        let call = self
            .builder
            .build_call(f, &[path_val.into()], "ld.count.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret = call
            .try_as_basic_value()
            .left()
            .expect("returns i64");
        Ok((ret, CodegenTy::Int))
    }

    /// Phase 2e: lower `std::io::fs::list_dir_at(path: String,
    /// idx: Int) -> String`. Returns the `idx`-th entry name
    /// (0-indexed), or the empty string if out of range. Shares
    /// the global payload arena cache with `list_dir_count`.
    fn lower_std_io_fs_list_dir_at(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir_at takes 2 args (path, idx), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if path_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir_at: path must be String, got {:?}",
                path_ty
            )));
        }
        let (idx_val, idx_ty) = self.lower_expr(&args[1], scope)?;
        if idx_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::list_dir_at: idx must be Int, got {:?}",
                idx_ty
            )));
        }
        let f = self
            .module
            .get_function("lotus_fs_list_dir_at")
            .expect("lotus_fs_list_dir_at declared");
        let call = self
            .builder
            .build_call(
                f,
                &[path_val.into(), idx_val.into()],
                "ld.at.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("returns ptr");
        Ok((ptr, CodegenTy::String))
    }

    /// Phase 2f: lower `std::io::fs::read_file_status(path:
    /// String) -> Int`. Returns 0 on success or the platform
    /// errno on failure — distinguishes "empty file" (status=0,
    /// `read_file(path)` returns "") from "missing / unreadable
    /// file" (status=errno, `read_file(path)` returns ""). Paired
    /// with the existing `read_file` for content; both walk the
    /// same kernel cache, so the cost of the second call is the
    /// hot-cache stat+open+read.
    fn lower_std_io_fs_read_file_status(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::read_file_status takes 1 arg (path), got {}",
                args.len()
            )));
        }
        let (path_val, path_ty) = self.lower_expr(&args[0], scope)?;
        if path_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "std::io::fs::read_file_status: path must be String, got {:?}",
                path_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let f = self
            .module
            .get_function("lotus_fs_read_file_status")
            .expect("lotus_fs_read_file_status declared");
        let call = self
            .builder
            .build_call(f, &[path_val.into()], "rfs.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let _ = i32_t;
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "rfs.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// Lower `std::io::tcp::__close_fd(fd: Int) -> Int`. Returns
    /// 0 on success, -1 on error (errno set). Most callers
    /// discard the return.
    fn lower_std_io_tcp_close_fd(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__close_fd takes 1 arg (fd), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::io::tcp::__close_fd: fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_tcp_close_fd")
            .expect("lotus_tcp_close_fd declared");
        let call = self
            .builder
            .build_call(f, &[fd_i32.into()], "close.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_i32 = call
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let ret_i64 = self
            .builder
            .build_int_s_extend(ret_i32, i64_t, "close.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((ret_i64.into(), CodegenTy::Int))
    }

    /// Statement-level locus instantiation `T { f: v, ... };`.
    /// Allocates a struct on the caller's stack, fills its fields
    /// (defaults overridden by the call site), then calls birth()
    /// and run() if present. The locus is ephemeral: when the
    /// surrounding fn returns the alloca is reclaimed. Long-lived
    /// loci wait on the cooperative scheduler + region allocator.
    /// m82: classify whether an expression is a struct literal
    /// that resolves to a locus (user-declared or stdlib-bundled).
    /// Used by `Stmt::Let` to gate `defer_next_locus_dissolve`:
    /// only locus literals produce a deferred-dissolve binding;
    /// user-type literals, scalars, calls, etc. stay on the eager
    /// path. Stdlib path-qualified locus literals
    /// (`std::io::tcp::Stream { ... }`) are matched via
    /// `stdlib_locus_for_path` so they get the same treatment as
    /// bare-name user loci.
    fn expr_is_locus_literal(&self, e: &Expr) -> bool {
        if let Expr::Struct { path, .. } = e {
            if path.segments.len() == 1 {
                return self
                    .user_loci
                    .contains_key(&path.segments[0].name);
            }
            let segs: Vec<&str> = path
                .segments
                .iter()
                .map(|s| s.name.as_str())
                .collect();
            // Use the generalized lookup, then narrow to loci.
            // m84: path-qualified stdlib `type` records resolve via
            // the same table; we mustn't accidentally classify them
            // as locus literals (they have no dissolve to defer).
            if let Some(mangled) = stdlib_mangled_for_path(&segs) {
                return self.user_loci.contains_key(mangled);
            }
        }
        false
    }

    fn lower_locus_instantiation(
        &mut self,
        locus_name: &str,
        inits: &[StructInit],
        scope: &Scope<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        // m82: the let-binding above us may have signaled that
        // this locus's dissolve should be deferred to the
        // enclosing fn's scope-exit flush. Take the flag now —
        // before any nested `lower_expr` calls below — so default
        // / override expressions that themselves construct loci
        // don't accidentally consume our flag and skip their own
        // eager dissolve. Outermost instantiation owns it; nested
        // ones see false.
        let defer_for_let = std::mem::take(&mut self.defer_next_locus_dissolve);
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

        // Deferred-dissolve gating: when this locus's dissolve
        // is deferred to a fn-exit flush (let-bound `let x = X{};`
        // or bus-subscribing long-lived locus), the body that
        // contains this `let` may not execute on every control-
        // flow path — e.g., an early `return` before the let.
        // The flush iterates frame entries unconditionally; if
        // we emit the struct's alloca at the let position, an
        // early-return path leaves it uninitialized and the
        // flush dereferences garbage (segv at a low address as
        // arena field offset is read off a near-null pointer).
        //
        // Fix: hoist the alloca to the fn entry block and
        // zero-init the arena field there. The let position
        // overwrites the arena field with the real arena when
        // it runs. The flush null-checks the arena field per
        // entry (see flush_dissolve_frame_kind) and skips
        // entries whose let never executed.
        let early_defer = defer_for_let
            || !info.subscriptions.is_empty();
        // 3d+3e: if the current self's locus accepts a child of
        // this locus's type, the parent is about to retain a
        // pointer to this instance via its synthetic
        // `__children[]` array (appended below in the accept/
        // append block). When the parent reads through that
        // array later — including in a *different* lifecycle
        // method than the one we're being instantiated in — a
        // stack alloca would dangle the moment the spawning
        // method returns. Detect that case here and route the
        // struct allocation through the parent's arena so it
        // lives until the parent's arena is destroyed. The
        // deferred-dissolve push at the end of this fn is also
        // suppressed so the spawning method's exit flush
        // doesn't tear the child down. v1 trade-off: the
        // child's drain()/dissolve() bodies don't fire on
        // process exit — a children-cascade at parent dissolve
        // would tighten this; deferred to v1.x. See the
        // resolution note in notes/aperio-friction.md
        // `nested-locus-child-field-reads-return-garbage`.
        let parent_accepts_us = if let Some(cs) = self.current_self.as_ref() {
            self.user_loci
                .get(&cs.locus_name)
                .and_then(|p| p.accept_param.as_ref().cloned())
                .map(|(_, child_ty)| child_ty == locus_name)
                .unwrap_or(false)
        } else {
            false
        };
        // m90 (3f fix): if the current fn declares `-> Self` for this
        // locus, the instance can escape to the caller. A stack alloca
        // becomes dangling the moment the method returns, so the first
        // post-return read of `s.field` (or `s.method()`) sees still-
        // valid stack memory but the second sees overwritten state.
        // Detect the escape ahead of time and heap-allocate via the
        // program-lifetime payload arena instead. The eager dissolve
        // + arena_destroy are also skipped below — the locus is
        // semantically "moved" to the caller and lives for the
        // program. v1 trade-off; a return-slot ABI (caller-provided
        // out-pointer + scoped dissolve in the caller's frame) would
        // tighten this without leaking. The same heap path also
        // covers `let s = X{}; ...; return s;` because the let-bound
        // literal is instantiated with `current_user_fn_ret` still
        // pointing at the matching LocusRef.
        let returns_this_locus = self
            .current_user_fn_ret
            .as_ref()
            .and_then(|r| r.as_ref())
            .map(|t| matches!(t, CodegenTy::LocusRef(n) if n == locus_name))
            .unwrap_or(false);
        let self_ptr = if returns_this_locus {
            let alloc_fn = self
                .module
                .get_function("lotus_bus_payload_arena_alloc")
                .expect("lotus_bus_payload_arena_alloc declared");
            let i64_t = self.context.i64_type();
            let size = info
                .struct_ty
                .size_of()
                .expect("locus struct ty has known size");
            self.builder
                .build_call(
                    alloc_fn,
                    &[size.into(), i64_t.const_int(8, false).into()],
                    &format!("{}.self.heap", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .try_as_basic_value()
                .left()
                .expect("lotus_bus_payload_arena_alloc returns ptr")
                .into_pointer_value()
        } else if parent_accepts_us {
            // 3d+3e fix: allocate the child struct in parent's arena.
            // Lives until parent's arena_destroy, so cross-lifecycle
            // reads through self.children stay valid (e.g. child
            // birthed in parent's birth(), read in parent's run()).
            let parent_self = self
                .current_self
                .as_ref()
                .cloned()
                .expect("parent_accepts_us implies current_self");
            let parent_info = self
                .user_loci
                .get(&parent_self.locus_name)
                .cloned()
                .expect("parent locus declared");
            let ptr_t = self.context.ptr_type(AddressSpace::default());
            let arena_field_ptr = self
                .builder
                .build_struct_gep(
                    parent_info.struct_ty,
                    parent_self.self_ptr,
                    parent_info.arena_field_idx,
                    &format!("{}.parent_arena.gep", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let parent_arena = self
                .builder
                .build_load(
                    ptr_t,
                    arena_field_ptr,
                    &format!("{}.parent_arena", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let alloc_fn = self
                .module
                .get_function("lotus_arena_alloc")
                .expect("lotus_arena_alloc declared");
            let i64_t = self.context.i64_type();
            let size = info
                .struct_ty
                .size_of()
                .expect("locus struct ty has known size");
            self.builder
                .build_call(
                    alloc_fn,
                    &[
                        parent_arena.into(),
                        size.into(),
                        i64_t.const_int(8, false).into(),
                    ],
                    &format!("{}.self.in_parent_arena", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .try_as_basic_value()
                .left()
                .expect("lotus_arena_alloc returns ptr")
                .into_pointer_value()
        } else if early_defer && self.current_fn.is_some() {
            self.alloca_in_entry_with_nulled_arena(
                info.struct_ty,
                info.arena_field_idx,
                &format!("{}.self", locus_name),
            )?
        } else {
            self.builder
                .build_alloca(info.struct_ty, &format!("{}.self", locus_name))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };

        // First — initialize the synthetic `__arena` field
        // (struct slot 0) with a fresh arena. Allocations made
        // on behalf of this locus during the rest of
        // instantiation (composite-literal defaults / overrides)
        // and during its lifecycle method bodies will route
        // through `arena_alloc`, which prefers `current_self`'s
        // arena field over the program global.
        //
        // m22: if our parent is a chunked-class locus actively
        // accepting us (current_self set, parent declares
        // accept(child: ThisLocus), parent.projection_class ==
        // Chunked), allocate as a sub-region of the parent's
        // arena rather than a fresh top-level arena. Parent
        // tracks a slot index for us; on dissolve, our slot
        // returns to the parent's free-list for reuse.
        // m22+m23: chunked AND recognition parents both route
        // accepted children through `lotus_arena_create_subregion`.
        // For chunked, that's the spec's per-coordinatee
        // sub-region with free-list bookkeeping. For recognition,
        // v0 reuses the chunked path as a deliberate stub —
        // recognition's pre-allocated bitmap pool is a perf
        // optimization (avoids malloc per accept) that v0 defers
        // until a workload exercises it. Functionally equivalent
        // to chunked at this layer; spec/memory.md flags the gap.
        let parent_subregion_accept = if let Some(cs) = self.current_self.as_ref() {
            let parent_info = self
                .user_loci
                .get(&cs.locus_name)
                .cloned()
                .expect("current_self points to a declared locus");
            let parent_accepts_us = parent_info
                .accept_param
                .as_ref()
                .map(|(_, child_ty)| child_ty == locus_name)
                .unwrap_or(false);
            if parent_accepts_us
                && matches!(
                    parent_info.projection_class,
                    ProjectionClass::Chunked | ProjectionClass::Recognition
                )
            {
                Some(cs.self_ptr)
            } else {
                None
            }
        } else {
            None
        };
        let new_arena = if let Some(parent_self_ptr) = parent_subregion_accept {
            let parent_info = self
                .user_loci
                .get(&self.current_self.as_ref().unwrap().locus_name)
                .cloned()
                .expect("parent declared");
            let arena_field_ptr = self
                .builder
                .build_struct_gep(
                    parent_info.struct_ty,
                    parent_self_ptr,
                    parent_info.arena_field_idx,
                    "parent.__arena.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let parent_arena = self
                .builder
                .build_load(
                    self.context.ptr_type(AddressSpace::default()),
                    arena_field_ptr,
                    "parent.__arena",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let subregion_fn = self
                .module
                .get_function("lotus_arena_create_subregion")
                .expect("lotus_arena_create_subregion declared");
            self.builder
                .build_call(
                    subregion_fn,
                    &[parent_arena.into()],
                    &format!("{}.arena.sub", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .try_as_basic_value()
                .left()
                .expect("subregion_create returns ptr")
        } else {
            let arena_create = self
                .module
                .get_function("lotus_arena_create")
                .expect("lotus_arena_create declared");
            self.builder
                .build_call(arena_create, &[], &format!("{}.arena", locus_name))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .try_as_basic_value()
                .left()
                .expect("arena_create returns ptr")
        };
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

        // F.22 capacity slots: after slot 0 (arena) is set, init
        // each declared slot in declaration order by calling
        // `lotus_pool_create(size, 8)` or `lotus_heap_create(size,
        // 8)` and storing the returned allocator pointer into the
        // slot's struct field. Per spec §F.22 §"Slot lifetime",
        // slot init runs after slot 0 and before the locus's own
        // field initializers. The 8-byte alignment matches Aperio
        // v0's universal scalar alignment — every value-shape
        // type lays out at 8-byte alignment in the locus struct,
        // so cells inherit the same.
        for slot in &info.capacity_slots {
            let cell_size = self
                .llvm_basic_type(&slot.elem_ty)
                .size_of()
                .expect("cell type has known size at LLVM level");
            let align_const = self.context.i64_type().const_int(8, false);
            let create_fn_name = match slot.kind {
                CapacitySlotKind::Pool => "lotus_pool_create",
                CapacitySlotKind::Heap => "lotus_heap_create",
            };
            let create_fn = self
                .module
                .get_function(create_fn_name)
                .expect("F.22 allocator extern declared");
            let allocator_ptr = self
                .builder
                .build_call(
                    create_fn,
                    &[cell_size.into(), align_const.into()],
                    &format!("{}.{}.create", locus_name, slot.name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .try_as_basic_value()
                .left()
                .expect("F.22 allocator create returns ptr");
            let slot_field_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    slot.struct_field_idx,
                    &format!("{}.__slot_{}.ptr", locus_name, slot.name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(slot_field_ptr, allocator_ptr)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }

        // Initialize each field. Overrides go through lower_expr in
        // the caller's scope so any expression — not just literals —
        // can be passed. Defaults are either pre-resolved scalar
        // literals (DefaultInit::Const → const_param) or deferred
        // expressions (DefaultInit::Expr → lower_expr) that may
        // construct composite values like `Kernel { ... }` at
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

        // m40: zero-init the synthetic __restart_count field.
        // Always present on every locus struct so the
        // `restart(child)` recovery primitive can bump it
        // without first checking whether the locus opted in.
        // Cap of 2 attempts per locus lifetime — past that,
        // restart() returns false and the violation falls
        // through to the parent's collapse path.
        let rc_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.restart_count_field_idx,
                &format!("{}.__restart_count.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let zero = self.context.i64_type().const_int(0, false);
        self.builder
            .build_store(rc_ptr, zero)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // m41: zero-init the synthetic __quarantined flag.
        let q_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.quarantined_field_idx,
                &format!("{}.__quarantined.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(q_ptr, zero)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // m45: zero-init the synthetic __restart_in_place_pending
        // flag. restart_in_place(c) sets it to 1; the rerun
        // branch in __birth_closures reads + clears it.
        let rip_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.restart_in_place_pending_field_idx,
                &format!("{}.__restart_in_place_pending.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(rip_ptr, zero)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // m42: init the synthetic __parent_self / __parent_on_failure
        // fields. Resolve the (parent_self, on_failure_fn) pair via
        // the same routing the birth/dissolve epochs use; the bus
        // drain loop's tick wrapper reads these later when firing
        // tick-epoch closures (it has no static call-site context
        // for parent routing, so we bake it onto the struct here).
        // Loci without tick closures still pay the 16 bytes — the
        // uniform layout is worth more than the overhead.
        let (parent_self_val, parent_handler_val) =
            self.resolve_failure_route(locus_name);
        let parent_self_slot = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.parent_self_field_idx,
                &format!("{}.__parent_self.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(parent_self_slot, parent_self_val)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let parent_handler_slot = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.parent_on_failure_field_idx,
                &format!("{}.__parent_on_failure.ptr", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(parent_handler_slot, parent_handler_val)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // m43: init each __duration_last_fire_<i> field to
        // monotonic-now so the first fire happens after the
        // declared `N` elapses (not immediately at birth).
        // One time::monotonic() call per duration closure —
        // a tiny cost paid only for loci that declare
        // duration epochs.
        if !info.duration_last_fire_field_idxs.is_empty() {
            let (now_v, _) = self.lower_time_monotonic(&[])?;
            let now = now_v.into_int_value();
            for (i, field_idx) in info
                .duration_last_fire_field_idxs
                .iter()
                .enumerate()
            {
                let slot = self
                    .builder
                    .build_struct_gep(
                        info.struct_ty,
                        self_ptr,
                        *field_idx,
                        &format!(
                            "{}.__duration_last_fire[{}].ptr",
                            locus_name, i
                        ),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(slot, now)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
        }

        // m46: zero each closure-accumulator slot at instantiation.
        // The slot's type drives the zero choice (Int/Duration use
        // i64 zero; Float/Decimal use f64 zero). Each `sum(self.X)`
        // detected during locus-decl gave us one slot.
        for slots in info.accumulators_per_closure.values() {
            for (i, slot) in slots.iter().enumerate() {
                self.zero_accumulator_slot(
                    info.struct_ty,
                    self_ptr,
                    slot,
                    &format!(
                        "{}.__acc[{}].ptr",
                        locus_name, i
                    ),
                )?;
            }
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
        // For pinned-with-subscriptions loci we'll call this loop
        // again BELOW (after the mailbox alloca), passing the
        // mailbox pointer; cooperative loci register here with
        // mailbox = None (route through the global queue).
        let pinned_subscriptions =
            matches!(info.schedule_class, ScheduleClass::Pinned(_))
                && !info.subscriptions.is_empty();
        if !pinned_subscriptions {
            for (subject, handler_name, payload_type) in &info.subscriptions {
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
                self.emit_bus_register(
                    subject,
                    self_ptr,
                    handler_fn,
                    None,
                    payload_type,
                )?;
            }
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
        let is_pinned =
            matches!(info.schedule_class, ScheduleClass::Pinned(_));

        // m28a + m28b: pinned-class loci spawn a pthread that runs
        // the locus's full lifecycle on its own thread:
        //   birth → run → (mailbox loop, if subscriptions) → drain → dissolve
        // Main thread joins at scope exit (deferred_dissolves
        // frame) before destroying the locus's arena. We synthesize
        // a per-locus thread_main whose signature matches pthread's
        // start-routine contract exactly (`ptr (ptr)`), so
        // pthread_create gets a direct function pointer with
        // self_ptr as its argument — no C adapter, no args struct.
        //
        // m28b: when the locus declares bus subscriptions, the
        // synthesized thread_main includes a mailbox loop after
        // run() — the pinned thread blocks in
        // lotus_mailbox_drain_one until cells arrive, processes
        // them one at a time (handler-atomic per substrate cell),
        // and exits the loop only when shutdown is signaled. The
        // mailbox itself is allocated at instantiation time and
        // stored in the locus's __mailbox field so the dispatch
        // path (which only sees the table-recorded mailbox ptr)
        // and the deferred-dissolve flush (which signals
        // shutdown) can both reach it.
        //
        // Still gated: accept (children of pinned would need
        // cross-thread cascade-dissolve coordination which adds
        // significant complexity beyond m28b), closures.
        if is_pinned {
            let ptr_t = self.context.ptr_type(AddressSpace::default());
            if info.methods.contains_key("accept") {
                return Err(CodegenError::Unsupported(format!(
                    "pinned locus `{}` declares `accept()`; pinned coordinators \
                     wait on a future cross-thread cascade-dissolve milestone",
                    locus_name
                )));
            }
            if info.birth_closures_fn.is_some()
                || info.dissolve_closures_fn.is_some()
            {
                return Err(CodegenError::Unsupported(format!(
                    "pinned locus `{}` declares closures; cross-thread closure \
                     routing not yet supported",
                    locus_name
                )));
            }

            let i64_t = self.context.i64_type();
            let i32_t = self.context.i32_type();

            // m28b: if the locus subscribes, allocate its mailbox
            // and store the pointer in the locus's __mailbox slot.
            // Then register all subscriptions with that mailbox so
            // bus dispatch routes cells here instead of to the
            // global queue.
            let mailbox_ptr_opt: Option<PointerValue<'ctx>> =
                if let Some(mb_idx) = info.mailbox_field_idx {
                    let create_fn = self
                        .module
                        .get_function("lotus_mailbox_create")
                        .expect("lotus_mailbox_create declared");
                    let mb_ptr = self
                        .builder
                        .build_call(
                            create_fn,
                            &[],
                            &format!("{}.mailbox.create", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                        .try_as_basic_value()
                        .left()
                        .expect("lotus_mailbox_create returns ptr")
                        .into_pointer_value();
                    let mb_slot = self
                        .builder
                        .build_struct_gep(
                            info.struct_ty,
                            self_ptr,
                            mb_idx,
                            &format!("{}.__mailbox.ptr", locus_name),
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    self.builder
                        .build_store(mb_slot, mb_ptr)
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    for (subject, handler_name, payload_type) in
                        &info.subscriptions
                    {
                        let handler_fn = info
                            .user_methods
                            .get(handler_name)
                            .copied()
                            .ok_or_else(|| {
                                CodegenError::Unsupported(format!(
                                    "locus `{}` subscribes to `{}` with handler \
                                     `{}` but no such method declared",
                                    locus_name, subject, handler_name
                                ))
                            })?;
                        self.emit_bus_register(
                            subject,
                            self_ptr,
                            handler_fn,
                            Some(mb_ptr),
                            payload_type,
                        )?;
                    }
                    Some(mb_ptr)
                } else {
                    None
                };

            // Synthesize __pinned_main_<LocusName>(self_ptr) -> ptr.
            // Body: birth → run → (mailbox loop if subscriptions) →
            // drain → dissolve, returning null.
            let saved_block = self
                .builder
                .get_insert_block()
                .expect("pinned spawn inside an active block");
            let thread_main_name =
                format!("__pinned_main_{}", locus_name);
            let thread_main_ty =
                ptr_t.fn_type(&[ptr_t.into()], false);
            let thread_main = self
                .module
                .add_function(&thread_main_name, thread_main_ty, None);
            let entry_bb = self
                .context
                .append_basic_block(thread_main, "entry");
            self.builder.position_at_end(entry_bb);
            let thread_self =
                thread_main.get_nth_param(0).unwrap().into_pointer_value();
            for kind in &["birth", "run"] {
                if let Some(method) = info.methods.get(*kind) {
                    self.builder
                        .build_call(
                            *method,
                            &[thread_self.into()],
                            &format!(
                                "{}.{}.thread_call",
                                locus_name, kind
                            ),
                        )
                        .map_err(|e| {
                            CodegenError::LlvmEmit(e.to_string())
                        })?;
                    // m42: tick fires after run() on the pinned
                    // thread too. Use the wrapper here (it loads
                    // parent fields from the struct) since we're
                    // off the main thread and resolve_failure_route
                    // wouldn't see the right `current_self`.
                    // m43-followup: duration fires here too via
                    // the matching wrapper, closing the v0 limit
                    // where pinned post-run() didn't fire duration.
                    if *kind == "run" {
                        for (wrapper_opt, tag) in [
                            (info.tick_wrapper_fn, "tick"),
                            (info.duration_wrapper_fn, "duration"),
                        ] {
                            if let Some(wrapper) = wrapper_opt {
                                self.builder
                                    .build_call(
                                        wrapper,
                                        &[thread_self.into()],
                                        &format!(
                                            "{}.{}.post_run.thread_call",
                                            locus_name, tag
                                        ),
                                    )
                                    .map_err(|e| {
                                        CodegenError::LlvmEmit(e.to_string())
                                    })?;
                            }
                        }
                    }
                }
            }
            // m28b: mailbox loop. Reload the mailbox ptr from the
            // locus's __mailbox slot (we're on the pinned thread,
            // not the main thread, so we can't capture mailbox_ptr
            // from the enclosing build context — re-derive it from
            // self_ptr). Loop calls lotus_mailbox_drain_one, which
            // returns 0 on shutdown-empty; break the loop and run
            // drain/dissolve.
            if let Some(mb_idx) = info.mailbox_field_idx {
                let mb_slot_in_thread = self
                    .builder
                    .build_struct_gep(
                        info.struct_ty,
                        thread_self,
                        mb_idx,
                        "thread.mailbox.ptr",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let mb_in_thread = self
                    .builder
                    .build_load(ptr_t, mb_slot_in_thread, "thread.mailbox")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_pointer_value();
                let drain_one_fn = self
                    .module
                    .get_function("lotus_mailbox_drain_one")
                    .expect("lotus_mailbox_drain_one declared");
                let loop_header =
                    self.context.append_basic_block(thread_main, "mb.header");
                let loop_after =
                    self.context.append_basic_block(thread_main, "mb.after");
                self.builder
                    .build_unconditional_branch(loop_header)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder.position_at_end(loop_header);
                let drained = self
                    .builder
                    .build_call(
                        drain_one_fn,
                        &[mb_in_thread.into()],
                        "mb.drain.one",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("lotus_mailbox_drain_one returns i32")
                    .into_int_value();
                let keep_going = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        drained,
                        i32_t.const_int(0, false),
                        "mb.keep.going",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_conditional_branch(
                        keep_going, loop_header, loop_after,
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder.position_at_end(loop_after);
            }
            for kind in &["drain", "dissolve"] {
                if let Some(method) = info.methods.get(*kind) {
                    self.builder
                        .build_call(
                            *method,
                            &[thread_self.into()],
                            &format!(
                                "{}.{}.thread_call",
                                locus_name, kind
                            ),
                        )
                        .map_err(|e| {
                            CodegenError::LlvmEmit(e.to_string())
                        })?;
                }
            }
            self.builder
                .build_return(Some(&ptr_t.const_null()))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            // Restore builder to the calling fn so the rest of
            // the instantiation (pthread_create) emits there.
            self.builder.position_at_end(saved_block);

            // pthread_t alloca in the enclosing fn frame.
            let tid_alloca = self
                .builder
                .build_alloca(i64_t, &format!("{}.tid", locus_name))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let thread_main_ptr =
                thread_main.as_global_value().as_pointer_value();
            let null_attr = ptr_t.const_null();
            let create_fn = self
                .module
                .get_function("pthread_create")
                .expect("pthread_create declared");
            self.builder
                .build_call(
                    create_fn,
                    &[
                        tid_alloca.into(),
                        null_attr.into(),
                        thread_main_ptr.into(),
                        self_ptr.into(),
                    ],
                    &format!("{}.pthread_create", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let _ = mailbox_ptr_opt;

            // m28c: optional CPU-core affinity. If the locus
            // declared `: schedule pinned(core = N)`, route the
            // freshly-created tid through pthread_setaffinity_np
            // (via the C-side helper) so the OS scheduler keeps
            // this thread on the requested logical CPU.
            if let ScheduleClass::Pinned(Some(core)) = info.schedule_class {
                let tid_for_aff = self
                    .builder
                    .build_load(i64_t, tid_alloca, "pinned.tid.aff")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let core_const = i32_t.const_int(core as u64, true);
                let set_aff_fn = self
                    .module
                    .get_function("lotus_set_core_affinity")
                    .expect("lotus_set_core_affinity declared");
                self.builder
                    .build_call(
                        set_aff_fn,
                        &[tid_for_aff.into(), core_const.into()],
                        &format!("{}.set_aff", locus_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }

            // Defer pthread_join + arena destroy to scope exit.
            // flush_dissolve_frame skips drain/dissolve for pinned
            // entries — those already ran on the pinned thread
            // before it returned (and pthread_join blocks until
            // that return). For pinned-with-subscriptions, the
            // flush ALSO signals the mailbox shutdown before
            // joining, so the pinned thread breaks out of its
            // mailbox loop and proceeds to drain/dissolve.
            if let Some(top) = self.deferred_dissolves.last_mut() {
                top.push((self_ptr, locus_name.to_string(), Some(tid_alloca)));
            } else {
                return Err(CodegenError::Unsupported(format!(
                    "pinned locus `{}` instantiated outside any tracked \
                     scope (no deferred-dissolve frame)",
                    locus_name
                )));
            }

            return Ok(self_ptr);
        }

        // m39: birth-epoch closures fire right after birth()
        // returns. We emit birth() + __birth_closures + run() in
        // sequence — the closure check sits between birth (which
        // initializes state) and run (which depends on that
        // state's invariants). If birth violates and the parent
        // has a matching on_failure handler, that handler runs;
        // otherwise the runtime exits with a diagnostic.
        if let Some(birth_fn) = info.methods.get("birth") {
            self.builder
                .build_call(
                    *birth_fn,
                    &[self_ptr.into()],
                    &format!("{}.birth.call", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        if let Some(birth_closures_fn) = info.birth_closures_fn {
            let (parent_self, handler_ptr) =
                self.resolve_failure_route(&locus_name);
            self.builder
                .build_call(
                    birth_closures_fn,
                    &[
                        self_ptr.into(),
                        parent_self.into(),
                        handler_ptr.into(),
                    ],
                    &format!("{}.__birth_closures.call", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        // m41: gate run() on __quarantined. If a parent's
        // on_failure called quarantine(self) during the birth-
        // closure check above, the flag is now set and we skip
        // run() entirely. Drain / dissolve still fire below.
        if let Some(run_fn) = info.methods.get("run") {
            let i64_t = self.context.i64_type();
            let q_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    info.quarantined_field_idx,
                    &format!("{}.run.quarantined.ptr", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let q_val = self
                .builder
                .build_load(i64_t, q_ptr, "run.quarantined")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let zero = i64_t.const_int(0, false);
            let active = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::EQ,
                    q_val.into_int_value(),
                    zero,
                    "run.active",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let func = self.current_fn.expect("current_fn set");
            let run_bb =
                self.context.append_basic_block(func, "run.do");
            let after_run_bb =
                self.context.append_basic_block(func, "run.after");
            self.builder
                .build_conditional_branch(active, run_bb, after_run_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder.position_at_end(run_bb);
            self.builder
                .build_call(
                    *run_fn,
                    &[self_ptr.into()],
                    &format!("{}.run.call", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            // m42: tick fires after run() returns — run() is a
            // substrate cell just like a bus handler. Place the
            // call in the active branch so it doesn't fire on a
            // skipped (quarantined) run().
            if let Some(tick_fn) = info.tick_closures_fn {
                let (parent_self_t, handler_ptr_t) =
                    self.resolve_failure_route(&locus_name);
                self.builder
                    .build_call(
                        tick_fn,
                        &[
                            self_ptr.into(),
                            parent_self_t.into(),
                            handler_ptr_t.into(),
                        ],
                        &format!(
                            "{}.__tick_closures.post_run.call",
                            locus_name
                        ),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            // m43: duration shares the cell-boundary cadence
            // with tick — fires only when declared `N` has
            // elapsed since last fire of each duration closure.
            if let Some(duration_fn) = info.duration_closures_fn {
                let (parent_self_d, handler_ptr_d) =
                    self.resolve_failure_route(&locus_name);
                self.builder
                    .build_call(
                        duration_fn,
                        &[
                            self_ptr.into(),
                            parent_self_d.into(),
                            handler_ptr_d.into(),
                        ],
                        &format!(
                            "{}.__duration_closures.post_run.call",
                            locus_name
                        ),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            self.builder
                .build_unconditional_branch(after_run_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder.position_at_end(after_run_bb);
        }
        // m82: `defer_for_let` joins `is_long_lived` as a reason to
        // route this locus through the deferred-dissolve frame
        // instead of dissolving eagerly here. Both end up on the
        // same flush path at fn-exit (drain → __dissolve_closures
        // → dissolve → arena_destroy), preserving F.4 ordering.
        // The semantic distinction:
        //   - `is_long_lived` (locus has bus subscriptions): MUST
        //     defer so the locus stays alive to receive published
        //     events between birth and scope exit.
        //   - `defer_for_let` (this is a let-binding RHS): chooses
        //     to defer so user code can call methods on the bound
        //     handle after the struct-literal expression returns.
        // Pinned loci already took the `is_pinned` branch above
        // and don't reach this block.
        // m90 (3f fix): when this instantiation will escape via fn
        // return (returns_this_locus from above), suppress the
        // eager dissolve + arena_destroy and DO NOT push onto the
        // deferred_dissolves frame either — the fn-exit flush
        // would otherwise dissolve it on the way out. The locus
        // leaks (heap allocation + uncleaned arena live until
        // process exit); see the alloca branch above for the
        // trade-off note.
        // 3d+3e fix: parent-accepted children behave the same way
        // — the parent's children-array retains the pointer past
        // the spawning fn's stack frame, so dissolve happens at
        // parent's arena_destroy (which frees the child's struct
        // memory wholesale). The child's drain/dissolve method
        // bodies are skipped for v1; a children-cascade at parent
        // dissolve tightens this in v1.x.
        let defer = is_long_lived
            || defer_for_let
            || returns_this_locus
            || parent_accepts_us;
        if !defer {
            // drain → __dissolve_closures → dissolve. Mirrors the
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
            if let Some(closures_fn) = info.dissolve_closures_fn {
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
                        &format!(
                            "{}.__dissolve_closures.call",
                            locus_name
                        ),
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
        } else if returns_this_locus {
            // Intentionally no-op: see m90 note above. The locus
            // outlives this fn's frame by design.
        } else if parent_accepts_us {
            // Intentionally no-op: see 3d+3e note above. Parent's
            // arena_destroy will wholesale-free the child struct
            // when the parent itself dissolves. Drain/dissolve
            // bodies don't fire on the child — v1 trade-off,
            // matches `returns_this_locus`.
        } else if let Some(top) = self.deferred_dissolves.last_mut() {
            top.push((self_ptr, locus_name.to_string(), None));
        } else {
            // Should be unreachable: every fn body / lifecycle
            // body opens a frame in lower_program/method body
            // setup. If we hit this, the locus instantiation is
            // outside any tracked scope and won't get cleaned up.
            return Err(CodegenError::Unsupported(format!(
                "deferred-dissolve locus `{}` instantiated outside any tracked \
                 scope (no deferred-dissolve frame); long-lived={}, let-bound={}",
                locus_name, is_long_lived, defer_for_let,
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
    ) -> Result<Option<(BasicValueEnum<'ctx>, CodegenTy)>, CodegenError> {
        let cs = self.current_self.as_ref().cloned().ok_or_else(|| {
            CodegenError::Unsupported(format!(
                "self.{}(...) outside a locus method",
                method_name
            ))
        })?;

        // m80: if `method_name` is a function-pointer field on
        // self, indirect-call through it. This is how Listener
        // (m82) invokes its on_connection callback per accepted
        // connection. Field-shadows-method semantics: a struct
        // field named the same as a method takes precedence,
        // which is the conventional v0 behavior for any
        // user-supplied callback override.
        if let Some((field_idx, field_ty)) =
            cs.fields.get(method_name).cloned()
        {
            if let CodegenTy::FnPtr {
                args: arg_tys,
                ret: ret_ty,
            } = field_ty
            {
                if args.len() != arg_tys.len() {
                    return Err(CodegenError::Unsupported(format!(
                        "self.{} (fn pointer): expected {} args, got {}",
                        method_name,
                        arg_tys.len(),
                        args.len()
                    )));
                }
                // m80: every user free fn has an implicit
                // `__caller_arena: ptr` first param (m49 calling
                // convention). The fn-pointer points to that same
                // ABI-shaped fn, so the indirect call must
                // prepend the caller's arena. Captured here
                // BEFORE arg lowering — same discipline as
                // lower_user_fn_call.
                let caller_arena = self.current_arena_ptr()?;
                let ptr_t = self.context.ptr_type(AddressSpace::default());
                let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                    Vec::with_capacity(args.len() + 1);
                let mut llvm_param_tys: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> =
                    Vec::with_capacity(args.len() + 1);
                call_args.push(caller_arena.into());
                llvm_param_tys.push(ptr_t.into());
                // Lower each user-visible arg, type-checking
                // against the declared FnPtr signature.
                for (i, a) in args.iter().enumerate() {
                    let (v, vt) = self.lower_expr(a, scope)?;
                    if vt != arg_tys[i] {
                        return Err(CodegenError::Unsupported(format!(
                            "self.{} arg {}: expected {:?}, got {:?}",
                            method_name, i, arg_tys[i], vt
                        )));
                    }
                    call_args.push(v.into());
                    llvm_param_tys.push(self.llvm_basic_type(&arg_tys[i]).into());
                }
                // GEP+load the field pointer.
                let field_ptr = self
                    .builder
                    .build_struct_gep(
                        cs.struct_ty,
                        cs.self_ptr,
                        field_idx,
                        "fnptr.field.ptr",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let ptr_ty = self.context.ptr_type(AddressSpace::default());
                let fn_value_ptr = self
                    .builder
                    .build_load(ptr_ty, field_ptr, "fnptr.value")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_pointer_value();
                // Synthesize the LLVM FunctionType from the
                // FnPtr's args/ret. For void-returning FnPtrs the
                // call returns no usable value; for value-returning
                // we propagate the result + CodegenTy up.
                let fn_ty = match &ret_ty {
                    None => self
                        .context
                        .void_type()
                        .fn_type(&llvm_param_tys, false),
                    Some(rt) => {
                        let lr = self.llvm_basic_type(rt);
                        match lr {
                            inkwell::types::BasicTypeEnum::IntType(t) => {
                                t.fn_type(&llvm_param_tys, false)
                            }
                            inkwell::types::BasicTypeEnum::FloatType(t) => {
                                t.fn_type(&llvm_param_tys, false)
                            }
                            inkwell::types::BasicTypeEnum::PointerType(t) => {
                                t.fn_type(&llvm_param_tys, false)
                            }
                            _ => {
                                return Err(CodegenError::Unsupported(
                                    format!(
                                        "fn-pointer return type {:?} not yet supported",
                                        rt
                                    ),
                                ));
                            }
                        }
                    }
                };
                let call = self
                    .builder
                    .build_indirect_call(
                        fn_ty,
                        fn_value_ptr,
                        &call_args,
                        "fnptr.call",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                return Ok(match ret_ty {
                    None => None,
                    Some(rt) => {
                        let v = call
                            .try_as_basic_value()
                            .left()
                            .expect("non-void fn-pointer call should yield a value");
                        Some((v, *rt))
                    }
                });
            }
        }

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
        // Caller may omit a contiguous tail of defaulted params
        // (suffix-only rule enforced at decl time). Each missing
        // slot's default expression evaluates at the call site
        // — same semantics as free-fn defaults (m32).
        if args.len() > sig.params.len() {
            return Err(CodegenError::Unsupported(format!(
                "self.{}: expected at most {} args, got {}",
                method_name,
                sig.params.len(),
                args.len()
            )));
        }
        for (i, p) in sig.params.iter().enumerate() {
            if i >= args.len() && p.default.is_none() {
                return Err(CodegenError::Unsupported(format!(
                    "self.{}: required param `{}` not provided (only {} \
                     args given)",
                    method_name,
                    p.name.name,
                    args.len()
                )));
            }
        }
        let mut llvm_args: Vec<BasicMetadataValueEnum> =
            Vec::with_capacity(sig.params.len() + 1);
        llvm_args.push(cs.self_ptr.into());
        for i in 0..sig.params.len() {
            let (v, ty) = if i < args.len() {
                self.lower_expr(&args[i], scope)?
            } else {
                let default_expr =
                    sig.params[i].default.as_ref().expect("checked above");
                self.lower_expr(default_expr, scope)?
            };
            let want = self.type_expr_to_codegen_ty(&sig.params[i].ty)?;
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
                let rt = self.type_expr_to_codegen_ty(t)?;
                let v = call
                    .try_as_basic_value()
                    .left()
                    .expect("non-void method returns a basic value");
                Ok(Some((v, rt)))
            }
        }
    }

    /// Lower `receiver.method_name(args)` where `receiver` is any
    /// expression yielding a LocusRef value (typically a
    /// let-binding to a locus literal, a fn parameter of locus
    /// m83: shared indirect-call lowering for fn-pointer values.
    /// Used by both `lower_self_method_call` (when a self-field
    /// of FnPtr type is invoked) and the bare-name call dispatch
    /// (when a local variable of FnPtr type is invoked, e.g.
    /// `on_conn(s)` inside `__handle_one_connection`). Encodes
    /// the m80/m49 calling convention: every user free fn has an
    /// implicit `__caller_arena: ptr` first param, so the indirect
    /// call must prepend the caller's arena before the user-visible
    /// args. Type-checks each arg against the FnPtr's declared
    /// arg_tys; rejects arity mismatches. `ret_ty.is_none()`
    /// signals void-returning — the caller should treat that as
    /// "no value produced".
    fn emit_fnptr_indirect_call(
        &mut self,
        fn_value_ptr: PointerValue<'ctx>,
        arg_tys: &[CodegenTy],
        ret_ty: Option<&CodegenTy>,
        args: &[Expr],
        scope: &Scope<'ctx>,
        callee_label: &str,
    ) -> Result<Option<(BasicValueEnum<'ctx>, CodegenTy)>, CodegenError> {
        if args.len() != arg_tys.len() {
            return Err(CodegenError::Unsupported(format!(
                "{} (fn pointer): expected {} args, got {}",
                callee_label,
                arg_tys.len(),
                args.len()
            )));
        }
        let caller_arena = self.current_arena_ptr()?;
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
            Vec::with_capacity(args.len() + 1);
        let mut llvm_param_tys: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> =
            Vec::with_capacity(args.len() + 1);
        call_args.push(caller_arena.into());
        llvm_param_tys.push(ptr_t.into());
        for (i, a) in args.iter().enumerate() {
            let (v, vt) = self.lower_expr(a, scope)?;
            if vt != arg_tys[i] {
                return Err(CodegenError::Unsupported(format!(
                    "{} arg {}: expected {:?}, got {:?}",
                    callee_label, i, arg_tys[i], vt
                )));
            }
            call_args.push(v.into());
            llvm_param_tys.push(self.llvm_basic_type(&arg_tys[i]).into());
        }
        let fn_ty = match ret_ty {
            None => self
                .context
                .void_type()
                .fn_type(&llvm_param_tys, false),
            Some(rt) => {
                let lr = self.llvm_basic_type(rt);
                match lr {
                    inkwell::types::BasicTypeEnum::IntType(t) => {
                        t.fn_type(&llvm_param_tys, false)
                    }
                    inkwell::types::BasicTypeEnum::FloatType(t) => {
                        t.fn_type(&llvm_param_tys, false)
                    }
                    inkwell::types::BasicTypeEnum::PointerType(t) => {
                        t.fn_type(&llvm_param_tys, false)
                    }
                    _ => {
                        return Err(CodegenError::Unsupported(format!(
                            "fn-pointer return type {:?} not yet supported",
                            rt
                        )));
                    }
                }
            }
        };
        let call = self
            .builder
            .build_indirect_call(
                fn_ty,
                fn_value_ptr,
                &call_args,
                "fnptr.call",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(match ret_ty {
            None => None,
            Some(rt) => {
                let v = call
                    .try_as_basic_value()
                    .left()
                    .expect("non-void fn-pointer call should yield a value");
                Some((v, rt.clone()))
            }
        })
    }

    /// type, or a child accepted via the F.7 hook). The shape
    /// mirrors lower_self_method_call but the self_ptr comes
    /// from the lowered receiver rather than current_self —
    /// allowing user code to call methods on locus values
    /// outside of self-context.
    ///
    /// m81: required for the std::io::tcp::Stream pattern where
    /// user code receives a Stream via callback and invokes
    /// `s.send(msg)` / `s.recv(n)` on it. Also generally useful
    /// for any "locus as service handle" idiom.
    fn lower_external_method_call(
        &mut self,
        receiver_expr: &Expr,
        method_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<Option<(BasicValueEnum<'ctx>, CodegenTy)>, CodegenError> {
        // F.22: `self.<slot>.<method>(args)` routes directly to the
        // C allocator without going through normal locus-method
        // dispatch. The receiver here is `self.<slot>` — a Field
        // expression whose base is KwSelf and whose name matches a
        // declared capacity slot on the current self's locus. We
        // catch this before lowering the receiver because slots
        // don't have a value-level CodegenTy that survives outside
        // a method-call position; lowering `self.<slot>` as a
        // standalone expression would error.
        //
        // The helper's outer Option distinguishes "not a slot,
        // fall through" (None) from "handled" (Some(inner)) where
        // inner is the method's value-or-void return.
        if let Some(slot_result) = self.try_lower_capacity_slot_method_call(
            receiver_expr,
            method_name,
            args,
            scope,
        )? {
            return Ok(slot_result);
        }
        let (recv_val, recv_ty) = self.lower_expr(receiver_expr, scope)?;
        // F.20 Phase B: dispatch through an interface fat pointer.
        // The receiver value is a pointer to a `{data, vtable}`
        // struct laid out by `coerce_to_interface`. Load data
        // (the underlying locus pointer) for the implicit self
        // arg, load vtable, GEP to the method-index slot, load
        // the fn pointer, and indirect-call. Method index is the
        // method's position in the interface's declaration list.
        if let CodegenTy::Interface(iface_name) = &recv_ty {
            return self.lower_iface_method_call(
                recv_val.into_pointer_value(),
                iface_name,
                method_name,
                args,
                scope,
            );
        }
        // v1.x-8: `record.field(args)` where `field` is a
        // fn-pointer field on a user struct lowers as a struct
        // GEP-load of the fn pointer followed by an indirect
        // call. Mirrors lower_self_method_call's m83 path for
        // self-fields of FnPtr type. Closes the friction-log
        // entry `type-records-cannot-hold-fn-pointer-fields`.
        if let CodegenTy::TypeRef(type_name) = &recv_ty {
            if let Some(info) = self.user_types.get(type_name).cloned() {
                if let Some((idx, field_ty)) =
                    info.fields.get(method_name).cloned()
                {
                    if let CodegenTy::FnPtr {
                        args: fn_args,
                        ret: fn_ret,
                    } = field_ty.clone()
                    {
                        let recv_ptr = recv_val.into_pointer_value();
                        let field_ptr = self
                            .builder
                            .build_struct_gep(
                                info.struct_ty,
                                recv_ptr,
                                idx,
                                &format!(
                                    "{}.{}.ptr",
                                    type_name, method_name
                                ),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        let ptr_t = self
                            .context
                            .ptr_type(AddressSpace::default());
                        let fn_value_ptr = self
                            .builder
                            .build_load(
                                ptr_t,
                                field_ptr,
                                &format!("{}.{}", type_name, method_name),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?
                            .into_pointer_value();
                        return self.emit_fnptr_indirect_call(
                            fn_value_ptr,
                            &fn_args,
                            fn_ret.as_deref(),
                            args,
                            scope,
                            &format!("{}.{}", type_name, method_name),
                        );
                    }
                }
            }
        }
        let locus_name = match recv_ty {
            CodegenTy::LocusRef(n) => n,
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "method call on non-locus value of type {:?}",
                    other
                )));
            }
        };
        let info = self
            .user_loci
            .get(&locus_name)
            .cloned()
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "method call: unknown locus `{}`",
                    locus_name
                ))
            })?;
        let func = info
            .user_methods
            .get(method_name)
            .copied()
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "locus `{}` has no method `{}`",
                    locus_name, method_name
                ))
            })?;
        // Look up the method's source-level signature.
        struct MethodSig {
            params: Vec<Param>,
            ret: Option<TypeExpr>,
        }
        let sig: MethodSig = self
            .program
            .items
            .iter()
            .find_map(|item| match item {
                TopDecl::Locus(l) if l.name.name == locus_name => l
                    .members
                    .iter()
                    .find_map(|m| match m {
                        LocusMember::Fn(fd) if fd.name.name == method_name => {
                            Some(MethodSig {
                                params: fd.params.clone(),
                                ret: fd.ret.clone(),
                            })
                        }
                        _ => None,
                    }),
                _ => None,
            })
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "method `{}` declaration not found on locus `{}`",
                    method_name, locus_name
                ))
            })?;
        if args.len() > sig.params.len() {
            return Err(CodegenError::Unsupported(format!(
                "{}.{}: expected at most {} args, got {}",
                locus_name,
                method_name,
                sig.params.len(),
                args.len()
            )));
        }
        let mut llvm_args: Vec<BasicMetadataValueEnum<'ctx>> =
            Vec::with_capacity(sig.params.len() + 1);
        llvm_args.push(recv_val.into_pointer_value().into());
        for i in 0..sig.params.len() {
            let (v, ty) = if i < args.len() {
                self.lower_expr(&args[i], scope)?
            } else {
                let default_expr = sig.params[i]
                    .default
                    .as_ref()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "{}.{}: required param `{}` not provided",
                            locus_name, method_name, sig.params[i].name.name
                        ))
                    })?;
                self.lower_expr(default_expr, scope)?
            };
            let want = self.type_expr_to_codegen_ty(&sig.params[i].ty)?;
            if ty != want {
                return Err(CodegenError::Unsupported(format!(
                    "{}.{} arg {} type mismatch: expected {:?}, got {:?}",
                    locus_name, method_name, i, want, ty
                )));
            }
            llvm_args.push(v.into());
        }
        let call = self
            .builder
            .build_call(
                func,
                &llvm_args,
                &format!("{}.{}.call", locus_name, method_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        match &sig.ret {
            None => Ok(None),
            Some(t) => {
                let rt = self.type_expr_to_codegen_ty(t)?;
                let v = call
                    .try_as_basic_value()
                    .left()
                    .expect("non-void method returns a basic value");
                Ok(Some((v, rt)))
            }
        }
    }

    /// F.22 slot dispatch. Checks if `receiver_expr` is `self.<X>`
    /// where X is a declared capacity slot on the current self's
    /// locus, and if so routes the method call directly to the
    /// matching `lotus_pool_*` / `lotus_heap_*` C primitive.
    ///
    /// Returns `Ok(None)` when the receiver isn't a slot — caller
    /// falls through to ordinary external-method dispatch.
    /// Returns `Ok(Some(...))` when dispatch succeeded.
    /// Returns `Err` for diagnosable mismatches (wrong method
    /// for slot kind, wrong arg count, wrong cell type).
    ///
    /// Surface (per spec §F.22):
    ///   pool: acquire() -> Cell(T); release(c: Cell(T)) -> void
    ///   heap: alloc()   -> Cell(T); free(c:    Cell(T)) -> void
    fn try_lower_capacity_slot_method_call(
        &mut self,
        receiver_expr: &Expr,
        method_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<
        // Outer None = "not a slot, caller falls through to
        // ordinary external-method dispatch". Outer Some(inner)
        // = "handled"; inner mirrors the regular method-call
        // result (Some(value, ty) for non-void, None for void).
        Option<Option<(BasicValueEnum<'ctx>, CodegenTy)>>,
        CodegenError,
    > {
        let slot_name = match receiver_expr {
            Expr::Field { receiver, name, .. }
                if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
            {
                name.name.clone()
            }
            _ => return Ok(None),
        };
        let Some(cs) = self.current_self.as_ref().cloned() else {
            return Ok(None);
        };
        let Some(info) = self.user_loci.get(&cs.locus_name).cloned()
        else {
            return Ok(None);
        };
        let Some(slot) = info
            .capacity_slots
            .iter()
            .find(|s| s.name == slot_name)
            .cloned()
        else {
            return Ok(None);
        };
        // Slot exists; from here, any mismatch is a hard error
        // (returning Ok(None) would fall through to a less-clear
        // diagnostic about "method on non-locus" since slot fields
        // don't have a LocusRef type).
        enum Op {
            Acquire,
            Release,
            Alloc,
            Free,
        }
        let op = match (slot.kind, method_name) {
            (CapacitySlotKind::Pool, "acquire") => Op::Acquire,
            (CapacitySlotKind::Pool, "release") => Op::Release,
            (CapacitySlotKind::Heap, "alloc") => Op::Alloc,
            (CapacitySlotKind::Heap, "free") => Op::Free,
            (CapacitySlotKind::Pool, other) => {
                return Err(CodegenError::Unsupported(format!(
                    "pool slot `{}`: method `{}` not available — use \
                     `acquire()` to borrow a cell or `release(cell)` \
                     to return one",
                    slot_name, other
                )));
            }
            (CapacitySlotKind::Heap, other) => {
                return Err(CodegenError::Unsupported(format!(
                    "heap slot `{}`: method `{}` not available — use \
                     `alloc()` to allocate a cell or `free(cell)` to \
                     release one",
                    slot_name, other
                )));
            }
        };
        // Load the allocator pointer from the slot field.
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let slot_field_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                cs.self_ptr,
                slot.struct_field_idx,
                &format!("self.__slot_{}.ptr", slot_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let allocator_ptr = self
            .builder
            .build_load(
                ptr_t,
                slot_field_ptr,
                &format!("self.__slot_{}", slot_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        match op {
            Op::Acquire | Op::Alloc => {
                if !args.is_empty() {
                    return Err(CodegenError::Unsupported(format!(
                        "slot `{}`.{}: takes no args, got {}",
                        slot_name,
                        method_name,
                        args.len()
                    )));
                }
                let fn_name = match op {
                    Op::Acquire => "lotus_pool_acquire",
                    Op::Alloc => "lotus_heap_alloc",
                    _ => unreachable!(),
                };
                let fn_value = self
                    .module
                    .get_function(fn_name)
                    .expect("F.22 allocator extern declared");
                let cell_ptr = self
                    .builder
                    .build_call(
                        fn_value,
                        &[allocator_ptr.into()],
                        &format!("{}.{}.call", slot_name, method_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .try_as_basic_value()
                    .left()
                    .expect("allocator acquire/alloc returns ptr");
                let origin = Some((cs.locus_name.clone(), slot_name.clone()));
                Ok(Some(Some((
                    cell_ptr,
                    CodegenTy::Cell(Box::new(slot.elem_ty.clone()), origin),
                ))))
            }
            Op::Release | Op::Free => {
                if args.len() != 1 {
                    return Err(CodegenError::Unsupported(format!(
                        "slot `{}`.{}: takes exactly 1 cell arg, got {}",
                        slot_name,
                        method_name,
                        args.len()
                    )));
                }
                let (cell_val, cell_ty) =
                    self.lower_expr(&args[0], scope)?;
                // v1.x-5: enforce slot-of-origin. The cell's type
                // carries (origin_locus, origin_slot); we reject
                // any cell whose origin doesn't match the slot
                // we're releasing into. Catches the v1 UB shape
                // where a Cell<Int> from slot `a` was silently
                // releasable into slot `b`.
                let expected_elem = slot.elem_ty.clone();
                let expected_origin =
                    (cs.locus_name.clone(), slot_name.clone());
                match &cell_ty {
                    CodegenTy::Cell(actual_elem, actual_origin)
                        if **actual_elem == expected_elem =>
                    {
                        match actual_origin {
                            Some(o) if o == &expected_origin => {}
                            Some((origin_locus, origin_slot)) => {
                                return Err(CodegenError::Unsupported(format!(
                                    "slot `{}`.{}: cell originated from \
                                     `{}.{}` — cells can only be released \
                                     into the slot they came from (v1.x-5 \
                                     slot-of-origin tracking)",
                                    slot_name,
                                    method_name,
                                    origin_locus,
                                    origin_slot
                                )));
                            }
                            None => {
                                return Err(CodegenError::Unsupported(format!(
                                    "slot `{}`.{}: cell has no slot origin \
                                     — v1 cells must be acquired from a \
                                     specific slot",
                                    slot_name, method_name
                                )));
                            }
                        }
                    }
                    other => {
                        return Err(CodegenError::Unsupported(format!(
                            "slot `{}`.{}: expected Cell<{:?}>, got {:?}",
                            slot_name, method_name, expected_elem, other
                        )));
                    }
                }
                let fn_name = match op {
                    Op::Release => "lotus_pool_release",
                    Op::Free => "lotus_heap_free",
                    _ => unreachable!(),
                };
                let fn_value = self
                    .module
                    .get_function(fn_name)
                    .expect("F.22 allocator extern declared");
                self.builder
                    .build_call(
                        fn_value,
                        &[allocator_ptr.into(), cell_val.into()],
                        &format!("{}.{}.call", slot_name, method_name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                // release/free return void at the C ABI. Outer
                // Some marks "we handled this"; inner None
                // signals the value-less return. The expr-position
                // caller in `lower_expr` errors on an inner None,
                // which matches the user's expectation that
                // `let x = self.X.release(c);` is meaningless.
                Ok(Some(None))
            }
        }
    }

    /// F.20 Phase B: lower a method call where the receiver is an
    /// interface value. The receiver is the fat-pointer struct
    /// produced by `coerce_to_interface`; data slot holds the
    /// underlying locus pointer, vtable slot holds the
    /// `__vt.<locus>.<iface>` global address. Method index in the
    /// vtable is the method's position in the interface's
    /// declaration list; the LLVM FunctionType for the indirect
    /// call is synthesized from the interface method signature
    /// (with `self: ptr` prepended to match the locus-method ABI).
    fn lower_iface_method_call(
        &mut self,
        fat_ptr: PointerValue<'ctx>,
        iface_name: &str,
        method_name: &str,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<Option<(BasicValueEnum<'ctx>, CodegenTy)>, CodegenError> {
        let iface_decl = self
            .program
            .items
            .iter()
            .find_map(|item| match item {
                TopDecl::Interface(i) if i.name.name == iface_name => {
                    Some(i.clone())
                }
                _ => None,
            })
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "interface `{}` not declared",
                    iface_name
                ))
            })?;
        let (method_idx, method_sig) = iface_decl
            .methods
            .iter()
            .enumerate()
            .find(|(_, m)| m.name.name == method_name)
            .map(|(i, m)| (i, m.clone()))
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "interface `{}` has no method `{}`",
                    iface_name, method_name
                ))
            })?;
        if args.len() != method_sig.params.len() {
            return Err(CodegenError::Unsupported(format!(
                "{}.{} (interface): expected {} arg(s), got {}",
                iface_name,
                method_name,
                method_sig.params.len(),
                args.len()
            )));
        }
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let fat_struct_ty = self.iface_fat_struct_ty();
        // Load data ptr (slot 0) and vtable ptr (slot 1).
        let data_slot_ptr = self
            .builder
            .build_struct_gep(fat_struct_ty, fat_ptr, 0, "iface.data.gep")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let data_ptr = self
            .builder
            .build_load(ptr_t, data_slot_ptr, "iface.data")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        let vtable_slot_ptr = self
            .builder
            .build_struct_gep(fat_struct_ty, fat_ptr, 1, "iface.vtable.gep")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let vtable_ptr = self
            .builder
            .build_load(ptr_t, vtable_slot_ptr, "iface.vtable")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        // Index into the vtable: `[N x ptr]`. We treat the vtable
        // pointer as a flat `[max x ptr]` and GEP with the method
        // index — LLVM is type-erased on the runtime side, so the
        // exact N in the array_type doesn't matter for the GEP.
        let vtable_ty = ptr_t.array_type(iface_decl.methods.len() as u32);
        let i32_t = self.context.i32_type();
        let zero = i32_t.const_zero();
        let idx = i32_t.const_int(method_idx as u64, false);
        let fn_slot_ptr = unsafe {
            self.builder
                .build_in_bounds_gep(
                    vtable_ty,
                    vtable_ptr,
                    &[zero, idx],
                    &format!("iface.{}.{}.slot", iface_name, method_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        };
        let fn_ptr = self
            .builder
            .build_load(ptr_t, fn_slot_ptr, "iface.fn")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        // Lower args; check each against the interface-declared
        // type. The typechecker already enforces structural impl
        // against the interface signature, so the locus's matching
        // method has compatible types.
        let mut llvm_args: Vec<BasicMetadataValueEnum<'ctx>> =
            Vec::with_capacity(args.len() + 1);
        let mut llvm_param_tys: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> =
            Vec::with_capacity(args.len() + 1);
        llvm_args.push(data_ptr.into());
        llvm_param_tys.push(ptr_t.into());
        for (i, a) in args.iter().enumerate() {
            let (v, ty) = self.lower_expr(a, scope)?;
            let want = self.type_expr_to_codegen_ty(&method_sig.params[i].ty)?;
            // Same Locus→Interface coercion the free-fn call site
            // does — keeps interface-typed method params usable.
            let (v, ty) = if let (
                CodegenTy::Interface(iname),
                CodegenTy::LocusRef(l),
            ) = (&want, &ty)
            {
                let fat = self.coerce_to_interface(
                    v.into_pointer_value(),
                    l,
                    iname,
                )?;
                (fat.into(), want.clone())
            } else {
                (v, ty)
            };
            if ty != want {
                return Err(CodegenError::Unsupported(format!(
                    "{}.{} (interface) arg {} type mismatch: expected {:?}, got {:?}",
                    iface_name, method_name, i, want, ty
                )));
            }
            llvm_args.push(v.into());
            llvm_param_tys.push(self.llvm_basic_type(&want).into());
        }
        let ret_codegen_ty = match &method_sig.ret {
            Some(t) => Some(self.type_expr_to_codegen_ty(t)?),
            None => None,
        };
        let fn_ty = match &ret_codegen_ty {
            None => self.context.void_type().fn_type(&llvm_param_tys, false),
            Some(rt) => match self.llvm_basic_type(rt) {
                inkwell::types::BasicTypeEnum::IntType(t) => {
                    t.fn_type(&llvm_param_tys, false)
                }
                inkwell::types::BasicTypeEnum::FloatType(t) => {
                    t.fn_type(&llvm_param_tys, false)
                }
                inkwell::types::BasicTypeEnum::PointerType(t) => {
                    t.fn_type(&llvm_param_tys, false)
                }
                other => {
                    return Err(CodegenError::Unsupported(format!(
                        "interface method return type {:?} not yet supported \
                         by Phase B dispatch",
                        other
                    )));
                }
            },
        };
        let call = self
            .builder
            .build_indirect_call(
                fn_ty,
                fn_ptr,
                &llvm_args,
                &format!("iface.{}.{}.call", iface_name, method_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(match ret_codegen_ty {
            None => None,
            Some(rt) => {
                let v = call
                    .try_as_basic_value()
                    .left()
                    .expect("non-void interface dispatch yields a value");
                Some((v, rt))
            }
        })
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
    /// m46: zero every closure's accumulator slots that don't
    /// list `event` in their `persists_through(...)` clause.
    /// Default behavior is reset; `persists_through` is opt-out
    /// per spec/runtime.md "recovery-event interaction." Called
    /// from `restart` / `restart_in_place` / `quarantine` recovery
    /// dispatch (m40 / m45 / m41 sites).
    fn emit_accumulator_reset_for_event(
        &mut self,
        info: &LocusInfo<'ctx>,
        self_ptr: PointerValue<'ctx>,
        event: &str,
        locus_name: &str,
    ) -> Result<(), CodegenError> {
        let groups: Vec<(String, Vec<AccumulatorSlot>)> = info
            .accumulators_per_closure
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (closure_name, slots) in groups {
            let persists = info
                .persists_through_per_closure
                .get(&closure_name)
                .map(|v| v.iter().any(|e| e == event))
                .unwrap_or(false);
            if persists {
                continue;
            }
            for (i, slot) in slots.iter().enumerate() {
                self.zero_accumulator_slot(
                    info.struct_ty,
                    self_ptr,
                    slot,
                    &format!(
                        "{}.{}.acc[{}].reset.{}",
                        locus_name, closure_name, i, event
                    ),
                )?;
            }
        }
        Ok(())
    }

    /// m46 / m46-vocab: store zero values into an accumulator
    /// slot's struct fields. Sum has one field of inner's type;
    /// Count has one i64; Mean has both — sum slot of inner's
    /// type plus count i64. Used at instantiation (initial zero)
    /// and at recovery dispatch when the event isn't listed in
    /// `persists_through`.
    fn zero_accumulator_slot(
        &mut self,
        struct_ty: inkwell::types::StructType<'ctx>,
        self_ptr: PointerValue<'ctx>,
        slot: &AccumulatorSlot,
        name: &str,
    ) -> Result<(), CodegenError> {
        match slot.kind {
            AccumulatorKind::Sum => {
                self.zero_one_field(
                    struct_ty,
                    self_ptr,
                    slot.field_idx,
                    &slot.inner_ty,
                    name,
                )?;
            }
            AccumulatorKind::Count => {
                self.zero_one_field(
                    struct_ty,
                    self_ptr,
                    slot.field_idx,
                    &CodegenTy::Int,
                    name,
                )?;
            }
            AccumulatorKind::Mean => {
                self.zero_one_field(
                    struct_ty,
                    self_ptr,
                    slot.field_idx,
                    &slot.inner_ty,
                    &format!("{}.sum", name),
                )?;
                let count_idx = slot
                    .field_idx_2
                    .expect("mean slot has count field");
                self.zero_one_field(
                    struct_ty,
                    self_ptr,
                    count_idx,
                    &CodegenTy::Int,
                    &format!("{}.count", name),
                )?;
            }
        }
        Ok(())
    }

    fn zero_one_field(
        &mut self,
        struct_ty: inkwell::types::StructType<'ctx>,
        self_ptr: PointerValue<'ctx>,
        field_idx: u32,
        ty: &CodegenTy,
        name: &str,
    ) -> Result<(), CodegenError> {
        let slot_ptr = self
            .builder
            .build_struct_gep(struct_ty, self_ptr, field_idx, name)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        match ty {
            CodegenTy::Int | CodegenTy::Duration => {
                let z = self.context.i64_type().const_int(0, false);
                self.builder
                    .build_store(slot_ptr, z)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            CodegenTy::Float => {
                let z = self.context.f64_type().const_float(0.0);
                self.builder
                    .build_store(slot_ptr, z)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            CodegenTy::Decimal => {
                let z = i128_const(self.context, 0);
                self.builder
                    .build_store(slot_ptr, z)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            _ => {
                return Err(CodegenError::Unsupported(format!(
                    "accumulator slot field type {:?} unexpected", ty
                )));
            }
        }
        Ok(())
    }

    /// m46 / m46-vocab: sample-update an accumulator slot. Sum
    /// evaluates the inner expr and adds it to the running-sum
    /// field. Count just bumps an i64 by 1 (no inner expr). Mean
    /// does both — sum += inner, count += 1. Called for every
    /// accumulator slot of a closure right BEFORE the closure's
    /// assertion is lowered — so the assertion's substitutions
    /// read the post-update value (natural "sum/count/mean across
    /// cells through this moment" semantics).
    fn update_accumulator_slot(
        &mut self,
        struct_ty: inkwell::types::StructType<'ctx>,
        self_ptr: PointerValue<'ctx>,
        slot: &AccumulatorSlot,
        scope: &Scope<'ctx>,
        name: &str,
    ) -> Result<(), CodegenError> {
        match slot.kind {
            AccumulatorKind::Sum => {
                let inner = slot
                    .inner_expr
                    .as_ref()
                    .expect("sum slot carries inner");
                let (sample_val, sample_ty) = self.lower_expr(inner, scope)?;
                if sample_ty != slot.inner_ty {
                    return Err(CodegenError::Unsupported(format!(
                        "accumulator inner expr re-lowered with type {:?} but \
                         slot was declared {:?}",
                        sample_ty, slot.inner_ty
                    )));
                }
                self.add_to_field(
                    struct_ty,
                    self_ptr,
                    slot.field_idx,
                    &slot.inner_ty,
                    sample_val,
                    name,
                )?;
            }
            AccumulatorKind::Count => {
                let one = self.context.i64_type().const_int(1, true);
                self.add_to_field(
                    struct_ty,
                    self_ptr,
                    slot.field_idx,
                    &CodegenTy::Int,
                    one.into(),
                    name,
                )?;
            }
            AccumulatorKind::Mean => {
                let inner = slot
                    .inner_expr
                    .as_ref()
                    .expect("mean slot carries inner");
                let (sample_val, sample_ty) = self.lower_expr(inner, scope)?;
                if sample_ty != slot.inner_ty {
                    return Err(CodegenError::Unsupported(format!(
                        "accumulator inner expr re-lowered with type {:?} but \
                         mean slot was declared {:?}",
                        sample_ty, slot.inner_ty
                    )));
                }
                self.add_to_field(
                    struct_ty,
                    self_ptr,
                    slot.field_idx,
                    &slot.inner_ty,
                    sample_val,
                    &format!("{}.sum", name),
                )?;
                let count_idx = slot
                    .field_idx_2
                    .expect("mean slot has count field");
                let one = self.context.i64_type().const_int(1, true);
                self.add_to_field(
                    struct_ty,
                    self_ptr,
                    count_idx,
                    &CodegenTy::Int,
                    one.into(),
                    &format!("{}.count", name),
                )?;
            }
        }
        Ok(())
    }

    fn add_to_field(
        &mut self,
        struct_ty: inkwell::types::StructType<'ctx>,
        self_ptr: PointerValue<'ctx>,
        field_idx: u32,
        ty: &CodegenTy,
        sample_val: BasicValueEnum<'ctx>,
        name: &str,
    ) -> Result<(), CodegenError> {
        let slot_ptr = self
            .builder
            .build_struct_gep(struct_ty, self_ptr, field_idx, name)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        match ty {
            CodegenTy::Int | CodegenTy::Duration => {
                let i64_t = self.context.i64_type();
                let prev = self
                    .builder
                    .build_load(i64_t, slot_ptr, &format!("{}.prev", name))
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                let next = self
                    .builder
                    .build_int_add(
                        prev,
                        sample_val.into_int_value(),
                        &format!("{}.next", name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(slot_ptr, next)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            CodegenTy::Float => {
                let f64_t = self.context.f64_type();
                let prev = self
                    .builder
                    .build_load(f64_t, slot_ptr, &format!("{}.prev", name))
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_float_value();
                let next = self
                    .builder
                    .build_float_add(
                        prev,
                        sample_val.into_float_value(),
                        &format!("{}.next", name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(slot_ptr, next)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            CodegenTy::Decimal => {
                let i128_t = self.context.i128_type();
                let prev = self
                    .builder
                    .build_load(i128_t, slot_ptr, &format!("{}.prev", name))
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                let next = self
                    .builder
                    .build_int_add(
                        prev,
                        sample_val.into_int_value(),
                        &format!("{}.next", name),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(slot_ptr, next)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            _ => {
                return Err(CodegenError::Unsupported(format!(
                    "accumulator field type {:?} unexpected", ty
                )));
            }
        }
        Ok(())
    }

    /// m46 / m46-vocab: emit a load from the *next* accumulator
    /// slot in the current `accumulator_ctx`. Called from
    /// `lower_expr`'s Call/Sum match when an accumulator builtin
    /// is encountered inside a closure assertion. Advances
    /// `next_idx` by one. Returns `(value, slot.ty)` so the
    /// assertion's left/right typing rule applies cleanly.
    ///
    /// Sum: load the inner-typed slot.
    /// Count: load the i64 slot, return as Int.
    /// Mean: load sum + count, divide, cast to f64, return as Float.
    fn lower_accumulator_load(
        &mut self,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let ctx = self.accumulator_ctx.as_mut().ok_or_else(|| {
            CodegenError::Unsupported(
                "internal: lower_accumulator_load called without ctx".into(),
            )
        })?;
        if ctx.next_idx >= ctx.slots.len() {
            return Err(CodegenError::Unsupported(format!(
                "more accumulator calls encountered during assertion lowering \
                 than slots allocated ({}); detection and lowering walks \
                 disagree",
                ctx.slots.len()
            )));
        }
        let slot = ctx.slots[ctx.next_idx].clone();
        let struct_ty = ctx.struct_ty;
        let self_ptr = ctx.self_ptr;
        ctx.next_idx += 1;
        match slot.kind {
            AccumulatorKind::Sum => {
                let v = self.load_field_typed(
                    struct_ty,
                    self_ptr,
                    slot.field_idx,
                    &slot.inner_ty,
                    &format!("acc[{}].sum.load", slot.field_idx),
                )?;
                Ok((v, slot.ty))
            }
            AccumulatorKind::Count => {
                let v = self.load_field_typed(
                    struct_ty,
                    self_ptr,
                    slot.field_idx,
                    &CodegenTy::Int,
                    &format!("acc[{}].count.load", slot.field_idx),
                )?;
                Ok((v, CodegenTy::Int))
            }
            AccumulatorKind::Mean => {
                let count_idx = slot
                    .field_idx_2
                    .expect("mean slot has count field");
                // Load sum (inner-typed) and count (i64).
                let sum_v = self.load_field_typed(
                    struct_ty,
                    self_ptr,
                    slot.field_idx,
                    &slot.inner_ty,
                    &format!("acc[{}].mean.sum.load", slot.field_idx),
                )?;
                let count_v = self
                    .builder
                    .build_load(
                        self.context.i64_type(),
                        self.builder
                            .build_struct_gep(
                                struct_ty,
                                self_ptr,
                                count_idx,
                                &format!("acc[{}].mean.count.ptr", count_idx),
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?,
                        &format!("acc[{}].mean.count.load", count_idx),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                    .into_int_value();
                // Cast both to f64, divide.
                let f64_t = self.context.f64_type();
                let sum_f = match slot.inner_ty {
                    CodegenTy::Int | CodegenTy::Duration => self
                        .builder
                        .build_signed_int_to_float(
                            sum_v.into_int_value(),
                            f64_t,
                            "mean.sum.f",
                        )
                        .map_err(|e| {
                            CodegenError::LlvmEmit(e.to_string())
                        })?,
                    CodegenTy::Float => sum_v.into_float_value(),
                    CodegenTy::Decimal => {
                        // Decimal sum is i128 with implicit scale 9.
                        // Cast to f64 then divide by 10^9 so the
                        // result is in real units before the
                        // count division. Loses some precision but
                        // mean is inherently real-valued so f64
                        // is the natural output type anyway.
                        let raw = self
                            .builder
                            .build_signed_int_to_float(
                                sum_v.into_int_value(),
                                f64_t,
                                "mean.sum.dec.raw",
                            )
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?;
                        let scale = f64_t.const_float(1_000_000_000.0);
                        self.builder
                            .build_float_div(raw, scale, "mean.sum.dec.f")
                            .map_err(|e| {
                                CodegenError::LlvmEmit(e.to_string())
                            })?
                    }
                    ref other => {
                        return Err(CodegenError::Unsupported(format!(
                            "mean accumulator inner type {:?} unexpected",
                            other
                        )));
                    }
                };
                let count_f = self
                    .builder
                    .build_signed_int_to_float(
                        count_v,
                        f64_t,
                        "mean.count.f",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let mean = self
                    .builder
                    .build_float_div(sum_f, count_f, "mean.div")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Ok((mean.into(), CodegenTy::Float))
            }
        }
    }

    fn load_field_typed(
        &mut self,
        struct_ty: inkwell::types::StructType<'ctx>,
        self_ptr: PointerValue<'ctx>,
        field_idx: u32,
        ty: &CodegenTy,
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let slot_ptr = self
            .builder
            .build_struct_gep(
                struct_ty,
                self_ptr,
                field_idx,
                &format!("{}.ptr", name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let v: BasicValueEnum<'ctx> = match ty {
            CodegenTy::Int | CodegenTy::Duration => self
                .builder
                .build_load(self.context.i64_type(), slot_ptr, name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?,
            CodegenTy::Float => self
                .builder
                .build_load(self.context.f64_type(), slot_ptr, name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?,
            CodegenTy::Decimal => self
                .builder
                .build_load(self.context.i128_type(), slot_ptr, name)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?,
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "load_field_typed: unsupported type {:?}", other
                )));
            }
        };
        Ok(v)
    }

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
        epoch: EpochSpec,
    ) -> Result<(), CodegenError> {
        let scope = Scope::default();

        // m46: install the accumulator-substitution context for
        // this closure (if it has any sum() slots), then sample-
        // update each slot before evaluating the assertion. The
        // assertion's `sum(...)` references will then load the
        // post-update value, so each fire's "running total" is
        // the natural reading of "sum across cells through this
        // moment."
        let info = self
            .user_loci
            .get(locus_name)
            .cloned()
            .expect("closure check on unknown locus");
        let cs = self
            .current_self
            .clone()
            .expect("lower_closure_check called outside a locus body");
        let slots = info
            .accumulators_per_closure
            .get(closure_name)
            .cloned()
            .unwrap_or_default();
        if !slots.is_empty() {
            for (i, slot) in slots.iter().enumerate() {
                self.update_accumulator_slot(
                    info.struct_ty,
                    cs.self_ptr,
                    slot,
                    &scope,
                    &format!(
                        "{}.{}.acc[{}].sample",
                        locus_name, closure_name, i
                    ),
                )?;
            }
            self.accumulator_ctx = Some(AccumulatorCtx {
                slots: slots.clone(),
                next_idx: 0,
                self_ptr: cs.self_ptr,
                struct_ty: info.struct_ty,
            });
        }

        let (lv, lt) = self.lower_expr(&ass.left, &scope)?;
        let (rv, rt) = self.lower_expr(&ass.right, &scope)?;
        if lt != rt {
            self.accumulator_ctx = None;
            return Err(CodegenError::Unsupported(format!(
                "closure `{}` on `{}`: left/right types differ ({:?} vs {:?})",
                closure_name, locus_name, lt, rt
            )));
        }
        let (tv, tt) = self.lower_expr(&ass.tolerance, &scope)?;
        if tt != lt {
            self.accumulator_ctx = None;
            return Err(CodegenError::Unsupported(format!(
                "closure `{}` on `{}`: tolerance type differs ({:?} vs operand {:?})",
                closure_name, locus_name, tt, lt
            )));
        }
        // Substitution complete; clear ctx so any later expression
        // lowering on this thread doesn't accidentally hit it.
        self.accumulator_ctx = None;

        // Track the signed-i64 diff for Int/Duration closures so
        // we can populate ClosureViolation.diff at routing time.
        // For Float/Decimal closures, diff is f64 and we store 0
        // in the violation (the interpreter exposes a polymorphic
        // diff there, which v0 codegen's static struct can't
        // express).
        let mut int_diff: Option<inkwell::values::IntValue<'ctx>> = None;

        let pass = match &lt {
            CodegenTy::Int | CodegenTy::Duration | CodegenTy::Decimal => {
                let l = lv.into_int_value();
                let r = rv.into_int_value();
                let t = tv.into_int_value();
                let diff = self
                    .builder
                    .build_int_sub(l, r, "diff")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                int_diff = Some(diff);
                let zero: inkwell::values::IntValue<'ctx> =
                    if matches!(lt, CodegenTy::Decimal) {
                        i128_const(self.context, 0)
                    } else {
                        self.context.i64_type().const_int(0, false)
                    };
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
            CodegenTy::Float => {
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
        // m48: Decimal closures produce an i128 diff; the violation's
        // diff field is i64 (carries the natural domain's diff for
        // Int / Duration). Truncate i128 → i64 — diff is diagnostic
        // only, never recomputed against the original mantissa, so
        // precision loss past 2^63 ns / mantissa-units is acceptable
        // for v0.1.
        let diff_val = if diff_val.get_type().get_bit_width() != 64 {
            self.builder
                .build_int_truncate(diff_val, i64_t, "diff.trunc")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
        } else {
            diff_val
        };
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
        let cs_struct_ty = self
            .current_self
            .as_ref()
            .expect("__closures runs with current_self set")
            .struct_ty;
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let void_t = self.context.void_type();
        let handler_callee_ty = void_t.fn_type(
            &[ptr_t.into(), ptr_t.into(), ptr_t.into()],
            false,
        );

        // m40: birth-epoch closures snapshot the pre-call value of
        // __restart_count so we can detect whether the parent's
        // on_failure body called restart(self). If it did and the
        // count is within the cap, we re-run birth() + the entire
        // __birth_closures fn before returning (a recursive call
        // into the synthesized eval fn). Dissolve-epoch closures
        // skip this — restart isn't applicable at end-of-life.
        let info = self
            .user_loci
            .get(locus_name)
            .cloned()
            .expect("locus declared in pass A1");
        let i64_t = self.context.i64_type();
        let pre_count: Option<inkwell::values::IntValue<'ctx>> =
            if matches!(epoch, EpochSpec::Birth)
                && info.birth_closures_fn.is_some()
                && info.methods.contains_key("birth")
            {
                let rc_ptr = self
                    .builder
                    .build_struct_gep(
                        cs_struct_ty,
                        child_self,
                        info.restart_count_field_idx,
                        "restart.count.pre.ptr",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                let v = self
                    .builder
                    .build_load(i64_t, rc_ptr, "restart.count.pre")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                Some(v.into_int_value())
            } else {
                None
            };

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

        if let Some(pre) = pre_count {
            // Post-handler restart check.
            // bumped = post > pre; under_cap = post <= 2;
            // should_rerun = bumped && under_cap.
            let rc_ptr = self
                .builder
                .build_struct_gep(
                    cs_struct_ty,
                    child_self,
                    info.restart_count_field_idx,
                    "restart.count.post.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let post = self
                .builder
                .build_load(i64_t, rc_ptr, "restart.count.post")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_int_value();
            let bumped = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::SGT,
                    post,
                    pre,
                    "restart.bumped",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let cap = i64_t.const_int(2, false);
            let under_cap = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::SLE,
                    post,
                    cap,
                    "restart.under_cap",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let should_rerun = self
                .builder
                .build_and(bumped, under_cap, "restart.should_rerun")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let func = self
                .current_fn
                .expect("current_fn set in __birth_closures body");
            let rerun_bb = self
                .context
                .append_basic_block(func, "restart.rerun");
            self.builder
                .build_conditional_branch(should_rerun, rerun_bb, post_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            // rerun_bb: m45 — gate on
            // __restart_in_place_pending. If set, branch to a
            // zero-fields pass (re-init each user field from
            // its declared default) and clear the flag before
            // call_birth_bb. Otherwise branch direct to
            // call_birth_bb. Both converge on the call_birth
            // block which fires birth + __birth_closures.
            self.builder.position_at_end(rerun_bb);
            let rip_ptr = self
                .builder
                .build_struct_gep(
                    cs_struct_ty,
                    child_self,
                    info.restart_in_place_pending_field_idx,
                    "restart_in_place.pending.load.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let rip_val = self
                .builder
                .build_load(i64_t, rip_ptr, "restart_in_place.pending")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_int_value();
            let is_in_place = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    rip_val,
                    i64_t.const_int(0, false),
                    "restart_in_place.is_pending",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let zero_fields_bb = self.context.append_basic_block(
                func,
                "restart_in_place.zero_fields",
            );
            let call_birth_bb = self.context.append_basic_block(
                func,
                "restart.call_birth",
            );
            self.builder
                .build_conditional_branch(
                    is_in_place,
                    zero_fields_bb,
                    call_birth_bb,
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

            // zero_fields_bb: re-store each declared default
            // into its user field, then clear the in-place
            // flag so a subsequent restart() (without _in_place)
            // doesn't accidentally repeat the zero pass.
            // Composite-default literals re-allocate in this
            // locus's own arena (via current_arena_override),
            // matching the instantiation-time discipline.
            self.builder.position_at_end(zero_fields_bb);
            let arena_slot = self
                .builder
                .build_struct_gep(
                    cs_struct_ty,
                    child_self,
                    info.arena_field_idx,
                    "restart_in_place.arena.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let locus_arena = self
                .builder
                .build_load(
                    self.context.ptr_type(AddressSpace::default()),
                    arena_slot,
                    "restart_in_place.arena",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                .into_pointer_value();
            let prev_override = self.current_arena_override;
            self.current_arena_override = Some(locus_arena);
            let scope = Scope::default();
            let defaults_snapshot = info.defaults.clone();
            for (fname, default) in &defaults_snapshot {
                let (val, _) = match default {
                    DefaultInit::Const(pv) => self.const_param(pv),
                    DefaultInit::Expr(e) => self.lower_expr(e, &scope)?,
                };
                let (slot_idx, _) = info
                    .fields
                    .get(fname)
                    .cloned()
                    .expect("field declared by declare_locus_struct");
                let field_slot = self
                    .builder
                    .build_struct_gep(
                        cs_struct_ty,
                        child_self,
                        slot_idx,
                        &format!("restart_in_place.{}.ptr", fname),
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                self.builder
                    .build_store(field_slot, val)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            }
            self.current_arena_override = prev_override;
            // Clear the pending flag; otherwise a subsequent
            // restart() (without _in_place) would zero again.
            self.builder
                .build_store(rip_ptr, i64_t.const_int(0, false))
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_unconditional_branch(call_birth_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

            // call_birth_bb: call birth(self) + recursively
            // call __birth_closures(self, parent_self,
            // on_failure), then ret void. The recursive call
            // may itself fail + restart, so the cap is
            // enforced naturally as the counter accumulates
            // across attempts.
            self.builder.position_at_end(call_birth_bb);
            let birth_fn = *info
                .methods
                .get("birth")
                .expect("birth method present");
            self.builder
                .build_call(
                    birth_fn,
                    &[child_self.into()],
                    "restart.birth.call",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let birth_closures_fn = info
                .birth_closures_fn
                .expect("birth_closures_fn present");
            self.builder
                .build_call(
                    birth_closures_fn,
                    &[
                        child_self.into(),
                        parent_self_or_null.into(),
                        on_failure_or_null.into(),
                    ],
                    "restart.birth_closures.call",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_return(None)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        } else {
            self.builder
                .build_unconditional_branch(post_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }

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
    /// m40: lower a `restart(child);` recovery call. Bumps
    /// child.__restart_count by 1; the post-on_failure dispatch
    /// inside __birth_closures re-runs birth() + birth-epoch
    /// closures iff the new count is <= 2 (the v0 cap).
    /// Beyond the cap, the bump still happens but the dispatch
    /// path skips the re-run, falling through to the parent's
    /// collapse path. The intent is design-time configurable
    /// (cap = 2 today, may become a per-locus annotation
    /// later) with runtime cost = one i64 load + add + store
    /// per call.
    fn lower_restart_call(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        self.lower_restart_call_kind(args, scope, false)
    }

    /// m45: lower `restart_in_place(c);`. Same shape as
    /// `restart(c)` — bumps `__restart_count` to drive the
    /// rerun branch in `__birth_closures` — but additionally
    /// sets `__restart_in_place_pending = 1` so the rerun
    /// branch zeros user fields back to declared defaults
    /// before re-running birth(). Cap (2 attempts) is shared
    /// with the regular restart path.
    fn lower_restart_in_place_call(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        self.lower_restart_call_kind(args, scope, true)
    }

    fn lower_restart_call_kind(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        in_place: bool,
    ) -> Result<BlockEnd, CodegenError> {
        let kind = if in_place { "restart_in_place" } else { "restart" };
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "{}() takes exactly one argument, got {}",
                kind,
                args.len()
            )));
        }
        let (val, ty) = self.lower_expr(&args[0], scope)?;
        let locus_name = match &ty {
            CodegenTy::LocusRef(n) => n.clone(),
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "{}() requires a locus reference; got {:?}",
                    kind, other
                )));
            }
        };
        let info = self
            .user_loci
            .get(&locus_name)
            .cloned()
            .expect("LocusRef points to a declared locus");
        let child_ptr = val.into_pointer_value();
        let i64_t = self.context.i64_type();
        let rc_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                child_ptr,
                info.restart_count_field_idx,
                "restart.count.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let cur = self
            .builder
            .build_load(i64_t, rc_ptr, "restart.count.cur")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let one = i64_t.const_int(1, false);
        let next = self
            .builder
            .build_int_add(cur.into_int_value(), one, "restart.count.next")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(rc_ptr, next)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        if in_place {
            // m45: flag the next re-run as in-place. The rerun
            // branch in __birth_closures will see this set,
            // re-init user fields from declared defaults, and
            // clear the flag back to 0 before calling birth().
            let rip_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    child_ptr,
                    info.restart_in_place_pending_field_idx,
                    "restart_in_place.pending.ptr",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            self.builder
                .build_store(rip_ptr, one)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        // m46: zero each closure's accumulators unless its
        // `persists_through(...)` clause names this recovery
        // event. Default = reset.
        self.emit_accumulator_reset_for_event(
            &info, child_ptr, kind, &locus_name,
        )?;
        Ok(BlockEnd::Open)
    }

    /// m41: lower a `quarantine(child);` recovery call. Sets
    /// child.__quarantined = 1, then calls
    /// `lotus_bus_quarantine_self(child_ptr)` to deregister any
    /// bus subscriptions (the C runtime walks its entries vec and
    /// nulls out the subject of every match — dispatch then skips
    /// those slots, so quarantined subscribers stop receiving bus
    /// messages). Lifecycle dispatch in lower_locus_instantiation
    /// reads the flag after birth + __birth_closures and skips
    /// run() if set; drain / dissolve still fire (cleanup is
    /// unconditional). Repeat calls are idempotent.
    fn lower_quarantine_call(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<BlockEnd, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "quarantine() takes exactly one argument, got {}",
                args.len()
            )));
        }
        let (val, ty) = self.lower_expr(&args[0], scope)?;
        let locus_name = match &ty {
            CodegenTy::LocusRef(n) => n.clone(),
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "quarantine() requires a locus reference; got {:?}",
                    other
                )));
            }
        };
        let info = self
            .user_loci
            .get(&locus_name)
            .cloned()
            .expect("LocusRef points to a declared locus");
        let child_ptr = val.into_pointer_value();
        let i64_t = self.context.i64_type();
        let q_ptr = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                child_ptr,
                info.quarantined_field_idx,
                "quarantine.flag.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let one = i64_t.const_int(1, false);
        self.builder
            .build_store(q_ptr, one)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // m41b semantic, m45-followup form: hand the child off to
        // the C runtime, which walks its entries vec and nulls out
        // every matching subscription's subject. Idempotent — repeat
        // calls just keep finding already-nulled entries and writing
        // NULL again.
        if self.bus_state.is_some() {
            let unsub_fn = self
                .module
                .get_function("lotus_bus_quarantine_self")
                .expect("lotus_bus_quarantine_self declared in declare_builtins");
            self.builder
                .build_call(
                    unsub_fn,
                    &[child_ptr.into()],
                    "bus.quarantine.self.call",
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        // m46: zero each closure's accumulators unless its
        // `persists_through(...)` clause names "quarantine".
        self.emit_accumulator_reset_for_event(
            &info, child_ptr, "quarantine", &locus_name,
        )?;
        Ok(BlockEnd::Open)
    }

    /// m44: lower a `check_closures();` builtin call. Fires
    /// every explicit-epoch closure on the current self —
    /// the user-triggered audit checkpoint. Routes violations
    /// through the same parent on_failure path the
    /// birth/dissolve/tick epochs use (resolved at the call
    /// site via `resolve_failure_route`). Takes 0 arguments;
    /// implicit self is read from `current_self`. A no-op
    /// when the locus has no explicit-epoch closures —
    /// rather than rejecting at compile time, this lets a
    /// user idiomatically call `check_closures()` after a
    /// state change without first checking whether any
    /// such closure exists.
    fn lower_check_closures_call(
        &mut self,
        args: &[Expr],
    ) -> Result<(), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "check_closures() takes 0 arguments, got {}",
                args.len()
            )));
        }
        let cs = self.current_self.as_ref().ok_or_else(|| {
            CodegenError::Unsupported(
                "check_closures() must be called from inside a locus body"
                    .into(),
            )
        })?;
        let locus_name = cs.locus_name.clone();
        let self_ptr = cs.self_ptr;
        let info = self
            .user_loci
            .get(&locus_name)
            .cloned()
            .expect("current_self points to a declared locus");
        let Some(explicit_fn) = info.explicit_closures_fn else {
            // No explicit closures on this locus — silent no-op.
            return Ok(());
        };
        // Read __parent_self / __parent_on_failure baked onto
        // the struct at instantiation time. resolve_failure_route
        // is the wrong helper here — it answers "as a parent
        // running my child's instantiation, what's MY
        // failure_handler for that child?" — but check_closures
        // routes the caller's-OWN violations to the caller's
        // parent's on_failure, which is what the m42 struct
        // fields hold.
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let parent_self_slot = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.parent_self_field_idx,
                "explicit.parent_self.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let parent_self = self
            .builder
            .build_load(ptr_t, parent_self_slot, "explicit.parent_self")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        let handler_slot = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                self_ptr,
                info.parent_on_failure_field_idx,
                "explicit.parent_handler.ptr",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let handler_ptr = self
            .builder
            .build_load(ptr_t, handler_slot, "explicit.parent_handler")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_pointer_value();
        self.builder
            .build_call(
                explicit_fn,
                &[
                    self_ptr.into(),
                    parent_self.into(),
                    handler_ptr.into(),
                ],
                &format!("{}.__explicit_closures.call", locus_name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }

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
        if ty != CodegenTy::TypeRef("ClosureViolation".into()) {
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

    /// Lower a `subject <- payload;` statement to a single call to
    /// the C-runtime `lotus_bus_dispatch(queue, subject, payload, size)`.
    /// Subject must evaluate to a String pointer; payload must be a
    /// TypeRef value (a pointer to a user-type struct). The C
    /// runtime walks its (heap-grown) entries vec and routes each
    /// match either to the cooperative queue or to a pinned
    /// subscriber's mailbox, by mailbox-null-or-not at registration.
    fn lower_send(
        &mut self,
        subject: &Expr,
        value: &Expr,
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        let _ = self.bus_state.ok_or_else(|| {
            CodegenError::Unsupported(
                "bus send `<-` used but no `bus subscribe` declared in \
                 program — nothing to dispatch to"
                    .to_string(),
            )
        })?;
        let (subj_val, subj_ty) = self.lower_expr(subject, scope)?;
        if subj_ty != CodegenTy::String {
            return Err(CodegenError::Unsupported(format!(
                "bus send subject must be String; got {:?}",
                subj_ty
            )));
        }
        let (payload_val, payload_ty) = self.lower_expr(value, scope)?;
        // m47-payloads-followup: bus payload is either a
        // user-type struct pointer OR a has-payload enum
        // pointer. Both lower to a ptr value + a sized storage
        // struct. m60: payload bytes flow through __serialize_T
        // before reaching lotus_bus_dispatch, so the wire format
        // is governed by the per-type serializer rather than
        // implicit struct-layout assumption.
        let (payload_type_name, payload_struct_ty) = match &payload_ty {
            CodegenTy::TypeRef(name) => {
                let info = self
                    .user_types
                    .get(name)
                    .cloned()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "bus payload type `{}` not declared",
                            name
                        ))
                    })?;
                (name.clone(), info.struct_ty)
            }
            CodegenTy::Enum(name) => {
                let info = self
                    .user_enums
                    .get(name)
                    .cloned()
                    .ok_or_else(|| {
                        CodegenError::Unsupported(format!(
                            "bus payload enum `{}` not declared",
                            name
                        ))
                    })?;
                if !info.has_payload {
                    return Err(CodegenError::Unsupported(format!(
                        "bus send of no-payload enum `{}` not supported \
                         at v0.1 — wrap in a struct or add a variant payload",
                        name
                    )));
                }
                (name.clone(), self.enum_storage_struct(&info))
            }
            other => {
                return Err(CodegenError::Unsupported(format!(
                    "bus send payload must be a user-type or has-payload \
                     enum value; got {:?}",
                    other
                )));
            }
        };
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let i64_t = self.context.i64_type();
        let payload_size_iv = payload_struct_ty
            .size_of()
            .expect("payload struct has known size");

        // m70: pass struct bytes directly to lotus_bus_dispatch
        // along with the per-subject __serialize_T fn pointer.
        // The dispatcher does local enqueue with struct bytes
        // (preserving in-process semantics: String pointers stay
        // valid because the publisher's arena outlives the
        // immediate dispatch), and serializes through the
        // supplied fn into wire bytes for cross-process fanout.
        // Pre-m70 lower_send allocated a scratch buffer + called
        // __serialize_T inline; m70 moves serialization into the
        // C runtime so the wire bytes are only computed when
        // they're about to be sent.
        let ser_fn = self
            .serializers
            .get(&payload_type_name)
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "no serializer for bus payload `{}` — pass A3 should \
                     have synthesized one",
                    payload_type_name
                ))
            })?
            .serialize;

        let queue_global = self
            .module
            .get_global("lotus.bus_queue.global")
            .expect("bus queue global declared");
        let queue_ptr = self
            .builder
            .build_load(
                ptr_t,
                queue_global.as_pointer_value(),
                "bus.dispatch.queue",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let dispatch_fn = self
            .module
            .get_function("lotus_bus_dispatch")
            .expect("lotus_bus_dispatch declared in declare_builtins");
        let _ = i64_t;
        self.builder
            .build_call(
                dispatch_fn,
                &[
                    queue_ptr.into(),
                    subj_val.into(),
                    payload_val.into(),
                    payload_size_iv.into(),
                    ser_fn.as_global_value().as_pointer_value().into(),
                ],
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

    /// m47-payloads: storage struct for a payload-bearing enum:
    /// `{ i32 tag, [N x i8] body }` where N is `payload_bytes`.
    /// The struct is the same for every variant of the enum;
    /// per-variant payloads sit inside the `body` byte array,
    /// re-interpreted by the variant's field types at access
    /// time. Caller must check `has_payload` before calling.
    fn enum_storage_struct(
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

    /// m47-payloads: allocate a payload-bearing enum value in
    /// the current arena, store the variant tag, then store
    /// each payload field into the body byte array (interpreted
    /// per the variant's field types). Returns the pointer to
    /// the storage struct. For no-payload variants of a
    /// payload-bearing enum, pass an empty `field_vals` —
    /// only the tag is written, body bytes stay uninitialized
    /// but are never read for those variants.
    fn lower_enum_variant_alloc(
        &mut self,
        info: &EnumInfo,
        tag: u32,
        field_vals: &[(BasicValueEnum<'ctx>, CodegenTy)],
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let storage_ty = self.enum_storage_struct(info);
        let size = storage_ty
            .size_of()
            .expect("enum storage struct has known size");
        let ptr = self.arena_alloc(size, "enum.alloc")?;
        let i32_t = self.context.i32_type();
        let tag_ptr = self
            .builder
            .build_struct_gep(storage_ty, ptr, 0, "enum.tag.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(tag_ptr, i32_t.const_int(tag as u64, false))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        if !field_vals.is_empty() {
            // Body lives at field index 1. We GEP into the body
            // byte array and use it as a base; each field stores
            // at an 8-byte stride (per codegen_ty_size_bytes,
            // which rounds everything to 8 except Decimal at 16).
            let body_ptr = self
                .builder
                .build_struct_gep(storage_ty, ptr, 1, "enum.body.ptr")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let mut offset: u64 = 0;
            let i8_t = self.context.i8_type();
            for (val, ty) in field_vals {
                let off_const = self
                    .context
                    .i64_type()
                    .const_int(offset, false);
                let slot_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            i8_t,
                            body_ptr,
                            &[off_const],
                            "enum.field.ptr",
                        )
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
                };
                self.builder
                    .build_store(slot_ptr, *val)
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                offset += codegen_ty_size_bytes(self.context, ty);
            }
        }
        Ok(ptr)
    }

    /// m47-payloads: load a payload-bearing enum's tag through
    /// the storage-struct pointer. Used by both println and
    /// pattern matching when the scrutinee is a has-payload
    /// enum value.
    fn load_enum_tag(
        &mut self,
        info: &EnumInfo,
        enum_ptr: PointerValue<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        let storage_ty = self.enum_storage_struct(info);
        let i32_t = self.context.i32_type();
        let tag_ptr = self
            .builder
            .build_struct_gep(storage_ty, enum_ptr, 0, "enum.tag.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let tag = self
            .builder
            .build_load(i32_t, tag_ptr, "enum.tag")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        Ok(tag)
    }

    /// m47-payloads: emit deep equality between two has-payload
    /// enum values. Tag-equality is necessary; on a tag match we
    /// dispatch via switch into per-variant per-field comparison.
    /// String fields compare via `lotus_str_eq`; primitive fields
    /// compare via the appropriate int / float predicate. Returns
    /// an i1 holding the equality result.
    ///
    /// Sequencing: branch out of the current block into a fresh
    /// `cont` block that PHIs the result; per-variant blocks
    /// compute their own AND-chain and branch into `cont`. Caller
    /// is responsible for picking up at `cont` and using the
    /// returned i1.
    fn lower_enum_deep_eq(
        &mut self,
        info: &EnumInfo,
        lv_ptr: PointerValue<'ctx>,
        rv_ptr: PointerValue<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        let func = self
            .current_fn
            .expect("lower_enum_deep_eq inside a function body");
        let bool_t = self.context.bool_type();
        let i32_t = self.context.i32_type();

        let lt = self.load_enum_tag(info, lv_ptr)?;
        let rt = self.load_enum_tag(info, rv_ptr)?;
        let tag_eq = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, lt, rt, "enum.tag.eq")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let cont_bb = self.context.append_basic_block(func, "enum.eq.cont");
        let dispatch_bb =
            self.context.append_basic_block(func, "enum.eq.dispatch");
        let mismatch_bb =
            self.context.append_basic_block(func, "enum.eq.mismatch");
        self.builder
            .build_conditional_branch(tag_eq, dispatch_bb, mismatch_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Per-variant blocks: each computes its own AND-chain over
        // field-eqs and branches into cont with that result.
        // Default landing for the (impossible-after-tag-eq) tag-
        // out-of-range case is `true` — same as having no
        // remaining payload to compare.
        let mut variant_blocks: Vec<(
            inkwell::values::IntValue<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
            inkwell::values::IntValue<'ctx>,
        )> = Vec::new();
        let default_bb =
            self.context.append_basic_block(func, "enum.eq.default");

        for (idx, v) in info.variants.iter().enumerate() {
            let v = v.clone();
            let case_bb = self
                .context
                .append_basic_block(func, &format!("enum.eq.v{}", idx));
            self.builder.position_at_end(case_bb);
            let mut acc: inkwell::values::IntValue<'ctx> =
                bool_t.const_int(1, false);
            if !v.field_tys.is_empty() {
                let l_fields =
                    self.load_enum_payload_fields(info, lv_ptr, idx)?;
                let r_fields =
                    self.load_enum_payload_fields(info, rv_ptr, idx)?;
                for (j, ((lv_f, ty), (rv_f, _))) in
                    l_fields.iter().zip(r_fields.iter()).enumerate()
                {
                    let cmp = self.lower_match_eq_cmp(
                        *lv_f,
                        *rv_f,
                        ty,
                        &format!("enum.eq.v{}.f{}", idx, j),
                    )?;
                    acc = self
                        .builder
                        .build_and(acc, cmp, &format!("enum.eq.v{}.acc", idx))
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                }
            }
            self.builder
                .build_unconditional_branch(cont_bb)
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            variant_blocks.push((
                i32_t.const_int(idx as u64, false),
                case_bb,
                acc,
            ));
        }

        // Default block: same shape — branch to cont with `true`.
        self.builder.position_at_end(default_bb);
        let default_acc = bool_t.const_int(1, false);
        self.builder
            .build_unconditional_branch(cont_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Mismatch block: result is false, branch to cont.
        self.builder.position_at_end(mismatch_bb);
        let mismatch_acc = bool_t.const_int(0, false);
        self.builder
            .build_unconditional_branch(cont_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // Dispatch block: switch on lt into the per-variant blocks.
        self.builder.position_at_end(dispatch_bb);
        let cases: Vec<(
            inkwell::values::IntValue<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
        )> = variant_blocks
            .iter()
            .map(|(c, b, _)| (*c, *b))
            .collect();
        self.builder
            .build_switch(lt, default_bb, &cases)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // cont: PHI over the per-block results.
        self.builder.position_at_end(cont_bb);
        let phi = self
            .builder
            .build_phi(bool_t, "enum.eq.phi")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let mut incoming: Vec<(
            &dyn inkwell::values::BasicValue<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
        )> = Vec::new();
        // SAFETY: each `acc` has bool_t type; PHI accepts any
        // BasicValue, so we coerce via &inkwell::values::IntValue
        // which implements BasicValue.
        for (_, bb, acc) in &variant_blocks {
            incoming.push((acc, *bb));
        }
        incoming.push((&default_acc, default_bb));
        incoming.push((&mismatch_acc, mismatch_bb));
        phi.add_incoming(&incoming);
        Ok(phi.as_basic_value().into_int_value())
    }

    /// m47-payloads: load each payload field of a variant from
    /// the body byte array. Field offsets are 8-byte strides
    /// (16 for Decimal). Used at pattern-match time when the
    /// constructor pattern carries bindings.
    fn load_enum_payload_fields(
        &mut self,
        info: &EnumInfo,
        enum_ptr: PointerValue<'ctx>,
        variant_idx: usize,
    ) -> Result<Vec<(BasicValueEnum<'ctx>, CodegenTy)>, CodegenError> {
        let storage_ty = self.enum_storage_struct(info);
        let body_ptr = self
            .builder
            .build_struct_gep(storage_ty, enum_ptr, 1, "enum.body.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let mut out: Vec<(BasicValueEnum<'ctx>, CodegenTy)> = Vec::new();
        let mut offset: u64 = 0;
        let i8_t = self.context.i8_type();
        let v = info.variants[variant_idx].clone();
        for ty in &v.field_tys {
            let off_const = self
                .context
                .i64_type()
                .const_int(offset, false);
            let slot_ptr = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        i8_t,
                        body_ptr,
                        &[off_const],
                        "enum.field.ptr",
                    )
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            };
            let llvm_ty = self.llvm_basic_type(ty);
            let val = self
                .builder
                .build_load(llvm_ty, slot_ptr, "enum.field")
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            out.push((val, ty.clone()));
            offset += codegen_ty_size_bytes(self.context, ty);
        }
        Ok(out)
    }

    /// m47-followup: lazily build (or look up) a `[N x ptr]`
    /// global of "EnumName::VariantName" string pointers for an
    /// enum, indexed by variant tag. Used by `println` so an
    /// enum value renders the same string the interpreter's
    /// `Value::EnumVariant::display` produces. Returns the
    /// global's address so a single GEP + load reads any
    /// variant's name.
    fn enum_names_array(
        &mut self,
        enum_name: &str,
    ) -> Result<inkwell::values::GlobalValue<'ctx>, CodegenError> {
        let global_name = format!("lotus.enum.{}.names", enum_name);
        if let Some(g) = self.module.get_global(&global_name) {
            return Ok(g);
        }
        let info = self
            .user_enums
            .get(enum_name)
            .cloned()
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "enum `{}` not registered",
                    enum_name
                ))
            })?;
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let mut entries: Vec<inkwell::values::PointerValue<'ctx>> = Vec::new();
        for v in &info.variants {
            let label = format!("{}::{}", enum_name, v.name);
            entries.push(self.global_string(&label));
        }
        let array_ty = ptr_t.array_type(entries.len() as u32);
        let init = ptr_t.const_array(&entries);
        let g = self.module.add_global(array_ty, None, &global_name);
        g.set_initializer(&init);
        g.set_linkage(inkwell::module::Linkage::Internal);
        g.set_constant(true);
        Ok(g)
    }

    /// F.20 Phase B: synthesize (once) and return the LLVM global
    /// holding the vtable for a (locus, interface) pair. The vtable
    /// is `[N x ptr]` where N is the interface's method count and
    /// slot i is the locus method that satisfies interface method i
    /// (matched by name; the typechecker has already verified the
    /// structural impl). Cached in `self.vtables` keyed by
    /// (locus, iface) so multiple coercion sites share one global.
    fn ensure_vtable(
        &mut self,
        locus_name: &str,
        iface_name: &str,
    ) -> Result<inkwell::values::GlobalValue<'ctx>, CodegenError> {
        let key = (locus_name.to_string(), iface_name.to_string());
        if let Some(g) = self.vtables.get(&key) {
            return Ok(*g);
        }
        // Find the interface decl in the AST so we can pull method
        // order. Method bodies are not allowed (no defaults at v0);
        // signatures-only is exactly what we need.
        let iface_methods: Vec<String> = self
            .program
            .items
            .iter()
            .find_map(|item| match item {
                TopDecl::Interface(i) if i.name.name == iface_name => Some(
                    i.methods.iter().map(|m| m.name.name.clone()).collect(),
                ),
                _ => None,
            })
            .ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "vtable synth: interface `{}` not declared",
                    iface_name
                ))
            })?;
        let info = self.user_loci.get(locus_name).cloned().ok_or_else(|| {
            CodegenError::Unsupported(format!(
                "vtable synth: locus `{}` not declared",
                locus_name
            ))
        })?;
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let mut entries: Vec<inkwell::values::PointerValue<'ctx>> =
            Vec::with_capacity(iface_methods.len());
        for method_name in &iface_methods {
            let func = info.user_methods.get(method_name).ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "vtable synth: locus `{}` has no method `{}` (interface `{}`)",
                    locus_name, method_name, iface_name
                ))
            })?;
            entries.push(func.as_global_value().as_pointer_value());
        }
        let array_ty = ptr_t.array_type(entries.len() as u32);
        let init = ptr_t.const_array(&entries);
        let global_name = format!("__vt.{}.{}", locus_name, iface_name);
        let g = self.module.add_global(array_ty, None, &global_name);
        g.set_initializer(&init);
        g.set_linkage(inkwell::module::Linkage::Internal);
        g.set_constant(true);
        self.vtables.insert(key, g);
        Ok(g)
    }

    /// F.20 Phase B: build a fat-pointer interface value from a
    /// concrete locus pointer. Allocates a 16-byte `{data, vtable}`
    /// struct in the current arena, stores the locus pointer at
    /// slot 0 and the per-(locus, iface) vtable global address at
    /// slot 1, and returns a pointer to the struct. The returned
    /// pointer is the LLVM-level representation of a value whose
    /// CodegenTy is `Interface(iface_name)`.
    fn coerce_to_interface(
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

    /// The LLVM struct type for an interface fat pointer, used by
    /// coercion (store) and dispatch (load) sites so both agree on
    /// slot layout. Two ptr slots: data, then vtable.
    fn iface_fat_struct_ty(&self) -> inkwell::types::StructType<'ctx> {
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(&[ptr_t.into(), ptr_t.into()], false)
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
        // m49: when lowering a non-main free fn body, allocations
        // route through the per-call subregion (held in
        // current_user_fn_arena's alloca) instead of the program-
        // wide arena.global. arena.global remains the fallback for
        // main only — main's body sets `in_main` and never touches
        // current_user_fn_arena.
        if let Some(fn_arena_alloca) = self.current_user_fn_arena {
            let arena_ptr = self
                .builder
                .build_load(ptr_t, fn_arena_alloca, "fn.arena.cur")
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

    /// Allocate a locus struct in the current fn's entry block
    /// and store NULL into its arena field there. Used for
    /// deferred-dissolve loci (let-bound or bus-subscribed) so
    /// the dissolve frame's flush can safely null-check the
    /// arena field per entry — entries whose `let` never
    /// executed on this control-flow path read NULL and get
    /// skipped, instead of dereferencing uninitialized stack.
    fn alloca_in_entry_with_nulled_arena(
        &mut self,
        struct_ty: inkwell::types::StructType<'ctx>,
        arena_field_idx: u32,
        name: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let func = self
            .current_fn
            .expect("alloca_in_entry called outside a fn body");
        let entry_bb = func
            .get_first_basic_block()
            .expect("fn has an entry block");
        let saved = self.builder.get_insert_block();
        // Position before the entry block's terminator (if any)
        // — for unfinished entry blocks the builder positions at
        // the end; for finished ones we use position_before of
        // the first instruction so the alloca lands at the top.
        if let Some(first_instr) = entry_bb.get_first_instruction() {
            self.builder.position_before(&first_instr);
        } else {
            self.builder.position_at_end(entry_bb);
        }
        let ptr_t = self.context.ptr_type(AddressSpace::default());
        let slot = self
            .builder
            .build_alloca(struct_ty, name)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let arena_field_ptr = self
            .builder
            .build_struct_gep(
                struct_ty,
                slot,
                arena_field_idx,
                &format!("{}.__arena.entry_null", name),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(arena_field_ptr, ptr_t.const_null())
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        if let Some(bb) = saved {
            self.builder.position_at_end(bb);
        }
        Ok(slot)
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
        // Deregister from the bus router BEFORE freeing the arena.
        // Without this step, a stale entry in the C-runtime entries
        // vec would point self_ptr at memory whose arena is about
        // to be freed; a subsequent `<-` to one of this locus's
        // subscriptions would have dispatch read `*(arena_t **)
        // self_ptr` after free, then memcpy a payload into freed
        // chunks. Today's programs don't publish post-dissolve,
        // but the invariant is fragile — close it here using the
        // same null-subject-sentinel mechanism `quarantine(c)`
        // already uses (m41b / m45-followup-2). No-op when the
        // program has no subscribes.
        if self.bus_state.is_some() {
            let unsub_fn = self
                .module
                .get_function("lotus_bus_quarantine_self")
                .expect("lotus_bus_quarantine_self declared");
            self.builder
                .build_call(
                    unsub_fn,
                    &[self_ptr.into()],
                    &format!("{}.bus.deregister.call", locus_name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }
        // F.22: tear down capacity slots in reverse declaration
        // order, before slot 0 / arena destroy. Each slot loads
        // its allocator pointer from `__slot_<name>` and calls
        // the matching destroy fn. Per spec §F.22, slot teardown
        // sits between drain/dissolve closures and the arena's
        // wholesale free, so cells outlive everything except
        // the arena itself during dissolve.
        for slot in info.capacity_slots.iter().rev() {
            let slot_field_ptr = self
                .builder
                .build_struct_gep(
                    info.struct_ty,
                    self_ptr,
                    slot.struct_field_idx,
                    &format!("{}.__slot_{}.ptr", locus_name, slot.name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let allocator = self
                .builder
                .build_load(
                    ptr_t,
                    slot_field_ptr,
                    &format!("{}.__slot_{}", locus_name, slot.name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
            let destroy_fn_name = match slot.kind {
                CapacitySlotKind::Pool => "lotus_pool_destroy",
                CapacitySlotKind::Heap => "lotus_heap_destroy",
            };
            let destroy_fn = self
                .module
                .get_function(destroy_fn_name)
                .expect("F.22 allocator destroy extern declared");
            self.builder
                .build_call(
                    destroy_fn,
                    &[allocator.into()],
                    &format!("{}.{}.destroy", locus_name, slot.name),
                )
                .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        }

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
    locals: BTreeMap<String, (PointerValue<'ctx>, CodegenTy)>,
}

#[derive(Debug, Clone)]
enum ParamValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Duration(i64),
    Decimal(i128),
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
            let m = parse_decimal_to_i128_scale9(s).ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "Decimal literal `{}` failed to parse",
                    s
                ))
            })?;
            Ok(ParamValue::Decimal(m))
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

/// m48: parse a Decimal literal source spelling (e.g. `"100.40d"`,
/// `"0.001"`) into the i128 mantissa with implicit scale 9.
/// Strips an optional trailing `d`, an optional leading sign,
/// underscores. Returns None on malformed input.
fn parse_decimal_to_i128_scale9(s: &str) -> Option<i128> {
    let s = s.strip_suffix('d').unwrap_or(s);
    let (sign, rest) = match s.as_bytes().first() {
        Some(b'-') => (-1i128, &s[1..]),
        Some(b'+') => (1i128, &s[1..]),
        _ => (1i128, s),
    };
    if rest.is_empty() {
        return None;
    }
    let mut mantissa: i128 = 0;
    let mut frac_digits: u32 = 0;
    let mut seen_dot = false;
    let mut seen_digit = false;
    for c in rest.chars() {
        match c {
            '0'..='9' => {
                seen_digit = true;
                if !seen_dot || frac_digits < 9 {
                    mantissa = mantissa.checked_mul(10)?;
                    mantissa = mantissa.checked_add((c as u8 - b'0') as i128)?;
                    if seen_dot {
                        frac_digits += 1;
                    }
                }
                // Drops digits past 9 fractional places (codegen
                // truncates at scale 9; no rounding for v0.1).
            }
            '.' if !seen_dot => seen_dot = true,
            '_' => {}
            _ => return None,
        }
    }
    if !seen_digit {
        return None;
    }
    while frac_digits < 9 {
        mantissa = mantissa.checked_mul(10)?;
        frac_digits += 1;
    }
    Some(sign * mantissa)
}

/// Build an i128 LLVM constant from a Rust i128. inkwell's
/// `const_int_arbitrary_precision` takes a slice of i64 limbs;
/// pack the i128 as low/high halves so the constant matches the
/// host i128 bit pattern.
fn i128_const<'ctx>(
    ctx: &'ctx inkwell::context::Context,
    v: i128,
) -> inkwell::values::IntValue<'ctx> {
    let lo = (v as u128) as u64;
    let hi = ((v as u128) >> 64) as u64;
    ctx.i128_type().const_int_arbitrary_precision(&[lo, hi])
}

/// m47-payloads: byte-size of a Lotus type for enum payload
/// layout. Pads each field to 8-byte alignment so the per-variant
/// fields can be GEP'd via uniform 8-byte strides without
/// per-field alignment computation. This overestimates for
/// Bool / i32 enum-tag fields, which is fine — `payload_bytes`
/// becomes the body's `[N x i8]` length, not a tight pack.
/// Decimal at i128 takes 16 bytes; everything else (Int/Float/
/// String-ptr/Bool/Duration/Time/LocusRef/TypeRef/Array/Tuple/
/// Enum-as-pointer/Enum-as-i32) round-trips at 8.
fn codegen_ty_size_bytes<'ctx>(
    _ctx: &'ctx inkwell::context::Context,
    t: &CodegenTy,
) -> u64 {
    match t {
        CodegenTy::Decimal => 16,
        _ => 8,
    }
}

/// m46 / m46-vocab (closure accumulators): walk an expression
/// tree, append every accumulator-builtin call's (kind,
/// inner_expr_or_none) to `out` in tree traversal order.
///
/// Three forms recognized:
/// - `Expr::Sum(inner)` (parser-dedicated AST variant for `sum(x)`)
/// - `Call(Ident("count"), [])` for the no-arg count accumulator
/// - `Call(Ident("mean"), [arg])` for the running mean
///
/// Doesn't recurse into an accumulator's own argument — nested
/// accumulators are rejected at type-inference time anyway.
fn collect_sum_calls(expr: &Expr, out: &mut Vec<(AccumulatorKind, Option<Expr>)>) {
    match expr {
        Expr::Sum(inner, _) => {
            out.push((AccumulatorKind::Sum, Some((**inner).clone())));
        }
        Expr::Call { callee, args, .. } => {
            if let Expr::Ident(id) = callee.as_ref() {
                if id.name == "count" && args.is_empty() {
                    out.push((AccumulatorKind::Count, None));
                    return;
                }
                if id.name == "mean" && args.len() == 1 {
                    out.push((AccumulatorKind::Mean, Some(args[0].clone())));
                    return;
                }
            }
            collect_sum_calls(callee, out);
            for a in args {
                collect_sum_calls(a, out);
            }
        }
        Expr::Binary { left, right, .. } => {
            collect_sum_calls(left, out);
            collect_sum_calls(right, out);
        }
        Expr::Unary { operand, .. } => collect_sum_calls(operand, out),
        Expr::Field { receiver, .. } => collect_sum_calls(receiver, out),
        Expr::Index { receiver, index, .. } => {
            collect_sum_calls(receiver, out);
            collect_sum_calls(index, out);
        }
        _ => {}
    }
}

/// m46: infer the CodegenTy of an accumulator's inner expression.
/// v0 supports `self.X` reads only, where X is a numeric param —
/// type comes straight from the locus's param map (`fields`).
/// Anything else errors with a concrete message naming the closure
/// + locus so the user knows where to look.
fn infer_accumulator_inner_type(
    locus_name: &str,
    closure_name: &str,
    inner: &Expr,
    fields: &BTreeMap<String, (u32, CodegenTy)>,
) -> Result<CodegenTy, CodegenError> {
    if let Expr::Field { receiver, name, .. } = inner {
        if let Expr::KwSelf(_) = receiver.as_ref() {
            let (_, ty) = fields.get(&name.name).ok_or_else(|| {
                CodegenError::Unsupported(format!(
                    "closure `{}` on locus `{}`: accumulator `sum(self.{})` \
                     references unknown field",
                    closure_name, locus_name, name.name
                ))
            })?;
            match ty {
                CodegenTy::Int
                | CodegenTy::Float
                | CodegenTy::Decimal
                | CodegenTy::Duration => return Ok(ty.clone()),
                other => {
                    return Err(CodegenError::Unsupported(format!(
                        "closure `{}` on locus `{}`: accumulator `sum(self.{})` \
                         requires a numeric type (Int / Float / Decimal / \
                         Duration); got {:?}",
                        closure_name, locus_name, name.name, other
                    )))
                }
            }
        }
    }
    Err(CodegenError::Unsupported(format!(
        "closure `{}` on locus `{}`: accumulator inner expr must be `self.X` \
         in v0 (got a more complex form); reduce to a single field reference",
        closure_name, locus_name
    )))
}

/// LLVM produces architecture-specific triples; expose a way
/// to override for cross-compilation tests later.
pub fn host_triple() -> TargetTriple {
    TargetMachine::get_default_triple()
}
