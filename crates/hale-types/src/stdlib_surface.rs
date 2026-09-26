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

use hale_syntax::ast::PrimType;

use crate::ty::Ty;

/// M3 stage 2 (2026-07-02): const-constructible type vocabulary for
/// the signature table. Maps to `Ty` at check time. `Any` types as
/// Unknown — bidirectionally assignable — for the rare polymorphic
/// arg; use it rather than guessing.
#[derive(Clone, Copy, PartialEq, Eq)]
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

/// One signature row. `fallible` carries the stdlib error type's
/// NAME (users declare the shape locally; resolve.rs's
/// check_stdlib_error_shadowing validates it), producing
/// `Ty::Fallible { success: ret, payload: Named(name) }` so `or`
/// dispositions check the substitute/handler against the REAL
/// success type instead of Unknown.
pub struct FnSig {
    pub ns: &'static [&'static str],
    pub name: &'static str,
    pub params: &'static [SigTy],
    pub ret: SigTy,
    pub fallible: Option<&'static str>,
}

/// Look up the signature for a full `std::...` path (segs including
/// the leading "std").
pub fn signature_for(segs: &[&str]) -> Option<&'static FnSig> {
    if segs.first() != Some(&"std") {
        return None;
    }
    SIGS.iter().find(|s| {
        segs.len() == s.ns.len() + 2
            && segs[1..=s.ns.len()] == *s.ns
            && segs[s.ns.len() + 1] == s.name
    })
}

impl FnSig {
    /// Type of a BARE (no `or`) call. Stdlib fallible path-calls
    /// are dual-mode at codegen: with `or` they take the fallible
    /// ABI; without, they're the legacy direct form whose return
    /// differs per fn (read_file → the String, write_file → an Int
    /// status). We don't model the legacy zoo — bare fallible calls
    /// stay Unknown (the status quo), while `or` positions get the
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

    pub fn display_path(&self) -> String {
        format!("std::{}::{}", self.ns.join("::"), self.name)
    }
}

/// One namespace's accepted surface.
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

/// One registry row: the fn name plus its effect classification.
#[derive(Debug, Clone, Copy)]
pub struct FnEntry {
    pub name: &'static str,
    pub effects: EffectSet,
}

// The unclassified-default row constructor `f(name)` used to live
// here. It is gone because nothing calls it: every registry row below
// is classified (#265 phase 2 finished the sweep). Its absence is a
// small enforcement — adding an unclassified row now means
// deliberately reintroducing a constructor for one, rather than
// reaching for the one already sitting in the file.

/// #265 phase 2: a CLASSIFIED registry row. Every effectful and
/// pure stdlib fn carries its leaf effect set; `f(..)` remains for
/// the residue still awaiting classification (an assertion treats
/// those conservatively as may-do-anything).
const fn e(name: &'static str, effects: EffectSet) -> FnEntry {
    FnEntry { name, effects }
}

pub struct NsSurface {
    /// Path segments after `std` identifying the namespace
    /// (e.g. `["io", "fs"]` for `std::io::fs`). Longest match wins,
    /// so `std::io::fs` shadows a hypothetical `std::io` table for
    /// three-segment paths.
    pub ns: &'static [&'static str],
    /// Accepted functions within the namespace (name + effect
    /// classification).
    pub fns: &'static [FnEntry],
    /// Prefixes the dispatch accepts open-endedly (rare). A name
    /// starting with one of these passes without being listed.
    pub open_prefixes: &'static [&'static str],
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
pub const SURFACES: &[NsSurface] = &[
    // #353: linear-time regex. Pure — the engine allocates nothing on
    // the match path beyond fixed state lists sized from the pattern,
    // so a match is countable against a budget.
    NsSurface {
        ns: &["regex"],
        fns: &[
            e("matches", EffectSet::PURE),
            e("find", EffectSet::PURE),
            e("valid", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["bus"],
        fns: &[
            e("__local_dispatch", EffectSet::PUBLISH),
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
            e("__new", EffectSet::SYSCALL),
            e("__free", EffectSet::SYSCALL),
            // Datagram read straight into the ring.
            e("__recv_into", EffectSet::SYSCALL),
            // Cursor arithmetic over an already-mapped region.
            e("__commit", EffectSet::PURE),
            e("__consume", EffectSet::PURE),
            e("__readable", EffectSet::PURE),
            e("__writable", EffectSet::PURE),
            e("__len", EffectSet::PURE),
            e("__capacity", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["bytes"],
        fns: &[
            e("__is_alloc_fail", EffectSet::PURE), e("at", EffectSet::PURE), e("clone", EffectSet::PURE), e("concat", EffectSet::PURE), e("find_byte", EffectSet::PURE),
            e("from_int", EffectSet::PURE), e("from_string", EffectSet::PURE), e("read_f32_le", EffectSet::PURE), e("read_f64_be", EffectSet::PURE),
            e("read_f64_le", EffectSet::PURE), e("read_i16_be", EffectSet::PURE), e("read_i16_le", EffectSet::PURE), e("read_i32_be", EffectSet::PURE),
            e("read_i32_le", EffectSet::PURE), e("read_i64_be", EffectSet::PURE), e("read_i64_le", EffectSet::PURE), e("read_i8", EffectSet::PURE),
            e("read_u16_be", EffectSet::PURE), e("read_u16_le", EffectSet::PURE), e("read_u32_be", EffectSet::PURE), e("read_u32_le", EffectSet::PURE),
            e("read_u64_be", EffectSet::PURE), e("read_u64_le", EffectSet::PURE), e("read_u8", EffectSet::PURE), e("slice", EffectSet::PURE),
            e("write_f32_le", EffectSet::PURE), e("write_f64_be", EffectSet::PURE), e("write_f64_le", EffectSet::PURE), e("write_i16_be", EffectSet::PURE),
            e("write_i16_le", EffectSet::PURE), e("write_i32_be", EffectSet::PURE), e("write_i32_le", EffectSet::PURE), e("write_i64_be", EffectSet::PURE),
            e("write_i64_le", EffectSet::PURE), e("write_i8", EffectSet::PURE), e("write_u16_be", EffectSet::PURE), e("write_u16_le", EffectSet::PURE),
            e("write_u32_be", EffectSet::PURE), e("write_u32_le", EffectSet::PURE), e("write_u64_be", EffectSet::PURE), e("write_u64_le", EffectSet::PURE),
            e("write_u8", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["bytes", "builder"],
        fns: &[
            e("__append", EffectSet::PURE), e("__append_f32", EffectSet::PURE), e("__append_f64", EffectSet::PURE), e("__append_pad", EffectSet::PURE),
            e("__append_scalar", EffectSet::PURE), e("__append_slice", EffectSet::PURE), e("__append_str", EffectSet::PURE), e("__clear", EffectSet::PURE),
            e("__finish", EffectSet::PURE), e("__free", EffectSet::PURE), e("__len", EffectSet::PURE), e("__new", EffectSet::PURE), e("__shift_front", EffectSet::PURE),
            e("__snapshot", EffectSet::PURE), e("__text_view", EffectSet::PURE), e("__view", EffectSet::PURE), e("__xor_mask_into", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["tar"],
        fns: &[
            e("entries", EffectSet::PURE), e("entry_data", EffectSet::PURE), e("entry_name", EffectSet::PURE), e("entry_size", EffectSet::PURE),
            e("entry_type", EffectSet::PURE), e("finish", EffectSet::PURE), e("pack", EffectSet::PURE), e("pack_dir", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["compress"],
        fns: &[
            e("gunzip", EffectSet::PURE), e("gzip", EffectSet::PURE), e("unzstd", EffectSet::PURE), e("zstd", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["crypto"],
        fns: &[
            e("crc32", EffectSet::PURE), e("ecdsa_p256_sign", EffectSet::PURE), e("ecdsa_p256_verify", EffectSet::PURE), e("hmac_sha256", EffectSet::PURE),
            e("hmac_sha512", EffectSet::PURE), e("sha1", EffectSet::PURE), e("sha256", EffectSet::PURE), e("sha512", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["ring"],
        fns: &[
            e("__spsc_emit", EffectSet::SYSCALL),
            e("__spsc_init", EffectSet::SYSCALL),
            e("__spsc_note_drop", EffectSet::SYSCALL),
            e("__spsc_read", EffectSet::SYSCALL),
            e("__spsc_set_tag_b", EffectSet::SYSCALL),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["decimal"],
        fns: &[
            e("format", EffectSet::PURE),
            e("to_float", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["diag"],
        fns: &[
            e("heap_alloc_count", EffectSet::SYSCALL), e("syscall_count", EffectSet::SYSCALL),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["env"],
        fns: &[
            e("arg", EffectSet::ENV), e("arg_or", EffectSet::ENV), e("args_count", EffectSet::ENV), e("var", EffectSet::ENV), e("var_exists", EffectSet::ENV),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["http"],
        fns: &[
            e("build_context", EffectSet::PURE), e("get", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("header", EffectSet::PURE), e("is_route", EffectSet::PURE), e("parse_request", EffectSet::PURE), e("parse_url", EffectSet::PURE), e("path_param", EffectSet::PURE),
            e("post", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("query_param", EffectSet::PURE), e("request", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("write_response", EffectSet::SYSCALL.union(EffectSet::BLOCK)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "file"],
        fns: &[
            e("__at_eof", EffectSet::SYSCALL), e("__close", EffectSet::SYSCALL), e("__open", EffectSet::SYSCALL), e("__read_line", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("__seek", EffectSet::SYSCALL),
            e("__write_bytes", EffectSet::SYSCALL), e("at_eof", EffectSet::SYSCALL), e("open", EffectSet::SYSCALL), e("read_line", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("seek", EffectSet::SYSCALL),
            e("write_bytes", EffectSet::SYSCALL), e("write_line", EffectSet::SYSCALL),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "fs"],
        fns: &[
            e("extension", EffectSet::SYSCALL), e("file_exists", EffectSet::SYSCALL), e("file_size", EffectSet::SYSCALL),             e("list_dir_at", EffectSet::SYSCALL), e("list_dir_count", EffectSet::SYSCALL), e("mkdir", EffectSet::SYSCALL), e("mktemp", EffectSet::SYSCALL),
            e("read_bytes", EffectSet::SYSCALL), e("read_file", EffectSet::SYSCALL), e("rename", EffectSet::SYSCALL), e("unlink", EffectSet::SYSCALL), e("write_bytes", EffectSet::SYSCALL),
            e("write_file", EffectSet::SYSCALL),
            e("write_file_append", EffectSet::SYSCALL),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "stdin"],
        fns: &[
            e("read_byte", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("read_line", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("read_line_status", EffectSet::SYSCALL.union(EffectSet::BLOCK)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "stdout"],
        fns: &[
            e("write_bytes", EffectSet::SYSCALL),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["api"],
        fns: &[
            e("local_context", EffectSet::ALLOC),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "unix"],
        fns: &[
            e("connect", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("connect_wait", EffectSet::SYSCALL.union(EffectSet::BLOCK)),
            e("group_id", EffectSet::SYSCALL),
            e("listen_socket", EffectSet::SYSCALL),
            e("peer_gid", EffectSet::SYSCALL), e("peer_group_at", EffectSet::SYSCALL), e("peer_groups_count", EffectSet::SYSCALL),
            e("peer_pid", EffectSet::SYSCALL), e("peer_uid", EffectSet::SYSCALL),
            e("user_id", EffectSet::SYSCALL),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "tcp"],
        fns: &[
            e("__accept_one", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("__close_fd", EffectSet::SYSCALL), e("__connect", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("__io_error_kind", EffectSet::PURE),
            e("__last_io_status", EffectSet::PURE), e("__listen_socket", EffectSet::SYSCALL),
            e("__recv", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("__recv_bytes", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("__send", EffectSet::SYSCALL), e("__send_bytes", EffectSet::SYSCALL),
            e("__set_recv_timeout_ns", EffectSet::SYSCALL), e("__shutdown_listen_socket", EffectSet::SYSCALL),
            e("accept_one", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("close_fd", EffectSet::SYSCALL), e("connect", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("connect_wait", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("last_recv_kernel_ns", EffectSet::PURE),
            e("last_recv_user_ns", EffectSet::PURE), e("listen_socket", EffectSet::SYSCALL), e("recv_into", EffectSet::SYSCALL.union(EffectSet::BLOCK)),
            e("recv_stamped_into", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("send_fd", EffectSet::SYSCALL), e("set_nodelay", EffectSet::SYSCALL),
            e("set_recv_timeout", EffectSet::SYSCALL), e("set_rx_timestamps", EffectSet::SYSCALL), e("set_send_timeout", EffectSet::SYSCALL),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "tls"],
        fns: &[
            e("close", EffectSet::SYSCALL), e("connect", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("last_recv_kernel_ns", EffectSet::PURE), e("last_recv_user_ns", EffectSet::PURE),
            e("recv_bytes", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("recv_into", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("recv_stamped_into", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("send_bytes", EffectSet::SYSCALL),
            e("set_nodelay", EffectSet::SYSCALL), e("set_recv_timeout", EffectSet::SYSCALL), e("set_rx_timestamps", EffectSet::SYSCALL),
            e("set_send_timeout", EffectSet::SYSCALL), e("upgrade", EffectSet::SYSCALL.union(EffectSet::BLOCK)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "udp"],
        fns: &[
            e("__bind", EffectSet::SYSCALL), e("__close", EffectSet::SYSCALL), e("__recv", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("__send", EffectSet::SYSCALL), e("bind", EffectSet::SYSCALL), e("close", EffectSet::SYSCALL),
            e("get_option_int", EffectSet::SYSCALL), e("join_group", EffectSet::SYSCALL), e("last_source_host", EffectSet::PURE),
            e("last_source_port", EffectSet::PURE), e("leave_group", EffectSet::SYSCALL), e("recv", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("recv_into", EffectSet::SYSCALL.union(EffectSet::BLOCK)),
            e("recv_with_source", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("send", EffectSet::SYSCALL), e("set_multicast_iface", EffectSet::SYSCALL),
            e("set_multicast_loop", EffectSet::SYSCALL), e("set_multicast_ttl", EffectSet::SYSCALL), e("set_option_bool", EffectSet::SYSCALL),
            e("set_option_int", EffectSet::SYSCALL), e("set_recv_timeout", EffectSet::SYSCALL), e("set_send_timeout", EffectSet::SYSCALL),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["json"],
        fns: &[
            e("array_first", EffectSet::PURE), e("array_first_span", EffectSet::PURE), e("array_next", EffectSet::PURE),
            e("array_next_span", EffectSet::PURE), e("escape_string", EffectSet::PURE), e("find_bool_field", EffectSet::PURE),
            e("find_field_range_in", EffectSet::PURE), e("find_field_raw", EffectSet::PURE), e("find_field_raw_in", EffectSet::PURE),
            e("find_int_field", EffectSet::PURE), e("find_string_field", EffectSet::PURE), e("iter_find_bool_field", EffectSet::PURE),
            e("iter_find_field_range", EffectSet::PURE), e("iter_find_field_raw", EffectSet::PURE),
            e("iter_find_int_field", EffectSet::PURE), e("iter_find_string_field", EffectSet::PURE),
            e("iter_find_string_field_range", EffectSet::PURE), e("iter_substring", EffectSet::PURE),
            e("next_non_ws", EffectSet::PURE), e("next_quote_or_bs", EffectSet::PURE), e("next_struct_or_quote", EffectSet::PURE),
            e("obj_key_eq", EffectSet::PURE), e("obj_key_len", EffectSet::PURE), e("obj_key_string", EffectSet::PURE), e("obj_value_bool", EffectSet::PURE),
            e("obj_value_float", EffectSet::PURE), e("obj_value_int", EffectSet::PURE), e("obj_value_raw", EffectSet::PURE),
            e("obj_value_string", EffectSet::PURE), e("object_first", EffectSet::PURE), e("object_next", EffectSet::PURE),
            // GH #719: the typed field read — a byte scan over the
            // caller's String, like every other reader here.
            e("string_field", EffectSet::PURE),
            e("unescape_string", EffectSet::PURE),
            // GH #754: syntax validation. Both are byte scans over
            // one immutable String with no allocation at all — the
            // uniqueness table is a fixed local array — so PURE is
            // the honest class, not ALLOC.
            e("valid", EffectSet::PURE), e("valid_object", EffectSet::PURE),
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
            e("node_child", EffectSet::PURE),
            e("node_child_count", EffectSet::PURE),
            e("node_end_byte", EffectSet::PURE),
            e("node_is_named", EffectSet::PURE),
            e("node_kind", EffectSet::PURE),
            e("node_named_child", EffectSet::PURE),
            e("node_named_child_count", EffectSet::PURE),
            e("node_start_byte", EffectSet::PURE),
            e("node_text", EffectSet::PURE),
            e("parse_go", EffectSet::ALLOC),
            e("root_node", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["shm"],
        fns: &[
            e("last_record_kernel_ns", EffectSet::PURE),
            e("last_record_seq", EffectSet::PURE),
            e("last_record_user_ns", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["math"],
        fns: &[
            e("acos", EffectSet::PURE), e("asin", EffectSet::PURE), e("atan", EffectSet::PURE), e("atan2", EffectSet::PURE), e("ceil", EffectSet::PURE), e("cos", EffectSet::PURE), e("exp", EffectSet::PURE),
            e("float_to_int", EffectSet::PURE), e("floor", EffectSet::PURE), e("inf", EffectSet::PURE), e("int_to_float", EffectSet::PURE), e("is_nan", EffectSet::PURE),
            e("log", EffectSet::PURE), e("nan", EffectSet::PURE), e("pow", EffectSet::PURE), e("round", EffectSet::PURE), e("sin", EffectSet::PURE), e("sqrt", EffectSet::PURE), e("tan", EffectSet::PURE), e("tanh", EffectSet::PURE),
            e("trunc", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["os"],
        fns: &[
            e("getrandom", EffectSet::ENTROPY.union(EffectSet::SYSCALL)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["process"],
        fns: &[
            e("__kill_escalate", EffectSet::SYSCALL), e("__pipe_read", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("__pipe_write", EffectSet::SYSCALL),
            e("__signal_pid", EffectSet::SYSCALL), e("__spawn", EffectSet::SYSCALL), e("__try_wait_pid", EffectSet::SYSCALL), e("__wait_pid", EffectSet::SYSCALL.union(EffectSet::BLOCK)),
            // GH #716: adopt closes the outgoing handle's fds and
            // TERM/KILL-reaps its process, so it carries the same
            // syscall class as kill — not PURE, despite reading like
            // an assignment.
            e("adopt", EffectSet::SYSCALL),
            e("dump_arena_residency", EffectSet::SYSCALL), e("dump_pool_residency", EffectSet::SYSCALL),
            e("exit", EffectSet::SYSCALL), e("kill", EffectSet::SYSCALL), e("pid", EffectSet::SYSCALL), e("uid", EffectSet::SYSCALL), e("read_stderr", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("read_stdout", EffectSet::SYSCALL.union(EffectSet::BLOCK)),
            e("rss_bytes", EffectSet::SYSCALL), e("run", EffectSet::SYSCALL.union(EffectSet::BLOCK)), e("signal", EffectSet::SYSCALL), e("spawn", EffectSet::SYSCALL), e("try_wait", EffectSet::SYSCALL), e("wait", EffectSet::SYSCALL.union(EffectSet::BLOCK)),
            e("write_stdin", EffectSet::SYSCALL),
        ],
        open_prefixes: &[],
    },
    // GH #469 B: `std::log::kv(k, v)` renders one structured field.
    // Pure — it is string formatting; the I/O happens in the sink,
    // which is an ordinary bus subscriber.
    NsSurface {
        ns: &["log"],
        fns: &[e("kv", EffectSet::PURE)],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["metrics"],
        fns: &[
            e("counter", EffectSet::PURE), e("gauge", EffectSet::PURE), e("histogram", EffectSet::PURE), e("labels_append", EffectSet::PURE),
            e("labels_empty", EffectSet::PURE), e("labels_one", EffectSet::PURE), e("labels_two", EffectSet::PURE), e("metric_key", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["rand"],
        fns: &[
            e("next_int", EffectSet::ENTROPY), e("seed_from_time", EffectSet::ENTROPY.union(EffectSet::TIME)),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["str"],
        fns: &[
            e("builder_append", EffectSet::PURE), e("builder_finish", EffectSet::PURE), e("builder_len", EffectSet::PURE),
            e("builder_new", EffectSet::PURE), e("byte_at_unchecked", EffectSet::PURE),             e("can_parse_float", EffectSet::PURE), e("can_parse_int", EffectSet::PURE), e("clone", EffectSet::PURE), e("from_bytes", EffectSet::PURE),
            // GH #720 — ByteView byte scanning. All three are pure:
            // `bytes_view` is one strlen, `byte_at` a bounds-checked
            // load, `slice` / `range_copy` an arena copy of a range
            // the caller already bounded.
            e("byte_at", EffectSet::PURE), e("bytes_view", EffectSet::PURE), e("range_copy", EffectSet::PURE), e("slice", EffectSet::PURE),
            e("index_of", EffectSet::PURE), e("contains", EffectSet::PURE), e("split_into", EffectSet::PURE), e("join", EffectSet::PURE), e("cp_count", EffectSet::PURE), e("cp_at", EffectSet::PURE), e("cp_size", EffectSet::PURE), e("starts_with", EffectSet::PURE), e("ends_with", EffectSet::PURE), e("lower", EffectSet::PURE), e("pad_left", EffectSet::PURE), e("pad_right", EffectSet::PURE), e("parse_decimal", EffectSet::PURE),
            e("parse_float", EffectSet::PURE), e("parse_int", EffectSet::PURE), e("range_eq", EffectSet::PURE), e("range_parse_decimal", EffectSet::PURE),
            e("range_parse_int", EffectSet::PURE), e("repeat", EffectSet::PURE), e("replace", EffectSet::PURE), e("substring", EffectSet::PURE), e("trim", EffectSet::PURE),
            e("upper", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["term"],
        fns: &[
            e("__raw_disable", EffectSet::SYSCALL), e("__raw_enable", EffectSet::SYSCALL), e("__size_packed", EffectSet::SYSCALL), e("is_tty", EffectSet::SYSCALL),
            e("size", EffectSet::SYSCALL),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["test"],
        fns: &[
            e("assert", EffectSet::PURE), e("assert_eq_int", EffectSet::PURE), e("assert_eq_str", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["text"],
        fns: &[
            e("is_alnum", EffectSet::PURE), e("is_alpha", EffectSet::PURE), e("is_digit", EffectSet::PURE), e("is_whitespace", EffectSet::PURE),
            e("is_word_char", EffectSet::PURE), e("md_to_html", EffectSet::PURE), e("tokenize_words_into", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["text", "base64"],
        fns: &[
            e("decode", EffectSet::PURE), e("encode", EffectSet::PURE), e("url_encode", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["time"],
        fns: &[
            e("parse_iso8601", EffectSet::PURE), e("can_parse_iso8601", EffectSet::PURE),
            e("current", EffectSet::TIME), e("iso8601", EffectSet::PURE), e("parse_time", EffectSet::PURE),
            e("unix", EffectSet::PURE), e("nanos", EffectSet::PURE), e("from_nanos", EffectSet::PURE),
            e("monotonic", EffectSet::TIME), e("monotonic_ns", EffectSet::TIME), e("now", EffectSet::TIME), e("sleep", EffectSet::SYSCALL.union(EffectSet::BLOCK).union(EffectSet::TIME)), e("time_from_unix", EffectSet::PURE),
        ],
        open_prefixes: &[],
    },
];

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
    for entry in surface.fns {
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
    if surface.fns.iter().any(|e| e.name == name) {
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

// M3 stage 2 signature rows — see FnSig. Filled from the
// per-function lowering verification (each lowering fn's arg-count
// checks + type coercions read directly, cross-checked against
// spec/stdlib.md); UNCERTAIN rows are EXCLUDED, not guessed:
// - str::builder_* / can_parse_decimal (spec lists it, dispatch
//   doesn't implement it — flagged for the spec);
// - everything io::fs/tcp/tls/udp/file (String-heavy tranche 2).
//
// GH #771: a type slot is a bare `SigTy` variant (`Str`, `Int`) OR
// `Named("__JsonString")` — the tuple variant, written the way the
// enum spells it. Both positions take both forms, so the rows for
// struct-returning helpers read like every other row:
//
//     sig!(NS_JSON, "string_field", [Str, Str], Named("__JsonString"))
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
macro_rules! sig {
    ($ns:expr, $name:literal,
     [$($p:ident $(($pn:literal))?),*],
     $ret:ident $(($rn:literal))?) => {
        FnSig { ns: $ns, name: $name,
                params: &[$(SigTy::$p $(($pn))?),*],
                ret: SigTy::$ret $(($rn))?,
                fallible: None }
    };
    ($ns:expr, $name:literal,
     [$($p:ident $(($pn:literal))?),*],
     $ret:ident $(($rn:literal))?, $err:literal) => {
        FnSig { ns: $ns, name: $name,
                params: &[$(SigTy::$p $(($pn))?),*],
                ret: SigTy::$ret $(($rn))?,
                fallible: Some($err) }
    };
}

const NS_MATH: &[&str] = &["math"];
const NS_REGEX: &[&str] = &["regex"];
const NS_TIME: &[&str] = &["time"];
const NS_ENV: &[&str] = &["env"];
const NS_DEC: &[&str] = &["decimal"];
const NS_PROC: &[&str] = &["process"];
const NS_STR: &[&str] = &["str"];
const NS_STDIN: &[&str] = &["io", "stdin"];
const NS_STDOUT: &[&str] = &["io", "stdout"];
const NS_BYTES: &[&str] = &["bytes"];
const NS_CRYPTO: &[&str] = &["crypto"];
const NS_COMPRESS: &[&str] = &["compress"];
const NS_TAR: &[&str] = &["tar"];
const NS_B64: &[&str] = &["text", "base64"];
const NS_RAND: &[&str] = &["rand"];
const NS_FS: &[&str] = &["io", "fs"];
const NS_JSON: &[&str] = &["json"];
const NS_HTTP: &[&str] = &["http"];
const NS_FILE: &[&str] = &["io", "file"];
const NS_TCP: &[&str] = &["io", "tcp"];
const NS_TLS: &[&str] = &["io", "tls"];
const NS_UDP: &[&str] = &["io", "udp"];
const NS_UNIX: &[&str] = &["io", "unix"];
const NS_API: &[&str] = &["api"];
const NS_TEXT: &[&str] = &["text"];
const NS_TERM: &[&str] = &["term"];
const NS_LOG: &[&str] = &["log"];
const NS_DIAG: &[&str] = &["diag"];
const NS_OS: &[&str] = &["os"];

pub const SIGS: &[FnSig] = &[
    // std::math — unary/binary fns sitofp-coerce Int args.
    sig!(NS_MATH, "sqrt", [Float], Float),
    sig!(NS_MATH, "exp", [Float], Float),
    sig!(NS_MATH, "log", [Float], Float),
    sig!(NS_MATH, "floor", [Float], Float),
    sig!(NS_MATH, "ceil", [Float], Float),
    sig!(NS_MATH, "pow", [Float, Float], Float),
    sig!(NS_MATH, "tanh", [Float], Float),
    sig!(NS_MATH, "nan", [], Float),
    sig!(NS_MATH, "inf", [], Float),
    sig!(NS_MATH, "is_nan", [Float], Bool),
    sig!(NS_MATH, "sin", [Float], Float),
    sig!(NS_MATH, "cos", [Float], Float),
    sig!(NS_MATH, "tan", [Float], Float),
    sig!(NS_MATH, "asin", [Float], Float),
    sig!(NS_MATH, "acos", [Float], Float),
    sig!(NS_MATH, "atan", [Float], Float),
    sig!(NS_MATH, "atan2", [Float, Float], Float),
    sig!(NS_MATH, "int_to_float", [Int], Float),
    sig!(NS_MATH, "float_to_int", [Float], Int),
    sig!(NS_MATH, "round", [Float], Int),
    sig!(NS_MATH, "trunc", [Float], Int),
    // std::time — sleep takes Duration (Int rejected in lowering);
    // now() is epoch SECONDS as Int; time_from_unix returns Time.
    // #353: linear-time regex — pure over immutable inputs.
    sig!(NS_REGEX, "matches", [Str, Str], Bool),
    sig!(NS_REGEX, "find", [Str, Str], Int),
    sig!(NS_REGEX, "valid", [Str], Bool),
    sig!(NS_TIME, "monotonic", [], Duration),
    sig!(NS_TIME, "monotonic_ns", [], Int),
    sig!(NS_TIME, "sleep", [Duration], Unit),
    sig!(NS_TIME, "now", [], Int),
    sig!(NS_TIME, "time_from_unix", [Int], Time),
    // GH #607: Time is a value. `current()` reads the wall clock as an
    // instant (the `now()` above stays epoch seconds as Int);
    // `iso8601` / `parse_time` round-trip through text; `unix` and
    // `nanos` / `from_nanos` are the integer views.
    sig!(NS_TIME, "current", [], Time),
    sig!(NS_TIME, "iso8601", [Time], Str),
    sig!(NS_TIME, "parse_time", [Str], Time, "ParseError"),
    sig!(NS_TIME, "unix", [Time], Int),
    sig!(NS_TIME, "nanos", [Time], Int),
    sig!(NS_TIME, "from_nanos", [Int], Time),
    // #353: the inverse of `time_from_unix`, which already yields
    // ISO-8601 text. Returns unix seconds. UTC only, and PURE — it
    // reads no clock and no TZ.
    sig!(NS_TIME, "parse_iso8601", [Str], Int, "ParseError"),
    sig!(NS_TIME, "can_parse_iso8601", [Str], Bool),
    // std::env
    sig!(NS_ENV, "args_count", [], Int),
    sig!(NS_ENV, "arg", [Int], Str),
    sig!(NS_ENV, "arg_or", [Int, Str], Str),
    sig!(NS_ENV, "var", [Str], Str),
    sig!(NS_ENV, "var_exists", [Str], Bool),
    // std::decimal
    sig!(NS_DEC, "to_float", [Decimal], Float),
    // std::process (scalar subset; run/spawn/wait/... in tranche 2)
    sig!(NS_PROC, "pid", [], Int),
    sig!(NS_PROC, "uid", [], Int),
    sig!(NS_PROC, "exit", [Int], Unit),
    sig!(NS_PROC, "rss_bytes", [], Int),
    sig!(NS_PROC, "dump_arena_residency", [], Int),
    sig!(NS_PROC, "dump_pool_residency", [], Int),
    // std::str (builder_* excluded — opaque handle API)
    sig!(NS_STR, "parse_int", [Str], Int, "ParseError"),
    sig!(NS_STR, "parse_float", [Str], Float, "ParseError"),
    sig!(NS_STR, "parse_decimal", [Str], Decimal, "ParseError"),
    sig!(NS_STR, "can_parse_int", [Str], Bool),
    sig!(NS_STR, "can_parse_float", [Str], Bool),
    sig!(NS_STR, "range_parse_int", [Str, Int, Int], Int, "ParseError"),
    sig!(
        NS_STR,
        "range_parse_decimal",
        [Str, Int, Int],
        Decimal,
        "ParseError"
    ),
    sig!(NS_STR, "range_eq", [Str, Int, Int, Str], Bool),
    sig!(NS_STR, "byte_at_unchecked", [Str, Int], Int),
    // GH #720 — the ByteView surface. The view is the struct
    // `str_view.hl` declares, named here by the mangled name
    // PATH_RENAMES maps `std::str::ByteView` onto, so a view built
    // by `bytes_view` and a view annotated by hand are the same
    // type to the checker. `range_copy` is the strlen-free
    // substring `slice` is built on — public because a scanner
    // that already tracks its own bounds (the JSON range walkers)
    // wants it directly, and named `range_*` because it carries
    // that family's caller-owns-the-bounds contract.
    FnSig {
        ns: NS_STR,
        name: "bytes_view",
        params: &[SigTy::Str],
        ret: SigTy::Named("__StrByteView"),
        fallible: None,
    },
    FnSig {
        ns: NS_STR,
        name: "byte_at",
        params: &[SigTy::Named("__StrByteView"), SigTy::Int],
        ret: SigTy::Int,
        fallible: None,
    },
    sig!(
        NS_STR,
        "slice",
        [Named("__StrByteView"), Int, Int],
        Str
    ),
    sig!(NS_STR, "range_copy", [Str, Int, Int, Int], Str),
    // GH #535 (DNA F.8): the flat-object json readers are Hale-source
    // stdlib fns with no rename entry, so a call typed Unknown and an
    // `or` on one slid through the checker to fail at build. Tabled,
    // they type precisely and an `or` is refused where it is written.
    sig!(NS_JSON, "find_string_field", [Str, Str], Str),
    sig!(NS_JSON, "find_int_field", [Str, Str], Int),
    sig!(NS_JSON, "find_bool_field", [Str, Str], Bool),
    sig!(NS_JSON, "find_field_raw", [Str, Str], Str),
    sig!(NS_JSON, "escape_string", [Str], Str),
    sig!(NS_JSON, "unescape_string", [Str], Str),
    // GH #754: one well-formed RFC 8259 value / a top-level object
    // with unique literal keys.
    sig!(NS_JSON, "valid", [Str], Bool),
    sig!(NS_JSON, "valid_object", [Str], Bool),
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
    sig!(NS_JSON, "string_field", [Str, Str], Named("__JsonString")),
    sig!(
        NS_JSON,
        "array_first",
        [Str],
        Named("__JsonArrayIter")
    ),
    sig!(
        NS_JSON,
        "array_next",
        [Named("__JsonArrayIter")],
        Named("__JsonArrayIter")
    ),
    sig!(
        NS_JSON,
        "array_first_span",
        [Str],
        Named("__JsonArrayIterSpan")
    ),
    sig!(
        NS_JSON,
        "array_next_span",
        [Named("__JsonArrayIterSpan"), Str],
        Named("__JsonArrayIterSpan")
    ),
    sig!(
        NS_JSON,
        "object_first",
        [Str],
        Named("__JsonObjectIterSpan")
    ),
    sig!(
        NS_JSON,
        "object_next",
        [Named("__JsonObjectIterSpan"), Str],
        Named("__JsonObjectIterSpan")
    ),
    sig!(
        NS_JSON,
        "iter_find_field_range",
        [Named("__JsonArrayIterSpan"), Str, Str],
        Named("__JsonFieldRange")
    ),
    sig!(
        NS_JSON,
        "iter_find_string_field_range",
        [Named("__JsonArrayIterSpan"), Str, Str],
        Named("__JsonFieldRange")
    ),
    sig!(
        NS_JSON,
        "find_field_range_in",
        [Str, Str, Int, Int],
        Named("__JsonFieldRange")
    ),
    // GH #771: the one non-json member of the same class — also
    // dispatch-routed, also a struct return.
    sig!(
        NS_HTTP,
        "parse_request",
        [Str],
        Named("__StdHttpRequest")
    ),
    sig!(NS_STR, "index_of", [Str, Str], Int),
    // #353: the everyday predicates. The runtime carried
    // `lotus_str_contains` / `_starts_with` all along; `ends_with` is
    // new. Pure reads over immutable data — no effects.
    sig!(NS_STR, "split_into", [Str, Str, Any], Unit),
    sig!(NS_STR, "join", [Any, Str], Str),
    sig!(NS_STR, "cp_count", [Str], Int),
    sig!(NS_STR, "cp_at", [Str, Int], Int),
    sig!(NS_STR, "cp_size", [Str, Int], Int),
    sig!(NS_STR, "contains", [Str, Str], Bool),
    sig!(NS_STR, "starts_with", [Str, Str], Bool),
    sig!(NS_STR, "ends_with", [Str, Str], Bool),
    sig!(NS_STR, "lower", [Str], Str),
    // GH #469 B: one structured log field, logfmt-quoted.
    sig!(NS_LOG, "kv", [Str, Str], Str),
    sig!(NS_STR, "upper", [Str], Str),
    sig!(NS_STR, "trim", [Str], Str),
    sig!(NS_STR, "substring", [Str, Int, Int], Str),
    sig!(NS_STR, "replace", [Str, Str, Str], Str),
    sig!(NS_STR, "repeat", [Str, Int], Str),
    sig!(NS_STR, "pad_left", [Str, Int, Str], Str),
    sig!(NS_STR, "pad_right", [Str, Int, Str], Str),
    sig!(NS_STR, "from_bytes", [Bytes], Str),
    sig!(NS_STR, "clone", [Str], Str),
    // std::io::stdin / stdout
    sig!(NS_STDIN, "read_line", [], Str),
    sig!(NS_STDIN, "read_line_status", [], Int),
    sig!(NS_STDIN, "read_byte", [Int], Int),
    sig!(NS_STDOUT, "write_bytes", [Str], Int),
    // std::bytes — reads accept Bytes/BytesView/BytesMut; writes
    // require a BytesMut window (accepts() stays permissive on the
    // family, favoring no-false-error over full strictness).
    sig!(NS_BYTES, "at", [Bytes, Int], Int, "IndexError"),
    sig!(NS_BYTES, "slice", [Bytes, Int, Int], Bytes),
    sig!(NS_BYTES, "from_string", [Str], Bytes),
    sig!(NS_BYTES, "from_int", [Int], Bytes),
    sig!(NS_BYTES, "concat", [Bytes, Bytes], Bytes),
    sig!(NS_BYTES, "clone", [Bytes], Bytes),
    sig!(NS_BYTES, "find_byte", [Bytes, Int, Int], Int),
    sig!(NS_BYTES, "read_u8", [Bytes, Int], Int, "IndexError"),
    sig!(NS_BYTES, "read_u16_le", [Bytes, Int], Int, "IndexError"),
    sig!(NS_BYTES, "read_u16_be", [Bytes, Int], Int, "IndexError"),
    sig!(NS_BYTES, "read_u32_le", [Bytes, Int], Int, "IndexError"),
    sig!(NS_BYTES, "read_u32_be", [Bytes, Int], Int, "IndexError"),
    sig!(NS_BYTES, "read_u64_le", [Bytes, Int], Int, "IndexError"),
    sig!(NS_BYTES, "read_u64_be", [Bytes, Int], Int, "IndexError"),
    sig!(NS_BYTES, "read_i8", [Bytes, Int], Int, "IndexError"),
    sig!(NS_BYTES, "read_i16_le", [Bytes, Int], Int, "IndexError"),
    sig!(NS_BYTES, "read_i16_be", [Bytes, Int], Int, "IndexError"),
    sig!(NS_BYTES, "read_i32_le", [Bytes, Int], Int, "IndexError"),
    sig!(NS_BYTES, "read_i32_be", [Bytes, Int], Int, "IndexError"),
    sig!(NS_BYTES, "read_i64_le", [Bytes, Int], Int, "IndexError"),
    sig!(NS_BYTES, "read_i64_be", [Bytes, Int], Int, "IndexError"),
    sig!(NS_BYTES, "read_f32_le", [Bytes, Int], Float, "IndexError"),
    sig!(NS_BYTES, "read_f64_le", [Bytes, Int], Float, "IndexError"),
    sig!(NS_BYTES, "read_f64_be", [Bytes, Int], Float, "IndexError"),
    sig!(NS_BYTES, "write_u8", [BytesMut, Int, Int], Int, "IndexError"),
    sig!(NS_BYTES, "write_u16_le", [BytesMut, Int, Int], Int, "IndexError"),
    sig!(NS_BYTES, "write_u16_be", [BytesMut, Int, Int], Int, "IndexError"),
    sig!(NS_BYTES, "write_u32_le", [BytesMut, Int, Int], Int, "IndexError"),
    sig!(NS_BYTES, "write_u32_be", [BytesMut, Int, Int], Int, "IndexError"),
    sig!(NS_BYTES, "write_u64_le", [BytesMut, Int, Int], Int, "IndexError"),
    sig!(NS_BYTES, "write_u64_be", [BytesMut, Int, Int], Int, "IndexError"),
    sig!(NS_BYTES, "write_i8", [BytesMut, Int, Int], Int, "IndexError"),
    sig!(NS_BYTES, "write_i16_le", [BytesMut, Int, Int], Int, "IndexError"),
    sig!(NS_BYTES, "write_i16_be", [BytesMut, Int, Int], Int, "IndexError"),
    sig!(NS_BYTES, "write_i32_le", [BytesMut, Int, Int], Int, "IndexError"),
    sig!(NS_BYTES, "write_i32_be", [BytesMut, Int, Int], Int, "IndexError"),
    sig!(NS_BYTES, "write_i64_le", [BytesMut, Int, Int], Int, "IndexError"),
    sig!(NS_BYTES, "write_i64_be", [BytesMut, Int, Int], Int, "IndexError"),
    sig!(NS_BYTES, "write_f32_le", [BytesMut, Int, Float], Int, "IndexError"),
    sig!(NS_BYTES, "write_f64_le", [BytesMut, Int, Float], Int, "IndexError"),
    sig!(NS_BYTES, "write_f64_be", [BytesMut, Int, Float], Int, "IndexError"),
    // std::crypto
    sig!(NS_CRYPTO, "sha1", [Bytes], Bytes),
    sig!(NS_CRYPTO, "sha256", [Bytes], Bytes),
    sig!(NS_CRYPTO, "sha512", [Bytes], Bytes),
    sig!(NS_CRYPTO, "crc32", [Bytes], Int),
    sig!(NS_CRYPTO, "hmac_sha256", [Bytes, Bytes], Bytes),
    sig!(NS_CRYPTO, "hmac_sha512", [Bytes, Bytes], Bytes),
    // std::compress (GH #254): one-shot Bytes -> Bytes. gzip pair
    // rides zlib (-lz, universal); zstd pair dlopens libzstd at
    // first use — on a machine without it the call fails
    // kind="not_found" rather than the program failing to link.
    sig!(NS_COMPRESS, "gzip", [Bytes], Bytes, "IoError"),
    sig!(NS_COMPRESS, "gunzip", [Bytes], Bytes, "IoError"),
    sig!(NS_COMPRESS, "zstd", [Bytes], Bytes, "IoError"),
    sig!(NS_COMPRESS, "unzstd", [Bytes], Bytes, "IoError"),
    // std::tar (GH #254): one-shot ustar over Bytes. Read side is
    // indexed (list-then-extract shape); write side is append-style
    // (start from empty Bytes, pack entries, finish appends the
    // terminating zero blocks).
    sig!(NS_TAR, "entries", [Bytes], Int, "IoError"),
    sig!(NS_TAR, "entry_name", [Bytes, Int], Str, "IoError"),
    sig!(NS_TAR, "entry_size", [Bytes, Int], Int, "IoError"),
    sig!(NS_TAR, "entry_type", [Bytes, Int], Str, "IoError"),
    sig!(NS_TAR, "entry_data", [Bytes, Int], Bytes, "IoError"),
    sig!(NS_TAR, "pack", [Bytes, Str, Bytes], Bytes, "IoError"),
    sig!(NS_TAR, "pack_dir", [Bytes, Str], Bytes, "IoError"),
    sig!(NS_TAR, "finish", [Bytes], Bytes, "IoError"),
    // std::text::base64
    sig!(NS_B64, "encode", [Bytes], Str),
    sig!(NS_B64, "decode", [Str], Bytes),
    sig!(NS_B64, "url_encode", [Bytes], Str),
    // std::rand
    sig!(NS_RAND, "next_int", [Int], Int),
    sig!(NS_RAND, "seed_from_time", [], Unit),
    // ── Tranche 2 (2026-07-02): the I/O namespaces. Verified the
    // same way (per-fn lowering read). EXCLUDED-not-guessed: all
    // std::json/std::http rows and process write_stdin/read_std*
    // (routed through Hale-stdlib __ fns — codegen never validates
    // their args, so there's no ground truth to table);
    // io::file::write_line, io::tcp set_recv/send_timeout (lowering
    // ambiguous); io::fs::list_dir (spec-only); the 7 spec'd
    // std::io::tls fns with NO lowering (recv_stamped_into,
    // last_recv_*, set_*) — names-only keeps them permissive.
    // Handle args are plain Int FDs at the path-call level (the
    // File/Stream locus wrappers live in stdlib .hl seeds).
    sig!(NS_FS, "read_file", [Str], Str, "IoError"),
    sig!(NS_FS, "read_bytes", [Str], Bytes, "IoError"),
    sig!(NS_FS, "write_file", [Str, Str], Unit, "IoError"),
    sig!(NS_FS, "write_bytes", [Str, Bytes], Unit, "IoError"),
    // GH #535 (DNA F.9): the `or` form lowers through the same
    // fallible channel as write_file (Unit success); the BARE legacy
    // call returns an Int status and stays typed Unknown like every
    // bare fallible row. The row used to say Int and the checker
    // admitted `let n = ... or 0`, which codegen then refused with a
    // message about something else.
    sig!(NS_FS, "write_file_append", [Str, Str], Unit, "IoError"),
    sig!(NS_FS, "file_size", [Str], Int, "IoError"),
    sig!(NS_FS, "mkdir", [Str], Unit, "IoError"),
    sig!(NS_FS, "rename", [Str, Str], Unit, "IoError"),
    sig!(NS_FS, "unlink", [Str], Unit, "IoError"),
    sig!(NS_FS, "mktemp", [Str, Str], Str, "IoError"),
    sig!(NS_FS, "list_dir_count", [Str], Int, "IoError"),
    sig!(NS_FS, "list_dir_at", [Str, Int], Str, "IoError"),
    sig!(NS_FS, "file_exists", [Str], Bool),
    sig!(NS_FS, "extension", [Str], Str),
    sig!(NS_FILE, "open", [Str, Str], Int, "IoError"),
    sig!(NS_FILE, "write_bytes", [Int, Bytes], Unit, "IoError"),
    sig!(NS_FILE, "seek", [Int, Int], Unit, "IoError"),
    sig!(NS_FILE, "read_line", [Int], Str),
    sig!(NS_FILE, "close", [Int], Int),
    sig!(NS_FILE, "at_eof", [Int], Bool),
    sig!(NS_TCP, "listen_socket", [Str, Int], Int, "IoError"),
    sig!(NS_TCP, "connect", [Str, Int], Int, "IoError"),
    sig!(NS_TCP, "connect_wait", [Str, Int, Duration], Int, "IoError"),
    sig!(NS_TCP, "accept_one", [Int], Int, "IoError"),
    // GH #1106: AF_UNIX stream sockets; accept/recv/send/close are tcp's.
    sig!(NS_UNIX, "listen_socket", [Str], Int, "IoError"),
    sig!(NS_UNIX, "connect", [Str], Int, "IoError"),
    sig!(NS_UNIX, "connect_wait", [Str, Duration], Int, "IoError"),
    sig!(NS_UNIX, "peer_uid", [Int], Int),
    sig!(NS_UNIX, "peer_gid", [Int], Int),
    sig!(NS_UNIX, "peer_pid", [Int], Int),
    // GH #1109: the static role table's `user:` / `group:` spellings.
    sig!(NS_UNIX, "user_id", [Str], Int),
    sig!(NS_UNIX, "group_id", [Str], Int),
    sig!(NS_UNIX, "peer_groups_count", [Int], Int),
    sig!(NS_UNIX, "peer_group_at", [Int, Int], Int),
    // GH #1108: the local context a handler reached in-process gets.
    sig!(NS_API, "local_context", [], Named("__StdApiContext")),
    sig!(NS_TCP, "close_fd", [Int], Int),
    // GH #829: the `buf` slot is not polymorphic — the lowering
    // (`lower_recv_into_common`) accepts exactly one codegen type,
    // `LocusRef("__StdBytesBytesBuilder")`, and refuses everything
    // else with a spanless "buf must be std::bytes::BytesBuilder".
    // `Any` here made `recv_into(0, 0, 64)` a check-clean program
    // that does not build. Named by the mangled spelling, which is
    // what both `resolve_type_expr` (through `PATH_RENAMES`) and the
    // stdlib's own `locus __StdBytesBytesBuilder` produce.
    sig!(NS_TCP, "recv_into", [Int, Named("__StdBytesBytesBuilder"), Int], Int),
    sig!(
        NS_TCP,
        "recv_stamped_into",
        [Int, Named("__StdBytesBytesBuilder"), Int],
        Int
    ),
    sig!(NS_TCP, "last_recv_kernel_ns", [], Int),
    sig!(NS_TCP, "last_recv_user_ns", [], Int),
    sig!(NS_TCP, "set_nodelay", [Int, Bool], Unit, "IoError"),
    sig!(NS_TCP, "set_rx_timestamps", [Int, Bool], Unit, "IoError"),
    sig!(NS_TLS, "connect", [Str, Int], Int, "IoError"),
    sig!(NS_TLS, "upgrade", [Int, Str, Bool], Int, "IoError"),
    sig!(NS_TLS, "send_bytes", [Int, Bytes], Int),
    sig!(NS_TLS, "recv_bytes", [Int, Int], Bytes),
    sig!(NS_TLS, "recv_into", [Int, Named("__StdBytesBytesBuilder"), Int], Int),
    // GH #829: `tls::recv_stamped_into` is dispatched by codegen
    // through the same `lower_recv_into_common` and is named by the
    // surface table, but had no signature row at all — so neither
    // its arity nor its buffer was checked. The family is only
    // closed if every member of it is.
    sig!(
        NS_TLS,
        "recv_stamped_into",
        [Int, Named("__StdBytesBytesBuilder"), Int],
        Int
    ),
    sig!(NS_TLS, "close", [Int], Int),
    sig!(NS_UDP, "bind", [Str, Int], Int, "IoError"),
    sig!(NS_UDP, "send", [Int, Str, Int, Any], Unit, "IoError"),
    sig!(NS_UDP, "recv", [Int, Int], Bytes, "IoError"),
    sig!(NS_UDP, "recv_into", [Int, Named("__StdBytesBytesBuilder"), Int], Int),
    sig!(NS_UDP, "close", [Int], Int),
    sig!(NS_UDP, "recv_with_source", [Int, Int], Bytes, "IoError"),
    sig!(NS_UDP, "join_group", [Int, Str, Str], Unit, "IoError"),
    sig!(NS_UDP, "leave_group", [Int, Str, Str], Unit, "IoError"),
    sig!(NS_UDP, "set_multicast_ttl", [Int, Int], Unit, "IoError"),
    sig!(NS_UDP, "set_multicast_loop", [Int, Any], Unit, "IoError"),
    sig!(NS_UDP, "set_multicast_iface", [Int, Str], Unit, "IoError"),
    sig!(NS_UDP, "set_option_int", [Int, Int, Int, Int], Unit, "IoError"),
    sig!(NS_UDP, "set_option_bool", [Int, Int, Int, Bool], Unit, "IoError"),
    sig!(NS_UDP, "get_option_int", [Int, Int, Int], Int, "IoError"),
    sig!(NS_UDP, "set_recv_timeout", [Int, Duration], Unit, "IoError"),
    sig!(NS_UDP, "set_send_timeout", [Int, Duration], Unit, "IoError"),
    sig!(NS_UDP, "last_source_host", [], Str),
    sig!(NS_UDP, "last_source_port", [], Int),
    // std::process child management — success types are internal
    // handles (__StdProcessSpawnHandle etc.); Any keeps arity +
    // arg + fallible checking without naming them.
    sig!(NS_PROC, "run", [Str], Any, "IoError"),
    sig!(NS_PROC, "spawn", [Str], Any, "IoError"),
    sig!(NS_PROC, "wait", [Int], Any, "IoError"),
    sig!(NS_PROC, "kill", [Int], Unit, "IoError"),
    // std::text byte-class predicates + tokenizer (vec target is a
    // user @form(vec) locus — Any).
    sig!(NS_TEXT, "is_alpha", [Int], Bool),
    sig!(NS_TEXT, "is_digit", [Int], Bool),
    sig!(NS_TEXT, "is_alnum", [Int], Bool),
    sig!(NS_TEXT, "is_whitespace", [Int], Bool),
    sig!(NS_TEXT, "is_word_char", [Int], Bool),
    sig!(NS_TEXT, "tokenize_words_into", [Str, Any], Unit),
    // std::term / std::diag / std::os
    sig!(NS_TERM, "is_tty", [Int], Bool),
    sig!(NS_DIAG, "heap_alloc_count", [], Int),
    sig!(NS_DIAG, "syscall_count", [Str], Int),
    sig!(NS_OS, "getrandom", [Int], Bytes, "IoError"),
];


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
    if segs.len() < 2 || segs[0] != "std" {
        return None;
    }
    let (ns_segs, name) = (&segs[1..segs.len() - 1], segs[segs.len() - 1]);
    let surface = SURFACES
        .iter()
        .filter(|s| {
            s.ns.len() == ns_segs.len()
                && s.ns.iter().zip(ns_segs).all(|(a, b)| a == b)
        })
        .next()?;
    surface
        .fns
        .iter()
        .find(|e| e.name == name)
        .map(|e| e.effects)
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
