//! The shadow facility (F.40 phase 0, step 0.5).
//!
//! A migration moves a semantic family onto one authoritative
//! producer. Before a consumer switches, the new derivation runs
//! **beside** the old one over a corpus and every disagreement is
//! reported as a divergence a contributor can act on: the family,
//! the program, the key the two rows share, the old and new facts,
//! the witnesses that decided them, and the slice that depends on
//! the row. That is F.39's A2 step ("computed and checked in shadow
//! mode") made reusable, and it is how every phase-1 move is
//! verified.
//!
//! Three rules from the final direction shape the API:
//!
//! - **The current compiler is a compatibility reference, not a
//!   correctness oracle.** A divergence is classified, not just
//!   detected: a migration regression, a known old bug, an
//!   intentional correction, or an unresolved spec disagreement.
//!   The classified list is a pinned fixture beside the test; the
//!   exit gate for a migration is *no unexplained divergences*, which
//!   means no `Unclassified` and no `Regression` entries remain.
//! - **Compare through an explicit correspondence, never raw id
//!   equality.** [`Report::compare_rows`] takes each producer's rows
//!   under their own keys with a mapping onto one shared key. A row
//!   the mapping has no key for is [`Kind::Unmapped`], reported
//!   rather than given an invented key; a mapping that sends two
//!   rows of one side to one key is a [`Kind::Collision`] listing
//!   every native key, rather than silently dropping one (the #1210
//!   failure mode); a key on one side only is [`Kind::OnlyOld`] or
//!   [`Kind::OnlyNew`]. The fixture records the kind, so a
//!   disagreement that turns into a collision is unexplained again,
//!   and a collision's facts are every colliding row on each side, so
//!   a row of it that changes, joins or leaves is unexplained again too.
//! - **A program is named by its content.** The corpus's `file.rs#N`
//!   ordinal moves when a literal is added above it; a shadow keys
//!   its fixture by [`program_id`], the origin path plus a digest of
//!   the source, so the fixture does not churn with the corpus.
//!
//! The facility knows no family: rows are strings by the time they
//! arrive here, and the correspondence is the caller's. What it owns
//! is the report, the fixture format and the gate.

use std::collections::BTreeMap;
use std::fmt::Display;

/// What shape a disagreement has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Both producers have a row for the key and the facts differ.
    Disagreement,
    /// Only the old producer has a row.
    OnlyOld,
    /// Only the new producer has a row.
    OnlyNew,
    /// The correspondence sent two rows of one side to one key: the
    /// mapping is not injective, so the comparison could not be
    /// made.
    Collision,
    /// The correspondence has no key for a row: the row has no
    /// counterpart in the other producer's space, and saying so is
    /// the point.
    Unmapped,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Disagreement => "disagreement",
            Kind::OnlyOld => "only-old",
            Kind::OnlyNew => "only-new",
            Kind::Collision => "collision",
            Kind::Unmapped => "unmapped",
        }
    }
    fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "disagreement" => Kind::Disagreement,
            "only-old" => Kind::OnlyOld,
            "only-new" => Kind::OnlyNew,
            "collision" => Kind::Collision,
            "unmapped" => Kind::Unmapped,
            _ => return None,
        })
    }
}

/// One disagreement between the old and the new producer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    pub family: String,
    pub kind: Kind,
    /// The program, as [`program_id`] names it.
    pub program: String,
    /// The correspondence key both producers were mapped onto.
    pub key: String,
    /// The old producer's fact, or `None` when it has no row. For a
    /// collision, every old row the key collected, as
    /// [`collision_facts`] renders them, so the fixture pins all of
    /// them and not only the first.
    pub old: Option<String>,
    /// The new producer's fact, or `None` when it has no row; for a
    /// collision, every new row the key collected, as for `old`.
    pub new: Option<String>,
    /// `old` as the report prints it: the fact itself, or for a
    /// collision its rows as `native=value; …`. For the reader only;
    /// `old` is what a classification is held to.
    pub old_shown: Option<String>,
    /// `new` as the report prints it, as for `old_shown`.
    pub new_shown: Option<String>,
    /// What decided the two rows, rendered for the fixer.
    pub witnesses: Vec<String>,
    /// The smallest slice that depends on the row: the consumers
    /// whose answer changes if the row does.
    pub slice: Vec<String>,
    /// The producers' own keys behind the shared key, rendered: for a
    /// collision, every native key that mapped onto it, with its
    /// value; for an unmapped row, the one row. Empty for an ordinary
    /// disagreement.
    pub natives: Vec<String>,
}

/// What a divergence turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Not yet looked at. Fails the gate.
    Unclassified,
    /// The new producer is wrong. Fails the gate.
    Regression,
    /// The old producer was wrong and the new one is right, or the
    /// two old producers disagree and the code says which is wrong;
    /// the note names the bug.
    KnownOldBug,
    /// A named semantic decision changed the answer on purpose; the
    /// note names the decision and its regression test.
    Correction,
    /// The spec and the implementation disagree and nobody has
    /// decided yet; the note says where. Passes the gate only so
    /// the disagreement is recorded, and the migration's PR body
    /// carries the open decision.
    SpecDisagreement,
}

impl Class {
    fn label(self) -> &'static str {
        match self {
            Class::Unclassified => "unclassified",
            Class::Regression => "regression",
            Class::KnownOldBug => "known-old-bug",
            Class::Correction => "correction",
            Class::SpecDisagreement => "spec-disagreement",
        }
    }
    fn parse(s: &str) -> Option<Class> {
        Some(match s {
            "unclassified" => Class::Unclassified,
            "regression" => Class::Regression,
            "known-old-bug" => Class::KnownOldBug,
            "correction" => Class::Correction,
            "spec-disagreement" => Class::SpecDisagreement,
            _ => return None,
        })
    }
}

/// One line of the classified fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    pub program: String,
    pub key: String,
    pub kind: Kind,
    pub class: Class,
    pub old: Option<String>,
    pub new: Option<String>,
    pub note: String,
}

/// A program's stable name: its origin with any `#<ordinal>` replaced
/// by `#<digest>` of the source, so a literal added above it in a
/// test file does not rename it.
pub fn program_id(origin: &str, source: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in source.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    let base = origin.split('#').next().unwrap_or(origin);
    format!("{base}#{:08x}", (h >> 32) as u32 ^ h as u32)
}

/// One side's rows behind a collided key, as the fact the fixture
/// pins: the complete `(native key, value)` tuples in canonical
/// (sorted) order, each component length-framed as `<bytes>:<text>`, a
/// row `<native>=<value>`, rows joined by `; `. Carrying all of them
/// rather than the first is what makes a change to any colliding row,
/// or a row joining or leaving the collision, unexplained again; the
/// framing is what makes the text an identity at all — unframed, a key
/// or value holding `=` or `; ` reads as another split of the same rows
/// (`a=b` → `c` and `a` → `b=c` both read `a=b=c`), so a changed
/// collision kept its classification (outside review of #1278,
/// finding 1).
fn collision_facts<V: Display>(rows: &[(String, &V)]) -> String {
    collision_rows(rows)
        .iter()
        .map(|(nk, v)| format!("{}:{nk}={}:{v}", nk.len(), v.len()))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The same rows as the report prints them, `native=value; …`: for the
/// reader, and not an identity (two collisions can print alike).
fn collision_shown<V: Display>(rows: &[(String, &V)]) -> String {
    collision_rows(rows)
        .iter()
        .map(|(nk, v)| format!("{nk}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn collision_rows<V: Display>(rows: &[(String, &V)]) -> Vec<(String, String)> {
    let mut tuples: Vec<(String, String)> =
        rows.iter().map(|(nk, v)| (nk.clone(), v.to_string())).collect();
    tuples.sort();
    tuples
}

/// A fixture field as the TSV carries it: `\` as `\\`, a tab as `\t`, a
/// newline as `\n`, a carriage return as `\r`, and a field that is
/// exactly `-` (which reads as "no row") as `\-`, so every field
/// survives [`parse_fixture`] as it was.
fn escape_field(s: &str) -> String {
    if s == "-" {
        return "\\-".to_string();
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

/// [`escape_field`] undone; an escape it never writes is an error.
fn unescape_field(s: &str) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('-') if s == "\\-" => out.push('-'),
            other => {
                return Err(format!(
                    "unknown escape `\\{}` in `{s}`",
                    other.map(String::from).unwrap_or_default()
                ))
            }
        }
    }
    Ok(out)
}

/// The shadow's result over a corpus.
#[derive(Debug, Default)]
pub struct Report {
    pub family: String,
    pub programs: usize,
    pub rows_compared: usize,
    pub divergences: Vec<Divergence>,
}

impl Report {
    pub fn new(family: &str) -> Report {
        Report {
            family: family.to_string(),
            ..Default::default()
        }
    }

    /// Compare one program's rows from both producers, each under its
    /// own key, through the correspondence `map_old` / `map_new` onto
    /// one shared key. A row the correspondence has no key for is
    /// `Unmapped`; two rows of one side mapped to one key are a
    /// `Collision` (with every native key listed); a key on one side
    /// only is `OnlyOld` / `OnlyNew`. `witness(key)` and `slice(key)`
    /// render what decided the rows and what depends on them.
    #[allow(clippy::too_many_arguments)]
    pub fn compare_rows<KO, KN, K, V>(
        &mut self,
        program: &str,
        old: &[(KO, V)],
        new: &[(KN, V)],
        map_old: impl Fn(&KO) -> Option<K>,
        map_new: impl Fn(&KN) -> Option<K>,
        witness: impl Fn(&K) -> Vec<String>,
        slice: impl Fn(&K) -> Vec<String>,
    ) where
        KO: Display,
        KN: Display,
        K: Ord + Display + Clone,
        V: PartialEq + Display,
    {
        self.programs += 1;
        let mut old_map: BTreeMap<K, Vec<(String, &V)>> = BTreeMap::new();
        let mut new_map: BTreeMap<K, Vec<(String, &V)>> = BTreeMap::new();
        for (k, v) in old {
            match map_old(k) {
                Some(key) => old_map.entry(key).or_default().push((k.to_string(), v)),
                None => self.divergences.push(Divergence {
                    family: self.family.clone(),
                    kind: Kind::Unmapped,
                    program: program.to_string(),
                    key: k.to_string(),
                    old: Some(v.to_string()),
                    new: None,
                    old_shown: Some(v.to_string()),
                    new_shown: None,
                    witnesses: Vec::new(),
                    slice: Vec::new(),
                    natives: vec![format!("old `{k}` = {v}")],
                }),
            }
        }
        for (k, v) in new {
            match map_new(k) {
                Some(key) => new_map.entry(key).or_default().push((k.to_string(), v)),
                None => self.divergences.push(Divergence {
                    family: self.family.clone(),
                    kind: Kind::Unmapped,
                    program: program.to_string(),
                    key: k.to_string(),
                    old: None,
                    new: Some(v.to_string()),
                    old_shown: None,
                    new_shown: Some(v.to_string()),
                    witnesses: Vec::new(),
                    slice: Vec::new(),
                    natives: vec![format!("new `{k}` = {v}")],
                }),
            }
        }
        let mut keys: Vec<&K> = old_map.keys().chain(new_map.keys()).collect();
        keys.sort();
        keys.dedup();
        for k in keys {
            let o = old_map.get(k);
            let n = new_map.get(k);
            let collided =
                o.map(|v| v.len() > 1).unwrap_or(false) || n.map(|v| v.len() > 1).unwrap_or(false);
            if collided {
                let mut natives = Vec::new();
                for (nk, v) in o.into_iter().flatten() {
                    natives.push(format!("old `{nk}` = {v}"));
                }
                for (nk, v) in n.into_iter().flatten() {
                    natives.push(format!("new `{nk}` = {v}"));
                }
                self.divergences.push(Divergence {
                    family: self.family.clone(),
                    kind: Kind::Collision,
                    program: program.to_string(),
                    key: k.to_string(),
                    old: o.map(|rows| collision_facts(rows)),
                    new: n.map(|rows| collision_facts(rows)),
                    old_shown: o.map(|rows| collision_shown(rows)),
                    new_shown: n.map(|rows| collision_shown(rows)),
                    witnesses: witness(k),
                    slice: slice(k),
                    natives,
                });
                continue;
            }
            self.rows_compared += 1;
            let o = o.and_then(|v| v.first()).map(|(_, v)| *v);
            let n = n.and_then(|v| v.first()).map(|(_, v)| *v);
            let kind = match (o, n) {
                (Some(a), Some(b)) if a == b => continue,
                (Some(_), Some(_)) => Kind::Disagreement,
                (Some(_), None) => Kind::OnlyOld,
                (None, Some(_)) => Kind::OnlyNew,
                (None, None) => continue,
            };
            self.divergences.push(Divergence {
                family: self.family.clone(),
                kind,
                program: program.to_string(),
                key: k.to_string(),
                old: o.map(|v| v.to_string()),
                new: n.map(|v| v.to_string()),
                old_shown: o.map(|v| v.to_string()),
                new_shown: n.map(|v| v.to_string()),
                witnesses: witness(k),
                slice: slice(k),
                natives: Vec::new(),
            });
        }
    }

    /// The simple form: both producers already keyed by the shared
    /// key (a `BTreeMap` cannot collide).
    pub fn compare<K, V>(
        &mut self,
        program: &str,
        old: &BTreeMap<K, V>,
        new: &BTreeMap<K, V>,
        witness: impl Fn(&K) -> Vec<String>,
    ) where
        K: Ord + Display + Clone,
        V: PartialEq + Display + Clone,
    {
        let o: Vec<(K, V)> = old.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let n: Vec<(K, V)> = new.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        self.compare_rows(
            program,
            &o,
            &n,
            |k| Some(k.clone()),
            |k| Some(k.clone()),
            witness,
            |_| Vec::new(),
        );
    }

    /// The fixer's view: one block per divergence.
    pub fn render(&self) -> String {
        let mut s = format!(
            "shadow `{}`: {} programs, {} rows compared, {} divergence(s)\n",
            self.family,
            self.programs,
            self.rows_compared,
            self.divergences.len()
        );
        for d in &self.divergences {
            s.push_str(&format!(
                "\n{}  key `{}`  [{}]\n  old: {}\n  new: {}\n",
                d.program,
                d.key,
                d.kind.label(),
                d.old_shown.as_deref().unwrap_or("(no row)"),
                d.new_shown.as_deref().unwrap_or("(no row)")
            ));
            for w in &d.witnesses {
                s.push_str(&format!("  because: {w}\n"));
            }
            for x in &d.slice {
                s.push_str(&format!("  depends: {x}\n"));
            }
            for x in &d.natives {
                s.push_str(&format!("  native: {x}\n"));
            }
        }
        s
    }

    /// Hold the report against the classified fixture. Returns the
    /// divergences that are not explained (absent from the fixture,
    /// or classified `Unclassified` / `Regression`, or whose facts
    /// moved since they were classified) and the fixture lines that
    /// no longer match any divergence (stale).
    pub fn explain<'a>(
        &'a self,
        fixture: &'a [Classified],
    ) -> (
        Vec<(&'a Divergence, Option<&'a Classified>)>,
        Vec<&'a Classified>,
    ) {
        let mut unexplained = Vec::new();
        for d in &self.divergences {
            let c = fixture
                .iter()
                .find(|c| c.program == d.program && c.key == d.key);
            match c {
                Some(c)
                    if matches!(
                        c.class,
                        Class::KnownOldBug | Class::Correction | Class::SpecDisagreement
                    ) && c.kind == d.kind
                        && c.old == d.old
                        && c.new == d.new => {}
                other => unexplained.push((d, other)),
            }
        }
        let stale: Vec<&Classified> = fixture
            .iter()
            .filter(|c| {
                !self
                    .divergences
                    .iter()
                    .any(|d| d.program == c.program && d.key == c.key)
            })
            .collect();
        (unexplained, stale)
    }

    /// The fixture text after this run: every divergence, keeping the
    /// class and note of a line that already explained it, marking
    /// the rest `unclassified`; stale lines dropped.
    pub fn render_fixture(&self, existing: &[Classified]) -> String {
        let mut s = String::from(
            "# Classified divergences of a shadow (F.40). Generated by the shadow test in\n\
             # regeneration mode (the test's doc says how); classify each `unclassified` line by\n\
             # hand (known-old-bug, correction, spec-disagreement) with a note, or fix the\n\
             # regression. Columns:\n\
             # program <TAB> key <TAB> kind <TAB> class <TAB> old <TAB> new <TAB> note\n\
             # A field writes `\\` as `\\\\`, a tab as `\\t`, a newline as `\\n`, a carriage return\n\
             # as `\\r`; an old or new of `-` is no row, and a field that is `-` itself is `\\-`.\n",
        );
        for d in &self.divergences {
            let prev = existing
                .iter()
                .find(|c| c.program == d.program && c.key == d.key);
            let (class, note) = match prev {
                Some(c) if c.kind == d.kind && c.old == d.old && c.new == d.new => {
                    (c.class, c.note.clone())
                }
                Some(c) => (
                    Class::Unclassified,
                    format!(
                        "facts moved since classified as {}: {}",
                        c.class.label(),
                        c.note
                    ),
                ),
                None => (Class::Unclassified, String::new()),
            };
            let fact = |f: &Option<String>| f.as_deref().map(escape_field).unwrap_or_else(|| "-".to_string());
            s.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                escape_field(&d.program),
                escape_field(&d.key),
                d.kind.label(),
                class.label(),
                fact(&d.old),
                fact(&d.new),
                escape_field(&note)
            ));
        }
        s
    }
}

/// Parse a classified fixture. Malformed lines are errors, not
/// silently skipped: a fixture that parses as empty would pass every
/// gate.
pub fn parse_fixture(text: &str) -> Result<Vec<Classified>, String> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() != 7 {
            return Err(format!(
                "line {}: expected 7 tab-separated columns, found {}",
                i + 1,
                cols.len()
            ));
        }
        let kind = Kind::parse(cols[2])
            .ok_or_else(|| format!("line {}: unknown kind `{}`", i + 1, cols[2]))?;
        let class = Class::parse(cols[3])
            .ok_or_else(|| format!("line {}: unknown class `{}`", i + 1, cols[3]))?;
        let field = |c: &str| unescape_field(c).map_err(|e| format!("line {}: {e}", i + 1));
        let cell = |c: &str| if c == "-" { Ok(None) } else { field(c).map(Some) };
        out.push(Classified {
            program: field(cols[0])?,
            key: field(cols[1])?,
            kind,
            class,
            old: cell(cols[4])?,
            new: cell(cols[5])?,
            note: field(cols[6])?,
        });
    }
    Ok(out)
}

/// Render the unexplained divergences and stale lines as the gate's
/// failure message.
pub fn gate_message(
    report: &Report,
    unexplained: &[(&Divergence, Option<&Classified>)],
    stale: &[&Classified],
    fixture_path: &str,
) -> String {
    let mut s = format!(
        "shadow `{}` over {} programs ({} rows): {} unexplained divergence(s), {} stale fixture line(s).\n",
        report.family,
        report.programs,
        report.rows_compared,
        unexplained.len(),
        stale.len()
    );
    for (d, c) in unexplained {
        s.push_str(&format!(
            "\n{}  key `{}`  [{}]\n  old: {}\n  new: {}\n  status: {}\n",
            d.program,
            d.key,
            d.kind.label(),
            d.old_shown.as_deref().unwrap_or("(no row)"),
            d.new_shown.as_deref().unwrap_or("(no row)"),
            match c {
                None => "not in the fixture".to_string(),
                Some(c) => format!("{} — {}", c.class.label(), c.note),
            }
        ));
        for w in &d.witnesses {
            s.push_str(&format!("  because: {w}\n"));
        }
        for x in &d.slice {
            s.push_str(&format!("  depends: {x}\n"));
        }
        for x in &d.natives {
            s.push_str(&format!("  native: {x}\n"));
        }
    }
    for c in stale {
        s.push_str(&format!(
            "\nstale: {}  key `{}` no longer diverges; drop the line\n",
            c.program, c.key
        ));
    }
    s.push_str(&format!(
        "\nRegenerate {fixture_path} in the shadow test's regeneration mode (its doc says how), then \
         classify every `unclassified` line (known-old-bug, correction, spec-disagreement) with a \
         note, or fix the regression."
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_reports_only_disagreements_and_explain_gates_them() {
        let mut r = Report::new("placement");
        let old: BTreeMap<&str, &str> = [("A", "main"), ("B", "pool:x"), ("C", "pinned")].into();
        let new: BTreeMap<&str, &str> = [("A", "main"), ("B", "pool:y"), ("D", "main")].into();
        r.compare("p.hl", &old, &new, |k| vec![format!("decl of {k}")]);
        assert_eq!(r.rows_compared, 4);
        assert_eq!(r.divergences.len(), 3);
        let kinds: Vec<Kind> = r.divergences.iter().map(|d| d.kind).collect();
        assert_eq!(
            kinds,
            vec![Kind::Disagreement, Kind::OnlyOld, Kind::OnlyNew]
        );
        let fixture = parse_fixture(&r.render_fixture(&[])).unwrap();
        assert!(fixture.iter().all(|c| c.class == Class::Unclassified));
        let (unexplained, stale) = r.explain(&fixture);
        assert_eq!(unexplained.len(), 3);
        assert!(stale.is_empty());
        let classified: Vec<Classified> = fixture
            .into_iter()
            .map(|mut c| {
                c.class = Class::KnownOldBug;
                c.note = "the old walk misses nested loci".into();
                c
            })
            .collect();
        let (unexplained, _) = r.explain(&classified);
        assert!(unexplained.is_empty());
        // a moved fact is unexplained again
        let mut moved = classified.clone();
        moved[0].new = Some("elsewhere".into());
        let (unexplained, _) = r.explain(&moved);
        assert_eq!(unexplained.len(), 1);
        let text = r.render_fixture(&moved);
        assert!(text.contains("facts moved since classified"));
    }

    #[test]
    fn a_non_injective_correspondence_is_a_collision_not_a_dropped_row() {
        let mut r = Report::new("ownership");
        // two old rows (span-keyed) that one name maps onto
        let old: Vec<(&str, &str)> = vec![("(10, 12)", "caller"), ("(40, 42)", "binding")];
        let new: Vec<(u32, &str)> = vec![(7, "caller")];
        r.compare_rows(
            "p.hl",
            &old,
            &new,
            |_span| Some("r".to_string()),
            |_id| Some("r".to_string()),
            |_| vec![],
            |_| vec!["the let's reclaim".into()],
        );
        assert_eq!(r.divergences.len(), 1);
        assert_eq!(r.divergences[0].kind, Kind::Collision);
        assert_eq!(
            r.divergences[0].slice,
            vec!["the let's reclaim".to_string()]
        );
        assert_eq!(
            r.divergences[0].natives,
            vec![
                "old `(10, 12)` = caller".to_string(),
                "old `(40, 42)` = binding".to_string(),
                "new `7` = caller".to_string()
            ],
            "a collision lists every native key so the fixer sees the two spans"
        );
        assert_eq!(r.rows_compared, 0, "a collided key is not compared");
        assert_eq!(
            r.divergences[0].old.as_deref(),
            Some("8:(10, 12)=6:caller; 8:(40, 42)=7:binding")
        );
        assert_eq!(r.divergences[0].new.as_deref(), Some("1:7=6:caller"));
        // the report prints the rows for the reader, unframed
        assert_eq!(
            r.divergences[0].old_shown.as_deref(),
            Some("(10, 12)=caller; (40, 42)=binding")
        );
        assert!(r.render().contains("  old: (10, 12)=caller; (40, 42)=binding\n"));
    }

    /// The collision probe: old rows `span1`, `span2` and new row `7`,
    /// all mapped onto one key.
    fn collide(old: &[(&str, &str)]) -> Report {
        let mut r = Report::new("ownership");
        let new: Vec<(u32, &str)> = vec![(7, "caller")];
        r.compare_rows(
            "p.hl",
            old,
            &new,
            |_span| Some("r".to_string()),
            |_id| Some("r".to_string()),
            |_| vec![],
            |_| vec![],
        );
        r
    }

    /// The report's fixture with every line classified.
    fn classified(r: &Report) -> Vec<Classified> {
        parse_fixture(&r.render_fixture(&[]))
            .unwrap()
            .into_iter()
            .map(|mut c| {
                c.class = Class::KnownOldBug;
                c.note = "the old producer keys a let by its span".into();
                c
            })
            .collect()
    }

    #[test]
    fn a_change_to_any_row_of_a_classified_collision_is_unexplained_again() {
        let base = collide(&[("span1", "caller"), ("span2", "binding")]);
        let fixture = classified(&base);
        assert_eq!(
            fixture[0].old.as_deref(),
            Some("5:span1=6:caller; 5:span2=7:binding"),
            "the fixture pins every colliding row"
        );
        assert!(base.explain(&fixture).0.is_empty());
        // the rows' order does not matter: the facts are sorted
        let reordered = collide(&[("span2", "binding"), ("span1", "caller")]);
        assert!(reordered.explain(&fixture).0.is_empty());
        for (what, rows) in [
            (
                "a secondary row that changes",
                vec![("span1", "caller"), ("span2", "wrong-arena")],
            ),
            (
                "the first row that changes",
                vec![("span1", "wrong-arena"), ("span2", "binding")],
            ),
            (
                "a row that joins the collision",
                vec![
                    ("span1", "caller"),
                    ("span2", "binding"),
                    ("span3", "caller"),
                ],
            ),
        ] {
            let r = collide(&rows);
            assert_eq!(r.divergences[0].kind, Kind::Collision, "{what}");
            assert_eq!(
                r.explain(&fixture).0.len(),
                1,
                "{what} is unexplained again"
            );
        }
        // a row that leaves a classified three-row collision, which is
        // still a collision
        let three = collide(&[
            ("span1", "caller"),
            ("span2", "binding"),
            ("span3", "caller"),
        ]);
        let fixture3 = classified(&three);
        assert!(three.explain(&fixture3).0.is_empty());
        let left = collide(&[("span1", "caller"), ("span3", "caller")]);
        assert_eq!(left.divergences[0].kind, Kind::Collision);
        assert_eq!(
            left.explain(&fixture3).0.len(),
            1,
            "a row that leaves is unexplained again"
        );
        // a row that leaves a two-row collision ends it: the line is stale
        let one = collide(&[("span1", "caller")]);
        assert!(one.divergences.is_empty());
        assert_eq!(
            one.explain(&fixture).1.len(),
            1,
            "the classification is stale"
        );
    }

    /// A collision's identity is injective over its rows: a separator
    /// inside a key or a value cannot make two different sets of rows
    /// read alike (outside review of #1278, finding 1).
    #[test]
    fn a_collisions_identity_tells_every_set_of_rows_apart() {
        let old_of = |rows: &[(&str, &str)]| collide(rows).divergences[0].old.clone().unwrap();
        for (a, b) in [
            (
                vec![("a=b", "c"), ("d", "e")],
                vec![("a", "b=c"), ("d", "e")],
            ),
            (
                vec![("a", "one; b=two"), ("c", "three")],
                vec![("a", "one"), ("b", "two; c=three")],
            ),
        ] {
            assert_ne!(old_of(&a), old_of(&b), "{a:?} and {b:?}");
            // the unframed rendering is the ambiguity, which is why it
            // is only shown
            assert_eq!(
                collide(&a).divergences[0].old_shown,
                collide(&b).divergences[0].old_shown
            );
        }

        // `=` and `; ` inside a key and inside a value: the fixture
        // round-trips, a reorder is still explained, and a change, an
        // added row and a removed one are unexplained again
        let rows = [("k=1; x", "v; y=2"), ("k2", "a=b; c"), ("k3", "plain")];
        let base = collide(&rows);
        let fixture = classified(&base);
        assert_eq!(fixture[0].old, base.divergences[0].old);
        assert!(base.explain(&fixture).0.is_empty());
        let reordered = collide(&[rows[2], rows[0], rows[1]]);
        assert!(reordered.explain(&fixture).0.is_empty(), "a reorder is explained");
        for (what, changed) in [
            ("a changed value", vec![("k=1; x", "v; y=3"), rows[1], rows[2]]),
            ("a changed key", vec![("k=1; y", "v; y=2"), rows[1], rows[2]]),
            (
                "an added row",
                vec![rows[0], rows[1], rows[2], ("k4", "=; ")],
            ),
            ("a removed row", vec![rows[0], rows[1]]),
            // the same text split differently
            (
                "a separator moved across a row",
                vec![("k=1; x", "v"), ("y=2", ""), rows[1], rows[2]],
            ),
        ] {
            let r = collide(&changed);
            assert_eq!(r.divergences[0].kind, Kind::Collision, "{what}");
            assert_eq!(r.explain(&fixture).0.len(), 1, "{what} is unexplained again");
        }
    }

    /// Every fixture field survives the TSV: a tab, a newline, a
    /// carriage return, a backslash, and a fact that is `-` itself.
    #[test]
    fn a_fixture_field_round_trips_whatever_it_holds() {
        let mut r = Report::new("placement");
        let old: BTreeMap<&str, &str> = [
            ("tab", "one\ttwo"),
            ("newline", "first\nsecond\r\n"),
            ("slash", "a\\tb\\"),
            ("dash", "-"),
            ("key\twith\ntab", "x"),
        ]
        .into();
        let new: BTreeMap<&str, &str> = [("tab", "y"), ("newline", "y"), ("slash", "y"), ("dash", "y")].into();
        r.compare("p\t.hl", &old, &new, |_| vec![]);
        assert_eq!(r.divergences.len(), 5);
        let text = r.render_fixture(&[]);
        let fixture = parse_fixture(&text).unwrap();
        assert_eq!(fixture.len(), 5);
        for (c, d) in fixture.iter().zip(&r.divergences) {
            assert_eq!((&c.program, &c.key, &c.old, &c.new), (&d.program, &d.key, &d.old, &d.new));
        }
        // the dash is a fact, not "no row"; the only-old key's new is none
        let dash = fixture.iter().find(|c| c.key == "dash").unwrap();
        assert_eq!(dash.old.as_deref(), Some("-"));
        let only_old = fixture.iter().find(|c| c.key == "key\twith\ntab").unwrap();
        assert_eq!((only_old.kind, only_old.new.as_deref()), (Kind::OnlyOld, None));
        // a note with a newline survives a regeneration too
        let noted: Vec<Classified> = fixture
            .into_iter()
            .map(|mut c| {
                c.class = Class::Correction;
                c.note = "decided\tin\n#123".into();
                c
            })
            .collect();
        assert!(r.explain(&noted).0.is_empty());
        assert_eq!(parse_fixture(&r.render_fixture(&noted)).unwrap(), noted);
        // an escape the writer never writes is an error, not a guess
        assert!(parse_fixture("a\\q\tb\tonly-old\tcorrection\tx\t-\tn").is_err());
        assert!(parse_fixture("a\tb\tonly-old\tcorrection\tx\\\t-\tn").is_err());
    }

    #[test]
    fn a_row_the_correspondence_cannot_map_is_reported_not_invented() {
        let mut r = Report::new("ownership");
        let old: Vec<(&str, &str)> = vec![("a", "x"), ("orphan", "y")];
        let new: Vec<(&str, &str)> = vec![("a", "x")];
        r.compare_rows(
            "p.hl",
            &old,
            &new,
            |k| {
                if *k == "orphan" {
                    None
                } else {
                    Some(k.to_string())
                }
            },
            |k| Some(k.to_string()),
            |_| vec![],
            |_| vec![],
        );
        assert_eq!(r.divergences.len(), 1);
        assert_eq!(r.divergences[0].kind, Kind::Unmapped);
        assert_eq!(r.divergences[0].key, "orphan");
        assert_eq!(r.rows_compared, 1);
        let mut fixture = parse_fixture(&r.render_fixture(&[])).unwrap();
        assert_eq!(fixture[0].kind, Kind::Unmapped);
        fixture[0].class = Class::KnownOldBug;
        fixture[0].note = "the old producer keys a synthesized site".into();
        assert!(r.explain(&fixture).0.is_empty());
        fixture[0].kind = Kind::OnlyOld;
        assert_eq!(
            r.explain(&fixture).0.len(),
            1,
            "a kind that moves is unexplained again"
        );
    }

    #[test]
    fn a_program_is_named_by_its_content_not_its_ordinal() {
        let a = program_id("tests/x.rs#3", "fn main() {}");
        let b = program_id("tests/x.rs#9", "fn main() {}");
        let c = program_id("tests/x.rs#3", "fn main() { }");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with("tests/x.rs#"));
        assert_eq!(
            program_id("a/b.hl", "x"),
            format!("a/b.hl#{}", &program_id("a/b.hl", "x")[7..])
        );
    }

    #[test]
    fn a_malformed_fixture_is_an_error() {
        assert!(parse_fixture("a\tb\tc").is_err());
        assert!(parse_fixture("a\tb\tonly-old\tnope\t-\t-\t").is_err());
        assert!(parse_fixture("a\tb\tsideways\tknown-old-bug\t-\t-\t").is_err());
        assert!(parse_fixture("# only a comment\n").unwrap().is_empty());
    }
}
