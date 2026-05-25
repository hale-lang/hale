//! F.32-3 (2026-05-25) — codegen-aware per-pool chunk-size
//! hint. The Fresh-strategy locus instantiation site switches
//! from `lotus_arena_create_labeled` to `_labeled_sized` when
//! the locus is being instantiated on a non-`main`
//! cooperative pool. The hint is computed at compile time
//! from the count of loci placed on the same pool.
//!
//! This test pins down that the multi-locus-per-pool codegen
//! path works end-to-end: three different locus types placed
//! on the same `io` cooperative pool all receive a published
//! tick, the binary exits cleanly, and the sized arena calls
//! don't trip an assert or crash the residency dump.
//!
//! The hint *value* for N=3 still clamps to the 64K default
//! (the formula `524288 / N / 2 → 87381`, rounded down to
//! power of 2 = 65536, clamped to [4096, 65536] = 65536), so
//! observable behavior is identical to a 1-locus pool — that's
//! the whole point of the runtime clamp. The win materializes
//! at N >= 16. Asserting the actual hint value requires LLVM
//! IR dump parsing; out of scope here. What's covered: the
//! code path is exercised and the program doesn't break.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lt-coop-pool-multi-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

#[test]
fn multi_locus_per_coop_pool_codegen_path() {
    let src = r#"
        type Tick { n: Int; }

        locus SubA {
            bus { subscribe "tick" as on_tick of type Tick; }
            fn on_tick(t: Tick) { println("a ", t.n); }
        }
        locus SubB {
            bus { subscribe "tick" as on_tick of type Tick; }
            fn on_tick(t: Tick) { println("b ", t.n); }
        }
        locus SubC {
            bus { subscribe "tick" as on_tick of type Tick; }
            fn on_tick(t: Tick) { println("c ", t.n); }
        }

        main locus App {
            params {
                a: SubA = SubA { };
                b: SubB = SubB { };
                c: SubC = SubC { };
            }
            placement {
                a: cooperative(pool = io);
                b: cooperative(pool = io);
                c: cooperative(pool = io);
            }
            bus { publish "tick" of type Tick; }
            run() {
                "tick" <- Tick { n: 1 };
                std::time::sleep(100ms);
                println("main done");
            }
        }

        fn main() { App { }; }
    "#;

    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path("3on1");
    build_executable(&program, &bin).expect("build");

    let out = Command::new(&bin).output().expect("run binary");
    let _ = std::fs::remove_file(&bin);

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(
        out.status.success(),
        "binary exited non-zero: {:?}\nstdout: {}\nstderr: {}",
        out.status,
        stdout,
        stderr,
    );
    for tag in ["a 1", "b 1", "c 1", "main done"] {
        assert!(
            stdout.contains(tag),
            "expected `{}` in stdout; full output:\n{}",
            tag,
            stdout
        );
    }
}
