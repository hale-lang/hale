//! A free fn whose allocations cannot escape except through its return
//! value allocates into its own per-call arena (GH #1148).
//!
//! Every free fn used to allocate into its CALLER's arena (m49: codegen
//! has no general escape analysis, and a value built in the callee may
//! be stored somewhere that outlives it). For a line scanner that walks
//! a String by re-slicing a local — `rest = rest[(nl + 1)..len(rest)]`
//! — that kept one suffix copy per iteration alive until the caller's
//! scope ended, so a caller asking for each line in turn held a cubic
//! amount of garbage at once. A downstream handoff's API head scanning
//! a ~600-row record grew past its 512 MiB address-space bound inside
//! one command and died on the NULL the exhausted allocator returned
//! (`strlen(NULL)`).
//!
//! A "scratch-local" fn — String/scalar params and return, no struct or
//! locus literal, no method call, calls only to its own class, value
//! builtins and pure `std::` namespaces — now allocates into its own
//! subregion, freed at return after the return value is copied out.

use std::process::Command;

#[path = "support/build.rs"]
mod build_opts;
#[path = "support/harness.rs"]
mod harness;

/// The scanner, verbatim in shape from the reported program.
const SCANNER: &str = r#"
fn line_at(list: String, k: Int) -> String {
    let mut rest = list;
    let mut i = 0;
    while len(rest) > 0 {
        let nl = std::str::index_of(rest, "\n");
        let line = if nl < 0 { rest } else { rest[0..nl] };
        if i == k { return line; }
        i = i + 1;
        if nl < 0 { rest = ""; } else { rest = rest[(nl + 1)..len(rest)]; }
    }
    return "";
}
"#;

fn run(bin: &std::path::Path) -> (String, String, bool) {
    let out = Command::new(bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

/// Asking for the last of 200 lines 60 times: each call re-slices
/// ~2 MiB of suffixes. In the caller's arena that was ~120 MiB resident
/// by the end; in the fn's own arena it is one call's worth.
#[test]
fn a_scanner_called_in_a_loop_leaves_nothing_behind() {
    let src = format!(
        r#"{SCANNER}
fn main() {{
    let mut text = "";
    let mut r = 0;
    while r < 200 {{
        text = text + "row " + to_string(r) + " " + std::str::repeat("x", 90) + "\n";
        r = r + 1;
    }}
    let before = std::io::fs::read_file("/proc/self/statm") or "";
    let mut total = 0;
    let mut k = 0;
    while k < 60 {{
        total = total + len(line_at(text, 199));
        k = k + 1;
    }}
    let after = std::io::fs::read_file("/proc/self/statm") or "";
    println("total=" + to_string(total));
    print("before="); println(before);
    print("after="); println(after);
}}
"#
    );
    let bin = harness::unique_bin("hale_scratch_local_residency");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let (stdout, stderr, ok) = run(&bin);
    assert!(ok, "stdout={stdout:?} stderr={stderr:?}");
    let field = |key: &str| -> &str {
        stdout
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .unwrap_or_else(|| panic!("no {key} in {stdout:?}"))
    };
    assert_eq!(
        field("total="),
        (60 * 98).to_string(),
        "each call returns row 199"
    );
    let before = harness::statm_resident_bytes(field("before="));
    let after = harness::statm_resident_bytes(field("after="));
    let grew = after - before;
    assert!(
        grew < 32 << 20,
        "60 scanner calls grew the resident set by {} MiB — their \
         suffix copies are outliving the calls",
        grew >> 20
    );
}

/// The value a scratch-local fn returns is built in the arena it frees
/// on return, and here it passes through a second one: it must be
/// copied out at each return, not read after the free. Heap values
/// throughout (a literal would sit in static memory and hide the free).
#[test]
fn a_returned_value_survives_the_callees_arena_under_asan() {
    let src = format!(
        r#"{SCANNER}
fn shout(list: String, k: Int) -> String {{
    let line = line_at(list, k);
    return std::str::upper(line) + "!";
}}
fn main() {{
    let text = std::str::lower("ALPHA\nBETA\nGAMMA\n") + to_string(42);
    let mut out = "";
    let mut k = 0;
    while k < 4 {{
        out = out + shout(text, k) + " ";
        k = k + 1;
    }}
    println(out);
}}
"#
    );
    let bin = harness::unique_bin("hale_scratch_local_asan");
    harness::build_source_asan(&src, &bin);
    let (stdout, stderr, ok) = run(&bin);
    assert!(!stderr.contains("AddressSanitizer"), "{stderr}");
    assert!(ok, "stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stdout, "ALPHA! BETA! GAMMA! 42! \n");
}

/// The class is narrow on purpose: a fn that can reach a value living
/// beyond the call keeps allocating in its caller's arena. The scanner
/// allocates from its own arena (and its result, stored into a locus
/// field, is copied there); a fn taking a `type` value, one that builds
/// a struct, and one calling a Hale-source stdlib fn that returns a
/// struct (`std::str::bytes_view`, whose copy of its argument the
/// caller's arena can skip and a fresh one cannot) stay on the caller's.
#[test]
fn only_fns_that_cannot_leak_an_allocation_get_their_own_arena() {
    let src = format!(
        r#"{SCANNER}
type Row {{ text: String = ""; }}
locus Keeper {{
    params {{ last: String = ""; }}
    fn keep(s: String) {{ self.last = s; }}
}}
fn takes_type(r: Row) -> String {{ return r.text + "!"; }}
fn builds_type(s: String) -> String {{ let r = Row {{ text: s + "?" }}; return r.text; }}
fn view_len(s: String) -> Int {{ let v = std::str::bytes_view(s); return v.n; }}
fn main() {{
    let k = Keeper {{ }};
    k.keep(line_at("a\nb\n", 1));
    println(takes_type(Row {{ text: std::str::upper("x") }}));
    println(builds_type(std::str::upper("y")));
    println(to_string(view_len(std::str::upper("zz"))));
    println(k.last);
}}
"#
    );
    let bin = harness::unique_bin("hale_scratch_local_ir");
    let ir = harness::build_source_ir_text(&src, &bin).expect("build");
    let _ = std::fs::remove_file(&bin);
    let body = |name: &str| -> String {
        let start = ir
            .find(&format!("@{name}("))
            .and_then(|i| ir[..i].rfind("define "))
            .unwrap_or_else(|| panic!("no definition of {name}"));
        let end = start + ir[start..].find("\n}\n").expect("fn end");
        ir[start..end].to_string()
    };
    assert!(
        body("line_at").contains("fn.scratch_local.cur"),
        "the scanner should allocate from its own arena"
    );
    for name in ["takes_type", "builds_type", "view_len"] {
        assert!(
            !body(name).contains("fn.scratch_local.cur"),
            "{name} can reach a value outliving the call and must keep \
             allocating in its caller's arena"
        );
    }
}
