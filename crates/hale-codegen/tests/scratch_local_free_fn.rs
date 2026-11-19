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
/// field, is copied there), and so, since GH #2090, do a fn taking a
/// plain `type` value and one building one. A fn writing into its
/// record parameter's list (the parameter is the caller's value), one
/// taking a record that holds an array, and one calling a Hale-source
/// stdlib fn that returns a struct (`std::str::bytes_view`, whose copy
/// of its argument the caller's arena can skip and a fresh one cannot)
/// stay on the caller's.
#[test]
fn only_fns_that_cannot_leak_an_allocation_get_their_own_arena() {
    let src = format!(
        r#"{SCANNER}
type Row {{ text: String = ""; tags: bounded[Row2; 2]; }}
type Row2 {{ text: String = ""; }}
type Framed {{ text: String = ""; pad: [Int; 2] = [0, 0]; }}
type Wrap {{ inner: Row; }}
type Rows {{ items: bounded[Row; 2]; }}
locus Keeper {{
    params {{ last: String = ""; }}
    fn keep(s: String) {{ self.last = s; }}
}}
fn takes_type(r: Row) -> String {{ return r.text + "!"; }}
fn builds_type(s: String) -> String {{ let r = Row {{ text: s + "?" }}; return r.text; }}
fn fills_param(r: Row) -> Int {{ push(r.tags, Row2 {{ text: r.text + "!" }}) or discard; return count(r.tags); }}
fn takes_framed(f: Framed) -> String {{ return f.text + "!"; }}
fn view_len(s: String) -> Int {{ let v = std::str::bytes_view(s); return v.n; }}
fn wraps_param(r: Row) -> Int {{ let mut w = Wrap {{ inner: r }}; push(w.inner.tags, Row2 {{ text: r.text + "!" }}) or discard; return count(w.inner.tags); }}
fn shadows_let(rs: Rows) -> Int {{ let mut r = Row {{ }}; for r in rs.items {{ push(r.tags, Row2 {{ text: r.text + "!" }}) or discard; }} return count(r.tags); }}
fn let_after_for(rs: Rows) -> Int {{ let mut n = 0; for r in rs.items {{ n = n + len(r.text); }} let mut r = Row {{ text: "x" }}; push(r.tags, Row2 {{ text: "y" + to_string(n) }}) or discard; return count(r.tags); }}
fn main() {{
    let k = Keeper {{ }};
    k.keep(line_at("a\nb\n", 1));
    println(takes_type(Row {{ text: std::str::upper("x") }}));
    println(builds_type(std::str::upper("y")));
    println(to_string(fills_param(Row {{ text: std::str::upper("w") }})));
    println(takes_framed(Framed {{ text: std::str::upper("v") }}));
    println(to_string(view_len(std::str::upper("zz"))));
    let mut rs = Rows {{ }};
    push(rs.items, Row {{ text: std::str::upper("u") }}) or discard;
    println(to_string(wraps_param(Row {{ text: std::str::upper("t") }}) + shadows_let(rs) + let_after_for(rs)));
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
    for name in ["line_at", "takes_type", "builds_type", "let_after_for"] {
        assert!(
            body(name).contains("fn.scratch_local.cur"),
            "{name} should allocate from its own arena"
        );
    }
    for name in ["fills_param", "takes_framed", "view_len", "wraps_param", "shadows_let"] {
        assert!(
            !body(name).contains("fn.scratch_local.cur"),
            "{name} can reach a value outliving the call and must keep \
             allocating in its caller's arena"
        );
    }
}

/// GH #2090: a helper that builds a local record holding a `bounded` list
/// (rows with heap Strings and tags) and returns a count, called 20,000
/// times. Its records die at its return, whoever calls it: from `fn
/// main`'s own loop, which has no per-iteration arena, they used to stay
/// in main's arena, one batch per call (about 120 MiB here); from a locus
/// method's loop they were already reclaimed with the iteration.
const ROWS: &str = r#"
type Tag { name: String = ""; }
type Row { id: String = ""; tags: bounded[Tag; 4]; }
type Rows { items: bounded[Row; 64]; }
fn rows(k: Int) -> Int {
    let mut rs = Rows { };
    let mut j = 0;
    while j < 32 {
        let mut r = Row { id: f"row-{k}-{j}" };
        push(r.tags, Tag { name: f"t{j}" }) or discard;
        push(rs.items, r) or discard;
        j = j + 1;
    }
    let mut n = 0;
    for r in rs.items { n = n + len(r.id) + count(r.tags); }
    return n;
}
"#;

fn rows_growth(driver: &str, name: &str) -> (String, i64) {
    let src = format!(
        r#"{ROWS}
locus Driver {{
    params {{ total: Int = 0; }}
    fn go(calls: Int) {{
        let mut k = 0;
        while k < calls {{ self.total = self.total + rows(k); k = k + 1; }}
    }}
}}
fn main() {{
    let warm = rows(0);
    let before = std::io::fs::read_file("/proc/self/statm") or "";
    let mut total = warm * 0;
    {driver}
    let after = std::io::fs::read_file("/proc/self/statm") or "";
    println("total=" + to_string(total));
    print("before="); println(before);
    print("after="); println(after);
}}
"#
    );
    let bin = harness::unique_bin(name);
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let (stdout, stderr, ok) = run(&bin);
    assert!(ok, "stdout={stdout:?} stderr={stderr:?}");
    let field = |key: &str| -> String {
        stdout
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .unwrap_or_else(|| panic!("no {key} in {stdout:?}"))
            .to_string()
    };
    let grew = harness::statm_resident_bytes(&field("after=")) as i64
        - harness::statm_resident_bytes(&field("before=")) as i64;
    (field("total="), grew)
}

/// 20,000 calls: each returns the sum over its 32 rows of the id's length
/// and one tag.
fn rows_total() -> String {
    let mut t = 0usize;
    for k in 0..20000 {
        for j in 0..32 {
            t += format!("row-{k}-{j}").len() + 1;
        }
    }
    t.to_string()
}

#[test]
fn a_helpers_local_bounded_records_die_at_its_return_from_mains_loop() {
    let (total, grew) = rows_growth(
        "let mut k = 0; while k < 20000 { total = total + rows(k); k = k + 1; }",
        "hale_scratch_local_rows_main",
    );
    assert_eq!(total, rows_total());
    assert!(
        grew < 16 << 20,
        "20,000 calls from main's loop grew the resident set by {} MiB — the \
         helper's records are outliving its calls",
        grew >> 20
    );
}

#[test]
fn a_helpers_local_bounded_records_die_at_its_return_from_a_locus_loop() {
    let (total, grew) = rows_growth(
        "let d = Driver { }; d.go(20000); total = d.total;",
        "hale_scratch_local_rows_locus",
    );
    assert_eq!(total, rows_total());
    assert!(grew < 16 << 20, "grew {} MiB from a locus method's loop", grew >> 20);
}

/// The records that escape are left where they were: a fn returning a
/// record and one writing into its record parameter's list keep their
/// caller's arena, and a scratch-local fn handed those records (`tagged`)
/// only reads them. Everything reads back long after the calls, under
/// ASan.
#[test]
fn an_escaping_record_survives_the_callees_arena_under_asan() {
    let src = r#"
type Tag { name: String = ""; }
type Row { id: String = ""; tags: bounded[Tag; 4]; }
type Rows { items: bounded[Row; 8]; }
fn make(k: Int) -> Row {
    let mut r = Row { id: std::str::upper(f"row-{k}") };
    push(r.tags, Tag { name: f"t{k}" + "!" }) or discard;
    return r;
}
fn relabel(r: Row) -> Row {
    let mut v = Row { id: r.id + "?" };
    v.tags = r.tags;
    push(v.tags, Tag { name: std::str::upper(r.id) }) or discard;
    return v;
}
fn fill(r: Row, k: Int) -> Int {
    push(r.tags, Tag { name: f"f{k}" + "!" }) or discard;
    return count(r.tags);
}
fn tagged(r: Row) -> String {
    let mut out = r.id;
    for t in r.tags { out = out + "/" + t.name; }
    return out;
}
fn main() {
    let mut keep = Rows { };
    let mut k = 0;
    while k < 6 {
        push(keep.items, relabel(make(k))) or discard;
        k = k + 1;
    }
    let mut late = Row { id: std::str::lower("LATE") };
    fill(late, 1);
    fill(late, 2);
    let mut out = "";
    for r in keep.items {
        out = out + r.id;
        for t in r.tags { out = out + " " + t.name; }
        out = out + ";";
    }
    for t in late.tags { out = out + " " + t.name; }
    println(out);
    println(tagged(late));
}
"#;
    let bin = harness::unique_bin("hale_scratch_local_record_asan");
    harness::build_source_asan(src, &bin);
    let (stdout, stderr, ok) = run(&bin);
    assert!(!stderr.contains("AddressSanitizer"), "{stderr}");
    assert!(ok, "stdout={stdout:?} stderr={stderr:?}");
    let mut want = String::new();
    for k in 0..6 {
        want += &format!("ROW-{k}? t{k}! ROW-{k};");
    }
    want += " f1! f2!\nlate/f1!/f2!\n";
    assert_eq!(stdout, want);
}

/// GH #2090 review: a write is only the fn's own when its root is a value
/// the fn made. A `let` local whose record literal holds a parameter
/// (`Wrap { inner: r }` holds the caller's `r` by reference), and a `for`
/// binding over a parameter's list that reuses a `let` name, are the
/// caller's storage: a fn writing through either keeps its caller's
/// arena, so what it pushes outlives the call. Each pushed name is read
/// back after a second call reuses whatever the first one freed, under
/// ASan.
#[test]
fn a_write_through_the_callers_storage_keeps_the_callers_arena_under_asan() {
    let cases = [
        (
            "literal_holding_a_param",
            r#"
type Tag { name: String = ""; }
type Row { tags: bounded[Tag; 4]; }
type Wrap { inner: Row; }
fn add(r: Row, k: Int) -> Int {
    let mut w = Wrap { inner: r };
    push(w.inner.tags, Tag { name: f"tag-{k}" }) or discard;
    return count(w.inner.tags);
}
fn main() {
    let mut r = Row { };
    add(r, 1);
    add(r, 2);
    for t in r.tags { println(t.name); }
}
"#,
        ),
        (
            "for_binding_reusing_a_let_name",
            r#"
type Tag { name: String = ""; }
type Row { tags: bounded[Tag; 4]; }
type Rows { items: bounded[Row; 4]; }
fn add(rs: Rows, k: Int) -> Int {
    let mut r = Row { };
    for r in rs.items { push(r.tags, Tag { name: f"tag-{k}" }) or discard; }
    return 0;
}
fn main() {
    let mut rs = Rows { };
    push(rs.items, Row { }) or discard;
    add(rs, 1);
    add(rs, 2);
    for r in rs.items { for t in r.tags { println(t.name); } }
}
"#,
        ),
        (
            "element_read_from_a_params_list",
            r#"
type Tag { name: String = ""; }
type Row { tags: bounded[Tag; 4]; }
type Rows { items: bounded[Row; 4]; }
type Holder { row: Row; }
fn add(rs: Rows, k: Int) -> Int {
    let mut h = Holder { row: at(rs.items, 0) or Row { } };
    push(h.row.tags, Tag { name: f"tag-{k}" }) or discard;
    return count(h.row.tags);
}
fn main() {
    let mut rs = Rows { };
    push(rs.items, Row { }) or discard;
    add(rs, 1);
    add(rs, 2);
    let mut n = 0;
    for r in rs.items { for t in r.tags { n = n + len(t.name); } }
    println(to_string(n));
}
"#,
        ),
    ];
    for (name, src) in cases {
        let bin = harness::unique_bin(&format!("hale_scratch_local_write_root_{name}"));
        harness::build_source_asan(src, &bin);
        let (stdout, stderr, ok) = run(&bin);
        assert!(!stderr.contains("AddressSanitizer"), "{name}: {stderr}");
        assert!(ok, "{name}: stdout={stdout:?} stderr={stderr:?}");
        if name != "element_read_from_a_params_list" {
            assert_eq!(stdout, "tag-1\ntag-2\n", "{name}");
        }
    }
}
