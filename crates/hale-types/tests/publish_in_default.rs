//! GH #1565: a publish written in a default is refused at check. A
//! default is evaluated where the literal or call that leaves it is,
//! which holds none of the declaration's bus, so each of these used to
//! check and then fail `hale build` with "unknown identifier `Pings`"
//! (a check/build divergence: the programs are raw literals, so
//! `corpus_check_build_agreement` harvests them, and the sweep would have
//! built each one the checker accepted).
//! A method the default calls may publish.

#[path = "support/entries.rs"]
mod entries;

fn errors(src: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(src).unwrap_or_else(|d| panic!("parse: {:?}", d[0].message));
    entries::check_program(&program).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect()
}

const REFUSAL: &str = "a publish in a default: a default is evaluated where the literal or call that leaves it is";

#[test]
fn a_publish_in_a_main_locus_param_default_is_refused() {
    let errs = errors(
        r#"type Ping { id: Int = 0; }
topic Pings { payload: Ping; subject: "t.pings"; }
locus Sub {
    bus { subscribe Pings as on_ping; }
    fn on_ping(p: Ping) { println("ping"); }
}
main locus App {
    params { sub: Sub = Sub { }; n: Int = { Pings <- Ping { id: 1 }; 0 }; }
    bus { publish Pings; }
    run() { }
}
fn main() { App { }; }
"#,
    );
    assert!(errs.iter().any(|e| e.contains(REFUSAL)), "{errs:#?}");
}

#[test]
fn a_publish_in_a_child_param_default_is_refused() {
    let errs = errors(
        r#"type Ping { id: Int = 0; }
topic Pings { payload: Ping; subject: "t.pings"; }
locus Sub {
    bus { subscribe Pings as on_ping; }
    fn on_ping(p: Ping) { println("ping"); }
}
locus Kid {
    params { n: Int = { Pings <- Ping { id: 1 }; 0 }; }
    bus { publish Pings; }
}
main locus App {
    params { sub: Sub = Sub { }; k: Kid = Kid { }; }
    run() { }
}
fn main() { App { }; }
"#,
    );
    assert!(errs.iter().any(|e| e.contains(REFUSAL)), "{errs:#?}");
}

#[test]
fn a_publish_in_a_record_field_default_is_refused() {
    let errs = errors(
        r#"type Ping { id: Int = 0; }
topic Pings { payload: Ping; subject: "t.pings"; }
type Cfg { n: Int = { Pings <- Ping { id: 1 }; 0 }; }
locus Sub {
    bus { subscribe Pings as on_ping; }
    fn on_ping(p: Ping) { println("ping"); }
}
main locus App {
    params { sub: Sub = Sub { }; }
    bus { publish Pings; }
    birth() { let c = Cfg { }; println(to_string(c.n)); }
    run() { }
}
fn main() { App { }; }
"#,
    );
    assert!(errs.iter().any(|e| e.contains(REFUSAL)), "{errs:#?}");
}

#[test]
fn a_publish_in_a_method_param_default_is_refused() {
    let errs = errors(
        r#"type Ping { id: Int = 0; }
topic Pings { payload: Ping; subject: "t.pings"; }
locus Sub {
    bus { subscribe Pings as on_ping; }
    fn on_ping(p: Ping) { println("ping"); }
}
main locus App {
    params { sub: Sub = Sub { }; }
    bus { publish Pings; }
    fn note(n: Int = { Pings <- Ping { id: 1 }; 0 }) -> Int { return n; }
    birth() { println(to_string(self.note())); }
    run() { }
}
fn main() { App { }; }
"#,
    );
    assert!(errs.iter().any(|e| e.contains(REFUSAL)), "{errs:#?}");
}

/// The way out: a default that calls a method that publishes checks (and
/// builds: the method's body is a locus body, its bus in scope).
#[test]
fn a_default_calling_a_method_that_publishes_checks() {
    let errs = errors(
        r#"type Ping { id: Int = 0; }
topic Pings { payload: Ping; subject: "t.pings"; }
locus Sub {
    bus { subscribe Pings as on_ping; }
    fn on_ping(p: Ping) { println("ping"); }
}
locus Pub {
    bus { publish Pings; }
    fn announce() -> Int { Pings <- Ping { id: 1 }; return 0; }
}
main locus App {
    params { sub: Sub = Sub { }; p: Pub = Pub { }; n: Int = self.p.announce(); }
    run() { }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(errs, Vec::<String>::new());
}
