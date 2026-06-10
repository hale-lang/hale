//! GH #18 item 5 — resource-budget tracking, the count slice.
//!
//! A static tally of the language-visible resources a program acquires —
//! a "linter" signal + the basis for a CI ceiling gate ("this PR raised
//! the fd/thread/subject count — intentional?"). This slice covers the
//! cheap, structural, top-level-walk resources with **zero false
//! positives** (it's a count):
//!
//! - **OS threads** = pinned loci (`PlacementSpec::Pinned` placement
//!   entries — one `pthread` each).
//! - **Cooperative pools** = distinct `cooperative(pool = X)` names (one
//!   shared OS thread each; `None` → the program's `main` thread).
//! - **Bus subjects** = distinct registered subject strings
//!   (subscribe/publish `canonical()` + `topic` decls — router entries).
//!
//! Held-fd counts + leak detection (which reuses `alloc_summary`'s
//! unbounded-context dataflow) are the next stages — see
//! `notes/resource-budgets.md`.

use std::collections::BTreeSet;

use hale_syntax::ast::*;

/// Per-program resource tally.
#[derive(Debug, Clone, Default)]
pub struct ResourceBudget {
    /// Pinned placement entries — one OS thread (`pthread`) each.
    pub pinned_threads: usize,
    /// Distinct cooperative pool names (one shared OS thread each).
    /// `main` is the program's own thread (a `cooperative` with no pool).
    pub cooperative_pools: BTreeSet<String>,
    /// Distinct bus subject strings (router table entries).
    pub bus_subjects: BTreeSet<String>,
}

/// Walk the bundle and tally the structural resources.
pub fn budget_for_programs(programs: &[&Program]) -> ResourceBudget {
    let mut b = ResourceBudget::default();
    for program in programs {
        for item in &program.items {
            match item {
                TopDecl::Topic(t) => {
                    // A topic decl is a router registration; its subject
                    // defaults to the topic name (a subscribe/publish that
                    // references it dedupes via the same canonical string).
                    b.bus_subjects.insert(t.name.name.clone());
                }
                TopDecl::Locus(l) => collect_locus(l, &mut b),
                TopDecl::Module(m) => {
                    for it in &m.items {
                        if let TopDecl::Locus(l) = it {
                            collect_locus(l, &mut b);
                        } else if let TopDecl::Topic(t) = it {
                            b.bus_subjects.insert(t.name.name.clone());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    b
}

fn collect_locus(l: &LocusDecl, b: &mut ResourceBudget) {
    for member in &l.members {
        match member {
            LocusMember::Placement(pb) => {
                for entry in &pb.entries {
                    match &entry.spec {
                        PlacementSpec::Pinned { .. } => b.pinned_threads += 1,
                        PlacementSpec::Cooperative { pool } => {
                            let name = pool
                                .as_ref()
                                .map(|p| p.name.clone())
                                .unwrap_or_else(|| "main".to_string());
                            b.cooperative_pools.insert(name);
                        }
                    }
                }
            }
            LocusMember::Bus(bus) => {
                for bm in &bus.members {
                    let subject = match bm {
                        BusMember::Subscribe { subject, .. } => subject,
                        BusMember::Publish { subject, .. } => subject,
                    };
                    b.bus_subjects.insert(subject.canonical().to_string());
                }
            }
            _ => {}
        }
    }
}

impl ResourceBudget {
    /// Human-readable dump for `--dump-resource-budget`.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("# resource budget (GH #18 item 5, count slice)\n\n");
        out.push_str(&format!("OS threads (pinned loci):  {}\n", self.pinned_threads));
        out.push_str(&format!(
            "cooperative pools:         {}{}\n",
            self.cooperative_pools.len(),
            if self.cooperative_pools.is_empty() {
                String::new()
            } else {
                format!(
                    "  [{}]",
                    self.cooperative_pools.iter().cloned().collect::<Vec<_>>().join(", ")
                )
            }
        ));
        out.push_str(&format!("bus subjects:              {}\n", self.bus_subjects.len()));
        for s in &self.bus_subjects {
            out.push_str(&format!("    - {}\n", s));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hale_syntax::parse_source;

    fn budget(src: &str) -> ResourceBudget {
        let program = parse_source(src).expect("parse");
        budget_for_programs(&[&program])
    }

    #[test]
    fn counts_bus_subjects_distinctly() {
        // topic + a subscribe + a publish on two distinct subjects → 2.
        let src = r#"
            type T { n: Int; }
            locus C {
                bus {
                    subscribe "in" as on_in of type T;
                    publish "out" of type T;
                }
                fn on_in(m: T) { let _ = m.n; }
            }
            fn main() { }
        "#;
        let b = budget(src);
        assert_eq!(b.bus_subjects.len(), 2, "subjects: {:?}", b.bus_subjects);
        assert!(b.bus_subjects.contains("in"));
        assert!(b.bus_subjects.contains("out"));
    }

    #[test]
    fn dedupes_subject_across_subscribe_and_publish() {
        let src = r#"
            type T { n: Int; }
            locus A { bus { publish "ev" of type T; } }
            locus B {
                bus { subscribe "ev" as on_ev of type T; }
                fn on_ev(m: T) { let _ = m.n; }
            }
            fn main() { }
        "#;
        let b = budget(src);
        assert_eq!(b.bus_subjects.len(), 1, "same subject should dedupe: {:?}", b.bus_subjects);
    }

    #[test]
    fn no_resources_in_a_plain_program() {
        let b = budget("fn main() { println(\"hi\"); }");
        assert_eq!(b.pinned_threads, 0);
        assert!(b.cooperative_pools.is_empty());
        assert!(b.bus_subjects.is_empty());
    }
}
