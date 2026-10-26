//! Admission of the portable parent callback account, directly over JSON.
//! This checks consistency; it neither rebuilds a model nor rechecks source.

use super::{only_keys, RefContext};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate(v: &Value, cx: &RefContext) -> Result<(), String> {
    let universe = v["law"]["fn_universe"]
        .as_array()
        .ok_or("missing function universe")?;
    let expected_bodies: BTreeSet<_> = universe
        .iter()
        .filter(|f| {
            f["kind"] == "hook"
                && v["phases"][f["display"].as_str().unwrap_or("")]["phase"]
                    .as_str()
                    .is_some_and(|s| matches!(s, "accept" | "release"))
        })
        .map(|f| f["display"].as_str().unwrap())
        .collect();
    let names = [
        "protocol_definitions",
        "protocol_constructors",
        "protocol_contexts",
        "protocol_evaluations",
    ];
    let present = names.iter().any(|n| v["relations"].get(*n).is_some());
    if !matches!(v["schema"].as_str(), Some("1.35" | "1.36")) {
        if present || !expected_bodies.is_empty() {
            return Err(
                "parent protocol accounts require topology schema 1.35; re-export from source"
                    .into(),
            );
        }
        // Old documents retain their existing horizon. They cannot claim the
        // new account merely by adding an unvalidated relation.
        return Ok(());
    }
    if !present && expected_bodies.is_empty() {
        return Ok(());
    }
    let rows = |name: &str| -> Result<Vec<&Value>, String> {
        v["relations"][name]
            .as_array()
            .map(|a| a.iter().collect())
            .ok_or_else(|| format!("{name} must be an array"))
    };
    let optional_rows = |name: &str| -> Vec<&Value> {
        v["relations"][name]
            .as_array()
            .into_iter()
            .flatten()
            .collect()
    };
    let text = |row: &Value, key: &str| {
        row[key]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| format!("parent {key} must be nonempty"))
    };
    let number = |row: &Value, key: &str| {
        row[key]
            .as_u64()
            .filter(|n| *n < u32::MAX as u64)
            .map(|n| n as u32)
            .ok_or_else(|| format!("parent {key} must be a finite u32"))
    };
    let flags = |row: &Value| -> Result<(bool, bool), String> {
        let looped = row["loop"]
            .as_bool()
            .ok_or("parent loop must be a boolean")?;
        let unbounded = row["unbounded"]
            .as_bool()
            .ok_or("parent unbounded must be a boolean")?;
        if unbounded && !looped {
            return Err("unbounded parent context must be looped".into());
        }
        Ok((looped, unbounded))
    };
    let function_index = |name: &str| {
        universe
            .iter()
            .position(|f| f["display"] == name)
            .ok_or_else(|| format!("parent account names no function `{name}`"))
    };
    let stage = |row: &Value| match row["stage"].as_str() {
        Some("accept") => Ok(hale_model::ProtocolStage::Accept),
        Some("release") => Ok(hale_model::ProtocolStage::Release),
        _ => Err("invalid parent protocol stage".to_string()),
    };
    let mut definitions = BTreeMap::new();
    let mut positions = BTreeSet::new();
    let mut previous = None;
    for d in rows("protocol_definitions")? {
        only_keys(
            d,
            "protocol definition",
            &["function", "owner", "stage", "child", "interface", "empty"],
            &[],
        )?;
        let function = text(d, "function")?;
        let owner = text(d, "owner")?;
        let stage = stage(d)?;
        let empty = d["empty"]
            .as_bool()
            .ok_or("parent empty must be a boolean")?;
        let interface = d["interface"]
            .as_bool()
            .ok_or("parent interface must be a boolean")?;
        let child = match &d["child"] {
            Value::Null => None,
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            _ => return Err("parent child must be null or a nonempty symbol".into()),
        };
        let index = function_index(&function)?;
        let body = &universe[index];
        let (raw_owner, _, _) = cx
            .loci
            .iter()
            .find(|(_, display, _)| *display == owner)
            .ok_or("parent definition names no locus")?;
        if previous.is_some_and(|old| old >= index)
            || !expected_bodies.contains(function.as_str())
            || !cx.fn_summarized.contains(&function)
            || body["owner"] != owner
            || body["name"] != format!("{raw_owner}::{}", stage.as_str())
            || function != format!("{owner}::{}", stage.as_str())
            || v["phases"][&function]["phase"] != stage.as_str()
            || !positions.insert((owner.clone(), stage))
        {
            return Err("parent definition identity, phase and ownership must agree".into());
        }
        if empty
            && (v["effects"][&function]
                .as_array()
                .is_some_and(|a| !a.is_empty())
                || ["calls", "initializes", "protocol_contexts"]
                    .iter()
                    .any(|name| optional_rows(name).iter().any(|r| r["from"] == function))
                || optional_rows("constructions")
                    .iter()
                    .any(|r| r["function"] == function)
                || optional_rows("publishes")
                    .iter()
                    .any(|r| r["fn"] == function))
        {
            return Err("an empty parent body cannot hide represented execution".into());
        }
        previous = Some(index);
        definitions.insert(function, (owner, stage, child, interface, empty));
    }
    if definitions
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != expected_bodies
    {
        return Err("every authored parent callback requires exactly one definition".into());
    }
    let constructors = optional_rows("constructions");
    let initializations = optional_rows("initializes");
    let arguments = optional_rows("argument_evaluations");
    let mut constructor_symbols = BTreeSet::new();
    let mut previous = None;
    for row in rows("protocol_constructors")? {
        only_keys(
            row,
            "protocol constructor",
            &["function", "site", "child"],
            &[],
        )?;
        let function = text(row, "function")?;
        let site = number(row, "site")?;
        let child = text(row, "child")?;
        let key = (function_index(&function)?, site, child.clone());
        if previous.as_ref().is_some_and(|old| old >= &key)
            || !constructors.iter().any(|c| {
                c["function"] == function && c["site"] == site && c["owner"]["kind"] == "locus"
            })
        {
            return Err(
                "parent constructor symbols require sorted, unique actual locus constructors"
                    .into(),
            );
        }
        previous = Some(key);
        constructor_symbols.insert((function, site, child));
    }
    let mut expected = BTreeSet::new();
    let mut previous = None;
    for (index, c) in rows("protocol_contexts")?.iter().enumerate() {
        only_keys(
            c,
            "protocol context",
            &[
                "from",
                "root",
                "constructor_function",
                "constructor",
                "child",
                "owner",
                "loop",
                "unbounded",
            ],
            &[],
        )?;
        only_keys(&c["root"], "protocol root", &["kind", "site"], &[])?;
        let from = text(c, "from")?;
        let constructor_function = text(c, "constructor_function")?;
        let owner = text(c, "owner")?;
        let child_symbol = text(c, "child")?;
        let ordinal = number(c, "constructor")?;
        let root_site = number(&c["root"], "site")?;
        let kind = match c["root"]["kind"].as_str() {
            Some("construction") => 0,
            Some("call") => 1,
            _ => return Err("invalid parent root namespace".into()),
        };
        let from_index = function_index(&from)?;
        if universe[from_index]["owner"]
            .as_str()
            .is_none_or(str::is_empty)
            || universe[from_index]["kind"].as_str().is_some_and(|k| {
                matches!(
                    k,
                    "argument_default" | "field_default" | "parameter_default"
                )
            })
        {
            return Err("a parent context requires its actual locus caller".into());
        }
        let owner_index = cx
            .loci
            .iter()
            .position(|(_, display, _)| *display == owner)
            .ok_or("parent context names no owner")?;
        let key = (
            from_index,
            kind,
            root_site,
            function_index(&constructor_function)?,
            ordinal,
            owner_index,
            child_symbol.clone(),
        );
        if previous.as_ref().is_some_and(|old| old >= &key) {
            return Err("parent contexts must be sorted and unique".into());
        }
        previous = Some(key);
        let actual = constructors
            .iter()
            .find(|r| r["function"] == constructor_function && r["site"] == ordinal)
            .ok_or("parent context names no actual constructor")?;
        let child = actual["owner"]["name"]
            .as_str()
            .filter(|_| actual["owner"]["kind"] == "locus")
            .ok_or("data types have no parent admission protocol")?;
        let (_raw_child, _, _) = cx
            .loci
            .iter()
            .find(|(_, display, _)| *display == child)
            .ok_or("parent child names no locus")?;
        let mut pending = Vec::new();
        let root = if kind == 0 {
            pending.extend(
                initializations
                    .iter()
                    .filter(|r| r["from"] == from && r["site"] == root_site)
                    .filter_map(|r| r["to"].as_str()),
            );
            constructors
                .iter()
                .copied()
                .find(|r| r["function"] == from && r["site"] == root_site)
        } else {
            pending.extend(
                arguments
                    .iter()
                    .filter(|r| r["from"] == from && r["site"] == root_site)
                    .filter_map(|r| r["to"].as_str()),
            );
            optional_rows("argument_calls")
                .into_iter()
                .find(|r| r["from"] == from && r["site"] == root_site)
        }
        .ok_or("parent root names no constructor or authored argument call")?;
        let a = flags(actual)?;
        let b = flags(root)?;
        if flags(c)? != (a.0 || b.0, a.1 || b.1) {
            return Err("parent context changed its root/constructor repetition".into());
        }
        let direct = kind == 0 && from == constructor_function && ordinal == root_site;
        let mut seen = BTreeSet::new();
        while let Some(body) = pending.pop() {
            if !seen.insert(body) {
                continue;
            }
            pending.extend(
                initializations
                    .iter()
                    .filter(|r| r["from"] == body)
                    .filter_map(|r| r["to"].as_str()),
            );
            pending.extend(
                arguments
                    .iter()
                    .filter(|r| r["from"] == body)
                    .filter_map(|r| r["to"].as_str()),
            );
        }
        if !direct && !seen.contains(constructor_function.as_str()) {
            return Err("nested parent construction must follow initializer evaluations, never ordinary calls".into());
        }
        if !v["unknowns"].as_array().into_iter().flatten().any(|u| {
            u["fn"] == from
                && u["reasons"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|r| r == "unmodeled_initialization")
        }) {
            return Err("parent opportunity requires scoped invocation-context coverage".into());
        }
        if definitions
            .values()
            .any(|(declared, _, _, interface, empty)| *declared == owner && *interface && !empty)
        {
            return Err(
                "interface parent matching remains outside the portable nominal account".into(),
            );
        }
        if !constructor_symbols.contains(&(
            constructor_function.clone(),
            ordinal,
            child_symbol.clone(),
        )) {
            return Err(
                "parent child specialization has no actual constructor symbol account".into(),
            );
        }
        let before = expected.len();
        for (to, (declared, stage, child, _interface, empty)) in &definitions {
            if *declared == owner && !empty && child.as_ref().is_some_and(|c| c == &child_symbol) {
                expected.insert((index as u32, *stage, to.clone()));
            }
        }
        if expected.len() == before {
            return Err("parent context requires a matching nonempty callback".into());
        }
    }
    let mut actual = Vec::new();
    for row in rows("protocol_evaluations")? {
        only_keys(
            row,
            "protocol evaluation",
            &["context", "to", "stage", "guard"],
            &[],
        )?;
        let stage = stage(row)?;
        if row["guard"] != stage.guard() {
            return Err("parent guard must match its stage".into());
        }
        actual.push((number(row, "context")?, stage, text(row, "to")?));
    }
    if actual != expected.into_iter().collect::<Vec<_>>() {
        return Err(
            "parent evaluations must equal exactly their contexts' matching nonempty callbacks"
                .into(),
        );
    }
    Ok(())
}
