//! Allocation-summary sites keyed by what an unrelated edit leaves
//! alone.
//!
//! A site's span is a byte range into the merged seed, so an edit
//! anywhere earlier in the seed (a comment in another file, a fn above
//! it) moves every later span, and a pin written in spans moves with
//! it. The pins of the `alloc_summary_*` tests key a site instead by
//! the fn that holds it, its kind and its ordinal among that fn's sites
//! of the kind, in the fn's own order: `Provider::submit
//! CollectionInsert("vec") #1 InvokedUnboundedly`. That moves only when
//! the fn itself gains, loses or reorders a site of the kind, which is
//! a change the pin should see.
//!
//! A test file includes it as it includes the entries:
//!
//! ```text
//! #[path = "support/site_keys.rs"]
//! mod site_keys;
//! ```

#![allow(dead_code)]

use std::collections::BTreeMap;

use hale_types::alloc_summary::{AllocSummary, LeakSite};

/// `l` as `<owner> [store ]<kind> #<ordinal> <reason>`: the ordinal
/// counts the owner's sites of `l`'s kind, in its own order, from 1; a
/// site that is a call's store (the dump's `store` lines) is counted
/// among those and says `store`, and a call's fresh result
/// (`CallResult`, whose kind names the callee) among the fn's call
/// results.
pub fn leak_key(summary: &AllocSummary, l: &LeakSite) -> String {
    let f = summary
        .fns
        .get(&l.owner)
        .unwrap_or_else(|| panic!("a leak site of {}, which has no row", l.owner.display()));
    let results: Vec<_> = f.call_results.iter().map(|r| r.site.clone()).collect();
    for (list, store) in [(&f.sites, ""), (&f.call_stores, "store "), (&results, "")] {
        let mut n = 0;
        for s in list.iter().filter(|s| s.kind == l.kind) {
            n += 1;
            if s.span == l.span {
                return format!("{} {}{:?} #{} {:?}", l.owner.display(), store, l.kind, n, l.reason);
            }
        }
    }
    panic!("a leak site of {} that is none of its sites: {:?}", l.owner.display(), l.kind)
}

/// A dump (`AllocSummary::render`) with each `@<start>..<end>` written
/// as `#<ordinal>`: the line's ordinal among the lines of its block (a
/// `fn` or `locus` line and what follows it) that lead with the same two
/// words (`alloc string-concat`, `loop while`, `store vec-insert`). An
/// `alloc` line's ordinal is the [`leak_key`] ordinal of its site.
pub fn stable_dump(dump: &str) -> String {
    let mut out = String::with_capacity(dump.len());
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for line in dump.lines() {
        if line.starts_with("fn ") || line.starts_with("locus ") || line.starts_with('#') {
            seen.clear();
        }
        match span_at(line) {
            Some((at, end)) => {
                let lead: Vec<&str> = line.split_whitespace().take(2).collect();
                let n = seen.entry(lead.join(" ")).or_default();
                *n += 1;
                out.push_str(&line[..at]);
                out.push_str(&format!("#{n}"));
                out.push_str(&line[end..]);
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

/// Where `line`'s `@<digits>..<digits>` starts and ends, if it has one.
fn span_at(line: &str) -> Option<(usize, usize)> {
    let b = line.as_bytes();
    let digits = |mut i: usize| {
        let s = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        (i > s).then_some(i)
    };
    for (at, _) in line.match_indices('@') {
        let Some(i) = digits(at + 1) else { continue };
        if !line[i..].starts_with("..") {
            continue;
        }
        if let Some(end) = digits(i + 2) {
            return Some((at, end));
        }
    }
    None
}
