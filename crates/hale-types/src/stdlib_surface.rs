//! Typecheck M3 stage 1 (2026-07-02): the stdlib path-call NAME
//! surface — typo detection for `std::<ns>::<fn>(...)` calls.
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
pub struct NsSurface {
    /// Path segments after `std` identifying the namespace
    /// (e.g. `["io", "fs"]` for `std::io::fs`). Longest match wins,
    /// so `std::io::fs` shadows a hypothetical `std::io` table for
    /// three-segment paths.
    pub ns: &'static [&'static str],
    /// Accepted function names within the namespace.
    pub fns: &'static [&'static str],
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
    &["std", "http", "Handler"],
    &["std", "http", "Request"],
    &["std", "http", "Response"],
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
    &["std", "log", "LogEvent"],
    &["std", "log", "Logger"],
    &["std", "log", "StdoutSink"],
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
            "__local_dispatch",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["bytes"],
        fns: &[
            "__is_alloc_fail", "at", "clone", "concat", "find_byte",
            "from_int", "from_string", "read_f32_le", "read_f64_be",
            "read_f64_le", "read_i16_be", "read_i16_le", "read_i32_be",
            "read_i32_le", "read_i64_be", "read_i64_le", "read_i8",
            "read_u16_be", "read_u16_le", "read_u32_be", "read_u32_le",
            "read_u64_be", "read_u64_le", "read_u8", "slice",
            "write_f32_le", "write_f64_be", "write_f64_le", "write_i16_be",
            "write_i16_le", "write_i32_be", "write_i32_le", "write_i64_be",
            "write_i64_le", "write_i8", "write_u16_be", "write_u16_le",
            "write_u32_be", "write_u32_le", "write_u64_be", "write_u64_le",
            "write_u8",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["bytes", "builder"],
        fns: &[
            "__append", "__append_f32", "__append_f64", "__append_pad",
            "__append_scalar", "__append_slice", "__append_str", "__clear",
            "__finish", "__free", "__len", "__new", "__shift_front",
            "__snapshot", "__text_view", "__view", "__xor_mask_into",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["crypto"],
        fns: &[
            "crc32", "ecdsa_p256_sign", "ecdsa_p256_verify", "hmac_sha256",
            "hmac_sha512", "sha1", "sha256", "sha512",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["decimal"],
        fns: &[
            "to_float",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["diag"],
        fns: &[
            "heap_alloc_count", "syscall_count",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["env"],
        fns: &[
            "arg", "arg_or", "args_count", "var", "var_exists",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["http"],
        fns: &[
            "header", "parse_request", "write_response",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "file"],
        fns: &[
            "__at_eof", "__close", "__open", "__read_line", "__seek",
            "__write_bytes", "at_eof", "open", "read_line", "seek",
            "write_bytes", "write_line",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "fs"],
        fns: &[
            "extension", "file_exists", "file_size", "list_dir",
            "list_dir_at", "list_dir_count", "mkdir", "mktemp",
            "read_bytes", "read_file", "rename", "unlink", "write_file",
            "write_file_append",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "stdin"],
        fns: &[
            "read_byte", "read_line", "read_line_status",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "stdout"],
        fns: &[
            "write_bytes",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "tcp"],
        fns: &[
            "__accept_one", "__close_fd", "__connect", "__listen_socket",
            "__recv", "__recv_bytes", "__send", "__send_bytes",
            "__set_recv_timeout_ns", "__shutdown_listen_socket",
            "accept_one", "close_fd", "connect", "last_recv_kernel_ns",
            "last_recv_user_ns", "listen_socket", "recv_into",
            "recv_stamped_into", "set_nodelay", "set_recv_timeout",
            "set_rx_timestamps", "set_send_timeout",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "tls"],
        fns: &[
            "close", "connect", "last_recv_kernel_ns", "last_recv_user_ns",
            "recv_bytes", "recv_into", "recv_stamped_into", "send_bytes",
            "set_nodelay", "set_recv_timeout", "set_rx_timestamps",
            "set_send_timeout",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["io", "udp"],
        fns: &[
            "__bind", "__close", "__recv", "__send", "bind", "close",
            "get_option_int", "join_group", "last_source_host",
            "last_source_port", "leave_group", "recv", "recv_into",
            "recv_with_source", "send", "set_multicast_iface",
            "set_multicast_loop", "set_multicast_ttl", "set_option_bool",
            "set_option_int", "set_recv_timeout", "set_send_timeout",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["json"],
        fns: &[
            "array_first", "array_first_span", "array_next",
            "array_next_span", "escape_string", "find_bool_field",
            "find_field_range_in", "find_field_raw", "find_field_raw_in",
            "find_int_field", "find_string_field", "iter_find_bool_field",
            "iter_find_field_range", "iter_find_field_raw",
            "iter_find_int_field", "iter_find_string_field",
            "iter_find_string_field_range", "iter_substring",
            "next_non_ws", "next_quote_or_bs", "next_struct_or_quote",
            "obj_key_eq", "obj_key_len", "obj_value_bool",
            "obj_value_float", "obj_value_int", "obj_value_raw",
            "obj_value_string", "object_first", "object_next",
            "unescape_string",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["math"],
        fns: &[
            "acos", "asin", "atan", "atan2", "ceil", "cos", "exp",
            "float_to_int", "floor", "inf", "int_to_float", "is_nan",
            "log", "nan", "pow", "round", "sin", "sqrt", "tan", "tanh",
            "trunc",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["os"],
        fns: &[
            "getrandom",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["process"],
        fns: &[
            "__kill_escalate", "__pipe_read", "__pipe_write", "__spawn",
            "__wait_pid", "dump_arena_residency", "dump_pool_residency",
            "exit", "kill", "pid", "read_stderr", "read_stdout",
            "rss_bytes", "run", "spawn", "wait", "write_stdin",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["rand"],
        fns: &[
            "next_int", "seed_from_time",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["str"],
        fns: &[
            "builder_append", "builder_finish", "builder_len",
            "builder_new", "byte_at_unchecked", "can_parse_decimal",
            "can_parse_float", "can_parse_int", "clone", "from_bytes",
            "index_of", "lower", "pad_left", "pad_right", "parse_decimal",
            "parse_float", "parse_int", "range_eq", "range_parse_decimal",
            "range_parse_int", "repeat", "replace", "substring", "trim",
            "upper",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["term"],
        fns: &[
            "__raw_disable", "__raw_enable", "__size_packed", "is_tty",
            "size",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["test"],
        fns: &[
            "assert", "assert_eq_int", "assert_eq_str",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["text"],
        fns: &[
            "is_alnum", "is_alpha", "is_digit", "is_whitespace",
            "is_word_char", "md_to_html", "tokenize_words_into",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["text", "base64"],
        fns: &[
            "decode", "encode", "url_encode",
        ],
        open_prefixes: &[],
    },
    NsSurface {
        ns: &["time"],
        fns: &[
            "monotonic", "monotonic_ns", "now", "sleep", "time_from_unix",
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
    for cand in surface.fns {
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
    if surface.fns.contains(&name) {
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
