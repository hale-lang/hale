//! Typecheck M3 stage 1 (2026-07-02): the stdlib path-call NAME
//! surface — typo detection for `std::<ns>::<fn>(...)` calls.
//!
//! R2 (2026-07-29): this table is becoming THE stdlib registry —
//! the single row per fn that the four parallel structures (this
//! surface, `signature_for`, the codegen dispatch arms, the docs)
//! converge on. Each entry now carries an [`EffectSet`] column:
//! the classified frontier #265's effect assertions query. Every
//! entry starts `UNCLASSIFIED`; #265 step 4 classifies the
//! surface, after which an unclassified entry becomes a build
//! error (the "frontier stays true forever" discipline).
//!
//! Names only, deliberately: a wrong name entry here produces a
//! cheap, obvious false "unknown stdlib function" that's fixed by
//! adding the name; a wrong SIGNATURE entry (stage 2) produces an
//! expensive false type mismatch on valid code. Namespaces absent
//! from this table keep the historical permissive behavior
//! (`Ty::Unknown`), so incompleteness degrades to the status quo,
//! never to a false error — EXCEPT within a tabled namespace, where
//! an unknown name is a hard error with a did-you-mean.
//!
//! Source of truth: the codegen dispatch in
//! `crates/hale-codegen/src/stdlib/*.rs` (+ the fallible path-call
//! dispatch in `channels/mod.rs`), cross-checked against
//! `spec/stdlib.md`'s module-surface table. When those two
//! disagree, the DISPATCH is reality; fix the spec.
//!
//! F.40 phase 4, S2: the surface and the signature table are one
//! table, [`SURFACES`], with one [`StdFn`] row per function: its name,
//! whether user code may call it ([`Visibility`]), its effect set, its
//! signature when it has one, and how it lowers ([`Lower`]). Every
//! question below — [`lookup`], [`unknown_fn_error`], [`suggest`],
//! [`effects_for`], [`signature_for`] — reads that one row.

use hale_syntax::ast::PrimType;

use crate::ty::Ty;

/// M3 stage 2 (2026-07-02): const-constructible type vocabulary for
/// the signature table. Maps to `Ty` at check time. `Any` types as
/// Unknown — bidirectionally assignable — for the rare polymorphic
/// arg; use it rather than guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigTy {
    Int,
    Uint,
    Float,
    Bool,
    Str,
    Bytes,
    BytesMut,
    Decimal,
    Duration,
    Time,
    Unit,
    Any,
    /// A stdlib locus/struct handle (File, Stream, Child, ...).
    /// Matches `Ty::Named` by (last-segment) name. Use only when
    /// the handle's typecheck-side name is verified — when in
    /// doubt, `Any` keeps arity/other-arg checking without the
    /// mistyping risk.
    Named(&'static str),
}

impl SigTy {
    pub fn to_ty(self) -> Ty {
        match self {
            SigTy::Int => Ty::Prim(PrimType::Int),
            SigTy::Uint => Ty::Prim(PrimType::Uint),
            SigTy::Float => Ty::Prim(PrimType::Float),
            SigTy::Bool => Ty::Prim(PrimType::Bool),
            SigTy::Str => Ty::Prim(PrimType::String),
            SigTy::Bytes => Ty::Prim(PrimType::Bytes),
            SigTy::BytesMut => Ty::Prim(PrimType::BytesMut),
            SigTy::Decimal => Ty::Prim(PrimType::Decimal),
            SigTy::Duration => Ty::Prim(PrimType::Duration),
            SigTy::Time => Ty::Prim(PrimType::Time),
            SigTy::Unit => Ty::Unit,
            SigTy::Any => Ty::Unknown,
            SigTy::Named(n) => Ty::Named(n.to_string()),
        }
    }

    /// Arg-position acceptance — strict prim equality plus the
    /// coercions the LOWERING actually performs (verified per-fn,
    /// 2026-07-02), permissive on Unknown either side:
    /// - Bytes family: BytesView/BytesMut are runtime-identical
    ///   windows; readers accept all three (raw `_raw` siblings).
    /// - Str accepts StringView (unpack_view_if_needed at every
    ///   String-arg position).
    /// - Float accepts Int/Uint (math fns sitofp-coerce).
    pub fn accepts(self, got: &Ty) -> bool {
        if matches!(got, Ty::Unknown) || self == SigTy::Any {
            return true;
        }
        match (self, got) {
            (
                SigTy::Bytes | SigTy::BytesMut,
                Ty::Prim(
                    PrimType::Bytes
                    | PrimType::BytesView
                    | PrimType::BytesMut,
                ),
            ) => true,
            (
                SigTy::Str,
                Ty::Prim(PrimType::String | PrimType::StringView),
            ) => true,
            (
                SigTy::Float,
                Ty::Prim(
                    PrimType::Float | PrimType::Int | PrimType::Uint,
                ),
            ) => true,
            _ => self.to_ty().assignable_from(got),
        }
    }
}

/// A function's signature: its row's `sig` column. `fallible`
/// carries the stdlib error type's NAME (users declare the shape
/// locally; resolve.rs's check_stdlib_error_shadowing validates it),
/// producing `Ty::Fallible { success: ret, payload: Named(name) }` so
/// `or` dispositions check the substitute/handler against the REAL
/// success type instead of Unknown.
#[derive(Debug, Clone, Copy)]
pub struct Sig {
    pub params: &'static [SigTy],
    pub ret: SigTy,
    pub fallible: Option<&'static str>,
}

/// Look up the signature for a full `std::...` path (segs including
/// the leading "std"). Any row answers, public or internal: an
/// internal row's signature still types a call the checker sees.
pub fn signature_for(segs: &[&str]) -> Option<&'static Sig> {
    row(segs).and_then(|f| f.sig.as_ref())
}

impl Sig {
    /// Type of a BARE (no `or`) call. A bare call of a fallible row is
    /// the bare-fallible law's error, and lowering has no bare form of
    /// one (F.40 phase 4, S5), so it types Unknown and the call reports
    /// that one error and no type mismatch, while `or` positions get the
    /// precise types via `or_types` (consulted by the Or arm).
    pub fn ret_ty(&self) -> Ty {
        match self.fallible {
            Some(_) => Ty::Unknown,
            None => self.ret.to_ty(),
        }
    }

    /// (success, payload) for `call() or ...` positions. None for
    /// non-fallible rows.
    pub fn or_types(&self) -> Option<(Ty, Ty)> {
        self.fallible.map(|err| {
            (self.ret.to_ty(), Ty::Named(err.to_string()))
        })
    }
}

/// R2/#265: one effect-class bitmask per stdlib fn — the leaf
/// lattice of the effect-assertion engine (`crate::callgraph`).
/// Const-constructible so the registry stays a static table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectSet(pub u64);

impl EffectSet {
    pub const PURE: EffectSet = EffectSet(0);
    pub const SYSCALL: EffectSet = EffectSet(1 << 0);
    pub const BLOCK: EffectSet = EffectSet(1 << 1);
    pub const PUBLISH: EffectSet = EffectSet(1 << 2);
    pub const TIME: EffectSet = EffectSet(1 << 3);
    pub const ENTROPY: EffectSet = EffectSet(1 << 4);
    pub const ENV: EffectSet = EffectSet(1 << 5);
    pub const ALLOC: EffectSet = EffectSet(1 << 6);
    /// GH #436 follow-up: privileged use of confined secret material.
    /// Bits 7-9 sit below `BUILTIN_BITS`, so this costs no user
    /// capacity.
    pub const SECRET_USE: EffectSet = EffectSet(1 << 7);
    /// Not yet classified (#265 step 4 turns the surface; until
    /// then queries must treat this as "may do anything").
    pub const UNCLASSIFIED: EffectSet = EffectSet(u64::MAX);

    pub const fn union(self, o: EffectSet) -> EffectSet {
        EffectSet(self.0 | o.0)
    }
    pub fn contains(self, o: EffectSet) -> bool {
        (self.0 & o.0) == o.0
    }
    pub fn is_unclassified(self) -> bool {
        self.0 == u64::MAX
    }
}

/// Who may call a stdlib function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// A name user code may call: [`unknown_fn_error`] accepts it,
    /// [`effects_for`] classifies it, [`suggest`] offers it, the
    /// catalogue documents it.
    Public,
    /// Called only by the stdlib's own seeds, and invisible to
    /// [`unknown_fn_error`] (a call from user code is an unknown
    /// function), [`effects_for`], [`suggest`] and the catalogue: the
    /// paths a dispatcher has an arm for and the surface never listed.
    /// An internal row carries [`EffectSet::UNCLASSIFIED`], which no
    /// query reads.
    Internal,
}

/// How a stdlib function lowers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lower {
    /// Lowered natively: a dispatcher has an arm for it.
    Intrinsic(IntrinsicId),
    /// A Hale body in the stdlib seeds that a dispatcher arm calls by
    /// name (`lower_user_fn_call("__md_to_html", ..)`). Not a rename: a
    /// rename enters the call graph, and the body's effects would then
    /// be inferred rather than read from the row.
    HaleBody(&'static str),
    /// An overload on the first argument: one Hale body per receiver
    /// type, each pair `(type name, body)` naming the receiver's mangled
    /// type (`__StdHttpRequest`) and the body that serves it. Lowering
    /// lowers the receiver once and calls the body its type picks.
    HaleBodyByReceiver(&'static [(&'static str, &'static str)]),
    /// Reached through `hale_stdlib::PATH_RENAMES` by the dispatchers'
    /// fallback.
    Renamed,
    /// No way to lower it today: lowering answers "not implemented".
    Unlowered,
}

/// One stdlib function: what it is, and how it lowers.
#[derive(Debug, Clone, Copy)]
pub struct StdFn {
    pub name: &'static str,
    pub visibility: Visibility,
    /// R2/#265: the leaf effect class the effect assertions query.
    pub effects: EffectSet,
    /// `None`: the function has no signature row; the checker types
    /// its call `Unknown`.
    pub sig: Option<Sig>,
    pub lower: Lower,
}

impl StdFn {
    pub fn is_public(&self) -> bool {
        self.visibility == Visibility::Public
    }
}

/// One namespace's accepted surface.
pub struct NsSurface {
    /// Path segments after `std` identifying the namespace
    /// (e.g. `["io", "fs"]` for `std::io::fs`). Longest match wins,
    /// so `std::io::fs` shadows a hypothetical `std::io` table for
    /// three-segment paths.
    pub ns: &'static [&'static str],
    /// The namespace's functions, public and internal. A reader of
    /// the user-facing surface walks [`NsSurface::public`].
    pub fns: &'static [StdFn],
    /// Prefixes the dispatch accepts open-endedly (rare). A name
    /// starting with one of these passes without being listed.
    pub open_prefixes: &'static [&'static str],
}

impl NsSurface {
    /// The functions user code may call, in table order.
    pub fn public(&self) -> impl Iterator<Item = &'static StdFn> {
        self.fns.iter().filter(|f| f.is_public())
    }
}

/// The row for a full `std::...` path (segs including the leading
/// "std"), public or internal.
pub fn row(segs: &[&str]) -> Option<&'static StdFn> {
    let (name, ns) = segs.split_last()?;
    if ns.first() != Some(&"std") {
        return None;
    }
    SURFACES.iter().find(|s| s.ns == &ns[1..])?.fns.iter().find(|f| f.name == *name)
}

/// Every row with its namespace, in table order.
pub fn rows() -> impl Iterator<Item = (&'static NsSurface, &'static StdFn)> {
    SURFACES.iter().flat_map(|s| s.fns.iter().map(move |f| (s, f)))
}

/// Locus/type paths that appear in path position but are NOT fn
/// calls (`std::io::file::File { ... }` etc.) — never flagged.
pub const LOCUS_PATHS: &[&[&str]] = &[
    &["std", "bus", "Adapter"],
    // R2 parity: the event-driven datagram ingest handle is a
    // LOCUS (`std::io::udp::Reader { addr, port, cap }`), not a
    // path-call — it was missing from this list, so the parity
    // check saw it as an unregistered lowered path.
    &["std", "io", "udp", "Reader"],
    &["std", "bytes", "BytesBuilder"],
    &["std", "cli", "Resolver"],
    &["std", "http", "Client"],
    &["std", "bus", "UnixTransport"],
    &["std", "http", "ClientRequest"],
    &["std", "http", "ClientResponse"],
    &["std", "http", "Context"],
    &["std", "http", "HttpError"],
    &["std", "http", "Url"],
    &["std", "http", "Handler"],
    &["std", "http", "Middleware"],
    &["std", "http", "NotFound404"],
    &["std", "http", "Request"],
    &["std", "http", "Response"],
    &["std", "http", "RouteEntry"],
    &["std", "http", "RouteHandler"],
    &["std", "http", "RouteParams"],
    &["std", "http", "Router"],
    &["std", "http", "Server"],
    &["std", "io", "MirrorRing"],
    &["std", "io", "file", "File"],
    &["std", "io", "tcp", "Listener"],
    &["std", "io", "tcp", "LogEvent"],
    &["std", "io", "tcp", "Stream"],
    &["std", "iter", "Lines"],
    &["std", "json", "ArrayIter"],
    &["std", "json", "ArrayIterSpan"],
    &["std", "json", "Builder"],
    &["std", "json", "JsonFieldRange"],
    &["std", "json", "JsonString"],
    &["std", "json", "ObjectIterSpan"],
    &["std", "lang", "Lang"],
    &["std", "lang", "Morpheme"],
    &["std", "log", "ConsoleSink"],
    &["std", "log", "FileSink"],
    &["std", "log", "LogEvent"],
    &["std", "log", "Logger"],
    &["std", "log", "StdoutSink"],
    &["std", "metrics", "Counter"],
    &["std", "metrics", "Endpoint"],
    &["std", "metrics", "Gauge"],
    &["std", "metrics", "Histogram"],
    &["std", "metrics", "HistogramData"],
    &["std", "metrics", "HistogramList"],
    &["std", "metrics", "Labels"],
    &["std", "metrics", "MetricEntry"],
    &["std", "metrics", "MetricMap"],
    &["std", "metrics", "Registry"],
    &["std", "name", "Convention"],
    &["std", "secret", "Credential"],
    &["std", "secret", "Signer"],
    &["std", "process", "Child"],
    &["std", "process", "ProcessOutput"],
    &["std", "source", "Walk"],
    &["std", "str", "ByteView"],
    &["std", "str", "ParseError"],
    &["std", "tagged", "Accumulator"],
    &["std", "term", "RawMode"],
    &["std", "term", "TermSize"],
    &["std", "text", "FileSink"],
    &["std", "text", "Sink"],
    &["std", "text", "StdoutSink"],
    &["std", "text", "StringSink"],
    &["std", "yaml", "Builder"],
    &["std", "yaml", "Reader"],
];

// The row constructors. A row reads
//
//     row!("parse_int", PURE, [Str] -> Int ! "ParseError", Intrinsic(StrParseInt)),
//
// — name, effect classes (`SYSCALL | BLOCK`), signature, lowering —
// and an internal one drops the effect column:
//
//     internal!("__note_pass", _, Intrinsic(TestNotePassRaw)),
//
// The signature is `_` when the function has none (its call types
// `Unknown`), else the parameters, the success type and, after `!`, the
// stdlib error type's name when the function is fallible. Its rows were
// filled from the per-function lowering verification (each lowering
// fn's arg-count checks + type coercions read directly, cross-checked
// against spec/stdlib.md); UNCERTAIN signatures are EXCLUDED, not
// guessed (M3 stage 2, 2026-07-02).
//
// GH #771: a type slot is a bare `SigTy` variant (`Str`, `Int`) OR
// `Named("__JsonString")` — the tuple variant, written the way the
// enum spells it. Both positions take both forms, so the rows for
// struct-returning helpers read like every other row:
//
//     row!("string_field", PURE, [Str, Str] -> Named("__JsonString"), HaleBody("__json_string_field")),
//
// The name is the MANGLED one, because that is the one the checker
// sees: `resolve_type_expr` puts a user's `std::json::JsonString`
// through `hale_stdlib::PATH_RENAMES` and the stdlib's own `type`
// declaration IS `__JsonString`, so the two meet at the mangled
// spelling and unify. (Diagnostics demangle it back — a wrong field
// reads `no field `knd` on `std::json::JsonString``.) A typo here
// would silently mean "some nominal type nobody declared", so
// `crates/hale-types/tests/stdlib_named_returns.rs` pins every
// `Named` name in this table against `PATH_RENAMES`.
//
// The unclassified-default row constructor `f(name)` used to live
// here. It is gone because nothing calls it: every public row is
// classified (#265 phase 2 finished the sweep). Its absence is a small
// enforcement — adding an unclassified row now means deliberately
// reintroducing a constructor for one.
macro_rules! row {
    ($name:literal, $($eff:ident)|+, $sig:tt $(-> $ret:ident $(($rn:literal))? $(! $err:literal)?)?, $($lower:tt)+) => {
        StdFn {
            name: $name,
            visibility: Visibility::Public,
            effects: EffectSet($(EffectSet::$eff.0)|+),
            sig: sig!($sig $(-> $ret $(($rn))? $(! $err)?)?),
            lower: lower!($($lower)+),
        }
    };
}

macro_rules! internal {
    ($name:literal, $sig:tt $(-> $ret:ident $(($rn:literal))? $(! $err:literal)?)?, $($lower:tt)+) => {
        StdFn {
            name: $name,
            visibility: Visibility::Internal,
            effects: EffectSet::UNCLASSIFIED,
            sig: sig!($sig $(-> $ret $(($rn))? $(! $err)?)?),
            lower: lower!($($lower)+),
        }
    };
}

macro_rules! sig {
    (_) => {
        None
    };
    ([$($p:ident $(($pn:literal))?),*] -> $ret:ident $(($rn:literal))? $(! $err:literal)?) => {
        Some(Sig {
            params: &[$(SigTy::$p $(($pn))?),*],
            ret: SigTy::$ret $(($rn))?,
            fallible: fallible!($($err)?),
        })
    };
}

macro_rules! fallible {
    () => {
        None
    };
    ($err:literal) => {
        Some($err)
    };
}

macro_rules! lower {
    (Intrinsic($id:ident)) => {
        Lower::Intrinsic(IntrinsicId::$id)
    };
    (HaleBody($body:literal)) => {
        Lower::HaleBody($body)
    };
    (HaleBodyByReceiver([$(($ty:literal, $body:literal)),+ $(,)?])) => {
        Lower::HaleBodyByReceiver(&[$(($ty, $body)),+])
    };
    (Renamed) => {
        Lower::Renamed
    };
    (Unlowered) => {
        Lower::Unlowered
    };
}

// Table policy: entries are the UNION of the codegen dispatch (the
// truth — mechanically extracted from the ["std", ...] slice
// patterns across codegen.rs, channels/mod.rs, and stdlib/*.rs)
// and spec/stdlib.md. Including a name the dispatch rejects is
// free (no typo detection for it); OMITTING a dispatched name
// causes a false compile error on valid code. Namespaces whose
// dispatch matches non-literally (std::io::sockopt constants,
// std::io::mirror, std::shm, std::ts) are deliberately NOT tabled
// — they keep the permissive Unknown behavior. Regenerate with
// the extraction described in notes/typecheck-m3.md stage 1.
//
// Every path a dispatcher has an arm for has a row here: those the
// surface never listed are `internal!` rows, after their namespace's
// public ones. A row's last column says how it lowers, and
// `crates/hale-codegen/tests/stdlib_registry_parity.rs` holds that
// column to the dispatchers and to `PATH_RENAMES`.
pub const SURFACES: &[NsSurface] = &[
    // #353: linear-time regex. Pure — the engine allocates nothing on
    // the match path beyond fixed state lists sized from the pattern,
    // so a match is countable against a budget.
    NsSurface {
        ns: &["regex"],
        fns: &[
            row!("matches", PURE, [Str, Str] -> Bool, Intrinsic(RegexMatches)),
            row!("find", PURE, [Str, Str] -> Int, Intrinsic(RegexFind)),
            row!("valid", PURE, [Str] -> Bool, Intrinsic(RegexValid)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["bus"],
        fns: &[
            row!("__local_dispatch", PUBLISH, [Str, Bytes] -> Int, Intrinsic(BusLocalDispatchRaw)),
            // GH #233: the unix transports' lifecycle primitives, called
            // by `__StdBusUnixConnectTransport` / `__StdBusUnixListenTransport`.
            internal!("__binding_fail", _, Intrinsic(BusBindingFailRaw)),
            internal!("__transport_realize", _, Intrinsic(BusTransportRealizeRaw)),
            internal!("__transport_reclaim", _, Intrinsic(BusTransportReclaimRaw)),
            internal!("__transport_spawn_server", _, Intrinsic(BusTransportSpawnServerRaw)),
        ],
        open_prefixes: &[],
    },
    // The `std::io::MirrorRing` locus's backing primitives. Internal
    // (`__`-prefixed, not a user-facing surface) but still frontier
    // LEAVES — `mirror_ring.hl` calls them, so an effect assertion
    // reaching a MirrorRing method reaches these. An unregistered
    // namespace is invisible to classification, which is the hole
    // this whole pass exists to close; "internal" is not a reason to
    // leave a leaf unclassified.
    NsSurface {
        ns: &["io", "mirror"],
        fns: &[
            // Double-mmap setup and teardown: mmap/munmap.
            // `__new` and `__recv_into` count their arguments. The other
            // seven have no signature (F.40 phase 4, S6): their helper
            // reads the arguments it needs without counting them and
            // ignores any more, so a signature would refuse calls lowering
            // builds.
            row!("__new", SYSCALL, [Int] -> Int, Intrinsic(IoMirrorNewRaw)),
            row!("__free", SYSCALL, _, Intrinsic(IoMirrorFreeRaw)),
            // Datagram read straight into the ring.
            row!("__recv_into", SYSCALL, [Int, Int, Int] -> Int, Intrinsic(IoMirrorRecvIntoRaw)),
            // Cursor arithmetic over an already-mapped region.
            row!("__commit", PURE, _, Intrinsic(IoMirrorCommitRaw)),
            row!("__consume", PURE, _, Intrinsic(IoMirrorConsumeRaw)),
            row!("__readable", PURE, _, Intrinsic(IoMirrorReadableRaw)),
            row!("__writable", PURE, _, Intrinsic(IoMirrorWritableRaw)),
            row!("__len", PURE, _, Intrinsic(IoMirrorLenRaw)),
            row!("__capacity", PURE, _, Intrinsic(IoMirrorCapacityRaw)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["bytes"],
        fns: &[
            row!("__is_alloc_fail", PURE, [Bytes] -> Int, Intrinsic(BytesIsAllocFailRaw)),
            // std::bytes — reads accept Bytes/BytesView/BytesMut; writes
            // require a BytesMut window (accepts() stays permissive on the
            // family, favoring no-false-error over full strictness).
            row!("at", PURE, [Bytes, Int] -> Int ! "IndexError", Intrinsic(BytesAt)),
            row!("clone", PURE, [Bytes] -> Bytes, Intrinsic(BytesClone)),
            row!("concat", PURE, [Bytes, Bytes] -> Bytes, Intrinsic(BytesConcat)),
            row!("find_byte", PURE, [Bytes, Int, Int] -> Int, Intrinsic(BytesFindByte)),
            row!("from_int", PURE, [Int] -> Bytes, Intrinsic(BytesFromInt)),
            row!("from_string", PURE, [Str] -> Bytes, Intrinsic(BytesFromString)),
            row!("read_f32_le", PURE, [Bytes, Int] -> Float ! "IndexError", Intrinsic(BytesReadF32Le)),
            row!("read_f64_be", PURE, [Bytes, Int] -> Float ! "IndexError", Intrinsic(BytesReadF64Be)),
            row!("read_f64_le", PURE, [Bytes, Int] -> Float ! "IndexError", Intrinsic(BytesReadF64Le)),
            row!("read_i16_be", PURE, [Bytes, Int] -> Int ! "IndexError", Intrinsic(BytesReadI16Be)),
            row!("read_i16_le", PURE, [Bytes, Int] -> Int ! "IndexError", Intrinsic(BytesReadI16Le)),
            row!("read_i32_be", PURE, [Bytes, Int] -> Int ! "IndexError", Intrinsic(BytesReadI32Be)),
            row!("read_i32_le", PURE, [Bytes, Int] -> Int ! "IndexError", Intrinsic(BytesReadI32Le)),
            row!("read_i64_be", PURE, [Bytes, Int] -> Int ! "IndexError", Intrinsic(BytesReadI64Be)),
            row!("read_i64_le", PURE, [Bytes, Int] -> Int ! "IndexError", Intrinsic(BytesReadI64Le)),
            row!("read_i8", PURE, [Bytes, Int] -> Int ! "IndexError", Intrinsic(BytesReadI8)),
            row!("read_u16_be", PURE, [Bytes, Int] -> Int ! "IndexError", Intrinsic(BytesReadU16Be)),
            row!("read_u16_le", PURE, [Bytes, Int] -> Int ! "IndexError", Intrinsic(BytesReadU16Le)),
            row!("read_u32_be", PURE, [Bytes, Int] -> Int ! "IndexError", Intrinsic(BytesReadU32Be)),
            row!("read_u32_le", PURE, [Bytes, Int] -> Int ! "IndexError", Intrinsic(BytesReadU32Le)),
            row!("read_u64_be", PURE, [Bytes, Int] -> Int ! "IndexError", Intrinsic(BytesReadU64Be)),
            row!("read_u64_le", PURE, [Bytes, Int] -> Int ! "IndexError", Intrinsic(BytesReadU64Le)),
            row!("read_u8", PURE, [Bytes, Int] -> Int ! "IndexError", Intrinsic(BytesReadU8)),
            row!("slice", PURE, [Bytes, Int, Int] -> Bytes, Intrinsic(BytesSlice)),
            row!("write_f32_le", PURE, [BytesMut, Int, Float] -> Int ! "IndexError", Intrinsic(BytesWriteF32Le)),
            row!("write_f64_be", PURE, [BytesMut, Int, Float] -> Int ! "IndexError", Intrinsic(BytesWriteF64Be)),
            row!("write_f64_le", PURE, [BytesMut, Int, Float] -> Int ! "IndexError", Intrinsic(BytesWriteF64Le)),
            row!("write_i16_be", PURE, [BytesMut, Int, Int] -> Int ! "IndexError", Intrinsic(BytesWriteI16Be)),
            row!("write_i16_le", PURE, [BytesMut, Int, Int] -> Int ! "IndexError", Intrinsic(BytesWriteI16Le)),
            row!("write_i32_be", PURE, [BytesMut, Int, Int] -> Int ! "IndexError", Intrinsic(BytesWriteI32Be)),
            row!("write_i32_le", PURE, [BytesMut, Int, Int] -> Int ! "IndexError", Intrinsic(BytesWriteI32Le)),
            row!("write_i64_be", PURE, [BytesMut, Int, Int] -> Int ! "IndexError", Intrinsic(BytesWriteI64Be)),
            row!("write_i64_le", PURE, [BytesMut, Int, Int] -> Int ! "IndexError", Intrinsic(BytesWriteI64Le)),
            row!("write_i8", PURE, [BytesMut, Int, Int] -> Int ! "IndexError", Intrinsic(BytesWriteI8)),
            row!("write_u16_be", PURE, [BytesMut, Int, Int] -> Int ! "IndexError", Intrinsic(BytesWriteU16Be)),
            row!("write_u16_le", PURE, [BytesMut, Int, Int] -> Int ! "IndexError", Intrinsic(BytesWriteU16Le)),
            row!("write_u32_be", PURE, [BytesMut, Int, Int] -> Int ! "IndexError", Intrinsic(BytesWriteU32Be)),
            row!("write_u32_le", PURE, [BytesMut, Int, Int] -> Int ! "IndexError", Intrinsic(BytesWriteU32Le)),
            row!("write_u64_be", PURE, [BytesMut, Int, Int] -> Int ! "IndexError", Intrinsic(BytesWriteU64Be)),
            row!("write_u64_le", PURE, [BytesMut, Int, Int] -> Int ! "IndexError", Intrinsic(BytesWriteU64Le)),
            row!("write_u8", PURE, [BytesMut, Int, Int] -> Int ! "IndexError", Intrinsic(BytesWriteU8)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["bytes", "builder"],
        fns: &[
            // The `BytesBuilder` locus's primitives (F.40 phase 4, S6: the
            // signatures their helpers enforce). The handle is the C
            // builder's pointer as an Int; `__text_view` and `__view`
            // return a StringView and a BytesView, which a signature cannot
            // state, so their success is `Any`. `__finish` and `__snapshot`
            // (`(Int) -> Bytes`) stay unsigned: the stdlib call fixture
            // prints their value, which the check refuses for a `Bytes`.
            row!("__append", PURE, [Int, Bytes] -> Int, Intrinsic(BytesBuilderAppendRaw)),
            row!("__append_f32", PURE, [Int, Float, Int] -> Int, Intrinsic(BytesBuilderAppendF32Raw)),
            row!("__append_f64", PURE, [Int, Float, Int] -> Int, Intrinsic(BytesBuilderAppendF64Raw)),
            row!("__append_pad", PURE, [Int, Int] -> Int, Intrinsic(BytesBuilderAppendPadRaw)),
            row!("__append_scalar", PURE, [Int, Int, Int, Int] -> Int, Intrinsic(BytesBuilderAppendScalarRaw)),
            row!("__append_slice", PURE, [Int, Bytes, Int, Int] -> Int, Intrinsic(BytesBuilderAppendSliceRaw)),
            row!("__append_str", PURE, [Int, Str] -> Int, Intrinsic(BytesBuilderAppendStrRaw)),
            row!("__clear", PURE, [Int] -> Int, Intrinsic(BytesBuilderClearRaw)),
            row!("__finish", PURE, _, Intrinsic(BytesBuilderFinishRaw)),
            row!("__free", PURE, [Int] -> Int, Intrinsic(BytesBuilderFreeRaw)),
            row!("__len", PURE, [Int] -> Int, Intrinsic(BytesBuilderLenRaw)),
            row!("__new", PURE, [Int] -> Int, Intrinsic(BytesBuilderNewRaw)),
            row!("__shift_front", PURE, [Int, Int] -> Int, Intrinsic(BytesBuilderShiftFrontRaw)),
            row!("__snapshot", PURE, _, Intrinsic(BytesBuilderSnapshotRaw)),
            row!("__text_view", PURE, [Int] -> Any, Intrinsic(BytesBuilderTextViewRaw)),
            row!("__view", PURE, [Int] -> Any, Intrinsic(BytesBuilderViewRaw)),
            row!("__xor_mask_into", PURE, [Int, Bytes, Int] -> Int, Intrinsic(BytesBuilderXorMaskIntoRaw)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["tar"],
        fns: &[
            // std::tar (GH #254): one-shot ustar over Bytes. Read side is
            // indexed (list-then-extract shape); write side is append-style
            // (start from empty Bytes, pack entries, finish appends the
            // terminating zero blocks).
            row!("entries", PURE, [Bytes] -> Int ! "IoError", Intrinsic(TarEntries)),
            row!("entry_data", PURE, [Bytes, Int] -> Bytes ! "IoError", Intrinsic(TarEntryData)),
            row!("entry_name", PURE, [Bytes, Int] -> Str ! "IoError", Intrinsic(TarEntryName)),
            row!("entry_size", PURE, [Bytes, Int] -> Int ! "IoError", Intrinsic(TarEntrySize)),
            row!("entry_type", PURE, [Bytes, Int] -> Str ! "IoError", Intrinsic(TarEntryType)),
            row!("finish", PURE, [Bytes] -> Bytes ! "IoError", Intrinsic(TarFinish)),
            row!("pack", PURE, [Bytes, Str, Bytes] -> Bytes ! "IoError", Intrinsic(TarPack)),
            row!("pack_dir", PURE, [Bytes, Str] -> Bytes ! "IoError", Intrinsic(TarPackDir)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["compress"],
        fns: &[
            row!("gunzip", PURE, [Bytes] -> Bytes ! "IoError", Intrinsic(CompressGunzip)),
            // std::compress (GH #254): one-shot Bytes -> Bytes. gzip pair
            // rides zlib (-lz, universal); zstd pair dlopens libzstd at
            // first use — on a machine without it the call fails
            // kind="not_found" rather than the program failing to link.
            row!("gzip", PURE, [Bytes] -> Bytes ! "IoError", Intrinsic(CompressGzip)),
            row!("unzstd", PURE, [Bytes] -> Bytes ! "IoError", Intrinsic(CompressUnzstd)),
            row!("zstd", PURE, [Bytes] -> Bytes ! "IoError", Intrinsic(CompressZstd)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["crypto"],
        fns: &[
            row!("crc32", PURE, [Bytes] -> Int, Intrinsic(CryptoCrc32)),
            // One mode, fallible (F.40 phase 4, S5): the bare form that
            // answered an empty Bytes on a bad key is gone.
            row!("ecdsa_p256_sign", PURE, [Bytes, Bytes] -> Bytes ! "CryptoError", Intrinsic(CryptoEcdsaP256Sign)),
            row!("ecdsa_p256_verify", PURE, [Bytes, Bytes, Bytes] -> Bool, Intrinsic(CryptoEcdsaP256Verify)),
            row!("hmac_sha256", PURE, [Bytes, Bytes] -> Bytes, Intrinsic(CryptoHmacSha256)),
            row!("hmac_sha512", PURE, [Bytes, Bytes] -> Bytes, Intrinsic(CryptoHmacSha512)),
            row!("sha1", PURE, [Bytes] -> Bytes, Intrinsic(CryptoSha1)),
            row!("sha256", PURE, [Bytes] -> Bytes, Intrinsic(CryptoSha256)),
            row!("sha512", PURE, [Bytes] -> Bytes, Intrinsic(CryptoSha512)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["ring"],
        fns: &[
            row!("__spsc_emit", SYSCALL, [Int, Int, Int, Int, Int] -> Unit, Intrinsic(RingSpscEmitRaw)),
            row!("__spsc_init", SYSCALL, [Int, Int, Int, Int] -> Unit, Intrinsic(RingSpscInitRaw)),
            row!("__spsc_note_drop", SYSCALL, [Int] -> Unit, Intrinsic(RingSpscNoteDropRaw)),
            row!("__spsc_read", SYSCALL, [Int, Int, Int, Int, Int, Int, Int] -> Int, Intrinsic(RingSpscReadRaw)),
            row!("__spsc_set_tag_b", SYSCALL, [Int, Int] -> Unit, Intrinsic(RingSpscSetTagBRaw)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["decimal"],
        fns: &[
            row!("format", PURE, [Decimal, Int] -> Str, Intrinsic(DecimalFormat)),
            row!("to_float", PURE, [Decimal] -> Float, Intrinsic(DecimalToFloat)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["diag"],
        fns: &[
            row!("heap_alloc_count", SYSCALL, [] -> Int, Intrinsic(DiagHeapAllocCount)),
            row!("syscall_count", SYSCALL, [Str] -> Int, Intrinsic(DiagSyscallCount)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["env"],
        fns: &[
            row!("arg", ENV, [Int] -> Str, Intrinsic(EnvArg)),
            row!("arg_or", ENV, [Int, Str] -> Str, Intrinsic(EnvArgOr)),
            row!("args_count", ENV, [] -> Int, Intrinsic(EnvArgsCount)),
            row!("var", ENV, [Str] -> Str, Intrinsic(EnvVar)),
            row!("var_exists", ENV, [Str] -> Bool, Intrinsic(EnvVarExists)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["http"],
        fns: &[
            row!("build_context", PURE, _, Renamed),
            row!("get", SYSCALL | BLOCK, _, Renamed),
            // The header of a Request or of a Response: the receiver's type
            // picks the body (F.40 phase 4, S5; an arm chose it from the
            // receiver's lowered type until then, lowering it twice).
            row!("header", PURE, _, HaleBodyByReceiver([
                ("__StdHttpRequest", "__http_request_header"),
                ("__StdHttpResponse", "__http_response_header"),
            ])),
            row!("is_route", PURE, _, Renamed),
            // GH #771: the one non-json member of the same class — also
            // dispatch-routed, also a struct return.
            row!("parse_request", PURE, [Str] -> Named("__StdHttpRequest"), HaleBody("__parse_http_request")),
            row!("parse_url", PURE, _, Renamed),
            row!("path_param", PURE, _, Renamed),
            row!("post", SYSCALL | BLOCK, _, Renamed),
            row!("query_param", PURE, _, Renamed),
            row!("request", SYSCALL | BLOCK, _, Renamed),
            row!("write_response", SYSCALL | BLOCK, _, HaleBody("__write_http_response")),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "file"],
        fns: &[
            row!("__at_eof", SYSCALL, [Int] -> Bool, Intrinsic(IoFileAtEofRaw)),
            row!("__close", SYSCALL, [Int] -> Int, Intrinsic(IoFileCloseRaw)),
            // The three primitives `file.hl`'s `File` wraps lower only under
            // an `or`, so their rows say they can fail and a bare call is
            // the checker's (F.40 phase 4, S5).
            row!("__open", SYSCALL, [Str, Str] -> Int ! "IoError", Intrinsic(IoFileOpenRaw)),
            row!("__read_line", SYSCALL | BLOCK, [Int] -> Str, Intrinsic(IoFileReadLineRaw)),
            row!("__seek", SYSCALL, [Int, Int] -> Unit ! "IoError", Intrinsic(IoFileSeekRaw)),
            row!("__write_bytes", SYSCALL, [Int, Bytes] -> Unit ! "IoError", Intrinsic(IoFileWriteBytesRaw)),
            row!("at_eof", SYSCALL, [Int] -> Bool, Renamed),
            row!("open", SYSCALL, [Str, Str] -> Int ! "IoError", Renamed),
            row!("read_line", SYSCALL | BLOCK, [Int] -> Str, Renamed),
            row!("seek", SYSCALL, [Int, Int] -> Unit ! "IoError", Renamed),
            row!("write_bytes", SYSCALL, [Int, Bytes] -> Unit ! "IoError", Renamed),
            row!("write_line", SYSCALL, _, Renamed),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "fs"],
        fns: &[
            row!("extension", SYSCALL, [Str] -> Str, Intrinsic(IoFsExtension)),
            row!("file_exists", SYSCALL, [Str] -> Bool, Intrinsic(IoFsFileExists)),
            row!("file_size", SYSCALL, [Str] -> Int ! "IoError", Intrinsic(IoFsFileSize)),
            row!("list_dir_at", SYSCALL, [Str, Int] -> Str ! "IoError", Intrinsic(IoFsListDirAt)),
            row!("list_dir_count", SYSCALL, [Str] -> Int ! "IoError", Intrinsic(IoFsListDirCount)),
            row!("mkdir", SYSCALL, [Str] -> Unit ! "IoError", Intrinsic(IoFsMkdir)),
            row!("mktemp", SYSCALL, [Str, Str] -> Str ! "IoError", Intrinsic(IoFsMktemp)),
            row!("read_bytes", SYSCALL, [Str] -> Bytes ! "IoError", Intrinsic(IoFsReadBytes)),
            // ── Tranche 2 (2026-07-02): the I/O namespaces. Verified the
            // same way (per-fn lowering read). EXCLUDED-not-guessed: all
            // std::json/std::http rows and process write_stdin/read_std*
            // (routed through Hale-stdlib __ fns — codegen never validates
            // their args, so there's no ground truth to table);
            // io::file::write_line (lowering ambiguous; the tcp timeout
            // setters, once here too, have their rows since F.40 phase 4,
            // S5); io::fs::list_dir (spec-only); the 7 spec'd
            // std::io::tls fns with NO lowering then (recv_stamped_into,
            // last_recv_*, set_*), all lowered and signed since (the last
            // two at F.40 phase 4, S6).
            // Handle args are plain Int FDs at the path-call level (the
            // File/Stream locus wrappers live in stdlib .hl seeds).
            row!("read_file", SYSCALL, [Str] -> Str ! "IoError", Intrinsic(IoFsReadFile)),
            row!("rename", SYSCALL, [Str, Str] -> Unit ! "IoError", Intrinsic(IoFsRename)),
            row!("unlink", SYSCALL, [Str] -> Unit ! "IoError", Intrinsic(IoFsUnlink)),
            row!("write_bytes", SYSCALL, [Str, Bytes] -> Unit ! "IoError", Intrinsic(IoFsWriteBytes)),
            row!("write_file", SYSCALL, [Str, Str] -> Unit ! "IoError", Intrinsic(IoFsWriteFile)),
            row!("__write_private", SYSCALL, [Str, Bytes] -> Unit ! "IoError", Intrinsic(IoFsWritePrivateRaw)),
            // GH #535 (DNA F.9): the `or` form lowers through the same
            // fallible channel as write_file (Unit success); a bare call
            // types Unknown like every bare fallible row (its Int-status
            // bare form left lowering at F.40 phase 4, S5). The row used to say Int and the checker
            // admitted `let n = ... or 0`, which codegen then refused with a
            // message about something else.
            row!("write_file_append", SYSCALL, [Str, Str] -> Unit ! "IoError", Intrinsic(IoFsWriteFileAppend)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "stdin"],
        fns: &[
            row!("read_byte", SYSCALL | BLOCK, [Int] -> Int, Intrinsic(IoStdinReadByte)),
            row!("read_line", SYSCALL | BLOCK, [] -> Str, Intrinsic(IoStdinReadLine)),
            row!("read_line_status", SYSCALL | BLOCK, [] -> Int, Intrinsic(IoStdinReadLineStatus)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "stdout"],
        fns: &[
            row!("write_bytes", SYSCALL, [Str] -> Int, Intrinsic(IoStdoutWriteBytes)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["api"],
        fns: &[
            // GH #1108: the local context a handler reached in-process gets.
            row!("local_context", ALLOC, [] -> Named("__StdApiContext"), HaleBody("__api_local_context")),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "unix"],
        fns: &[
            row!("connect", SYSCALL | BLOCK, [Str] -> Int ! "IoError", Intrinsic(IoUnixConnect)),
            row!("connect_wait", SYSCALL | BLOCK, [Str, Duration] -> Int ! "IoError", Intrinsic(IoUnixConnectWait)),
            row!("group_id", SYSCALL, [Str] -> Int, Intrinsic(IoUnixGroupId)),
            // GH #1106: AF_UNIX stream sockets; accept/recv/send/close are tcp's.
            row!("listen_socket", SYSCALL, [Str] -> Int ! "IoError", Intrinsic(IoUnixListenSocket)),
            row!("peer_gid", SYSCALL, [Int] -> Int, Intrinsic(IoUnixPeerGid)),
            row!("peer_group_at", SYSCALL, [Int, Int] -> Int, Intrinsic(IoUnixPeerGroupAt)),
            row!("peer_groups_count", SYSCALL, [Int] -> Int, Intrinsic(IoUnixPeerGroupsCount)),
            row!("peer_pid", SYSCALL, [Int] -> Int, Intrinsic(IoUnixPeerPid)),
            row!("peer_uid", SYSCALL, [Int] -> Int, Intrinsic(IoUnixPeerUid)),
            // GH #1109: the static role table's `user:` / `group:` spellings.
            row!("user_id", SYSCALL, [Str] -> Int, Intrinsic(IoUnixUserId)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "tcp"],
        fns: &[
            row!("__accept_one", SYSCALL | BLOCK, [Int] -> Int, Intrinsic(IoTcpAcceptOneRaw)),
            row!("__close_fd", SYSCALL, [Int] -> Int, Intrinsic(IoTcpCloseFdRaw)),
            row!("__connect", SYSCALL | BLOCK, [Str, Int] -> Int, Intrinsic(IoTcpConnectRaw)),
            row!("__io_error_kind", PURE, [Int] -> Str, Intrinsic(IoTcpIoErrorKindRaw)),
            row!("__last_io_status", PURE, [] -> Int, Intrinsic(IoTcpLastIoStatusRaw)),
            row!("__listen_socket", SYSCALL, [Str, Int] -> Int, Intrinsic(IoTcpListenSocketRaw)),
            row!("__recv", SYSCALL | BLOCK, [Int, Int] -> Str, Intrinsic(IoTcpRecvRaw)),
            row!("__recv_bytes", SYSCALL | BLOCK, [Int, Int] -> Bytes, Intrinsic(IoTcpRecvBytesRaw)),
            row!("__send", SYSCALL, [Int, Str] -> Int, Intrinsic(IoTcpSendRaw)),
            row!("__send_bytes", SYSCALL, [Int, Bytes] -> Int, Intrinsic(IoTcpSendBytesRaw)),
            row!("__set_recv_timeout_ns", SYSCALL, [Int, Int] -> Int, Intrinsic(IoTcpSetRecvTimeoutNsRaw)),
            row!("__shutdown_listen_socket", SYSCALL, [Int] -> Int, Intrinsic(IoTcpShutdownListenSocketRaw)),
            row!("accept_one", SYSCALL | BLOCK, [Int] -> Int ! "IoError", Intrinsic(IoTcpAcceptOne)),
            row!("close_fd", SYSCALL, [Int] -> Int, Intrinsic(IoTcpCloseFd)),
            row!("connect", SYSCALL | BLOCK, [Str, Int] -> Int ! "IoError", Intrinsic(IoTcpConnect)),
            row!("connect_wait", SYSCALL | BLOCK, [Str, Int, Duration] -> Int ! "IoError", Intrinsic(IoTcpConnectWait)),
            row!("last_recv_kernel_ns", PURE, [] -> Int, Intrinsic(IoTcpLastRecvKernelNs)),
            row!("last_recv_user_ns", PURE, [] -> Int, Intrinsic(IoTcpLastRecvUserNs)),
            row!("listen_socket", SYSCALL, [Str, Int] -> Int ! "IoError", Intrinsic(IoTcpListenSocket)),
            // GH #829: the `buf` slot is not polymorphic — the lowering
            // (`lower_recv_into_common`) accepts exactly one codegen type,
            // `LocusRef("__StdBytesBytesBuilder")`, and refuses everything
            // else with a spanless "buf must be std::bytes::BytesBuilder".
            // `Any` here made `recv_into(0, 0, 64)` a check-clean program
            // that does not build. Named by the mangled spelling, which is
            // what both `resolve_type_expr` (through `PATH_RENAMES`) and the
            // stdlib's own `locus __StdBytesBytesBuilder` produce.
            row!("recv_into", SYSCALL | BLOCK, [Int, Named("__StdBytesBytesBuilder"), Int] -> Int, Intrinsic(IoTcpRecvInto)),
            row!("recv_stamped_into", SYSCALL | BLOCK, [Int, Named("__StdBytesBytesBuilder"), Int] -> Int, Intrinsic(IoTcpRecvStampedInto)),
            row!("send_fd", SYSCALL, _, Renamed),
            row!("set_nodelay", SYSCALL, [Int, Bool] -> Unit ! "IoError", Intrinsic(IoTcpSetNodelay)),
            row!("set_recv_timeout", SYSCALL, [Int, Duration] -> Unit ! "IoError", Intrinsic(IoTcpSetRecvTimeout)),
            row!("set_rx_timestamps", SYSCALL, [Int, Bool] -> Unit ! "IoError", Intrinsic(IoTcpSetRxTimestamps)),
            row!("set_send_timeout", SYSCALL, [Int, Duration] -> Unit ! "IoError", Intrinsic(IoTcpSetSendTimeout)),
        ],
        open_prefixes: &[],
    },
    // The named platform constants (`std::io::sockopt::SO_REUSEADDR()`):
    // zero-argument getters, each a C function returning the platform's
    // number, so a program never hardcodes one. Codegen has always lowered
    // them (`SOCKOPT_NAMES`); this table is what the checker consults, and
    // without an entry the namespace was refused as unknown. Keep in step
    // with `SOCKOPT_NAMES` (a test in this crate's suite compares them).
    NsSurface {
        ns: &["io", "sockopt"],
        fns: &[
            row!("IPPROTO_IP", PURE, [] -> Int, Intrinsic(IoSockoptIpprotoIp)),
            row!("IPPROTO_IPV6", PURE, [] -> Int, Intrinsic(IoSockoptIpprotoIpv6)),
            row!("IPPROTO_TCP", PURE, [] -> Int, Intrinsic(IoSockoptIpprotoTcp)),
            row!("IPPROTO_UDP", PURE, [] -> Int, Intrinsic(IoSockoptIpprotoUdp)),
            row!("IP_ADD_MEMBERSHIP", PURE, [] -> Int, Intrinsic(IoSockoptIpAddMembership)),
            row!("IP_DROP_MEMBERSHIP", PURE, [] -> Int, Intrinsic(IoSockoptIpDropMembership)),
            row!("IP_MTU_DISCOVER", PURE, [] -> Int, Intrinsic(IoSockoptIpMtuDiscover)),
            row!("IP_MULTICAST_IF", PURE, [] -> Int, Intrinsic(IoSockoptIpMulticastIf)),
            row!("IP_MULTICAST_LOOP", PURE, [] -> Int, Intrinsic(IoSockoptIpMulticastLoop)),
            row!("IP_MULTICAST_TTL", PURE, [] -> Int, Intrinsic(IoSockoptIpMulticastTtl)),
            row!("IP_PKTINFO", PURE, [] -> Int, Intrinsic(IoSockoptIpPktinfo)),
            row!("IP_PMTUDISC_DO", PURE, [] -> Int, Intrinsic(IoSockoptIpPmtudiscDo)),
            row!("IP_PMTUDISC_DONT", PURE, [] -> Int, Intrinsic(IoSockoptIpPmtudiscDont)),
            row!("IP_PMTUDISC_PROBE", PURE, [] -> Int, Intrinsic(IoSockoptIpPmtudiscProbe)),
            row!("IP_PMTUDISC_WANT", PURE, [] -> Int, Intrinsic(IoSockoptIpPmtudiscWant)),
            row!("IP_TOS", PURE, [] -> Int, Intrinsic(IoSockoptIpTos)),
            row!("IP_TTL", PURE, [] -> Int, Intrinsic(IoSockoptIpTtl)),
            row!("SOL_SOCKET", PURE, [] -> Int, Intrinsic(IoSockoptSolSocket)),
            row!("SO_BINDTODEVICE", PURE, [] -> Int, Intrinsic(IoSockoptSoBindtodevice)),
            row!("SO_BROADCAST", PURE, [] -> Int, Intrinsic(IoSockoptSoBroadcast)),
            row!("SO_KEEPALIVE", PURE, [] -> Int, Intrinsic(IoSockoptSoKeepalive)),
            row!("SO_LINGER", PURE, [] -> Int, Intrinsic(IoSockoptSoLinger)),
            row!("SO_PRIORITY", PURE, [] -> Int, Intrinsic(IoSockoptSoPriority)),
            row!("SO_RCVBUF", PURE, [] -> Int, Intrinsic(IoSockoptSoRcvbuf)),
            row!("SO_RCVTIMEO", PURE, [] -> Int, Intrinsic(IoSockoptSoRcvtimeo)),
            row!("SO_REUSEADDR", PURE, [] -> Int, Intrinsic(IoSockoptSoReuseaddr)),
            row!("SO_REUSEPORT", PURE, [] -> Int, Intrinsic(IoSockoptSoReuseport)),
            row!("SO_SNDBUF", PURE, [] -> Int, Intrinsic(IoSockoptSoSndbuf)),
            row!("SO_SNDTIMEO", PURE, [] -> Int, Intrinsic(IoSockoptSoSndtimeo)),
            row!("TCP_NODELAY", PURE, [] -> Int, Intrinsic(IoSockoptTcpNodelay)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "tls"],
        fns: &[
            row!("close", SYSCALL, [Int] -> Int, Intrinsic(IoTlsClose)),
            row!("connect", SYSCALL | BLOCK, [Str, Int] -> Int ! "IoError", Intrinsic(IoTlsConnect)),
            row!("last_recv_kernel_ns", PURE, [] -> Int, Intrinsic(IoTlsLastRecvKernelNs)),
            row!("last_recv_user_ns", PURE, [] -> Int, Intrinsic(IoTlsLastRecvUserNs)),
            row!("recv_bytes", SYSCALL | BLOCK, [Int, Int] -> Bytes, Intrinsic(IoTlsRecvBytes)),
            row!("recv_into", SYSCALL | BLOCK, [Int, Named("__StdBytesBytesBuilder"), Int] -> Int, Intrinsic(IoTlsRecvInto)),
            // GH #829: `tls::recv_stamped_into` is dispatched by codegen
            // through the same `lower_recv_into_common` and is named by the
            // surface table, but had no signature row at all — so neither
            // its arity nor its buffer was checked. The family is only
            // closed if every member of it is.
            row!("recv_stamped_into", SYSCALL | BLOCK, [Int, Named("__StdBytesBytesBuilder"), Int] -> Int, Intrinsic(IoTlsRecvStampedInto)),
            row!("send_bytes", SYSCALL, [Int, Bytes] -> Int, Intrinsic(IoTlsSendBytes)),
            row!("set_nodelay", SYSCALL, [Int, Bool] -> Unit ! "IoError", Intrinsic(IoTlsSetNodelay)),
            row!("set_recv_timeout", SYSCALL, [Int, Duration] -> Unit ! "IoError", Intrinsic(IoTlsSetRecvTimeout)),
            row!("set_rx_timestamps", SYSCALL, [Int, Bool] -> Unit ! "IoError", Intrinsic(IoTlsSetRxTimestamps)),
            row!("set_send_timeout", SYSCALL, [Int, Duration] -> Unit ! "IoError", Intrinsic(IoTlsSetSendTimeout)),
            row!("upgrade", SYSCALL | BLOCK, [Int, Str, Bool] -> Int ! "IoError", Intrinsic(IoTlsUpgrade)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "udp"],
        fns: &[
            row!("__bind", SYSCALL, [Str, Int] -> Int ! "IoError", Intrinsic(IoUdpBindRaw)),
            row!("__close", SYSCALL, [Int] -> Int, Intrinsic(IoUdpCloseRaw)),
            row!("__recv", SYSCALL | BLOCK, [Int, Int] -> Bytes ! "IoError", Intrinsic(IoUdpRecvRaw)),
            row!("__send", SYSCALL, [Int, Str, Int, Str] -> Unit ! "IoError", Intrinsic(IoUdpSendRaw)),
            row!("bind", SYSCALL, [Str, Int] -> Int ! "IoError", Intrinsic(IoUdpBind)),
            row!("close", SYSCALL, [Int] -> Int, Intrinsic(IoUdpClose)),
            row!("get_option_int", SYSCALL, [Int, Int, Int] -> Int ! "IoError", Intrinsic(IoUdpGetOptionInt)),
            row!("join_group", SYSCALL, [Int, Str, Str] -> Unit ! "IoError", Intrinsic(IoUdpJoinGroup)),
            row!("last_source_host", PURE, [] -> Str, Intrinsic(IoUdpLastSourceHost)),
            row!("last_source_port", PURE, [] -> Int, Intrinsic(IoUdpLastSourcePort)),
            row!("leave_group", SYSCALL, [Int, Str, Str] -> Unit ! "IoError", Intrinsic(IoUdpLeaveGroup)),
            row!("recv", SYSCALL | BLOCK, [Int, Int] -> Bytes ! "IoError", Intrinsic(IoUdpRecv)),
            row!("recv_into", SYSCALL | BLOCK, [Int, Named("__StdBytesBytesBuilder"), Int] -> Int, Intrinsic(IoUdpRecvInto)),
            row!("recv_with_source", SYSCALL | BLOCK, [Int, Int] -> Bytes ! "IoError", Intrinsic(IoUdpRecvWithSource)),
            row!("send", SYSCALL, [Int, Str, Int, Str] -> Unit ! "IoError", Intrinsic(IoUdpSend)),
            row!("set_multicast_iface", SYSCALL, [Int, Str] -> Unit ! "IoError", Intrinsic(IoUdpSetMulticastIface)),
            row!("set_multicast_loop", SYSCALL, [Int, Any] -> Unit ! "IoError", Intrinsic(IoUdpSetMulticastLoop)),
            row!("set_multicast_ttl", SYSCALL, [Int, Int] -> Unit ! "IoError", Intrinsic(IoUdpSetMulticastTtl)),
            row!("set_option_bool", SYSCALL, [Int, Int, Int, Bool] -> Unit ! "IoError", Intrinsic(IoUdpSetOptionBool)),
            row!("set_option_int", SYSCALL, [Int, Int, Int, Int] -> Unit ! "IoError", Intrinsic(IoUdpSetOptionInt)),
            row!("set_recv_timeout", SYSCALL, [Int, Duration] -> Unit ! "IoError", Intrinsic(IoUdpSetRecvTimeout)),
            row!("set_send_timeout", SYSCALL, [Int, Duration] -> Unit ! "IoError", Intrinsic(IoUdpSetSendTimeout)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["json"],
        fns: &[
            row!("array_first", PURE, [Str] -> Named("__JsonArrayIter"), HaleBody("__json_array_first")),
            row!("array_first_span", PURE, [Str] -> Named("__JsonArrayIterSpan"), HaleBody("__json_array_first_span")),
            row!("array_next", PURE, [Named("__JsonArrayIter")] -> Named("__JsonArrayIter"), HaleBody("__json_array_next")),
            row!("array_next_span", PURE, [Named("__JsonArrayIterSpan"), Str] -> Named("__JsonArrayIterSpan"), HaleBody("__json_array_next_span")),
            row!("escape_string", PURE, [Str] -> Str, HaleBody("__json_escape_string")),
            row!("find_bool_field", PURE, [Str, Str] -> Bool, HaleBody("__json_find_bool_field")),
            row!("find_field_range_in", PURE, [Str, Str, Int, Int] -> Named("__JsonFieldRange"), HaleBody("__json_find_field_range_in")),
            row!("find_field_raw", PURE, [Str, Str] -> Str, HaleBody("__json_find_field_raw")),
            row!("find_field_raw_in", PURE, _, HaleBody("__json_find_field_raw_in")),
            row!("find_int_field", PURE, [Str, Str] -> Int, HaleBody("__json_find_int_field")),
            // GH #535 (DNA F.8): the flat-object json readers are Hale-source
            // stdlib fns with no rename entry, so a call typed Unknown and an
            // `or` on one slid through the checker to fail at build. Tabled,
            // they type precisely and an `or` is refused where it is written.
            row!("find_string_field", PURE, [Str, Str] -> Str, HaleBody("__json_find_string_field")),
            row!("iter_find_bool_field", PURE, _, HaleBody("__json_iter_find_bool_field")),
            row!("iter_find_field_range", PURE, [Named("__JsonArrayIterSpan"), Str, Str] -> Named("__JsonFieldRange"), HaleBody("__json_iter_find_field_range")),
            row!("iter_find_field_raw", PURE, _, HaleBody("__json_iter_find_field_raw")),
            row!("iter_find_int_field", PURE, _, HaleBody("__json_iter_find_int_field")),
            row!("iter_find_string_field", PURE, _, HaleBody("__json_iter_find_string_field")),
            row!("iter_find_string_field_range", PURE, [Named("__JsonArrayIterSpan"), Str, Str] -> Named("__JsonFieldRange"), HaleBody("__json_iter_find_string_field_range")),
            row!("iter_substring", PURE, _, HaleBody("__json_iter_substring")),
            row!("next_non_ws", PURE, _, Intrinsic(JsonNextNonWs)),
            row!("next_quote_or_bs", PURE, _, Intrinsic(JsonNextQuoteOrBs)),
            row!("next_struct_or_quote", PURE, _, Intrinsic(JsonNextStructOrQuote)),
            row!("obj_key_eq", PURE, _, HaleBody("__json_obj_key_eq")),
            row!("obj_key_len", PURE, _, HaleBody("__json_obj_key_len")),
            row!("obj_key_string", PURE, _, HaleBody("__json_obj_key_string")),
            row!("obj_value_bool", PURE, _, HaleBody("__json_obj_value_bool")),
            row!("obj_value_float", PURE, _, HaleBody("__json_obj_value_float")),
            row!("obj_value_int", PURE, _, HaleBody("__json_obj_value_int")),
            row!("obj_value_raw", PURE, _, HaleBody("__json_obj_value_raw")),
            row!("obj_value_string", PURE, _, HaleBody("__json_obj_value_string")),
            row!("object_first", PURE, [Str] -> Named("__JsonObjectIterSpan"), HaleBody("__json_obj_first_span")),
            row!("object_next", PURE, [Named("__JsonObjectIterSpan"), Str] -> Named("__JsonObjectIterSpan"), HaleBody("__json_obj_next_span")),
            // GH #719: the typed field read — a byte scan over the
            // caller's String, like every other reader here.
            // GH #771: the struct-returning half of the same surface. These
            // reach their `__json_*` implementations through a codegen
            // DISPATCH arm rather than a `PATH_RENAMES` entry, so the
            // "renames to a registered Hale-source fn" path in check.rs
            // (GH #470) never fired for them and the call typed `Unknown` —
            // which made `.knd` on a `JsonString`, and every arity or
            // fallibility question about anything reached through one, a
            // silent pass. Each return name is the `type` declared in
            // `crates/hale-stdlib/hl/json.hl`; the receiver params are named
            // too, so a swapped `(json, it)` argument pair is a located
            // error rather than a runtime field read at the wrong offset.
            row!("string_field", PURE, [Str, Str] -> Named("__JsonString"), HaleBody("__json_string_field")),
            row!("unescape_string", PURE, [Str] -> Str, HaleBody("__json_unescape_string")),
            // GH #754: syntax validation. Both are byte scans over
            // one immutable String with no allocation at all — the
            // uniqueness table is a fixed local array — so PURE is
            // the honest class, not ALLOC.
            // GH #754: one well-formed RFC 8259 value / a top-level object
            // with unique literal keys.
            row!("valid", PURE, [Str] -> Bool, HaleBody("__json_valid")),
            row!("valid_object", PURE, [Str] -> Bool, HaleBody("__json_valid_object")),
        ],
        open_prefixes: &[],
    },
    // GH #265 / R2 parity: `std::ts` (tree-sitter parsing) and
    // `std::shm` (shared-memory ring reads) are lowered by codegen
    // and called from stdlib `.hl`, but were absent from this table
    // — so they typed as `Ty::Unknown` (no arity/fallibility
    // checking) AND escaped effect classification entirely, which
    // would let a `@no_syscall` fn call them unchallenged. Parsing
    // and shm reads both touch the OS.
    NsSurface {
        ns: &["ts"],
        fns: &[
            row!("node_child", PURE, _, Intrinsic(TsNodeChild)),
            row!("node_child_count", PURE, _, Intrinsic(TsNodeChildCount)),
            row!("node_end_byte", PURE, _, Intrinsic(TsNodeEndByte)),
            row!("node_is_named", PURE, _, Intrinsic(TsNodeIsNamed)),
            row!("node_kind", PURE, _, Intrinsic(TsNodeKind)),
            row!("node_named_child", PURE, _, Intrinsic(TsNodeNamedChild)),
            row!("node_named_child_count", PURE, _, Intrinsic(TsNodeNamedChildCount)),
            row!("node_start_byte", PURE, _, Intrinsic(TsNodeStartByte)),
            row!("node_text", PURE, _, Intrinsic(TsNodeText)),
            row!("parse_go", ALLOC, _, Intrinsic(TsParseGo)),
            row!("root_node", PURE, _, Intrinsic(TsRootNode)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["shm"],
        fns: &[
            row!("last_record_kernel_ns", PURE, _, Intrinsic(ShmLastRecordKernelNs)),
            row!("last_record_seq", PURE, _, Intrinsic(ShmLastRecordSeq)),
            row!("last_record_user_ns", PURE, _, Intrinsic(ShmLastRecordUserNs)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["math"],
        fns: &[
            row!("acos", PURE, [Float] -> Float, Intrinsic(MathAcos)),
            row!("asin", PURE, [Float] -> Float, Intrinsic(MathAsin)),
            row!("atan", PURE, [Float] -> Float, Intrinsic(MathAtan)),
            row!("atan2", PURE, [Float, Float] -> Float, Intrinsic(MathAtan2)),
            row!("ceil", PURE, [Float] -> Float, Intrinsic(MathCeil)),
            row!("cos", PURE, [Float] -> Float, Intrinsic(MathCos)),
            row!("exp", PURE, [Float] -> Float, Intrinsic(MathExp)),
            row!("float_to_int", PURE, [Float] -> Int, Intrinsic(MathFloatToInt)),
            row!("floor", PURE, [Float] -> Float, Intrinsic(MathFloor)),
            row!("inf", PURE, [] -> Float, Intrinsic(MathInf)),
            row!("int_to_float", PURE, [Int] -> Float, Intrinsic(MathIntToFloat)),
            row!("is_nan", PURE, [Float] -> Bool, Intrinsic(MathIsNan)),
            row!("log", PURE, [Float] -> Float, Intrinsic(MathLog)),
            row!("nan", PURE, [] -> Float, Intrinsic(MathNan)),
            row!("pow", PURE, [Float, Float] -> Float, Intrinsic(MathPow)),
            row!("round", PURE, [Float] -> Int, Intrinsic(MathRound)),
            row!("sin", PURE, [Float] -> Float, Intrinsic(MathSin)),
            // std::math — unary/binary fns sitofp-coerce Int args.
            row!("sqrt", PURE, [Float] -> Float, Intrinsic(MathSqrt)),
            row!("tan", PURE, [Float] -> Float, Intrinsic(MathTan)),
            row!("tanh", PURE, [Float] -> Float, Intrinsic(MathTanh)),
            row!("trunc", PURE, [Float] -> Int, Intrinsic(MathTrunc)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["os"],
        fns: &[
            row!("getrandom", SYSCALL | ENTROPY, [Int] -> Bytes ! "IoError", Intrinsic(OsGetrandom)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["process"],
        fns: &[
            // The primitives `process.hl` wraps lower only under an `or`, so
            // their rows say they can fail (F.40 phase 4, S5). The handles
            // `__spawn` and the two waits return (`__StdProcessSpawnHandle`,
            // `__StdProcessWaitOutcome`) have no public spelling, so their
            // success is `Any`, as `run`'s is.
            row!("__kill_escalate", SYSCALL, [Int] -> Unit ! "IoError", Intrinsic(ProcessKillEscalateRaw)),
            row!("__pipe_read", SYSCALL | BLOCK, [Int] -> Str ! "IoError", Intrinsic(ProcessPipeReadRaw)),
            row!("__pipe_write", SYSCALL, [Int, Str] -> Int ! "IoError", Intrinsic(ProcessPipeWriteRaw)),
            row!("__signal_pid", SYSCALL, [Int, Int] -> Unit ! "IoError", Intrinsic(ProcessSignalPidRaw)),
            row!("__spawn", SYSCALL, [Str] -> Any ! "IoError", Intrinsic(ProcessSpawnRaw)),
            row!("__try_wait_pid", SYSCALL, [Int] -> Any ! "IoError", Intrinsic(ProcessTryWaitPidRaw)),
            row!("__wait_pid", SYSCALL | BLOCK, [Int] -> Any ! "IoError", Intrinsic(ProcessWaitPidRaw)),
            // GH #716: adopt closes the outgoing handle's fds and
            // TERM/KILL-reaps its process, so it carries the same
            // syscall class as kill — not PURE, despite reading like
            // an assignment.
            // Reached through its rename like the other `process.hl`
            // wrappers: a statement calls a body that returns nothing (F.40
            // phase 4, S5; a hand-kept statement branch until then).
            row!("adopt", SYSCALL, _, Renamed),
            row!("dump_arena_residency", SYSCALL, [] -> Int, Intrinsic(ProcessDumpArenaResidency)),
            row!("dump_pool_residency", SYSCALL, [] -> Int, Intrinsic(ProcessDumpPoolResidency)),
            row!("exit", SYSCALL, [Int] -> Unit, Intrinsic(ProcessExit)),
            row!("kill", SYSCALL, [Int] -> Unit ! "IoError", Renamed),
            row!("pid", SYSCALL, [] -> Int, Intrinsic(ProcessPid)),
            row!("uid", SYSCALL, [] -> Int, Intrinsic(ProcessUid)),
            row!("read_stderr", SYSCALL | BLOCK, _, Renamed),
            row!("read_stdout", SYSCALL | BLOCK, _, Renamed),
            row!("rss_bytes", SYSCALL, [] -> Int, Intrinsic(ProcessRssBytes)),
            // std::process child management — success types are internal
            // handles (__StdProcessSpawnHandle etc.); Any keeps arity +
            // arg + fallible checking without naming them.
            row!("run", SYSCALL | BLOCK, [Str] -> Any ! "IoError", Intrinsic(ProcessRun)),
            row!("signal", SYSCALL, _, Renamed),
            row!("spawn", SYSCALL, [Str] -> Any ! "IoError", Renamed),
            row!("try_wait", SYSCALL, _, Renamed),
            row!("wait", SYSCALL | BLOCK, [Int] -> Any ! "IoError", Renamed),
            row!("write_stdin", SYSCALL, _, Renamed),
        ],
        open_prefixes: &[],
    },
    // GH #469 B: `std::log::kv(k, v)` renders one structured field.
    // Pure — it is string formatting; the I/O happens in the sink,
    // which is an ordinary bus subscriber.
    NsSurface {
        ns: &["log"],
        fns: &[
            // GH #469 B: one structured log field, logfmt-quoted.
            row!("kv", PURE, [Str, Str] -> Str, Renamed),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["metrics"],
        fns: &[
            row!("counter", PURE, _, Renamed),
            row!("gauge", PURE, _, Renamed),
            row!("histogram", PURE, _, Renamed),
            row!("labels_append", PURE, _, Renamed),
            row!("labels_empty", PURE, _, Renamed),
            row!("labels_one", PURE, _, Renamed),
            row!("labels_two", PURE, _, Renamed),
            row!("metric_key", PURE, _, Renamed),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["rand"],
        fns: &[
            row!("next_int", ENTROPY, [Int] -> Int, Intrinsic(RandNextInt)),
            row!("seed_from_time", TIME | ENTROPY, [] -> Unit, Intrinsic(RandSeedFromTime)),
        ],
        open_prefixes: &[],
    },
    // GH #989: the vault: source's two free-fn escapes — `Credential`
    // and `Signer` are LOCUS paths (tracked as such, not here) with
    // no registry row of their own, so their methods carry no entry;
    // these are the first `std::secret` free fns.
    NsSurface {
        ns: &["secret"],
        fns: &[
            row!("vault_local_dir", ENV, _, Renamed),
            row!("vault_name_is_safe", PURE, _, Renamed),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["str"],
        fns: &[
            row!("builder_append", PURE, _, Intrinsic(StrBuilderAppend)),
            row!("builder_finish", PURE, _, Intrinsic(StrBuilderFinish)),
            row!("builder_len", PURE, _, Intrinsic(StrBuilderLen)),
            row!("builder_new", PURE, _, Intrinsic(StrBuilderNew)),
            row!("byte_at_unchecked", PURE, [Str, Int] -> Int, Intrinsic(StrByteAtUnchecked)),
            row!("can_parse_float", PURE, [Str] -> Bool, Intrinsic(StrCanParseFloat)),
            row!("can_parse_int", PURE, [Str] -> Bool, Intrinsic(StrCanParseInt)),
            row!("clone", PURE, [Str] -> Str, Intrinsic(StrClone)),
            row!("from_bytes", PURE, [Bytes] -> Str, Intrinsic(StrFromBytes)),
            // GH #720 — ByteView byte scanning. All three are pure:
            // `bytes_view` is one strlen, `byte_at` a bounds-checked
            // load, `slice` / `range_copy` an arena copy of a range
            // the caller already bounded.
            row!("byte_at", PURE, [Named("__StrByteView"), Int] -> Int, Renamed),
            // GH #720 — the ByteView surface. The view is the struct
            // `str_view.hl` declares, named here by the mangled name
            // PATH_RENAMES maps `std::str::ByteView` onto, so a view built
            // by `bytes_view` and a view annotated by hand are the same
            // type to the checker. `range_copy` is the strlen-free
            // substring `slice` is built on — public because a scanner
            // that already tracks its own bounds (the JSON range walkers)
            // wants it directly, and named `range_*` because it carries
            // that family's caller-owns-the-bounds contract.
            row!("bytes_view", PURE, [Str] -> Named("__StrByteView"), Renamed),
            row!("range_copy", PURE, [Str, Int, Int, Int] -> Str, Intrinsic(StrRangeCopy)),
            row!("slice", PURE, [Named("__StrByteView"), Int, Int] -> Str, Renamed),
            row!("index_of", PURE, [Str, Str] -> Int, Intrinsic(StrIndexOf)),
            row!("contains", PURE, [Str, Str] -> Bool, Intrinsic(StrContains)),
            // #353: the everyday predicates. The runtime carried
            // `lotus_str_contains` / `_starts_with` all along; `ends_with` is
            // new. Pure reads over immutable data — no effects.
            row!("split_into", PURE, [Str, Str, Any] -> Unit, Intrinsic(StrSplitInto)),
            row!("join", PURE, [Any, Str] -> Str, Intrinsic(StrJoin)),
            row!("cp_count", PURE, [Str] -> Int, Intrinsic(StrCpCount)),
            row!("cp_at", PURE, [Str, Int] -> Int, Intrinsic(StrCpAt)),
            row!("cp_size", PURE, [Str, Int] -> Int, Intrinsic(StrCpSize)),
            row!("starts_with", PURE, [Str, Str] -> Bool, Intrinsic(StrStartsWith)),
            row!("ends_with", PURE, [Str, Str] -> Bool, Intrinsic(StrEndsWith)),
            row!("lower", PURE, [Str] -> Str, Intrinsic(StrLower)),
            row!("pad_left", PURE, [Str, Int, Str] -> Str, Intrinsic(StrPadLeft)),
            row!("pad_right", PURE, [Str, Int, Str] -> Str, Intrinsic(StrPadRight)),
            row!("parse_decimal", PURE, [Str] -> Decimal ! "ParseError", Intrinsic(StrParseDecimal)),
            row!("parse_float", PURE, [Str] -> Float ! "ParseError", Intrinsic(StrParseFloat)),
            row!("parse_int", PURE, [Str] -> Int ! "ParseError", Intrinsic(StrParseInt)),
            row!("range_eq", PURE, [Str, Int, Int, Str] -> Bool, Intrinsic(StrRangeEq)),
            row!("range_parse_decimal", PURE, [Str, Int, Int] -> Decimal ! "ParseError", Intrinsic(StrRangeParseDecimal)),
            row!("range_parse_int", PURE, [Str, Int, Int] -> Int ! "ParseError", Intrinsic(StrRangeParseInt)),
            row!("repeat", PURE, [Str, Int] -> Str, Intrinsic(StrRepeat)),
            row!("replace", PURE, [Str, Str, Str] -> Str, Intrinsic(StrReplace)),
            row!("substring", PURE, [Str, Int, Int] -> Str, Intrinsic(StrSubstring)),
            row!("trim", PURE, [Str] -> Str, Intrinsic(StrTrim)),
            row!("upper", PURE, [Str] -> Str, Intrinsic(StrUpper)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["term"],
        fns: &[
            row!("__raw_disable", SYSCALL, _, Intrinsic(TermRawDisableRaw)),
            row!("__raw_enable", SYSCALL, _, Intrinsic(TermRawEnableRaw)),
            row!("__size_packed", SYSCALL, _, Intrinsic(TermSizePackedRaw)),
            row!("is_tty", SYSCALL, [Int] -> Bool, Intrinsic(TermIsTty)),
            row!("size", SYSCALL, _, Renamed),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["test"],
        fns: &[
            row!("assert", PURE, _, HaleBody("__test_assert")),
            row!("assert_eq_int", PURE, _, HaleBody("__test_assert_eq_int")),
            row!("assert_eq_str", PURE, _, HaleBody("__test_assert_eq_str")),
            // GH #230 / #717: the pass counter and the recorded-failure
            // latch, called by the `__test_assert*` bodies and
            // `__test_fail_trailer`.
            internal!("__failed", _, Intrinsic(TestFailedRaw)),
            internal!("__note_fail", _, Intrinsic(TestNoteFailRaw)),
            internal!("__note_pass", _, Intrinsic(TestNotePassRaw)),
            internal!("__passes", _, Intrinsic(TestPassesRaw)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["text"],
        fns: &[
            row!("is_alnum", PURE, [Int] -> Bool, Intrinsic(TextIsAlnum)),
            // std::text byte-class predicates + tokenizer (vec target is a
            // user @form(vec) locus — Any).
            row!("is_alpha", PURE, [Int] -> Bool, Intrinsic(TextIsAlpha)),
            row!("is_digit", PURE, [Int] -> Bool, Intrinsic(TextIsDigit)),
            row!("is_whitespace", PURE, [Int] -> Bool, Intrinsic(TextIsWhitespace)),
            row!("is_word_char", PURE, [Int] -> Bool, Intrinsic(TextIsWordChar)),
            row!("md_to_html", PURE, _, HaleBody("__md_to_html")),
            row!("tokenize_words_into", PURE, [Str, Any] -> Unit, Intrinsic(TextTokenizeWordsInto)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["text", "base64"],
        fns: &[
            row!("decode", PURE, [Str] -> Bytes, Intrinsic(TextBase64Decode)),
            row!("encode", PURE, [Bytes] -> Str, Intrinsic(TextBase64Encode)),
            row!("url_encode", PURE, [Bytes] -> Str, Intrinsic(TextBase64UrlEncode)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["time"],
        fns: &[
            // #353: the inverse of `time_from_unix`, which already yields
            // ISO-8601 text. Returns unix seconds. UTC only, and PURE — it
            // reads no clock and no TZ.
            row!("parse_iso8601", PURE, [Str] -> Int ! "ParseError", Intrinsic(TimeParseIso8601)),
            row!("can_parse_iso8601", PURE, [Str] -> Bool, Intrinsic(TimeCanParseIso8601)),
            // GH #607: Time is a value. `current()` reads the wall clock as an
            // instant (`now()` stays epoch seconds as Int);
            // `iso8601` / `parse_time` round-trip through text; `unix` and
            // `nanos` / `from_nanos` are the integer views.
            row!("current", TIME, [] -> Time, Intrinsic(TimeCurrent)),
            row!("iso8601", PURE, [Time] -> Str, Intrinsic(TimeIso8601)),
            row!("parse_time", PURE, [Str] -> Time ! "ParseError", Intrinsic(TimeParseTime)),
            row!("unix", PURE, [Time] -> Int, Intrinsic(TimeUnix)),
            row!("nanos", PURE, [Time] -> Int, Intrinsic(TimeNanos)),
            row!("from_nanos", PURE, [Int] -> Time, Intrinsic(TimeFromNanos)),
            // std::time — sleep takes Duration (Int rejected in lowering);
            // now() is epoch SECONDS as Int; time_from_unix returns Time.
            row!("monotonic", TIME, [] -> Duration, Intrinsic(TimeMonotonic)),
            row!("monotonic_ns", TIME, [] -> Int, Intrinsic(TimeMonotonicNs)),
            row!("now", TIME, [] -> Int, Intrinsic(TimeNow)),
            row!("sleep", SYSCALL | BLOCK | TIME, [Duration] -> Unit, Intrinsic(TimeSleep)),
            row!("time_from_unix", PURE, [Int] -> Time, Intrinsic(TimeTimeFromUnix)),
        ],
        open_prefixes: &[],
    },
];

/// One id per natively lowered stdlib function (a row whose lowering is
/// [`Lower::Intrinsic`]), in table order. The name is mechanical and
/// injective: the path's segments after `std` in CamelCase, a leading
/// `__` rendered as a trailing `Raw` (`std::io::tcp::__connect` is
/// `IoTcpConnectRaw`). `stdlib_registry_parity` holds every row's id to that
/// rendering of its path, and this file's tests no two rows to one id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntrinsicId {
    RegexMatches,
    RegexFind,
    RegexValid,
    BusLocalDispatchRaw,
    BusBindingFailRaw,
    BusTransportRealizeRaw,
    BusTransportReclaimRaw,
    BusTransportSpawnServerRaw,
    IoMirrorNewRaw,
    IoMirrorFreeRaw,
    IoMirrorRecvIntoRaw,
    IoMirrorCommitRaw,
    IoMirrorConsumeRaw,
    IoMirrorReadableRaw,
    IoMirrorWritableRaw,
    IoMirrorLenRaw,
    IoMirrorCapacityRaw,
    BytesIsAllocFailRaw,
    BytesAt,
    BytesClone,
    BytesConcat,
    BytesFindByte,
    BytesFromInt,
    BytesFromString,
    BytesReadF32Le,
    BytesReadF64Be,
    BytesReadF64Le,
    BytesReadI16Be,
    BytesReadI16Le,
    BytesReadI32Be,
    BytesReadI32Le,
    BytesReadI64Be,
    BytesReadI64Le,
    BytesReadI8,
    BytesReadU16Be,
    BytesReadU16Le,
    BytesReadU32Be,
    BytesReadU32Le,
    BytesReadU64Be,
    BytesReadU64Le,
    BytesReadU8,
    BytesSlice,
    BytesWriteF32Le,
    BytesWriteF64Be,
    BytesWriteF64Le,
    BytesWriteI16Be,
    BytesWriteI16Le,
    BytesWriteI32Be,
    BytesWriteI32Le,
    BytesWriteI64Be,
    BytesWriteI64Le,
    BytesWriteI8,
    BytesWriteU16Be,
    BytesWriteU16Le,
    BytesWriteU32Be,
    BytesWriteU32Le,
    BytesWriteU64Be,
    BytesWriteU64Le,
    BytesWriteU8,
    BytesBuilderAppendRaw,
    BytesBuilderAppendF32Raw,
    BytesBuilderAppendF64Raw,
    BytesBuilderAppendPadRaw,
    BytesBuilderAppendScalarRaw,
    BytesBuilderAppendSliceRaw,
    BytesBuilderAppendStrRaw,
    BytesBuilderClearRaw,
    BytesBuilderFinishRaw,
    BytesBuilderFreeRaw,
    BytesBuilderLenRaw,
    BytesBuilderNewRaw,
    BytesBuilderShiftFrontRaw,
    BytesBuilderSnapshotRaw,
    BytesBuilderTextViewRaw,
    BytesBuilderViewRaw,
    BytesBuilderXorMaskIntoRaw,
    TarEntries,
    TarEntryData,
    TarEntryName,
    TarEntrySize,
    TarEntryType,
    TarFinish,
    TarPack,
    TarPackDir,
    CompressGunzip,
    CompressGzip,
    CompressUnzstd,
    CompressZstd,
    CryptoCrc32,
    CryptoEcdsaP256Sign,
    CryptoEcdsaP256Verify,
    CryptoHmacSha256,
    CryptoHmacSha512,
    CryptoSha1,
    CryptoSha256,
    CryptoSha512,
    RingSpscEmitRaw,
    RingSpscInitRaw,
    RingSpscNoteDropRaw,
    RingSpscReadRaw,
    RingSpscSetTagBRaw,
    DecimalFormat,
    DecimalToFloat,
    DiagHeapAllocCount,
    DiagSyscallCount,
    EnvArg,
    EnvArgOr,
    EnvArgsCount,
    EnvVar,
    EnvVarExists,
    IoFileAtEofRaw,
    IoFileCloseRaw,
    IoFileOpenRaw,
    IoFileReadLineRaw,
    IoFileSeekRaw,
    IoFileWriteBytesRaw,
    IoFsExtension,
    IoFsFileExists,
    IoFsFileSize,
    IoFsListDirAt,
    IoFsListDirCount,
    IoFsMkdir,
    IoFsMktemp,
    IoFsReadBytes,
    IoFsReadFile,
    IoFsRename,
    IoFsUnlink,
    IoFsWriteBytes,
    IoFsWriteFile,
    IoFsWritePrivateRaw,
    IoFsWriteFileAppend,
    IoStdinReadByte,
    IoStdinReadLine,
    IoStdinReadLineStatus,
    IoStdoutWriteBytes,
    IoUnixConnect,
    IoUnixConnectWait,
    IoUnixGroupId,
    IoUnixListenSocket,
    IoUnixPeerGid,
    IoUnixPeerGroupAt,
    IoUnixPeerGroupsCount,
    IoUnixPeerPid,
    IoUnixPeerUid,
    IoUnixUserId,
    IoTcpAcceptOneRaw,
    IoTcpCloseFdRaw,
    IoTcpConnectRaw,
    IoTcpIoErrorKindRaw,
    IoTcpLastIoStatusRaw,
    IoTcpListenSocketRaw,
    IoTcpRecvRaw,
    IoTcpRecvBytesRaw,
    IoTcpSendRaw,
    IoTcpSendBytesRaw,
    IoTcpSetRecvTimeoutNsRaw,
    IoTcpShutdownListenSocketRaw,
    IoTcpAcceptOne,
    IoTcpCloseFd,
    IoTcpConnect,
    IoTcpConnectWait,
    IoTcpLastRecvKernelNs,
    IoTcpLastRecvUserNs,
    IoTcpListenSocket,
    IoTcpRecvInto,
    IoTcpRecvStampedInto,
    IoTcpSetNodelay,
    IoTcpSetRecvTimeout,
    IoTcpSetRxTimestamps,
    IoTcpSetSendTimeout,
    IoSockoptIpprotoIp,
    IoSockoptIpprotoIpv6,
    IoSockoptIpprotoTcp,
    IoSockoptIpprotoUdp,
    IoSockoptIpAddMembership,
    IoSockoptIpDropMembership,
    IoSockoptIpMtuDiscover,
    IoSockoptIpMulticastIf,
    IoSockoptIpMulticastLoop,
    IoSockoptIpMulticastTtl,
    IoSockoptIpPktinfo,
    IoSockoptIpPmtudiscDo,
    IoSockoptIpPmtudiscDont,
    IoSockoptIpPmtudiscProbe,
    IoSockoptIpPmtudiscWant,
    IoSockoptIpTos,
    IoSockoptIpTtl,
    IoSockoptSolSocket,
    IoSockoptSoBindtodevice,
    IoSockoptSoBroadcast,
    IoSockoptSoKeepalive,
    IoSockoptSoLinger,
    IoSockoptSoPriority,
    IoSockoptSoRcvbuf,
    IoSockoptSoRcvtimeo,
    IoSockoptSoReuseaddr,
    IoSockoptSoReuseport,
    IoSockoptSoSndbuf,
    IoSockoptSoSndtimeo,
    IoSockoptTcpNodelay,
    IoTlsClose,
    IoTlsConnect,
    IoTlsLastRecvKernelNs,
    IoTlsLastRecvUserNs,
    IoTlsRecvBytes,
    IoTlsRecvInto,
    IoTlsRecvStampedInto,
    IoTlsSendBytes,
    IoTlsSetNodelay,
    IoTlsSetRecvTimeout,
    IoTlsSetRxTimestamps,
    IoTlsSetSendTimeout,
    IoTlsUpgrade,
    IoUdpBindRaw,
    IoUdpCloseRaw,
    IoUdpRecvRaw,
    IoUdpSendRaw,
    IoUdpBind,
    IoUdpClose,
    IoUdpGetOptionInt,
    IoUdpJoinGroup,
    IoUdpLastSourceHost,
    IoUdpLastSourcePort,
    IoUdpLeaveGroup,
    IoUdpRecv,
    IoUdpRecvInto,
    IoUdpRecvWithSource,
    IoUdpSend,
    IoUdpSetMulticastIface,
    IoUdpSetMulticastLoop,
    IoUdpSetMulticastTtl,
    IoUdpSetOptionBool,
    IoUdpSetOptionInt,
    IoUdpSetRecvTimeout,
    IoUdpSetSendTimeout,
    JsonNextNonWs,
    JsonNextQuoteOrBs,
    JsonNextStructOrQuote,
    TsNodeChild,
    TsNodeChildCount,
    TsNodeEndByte,
    TsNodeIsNamed,
    TsNodeKind,
    TsNodeNamedChild,
    TsNodeNamedChildCount,
    TsNodeStartByte,
    TsNodeText,
    TsParseGo,
    TsRootNode,
    ShmLastRecordKernelNs,
    ShmLastRecordSeq,
    ShmLastRecordUserNs,
    MathAcos,
    MathAsin,
    MathAtan,
    MathAtan2,
    MathCeil,
    MathCos,
    MathExp,
    MathFloatToInt,
    MathFloor,
    MathInf,
    MathIntToFloat,
    MathIsNan,
    MathLog,
    MathNan,
    MathPow,
    MathRound,
    MathSin,
    MathSqrt,
    MathTan,
    MathTanh,
    MathTrunc,
    OsGetrandom,
    ProcessKillEscalateRaw,
    ProcessPipeReadRaw,
    ProcessPipeWriteRaw,
    ProcessSignalPidRaw,
    ProcessSpawnRaw,
    ProcessTryWaitPidRaw,
    ProcessWaitPidRaw,
    ProcessDumpArenaResidency,
    ProcessDumpPoolResidency,
    ProcessExit,
    ProcessPid,
    ProcessUid,
    ProcessRssBytes,
    ProcessRun,
    RandNextInt,
    RandSeedFromTime,
    StrBuilderAppend,
    StrBuilderFinish,
    StrBuilderLen,
    StrBuilderNew,
    StrByteAtUnchecked,
    StrCanParseFloat,
    StrCanParseInt,
    StrClone,
    StrFromBytes,
    StrRangeCopy,
    StrIndexOf,
    StrContains,
    StrSplitInto,
    StrJoin,
    StrCpCount,
    StrCpAt,
    StrCpSize,
    StrStartsWith,
    StrEndsWith,
    StrLower,
    StrPadLeft,
    StrPadRight,
    StrParseDecimal,
    StrParseFloat,
    StrParseInt,
    StrRangeEq,
    StrRangeParseDecimal,
    StrRangeParseInt,
    StrRepeat,
    StrReplace,
    StrSubstring,
    StrTrim,
    StrUpper,
    TermRawDisableRaw,
    TermRawEnableRaw,
    TermSizePackedRaw,
    TermIsTty,
    TestFailedRaw,
    TestNoteFailRaw,
    TestNotePassRaw,
    TestPassesRaw,
    TextIsAlnum,
    TextIsAlpha,
    TextIsDigit,
    TextIsWhitespace,
    TextIsWordChar,
    TextTokenizeWordsInto,
    TextBase64Decode,
    TextBase64Encode,
    TextBase64UrlEncode,
    TimeParseIso8601,
    TimeCanParseIso8601,
    TimeCurrent,
    TimeIso8601,
    TimeParseTime,
    TimeUnix,
    TimeNanos,
    TimeFromNanos,
    TimeMonotonic,
    TimeMonotonicNs,
    TimeNow,
    TimeSleep,
    TimeTimeFromUnix,
}

/// Longest-prefix namespace lookup for a full `std::...` path
/// (segs INCLUDING the leading "std"). Returns the surface and the
/// index of the fn-name segment.
pub fn lookup(segs: &[&str]) -> Option<(&'static NsSurface, usize)> {
    if segs.first() != Some(&"std") {
        return None;
    }
    let mut best: Option<(&'static NsSurface, usize)> = None;
    for s in SURFACES {
        let want = s.ns.len();
        // Path must be exactly ns + one fn segment.
        if segs.len() == want + 2 && segs[1..=want] == *s.ns {
            match best {
                Some((b, _)) if b.ns.len() >= want => {}
                _ => best = Some((s, want + 1)),
            }
        }
    }
    best
}

/// True iff the full path names a known stdlib locus/type (never a
/// fn typo).
pub fn is_locus_path(segs: &[&str]) -> bool {
    LOCUS_PATHS.iter().any(|p| *p == segs)
}

/// Nearest tabled namespace, for the did-you-mean on an unknown one.
fn nearest_namespace(segs: &[&str]) -> Option<String> {
    let want = segs.join("::");
    let mut best: Option<(String, usize)> = None;
    for s in SURFACES {
        let cand = s.ns.join("::");
        let d = edit_distance(&want, &cand);
        if d * 2 <= cand.len().max(want.len()) {
            match &best {
                Some((_, bd)) if *bd <= d => {}
                _ => best = Some((cand, d)),
            }
        }
    }
    best.map(|(n, _)| n)
}

/// Nearest known name within the namespace, for the did-you-mean
/// hint. Only offered when the edit distance is small relative to
/// the name length (a distance-2 match on a 3-char name is noise).
pub fn suggest(surface: &NsSurface, name: &str) -> Option<&'static str> {
    let mut best: Option<(&'static str, usize)> = None;
    for entry in surface.public() {
        let cand = &entry.name;
        let d = edit_distance(name, cand);
        match best {
            Some((_, bd)) if bd <= d => {}
            _ => best = Some((cand, d)),
        }
    }
    match best {
        Some((cand, d)) if d <= 2 && name.len() >= 4 => Some(cand),
        Some((cand, 1)) => Some(cand),
        _ => None,
    }
}

/// GH #241: generic nearest-name suggestion for user-scope
/// diagnostics (unknown field/method/type names), same threshold
/// policy as the stdlib `suggest` above: distance ≤ 2 on names of
/// length ≥ 4, or distance 1 on anything.
pub fn nearest_name<'a, I>(name: &str, candidates: I) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut best: Option<(&'a str, usize)> = None;
    for cand in candidates {
        let d = edit_distance(name, cand);
        match best {
            Some((_, bd)) if bd <= d => {}
            _ => best = Some((cand, d)),
        }
    }
    match best {
        Some((cand, d)) if d <= 2 && name.len() >= 4 => {
            Some(cand.to_string())
        }
        Some((cand, 1)) => Some(cand.to_string()),
        _ => None,
    }
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1)
                .min(cur[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

// GH #722: the advice strings. One per operation, shared by every
// spelling of it, so a new row cannot invent a call shape — the
// tests pin this set and typecheck each shape.
const LEN_OF_STRING: &str =
    "the length of a String is the builtin `len(s)`";
const LEN_OF_BYTES: &str = "the length of a Bytes is the builtin `len(b)`";
const LEN_OF_ARRAY: &str = "the length of an array is the builtin `len(a)`";
const ABS_OF: &str = "the absolute value of a number is the builtin `abs(x)`";
const MIN_OF: &str = "the smaller of two numbers is the builtin `min(a, b)`";
const MAX_OF: &str = "the larger of two numbers is the builtin `max(a, b)`";
const PRINT_TO: &str = "writing to stdout is the builtin `print(x)`";
const PRINTLN_TO: &str =
    "writing a line to stdout is the builtin `println(x)`";
const RENDER_AS: &str =
    "rendering a value as a String is the builtin `to_string(x)`";

/// GH #722: the conventional spellings — the `std::` path an author
/// arriving from Rust / Go / Python / Java reaches for — of
/// operations Hale answers with a BARE BUILTIN rather than a stdlib
/// fn. Edit distance cannot bridge a namespace→builtin move, and
/// left to itself it actively misleads: `std::string::len` drew
/// "did you mean `std::ring`?" and `std::math::min` drew "did you
/// mean `std::math::sin`?".
///
/// Every row's advice is a real builtin call shape at the arity the
/// codegen dispatch enforces (`lower_len_builtin`,
/// `lower_to_string_builtin` and `lower_math_builtin`: one arg for
/// `len`/`to_string`/`abs`, two for `min`/`max`; the printers are
/// variadic). Operations with NO builtin equivalent are deliberately
/// absent — `std::json::parse`, `std::str::concat` and
/// `std::str::to_lower` keep the plain unknown-fn diagnostic rather
/// than gain a suggestion that is not a valid call.
pub const BUILTIN_SPELLINGS: &[(&[&str], &str)] = &[
    (&["std", "bytes", "len"], LEN_OF_BYTES),
    (&["std", "cmp", "max"], MAX_OF),
    (&["std", "cmp", "min"], MIN_OF),
    (&["std", "fmt", "print"], PRINT_TO),
    (&["std", "fmt", "println"], PRINTLN_TO),
    (&["std", "io", "print"], PRINT_TO),
    (&["std", "io", "println"], PRINTLN_TO),
    (&["std", "io", "stdout", "print"], PRINT_TO),
    (&["std", "io", "stdout", "println"], PRINTLN_TO),
    (&["std", "math", "abs"], ABS_OF),
    (&["std", "math", "max"], MAX_OF),
    (&["std", "math", "min"], MIN_OF),
    (&["std", "str", "from_int"], RENDER_AS),
    (&["std", "str", "len"], LEN_OF_STRING),
    (&["std", "str", "length"], LEN_OF_STRING),
    (&["std", "str", "to_string"], RENDER_AS),
    (&["std", "string", "len"], LEN_OF_STRING),
    (&["std", "string", "length"], LEN_OF_STRING),
    (&["std", "vec", "len"], LEN_OF_ARRAY),
];

/// GH #722: the table lookup, consulted before either did-you-mean.
/// `None` for every path that is not a tabled spelling, so unrelated
/// unknown functions keep their existing diagnostic.
fn builtin_spelling_error(segs: &[&str]) -> Option<String> {
    let advice = BUILTIN_SPELLINGS
        .iter()
        .find(|(path, _)| *path == segs)
        .map(|(_, advice)| *advice)?;
    Some(format!(
        "`{}` is not a stdlib function; {}",
        segs.join("::"),
        advice
    ))
}

/// GH #722, member half: `s.len()` / `s.length` is the same
/// namespace→builtin move in member spelling. Returns the advice to
/// append to an existing "no field" error, for the primitives whose
/// length the `len` builtin actually answers — a `@form` collection
/// has a real `.len()` method and never reaches the error site.
pub fn builtin_member_advice(recv: &Ty, field: &str) -> Option<&'static str> {
    if !matches!(field, "len" | "length" | "size") {
        return None;
    }
    match recv {
        Ty::Prim(PrimType::String) | Ty::Prim(PrimType::StringView) => {
            Some(LEN_OF_STRING)
        }
        Ty::Prim(PrimType::Bytes) | Ty::Prim(PrimType::BytesView) => {
            Some(LEN_OF_BYTES)
        }
        _ => None,
    }
}

/// The stage-1 check: for a call whose callee is a `std::` path,
/// return an error message when the namespace is tabled and the fn
/// name is unknown. `None` means "fine or not our business".
pub fn unknown_fn_error(segs: &[&str]) -> Option<String> {
    if is_locus_path(segs) {
        return None;
    }
    // GH #722: a conventional spelling of a builtin is answered by
    // the explicit table, before either generic did-you-mean.
    if let Some(msg) = builtin_spelling_error(segs) {
        return Some(msg);
    }
    // #353 item 9: an UNTABLED namespace used to short-circuit here —
    // `lookup` returned None and the call was waved through as "not our
    // business". So `std::totally::fake()` passed `hale check` and only
    // codegen rejected it, which meant a typo'd or imagined stdlib call
    // was invisible to the checker, to the CI check gate and to the
    // LSP. `std::` is a closed namespace; an unknown one is an error.
    if lookup(segs).is_none() && segs.first() == Some(&"std") && segs.len() >= 2
    {
        let ns = segs[..segs.len() - 1].join("::");
        let known = nearest_namespace(&segs[1..segs.len() - 1]);
        let hint = match known {
            Some(k) => format!(" — did you mean `std::{}`?", k),
            None => String::new(),
        };
        return Some(format!("unknown stdlib namespace `{}`{}", ns, hint));
    }
    let (surface, fn_idx) = lookup(segs)?;
    let name = segs[fn_idx];
    if surface.public().any(|e| e.name == name) {
        return None;
    }
    if surface
        .open_prefixes
        .iter()
        .any(|p| name.starts_with(p))
    {
        return None;
    }
    let ns_path = format!("std::{}", surface.ns.join("::"));
    let hint = match suggest(surface, name) {
        Some(s) => format!(" — did you mean `{}::{}`?", ns_path, s),
        None => String::new(),
    };
    Some(format!(
        "unknown stdlib function `{}::{}`{}",
        ns_path, name, hint
    ))
}

/// R2/#265: the effect classification for a fully-qualified stdlib
/// path (`["std", ns.., fn]`), or None when the path isn't in the
/// registry. UNCLASSIFIED entries are exactly that — the caller
/// must treat them as may-do-anything until #265 classifies the
/// surface.
/// Language BUILTINS that carry effects. These are not `std::` paths
/// — they are bare idents the parser knows — so they sit outside
/// `SURFACES` and were invisible to the frontier: a `@no_syscall` fn
/// could `println` freely while the violation diagnostic for
/// `std::io::fs::*` described the syscall class as covering "stdio".
/// The surface contradicted itself; writing to a stream is a
/// `write(2)`, it can block, and a hot-path certificate that permits
/// it is not certifying what it claims.
pub fn builtin_effects(name: &str) -> Option<EffectSet> {
    match name {
        "println" | "print" | "eprintln" | "eprint" => {
            Some(EffectSet::SYSCALL)
        }
        _ => None,
    }
}

pub fn effects_for(segs: &[&str]) -> Option<EffectSet> {
    if segs.len() == 1 {
        return builtin_effects(segs[0]);
    }
    row(segs).filter(|f| f.is_public()).map(|f| f.effects)
}

/// GH #791: the `block`-classified stdlib leaves that **park** on a
/// `where async_io` cooperative pool instead of holding its worker.
///
/// The `block` classification in [`SURFACES`] is a property of the
/// CALL — "this waits" — and stays placement-independent: the same
/// `sleep` on a classic pool really does hold that pool's OS thread.
/// What depends on placement is whether waiting *stalls anyone
/// else*. On an async_io pool these leaves swap the coro out and the
/// worker goes on draining, so the placement-implied advisory
/// (`effects::placement_implied_diags`) must not count them — it
/// used to, which made every async handler that sleeps carry a
/// warning whose suggested fix (`@no_block`) is a compile error on
/// the correct program.
///
/// Enumerated from the park lowering, not guessed. Each entry's
/// lowering reaches a runtime primitive that calls
/// `lotus_coop_park_on_fd{,_deadline}` or
/// `lotus_time_sleep_park_try` when `lotus_io_on_async_io_pool()`,
/// and has no blocking fallback on that path:
///
/// | path | runtime primitive |
/// |---|---|
/// | `std::time::sleep` | `lotus_time_sleep_park_try` (timer-only park, PR #285) |
/// | `std::io::tcp::accept_one`, `__accept_one` | `lotus_tcp_accept_one` |
/// | `std::io::tcp::__recv` | `lotus_tcp_recv_str` |
/// | `std::io::tcp::__recv_bytes` | `lotus_tcp_recv_bytes` |
/// | `std::io::tcp::recv_into` | `lotus_tcp_recv_into` |
/// | `std::io::tcp::recv_stamped_into` | `lotus_tcp_recv_stamped` |
/// | `std::io::udp::recv`, `__recv`, `recv_with_source` | `lotus_udp_recvfrom_async` |
/// | `std::io::udp::recv_into` | `lotus_udp_recv_into` |
/// | `std::io::tls::recv_into` | `lotus_tls_recv_into` |
/// | `std::io::tls::recv_stamped_into` | `lotus_tls_recv_stamped_into` |
///
/// Deliberately ABSENT, and each for a reason read off the runtime:
/// `tcp::connect` / `__connect` (`lotus_tcp_connect` — a blocking
/// `connect(2)`, no park path), `tls::connect` / `upgrade` (blocking
/// handshake), `tls::recv_bytes` (a plain `SSL_read`; only the
/// `recv_into` family got the async_io park), `io::file` /
/// `io::stdin` reads (regular files and the tty are not epoll-park
/// targets here), `std::process::{run, wait, read_stdout,
/// read_stderr}` and `std::http::*` (which reaches `connect`).
/// These still stall the worker, so they still warn.
pub const ASYNC_IO_PARKING: &[&[&str]] = &[
    &["std", "io", "tcp", "__accept_one"],
    &["std", "io", "tcp", "__recv"],
    &["std", "io", "tcp", "__recv_bytes"],
    &["std", "io", "tcp", "accept_one"],
    &["std", "io", "tcp", "recv_into"],
    &["std", "io", "tcp", "recv_stamped_into"],
    &["std", "io", "tls", "recv_into"],
    &["std", "io", "tls", "recv_stamped_into"],
    &["std", "io", "udp", "__recv"],
    &["std", "io", "udp", "recv"],
    &["std", "io", "udp", "recv_into"],
    &["std", "io", "udp", "recv_with_source"],
    &["std", "time", "sleep"],
];

/// Does this stdlib path park (rather than hold the worker) when it
/// runs on a `where async_io` cooperative pool? See
/// [`ASYNC_IO_PARKING`].
pub fn parks_on_async_io(segs: &[&str]) -> bool {
    ASYNC_IO_PARKING.iter().any(|p| *p == segs)
}

/// GH #830: the `block`-classified leaves that, on a CLASSIC (non-
/// `async_io`) cooperative pool, do **not** hold the pool's worker
/// for the whole wait — so blocking there stalls nobody and is not
/// a finding for `check_cooperative_pool_blocking`.
///
/// This is a different subtraction from [`ASYNC_IO_PARKING`], and
/// deliberately so. The park list is the *async_io-pool* rule: those
/// leaves swap the coro out, which only happens when the pool has an
/// event loop. `check_cooperative_pool_blocking` never looks at an
/// async_io pool — the placement walk `continue`s on the `where
/// async_io` constraint before it reads a single call — so on every
/// placement it *does* look at, a parking leaf takes its blocking
/// path and really does hold the thread. Subtracting the park list
/// there would delete `tcp::recv_into`, `tls::recv_into`,
/// `udp::recv` and five more from the lint, which is the whole
/// shape the lint exists to catch.
///
/// One entry, read off the lowering rather than guessed:
///
/// | path | why it yields |
/// |---|---|
/// | `std::time::sleep` | `lower_time_sleep` chunks the sleep into ≤100ms slices and drains the pool's bus queue between them, so a sleeping locus keeps the queue serviced ~10×/s |
///
/// That slicing is what makes "handlers plus a `time::sleep` loop"
/// the *prescribed* event-driven shape — the shape both blocking
/// diagnostics name as the fix — so counting `sleep` as a stall
/// would have the lint flag its own advice.
pub const COOPERATIVE_YIELDING_BLOCK_LEAVES: &[&[&str]] =
    &[&["std", "time", "sleep"]];

/// Does this stdlib path hold a classic cooperative pool's OS thread
/// for the duration of its wait? The registry's `block` rows minus
/// [`COOPERATIVE_YIELDING_BLOCK_LEAVES`] — one classification, not a
/// second hand list (GH #830).
pub fn holds_cooperative_worker(segs: &[&str]) -> bool {
    effects_for(segs).is_some_and(|e| e.contains(EffectSet::BLOCK))
        && !COOPERATIVE_YIELDING_BLOCK_LEAVES.iter().any(|p| *p == segs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn path(s: &NsSurface, f: &StdFn) -> String {
        format!("std::{}::{}", s.ns.join("::"), f.name)
    }

    #[test]
    fn one_row_per_path_and_one_surface_per_namespace() {
        let mut seen = BTreeMap::new();
        for s in SURFACES {
            assert!(seen.insert(s.ns.join("::"), ()).is_none(), "two surfaces for std::{}", s.ns.join("::"));
        }
        let mut rows_seen = BTreeMap::new();
        for (s, f) in rows() {
            assert!(rows_seen.insert(path(s, f), ()).is_none(), "two rows for {}", path(s, f));
        }
    }

    /// No two rows share an intrinsic id. (That each id is its path's
    /// mechanical name is held by `stdlib_registry_parity`.)
    #[test]
    fn no_two_rows_share_an_intrinsic_id() {
        let mut ids = BTreeMap::new();
        for (s, f) in rows() {
            if let Lower::Intrinsic(id) = f.lower {
                if let Some(other) = ids.insert(id, path(s, f)) {
                    panic!("{other} and {} share an intrinsic id", path(s, f));
                }
            }
        }
        assert!(ids.len() > 300, "only {} intrinsic rows", ids.len());
    }

    /// A public row is classified (the frontier stays true); an internal
    /// row carries `UNCLASSIFIED`, which no query reads.
    #[test]
    fn a_rows_effects_are_classified_exactly_when_it_is_public() {
        for (s, f) in rows() {
            assert_eq!(
                f.effects.is_unclassified(),
                !f.is_public(),
                "{} is {:?} with effects {:?}",
                path(s, f),
                f.visibility,
                f.effects
            );
        }
    }

    /// Each namespace lists its public rows before its internal ones,
    /// and holds at least one public row (an all-internal namespace
    /// would make `lookup` find a namespace user code cannot name).
    #[test]
    fn internal_rows_follow_their_namespaces_public_ones() {
        for s in SURFACES {
            let first_internal = s.fns.iter().position(|f| !f.is_public()).unwrap_or(s.fns.len());
            assert!(first_internal > 0, "std::{} has no public row", s.ns.join("::"));
            assert!(
                s.fns[first_internal..].iter().all(|f| !f.is_public()),
                "std::{} lists a public row after an internal one",
                s.ns.join("::")
            );
        }
    }
}
