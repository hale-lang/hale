//! Codec thread safety uses the shared execution account, including defaults
//! and parent callbacks. Missing execution facts cannot establish purity.
#[path = "support/entries.rs"]
mod entries;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_types::purity::{Impurity, Purity};

const CODEC: &str = r#"
type Message { n: Int = 0; }
type RequestId = distinct Int;
type CodecError { n: Int = 0; }
topic Wire { payload: Message; subject: "wire"; }
locus Child { }
locus Codec {
    params { seen: Int = 0; }
    accept(c: Child) { self.seen = self.seen + 1; }
    fn encode(v: Message) -> Bytes fallible(CodecError) {
        Child { };
        return std::bytes::from_string("x");
    }
    fn decode(b: Bytes) -> Message fallible(CodecError) { return Message { }; }
}
main locus App {
    bus { publish Wire; }
    bindings { Wire: unix("/tmp/hale-unused-purity.sock", role: connect) codec(Codec { }); }
}
fn main() { App { }; }
"#;

fn snapshot(source: &str) -> Snapshot {
    entries::load_files(&[("main.hl", source)], Config::check(true, false))
}
fn purity(s: &Snapshot, name: &str) -> Purity {
    let rows = s.demand_effects().unwrap();
    let key = rows
        .summary
        .fns
        .keys()
        .find(|k| k.display() == name)
        .unwrap();
    rows.purity(key).unwrap().clone()
}
fn codec_refused(s: &Snapshot) {
    let checked = s.demand_check().unwrap();
    let errors: Vec<_> = checked.diags.iter().filter(|d| d.is_error()).collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].message.contains("Codec.encode")
            && errors[0].message.contains("arbitrary threads"),
        "{errors:?}"
    );
}

#[test]
fn accept_mutation_and_empty_accept_tracking_both_refuse_codec_purity() {
    for source in [
        CODEC.to_string(),
        CODEC.replace("self.seen = self.seen + 1;", ""),
    ] {
        let s = snapshot(&source);
        assert!(matches!(
            purity(&s, "Codec::encode"),
            Purity::Impure(Impurity::ParentAdmission { .. })
        ));
        assert!(matches!(purity(&s, "Codec::decode"), Purity::Pure));
        codec_refused(&s);
        assert_eq!(s.builds()["protocol_execution"], 1);
        assert_eq!(s.builds()["model"], 0);
    }
}

#[test]
fn ordinary_codec_and_local_data_mutation_remain_pure() {
    let source = CODEC.replace(
        "Child { };",
        "let mut data = Message { }; data.n = max(v.n, 1); let text = to_string(data.n); let size = len(text); let id = RequestId(size);",
    );
    let s = snapshot(&source);
    assert!(matches!(purity(&s, "Codec::encode"), Purity::Pure));
    let checked = s.demand_check().unwrap();
    assert!(
        !checked.diags.iter().any(|d| d.is_error()),
        "{:?}",
        checked.diags
    );
}

#[test]
fn omitted_argument_and_record_defaults_follow_their_callers() {
    let source = r#"
fn observe() -> Int { println("default"); return 1; }
fn calculate(n: Int = observe()) -> Int { return n; }
type Data { n: Int = observe(); }
fn omitted_argument() { let n = calculate(); }
fn supplied_argument() { let n = calculate(2); }
fn omitted_field() { let data = Data { }; }
fn supplied_field() { let data = Data { n: 2 }; }
fn main() { }
"#;
    let s = snapshot(source);
    for name in ["omitted_argument", "omitted_field"] {
        assert!(matches!(purity(&s, name), Purity::Impure(_)), "{name}");
    }
    for name in ["supplied_argument", "supplied_field", "calculate"] {
        assert!(matches!(purity(&s, name), Purity::Pure), "{name}");
    }
}

#[test]
fn parent_tracking_in_defaults_is_attached_to_the_contextual_root() {
    let source = r#"
locus Child { }
fn use_child(c: Child = Child { }) -> Int { return 1; }
type Data { n: Int = use_child(); }
locus Parent {
    accept(c: Child) { }
    fn argument() { use_child(); }
    fn field() { let data = Data { }; }
}
fn free() { use_child(); }
fn main() { let parent = Parent { }; parent.argument(); parent.field(); free(); }
"#;
    let s = snapshot(source);
    for name in ["Parent::argument", "Parent::field"] {
        assert!(
            matches!(
                purity(&s, name),
                Purity::Impure(Impurity::ParentAdmission { .. })
            ),
            "{name}"
        );
    }
    assert!(matches!(purity(&s, "free"), Purity::Pure));
    assert!(matches!(purity(&s, "use_child"), Purity::Pure));
    let checked = s.demand_check().unwrap();
    assert!(
        !checked.diags.iter().any(|d| d.is_error()),
        "{:?}",
        checked.diags
    );
}

#[test]
fn constructor_enters_birth_and_teardown_bodies() {
    let source = r#"
locus Child { birth() { println("birth"); } }
locus End { dissolve() { println("end"); } }
fn start() { Child { }; }
fn finish() { End { }; }
fn main() { }
"#;
    let s = snapshot(source);
    for name in ["start", "finish"] {
        assert!(matches!(purity(&s, name), Purity::Impure(_)), "{name}");
    }
}

#[test]
fn unknown_execution_is_refused_and_known_impurity_is_retained() {
    let source = r#"
fn opaque(cb: fn (Int) -> Int) -> Int { return cb(1); }
fn loud(cb: fn (Int) -> Int) { let n = opaque(cb); println(n); }
fn cycle_a() { cycle_b(); }
fn cycle_b() { cycle_a(); }
locus Child { birth() { } run() { let n = 2; } }
fn lifecycle() { Child { }; }
fn main() { }
"#;
    let s = snapshot(source);
    for name in ["opaque", "cycle_a", "cycle_b", "lifecycle"] {
        assert!(
            matches!(purity(&s, name), Purity::Uncertified { .. }),
            "{name}: {:?}",
            purity(&s, name)
        );
    }
    assert!(matches!(purity(&s, "loud"), Purity::Impure(_)));
    let unknown_codec = CODEC
        .replace("accept(c: Child) { self.seen = self.seen + 1; }", "")
        .replace("locus Child { }", "locus Child { run() { let n = 2; } }");
    let codec = snapshot(&unknown_codec);
    assert!(matches!(
        purity(&codec, "Codec::encode"),
        Purity::Uncertified { .. }
    ));
    codec_refused(&codec);
}

#[test]
fn mutation_through_a_locus_alias_needs_a_confinement_account() {
    let source = r#"
locus Counter { params { n: Int = 0; } }
locus Holder { params { counter: Counter; } }
type CounterAlias = Counter;
locus Codec {
    params { counter: Counter = Counter { }; }
    fn mutate() { let other = self.counter; other.n = other.n + 1; }
    fn nested(holder: Holder) { let data = holder; data.counter.n = 1; }
    fn aliased(other: CounterAlias) { other.n = 1; }
}


fn main() { }
"#;
    let s = snapshot(source);
    for name in ["Codec::mutate", "Codec::nested", "Codec::aliased"] {
        assert!(
            matches!(purity(&s, name), Purity::Uncertified { .. }),
            "{name}"
        );
    }
    let checked = s.demand_check().unwrap();
    assert!(
        !checked.diags.iter().any(|d| d.is_error()),
        "{:?}",
        checked.diags
    );
}

#[test]
fn ffi_calls_are_known_impure() {
    let source = r#"
@ffi("c") fn c_helper(n: Int) -> Int;
fn foreign_call() -> Int { return c_helper(1); }
fn main() { }
"#;
    let s = snapshot(source);
    assert!(matches!(purity(&s, "foreign_call"), Purity::Impure(_)));
}

#[test]
fn module_bodies_and_imported_helpers_share_purity_keys() {
    let main = r#"
import "lib" as library;
module inner { fn shout() { println("module"); } }
fn imported() { library::shout(); }
fn local() { shout(); }
fn main() { }
"#;
    let library = "fn shout() { println(\"library\"); }";
    let s = entries::load_files(
        &[("main.hl", main), ("lib/lib.hl", library)],
        Config::check(true, false),
    );
    for name in ["imported", "local"] {
        assert!(matches!(purity(&s, name), Purity::Impure(_)), "{name}");
    }
}

#[test]
fn purity_refreshes_under_editor_typing_reuse() {
    let clean = CODEC.replace("Child { };", "");
    let old = snapshot(&clean);
    assert!(matches!(purity(&old, "Codec::encode"), Purity::Pure));
    let edited = snapshot(CODEC).reusing_typing(old);
    codec_refused(&edited);
    assert!(matches!(
        purity(&edited, "Codec::encode"),
        Purity::Impure(_)
    ));
    assert!(edited.typing_reuse().is_some());
    assert_eq!(edited.builds()["protocol_execution"], 1);
}

#[test]
fn unresolved_generic_ancestor_cannot_establish_purity() {
    let source = r#"
locus Child { }
locus Maker { fn make() { Child { }; } }
locus Owner<T> {
    params { maker: Maker = Maker { }; }
    accept(c: T) { }
}
fn main() { let owner: Owner<Child> = Owner { }; owner.maker.make(); }
"#;
    let s = snapshot(source);
    assert!(matches!(
        purity(&s, "Maker::make"),
        Purity::Uncertified { .. }
    ));
    let unrelated = source
        .replace("maker: Maker = Maker { };", "marker: Int = 0;")
        .replace(
            "owner.maker.make();",
            "let maker = Maker { }; maker.make();",
        );
    assert!(matches!(
        purity(&snapshot(&unrelated), "Maker::make"),
        Purity::Pure
    ));
}

// These declaration-only probes intentionally omit foreign executable bodies.
// Ordinary strings keep partial/negative API inputs out of the program corpus.
fn declaration_only(source: &str) -> (hale_types::alloc_summary::AllocSummary, Purity) {
    let mut program = hale_syntax::parse_source(source).unwrap();
    let ids = hale_types::snapshot::mint([("probe.hl", &mut program)], &[]);
    let summary = hale_types::alloc_summary::summarize_identified(&[(&program, &ids)], &[]);
    let key = summary.resolve(None, "adjust").unwrap();
    let result = hale_types::purity::infer_from_summary(&summary, &Default::default())[key].clone();
    (summary, result)
}

#[test]
fn stdlib_record_confinement_uses_declarations_without_importing_bodies() {
    for source in [
        "fn adjust(value: __StdMetricsHistogramData) { let mut local = value; local.count = 2; } fn main() { }",
        "type Wrapped { value: __StdMetricsHistogramData; } fn adjust(value: Wrapped) { let mut local = value; local.value.count = 2; } fn main() { }",
    ] {
        let (summary, result) = declaration_only(source);
        assert!(matches!(result, Purity::Pure), "{result:?}");
        assert_eq!(summary.fns.len(), 2, "record visibility imports no executable rows");
        assert!(summary.analysis_copy.is_empty() && summary.initializers.is_empty());
        let complete = snapshot(source);
        assert!(!complete.demand_check().unwrap().diags.iter().any(|d| d.is_error()));
        assert!(matches!(purity(&complete, "adjust"), Purity::Pure));
    }
}

#[test]
fn a_local_nominal_override_does_not_inherit_a_stdlib_record_proof() {
    let source = "type __StdMetricsHistogramData = Missing; fn adjust(value: __StdMetricsHistogramData) { let mut local = value; local.count = 2; } fn main() { }";
    let (_, result) = declaration_only(source);
    assert!(matches!(&result, Purity::Uncertified { reason, .. } if reason.contains("receiver-confinement")), "{result:?}");
}

#[test]
fn data_declarations_do_not_close_an_unknown_callback() {
    let source = "fn adjust(value: __StdMetricsHistogramData, callback: fn()) { let mut local = value; local.count = 2; callback(); } fn main() { }";
    let (summary, result) = declaration_only(source);
    assert_eq!(summary.fns.len(), 2);
    assert!(matches!(result, Purity::Uncertified { .. }), "{result:?}");
}

#[test]
fn data_declarations_do_not_hide_executable_defaults() {
    let source = "fn default_n() -> Int { println(1); return 0; } type Wrapped { value: __StdMetricsHistogramData; n: Int = default_n(); } fn adjust(value: __StdMetricsHistogramData) { let mut local = value; local.count = 2; let wrapped = Wrapped { value: local }; } fn main() { }";
    let (summary, result) = declaration_only(source);
    assert!(!summary.initializers.is_empty());
    assert!(summary.analysis_copy.is_empty());
    assert!(matches!(result, Purity::Impure(_)), "{result:?}");
}

#[test]
fn record_shapes_do_not_confine_managed_receivers_or_nested_handles() {
    for source in [
        "locus State { params { count: Int = 0; } } fn adjust(value: State) { value.count = 2; } fn main() { }",
        "locus State { params { count: Int = 0; } } type Wrapped { state: State; } fn adjust(value: Wrapped) { let mut local = value; local.state.count = 2; } fn main() { }",
    ] {
        let (_, result) = declaration_only(source);
        assert!(matches!(&result, Purity::Uncertified { reason, .. } if reason.contains("receiver-confinement")), "{result:?}");
    }
}

#[test]
fn generic_parameter_spellings_cannot_borrow_a_nominal_record_proof() {
    for source in [
        "fn adjust<__StdMetricsHistogramData>(value: __StdMetricsHistogramData) { value.count = 2; } fn main() { }",
        "locus State { params { count: Int = 0; } } type Wrapped<__StdMetricsHistogramData> { value: __StdMetricsHistogramData; } fn adjust(value: Wrapped<State>) { value.value.count = 2; } fn main() { }",
        "type Data { count: Int = 0; } locus State { params { count: Int = 0; } } type Wrapped<Data> { value: Data; } fn adjust(value: Wrapped<State>) { value.value.count = 2; } fn main() { }",
    ] {
        let (_, result) = declaration_only(source);
        assert!(matches!(&result, Purity::Uncertified { reason, .. } if reason.contains("receiver-confinement")), "{result:?}");
        if source.starts_with("type Data") {
            let complete = snapshot(source);
            assert!(!complete.demand_check().unwrap().diags.iter().any(|d| d.is_error()));
            assert!(matches!(purity(&complete, "adjust"), Purity::Uncertified { .. }));
        }
    }
}

#[test]
fn generic_record_name_collision_cannot_certify_a_concurrent_codec() {
    let source = CODEC
        .replace("locus Child { }", "type Data { count: Int = 0; } locus State { params { count: Int = 0; } } type Wrapped<Data> { value: Data; } fn adjust(value: Wrapped<State>) { value.value.count = 2; }")
        .replace("params { seen: Int = 0; }", "params { state: State = State { }; }")
        .replace("    accept(c: Child) { self.seen = self.seen + 1; }", "")
        .replace("Child { };", "let wrapped: Wrapped<State> = Wrapped { value: self.state }; adjust(wrapped);");
    let s = snapshot(&source);
    assert!(matches!(purity(&s, "adjust"), Purity::Uncertified { .. }));
    assert!(matches!(purity(&s, "Codec::encode"), Purity::Uncertified { .. }));
    assert!(matches!(purity(&s, "Codec::decode"), Purity::Pure));
    codec_refused(&s);
}
