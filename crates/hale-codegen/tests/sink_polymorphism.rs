//! std::text::Sink stdlib migration — polymorphism end-to-end.
//!
//! Phase 1 of the post-F.20-Phase-B roadmap replaces the tagged
//! `__StdTextSink` (branching on `dest: String`) with one
//! `Sink` interface + three concrete loci (`StdoutSink`,
//! `StringSink`, `FileSink`). These tests prove the same
//! `render(s: Sink)` fn signature dispatches to each variant's
//! body via the F.20 Phase B vtable, and that the variants'
//! observable behavior matches their tagged-locus predecessors.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_sink_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn stdout_sink_streams_to_stdout() {
    let src = r#"
        fn render(s: std::text::Sink) {
            s.line("hello");
            s.line("world");
        }

        fn main() {
            let s = std::text::StdoutSink { };
            render(s);
        }
    "#;
    let (stdout, status) = build_and_run("stdout", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("hello\nworld\n"), "got: {:?}", stdout);
}

#[test]
fn string_sink_accumulates_into_buffer() {
    let src = r#"
        fn render(s: std::text::Sink) {
            s.line("alpha");
            s.line("beta");
        }

        fn main() {
            let s = std::text::StringSink { };
            render(s);
            println("buf=", s.result());
        }
    "#;
    let (stdout, status) = build_and_run("string", src);
    assert!(status.success(), "exit: {:?}", status);
    // StringSink doesn't print during render — its result() call
    // is the only line that hits stdout.
    assert!(
        stdout.contains("buf=alpha\nbeta\n"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn same_render_fn_dispatches_to_different_sinks() {
    // The load-bearing test: one fn taking a Sink interface,
    // called twice with two different concrete sinks. If the
    // F.20 Phase B vtable dispatch is broken, the same body
    // would run twice or one would be silent.
    let src = r#"
        fn write_one(s: std::text::Sink, msg: String) {
            s.line(msg);
        }

        fn main() {
            let out = std::text::StdoutSink { };
            let buf = std::text::StringSink { };
            write_one(out, "to-stdout");
            write_one(buf, "to-string");
            // out wrote to stdout already; buf accumulated.
            println("captured=", buf.result());
        }
    "#;
    let (stdout, status) = build_and_run("polymorphic", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(
        stdout.contains("to-stdout\n"),
        "stdout sink didn't write: {:?}",
        stdout
    );
    assert!(
        stdout.contains("captured=to-string\n"),
        "string sink didn't accumulate: {:?}",
        stdout
    );
}

#[test]
fn file_sink_appends_to_path() {
    // FileSink writes via std::io::fs::write_file_append (m96)
    // so each write streams to disk. Test pattern: write to a
    // temp file from inside an Hale program, then read it
    // back from the harness to assert content.
    let tmpfile = std::env::temp_dir().join("hale_test_filesink.txt");
    let _ = std::fs::remove_file(&tmpfile);
    let path_str = tmpfile.to_string_lossy().to_string();

    let src = format!(
        r#"
        fn render(s: std::text::Sink) {{
            s.line("first");
            s.write("partial");
            s.newline();
            s.line("third");
        }}

        fn main() {{
            let s = std::text::FileSink {{ path: "{}" }};
            render(s);
            println("done");
        }}
    "#,
        path_str
    );
    let (stdout, status) = build_and_run("file", &src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("done"), "got: {:?}", stdout);

    let written = std::fs::read_to_string(&tmpfile).expect("file written");
    let _ = std::fs::remove_file(&tmpfile);
    assert_eq!(
        written, "first\npartial\nthird\n",
        "FileSink content mismatch: {:?}",
        written
    );
}

#[test]
fn three_sinks_one_render_consistent_output() {
    // Same render fn, three sinks, identical content emitted.
    // Stdout and File should land the same bytes; String's
    // result() exposes the same bytes for assertion.
    let tmpfile = std::env::temp_dir().join("hale_test_filesink_consistency.txt");
    let _ = std::fs::remove_file(&tmpfile);
    let path_str = tmpfile.to_string_lossy().to_string();

    let src = format!(
        r#"
        fn render(s: std::text::Sink) {{
            s.write("a=");
            s.line("1");
            s.line("b");
        }}

        fn main() {{
            let out = std::text::StdoutSink {{ }};
            let buf = std::text::StringSink {{ }};
            let f   = std::text::FileSink {{ path: "{}" }};
            render(out);
            render(buf);
            render(f);
            println("string-result=", buf.result());
        }}
    "#,
        path_str
    );
    let (stdout, status) = build_and_run("consistency", &src);
    assert!(status.success(), "exit: {:?}", status);
    // Stdout-sink wrote `a=1\nb\n`.
    assert!(stdout.contains("a=1\nb\n"), "stdout: {:?}", stdout);
    // String-sink result matches.
    assert!(
        stdout.contains("string-result=a=1\nb\n"),
        "string: {:?}",
        stdout
    );
    // File-sink wrote the same bytes.
    let written = std::fs::read_to_string(&tmpfile).expect("file");
    let _ = std::fs::remove_file(&tmpfile);
    assert_eq!(written, "a=1\nb\n", "file: {:?}", written);
}
