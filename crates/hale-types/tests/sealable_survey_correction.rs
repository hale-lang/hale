//! A classified correction, pinned (F.40 phase 4, W4).
//!
//! `hale check --sealable` used to find out what blocks sealing a locus
//! by cloning the program, sealing every locus, re-checking the clone
//! and reading the owner's name out of each sealed diagnostic's text.
//! The clone was checked as a bare bundle, without the seed's import
//! renames, so a receiver built from an imported seed's locus by its
//! qualified name (`lib::Counter { }`), or returned by an imported fn,
//! typed as `Unknown`: the accesses through it were invisible, and the
//! survey called the imported locus free to seal, or listed fewer of
//! its params than the program reaches. Sealing it would then fail the
//! check the survey said it would pass.
//!
//! The survey now reads the `param_accesses` rows of the check the
//! command ran, imports resolved, which are the rows the sealed rule
//! judges. Measured against the old survey over the corpus examples,
//! `tests/hale`, the DNA mains and the survey tests' program: identical
//! everywhere but 16 DNA seeds, each this shape and each a blocker the
//! old survey missed, none one it dropped. Pinned here on a two-seed
//! program, and on one of the DNA seeds as measured.

use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_types::sealability::{survey, Sealable};

/// The survey of the seed at `dir`, loaded as `hale check <dir>` loads
/// it, and the snapshot's check. On a thread of its own: a whole DNA
/// seed's walk is deep.
fn over_seed(dir: &Path) -> (Vec<Sealable>, Vec<String>) {
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn_scoped(s, move || {
                let Ok(snap) = Snapshot::load(dir, LoadMode::WholeSeed, &Disk, Config::check(true, false)) else {
                    panic!("{} does not load", dir.display());
                };
                let typed = snap.demand_typed_bodies().expect("the typed bodies");
                let bundle = snap.bundle();
                let programs: Vec<_> = bundle.programs.values().copied().collect();
                let rows = survey(&programs, typed);
                let errors = snap
                    .demand_check()
                    .expect("checked")
                    .diags
                    .iter()
                    .filter(|d| d.is_error())
                    .map(|d| d.message.clone())
                    .collect();
                (rows, errors)
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

/// The row of the locus whose declared name ends with `name` (an
/// imported seed's locus is declared under its merged name).
fn row<'a>(rows: &'a [Sealable], name: &str) -> &'a Sealable {
    let found: Vec<&Sealable> = rows.iter().filter(|r| r.locus.ends_with(name)).collect();
    assert_eq!(found.len(), 1, "one locus named `{name}`: {:?}", rows.iter().map(|r| &r.locus).collect::<Vec<_>>());
    found[0]
}

/// A scratch directory with `files` written into it.
fn seed(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hale-sealable-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, text) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

const LIB: &str = "locus Counter {\n    params { n: Int = 0; step: Int = 1; }\n    fn get() -> Int { return self.n; }\n}\n\
                   fn counter() -> Counter { return Counter { }; }\n";

const APP: &str = "import \"./lib\" as lib;\n\
                   fn peek() -> Int { let c = lib::Counter { }; return c.n; }\n\
                   fn bump() -> Int { let c = lib::counter(); return c.step + c.get(); }\n\
                   main locus App { }\n\
                   fn main() { println(peek() + bump()); }\n";

#[test]
fn an_imported_locus_reached_through_its_qualified_name_is_blocked() {
    let dir = seed("open", &[("main.hl", APP), ("lib/main.hl", LIB)]);
    let (rows, errors) = over_seed(&dir);
    assert!(errors.is_empty(), "the program checks: {errors:?}");
    let counter = row(&rows, "Counter");
    let params: Vec<&str> = counter.blockers.iter().map(|b| b.rsplit('.').next().unwrap()).collect();
    assert_eq!(
        params,
        ["n", "step"],
        "`c.n` through `lib::Counter {{ }}` and `c.step` through `lib::counter()` reach into the imported \
         locus from outside: {:?}",
        counter.blockers
    );
    assert!(row(&rows, "App").blockers.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sealing_what_the_survey_blocks_fails_the_check_at_those_accesses() {
    let dir = seed("sealed", &[("main.hl", APP), ("lib/main.hl", &LIB.replace("locus Counter", "@sealed locus Counter"))]);
    let (_, errors) = over_seed(&dir);
    let sealed: Vec<&String> = errors.iter().filter(|m| m.contains("is `@sealed`")).collect();
    assert_eq!(sealed.len(), 2, "the two accesses the survey names: {errors:?}");
    assert!(sealed[0].contains("Counter.n` reads one from outside"), "{sealed:?}");
    assert!(sealed[1].contains("Counter.step` reads one from outside"), "{sealed:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// One DNA seed as measured: `dna/face/service` builds its web assets
/// as `assets::WebAssets { }` and reads and writes their `error` and
/// `index` from `main`; the old survey called `WebAssets` free.
#[test]
fn the_dna_face_service_survey_names_the_web_assets_accesses() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dna/face/service").canonicalize().unwrap();
    let (rows, _) = over_seed(&dir);
    let web = row(&rows, "__WebAssets");
    let params: Vec<&str> = web.blockers.iter().map(|b| b.rsplit('.').next().unwrap()).collect();
    assert_eq!(params, ["error", "index"], "{:?}", web.blockers);
}
