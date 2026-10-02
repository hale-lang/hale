//! The plan notation the lifecycle trace oracle reads (F.40 phase 3,
//! L2): one line per declaration, its steps in the order they hold
//! within one domain, and the edges between events.
//!
//! ```text
//! <Decl>[*N|*+]: <Kind>[@Spine][*N][=Terminal][!domain] ...
//! edge <Decl>.<Kind>[@Spine].<Point> -> <Decl>.<Kind>[@Spine].<Point>
//! ```
//!
//! `Decl` is the lowered declaration name, `-` for a process-level
//! obligation. A step is owed once by each of `N` subjects (default 1;
//! `*+` at least one): instances, or incarnations for `Birth` and
//! `Run`. `=Terminal` names the end it reaches (default `Completed`);
//! `!domain` claims the thread it runs on. An edge's `Point` is
//! `Entered`, `Completed` or `Ended`; within one declaration it holds
//! per incarnation between two steps owed per incarnation, otherwise
//! per instance, and across declarations for every instance of the
//! first. The steps of a line are ordered on the same subjects.
//!
//! The lifecycle fixtures (`lifecycle_fixtures.rs`, a plan per decision
//! line) and the lifecycle matrix (`lifecycle_matrix.rs`, a plan per
//! cell) both write their plans in it.

use hale_types::lifecycle::trace::{Count, Expected, Owed};
use hale_types::lifecycle::{Event, Multiplicity, ObligationId, ObligationKind, Point, Spine, Terminal};

/// The kinds owed once per incarnation; the rest once per instance.
fn multiplicity(kind: ObligationKind) -> Multiplicity {
    match kind {
        ObligationKind::Birth | ObligationKind::Run | ObligationKind::RunEnd | ObligationKind::Closures => {
            Multiplicity::OncePerIncarnation
        }
        _ => Multiplicity::OncePerInstance,
    }
}

fn parse_count(s: &str) -> Count {
    match s {
        "+" => Count::AtLeast(1),
        n => Count::Exactly(n.parse().unwrap_or_else(|_| panic!("plan: bad count {n:?}"))),
    }
}

/// `Kind[@Spine]` into its parts.
fn kind_spine(s: &str) -> (ObligationKind, Option<Spine>) {
    let (k, sp) = match s.split_once('@') {
        Some((k, sp)) => (k, Some(Spine::from_name(sp).unwrap_or_else(|| panic!("plan: unknown spine {sp}")))),
        None => (s, None),
    };
    (ObligationKind::from_name(k).unwrap_or_else(|| panic!("plan: unknown kind {k}")), sp)
}

fn decl_of(s: &str) -> Option<String> {
    (s != "-").then(|| s.to_string())
}

/// A plan in the notation above.
pub fn plan(text: &str) -> Expected {
    let mut exp = Expected::default();
    let mut edges: Vec<(&str, &str)> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(edge) = line.strip_prefix("edge ") {
            let (a, b) = edge.split_once(" -> ").unwrap_or_else(|| panic!("plan: bad edge {line:?}"));
            edges.push((a, b));
            continue;
        }
        let (head, steps) = line.split_once(':').unwrap_or_else(|| panic!("plan: bad line {line:?}"));
        let (decl, line_count) = match head.split_once('*') {
            Some((d, c)) => (d, parse_count(c)),
            None => (head, Count::Exactly(1)),
        };
        let mut seq = Vec::new();
        for step in steps.split_whitespace() {
            let (step, domain) = match step.split_once('!') {
                Some((s, d)) => (s, Some(d.to_string())),
                None => (step, None),
            };
            let (step, ends) = match step.split_once('=') {
                Some((s, t)) => {
                    (s, Point::Terminal(Terminal::from_name(t).unwrap_or_else(|| panic!("plan: unknown terminal {t}"))))
                }
                None => (step, Point::Completed),
            };
            let (step, count) = match step.split_once('*') {
                Some((s, c)) => (s, parse_count(c)),
                None => (step, line_count),
            };
            let (kind, spine) = kind_spine(step);
            seq.push(ObligationId(exp.owed.len() as u32));
            exp.owed.push(Owed {
                decl: decl_of(decl),
                kind,
                spine,
                multiplicity: multiplicity(kind),
                count,
                ends,
                domain,
            });
        }
        exp.sequences.push(seq);
    }
    for (a, b) in edges {
        let ev = |r: &str| -> Event {
            let parts: Vec<&str> = r.split('.').collect();
            let [decl, ks, point] = parts[..] else { panic!("plan: bad edge end {r:?}") };
            let (kind, spine) = kind_spine(ks);
            let decl = decl_of(decl);
            let i = exp
                .owed
                .iter()
                .position(|o| o.decl == decl && o.kind == kind && o.spine == spine)
                .unwrap_or_else(|| panic!("plan: edge names {r:?}, which no line owes"));
            Event {
                obligation: ObligationId(i as u32),
                point: Point::from_name(point).unwrap_or_else(|| panic!("plan: bad point {point}")),
            }
        };
        exp.edges.push((ev(a), ev(b)));
    }
    exp
}
