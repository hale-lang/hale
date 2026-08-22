//! GH #476 Change 3 — the projection differential.
//!
//! `topology_projection::project_model_half(derive(bundle))` must be
//! BYTE-IDENTICAL to the LEGACY model-half gathering (Change 6
//! inverted the direction: production emits the projection, and
//! the legacy arm — `dump_topology_parts(..).1` — exists only for
//! this differential until Change 9 retires it); it must also be
//! from its own derivation, for every program in the corpus — the
//! epic's exit criterion ("reproduce the legacy `TopologyShapeV1`
//! hash exactly over the corpus before any cutover"). Until the
//! Change-6 versioned transition this differential is a permanent
//! conformance gate: a model-builder change that would alter the
//! artifact identity fails HERE, loudly, instead of silently
//! re-keying `.halerec` replay admission.

use std::collections::BTreeMap;

use hale_types::model_builder::derive_application_model;
use hale_types::topology_projection::{
    project_model_half, project_shape_hash,
};
use hale_types::Bundle;

/// The hashed model half of a rendered artifact: everything from
/// `"sorts"` up to (not including) the unhashed `"sources"` map.
/// The EMITTED model half (the projection, post-Change 6).
fn model_half_of(artifact: &str) -> &str {
    let start = artifact
        .find("  \"sorts\": {")
        .expect("artifact has a sorts section");
    let end = artifact
        .find(",\n  \"sources\": [")
        .expect("artifact has a sources section");
    &artifact[start..end]
}

/// The DIFFERENTIAL's comparison arm: the legacy gathering string
/// `dump_topology_parts` returns beside the artifact. Kept as a
/// named helper so the direction of the comparison is explicit —
/// production emits the projection; the legacy arm exists only
/// here.
fn legacy_arm(legacy_half: &str) -> &str {
    legacy_half
}

/// Change 7 (schema 1.12): `endpoint_identity` is new hashed
/// content the legacy gathering never produced — strip it before a
/// V1 byte comparison.
fn strip_endpoint_identity(projected: &str) -> &str {
    projected
        .split(",\n  \"endpoint_identity\": [")
        .next()
        .unwrap_or(projected)
}

fn artifact_shape_hash(artifact: &str) -> u64 {
    artifact
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("\"shape_hash\": \"")?
                .strip_suffix("\",")
        })
        .and_then(|h| u64::from_str_radix(h, 16).ok())
        .expect("artifact has a shape_hash")
}

/// First divergence between two strings, with context — a byte
/// differential over a 60-line JSON body needs to say WHERE.
fn first_diff(a: &str, b: &str) -> String {
    let pos = a
        .bytes()
        .zip(b.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| a.len().min(b.len()));
    let lo = pos.saturating_sub(120);
    let a_end = (pos + 160).min(a.len());
    let b_end = (pos + 160).min(b.len());
    format!(
        "first divergence at byte {}:\n--- legacy:\n…{}…\n--- projected:\n…{}…",
        pos,
        &a[lo..a_end],
        &b[lo..b_end]
    )
}

fn check_one(
    origin: &str,
    program: &hale_syntax::ast::Program,
    bad: &mut Vec<String>,
) {
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || {
            let mut programs = BTreeMap::new();
            programs.insert("app.hl".to_string(), program);
            let bundle = Bundle::new(programs);
            let (art, legacy_half, legacy_tail) =
                hale_types::topology::dump_topology_parts(&bundle);
            let model = derive_application_model(&bundle);
            (art, legacy_half, legacy_tail, model)
        },
    ));
    let Ok((art, legacy_half, legacy_tail, model)) = caught else {
        bad.push(format!("{}: PANIC", origin));
        return;
    };
    // The differential is a property of CHECKED programs — the
    // corpus's negative fixtures can carry shapes the two
    // derivations legitimately read differently, and the checker
    // refuses them before either output exists.
    let checks_clean = hale_types::check_program(program)
        .iter()
        .all(|d| !d.is_error());
    if !checks_clean {
        return;
    }
    let legacy = legacy_arm(&legacy_half);
    let _ = model_half_of(&art);
    let projected_full = project_model_half(&model);
    // Change 7 (schema 1.12): `endpoint_identity` is NEW hashed
    // content the legacy gathering never produced — the versioned
    // shape transition. The differential's charter is the V1
    // subset: strip the new section, compare the rest byte-exact.
    let projected = strip_endpoint_identity(&projected_full);
    if legacy != projected {
        bad.push(format!(
            "{}: model half diverges.\n{}",
            origin,
            first_diff(legacy, &projected)
        ));
        return;
    }
    if artifact_shape_hash(&art) != project_shape_hash(&model) {
        bad.push(format!(
            "{}: byte-equal halves but hash mismatch (?)",
            origin
        ));
        return;
    }
    // Round 2: the UNHASHED tail (sources/provenance/topics) is
    // projected too — same differential, same direction.
    let projected_tail =
        hale_types::topology_projection::project_unhashed_tail(
            &model,
        );
    if legacy_tail != projected_tail {
        bad.push(format!(
            "{}: unhashed tail diverges.\n{}",
            origin,
            first_diff(&legacy_tail, &projected_tail)
        ));
    }
}

/// THE Change-3 gate: byte equality over every checkable corpus
/// program.
#[test]
fn projection_matches_legacy_over_the_corpus() {
    let mut bad: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for p in
        hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok())
    {
        let Ok(program) = hale_syntax::parse_source(&p.source) else {
            continue;
        };
        let before = bad.len();
        check_one(&p.origin, &program, &mut bad);
        if bad.len() == before {
            checked += 1;
        }
    }
    assert!(
        checked > 100,
        "the corpus sweep must actually cover programs (got {})",
        checked
    );
    assert!(
        bad.is_empty(),
        "{} corpus programs diverge:\n{}",
        bad.len(),
        bad.join("\n\n")
    );
}

/// The same equality on a fixture exercising every hashed section at
/// once — sealed loci, interface dispatch, stdlib contraction,
/// groups, labels, phases, seeds are corpus-covered, but supervision
/// + keyed topics + literal endpoints in ONE artifact pins the
/// interleaving.
#[test]
fn projection_matches_legacy_on_a_dense_fixture() {
    let src = r#"
type Reading { sensor: Int = 0; v: Int = 0; }
type Note { text: String = ""; }
topic Readings {
    payload: Reading;
    subject: "sense.reading";
    keyed_by sensor;
}
topic Cmds { payload: Note; subject: "ctl.cmd"; }

interface Notifier {
    fn notify(v: Int) -> Int;
}

fn double(v: Int) -> Int { return v * 2; }

@sealed
locus Vault {
    params { secret: Int = 0; }
    fn peek() -> Int { return self.secret; }
}

locus Child {
    params { n: Int = 0; }
    fn notify(v: Int) -> Int { return v; }
}

locus Worker {
    params { c: Child = Child { }; }
    bus {
        publish Cmds;
        subscribe Readings as on_reading where key == 3;
        subscribe "raw.wire" as on_raw of type Note;
    }
    fn on_reading(r: Reading) {
        let mut i = 0;
        while i < 3 {
            let d = double(r.v);
            Cmds <- Note { };
            i = i + 1;
        }
    }
    fn on_raw(n: Note) { }
    on_failure(c: Child, err: ClosureViolation) {
        restart (c) for 2;
    }
}

group Handlers = { Worker, Child };

main locus App {
    params { w: Worker = Worker { }; v: Vault = Vault { }; }
    run() { println(self.v.peek()); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bad = Vec::new();
    check_one("dense fixture", &program, &mut bad);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    // …and the fixture actually reaches the sections it claims to:
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let (art, legacy_half, _legacy_tail) =
        hale_types::topology::dump_topology_parts(&bundle);
    for needle in [
        "\"sealed\": [\"Vault\"]",
        "\"supervision\": [\n    {\"locus\": \"Worker\"",
        "\"retry_bound\": 2",
        "\"Handlers\": [\"Worker\", \"Child\"]",
        "\"subject\": \"raw.wire\"",
    ] {
        assert!(
            art.contains(needle),
            "fixture must exercise `{}`:\n{}",
            needle,
            art
        );
    }
}

/// Cross-seed: display-spelled artifact strings from a raw-keyed
/// model — the projection's whole job is this mapping.
#[test]
fn projection_matches_legacy_across_seeds() {
    let lib = r#"
type __lib_x_kv_Item { n: Int = 0; }
topic __lib_x_kv_Changed { payload: __lib_x_kv_Item; }
locus __lib_x_kv_Store {
    params { n: Int = 0; }
    bus { publish __lib_x_kv_Changed; }
    fn bump() { self.n = self.n + 1; __lib_x_kv_Changed <- __lib_x_kv_Item { }; }
}
"#;
    let main_src = r#"
locus Reader {
    bus { subscribe __lib_x_kv_Changed as on_changed; }
    fn on_changed(i: __lib_x_kv_Item) { }
}
main locus App {
    params { s: __lib_x_kv_Store = __lib_x_kv_Store { }; r: Reader = Reader { }; }
    run() { self.s.bump(); }
}
fn main() { App { }; }
"#;
    let main_p = hale_syntax::parse_source(main_src).expect("parse main");
    let lib_p = hale_syntax::parse_source(lib).expect("parse lib");
    let mut programs = BTreeMap::new();
    programs.insert("app/main.hl".to_string(), &main_p);
    programs.insert("lib/kv.hl".to_string(), &lib_p);
    let mut bundle = Bundle::new(programs);
    bundle.import_renames = vec![
        (
            vec!["kv".to_string(), "Item".to_string()],
            "__lib_x_kv_Item".to_string(),
        ),
        (
            vec!["kv".to_string(), "Changed".to_string()],
            "__lib_x_kv_Changed".to_string(),
        ),
        (
            vec!["kv".to_string(), "Store".to_string()],
            "__lib_x_kv_Store".to_string(),
        ),
    ];
    let (art, legacy_half, _legacy_tail) =
        hale_types::topology::dump_topology_parts(&bundle);
    let model = derive_application_model(&bundle);
    let legacy = legacy_arm(&legacy_half);
    let _ = model_half_of(&art);
    let projected_full = project_model_half(&model);
    let projected = strip_endpoint_identity(&projected_full);
    assert!(
        legacy.contains("kv::Store"),
        "artifact spells the import author-side:\n{}",
        legacy
    );
    assert_eq!(
        legacy,
        projected,
        "{}",
        first_diff(legacy, &projected)
    );
}

/// Direct comparison without the check gate — for fixtures pinning
/// legacy-order behavior the checker may or may not accept; both
/// derivations are total over parseable programs.
fn assert_projection_matches(src: &str, label: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let (art, legacy_half, _legacy_tail) =
        hale_types::topology::dump_topology_parts(&bundle);
    let model = derive_application_model(&bundle);
    let legacy = legacy_arm(&legacy_half).to_string();
    let _ = model_half_of(&art);
    let projected_full = project_model_half(&model);
    let projected = strip_endpoint_identity(&projected_full);
    assert_eq!(
        legacy,
        projected,
        "{}: {}",
        label,
        first_diff(&legacy, &projected)
    );
    legacy
}

/// P1 (round 11): when one (from, to) pair is dispatched through
/// several interfaces, the legacy encoder's last-in-source site
/// wins — NOT the lexicographically last interface the model's
/// canonical row order visits last. Source order here is Z then A,
/// so V1 selects A.
#[test]
fn via_interface_tie_follows_authored_order() {
    let src = r#"
interface AIface { fn notify(v: Int) -> Int; }
interface ZIface { fn notify(v: Int) -> Int; }
locus Conformer {
    params { n: Int = 0; }
    fn notify(v: Int) -> Int { return v + self.n; }
}
fn relay(z: ZIface, a: AIface, v: Int) -> Int {
    let first = z.notify(v);
    let second = a.notify(v);
    return first + second;
}
main locus App {
    params { c: Conformer = Conformer { }; }
    run() { println(relay(self.c, self.c, 1)); }
}
fn main() { App { }; }
"#;
    let legacy = assert_projection_matches(src, "via_interface tie");
    assert!(
        legacy.contains("\"via_interface\": \"AIface\""),
        "the LAST authored dispatch (AIface) wins:\n{}",
        legacy
    );
}

/// P1 (round 11): multi-class carrier labels render in
/// render_effects_named order — built-ins in fixed order, then USER
/// classes in declaration order — never lexical. `zebra` is
/// declared before `alpha` and must render before it.
#[test]
fn label_order_follows_declaration_not_lexical() {
    let src = r#"
effect zebra;
effect alpha;
@effects(is: { zebra, alpha })
fn classified() { println(1); }
main locus App {
    run() { classified(); }
}
fn main() { App { }; }
"#;
    let legacy =
        assert_projection_matches(src, "label declaration order");
    assert!(
        legacy.contains("[\"zebra\", \"alpha\"]"),
        "declaration order, not lexical:\n{}",
        legacy
    );
}

/// P1 (round 11): supervision handlers sharing (locus, child) keep
/// AUTHORED order under the legacy stable sort — not the model's
/// canonical error-type order. ZTrouble is authored before ATrouble.
#[test]
fn supervision_ties_keep_authored_order() {
    let src = r#"
type ZTrouble { n: Int = 0; }
type ATrouble { n: Int = 0; }
locus Child {
    params { n: Int = 0; }
}
locus Parent {
    params { c: Child = Child { }; }
    on_failure(c: Child, err: ZTrouble) {
        restart (c);
    }
    on_failure(c: Child, err: ATrouble) {
        quarantine (c);
    }
}
main locus App {
    params { p: Parent = Parent { }; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let legacy =
        assert_projection_matches(src, "supervision authored order");
    let z = legacy.find("\"err\": \"ZTrouble\"").expect("Z row");
    let a = legacy.find("\"err\": \"ATrouble\"").expect("A row");
    assert!(
        z < a,
        "authored order (Z first) survives the stable sort:\n{}",
        legacy
    );
}

/// P1 (round 11): V1 runs name() over EVERY subject string — a
/// literal subject whose text equals an imported declaration's raw
/// symbol demangles in the legacy artifact, so the projection must
/// apply the same map.
#[test]
fn literal_subject_colliding_with_raw_symbol_demangles() {
    let lib = r#"
type __lib_x_kv_Item { n: Int = 0; }
"#;
    let main_src = r#"
type Note { text: String = ""; }
locus Sink {
    bus { subscribe "__lib_x_kv_Item" as on_x of type Note; }
    fn on_x(n: Note) { }
}
main locus App {
    params { s: Sink = Sink { }; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let main_p = hale_syntax::parse_source(main_src).expect("parse main");
    let lib_p = hale_syntax::parse_source(lib).expect("parse lib");
    let mut programs = BTreeMap::new();
    programs.insert("app/main.hl".to_string(), &main_p);
    programs.insert("lib/kv.hl".to_string(), &lib_p);
    let mut bundle = Bundle::new(programs);
    bundle.import_renames = vec![(
        vec!["kv".to_string(), "Item".to_string()],
        "__lib_x_kv_Item".to_string(),
    )];
    let (art, legacy_half, _legacy_tail) =
        hale_types::topology::dump_topology_parts(&bundle);
    let model = derive_application_model(&bundle);
    let legacy = legacy_arm(&legacy_half);
    let _ = model_half_of(&art);
    let projected_full = project_model_half(&model);
    let projected = strip_endpoint_identity(&projected_full);
    assert!(
        legacy.contains("\"subject\": \"kv::Item\""),
        "V1 demangles the colliding literal:\n{}",
        legacy
    );
    assert_eq!(legacy, projected, "{}", first_diff(legacy, projected));
}

/// P1 (round 12): the V1 display map is EXACT-renames scope. A
/// literal subject spelled like an imported locus's METHOD identity
/// (`__lib_x_kv_Store::bump`) is not a renames key — legacy name()
/// leaves it verbatim, and so must the projection. (The type-symbol
/// collision above demangles; the method shape must not.)
#[test]
fn method_shaped_literal_subject_does_not_demangle() {
    let lib = r#"
type __lib_x_kv_Event { n: Int = 0; }
locus __lib_x_kv_Store {
    params { n: Int = 0; }
    fn bump() { self.n = self.n + 1; }
}
"#;
    let main_src = r#"
type Note { text: String = ""; }
locus Sink {
    bus { subscribe "__lib_x_kv_Store::bump" as on_ev of type Note; }
    fn on_ev(n: Note) { }
}
main locus App {
    params { s: Sink = Sink { }; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let main_p = hale_syntax::parse_source(main_src).expect("parse main");
    let lib_p = hale_syntax::parse_source(lib).expect("parse lib");
    let mut programs = BTreeMap::new();
    programs.insert("app/main.hl".to_string(), &main_p);
    programs.insert("lib/kv.hl".to_string(), &lib_p);
    let mut bundle = Bundle::new(programs);
    bundle.import_renames = vec![
        (
            vec!["kv".to_string(), "Event".to_string()],
            "__lib_x_kv_Event".to_string(),
        ),
        (
            vec!["kv".to_string(), "Store".to_string()],
            "__lib_x_kv_Store".to_string(),
        ),
    ];
    let (art, legacy_half, _legacy_tail) =
        hale_types::topology::dump_topology_parts(&bundle);
    let model = derive_application_model(&bundle);
    let legacy = legacy_arm(&legacy_half);
    let _ = model_half_of(&art);
    let projected_full = project_model_half(&model);
    let projected = strip_endpoint_identity(&projected_full);
    assert!(
        legacy.contains("\"subject\": \"__lib_x_kv_Store::bump\""),
        "V1 keeps the method-shaped literal verbatim:\n{}",
        legacy
    );
    assert_eq!(legacy, projected, "{}", first_diff(legacy, projected));
}

/// P1 (round 12): labels and effects are V1-universe sections. The
/// projector is public over any lawful model — a non-V1 function
/// (module-scoped, unanalyzed) carrying a label or an effect set
/// must NOT surface in a section whose fns are absent from
/// sorts.fns. Constructed directly: the current builder never
/// populates behavior on non-V1 fns, and this pin must hold as it
/// gains coverage.
#[test]
fn labels_and_effects_are_restricted_to_the_v1_universe() {
    use hale_model::*;
    let mut prov = ProvenanceTable::default();
    prov.records.push(Provenance::Synthetic {
        origin: "test".to_string(),
    });
    let p = ProvenanceId(0);
    let f = |name: &str, effects: Vec<&str>| Function {
        analyzed: true,
        summarized: true,
        owner: None,
        name: name.to_string(),
        display: name.to_string(),
        kind: FunctionKind::Free,
        effects: effects.into_iter().map(String::from).collect(),
        direct_effects: Vec::new(),
        attribution: Vec::new(),
        opaque_call: false,
        carries_user_class: false,
        provenance: p,
    };
    let mut m = ApplicationModel {
        header: ModelHeader {
            semantics: MODEL_SEMANTICS_V1,
            entrypoint: "main".to_string(),
        },
        entities: Entities {
            functions: vec![
                f("analyzed", vec!["alloc"]),
                f("hidden_extra", vec!["syscall"]),
            ],
            ..Entities::default()
        },
        relations: Relations::default(),
        labels: vec![
            LabelRow {
                at: EntityRef::Function(FunctionId(0)),
                label: "money".to_string(),
                provenance: p,
            },
            LabelRow {
                at: EntityRef::Function(FunctionId(1)),
                label: "money".to_string(),
                provenance: p,
            },
        ],
        weights: Vec::new(),
        holes: Vec::new(),
        capabilities: Capabilities::default(),
        provenance: prov,
        legacy: LegacyProjection {
            dispatch_gates: Vec::new(),
            topology_v1_fns: vec![FunctionId(0)],
            topology_v1_calls_via_stdlib: Vec::new(),
            stdlib_absorption: Vec::new(),
        },
    };
    let out = project_model_half(&m);
    assert!(
        out.contains("\"analyzed\": [\"alloc\"]"),
        "V1 fn keeps its effects:\n{}",
        out
    );
    assert!(
        !out.contains("hidden_extra"),
        "a non-V1 fn appears in NO fn-keyed section:\n{}",
        out
    );
    // …and with the extra fn enrolled, both sections carry it —
    // the filter is the universe, not the function.
    m.legacy.topology_v1_fns = vec![FunctionId(0), FunctionId(1)];
    let out = project_model_half(&m);
    assert!(out.contains("\"hidden_extra\": [\"syscall\"]"));
    assert!(out.contains("\"hidden_extra\": [\"money\"]"));
}

/// P1 (round 13): sealed renders DISPLAY values in RAW-name order —
/// the legacy encoder sorts raw and demangles only while
/// serializing. Two imports with deliberately reversed raw/alias
/// order pin it: raw `__lib_a_pack_Z` < `__lib_b_pack_A`, displays
/// `z::Z` then `a::A` — non-lexical display order is CORRECT.
#[test]
fn sealed_order_is_raw_not_display() {
    let lib_a = r#"
@sealed
locus __lib_a_pack_Z {
    params { n: Int = 0; }
}
"#;
    let lib_b = r#"
@sealed
locus __lib_b_pack_A {
    params { n: Int = 0; }
}
"#;
    let main_src = r#"
main locus App {
    params { z: __lib_a_pack_Z = __lib_a_pack_Z { }; a: __lib_b_pack_A = __lib_b_pack_A { }; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let main_p = hale_syntax::parse_source(main_src).expect("parse main");
    let a_p = hale_syntax::parse_source(lib_a).expect("parse lib a");
    let b_p = hale_syntax::parse_source(lib_b).expect("parse lib b");
    let mut programs = BTreeMap::new();
    programs.insert("app/main.hl".to_string(), &main_p);
    programs.insert("lib/a.hl".to_string(), &a_p);
    programs.insert("lib/b.hl".to_string(), &b_p);
    let mut bundle = Bundle::new(programs);
    bundle.import_renames = vec![
        (
            vec!["z".to_string(), "Z".to_string()],
            "__lib_a_pack_Z".to_string(),
        ),
        (
            vec!["a".to_string(), "A".to_string()],
            "__lib_b_pack_A".to_string(),
        ),
    ];
    let (art, legacy_half, _legacy_tail) =
        hale_types::topology::dump_topology_parts(&bundle);
    let model = derive_application_model(&bundle);
    let legacy = legacy_arm(&legacy_half);
    let _ = model_half_of(&art);
    let projected_full = project_model_half(&model);
    let projected = strip_endpoint_identity(&projected_full);
    assert!(
        legacy.contains("\"sealed\": [\"z::Z\", \"a::A\"]"),
        "raw order, display values:\n{}",
        legacy
    );
    assert_eq!(legacy, projected, "{}", first_diff(legacy, projected));
}

/// P1 (round 13): a retry bound is the literal AS WRITTEN — i64.
/// `for 4294967296` is check-clean; a u32 field silently truncated
/// it to 0 in the projected hash.
#[test]
fn retry_bound_keeps_the_full_literal() {
    let src = r#"
locus Child {
    params { n: Int = 0; }
}
locus Parent {
    params { c: Child = Child { }; }
    on_failure(c: Child, err: ClosureViolation) {
        restart (c) for 4294967296;
    }
}
main locus App {
    params { p: Parent = Parent { }; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let legacy = assert_projection_matches(src, "retry bound width");
    assert!(
        legacy.contains("\"retry_bound\": 4294967296"),
        "the literal survives at full width:\n{}",
        legacy
    );
}

/// P1 (round 14): duplicate-signature handlers — identical
/// (locus, child, error_type) — are check-clean, and the legacy
/// artifact serializes one row PER DECLARATION. The model's
/// canonical key includes the authored ordinal so both rows exist,
/// and the projection renders both in authored order.
#[test]
fn duplicate_signature_supervision_rows_both_survive() {
    let src = r#"
locus Child {
    params { n: Int = 0; }
}
locus Parent {
    params { c: Child = Child { }; }
    on_failure(c: Child, err: ClosureViolation) {
        restart (c);
    }
    on_failure(c: Child, err: ClosureViolation) {
        quarantine (c);
    }
}
main locus App {
    params { p: Parent = Parent { }; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let legacy = assert_projection_matches(
        src,
        "duplicate-signature supervision",
    );
    let restart = legacy
        .find("\"ops\": [\"restart\"]")
        .expect("first handler row");
    let quarantine = legacy
        .find("\"ops\": [\"quarantine\"]")
        .expect("second handler row");
    assert!(
        restart < quarantine,
        "both rows, authored order:\n{}",
        legacy
    );
}
