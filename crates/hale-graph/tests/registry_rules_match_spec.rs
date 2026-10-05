//! The registry's rule lists are held to the spec's (F.40 phase 4, W1).
//!
//! `registry_guard` fails a registered rule that has no evaluator; it
//! cannot see a rule nobody registered. This test reads each list the
//! spec states (a file and a heading: `RULE_LISTS`), extracts its rules
//! with their bold titles, and fails, naming the rule, when the spec
//! numbers one the registry does not list, the registry lists one the
//! spec does not, or a title differs (a renumbering or a retitling in
//! the spec fails the build until the registry follows).

use std::collections::BTreeMap;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The lines of the section under `heading` at `level`, up to the next
/// heading of the same or a higher level (fenced code is not scanned
/// for headings).
fn section(text: &str, heading: &str, level: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        let hashes = line.chars().take_while(|c| *c == '#').count();
        let is_heading = !fenced && hashes > 0 && line[hashes..].starts_with(' ');
        if is_heading {
            if inside && hashes <= level {
                break;
            }
            if !inside && hashes == level && line[hashes..].trim() == heading {
                inside = true;
                continue;
            }
        }
        if inside {
            out.push(line.to_string());
        }
    }
    out
}

/// A bold title opening at `start` (just past its `**`) and closing at
/// the first `**` after it, which may be on a later line; whitespace
/// collapsed to single spaces.
fn bold_title(first: &str, rest: &[String]) -> Option<String> {
    let mut acc = String::new();
    let mut piece = first;
    let mut more = rest.iter();
    loop {
        if let Some(end) = piece.find("**") {
            acc.push_str(&piece[..end]);
            break;
        }
        acc.push_str(piece);
        acc.push(' ');
        piece = more.next()?;
    }
    Some(acc.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Numbered rules of a section: `N. **Title.**` at column 0 (fenced
/// code skipped), by number.
fn numbered_rules(lines: &[String]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut fenced = false;
    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        if fenced {
            continue;
        }
        let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 || !line[digits..].starts_with(". **") {
            continue;
        }
        let n: usize = line[..digits].parse().unwrap();
        let title = bold_title(&line[digits + 4..], &lines[i + 1..])
            .unwrap_or_else(|| panic!("rule {n}: its bold title never closes"));
        out.push((n, title));
    }
    out
}

#[test]
fn rule_lists_match_the_spec() {
    let mut problems = Vec::new();
    for list in hale_graph::RULE_LISTS {
        let path = root().join(list.spec);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let lines = section(&text, list.heading, list.level);
        assert!(
            !lines.is_empty(),
            "{} has no `{}` section: the scrape is vacuous",
            list.spec,
            list.heading
        );
        let at = format!("{} § {}", list.spec, list.heading);

        let spec: Vec<(usize, String)> = numbered_rules(&lines);
        for (i, (n, _)) in spec.iter().enumerate() {
            if *n != i + 1 {
                problems.push(format!("{at}: rule numbers are not 1, 2, 3, ..: `{n}` at position {}", i + 1));
            }
        }
        let spec: BTreeMap<usize, String> = spec.into_iter().collect();

        let prefix = format!("{}/", list.key);
        let mut held: BTreeMap<usize, (&str, &str)> = BTreeMap::new();
        for r in hale_graph::rules().iter().filter(|r| r.id.starts_with(&prefix)) {
            match r.id[prefix.len()..].parse::<usize>() {
                Ok(n) => {
                    if held.insert(n, (r.id, r.title)).is_some() {
                        problems.push(format!("registry lists `{}` twice", r.id));
                    }
                }
                Err(_) => problems.push(format!("`{}` is not numbered under `{}`", r.id, list.key)),
            }
        }

        // Non-vacuity: the scrape finds as many as the registry holds,
        // and the registry holds some.
        assert!(
            !spec.is_empty() && !held.is_empty(),
            "{at}: the scrape found {} rules and the registry holds {}: vacuous",
            spec.len(),
            held.len()
        );
        for (n, title) in &spec {
            match held.get(n) {
                None => problems.push(format!(
                    "{at}: rule {n} (\"{title}\") is numbered by the spec and not registered as `{prefix}{n}`"
                )),
                Some((id, t)) if t != title => problems.push(format!(
                    "{at}: rule {n} is titled \"{title}\" in the spec and \"{t}\" in the registry (`{id}`)"
                )),
                Some(_) => {}
            }
        }
        for (n, (id, t)) in &held {
            if !spec.contains_key(n) {
                problems.push(format!(
                    "`{id}` (\"{t}\") is registered and the spec's {at} does not number a rule {n}"
                ));
            }
        }
        assert_eq!(
            spec.len(),
            held.len(),
            "{at}: the scrape found {} rules, the registry holds {}",
            spec.len(),
            held.len()
        );
    }
    assert!(
        problems.is_empty(),
        "the registry and the spec's rule lists disagree:\n{}",
        problems.join("\n")
    );
}

#[test]
fn the_scrape_reads_a_wrapped_title_and_skips_fenced_text() {
    let text = "## A\n\n### Rules\n\n1. **One.** body\n   ```\n2. **Not a rule**\n   ```\n\n2. **Two wraps\n   here (error).** body\n\n### Next\n\n3. **Outside.**\n";
    let lines = section(text, "Rules", 3);
    assert_eq!(
        numbered_rules(&lines),
        vec![(1, "One.".to_string()), (2, "Two wraps here (error).".to_string())]
    );
}
