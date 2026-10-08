//! GH #1417 (R2a): the adapter a serve site builds reads a surface's rows
//! as `surfaces::surface_rows` does. The pass runs before the check and
//! cannot call it, so it mirrors the combination rule (an `api` block's
//! rows and the `@rpc` rows of the seed's own loci are one surface when
//! the names coincide; an imported seed's loci feed its alias's surface;
//! a block of another name takes no local `@rpc` row) and this pins that
//! the two agree, member list and digest, on every program the api
//! fixtures hold and on the collision cases.

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_types::surfaces::default_surface_name;

fn load(path: &std::path::Path) -> Snapshot {
    Snapshot::load(path, LoadMode::WholeSeed, &Disk, Config::check(false, false))
        .ok()
        .unwrap_or_else(|| panic!("{} loads", path.display()))
}

/// Every surface of `snap`: the adapter's selection against `surface_rows`'.
/// The count of surfaces compared.
fn agree(snap: &Snapshot, what: &str) -> usize {
    let rows = snap.demand_surface_rows().expect("rows");
    let keys: std::collections::BTreeSet<String> = snap.programs().keys().map(|p| p.display().to_string()).collect();
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    let default = default_surface_name(&keys);
    let programs: Vec<&hale_syntax::ast::Program> = snap.programs().values().collect();
    for s in &rows.surfaces {
        let want: Vec<String> = rows.rows_of(&s.name).map(|r| r.member.clone()).collect();
        let (members, digest) = hale_types::rpc_expand::selection(&programs, &s.name, snap.import_renames(), &default);
        assert_eq!(members, want, "{what}: the members of `{}`", s.name);
        assert_eq!(digest, s.digest, "{what}: the digest of `{}`", s.name);
    }
    rows.surfaces.len()
}

#[test]
fn the_adapter_reads_the_rows_surface_rows_does_over_the_api_programs() {
    let root = hale_corpus::repo_root();
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("tests/hale/api"))
        .expect("tests/hale/api")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "hl"))
        .collect();
    files.push(root.join("tests/api-contract/program.hl"));
    files.sort();
    let mut compared = 0;
    for f in &files {
        compared += agree(&load(f), &f.display().to_string());
    }
    assert!(files.len() > 10 && compared > 10, "the fixtures were compared: {} files, {compared} surfaces", files.len());
}

fn in_dir(name: &str, src: &str) -> Snapshot {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("api_agree_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join(format!("{name}.hl"));
    std::fs::write(&path, src).expect("write");
    let snap = load(&path);
    let _ = std::fs::remove_dir_all(&dir);
    snap
}

const LOCI: &str = "
locus Local { @rpc fn secret(x: Int) -> Int { return x + 100; } }
locus Other { fn echo(x: Int) -> Int { return x + 1; } }
";

#[test]
fn a_block_named_for_the_default_surface_keeps_the_local_rpc_rows() {
    let snap = in_dir("combo", &format!("{LOCI}\napi combo {{ rpc Other::echo; }}\n"));
    assert_eq!(agree(&snap, "combo"), 1);
    let rows = snap.demand_surface_rows().expect("rows");
    let members: Vec<&str> = rows.rows_of("combo").map(|r| r.member.as_str()).collect();
    assert_eq!(members, vec!["Local::secret", "Other::echo"]);
}

#[test]
fn a_block_named_otherwise_excludes_the_local_rpc_rows_and_the_default_surface_keeps_them() {
    let snap = in_dir("seedname", &format!("{LOCI}\napi elsewhere {{ rpc Other::echo; }}\n"));
    assert_eq!(agree(&snap, "seedname"), 2);
    let rows = snap.demand_surface_rows().expect("rows");
    let of = |s: &str| rows.rows_of(s).map(|r| r.member.clone()).collect::<Vec<_>>();
    assert_eq!(of("elsewhere"), vec!["Other::echo"]);
    assert_eq!(of("seedname"), vec!["Local::secret"]);
}
