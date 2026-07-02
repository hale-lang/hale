//! Aliasing stage 2 (2026-07-02): `noalias` on `self` for locus
//! methods where both reentrancy channels are provably closed —
//! elidable (non-allocating ⇒ no publish, no callee exit-drains)
//! AND all params by-value scalars. Modes participate via the
//! elidable fixpoint under their synthetic names.

use hale_codegen::build_executable;
use hale_syntax::parse_source;

fn ir_for(src: &str) -> String {
    let program = parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    static NEXT: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    bin.push(format!("hale_noalias_{}_{}", std::process::id(), n));
    std::env::set_var("LOTUS_DUMP_IR", "1");
    build_executable(&program, &bin).expect("build");
    std::env::remove_var("LOTUS_DUMP_IR");
    let ll = bin.with_extension("ll");
    let ir = std::fs::read_to_string(&ll).expect("IR dumped");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ll);
    ir
}

#[test]
fn scalar_elidable_methods_and_modes_get_noalias_self() {
    let ir = ir_for(
        r#"
        locus Counter {
            params { n: Int = 0; scale: Float = 1.0; }
            fn inc(by: Int) -> Int {
                self.n = self.n + by;
                return self.n;
            }
            mode bulk() -> Float { return self.scale; }
            mode harmonic() -> Float { return self.scale; }
            mode resolution() -> Float { return self.scale; }
        }
        fn main() {
            let c = Counter { };
            println(c.inc(1), " ", c.bulk());
        }
    "#,
    );
    for f in ["Counter.inc", "Counter.bulk", "Counter.harmonic"] {
        let line = ir
            .lines()
            .find(|l| l.contains(&format!("define")) && l.contains(f))
            .unwrap_or_else(|| panic!("{} defined", f));
        assert!(
            line.contains("ptr noalias %0"),
            "{} must carry noalias self: {}",
            f,
            line
        );
    }
}

#[test]
fn pointer_params_and_publishing_methods_stay_unmarked() {
    let ir = ir_for(
        r#"
        type Note { text: String; }
        locus Log {
            params { count: Int = 0; }
            bus { publish "note" of type Note; }
            // String param — channel 2 open — no noalias.
            fn label(s: String) -> Int {
                self.count = self.count + len(s);
                return self.count;
            }
            // Publishes (allocates payload) — channel 1 open.
            fn fire(v: Int) -> Int {
                "note" <- Note { text: "x" };
                return v;
            }
        }
        fn main() {
            let l = Log { };
            println(l.label("hi"), " ", l.fire(1));
        }
    "#,
    );
    for f in ["Log.label", "Log.fire"] {
        let line = ir
            .lines()
            .find(|l| l.contains("define") && l.contains(f))
            .unwrap_or_else(|| panic!("{} defined", f));
        assert!(
            !line.contains("noalias"),
            "{} must NOT carry noalias self: {}",
            f,
            line
        );
    }
}
