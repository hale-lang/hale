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

const REFUSALS: &[&str] = &["malformed", "digest_mismatch", "unauthenticated", "unauthorized", "full", "shutting_down"];

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
