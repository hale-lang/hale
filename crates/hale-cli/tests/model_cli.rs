//! GH #476 Change 2 — the demand contract, proven cross-process.
//!
//! The epic's LSP/performance rule: the model is lazy and demanded
//! by consumers; a diagnostics-only check must PROVABLY not
//! construct it ("cached" must not become "always built"). The
//! builder prints one stderr line under HALE_MODEL_TRACE=1 — these
//! tests run the real binary and read that channel.

use std::path::PathBuf;
use std::process::Command;

fn hale() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hale"))
}

fn workdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("hale_modelcli_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

const APP: &str = r#"
type T { n: Int = 0; }
topic Evt { payload: T; subject: "evt"; }
locus Sub {
    params { seen: Int = 0; }
    bus { subscribe Evt as on_e; }
    fn on_e(t: T) { self.seen = self.seen + 1; }
}
group subs = { Sub };
main locus App {
    params { s: Sub = Sub { }; }
    bus { publish Evt; }
    claims {
        wired: require subscribes(some subs, topic Evt);
    }
    run() { Evt <- T { n: 1 }; }
}
fn main() { App { }; }
"#;

#[test]
fn plain_check_never_builds_the_model() {
    let dir = workdir("nodemand");
    let src = dir.join("app.hl");
    std::fs::write(&src, APP).unwrap();

    // Diagnostics-only check — with CLAIMS present, even: nothing
    // demands the model until the Change-5 evaluator migration, so
    // the builder must not run.
    let out = hale()
        .arg("check")
        .arg(&src)
        .env("HALE_MODEL_TRACE", "1")
        .output()
        .expect("hale check");
    assert!(out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !err.contains("[hale-model]"),
        "a no-demand check must not derive the model:\n{}",
        err
    );

    // The artifact dump path DOES demand it (GH #476 Change 6:
    // the artifact's law rows are projected from the model). The
    // demand boundary this canary guards is the diagnostics-only
    // path above — artifact emission is a legitimate consumer.
    let out = hale()
        .arg("check")
        .arg(&src)
        .arg("--dump-topology")
        .env("HALE_MODEL_TRACE", "1")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("[hale-model]"),
        "artifact emission projects the model (Change 6)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn model_dump_demands_derives_and_is_deterministic() {
    let dir = workdir("demand");
    let src = dir.join("app.hl");
    std::fs::write(&src, APP).unwrap();

    let out = hale()
        .arg("model")
        .arg("dump")
        .arg(&src)
        .env("HALE_MODEL_TRACE", "1")
        .output()
        .expect("hale model dump");
    assert!(
        out.status.success(),
        "dump failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("[hale-model]"),
        "the dump IS the demand — the trace line must appear"
    );
    let first = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(first.contains("# hale ApplicationModel"));
    assert!(first.contains("entrypoint App"));
    assert!(first.contains("subscribes (1):"));

    let again = hale().arg("model").arg("dump").arg(&src).output().unwrap();
    assert_eq!(
        first,
        String::from_utf8_lossy(&again.stdout),
        "two dumps are byte-identical"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cross_seed_models_carry_the_imported_universe() {
    let dir = workdir("xseed");
    let app = dir.join("app");
    let lib = dir.join("lib/kv");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(dir.join("hale.toml"), "name = \"xseed-model\"\n")
        .unwrap();
    std::fs::write(
        lib.join("kv.hl"),
        r#"
type Pair { k: Int = 0; v: Int = 0; }
locus Store {
    params { total: Int = 0; }
    fn get(k: Int) -> Int { return self.total + k; }
}
fn helper(v: Int) -> Int { return v + 1; }
"#,
    )
    .unwrap();
    std::fs::write(
        app.join("main.hl"),
        r#"
import "lib/kv" as p;
group kvs = { p::* };
main locus App {
    params { s: p::Store = p::Store { }; }
    run() {
        let a = self.s.get(1);
        let b = p::helper(a);
        println(b);
    }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let out = hale().arg("model").arg("dump").arg(&app).output().unwrap();
    assert!(
        out.status.success(),
        "cross-seed dump failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let dump = String::from_utf8_lossy(&out.stdout);
    // Imported decls appear author-spelled; the glob selector stays
    // AUTHORED (unexpanded) while membership resolves.
    assert!(dump.contains("p::Store"), "imported locus:\n{}", dump);
    assert!(dump.contains("p::helper"), "imported free fn:\n{}", dump);
    assert!(
        dump.contains("kvs[0] = p::* (glob)"),
        "authored glob selector survives:\n{}",
        dump
    );
    assert!(dump.contains("p::Pair"), "imported type in universe");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ill_typed_programs_refuse_a_model() {
    let dir = workdir("refuse");
    let src = dir.join("bad.hl");
    std::fs::write(
        &src,
        "fn main() { let x: Int = \"not an int\"; println(x); }\n",
    )
    .unwrap();
    let out = hale().arg("model").arg("dump").arg(&src).output().unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("refusing to derive a model"),
        "same refusal rule as the artifact"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
