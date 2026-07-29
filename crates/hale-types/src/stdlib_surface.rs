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
pub struct EffectSet(pub u32);

impl EffectSet {
    pub const PURE: EffectSet = EffectSet(0);
    pub const SYSCALL: EffectSet = EffectSet(1 << 0);
    pub const BLOCK: EffectSet = EffectSet(1 << 1);
    pub const PUBLISH: EffectSet = EffectSet(1 << 2);
    pub const TIME: EffectSet = EffectSet(1 << 3);
    pub const ENTROPY: EffectSet = EffectSet(1 << 4);
    pub const ENV: EffectSet = EffectSet(1 << 5);
    pub const ALLOC: EffectSet = EffectSet(1 << 6);
    /// Not yet classified (#265 step 4 turns the surface; until
    /// then queries must treat this as "may do anything").
    pub const UNCLASSIFIED: EffectSet = EffectSet(u32::MAX);

    pub const fn union(self, o: EffectSet) -> EffectSet {
        EffectSet(self.0 | o.0)
    }
    pub fn contains(self, o: EffectSet) -> bool {
        (self.0 & o.0) == o.0
    }
    pub fn is_unclassified(self) -> bool {
        self.0 == u32::MAX
    }
}

/// One registry row: the fn name plus its effect classification.
#[derive(Debug, Clone, Copy)]
pub struct FnEntry {
    pub name: &'static str,
    pub effects: EffectSet,
}

/// Row constructor for the (current) unclassified default — keeps
/// the table visually close to the old bare-name lists.
const fn f(name: &'static str) -> FnEntry {
    FnEntry { name, effects: EffectSet::UNCLASSIFIED }
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
    &["std", "process", "Child"],
    &["std", "process", "ProcessOutput"],
    &["std", "source", "Walk"],
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
    NsSurface {
        ns: &["bus"],
        fns: &[
            f("__local_dispatch"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["bytes"],
        fns: &[
            f("__is_alloc_fail"), f("at"), f("clone"), f("concat"), f("find_byte"),
            f("from_int"), f("from_string"), f("read_f32_le"), f("read_f64_be"),
            f("read_f64_le"), f("read_i16_be"), f("read_i16_le"), f("read_i32_be"),
            f("read_i32_le"), f("read_i64_be"), f("read_i64_le"), f("read_i8"),
            f("read_u16_be"), f("read_u16_le"), f("read_u32_be"), f("read_u32_le"),
            f("read_u64_be"), f("read_u64_le"), f("read_u8"), f("slice"),
            f("write_f32_le"), f("write_f64_be"), f("write_f64_le"), f("write_i16_be"),
            f("write_i16_le"), f("write_i32_be"), f("write_i32_le"), f("write_i64_be"),
            f("write_i64_le"), f("write_i8"), f("write_u16_be"), f("write_u16_le"),
            f("write_u32_be"), f("write_u32_le"), f("write_u64_be"), f("write_u64_le"),
            f("write_u8"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["bytes", "builder"],
        fns: &[
            f("__append"), f("__append_f32"), f("__append_f64"), f("__append_pad"),
            f("__append_scalar"), f("__append_slice"), f("__append_str"), f("__clear"),
            f("__finish"), f("__free"), f("__len"), f("__new"), f("__shift_front"),
            f("__snapshot"), f("__text_view"), f("__view"), f("__xor_mask_into"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["tar"],
        fns: &[
            f("entries"), f("entry_data"), f("entry_name"), f("entry_size"),
            f("entry_type"), f("finish"), f("pack"), f("pack_dir"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["compress"],
        fns: &[
            f("gunzip"), f("gzip"), f("unzstd"), f("zstd"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["crypto"],
        fns: &[
            f("crc32"), f("ecdsa_p256_sign"), f("ecdsa_p256_verify"), f("hmac_sha256"),
            f("hmac_sha512"), f("sha1"), f("sha256"), f("sha512"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["ring"],
        fns: &[
            f("__spsc_emit"),
            f("__spsc_init"),
            f("__spsc_note_drop"),
            f("__spsc_read"),
            f("__spsc_set_tag_b"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["decimal"],
        fns: &[
            f("format"),
            f("to_float"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["diag"],
        fns: &[
            f("heap_alloc_count"), f("syscall_count"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["env"],
        fns: &[
            f("arg"), f("arg_or"), f("args_count"), f("var"), f("var_exists"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["http"],
        fns: &[
            f("get"), f("header"), f("parse_request"), f("parse_url"), f("path_param"),
            f("post"), f("query_param"), f("request"), f("write_response"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "file"],
        fns: &[
            f("__at_eof"), f("__close"), f("__open"), f("__read_line"), f("__seek"),
            f("__write_bytes"), f("at_eof"), f("open"), f("read_line"), f("seek"),
            f("write_bytes"), f("write_line"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "fs"],
        fns: &[
            f("extension"), f("file_exists"), f("file_size"), f("list_dir"),
            f("list_dir_at"), f("list_dir_count"), f("mkdir"), f("mktemp"),
            f("read_bytes"), f("read_file"), f("rename"), f("unlink"), f("write_bytes"),
            f("write_file"),
            f("write_file_append"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "stdin"],
        fns: &[
            f("read_byte"), f("read_line"), f("read_line_status"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "stdout"],
        fns: &[
            f("write_bytes"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "tcp"],
        fns: &[
            f("__accept_one"), f("__close_fd"), f("__connect"), f("__io_error_kind"),
            f("__last_io_status"), f("__listen_socket"),
            f("__recv"), f("__recv_bytes"), f("__send"), f("__send_bytes"),
            f("__set_recv_timeout_ns"), f("__shutdown_listen_socket"),
            f("accept_one"), f("close_fd"), f("connect"), f("last_recv_kernel_ns"),
            f("last_recv_user_ns"), f("listen_socket"), f("recv_into"),
            f("recv_stamped_into"), f("send_fd"), f("set_nodelay"),
            f("set_recv_timeout"), f("set_rx_timestamps"), f("set_send_timeout"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "tls"],
        fns: &[
            f("close"), f("connect"), f("last_recv_kernel_ns"), f("last_recv_user_ns"),
            f("recv_bytes"), f("recv_into"), f("recv_stamped_into"), f("send_bytes"),
            f("set_nodelay"), f("set_recv_timeout"), f("set_rx_timestamps"),
            f("set_send_timeout"), f("upgrade"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "udp"],
        fns: &[
            f("__bind"), f("__close"), f("__recv"), f("__send"), f("bind"), f("close"),
            f("get_option_int"), f("join_group"), f("last_source_host"),
            f("last_source_port"), f("leave_group"), f("recv"), f("recv_into"),
            f("recv_with_source"), f("send"), f("set_multicast_iface"),
            f("set_multicast_loop"), f("set_multicast_ttl"), f("set_option_bool"),
            f("set_option_int"), f("set_recv_timeout"), f("set_send_timeout"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["json"],
        fns: &[
            f("array_first"), f("array_first_span"), f("array_next"),
            f("array_next_span"), f("escape_string"), f("find_bool_field"),
            f("find_field_range_in"), f("find_field_raw"), f("find_field_raw_in"),
            f("find_int_field"), f("find_string_field"), f("iter_find_bool_field"),
            f("iter_find_field_range"), f("iter_find_field_raw"),
            f("iter_find_int_field"), f("iter_find_string_field"),
            f("iter_find_string_field_range"), f("iter_substring"),
            f("next_non_ws"), f("next_quote_or_bs"), f("next_struct_or_quote"),
            f("obj_key_eq"), f("obj_key_len"), f("obj_key_string"), f("obj_value_bool"),
            f("obj_value_float"), f("obj_value_int"), f("obj_value_raw"),
            f("obj_value_string"), f("object_first"), f("object_next"),
            f("unescape_string"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["math"],
        fns: &[
            f("acos"), f("asin"), f("atan"), f("atan2"), f("ceil"), f("cos"), f("exp"),
            f("float_to_int"), f("floor"), f("inf"), f("int_to_float"), f("is_nan"),
            f("log"), f("nan"), f("pow"), f("round"), f("sin"), f("sqrt"), f("tan"), f("tanh"),
            f("trunc"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["os"],
        fns: &[
            f("getrandom"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["process"],
        fns: &[
            f("__kill_escalate"), f("__pipe_read"), f("__pipe_write"),
            f("__signal_pid"), f("__spawn"), f("__try_wait_pid"), f("__wait_pid"),
            f("dump_arena_residency"), f("dump_pool_residency"),
            f("exit"), f("kill"), f("pid"), f("read_stderr"), f("read_stdout"),
            f("rss_bytes"), f("run"), f("signal"), f("spawn"), f("try_wait"), f("wait"),
            f("write_stdin"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["metrics"],
        fns: &[
            f("counter"), f("gauge"), f("histogram"), f("labels_append"),
            f("labels_empty"), f("labels_one"), f("labels_two"), f("metric_key"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["rand"],
        fns: &[
            f("next_int"), f("seed_from_time"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["str"],
        fns: &[
            f("builder_append"), f("builder_finish"), f("builder_len"),
            f("builder_new"), f("byte_at_unchecked"), f("can_parse_decimal"),
            f("can_parse_float"), f("can_parse_int"), f("clone"), f("from_bytes"),
            f("index_of"), f("lower"), f("pad_left"), f("pad_right"), f("parse_decimal"),
            f("parse_float"), f("parse_int"), f("range_eq"), f("range_parse_decimal"),
            f("range_parse_int"), f("repeat"), f("replace"), f("substring"), f("trim"),
            f("upper"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["term"],
        fns: &[
            f("__raw_disable"), f("__raw_enable"), f("__size_packed"), f("is_tty"),
            f("size"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["test"],
        fns: &[
            f("assert"), f("assert_eq_int"), f("assert_eq_str"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["text"],
        fns: &[
            f("is_alnum"), f("is_alpha"), f("is_digit"), f("is_whitespace"),
            f("is_word_char"), f("md_to_html"), f("tokenize_words_into"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["text", "base64"],
        fns: &[
            f("decode"), f("encode"), f("url_encode"),
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["time"],
        fns: &[
            f("monotonic"), f("monotonic_ns"), f("now"), f("sleep"), f("time_from_unix"),
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

/// The stage-1 check: for a call whose callee is a `std::` path,
/// return an error message when the namespace is tabled and the fn
/// name is unknown. `None` means "fine or not our business".
pub fn unknown_fn_error(segs: &[&str]) -> Option<String> {
    if is_locus_path(segs) {
        return None;
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
macro_rules! sig {
    ($ns:expr, $name:literal, [$($p:ident),*], $ret:ident) => {
        FnSig { ns: $ns, name: $name,
                params: &[$(SigTy::$p),*], ret: SigTy::$ret,
                fallible: None }
    };
    ($ns:expr, $name:literal, [$($p:ident),*], $ret:ident, $err:literal) => {
        FnSig { ns: $ns, name: $name,
                params: &[$(SigTy::$p),*], ret: SigTy::$ret,
                fallible: Some($err) }
    };
}

const NS_MATH: &[&str] = &["math"];
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
const NS_FILE: &[&str] = &["io", "file"];
const NS_TCP: &[&str] = &["io", "tcp"];
const NS_TLS: &[&str] = &["io", "tls"];
const NS_UDP: &[&str] = &["io", "udp"];
const NS_TEXT: &[&str] = &["text"];
const NS_TERM: &[&str] = &["term"];
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
    sig!(NS_TIME, "monotonic", [], Duration),
    sig!(NS_TIME, "monotonic_ns", [], Int),
    sig!(NS_TIME, "sleep", [Duration], Unit),
    sig!(NS_TIME, "now", [], Int),
    sig!(NS_TIME, "time_from_unix", [Int], Time),
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
    sig!(NS_STR, "index_of", [Str, Str], Int),
    sig!(NS_STR, "lower", [Str], Str),
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
    sig!(NS_FS, "write_file_append", [Str, Str], Int, "IoError"),
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
    sig!(NS_TCP, "accept_one", [Int], Int, "IoError"),
    sig!(NS_TCP, "close_fd", [Int], Int),
    sig!(NS_TCP, "recv_into", [Int, Any, Int], Int),
    sig!(NS_TCP, "recv_stamped_into", [Int, Any, Int], Int),
    sig!(NS_TCP, "last_recv_kernel_ns", [], Int),
    sig!(NS_TCP, "last_recv_user_ns", [], Int),
    sig!(NS_TCP, "set_nodelay", [Int, Bool], Unit, "IoError"),
    sig!(NS_TCP, "set_rx_timestamps", [Int, Bool], Unit, "IoError"),
    sig!(NS_TLS, "connect", [Str, Int], Int, "IoError"),
    sig!(NS_TLS, "upgrade", [Int, Str, Bool], Int, "IoError"),
    sig!(NS_TLS, "send_bytes", [Int, Bytes], Int),
    sig!(NS_TLS, "recv_bytes", [Int, Int], Bytes),
    sig!(NS_TLS, "recv_into", [Int, Any, Int], Int),
    sig!(NS_TLS, "close", [Int], Int),
    sig!(NS_UDP, "bind", [Str, Int], Int, "IoError"),
    sig!(NS_UDP, "send", [Int, Str, Int, Any], Unit, "IoError"),
    sig!(NS_UDP, "recv", [Int, Int], Bytes, "IoError"),
    sig!(NS_UDP, "recv_into", [Int, Any, Int], Int),
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
pub fn effects_for(segs: &[&str]) -> Option<EffectSet> {
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
