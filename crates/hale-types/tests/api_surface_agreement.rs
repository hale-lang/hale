//! GH #1417 (R2a): the adapter a serve site builds reads a surface's rows
//! as `surfaces::surface_rows` does. The pass runs before the check and
//! cannot call it, so the member selection lives once
//! (`surfaces::select_members`, which both call: an `api` block's rows and
//! the `@rpc` rows of the seed's own loci are one surface when the names
//! coincide; an imported seed's loci feed its alias's surface; a block of
//! another name takes no local `@rpc` row). The selection agrees by
//! construction; this pins the digest, and the member list, on every
//! program the api fixtures hold and on the collision cases.

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

/// A program importing a library with one `@rpc` handler under `alias`.
fn with_lib(name: &str, src: &str, alias: &str) -> Snapshot {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("api_agree_{}_{name}", std::process::id()));
    std::fs::create_dir_all(dir.join("lib")).expect("dir");
    std::fs::write(dir.join("lib/main.hl"), "locus Echo { @rpc fn echo(x: Int) -> Int { return x + 1; } }\nfn main() { }\n")
        .expect("write lib");
    let path = dir.join(format!("{name}.hl"));
    std::fs::write(&path, format!("import \"./lib\" as {alias};\n{src}")).expect("write");
    let snap = load(&path);
    let _ = std::fs::remove_dir_all(&dir);
    snap
}

/// What `select_members` selects for each surface `surface_rows` lists, by
/// surface; asserted to be the rows' own list.
fn selected(snap: &Snapshot) -> Vec<(String, Vec<String>)> {
    let rows = snap.demand_surface_rows().expect("rows");
    let keys: std::collections::BTreeSet<String> = snap.programs().keys().map(|p| p.display().to_string()).collect();
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    let default = default_surface_name(&keys);
    let programs: Vec<&hale_syntax::ast::Program> = snap.programs().values().collect();
    let mut out = Vec::new();
    for s in &rows.surfaces {
        let mut got = hale_types::surfaces::select_members(&programs, Some(&s.name), snap.import_renames(), &default);
        got.sort_by(|a, b| (a.member.as_bytes(), a.written_at).cmp(&(b.member.as_bytes(), b.written_at)));
        let got: Vec<String> = got.into_iter().map(|m| m.member).collect();
        let want: Vec<String> = rows.rows_of(&s.name).map(|r| r.member.clone()).collect();
        assert_eq!(got, want, "the selection of `{}` is the rows' list", s.name);
        out.push((s.name.clone(), got));
    }
    let all = hale_types::surfaces::select_members(&programs, None, snap.import_renames(), &default);
    assert_eq!(all.len(), rows.rows.len(), "every row is a selected member");
    out
}

fn names(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

#[test]
fn select_members_is_the_rows_list_over_the_four_name_situations() {
    // plain default: the local rows are the default surface's
    let snap = with_lib("plainsel", LOCI, "toy");
    assert_eq!(
        selected(&snap),
        vec![("plainsel".to_string(), names(&["Local::secret"])), ("toy".to_string(), names(&["toy::Echo::echo"]))]
    );
    // a block named as the default surface keeps the local rows
    let snap = with_lib("blocksel", &format!("{LOCI}\napi blocksel {{ rpc Other::echo; }}\n"), "toy");
    assert_eq!(
        selected(&snap),
        vec![
            ("blocksel".to_string(), names(&["Local::secret", "Other::echo"])),
            ("toy".to_string(), names(&["toy::Echo::echo"])),
        ]
    );
    // an alias named as the default surface: one surface, both kept
    let snap = with_lib("aliassel", LOCI, "aliassel");
    assert_eq!(selected(&snap), vec![("aliassel".to_string(), names(&["Local::secret", "aliassel::Echo::echo"]))]);
    // a block and an alias both named as the default surface
    let snap = with_lib("bothsel", &format!("{LOCI}\napi bothsel {{ rpc Other::echo; }}\n"), "bothsel");
    assert_eq!(
        selected(&snap),
        vec![("bothsel".to_string(), names(&["Local::secret", "Other::echo", "bothsel::Echo::echo"]))]
    );
}

#[test]
fn the_adapter_agrees_when_an_alias_is_named_for_the_default_surface() {
    let snap = with_lib("aliasagree", LOCI, "aliasagree");
    assert_eq!(agree(&snap, "alias"), 1);
    let snap = with_lib("bothagree", &format!("{LOCI}\napi bothagree {{ rpc Other::echo; }}\n"), "bothagree");
    assert_eq!(agree(&snap, "both"), 1);
}
