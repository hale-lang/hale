//! Crumb batch-2 item 1 — C→Hale re-entry (`@export fn`, native).
//!
//! The port's critical-path shape: Hale calls into C through
//! `@ffi("c")` (QuickJS in Crumb; a plain driver here), and the C
//! code calls BACK into Hale through `@export fn` symbols — same
//! thread, during the in-flight `@ffi` call (the v1 contract).
//! The re-entered fns marshal Int and String both ways AND
//! publish onto the bus, proving the re-entry composes with the
//! established runtime context rather than just returning
//! scalars.

use std::process::Command;

#[test]
fn c_calls_back_into_exported_hale_fns() {
    let root = std::env::temp_dir().join(format!(
        "hale_reentry_{}",
        std::process::id()
    ));
    let lib = root.join("vendor/cb");
    let app = root.join("app");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&lib).expect("mkdir lib");
    std::fs::create_dir_all(&app).expect("mkdir app");
    std::fs::write(root.join("hale.toml"), "").expect("marker");

    std::fs::write(
        lib.join("glue.c"),
        r#"
long long hale_add(long long a, long long b);
const char *hale_tag(const char *s);
long long hale_emit(long long n);

long long c_driver(long long x) {
  const char *t = hale_tag("from-c");
  if (!t || t[0] != 't') return -1;
  /* re-enter three times; each publishes on the bus */
  for (int i = 0; i < 3; i++) {
    if (hale_emit(i) != i) return -2;
  }
  return hale_add(x, 22);
}
"#,
    )
    .expect("write glue.c");
    std::fs::write(
        lib.join("hale.toml"),
        "[ffi]\ncsrc = [\"glue.c\"]\nlink = []\n",
    )
    .expect("write lib toml");
    std::fs::write(
        lib.join("cb.hl"),
        r#"@ffi("c") fn c_driver(x: Int) -> Int;

fn drive(x: Int) -> Int {
    return c_driver(x);
}
"#,
    )
    .expect("write cb.hl");

    std::fs::write(
        app.join("main.hl"),
        r#"import "vendor/cb" as cb;

type Ping { n: Int = 0; }

locus Sink {
    params { seen: Int = 0; sum: Int = 0; }
    bus { subscribe "reentry.ping" as on_p of type Ping; }
    fn on_p(p: Ping) {
        self.seen = self.seen + 1;
        self.sum = self.sum + p.n;
    }
    fn report() -> Int { return self.seen * 100 + self.sum; }
}

@export fn hale_add(a: Int, b: Int) -> Int {
    return a + b;
}

@export fn hale_tag(s: String) -> String {
    return "tagged:" + s;
}

locus Emitter {
    params { n: Int = 0; }
    bus { publish "reentry.ping" of type Ping; }
    birth() {
        "reentry.ping" <- Ping { n: self.n };
    }
}

@export fn hale_emit(n: Int) -> Int {
    // Locus instantiation from re-entered code — the eager child
    // births, publishes, and dissolves inside the callback.
    Emitter { n: n };
    return n;
}

main locus App {
    params { sink: Sink = Sink { }; }
    run() {
        let r = cb::drive(20);
        // main-thread publishes from re-entry enqueue to the coop
        // queue; a sliced sleep drains them before we assert.
        std::time::sleep(150ms);
        if r != 42 {
            println("BAD r=", r);
            std::process::exit(1);
        }
        // 3 pings, sum 0+1+2 = 3 -> 303
        if self.sink.report() != 303 {
            println("BAD sink=", self.sink.report());
            std::process::exit(1);
        }
        println("reentry ok");
    }
}

fn main() { App { }; }
"#,
    )
    .expect("write main.hl");

    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(&app)
        .output()
        .expect("hale build");
    assert!(
        out.status.success(),
        "build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let bin = app.join("app");
    let run = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_dir_all(&root);
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success() && stdout.contains("reentry ok"),
        "stdout: {:?}\nstderr: {:?}",
        stdout,
        String::from_utf8_lossy(&run.stderr)
    );
}
