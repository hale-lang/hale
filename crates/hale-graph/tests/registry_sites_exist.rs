//! Every site the registry names exists in the tree.
//!
//! A registry that cites a function which was renamed or deleted is
//! worse than none: it tells the next contributor to read code that
//! is not there. So every `Site { path, symbol }` in every family,
//! legacy producer, consumer, seam, rule evaluator and Debug-scan
//! entry is checked against the workspace: the file exists and its
//! text contains the symbol. A symbol may be a function, a type, a
//! constant or, for a site inside a large function, a distinctive
//! text fragment.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn check_site(root: &Path, site: &hale_graph::Site, where_: &str, missing: &mut Vec<String>) {
    let path = root.join(site.path);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            if !text.contains(site.symbol) {
                missing.push(format!(
                    "{where_}: `{}` is not in {} (renamed or deleted; fix the registry)",
                    site.symbol, site.path
                ));
            }
        }
        Err(_) => missing.push(format!("{where_}: {} does not exist", site.path)),
    }
}

#[test]
fn every_registered_site_exists() {
    let root = workspace_root();
    let mut missing = Vec::new();
    let mut seen = 0usize;
    for f in hale_graph::families() {
        if let Some(p) = &f.producer {
            check_site(
                &root,
                p,
                &format!("family `{}` producer", f.name),
                &mut missing,
            );
            seen += 1;
        }
        for l in f.legacy {
            check_site(
                &root,
                &l.site,
                &format!("family `{}` legacy", f.name),
                &mut missing,
            );
            seen += 1;
        }
        for c in f.consumers {
            if let Some(s) = &c.site {
                check_site(
                    &root,
                    s,
                    &format!("family `{}` consumer {}", f.name, c.who),
                    &mut missing,
                );
                seen += 1;
            }
        }
        for s in f.seams {
            for allowed in s.allowed {
                let site = hale_graph::Site {
                    path: allowed,
                    symbol: s.symbol,
                };
                check_site(
                    &root,
                    &site,
                    &format!("family `{}` seam `{}`", f.name, s.symbol),
                    &mut missing,
                );
                seen += 1;
            }
        }
    }
    for r in hale_graph::rules() {
        if let Some(e) = &r.evaluator {
            check_site(
                &root,
                e,
                &format!("rule `{}` evaluator", r.id),
                &mut missing,
            );
            seen += 1;
        }
    }
    for d in hale_graph::DEBUG_SCANS {
        let site = hale_graph::Site {
            path: d.path,
            symbol: d.fragment,
        };
        check_site(&root, &site, "debug scan", &mut missing);
        seen += 1;
    }
    assert!(seen > 150, "the site scan is vacuous ({seen} sites)");
    assert!(
        missing.is_empty(),
        "{} registered site(s) do not exist:\n{}",
        missing.len(),
        missing.join("\n")
    );
}
