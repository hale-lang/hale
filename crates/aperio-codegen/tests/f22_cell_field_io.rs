//! v1.x-2: F.22 struct-cell field IO. Cells of struct types
//! support `cell.field = v;` writes and `cell.field` reads,
//! same as TypeRef-shaped struct values. Primitive cells
//! (Cell<Int> etc.) still reject field access at v1.

use std::process::Command;

use aperio_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_f22_cell_io_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn struct_cell_field_write_then_read_round_trip() {
    let src = r#"
        type Entry { key: Int; value: Int; }
        locus MapL {
            capacity {
                pool entries of Entry;
            }
            birth {
                let cell = self.entries.acquire();
                cell.key = 42;
                cell.value = 99;
                println("key=", cell.key);
                println("value=", cell.value);
            }
        }
        fn main() {
            let _ = MapL { };
        }
    "#;
    let bin = build("struct_field_io", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("key=42"), "missing key: {:?}", stdout);
    assert!(stdout.contains("value=99"), "missing value: {:?}", stdout);
}

#[test]
fn heap_cell_field_io() {
    let src = r#"
        type Record { tag: Int; }
        locus HeapL {
            capacity {
                heap records of Record;
            }
            birth {
                let r = self.records.alloc();
                r.tag = 7;
                println("tag=", r.tag);
            }
        }
        fn main() {
            let _ = HeapL { };
        }
    "#;
    let bin = build("heap_field_io", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("tag=7"), "missing log: {:?}", stdout);
}

#[test]
fn primitive_cell_field_access_rejected() {
    // Cell<Int> doesn't have fields — field access errors at
    // build time with a focused message.
    let src = r#"
        locus IntPoolL {
            capacity {
                pool entries of Int;
            }
            birth {
                let c = self.entries.acquire();
                let _ = c.foo;
            }
        }
        fn main() { }
    "#;
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("aperio_test_f22_cell_io_primitive_rejected");
    let err = build_executable(&program, &bin)
        .expect_err("primitive cell field access should reject");
    let msg = format!("{}", err);
    assert!(
        msg.to_lowercase().contains("cell") || msg.contains("primitive"),
        "expected cell-primitive diagnostic, got: {}",
        msg
    );
}

#[test]
fn pool_cell_recycled_post_release_then_rewrite_works() {
    // Cells released to a Pool free-list have their first
    // sizeof(void*) bytes overwritten by the free-list pointer
    // — that's how the embedded free-list threads through cells
    // when they're free. So released cells lose any user data
    // in the first 8 bytes; the rest is preserved.
    //
    // User-facing contract: AFTER acquire, treat the cell as
    // freshly-initialized. Re-write any fields you care about
    // before reading them. This test exercises that contract:
    // acquire → write → release → acquire → REWRITE → read.
    let src = r#"
        type Entry { key: Int; value: Int; }
        locus RecycleL {
            capacity {
                pool entries of Entry;
            }
            birth {
                let c1 = self.entries.acquire();
                c1.key = 1;
                c1.value = 10;
                self.entries.release(c1);
                let c2 = self.entries.acquire();
                c2.key = 2;
                c2.value = 20;
                println("rewritten-key=", c2.key);
                println("rewritten-value=", c2.value);
            }
        }
        fn main() {
            let _ = RecycleL { };
        }
    "#;
    let bin = build("recycled_rewrite", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("rewritten-key=2"),
        "missing key: {:?}",
        stdout
    );
    assert!(
        stdout.contains("rewritten-value=20"),
        "missing value: {:?}",
        stdout
    );
}
