//! Phase-0 in-place Bytes ops — the recv-loop accumulator surface
//! that pond/websocket flagged as a substrate gap (FRICTION § "per-
//! frame Bytes allocations accumulate"). Now reached via the
//! BytesBuilder locus's method surface (shift_front / clear /
//! snapshot) so a long-lived holder can recycle a single allocation
//! across many iterations. Free is implicit via dissolve at scope
//! exit — no more explicit `builder_free` call.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_test_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn builder_shift_front_drops_leading_bytes() {
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("hello world"));
            b.shift_front(6);
            let snap = b.snapshot();
            println("len=", b.len());
            println("body=", std::str::from_bytes(snap));
        }
    "#;
    let (stdout, status) = build_and_run("bb_shift_front", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("len=5"), "got: {:?}", stdout);
    assert!(stdout.contains("body=world"), "got: {:?}", stdout);
}

#[test]
fn builder_shift_front_past_len_empties() {
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("xyz"));
            b.shift_front(100);
            println("len=", b.len());
        }
    "#;
    let (stdout, status) = build_and_run("bb_shift_past", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("len=0"), "got: {:?}", stdout);
}

#[test]
fn builder_clear_keeps_capacity_drops_len() {
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("abcdef"));
            b.clear();
            println("after_clear=", b.len());
            b.append(std::bytes::from_string("xy"));
            println("after_append=", b.len());
            let snap = b.snapshot();
            println("body=", std::str::from_bytes(snap));
        }
    "#;
    let (stdout, status) = build_and_run("bb_clear", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("after_clear=0"), "got: {:?}", stdout);
    assert!(stdout.contains("after_append=2"), "got: {:?}", stdout);
    assert!(stdout.contains("body=xy"), "got: {:?}", stdout);
}

#[test]
fn builder_snapshot_leaves_builder_unchanged() {
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("snap-me"));
            let s1 = b.snapshot();
            let s2 = b.snapshot();
            println("len_after=", b.len());
            println("s1=", std::str::from_bytes(s1));
            println("s2=", std::str::from_bytes(s2));
        }
    "#;
    let (stdout, status) = build_and_run("bb_snapshot", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("len_after=7"), "got: {:?}", stdout);
    assert!(stdout.contains("s1=snap-me"), "got: {:?}", stdout);
    assert!(stdout.contains("s2=snap-me"), "got: {:?}", stdout);
}

#[test]
fn recv_loop_simulation_recycles_capacity() {
    // The shape that motivates Phase 0: a recv loop that appends
    // a chunk, peels a fixed-length frame off the front, then
    // repeats. Builder len cycles back to 0 each iteration and
    // capacity stays put (no fresh malloc per frame).
    //
    // We can't directly assert the malloc count from Aperio, but
    // we can verify the body cycles cleanly and the final state
    // is empty.
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            let frame_a = std::bytes::from_string("AAAA");
            let frame_b = std::bytes::from_string("BBBB");
            let mut i = 0;
            while i < 100 {
                b.append(frame_a);
                b.append(frame_b);
                b.shift_front(4);
                b.shift_front(4);
                i = i + 1;
            }
            println("final_len=", b.len());
            b.append(std::bytes::from_string("done"));
            let snap = b.snapshot();
            println("body=", std::str::from_bytes(snap));
        }
    "#;
    let (stdout, status) = build_and_run("bb_recv_sim", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("final_len=0"), "got: {:?}", stdout);
    assert!(stdout.contains("body=done"), "got: {:?}", stdout);
}
