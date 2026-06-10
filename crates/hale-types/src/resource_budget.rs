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
use hale_syntax::Diag;

use crate::alloc_summary::{self, Callee, Escape};

/// Stdlib path-calls that acquire a held OS resource (a file descriptor)
/// — the result is a locus (`File` / `Stream` / `Listener`) that closes
/// the fd on dissolve. If such a call's result is stored resident
/// (`self`-store) and the call runs in an unbounded context, the fd
/// accumulates. Unmangled paths (this analysis runs pre-rename).
const FD_ACQUIRING_PATHS: &[&str] = &[
    "std::io::file::open",
    "std::io::tcp::connect",
    "std::io::tcp::listen_socket",
    "std::io::tcp::__listen_socket",
    "std::io::tcp::accept_one",
    "std::io::tcp::__accept_one",
];

/// GH #18 item 5, leak-detection stage: warn on an fd-acquiring call whose
/// result is stored resident (`self`) in an unboundedly-invoked fn or an
/// unbounded loop — the fd accumulates. Reuses `alloc_summary`'s
/// call-result escape tagging + unbounded-context dataflow (the gap that
/// item 1's site-only escape tagging left open). Opt-in via
/// `hale check --warn-resource-leak`.
pub fn resource_leak_diags(programs: &[&Program]) -> Vec<Diag> {
    let summary = alloc_summary::summarize_programs(programs);
    let unbounded = summary.unbounded_invoked();
    let mut out = Vec::new();
    for f in summary.fns.values() {
        for c in &f.calls {
            let path = match &c.callee {
                Callee::Unresolved(p) => p,
                Callee::Resolved(_) => continue,
            };
            if !FD_ACQUIRING_PATHS.contains(&path.as_str()) {
                continue;
            }
            // The fd holder must escape its scope (stored resident) — a
            // `Local` holder is bound + dissolved per iteration (the fd
            // closes), so it's bounded.
            if !matches!(c.escape, Escape::StoredToSelf) {
                continue;
            }
            // ... in an unbounded context (a per-message handler, or a
            // call inside an unbounded loop). A one-shot self-store (e.g.
            // a server's single listener in birth) is fine.
            if !(c.in_unbounded_loop || unbounded.contains(&f.key)) {
                continue;
            }
            out.push(Diag::warn(
                c.span,
                format!(
                    "unbounded fd acquisition: `{}` opens a held resource stored to \
                     `self` in `{}`, which runs unboundedly (a per-message handler or a \
                     call inside an unbounded loop) — the file descriptor accumulates \
                     resident. Dissolve the holder per iteration (a scoped `let`), or \
                     keep a single long-lived holder instead of re-opening.",
                    path,
                    f.key.display()
                ),
            ));
        }
    }
    out
}

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

    // ---- leak detection (result-escape tagging) ----

    fn leaks(src: &str) -> Vec<String> {
        let program = parse_source(src).expect("parse");
        resource_leak_diags(&[&program]).iter().map(|d| d.message.clone()).collect()
    }

    #[test]
    fn fd_stored_to_self_in_handler_is_flagged() {
        // A per-message handler that opens an fd and stores it resident →
        // the fd accumulates per message. The result-escape tag (the call's
        // result flows to self) + the unbounded handler context catch it.
        let src = r#"
            type Msg { path: String; }
            locus Opener {
                params { f: Int = 0; }
                bus { subscribe "open" as on_open of type Msg; }
                fn on_open(m: Msg) {
                    self.f = std::io::file::open(m.path, "r") or raise;
                }
            }
            fn main() { }
        "#;
        let ls = leaks(src);
        assert_eq!(ls.len(), 1, "expected 1 fd-leak; got {:?}", ls);
        assert!(ls[0].contains("unbounded fd acquisition"), "got: {:?}", ls);
    }

    #[test]
    fn local_fd_in_handler_is_not_flagged() {
        // The fd is bound to a `let` (not stored), so it dissolves at scope
        // exit — bounded, not a leak.
        let src = r#"
            type Msg { path: String; }
            locus Opener {
                bus { subscribe "open" as on_open of type Msg; }
                fn on_open(m: Msg) {
                    let f = std::io::file::open(m.path, "r") or raise;
                }
            }
            fn main() { }
        "#;
        assert!(leaks(src).is_empty(), "a let-scoped fd must not be flagged: {:?}", leaks(src));
    }

    #[test]
    fn fd_stored_in_birth_is_not_flagged() {
        // One-shot self-store (birth) — a single long-lived holder, not a
        // per-iteration accumulation.
        let src = r#"
            locus Server {
                params { sock: Int = 0; }
                birth() {
                    self.sock = std::io::tcp::listen_socket(8080) or raise;
                }
            }
            fn main() { }
        "#;
        assert!(leaks(src).is_empty(), "a one-shot birth open is not a leak: {:?}", leaks(src));
    }
}
