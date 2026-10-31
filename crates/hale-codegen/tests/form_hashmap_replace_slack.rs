//! A `@form(hashmap)` cell replaced with Strings of varying length is
//! reclaimed whole: the map's arena stops growing once every length has
//! been seen.
//!
//! A replaced cell's Strings were retired at their strlen+1 through the
//! arena's general freelist, which hands a block to any request up to a
//! quarter larger. A block reused for a shorter value was retired again
//! at the shorter size, so its slack was lost for good, and a map whose
//! rows were replaced with values of varying length grew its arena
//! without bound: about 2.3 KB per 10,000 replaces, too slowly for RSS
//! to show over a few thousand requests, and without end. From a map's
//! first replace its Strings are field blocks of their size class (the
//! GH #1490 machinery a locus's String fields use), which a replace
//! retires whole onto the class's list.
//!
//! Each program warms every length and key, dumps arena residency, runs
//! to 100,000 cycles and dumps again: the map's arena (labelled with its
//! locus name) must have used the same bytes at both points, and every
//! row must read back as it was last written. The same program under
//! AddressSanitizer checks that no reclaimed block is still read.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The bytes used in the arena labelled `label`, per residency dump.
fn arena_used(stderr: &str, label: &str) -> Vec<u64> {
    let tag = format!("label={label}]");
    stderr
        .lines()
        .filter(|l| l.contains(&tag))
        .map(|l| {
            l.split_whitespace()
                .find_map(|w| w.strip_prefix("used="))
                .and_then(|v| v.parse().ok())
                .expect("a residency row carries used=N")
        })
        .collect()
}

/// `step` is one cycle over `self.m` with `self.n` counting cycles and
/// `v` a String whose length cycles through 0..72; `check` reads every
/// row back after the churn and exits 3 on a mismatch.
fn program(map_decl: &str, step: &str, check: &str) -> String {
    format!(
        r#"
        type In {{ s: String = ""; }}
        type Row {{ k: Int = 0; name: String = ""; note: String = ""; inner: In = In {{ }}; }}
        type Named {{ id: String = ""; name: String = ""; }}
        {map_decl}
        locus H {{
            params {{ m: M = M {{ }}; n: Int = 0; }}
            fn step() {{
                self.n = self.n + 1;
                let v = std::str::repeat("ab", self.n % 37);
                {step}
            }}
            fn check() {{ {check} }}
        }}
        fn main() {{
            let h = H {{ }};
            let mut i = 0;
            while i < 2000 {{ h.step(); i = i + 1; }}
            let _warm = std::process::dump_arena_residency();
            while i < 100000 {{ h.step(); i = i + 1; }}
            let _late = std::process::dump_arena_residency();
            h.check();
        }}
        "#
    )
}

fn assert_flat(name: &str, map_decl: &str, step: &str, check: &str) {
    let src = program(map_decl, step, check);
    let bin = harness::unique_bin(&format!("hale_hashmap_replace_slack_{name}"));
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).env("LOTUS_ARENA_RESIDENCY", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{name}: non-zero {:?}; stderr: {stderr}", out.status);
    let used = arena_used(&stderr, "M");
    assert!(used.len() >= 2, "{name}: expected two residency dumps; stderr: {stderr}");
    assert_eq!(
        used[0], used[1],
        "{name}: the map's arena grew between 2,000 and 100,000 replaces; stderr: {stderr}"
    );

    let asan = harness::unique_bin(&format!("hale_hashmap_replace_slack_{name}_asan"));
    let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    build_opts::build_source(&src, &asan, &options).expect("asan build");
    let out = Command::new(&asan).output().expect("asan run");
    let _ = std::fs::remove_file(&asan);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{name}: under ASan, non-zero {:?}; stderr: {stderr}", out.status);
    assert!(!stderr.contains("AddressSanitizer"), "{name}: {stderr}");
}

/// The last value written under key `k` (cycle `n`'s key is `n % 4`).
const EXPECT: &str = r#"
    let mut k = 0;
    while k < 4 {
        let mut n = self.n;
        while n % 4 != k { n = n - 1; }
        let want = std::str::repeat("ab", n % 37);
"#;

#[test]
fn replaced_strings_of_varying_length_are_reclaimed() {
    assert_flat(
        "top",
        "@form(hashmap) locus M { capacity { pool entries of Row indexed_by k; } }",
        r#"self.m.set(Row { k: self.n % 4, name: v, note: "n:" + v, inner: In { s: v + "!" } });"#,
        &format!(
            r#"{EXPECT}
                let r = self.m.get(k) or Row {{ }};
                if r.name != want || r.note != "n:" + want || r.inner.s != want + "!" {{ std::process::exit(3); }}
                k = k + 1;
            }}"#
        ),
    );
}

/// A String-keyed map: the key is one of the cell's Strings too, kept
/// across the replace (the same key's clone survives).
#[test]
fn a_string_keyed_map_reclaims_its_replaced_values() {
    assert_flat(
        "skey",
        "@form(hashmap) locus M { capacity { pool entries of Named indexed_by id; } }",
        r#"self.m.set(Named { id: "key-" + to_string(self.n % 4), name: v });"#,
        &format!(
            r#"{EXPECT}
                let r = self.m.get("key-" + to_string(k)) or Named {{ }};
                if r.name != want {{ std::process::exit(3); }}
                k = k + 1;
            }}"#
        ),
    );
}

/// Get-modify-set, the update-in-place shape: the row read back and
/// stored with one field changed.
#[test]
fn get_modify_set_of_a_varying_field_is_reclaimed() {
    assert_flat(
        "rmw",
        "@form(hashmap) locus M { capacity { pool entries of Row indexed_by k; } }",
        r#"let mut r = self.m.get(self.n % 4) or Row { k: self.n % 4 }; r.name = v; self.m.set(r);"#,
        &format!(
            r#"{EXPECT}
                let r = self.m.get(k) or Row {{ }};
                if r.name != want {{ std::process::exit(3); }}
                k = k + 1;
            }}"#
        ),
    );
}
