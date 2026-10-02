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

/// A use inside a module-nested fn and inside an `on_failure` handler,
/// which the allocation summary keys no body for.
#[test]
fn nested_and_failure_bodies_are_walked() {
    refused_on_wasm32(
        "module inner {\n    fn danger() {\n        std::process::exit(1);\n    }\n}\n\nfn main() { danger(); }\n",
        &[(3, 9, &format!("`std::process::exit` is unavailable under {{selector}}: {PROCESS}"))],
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
