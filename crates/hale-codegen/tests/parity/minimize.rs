//! A delta-debugging minimizer: remove lines while the divergence
//! persists.
//!
//! ddmin over each file's lines: try removing a chunk of `n` lines at
//! every offset, keep any removal after which `still` holds, halve `n`
//! when a pass removes nothing, stop at single lines or when the budget
//! of `still` calls runs out. A removal that unbalances a brace fails to
//! parse, which the check rejects at once, so the blind line-level search
//! costs little more than a statement-aware one and needs no grammar.

use crate::gate::Files;

/// The smallest program found for which `still` holds, and how many
/// times `still` was asked.
pub fn minimize(files: &Files, still: &dyn Fn(&Files) -> bool, budget: usize) -> (Files, usize) {
    let mut cur = files.clone();
    let mut asked = 0usize;
    // Whole files first (a sibling the divergence does not need).
    let names: Vec<String> = cur.keys().cloned().collect();
    for name in names {
        if name == crate::mutators::MAIN || cur.len() == 1 {
            continue;
        }
        let mut trial = cur.clone();
        trial.remove(&name);
        asked += 1;
        if still(&trial) {
            cur = trial;
        }
    }
    let names: Vec<String> = cur.keys().cloned().collect();
    for name in names {
        let mut lines: Vec<String> = cur[&name].lines().map(String::from).collect();
        let mut n = (lines.len() / 2).max(1);
        loop {
            let mut removed_any = false;
            let mut i = 0;
            while i < lines.len() {
                if asked >= budget {
                    cur.insert(name.clone(), lines.join("\n") + "\n");
                    return (cur, asked);
                }
                let end = (i + n).min(lines.len());
                // a chunk of blank or comment lines changes nothing a
                // compiler sees: drop it without asking
                let trivial = lines[i..end].iter().all(|l| {
                    let t = l.trim();
                    t.is_empty() || t.starts_with("//")
                });
                let mut trial_lines = lines.clone();
                trial_lines.drain(i..end);
                if trivial {
                    lines = trial_lines;
                    continue;
                }
                let mut trial = cur.clone();
                trial.insert(name.clone(), trial_lines.join("\n") + "\n");
                asked += 1;
                if still(&trial) {
                    lines = trial_lines;
                    removed_any = true;
                } else {
                    i += n;
                }
            }
            cur.insert(name.clone(), lines.join("\n") + "\n");
            if n == 1 && !removed_any {
                break;
            }
            if !removed_any {
                n = (n / 2).max(1);
            }
        }
    }
    (cur, asked)
}
