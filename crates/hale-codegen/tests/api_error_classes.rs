//! spec/api.md § Error classes: a variant of a handler's error type that
//! declares a class (`@not_found Missing`) is answered with the class's
//! status by each transport, and an unclassed variant, or an error type
//! that is a record, keeps the handler error's status. One program serves
//! one surface over HTTP (`BIND`), gRPC (`BIND2`) and a Unix socket
//! (`SOCK`).

use super::api_grpc::h2_client::Client;
use super::api_grpc::{call, message, JSON, WAIT};
use super::api_grpc::http_rpc::*;

const PROGRAM: &str = r#"
type OrderError = enum { @not_found Missing, @conflict Taken(String), Closed, @unavailable Busy(Int, String), @conflict refusal(String) };
type Plain { code: String; }
type Ask { id: Int; }

api Shop {
    rpc Orders::get;
    rpc Orders::plain;
}

locus Tokens {
    fn principal(token: String) -> std::api::Principal { return std::api::Principal { mode: "bearer", name: token }; }
    fn refused() -> String { return "no such token"; }
}

locus Orders {
    fn get(a: Ask) -> Int fallible(OrderError) {
        if a.id == 1 { fail OrderError::Missing; }
        if a.id == 2 { fail OrderError::Taken("bob"); }
        if a.id == 3 { fail OrderError::Closed; }
        if a.id == 4 { fail OrderError::Busy(2, "later"); }
        if a.id == 5 { fail OrderError::refusal("x"); }
        return a.id;
    }
    fn plain(a: Ask) -> Int fallible(Plain) {
        fail Plain { code: "no" };
    }
}

main locus Shop {
    params {
        bearer: Tokens = Tokens { };
        orders: Orders = Orders { };
        web: api::Exposure = api::serve(Shop, http::Rpc { bind: std::env::var("BIND"), codec: json, principals: self.bearer }, as: "web", receivers: { Orders: self.orders }, bound: 8, on_full: refuse);
        rpc: api::Exposure = api::serve(Shop, grpc::Rpc { bind: std::env::var("BIND2"), codec: json, principals: self.bearer }, as: "rpc", receivers: { Orders: self.orders }, bound: 8, on_full: refuse);
        sock: api::Exposure = api::serve(Shop, unix::Rpc { path: std::env::var("SOCK") }, as: "sock", receivers: { Orders: self.orders }, bound: 8, on_full: refuse);
    }
    run() {
        let trigger = std::env::var("TRIGGER");
        while !self.draining && (std::io::fs::read_file(trigger) or "") == "" {
            std::time::sleep(10ms);
        }
        self.web.stop();
        self.rpc.stop();
        self.sock.stop();
        println("stopped");
    }
}

fn main() { Shop { }; }
"#;

#[test]
fn each_transport_answers_a_handler_error_with_its_class() {
    let bin = super::api_grpc::build_variant(PROGRAM, "api_error_classes");
    let server = Server::start(&bin, &[]);
    server.ready();

    // HTTP: the class's status, the error's JSON as the body
    let post = |id: i64, member: &str| request(server.port, "POST", &format!("/call/Orders::{member}"), &[bearer("x")], Some(&format!("{{\"id\":{id}}}")));
    // a classed one names its class in `Hale-Error-Class`, which tells it
    // from a refusal of the same status whatever its body (a variant named
    // `refusal` included); an unclassed one, and a refusal, carry none
    for (id, status, body, class) in [
        (0, 200, "0", None),
        (1, 404, "\"Missing\"", Some("not_found")),
        (2, 409, "{\"Taken\":\"bob\"}", Some("conflict")),
        (3, 422, "\"Closed\"", None),
        (4, 503, "{\"Busy\":[2,\"later\"]}", Some("unavailable")),
        (5, 409, "{\"refusal\":\"x\"}", Some("conflict")),
    ] {
        let r = post(id, "get");
        assert_eq!((r.status, r.body.trim(), r.header("Hale-Error-Class")), (status, body, class), "HTTP, id {id}: {r:?}");
    }
    let refused = request(server.port, "POST", "/call/Orders::get", &[bearer("x"), h("Hale-Surface-Digest", "fnv1a64:0000000000000000")], Some("{\"id\":1}"));
    assert_eq!((refused.status, refused.header("Hale-Error-Class")), (409, None), "a refusal names no class: {refused:?}");
    let r = post(1, "plain");
    assert_eq!((r.status, r.body.trim()), (422, "{\"code\":\"no\"}"), "a record error is unclassed: {r:?}");

    // gRPC: the class's code, FAILED_PRECONDITION for none
    let mut c = Client::connect_at(server.port2());
    for (i, (id, code)) in [(1, "5"), (2, "10"), (3, "9"), (4, "14")].iter().enumerate() {
        let stream = 2 * i as u32 + 1;
        call(&mut c, stream, "/Shop/Orders.get", Some("x"), None, JSON, &message(&format!("{{\"id\":{id}}}")));
        let got = c.response(stream, WAIT);
        let status = got.headers.iter().find(|(k, _)| k == "grpc-status").map(|(_, v)| v.as_str());
        assert_eq!(status, Some(*code), "gRPC, id {id}: {got:?}");
        let message = got.headers.iter().find(|(k, _)| k == "grpc-message").map(|(_, v)| v.as_str());
        assert_eq!(message, Some("handler_error"), "gRPC, id {id}: {got:?}");
    }

    // Unix: the reply line names the class beside the error
    let ask = |id: i64| unix_ask(&server.sock(), &format!("{{\"call\":\"Orders::get\",\"payload\":{{\"id\":{id}}},\"id\":{id}}}"));
    let line = ask(1);
    assert!(line.contains("\"ok\":false,\"error\":\"Missing\",\"class\":\"not_found\""), "{line}");
    let line = ask(3);
    assert!(line.contains("\"ok\":false,\"error\":\"Closed\"") && !line.contains("\"class\""), "an unclassed variant names none: {line}");

    let done = server.finish();
    assert!(done.status.success(), "the program ended cleanly: {:?}\n{}\n{}", done.status, done.stdout, done.stderr);
}
