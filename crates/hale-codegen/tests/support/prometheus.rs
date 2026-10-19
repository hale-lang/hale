//! A checker for the Prometheus text exposition (format 0.0.4), strict
//! where the reference parser is: a family's `# TYPE` line at most once
//! and before its first sample, a family's samples together, metric and
//! label names well formed, label values quoted with only `\\`, `\"` and
//! `\n` escaped, every value a float (`NaN`, `+Inf`, `-Inf` included), and
//! a histogram's samples its `_bucket` (with `le`), `_sum` and `_count`.
//! Returns the families in the order they appear.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

fn is_name(s: &str, colon: bool) -> bool {
    let mut cs = s.chars();
    let ok_first = |c: char| c.is_ascii_alphabetic() || c == '_' || (colon && c == ':');
    match cs.next() {
        Some(c) if ok_first(c) => cs.all(|c| ok_first(c) || c.is_ascii_digit()),
        _ => false,
    }
}

fn is_value(s: &str) -> bool {
    matches!(s, "NaN" | "+Inf" | "-Inf") || s.parse::<f64>().is_ok()
}

/// `{k="v",…}` at the start of `s`: the labels and the rest of the line.
fn labels(s: &str) -> Result<(Vec<(String, String)>, &str), String> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 1;
    loop {
        if b.get(i) == Some(&b'}') {
            return Ok((out, &s[i + 1..]));
        }
        let eq = s[i..].find('=').ok_or("a label without `=`")? + i;
        let key = &s[i..eq];
        if !is_name(key, false) {
            return Err(format!("label name `{key}`"));
        }
        if b.get(eq + 1) != Some(&b'"') {
            return Err(format!("label `{key}`'s value is not quoted"));
        }
        let mut j = eq + 2;
        let mut v = String::new();
        loop {
            match b.get(j) {
                None => return Err("an unterminated label value".into()),
                Some(b'"') => break,
                Some(b'\\') => {
                    match b.get(j + 1) {
                        Some(b'\\') => v.push('\\'),
                        Some(b'"') => v.push('"'),
                        Some(b'n') => v.push('\n'),
                        other => return Err(format!("escape `\\{:?}`", other.map(|c| *c as char))),
                    }
                    j += 2;
                }
                Some(_) => {
                    let c = s[j..].chars().next().unwrap();
                    if c == '\n' {
                        return Err("a raw line break in a label value".into());
                    }
                    v.push(c);
                    j += c.len_utf8();
                }
            }
        }
        out.push((key.to_string(), v));
        i = j + 1;
        match b.get(i) {
            Some(b',') => i += 1,
            Some(b'}') => {}
            _ => return Err("labels not separated by `,`".into()),
        }
    }
}

/// Check `text`; the families, in order, with their kinds.
pub fn parse(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut types: BTreeMap<String, String> = BTreeMap::new();
    let mut order: Vec<(String, String)> = Vec::new();
    let mut closed: BTreeSet<String> = BTreeSet::new();
    let mut current: Option<String> = None;
    for (n, line) in text.lines().enumerate() {
        let at = |e: String| format!("line {}: {e}: {line:?}", n + 1);
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("# TYPE ") {
            let (name, kind) = rest.split_once(' ').ok_or_else(|| at("a TYPE line without a kind".into()))?;
            if !is_name(name, true) {
                return Err(at(format!("metric name `{name}`")));
            }
            if !matches!(kind, "counter" | "gauge" | "histogram" | "summary" | "untyped") {
                return Err(at(format!("kind `{kind}`")));
            }
            if types.insert(name.to_string(), kind.to_string()).is_some() {
                return Err(at(format!("a second TYPE line for `{name}`")));
            }
            if closed.contains(name) || current.as_deref() == Some(name) {
                return Err(at(format!("`{name}`'s TYPE line after its samples")));
            }
            if let Some(c) = current.take() {
                closed.insert(c);
            }
            current = Some(name.to_string());
            order.push((name.to_string(), kind.to_string()));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let name_end = line.find(['{', ' ']).ok_or_else(|| at("a sample without a value".into()))?;
        let name = &line[..name_end];
        if !is_name(name, true) {
            return Err(at(format!("metric name `{name}`")));
        }
        let (lbls, rest) = if line[name_end..].starts_with('{') { labels(&line[name_end..]).map_err(&at)? } else { (Vec::new(), &line[name_end..]) };
        let mut keys = BTreeSet::new();
        for (k, _) in &lbls {
            if !keys.insert(k.clone()) {
                return Err(at(format!("label `{k}` twice")));
            }
        }
        let rest = rest.strip_prefix(' ').ok_or_else(|| at("no space before the value".into()))?;
        let mut parts = rest.split(' ');
        let value = parts.next().unwrap_or("");
        if !is_value(value) {
            return Err(at(format!("value `{value}`")));
        }
        if let Some(ts) = parts.next() {
            ts.parse::<i64>().map_err(|_| at(format!("timestamp `{ts}`")))?;
        }
        // the family the sample belongs to
        let family = ["_bucket", "_sum", "_count"]
            .iter()
            .find_map(|sfx| {
                name.strip_suffix(sfx).filter(|f| types.get(*f).map(|k| k == "histogram").unwrap_or(false))
            })
            .unwrap_or(name)
            .to_string();
        if types.get(&family).map(String::as_str) == Some("histogram") {
            if name == family {
                return Err(at(format!("a histogram's sample named `{name}`")));
            }
            if name.ends_with("_bucket") && !keys.contains("le") {
                return Err(at("a bucket without `le`".into()));
            }
        }
        if current.as_deref() != Some(family.as_str()) {
            if closed.contains(&family) {
                return Err(at(format!("`{family}`'s samples are not together")));
            }
            if let Some(c) = current.take() {
                closed.insert(c);
            }
            if !types.contains_key(&family) {
                order.push((family.clone(), "untyped".to_string()));
            }
            current = Some(family);
        }
    }
    Ok(order)
}
