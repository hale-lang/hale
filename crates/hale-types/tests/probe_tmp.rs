use std::collections::BTreeMap;
use hale_types::Bundle;

#[test]
fn probe() {
    let src = r#"
        interface Notifier { fn send(n: Int) -> Int; }
        locus Email { fn send(n: Int) -> Int { return n; } }
        locus Audit { fn send(a: Int, b: Int) -> Int { return a + b; } }
        type Route { handler: Notifier; }
        locus A {
            fn go(n: Int) -> Int {
                let r = Route { handler: Email { } };
                return r.handler.send(n);
            }
        }
        group a_side = { A };
        group sinks = { Audit };
        main locus App {
            params { a: A = A { }; }
            claims { iso: forbid reaches(a_side, sinks); }
        }
        fn main() { App { }; }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let model = hale_types::model_builder::derive_application_model(&bundle);
    for h in &model.holes {
        println!("HOLE {:?} at {:?}: {}", h.kind, h.at, h.reason);
    }
    for c in &model.relations.calls {
        println!("CALL {:?} -> {:?} {:?}", c.from, c.to, c.dispatch);
    }
    for d in &model.relations.dead_interface_calls {
        println!("DEAD {:?} {} . {}", d.from, d.interface, d.method);
    }
    for (i, f) in model.entities.functions.iter().enumerate() {
        println!("FN {} {}", i, f.name);
    }
    let programs_v: Vec<&hale_syntax::ast::Program> = vec![&program];
    let top = hale_types::resolve::build_top_scope(&bundle).0;
    let graph = hale_types::bus_graph::build_bus_graph(&bundle, &top);
    let (diags, outcomes, _a) =
        hale_types::claims::claims_report_with_identities(&programs_v, &graph, &[]);
    for o in &outcomes {
        println!("OUTCOME {} {:?}", o.name, o.result);
    }
    for d in &diags {
        println!("DIAG {:?} {}", d.span, d.message);
    }
}
