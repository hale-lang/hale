//! The model's arrangement is the placement table's rows, projected
//! (F.40 phase 3, P1 part 4; `notes/f40-placement-correspondence.md`
//! § 2.4), held to the three identity contracts the design keeps apart.
//!
//! 1. **Shape identity.** The arrangement is outside the shape half, so
//!    `shape_hash` does not read it: each program's hash is the value the
//!    builder before the switch (#1315) stamped, and clearing the
//!    arrangement leaves it where it is.
//! 2. **Observation entity ids.** `obs_entity_ids` stamps subjects, locus
//!    declarations and bindings, never instances: the digest is the old
//!    build's, and clearing the arrangement leaves it where it is.
//! 3. **Arrangement-instance correspondence.** `LocusInstanceId` is the
//!    index in path order, so an added or removed path renumbers what
//!    sorts after it. No numeric id is promised stable: each table below
//!    is path → old id (#1315's build) → new id, per M row, and a
//!    consumer that holds an instance across builds joins by path.
//!
//! Each program loads through the frontend's snapshot, so the model reads
//! the table the snapshot demands.

use std::path::PathBuf;

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_model::ApplicationModel;

fn snapshot(src: &str) -> Snapshot {
    let program = hale_syntax::parse_source(src).expect("parse");
    Snapshot::from_program(program, Vec::new(), Config::check(false, false)).unwrap_or_else(|_| panic!("load"))
}

fn fixture(rel: &str) -> Snapshot {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(rel);
    Snapshot::load(&path, LoadMode::WholeSeed, &Disk, Config::check(path.is_dir(), false))
        .unwrap_or_else(|_| panic!("load {rel}"))
}

fn model(s: &Snapshot) -> &ApplicationModel {
    let m = s.demand_model().unwrap_or_else(|b| panic!("the model is blocked: {:?}", b.because));
    assert_eq!(m.validate(), Ok(()), "the projected arrangement is a lawful model");
    m
}

/// The arrangement, one line per instance in id order: `id path decl
/// replica domain parent`.
fn arrangement(m: &ApplicationModel) -> Vec<String> {
    let e = &m.entities;
    let r = &m.relations;
    e.locus_instances
        .iter()
        .enumerate()
        .map(|(i, inst)| {
            let domain = r
                .placed_in
                .iter()
                .find(|p| p.instance.0 as usize == i)
                .map(|p| e.thread_domains[p.domain.0 as usize].name.as_str())
                .unwrap_or("-");
            let parent = r
                .owns
                .iter()
                .find(|o| o.child.0 as usize == i)
                .map(|o| o.parent.0.to_string())
                .unwrap_or_else(|| "-".into());
            let replica = inst.replica.map(|r| r.to_string()).unwrap_or_else(|| "-".into());
            format!("{i} {} {} {replica} {domain} {parent}", inst.path, e.loci[inst.decl.0 as usize].name)
        })
        .collect()
}

/// The locus-grained holes, `decl reason`.
fn locus_holes(m: &ApplicationModel) -> Vec<String> {
    m.holes
        .iter()
        .filter_map(|h| match h.at {
            hale_model::EntityRef::LocusDecl(d) => {
                Some(format!("{} {}", m.entities.loci[d.0 as usize].name, h.reason))
            }
            _ => None,
        })
        .collect()
}

fn shape(m: &ApplicationModel) -> String {
    format!("{:016x}", hale_types::topology_projection::project_shape_hash(m))
}

fn obs(m: &ApplicationModel) -> String {
    format!("{:016x}", hale_model::obs_ids::digest(&hale_model::obs_ids::obs_entity_ids(m)))
}

/// Contracts 1 and 2: the hashes are the old build's, and neither reads
/// the arrangement (clearing its tables moves neither).
fn identities_hold(m: &ApplicationModel, old_shape: &str, old_obs: &str) {
    assert_eq!(shape(m), old_shape, "contract 1: shape_hash is the one #1315's builder stamped");
    assert_eq!(obs(m), old_obs, "contract 2: the obs-id digest is the one #1315's builder stamped");
    let mut bare = m.clone();
    bare.entities.locus_instances.clear();
    bare.relations.realizes.clear();
    bare.relations.placed_in.clear();
    bare.relations.owns.clear();
    bare.relations.affined_to.clear();
    assert_eq!(shape(&bare), old_shape, "contract 1: shape_hash does not read the arrangement");
    assert_eq!(obs(&bare), old_obs, "contract 2: the obs ids do not read the arrangement");
}

/// Contract 3's remapping for one program: each `(path, old id, new id)`,
/// `None` where the path is absent from that build. Every path the new
/// arrangement holds is listed.
fn remapped(m: &ApplicationModel, table: &[(&str, Option<u32>, Option<u32>)]) {
    let ids: Vec<(String, u32)> =
        m.entities.locus_instances.iter().enumerate().map(|(i, x)| (x.path.clone(), i as u32)).collect();
    for (path, _old, new) in table {
        let now = ids.iter().find(|(p, _)| p == path).map(|(_, i)| *i);
        assert_eq!(now, *new, "contract 3: `{path}`'s id");
    }
    let listed: Vec<&str> = table.iter().filter(|(_, _, n)| n.is_some()).map(|(p, _, _)| *p).collect();
    let all: Vec<&str> = ids.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(all, listed, "contract 3: every path the arrangement holds is in the table");
}

/// M-1: the arrangement is rooted at the root lowering deploys, never at
/// an imported `main`. The seed has no `main locus` of its own; the
/// builder before the switch rooted the arrangement at the library's
/// (the first `main` in program order). The table has no root, so the
/// arrangement is empty, and with it every domain the library's
/// placement named. The model dump's dispatch plan and capability lines
/// read the arrangement and move with it.
#[test]
fn the_arrangement_is_rooted_at_the_deployed_main() {
    let s = fixture("model_arrangement/imported_root/seed");
    let m = model(&s);
    let app = "__lib_tests__fixtures__model_arrangement__imported_root__lib___lib__App";
    let (side, w, leaf) = (format!("{app}.side"), format!("{app}.w"), format!("{app}.w.leaf"));
    remapped(m, &[(app, Some(0), None), (&side, Some(1), None), (&w, Some(2), None), (&leaf, Some(3), None)]);
    assert!(m.entities.thread_domains.is_empty(), "no domain is the imported root's: {:?}", m.entities.thread_domains);
    identities_hold(m, "ac9ffc19e280a6f2", "eb9f93ef2d701a6e");
}

const M3: &str = r#"
fn ignore_conn(s: std::io::tcp::Stream) { }

locus Listener {
    params {
        n: Int = 0;
    }
}

locus Tail { }

main locus App {
    params {
        a: Listener = Listener { };
        l: std::io::tcp::Listener = std::io::tcp::Listener {
            host: "127.0.0.1",
            port: 0,
            max_accepts: -1,
            on_connection: ignore_conn,
        };
        z: Tail = Tail { };
    }
    placement {
        l: cooperative(pool = io);
    }
}

fn main() { App { }; }
"#;

/// M-3: a field typed by a qualified stdlib path whose last segment
/// names a user locus was arranged as the user's locus, on the stdlib
/// field's pool. The table realizes the stdlib declaration, which the
/// user-only projection leaves out (U-4): the wrongly inferred
/// `Listener` goes, the later id shifts down, and `pool:io` is no
/// domain of the arrangement any more.
#[test]
fn the_wrongly_inferred_listener_is_removed() {
    let s = snapshot(M3);
    let m = model(&s);
    remapped(m, &[("App", Some(0), Some(0)), ("App.a", Some(1), Some(1)), ("App.l", Some(2), None), ("App.z", Some(3), Some(2))]);
    assert_eq!(arrangement(m), ["0 App App - main -", "1 App.a Listener - main 0", "2 App.z Tail - main 0"]);
    identities_hold(m, "f9d60755d77ddbcb", "e0c4bf36558405b3");
}

/// U-4 (M-2): the projection is user-only. The table places the stdlib
/// instance (here on pool `io`), and the arrangement leaves it out with
/// its subtree: its declaration is no entity of this model, and adding
/// one would be shape. No hole stands for it; the coverage is partial by
/// design, and the table answers every placement question.
#[test]
fn stdlib_instances_are_left_out_of_the_arrangement() {
    let s = snapshot(M3);
    let t = s.demand_placement().unwrap_or_else(|_| panic!("placement is blocked"));
    let stdlib: Vec<String> = t
        .instances
        .iter()
        .filter(|(_, r)| {
            r.realizes.as_ref().is_some_and(|d| d.site.universe == hale_types::placement::SiteUniverse::StdlibAnalysis)
        })
        .map(|(k, _)| k.path.iter().map(|s| s.field.as_str()).collect::<Vec<_>>().join("."))
        .collect();
    assert_eq!(stdlib, ["l"], "the table places the stdlib field");
    let m = model(&s);
    assert!(!m.entities.locus_instances.iter().any(|i| i.path.starts_with("App.l")));
    assert!(locus_holes(m).is_empty(), "{:?}", locus_holes(m));
}

const M5: &str = r#"
interface Router {
    fn route() -> Int;
}

locus RouterV1 {
    fn route() -> Int { return 1; }
}

type Held = Holder;

locus Leaf { }

locus Holder {
    params {
        t: Leaf = Leaf { };
    }
}

locus Tail { }

main locus App {
    params {
        h: Held = Holder { };
        r: Router = RouterV1 { };
        z: Tail = Tail { };
    }
    placement {
        h: pinned;
    }
}

fn main() { App { }; }
"#;

/// M-5: an aliased and a contract-typed user field, which the builder
/// before the switch skipped (it read a field's last segment and kept
/// only a user locus), are arranged as the declarations they realize,
/// with what nests under them; the later id shifts up.
#[test]
fn aliased_and_contract_typed_fields_are_added() {
    let s = snapshot(M5);
    let m = model(&s);
    remapped(
        m,
        &[
            ("App", Some(0), Some(0)),
            ("App.h", None, Some(1)),
            ("App.h.t", None, Some(2)),
            ("App.r", None, Some(3)),
            ("App.z", Some(1), Some(4)),
        ],
    );
    assert_eq!(
        arrangement(m),
        [
            "0 App App - main -",
            "1 App.h Holder - pinned:App.h 0",
            "2 App.h.t Leaf - pinned:App.h 1",
            "3 App.r RouterV1 - main 0",
            "4 App.z Tail - main 0",
        ]
    );
    identities_hold(m, "59a01f7667f8dbf9", "10ad616cf9728b3e");
}

/// M-9: below a held row whose source the producer cannot link the
/// table asserts nothing, and the declared type's default subtree the
/// builder before the switch fabricated there goes.
#[test]
fn nothing_is_arranged_below_an_unlinked_held_row() {
    let s = fixture("placement/held_unlinked.hl");
    let m = model(&s);
    remapped(
        m,
        &[
            ("App", Some(0), Some(0)),
            ("App.h", Some(1), Some(1)),
            ("App.h.roles", Some(2), Some(2)),
            ("App.h.roles.k", Some(3), None),
        ],
    );
    identities_hold(m, "61b5aeea6b3720f1", "af7c2c5ce5fa9bea");
}

/// M-7: the entry's implicit construction adds no row of its own. A
/// `main locus` no literal builds is arranged from its declaration's
/// defaults, as before, and a literal `fn main` builds besides the root
/// stays outside the arrangement, a birth its hole accounts for.
#[test]
fn the_entry_adds_no_arrangement_row_of_its_own() {
    let claims_only = snapshot("locus Leaf { }\nmain locus App {\n    params {\n        l: Leaf = Leaf { };\n    }\n}\n");
    let m = model(&claims_only);
    assert_eq!(arrangement(m), ["0 App App - main -", "1 App.l Leaf - main 0"]);
    identities_hold(m, "09f0e56b070be72d", "ccc8c3ec47f273ff");

    let with_side = snapshot(
        "locus Side { }\nlocus Leaf { }\nmain locus App {\n    params {\n        l: Leaf = Leaf { };\n    }\n}\n\
         fn main() {\n    let s = Side { };\n    App { };\n}\n",
    );
    let m = model(&with_side);
    assert_eq!(arrangement(m), ["0 App App - main -", "1 App.l Leaf - main 0"]);
    assert_eq!(
        locus_holes(m),
        ["Side instance born outside the arrangement: owner and placement resolve at runtime"]
    );
    identities_hold(m, "d41addd316edbb9f", "db323984f4298dfa");
}

/// The model's `replica` column is the replica row's own index, never an
/// ancestor's copied into a descendant (`validate` requires `None` on
/// every path whose last component is not a replica), while the path
/// carries it for every row under the replicated field.
#[test]
fn a_replica_index_stays_on_its_own_row() {
    let s = snapshot(
        "locus Leaf { }\nlocus Wide {\n    params {\n        inner: Leaf = Leaf { };\n    }\n}\n\
         main locus App {\n    params {\n        v: Wide = Wide { };\n    }\n    placement {\n        v: pinned(replicas = 2);\n    }\n}\n\
         fn main() { App { }; }\n",
    );
    let m = model(&s);
    assert_eq!(
        arrangement(m),
        [
            "0 App App - main -",
            "1 App.v[0] Wide 0 pinned:App.v[0] 0",
            "2 App.v[0].inner Leaf - pinned:App.v[0] 1",
            "3 App.v[1] Wide 1 pinned:App.v[1] 0",
            "4 App.v[1].inner Leaf - pinned:App.v[1] 3",
        ]
    );
    identities_hold(m, "e1db47d63e2839d3", "b7e5edf4a6b220ce");
}

/// Contract 3's construction templates: a path has no construction
/// component, so the root's two literals reach `App.gw.router` with two
/// realizations. The path is left out, and each declaration realized
/// there is a hole naming it; `App.gw`, on which they agree, is arranged.
#[test]
fn templates_that_disagree_at_a_path_leave_it_a_hole() {
    let s = snapshot(
        r#"
interface Router {
    fn route() -> Int;
}

locus RouterV1 {
    fn route() -> Int { return 1; }
}

locus RouterV2 {
    fn route() -> Int { return 2; }
}

locus Gateway {
    params {
        router: Router = RouterV1 { };
    }
}

main locus App {
    params {
        gw: Gateway = Gateway { };
    }
}

fn build(two: Bool) {
    if two {
        App { gw: Gateway { router: RouterV2 { } } };
    } else {
        App { };
    }
}

fn main() { build(true); }
"#,
    );
    let m = model(&s);
    assert_eq!(arrangement(m), ["0 App App - main -", "1 App.gw Gateway - main 0"]);
    let disagree = "the root's construction templates disagree at `App.gw.router`: the arrangement names no instance there";
    let holes = locus_holes(m);
    for decl in ["RouterV1", "RouterV2"] {
        assert!(holes.contains(&format!("{decl} {disagree}")), "{decl}: {holes:?}");
    }
    identities_hold(m, "7a3dab914ecdbe5c", "23e954518d6a8109");
}

/// U-5: the model projects `affined_to` from the table's resolved domain
/// affinity, never from the written entries. Two entries name pool `io`
/// and one carries `cores = 2..4`: one row for the pool, the set its one
/// worker may run on. `pinned(cores = { 5, 6 }, replicas = 2)` gives each
/// replica's thread its own core. A CPU set is a column of a thread
/// domain and never a domain of its own, so the domains are the ones the
/// instances run in, and a pinned field with no affinity has no row. The
/// model dump gains its `affined_to` lines only where a domain has a set;
/// shape identity's own check is separate (contract 1: the arrangement,
/// this column included, is outside the shape half).
#[test]
fn the_affinity_column_is_projected() {
    let s = snapshot(
        "locus Worker { }\n\nmain locus App {\n    params {\n        a: Worker = Worker { };\n        b: Worker = Worker { };\n        \
         w: Worker = Worker { };\n        free: Worker = Worker { };\n    }\n    placement {\n        \
         a: cooperative(pool = io, cores = 2..4);\n        b: cooperative(pool = io);\n        \
         w: pinned(cores = { 5, 6 }, replicas = 2);\n        free: pinned;\n    }\n}\n\nfn main() { App { }; }\n",
    );
    let m = model(&s);
    let domains: Vec<&str> = m.entities.thread_domains.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(domains, ["main", "pinned:App.free", "pinned:App.w[0]", "pinned:App.w[1]", "pool:io"]);
    let rows: Vec<(String, Vec<u32>)> = m
        .relations
        .affined_to
        .iter()
        .map(|a| (m.entities.thread_domains[a.domain.0 as usize].name.clone(), a.cores.0.clone()))
        .collect();
    assert_eq!(
        rows,
        [
            ("pinned:App.w[0]".to_string(), vec![5]),
            ("pinned:App.w[1]".to_string(), vec![6]),
            ("pool:io".to_string(), vec![2, 3]),
        ]
    );
    let dump = hale_types::model_builder::render_internal(m);
    let at = dump.find("affined_to (3):\n").expect("the dump lists the CPU sets");
    assert_eq!(
        &dump[at..at + dump[at..].find("dispatch_plan").unwrap()],
        "affined_to (3):\n  pinned:App.w[0] cpus [5]\n  pinned:App.w[1] cpus [6]\n  pool:io cpus [2,3]\n"
    );
    identities_hold(m, "6e7cb77bb037547e", "6f7ec26b0442cb4d");
}

/// U-5: a program with no affinity dumps no `affined_to` lines.
#[test]
fn a_program_with_no_affinity_dumps_no_cpu_sets() {
    let s = snapshot(M5);
    let m = model(&s);
    assert!(m.relations.affined_to.is_empty());
    assert!(!hale_types::model_builder::render_internal(m).contains("affined_to"));
}

/// Review of #1332: an unenumerated descendant in one construction
/// cannot be filled in from another construction's default subtree.
#[test]
fn unenumerated_descendants_in_any_template_are_not_arranged() {
    let source = r#"
interface Router { fn route() -> Int; }
locus RouterV1 { fn route() -> Int { return 1; } }
locus RouterV2 { fn route() -> Int { return 2; } }
locus Roles { params { router: Router = RouterV1 { }; } }
locus Holder { params { roles: Roles = Roles { }; } }
main locus App { params { h: Holder = Holder { }; } }
fn build_roles() -> Roles { return Roles { router: RouterV2 { } }; }
fn start(r: Roles) { App { h: Holder { roles: r } }; }
fn main() {
    if true { App { }; }
    else { start(Roles { router: RouterV2 { } }); }
}
"#;
    for (case, source) in [
        ("unlinked held value", source.to_string()),
        ("unenumerable factory", source.replace("roles: r", "roles: build_roles()")),
        ("alternatives within one construction", source
            .replace("App { h: Holder { roles: r } };", "App { h: if true { Holder { } } else { Holder { roles: r } } };")
            .replace("if true { App { }; }\n    else { start(Roles { router: RouterV2 { } }); }", "start(Roles { router: RouterV2 { } });")),
    ] {
        let s = snapshot(&source);
        let checked = s.demand_check().expect("check available");
        assert!(checked.diags.is_empty(), "{case}: {:?}", checked.diags);
        let m = model(&s);
        assert_eq!(arrangement(m), [
            "0 App App - main -",
            "1 App.h Holder - main 0",
            "2 App.h.roles Roles - main 1",
        ], "{case}");
        let expected = "RouterV1 the root's construction templates disagree at `App.h.roles.router`: the arrangement names no instance there";
        assert!(locus_holes(m).iter().any(|h| h == expected), "{case}: {:?}", locus_holes(m));
    }
    // Complete coverage of two equal realizations still publishes the path.
    let s = snapshot(&source.replace("roles: r", "roles: Roles { }"));
    let m = model(&s);
    assert!(arrangement(m).iter().any(|row| row.contains(" App.h.roles.router RouterV1 ")));
}

/// C3: API binding expressions are copied into generated params with
/// their source spans intact. Their source location is not their birth
/// context, and must not erase the adapter's model dispatch domains.
#[test]
fn copied_api_binding_expressions_keep_params_birth_provenance() {
    let src = include_str!("../../../tests/hale/api_binding_run_test.hl");
    let program = hale_syntax::parse_source(src).expect("parse API fixture");
    let mut config = Config::check(false, false);
    // This fixture exercises async_io. Derive its Linux model on every
    // test host; no native code is emitted or run here.
    config.target = hale_frontend::snapshot::Target {
        name: "x86_64-unknown-linux-gnu".into(),
        spec: hale_types::target::TargetSpec::parse("x86_64-unknown-linux-gnu").unwrap(),
        explicit: true,
    };
    let s = Snapshot::from_program(program, Vec::new(), config).unwrap_or_else(|_| panic!("load"));
    let m = model(&s);
    let graph = s.demand_ownership_graph().expect("ownership");
    for child in ["Table", "Tokens", "__ApiBinding", "__ApiHttp"] {
        let sites: Vec<_> = graph.sites.iter().filter(|site| site.child_ty == child).collect();
        assert!(!sites.is_empty(), "binding copy of {child} is a graph row");
        assert!(sites.iter().all(|site| site.params_default), "{sites:?}");
        let lid = m.entities.loci.iter().position(|l| l.name == child).unwrap();
        assert!(!m.holes.iter().any(|h| h.at == hale_model::EntityRef::LocusDecl(hale_model::LocusDeclId(lid as u32))
            && h.kind == hale_model::HoleKind::RuntimeInheritedPlacement), "{child}: {:?}", locus_holes(m));
    }
    let plan = hale_model::dispatch_plan::DispatchPlan::derive(m);
    let pings = plan.subjects.iter().find(|p| p.subject == "__api.call.Pings").expect("API call dispatch");
    assert_eq!(pings.publisher_domains, ["pool:__api_io"], "the API adapter's publisher is arranged: {pings:?}");
    assert_eq!(pings.subscriber_domains, ["pool:work"]);
    assert_eq!(locus_holes(m), [
        "__ApiHttpPeer instance born outside the arrangement: owner and placement resolve at runtime",
        "__ApiPeer instance born outside the arrangement: owner and placement resolve at runtime",
    ], "connection peers remain dynamic");
}

/// C3: even an overlapping span cannot turn a method-body birth into a
/// params default. The model must keep the child's dynamic-placement
/// hole beside its arranged instance.
#[test]
fn body_birth_with_a_params_span_stays_dynamic() {
    use hale_syntax::ast::{LocusMember, TopDecl};
    let mut p = hale_syntax::parse_source(r#"
        locus Kid { }
        main locus App {
            params { k: Kid = Kid { }; }
            run() { let extra = Kid { }; }
        }
        fn main() { App { }; }
    "#).unwrap();
    for item in &mut p.items {
        if let TopDecl::Locus(l) = item {
            for member in &mut l.members {
                if let LocusMember::Params(pb) = member {
                    pb.span = l.span;
                }
            }
        }
    }
    let s = Snapshot::from_program(p, Vec::new(), Config::check(false, false)).unwrap_or_else(|_| panic!("load"));
    let m = model(&s);
    let graph = s.demand_ownership_graph().expect("ownership");
    let flags: Vec<_> = graph.sites.iter().filter(|s| s.child_ty == "Kid").map(|s| s.params_default).collect();
    assert_eq!(flags, [true, false]);
    assert!(locus_holes(m).iter().any(|h| h.starts_with("Kid instance born outside the arrangement")), "{:?}", locus_holes(m));
    assert_eq!(arrangement(m), ["0 App App - main -", "1 App.k Kid - main 0"]);
}

/// C3: a qualified stdlib birth whose leaf matches a user declaration
/// belongs to the stdlib, so it cannot add a placement hole to that
/// user's locus. The model intentionally contains user declarations.
#[test]
fn qualified_birth_does_not_join_an_unrelated_user_name() {
    let s = snapshot(r#"
        locus Stream { }
        main locus App {
            params { own: Stream = Stream { }; }
            run() {
                let external = std::io::tcp::Stream { conn_fd: -1, owns_fd: false };
            }
        }
        fn main() { App { }; }
    "#);
    let m = model(&s);
    assert!(locus_holes(m).is_empty(), "{:?}", locus_holes(m));
    assert_eq!(arrangement(m), ["0 App App - main -", "1 App.own Stream - main 0"]);
}
