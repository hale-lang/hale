//! WS1#4 — whole-value reassignment of a nested locus param that
//! holds an `@ffi`-acquired resource handle.
//!
//! a downstream app (refgw-evm `set_rpc_ws`) reported that `self.conn =
//! ws::WsClient { url: …, … }` to swap an endpoint left the new
//! client half-initialized — `conn.url` logged `(null)` and the
//! first `read_msg()` cored — while in-place mutation
//! (`self.conn.url = …`) worked. My earlier synthetic probes
//! (`ws1_cross_seed_locus_reassign`) used a plain `Int` "handle"
//! set in `birth()` and passed, so the residual is specific to a
//! locus that holds a *real FFI resource* and whose `birth()`
//! acquires it. This models that faithfully:
//!
//!   - `Conn.birth()` acquires an `@ffi` handle (a fake fd from a C
//!     side-table that tracks live handles).
//!   - `Conn.drain()` releases it.
//!   - `Gw` holds `conn: Conn` as a nested param and reassigns the
//!     WHOLE locus in `reconnect()`.
//!
//! The correct semantics (what reassignment SHOULD do) is a
//! lifecycle transition: dissolve the old (→ release its handle)
//! and fully instantiate the new (params + birth → acquire a new
//! handle). The test observes both:
//!   - new instance fully initialized? (url = "second", fd live)
//!   - old instance released?           (live-handle count back to 1)
//! A half-init (empty url / dead fd) or a leak (count climbs) or a
//! crash all fail it.

use std::process::Command;

use hale_codegen::{build_executable_with_options, BuildOptions};

fn build_with_csrc(name: &str, hale_src: &str, csrc_body: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(hale_src).expect("parse");
    let mut tmpdir = std::env::temp_dir();
    tmpdir.push(format!("hale_test_ws1_ffi_reassign_{}", name));
    let _ = std::fs::create_dir_all(&tmpdir);
    let csrc_path = tmpdir.join("glue.c");
    std::fs::write(&csrc_path, csrc_body).expect("write csrc");
    let bin = tmpdir.join("main");
    let options = BuildOptions {
        link_libs: Vec::new(),
        csrc_files: vec![csrc_path.clone()],
        ..Default::default()
    };
    build_executable_with_options(&program, &bin, &[], &options).expect("build");
    let _ = std::fs::remove_file(&csrc_path);
    bin
}

const GLUE: &str = r#"
    #include <stdint.h>
    static int g_open[4096];
    static int64_t g_next = 1;
    static int64_t g_live = 0;
    int64_t ffi_conn_open(const char *url) {
        (void)url;
        int64_t h = g_next++;
        if (h >= 0 && h < 4096) g_open[h] = 1;
        g_live++;
        return h;
    }
    void ffi_conn_close(int64_t h) {
        if (h > 0 && h < 4096 && g_open[h]) { g_open[h] = 0; g_live--; }
    }
    int64_t ffi_open_count(void) { return g_live; }
    int64_t ffi_handle_alive(int64_t h) {
        return (h > 0 && h < 4096 && g_open[h]) ? 1 : 0;
    }
"#;

const SRC: &str = r#"
    @ffi("c") fn ffi_conn_open(url: String) -> Int;
    @ffi("c") fn ffi_conn_close(h: Int) -> ();
    @ffi("c") fn ffi_open_count() -> Int;
    @ffi("c") fn ffi_handle_alive(h: Int) -> Int;

    locus Conn {
        params { url: String = ""; fd: Int = 0; }
        birth() { self.fd = ffi_conn_open(self.url); }
        drain() { ffi_conn_close(self.fd); }
        fn url_of() -> String { return self.url; }
        fn fd_of() -> Int { return self.fd; }
        fn alive() -> Int { return ffi_handle_alive(self.fd); }
    }

    locus Gw {
        params { conn: Conn = Conn { url: "wss://first" }; }
        fn reconnect() { self.conn = Conn { url: "wss://second" }; }
        run() {
            println("init   url=", self.conn.url_of(),
                    " fd=", self.conn.fd_of(),
                    " alive=", self.conn.alive(),
                    " live=", ffi_open_count());
            self.reconnect();
            println("reconn url=", self.conn.url_of(),
                    " fd=", self.conn.fd_of(),
                    " alive=", self.conn.alive(),
                    " live=", ffi_open_count());
        }
    }

    fn main() { Gw { }; }
"#;

// WS1#4 regression gate. `self.conn = Conn {…}` is lowered as a
// lifecycle transition (`lower_locus_field_reassign`): the old
// instance is reclaimed (its `@ffi` handle released) and the new
// one is constructed into self's arena, owned by the field and not
// scope-dissolved — so the field points at a LIVE instance. Was a
// use-after-free + leak before the fix (the RHS was a scope-bound
// temporary dissolved at method exit; the old value leaked).
#[test]
fn ffi_handle_locus_whole_reassign() {
    let bin = build_with_csrc("reassign", SRC, GLUE);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // Diagnostic first — print what actually happened regardless of
    // pass/fail, so a reproduction is legible in CI logs.
    eprintln!("--- stdout ---\n{}\n--- status: {:?} ---\n{}", stdout, out.status, stderr);

    assert!(
        out.status.success(),
        "binary crashed on reassignment (WS1#4 — segfault): {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        stdout,
        stderr
    );
    let reconn = stdout
        .lines()
        .find(|l| l.starts_with("reconn "))
        .unwrap_or("")
        .to_string();
    // The reassigned-in instance must be LIVE when read after the
    // reassignment: its url set, AND its `@ffi` handle still open.
    assert!(
        reconn.contains("url=wss://second"),
        "new instance's url not set by reassignment. reconn line: {:?}",
        reconn
    );
    assert!(
        reconn.contains("alive=1"),
        "WS1#4 REPRODUCES: after `self.conn = Conn {{…}}` the new \
         instance's handle is already dead — the RHS locus literal was \
         dissolved as a scope-bound temporary, so `self.conn` points at \
         a torn-down locus (in a downstream app: closed socket → read_msg cores). \
         reconn line: {:?}",
        reconn
    );
    // Exactly one handle open after reconnect AND it's the live new
    // one (asserted above) ⇒ the old was released. A leak shows as
    // the new being dead (alive=0) while live stays 1 = the OLD.
    assert!(
        reconn.contains("live=1"),
        "WS1#4: handle accounting wrong after reassignment (old leaked \
         and/or new not opened). reconn line: {:?}",
        reconn
    );
}
