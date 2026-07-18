//! Cell single-owner (2026-07-18): a @form(hashmap) cell must never
//! share a String/Bytes blob with the self-storage struct it was
//! set from — the anchor walk snapshots the value and force-copies
//! same-arena pointers (lotus_*_clone_cell_owned). The probe that
//! proved the pre-fix aliasing deterministically: set(self.rec),
//! then overwrite self.rec.val IN PLACE with a same-length string
//! (the fit path rewrites the blob's bytes) — pre-fix the cell's
//! val visibly changed with it.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_cell_alias_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

#[test]
fn cell_does_not_alias_self_storage() {
    let src = r#"
        type Rec {
            key:  String = "";
            val:  String = "";
        }
        @form(hashmap)
        locus Store {
            capacity { pool recs of Rec indexed_by key; }
        }
        locus App {
            params { m: Store; rec: Rec = Rec { }; }
            fn seed(tag: String) {
                self.rec = Rec {
                    key: "k" + "r",
                    val: "original-" + tag + "-payload"
                };
            }
            fn store_whole_field() {
                self.m.set(self.rec);
            }
            fn mutate_inplace() {
                // Same length as the seeded val: the fit path
                // rewrites the blob bytes in place.
                self.rec.val = "REWRITTEN-in-place!";
            }
            fn churn(i: Int) {
                // Whole-struct replaces retire the old field blobs;
                // if the cell aliased one, the reuse would corrupt
                // or ASan-flag it.
                self.rec = Rec {
                    key: "other-" + i,
                    val: "churned-" + i + "-abcdefghijklmnop"
                };
            }
            fn read_kr() -> String {
                let e = self.m.get("kr") or Rec { };
                return e.val;
            }
        }
        fn main() {
            let app = App { m: Store { } };
            app.seed("vr");
            app.store_whole_field();
            app.mutate_inplace();
            println("after-mutate [", app.read_kr(), "]");
            // The self field itself must have taken the in-place
            // rewrite (single-owner cuts sharing, not semantics).
            println("field [", app.rec.val, "]");
            let mut i = 0;
            while i < 200 {
                app.churn(i);
                i = i + 1;
            }
            println("after-churn [", app.read_kr(), "]");
        }
    "#;
    let (out, status) = build_and_run("self_storage", src);
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    // The cell keeps the seeded value through both the in-place
    // source mutation and the retire/reuse churn.
    assert!(
        out.contains("after-mutate [original-vr-payload]"),
        "cell aliased the self field:\n{}",
        out
    );
    assert!(
        out.contains("after-churn [original-vr-payload]"),
        "cell dangled after source churn:\n{}",
        out
    );
    assert!(out.contains("field [REWRITTEN-in-place!]"), "got:\n{}", out);
}

#[test]
fn get_then_set_under_new_key_owns_its_blobs() {
    let src = r#"
        type Rec {
            key:  String = "";
            val:  String = "";
        }
        @form(hashmap)
        locus Store {
            capacity { pool recs of Rec indexed_by key; }
        }
        locus App {
            params { m: Store; }
            fn seed(tag: String) {
                self.m.set(Rec {
                    key: "k" + "2",
                    val: "original-" + tag + "-payload"
                });
            }
            fn copy_under_new_key() {
                let mut e = self.m.get("k2") or Rec { };
                e.key = "k" + "1";
                self.m.set(e);
            }
            fn churn(i: Int) {
                self.m.set(Rec {
                    key: "k" + "2",
                    val: "churned-" + i + "-abcdefghijklmnop"
                });
            }
            fn read_k1() -> String {
                let e = self.m.get("k1") or Rec { };
                return e.val;
            }
        }
        fn main() {
            let app = App { m: Store { } };
            app.seed("v2");
            app.copy_under_new_key();
            let mut i = 0;
            while i < 200 {
                app.churn(i);
                i = i + 1;
            }
            println("k1 [", app.read_k1(), "]");
        }
    "#;
    let (out, status) = build_and_run("get_set", src);
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    // k2's churn retires ITS old blobs every set; k1's copy must
    // survive every flush + reuse cycle.
    assert!(
        out.contains("k1 [original-v2-payload]"),
        "cross-cell blob was shared:\n{}",
        out
    );
}
