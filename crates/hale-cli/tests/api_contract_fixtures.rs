//! GH #1417, step R0: the API surface contract's consumer fixtures
//! (`tests/api-contract/`, spec/api.md § The witness) hold to the
//! contract before any producer of them exists.
//!
//! Every description and the inventory validate against
//! `spec/api-description.schema.json`. No JSON Schema crate is in the
//! workspace, so the validator here implements the keywords the schema
//! uses, and refuses a schema that uses one it does not implement, so
//! no keyword is silently unchecked; `the_validator_refuses_*` holds the
//! validator itself to the schema's refusals. Beside the schema, the
//! contract's laws over the fixtures: an exposure's identity, a
//! caller's description as exactly the filter of its surface's rows,
//! the digests `digest.md` folds, and each wire record's encoding of its
//! outcome. From R1 the compiler emits these documents, and the tests at
//! the end of this file hold what `hale check --api` prints over
//! `program.hl` to the same files, byte for byte.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

fn contract_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/api-contract")
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

fn schema() -> Value {
    read_json(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/api-description.schema.json"))
}

fn inventory() -> Value {
    read_json(&contract_dir().join("inventory.json"))
}

/// Every `<exposure>.<caller>.description.json`, by file name.
fn descriptions() -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(contract_dir()).expect("tests/api-contract") {
        let path = entry.expect("entry").path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        if name.ends_with(".description.json") {
            out.insert(name, read_json(&path));
        }
    }
    assert!(out.len() >= 8, "expected the eight descriptions of tests/api-contract, found {:?}", out.keys());
    out
}

/// Every `wire/<transport>/<outcome>.json`, as (transport, file stem, record).
fn wire_records() -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    for transport in ["unix", "http"] {
        let dir = contract_dir().join("wire").join(transport);
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .map(|e| e.expect("entry").path())
            .collect();
        paths.sort();
        for path in paths {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            out.push((transport.to_string(), stem, read_json(&path)));
        }
    }
    out
}

// ------------------------------------------------------------------
// The validator: the keywords spec/api-description.schema.json uses.

/// Annotation keywords: they constrain nothing, so validation skips
/// them. Any other keyword the validator does not implement is a panic.
const ANNOTATIONS: &[&str] = &["$schema", "$id", "$defs", "$comment", "title", "description", "default", "examples"];

/// Validate `v` against `schema`, resolving `$ref` as a JSON pointer
/// into `root`; every failure is pushed to `errs` with where it is.
fn validate(root: &Value, schema: &Value, v: &Value, at: &str, errs: &mut Vec<String>) {
    let Some(obj) = schema.as_object() else {
        match schema {
            Value::Bool(true) => {}
            Value::Bool(false) => errs.push(format!("{at}: no value is allowed here")),
            other => panic!("schema at {at} is not a schema: {other}"),
        }
        return;
    };
    for (key, sub) in obj {
        if ANNOTATIONS.contains(&key.as_str()) || key.starts_with("x-") {
            continue;
        }
        match key.as_str() {
            "$ref" => {
                let target = resolve(root, sub.as_str().expect("$ref is a string"));
                validate(root, target, v, at, errs);
            }
            "not" => {
                let mut e = Vec::new();
                validate(root, sub, v, at, &mut e);
                if e.is_empty() {
                    errs.push(format!("{at}: {v} is what `not` forbids"));
                }
            }
            "oneOf" | "anyOf" => {
                let branches = sub.as_array().expect("oneOf/anyOf is an array");
                let passing = branches
                    .iter()
                    .filter(|b| {
                        let mut e = Vec::new();
                        validate(root, b, v, at, &mut e);
                        e.is_empty()
                    })
                    .count();
                let ok = if key == "oneOf" { passing == 1 } else { passing >= 1 };
                if !ok {
                    errs.push(format!("{at}: {passing} of the {key} branches hold"));
                }
            }
            "type" => {
                let names: Vec<&str> = match sub {
                    Value::String(s) => vec![s.as_str()],
                    Value::Array(a) => a.iter().map(|t| t.as_str().expect("type name")).collect(),
                    other => panic!("type at {at} is {other}"),
                };
                if !names.iter().any(|t| is_type(v, t)) {
                    errs.push(format!("{at}: {v} is not {}", names.join(" or ")));
                }
            }
            "const" => {
                if v != sub {
                    errs.push(format!("{at}: {v} is not the const {sub}"));
                }
            }
            "enum" => {
                if !sub.as_array().expect("enum is an array").contains(v) {
                    errs.push(format!("{at}: {v} is not one of {sub}"));
                }
            }
            "required" => {
                if let Some(o) = v.as_object() {
                    for field in sub.as_array().expect("required is an array") {
                        let field = field.as_str().expect("required names a field");
                        if !o.contains_key(field) {
                            errs.push(format!("{at}: `{field}` is required"));
                        }
                    }
                }
            }
            "properties" => {
                if let Some(o) = v.as_object() {
                    for (field, fs) in sub.as_object().expect("properties is an object") {
                        if let Some(fv) = o.get(field) {
                            validate(root, fs, fv, &format!("{at}.{field}"), errs);
                        }
                    }
                }
            }
            "additionalProperties" => {
                if let Some(o) = v.as_object() {
                    let declared = obj.get("properties").and_then(Value::as_object);
                    for (field, fv) in o {
                        if declared.map_or(false, |d| d.contains_key(field)) {
                            continue;
                        }
                        validate(root, sub, fv, &format!("{at}.{field}"), errs);
                    }
                }
            }
            "items" => {
                if let Some(a) = v.as_array() {
                    for (i, item) in a.iter().enumerate() {
                        validate(root, sub, item, &format!("{at}[{i}]"), errs);
                    }
                }
            }
            "minItems" => {
                if let Some(a) = v.as_array() {
                    if (a.len() as u64) < sub.as_u64().expect("minItems") {
                        errs.push(format!("{at}: fewer than {sub} items"));
                    }
                }
            }
            "uniqueItems" => {
                if let (Some(a), Some(true)) = (v.as_array(), sub.as_bool()) {
                    for (i, x) in a.iter().enumerate() {
                        if a[..i].contains(x) {
                            errs.push(format!("{at}: {x} appears twice"));
                        }
                    }
                }
            }
            "minLength" => {
                if let Some(s) = v.as_str() {
                    if (s.chars().count() as u64) < sub.as_u64().expect("minLength") {
                        errs.push(format!("{at}: shorter than {sub}"));
                    }
                }
            }
            "minimum" => {
                if let Some(n) = v.as_f64() {
                    if n < sub.as_f64().expect("minimum") {
                        errs.push(format!("{at}: {n} is below {sub}"));
                    }
                }
            }
            "pattern" => {
                if let Some(s) = v.as_str() {
                    if !Pattern::parse(sub.as_str().expect("pattern")).matches(s) {
                        errs.push(format!("{at}: `{s}` does not match {sub}"));
                    }
                }
            }
            other => panic!("the schema uses `{other}` at {at}, which this validator does not implement"),
        }
    }
}

fn is_type(v: &Value, t: &str) -> bool {
    match t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        other => panic!("unknown JSON Schema type `{other}`"),
    }
}

/// A `$ref` within the document: `#` and a JSON pointer.
fn resolve<'a>(root: &'a Value, reference: &str) -> &'a Value {
    let pointer = reference
        .strip_prefix('#')
        .unwrap_or_else(|| panic!("`{reference}` is not a reference within the document"));
    root.pointer(pointer).unwrap_or_else(|| panic!("`{reference}` resolves to nothing"))
}

/// The anchored subset of a regular expression the schema's patterns
/// are written in: literals, `\`-escapes of punctuation, bracket classes
/// with ranges, and the quantifiers `*`, `+`, `?`, `{n}`, `{n,}`, `{n,m}`.
/// Anything else panics, so a pattern this cannot check fails the test.
struct Pattern {
    atoms: Vec<(Atom, usize, Option<usize>)>,
}

enum Atom {
    Char(char),
    Class(Vec<(char, char)>),
}

impl Atom {
    fn accepts(&self, c: char) -> bool {
        match self {
            Atom::Char(x) => *x == c,
            Atom::Class(ranges) => ranges.iter().any(|(lo, hi)| (*lo..=*hi).contains(&c)),
        }
    }
}

impl Pattern {
    fn parse(src: &str) -> Pattern {
        let body = src
            .strip_prefix('^')
            .and_then(|s| s.strip_suffix('$'))
            .unwrap_or_else(|| panic!("pattern `{src}` is not anchored at both ends"));
        let cs: Vec<char> = body.chars().collect();
        let mut atoms = Vec::new();
        let mut i = 0;
        while i < cs.len() {
            let atom = match cs[i] {
                '\\' => {
                    i += 1;
                    let c = cs[i];
                    assert!(!c.is_alphanumeric(), "pattern `{src}`: `\\{c}` is not supported");
                    Atom::Char(c)
                }
                '[' => {
                    let mut ranges = Vec::new();
                    i += 1;
                    while cs[i] != ']' {
                        let lo = if cs[i] == '\\' {
                            i += 1;
                            cs[i]
                        } else {
                            cs[i]
                        };
                        if cs[i + 1] == '-' && cs[i + 2] != ']' {
                            ranges.push((lo, cs[i + 2]));
                            i += 3;
                        } else {
                            ranges.push((lo, lo));
                            i += 1;
                        }
                    }
                    Atom::Class(ranges)
                }
                c @ ('(' | ')' | '|' | '.' | '^' | '$' | '*' | '+' | '?' | '{') => {
                    panic!("pattern `{src}`: `{c}` is not supported here")
                }
                c => Atom::Char(c),
            };
            i += 1;
            let (min, max) = match cs.get(i) {
                Some('*') => (0, None),
                Some('+') => (1, None),
                Some('?') => (0, Some(1)),
                Some('{') => {
                    let close = (i..cs.len()).find(|&j| cs[j] == '}').expect("unclosed {");
                    let inner: String = cs[i + 1..close].iter().collect();
                    let bounds = match inner.split_once(',') {
                        None => {
                            let n = inner.parse().expect("count");
                            (n, Some(n))
                        }
                        Some((lo, "")) => (lo.parse().expect("count"), None),
                        Some((lo, hi)) => (lo.parse().expect("count"), Some(hi.parse().expect("count"))),
                    };
                    i = close;
                    bounds
                }
                _ => {
                    atoms.push((atom, 1, Some(1)));
                    continue;
                }
            };
            i += 1;
            atoms.push((atom, min, max));
        }
        Pattern { atoms }
    }

    fn matches(&self, s: &str) -> bool {
        let cs: Vec<char> = s.chars().collect();
        self.from(0, &cs, 0)
    }

    /// Whether atoms `k..` match `cs[pos..]` whole: each atom takes as
    /// many characters as it can, and gives them back one at a time.
    fn from(&self, k: usize, cs: &[char], pos: usize) -> bool {
        let Some((atom, min, max)) = self.atoms.get(k) else {
            return pos == cs.len();
        };
        let mut n = 0;
        while pos + n < cs.len() && max.map_or(true, |m| n < m) && atom.accepts(cs[pos + n]) {
            n += 1;
        }
        (*min..=n).rev().any(|take| self.from(k + 1, cs, pos + take))
    }
}

fn errors_against_schema(doc: &Value) -> Vec<String> {
    let root = schema();
    let mut errs = Vec::new();
    validate(&root, &root, doc, "$", &mut errs);
    errs
}

// ------------------------------------------------------------------
// The documents.

#[test]
fn every_document_conforms_to_the_schema() {
    let mut docs: Vec<(String, Value)> = descriptions().into_iter().collect();
    docs.push(("inventory.json".to_string(), inventory()));
    for (name, doc) in &docs {
        let errs = errors_against_schema(doc);
        assert!(errs.is_empty(), "{name} does not conform to spec/api-description.schema.json:\n{}", errs.join("\n"));
    }
}

#[test]
fn the_validator_refuses_what_the_schema_forbids() {
    let base = read_json(&contract_dir().join("public.alice.description.json"));
    let mutations: Vec<(&str, Box<dyn Fn(&mut Value)>)> = vec![
        ("a required field missing", Box::new(|d| {
            d.as_object_mut().unwrap().remove("digest");
        })),
        ("a field the format does not have", Box::new(|d| {
            d["route"] = json!("/x");
        })),
        ("a digest of another form", Box::new(|d| d["digest"] = json!("fnv1a64:FE65E6D3036EE1EB"))),
        ("an HTTP status the v1 mapping does not give", Box::new(|d| d["outcomes"]["handler_error"]["status"] = json!(400))),
        ("a refusal kind the contract does not have", Box::new(|d| {
            d["outcomes"]["refusal"]["status"]["over_bound"] = json!(503);
        })),
        ("a role named twice", Box::new(|d| d["members"][0]["requires"] = json!(["trader", "trader"]))),
        ("a member that is not Locus::fn", Box::new(|d| d["members"][0]["name"] = json!("cancel"))),
        ("an exposure identity with no name", Box::new(|d| d["exposure"] = json!("Public@fnv1a64:fe65e6d3036ee1eb/"))),
        ("a version this format is not", Box::new(|d| d["description"] = json!(2))),
    ];
    for (what, mutate) in mutations {
        let mut doc = base.clone();
        mutate(&mut doc);
        assert!(!errors_against_schema(&doc).is_empty(), "the schema accepts {what}");
    }
    let mut inv = inventory();
    inv["exposures"][0]["receivers"] = json!([]);
    assert!(!errors_against_schema(&inv).is_empty(), "the schema accepts an exposure with no receiver");
    let mut inv = inventory();
    inv["hubs"][0]["streams"][0]["on_full"] = json!("refuse");
    assert!(!errors_against_schema(&inv).is_empty(), "the schema accepts `refuse` as a stream's on_full (decision 17: drop_old or drop_new)");
}

/// The inventory's row of the named surface, exposure, hub.
fn surface_of<'a>(inv: &'a Value, name: &str) -> &'a Value {
    inv["surfaces"].as_array().unwrap().iter().find(|s| s["name"] == name).unwrap_or_else(|| panic!("no surface `{name}` in the inventory"))
}

fn exposure_of<'a>(inv: &'a Value, id: &str) -> &'a Value {
    inv["exposures"].as_array().unwrap().iter().find(|e| e["exposure"] == id).unwrap_or_else(|| panic!("no exposure `{id}` in the inventory"))
}

fn hub_of<'a>(inv: &'a Value, id: &str) -> &'a Value {
    inv["hubs"].as_array().unwrap().iter().find(|h| h["exposure"] == id).unwrap_or_else(|| panic!("no hub exposure `{id}` in the inventory"))
}

/// A hub exposure's description: one with no surface.
fn is_hub(d: &Value) -> bool {
    d["surface"].is_null()
}

fn strings(v: &Value) -> BTreeSet<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

#[test]
fn an_exposure_is_its_surface_its_digest_and_its_name() {
    let inv = inventory();
    for e in inv["exposures"].as_array().unwrap() {
        let id = format!("{}@{}/{}", e["surface"].as_str().unwrap(), e["digest"].as_str().unwrap(), e["name"].as_str().unwrap());
        assert_eq!(e["exposure"], json!(id), "an inventory exposure's identity is surface@digest/name");
        assert_eq!(e["digest"], surface_of(&inv, e["surface"].as_str().unwrap())["digest"], "{id}: the exposure carries its surface's digest");
    }
    for h in inv["hubs"].as_array().unwrap() {
        let id = format!("hub@{}/{}", h["digest"].as_str().unwrap(), h["name"].as_str().unwrap());
        assert_eq!(h["exposure"], json!(id), "a hub exposure's identity is hub@<stream digest>/name");
    }
    let names: Vec<&str> = inv["exposures"].as_array().unwrap().iter().chain(inv["hubs"].as_array().unwrap()).map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), names.iter().collect::<BTreeSet<_>>().len(), "an exposure is named once, a hub's among them");

    let mut digest_of_exposure: BTreeMap<String, (String, String)> = BTreeMap::new();
    for (file, d) in descriptions() {
        let id = d["exposure"].as_str().unwrap();
        if is_hub(&d) {
            assert_eq!(id, format!("hub@{}/{}", d["digest"].as_str().unwrap(), d["name"].as_str().unwrap()), "{file}");
            assert!(file.starts_with(&format!("{}.", d["name"].as_str().unwrap())), "{file} is named by its exposure");
            let h = hub_of(&inv, id);
            for field in ["digest", "listener", "codec"] {
                assert_eq!(d[field], h[field], "{file}: `{field}` is the inventory hub's");
            }
            assert_eq!(d["outcomes"]["transport"], d["listener"]["transport"], "{file}: the outcome encoding is its listener's");
            continue;
        }
        assert_eq!(id, format!("{}@{}/{}", d["surface"].as_str().unwrap(), d["digest"].as_str().unwrap(), d["name"].as_str().unwrap()), "{file}");
        assert!(file.starts_with(&format!("{}.", d["name"].as_str().unwrap())), "{file} is named by its exposure");
        let e = exposure_of(&inv, id);
        for field in ["surface", "digest", "listener", "codec"] {
            assert_eq!(d[field], e[field], "{file}: `{field}` is the inventory's");
        }
        assert_eq!(d["outcomes"]["transport"], d["listener"]["transport"], "{file}: the outcome encoding is its listener's");
        digest_of_exposure.insert(d["name"].as_str().unwrap().to_string(), (d["surface"].as_str().unwrap().to_string(), d["digest"].as_str().unwrap().to_string()));
    }
    // The same surface served twice: one digest, two exposures (spec/api.md § The description).
    assert_eq!(digest_of_exposure["public"], digest_of_exposure["partner"]);
    assert_ne!(exposure_of(&inv, &format!("Public@{}/public", digest_of_exposure["public"].1))["roles"], exposure_of(&inv, &format!("Public@{}/partner", digest_of_exposure["partner"].1))["roles"], "the two exposures of Public are under two role sources");
    // The hub's grants are the hub's: its role source is none of the
    // rpc exposures', so `operator` through the hub (a bearer) and
    // through `admin` (a Unix peer) are two grants.
    for hub in inv["hubs"].as_array().unwrap() {
        for e in inv["exposures"].as_array().unwrap() {
            assert_ne!(hub["roles"], e["roles"], "hub {} shares its role source with exposure {}", hub["instance"], e["name"]);
        }
    }
}

/// Every `#/schemas/<T>` a value names, recursively.
fn referenced_types(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(o) => {
            if let Some(r) = o.get("$ref").and_then(Value::as_str) {
                out.insert(r.strip_prefix("#/schemas/").unwrap_or_else(|| panic!("`{r}` is not a schema of the document")).to_string());
            }
            o.values().for_each(|x| referenced_types(x, out));
        }
        Value::Array(a) => a.iter().for_each(|x| referenced_types(x, out)),
        _ => {}
    }
}

#[test]
fn a_description_lists_exactly_what_its_caller_may_call() {
    let inv = inventory();
    let mut differ: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (file, d) in descriptions() {
        let roles = strings(&d["caller"]["roles"]);
        // The rows the exposure offers: a surface's members (none for a
        // hub exposure), and the stream rows of every hub at its listener.
        let rows: Vec<&Value> = match d["surface"].as_str() {
            Some(name) => surface_of(&inv, name)["members"].as_array().unwrap().iter().collect(),
            None => Vec::new(),
        };
        let stream_rows: Vec<&Value> = inv["hubs"].as_array().unwrap().iter().filter(|h| h["listener"] == d["listener"]).flat_map(|h| h["streams"].as_array().unwrap()).collect();
        let may: Vec<&Value> = rows.iter().copied().filter(|m| strings(&m["requires"]).is_subset(&roles)).collect();
        let listed: Vec<&Value> = d["members"].as_array().unwrap().iter().collect();
        assert_eq!(listed, may, "{file}: the members are exactly the surface's rows whose requires the caller holds, in the surface's order");
        let may: Vec<&Value> = stream_rows.iter().copied().filter(|s| strings(&s["requires"]).is_subset(&roles)).collect();
        let listed: Vec<&Value> = d["streams"].as_array().unwrap().iter().collect();
        assert_eq!(listed, may, "{file}: the streams are exactly the stream rows of the hubs at its listener whose requires the caller holds");
        let named: BTreeSet<String> = rows.iter().chain(&stream_rows).flat_map(|m| strings(&m["requires"])).collect();
        assert!(roles.is_subset(&named), "{file}: the caller's roles are among those the exposure requires");

        let mut used = BTreeSet::new();
        referenced_types(&d["members"], &mut used);
        referenced_types(&d["streams"], &mut used);
        let carried: BTreeSet<String> = d["schemas"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(carried, used, "{file}: the document carries the schema of every type it names, and nothing else");
        for (t, s) in d["schemas"].as_object().unwrap() {
            assert_eq!(s, &inv["schemas"][t], "{file}: `{t}`'s schema is the inventory's");
        }
        differ.entry(d["caller"]["principal"]["name"].as_str().unwrap().to_string()).or_default().insert(
            d["members"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap()).collect::<Vec<_>>().join(","),
        );
    }
    // Grants belong to the role-source instance: `alice` holds `trader`
    // under `public` and nothing under `partner`, so her two descriptions
    // of the one surface differ.
    assert_eq!(differ["alice"].len(), 2, "alice's descriptions under public and partner differ: {:?}", differ["alice"]);
}

// ------------------------------------------------------------------
// The error column (F.42).

/// A `fallible(ClosureViolation)` handler's error column: its failure is
/// the server error, and no document carries a schema for it.
const STRUCTURAL: &str = "ClosureViolation";

/// The members of `program.hl` that may violate and return a value.
const VIOLATING: &[&str] = &["Orders::place", "Ledger::rebalance"];

/// Every member row a document lists, under the surface it is listed in.
fn member_rows(doc: &Value) -> Vec<&Value> {
    let mut out: Vec<&Value> = doc["members"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    for s in doc["surfaces"].as_array().into_iter().flatten() {
        out.extend(s["members"].as_array().unwrap());
    }
    out
}

#[test]
fn a_violating_handler_is_the_server_error_and_carries_no_error_schema() {
    let mut docs: Vec<(String, Value)> = descriptions().into_iter().collect();
    docs.push(("inventory.json".to_string(), inventory()));
    let mut seen = BTreeSet::new();
    for (file, doc) in &docs {
        for m in member_rows(doc) {
            let name = m["name"].as_str().unwrap();
            if VIOLATING.contains(&name) {
                assert_eq!(m["error"], json!(STRUCTURAL), "{file}: `{name}` is `fallible(ClosureViolation)`, so its error is the string");
                seen.insert(name.to_string());
            } else {
                assert_ne!(m["error"], json!(STRUCTURAL), "{file}: `{name}` does not violate");
            }
        }
        assert!(doc["schemas"].get(STRUCTURAL).is_none(), "{file}: no document carries a ClosureViolation schema");
    }
    assert_eq!(seen, VIOLATING.iter().map(|s| s.to_string()).collect(), "both violating handlers are listed somewhere");

    // Giving `place` an error schema is refused: its error type is
    // ClosureViolation, whose failure has none to give.
    let base = read_json(&contract_dir().join("public.alice.description.json"));
    let at = base["members"].as_array().unwrap().iter().position(|m| m["name"] == "Orders::place").expect("alice may place");
    let mut doc = base.clone();
    doc["members"][at]["error"] = json!({"$ref": "#/schemas/ClosureViolation"});
    assert!(!errors_against_schema(&doc).is_empty(), "the schema accepts a reference to a ClosureViolation schema");
    let mut doc = base.clone();
    doc["schemas"][STRUCTURAL] = json!({"type": "object", "properties": {"locus": {"type": "string"}, "closure": {"type": "string"}, "diff": {"type": "integer"}}});
    assert!(!errors_against_schema(&doc).is_empty(), "the schema accepts a ClosureViolation schema in the document");
    let mut doc = base.clone();
    doc["members"][at]["error"] = json!("OrderError");
    assert!(!errors_against_schema(&doc).is_empty(), "the schema accepts an error named by a string other than ClosureViolation");
}

// ------------------------------------------------------------------
// The digest.

/// The ```text block after the marker `<!-- <marker> -->` in digest.md.
fn block_after(md: &str, marker: &str) -> String {
    let at = md.find(&format!("<!-- {marker}")).unwrap_or_else(|| panic!("digest.md has no `{marker}` block"));
    let rest = &md[at..];
    let open = rest.find("```text\n").expect("a text block after the marker") + "```text\n".len();
    let close = rest[open..].find("```").expect("the block closes");
    rest[open..open + close].to_string()
}

fn hex_bytes(block: &str) -> Vec<u8> {
    let hex: String = block.chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(hex.len() % 2, 0, "an even count of hex digits");
    (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex")).collect()
}

#[test]
fn the_digests_are_the_ones_digest_md_folds() {
    let md = std::fs::read_to_string(contract_dir().join("digest.md")).expect("digest.md");
    let inv = inventory();

    let mut shape_hash: BTreeMap<String, String> = BTreeMap::new();
    for line in block_after(&md, "shapes").lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let [ty, shape, hash] = f[..] else { panic!("a shapes line is type, shape, hash: `{line}`") };
        assert_eq!(format!("{:016x}", hale_graph::identity::fnv64(shape.as_bytes())), hash, "{ty}: the hash is the fold of `{shape}`");
        shape_hash.insert(ty.to_string(), hash.to_string());
    }

    let mut folded = 0;
    for s in inv["surfaces"].as_array().unwrap() {
        let name = s["name"].as_str().unwrap();
        let bytes = hex_bytes(&block_after(&md, &format!("input: {name}")));
        let digest = format!("fnv1a64:{:016x}", hale_graph::identity::fnv64(&bytes));
        assert_eq!(s["digest"], json!(digest), "{name}: the inventory's digest is the fold of digest.md's input");
        assert!(md.contains(&digest), "digest.md states {name}'s digest, {digest}");

        let text = String::from_utf8(bytes).expect("the input is UTF-8");
        assert!(text.ends_with('\n'), "every line of the input ends with LF");
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("hale-api-surface 1"), "{name}: the header line");
        let rows: Vec<&str> = lines.collect();
        let members = s["members"].as_array().unwrap();
        assert_eq!(rows.len(), members.len(), "{name}: one line per row");
        let mut previous = String::new();
        for (row, m) in rows.iter().zip(members) {
            let f: Vec<&str> = row.split('\t').collect();
            let [member, request, response, error, requires] = f[..] else { panic!("{name}: a row is five TAB-separated fields: `{row}`") };
            assert!(previous.as_str() < member, "{name}: rows in byte order of member");
            previous = member.to_string();
            assert_eq!(m["name"], json!(member), "{name}: the input's rows are the surface's, in its order");
            for (field, slot) in [("request", request), ("response", response), ("error", error)] {
                // A `fallible(ClosureViolation)` handler's error is the
                // string, not a schema, and its slot is that record's hash.
                let expect = match (m[field]["$ref"].as_str(), m[field].as_str()) {
                    (Some(r), _) => shape_hash[r.strip_prefix("#/schemas/").unwrap()].clone(),
                    (None, Some(t)) => shape_hash[t].clone(),
                    (None, None) => "-".to_string(),
                };
                assert_eq!(slot, expect, "{name} {member}: the {field} slot");
            }
            let mut req: Vec<String> = strings(&m["requires"]).into_iter().collect();
            req.sort();
            let expect = if req.is_empty() { "-".to_string() } else { req.join(",") };
            assert_eq!(requires, expect, "{name} {member}: the requires slot");
        }
        folded += 1;
    }
    assert_eq!(folded, 2, "both surfaces are folded");

    // The stream digest of each hub (spec/api.md § Streams).
    for h in inv["hubs"].as_array().unwrap() {
        let name = h["name"].as_str().unwrap();
        let bytes = hex_bytes(&block_after(&md, &format!("input: hub {name}")));
        let digest = format!("fnv1a64:{:016x}", hale_graph::identity::fnv64(&bytes));
        assert_eq!(h["digest"], json!(digest), "hub {name}: the inventory's stream digest is the fold of digest.md's input");
        assert!(md.contains(&digest), "digest.md states hub {name}'s stream digest, {digest}");
        let text = String::from_utf8(bytes).expect("the input is UTF-8");
        assert!(text.ends_with('\n'), "every line of the input ends with LF");
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("hale-api-hub 1"), "hub {name}: the header line");
        let rows: Vec<&str> = lines.collect();
        let streams = h["streams"].as_array().unwrap();
        assert_eq!(rows.len(), streams.len(), "hub {name}: one line per stream row");
        let mut previous = String::new();
        for (row, st) in rows.iter().zip(streams) {
            let f: Vec<&str> = row.split('\t').collect();
            let [topic, payload, direction, codec, bound, on_full, replay, requires] = f[..] else { panic!("hub {name}: a row is eight TAB-separated fields: `{row}`") };
            assert!(previous.as_str() < topic, "hub {name}: rows in byte order of topic");
            previous = topic.to_string();
            assert_eq!(st["topic"], json!(topic), "hub {name}: the input's rows are the hub's, in its order");
            let payload_type = st["payload"]["$ref"].as_str().unwrap().strip_prefix("#/schemas/").unwrap();
            assert_eq!(payload, shape_hash[payload_type], "hub {name} {topic}: the payload slot");
            assert_eq!(st["direction"], json!(direction), "hub {name} {topic}: the direction slot");
            assert_eq!(st["codec"], json!(codec), "hub {name} {topic}: the codec slot");
            assert_eq!(st["bound"].to_string(), bound, "hub {name} {topic}: the bound slot, in decimal");
            assert_eq!(st["on_full"], json!(on_full), "hub {name} {topic}: the on_full slot");
            assert_eq!(if st["replay"] == json!(true) { "1" } else { "0" }, replay, "hub {name} {topic}: the replay slot");
            let req: Vec<String> = strings(&st["requires"]).into_iter().collect();
            assert_eq!(requires, if req.is_empty() { "-".to_string() } else { req.join(",") }, "hub {name} {topic}: the requires slot");
        }
    }
}

// ------------------------------------------------------------------
// The hub exposure (spec/api.md § Streams).

#[test]
fn a_hub_exposure_lists_a_stream_exactly_when_its_caller_holds_requires() {
    let inv = inventory();
    let docs = descriptions();
    let hub = &inv["hubs"][0];
    let fills = hub["streams"].as_array().unwrap().iter().find(|s| s["topic"] == "Fills").expect("the hub binds Fills");
    let id = format!("hub@{}/{}", hub["digest"].as_str().unwrap(), hub["name"].as_str().unwrap());

    let dave = &docs["fills.dave.description.json"];
    let bob = &docs["fills.bob.description.json"];
    for (who, d) in [("dave", dave), ("bob", bob)] {
        assert_eq!(d["exposure"], json!(id), "{who}: the identity is hub@<stream digest>/<name>");
        assert!(d["surface"].is_null(), "{who}: a hub exposure has no surface");
        assert_eq!(d["members"], json!([]), "{who}: a hub exposure has no member");
        assert_eq!(d["outcomes"]["transport"], json!("ws"), "{who}: the ws form");
        assert_eq!(d["caller"]["principal"]["name"], json!(who), "{who}: the caller");
    }
    // dave holds operator under hub_roles: Fills, with its payload's schema.
    assert!(strings(&fills["requires"]).is_subset(&strings(&dave["caller"]["roles"])), "dave holds what Fills requires");
    assert_eq!(dave["streams"], json!([fills]), "the description for dave lists Fills, as the inventory's row");
    assert_eq!(dave["schemas"]["Fill"], inv["schemas"]["Fill"], "the description for dave carries Fill's schema");
    // bob holds nothing there: no stream, and no payload schema either.
    assert!(!strings(&fills["requires"]).is_subset(&strings(&bob["caller"]["roles"])), "bob does not hold what Fills requires");
    assert_eq!(bob["streams"], json!([]), "the description for bob lists no stream");
    assert_eq!(bob["schemas"], json!({}), "the description for bob carries no payload schema");

    // The ws form admits nothing of an rpc's.
    let mutations: Vec<(&str, Box<dyn Fn(&mut Value)>)> = vec![
        ("a member", Box::new(|d| d["members"] = json!([{"name": "Orders::place", "request": null, "response": null, "error": null, "requires": []}]))),
        ("an HTTP status", Box::new(|d| d["outcomes"]["result"] = json!({"status": 200, "body": "response"}))),
        ("a status on a frame", Box::new(|d| d["outcomes"]["event"]["status"] = json!(200))),
        ("a surface", Box::new(|d| d["surface"] = json!("Public"))),
        ("a frame the envelope does not have", Box::new(|d| d["outcomes"]["ping"] = json!({"type": "ping", "fields": []}))),
        ("a refusal kind a subscription cannot meet", Box::new(|d| d["outcomes"]["refusal"]["kinds"] = json!(["malformed", "unauthenticated", "unauthorized", "full", "shutting_down"]))),
        ("an identity that is a surface's", Box::new(|d| d["exposure"] = json!("Fills@fnv1a64:26970854397ab154/fills"))),
    ];
    for (what, mutate) in mutations {
        let mut doc = dave.clone();
        mutate(&mut doc);
        assert!(!errors_against_schema(&doc).is_empty(), "the schema accepts a ws document with {what}");
    }
    // And a surface's exposure may not take the ws form.
    let mut doc = docs["public.alice.description.json"].clone();
    doc["outcomes"] = dave["outcomes"].clone();
    assert!(!errors_against_schema(&doc).is_empty(), "the schema accepts the ws form on a surface's exposure");
}

// ------------------------------------------------------------------
// The wire.

const REFUSALS: &[&str] =
    &["malformed", "digest_mismatch", "unauthenticated", "unauthorized", "full", "shutting_down", "unavailable"];

fn http_status(outcome: &str, kind: Option<&str>) -> u64 {
    match (outcome, kind) {
        ("result", None) => 200,
        ("handler_error", None) => 422,
        ("server_error", None) => 500,
        ("refusal", Some("malformed")) => 400,
        ("refusal", Some("digest_mismatch")) => 409,
        ("refusal", Some("unauthenticated")) => 401,
        ("refusal", Some("unauthorized")) => 403,
        ("refusal", Some("full")) => 429,
        ("refusal", Some("shutting_down")) => 503,
        ("refusal", Some("unavailable")) => 503,
        other => panic!("no outcome {other:?}"),
    }
}

/// `v` conforms to the payload schema `ty` names in the inventory.
fn conforms(inv: &Value, type_ref: &Value, v: &Value, what: &str) {
    let mut errs = Vec::new();
    validate(inv, type_ref, v, what, &mut errs);
    assert!(errs.is_empty(), "{what} does not decode by its shape:\n{}", errs.join("\n"));
}

#[test]
fn every_wire_record_encodes_its_outcome() {
    let inv = inventory();
    let mut seen: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (transport, stem, r) in wire_records() {
        let at = format!("wire/{transport}/{stem}.json");
        assert_eq!(r["wire"], json!(1), "{at}: the record's version");
        assert_eq!(r["transport"], json!(transport), "{at}: the record's transport is its directory");
        let outcome = r["outcome"].as_str().unwrap();
        let kind = r["kind"].as_str();
        match outcome {
            "refusal" => assert!(REFUSALS.contains(&kind.expect("a refusal names its kind")), "{at}: kind"),
            "result" | "handler_error" | "server_error" => assert!(kind.is_none(), "{at}: only a refusal has a kind"),
            other => panic!("{at}: `{other}` is no outcome a transport records"),
        }
        let expect_stem = kind.map_or(outcome.to_string(), |k| format!("refusal_{k}"));
        assert_eq!(stem, expect_stem, "{at}: a record is named by its outcome");
        seen.entry(transport.clone()).or_default().insert(stem.clone());

        let exposure = exposure_of(&inv, r["exposure"].as_str().unwrap());
        assert_eq!(exposure["listener"]["transport"], json!(transport), "{at}: the exposure listens on this transport");
        let served = exposure["digest"].as_str().unwrap();
        let surface = surface_of(&inv, exposure["surface"].as_str().unwrap());

        let (member_name, payload, sent_digest, reply) = match transport.as_str() {
            "unix" => {
                let req = &r["request"];
                let reply = &r["reply"];
                assert!(reply["request_id"].as_u64().unwrap_or(0) > 0, "{at}: a request is given a request_id");
                assert_eq!(reply["id"], req["id"], "{at}: the client's id is echoed");
                assert_eq!(reply["caller"]["mode"], json!("unix"), "{at}: the answer carries the principal");
                assert_eq!(reply["caller"]["uid"], r["peer"]["uid"], "{at}: the principal is the peer's credentials");
                match (outcome, kind) {
                    ("result", _) => {
                        assert_eq!(reply["ok"], json!(true), "{at}");
                        assert!(reply.get("value").is_some(), "{at}: a result carries value");
                    }
                    ("handler_error", _) => {
                        assert_eq!(reply["ok"], json!(false), "{at}");
                        assert!(reply.get("error").is_some(), "{at}: a handler error carries error");
                    }
                    ("server_error", _) => {
                        assert_eq!(reply["ok"], json!(false), "{at}");
                        assert_eq!(reply["refusal"], json!({"kind": "server"}), "{at}: the server error says nothing more");
                    }
                    (_, Some(k)) => {
                        assert_eq!(reply["ok"], json!(false), "{at}");
                        assert_eq!(reply["refusal"]["kind"], json!(k), "{at}");
                        assert!(!reply["refusal"]["reason"].as_str().unwrap_or("").is_empty(), "{at}: a refusal gives its reason");
                    }
                    _ => unreachable!(),
                }
                let body = match outcome {
                    "result" => reply["value"].clone(),
                    "handler_error" => reply["error"].clone(),
                    _ => reply["refusal"].clone(),
                };
                (req["call"].as_str().unwrap().to_string(), req["payload"].clone(), req["digest"].as_str().map(String::from), body)
            }
            "http" => {
                let req = &r["request"];
                let reply = &r["reply"];
                assert_eq!(req["method"], json!("POST"), "{at}: a call is a POST");
                let path = req["path"].as_str().unwrap();
                let member = path.strip_prefix("/call/").unwrap_or_else(|| panic!("{at}: a call's path is /call/<member>"));
                assert!(req["headers"]["Authorization"].as_str().unwrap_or("").starts_with("Bearer "), "{at}: the bearer");
                assert_eq!(reply["status"], json!(http_status(outcome, kind)), "{at}: the v1 status");
                match (outcome, kind) {
                    ("server_error", _) => assert_eq!(reply["body"], json!({"refusal": {"kind": "server"}}), "{at}"),
                    (_, Some(k)) => {
                        assert_eq!(reply["body"]["refusal"]["kind"], json!(k), "{at}");
                        assert!(!reply["body"]["refusal"]["reason"].as_str().unwrap_or("").is_empty(), "{at}: a refusal gives its reason");
                    }
                    _ => {}
                }
                let body = match outcome {
                    "result" | "handler_error" => reply["body"].clone(),
                    _ => reply["body"]["refusal"].clone(),
                };
                (member.to_string(), req["body"].clone(), req["headers"]["Hale-Surface-Digest"].as_str().map(String::from), body)
            }
            _ => unreachable!(),
        };

        let row = surface["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == member_name)
            .unwrap_or_else(|| panic!("{at}: `{member_name}` is no member of the exposure's surface"));
        match (outcome, kind) {
            ("result", _) => {
                conforms(&inv, &row["request"], &payload, &format!("{at}: the request"));
                conforms(&inv, &row["response"], &reply, &format!("{at}: the response"));
            }
            ("handler_error", _) => {
                assert!(row["error"].is_object(), "{at}: only a member with an error schema has a handler error");
                conforms(&inv, &row["error"], &reply, &format!("{at}: the error"));
            }
            ("server_error", _) => {
                assert_eq!(row["error"], json!(STRUCTURAL), "{at}: a server error is a `fallible(ClosureViolation)` member's failure");
            }
            (_, Some("digest_mismatch")) => {
                assert_eq!(reply["served"], json!(served), "{at}: the refusal names the served digest");
                assert_ne!(sent_digest.as_deref(), Some(served), "{at}: the request carried another digest");
            }
            (_, Some("unauthorized")) => {
                assert_eq!(reply["requires"], row["requires"], "{at}: the refusal names the row's requires");
            }
            _ => {}
        }
        if kind != Some("digest_mismatch") {
            if let Some(d) = &sent_digest {
                assert_eq!(d, served, "{at}: a digest the request carries is the served one");
            }
        }
    }
    let mut every: BTreeSet<String> = ["result", "handler_error", "server_error"].iter().map(|s| s.to_string()).collect();
    every.extend(REFUSALS.iter().map(|k| format!("refusal_{k}")));
    for transport in ["unix", "http"] {
        assert_eq!(seen.get(transport), Some(&every), "{transport}: one record per outcome and per refusal kind");
    }
}

// ------------------------------------------------------------------
// R1: the compiler produces the documents from the rows (R1's exit
// criterion: byte for byte the fixtures above).

/// `hale <args>` over `program.hl`, its stdout; the run succeeded.
fn hale(args: &[&str]) -> String {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(contract_dir().join("program.hl"))
        .args(args)
        .env("HALE_SKIP_STALE_CHECK", "1")
        .output()
        .expect("run hale");
    assert!(out.status.success(), "hale check {args:?}:\n{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("UTF-8")
}

fn fixture_text(name: &str) -> String {
    std::fs::read_to_string(contract_dir().join(name)).unwrap_or_else(|e| panic!("read {name}: {e}"))
}

/// Each description fixture's exposure and caller: the principal the
/// exposure establishes, and the roles the caller holds under that
/// exposure's role source. What a caller holds is its role source's to
/// say when the program runs (`Grants::holds` is program code), so the
/// check takes it as an input (`--holds`); everything else is the rows'.
const CALLERS: &[(&str, &str, &str, &str)] = &[
    ("admin.uid-1000.description.json", "admin", r#"{"mode": "unix", "name": "uid:1000", "uid": 1000, "gid": 1000, "pid": 4242}"#, "operator"),
    ("admin.uid-1001.description.json", "admin", r#"{"mode": "unix", "name": "uid:1001", "uid": 1001, "gid": 1001, "pid": 4242}"#, ""),
    ("fills.bob.description.json", "fills", "bob", ""),
    ("fills.dave.description.json", "fills", "dave", "operator"),
    ("partner.alice.description.json", "partner", "alice", ""),
    ("partner.carol.description.json", "partner", "carol", "trader"),
    ("public.alice.description.json", "public", "alice", "trader"),
    ("public.bob.description.json", "public", "bob", ""),
];

#[test]
fn the_compiler_prints_the_inventory_byte_for_byte() {
    let got = hale(&["--api"]);
    assert_eq!(got, fixture_text("inventory.json"), "`hale check --api program.hl` is inventory.json");
    let doc: Value = serde_json::from_str(&got).expect("JSON");
    assert!(errors_against_schema(&doc).is_empty());
}

#[test]
fn the_compiler_prints_every_description_byte_for_byte() {
    let docs = descriptions();
    let named: BTreeSet<&str> = CALLERS.iter().map(|c| c.0).collect();
    assert_eq!(named, docs.keys().map(String::as_str).collect(), "a caller for every description fixture");
    for (file, exposure, caller, holds) in CALLERS {
        let mut args = vec!["--api", "--exposure", exposure, "--caller", caller];
        if !holds.is_empty() {
            args.extend(["--holds", holds]);
        }
        let got = hale(&args);
        assert_eq!(got, fixture_text(file), "{file}: the compiler's description is the fixture's, byte for byte");
        let doc: Value = serde_json::from_str(&got).expect("JSON");
        assert!(errors_against_schema(&doc).is_empty(), "{file}");
    }
}

/// R7: the same exposure served over `grpc::Rpc` is described with its
/// own listener and its own outcome encoding (spec/api.md § Outcomes, the
/// gRPC column), conforms to the schema, and is otherwise the HTTP
/// exposure's document: the members, the schemas and the caller do not
/// depend on the transport.
#[test]
fn a_grpc_exposure_is_described_with_the_grpc_outcomes() {
    let program = std::fs::read_to_string(contract_dir().join("program.hl")).expect("program.hl");
    let swapped = program.replacen("http::Rpc { bind: \"127.0.0.1:8080\"", "grpc::Rpc { bind: \"127.0.0.1:8080\"", 1);
    assert_ne!(swapped, program, "the contract still serves public over http::Rpc at :8080");
    let got = hale_over("grpc_described", &swapped, &["--api", "--exposure", "public", "--caller", "alice", "--holds", "trader"]);
    assert!(errors_against_schema(&got).is_empty(), "{:?}", errors_against_schema(&got));
    assert_eq!(got["listener"], json!({"transport": "grpc", "address": "127.0.0.1:8080"}));
    assert_eq!(
        got["outcomes"],
        json!({
            "transport": "grpc",
            "result": {"status": "OK", "body": "response"},
            "handler_error": {"status": "FAILED_PRECONDITION", "details": "error"},
            "refusal": {
                "details": "refusal",
                "status": {
                    "malformed": "INVALID_ARGUMENT",
                    "digest_mismatch": "FAILED_PRECONDITION",
                    "unauthenticated": "UNAUTHENTICATED",
                    "unauthorized": "PERMISSION_DENIED",
                    "full": "RESOURCE_EXHAUSTED",
                    "shutting_down": "UNAVAILABLE",
                    "unavailable": "UNAVAILABLE"
                }
            },
            "server_error": {"status": "INTERNAL", "details": "refusal"},
            "transport_failure": "the transport's own: the stream is reset or the connection ends without a status"
        })
    );
    let http: Value = serde_json::from_str(&hale(&["--api", "--exposure", "public", "--caller", "alice", "--holds", "trader"])).expect("JSON");
    for key in ["members", "schemas", "caller", "digest", "surface", "codec", "streams", "notes"] {
        assert_eq!(got[key], http[key], "`{key}` does not depend on the transport");
    }
}

/// The model's digests are the ones digest.md folds by hand, and each
/// row's shape hashes are digest.md's shapes.
#[test]
fn the_compilers_digests_are_digest_mds() {
    let md = fixture_text("digest.md");
    let dump = hale(&["--dump-model"]);
    let section: Vec<&str> = dump.lines().skip_while(|l| !l.starts_with("surfaces (")).collect();
    for (surface, digest) in [("Admin", "fnv1a64:40381db6685c9f75"), ("Public", "fnv1a64:a8930d6e7998e986")] {
        assert!(md.contains(digest), "digest.md states {surface}'s digest");
        assert!(section.contains(&format!("  {surface} {digest}").as_str()), "the model's {surface} digest is {digest}");
    }
    for line in block_after(&md, "shapes").lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let (ty, hash) = (f[0], f[2]);
        if ty == "Fill" {
            continue; // a stream's payload, no row's type
        }
        assert!(section.iter().any(|l| l.contains(&format!("{ty} #{hash}"))), "a row names {ty} with its shape hash {hash}");
    }
}

/// The OpenAPI, JSON Schema and MCP forms of each surface are
/// projections of its rows, pinned beside the R0 documents.
#[test]
fn the_surface_projections_are_their_fixtures() {
    for surface in ["Admin", "Public"] {
        for form in ["openapi", "json-schema", "mcp"] {
            let file = format!("{surface}.{form}.json");
            let got = hale(&["--api", "--surface", surface, &format!("--{form}")]);
            assert_eq!(got, fixture_text(&file), "{file}");
        }
        // The protobuf form is text: the `.proto` the gRPC transport speaks.
        assert_eq!(hale(&["--api", "--surface", surface, "--proto"]), fixture_text(&format!("{surface}.proto")), "{surface}.proto");
        // The forms carry the surface's digest and list exactly its rows.
        let inv = inventory();
        let s = surface_of(&inv, surface);
        let members: BTreeSet<String> =
            s["members"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap().to_string()).collect();
        let openapi: Value = serde_json::from_str(&fixture_text(&format!("{surface}.openapi.json"))).unwrap();
        assert_eq!(openapi["info"]["x-hale-digest"], s["digest"]);
        let paths: BTreeSet<String> = openapi["paths"]
            .as_object()
            .unwrap()
            .keys()
            .map(|p| p.strip_prefix("/call/").unwrap().to_string())
            .collect();
        assert_eq!(paths, members, "{surface}: a path per row");
        for (name, op) in openapi["paths"].as_object().unwrap() {
            let row = &s["members"].as_array().unwrap().iter().find(|m| format!("/call/{}", m["name"].as_str().unwrap()) == *name).unwrap();
            let responses = op["post"]["responses"].as_object().unwrap();
            assert_eq!(responses.contains_key("500"), row["error"] == json!(STRUCTURAL), "{surface} {name}: the server error is a ClosureViolation row's");
            assert_eq!(responses.contains_key("422"), row["error"].is_object(), "{surface} {name}: the handler error is an E row's");
        }
        let mcp: Value = serde_json::from_str(&fixture_text(&format!("{surface}.mcp.json"))).unwrap();
        let tools: BTreeSet<String> = mcp["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().replace("__", "::"))
            .collect();
        assert_eq!(tools, members, "{surface}: a tool per row");
    }
}

// ------------------------------------------------------------------
// The generators over programs of their own: what the fixture program
// does not use (a builtin record, a user type named like a generated
// component, a scalar request).

/// `hale check <src> <args>` over a program written for the test.
fn hale_over(name: &str, src: &str, args: &[&str]) -> Value {
    let dir = std::env::temp_dir().join(format!("hale_api_gen_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("program.hl");
    std::fs::write(&file, src).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&file)
        .args(args)
        .env("HALE_SKIP_STALE_CHECK", "1")
        .output()
        .expect("run hale");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "hale check {args:?}:\n{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("JSON")
}

#[test]
fn a_builtin_record_is_its_contract_shape_fields() {
    let src = r#"
locus Echo { fn echo(x: IndexError) -> IndexError { return x; } }
type Wrap { e: IndexError; n: Int; }
locus W { fn wrap(w: Wrap) -> Wrap { return w; } }
api Public { rpc Echo::echo; rpc W::wrap; }
fn main() { }
"#;
    let inv = hale_over("builtin", src, &["--api"]);
    let schemas = &inv["schemas"];
    assert_eq!(
        schemas["IndexError"],
        json!({
            "type": "object",
            "properties": {
                "kind": {"type": "string"}, "index": {"type": "integer"}, "len": {"type": "integer"}
            },
            "required": ["kind", "index", "len"]
        })
    );
    assert_eq!(schemas["Wrap"]["properties"]["e"], json!({"$ref": "#/schemas/IndexError"}));
    assert_eq!(schemas["Wrap"]["properties"]["n"], json!({"type": "integer"}));
    let members = inv["surfaces"][0]["members"].as_array().unwrap();
    let echo = members.iter().find(|m| m["name"] == "Echo::echo").unwrap();
    assert_eq!(echo["request"], json!({"$ref": "#/schemas/IndexError"}));
    assert_eq!(echo["response"], json!({"$ref": "#/schemas/IndexError"}));
}

#[test]
fn a_closure_violation_error_member_is_the_string_and_no_schema() {
    let src = r#"
locus Lv { fn go(x: Int) -> Int fallible(ClosureViolation) { return x; } }
api Public { rpc Lv::go; }
fn main() { }
"#;
    let inv = hale_over("violation", src, &["--api"]);
    let go = &inv["surfaces"][0]["members"][0];
    assert_eq!(go["error"], json!("ClosureViolation"));
    assert!(inv["schemas"].get("ClosureViolation").is_none(), "{}", inv["schemas"]);
}

#[test]
fn a_user_type_named_refusal_does_not_collide_with_the_generated_one() {
    let src = r#"
type Refusal { id: Int; }
locus R { fn go(r: Refusal) -> Refusal { return r; } }
api Public { rpc R::go; }
fn main() { }
"#;
    let doc = hale_over("refusal", src, &["--api", "--surface", "Public", "--openapi"]);
    let schemas = doc["components"]["schemas"].as_object().unwrap();
    assert_eq!(schemas.keys().map(String::as_str).collect::<Vec<_>>(), ["Refusal", "hale.Refusal"]);
    assert!(schemas["Refusal"]["properties"].get("id").is_some(), "the user's");
    assert!(schemas["hale.Refusal"]["properties"].get("refusal").is_some(), "the generated one");
    let op = &doc["paths"]["/call/R::go"]["post"];
    let r = |v: &Value| v["content"]["application/json"]["schema"]["$ref"].clone();
    assert_eq!(r(&op["requestBody"]), json!("#/components/schemas/Refusal"));
    assert_eq!(r(&op["responses"]["200"]), json!("#/components/schemas/Refusal"));
    assert_eq!(r(&op["responses"]["400"]), json!("#/components/schemas/hale.Refusal"));
    assert_eq!(r(&op["responses"]["503"]), json!("#/components/schemas/hale.Refusal"));
}

#[test]
fn an_mcp_input_is_an_object() {
    let src = r#"
type Order { id: Int; }
locus S { fn echo(x: Int) -> Int { return x; } fn place(o: Order) -> Int { return o.id; } }
api Public { rpc S::echo; rpc S::place; }
fn main() { }
"#;
    let doc = hale_over("mcp", src, &["--api", "--surface", "Public", "--mcp"]);
    let tool = |n: &str| doc["tools"].as_array().unwrap().iter().find(|t| t["name"] == n).unwrap().clone();
    assert_eq!(
        tool("S__echo")["inputSchema"],
        json!({"type": "object", "properties": {"x": {"type": "integer"}}, "required": ["x"]})
    );
    assert_eq!(
        tool("S__place")["inputSchema"],
        json!({"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]})
    );
}

// ------------------------------------------------------------------
// R8a: `hale api export`, a surface's bundle (spec/api.md § The
// clients): the forms of the contract, generated from the rows, with the
// digest, deterministic and checkable.

fn export_cmd(program: &Path, surface: &str, extra: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["api", "export", "--surface", surface])
        .args(extra)
        .arg(program)
        .env("HALE_SKIP_STALE_CHECK", "1")
        .output()
        .expect("run hale")
}

/// A scratch directory of this test's own.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hale_api_export_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn export_to(program: &Path, surface: &str, dir: &Path) {
    let out = export_cmd(program, surface, &["--out", dir.to_str().unwrap()]);
    assert!(out.status.success(), "hale api export {surface}:\n{}", String::from_utf8_lossy(&out.stderr));
}

/// The bundle's three forms are the contract's fixtures, its description
/// is the schema's inventory form of that surface, and its DIGEST is the
/// digest every document of the surface carries.
#[test]
fn the_witness_bundle_is_the_contract_fixtures() {
    let dir = scratch("witness");
    for surface in ["Admin", "Public"] {
        export_to(&contract_dir().join("program.hl"), surface, &dir);
        for form in ["openapi", "json-schema", "mcp"] {
            let file = format!("{surface}.{form}.json");
            let got = std::fs::read_to_string(dir.join(&file)).unwrap();
            assert_eq!(got, fixture_text(&file), "{file}");
        }
        assert_eq!(std::fs::read_to_string(dir.join(format!("{surface}.proto"))).unwrap(), fixture_text(&format!("{surface}.proto")));
        let description: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{surface}.description.json"))).unwrap()).unwrap();
        let errs = errors_against_schema(&description);
        assert!(errs.is_empty(), "{surface}.description.json does not conform to the schema:\n{}", errs.join("\n"));
        // every member of the surface, with its roles; only that surface
        let inv = inventory();
        let want = surface_of(&inv, surface);
        assert_eq!(description["surfaces"], json!([want]), "{surface}: its row table, every member with its roles");
        assert_eq!(description["inventory"], json!(1));
        for ex in description["exposures"].as_array().unwrap() {
            assert_eq!(ex["surface"], json!(surface), "{surface}: an exposure of another surface is not its");
        }
        assert_eq!(description["hubs"], inv["hubs"], "{surface}: the program's hubs carry the streams a client subscribes to");
        let digest = std::fs::read_to_string(dir.join("DIGEST")).unwrap();
        let mut lines = digest.lines();
        assert_eq!(lines.next(), want["digest"].as_str(), "{surface}: DIGEST names the surface's digest");
        assert!(lines.next().is_some_and(|l| l.starts_with("hale ")), "{surface}: DIGEST names the compiler's version");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The same surface yields the same bytes on two runs and in two
/// checkouts (a copy of the program at another path).
#[test]
fn a_bundle_is_the_same_bytes_on_two_runs_and_two_checkouts() {
    let a = scratch("det_a");
    let b = scratch("det_b");
    let checkout = scratch("det_checkout");
    let elsewhere = checkout.join("a/deeper/checkout");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let copy = elsewhere.join("program.hl");
    std::fs::copy(contract_dir().join("program.hl"), &copy).unwrap();
    export_to(&contract_dir().join("program.hl"), "Public", &a);
    export_to(&copy, "Public", &b);
    let mut names: Vec<String> = std::fs::read_dir(&a).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    assert_eq!(names.len(), 6, "{names:?}");
    for n in &names {
        assert_eq!(std::fs::read(a.join(n)).unwrap(), std::fs::read(b.join(n)).unwrap(), "{n} differs between two checkouts");
        let text = std::fs::read_to_string(a.join(n)).unwrap();
        assert!(!text.contains("hale_api_export"), "{n} names a path of the checkout");
    }
    // and a second run over the same program, into the same directory
    export_to(&contract_dir().join("program.hl"), "Public", &a);
    for n in &names {
        assert_eq!(std::fs::read(a.join(n)).unwrap(), std::fs::read(b.join(n)).unwrap(), "{n} differs between two runs");
    }
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
    let _ = std::fs::remove_dir_all(&checkout);
}

/// `--check` is silent on a current bundle and, after a row is edited,
/// refuses naming the digest that moved and the forms that differ.
#[test]
fn check_refuses_a_bundle_after_a_row_is_edited() {
    let dir = scratch("drift");
    let program = dir.join("program.hl");
    let src = std::fs::read_to_string(contract_dir().join("program.hl")).unwrap();
    std::fs::write(&program, &src).unwrap();
    let bundle = dir.join("bundle");
    export_to(&program, "Public", &bundle);
    let ok = export_cmd(&program, "Public", &["--check", bundle.to_str().unwrap()]);
    assert!(ok.status.success(), "a current bundle is current:\n{}", String::from_utf8_lossy(&ok.stderr));
    // edit a row: Orders::cancel needs operator, not trader
    let edited = src.replace("rpc Orders::cancel requires: [trader];", "rpc Orders::cancel requires: [operator];");
    assert_ne!(edited, src);
    std::fs::write(&program, &edited).unwrap();
    let out = export_cmd(&program, "Public", &["--check", bundle.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stderr));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("the surface Public moved"), "{err}");
    assert!(err.contains("fnv1a64:a8930d6e7998e986"), "names the committed digest: {err}");
    assert!(err.contains("Public.openapi.json differs"), "names the forms that differ: {err}");
    // a missing file is drift too
    std::fs::write(&program, &src).unwrap();
    std::fs::remove_file(bundle.join("Public.mcp.json")).unwrap();
    let out = export_cmd(&program, "Public", &["--check", bundle.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Public.mcp.json is missing"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// An imported type is named by its declared path under the import alias,
/// never by the mangled name that embeds the library's location.
#[test]
fn an_imported_type_is_named_by_its_path_under_the_alias() {
    let root = scratch("imported");
    let write = |dir: &Path| {
        let lib = dir.join("lib");
        let app = dir.join("app");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(
            lib.join("lib.hl"),
            "type Item { name: String; count: Int; }\ntype Tag = distinct Int;\ntype Stamped { item: Item; tag: Tag; }\n",
        )
        .unwrap();
        let main = [
            "import \"../lib\" as lib;",
            "locus Shelf { fn put(i: lib::Item) -> lib::Stamped { return lib::Stamped { item: i, tag: lib::Tag(1) }; } }",
            "api Store { rpc Shelf::put; }",
            "main locus App {",
            "    params { shelf: Shelf = Shelf { }; }",
            "    run() {",
            "        let h = api::serve(Store, unix::Rpc { path: \"/tmp/hale-r8a-store.sock\" }, as: \"store\", receivers: { Shelf: self.shelf }, bound: 8, on_full: refuse);",
            "        while !self.draining { std::time::sleep(100ms); }",
            "        h.stop();",
            "    }",
            "}",
            "fn main() { App { }; }",
            "",
        ]
        .join("\n");
        std::fs::write(app.join("main.hl"), main).unwrap();
        app.join("main.hl")
    };
    let one = write(&root.join("one"));
    let two = write(&root.join("a/much/deeper/checkout"));
    let (a, b) = (root.join("out_a"), root.join("out_b"));
    export_to(&one, "Store", &a);
    export_to(&two, "Store", &b);
    for n in ["Store.description.json", "Store.openapi.json", "Store.json-schema.json", "Store.mcp.json", "Store.proto", "DIGEST"] {
        let text = std::fs::read_to_string(a.join(n)).unwrap();
        assert!(!text.contains("__lib_"), "{n} embeds a mangled name:\n{text}");
        assert_eq!(text, std::fs::read_to_string(b.join(n)).unwrap(), "{n} differs between two checkouts");
    }
    let js: Value = serde_json::from_str(&std::fs::read_to_string(a.join("Store.json-schema.json")).unwrap()).unwrap();
    let defs = js["$defs"].as_object().unwrap();
    assert!(defs.contains_key("lib::Item") && defs.contains_key("lib::Stamped"), "{:?}", defs.keys().collect::<Vec<_>>());
    assert_eq!(defs["lib::Stamped"]["properties"]["item"]["$ref"], json!("#/$defs/lib::Item"));
    assert_eq!(defs["lib::Stamped"]["properties"]["tag"]["x-hale-type"], json!("lib::Tag"));
    // the protobuf form spells the `::` of an imported type `_`, and says what it was
    let proto = std::fs::read_to_string(a.join("Store.proto")).unwrap();
    assert!(proto.contains("message lib_Stamped {\n  lib_Item item = 1;\n"), "{proto}");
    assert!(proto.contains("// Hale: lib::Item"), "{proto}");
    assert!(proto.contains("optional int64 tag = 2; // Hale: lib::Tag"), "{proto}");
    assert!(proto.contains("rpc Shelf__put(lib_Item) returns (lib_Stamped);"), "{proto}");
    let _ = std::fs::remove_dir_all(&root);
}

/// The corpus's own served surface, the DNA's head commands, exports a
/// `.proto` (every shape its rows name has a proto3 encoding), the same bytes
/// twice, with a message for each of its records and an rpc for each member.
#[test]
fn the_dna_head_commands_surface_exports_a_proto() {
    let program = contract_dir().join("../../dna/api");
    let (a, b) = (scratch("dna_proto_a"), scratch("dna_proto_b"));
    export_to(&program, "HeadCommands", &a);
    export_to(&program, "HeadCommands", &b);
    let proto = std::fs::read_to_string(a.join("HeadCommands.proto")).unwrap();
    assert_eq!(proto, std::fs::read_to_string(b.join("HeadCommands.proto")).unwrap(), "two runs, the same bytes");
    assert!(proto.contains("service HeadCommands {"), "{proto}");
    let json: Value = serde_json::from_str(&std::fs::read_to_string(a.join("HeadCommands.json-schema.json")).unwrap()).unwrap();
    for name in json["$defs"].as_object().unwrap().keys() {
        assert!(proto.contains(&format!("\nmessage {} {{", name.replace("::", "_"))), "a message for {name}");
    }
    for name in json["x-hale-members"].as_object().unwrap().keys() {
        assert!(proto.contains(&format!("  rpc {}(", name.replace("::", "__"))), "an rpc for {name}");
    }
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}

/// The rules of the protobuf form over shapes the witness does not use: a
/// scalar request, response and error (carried in one-field messages), a row
/// that takes and returns nothing, a field with a default, a `json:` tag, a
/// record that reaches another, and two members whose rpc names share a
/// receiver. Field numbers are the declaration order; every scalar is
/// `optional`; a record is a field of its message type, not an optional one.
#[test]
fn the_protobuf_form_follows_the_rules_over_shapes_of_its_own() {
    let dir = scratch("proto_rules");
    let program = dir.join("program.hl");
    std::fs::write(
        &program,
        [
            "type Inner { a: Int; b: String; }",
            "type Outer { name: String; inner: Inner; ratio: Float = 0.5; live: Bool = false; id: Int `json:\"ident\"`; }",
            "locus Shelf {",
            "    fn size(n: Int) -> Int { return n; }",
            "    fn nothing() { }",
            "    fn name_of(o: Outer) -> String fallible(String) { return o.name; }",
            "    fn plain(o: Outer) -> Outer { return o; }",
            "}",
            "api Store { rpc Shelf::size; rpc Shelf::nothing; rpc Shelf::name_of; rpc Shelf::plain; }",
            "main locus App {",
            "    params { shelf: Shelf = Shelf { }; }",
            "    run() {",
            "        let h = api::serve(Store, unix::Rpc { path: \"/tmp/hale-r8b-store.sock\" }, as: \"store\", receivers: { Shelf: self.shelf }, bound: 8, on_full: refuse);",
            "        while !self.draining { std::time::sleep(100ms); }",
            "        h.stop();",
            "    }",
            "}",
            "fn main() { App { }; }",
            "",
        ]
        .join("\n"),
    )
    .unwrap();
    export_to(&program, "Store", &dir.join("out"));
    let proto = std::fs::read_to_string(dir.join("out/Store.proto")).unwrap();
    let body: Vec<&str> = proto.lines().filter(|l| !l.starts_with("//")).collect();
    let body = body.join("\n");
    // one rpc per member, named as an MCP tool is; a scalar rides in a one-field message
    assert!(body.contains("rpc Shelf__size(Shelf__sizeRequest) returns (Shelf__sizeResponse);"), "{body}");
    assert!(body.contains("rpc Shelf__nothing(HaleEmpty) returns (HaleEmpty);"), "{body}");
    assert!(body.contains("rpc Shelf__name_of(Outer) returns (Shelf__name_ofResponse);"), "{body}");
    assert!(body.contains("message Shelf__sizeRequest {\n  optional int64 value = 1;\n}"), "{body}");
    assert!(body.contains("message Shelf__name_ofError {\n  optional string value = 1;\n}"), "{body}");
    assert!(proto.contains("handler error: status 9, a detail of type type.hale.dev/Shelf__name_ofError"), "{proto}");
    // numbers are the declaration order, the `json:` tag names the field, a record is a message field
    assert!(
        body.contains(
            "message Outer {\n  optional string name = 1;\n  Inner inner = 2;\n  optional double ratio = 3;\n  optional bool live = 4;\n  optional int64 ident = 5;\n}"
        ),
        "{body}"
    );
    assert!(body.contains("message Inner {\n  optional int64 a = 1;\n  optional string b = 2;\n}"), "{body}");
    // the refusal's message is fixed
    assert!(body.contains("message HaleRefusal {\n  string kind = 1;\n  string reason = 2;\n  repeated string requires = 3;\n  string served = 4;\n}"), "{body}");
    // and `--check` names a `.proto` that drifted
    let out = export_cmd(&program, "Store", &["--check", dir.join("out").to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    std::fs::write(dir.join("out/Store.proto"), proto.replace("= 5;", "= 6;")).unwrap();
    let out = export_cmd(&program, "Store", &["--check", dir.join("out").to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Store.proto differs"), "{}", String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two fields of a record whose default proto3 JSON names collide (`a_b` and
/// `aB` are both `aB`; protoc compares them without regard to case, so `a` and
/// `_a` are `a` and `A`) make a `.proto` protoc refuses. `check --api
/// --surface X --proto` and `api export` refuse the record with both fields and
/// the name, and a control whose names differ is written. The JSON, OpenAPI,
/// schema and MCP forms have no such rule and are still written.
#[test]
fn a_record_whose_fields_share_a_json_name_has_no_proto() {
    let dir = scratch("proto_json_names");
    let program = |fields: &str| {
        let path = dir.join("program.hl");
        std::fs::write(
            &path,
            [
                &format!("type Data {{ {fields} }}"),
                "locus Shelf { fn put(d: Data) -> Data { return d; } }",
                "api Store { rpc Shelf::put; }",
                "main locus App {",
                "    params { shelf: Shelf = Shelf { }; }",
                "    run() {",
                "        let h = api::serve(Store, unix::Rpc { path: \"/tmp/hale-r8b-names.sock\" }, as: \"store\", receivers: { Shelf: self.shelf }, bound: 8, on_full: refuse);",
                "        h.stop();",
                "    }",
                "}",
                "fn main() { App { }; }",
                "",
            ]
            .join("\n"),
        )
        .unwrap();
        path
    };
    let check = |path: &Path| {
        std::process::Command::new(env!("CARGO_BIN_EXE_hale"))
            .args(["check"])
            .arg(path)
            .args(["--api", "--surface", "Store", "--proto"])
            .env("HALE_SKIP_STALE_CHECK", "1")
            .output()
            .expect("run hale")
    };
    for (fields, both, name) in [("a_b: Int; aB: Int;", ["`a_b`", "`aB`"], "the same default JSON name `aB`"), ("a: Int; _a: Int;", ["`a`", "`_a`"], "`a` and `A`")] {
        let path = program(fields);
        let out = check(&path);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{fields}: it was written:\n{}", String::from_utf8_lossy(&out.stdout));
        assert!(err.contains("Data") && both.iter().all(|f| err.contains(f)) && err.contains(name), "{fields}: {err}");
        let out = export_cmd(&path, "Store", &["--out", dir.join("out").to_str().unwrap()]);
        assert!(!out.status.success(), "{fields}: export wrote it");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(both.iter().all(|f| err.contains(f)) && err.contains(name), "{fields}: {err}");
    }
    // the control: names that differ
    let path = program("a_b: Int; aC: Int;");
    let out = check(&path);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("optional int64 aC = 2;"));
    // and the other forms are written for the colliding record
    let path = program("a_b: Int; aB: Int;");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["check"])
        .arg(&path)
        .args(["--api", "--surface", "Store", "--openapi"])
        .env("HALE_SKIP_STALE_CHECK", "1")
        .output()
        .expect("run hale");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
}
