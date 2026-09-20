//! GH #762 — a qualified path must use an alias the seed it is
//! written in declares.
//!
//! An import alias binds in its declaring seed only (spec
//! `projects.md`, "Scoped imports (A4)": there are no re-exports).
//! The per-build path-rename table is ONE table keyed by the alias as
//! written, so a seed that never imported anything could still write
//! `u::f()` and have it answered out of another seed's import row:
//!
//! - `lib` declares `fn f()`,
//! - `a` imports nothing and calls `u::f()`,
//! - `top` imports `../lib` as `u` and `../a` as `a`.
//!
//! `hale check` answered `ok` and `a`'s call ran `top`'s `u`. The
//! language says `a` has no `u` at all. GH #746 made the CONTESTED
//! case loud (two seeds binding one alias to different libraries);
//! uncontested, this stayed silent — a library quietly bound to
//! whatever its consumer happened to spell `u`, and the same library
//! compiled from a different app would call something else or not
//! compile at all.
//!
//! The rule is now a located check error at the path, naming the seed
//! that does declare the alias. The controls below are the other
//! half: a seed's OWN alias must keep working in every position a
//! qualified path can stand in, and the tree's own multi-seed
//! programs must not trip it.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Write a throwaway multi-seed tree under a pid-unique temp dir.
fn seed(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d: PathBuf = std::env::temp_dir().join(format!(
        "hale_alias_declared_{}_{}",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_dir_all(&d);
    for (name, src) in files {
        let p = d.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).expect("mkdir");
        std::fs::write(&p, src).expect("write");
    }
    d
}

fn hale(cwd: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("hale");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/
    p.pop(); // repo root
    p
}

/// The library seed the app reaches, in both directions.
const LIB: &str = "fn f() -> String { return \"from-lib\"; }\n";

/// The issue's shape: `a` declares no imports at all and writes
/// `u::f()`; `top` is the seed that declares `u`.
#[test]
fn a_seed_cannot_reach_an_alias_only_another_seed_declares() {
    let d = seed(
        "borrowed",
        &[
            ("lib/main.hl", LIB),
            ("a/main.hl", "fn via_a() -> String { return u::f(); }\n"),
            (
                "top/main.hl",
                "import \"../lib\" as u;\n\
                 import \"../a\" as a;\n\
                 fn main() {\n\
                 \x20   println(a::via_a());\n\
                 \x20   println(u::f());\n\
                 }\n",
            ),
        ],
    );
    let top = d.join("top");

    let (ok, out) = hale(&top, &["check", "."]);
    assert!(!ok, "check must refuse the borrowed alias:\n{out}");
    assert!(
        out.contains("`u` is not an import of this seed"),
        "the message must say whose alias it is not:\n{out}"
    );
    assert!(
        out.contains("declares it"),
        "the message must name the seed that does declare it:\n{out}"
    );
    assert!(
        out.contains("top"),
        "the declaring seed is `top`, and the message must say so:\n{out}"
    );
    assert!(
        out.contains("a/main.hl:1:"),
        "the error is located at the path in `a`, not in `top`:\n{out}"
    );
    assert_eq!(
        out.matches("is not an import of this seed").count(),
        1,
        "only the borrowing reference is refused — `top`'s own \
         `u::f()` is `top`'s to write:\n{out}"
    );

    // The single-file entry path resolves imports through its own
    // resolver; it must refuse the same program.
    let (ok, out) = hale(&top, &["run", "main.hl"]);
    assert!(!ok, "run must refuse it too:\n{out}");
    assert!(
        out.contains("`u` is not an import of this seed"),
        "run reports the same finding:\n{out}"
    );

    // ... and so must the directory `build` path.
    let (ok, out) = hale(&top, &["build", "."]);
    assert!(!ok, "build must refuse it too:\n{out}");
    assert!(
        out.contains("`u` is not an import of this seed"),
        "build reports the same finding:\n{out}"
    );

    let _ = std::fs::remove_dir_all(&d);
}

/// The diamond control: two seeds reach ONE library, each through an
/// alias it declares itself. Nothing is borrowed, so nothing is
/// refused — and the answers still come from the library.
#[test]
fn each_seed_declaring_its_own_alias_stays_green() {
    let d = seed(
        "diamond",
        &[
            ("lib/main.hl", LIB),
            (
                "a/main.hl",
                "import \"../lib\" as l;\n\
                 fn via_a() -> String { return l::f(); }\n",
            ),
            (
                "b/main.hl",
                "import \"../lib\" as l;\n\
                 fn via_b() -> String { return l::f(); }\n",
            ),
            (
                "top/main.hl",
                "import \"../a\" as a;\n\
                 import \"../b\" as b;\n\
                 import \"../lib\" as l;\n\
                 fn main() {\n\
                 \x20   println(a::via_a(), \" \", b::via_b(), \" \", l::f());\n\
                 }\n",
            ),
        ],
    );
    let top = d.join("top");
    let (ok, out) = hale(&top, &["check", "."]);
    assert!(ok, "every alias is declared where it is used:\n{out}");
    let (ok, out) = hale(&top, &["run", "main.hl"]);
    assert!(ok, "run: {out}");
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        vec!["from-lib from-lib from-lib"],
        "{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// A seed's FILES share one alias namespace, exactly as they share
/// one declaration namespace: `mid/b.hl` writes `u::f()` for the
/// import `mid/a.hl` declares, and that is its own import, not a
/// borrowed one.
#[test]
fn one_seeds_files_share_its_alias_namespace() {
    let d = seed(
        "multifile",
        &[
            ("lib/main.hl", LIB),
            (
                "mid/a.hl",
                "import \"../lib\" as u;\n\
                 fn mid_a() -> String { return u::f(); }\n",
            ),
            (
                "mid/b.hl",
                "fn mid_b() -> String { return u::f() + \"/b\"; }\n",
            ),
            (
                "top/main.hl",
                "import \"../mid\" as m;\n\
                 fn main() {\n\
                 \x20   println(m::mid_a(), \" \", m::mid_b());\n\
                 }\n",
            ),
        ],
    );
    let top = d.join("top");
    let (ok, out) = hale(&top, &["check", "."]);
    assert!(ok, "one seed, one alias namespace:\n{out}");
    let (ok, out) = hale(&top, &["run", "main.hl"]);
    assert!(ok, "run: {out}");
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        vec!["from-lib from-lib/b"],
        "{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// The other half of the rule: a seed's OWN alias in every position a
/// qualified path can stand in — a signature type, a call, a const, a
/// struct literal, an enum variant in expression AND pattern
/// position, a locus instantiation, a `bindings { }` topic and a
/// claim group member. `mid` is a LIBRARY seed (the side of the
/// import the rule is about), and `top` declares an alias of its own
/// for the same library so the table holds both.
#[test]
fn a_seeds_own_alias_in_every_path_position_stays_green() {
    let d = seed(
        "positions",
        &[
            (
                "lib/main.hl",
                "type Color = enum { Red, Green, Blue };\n\
                 \n\
                 type Box { v: Int; }\n\
                 \n\
                 type Tick { n: Int; }\n\
                 \n\
                 const TAG: String = \"lib-tag\";\n\
                 \n\
                 topic Beat {\n\
                 \x20   payload: Tick;\n\
                 \x20   subject: \"hale762.beat\";\n\
                 }\n\
                 \n\
                 fn f() -> String { return \"from-lib\"; }\n\
                 \n\
                 fn unbox(b: Box) -> Int { return b.v; }\n\
                 \n\
                 locus Worker {\n\
                 \x20   params {\n\
                 \x20       n: Int = 0;\n\
                 \x20   }\n\
                 \x20   fn bump() {\n\
                 \x20       self.n = self.n + 1;\n\
                 \x20   }\n\
                 }\n",
            ),
            (
                "mid/main.hl",
                "import \"../lib\" as u;\n\
                 \n\
                 group workers = { u::Worker };\n\
                 \n\
                 claims {\n\
                 \x20   workers_keep_no_secrets:\n\
                 \x20       forbid reaches(workers, effects(secret_use));\n\
                 }\n\
                 \n\
                 fn mid_call() -> String { return u::f(); }\n\
                 \n\
                 fn mid_const() -> String { return u::TAG; }\n\
                 \n\
                 fn mid_lit() -> Int { return u::unbox(u::Box { v: 11 }); }\n\
                 \n\
                 fn mid_type(b: u::Box) -> Int { return b.v; }\n\
                 \n\
                 fn mid_variant() -> String {\n\
                 \x20   return match u::Color::Green {\n\
                 \x20       u::Color::Red -> \"r\",\n\
                 \x20       u::Color::Green -> \"g\",\n\
                 \x20       u::Color::Blue -> \"b\",\n\
                 \x20   };\n\
                 }\n\
                 \n\
                 fn mid_worker() -> Int {\n\
                 \x20   let w = u::Worker { };\n\
                 \x20   w.bump();\n\
                 \x20   return w.n;\n\
                 }\n",
            ),
            (
                "top/main.hl",
                "import \"../mid\" as m;\n\
                 import \"../lib\" as own;\n\
                 \n\
                 main locus App {\n\
                 \x20   params {\n\
                 \x20       b: own::Box = own::Box { v: 5 };\n\
                 \x20   }\n\
                 \x20   bindings {\n\
                 \x20       own::Beat: unix(\"/tmp/hale-762-green.sock\", \
                 role: listen);\n\
                 \x20   }\n\
                 \x20   run() {\n\
                 \x20       println(m::mid_call(), \" \", m::mid_const());\n\
                 \x20       println(m::mid_lit(), \" \", m::mid_type(self.b), \
                 \" \", m::mid_variant());\n\
                 \x20       println(m::mid_worker(), \" \", own::TAG);\n\
                 \x20   }\n\
                 }\n\
                 \n\
                 fn main() { App { }; }\n",
            ),
        ],
    );
    let top = d.join("top");
    let (ok, out) = hale(&top, &["check", "."]);
    assert!(
        ok,
        "a seed's own alias is reachable in every path position:\n{out}"
    );
    let (ok, out) = hale(&top, &["run", "main.hl"]);
    assert!(ok, "run: {out}");
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        vec!["from-lib lib-tag", "11 5 g", "1 lib-tag"],
        "and every one of them still resolves to the library:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// The false-positive oracle: every multi-seed program the repo
/// itself carries. A rule that refuses a reference the language
/// allows would show up here first.
///
/// The assertion is the ABSENCE of this rule's finding rather than a
/// clean exit: a handful of fixture directories are known not to
/// check as a bundle (a directory of independent single-file
/// programs, a workspace-root-relative import that only resolves from
/// the repo root), and that is not this rule's business.
#[test]
fn the_repos_own_seeds_never_trip_the_rule() {
    let root = repo_root();
    let mut targets: Vec<PathBuf> = vec![
        root.join("dna/core"),
        root.join("dna/host"),
        root.join("iris/consumer/fuse-hl"),
    ];
    let examples = root.join("crates/hale-codegen/tests/fixtures/examples");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&examples)
        .expect("examples dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    assert!(dirs.len() > 50, "the example corpus went missing");
    targets.append(&mut dirs);

    for t in &targets {
        if !t.is_dir() {
            continue;
        }
        let (_, out) = hale(&root, &["check", &t.display().to_string()]);
        assert!(
            !out.contains("is not an import of this seed"),
            "GH #762's rule tripped on {}:\n{out}",
            t.display()
        );
    }
}
