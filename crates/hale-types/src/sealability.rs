//! GH #436: which loci could be `@sealed` today, and what it would cost.
//!
//! `@sealed` is opt-in, so adopting it across an existing codebase is a
//! question nobody can answer by reading: which loci already hold their
//! state privately, and which ones have callers reaching into them?
//! That is mechanically computable, and leaving it to inspection was
//! the part of "measure before building more" I wrongly called
//! not-code.
//!
//! **The survey runs the real check rather than reimplementing it.**
//! It clones the bundle, marks every locus `@sealed`, and re-checks:
//! each resulting diagnostic is a site that sealing would break. A
//! hand-written walk over `self.child.field` would drift from
//! `check_sealed_read` the first time either changed, and a survey that
//! disagrees with the checker is worse than none — it would tell you a
//! locus is free to seal when it is not.

use std::collections::BTreeMap;

use hale_syntax::ast::{Program, TopDecl};

/// One locus's verdict.
pub struct Sealable {
    pub locus: String,
    /// Sites outside the locus that read its `params`. Empty means
    /// sealing it today is a no-op.
    pub blockers: Vec<String>,
}

fn mark_all_sealed(items: &mut [TopDecl]) {
    for item in items {
        match item {
            TopDecl::Locus(l) => l.sealed = true,
            TopDecl::Module(m) => mark_all_sealed(&mut m.items),
            _ => {}
        }
    }
}

fn locus_names(items: &[TopDecl], out: &mut Vec<String>) {
    for item in items {
        match item {
            TopDecl::Locus(l) => out.push(l.name.name.clone()),
            TopDecl::Module(m) => locus_names(&m.items, out),
            _ => {}
        }
    }
}

/// Survey every locus in the bundle.
///
/// Already-sealed loci are included with no blockers — they are sealed,
/// so they trivially pass, and omitting them would make the report read
/// as though they were unexamined.
pub fn survey(programs: &[&Program]) -> Vec<Sealable> {
    let mut names: Vec<String> = Vec::new();
    for p in programs {
        locus_names(&p.items, &mut names);
    }
    names.sort();
    names.dedup();

    // Clone, seal everything, re-check. The diagnostics ARE the answer.
    let sealed: Vec<Program> = programs
        .iter()
        .map(|p| {
            let mut c = (*p).clone();
            mark_all_sealed(&mut c.items);
            c
        })
        .collect();
    let mut map: BTreeMap<String, &Program> = BTreeMap::new();
    for (i, p) in sealed.iter().enumerate() {
        map.insert(format!("{i}"), p);
    }
    let diags = crate::check_bundle(&crate::Bundle::new(map));

    let mut blockers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for d in diags {
        if !d.message.contains("is `@sealed`") {
            continue;
        }
        // "`L` is `@sealed`: … and `L.f` reads one from outside — …"
        let Some(rest) = d.message.strip_prefix('`') else { continue };
        let Some(end) = rest.find('`') else { continue };
        let owner = rest[..end].to_string();
        let site = d
            .message
            .split_once("and `")
            .and_then(|(_, r)| r.split_once('`'))
            .map(|(s, _)| s.to_string())
            .unwrap_or_else(|| owner.clone());
        blockers.entry(owner).or_default().push(site);
    }

    names
        .into_iter()
        .map(|locus| {
            let mut b = blockers.remove(&locus).unwrap_or_default();
            b.sort();
            b.dedup();
            Sealable { locus, blockers: b }
        })
        .collect()
}

/// Render the survey for the CLI.
pub fn render(rows: &[Sealable]) -> String {
    let free: Vec<&Sealable> =
        rows.iter().filter(|r| r.blockers.is_empty()).collect();
    let blocked: Vec<&Sealable> =
        rows.iter().filter(|r| !r.blockers.is_empty()).collect();

    let mut out = String::new();
    out.push_str(&format!(
        "sealability: {} of {} loci can be `@sealed` today\n",
        free.len(),
        rows.len()
    ));
    if !free.is_empty() {
        out.push_str("\n  free to seal (nothing outside reads their params):\n");
        for r in &free {
            out.push_str(&format!("    {}\n", r.locus));
        }
    }
    if !blocked.is_empty() {
        out.push_str("\n  would break callers:\n");
        for r in &blocked {
            out.push_str(&format!(
                "    {} — {} external read(s): {}\n",
                r.locus,
                r.blockers.len(),
                r.blockers.join(", ")
            ));
        }
    }
    out
}
