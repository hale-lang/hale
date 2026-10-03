//! F.40 phase 3, E2 (a classified correction): whether a thread
//! crosses the bus boundary is read off the placement table and the
//! binding rows, so an imported library's `main locus`, which lowering
//! never deploys, no longer makes its importer off-thread.
//!
//! The predicate (`program_has_offthread`) drives two things that must
//! stay exact negations: the `lotus_bus_mark_pinned` call (the runtime's
//! locked enqueue and drain) and the `no_pinned` flag every static
//! dispatch carries (1 is the unlocked `lotus_bus_queue_enqueue_st`).
//! Before the table, the predicate walked every declaration's
//! `placement { }` block, an imported `main`'s included, so a program
//! importing a library whose own `main` pins a field took the lock for a
//! thread nothing starts. Real seeds through `hale build`, the IR dumped
//! beside the binary (`LOTUS_DUMP_IR`, before the optimization pipeline,
//! so the flag is a literal argument).

use std::path::{Path, PathBuf};
use std::process::Command;

/// A library whose `main locus` pins a field, with an eligible local
/// subject of its own, and a free fn for the importer to call.
const LIB: &str = "\
type Local { v: Int; }

fn greeting() -> String { return \"from the library\"; }

locus Worker { run() { } }

locus Sink {
    bus { subscribe \"lib.local\" as on_local of type Local; }
    fn on_local(l: Local) { println(\"local \", l.v); }
}

main locus LibMain {
    params { w: Worker = Worker { }; }
    placement { w: pinned; }
    bus { publish \"lib.local\" of type Local; }
    run() {
        Sink { };
        \"lib.local\" <- Local { v: 1 };
    }
}

fn main() { LibMain { }; }
";

/// The importer: its own `main locus` places nothing and binds nothing,
/// and publishes an eligible local subject.
const APP: &str = "\
import \"lib\" as lib;

type Note { v: Int; }

locus Reader {
    bus { subscribe \"app.note\" as on_note of type Note; }
    fn on_note(n: Note) { println(\"note \", n.v); }
}

main locus App {
    bus { publish \"app.note\" of type Note; }
    run() {
        Reader { };
        println(lib::greeting());
        \"app.note\" <- Note { v: 1 };
    }
}

fn main() { App { }; }
";

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_offthread_import_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("lib")).unwrap();
    std::fs::write(d.join("hale.toml"), "[deps]\n").unwrap();
    std::fs::write(d.join("lib/hale.toml"), "[deps]\n").unwrap();
    std::fs::write(d.join("lib/main.hl"), LIB).unwrap();
    std::fs::write(d.join("main.hl"), APP).unwrap();
    d
}

/// `hale build <seed>` with the IR dumped beside the binary; the IR.
fn build_ir(seed: &Path, out: &Path) -> String {
    let built = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(seed)
        .arg("-o")
        .arg(out)
        .env("LOTUS_DUMP_IR", "1")
        .output()
        .expect("hale build");
    assert!(
        built.status.success(),
        "hale build {}: {}{}",
        seed.display(),
        String::from_utf8_lossy(&built.stdout),
        String::from_utf8_lossy(&built.stderr)
    );
    std::fs::read_to_string(out.with_extension("ll")).expect("the IR beside the binary")
}

/// The `no_pinned` argument (the call's last) of every static dispatch;
/// a build with debug info follows the call with `, !dbg !N`.
fn no_pinned_args(ir: &str) -> Vec<String> {
    ir.lines()
        .filter(|l| l.contains("call void @lotus_bus_dispatch_static("))
        .map(|l| {
            let args = l.rsplit_once("i32 ").expect("final i32 arg").1;
            args.split(')').next().unwrap_or("").trim().to_string()
        })
        .collect()
}

fn marks_pinned(ir: &str) -> bool {
    ir.lines().any(|l| l.contains("call void @lotus_bus_mark_pinned("))
}

#[test]
fn an_imported_undeployed_main_does_not_make_its_importer_off_thread() {
    let d = scratch();
    // The control: the library built as itself deploys its `main`, whose
    // pinned field is a thread beside the bus, so the bus takes the lock.
    let lib_ir = build_ir(&d.join("lib"), &d.join("lib.bin"));
    let lib_flags = no_pinned_args(&lib_ir);
    assert!(!lib_flags.is_empty(), "the library has an eligible static dispatch");
    assert!(marks_pinned(&lib_ir), "a deployed pinned placement marks the bus pinned");
    assert!(lib_flags.iter().all(|f| f == "0"), "deployed off-thread: no_pinned=0 everywhere: {lib_flags:?}");
    // The importer deploys its own `main`; the library's is inert here, so
    // nothing runs off the main thread.
    let app_ir = build_ir(&d, &d.join("app.bin"));
    let app_flags = no_pinned_args(&app_ir);
    assert!(!app_flags.is_empty(), "the importer has an eligible static dispatch");
    assert!(
        !marks_pinned(&app_ir),
        "the imported main's placement block is never deployed, so the bus is not marked pinned"
    );
    assert!(app_flags.iter().all(|f| f == "1"), "single-threaded: no_pinned=1 everywhere: {app_flags:?}");
    let _ = std::fs::remove_dir_all(&d);
}

/// The binding term reads the binding rows, which hold a module-nested
/// `main`'s entries as lowering's prelude lowers them: `fn main` deploys
/// the module's `main`, its unix listen entry starts a reader thread,
/// and the bus takes the lock. The old term scanned top-level items
/// only, so this program got `no_pinned=1` beside a live reader (the
/// GH #468 shape, one brace deeper).
const NESTED_BOUND_MAIN: &str = "\
module wired {
    type Tick { n: Int = 0; }
    type Local { v: Int; }
    topic Wire { payload: Tick; subject: \"m.wire\"; }
    locus Sink {
        bus { subscribe \"m.local\" as on_local of type Local; }
        fn on_local(l: Local) { println(\"local \", l.v); }
    }
    main locus App {
        bus { publish Wire; publish \"m.local\" of type Local; }
        bindings { Wire: unix(\"/tmp/hale_offthread_nested.sock\", role: listen); }
        run() {
            Sink { };
            \"m.local\" <- Local { v: 1 };
            Wire <- Tick { n: 1 };
        }
    }
}

fn main() { App { }; }
";

#[test]
fn a_module_nested_mains_binding_makes_the_program_off_thread() {
    let d = std::env::temp_dir().join(format!("hale_offthread_nested_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("hale.toml"), "[deps]\n").unwrap();
    std::fs::write(d.join("main.hl"), NESTED_BOUND_MAIN).unwrap();
    let ir = build_ir(&d, &d.join("app.bin"));
    let flags = no_pinned_args(&ir);
    assert!(!flags.is_empty(), "the program has an eligible static dispatch");
    assert!(marks_pinned(&ir), "the deployed main's listen binding is a thread beside the bus");
    assert!(flags.iter().all(|f| f == "0"), "off-thread: no_pinned=0 everywhere: {flags:?}");
    let _ = std::fs::remove_dir_all(&d);
}
