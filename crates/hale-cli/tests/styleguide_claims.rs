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
//! fail at build (`s[i]`, a qualified `std::time::Time`, an `Int`
//! returned as a `Float`) are pinned as build-time refusals beside a
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

/// §7 sharp edges: "Write the stdlib's time point bare: `Time`, not
/// `std::time::Time`." The bare spelling, as a field and as a cell,
/// must build; the qualified one passes check and fails at build.
#[test]
fn claim_qualified_std_time_time_fails_build() {
    let (_, bare_failed, bare_out) = check_then_build(
        "type Stamp { key: Int; at: Time; }\n\
         @form(vec)\n\
         locus Times { capacity { heap items of Time; } }\n\
         fn main() {\n\
             let s = Stamp { key: 1, at: Time(7s) };\n\
             let d: std::time::Duration = 3s;\n\
             let t = Times { };\n\
             t.push(s.at);\n\
             println(t.len(), \" \", d);\n\
         }",
        "time-bare",
    );
    assert!(
        !bare_failed,
        "§7 tells readers to write `Time` bare (and says \
         `std::time::Duration` builds); both must build:\n{}",
        bare_out
    );

    let (checked, failed, out) = check_then_build(
        "type Stamp { key: Int; at: std::time::Time; }\n\
         @form(vec)\n\
         locus Times { capacity { heap items of std::time::Time; } }\n\
         fn main() {\n\
             let s = Stamp { key: 1, at: Time(7s) };\n\
             let t = Times { };\n\
             t.push(s.at);\n\
             println(t.len());\n\
         }",
        "time-qualified",
    );
    assert!(
        failed && out.contains("not in stdlib path-renames table"),
        "spec/styleguide.md §7 says `std::time::Time` fails at build. It \
         builds now — delete the \"Write the stdlib's time point bare\" \
         entry and the residue in C7:\n{}",
        out
    );
    assert!(
        checked,
        "spec/styleguide.md §7 says `hale check` accepts a qualified \
         `std::time::Time`. It refuses it now — reword the entry:\n{}",
        out
    );
}

/// §7 sharp edges: "An `Int` returned from a `-> Float` fn typechecks
/// and fails at build." A fix is in progress; when it lands this
/// fails, and the entry goes.
#[test]
fn claim_int_returned_from_a_float_fn_fails_build() {
    let (checked, failed, out) = check_then_build(
        "fn ratio(n: Int) -> Float { return n; }\n\
         fn main() { println(ratio(3)); }",
        "float-return",
    );
    assert!(
        failed && out.contains("return type mismatch"),
        "spec/styleguide.md §7 says an `Int` returned from a `-> Float` \
         fn fails at build. It builds now — delete that sharp edge:\n{}",
        out
    );
    assert!(
        checked,
        "spec/styleguide.md §7 says `hale check` accepts an `Int` \
         returned from a `-> Float` fn. It refuses it now — the edge \
         is no longer build-only; reword or delete the entry:\n{}",
        out
    );
}

// ---- retirement gaps: a running program's resident set ------------

/// §7 open gaps: "Nested-compound fields of a replaced struct don't
/// retire, nor Bytes fields of a replaced `@form(hashmap)` cell",
/// beside a String-only struct, which does. Measured on v0.22.0 over
/// 200 000 replaces: the control +4 KiB, nested +6.0 MiB, map Bytes
/// +4.6 MiB.
#[test]
fn claim_replaced_nested_and_map_bytes_fields_do_not_retire() {
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
        g["nested"] > MIB,
        "spec/styleguide.md §7 says nested-compound fields of a replaced \
         struct don't retire (+6.0 MiB over 200 000 replaces on v0.22.0). \
         This run grew {} bytes — they retire now: update that open gap, \
         §1's \"Replace and let it retire\" and S4:\n{}",
        g["nested"],
        out
    );
    assert!(
        g["map_bytes"] > MIB,
        "spec/styleguide.md §7 says a Bytes field of a replaced \
         `@form(hashmap)` cell doesn't retire (+4.6 MiB over 200 000 \
         replaces on v0.22.0). This run grew {} bytes — it retires now: \
         update that open gap:\n{}",
        g["map_bytes"],
        out
    );
}

/// §7 open gaps: "`striped` / `lockfree` `@form` maps don't retire
/// replaced cells", beside a plain map and a `serialized` one, which
/// do. Measured on v0.22.0 over 200 000 replaces of 16 keys: plain
/// and serialized +4 KiB each, striped +2.3 MiB, lockfree +2.4 MiB.
#[test]
fn claim_striped_and_lockfree_maps_do_not_retire() {
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
    for control in ["plain", "serialized"] {
        assert!(
            g[control] < MIB / 2,
            "the control — a `{}` map whose cell is replaced 200 000 \
             times — must hold flat (§7 says `sync = serialized` retires \
             since 2026-08-03); it grew {} bytes:\n{}",
            control,
            g[control],
            out
        );
    }
    for mode in ["striped", "lockfree"] {
        assert!(
            g[mode] > MIB,
            "spec/styleguide.md §7 says a `sync = {}` map doesn't retire \
             replaced cells (~2.3 MiB over 200 000 replaces on v0.22.0). \
             This run grew {} bytes — it retires now: update the \
             \"`striped` / `lockfree` `@form` maps don't retire\" open \
             gap:\n{}",
            mode,
            g[mode],
            out
        );
    }
}
