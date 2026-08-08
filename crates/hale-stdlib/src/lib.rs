//! The Hale-source half of the standard library.
//!
//! A stdlib function can exist in three places, and for a long time
//! only two of them were reachable from the analyzer:
//!
//!   1. `hale-types::stdlib_surface` — the registry (typecheck
//!      signatures + effect classes).
//!   2. codegen `["std", ns, fn]` dispatch arms — native lowering.
//!   3. **these `.hl` modules** — stdlib written in Hale itself.
//!
//! (3) used to live as a `const` inside `hale-codegen`, downstream of
//! `hale-types`, so the effect analysis structurally could not see
//! these bodies. The consequence was a soundness hole: a method call
//! on a stdlib locus (`file.read_all()`, `resolver.get(…)`) was an
//! unresolved callgraph edge, so `@no_syscall` and friends passed
//! over real I/O. Hoisting the source into its own upstream crate
//! lets BOTH the compiler and the analyzer read it, and makes the
//! effects of these modules *inferred from their bodies* rather than
//! hand-transcribed into a table that drifts.
//!
//! This crate is deliberately dependency-free: it is source text and
//! one name-mapping table, nothing else.

/// Bundled Hale source for the stdlib. m73a established the
/// concat-with-user-source mechanism: the parsed stdlib `Program`
/// has its `items` appended to the user's `Program.items` before
/// `lower_program` runs, so each stdlib locus sits in `user_loci`
/// exactly like user-declared loci. Path-qualified references
/// (`std::io::tcp::Listener`) are rewritten at struct-literal
/// codegen sites to the mangled locus names declared in this
/// source via the `PATH_RENAMES` table below.
///
/// m93 split the single stdlib.hl into one file per domain.
/// Order matters: pass A1 walks loci in source order and resolves
/// each locus's param types as it goes. Listener references
/// Stream, so io_tcp.hl (which declares both, Stream first) lands
/// before http.hl (which references Stream in fn signatures).
/// core.hl lands first because text.hl depends on its
/// __replace_all / __html_escape helpers. test.hl is standalone
/// and could go anywhere — it ends up last by convention.
pub const AP_SOURCE: &str = concat!(
    include_str!("../hl/core.hl"),
    "\n",
    include_str!("../hl/io_tcp.hl"),
    "\n",
    // io_udp.hl declares the `Reader` handle and references `IoError`
    // (declared in io_tcp.hl) in its fn signatures, so it must land
    // after io_tcp.hl for pass A1 type resolution. Only path-call
    // primitives otherwise, so its position past io_tcp is the only
    // ordering constraint.
    include_str!("../hl/io_udp.hl"),
    "\n",
    include_str!("../hl/http.hl"),
    "\n",
    // Client surface (promoted from pond/http/client, 2026-07-17).
    // References http.hl's __http_find_header_in_block and
    // io_tcp.hl's IoError, so it lands after both.
    include_str!("../hl/http_client.hl"),
    "\n",
    // std::metrics (promoted from pond/metrics, 2026-07-18) —
    // Endpoint references std::http types, so it lands after http.hl.
    include_str!("../hl/metrics.hl"),
    "\n",
    include_str!("../hl/text.hl"),
    "\n",
    // std::secret (GH #436) — sealed key-holding loci. Only path
    // calls (std::crypto / std::bytes / std::env / std::io::fs /
    // std::str), which resolve at codegen time, so order is free.
    include_str!("../hl/secret.hl"),
    "\n",
    include_str!("../hl/test.hl"),
    "\n",
    include_str!("../hl/log.hl"),
    "\n",
    // m96: tree-sitter substrate. Standalone — references only
    // path-call primitives (`std::ts::*`) plus core builtins
    // (`println`, while, assignment), so order is flexible.
    // Lands last by convention.
    include_str!("../hl/ts.hl"),
    "\n",
    // Post-m102: language-agnostic AST query interface.
    // Wraps `std::ts::*` + per-language node-kind strings
    // behind a single `Lang` locus. Depends on `std::ts::*`
    // path calls and `std::str::index_of`; both are path
    // calls that resolve at codegen time, so source-order
    // dependency on ts.hl is just stylistic — the
    // path-call resolution is independent of bundle order.
    include_str!("../hl/lang.hl"),
    "\n",
    // Corpus-extraction pass: cross-cutting helpers lifted from
    // the apps/ tree into the std seed so they stop being
    // hand-rolled per consumer. Each is a namespace lotus
    // (empty params, methods only). Order between these is
    // flexible — they reference only path-call primitives.
    include_str!("../hl/iter.hl"),
    "\n",
    // tagged.hl depends on iter.hl (Lines is used internally in
    // every Accumulator method), so it must land after.
    include_str!("../hl/tagged.hl"),
    "\n",
    // name.hl is independent — only uses std::str::index_of.
    include_str!("../hl/name.hl"),
    "\n",
    // json.hl depends on iter.hl for build_array's line walk.
    include_str!("../hl/json.hl"),
    "\n",
    // yaml.hl depends on iter.hl for Reader's line walks.
    // Mirrors json.hl's shape (Builder is a namespace lotus
    // returning Strings).
    include_str!("../hl/yaml.hl"),
    "\n",
    // cli.hl is independent — only uses std::str::index_of,
    // std::str::parse_int / can_parse_int, and std::env::*
    // path calls. Lands after the other corpus helpers by
    // convention.
    include_str!("../hl/cli.hl"),
    "\n",
    // source.hl depends on iter.hl (Lines for entry iteration)
    // and references std::lang::Lang in its on_file fn-pointer
    // type — lang.hl must be declared before this file's parse
    // pass A1 resolves param types. lang.hl lands at line ~385
    // above, so source.hl lands here without issue.
    include_str!("../hl/source.hl"),
    "\n",
    // C2 — std::process. References std::io::tcp::__close_fd for
    // pipe-fd cleanup in Child.dissolve(), so io_tcp.hl (declared
    // near the top of this concat) must precede it. Independent
    // of other stdlib files otherwise.
    include_str!("../hl/process.hl"),
    "\n",
    // std::bus::Adapter interface contract for user-supplied
    // protocol-layer transports. No concrete impls live in std;
    // the runtime side of the binding variant lands in Wave B
    // of the bus-transport redesign (gated on F.20 Phase B
    // interface-value storage). Standalone — references only
    // Bytes and String, both core types.
    include_str!("../hl/bus.hl"),
    "\n",
    // std::io::file::File — held-open file I/O locus that
    // complements the one-shot std::io::fs::* path-calls.
    // Lifecycle (birth/dissolve) closes the fd at scope-exit
    // per the m82 deferred-dissolve mechanism. Returned String
    // data lives in the bus payload arena (program-lifetime).
    include_str!("../hl/file.hl"),
    "\n",
    // std::bytes::BytesBuilder — long-lived growing-byte-buffer
    // locus. Replaces the prior `std::bytes::builder_*` free-fn
    // surface so the typechecker can distinguish builder handles
    // from regular Bytes blobs (the two have incompatible runtime
    // ABIs). Lifecycle (birth/dissolve) wraps malloc /free of the
    // underlying lotus_str_builder_t header + buffer. References
    // only path-call primitives (`std::bytes::builder::__*`) that
    // resolve at codegen time; independent of order.
    include_str!("../hl/bytes_builder.hl"),
    "\n",
    // std::io::MirrorRing (#3): double-mmap wrap-free ring. Calls the
    // std::io::mirror::__* path-call primitives; order-independent.
    include_str!("../hl/mirror_ring.hl"),
    "\n",
    // std::term::RawMode guard locus (pond P4 stage 3). Calls the
    // std::term::__raw_* path-call primitives; order-independent.
    include_str!("../hl/term.hl"),
);

/// Maps each user-facing stdlib path (locus OR type) to the
/// mangled name declared in `AP_SOURCE`. The mangled
/// prefix (`__StdIo...`, `__StdHttp...`) makes collision with
/// user-declared identifiers impossible at v0. Each entry is
/// `&[&"std", ...]` → flat string. Whether the resolved name
/// refers to a locus or a type is determined downstream by
/// looking it up in `user_loci` / `user_types` — this table
/// is just the path → name mapping. Keep sorted by path for
/// review.
///
/// `hale-types` reads this to turn a struct-literal path
/// (`std::io::file::File`) into the mangled locus name the
/// bodies above actually declare, so a handle-method call
/// resolves to a real callgraph node.
pub const PATH_RENAMES: &[(&[&str], &str)] = &[
    (&["std", "bus", "Adapter"], "__StdBusAdapter"),
    // GH #233 steps 3-4: the connect-side substrate transport is
    // user-nameable so main can declare
    // `on_failure(t: std::bus::UnixTransport, err: ClosureViolation)`.
    // (The listen side re-arms instead of getting lost, so it
    // stays internal.)
    (&["std", "bus", "UnixTransport"], "__StdBusUnixConnectTransport"),
    (&["std", "bytes", "BytesBuilder"], "__StdBytesBytesBuilder"),
    (&["std", "cli", "Resolver"], "__StdCliResolver"),
    (&["std", "http", "Handler"], "__StdHttpHandler"),
    (&["std", "http", "Request"], "__StdHttpRequest"),
    (&["std", "http", "Response"], "__StdHttpResponse"),
    (&["std", "http", "Server"], "__StdHttpServer"),
    // Router battery (promoted from pond/router, 2026-07-17). The
    // free-fn surface (path_param / query_param) routes through
    // this table to its bare implementations in http.hl, same as
    // the std::process fns below.
    (&["std", "http", "Router"], "__StdHttpRouter"),
    (&["std", "http", "Context"], "__StdHttpContext"),
    (&["std", "http", "RouteParams"], "__StdHttpRouteParams"),
    (&["std", "http", "RouteHandler"], "__StdHttpRouteHandler"),
    (&["std", "http", "Middleware"], "__StdHttpMiddleware"),
    (&["std", "http", "RouteEntry"], "__StdHttpRouteEntry"),
    (&["std", "http", "NotFound404"], "__StdHttpNotFound404"),
    (&["std", "http", "path_param"], "__http_path_param"),
    (&["std", "http", "query_param"], "__http_query_param"),
    // Client surface (promoted from pond/http/client, 2026-07-17).
    (&["std", "http", "Client"], "__StdHttpClient"),
    (&["std", "http", "ClientRequest"], "__StdHttpClientRequest"),
    (&["std", "http", "ClientResponse"], "__StdHttpClientResponse"),
    (&["std", "http", "HttpError"], "__StdHttpError"),
    (&["std", "http", "Url"], "__StdHttpUrl"),
    (&["std", "http", "get"], "__http_client_get"),
    (&["std", "http", "post"], "__http_client_post"),
    (&["std", "http", "request"], "__http_client_request"),
    (&["std", "http", "parse_url"], "__http_parse_url"),
    (&["std", "io", "file", "File"], "__StdIoFileFile"),
    (&["std", "io", "MirrorRing"], "__StdIoMirrorRing"),
    (&["std", "io", "file", "open"], "__std_io_file_open"),
    (&["std", "io", "file", "read_line"], "__std_io_file_read_line"),
    (&["std", "io", "file", "at_eof"], "__std_io_file_at_eof"),
    (&["std", "io", "file", "write_bytes"], "__std_io_file_write_bytes"),
    (&["std", "io", "file", "write_line"], "__std_io_file_write_line"),
    (&["std", "io", "file", "seek"], "__std_io_file_seek"),
    (&["std", "term", "RawMode"], "__StdTermRawMode"),
    (&["std", "term", "TermSize"], "__StdTermTermSize"),
    (&["std", "term", "size"], "__std_term_size"),
    (&["std", "io", "tcp", "Listener"], "__StdIoTcpListener"),
    (&["std", "io", "tcp", "Stream"], "__StdIoTcpStream"),
    (&["std", "io", "tcp", "LogEvent"], "__StdIoTcpLogEvent"),
    // hale-bun upstream item 4b: public raw-fd send for takeover
    // consumers (the write-side companion to `close_fd`).
    (&["std", "io", "tcp", "send_fd"], "__std_io_tcp_send_fd"),
    (&["std", "io", "udp", "Reader"], "__StdIoUdpReader"),
    (&["std", "iter", "Lines"], "__StdIterLines"),
    (&["std", "json", "ArrayIter"], "__JsonArrayIter"),
    (&["std", "json", "ArrayIterSpan"], "__JsonArrayIterSpan"),
    (&["std", "json", "ObjectIterSpan"], "__JsonObjectIterSpan"),
    (&["std", "json", "Builder"], "__StdJsonBuilder"),
    (&["std", "json", "JsonFieldRange"], "__JsonFieldRange"),
    (&["std", "lang", "Lang"], "__StdLangLang"),
    (&["std", "lang", "Morpheme"], "__StdLangMorpheme"),
    (&["std", "log", "ConsoleSink"], "__StdLogConsoleSink"),
    (&["std", "log", "FileSink"], "__StdLogFileSink"),
    (&["std", "log", "LogEvent"], "__StdLogEvent"),
    (&["std", "log", "Logger"], "__StdLogLogger"),
    (&["std", "log", "StdoutSink"], "__StdLogStdoutSink"),
    // std::secret (GH #436).
    (&["std", "secret", "Credential"], "__StdSecretCredential"),
    (&["std", "secret", "Signer"], "__StdSecretSigner"),
    // std::metrics (promoted from pond/metrics, 2026-07-18).
    (&["std", "metrics", "Counter"], "__StdMetricsCounter"),
    (&["std", "metrics", "Endpoint"], "__StdMetricsEndpoint"),
    (&["std", "metrics", "Gauge"], "__StdMetricsGauge"),
    (&["std", "metrics", "Histogram"], "__StdMetricsHistogram"),
    (&["std", "metrics", "HistogramData"], "__StdMetricsHistogramData"),
    (&["std", "metrics", "HistogramList"], "__StdMetricsHistogramList"),
    (&["std", "metrics", "Labels"], "__StdMetricsLabels"),
    (&["std", "metrics", "MetricEntry"], "__StdMetricsEntry"),
    (&["std", "metrics", "MetricMap"], "__StdMetricsMap"),
    (&["std", "metrics", "Registry"], "__StdMetricsRegistry"),
    (&["std", "metrics", "counter"], "__metrics_counter"),
    (&["std", "metrics", "gauge"], "__metrics_gauge"),
    (&["std", "metrics", "histogram"], "__metrics_histogram"),
    (&["std", "metrics", "labels_append"], "__metrics_labels_append"),
    (&["std", "metrics", "labels_empty"], "__metrics_labels_empty"),
    (&["std", "metrics", "labels_one"], "__metrics_labels_one"),
    (&["std", "metrics", "labels_two"], "__metrics_labels_two"),
    (&["std", "metrics", "metric_key"], "__metrics_key"),
    (&["std", "name", "Convention"], "__StdNameConvention"),
    // C2 — subprocess. ProcessOutput is the captured-output struct
    // returned by `std::process::run`; Child is the lifecycle-bound
    // handle returned by `std::process::spawn`. The free-fn surface
    // (spawn/wait/kill/write_stdin/read_stdout/read_stderr) is
    // routed through this table too: each path-call resolves to
    // its bare `__std_process_*` implementation in process.hl.
    (&["std", "process", "Child"], "__StdProcessChild"),
    (&["std", "process", "ProcessOutput"], "__StdProcessOutput"),
    (&["std", "process", "kill"], "__std_process_kill"),
    (&["std", "process", "read_stderr"], "__std_process_read_stderr"),
    (&["std", "process", "read_stdout"], "__std_process_read_stdout"),
    (&["std", "process", "signal"], "__std_process_signal"),
    (&["std", "process", "spawn"], "__std_process_spawn"),
    (&["std", "process", "try_wait"], "__std_process_try_wait"),
    (&["std", "process", "wait"], "__std_process_wait"),
    (&["std", "process", "write_stdin"], "__std_process_write_stdin"),
    (&["std", "source", "Walk"], "__StdSourceWalk"),
    // v1.x polish (2026-05-20): qualified `std::str::ParseError`
    // resolves to the same bare ParseError the stdlib's parse_*
    // fns inject. Lets users disambiguate explicitly in fn
    // signatures and `or raise as e: std::str::ParseError`
    // bindings — useful when a project also has its own
    // local error types.
    (&["std", "str", "ParseError"], "ParseError"),
    (&["std", "tagged", "Accumulator"], "__StdTaggedAccumulator"),
    (&["std", "text", "Sink"], "__StdTextSink"),
    (&["std", "text", "StdoutSink"], "__StdTextStdoutSink"),
    (&["std", "text", "StringSink"], "__StdTextStringSink"),
    (&["std", "text", "FileSink"], "__StdTextFileSink"),
    (&["std", "yaml", "Builder"], "__StdYamlBuilder"),
    (&["std", "yaml", "Reader"], "__StdYamlReader"),
];
