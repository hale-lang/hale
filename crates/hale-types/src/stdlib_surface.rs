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
