//! The use producer and the admission law (F.40 phase 3, P3 2 of 3;
//! `notes/f40-capability-matrix.md` §1.4, §5's paired cases): every way
//! a program reaches a capability is a use with its witness chain, and
//! a use whose cell is `Reject` on the effective target is refused at
//! its first site in the program's own sources. Each case is checked on
//! the host (admitted), under a `target wasm { }` declaration and under
//! `--target wasm32` (refused, located, worded with what selected the
//! target), and a type-only variant is admitted everywhere.
//!
//! Case 12 (an import alias's wrapper) needs a second seed and is
//! `crates/hale-cli/tests/target_precedence.rs`'s.

use hale_syntax::parse_source;
use hale_types::capability::uses::{derive_capability_uses, Need, UseKind};
use hale_types::capability::{Capability, ConfiguredTarget};
use hale_types::target::TargetSpec;

/// The check's errors as (line, col, message), under a configured
/// target (`None`: the host, named by nothing).
fn check(src: &str, triple: Option<&str>) -> Vec<(usize, usize, String)> {
    let mut program = parse_source(src).expect("parse failed");
    hale_types::desugar_sequence::desugar_before_check(
        &mut [&mut program],
        &hale_types::desugar_sequence::Sequence { import_renames: &[], api: None, api_roles: None },
    )
    .unwrap();
    let ids = hale_types::snapshot::mint([("", &mut program)], &[]);
    let mut programs = std::collections::BTreeMap::new();
    programs.insert(String::new(), &program);
    let mut bundle = hale_types::Bundle::new(programs);
    bundle.snapshot = ids;
    if let Some(t) = triple {
        let spec = TargetSpec::parse(t).unwrap();
        bundle.target = ConfiguredTarget { name: spec.triple.to_string(), spec, explicit: true };
    }
    hale_types::check_bundle_opts_whole_program(&bundle, false)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| {
            let at = d.span.start.0 as usize;
            let before = &src[..at.min(src.len())];
            let line = before.matches('\n').count() + 1;
            let col = before.len() - before.rfind('\n').map_or(0, |i| i + 1) + 1;
            (line, col, d.message)
        })
        .collect()
}

/// The refusals under wasm32, selected each way, with the selector's
/// phrase substituted into `want`'s `{selector}`.
fn refused_on_wasm32(src: &str, want: &[(usize, usize, &str)]) {
    let declared = format!("{src}\ntarget wasm {{ }}\n");
    for (program, triple, selector) in
        [(src, Some("wasm32"), "`--target wasm32`"), (declared.as_str(), None, "`target wasm`")]
    {
        let got = check(program, triple);
        let want: Vec<(usize, usize, String)> =
            want.iter().map(|(l, c, m)| (*l, *c, m.replace("{selector}", selector))).collect();
        assert_eq!(got, want, "under {selector}:\n{program}");
    }
    assert_eq!(check(src, None), vec![], "the host admits it");
}

/// Admitted on every column.
fn admitted(src: &str) {
    for triple in [None, Some("wasm32"), Some("x86_64-unknown-linux-musl")] {
        assert_eq!(check(src, triple), vec![], "{triple:?}:\n{src}");
    }
}

const TCP: &str = "raw TCP sockets don't exist in the browser; use a WebSocket bus adapter (`ws://`) for networking";
const PROCESS: &str = "OS process control (`std::process`) isn't available in the browser";

/// Paired case 1: a call spelled with its stdlib path, refused with the
/// stdlib gate's wording verbatim.
#[test]
fn a_stdlib_call_is_refused_at_the_call() {
    refused_on_wasm32(
        "fn main() {\n    let _ = std::process::pid();\n}\n",
        &[(2, 13, &format!("`std::process::pid` is unavailable under {{selector}}: {PROCESS}"))],
    );
}

/// Paired case 10: a construction in a params initializer, which no fn
/// body holds; the witness is the locus's lifecycle down to the
/// primitive. A signature that only names the type is admitted.
#[test]
fn a_construction_is_refused_at_the_literal_with_its_lifecycle() {
    let src = "fn ignore_conn(s: std::io::tcp::Stream) { }\n\n\
               main locus App {\n    params {\n        l: std::io::tcp::Listener = std::io::tcp::Listener {\n            \
               host: \"127.0.0.1\",\n            port: 0,\n            max_accepts: -1,\n            on_connection: ignore_conn,\n        };\n    }\n}\n\n\
               fn main() { App { }; }\n";
    refused_on_wasm32(
        src,
        &[(
            5,
            37,
            &format!(
                "`std::io::tcp` is unavailable under {{selector}}: {TCP} — witness: `std::io::tcp::Listener` → `birth()` \
                 → `std::io::tcp::__listen_socket`"
            ),
        )],
    );
    admitted("fn keep(l: std::io::tcp::Listener) { }\n\nfn main() { }\n");
}

/// Paired case 11: a method on a handle, resolved through the receiver's
/// declared type; the parameter type itself is admitted.
#[test]
fn a_handle_method_is_refused_at_the_receiver_call() {
    refused_on_wasm32(
        "fn serve(conn: std::io::tcp::Stream) {\n    let _ = conn.recv(64) or \"\";\n}\n\nfn main() { }\n",
        &[(
            2,
            13,
            &format!(
                "`std::io::tcp` is unavailable under {{selector}}: {TCP} — witness: `std::io::tcp::Stream::recv` → \
                 `std::io::tcp::__recv`"
            ),
        )],
    );
    admitted("fn serve(conn: std::io::tcp::Stream) { }\n\nfn main() { }\n");
}

/// Paired case 13: a wrapper the program writes is judged at the use in
/// its own body, once; its callers carry nothing.
#[test]
fn a_wrapper_is_refused_once_inside_it() {
    refused_on_wasm32(
        "fn pid() -> Int {\n    return std::process::pid();\n}\n\nfn main() {\n    let a = pid();\n    let b = pid();\n    println(a + b);\n}\n",
        &[(2, 12, &format!("`std::process::pid` is unavailable under {{selector}}: {PROCESS}"))],
    );
}

/// A use inside a module-nested fn is read from the bundle's summary,
/// which includes nested declarations without a second producer.
#[test]
fn nested_and_failure_bodies_are_walked() {
    refused_on_wasm32(
        "module inner {\n    fn danger() {\n        std::process::exit(1);\n    }\n}\n\nfn main() { danger(); }\n",
        &[(3, 9, &format!("`std::process::exit` is unavailable under {{selector}}: {PROCESS}"))],
    );
}

/// A call in an index operand is a use: the summary walks the subscript
/// as any operand (the review of #1318: the admission read the summary's
/// call rows, and `xs[std::process::pid()]` reached neither a row nor a
/// hole).
#[test]
fn an_index_operand_is_refused_at_the_call() {
    refused_on_wasm32(
        "fn main() {\n    let xs = [0];\n    let _ = xs[std::process::pid()];\n}\n",
        &[(3, 16, &format!("`std::process::pid` is unavailable under {{selector}}: {PROCESS}"))],
    );
}

/// The graph's own walk of what the summary keys no body for — a params
/// initializer, an `on_failure` handler — evaluates a method's receiver,
/// so an index operand under it is a use too.
#[test]
fn an_index_operand_under_a_method_receiver_is_walked() {
    const HELPER: &str = "locus Helper {\n    params { v: Int = 0; }\n    fn ping() -> Int { return self.v; }\n}\n\n";
    refused_on_wasm32(
        &format!(
            "{HELPER}locus Kid {{\n    params {{ n: Int = [Helper {{ v: 1 }}][std::process::pid()].ping(); }}\n    \
             run() {{ println(self.n); }}\n}}\n\nfn main() {{ Kid {{ }}; }}\n"
        ),
        &[(7, 41, &format!("`std::process::pid` is unavailable under {{selector}}: {PROCESS}"))],
    );
    refused_on_wasm32(
        &format!(
            "{HELPER}locus Once {{\n    params {{ runs: Int = 0; }}\n    closure fuse {{ captures: runs; epoch inline; }}\n    \
             run() {{\n        self.runs = self.runs + 1;\n        if self.runs < 2 {{ violate fuse; }}\n    }}\n}}\n\n\
             main locus App {{\n    params {{ early: Once = Once {{ }}; seen: Int = 0; }}\n    \
             on_failure(c: Once, err: ClosureViolation) {{\n        let hs = [Helper {{ v: 1 }}];\n        \
             self.seen = hs[std::process::pid()].ping();\n    }}\n}}\n\nfn main() {{ App {{ }}; }}\n"
        ),
        &[(19, 24, &format!("`std::process::pid` is unavailable under {{selector}}: {PROCESS}"))],
    );
}

/// A hole: a call through a function-typed parameter has requirements
/// the graph cannot establish. Refused where the column rejects anything
/// in the stdlib family (wasm32); recorded, never silent, elsewhere.
#[test]
fn a_hole_is_refused_on_wasm32_and_recorded_elsewhere() {
    let src = "fn apply(f: fn(Int) -> Int, v: Int) -> Int {\n    return f(v);\n}\n\nfn main() { }\n";
    refused_on_wasm32(
        src,
        &[(
            2,
            12,
            "cannot establish what `f` requires on wasm32: it is called through a function-typed parameter, whose \
             target is not known here",
        )],
    );
    let program = parse_source(src).unwrap();
    let mut programs = std::collections::BTreeMap::new();
    programs.insert(String::new(), &program);
    let bundle = hale_types::Bundle::new(programs);
    let summary = hale_types::alloc_summary::derive_alloc_summary(&bundle);
    let uses = derive_capability_uses(&bundle, &summary);
    let holes: Vec<_> = uses.holes().collect();
    assert_eq!(holes.len(), 1, "{:?}", uses.uses);
    assert_eq!((holes[0].kind, holes[0].chain.as_slice()), (UseKind::Call, ["f".to_string()].as_slice()));
}

const UNRESOLVED: &str = "the callee is a function value the summary cannot resolve";

/// The review of #1318, round 3: a call through a local bound to a
/// stdlib path is the path's use, located at the local and witnessed
/// through it, in a fn's body and in a params initializer alike.
#[test]
fn a_call_through_a_let_bound_path_is_refused_through_the_local() {
    let want = format!("`std::process` is unavailable under {{selector}}: {PROCESS} — witness: `f` → `std::process::pid`");
    refused_on_wasm32("fn main() {\n    let f = std::process::pid;\n    let _ = f();\n}\n", &[(3, 13, &want)]);
    refused_on_wasm32(
        "locus Holder {\n    params { n: Int = { let f = std::process::pid; f() }; }\n    run() { println(self.n); }\n}\n\n\
         fn main() { Holder { }; }\n",
        &[(2, 52, &want)],
    );
}

/// A call in the program's own code through a local the bindings do not
/// follow to a fn — or through a computed callee — reaches the
/// program's function values of its arity (F.40 E5, a classified
/// correction: it was a hole). Each is the program's own fn, judged where
/// it is written, and these ask nothing. In a params initializer (the
/// admission's own walk, which does not resolve function values) it is
/// still a hole: wasm32 cannot admit what it might need, and the host
/// admits it.
#[test]
fn a_call_through_an_unresolved_local_is_a_hole_in_the_programs_own_code() {
    let fns = "fn one() -> Int { return 1; }\nfn two() -> Int { return 2; }\n\n";
    admitted(&format!("{fns}fn main() {{\n    let f = if len(\"ab\") == 2 {{ one }} else {{ two }};\n    println(f());\n}}\n"));
    refused_on_wasm32(
        &format!(
            "{fns}locus Holder {{\n    params {{ n: Int = {{ let f = if len(\"ab\") == 2 {{ one }} else {{ two }}; f() }}; }}\n    \
             run() {{ println(self.n); }}\n}}\n\nfn main() {{ Holder {{ }}; }}\n"
        ),
        &[(5, 73, &format!("cannot establish what `f()` requires on wasm32: {UNRESOLVED}"))],
    );
    admitted(&format!("{fns}fn main() {{\n    let fs = [one, two];\n    println(fs[0]());\n}}\n"));
    // A local bound to a fn of the program's own is followed: its body is
    // judged where it is written, and asks nothing here.
    admitted(&format!("{fns}fn main() {{\n    let f = one;\n    println(f());\n}}\n"));
}

/// The review of #1318, round 3 (loops): a loop is walked once, so a
/// local the loop reassigns is not followed where it is called ahead of
/// the assignment — on a later iteration it runs the value the
/// assignment stored. In a fn's body (the summary's walk) the call
/// reaches the program's function values of its arity (F.40 E5, a
/// classified correction: it was a hole); in a params initializer (the
/// admission's own walk) it is a hole. A local the loop only reads is
/// still followed to what it names.
#[test]
fn a_call_through_a_local_a_loop_reassigns_is_a_hole() {
    let fns = "fn one() -> Int { return 1; }\nfn two() -> Int { return 2; }\n\n";
    let hole = format!("cannot establish what `f()` requires on wasm32: {UNRESOLVED}");
    admitted(&format!(
        "{fns}fn main() {{\n    let mut f = one;\n    let mut i = 0;\n    while i < 2 {{\n        println(f());\n        \
         f = two;\n        i = i + 1;\n    }}\n}}\n"
    ));
    refused_on_wasm32(
        &format!(
            "{fns}locus Holder {{\n    params {{\n        n: Int = {{\n            let mut f = one;\n            \
             let mut n = 0;\n            for i in 0..2 {{\n                n = f();\n                f = two;\n            \
             }}\n            n\n        }};\n    }}\n    run() {{ println(self.n); }}\n}}\n\nfn main() {{ Holder {{ }}; }}\n"
        ),
        &[(10, 21, &hole)],
    );
    // The control: a local no assignment in the loop touches is followed
    // to the path it names, the witness through it.
    refused_on_wasm32(
        "fn main() {\n    let f = std::process::pid;\n    let mut i = 0;\n    while i < 2 {\n        println(f());\n        \
         i = i + 1;\n    }\n}\n",
        &[(5, 17, &format!("`std::process` is unavailable under {{selector}}: {PROCESS} — witness: `f` → `std::process::pid`"))],
    );
}

/// The declaration-level uses: placement entries and bindings are use
/// rows with their capability, located at the entry.
#[test]
fn declarations_are_use_rows() {
    let src = "locus W { run() { } }\n\nmain locus App {\n    params {\n        w: W = W { };\n        v: W = W { };\n    }\n    \
               placement {\n        w: pinned;\n        v: cooperative(pool = workers);\n    }\n}\n\nfn main() { App { }; }\n";
    let program = parse_source(src).unwrap();
    let mut programs = std::collections::BTreeMap::new();
    programs.insert(String::new(), &program);
    let bundle = hale_types::Bundle::new(programs);
    let summary = hale_types::alloc_summary::derive_alloc_summary(&bundle);
    let uses = derive_capability_uses(&bundle, &summary);
    let placed: Vec<(Need, &str)> = uses
        .uses
        .iter()
        .filter(|u| u.kind == UseKind::Placement)
        .map(|u| (u.need.clone(), &src[u.span.start.0 as usize..u.span.end.0 as usize]))
        .collect();
    assert_eq!(
        placed,
        vec![
            (Need::Capability(Capability::PinnedThreads), "w: pinned;"),
            (Need::Capability(Capability::PoolThreads), "v: cooperative(pool = workers);"),
        ]
    );
}

const MAIN_TWO: &str = "locus W { run() { } }\n\nmain locus App {\n    params {\n        w: W = W { };\n    }\n";

/// Paired cases 2–4 (T2): a placement that asks for a thread of its own
/// or a pool's is refused at its entry under wasm32; the host lowers it,
/// and musl refuses `async_io` with its own wording.
#[test]
fn threads_and_async_io_are_refused_at_the_placement_entry() {
    let one_thread = "the wasm32 module runs on its host's one thread (the loader stubs `pthread_create` with `() => 0`)";
    refused_on_wasm32(
        &format!("{MAIN_TWO}    placement {{\n        w: pinned;\n    }}\n}}\n\nfn main() {{ App {{ }}; }}\n"),
        &[(
            8,
            9,
            &format!(
                "placement entry `w`: `pinned` is not available under {{selector}} — a pinned locus owns a thread of its \
                 own, and {one_thread}; place it `cooperative` (pool `main`)"
            ),
        )],
    );
    refused_on_wasm32(
        &format!("{MAIN_TWO}    placement {{\n        w: cooperative(pool = workers);\n    }}\n}}\n\nfn main() {{ App {{ }}; }}\n"),
        &[(
            8,
            9,
            &format!(
                "placement entry `w`: a cooperative pool other than `main` is not available under {{selector}} — its \
                 workers are threads, and {one_thread}; place it `cooperative` (pool `main`)"
            ),
        )],
    );
    let async_io =
        format!("{MAIN_TWO}    placement {{\n        w: cooperative(pool = io) where async_io;\n    }}\n}}\n\nfn main() {{ App {{ }}; }}\n");
    let on_wasm = check(&async_io, Some("wasm32"));
    assert_eq!(on_wasm.len(), 2, "{on_wasm:?}");
    assert_eq!((on_wasm[1].0, on_wasm[1].1), (8, 41));
    assert!(on_wasm[1].2.starts_with("placement entry `w`: `async_io` pools aren't supported on wasm32 — "), "{on_wasm:?}");
    let musl = check(&async_io, Some("x86_64-unknown-linux-musl"));
    assert_eq!(musl.len(), 1, "{musl:?}");
    assert!(musl[0].2.starts_with("placement entry `w`: `async_io` pools aren't supported on musl Linux yet"), "{musl:?}");
    assert_eq!(check(&async_io, None), vec![]);
}

/// Paired case 5 (T2): a transport binding is refused at the binding.
#[test]
fn a_transport_binding_is_refused_at_the_binding() {
    let src = "type Tick {\n    n: Int = 0;\n}\n\ntopic Ping {\n    payload: Tick;\n    subject: \"demo.ping\";\n}\n\n\
               locus Counter {\n    bus {\n        subscribe Ping as on_ping;\n    }\n    fn on_ping(t: Tick) { }\n}\n\n\
               main locus App {\n    params {\n        counter: Counter = Counter { };\n    }\n    bindings {\n        \
               Ping: unix(\"/tmp/p.sock\", role: listen);\n    }\n}\n\nfn main() { App { }; }\n";
    refused_on_wasm32(
        src,
        &[(
            22,
            15,
            "bindings entry `Ping`: this transport is not available under {selector} — the browser sandbox has no \
             AF_UNIX sockets; keep the topic in-process, or reach the host through an `@ffi(\"js\")` host import",
        )],
    );
}

/// T3: the known stubs are refused with their substitute, wherever the
/// program reaches them.
#[test]
fn the_stub_namespaces_are_refused_on_wasm32() {
    let clock = "the browser module has no clock of its own: the shim's inline `clock_gettime` writes zero and \
                 `sleep`'s `clock_nanosleep` is an import stubbed to 0, so a read is always the epoch and a sleep never waits";
    refused_on_wasm32(
        "fn main() {\n    let t = std::time::now();\n    println(t);\n}\n",
        &[(2, 13, &format!("`std::time::now` is unavailable under {{selector}}: {clock}"))],
    );
    refused_on_wasm32(
        "fn main() {\n    let v = std::env::var(\"HOME\");\n    println(v);\n}\n",
        &[(
            2,
            13,
            "`std::env::var` is unavailable under {selector}: the browser has no process environment: the shim's \
             inline `getenv` returns NULL",
        )],
    );
}

/// Paired case 9 (T5): an `@ffi("js")` declaration on a native target is
/// refused at the declaration, called or not; wasm32 lowers it.
#[test]
fn a_js_import_is_refused_at_its_declaration_on_a_native_target() {
    let triple = TargetSpec::host().triple;
    let want = |line: usize| {
        (
            line,
            1,
            format!(
                "`@ffi(\"js\")` fn `console_log` is a host import of the wasm32 loader, and this program is built for \
                 `{triple}`: a native build has no loader to supply it, so it would be an undefined symbol at the link; \
                 build it for wasm32 (`target wasm {{ }}` or `--target wasm32`), or bind a C library with `@ffi(\"c\")`"
            ),
        )
    };
    for main in ["fn main() {\n    console_log(\"hi\");\n}\n", "fn main() {\n}\n"] {
        let src = format!("@ffi(\"js\") fn console_log(m: String);\n\n{main}");
        assert_eq!(check(&src, None), vec![want(1)], "{src}");
        assert_eq!(check(&src, Some("wasm32")), vec![], "{src}");
    }
}

/// An `@export`-only program: a host program refuses it at its first
/// `@export` (design §1.3), where codegen used to fail late and
/// unlocated; wasm32 lowers it.
#[test]
fn an_export_only_program_needs_wasm32() {
    let src = "@export fn go() -> Int {\n    return 1;\n}\n";
    let want = "a program with no `fn main` is an export-only module, which needs wasm32: declare `target wasm { }` \
                or build with `--target wasm32`";
    assert_eq!(check(src, None), vec![(1, 12, want.to_string())]);
    assert_eq!(check(src, Some("wasm32")), vec![]);
}

/// An `@export locus` that writes `run()`: refused at `run` under wasm32
/// by the check, where codegen refused it at build time; an ordinary
/// locus on the host.
#[test]
fn an_exported_locus_with_run_is_refused_at_run_on_wasm32() {
    let src = "@export locus Counter {\n    params { n: Int = 0; }\n    run() { }\n    fn bump() { self.n = self.n + 1; }\n}\n\nfn main() { }\n";
    let want = "@export locus `Counter` must not define `run()` — a wasm singleton is host-driven via its `@export` \
                methods, not a cooperative run loop";
    assert_eq!(check(src, Some("wasm32")), vec![(3, 5, want.to_string())]);
    assert_eq!(check(src, None), vec![]);
}
