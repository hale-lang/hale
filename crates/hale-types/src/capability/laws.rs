//! The matrix's own laws (design §1.6). Each law is a function that
//! returns its violations, so a test can show the law catching a
//! broken table as well as passing the real one.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use super::*;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every key the matrix must have a row for, one per kind.
fn expected_capabilities() -> BTreeSet<String> {
    let mut out: BTreeSet<String> = stdlib_namespaces().into_iter().map(|ns| format!("std::{ns}")).collect();
    let mut add = |c: Capability| {
        out.insert(c.label());
    };
    add(Capability::LinkLibrary);
    add(Capability::ExportSurface);
    for i in Inversion::ALL {
        add(Capability::EntryInversion(i));
    }
    for a in Abi::ALL {
        add(Capability::ForeignAbi(a));
        for t in FfiTypeClass::ALL {
            add(Capability::FfiType(t, a));
        }
    }
    add(Capability::AsyncIoPool);
    add(Capability::PinnedThreads);
    add(Capability::PoolThreads);
    for t in Transport::ALL {
        add(Capability::RemoteTransport(t));
    }
    add(Capability::BoundedWait);
    add(Capability::ProcessSignals);
    out
}

/// The namespaces the stdlib defines: the surface table's, and the
/// namespaces its locus and type paths live in.
fn stdlib_namespaces() -> BTreeSet<String> {
    let mut out: BTreeSet<String> =
        crate::stdlib_surface::SURFACES.iter().map(|s| s.ns.join("::")).collect();
    for p in crate::stdlib_surface::LOCUS_PATHS {
        out.insert(p[1..p.len() - 1].join("::"));
    }
    out
}

/// Law 1: every (target, key) pair has exactly one cell of its own
/// type. Columns are structural, so this is: every key has exactly one
/// row, and no row has a key outside the enumeration.
fn coverage_violations(m: &CapabilityMatrix) -> Vec<String> {
    let mut v = Vec::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for r in m.behaviours {
        *seen.entry(r.capability.label()).or_default() += 1;
    }
    let expected = expected_capabilities();
    for k in &expected {
        match seen.get(k) {
            None => v.push(format!("behaviour `{k}` has no row: no cell on any target")),
            Some(n) if *n > 1 => v.push(format!("behaviour `{k}` has {n} rows")),
            Some(_) => {}
        }
    }
    for k in seen.keys() {
        if !expected.contains(k) {
            v.push(format!("behaviour row `{k}` is not a capability the stdlib or the language defines"));
        }
    }
    for i in Invocation::ALL {
        let n = m.invocations.iter().filter(|r| r.invocation == i).count();
        if n != 1 {
            v.push(format!("invocation {i:?} has {n} rows"));
        }
    }
    for o in Obligation::ALL {
        let n = m.obligations.iter().filter(|r| r.obligation == o).count();
        if n != 1 {
            v.push(format!("obligation {o:?} has {n} rows"));
        }
    }
    v
}

fn behaviour_cells(m: &CapabilityMatrix) -> impl Iterator<Item = (TargetClass, Capability, &'static Behaviour)> + '_ {
    m.behaviours
        .iter()
        .flat_map(|r| TargetClass::ALL.into_iter().map(move |c| (c, r.capability, r.cells.get(c))))
}

fn witnesses(m: &CapabilityMatrix) -> Vec<(String, Witness)> {
    let mut out = Vec::new();
    for (c, cap, b) in behaviour_cells(m) {
        out.push((format!("{} × {}", cap.label(), c.name()), b.witness));
    }
    for r in m.invocations {
        for c in TargetClass::ALL {
            out.push((format!("{:?} × {}", r.invocation, c.name()), r.cells.get(c).witness));
        }
    }
    for r in m.obligations {
        for c in TargetClass::ALL {
            out.push((format!("{:?} × {}", r.obligation, c.name()), r.cells.get(c).witness));
        }
    }
    out
}

/// `spec/<file>.md § <heading>` names a heading the file has.
fn spec_anchor_exists(anchor: &str) -> Result<(), String> {
    let (file, heading) = anchor
        .split_once(" § ")
        .ok_or_else(|| format!("`{anchor}` is not `spec/<file>.md § <heading>`"))?;
    if !file.starts_with("spec/") {
        return Err(format!("`{anchor}` is not under spec/"));
    }
    let text = std::fs::read_to_string(repo_root().join(file)).map_err(|e| format!("`{file}`: {e}"))?;
    let found = text
        .lines()
        .any(|l| l.starts_with('#') && l.trim_start_matches('#').trim() == heading);
    if found {
        Ok(())
    } else {
        Err(format!("`{file}` has no heading `{heading}`"))
    }
}

/// `path::symbol` names a file that contains the symbol.
fn site_exists(site: &str) -> Result<(), String> {
    let (path, symbol) = site.split_once("::").ok_or_else(|| format!("`{site}` is not `path::symbol`"))?;
    let text = std::fs::read_to_string(repo_root().join(path)).map_err(|e| format!("`{path}`: {e}"))?;
    if text.contains(symbol) {
        Ok(())
    } else {
        Err(format!("`{path}` does not contain `{symbol}`"))
    }
}

/// Law 2: every refusal has non-empty wording, and every cell's spec
/// anchor and site exist.
fn witness_violations(m: &CapabilityMatrix) -> Vec<String> {
    let mut v = Vec::new();
    for (c, cap, b) in behaviour_cells(m) {
        if let Some(r) = b.refusal() {
            if r.render(&b.witness, &[]).trim().is_empty() || b.witness.reason.is_empty() {
                v.push(format!("{} × {}: a Reject with no wording", cap.label(), c.name()));
            }
        }
    }
    for r in m.invocations {
        for c in TargetClass::ALL {
            if let InvocationVerdict::Refused(f) = &r.cells.get(c).verdict {
                if f.wording.trim().is_empty() {
                    v.push(format!("{:?} × {}: a Refused with no wording", r.invocation, c.name()));
                }
            }
        }
    }
    for (what, wit) in witnesses(m) {
        if wit.reason.trim().is_empty() {
            v.push(format!("{what}: an empty reason"));
        }
        if let Err(e) = spec_anchor_exists(wit.spec) {
            v.push(format!("{what}: {e}"));
        }
        if let Err(e) = site_exists(wit.site) {
            v.push(format!("{what}: {e}"));
        }
    }
    v
}

/// Law 3: an approximating lowering only on layers 5 and 7.
fn approximation_violations(m: &CapabilityMatrix) -> Vec<String> {
    behaviour_cells(m)
        .filter(|(_, cap, b)| {
            matches!(b.verdict, BehaviourVerdict::Lower(Lowering::Approximate { .. }))
                && !matches!(cap.layer(), 5 | 7)
        })
        .map(|(c, cap, _)| format!("{} × {}: approximates on layer {}", cap.label(), c.name(), cap.layer()))
        .collect()
}

/// Law 4: every `Omit` premise holds on its own target. Returns the
/// obligations whose premise does not.
fn failing_premises(m: &CapabilityMatrix) -> BTreeSet<(TargetClass, Obligation)> {
    let mut out = BTreeSet::new();
    for r in m.obligations {
        for c in TargetClass::ALL {
            if let ObligationVerdict::Omit(p) = &r.cells.get(c).verdict {
                if !m.premise_holds(c, p) {
                    out.insert((c, r.obligation));
                }
            }
        }
    }
    out
}

/// Every `Proven` premise names a registered proof whose tests exist.
fn proof_violations(m: &CapabilityMatrix) -> Vec<String> {
    fn walk(p: &Premise, out: &mut Vec<ProofId>) {
        match p {
            Premise::Proven(id) => out.push(*id),
            Premise::All(ps) => ps.iter().for_each(|q| walk(q, out)),
            Premise::Rejects(_) | Premise::Refuses(_) => {}
        }
    }
    let mut v = Vec::new();
    for r in m.obligations {
        for c in TargetClass::ALL {
            if let ObligationVerdict::Omit(p) = &r.cells.get(c).verdict {
                let mut ids = Vec::new();
                walk(p, &mut ids);
                for id in ids {
                    match PROOFS.iter().find(|q| q.id == id) {
                        None => v.push(format!("{:?} × {}: proof `{}` is not registered", r.obligation, c.name(), id.0)),
                        Some(q) if q.tests.is_empty() => v.push(format!("proof `{}` names no test", id.0)),
                        Some(q) => {
                            for t in q.tests {
                                if !repo_root().join(t).exists() {
                                    v.push(format!("proof `{}`: test `{t}` does not exist", id.0));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    v
}

/// Law 5: `requires` is acyclic, and a `Lower` behaviour requires only
/// behaviours that are `Lower` on the same target.
fn requires_violations(m: &CapabilityMatrix) -> Vec<String> {
    let mut v = Vec::new();
    for (c, cap, b) in behaviour_cells(m) {
        for dep in b.requires {
            match m.behaviour(c, *dep) {
                None => v.push(format!("{} × {} requires {}, which has no row", cap.label(), c.name(), dep.label())),
                Some(d) if b.is_lower() && !d.is_lower() => v.push(format!(
                    "{} × {} is Lower and requires {}, which is Reject there",
                    cap.label(),
                    c.name(),
                    dep.label()
                )),
                Some(_) => {}
            }
        }
    }
    // Acyclic: a depth-first walk from every cell never returns to it.
    for (c, cap, _) in behaviour_cells(m) {
        let mut stack = vec![cap];
        let mut seen = BTreeSet::new();
        while let Some(x) = stack.pop() {
            let Some(b) = m.behaviour(c, x) else { continue };
            for dep in b.requires {
                if *dep == cap {
                    v.push(format!("{} × {}: a requires cycle", cap.label(), c.name()));
                } else if seen.insert(*dep) {
                    stack.push(*dep);
                }
            }
        }
    }
    v
}

/// Law 6: a cell's origin is its capability's; an `Environment`
/// behaviour has no use, so its refusal names nothing a use supplies.
fn origin_violations(m: &CapabilityMatrix) -> Vec<String> {
    let mut v = Vec::new();
    for (c, cap, b) in behaviour_cells(m) {
        if b.origin != cap.origin() {
            v.push(format!("{} × {}: origin {:?}, the capability's is {:?}", cap.label(), c.name(), b.origin, cap.origin()));
        }
        if b.origin == Origin::Environment {
            if let Some(r) = b.refusal() {
                if r.wording != "{reason}" || r.guidance.is_some() {
                    v.push(format!("{} × {}: an Environment refusal renders a use", cap.label(), c.name()));
                }
            }
        }
    }
    v
}

#[test]
fn every_pair_has_exactly_one_cell() {
    let v = coverage_violations(&derive_capability_matrix());
    assert!(v.is_empty(), "{}", v.join("\n"));
}

/// The coverage law fails on a missing cell: a table without a row is
/// refused, never answered by a default.
#[test]
fn a_missing_cell_fails_the_law() {
    let m = derive_capability_matrix();
    let dropped = CapabilityMatrix { behaviours: &m.behaviours[1..], ..m };
    let v = coverage_violations(&dropped);
    assert_eq!(v, vec!["behaviour `std::io::tcp` has no row: no cell on any target".to_string()]);
    let dropped = CapabilityMatrix { obligations: &m.obligations[1..], ..m };
    assert_eq!(coverage_violations(&dropped), vec!["obligation ReplayIngress has 0 rows".to_string()]);
    let dropped = CapabilityMatrix { invocations: &m.invocations[1..], ..m };
    assert_eq!(coverage_violations(&dropped), vec!["invocation Run has 0 rows".to_string()]);
}

#[test]
fn every_witness_and_refusal_is_anchored() {
    let v = witness_violations(&derive_capability_matrix());
    assert!(v.is_empty(), "{}", v.join("\n"));
}

#[test]
fn approximation_only_on_layers_five_and_seven() {
    let v = approximation_violations(&derive_capability_matrix());
    assert!(v.is_empty(), "{}", v.join("\n"));
}

/// The premises: every `Omit` holds on its own target, except a cell
/// KNOWN_OPEN names, whose premise must fail today, so the entry has to
/// go when it starts to hold. (T2 made wasm32's `PoolJoin`,
/// `IngressQuiesce` and `BindingConfig` premises hold; none is open.)
#[test]
fn every_omit_premise_holds_or_is_known_open() {
    let m = derive_capability_matrix();
    let failing = failing_premises(&m);
    let open: BTreeSet<(TargetClass, Obligation)> = KNOWN_OPEN
        .iter()
        .filter_map(|k| match k.cell {
            OpenCell::Premise(o) => Some((k.class, o)),
            OpenCell::Behaviour(_) | OpenCell::LateRefusal(_) => None,
        })
        .collect();
    assert_eq!(failing, open, "the failing premises are exactly KNOWN_OPEN's");
    let v = proof_violations(&m);
    assert!(v.is_empty(), "{}", v.join("\n"));
}

/// The premise law catches an omission nothing justifies.
#[test]
fn an_unjustified_omit_fails_the_law() {
    let m = derive_capability_matrix();
    // Host's ReplayIngress omitted on the premise wasm32 uses: the host
    // replays, so the premise does not hold there.
    assert!(!m.premise_holds(TargetClass::PosixAsync, &Premise::Refuses(Invocation::Replay)));
    assert!(m.premise_holds(TargetClass::Wasm32, &Premise::Refuses(Invocation::Replay)));
    assert!(!m.premise_holds(TargetClass::Wasm32, &Premise::Proven(ProofId("single_thread_no_live_waiter"))));
}

#[test]
fn requires_is_acyclic_and_consistent() {
    let v = requires_violations(&derive_capability_matrix());
    assert!(v.is_empty(), "{}", v.join("\n"));
}

#[test]
fn origins_are_the_capabilitys() {
    let v = origin_violations(&derive_capability_matrix());
    assert!(v.is_empty(), "{}", v.join("\n"));
}

/// Every KNOWN_OPEN behaviour is still today's answer: `Lower`, where
/// the design makes it `Reject`; a late refusal is `Reject` with the
/// linker's wording, where the design locates it. An entry whose cell
/// has moved has to go.
#[test]
fn known_open_behaviours_are_todays_answer() {
    let m = derive_capability_matrix();
    let mut seen = BTreeSet::new();
    for k in KNOWN_OPEN {
        assert!(seen.insert((k.class, k.cell)), "KNOWN_OPEN names {:?} twice", k.cell);
        match k.cell {
            OpenCell::Behaviour(cap) => {
                let b = m.behaviour(k.class, cap).unwrap_or_else(|| panic!("{} has no row", cap.label()));
                assert!(b.is_lower(), "{} × {} is no longer Lower: drop it from KNOWN_OPEN", cap.label(), k.class.name());
            }
            OpenCell::LateRefusal(cap) => {
                let b = m.behaviour(k.class, cap).unwrap_or_else(|| panic!("{} has no row", cap.label()));
                assert_eq!(
                    b.refusal().map(|r| r.wording),
                    Some(WASM_LD_WORDING),
                    "{} × {} is no longer the late link refusal: drop it from KNOWN_OPEN",
                    cap.label(),
                    k.class.name()
                );
            }
            OpenCell::Premise(_) => {}
        }
    }
}

/// The FFI-portable set does not vary by target or by ABI (design
/// §2.6): the one row shape that writes one cell into three columns
/// is held to that equality.
#[test]
fn ffi_types_are_target_independent() {
    let m = derive_capability_matrix();
    for r in m.behaviours {
        if let Capability::FfiType(t, _) = r.capability {
            let first = r.cells.posix_async;
            for c in TargetClass::ALL {
                assert_eq!(*r.cells.get(c), first, "{:?} varies by target", t);
            }
            for a in Abi::ALL {
                assert_eq!(m.behaviour(TargetClass::PosixAsync, Capability::FfiType(t, a)), Some(&first));
            }
        }
    }
}

/// Every stdlib source file defines namespaces the matrix has rows
/// for. A new file joins this list with its namespaces, or the law
/// fails: a namespace cannot arrive without a cell.
#[test]
fn every_stdlib_source_maps_to_namespace_rows() {
    // hale-stdlib's Hale sources (AP_FILES), by the namespace each
    // defines; `core.hl` holds internal helpers the others compose.
    const HL: &[(&str, &[&str])] = &[
        ("core.hl", &[]),
        ("str_view.hl", &["str"]),
        ("io_tcp.hl", &["io::tcp"]),
        ("io_udp.hl", &["io::udp"]),
        ("http.hl", &["http"]),
        ("http_client.hl", &["http"]),
        ("metrics.hl", &["metrics"]),
        ("text.hl", &["text"]),
        ("secret.hl", &["secret"]),
        ("api.hl", &["api"]),
        ("test.hl", &["test"]),
        ("log.hl", &["log"]),
        ("ts.hl", &["ts"]),
        ("lang.hl", &["lang"]),
        ("iter.hl", &["iter"]),
        ("tagged.hl", &["tagged"]),
        ("name.hl", &["name"]),
        ("json.hl", &["json"]),
        ("yaml.hl", &["yaml"]),
        ("cli.hl", &["cli"]),
        ("source.hl", &["source"]),
        ("process.hl", &["process"]),
        ("bus.hl", &["bus"]),
        ("file.hl", &["io::file"]),
        ("bytes_builder.hl", &["bytes"]),
        ("mirror_ring.hl", &["io"]),
        ("term.hl", &["term"]),
        ("time.hl", &["time"]),
    ];
    // Codegen's native stdlib modules, by the namespace each lowers.
    const CG: &[(&str, &[&str])] = &[
        ("mod.rs", &[]),
        ("bus.rs", &["bus"]),
        ("bytes.rs", &["bytes"]),
        ("compress.rs", &["compress"]),
        ("crypto.rs", &["crypto"]),
        ("decimal.rs", &["decimal"]),
        ("diag.rs", &["diag"]),
        ("env.rs", &["env"]),
        ("io_file.rs", &["io::file"]),
        ("io_fs.rs", &["io::fs"]),
        ("io_stdin.rs", &["io::stdin"]),
        ("io_tcp.rs", &["io::tcp"]),
        ("io_tls.rs", &["io::tls"]),
        ("io_udp.rs", &["io::udp"]),
        ("io_unix.rs", &["io::unix"]),
        ("math.rs", &["math"]),
        ("mirror.rs", &["io::mirror"]),
        ("process.rs", &["process"]),
        ("rand.rs", &["rand"]),
        ("ring.rs", &["ring"]),
        ("sockopt.rs", &["io::sockopt"]),
        ("str.rs", &["str"]),
        ("term.rs", &["term"]),
        ("text.rs", &["text"]),
        ("time.rs", &["time"]),
    ];
    let m = derive_capability_matrix();
    let has_row = |ns: &str| m.behaviours.iter().any(|r| r.capability == Capability::StdNamespace(leak(ns)));
    let mut v = Vec::new();
    for (file, _) in hale_stdlib::AP_FILES {
        match HL.iter().find(|(f, _)| f == file) {
            None => v.push(format!("hale-stdlib's `{file}` is not mapped to its namespaces")),
            Some((_, nss)) => v.extend(nss.iter().filter(|ns| !has_row(ns)).map(|ns| format!("`{file}`: std::{ns} has no row"))),
        }
    }
    let dir = repo_root().join("crates/hale-codegen/src/stdlib");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|f| f.ends_with(".rs"))
        .collect();
    files.sort();
    for file in &files {
        match CG.iter().find(|(f, _)| f == file) {
            None => v.push(format!("codegen's `stdlib/{file}` is not mapped to its namespaces")),
            Some((_, nss)) => v.extend(nss.iter().filter(|ns| !has_row(ns)).map(|ns| format!("`stdlib/{file}`: std::{ns} has no row"))),
        }
    }
    assert!(v.is_empty(), "{}", v.join("\n"));
}

/// Interning for the comparison above: rows are keyed by `'static`
/// strings, a file map by its literal.
fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// A target's class comes from its `(arch, os, env)`, never its name.
#[test]
fn every_buildable_target_has_a_class() {
    for t in TargetSpec::known() {
        let c = TargetClass::of(&t);
        match (t.os, t.env) {
            (TargetOs::Windows, _) => assert_eq!(c, None, "{}", t.triple),
            (TargetOs::None, _) => assert_eq!(c, Some(TargetClass::Wasm32), "{}", t.triple),
            (TargetOs::Linux, TargetEnv::Musl) => assert_eq!(c, Some(TargetClass::PosixNoAsync), "{}", t.triple),
            _ => assert_eq!(c, Some(TargetClass::PosixAsync), "{}", t.triple),
        }
    }
    for alias in ["wasm", "wasm32", "wasm32-unknown-unknown"] {
        assert_eq!(TargetClass::of(&TargetSpec::parse(alias).unwrap()), Some(TargetClass::Wasm32));
    }
    assert_eq!(TargetClass::of(&TargetSpec::host()), Some(TargetClass::PosixAsync));
}

/// The table's size, pinned so a change to it is a reviewed one.
#[test]
fn the_tables_have_the_reviewed_shape() {
    let m = derive_capability_matrix();
    let std = m.behaviours.iter().filter(|r| matches!(r.capability, Capability::StdNamespace(_))).count();
    let rejects: Vec<String> = behaviour_cells(&m)
        .filter(|(_, _, b)| !b.is_lower())
        .map(|(c, cap, _)| format!("{} × {}", cap.label(), c.name()))
        .collect();
    assert_eq!((m.behaviours.len(), std), (104, 47), "behaviour rows (all, std::)");
    assert_eq!(m.invocations.len(), 3);
    assert_eq!(m.obligations.len(), 9);
    // On wasm32: 17 namespaces (the 10 browser-unavailable, T3's 7),
    // `[ffi] link`, `@export locus` with `run()`, ProcessSignals, and
    // T2's pinned threads, pools, `async_io` and the three transports;
    // on both POSIX columns the export-only module, `--wrap-main` and
    // T5's `@ffi("js")`; `async_io` on musl; 9 FFI type classes × 2 ABIs
    // × 3 targets.
    assert_eq!(rejects.len(), 87, "{}", rejects.join("\n"));
}
