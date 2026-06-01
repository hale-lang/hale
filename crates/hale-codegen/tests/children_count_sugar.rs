//! F.11 entity-collection sugar: `self.children.count` (Int) and
//! `self.children.is_empty` (Bool) read the accept'd-child tracker's
//! live count, instead of hand-rolling a `for child in self.children`
//! counter loop. Lowers to a load of the `__child_count` field.

use std::process::Command;

use hale_codegen::build_executable;

#[test]
fn children_count_and_is_empty() {
    let src = r#"
        locus Worker { params { id: Int = 0; } }
        locus Mgr {
            params { n: Int = 0; }
            accept(c: Worker) { }
            birth() {
                let mut i: Int = 0;
                while i < self.n { Worker { id: i }; i = i + 1; }
            }
            fn report() {
                println("count=", self.children.count);
                if self.children.is_empty {
                    println("EMPTY");
                } else {
                    println("NONEMPTY");
                }
            }
        }
        main locus App {
            params {
                full:  Mgr = Mgr { n: 7 };
                empty: Mgr = Mgr { n: 0 };
            }
            run() {
                self.full.report();
                self.empty.report();
            }
        }
        fn main() { App { }; }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_children_count_{}", std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "non-zero exit; stdout: {stdout}");
    assert!(
        stdout.contains("count=7") && stdout.contains("NONEMPTY"),
        "expected count=7 + NONEMPTY for the 7-child Mgr; stdout: {stdout}"
    );
    assert!(
        stdout.contains("count=0") && stdout.contains("EMPTY"),
        "expected count=0 + EMPTY for the 0-child Mgr; stdout: {stdout}"
    );
}
