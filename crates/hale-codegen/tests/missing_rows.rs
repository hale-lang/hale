//! A row lowering requires is never answered by a default (F.40 phase
//! 3's exit): the snapshot provides it for every program that reaches
//! lowering, so a view without it is a compiler defect, and lowering
//! refuses it with a `CodegenError` naming the family and the row. One
//! test per required family: the harness's view of a program that reads
//! the row, built clean (the control), then with that row removed. The
//! registry names each test (`hale_graph::registry::Missing::Required`).

use hale_codegen::{build_resolved, CodegenError};
use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_types::resolved::LoweringView;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The harness's lowering view of `src`, the one
/// `build_executable_with_options` lowers, owned so a test can take a
/// row out of it.
fn view(src: &str) -> LoweringView {
    let program = hale_syntax::parse_source(src).expect("parses");
    let Ok(snap) = Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())) else {
        panic!("the program does not load");
    };
    snap.demand_lowering()
        .unwrap_or_else(|b| panic!("lowering blocked: {:?} {:?}", b.refused, b.because))
        .clone()
}

/// Lower `view` to an executable; `Ok` only when the executable was
/// written.
fn lower(tag: &str, view: &LoweringView) -> Result<(), CodegenError> {
    let bin = harness::unique_bin(&format!("missing_rows_{tag}"));
    let built = build_resolved(view, &bin, &build_opts::options());
    let written = bin.is_file();
    let _ = std::fs::remove_file(&bin);
    built?;
    assert!(written, "the build reported success and wrote no executable");
    Ok(())
}

/// `src` lowers with its view whole, and `remove` takes a row out of the
/// view: lowering then refuses, naming `family`. The message, and the
/// span where the read has one.
fn refused_without(
    tag: &str,
    src: &str,
    family: &str,
    remove: impl FnOnce(&mut LoweringView),
) -> (String, Option<hale_syntax::Span>) {
    let whole = view(src);
    if let Err(e) = lower(&format!("{tag}_control"), &whole) {
        panic!("the control does not lower: {e}");
    }
    let mut cut = whole;
    remove(&mut cut);
    let (msg, span) = match lower(tag, &cut) {
        Err(CodegenError::Unsupported(msg)) => (msg, None),
        Err(CodegenError::UnsupportedAt(msg, span)) => (msg, Some(span)),
        other => panic!("`{family}`: expected a refusal for the missing row, got {other:?}"),
    };
    assert!(
        msg.contains(&format!("required `{family}` row")),
        "the refusal names the family `{family}`: {msg}"
    );
    (msg, span)
}

/// The text at `span` in `src`.
fn at(src: &str, span: hale_syntax::Span) -> &str {
    &src[span.start.as_usize()..span.end.as_usize()]
}

/// `lifecycle_order`: every spine is emitted from the plan, so a view
/// without one is refused before anything is lowered (no spine falls
/// back to an order of its own, and the main locus's head is placed by
/// no assumption).
#[test]
fn lifecycle_order_a_view_without_the_plan_is_refused() {
    let src = "locus Worker {\n    run { println(\"w\"); }\n}\nfn main() { Worker { }; }\n";
    let (msg, _) = refused_without("lifecycle", src, "lifecycle_order", |v| v.lifecycle = None);
    assert!(msg.contains("lifecycle plan"), "{msg}");
}

/// `bindings`: the prelude reads each of the entry's `bindings { }`
/// entries' transport, role and codec from its row, found by the entry's
/// identity; an entry with no row is refused at its topic, not lowered
/// from the entry's text.
#[test]
fn bindings_an_entry_without_its_row_is_refused_at_its_topic() {
    let src = "type Tick { n: Int = 0; }\n\
               topic TickTopic { payload: Tick; subject: \"ticks\"; }\n\
               main locus App {\n    bus { publish TickTopic; }\n    \
               bindings { TickTopic: unix(\"/tmp/missing_rows_ticks.sock\"); }\n}\n\
               fn main() { App { }; }\n";
    let (_, span) = refused_without("bindings", src, "bindings", |v| {
        v.bindings = hale_types::binding_rows::BindingRows::default()
    });
    assert_eq!(at(src, span.expect("located")), "TickTopic");
}

/// A parent that handles its child's failure: the handler table and the
/// handler's body read its routing row.
const ON_FAILURE: &str = "locus Alpha {\n    params { n: Int = 0; }\n    \
    closure boom { captures: n; epoch inline; }\n    \
    fn go() { self.n = self.n + 1; violate boom; }\n}\n\
    main locus App {\n    params { a: Alpha = Alpha { }; seen: Int = 0; }\n    \
    on_failure(x: Alpha, err: ClosureViolation) { self.seen = self.seen + x.n; }\n    \
    run() { self.a.go(); }\n}\nfn main() { App { }; }\n";

/// `handler_routing`: the handler table is one fn per routing row,
/// keyed by the row's site; a declared `on_failure` with no row is
/// refused at its declaration, never routed by the child's name.
#[test]
fn handler_routing_a_handler_without_its_row_is_refused() {
    let (msg, span) = refused_without("handlers", ON_FAILURE, "handler_routing", |v| {
        v.handlers = hale_types::handler_routing::HandlerRouting::default()
    });
    assert!(msg.contains("on_failure"), "{msg}");
    assert_eq!(at(ON_FAILURE, span.expect("located")), "App");
}

/// `ownership`: every locus-producing expression has its owner row
/// before lowering (F.39); an instantiation with none is refused rather
/// than given whatever owner the frame would default to.
#[test]
fn ownership_an_instantiation_without_its_owner_row_is_refused() {
    let src = "locus A {\n    run() { println(\"a\"); }\n}\nfn main() { A { }; }\n";
    let (msg, _) = refused_without("ownership", src, "ownership", |v| {
        v.owner_table = hale_types::ownership::OwnerTable::default()
    });
    assert!(msg.contains("owner") || msg.contains("binding-facts"), "{msg}");
}

/// `restart`: a recovery statement's `for N` bound is the restart rows'
/// entry, keyed by the statement's span. The rows cannot be emptied
/// without the handler rows around them, so the statement lowering
/// lowers is moved off its row instead: lowering then finds no bound for
/// it and refuses there, never restarting without one.
#[test]
fn restart_a_bound_without_its_row_is_refused_at_the_statement() {
    use hale_syntax::ast::{LocusMember, Stmt, TopDecl};
    let src = "locus Worker {\n    params { attempts: Int = 0; target: Int = 3; }\n    \
               closure reached { self.attempts ~~ self.target within 0; epoch birth; }\n    \
               birth() { self.attempts = self.attempts + 1; }\n    run() { println(\"RUN\"); }\n}\n\
               locus Coordinator {\n    on_failure(c: Worker, err: ClosureViolation) {\n        \
               restart(c) for 5;\n    }\n    run() { Worker { target: 3 }; }\n}\n\
               fn main() { let c = Coordinator { }; }\n";
    let (msg, span) = refused_without("restart", src, "restart", |v| {
        let mut moved = 0;
        for item in &mut v.merged.items {
            let TopDecl::Locus(l) = item else { continue };
            for m in &mut l.members {
                let LocusMember::Failure(fd) = m else { continue };
                for s in &mut fd.body.stmts {
                    if let Stmt::Recovery { span, .. } = s {
                        *span = hale_syntax::Span::new(span.start.as_usize() + 1, span.end.as_usize());
                        moved += 1;
                    }
                }
            }
        }
        assert_eq!(moved, 1, "the handler's one recovery statement");
    });
    assert!(msg.contains("`for N`"), "{msg}");
    assert_eq!(at(src, span.expect("located")), "estart(c) for 5;");
}

/// `generics`: a generic fn call's type arguments are its typed-body
/// row and its specialization the monomorph table's; with no row the
/// call is refused at the call, never inferred by lowering.
#[test]
fn generics_a_call_without_its_row_is_refused_at_the_call() {
    let src = "fn first<T>(x: T) -> T { return x; }\nfn main() { let v = first(42); println(\"v=\", v); }\n";
    let (msg, span) = refused_without("generics", src, "generics", |v| {
        v.typed = hale_types::typed_bodies::TypedBodies::default()
    });
    assert!(msg.contains("generic fn `first`"), "{msg}");
    assert_eq!(at(src, span.expect("located")), "first");
}

/// `surfaces`: storage routing asks the conformance column whether the
/// locus a `-> Interface` fn returns satisfies it; a pair with no row is
/// refused, never answered by comparing method names.
#[test]
fn surfaces_a_pair_without_its_conformance_row_is_refused() {
    let src = "interface Greeter { fn greet() -> Int; }\n\
               locus Hi { params { n: Int = 1; } fn greet() -> Int { return self.n; } }\n\
               fn make() -> Greeter { return Hi { n: 4 }; }\n\
               fn main() { let g = make(); println(g.greet()); }\n";
    let (msg, _) = refused_without("surfaces", src, "surfaces", |v| {
        v.typed = hale_types::typed_bodies::TypedBodies::default()
    });
    assert!(msg.contains("conformance column"), "{msg}");
}

/// `law_backstops`: the cross-pool value law judges every value use of
/// a literal the plan posts to another thread before lowering. A view
/// whose bubble plan posts a literal the law never judged (here, a plan
/// row added to a clean view) reaches lowering's backstop, which refuses
/// it at the literal, naming the missing judgment, and lowers no null
/// value.
#[test]
fn law_backstops_a_value_use_the_law_did_not_judge_is_refused_at_the_literal() {
    let src = "locus Ship { params { hull: Int = 0; } }\n\
               main locus World { run() { let a = Ship { hull: 7 }; println(a.hull); } }\n\
               fn main() { World { }; }\n";
    let (msg, span) = refused_without("law_backstops", src, "law_backstops", |v| {
        v.bubble.crosspool.insert(("World".to_string(), "Ship".to_string()), "World".to_string());
    });
    assert!(msg.contains("cross-pool value law"), "{msg}");
    assert_eq!(at(src, span.expect("located")), "Ship { hull: 7 }");
}

/// `entrypoint`: the `main locus` lowering deploys and the `fn main` it
/// emits are the entry row's; a view without the row is refused before
/// anything is lowered, never given a `fn main` found by name.
#[test]
fn entrypoint_a_view_without_the_entry_row_is_refused() {
    let src = "main locus App {\n    run() { println(\"hi\"); }\n}\nfn main() { App { }; }\n";
    let (msg, _) = refused_without("entrypoint", src, "entrypoint", |v| v.entry = None);
    assert!(msg.contains("entry row"), "{msg}");
}

/// `forms`: a ring buffer's slot is laid out by its form row's `cap`;
/// a declaration with no row is refused at its name, never laid out from
/// its written `cap =`.
#[test]
fn forms_a_form_without_its_row_is_refused_at_its_declaration() {
    let src = "type Frame { seq: Int; }\n\
               @form(ring_buffer, cap = 16)\n\
               locus FrameBuffer {\n    capacity { pool history of Frame; }\n}\n\
               fn main() { let _ = FrameBuffer { }; }\n";
    let (msg, span) = refused_without("forms", src, "forms", |v| {
        v.forms = hale_types::form_rows::FormRows::default()
    });
    assert!(msg.contains("has no form row"), "{msg}");
    assert_eq!(at(src, span.expect("located")), "FrameBuffer");
}

/// `sync_inference`: a hashmap's slot is laid out by its form row's
/// effective discipline, inference's pick included; a declaration with
/// no row is refused at its name, never laid out from its written
/// `sync =` (which would drop an inferred discipline).
#[test]
fn sync_inference_a_hashmap_without_its_row_is_refused_at_its_declaration() {
    let src = "type Entry { name: String; v: Int; }\n\
               @form(hashmap)\n\
               locus RegistryL {\n    capacity { pool entries of Entry indexed_by name; }\n}\n\
               fn main() { let _ = RegistryL { }; }\n";
    let (msg, span) = refused_without("sync_inference", src, "sync_inference", |v| {
        v.forms = hale_types::form_rows::FormRows::default()
    });
    assert!(msg.contains("has no form row"), "{msg}");
    assert_eq!(at(src, span.expect("located")), "RegistryL");
}

/// `topics`: where lowering holds a topic declaration's name (its
/// observation shape, a binding entry's topic, a rewritten send's), the
/// wire subject and payload are the topic's row; a topic with no row is
/// refused at its name, never registered under its name as a subject.
#[test]
fn topics_a_declared_topic_without_its_row_is_refused_at_its_name() {
    let src = "type T { n: Int = 0; }\ntopic Evt { payload: T; subject: \"evt\"; }\n\
               locus Rx {\n    bus { subscribe Evt as on_evt; }\n    fn on_evt(t: T) { println(t.n); }\n}\n\
               fn main() { let r = Rx { }; Evt <- T { n: 1 }; }\n";
    let (msg, span) = refused_without("topics", src, "topics", |v| {
        v.top.topics = hale_types::topic_identity::TopicRows::default()
    });
    assert!(msg.contains("the topic `Evt` has no row"), "{msg}");
    assert_eq!(at(src, span.expect("located")), "Evt");
}

/// `placement`: the deployment plan is the placement table's root, the
/// entry the entry row names; a table with no root for it is refused at
/// the entry, never deployed as if nothing were placed.
#[test]
fn placement_an_entry_without_its_root_row_is_refused_at_the_entry() {
    let src = "locus Worker {\n    run() { println(\"w\"); }\n}\n\
               main locus App {\n    params { w: Worker = Worker { }; }\n    placement { w: pinned; }\n}\n\
               fn main() { App { }; }\n";
    let (msg, span) = refused_without("placement", src, "placement", |v| {
        v.placement = hale_types::placement::PlacementTable::default()
    });
    assert!(msg.contains("no root"), "{msg}");
    assert_eq!(at(src, span.expect("located")), "App");
}

/// `alloc_summary`: a locus's arena and its members' scratch are elided
/// by its elision rows, and a free fn's caller-arena publish by its row;
/// a declaration with none is refused, never lowered on a guess (the
/// locus's read was a panic).
#[test]
fn alloc_summary_a_declaration_without_its_routing_rows_is_refused() {
    let src = "locus C {\n    params { n: Int = 0; }\n    fn bump() { self.n = self.n + 1; }\n}\n\
               main locus App {\n    params { c: C = C { }; }\n    run() { self.c.bump(); }\n}\n\
               fn main() { App { }; }\n";
    let (msg, _) = refused_without("alloc_summary", src, "alloc_summary", |v| {
        v.alloc_routing = hale_types::alloc_routing::AllocRouting::default()
    });
    assert!(msg.contains("no elision rows") || msg.contains("no caller-arena row"), "{msg}");
}
