//! A declaration's dependents (F.40 phase 3, X2 1 of 3), checked by
//! mutation over the example corpus: the relation
//! (`Snapshot::declaration_dependents`, `hale_frontend::dependents`) is a
//! superset of what a fresh check re-derives.
//!
//! For every seed of the corpus and every fn and locus its own files
//! declare, the body of the declaration (a locus's first member with a
//! body) is edited three ways — emptied, given an ill-typed `let`, and
//! given a call to another fn of the seed — and the edited seed is
//! checked fresh. Each typing-stage diagnostic is attributed to the
//! declaration its span falls in, as an offset from the declaration's
//! start; a declaration whose list changed is one the check re-derived.
//! The relation's answer is its answer in both snapshots, before the
//! edit and after it (an edge the edit removes is the old snapshot's, one
//! it adds the new one's), joined by the declarations' correspondence
//! (program, kind, name, ordinal), which is the reuse rule's. Every
//! changed declaration must be in the answer, and no diagnostic outside
//! every declaration may change. An answer of `Whole` (the seed checked
//! whole) is never an edit of a fn or locus body, and is counted.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hale_frontend::dependents::{Declaration, Dependents};
use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Overlay;
use hale_syntax::ast::{LocusMember, TopDecl};
use hale_syntax::{Diag, Span, SpanOrigin};

/// A declaration across two snapshots: its program, kind, name, and its
/// ordinal among the declarations sharing those.
type DeclKey = (PathBuf, &'static str, String, usize);

/// A diagnostic as it sits in its declaration: offset from the
/// declaration's start, length, kind, message.
type Placed = (u32, u32, &'static str, String);

fn keys(decls: &[Declaration]) -> Vec<DeclKey> {
    let mut seen: BTreeMap<(PathBuf, &'static str, String), usize> = BTreeMap::new();
    decls
        .iter()
        .map(|d| {
            let n = seen.entry((d.program.clone(), d.kind, d.name.clone())).or_default();
            *n += 1;
            (d.program.clone(), d.kind, d.name.clone(), *n - 1)
        })
        .collect()
}

/// The typing stage's diagnostics, per declaration (by key), and the
/// ones no declaration holds.
fn attributed(snap: &Snapshot, diags: &[Diag]) -> (BTreeMap<DeclKey, Vec<Placed>>, Vec<(String, String)>) {
    let decls = snap.declarations();
    let keys = keys(decls);
    let mut per: BTreeMap<DeclKey, Vec<Placed>> = keys.iter().map(|k| (k.clone(), Vec::new())).collect();
    let mut outside = Vec::new();
    for d in diags {
        let at = d.span.start.0;
        let holder = (d.origin == SpanOrigin::Seed)
            .then(|| decls.iter().position(|x| x.span.start.0 <= at && at < x.span.end.0.max(x.span.start.0 + 1)))
            .flatten();
        match holder {
            Some(i) => per.get_mut(&keys[i]).expect("a key per declaration").push((
                at - decls[i].span.start.0,
                d.span.end.0.wrapping_sub(d.span.start.0),
                d.kind_str(),
                d.message.clone(),
            )),
            None => outside.push((d.kind_str().to_string(), d.message.clone())),
        }
    }
    outside.sort();
    (per, outside)
}

/// The answer for the declaration keyed `key` in `snap`, as keys; `None`
/// for `Whole`.
fn answer(snap: &Snapshot, key: &DeclKey) -> Option<BTreeSet<DeclKey>> {
    let decls = snap.declarations();
    let keys = keys(decls);
    let i = keys.iter().position(|k| k == key)?;
    let site = decls[i].site?;
    match snap.declaration_dependents(site).ok()? {
        Dependents::Whole(_) => None,
        Dependents::Decls(sites) => Some(
            decls
                .iter()
                .zip(&keys)
                .filter(|(d, _)| d.site.is_some_and(|s| sites.contains(&s)))
                .map(|(_, k)| k.clone())
                .collect(),
        ),
    }
}

/// The body a mutation edits: a fn's, or a locus's first member with one.
fn body_span(item: &TopDecl) -> Option<Span> {
    match item {
        TopDecl::Fn(f) => Some(f.body.span),
        TopDecl::Locus(l) => l.members.iter().find_map(|m| match m {
            LocusMember::Fn(f) => Some(f.body.span),
            LocusMember::Lifecycle(lc) => Some(lc.body.span),
            _ => None,
        }),
        _ => None,
    }
}

fn load(dir: &Path, overlays: &BTreeMap<PathBuf, String>) -> Option<Snapshot> {
    let snap = Snapshot::load(dir, LoadMode::Editor, &Overlay::new(overlays), Config::editor()).ok()?.linked().ok()?;
    snap.demand_typing().ok()?;
    Some(snap)
}

#[test]
fn a_declarations_dependents_cover_what_a_fresh_check_rederives() {
    let examples = hale_corpus::repo_root().join("crates/hale-codegen/tests/fixtures/examples");
    let mut seeds: Vec<PathBuf> =
        std::fs::read_dir(&examples).expect("the corpus").flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    seeds.sort();
    let chain = scratch_seed("reveal-chain", REVEAL_CHAIN);
    seeds.push(chain.clone());
    let (mut edits, mut whole, mut beyond, mut answered) = (0usize, 0usize, 0usize, 0usize);
    // the negative control: what an answer of the edited declaration
    // alone would miss, and where
    let mut alone_misses: Vec<String> = Vec::new();
    let mut misses: Vec<String> = Vec::new();
    for seed in &seeds {
        let none = BTreeMap::new();
        let Some(before) = load(seed, &none) else { continue };
        let before_diags = before.demand_typing().expect("typed").diags.clone();
        let (per0, out0) = attributed(&before, &before_diags);
        let decls = before.declarations().to_vec();
        let keys0 = keys(&decls);
        // a fn of the seed's own files a mutation can call: no params
        let callee = decls.iter().zip(&keys0).find_map(|(d, _)| {
            let TopDecl::Fn(f) = &before.programs()[&d.program].items[d.index] else { return None };
            let own = before.file_bases().iter().any(|(b, p, l)| {
                *b <= d.span.start.0 && d.span.start.0 < b + l && before.own_files().iter().any(|o| o.file_name() == p.file_name())
            });
            (own && f.params.is_empty() && f.generics.is_empty() && f.name.name != "main").then(|| f.name.name.clone())
        });
        for (d, key) in decls.iter().zip(&keys0) {
            if !matches!(d.kind, "fn" | "locus") {
                continue;
            }
            let item = &before.programs()[&d.program].items[d.index];
            let Some(body) = body_span(item) else { continue };
            // the file the body sits in, one of the seed's own
            let Some((base, path, _)) = before
                .file_bases()
                .iter()
                .find(|(b, _, l)| *b <= body.start.0 && body.end.0 <= b + l)
                .cloned()
            else {
                continue;
            };
            if !before.own_files().iter().any(|o| o.file_name() == path.file_name()) {
                continue;
            }
            let text = before.sources()[&path].clone();
            let (s, e) = ((body.start.0 - base) as usize, (body.end.0 - base) as usize);
            if !text[s..e].starts_with('{') || !text[s..e].ends_with('}') {
                continue;
            }
            let mut mutations = vec![
                ("emptied", format!("{}{{ }}{}", &text[..s], &text[e..])),
                ("ill-typed let", format!("{}{{ let x2_probe: Int = \"probe\"; {}", &text[..s], &text[s + 1..])),
                ("a print added", format!("{}{{ println(\"x2\"); {}", &text[..s], &text[s + 1..])),
            ];
            if let Some(c) = callee.as_ref().filter(|c| **c != d.name) {
                mutations.push(("a call added", format!("{}{{ {c}(); {}", &text[..s], &text[s + 1..])));
            }
            for (what, edited) in mutations {
                let overlays = BTreeMap::from([(path.clone(), edited)]);
                let Some(after) = load(seed, &overlays) else { continue };
                edits += 1;
                let after_diags = after.demand_typing().expect("typed").diags.clone();
                let (per1, out1) = attributed(&after, &after_diags);
                let name = format!("{} {} `{}` ({what})", seed.file_name().unwrap().to_string_lossy(), d.kind, d.name);
                if out0 != out1 {
                    misses.push(format!("{name}: a diagnostic outside every declaration changed: {out0:?} -> {out1:?}"));
                }
                let changed: BTreeSet<DeclKey> = per0
                    .keys()
                    .chain(per1.keys())
                    .filter(|k| per0.get(*k) != per1.get(*k))
                    .cloned()
                    .collect();
                let (Some(a0), Some(a1)) = (answer(&before, key), answer(&after, key)) else {
                    whole += 1;
                    continue;
                };
                answered += 1;
                beyond += changed.iter().filter(|k| *k != key).count();
                alone_misses.extend(changed.iter().filter(|k| *k != key).map(|k| format!("{name} -> {} `{}`", k.1, k.2)));
                for k in changed.difference(&a0.union(&a1).cloned().collect()) {
                    misses.push(format!(
                        "{name}: `{}` {} changed, not a dependent: {:?} -> {:?}",
                        k.2,
                        k.1,
                        per0.get(k),
                        per1.get(k)
                    ));
                }
            }
        }
    }
    println!(
        "{} seeds, {edits} edits: {answered} answered by the relation, {whole} whole; {beyond} declarations re-derived beyond the edited one",
        seeds.len()
    );
    let _ = std::fs::remove_dir_all(&chain);
    println!("re-derived beyond the edited declaration:\n{}", alone_misses.join("\n"));
    assert!(edits > 500, "the mutation covers the corpus: {edits} edits");
    assert!(misses.is_empty(), "{} misses:\n{}", misses.len(), misses.join("\n"));
    // The control: the edited declaration alone is not an answer. A
    // print added to `inner` makes it, `mid` and `enc` opaque to the
    // reveal rule, so `Api`'s reveal, two calls away, is refused: only
    // the callgraph's readers, closed, reach it.
    assert!(
        alone_misses.iter().any(|m| m.ends_with("reveal-chain fn `inner` (a print added) -> locus `Api`")),
        "the transitive reader is re-derived: {alone_misses:?}"
    );
}

/// The relation's domain is a body: an edit to what a declaration
/// declares is read through the scope, which no family records. A field
/// read on a parameter of its locus's type is no call, and nothing
/// builds the locus in the reader, so the reader is no dependent — and a
/// changed field type changes its check. That is why the reuse rule
/// checks the seed whole for such an edit.
#[test]
fn an_edit_to_a_declared_surface_escapes_the_families() {
    let before = "locus Rt {\n    params { n: Int = 1; }\n    fn go() -> Int { return self.n; }\n}\n\
fn use_it(r: Rt) -> Int { return r.n; }\n\
fn main() { println(use_it(Rt { })); }\n";
    let after = before.replace("params { n: Int = 1; }", "params { n: String = \"1\"; }");
    let dir = scratch_seed("declared-surface", before);
    let none = BTreeMap::new();
    let s0 = load(&dir, &none).expect("the seed checks");
    let s1 = load(&dir, &BTreeMap::from([(dir.join("main.hl"), after)])).expect("the edit checks");
    let (per0, _) = attributed(&s0, &s0.demand_typing().expect("typed").diags);
    let (per1, _) = attributed(&s1, &s1.demand_typing().expect("typed").diags);
    let use_it = keys(s0.declarations()).into_iter().find(|k| k.2 == "use_it").expect("use_it");
    let rt = keys(s0.declarations()).into_iter().find(|k| k.2 == "Rt").expect("Rt");
    assert_ne!(per0[&use_it], per1[&use_it], "the reader's check changes");
    for s in [&s0, &s1] {
        assert!(!answer(s, &rt).expect("Rt is placed").contains(&use_it), "and the families do not name it");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A seed where a body edit changes another declaration's check two
/// calls away: `Api` hands a revealed secret to the wire through `enc`,
/// which the reveal rule follows only while `enc`, `mid` and `inner` are
/// transparent.
const REVEAL_CHAIN: &str = "fn enc(s: String) -> String { return mid(s); }\n\
fn mid(s: String) -> String { return inner(s); }\n\
fn inner(s: String) -> String { return s + \"!\"; }\n\
fn unrelated() -> Int { return 1; }\n\
locus Api {\n    params { token: std::secret::Credential = std::secret::Credential { vault: \"api\" }; }\n    \
fn go() -> Bool {\n        \
let r = std::http::post(\"http://127.0.0.1:1/t\", std::bytes::from_string(\"secret=\" + enc(self.token.reveal_text())), \"text/plain\") or std::http::ClientResponse { status: 0, headers: \"\", body: b\"\" };\n        \
return true;\n    }\n}\n\
fn main() { let a = Api { }; }\n";

/// A one-file seed of this test's own: the pid keeps two runs apart, the
/// name two tests of one run (each removes its own directory).
fn scratch_seed(name: &str, text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hale-x2-dependents-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the scratch seed");
    std::fs::write(dir.join("main.hl"), text).expect("the scratch seed's file");
    dir.canonicalize().expect("canonical")
}
