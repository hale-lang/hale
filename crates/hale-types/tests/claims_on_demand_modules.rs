//! GH #2218: `std::m::*` holds the declarations of the stdlib files
//! appended on demand (`io_tcp_listener.hl`, `http_read.hl`), as a
//! `forbid reaches` target and as an `avoiding` gate.

#[path = "support/entries.rs"]
mod entries;
use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    entries::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

/// A server that holds a `std::io::tcp::Listener` (declared in
/// `io_tcp_listener.hl`, appended only to a program that names it) and a
/// locus that touches no socket. `CLAIMS` goes in the main locus.
const LISTENER_HOLDER: &str = r#"
    locus Server {
        params { l: std::io::tcp::Listener = std::io::tcp::Listener { port: 0 }; }
        fn stop() { self.l.accept_direct(); }
    }

    locus Quiet {
        params { n: Int = 0; }
        fn bump() -> Int { return self.n + 1; }
    }

    group server = { Server };
    group quiet = { Quiet };
    group tcp = { std::io::tcp::* };

    main locus App {
        params { s: Server = Server { }; q: Quiet = Quiet { }; }
        claims { CLAIMS }
    }

    fn main() { App { }; }
"#;

/// The target: `std::io::tcp::*` holds the Listener, so a walk entering
/// `Listener::accept_direct` hits it. It used to hold: the module was
/// built from the bundled stdlib alone, and the Listener's file is not
/// in it.
#[test]
fn a_module_target_holds_its_on_demand_file() {
    let ds = diags(&LISTENER_HOLDER.replace("CLAIMS", "c: forbid reaches(server, tcp);"));
    let hit = ds
        .iter()
        .find(|m| m.contains("claim `c` violated"))
        .unwrap_or_else(|| panic!("{ds:?}"));
    assert!(hit.contains("`Server::stop` -> `std::io::tcp::Listener::accept_direct`"), "{hit}");
}

/// The control: a locus of the same program that touches no socket
/// still holds the claim.
#[test]
fn a_module_target_holds_over_a_locus_that_never_enters_it() {
    let ds = diags(&LISTENER_HOLDER.replace("CLAIMS", "c: forbid reaches(quiet, tcp);"));
    assert!(!ds.iter().any(|m| m.contains("claim `c`")), "{ds:?}");
}

/// A fn that reads a streamed body and whose every path to `secret_use`
/// runs through `std::http` (`http_read.hl`, on demand).
const BODY_READER: &str = r#"
    fn fetch(r: std::http::BodyReader) -> Int {
        let b = r.chunk() or { return 0; };
        return len(b);
    }

    fn quiet(n: Int) -> Int { return n + 1; }

    group fetcher = { fetch };

    main locus App {
        params { n: Int = 0; }
        claims { CLAIMS }
    }

    fn main() { App { }; println(quiet(1)); }
"#;

/// The gate: `avoiding std::http::*` masks `std::http::BodyReader`'s
/// bodies, so the claim holds. It used to be refused with a witness
/// running straight through `BodyReader::chunk`.
#[test]
fn a_module_gate_masks_its_on_demand_file() {
    let ds = diags(&BODY_READER.replace(
        "CLAIMS",
        "c: forbid reaches(fetcher, effects(secret_use)) avoiding std::http::*;",
    ));
    assert!(!ds.iter().any(|m| m.contains("claim `c`")), "{ds:?}");
}

/// The canary for the gate: without it, the same claim is violated
/// through the reader.
#[test]
fn without_the_gate_the_on_demand_file_is_reached() {
    let ds = diags(&BODY_READER.replace("CLAIMS", "c: forbid reaches(fetcher, effects(secret_use));"));
    let hit = ds
        .iter()
        .find(|m| m.contains("claim `c` violated"))
        .unwrap_or_else(|| panic!("{ds:?}"));
    assert!(hit.contains("std::http::BodyReader::chunk"), "{hit}");
}

/// `program` carries the bodies of the stdlib files appended to it as
/// roots; a gate over their module masks them, and is no overlap with
/// `program`, whose author named none of them.
#[test]
fn a_module_gate_is_no_overlap_with_program_s_appended_stdlib() {
    let ds = diags(&BODY_READER.replace(
        "CLAIMS",
        "c: forbid reaches(program, effects(secret_use)) avoiding std::http::*;",
    ));
    assert!(!ds.iter().any(|m| m.contains("claim `c`")), "{ds:?}");
}
