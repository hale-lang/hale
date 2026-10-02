//! The lifecycle trace and its oracle (F.40 phase 3, L2).
//!
//! A build made with `HALE_LIFECYCLE_TRACE=1` (`BuildOptions::
//! lifecycle_trace`) writes one line per obligation event on stderr
//! (spec/runtime.md § The lifecycle trace):
//!
//! ```text
//! lc <seq> <Kind> <Point> spine=<Spine> dom=<domain> type=<T> inst=<n> inc=<n>
//! ```
//!
//! [`parse`] reads those lines into [`TraceEvent`]s, ordered by `seq`,
//! and keeps the rest of stderr apart. The runtime mints the subject
//! (instance and incarnation); [`RuntimeSubject::observed`] is how the
//! parser holds one, and nothing else does.
//!
//! [`Expected`] is what one run owes, as the oracle checks it: the
//! obligations ([`Owed`], indexed by [`ObligationId`]), the per-subject
//! sequences that must hold within one domain, and the edges between
//! [`Event`]s that must hold across domains. [`Expected::check`]
//! evaluates a trace against it and [`laws`] checks what every trace
//! owes whatever the plan: an end has an entry, an entry has an end, a
//! subject's reclaim happens once and after its birth.
//!
//! What a trace can and cannot show. The runtime numbers events with
//! one relaxed counter, so on one thread `seq` order is program order,
//! and if event a happens before event b then `seq(a) < seq(b)`. A
//! `seq` order that contradicts a required edge is therefore a real
//! violation; a `seq` order that agrees with one is evidence of this
//! execution, not a proof of the edge. Progress (a join that returns, a
//! wakeup that is not lost) stays with the deadline and matrix oracles.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::{Event, Multiplicity, ObligationId, ObligationKind, Point, RuntimeSubject, Spine};

/// Every trace line starts with this.
pub const LINE_PREFIX: &str = "lc ";

/// One line of the trace.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceEvent {
    /// The runtime's sequence number; the parsed trace is in this order.
    pub seq: u64,
    pub kind: ObligationKind,
    /// `Entered`, `Completed` or `Terminal(_)`; never `Ended`.
    pub point: Point,
    /// `None` where the emitting site cannot name the spine (a failure
    /// raised in place, a restart).
    pub spine: Option<Spine>,
    /// The thread: `main`, `pool:<name>`, `pinned:<n>`, `thread:<n>`.
    pub domain: String,
    /// The declaration's lowered name; `None` for a process-level
    /// obligation (the pool join, a wait-abort, a pre-drain).
    pub decl: Option<String>,
    /// `None` for a process-level obligation.
    pub subject: Option<RuntimeSubject>,
}

impl TraceEvent {
    /// The event of the plan's obligation `obligation` this line is.
    pub fn event(&self, obligation: ObligationId) -> Event {
        Event { obligation, point: self.point }
    }
}

/// A parsed trace: the events by `seq`, and stderr without them.
#[derive(Debug, Clone, Default)]
pub struct Trace {
    pub events: Vec<TraceEvent>,
    pub rest: String,
}

/// One line: `None` when it is not a trace line.
pub fn parse_line(line: &str) -> Option<Result<TraceEvent, String>> {
    let body = line.strip_prefix(LINE_PREFIX)?;
    Some(parse_body(body).map_err(|e| format!("{e}: {line:?}")))
}

fn parse_body(body: &str) -> Result<TraceEvent, String> {
    let mut words = body.split(' ');
    let mut next = |what: &str| words.next().ok_or_else(|| format!("no {what}"));
    let seq = next("seq")?.parse::<u64>().map_err(|e| format!("seq: {e}"))?;
    let kind_s = next("kind")?;
    let kind = ObligationKind::from_name(kind_s).ok_or_else(|| format!("unknown kind {kind_s}"))?;
    let point_s = next("point")?;
    let point = Point::from_name(point_s).ok_or_else(|| format!("unknown point {point_s}"))?;
    let mut field = |key: &str| -> Result<String, String> {
        let w = next(key)?;
        w.strip_prefix(&format!("{key}="))
            .map(str::to_string)
            .ok_or_else(|| format!("expected {key}=, got {w}"))
    };
    let spine_s = field("spine")?;
    let spine = match spine_s.as_str() {
        "-" => None,
        s => Some(Spine::from_name(s).ok_or_else(|| format!("unknown spine {s}"))?),
    };
    let domain = field("dom")?;
    let ty = field("type")?;
    let inst = field("inst")?;
    let inc = field("inc")?;
    let (decl, subject) = if ty == "-" {
        (None, None)
    } else {
        let instance = inst.parse::<u64>().map_err(|e| format!("inst: {e}"))?;
        let incarnation = inc.parse::<u32>().map_err(|e| format!("inc: {e}"))?;
        (Some(ty), Some(RuntimeSubject::observed(instance, incarnation)))
    };
    Ok(TraceEvent { seq, kind, point, spine, domain, decl, subject })
}

/// Split a process's stderr into its trace and the rest.
pub fn parse(stderr: &str) -> Result<Trace, String> {
    let mut trace = Trace::default();
    for line in stderr.lines() {
        match parse_line(line) {
            Some(ev) => trace.events.push(ev?),
            None => {
                trace.rest.push_str(line);
                trace.rest.push('\n');
            }
        }
    }
    trace.events.sort_by_key(|e| e.seq);
    Ok(trace)
}

// ------------------------------------------------------------ the oracle

/// How many subjects owe an obligation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Count {
    Exactly(usize),
    AtLeast(usize),
}

impl Count {
    fn holds(self, n: usize) -> bool {
        match self {
            Count::Exactly(k) => n == k,
            Count::AtLeast(k) => n >= k,
        }
    }
}

impl fmt::Display for Count {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Count::Exactly(k) => write!(f, "{k}"),
            Count::AtLeast(k) => write!(f, "at least {k}"),
        }
    }
}

/// One obligation a run owes, matched to trace events by declaration,
/// kind and (when named) spine.
#[derive(Debug, Clone, PartialEq)]
pub struct Owed {
    /// The declaration's lowered name; `None` for a process-level one.
    pub decl: Option<String>,
    pub kind: ObligationKind,
    /// Only this spine's events; `None` for any.
    pub spine: Option<Spine>,
    /// [`Multiplicity::OncePerIncarnation`] counts incarnations; any
    /// other counts instances (a process-level obligation counts its
    /// entries).
    pub multiplicity: Multiplicity,
    pub count: Count,
    /// The end it reaches: `Completed`, or a named terminal.
    pub ends: Point,
    /// A domain claim: every event of it ran on a thread whose label
    /// starts with this (`main`, `pool:side`, `pinned`).
    pub domain: Option<String>,
}

impl Owed {
    /// `Decl.Kind`, `Decl.Kind@Spine`, `-.Kind@Spine`.
    pub fn label(&self) -> String {
        let decl = self.decl.as_deref().unwrap_or("-");
        match self.spine {
            Some(s) => format!("{decl}.{}@{}", self.kind.name(), s.name()),
            None => format!("{decl}.{}", self.kind.name()),
        }
    }

    fn matches(&self, e: &TraceEvent) -> bool {
        e.kind == self.kind && e.decl == self.decl && (self.spine.is_none() || e.spine == self.spine)
    }
}

/// What one run owes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Expected {
    pub owed: Vec<Owed>,
    /// Per-subject program order: within one domain, each obligation's
    /// entry comes after the one before it in the list. The subject is
    /// the incarnation when both neighbours are owed per incarnation,
    /// otherwise the instance.
    pub sequences: Vec<Vec<ObligationId>>,
    /// `(a, b)`: event a happens before event b. For two obligations of
    /// one declaration, of the same subject (the incarnation when both
    /// are owed per incarnation, otherwise the instance); otherwise every
    /// subject owing a reaches it before any b.
    pub edges: Vec<(Event, Event)>,
}

/// One way a trace fails what it owes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// No event of an obligation the run owes.
    Missing { owed: String },
    Count { owed: String, want: String, got: usize },
    /// Entered and never ended, in a run that ended normally.
    Unended { owed: String, subject: String },
    WrongEnd { owed: String, subject: String, got: String },
    Duplicate { owed: String, subject: String, point: String },
    Domain { owed: String, subject: String, ran_on: String, claimed: String },
    /// Two obligations of one subject out of order on one domain.
    Order { first: String, then: String, subject: String },
    /// `after` happened with `before` not yet reached.
    Edge { before: String, after: String, subject: String },
    /// A law every trace owes.
    Law { what: String },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Violation::Missing { owed } => write!(f, "missing: {owed}"),
            Violation::Count { owed, want, got } => write!(f, "count: {owed} has {got} subjects, owes {want}"),
            Violation::Unended { owed, subject } => write!(f, "unended: {owed} {subject}"),
            Violation::WrongEnd { owed, subject, got } => write!(f, "end: {owed} {subject} ended {got}"),
            Violation::Duplicate { owed, subject, point } => write!(f, "duplicate: {owed} {subject} {point} twice"),
            Violation::Domain { owed, subject, ran_on, claimed } => {
                write!(f, "domain: {owed} {subject} ran on {ran_on}, claimed {claimed}")
            }
            Violation::Order { first, then, subject } => write!(f, "order: {then} before {first} {subject}"),
            Violation::Edge { before, after, subject } => write!(f, "edge: {after} {subject} with {before} not reached"),
            Violation::Law { what } => write!(f, "law: {what}"),
        }
    }
}

fn subject_label(s: Option<RuntimeSubject>) -> String {
    match s {
        Some(s) => format!("(inst {} inc {})", s.instance.raw(), s.incarnation.raw()),
        None => "(process)".to_string(),
    }
}

/// The occurrences of one obligation: per subject (instance or
/// incarnation), or per entry for a process-level one.
struct Group<'t> {
    subject: Option<RuntimeSubject>,
    entered: Vec<&'t TraceEvent>,
    ends: Vec<&'t TraceEvent>,
}

fn groups<'t>(owed: &Owed, trace: &'t Trace) -> Vec<Group<'t>> {
    let events: Vec<&TraceEvent> = trace.events.iter().filter(|e| owed.matches(e)).collect();
    if owed.decl.is_none() {
        // Process-level: each entry is an occurrence, its end the
        // next end on the same domain (nested ones close inner first).
        let mut out: Vec<Group> = Vec::new();
        let mut open: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for e in events {
            if e.point == Point::Entered {
                open.entry(e.domain.as_str()).or_default().push(out.len());
                out.push(Group { subject: None, entered: vec![e], ends: vec![] });
            } else if let Some(i) = open.get_mut(e.domain.as_str()).and_then(|v| v.pop()) {
                out[i].ends.push(e);
            } else {
                out.push(Group { subject: None, entered: vec![], ends: vec![e] });
            }
        }
        return out;
    }
    let mut by: BTreeMap<(u64, u32), Group> = BTreeMap::new();
    for e in events {
        let s = e.subject.expect("a declared obligation's event carries its subject");
        let inc = if owed.multiplicity == Multiplicity::OncePerIncarnation { s.incarnation.raw() } else { 0 };
        let g = by.entry((s.instance.raw(), inc)).or_insert(Group { subject: Some(s), entered: vec![], ends: vec![] });
        if e.point == Point::Entered {
            g.entered.push(e);
        } else {
            g.ends.push(e);
        }
    }
    by.into_values().collect()
}

impl Expected {
    /// Evaluate `trace`. `complete` says the process ended normally, so
    /// an obligation entered and never ended is a violation rather than
    /// the cut-off of a killed run.
    pub fn check(&self, trace: &Trace, complete: bool) -> Vec<Violation> {
        let mut out = Vec::new();
        let all: Vec<Vec<Group>> = self.owed.iter().map(|o| groups(o, trace)).collect();
        for (owed, gs) in self.owed.iter().zip(&all) {
            let label = owed.label();
            if gs.is_empty() {
                if owed.count != Count::Exactly(0) {
                    out.push(Violation::Missing { owed: label });
                }
                continue;
            }
            if !owed.count.holds(gs.len()) {
                out.push(Violation::Count { owed: label.clone(), want: owed.count.to_string(), got: gs.len() });
            }
            for g in gs {
                let subject = subject_label(g.subject);
                if g.entered.len() > 1 {
                    out.push(Violation::Duplicate { owed: label.clone(), subject: subject.clone(), point: "Entered".into() });
                }
                if g.ends.len() > 1 {
                    out.push(Violation::Duplicate { owed: label.clone(), subject: subject.clone(), point: "end".into() });
                }
                match g.ends.first() {
                    None if complete && !g.entered.is_empty() => {
                        out.push(Violation::Unended { owed: label.clone(), subject: subject.clone() })
                    }
                    Some(end) if !end.point.satisfies(owed.ends) => out.push(Violation::WrongEnd {
                        owed: label.clone(),
                        subject: subject.clone(),
                        got: end.point.name(),
                    }),
                    _ => {}
                }
                if let Some(claim) = &owed.domain {
                    if let Some(e) = g.entered.iter().chain(&g.ends).find(|e| !e.domain.starts_with(claim.as_str())) {
                        out.push(Violation::Domain {
                            owed: label.clone(),
                            subject: subject.clone(),
                            ran_on: e.domain.clone(),
                            claimed: claim.clone(),
                        });
                    }
                }
            }
        }
        // Within one domain, per subject: each obligation's first entry
        // after the first entry of the one before it. Two obligations
        // owed per incarnation are matched incarnation by incarnation;
        // otherwise per instance.
        let first_entries = |gs: &[Group<'_>], per_incarnation: bool| {
            let mut m: BTreeMap<SubjectKey, (u64, String, Option<RuntimeSubject>)> = BTreeMap::new();
            for g in gs {
                let Some(e) = g.entered.first() else { continue };
                let key = subject_key(g.subject, per_incarnation);
                if m.get(&key).is_none_or(|(seq, _, _)| e.seq < *seq) {
                    m.insert(key, (e.seq, e.domain.clone(), g.subject));
                }
            }
            m
        };
        for seq in &self.sequences {
            for pair in seq.windows(2) {
                let (a, b) = (pair[0].0 as usize, pair[1].0 as usize);
                let per_incarnation = self.per_incarnation(a, b);
                let firsts_a = first_entries(&all[a], per_incarnation);
                for (key, (seq_b, dom_b, subject)) in first_entries(&all[b], per_incarnation) {
                    if let Some((seq_a, dom_a, _)) = firsts_a.get(&key) {
                        if *dom_a == dom_b && *seq_a > seq_b {
                            out.push(Violation::Order {
                                first: self.owed[a].label(),
                                then: self.owed[b].label(),
                                subject: subject_label(subject),
                            });
                        }
                    }
                }
            }
        }
        for (before, after) in &self.edges {
            let (a, b) = (before.obligation.0 as usize, after.obligation.0 as usize);
            let (oa, ob) = (&self.owed[a], &self.owed[b]);
            let a_label = format!("{}.{}", oa.label(), before.point.name());
            let b_label = format!("{}.{}", ob.label(), after.point.name());
            let same_decl = oa.decl.is_some() && oa.decl == ob.decl;
            let per_incarnation = self.per_incarnation(a, b);
            for gb in &all[b] {
                let Some(eb) = gb.entered.iter().chain(&gb.ends).find(|e| e.point.satisfies(after.point)) else {
                    continue;
                };
                let reached = |ga: &Group| {
                    ga.entered.iter().chain(&ga.ends).any(|e| e.point.satisfies(before.point) && e.seq < eb.seq)
                };
                let ok = if same_decl {
                    let key = subject_key(gb.subject, per_incarnation);
                    all[a].iter().filter(|ga| subject_key(ga.subject, per_incarnation) == key).any(|ga| reached(ga))
                } else {
                    !all[a].is_empty() && all[a].iter().all(|ga| reached(ga))
                };
                if !ok {
                    out.push(Violation::Edge { before: a_label.clone(), after: b_label.clone(), subject: subject_label(gb.subject) });
                }
            }
        }
        out
    }

    /// Whether obligations `a` and `b` are both owed per incarnation, so
    /// a relation between them holds within each incarnation rather than
    /// across an instance's.
    fn per_incarnation(&self, a: usize, b: usize) -> bool {
        [a, b].iter().all(|&i| self.owed[i].multiplicity == Multiplicity::OncePerIncarnation)
    }
}

/// Which subjects a relation between two obligations matches: the
/// instance, and the incarnation too when both are owed per
/// incarnation; `None` for a process-level one.
type SubjectKey = Option<(u64, Option<u32>)>;

fn subject_key(s: Option<RuntimeSubject>, per_incarnation: bool) -> SubjectKey {
    s.map(|s| (s.instance.raw(), per_incarnation.then(|| s.incarnation.raw())))
}

/// What every trace owes, whatever the plan: an end has an entry; a
/// declared obligation is not re-entered while open; in a run that
/// ended normally, every entry has an end; a subject is reclaimed at
/// most once, and only after it was born (a second teardown of a
/// reclaimed struct shows as a reclaim of a subject never born, since
/// the runtime retires a number at its reclaim); birth happens once
/// per incarnation; and no other step is entered on an instance nothing
/// built, which is how a step on a reclaimed struct shows (a pinned join
/// may come first: the frame that instantiated the locus enters it
/// while the pinned thread has not yet begun the birth).
pub fn laws(trace: &Trace, complete: bool) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut open: BTreeMap<(ObligationKind, u64, u32), u32> = BTreeMap::new();
    let mut open_process: BTreeMap<(ObligationKind, &str), u32> = BTreeMap::new();
    let mut born: BTreeMap<u64, u32> = BTreeMap::new();
    let mut births: BTreeMap<(u64, u32), u32> = BTreeMap::new();
    let mut reclaims: BTreeMap<u64, u32> = BTreeMap::new();
    let mut built: BTreeSet<u64> = BTreeSet::new();
    for e in &trace.events {
        let what = |w: &str| Violation::Law {
            what: format!("{w}: {} {} {} on {}", e.decl.as_deref().unwrap_or("-"), e.kind.name(), subject_label(e.subject), e.domain),
        };
        match e.subject {
            Some(s) => {
                let inst = s.instance.raw();
                match e.kind {
                    ObligationKind::ParamsSettle
                    | ObligationKind::Accept
                    | ObligationKind::Birth
                    | ObligationKind::PinnedJoin => {
                        built.insert(inst);
                    }
                    // A reclaim of a number nothing built is the law below.
                    ObligationKind::Reclaim => {
                        built.insert(inst);
                    }
                    _ => {
                        if e.point == Point::Entered && built.insert(inst) {
                            out.push(Violation::Law {
                                what: format!(
                                    "{} of a subject never built: {} (inst {inst}) on {} (a step on a reclaimed struct?)",
                                    e.kind.name(),
                                    e.decl.as_deref().unwrap_or("-"),
                                    e.domain
                                ),
                            });
                        }
                    }
                }
                let key = (e.kind, s.instance.raw(), s.incarnation.raw());
                let n = open.entry(key).or_insert(0);
                if e.point == Point::Entered {
                    if *n > 0 {
                        out.push(what("re-entered while open"));
                    }
                    *n += 1;
                    match e.kind {
                        ObligationKind::Birth => {
                            *born.entry(s.instance.raw()).or_insert(0) += 1;
                            let b = births.entry((s.instance.raw(), s.incarnation.raw())).or_insert(0);
                            *b += 1;
                            if *b > 1 {
                                out.push(what("born twice in one incarnation"));
                            }
                        }
                        ObligationKind::ParamsSettle | ObligationKind::Accept => {
                            born.entry(s.instance.raw()).or_insert(0);
                        }
                        ObligationKind::Reclaim => {
                            if !born.contains_key(&s.instance.raw()) {
                                out.push(what("reclaimed, never born (a second teardown of a reclaimed struct?)"));
                            }
                            let r = reclaims.entry(s.instance.raw()).or_insert(0);
                            *r += 1;
                            if *r > 1 {
                                out.push(what("reclaimed twice"));
                            }
                        }
                        _ => {}
                    }
                } else if *n == 0 {
                    out.push(what("ended, never entered"));
                } else {
                    *n -= 1;
                }
            }
            None => {
                let n = open_process.entry((e.kind, e.domain.as_str())).or_insert(0);
                if e.point == Point::Entered {
                    *n += 1;
                } else if *n == 0 {
                    out.push(what("ended, never entered"));
                } else {
                    *n -= 1;
                }
            }
        }
    }
    if complete {
        for ((kind, inst, inc), n) in open {
            if n > 0 {
                out.push(Violation::Law { what: format!("never ended: {} (inst {inst} inc {inc})", kind.name()) });
            }
        }
        for ((kind, dom), n) in open_process {
            if n > 0 {
                out.push(Violation::Law { what: format!("never ended: {} on {dom}", kind.name()) });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace(lines: &[&str]) -> Trace {
        parse(&lines.join("\n")).expect("parses")
    }

    fn owed(decl: Option<&str>, kind: ObligationKind, spine: Option<Spine>) -> Owed {
        Owed {
            decl: decl.map(str::to_string),
            kind,
            spine,
            multiplicity: Multiplicity::OncePerInstance,
            count: Count::Exactly(1),
            ends: Point::Completed,
            domain: None,
        }
    }

    fn ev(i: u32, point: Point) -> Event {
        Event { obligation: ObligationId(i), point }
    }

    #[test]
    fn a_line_round_trips_its_fields_and_other_lines_stay_apart() {
        let t = trace(&[
            "lc 2 Run Terminal(CanceledAfterStart) spine=PoolRun dom=pool:io type=L inst=4 inc=1",
            "ClosureViolation: something",
            "lc 1 PoolJoin Entered spine=EagerTeardown dom=main type=- inst=- inc=-",
        ]);
        assert_eq!(t.events.len(), 2);
        assert_eq!(t.events[0].kind, ObligationKind::PoolJoin);
        assert_eq!(t.events[0].subject, None);
        let run = &t.events[1];
        assert_eq!(run.point, Point::Terminal(super::super::Terminal::CanceledAfterStart));
        assert_eq!(run.subject, Some(RuntimeSubject::observed(4, 1)));
        assert_eq!(run.domain, "pool:io");
        assert_eq!(run.event(ObligationId(3)), Event { obligation: ObligationId(3), point: run.point });
        assert_eq!(t.rest, "ClosureViolation: something\n");
        assert!(parse("lc 1 Nope Entered spine=- dom=main type=- inst=- inc=-").is_err());
    }

    /// The edge holds on the trace's order and fails on the reverse,
    /// and a missing step is a missing obligation.
    #[test]
    fn an_edge_discriminates_order_and_a_missing_step_is_reported() {
        let exp = Expected {
            owed: vec![
                owed(None, ObligationKind::WaitAbort, Some(Spine::EagerTeardown)),
                owed(None, ObligationKind::PoolJoin, Some(Spine::EagerTeardown)),
            ],
            sequences: vec![],
            edges: vec![(ev(0, Point::Completed), ev(1, Point::Entered))],
        };
        let good = trace(&[
            "lc 1 WaitAbort Entered spine=EagerTeardown dom=main type=- inst=- inc=-",
            "lc 2 WaitAbort Completed spine=EagerTeardown dom=main type=- inst=- inc=-",
            "lc 3 PoolJoin Entered spine=EagerTeardown dom=main type=- inst=- inc=-",
            "lc 4 PoolJoin Completed spine=EagerTeardown dom=main type=- inst=- inc=-",
        ]);
        assert_eq!(exp.check(&good, true), vec![]);
        let host = trace(&[
            "lc 1 PoolJoin Entered spine=EagerTeardown dom=main type=- inst=- inc=-",
            "lc 2 PoolJoin Completed spine=EagerTeardown dom=main type=- inst=- inc=-",
            "lc 3 WaitAbort Entered spine=EagerTeardown dom=main type=- inst=- inc=-",
            "lc 4 WaitAbort Completed spine=EagerTeardown dom=main type=- inst=- inc=-",
        ]);
        let v = exp.check(&host, true);
        assert!(matches!(v.as_slice(), [Violation::Edge { .. }]), "{v:?}");
        let hung = trace(&["lc 1 PoolJoin Entered spine=EagerTeardown dom=main type=- inst=- inc=-"]);
        let v: Vec<String> = exp.check(&hung, false).iter().map(|v| v.to_string()).collect();
        assert_eq!(
            v,
            [
                "missing: -.WaitAbort@EagerTeardown",
                "edge: -.PoolJoin@EagerTeardown.Entered (process) with -.WaitAbort@EagerTeardown.Completed not reached"
            ]
        );
    }

    /// `K`'s Birth and Run, owed per incarnation over two incarnations.
    fn birth_run(sequences: Vec<Vec<ObligationId>>, edges: Vec<(Event, Event)>) -> Expected {
        let per_incarnation = |kind| Owed {
            multiplicity: Multiplicity::OncePerIncarnation,
            count: Count::Exactly(2),
            ..owed(Some("K"), kind, None)
        };
        Expected { owed: vec![per_incarnation(ObligationKind::Birth), per_incarnation(ObligationKind::Run)], sequences, edges }
    }

    /// Incarnation 0 born then run; incarnation 1 restarted and run, its
    /// birth `late` after its run or not.
    fn restarted(late: bool) -> Trace {
        let birth = ["Birth Entered", "Birth Completed"];
        let mut inc1 = vec!["Restart Entered", "Restart Completed", "Run Entered", "Run Completed"];
        if late {
            inc1.extend(birth);
        } else {
            inc1.splice(2..2, birth);
        }
        let lines: Vec<String> = ["Birth Entered", "Birth Completed", "Run Entered", "Run Completed"]
            .iter()
            .map(|e| (e, 0))
            .chain(inc1.iter().map(|e| (e, 1)))
            .enumerate()
            .map(|(i, (e, inc))| format!("lc {} {e} spine=- dom=main type=K inst=1 inc={inc}", i + 1))
            .collect();
        parse(&lines.join("\n")).expect("parses")
    }

    /// A restarted incarnation's run before its own birth breaks the
    /// sequence, though incarnation 0 was born before any run.
    #[test]
    fn a_sequence_holds_within_each_incarnation() {
        let exp = birth_run(vec![vec![ObligationId(0), ObligationId(1)]], vec![]);
        assert_eq!(exp.check(&restarted(false), true), vec![]);
        let v: Vec<String> = exp.check(&restarted(true), true).iter().map(|v| v.to_string()).collect();
        assert_eq!(v, ["order: K.Run before K.Birth (inst 1 inc 1)"]);
        assert_eq!(laws(&restarted(true), true), vec![]);
    }

    /// The same, held by an edge alone.
    #[test]
    fn an_edge_between_per_incarnation_obligations_holds_within_each_incarnation() {
        let exp = birth_run(vec![], vec![(ev(0, Point::Completed), ev(1, Point::Entered))]);
        assert_eq!(exp.check(&restarted(false), true), vec![]);
        let v: Vec<String> = exp.check(&restarted(true), true).iter().map(|v| v.to_string()).collect();
        assert_eq!(v, ["edge: K.Run.Entered (inst 1 inc 1) with K.Birth.Completed not reached"]);
    }

    /// An obligation owed once per instance (params settlement) stays
    /// related to every incarnation of the instance: one settlement
    /// before incarnation 0 holds for the restarted birth too.
    #[test]
    fn a_per_instance_obligation_relates_to_every_incarnation() {
        let mut exp = birth_run(vec![], vec![]);
        exp.owed.truncate(1);
        exp.owed.insert(0, owed(Some("K"), ObligationKind::ParamsSettle, None));
        exp.sequences = vec![vec![ObligationId(0), ObligationId(1)]];
        exp.edges = vec![(ev(0, Point::Completed), ev(1, Point::Entered))];
        let settled = |first: bool| {
            let settle = ["ParamsSettle Entered 0", "ParamsSettle Completed 0"];
            let births = ["Birth Entered 0", "Birth Completed 0", "Birth Entered 1", "Birth Completed 1"];
            let order: Vec<&str> = if first {
                settle.iter().chain(&births).copied().collect()
            } else {
                births[..2].iter().chain(&settle).chain(&births[2..]).copied().collect()
            };
            let lines: Vec<String> = order
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let (e, inc) = e.rsplit_once(' ').unwrap();
                    format!("lc {} {e} spine=- dom=main type=K inst=1 inc={inc}", i + 1)
                })
                .collect();
            parse(&lines.join("\n")).expect("parses")
        };
        assert_eq!(exp.check(&settled(true), true), vec![]);
        let v: Vec<String> = exp.check(&settled(false), true).iter().map(|v| v.to_string()).collect();
        assert_eq!(
            v,
            [
                "order: K.Birth before K.ParamsSettle (inst 1 inc 0)",
                "edge: K.Birth.Entered (inst 1 inc 0) with K.ParamsSettle.Completed not reached"
            ]
        );
    }

    #[test]
    fn a_second_reclaim_and_an_unended_entry_break_the_laws() {
        let t = trace(&[
            "lc 1 Birth Entered spine=Instantiation dom=main type=K inst=1 inc=0",
            "lc 2 Birth Completed spine=Instantiation dom=main type=K inst=1 inc=0",
            "lc 3 Reclaim Entered spine=Reclaim dom=main type=K inst=1 inc=0",
            "lc 4 Reclaim Completed spine=Reclaim dom=main type=K inst=1 inc=0",
            "lc 5 Reclaim Entered spine=Cascade dom=main type=K inst=2 inc=0",
            "lc 6 Run Entered spine=PoolRun dom=pool:a type=K inst=1 inc=0",
        ]);
        let v: Vec<String> = laws(&t, true).iter().map(|v| v.to_string()).collect();
        assert!(v.iter().any(|l| l.contains("reclaimed, never born")), "{v:?}");
        assert!(v.iter().any(|l| l.contains("never ended: Run")), "{v:?}");
        assert!(laws(&t, false).iter().all(|l| !l.to_string().contains("never ended")));
    }

    /// A step on a struct whose number the runtime retired at its
    /// reclaim is a step on a number nothing built; a pinned join may
    /// precede the birth it waits for.
    #[test]
    fn a_step_on_a_number_nothing_built_breaks_the_laws() {
        let t = trace(&[
            "lc 1 PinnedJoin Entered spine=DeferredEntry dom=main type=P inst=1 inc=0",
            "lc 2 Birth Entered spine=PinnedMain dom=pinned:1 type=P inst=1 inc=0",
            "lc 3 Birth Completed spine=PinnedMain dom=pinned:1 type=P inst=1 inc=0",
            "lc 4 Run Entered spine=PoolRun dom=pool:side type=K inst=2 inc=0",
            "lc 5 Run Completed spine=PoolRun dom=pool:side type=K inst=2 inc=0",
        ]);
        let v: Vec<String> = laws(&t, false).iter().map(|v| v.to_string()).collect();
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].contains("Run of a subject never built"), "{v:?}");
    }
}
