//! A revealed secret is consumed in its statement (spec/semantics.md
//! § "`@sealed` and a revealed secret").
//!
//! `std::secret::Credential.reveal()` / `reveal_text()` may be called only
//! in a locus method, and the value must reach a wire write, a comparison
//! or a `@secret` parameter in the same statement, through composition
//! that cannot keep it. A `@secret` parameter is held to the same inside
//! its fn. Four Postgres sites are allowed by qualified name until `pq`
//! takes a `Credential`; a fifth is refused.

use std::path::{Path, PathBuf};
use std::process::Command;

/// `hale check` over `files` (`(relative path, source)`), the entry the
/// first one; returns (success, output).
fn check(files: &[(&str, &str)], tag: &str) -> (bool, String) {
    let d: PathBuf = std::env::temp_dir().join(format!("hale_secret_reveal_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    for (rel, src) in files {
        let f = d.join(rel);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, src).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["check", &d.join(files[0].0).to_string_lossy()])
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&d);
    (out.status.success(), text)
}

fn one(src: &str, tag: &str) -> (bool, String) {
    check(&[("main.hl", src)], tag)
}

const CONSUMED: &str = "must be consumed in the statement that";

/// A locus method's body around `stmt`, with a credential field.
fn method(stmt: &str, extra: &str) -> String {
    format!(
        "{extra}\nlocus Api {{\n    params {{ token: std::secret::Credential = std::secret::Credential {{ vault: \"api\" }}; last: String = \"\"; }}\n    fn go(u: std::http::Url, other: String) -> Bool {{\n        {stmt}\n        return true;\n    }}\n}}\nfn main() {{ let a = Api {{ }}; }}\n"
    )
}

#[test]
fn a_reveal_that_outlives_its_statement_is_refused() {
    let cases = [
        ("let", "let t = self.token.reveal_text();", "is bound to `t`"),
        ("interpolated_let", "let h = \"Bearer \" + self.token.reveal_text();", "is bound to `h`"),
        ("field", "self.last = self.token.reveal_text();", "is stored in a field of `self`"),
        ("process", "let r = std::process::run(\"curl\\n-H\\n\" + self.token.reveal_text()) or std::process::ProcessOutput { code: 1, signal: 0, stdout: \"\", stderr: \"\" };", "reaches `std::process::run`"),
        ("unused", "self.token.reveal_text();", "is not consumed"),
    ];
    for (tag, stmt, why) in cases {
        let (ok, out) = one(&method(stmt, ""), &format!("refused_{tag}"));
        assert!(!ok && out.contains(CONSUMED) && out.contains(why), "{tag}: {out}");
    }
    // returned
    let src = "locus Api {\n    params { token: std::secret::Credential = std::secret::Credential { vault: \"api\" }; }\n    fn dsn() -> String { return self.token.reveal_text(); }\n}\nfn main() { let a = Api { }; }\n";
    let (ok, out) = one(src, "refused_return");
    assert!(!ok && out.contains("here it is returned"), "{out}");
    // stored in a locus literal
    let src = "locus Holder { params { s: String = \"\"; } }\nlocus Api {\n    params { token: std::secret::Credential = std::secret::Credential { vault: \"api\" }; }\n    fn go() { let h = Holder { s: self.token.reveal_text() }; }\n}\nfn main() { let a = Api { }; }\n";
    let (ok, out) = one(src, "refused_locus_literal");
    assert!(!ok && out.contains("is stored in a `Holder { … }`"), "{out}");
}

#[test]
fn a_reveal_in_a_free_fn_is_refused() {
    let src = "fn token() -> Int {\n    let n = len(std::secret::Credential { vault: \"t\" }.reveal_text());\n    return n;\n}\nfn main() { let n = token(); }\n";
    let (ok, out) = one(src, "free_fn");
    assert!(!ok && out.contains("may be called only in a locus method, and `token` is a free fn"), "{out}");
}

#[test]
fn a_reveal_consumed_in_its_statement_is_accepted() {
    let cases = [
        // the header line is the wire write
        ("http_request", "let r = std::http::request(std::http::ClientRequest { method: \"POST\", url: u, headers: \"Authorization: Bearer \" + self.token.reveal_text(), body: b\"\" }) or std::http::ClientResponse { status: 0, headers: \"\", body: b\"\" };"),
        // a form body, through a pure fn and a stdlib conversion
        ("http_post_pure_fn", "let r = std::http::post(\"http://127.0.0.1:1/t\", std::bytes::from_string(\"secret=\" + enc(self.token.reveal_text())), \"text/plain\") or std::http::ClientResponse { status: 0, headers: \"\", body: b\"\" };"),
        // compared
        ("compare", "if self.token.reveal_text() == other { self.last = \"same\"; }"),
        ("matches", "if self.token.matches(std::bytes::from_string(other)) { self.last = \"same\"; }"),
        // straight onto a socket
        ("tcp", "std::io::tcp::send_fd(3, self.token.reveal()) or discard;"),
    ];
    let enc = "fn enc(s: String) -> String {\n    let mut out = \"\";\n    let mut i = 0;\n    while i < len(s) { out = out + s[i..(i + 1)]; i = i + 1; }\n    return out;\n}\n";
    for (tag, stmt) in cases {
        let (ok, out) = one(&method(stmt, enc), &format!("accepted_{tag}"));
        assert!(ok && !out.contains(CONSUMED), "{tag}: {out}");
    }
}

#[test]
fn a_secret_parameter_is_a_consumer_its_body_is_held_to() {
    // a chain of `@secret` parameters down to the socket is sound
    let chain = "locus Conn {\n    params { token: std::secret::Credential = std::secret::Credential { vault: \"n\" }; fd: Int = 3; }\n    fn write_all(@secret b: Bytes) -> Int { return std::io::tls::send_bytes(self.fd, b); }\n    fn write(@secret b: Bytes) -> Bool { return self.write_all(b) >= 0; }\n    fn write_str(@secret s: String) -> Bool { return self.write(std::bytes::from_string(s)); }\n    fn hello() -> Bool { return self.write_str(\"PASS \" + self.token.reveal_text() + \"\\r\\n\"); }\n}\nfn main() { let c = Conn { }; }\n";
    let (ok, out) = one(chain, "secret_chain");
    assert!(ok && !out.contains(CONSUMED), "{out}");
    // a `@secret` parameter that is returned, or bound, is refused in
    // its own fn — the marker is no hole
    let returned = "fn line(@secret pass: String) -> String { return \"PASS \" + pass; }\nfn main() { let l = line(\"x\"); }\n";
    let (ok, out) = one(returned, "secret_returned");
    assert!(!ok && out.contains("the `@secret` parameter `pass` of `line`") && out.contains("here it is returned"), "{out}");
    // a derivation is not a consumer: a derived key is password-equivalent
    let derived = "fn key(@secret pass: String) -> Bytes {\n    let k = std::crypto::sha256(std::bytes::from_string(pass));\n    return k;\n}\nfn main() { let k = key(\"x\"); }\n";
    let (ok, out) = one(derived, "secret_derived");
    assert!(!ok && out.contains("is bound to `k`"), "{out}");
}

#[test]
fn the_postgres_sites_are_allowed_by_name_and_a_fifth_is_refused() {
    let site = "fn role_password(role: String) -> String {\n    return std::secret::Credential { vault: role }.reveal_text();\n}\n";
    // the named site: a warning naming the deferral
    let (ok, out) = check(&[("dna/core/memory_schema.hl", site)], "pq_named");
    assert!(
        ok && out.contains("warning:") && out.contains("allowed here by name, in `dna::role_password`") && out.contains("Deferred: `pq` takes a `std::secret::Credential`"),
        "{out}"
    );
    // the same fn anywhere else is a fifth site: refused
    let (ok, out) = check(&[("dna/core/other.hl", site)], "pq_fifth");
    assert!(!ok && out.contains("may be called only in a locus method") && !out.contains("allowed here by name"), "{out}");
}
