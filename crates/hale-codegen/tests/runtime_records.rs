//! GH #1592 S2: the runtime writes its diagnostics as the records
//! `std::log`'s sinks write, and the two renderers are one format.
//!
//! The Hale sinks render an event in `log.hl` (`__log_render`); the C
//! runtime renders its own diagnostics in `lotus_diag_render`. A record
//! is held to the same bytes both ways: each event below goes through a
//! `StdoutSink` and through the runtime's renderer (its test surface,
//! `lotus_diag_render_to_stdout`, reached by `@ffi`), in every format,
//! and the lines must match, the time aside (each side stamps its own
//! second), whose shape is checked on its own.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// (level, logger, message, logfmt fields): the cases where the two
/// renderers could part — quoting, escapes, a line break, control
/// bytes, a bare key, an empty value, UTF-8.
const EVENTS: &[(i64, &str, &str, &str)] = &[
    (1, "lotus.tcp", "accepted", ""),
    (2, "lotus.tcp", "accept failed", "op=\"lotus_tcp_accept_one: accept\" errno=9 error=\"Bad file descriptor\""),
    (3, "lotus.tls", "SSL_write failed", "err=5 detail=\"a=b\" path=C:\\\\x"),
    (4, "lotus.bus", "line\nbreak", "why=\"two\\nlines\" flag empty="),
    (5, "lotus.drain", "tab\there \"quoted\"", "ctl=\"\x01\x1f\" utf8=héllo"),
];

/// The raw values of `stdlib_log::ESCAPES`, in its order: the table the
/// Hale sinks are held to, held to the runtime's renderer here, each as
/// a message and, through its logfmt token, as a field.
const ESCAPE_VALUES: &[&str] = &["left\tright", "cr\rhere", "back\\slash", "say \"hi\"", "ctl\u{1}x", "del\u{7f}x", "", "plain"];

fn events() -> Vec<(i64, String, String, String)> {
    assert_eq!(ESCAPE_VALUES.len(), super::stdlib_log::ESCAPES.len());
    let mut all: Vec<(i64, String, String, String)> =
        EVENTS.iter().map(|(l, g, m, f)| (*l, g.to_string(), m.to_string(), f.to_string())).collect();
    for (raw, (_, token, _)) in ESCAPE_VALUES.iter().zip(super::stdlib_log::ESCAPES) {
        all.push((2, "lotus.test".to_string(), raw.to_string(), format!("v={token}")));
    }
    all
}

fn program(format: &str) -> String {
    let mut body = String::new();
    for (level, logger, msg, fields) in events() {
        let (level, logger, msg, fields) = (&level, logger.as_str(), msg.as_str(), fields.as_str());
        let lit = hale_lit;
        let method = match level {
            1 => "info_kv",
            2 => "warn_kv",
            3 => "error_kv",
            4 => "debug_kv",
            _ => "trace_kv",
        };
        body.push_str(&format!(
            "    {{ let log = std::log::Logger {{ name: {} }}; log.{method}({}, {}); }}\n",
            lit(logger),
            lit(msg),
            lit(fields)
        ));
        body.push_str(&format!(
            "    lotus_diag_render_to_stdout({}, {level}, {}, {}, {}, std::time::now());\n",
            hale_lit(format),
            lit(logger),
            lit(msg),
            lit(fields)
        ));
    }
    format!(
        "@ffi(\"c\") fn lotus_diag_render_to_stdout(format: String, level: Int, logger: String, msg: String, fields: String, ts: Int) -> ();\n\
         fn main() {{\n    std::log::StdoutSink {{ format: {}, min_severity: 0 }};\n{body}}}\n",
        hale_lit(format)
    )
}

/// `s` as a Hale string literal: `\\`, `\"`, `\n`, `\t`, any other
/// control byte as `\xNN`, the rest as it is.
fn hale_lit(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The line with its time taken out, and the time.
fn untimed(format: &str, line: &str) -> (String, String) {
    match format {
        "logfmt" => {
            let rest = line.strip_prefix("time=").unwrap_or_else(|| panic!("{line:?}"));
            let (t, rest) = rest.split_once(' ').expect("fields after the time");
            (rest.to_string(), t.to_string())
        }
        "json" => {
            let rest = line.strip_prefix("{\"time\":\"").unwrap_or_else(|| panic!("{line:?}"));
            let (t, rest) = rest.split_once('"').expect("the time's close");
            (rest.to_string(), t.to_string())
        }
        _ => (line.to_string(), String::new()),
    }
}

#[test]
fn the_runtime_and_the_sinks_render_a_record_to_the_same_bytes() {
    for format in ["logfmt", "json", "text"] {
        let bin = harness::unique_bin(&format!("runtime_records_{format}"));
        build_opts::build_source(&program(format), &bin, &build_opts::options()).expect("build");
        let out = Command::new(&bin).env_remove("HALE_LOG_FORMAT").output().expect("run");
        let _ = std::fs::remove_file(&bin);
        assert!(out.status.success(), "{format}: {}", String::from_utf8_lossy(&out.stderr));
        // the text format keeps WARN and ERROR on stderr: hold the
        // runtime's lines to the sink's on whichever stream they share
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        let lines: Vec<&str> = text.lines().collect();
        if format == "text" {
            let sink: Vec<&str> = err.lines().chain(lines.iter().copied().filter(|l| !l.starts_with("[WARN") && !l.starts_with("[ERROR"))).collect();
            let runtime: Vec<&str> = lines.iter().copied().filter(|l| l.starts_with("[WARN") || l.starts_with("[ERROR")).collect();
            for r in &runtime {
                assert!(sink.contains(r), "text: {r:?} is a line the sink wrote too: {sink:?}");
            }
            continue;
        }
        assert_eq!(lines.len(), events().len() * 2, "{format}: a line per event per side: {text}");
        for pair in lines.chunks(2) {
            let (sink, sink_time) = untimed(format, pair[0]);
            let (runtime, runtime_time) = untimed(format, pair[1]);
            assert_eq!(runtime, sink, "{format}: the runtime's record and the sink's");
            for t in [&sink_time, &runtime_time] {
                assert!(t.len() == 20 && t.ends_with('Z') && t.as_bytes()[10] == b'T', "{format}: RFC 3339 UTC: {t:?}");
            }
        }
    }
}
