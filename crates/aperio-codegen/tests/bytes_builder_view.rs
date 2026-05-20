//! Phase-2 (1): zero-copy `view()` on the BytesBuilder locus.
//! Returns a non-owning Bytes pointer aliasing the builder's
//! `[i64 len][u8 data]` region — no allocation, no copy. The
//! pond/websocket use case is `parse_frame` reading via
//! `std::bytes::at` / `len` / `slice` over an rx_buf accumulator
//! without snapshotting per peel attempt (the dominant residual
//! leak source after Phase 1).
//!
//! Lifetime is documented-and-trusted at v1: the view is valid
//! until the next mutation on the source builder. The aliasing
//! property is what makes this a substrate primitive — a snapshot
//! could match the byte values but would re-allocate. These tests
//! exercise both the byte-equality contract and the aliasing
//! semantic (a view captured before an append sees stale len; a
//! view captured after sees the new len).

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_test_bb_view_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn view_returns_bytes_with_current_contents() {
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("hello"));
            let v = b.view();
            println("len=", len(v));
            println("b0=", std::bytes::at(v, 0) or -1);
            println("b4=", std::bytes::at(v, 4) or -1);
        }
    "#;
    let (stdout, status) = build_and_run("basic", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("len=5"), "got: {:?}", stdout);
    // 'h' = 0x68 = 104, 'o' = 0x6f = 111
    assert!(stdout.contains("b0=104"), "got: {:?}", stdout);
    assert!(stdout.contains("b4=111"), "got: {:?}", stdout);
}

#[test]
fn view_aliases_buffer_across_appends() {
    // Each view() call returns the CURRENT state. The aliasing
    // property: a view captured AFTER an append reflects the new
    // contents. (The trust contract says don't retain a view
    // across a mutation; we test that fresh views are coherent.)
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("foo"));
            println("v1_len=", len(b.view()));
            b.append(std::bytes::from_string("bar"));
            let v2 = b.view();
            println("v2_len=", len(v2));
            // 'f'=102 'o'=111 'b'=98 'a'=97 'r'=114
            println("v2_b0=", std::bytes::at(v2, 0) or -1);
            println("v2_b3=", std::bytes::at(v2, 3) or -1);
            println("v2_b5=", std::bytes::at(v2, 5) or -1);
        }
    "#;
    let (stdout, status) = build_and_run("aliases", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("v1_len=3"), "got: {:?}", stdout);
    assert!(stdout.contains("v2_len=6"), "got: {:?}", stdout);
    assert!(stdout.contains("v2_b0=102"), "got: {:?}", stdout);
    assert!(stdout.contains("v2_b3=98"),  "got: {:?}", stdout);
    assert!(stdout.contains("v2_b5=114"), "got: {:?}", stdout);
}

#[test]
fn view_composes_with_slice() {
    // Once `view()` returns a Bytes, the whole Bytes surface
    // works on it — including slice, which copies into the bus
    // payload arena. (slice still allocates; the view-then-slice
    // path is right when the caller needs a stable Bytes — not
    // when the caller only needs to read via at/len.)
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("hello world"));
            let mid = std::bytes::slice(b.view(), 6, 11);
            println("mid_len=", len(mid));
            println("mid_b0=", std::bytes::at(mid, 0) or -1);  // 'w' = 119
            println("mid_b4=", std::bytes::at(mid, 4) or -1);  // 'd' = 100
        }
    "#;
    let (stdout, status) = build_and_run("slice", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("mid_len=5"), "got: {:?}", stdout);
    assert!(stdout.contains("mid_b0=119"), "got: {:?}", stdout);
    assert!(stdout.contains("mid_b4=100"), "got: {:?}", stdout);
}

#[test]
fn view_reflects_shift_front() {
    // After shift_front, view() reflects the dropped prefix.
    // This is exactly the pond/websocket recv-loop pattern: the
    // peel point drops consumed bytes via shift_front; subsequent
    // view() calls see the remaining buffer.
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("AAAA-BBBB"));
            b.shift_front(5);
            let v = b.view();
            println("len=", len(v));
            // After shift: "BBBB". 'B' = 66.
            println("b0=", std::bytes::at(v, 0) or -1);
            println("b3=", std::bytes::at(v, 3) or -1);
        }
    "#;
    let (stdout, status) = build_and_run("shift", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("len=4"), "got: {:?}", stdout);
    assert!(stdout.contains("b0=66"), "got: {:?}", stdout);
    assert!(stdout.contains("b3=66"), "got: {:?}", stdout);
}

#[test]
fn view_after_clear_is_empty() {
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("ignored"));
            b.clear();
            let v = b.view();
            println("len=", len(v));
        }
    "#;
    let (stdout, status) = build_and_run("clear", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("len=0"), "got: {:?}", stdout);
}

#[test]
fn text_view_returns_string_aliasing_buffer() {
    // Phase-3 Site 2: text_view returns a non-owning String
    // pointer aliasing the builder's buffer. Length comes from
    // the C-string terminator that the builder maintains at
    // buf[len] after every mutation.
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("hello world"));
            let s = b.text_view();
            println("len=", len(s));
            println("s=", s);
        }
    "#;
    let (stdout, status) = build_and_run("text_view_basic", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("len=11"), "got: {:?}", stdout);
    assert!(stdout.contains("s=hello world"), "got: {:?}", stdout);
}

#[test]
fn text_view_updates_after_mutations() {
    // Each text_view() call returns the CURRENT state. Aliasing
    // contract: don't retain across a mutation; fresh views are
    // coherent. Mirrors the bytes view test shape.
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("foo"));
            let v1 = b.text_view();
            println("v1=", v1);
            b.append(std::bytes::from_string("bar"));
            let v2 = b.text_view();
            println("v2=", v2);
            b.shift_front(3);
            let v3 = b.text_view();
            println("v3=", v3);
            b.clear();
            let v4 = b.text_view();
            println("v4=", v4);
            println("v4_len=", len(v4));
        }
    "#;
    let (stdout, status) = build_and_run("text_view_mut", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("v1=foo"), "v1: {:?}", stdout);
    assert!(stdout.contains("v2=foobar"), "v2: {:?}", stdout);
    assert!(stdout.contains("v3=bar"), "v3: {:?}", stdout);
    assert!(stdout.contains("v4_len=0"), "v4_len: {:?}", stdout);
}

#[test]
fn text_view_survives_grow_realloc() {
    // initial_cap small, append more than fits → realloc fires.
    // text_view after the grow must still be NUL-terminated at
    // buf[len] — the realloc path must preserve the +1 reserve.
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 4 };
            b.append(std::bytes::from_string("this is much longer than four"));
            let s = b.text_view();
            println("len=", len(s));
            println("s=", s);
        }
    "#;
    let (stdout, status) = build_and_run("text_view_grow", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("len=29"), "got: {:?}", stdout);
    assert!(
        stdout.contains("s=this is much longer than four"),
        "got: {:?}",
        stdout
    );
}
