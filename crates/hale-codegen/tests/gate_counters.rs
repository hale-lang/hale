//! Test-time gate counters (#7 of the fast-protocol-I/O substrate plan):
//! `std::diag::heap_alloc_count()` and `std::diag::syscall_count(name)` read
//! the runtime's `__wrap_*` counters so a steady-state region can assert it
//! did zero heap allocations / exactly one syscall per poll. These are the
//! runtime/test-time complement to compile-time `--warn-unbounded-alloc`.

use std::process::Command;

use hale_codegen::build_executable;

fn pick_free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    l.local_addr().expect("local_addr").port()
}

fn build_and_run_argv(name: &str, src: &str, argv: &[&str]) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_gate_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).args(argv).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn heap_alloc_gate_zero_in_arithmetic_loop_nonzero_when_allocating() {
    // The gate must be available in a default build (not -1), a pure
    // arithmetic loop must allocate nothing (delta 0 — the steady-state
    // assertion), and a loop that builds bytes must move the counter
    // (proving it actually counts, not just always-0).
    let src = r#"
        fn main() {
            let avail = std::diag::heap_alloc_count();
            let a0 = std::diag::heap_alloc_count();
            let mut i = 0;
            let mut sum = 0;
            while i < 1000000 {
                sum = sum + i;
                i = i + 1;
            }
            let a1 = std::diag::heap_alloc_count();

            let b0 = std::diag::heap_alloc_count();
            let bld = std::bytes::BytesBuilder { };
            let mut j = 0;
            while j < 100000 {
                bld.append(std::bytes::from_int(65));
                j = j + 1;
            }
            let b1 = std::diag::heap_alloc_count();

            println("avail_ok=", avail >= 0);
            println("noalloc_delta=", a1 - a0);
            println("grew=", (b1 - b0) > 0, " sum_ok=", sum > 0);
        }
    "#;
    let (stdout, status) = build_and_run_argv("heap", src, &[]);
    assert!(status.success(), "exit: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("avail_ok=true"), "gate should be available in a default build; got: {:?}", stdout);
    assert!(
        stdout.contains("noalloc_delta=0"),
        "a pure arithmetic loop must do zero heap allocations; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("grew=true"),
        "the byte-building loop must move the heap counter; got: {:?}",
        stdout
    );
}

#[test]
fn syscall_gate_counts_one_read_per_recv_into() {
    let port = pick_free_port();
    // Loopback connect + accept, set a short recv timeout on the accepted
    // fd, then one recv_into with no data pending: recv_into does exactly
    // one read() (EAGAIN → the -2 timeout sentinel, no retry loop), so the
    // read counter moves by exactly 1.
    let src = r#"
        fn main() {
            let port = std::str::parse_int(std::env::arg(1)) or raise;
            let lfd = std::io::tcp::__listen_socket("127.0.0.1", port);
            let cfd = std::io::tcp::__connect("127.0.0.1", port);
            let afd = std::io::tcp::__accept_one(lfd);
            std::io::tcp::set_recv_timeout(afd, 150ms) or raise;
            let r0 = std::diag::syscall_count("read");
            let bld = std::bytes::BytesBuilder { };
            let got = std::io::tcp::recv_into(afd, bld, 256);
            let r1 = std::diag::syscall_count("read");
            println("reads=", r1 - r0, " got=", got);
        }
    "#;
    let (stdout, status) = build_and_run_argv("syscall", src, &[&port.to_string()]);
    assert!(status.success(), "exit: {:?}\nstdout: {}", status, stdout);
    assert!(
        stdout.contains("reads=1"),
        "one recv_into must be exactly one read() syscall; got: {:?}",
        stdout
    );
}
