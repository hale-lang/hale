//! GH #476 Change 3 — the projection differential.
//!
//! `topology_projection::project_model_half(derive(bundle))` must be
//! BYTE-IDENTICAL to the hashed model half `dump_topology` builds
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
/// `dump_topology` hashes exactly this substring.
fn model_half_of(artifact: &str) -> &str {
    let start = artifact
        .find("  \"sorts\": {")
        .expect("artifact has a sorts section");
    let end = artifact
        .find(",\n  \"sources\": [")
        .expect("artifact has a sources section");
    &artifact[start..end]
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
            let art = hale_types::topology::dump_topology(&bundle);
            let model = derive_application_model(&bundle);
            (art, model)
        },
    ));
    let Ok((art, model)) = caught else {
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
    let legacy = model_half_of(&art);
    let projected = project_model_half(&model);
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
    let art = hale_types::topology::dump_topology(&bundle);
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
    let art = hale_types::topology::dump_topology(&bundle);
    let model = derive_application_model(&bundle);
    let legacy = model_half_of(&art);
    let projected = project_model_half(&model);
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
    let art = hale_types::topology::dump_topology(&bundle);
    let model = derive_application_model(&bundle);
    let legacy = model_half_of(&art).to_string();
    let projected = project_model_half(&model);
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
    let art = hale_types::topology::dump_topology(&bundle);
    let model = derive_application_model(&bundle);
    let legacy = model_half_of(&art);
    let projected = project_model_half(&model);
    assert!(
        legacy.contains("\"subject\": \"kv::Item\""),
        "V1 demangles the colliding literal:\n{}",
        legacy
    );
    assert_eq!(legacy, projected, "{}", first_diff(legacy, &projected));
}
