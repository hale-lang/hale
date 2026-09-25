//! Wave B (bus-transport redesign) — adapter binding end-to-end.
//!
//! Tests the `Topic: MyLocus { ... };` binding form:
//!
//! 1. **Single subject, multiple publishes.** A producer publishes
//!    twice; the adapter's `send` fires twice with the bound
//!    subject.
//! 2. **Adapter carries state.** Adapter params survive registration
//!    (they're allocated in the program-lifetime payload arena via
//!    m90-style routing) and are readable inside `send`.
//! 3. **Two adapters, two subjects.** Independent adapter instances
//!    bound to different topics each see only their own subject's
//!    payloads.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!(
        "hale_adapter_binding_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn adapter_send_fires_on_publish() {
    let src = r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; subject: "beat"; }

        locus MyAdapter {
            params { label: String = "noname"; }
            fn send(subject: String, bytes: Bytes) {
                println("adapter[" + self.label + "] subject=" + subject);
            }
        }

        locus Producer {
            bus { publish Beat; }
            birth() {
                Beat <- Tick { n: 1 };
                Beat <- Tick { n: 2 };
            }
        }

        main locus App {
            bindings {
                Beat: MyAdapter { label: "T" };
            }
        }

        fn main() {
            App { };
            Producer { };
        }
    "#;
    let (stdout, status) = build_and_run("single", src);
    assert!(status.success(), "non-zero: {:?}", status);
    let count = stdout
        .lines()
        .filter(|l| l.contains("adapter[T] subject=beat"))
        .count();
    assert_eq!(count, 2, "expected 2 send calls; got stdout: {:?}", stdout);
}

#[test]
fn adapter_field_inits_reach_send_body() {
    // The adapter is instantiated with explicit field values; the
    // `send` body reads `self.label` and should see the bound
    // value, not the default. This confirms the field inits flow
    // through to the locus's params via the synthetic Expr::Struct
    // lowering.
    let src = r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; subject: "beat"; }

        locus MyAdapter {
            params { label: String = "default"; }
            fn send(subject: String, bytes: Bytes) {
                println("label=" + self.label);
            }
        }

        locus Producer {
            bus { publish Beat; }
            birth() {
                Beat <- Tick { n: 1 };
            }
        }

        main locus App {
            bindings {
                Beat: MyAdapter { label: "explicit-value" };
            }
        }

        fn main() {
            App { };
            Producer { };
        }
    "#;
    let (stdout, status) = build_and_run("field_init", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("label=explicit-value"),
        "expected init value, not default; got: {:?}",
        stdout
    );
    assert!(
        !stdout.contains("label=default"),
        "default leaked through: {:?}",
        stdout
    );
}

#[test]
fn two_adapters_two_subjects_route_independently() {
    let src = r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; subject: "beat"; }
        topic Pulse { payload: Tick; subject: "pulse"; }

        locus AdA {
            params { tag: String = "a"; }
            fn send(subject: String, bytes: Bytes) {
                println("A tag=" + self.tag + " subject=" + subject);
            }
        }

        locus AdB {
            params { tag: String = "b"; }
            fn send(subject: String, bytes: Bytes) {
                println("B tag=" + self.tag + " subject=" + subject);
            }
        }

        locus Producer {
            bus { publish Beat; publish Pulse; }
            birth() {
                Beat <- Tick { n: 1 };
                Pulse <- Tick { n: 2 };
            }
        }

        main locus App {
            bindings {
                Beat: AdA { tag: "alpha" };
                Pulse: AdB { tag: "beta" };
            }
        }

        fn main() {
            App { };
            Producer { };
        }
    "#;
    let (stdout, status) = build_and_run("two_adapters", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("A tag=alpha subject=beat"),
        "A should see beat: {:?}",
        stdout
    );
    assert!(
        stdout.contains("B tag=beta subject=pulse"),
        "B should see pulse: {:?}",
        stdout
    );
    assert!(
        !stdout.contains("A tag=alpha subject=pulse"),
        "A should NOT see pulse: {:?}",
        stdout
    );
    assert!(
        !stdout.contains("B tag=beta subject=beat"),
        "B should NOT see beat: {:?}",
        stdout
    );
}

#[test]
fn typecheck_rejects_non_locus_adapter_head() {
    // `type` head (not a locus) should be rejected with a focused
    // diag at typecheck.
    use hale_syntax::parse_source;
    use hale_types::{resolve::build_top_scope, symbol::Bundle, check::check_bundle};

    let src = r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; subject: "beat"; }

        type NotALocus { x: Int; }

        main locus App {
            bindings {
                Beat: NotALocus { x: 1 };
            }
        }
        fn main() { App { }; }
    "#;
    let program = parse_source(src).expect("parse");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert("test.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let (scope, mut diags) = build_top_scope(&bundle);
    diags.extend(check_bundle(&bundle, &scope, true));
    let msgs: Vec<String> = diags.iter().map(|d| d.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m.contains("not a locus")),
        "expected `not a locus` diagnostic; got: {:?}",
        msgs
    );
}

#[test]
fn build_rejects_locus_missing_send_method() {
    // The stdlib's `__StdBusAdapter` interface isn't part of the
    // typecheck-visible bundle (stdlib merge happens inside
    // build_executable), so the structural check at typecheck is
    // a no-op. Codegen catches the missing `send` method with a
    // focused diag.
    let program = hale_syntax::parse_source(r#"
        type Tick { n: Int; }
        topic Beat { payload: Tick; subject: "beat"; }

        locus Empty { }

        main locus App {
            bindings {
                Beat: Empty { };
            }
        }
        fn main() { App { }; }
    "#)
    .expect("parse");
    let bin = harness::unique_bin(&format!(
        "hale_adapter_missing_send_{}",
        std::process::id()
    ));
    let err = build_executable(&program, &bin).expect_err("expected codegen err");
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("has no `send` method"),
        "expected missing-send diag; got: {:?}",
        msg
    );
}

#[test]
fn codec_on_adapter_binding_encodes_published_value() {
    // GH #1040: a `codec(...)` on an adapter binding segfaulted in
    // `encode` on the first publish. The prelude built the codec
    // only for `unix(...)` bindings, so an adapter binding's thunk
    // called `encode` with a null `self`. The adapter's `send` must
    // receive exactly the bytes the codec's `encode` produced.
    let src = r#"
        type Msg { tag: Int = 0; who: String = ""; }
        type EncErr { kind: String = ""; }
        type DecErr { kind: String = ""; }
        topic MsgTopic { payload: Msg; subject: "codec.json.msgs"; }

        locus JsonCodec {
            fn encode(v: Msg) -> Bytes fallible(EncErr) {
                return std::bytes::from_string("{\"tag\":" + to_string(v.tag) + ",\"who\":\"" + v.who + "\"}");
            }
            fn decode(b: Bytes) -> Msg fallible(DecErr) {
                let t = std::str::from_bytes(b);
                return Msg { tag: std::json::find_int_field(t, "tag"), who: std::json::find_string_field(t, "who") };
            }
        }

        locus Sink {
            fn send(subject: String, bytes: Bytes) {
                println("send " + subject + " " + std::str::from_bytes(bytes));
            }
        }

        main locus App {
            bus { publish MsgTopic; }
            bindings {
                MsgTopic: Sink { } codec(JsonCodec { });
            }
            run() {
                MsgTopic <- Msg { tag: 42, who: "ana" };
                MsgTopic <- Msg { tag: 7, who: "bo" };
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, status) = build_and_run("codec_encode", src);
    assert!(status.success(), "non-zero: {:?}; stdout: {:?}", status, stdout);
    let sends: Vec<&str> =
        stdout.lines().filter(|l| l.starts_with("send ")).collect();
    assert_eq!(
        sends,
        vec![
            r#"send codec.json.msgs {"tag":42,"who":"ana"}"#,
            r#"send codec.json.msgs {"tag":7,"who":"bo"}"#,
        ],
        "the adapter must receive the codec's bytes; stdout: {:?}",
        stdout
    );
}

/// GH #1034: build `consumer_src` against one imported library seed
/// (`import "../lib" as lib;`), replicating the CLI's flow: mangle the
/// library, merge it, and collapse the consumer's qualified paths
/// through the per-build rename table (`apply_qualified_path_renames`,
/// which is where a joined `alias::Name` binding ident resolves).
fn build_with_lib(
    name: &str,
    lib_src: &str,
    consumer_src: &str,
) -> Result<std::path::PathBuf, hale_codegen::CodegenError> {
    use hale_codegen::mangle;
    let alias = "lib";
    let mut lib_prog = hale_syntax::parse_source(lib_src).expect("parse lib");
    let seed_renames = {
        let stems: Vec<(String, &hale_syntax::ast::Program)> =
            vec![("wire".to_string(), &lib_prog)];
        mangle::build_seed_renames(&stems, alias)
    };
    let renames: Vec<(Vec<String>, String)> = seed_renames
        .iter()
        .map(|(n, m)| (vec![alias.to_string(), n.clone()], m.clone()))
        .collect();
    mangle::mangle_with_renames(&mut lib_prog, &seed_renames);
    let mut consumer =
        hale_syntax::parse_source(consumer_src).expect("parse consumer");
    consumer.imports.clear();
    consumer.items.extend(lib_prog.items);
    mangle::apply_qualified_path_renames(&mut consumer, &renames);
    let bin = harness::unique_bin(&format!(
        "hale_adapter_binding_{}_{}",
        name,
        std::process::id()
    ));
    hale_codegen::build_executable_with_imports(&consumer, &bin, &renames)?;
    Ok(bin)
}

fn run_bin(bin: &std::path::Path) -> (String, std::process::ExitStatus) {
    let out = Command::new(bin).output().expect("run");
    let _ = std::fs::remove_file(bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

const TAP_ADAPTER: &str = r#"
    locus Tap {
        params { label: String = "noname"; }
        fn send(subject: String, bytes: Bytes) {
            println("tap[" + self.label + "] subject=" + subject);
        }
    }
"#;

#[test]
fn adapter_named_through_an_import_alias_binds_like_a_local_one() {
    // GH #1034: `bindings { T: lib::Tap { ... }; }` was a parse error
    // ("unknown transport constructor `lib`"), so a library could not
    // ship the adapter its topics are built around. The qualified
    // binding must behave exactly as the same adapter declared in the
    // program's own seed.
    let lib_src = format!(
        "type Tick {{ n: Int = 0; }}\n{TAP_ADAPTER}"
    );
    let consumer = |adapter: &str| {
        format!(
            r#"
            import "../lib" as lib;
            topic Beat {{ payload: lib::Tick; subject: "beat"; }}

            locus Producer {{
                bus {{ publish Beat; }}
                birth() {{
                    Beat <- lib::Tick {{ n: 1 }};
                    Beat <- lib::Tick {{ n: 2 }};
                }}
            }}

            main locus App {{
                bindings {{ Beat: {adapter} {{ label: "T" }}; }}
            }}

            fn main() {{
                App {{ }};
                Producer {{ }};
            }}
            "#
        )
    };
    let qualified = build_with_lib("alias_adapter", &lib_src, &consumer("lib::Tap"))
        .expect("build with the adapter named through the alias");
    let (q_stdout, q_status) = run_bin(&qualified);
    assert!(q_status.success(), "non-zero: {:?}; stdout: {:?}", q_status, q_stdout);

    let local_src = format!("{}\n{TAP_ADAPTER}", consumer("Tap"));
    let local = build_with_lib("local_adapter", "type Tick { n: Int = 0; }", &local_src)
        .expect("build with the adapter declared locally");
    let (l_stdout, l_status) = run_bin(&local);
    assert!(l_status.success(), "non-zero: {:?}; stdout: {:?}", l_status, l_stdout);

    assert_eq!(
        q_stdout.lines().filter(|l| *l == "tap[T] subject=beat").count(),
        2,
        "the aliased adapter's send must fire per publish; stdout: {:?}",
        q_stdout
    );
    assert_eq!(q_stdout, l_stdout, "aliased and local adapters must agree");
}

#[test]
fn adapter_and_codec_named_through_an_import_alias_run() {
    // GH #1034, the codec clause: `codec(lib::JsonCodec { })` was a
    // parse error ("expected {, got ColonColon"). The issue's whole
    // shape — a library that ships both the adapter and the codec its
    // topic is built around, bound through the import alias — must
    // run: every publish reaches the library's adapter as the bytes
    // the library's codec encoded, and a relay back through
    // `__local_dispatch` decodes through the same codec.
    let lib_src = r#"
        type Tick { n: Int = 0; }
        type EncErr { kind: String = ""; }
        type DecErr { kind: String = ""; }
        locus JsonCodec {
            fn encode(v: Tick) -> Bytes fallible(EncErr) {
                return std::bytes::from_string("{\"n\":" + to_string(v.n) + "}");
            }
            fn decode(b: Bytes) -> Tick fallible(DecErr) {
                return Tick { n: std::json::find_int_field(std::str::from_bytes(b), "n") };
            }
        }
        locus Relay {
            fn send(subject: String, bytes: Bytes) {
                println("wire " + std::str::from_bytes(bytes));
                std::bus::__local_dispatch(subject, bytes);
            }
        }
    "#;
    let consumer = r#"
        import "../lib" as lib;
        topic Beat { payload: lib::Tick; subject: "beat"; }

        locus Listener {
            bus { subscribe Beat as on_beat; }
            fn on_beat(t: lib::Tick) { println("heard " + to_string(t.n)); }
        }

        locus Producer {
            bus { publish Beat; }
            birth() {
                Beat <- lib::Tick { n: 1 };
                Beat <- lib::Tick { n: 2 };
            }
        }

        main locus App {
            bindings { Beat: lib::Relay { } codec(lib::JsonCodec { }); }
        }

        fn main() {
            App { };
            Listener { };
            Producer { };
        }
    "#;
    let bin = build_with_lib("alias_codec", lib_src, consumer)
        .expect("build with the adapter and codec named through the alias");
    let (stdout, status) = run_bin(&bin);
    assert!(status.success(), "non-zero: {:?}; stdout: {:?}", status, stdout);
    let wire: Vec<&str> = stdout.lines().filter(|l| l.starts_with("wire ")).collect();
    assert_eq!(
        wire,
        vec![r#"wire {"n":1}"#, r#"wire {"n":2}"#],
        "the library's adapter receives the library's codec's bytes; stdout: {:?}",
        stdout
    );
    // Local publish + codec-decoded relay for each tick.
    let heard = |n: &str| stdout.lines().filter(|l| *l == n).count();
    assert_eq!(heard("heard 1"), 2, "stdout: {:?}", stdout);
    assert_eq!(heard("heard 2"), 2, "stdout: {:?}", stdout);
}

/// GH #1038: the payload arena's resident bytes, from each
/// `std::process::dump_arena_residency()` in `stderr`, in order. An
/// arena not yet created reads as 0 — a program whose publishes no
/// longer touch it may never create it.
fn payload_arena_bytes_per_dump(stderr: &str) -> Vec<u64> {
    let mut out = Vec::new();
    let mut current: Option<u64> = None;
    for line in stderr.lines() {
        if line.starts_with("[arena_residency dump]") {
            if let Some(b) = current.take() {
                out.push(b);
            }
            current = Some(0);
        } else if line.contains("label=g_bus_payload_arena") {
            let bytes = line
                .split_whitespace()
                .find_map(|w| w.strip_prefix("bytes="))
                .and_then(|v| v.parse::<u64>().ok())
                .expect("a residency row carries bytes=N");
            current = Some(bytes);
        }
    }
    out.extend(current);
    out
}

#[test]
fn adapter_publish_leaves_the_payload_arena_flat() {
    // GH #1038: every publish through an adapter copied the wire
    // bytes into the program-lifetime payload arena for `send`'s
    // `bytes`, and nothing reclaimed them — ~47 bytes per publish,
    // until the arena's cap made the copy fail and the runtime
    // stopped calling `send` at all. The bytes now live for the call.
    let src = r#"
        type Note { n: Int = 0; text: String = ""; }
        topic Out { payload: Note; subject: "out"; }

        locus Sink {
            fn send(subject: String, bytes: Bytes) { }
        }

        main locus App {
            bindings { Out: Sink { }; }
            bus { publish Out; }
            @unbounded
            run() {
                Out <- Note { n: 0, text: "fixed payload text" };
                let _a = std::process::dump_arena_residency();
                let mut i = 1;
                while i < 20000 {
                    Out <- Note { n: i, text: "fixed payload text" };
                    i = i + 1;
                }
                let _b = std::process::dump_arena_residency();
            }
        }

        fn main() { App { }; }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!(
        "hale_adapter_binding_payload_flat_{}",
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero: {:?}; stderr: {stderr}", out.status);
    let dumps = payload_arena_bytes_per_dump(&stderr);
    // The two in-program dumps, then the one at exit.
    assert!(dumps.len() >= 2, "expected two residency dumps; stderr: {stderr}");
    assert_eq!(
        dumps[0], dumps[1],
        "the payload arena grew between 1 and 20000 adapter publishes; stderr: {stderr}"
    );
}

#[test]
fn adapter_send_bytes_are_kept_by_copy_and_survive_a_nested_use() {
    // GH #1038: `send`'s `bytes` lives in a per-thread bus scratch
    // for the call. A `send` that stores it keeps a copy (the next
    // call reads the previous message intact), and a `send` that
    // re-enters the bus — here, relaying through `__local_dispatch`
    // on a keyed topic, which decodes into the same scratch — does
    // not lose its own bytes when the inner use ends. Under ASan with
    // chunk pooling off, a scratch cleared too early is a
    // use-after-free, not a quiet stale read.
    let src = r#"
        type Msg { tag: Int = 0; who: String = ""; }
        type EncErr { kind: String = ""; }
        type DecErr { kind: String = ""; }
        topic Evt { payload: Msg; subject: "keep.evt"; keyed_by who; }

        locus JsonCodec {
            fn encode(v: Msg) -> Bytes fallible(EncErr) {
                return std::bytes::from_string("{\"tag\":" + to_string(v.tag) + ",\"who\":\"" + v.who + "\"}");
            }
            fn decode(b: Bytes) -> Msg fallible(DecErr) {
                let t = std::str::from_bytes(b);
                return Msg { tag: std::json::find_int_field(t, "tag"), who: std::json::find_string_field(t, "who") };
            }
        }

        locus Keep {
            params { last: Bytes = std::bytes::from_string("none"); }
            fn send(subject: String, bytes: Bytes) {
                println("prev=" + std::str::from_bytes(self.last) + " now=" + std::str::from_bytes(bytes));
                self.last = bytes;
                std::bus::__local_dispatch(subject, bytes);
                println("after=" + std::str::from_bytes(bytes) + " kept=" + std::str::from_bytes(self.last));
            }
        }

        locus Rev {
            params { id: String = ""; }
            bus { subscribe Evt as on_evt where key == self.id; }
            fn on_evt(m: Msg) { println("rcv " + m.who + " " + to_string(m.tag)); }
        }

        main locus App {
            params { r: Rev = Rev { id: "ana" }; }
            bus { publish Evt; }
            bindings { Evt: Keep { } codec(JsonCodec { }); }
            run() {
                Evt <- Msg { tag: 1, who: "ana" };
                Evt <- Msg { tag: 2, who: "ana" };
                Evt <- Msg { tag: 3, who: "bo" };
            }
        }

        fn main() { App { }; }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!(
        "hale_adapter_binding_send_scratch_{}",
        std::process::id()
    ));
    harness::build_asan(&program, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "non-zero: {:?}; stdout: {stdout}; stderr: {stderr}",
        out.status
    );
    let sends: Vec<&str> = stdout
        .lines()
        .filter(|l| l.starts_with("prev=") || l.starts_with("after="))
        .collect();
    assert_eq!(
        sends,
        vec![
            r#"prev=none now={"tag":1,"who":"ana"}"#,
            r#"after={"tag":1,"who":"ana"} kept={"tag":1,"who":"ana"}"#,
            r#"prev={"tag":1,"who":"ana"} now={"tag":2,"who":"ana"}"#,
            r#"after={"tag":2,"who":"ana"} kept={"tag":2,"who":"ana"}"#,
            r#"prev={"tag":2,"who":"ana"} now={"tag":3,"who":"bo"}"#,
            r#"after={"tag":3,"who":"bo"} kept={"tag":3,"who":"bo"}"#,
        ],
        "stdout: {stdout}"
    );
    // Local publish + keyed relay for each `ana` message; `bo` has no
    // subscriber.
    let rcv = |l: &str| stdout.lines().filter(|x| *x == l).count();
    assert_eq!(rcv("rcv ana 1"), 2, "stdout: {stdout}");
    assert_eq!(rcv("rcv ana 2"), 2, "stdout: {stdout}");
    assert!(!stdout.contains("rcv bo"), "stdout: {stdout}");
}

/// GH #1038 follow-up: parse `[bus call arenas] opened=N live_bytes=L
/// peak_bytes=P` from a `LOTUS_BUS_CALL_ARENA_STATS=1` run.
fn call_arena_stats(stderr: &str) -> Option<(u64, u64, u64)> {
    let line = stderr.lines().find(|l| l.starts_with("[bus call arenas]"))?;
    let field = |k: &str| -> Option<u64> {
        line.split_whitespace()
            .find_map(|w| w.strip_prefix(k))
            .and_then(|v| v.parse().ok())
    };
    Some((field("opened=")?, field("live_bytes=")?, field("peak_bytes=")?))
}

#[test]
fn a_parking_send_under_overlapping_publishes_stays_flat() {
    // GH #1038 follow-up: `send`'s `bytes` lived in a per-thread
    // scratch reclaimed only when the OUTERMOST use on the thread
    // ended. A `send` that parks on an async_io pool while other
    // publishes on that thread overlap it never let that happen, so
    // the scratch grew at the full per-message rate, uncapped. Each
    // `send` now gets its own arena, freed when the call returns: with
    // a worker publishing a message every millisecond and each `send`
    // sleeping 4 ms (about five sends parked at any moment), the bytes
    // held by open call arenas stay a handful of chunks however many
    // messages go through, and every arena is closed by exit. The
    // call arena is not a residency target, so this reads the bus's
    // own counters (`LOTUS_BUS_CALL_ARENA_STATS=1`).
    let src = r#"
        type Note { n: Int = 0; text: String = ""; }
        type Go { n: Int = 0; }
        topic Out { payload: Note; subject: "out"; }
        topic Trig { payload: Go; subject: "trig"; }

        locus Sink {
            fn send(subject: String, bytes: Bytes) { std::time::sleep(4ms); }
        }

        locus Worker {
            params { done: Int = 0; }
            bus { subscribe Trig as on_go; publish Out; }
            fn on_go(g: Go) {
                Out <- Note { n: g.n, text: "a fixed payload of some forty bytes...." };
                self.done = self.done + 1;
            }
        }

        main locus App {
            params { w: Worker = Worker { }; }
            placement { w: cooperative(pool = io) where async_io; }
            bindings { Out: Sink { }; }
            bus { publish Trig; }
            @unbounded
            run() {
                let mut i = 0;
                while i < 2000 {
                    Trig <- Go { n: i };
                    std::time::sleep(1ms);
                    i = i + 1;
                }
                let mut k = 0;
                while self.w.done < 2000 && k < 1000 {
                    std::time::sleep(10ms);
                    k = k + 1;
                }
                println("done=" + to_string(self.w.done));
            }
        }

        fn main() { App { }; }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!(
        "hale_adapter_binding_call_arena_{}",
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_BUS_CALL_ARENA_STATS", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero: {:?}; stderr: {stderr}", out.status);
    assert!(stdout.contains("done=2000"), "stdout: {stdout}");
    let (opened, live, peak) =
        call_arena_stats(&stderr).expect("a [bus call arenas] line");
    assert_eq!(opened, 2000, "one call arena per send; stderr: {stderr}");
    assert_eq!(live, 0, "every call arena closed by exit; stderr: {stderr}");
    // A chunk is 64 KiB. The old scratch held one payload per message
    // for as long as the overlap lasted; a per-call arena holds one
    // chunk per send in flight.
    assert!(
        peak <= 64 * 65536,
        "call arenas peaked at {peak} bytes for 2000 messages — they \
         should track the sends in flight, not the messages; stderr: {stderr}"
    );
}
