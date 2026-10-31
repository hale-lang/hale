//! `spec/styleguide.md` §7 claims that are about CODEGEN limits.
//!
//! Companion to `hale-types/tests/styleguide_snippets.rs`, which pins
//! the claims a typechecker can see. These two cannot live there,
//! because `hale check` does not see them at all — they surface only
//! at `hale build`.
//!
//! That is worth stating plainly, because it is the same shape as the
//! unknown-`std::`-namespace hole (#353 item 9): a reader who trusts
//! the checker is told nothing is wrong. §7's two headline absences
//! both behave this way, so a styleguide reader who writes
//! `Vec<User>` and runs `hale check` gets a clean bill of health and
//! then a build error.
//!
//! Each test asserts the CURRENT truth. If either gap closes, the
//! test fails and points at the styleguide entry to update.
//!
//! Two more families live here for the same reason — `hale check`
//! cannot see them. The sharp edges that pass a whole-seed check and
//! fail at build (`s[i]`, a qualified `std::time::Time` or
//! `std::time::Duration`) are pinned as build-time refusals beside a
//! check that passes. The retirement gaps (§7 open gaps) only show in
//! a running program's resident set, so their tests run one and read
//! `/proc/self/statm` per phase, with a control phase that must stay
//! flat.

use std::path::PathBuf;
use std::process::Command;

/// A fresh directory holding `src` as its seed's `main.hl`. The tag
/// keeps two tests of this binary, run as threads of one process,
/// apart.
fn seed_dir(src: &str, tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("hale-sgclaim-{}-{}", std::process::id(), tag));
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("main.hl"), src).expect("write");
    dir
}

fn output_text(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// `hale check` of the whole seed — the strict form, where a name
/// nothing binds is an error (GH #721) — then `hale build`. Answers
/// (check passed, build failed, build's text).
fn check_then_build(src: &str, tag: &str) -> (bool, bool, String) {
    let dir = seed_dir(src, tag);
    let check = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run hale check");
    let build = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(&dir)
        .arg("-o")
        .arg(dir.join("prog"))
        .output()
        .expect("run hale build");
    let _ = std::fs::remove_dir_all(&dir);
    (
        check.status.success(),
        !build.status.success(),
        format!("check:\n{}\nbuild:\n{}", output_text(&check), output_text(&build)),
    )
}

/// Builds and runs `src`; answers its stdout. Panics on a failed
/// build or a failed run, with both outputs.
fn build_and_run(src: &str, tag: &str) -> String {
    let dir = seed_dir(src, tag);
    let bin = dir.join("prog");
    let build = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(&dir)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("run hale build");
    assert!(build.status.success(), "build failed:\n{}", output_text(&build));
    let run = Command::new(&bin).output().expect("run program");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(run.status.success(), "program failed:\n{}", output_text(&run));
    String::from_utf8_lossy(&run.stdout).to_string()
}

/// Resident bytes at each `<tag>=<statm>` line the program printed,
/// in order. statm's second field is resident pages.
fn statm_phases(stdout: &str) -> Vec<(String, i64)> {
    stdout
        .lines()
        .filter_map(|l| {
            let (tag, rest) = l.split_once('=')?;
            let pages: i64 = rest.split_whitespace().nth(1)?.parse().ok()?;
            Some((tag.to_string(), pages * 4096))
        })
        .collect()
}

/// The growth of each phase over the one before it.
fn phase_growth(stdout: &str) -> std::collections::BTreeMap<String, i64> {
    let phases = statm_phases(stdout);
    assert!(phases.len() >= 2, "no statm phases in:\n{}", stdout);
    phases
        .windows(2)
        .map(|w| (w[1].0.clone(), w[1].1 - w[0].1))
        .collect()
}

/// An AddressSanitizer build holds every freed block in quarantine,
/// so a resident-set measurement says nothing about reclamation there
/// (`form_vec_set_retire.rs` skips the same way).
fn asan_build() -> bool {
    std::env::var("LOTUS_ASAN")
        .map(|v| v == "1" || v == "true" || v == "TRUE")
        .unwrap_or(false)
}

const MIB: i64 = 1024 * 1024;

fn build_fails(src: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir()
        .join(format!("hale-sgclaim-{}-{}", std::process::id(), tag));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("main.hl");
    std::fs::write(&f, src).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(&f)
        .output()
        .expect("run hale build");
    let _ = std::fs::remove_dir_all(&dir);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (!out.status.success(), text)
}

/// §7: "the *mechanism* IS missing … a generic payload enum declares
/// and compiles, but does not construct."
///
/// This entry was WRONG for months in the other direction — it said
/// generic enums "compile, construct, and match today". Nothing
/// checked it, so anyone following the styleguide hit an unsupported
/// error. Pinned now in both directions.
#[test]
fn claim_generic_payload_enums_do_not_construct() {
    // Non-generic constructs. Genericity is the whole difference, and
    // asserting that here is what makes a future failure legible.
    let (ng_failed, ng_out) = build_fails(
        "type Res = enum { Ok(Int), Err(String) };\n\
         fn main() { let r = Res::Ok(1);\n\
             match r { Res::Ok(n) -> println(n), Res::Err(m) -> println(m) } }",
        "nongeneric",
    );
    assert!(
        !ng_failed,
        "a NON-generic payload enum must still construct; if this \
         breaks, §7's framing is wrong in a new way:\n{}",
        ng_out
    );

    let (failed, out) = build_fails(
        "type Opt<T> = enum { Some(T), None };\n\
         fn main() { let r = Opt::Some(1);\n\
             match r { Opt::Some(n) -> println(n), Opt::None -> println(0) } }",
        "generic",
    );
    assert!(
        failed,
        "spec/styleguide.md §7 says a generic payload enum does not \
         construct. It now does — update that entry, and consider \
         whether `Option<T>` should ship:\n{}",
        out
    );
}

/// §7: "No parametric collection types (`List<T>` / `Map<K,V>`).
/// Collections are loci."
#[test]
fn claim_no_parametric_collection_types() {
    let (failed, out) = build_fails(
        "type User { active: Bool; }\n\
         fn f(v: Vec<User>) -> Int { return 1; }\n\
         fn main() { println(1); }",
        "vec",
    );
    assert!(
        failed,
        "spec/styleguide.md §7 says there are no parametric collection \
         types. `Vec<User>` now resolves — update that entry:\n{}",
        out
    );
}

/// The shape that makes both of the above worth pinning HERE rather
/// than in the typecheck harness: `hale check` accepts them.
#[test]
fn these_gaps_are_invisible_to_check() {
    let dir = std::env::temp_dir()
        .join(format!("hale-sgvis-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("main.hl");
    std::fs::write(
        &f,
        "type Opt<T> = enum { Some(T), None };\n\
         fn main() { let r = Opt::Some(1);\n\
             match r { Opt::Some(n) -> println(n), Opt::None -> println(0) } }",
    )
    .expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&f)
        .output()
        .expect("run hale check");
    let _ = std::fs::remove_dir_all(&dir);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("typechecked"),
        "documenting the current split: `hale check` accepts a generic \
         enum construction that `hale build` rejects. If check learns \
         to reject it, delete this test and move the claim into the \
         typecheck harness — the split is the thing being recorded, \
         not a behaviour worth preserving:\n{}",
        text
    );
}

// ---- sharp edges that pass `hale check` and fail at build ----------

/// §7 sharp edges: "No char-level `s[i]` — and the checker doesn't
/// say so". The slice the entry recommends must build.
#[test]
fn claim_char_index_passes_check_and_fails_build() {
    let (_, slice_failed, slice_out) = check_then_build(
        "fn main() { let s = \"hale\"; let i = 1; println(s[i..i + 1]); }",
        "char-slice",
    );
    assert!(
        !slice_failed,
        "§7 recommends `s[i..i + 1]` in place of `s[i]`; it must \
         build:\n{}",
        slice_out
    );

    let (checked, failed, out) = check_then_build(
        "fn main() { let s = \"hale\"; println(s[1]); }",
        "char-index",
    );
    assert!(
        failed && out.contains("indexing a non-array value"),
        "spec/styleguide.md §7 says `s[i]` on a String fails at build. \
         It builds now — update the \"No char-level `s[i]`\" entry \
         (and consider whether a `Char` type shipped with it):\n{}",
        out
    );
    assert!(
        checked,
        "spec/styleguide.md §7 says `hale check` accepts `s[i]`. It \
         refuses it now — reword the entry: the edge is no longer \
         invisible to the checker:\n{}",
        out
    );
}

/// §7 sharp edges: "Write the stdlib's time types bare: `Time` and
/// `Duration`, not `std::time::Time` or `std::time::Duration`." The
/// bare spellings, as a field, a cell and a parameter, must build, and
/// a qualified `let` annotation builds too; a qualified field, cell or
/// parameter passes check and fails at build.
#[test]
fn claim_qualified_std_time_time_fails_build() {
    let (_, bare_failed, bare_out) = check_then_build(
        "type Stamp { key: Int; at: Time; took: Duration; }\n\
         @form(vec)\n\
         locus Times { capacity { heap items of Time; } }\n\
         fn show(d: Duration) { println(d); }\n\
         fn main() {\n\
             let s = Stamp { key: 1, at: Time(7s), took: 2s };\n\
             let d: std::time::Duration = 3s;\n\
             let t = Times { };\n\
             t.push(s.at);\n\
             show(s.took);\n\
             println(t.len(), \" \", d);\n\
         }",
        "time-bare",
    );
    assert!(
        !bare_failed,
        "§7 tells readers to write `Time` and `Duration` bare (and says \
         a qualified `let` annotation builds); all of it must build:\n{}",
        bare_out
    );

    let (checked, failed, out) = check_then_build(
        "type Stamp { key: Int; at: std::time::Time; took: std::time::Duration; }\n\
         @form(vec)\n\
         locus Times { capacity { heap items of std::time::Time; } }\n\
         fn show(d: std::time::Duration) { println(d); }\n\
         fn main() {\n\
             let s = Stamp { key: 1, at: Time(7s), took: 2s };\n\
             let t = Times { };\n\
             t.push(s.at);\n\
             show(s.took);\n\
             println(t.len());\n\
         }",
        "time-qualified",
    );
    assert!(
        failed && out.contains("not in stdlib path-renames table"),
        "spec/styleguide.md §7 says a qualified `std::time::Time` or \
         `std::time::Duration` field, cell or parameter fails at build. It \
         builds now — delete the \"Write the stdlib's time types bare\" \
         entry and the residue in C7:\n{}",
        out
    );
    assert!(
        checked,
        "spec/styleguide.md §7 says `hale check` accepts a qualified \
         `std::time::Time` and `std::time::Duration`. It refuses them now — \
         reword the entry:\n{}",
        out
    );
}

/// The same entry, `Duration` alone: the mixed program above stops at
/// its `Time` field, so a fix to qualified `Duration` by itself would
/// leave it green. This one names the `Duration` message.
#[test]
fn claim_qualified_std_time_duration_alone_fails_build() {
    let (checked, failed, out) = check_then_build(
        "type Span { key: Int; took: std::time::Duration; }\n\
         fn main() {\n\
             let s = Span { key: 1, took: 2s };\n\
             println(s.took);\n\
         }",
        "duration-qualified",
    );
    assert!(
        checked,
        "spec/styleguide.md §7 says `hale check` accepts a qualified \
         `std::time::Duration` field. It refuses it now — reword the \
         entry:\n{}",
        out
    );
    assert!(
        failed && out.contains("qualified type `std::time::Duration` not in stdlib path-renames table"),
        "spec/styleguide.md §7 says a qualified `std::time::Duration` field \
         fails at build. It builds now — the entry's `Duration` half goes:\n{}",
        out
    );
}

// ---- retirement gaps: a running program's resident set ------------

/// §1 "Replace and let it retire": a replaced self-field struct retires
/// its whole old value — a String-only struct, one with a nested record
/// (a §7 open gap until the self-field store walked the replaced value
/// with the form retire descriptor: +6.0 MiB over 200 000 replaces on
/// v0.22.0) — and so does a replaced `@form(hashmap)` cell's Bytes field
/// (+4.6 MiB on v0.22.0, before the map reclaimed a replaced cell's
/// whole value). Every phase holds flat now.
#[test]
fn claim_replaced_nested_fields_and_map_cells_retire() {
    if asan_build() {
        eprintln!("skipped under LOTUS_ASAN: a resident-set budget says nothing past ASan's quarantine");
        return;
    }
    let out = build_and_run(
        r#"
type Flat { n: Int; s: String; }
type Inner { s: String; }
type Outer { n: Int; inner: Inner; }
type Cell { key: Int; b: Bytes; }

@form(hashmap)
locus Table { capacity { pool entries of Cell indexed_by key; } }

main locus App {
    params {
        flat: Flat = Flat { n: 0, s: "" };
        outer: Outer = Outer { n: 0, inner: Inner { s: "" } };
        t: Table = Table { };
    }
    fn put_flat(i: Int) { self.flat = Flat { n: i, s: "value-" + to_string(i) }; }
    fn put_outer(i: Int) { self.outer = Outer { n: i, inner: Inner { s: "value-" + to_string(i) } }; }
    fn put_cell(i: Int) {
        self.t.set(Cell { key: i % 16, b: std::bytes::from_string("value-" + to_string(i)) });
    }
    fn statm(tag: String) {
        print(tag);
        println(std::io::fs::read_file("/proc/self/statm") or "");
    }
    run() {
        let n = 200000;
        let mut i = 0;
        while i < 1000 { self.put_flat(i); self.put_outer(i); self.put_cell(i); i = i + 1; }
        self.statm("start=");
        i = 0;
        while i < n { self.put_flat(i); i = i + 1; }
        self.statm("flat=");
        i = 0;
        while i < n { self.put_outer(i); i = i + 1; }
        self.statm("nested=");
        i = 0;
        while i < n { self.put_cell(i); i = i + 1; }
        self.statm("map_bytes=");
    }
}
fn main() { App { }; }
"#,
        "retire-replace",
    );
    let g = phase_growth(&out);
    eprintln!("resident growth per phase (bytes): {:?}", g);
    assert!(
        g["flat"] < MIB / 2,
        "the control — a String-only struct replaced 200 000 times — \
         must hold flat for the gaps below to mean anything; it grew \
         {} bytes:\n{}",
        g["flat"],
        out
    );
    assert!(
        g["nested"] < MIB / 2,
        "spec/styleguide.md §1 says a replaced struct's nested-compound \
         fields retire with the rest of it (\"Replace and let it retire\"); \
         200 000 replaces grew {} bytes:\n{}",
        g["nested"],
        out
    );
    assert!(
        g["map_bytes"] < MIB / 2,
        "spec/styleguide.md §7 says a replaced `@form(hashmap)` cell's \
         Bytes field retires with the rest of its value; 200 000 replaces \
         of 16 keys grew {} bytes:\n{}",
        g["map_bytes"],
        out
    );
}

/// §7 says a replaced `@form(hashmap)` cell retires under every `sync =`
/// discipline. That was an open gap for `striped` and `lockfree` ("maps
/// don't retire replaced cells"): measured on v0.22.0 over 200 000
/// replaces of 16 keys, plain and serialized +4 KiB each, striped
/// +2.3 MiB, lockfree +2.4 MiB. Their reads now copy the cell out under
/// its claim and their writers retire, so all four hold flat.
#[test]
fn claim_maps_of_every_discipline_retire() {
    if asan_build() {
        eprintln!("skipped under LOTUS_ASAN: a resident-set budget says nothing past ASan's quarantine");
        return;
    }
    let out = build_and_run(
        r#"
type Cell { key: Int; s: String; }

@form(hashmap)
locus Plain { capacity { pool entries of Cell indexed_by key; } }
@form(hashmap, sync = serialized)
locus Ser { capacity { pool entries of Cell indexed_by key; } }
@form(hashmap, sync = striped)
locus Str { capacity { pool entries of Cell indexed_by key; } }
@form(hashmap, sync = lockfree, cap = 64)
locus Lf { capacity { pool entries of Cell indexed_by key; } }

main locus App {
    params { p: Plain = Plain { }; se: Ser = Ser { }; st: Str = Str { }; lf: Lf = Lf { }; }
    fn rp(i: Int) { self.p.set(Cell { key: i % 16, s: "value-" + to_string(i) }); }
    fn rse(i: Int) { self.se.set(Cell { key: i % 16, s: "value-" + to_string(i) }); }
    fn rst(i: Int) { self.st.set(Cell { key: i % 16, s: "value-" + to_string(i) }); }
    fn rlf(i: Int) { self.lf.set(Cell { key: i % 16, s: "value-" + to_string(i) }); }
    fn statm(tag: String) {
        print(tag);
        println(std::io::fs::read_file("/proc/self/statm") or "");
    }
    run() {
        let n = 200000;
        let mut i = 0;
        while i < 1000 { self.rp(i); self.rse(i); self.rst(i); self.rlf(i); i = i + 1; }
        self.statm("start=");
        i = 0;
        while i < n { self.rp(i); i = i + 1; }
        self.statm("plain=");
        i = 0;
        while i < n { self.rse(i); i = i + 1; }
        self.statm("serialized=");
        i = 0;
        while i < n { self.rst(i); i = i + 1; }
        self.statm("striped=");
        i = 0;
        while i < n { self.rlf(i); i = i + 1; }
        self.statm("lockfree=");
    }
}
fn main() { App { }; }
"#,
        "retire-sync",
    );
    let g = phase_growth(&out);
    eprintln!("resident growth per phase (bytes): {:?}", g);
    for mode in ["plain", "serialized", "striped", "lockfree"] {
        assert!(
            g[mode] < MIB / 2,
            "spec/styleguide.md §7 says a replaced `@form(hashmap)` cell \
             retires under every `sync =` discipline; a `{}` map whose cell \
             was replaced 200 000 times grew {} bytes:\n{}",
            mode,
            g[mode],
            out
        );
    }
}

// ---- 2.11, S15 and S16: what a running program shows ----------------

/// 2.11: the wire the shapes cross as. An absent presence pair is no
/// key and its flag is never written (a `null`, or a flag sent alone,
/// is an absence); a wire map is an object keyed by its field, and a
/// repeated key is `duplicate_key`; an enum is its variant, and an
/// absent enum field is `missing_field`; a set is the enum it is, the
/// member's type name its tag (2.9).
#[test]
fn claim_wire_shapes_cross_as_the_guide_says() {
    let out = build_and_run(
        r#"
type Status = enum { Up, Degraded(String), Down };
type Check { name: String; ok: Bool; }
type Reply {
    n: Int;
    note: String = "";
    has_note: Bool = false;
    checks: bounded[Check; 4] indexed_by name;
}
type Probe { status: Status; }
type NewOrder { id: Int; }
type Cancel { id: Int; }
type Inbound = NewOrder | Cancel;
type Env { m: Inbound; }
fn main() {
    let mut r = Reply { n: 1 };
    println(Reply::api_encode(r));
    r.note = "x";
    r.has_note = true;
    push(r.checks, Check { name: "db", ok: true }) or discard;
    println(Reply::api_encode(r));
    let back = Reply::api_decode("{\"n\":2,\"note\":null,\"has_note\":true}") or Reply { n: -1 };
    println(back.has_note);
    let dup = Reply::api_decode("{\"n\":2,\"checks\":{\"db\":{\"ok\":true},\"db\":{\"ok\":false}}}") or { println(err.kind, ": ", err.field); Reply { n: -1 } };
    println(Probe::api_encode(Probe { status: Status::Up }), " ", Probe::api_encode(Probe { status: Status::Degraded("cold") }));
    let p = Probe::api_decode("{}") or { println(err.kind, ": ", err.field); Probe { status: Status::Down } };
    println(Env::api_encode(Env { m: Cancel { id: 1 } }));
}
"#,
        "wire-shapes",
    );
    let want = [
        "{\"n\":1,\"checks\":{}}",
        "{\"n\":1,\"note\":\"x\",\"checks\":{\"db\":{\"ok\":true}}}",
        "false",
        "duplicate_key: checks",
        "{\"status\":\"Up\"} {\"status\":{\"Degraded\":\"cold\"}}",
        "missing_field: status",
        "{\"m\":{\"Cancel\":{\"id\":1}}}",
    ];
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(
        got, want,
        "spec/styleguide.md 2.11 (and 2.9's last bullet) describe these \
         encodings. If the codec changed, update the guide with it"
    );
}

/// The growth of the peak resident set (`VmHWM`, kB) the program
/// reports for one run of `mode` over `n` rows.
fn peak_growth_kb(bin: &std::path::Path, mode: &str, n: usize) -> i64 {
    let run = Command::new(bin)
        .arg(mode)
        .arg(n.to_string())
        .output()
        .expect("run program");
    let text = output_text(&run);
    assert!(run.status.success(), "{} {} failed:\n{}", mode, n, text);
    text.lines()
        .find_map(|l| l.strip_prefix("grew_kb="))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_else(|| panic!("no grew_kb= line from {} {}:\n{}", mode, n, text))
}

/// S15 and S16: a String grown with `+` in a loop, and a text read by
/// slicing off the rest, hold memory quadratic in the text, because
/// every intermediate stays in the method's scratch until it returns;
/// a `BytesBuilder`, and reading by position over a `ByteView`, hold it
/// once. Twice the rows is four times the memory for the first two.
#[test]
fn claim_text_built_or_read_by_copy_is_quadratic() {
    if asan_build() {
        eprintln!("skipped under LOTUS_ASAN: a resident-set budget says nothing past ASan's quarantine");
        return;
    }
    let dir = seed_dir(
        r#"
fn peak_kb() -> Int {
    let status = std::io::fs::read_file("/proc/self/status") or "";
    let at = std::str::index_of(status, "VmHWM:");
    if at < 0 { return 0; }
    let rest = std::str::trim(status[at + 6..len(status)]);
    return std::str::parse_int(rest[0..std::str::index_of(rest, " ")]) or 0;
}
fn row() -> String { return std::str::repeat("x", 499) + "\n"; }
fn by_concat(n: Int) -> Int {
    let mut out = "";
    let mut i = 0;
    while i < n { out = out + row(); i = i + 1; }
    return len(out);
}
fn text(n: Int) -> String {
    let b = std::bytes::BytesBuilder { initial_cap: 4096 };
    let mut i = 0;
    while i < n { b.append_str(row()); i = i + 1; }
    return std::str::from_bytes(b.snapshot() or b"");
}
fn by_rest(all: String) -> Int {
    let mut rest = all;
    let mut k = 0;
    let mut nl = std::str::index_of(rest, "\n");
    while nl >= 0 { k = k + 1; rest = rest[(nl + 1)..len(rest)]; nl = std::str::index_of(rest, "\n"); }
    return k;
}
fn line_end(v: std::str::ByteView, start: Int) -> Int {
    let mut i = start;
    while i < v.n { if std::str::byte_at(v, i) == 10 { return i; } i = i + 1; }
    return v.n;
}
fn by_position(all: String) -> Int {
    let v = std::str::bytes_view(all);
    let mut k = 0;
    let mut start = 0;
    while start < v.n { let nl = line_end(v, start); let line = std::str::slice(v, start, nl); start = nl + 1; if len(line) > 0 { k = k + 1; } }
    return k;
}
fn main() {
    let mode = std::env::arg(1);
    let n = std::str::parse_int(std::env::arg(2)) or 0;
    let mut all = "";
    if mode == "rest" || mode == "position" { all = text(n); }
    let before = peak_kb();
    let mut r = 0;
    if mode == "concat" { r = by_concat(n); }
    if mode == "builder" { r = len(text(n)); }
    if mode == "rest" { r = by_rest(all); }
    if mode == "position" { r = by_position(all); }
    println("result=", r);
    println("grew_kb=", peak_kb() - before);
}
"#,
        "quadratic-text",
    );
    let bin = dir.join("prog");
    let build = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(&dir)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("run hale build");
    assert!(build.status.success(), "build failed:\n{}", output_text(&build));
    let concat_half = peak_growth_kb(&bin, "concat", 500);
    let concat = peak_growth_kb(&bin, "concat", 1000);
    let builder = peak_growth_kb(&bin, "builder", 1000);
    let rest = peak_growth_kb(&bin, "rest", 1000);
    let position = peak_growth_kb(&bin, "position", 1000);
    let _ = std::fs::remove_dir_all(&dir);
    eprintln!(
        "peak growth (kB) over 500 KB of text: concat {} (half the rows: {}), builder {}, rest {}, position {}",
        concat, concat_half, builder, rest, position
    );
    assert!(
        concat > 100 * 1024 && concat > 3 * concat_half,
        "spec/styleguide.md S15 says `out = out + row` in a loop holds \
         memory quadratic in the text (about 250 MB for 500 KB in 1 000 \
         rows, four times what half the rows hold). It grew {} kB, and \
         {} kB for half: if the concat no longer copies, retire S15",
        concat,
        concat_half
    );
    assert!(
        builder < 16 * 1024,
        "S15 says a BytesBuilder holds the text once; it grew {} kB",
        builder
    );
    assert!(
        rest > 100 * 1024,
        "spec/styleguide.md S16 says slicing off the rest is quadratic \
         (about 250 MB for 500 KB in 1 000 lines); it grew {} kB",
        rest
    );
    assert!(
        position < 8 * 1024,
        "S16 says reading by position over a ByteView holds next to \
         nothing; it grew {} kB",
        position
    );
}

/// 2.10: a closed seam's frame is a turn, a method of its own that `run`
/// calls, so 20 000 frames hold flat, as the same chain does as a
/// callable seam called once per frame from a method. (Before the turn,
/// the chain was inlined into the one activation of `run` and kept about
/// 1.7 KB a frame, 34 MB over these frames, until the locus dissolved.)
#[test]
fn a_closed_seam_holds_flat_over_its_frames() {
    if asan_build() {
        eprintln!("skipped under LOTUS_ASAN: a resident-set budget says nothing past ASan's quarantine");
        return;
    }
    let out = build_and_run(
        r#"
type NewOrder { id: Int; note: String; }
type Cancel { id: Int; }
type Ack { id: Int; text: String; }
type Inbound = NewOrder | Cancel;
type Ended { why: String; }
locus Wire {
    fn decode(frame: String) -> Inbound {
        if frame == "cancel" { return Cancel { id: 1 }; }
        return NewOrder { id: 2, note: frame + " " + frame };
    }
    fn encode(a: Ack) -> String { return "ack " + to_string(a.id) + a.text; }
}
locus Desk {
    fn new_order(n: NewOrder) -> Ack { return Ack { id: n.id, text: n.note + "!" }; }
    fn cancel(c: Cancel) -> Ack { return Ack { id: c.id, text: "c" }; }
}
locus Feed {
    params { at: Int = 0; n: Int = 0; }
    fn next() -> String fallible(Ended) {
        self.at = self.at + 1;
        if self.at > self.n { fail Ended { why: "end" }; }
        return std::str::repeat("x", 200);
    }
}
locus Log { params { k: Int = 0; } fn write(line: String) { self.k = self.k + len(line); } }
locus Closed {
    params { feed: Feed = Feed { }; wire: Wire = Wire { }; desk: Desk = Desk { }; log: Log = Log { }; }
    seam {
        self.feed.next() -> self.wire.decode -> self.desk -> self.wire.encode -> self.log.write;
        or end;
    }
}
locus Stepped {
    params { feed: Feed = Feed { }; wire: Wire = Wire { }; desk: Desk = Desk { }; log: Log = Log { }; }
    seam answer(frame: String) { frame -> self.wire.decode -> self.desk -> self.wire.encode -> self.log.write; }
    fn step() -> Bool {
        let f = self.feed.next() or { return false; };
        self.answer(f);
        return true;
    }
    run() { while !self.draining { if !self.step() { return; } } }
}
fn statm(tag: String) {
    print(tag);
    println(std::io::fs::read_file("/proc/self/statm") or "");
}
fn main() {
    Stepped { feed: Feed { n: 1000 } };
    statm("start=");
    Stepped { feed: Feed { n: 20000 } };
    statm("stepped=");
    Closed { feed: Feed { n: 20000 } };
    statm("closed=");
}
"#,
        "closed-seam",
    );
    let g = phase_growth(&out);
    eprintln!("resident growth per phase (bytes): {:?}", g);
    assert!(
        g["stepped"] < MIB,
        "a callable seam called once per frame from a method holds \
         flat; 20 000 frames grew \
         {} bytes:\n{}",
        g["stepped"],
        out
    );
    assert!(
        g["closed"] < MIB,
        "spec/styleguide.md 2.10 says a closed seam frees each frame at \
         the end of its turn; 20 000 frames grew {} bytes:\n{}",
        g["closed"],
        out
    );
}
