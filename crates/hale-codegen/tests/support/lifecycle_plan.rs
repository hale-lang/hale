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
//! The plans the fixtures and the matrix hold their runs to are the
//! producer's (`hale_types::lifecycle::derive`), and [`render`] writes
//! one in this notation; the fixtures' negative controls, and the
//! fixtures the producer does not derive yet, write theirs by hand and
//! [`plan`] reads them. The fixtures parse and the matrix
//! renders, so each file leaves the other half dead.
#![allow(dead_code)]

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

/// A plan in the notation [`plan`] reads, one line per sequence, then
/// the edges: what a plan the producer derived says, written the way a
/// hand-written one is.
pub fn render(exp: &Expected) -> String {
    let step = |o: &Owed| -> String {
        let mut s = o.kind.name().to_string();
        if let Some(sp) = o.spine {
            s.push('@');
            s.push_str(sp.name());
        }
        match o.count {
            Count::Exactly(1) => {}
            Count::Exactly(n) => s.push_str(&format!("*{n}")),
            Count::AtLeast(1) => s.push_str("*+"),
            Count::AtLeast(n) => s.push_str(&format!("*>={n}")),
        }
        if o.ends != Point::Completed {
            if let Point::Terminal(t) = o.ends {
                s.push('=');
                s.push_str(&t.name());
            }
        }
        if let Some(d) = &o.domain {
            s.push('!');
            s.push_str(d);
        }
        s
    };
    let label = |e: &Event| -> String {
        let o = &exp.owed[e.obligation.0 as usize];
        let ks = match o.spine {
            Some(sp) => format!("{}@{}", o.kind.name(), sp.name()),
            None => o.kind.name().to_string(),
        };
        format!("{}.{ks}.{}", o.decl.as_deref().unwrap_or("-"), e.point.name())
    };
    let mut out = Vec::new();
    for seq in &exp.sequences {
        let Some(first) = seq.first() else { continue };
        let decl = exp.owed[first.0 as usize].decl.as_deref().unwrap_or("-");
        let steps: Vec<String> = seq.iter().map(|i| step(&exp.owed[i.0 as usize])).collect();
        out.push(format!("{decl}: {}", steps.join(" ")));
    }
    for (a, b) in &exp.edges {
        out.push(format!("edge {} -> {}", label(a), label(b)));
    }
    out.join("\n")
}

/// A violation with its instance numbers written `_`: `(inst 3 inc 0)`
/// is `(inst _ inc 0)`. The runtime mints the number, so a known-open
/// departure is written, and matched, in this form.
pub fn normalized(violation: &str) -> String {
    let mut out = String::new();
    let mut rest = violation;
    while let Some(i) = rest.find("(inst ") {
        let (head, tail) = rest.split_at(i + "(inst ".len());
        out.push_str(head);
        let digits = tail.len() - tail.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        out.push_str(if digits > 0 { "_" } else { "" });
        rest = &tail[digits..];
    }
    out.push_str(rest);
    out
}
