//! GH #18 item 5 — resource-budget tracking, the count slice.
//!
//! A static tally of the language-visible resources a program acquires —
//! a "linter" signal + the basis for a CI ceiling gate ("this PR raised
//! the fd/thread/subject count — intentional?"). It counts the resource,
//! never the declaration that asks for it:
//!
//! - **OS threads** are read from the placement table (F.40 phase 3, P1;
//!   the correspondence's § 2.8), partitioned by the scope that creates
//!   each thread. A pinned anchor of the root (a root field placed
//!   `pinned`, one per replica) is one thread per live occurrence of the
//!   construction that builds it: its count in one occurrence times that
//!   construction's bound, summed over the constructions, with an
//!   unbounded construction making the count an uncertainty that carries
//!   its reason. An adapter of the root's `bindings { }` is one thread,
//!   counted once whatever the root's bound. A nested row inherits its
//!   owner's thread and adds none.
//! - **Cooperative pools** are the table's worker pools, one per name
//!   however many instances run on it, and an affinity adds no thread.
//!   The main thread is the program's own: never a pool, and shown on a
//!   line of its own.
//! - **Bus subjects** = distinct registered subject strings
//!   (subscribe/publish `canonical()` + `topic` decls — router entries).
//! - **fd acquisition sites**, from `alloc_summary`'s own rows.
//!
//! Threads the runtime spawns outside placement are not placement facts
//! and are not counted: a transport binding's reader thread, and the
//! serve thread a stdlib transport's birth spawns. The dump says so on a
//! line of its own, with the root's transport bindings counted.
//!
//! Leak detection reuses `alloc_summary`'s unbounded-context dataflow;
//! see `notes/resource-budgets.md`.

use std::collections::BTreeSet;

use hale_syntax::ast::*;
use hale_syntax::Diag;

use crate::alloc_summary::{AllocKind, AllocSummary, Callee, Escape};
use crate::placement::{Bound, DomainKind, Origin, PlacementTable};
use crate::symbol::Bundle;

/// Held-fd loci instantiated directly (vs via a call) — `tcp::Listener { }`
/// holds a listening fd from birth. Matched on the *qualified* struct path
/// (so a user type named `Listener` doesn't collide). `File` comes via
/// `open()` (a call, counted above); `Stream` mostly via connect/accept.
const HELD_FD_LOCUS_PATHS: &[&str] =
    &["std::io::tcp::Listener", "std::io::tcp::Stream"];

/// Stdlib path-calls that acquire a held OS resource (a file descriptor)
/// — the result is a locus (`File` / `Stream` / `Listener`) that closes
/// the fd on dissolve. If such a call's result is stored resident
/// (`self`-store) and the call runs in an unbounded context, the fd
/// accumulates. Unmangled paths (this analysis runs pre-rename).
const FD_ACQUIRING_PATHS: &[&str] = &[
    "std::io::file::open",
    "std::io::tcp::connect",
    // GH #1030's waiting dial, a descriptor like `connect`'s (missed when
    // it was added; F.40 phase 4, S5).
    "std::io::tcp::connect_wait",
    "std::io::tcp::listen_socket",
    "std::io::tcp::__listen_socket",
    "std::io::tcp::accept_one",
    "std::io::tcp::__accept_one",
    "std::io::unix::listen_socket",
    "std::io::unix::connect",
    "std::io::unix::connect_wait",
];

/// GH #18 item 5, leak-detection stage: warn on an fd-acquiring call whose
/// result is stored resident (`self`) in an unboundedly-invoked fn or an
/// unbounded loop — the fd accumulates. Reuses `alloc_summary`'s
/// call-result escape tagging + unbounded-context dataflow (the gap that
/// item 1's site-only escape tagging left open). Opt-in via
/// `hale check --warn-resource-leak`. `summary` is the `alloc_summary`
/// family's; the warnings read its own rows, so the stdlib's analysis
/// copy (the TCP `Listener`'s hooks accept and store streams) is not the
/// program's acquisition.
pub fn resource_leak_diags(summary: &AllocSummary) -> Vec<Diag> {
    let summary = summary.program_rows();
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
#[derive(Debug, Clone)]
pub struct ResourceBudget {
    /// The threads of the root's pinned anchors: a root field placed
    /// `pinned`, one per replica, times the live occurrences of the
    /// construction that builds it, summed over the constructions. `Err`
    /// is the uncertainty, with its reason: a construction with anchors
    /// and no static bound.
    pub anchored_threads: Result<usize, String>,
    /// The adapters of the root's `bindings { }`: one thread each, built
    /// once by the bindings prelude.
    pub adapter_threads: usize,
    /// The worker pools, by name: one worker each, however many instances
    /// run on it. `main` is never one.
    pub cooperative_pools: BTreeSet<String>,
    /// Distinct bus subject strings (router table entries).
    pub bus_subjects: BTreeSet<String>,
    /// fd acquisition sites — the file-descriptor surface. Counts both
    /// fd-opening calls (`std::io::file::open` / `tcp::connect` / `listen`
    /// / `accept`) and direct held-fd locus instantiations
    /// (`tcp::Listener { }` / `tcp::Stream { }`). A static site count, not
    /// a runtime fd count.
    pub fd_open_sites: usize,
    /// The root's `bindings { }` entries that are no adapter: each runs a
    /// reader thread (and a stdlib transport's birth its serve thread)
    /// the runtime spawns outside placement, which the budget does not
    /// count.
    pub transport_bindings: usize,
}

impl ResourceBudget {
    /// The OS threads placement spawns: the pinned anchors' and the
    /// adapters'. Never a bound on all of the process's threads (the
    /// main thread, binding readers and transport serve threads are not
    /// in it).
    pub fn threads(&self) -> Result<usize, String> {
        self.anchored_threads.clone().map(|n| n + self.adapter_threads)
    }
}

/// Declared per-resource ceilings (from a project's resource-budget file).
/// `None` = no ceiling for that resource. A count exceeding its ceiling
/// fails `--check-resource-budget` — the CI gate ("this PR raised the
/// thread/subject count — intentional? bump the ceiling").
#[derive(Debug, Clone, Default)]
pub struct ResourceCeiling {
    pub pinned_threads: Option<usize>,
    pub cooperative_pools: Option<usize>,
    pub bus_subjects: Option<usize>,
    pub fd_open_sites: Option<usize>,
}

/// Compare a tallied budget against declared ceilings. Returns one
/// violation message per resource over its ceiling (empty = within
/// budget). Resources without a declared ceiling are unconstrained; an
/// uncertain thread count fails a declared thread ceiling, with its
/// reason.
pub fn check_ceiling(b: &ResourceBudget, c: &ResourceCeiling) -> Vec<String> {
    let mut v = Vec::new();
    if let (Err(why), Some(max)) = (b.threads(), c.pinned_threads) {
        v.push(format!(
            "pinned_threads (OS threads): uncertain ({why}), so no count is within the declared \
             ceiling of {max}"
        ));
    }
    let mut chk = |name: &str, actual: usize, ceil: Option<usize>| {
        if let Some(max) = ceil {
            if actual > max {
                v.push(format!(
                    "{}: {} exceeds the declared ceiling of {}",
                    name, actual, max
                ));
            }
        }
    };
    if let Ok(n) = b.threads() {
        chk("pinned_threads (OS threads)", n, c.pinned_threads);
    }
    chk("cooperative_pools", b.cooperative_pools.len(), c.cooperative_pools);
    chk("bus_subjects", b.bus_subjects.len(), c.bus_subjects);
    chk("fd_open_sites", b.fd_open_sites, c.fd_open_sites);
    v
}

/// Tally the program's resources: the threads and pools from `table`
/// (the snapshot's placement table, `Snapshot::demand_placement`), the
/// bus subjects from `bundle`'s declarations, the fd sites from
/// `summary`'s own rows (the `alloc_summary` family's summary).
pub fn budget_for_programs(bundle: &Bundle<'_>, table: &PlacementTable, summary: &AllocSummary) -> ResourceBudget {
    // The threads, partitioned by the scope that creates each: a root
    // construction's anchors under its bound, an adapter once.
    let mut anchored_threads: Result<usize, String> = Ok(0);
    let mut adapter_threads = 0usize;
    for (top, bound) in table.templates() {
        let anchors = table.per_occurrence(&top, &|k, r| table.is_anchor(k, r)) as usize;
        if anchors == 0 {
            continue;
        }
        if matches!(top.origin, Origin::Binding(_)) {
            adapter_threads += anchors;
            continue;
        }
        let live = match bound {
            Bound::Once => Ok(anchors),
            Bound::AtMost(n) => Ok(anchors * n as usize),
            Bound::Unbounded(why) => Err(why),
        };
        anchored_threads = match (anchored_threads, live) {
            (Ok(a), Ok(b)) => Ok(a + b),
            (Err(why), _) | (_, Err(why)) => Err(why),
        };
    }
    let cooperative_pools = table
        .domains
        .iter()
        .filter_map(|d| match &d.kind {
            DomainKind::Pool { name, .. } => Some(name.clone()),
            DomainKind::Main | DomainKind::Pinned { .. } => None,
        })
        .collect();
    let transport_bindings = table
        .root
        .as_ref()
        .and_then(|r| r.decl.decl(bundle))
        .map_or(0, |l| {
            l.members
                .iter()
                .filter_map(|m| match m {
                    LocusMember::Bindings(bb) => Some(&bb.entries),
                    _ => None,
                })
                .flatten()
                .filter(|e| !matches!(e.transport, TransportSpec::Adapter { .. }))
                .count()
        });
    let mut b = ResourceBudget {
        anchored_threads,
        adapter_threads,
        cooperative_pools,
        bus_subjects: BTreeSet::new(),
        fd_open_sites: 0,
        transport_bindings,
    };
    for program in bundle.programs.values() {
        for item in &program.items {
            match item {
                TopDecl::Topic(t) => {
                    // A topic decl is a router registration; its subject
                    // defaults to the topic name (a subscribe/publish that
                    // references it dedupes via the same canonical string).
                    b.bus_subjects.insert(t.name.name.clone());
                }
                TopDecl::Locus(l) => collect_subjects(l, &mut b),
                TopDecl::Module(m) => {
                    for it in &m.items {
                        if let TopDecl::Locus(l) = it {
                            collect_subjects(l, &mut b);
                        } else if let TopDecl::Topic(t) = it {
                            b.bus_subjects.insert(t.name.name.clone());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // fd acquisition sites — reuse alloc_summary. Two forms, both
    // unambiguous (qualified paths → zero FP): fd-opening *calls*
    // (open/connect/accept) and direct held-fd *locus instantiations*
    // (`tcp::Listener { }`). The program's own: the stdlib's analysis
    // copy opens fds inside the loci the program starts.
    let summary = summary.program_rows();
    let calls = summary
        .fns
        .values()
        .flat_map(|f| f.calls.iter())
        .filter(|c| matches!(&c.callee, Callee::Unresolved(p) if FD_ACQUIRING_PATHS.contains(&p.as_str())))
        .count();
    let loci = summary
        .fns
        .values()
        .flat_map(|f| f.sites.iter())
        .filter(|s| matches!(&s.kind, AllocKind::StructLit(n) if HELD_FD_LOCUS_PATHS.contains(&n.as_str())))
        .count();
    b.fd_open_sites = calls + loci;
    b
}

fn collect_subjects(l: &LocusDecl, b: &mut ResourceBudget) {
    for member in &l.members {
        if let LocusMember::Bus(bus) = member {
            for bm in &bus.members {
                let subject = match bm {
                    BusMember::Subscribe { subject, .. } => subject,
                    BusMember::Publish { subject, .. } => subject,
                };
                b.bus_subjects.insert(subject.canonical().to_string());
            }
        }
    }
}

impl ResourceBudget {
    /// Human-readable dump for `--dump-resource-budget`.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("# resource budget (GH #18 item 5, count slice)\n");
        out.push_str("#\n");
        out.push_str("# OS threads are the placement table's: one per pinned anchor (a root field\n");
        out.push_str("# placed `pinned`, one per replica) for each live occurrence of the construction\n");
        out.push_str("# that builds it, and one per adapter in the root's `bindings { }`, built once.\n");
        out.push_str("# A cooperative pool is one worker however many instances run on it; the main\n");
        out.push_str("# thread is the program's own, never a pool. Not counted: a transport\n");
        out.push_str("# binding's reader thread, and the serve thread a stdlib transport's birth spawns.\n\n");
        let threads = match self.threads() {
            Ok(n) => n.to_string(),
            Err(why) => format!("uncertain ({why})"),
        };
        out.push_str(&format!("OS threads (placement):   {}\n", threads));
        let anchored = match &self.anchored_threads {
            Ok(n) => n.to_string(),
            Err(why) => format!("uncertain ({why})"),
        };
        out.push_str(&format!("    pinned anchors:        {}\n", anchored));
        out.push_str(&format!("    adapters:              {}\n", self.adapter_threads));
        out.push_str("main thread:               1 (the program's own; not a pool)\n");
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
        out.push_str(&format!("fd acquisition sites:      {}\n", self.fd_open_sites));
        out.push_str(&format!(
            "not counted:               the reader threads of {} transport binding(s), and \
             any serve thread a stdlib transport's birth spawns\n",
            self.transport_bindings
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hale_syntax::parse_source;

    /// `f` over the bundle of a parsed program and its allocation summary,
    /// the one the bundle holds.
    fn summarized<T>(src: &str, f: impl FnOnce(&Bundle<'_>, &AllocSummary) -> T) -> T {
        let program = parse_source(src).expect("parse");
        let mut programs = std::collections::BTreeMap::new();
        programs.insert("app.hl".to_string(), &program);
        let bundle = Bundle::new(programs);
        let summary = crate::alloc_summary::derive_alloc_summary(&bundle);
        f(&bundle, &summary)
    }

    /// The subjects and fd sites, which read no placement: the table is
    /// empty. The placement accounting (threads, pools) is pinned through
    /// the frontend's load, in `tests/resource_budget.rs`.
    fn budget(src: &str) -> ResourceBudget {
        summarized(src, |bundle, summary| budget_for_programs(bundle, &PlacementTable::default(), summary))
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
    fn counts_fd_open_call_sites() {
        let src = r#"
            type Msg { path: String; }
            locus Opener {
                bus { subscribe "open" as on_open of type Msg; }
                fn on_open(m: Msg) {
                    let a = std::io::file::open(m.path, "r") or raise;
                    let c = std::io::tcp::connect("127.0.0.1", 80) or raise;
                    let n = m.path;
                }
            }
            fn main() { }
        "#;
        let b = budget(src);
        assert_eq!(b.fd_open_sites, 2, "expected 2 fd-open sites (open + connect)");
    }

    /// F.40 phase 4, S5 (a classified correction): the tcp `connect_wait`
    /// acquires a descriptor as `connect` does, and as the unix one was
    /// already counted; a program that calls it counts one more site.
    #[test]
    fn counts_tcp_connect_wait_as_an_fd_open_site() {
        let src = r#"
            fn dial() {
                let c = std::io::tcp::connect_wait("127.0.0.1", 80, 1s) or raise;
                let u = std::io::unix::connect_wait("/tmp/s.sock", 1s) or raise;
            }
            fn main() { }
        "#;
        let b = budget(src);
        assert_eq!(b.fd_open_sites, 2, "expected 2 fd-open sites (tcp + unix connect_wait)");
    }

    #[test]
    fn counts_held_fd_locus_instantiations() {
        // Direct held-fd locus instantiations, matched on the qualified
        // path (a user type named `Listener` would not collide).
        let src = r#"
            fn serve() {
                let l = std::io::tcp::Listener { port: 8080 };
                let s = std::io::tcp::Stream { fd: 3 };
            }
            fn main() { }
        "#;
        let b = budget(src);
        assert_eq!(b.fd_open_sites, 2, "Listener + Stream instantiations");
    }

    #[test]
    fn user_type_named_listener_does_not_collide() {
        // A local `Listener` (single-segment path) is NOT a held-fd locus.
        let src = r#"
            type Listener { id: Int; }
            fn make() -> Listener { return Listener { id: 1 }; }
            fn main() { }
        "#;
        assert_eq!(budget(src).fd_open_sites, 0);
    }

    #[test]
    fn ceiling_check_flags_over_budget_and_passes_within() {
        let src = r#"
            type T { n: Int; }
            locus C {
                bus {
                    subscribe "a" as oa of type T;
                    publish "b" of type T;
                    subscribe "c" as oc of type T;
                }
                fn oa(m: T) { let _ = m.n; }
                fn oc(m: T) { let _ = m.n; }
            }
            fn main() { }
        "#;
        let b = budget(src);
        assert_eq!(b.bus_subjects.len(), 3);
        // Ceiling 2 → over budget.
        let over = check_ceiling(
            &b,
            &ResourceCeiling { bus_subjects: Some(2), ..Default::default() },
        );
        assert_eq!(over.len(), 1, "{:?}", over);
        assert!(over[0].contains("bus_subjects: 3 exceeds"), "{:?}", over);
        // Ceiling 3 → within budget; no ceiling on the others → unconstrained.
        let within = check_ceiling(
            &b,
            &ResourceCeiling { bus_subjects: Some(3), ..Default::default() },
        );
        assert!(within.is_empty(), "should be within budget: {:?}", within);
    }

    #[test]
    fn no_resources_in_a_plain_program() {
        let b = budget("fn main() { println(\"hi\"); }");
        assert_eq!(b.threads(), Ok(0));
        assert!(b.cooperative_pools.is_empty());
        assert!(b.bus_subjects.is_empty());
    }

    // ---- leak detection (result-escape tagging) ----

    fn leaks(src: &str) -> Vec<String> {
        summarized(src, |_, summary| resource_leak_diags(summary).iter().map(|d| d.message.clone()).collect())
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
