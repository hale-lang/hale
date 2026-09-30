//! A revealed secret is consumed in its statement (spec/semantics.md
//! § "`@sealed` and a revealed secret").
//!
//! `std::secret::Credential.reveal()` / `reveal_text()` may be called only
//! in a locus method, and the value must reach a wire write's payload, a
//! comparison of the whole value or a `@secret` parameter in the same
//! statement, through composition that cannot keep it. A `@secret`
//! parameter is held to the same inside its fn. Four Postgres sites are
//! allowed by qualified name and pinned body until `pq` takes a
//! `Credential`; a fifth, or one of the four changed, is refused.

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
    assert!(!ok && out.contains("may be called only in a locus method, and the free fn `token` is not one"), "{out}");
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
        // a match whose patterns bind nothing compares the whole value
        ("match_literals", "match self.token.reveal_text() { \"a\" -> { self.last = \"a\"; }, _ -> { self.last = \"other\"; } }"),
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
    assert!(!ok && out.contains("the `@secret` parameter `pass` of the free fn `line`") && out.contains("here it is returned"), "{out}");
    // a derivation is not a consumer: a derived key is password-equivalent
    let derived = "fn key(@secret pass: String) -> Bytes {\n    let k = std::crypto::sha256(std::bytes::from_string(pass));\n    return k;\n}\nfn main() { let k = key(\"x\"); }\n";
    let (ok, out) = one(derived, "secret_derived");
    assert!(!ok && out.contains("is bound to `k`"), "{out}");
}

/// Each bypass the review of PR #1216 ran end to end (`hale check` said
/// `ok`, the program printed the secret), each refused where it is.
#[test]
fn the_reviewed_bypasses_are_refused() {
    let in_method = [
        // a pattern binds the revealed value
        ("match_binds", "match self.token.reveal_text() { s -> { self.last = s; } }", "is bound to `s` by a `match` pattern"),
        // a pattern binding is a value the pass does not type: revealing
        // through it counts
        ("match_rebinds_receiver", "self.last = match self.token { c -> c.reveal_text() };", "is stored in a field of `self`"),
        // an ordering is an oracle, not a comparison
        ("ordered", "if self.token.reveal_text() < \"m\" { self.last = \"low\"; }", "decides a branch"),
        // a comparison of a part answers a question about that part
        ("compared_part", "let hit = self.token.reveal_text()[0..1] == other;", "is compared after it is taken apart"),
        ("matched_part", "match self.token.reveal_text()[0..1] { \"a\" -> { self.last = \"a\"; }, _ -> { } }", "is compared after it is taken apart"),
        // a pure stdlib fn with an out-parameter keeps its argument
        ("out_param", "if stash(std::str::builder_new(), self.token.reveal_text()) { self.last = \"kept\"; }", "reaches `stash`"),
        // a locus param's insides outlive the call
        ("param_field", "if put(Holder { }, self.token.reveal_text()) { self.last = \"kept\"; }", "reaches `put`"),
        // a fn value is a call the pass cannot follow
        ("fn_value", "if apply(keep, self.token.reveal_text()) { self.last = \"kept\"; }", "reaches `apply`"),
        // the URL is not the wire's: an error repeats the host back
        ("post_url", "let r = std::http::post(self.token.reveal_text() + \"://x/\", b\"\", \"text/plain\") or std::http::ClientResponse { status: 0, headers: \"\", body: b\"\" };", "is argument 1 of `std::http::post`, which is not its payload"),
        // the `::` method spelling is the same reveal
        ("path2", "let t = self.token::reveal_text();", "is bound to `t`"),
    ];
    let helpers = "locus Holder { params { s: String = \"\"; } }\n\
                   fn stash(sb: Bytes, s: String) -> Bool { std::str::builder_append(sb, s); return true; }\n\
                   fn put(h: Holder, s: String) -> Bool { h.s = s; return true; }\n\
                   fn keep(s: String) -> Bool { return len(s) > 0; }\n\
                   fn apply(f: fn(String) -> Bool, s: String) -> Bool { return f(s); }\n";
    for (tag, stmt, why) in in_method {
        let (ok, out) = one(&method(stmt, helpers), &format!("bypass_{tag}"));
        assert!(!ok && out.contains(why), "{tag}: {out}");
    }
    // members the walk used to skip: a param default, `on_failure`
    let default = "locus Api {\n    params { t: String = std::secret::Credential { vault: \"api\" }.reveal_text(); }\n}\nfn main() { let a = Api { }; }\n";
    let (ok, out) = one(default, "bypass_param_default");
    assert!(!ok && out.contains("is the default of `t`, which the locus keeps"), "{out}");
    let failure = "locus Child { params { n: Int = 0; } }\nlocus Api {\n    params { token: std::secret::Credential = std::secret::Credential { vault: \"api\" }; seen: String = \"\"; c: Child = Child { }; }\n    on_failure(c: Child, err: ClosureViolation) { self.seen = self.token.reveal_text(); }\n}\nfn main() { let a = Api { }; }\n";
    let (ok, out) = one(failure, "bypass_on_failure");
    assert!(!ok && out.contains("is stored in a field of `self`"), "{out}");
    // an alias names the same credential
    let alias = "type Cred = std::secret::Credential;\nlocus Api {\n    params { token: Cred = std::secret::Credential { vault: \"api\" }; last: String = \"\"; }\n    fn go() { self.last = self.token.reveal_text(); }\n}\nfn main() { let a = Api { }; }\n";
    let (ok, out) = one(alias, "bypass_alias");
    assert!(!ok && out.contains("is stored in a field of `self`"), "{out}");
}

/// `role_password` as `dna/core/memory_schema.hl` declares it.
fn real_role_password() -> String {
    let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dna/core/memory_schema.hl")).unwrap();
    let at = src.find("fn role_password(").expect("dna/core declares role_password");
    let end = at + src[at..].find("\n}\n").expect("its end") + 3;
    format!("fn role_vault_name(role: String) -> String {{ return \"postgres-\" + role; }}\n\n{}", &src[at..end])
}

#[test]
fn the_postgres_sites_are_allowed_by_name_and_body_and_a_fifth_is_refused() {
    let site = real_role_password();
    // the named site, as reviewed: a warning naming the deferral
    let (ok, out) = check(&[("dna/core/memory_schema.hl", &site)], "pq_named");
    assert!(
        ok && out.contains("warning:") && out.contains("allowed here by name, in `dna::role_password`") && out.contains("Deferred: `pq` takes a `std::secret::Credential`"),
        "{out}"
    );
    // the same name with anything added to its body: refused, the pin named
    let changed = site.replace("    return std::secret::Credential", "    println(role);\n    return std::secret::Credential");
    assert_ne!(changed, site, "the edit applies");
    let (ok, out) = check(&[("dna/core/memory_schema.hl", &changed)], "pq_changed");
    assert!(!ok && out.contains("is allowed by name only with the body that was reviewed, and this one has changed"), "{out}");
    // the same body under another name is a fifth site: refused
    let fifth = site.replace("fn role_password(", "fn admin_password(");
    let (ok, out) = check(&[("dna/core/memory_schema.hl", &fifth)], "pq_fifth");
    assert!(!ok && out.contains("may be called only in a locus method") && !out.contains("allowed here by name"), "{out}");
    // an imported seed's own `memory_schema.hl` outside a `dna`
    // directory is no pin at all: a fifth site, refused by the rule
    let copy = "fn role_password(role: String) -> String {\n    return std::secret::Credential { vault: role }.reveal_text();\n}\n";
    let app = "import \"../lib\" as evil;\nfn main() { println(evil::role_password(\"x\")); }\n";
    let (ok, out) = check(&[("app/main.hl", app), ("lib/memory_schema.hl", copy)], "pq_imported_copy");
    assert!(!ok && out.contains("may be called only in a locus method") && !out.contains("allowed"), "{out}");
}

/// A pin names a declaration by its file (the pinned stem, under the
/// pinned library's directory), never by its spelling: a program's own
/// fn or method that happens to share a pinned name is no pin, checks
/// clean, and hears nothing about pins (outside review, finding 1).
#[test]
fn an_unrelated_declaration_that_spells_a_pinned_name_is_not_a_pin() {
    let benign_fn = "fn role_password(role: String) -> String { return role; }\nfn main() { println(role_password(\"x\")); }\n";
    for (path, tag) in [("main.hl", "pin_spell_main"), ("app/memory_schema.hl", "pin_spell_stem")] {
        let (ok, out) = check(&[(path, benign_fn)], tag);
        assert!(ok && !out.contains("allowed") && !out.contains("pin"), "{path}: {out}");
    }
    let benign_method = "locus ReferenceInfrastructure {\n    fn knowledge_database(project: String) -> Int { return 1; }\n}\nfn main() { let r = ReferenceInfrastructure { }; println(r.knowledge_database(\"p\")); }\n";
    let (ok, out) = check(&[("app/infra.hl", benign_method)], "pin_spell_method");
    assert!(ok && !out.contains("allowed") && !out.contains("pin"), "{out}");
    // one pinned name in a `scram.hl` that does not declare its
    // companion: no pin
    let scram = "fn salted_password(@secret password: String, salt: String) -> String { return salt; }\nfn main() { println(salted_password(\"p\", \"s\")); }\n";
    let (ok, out) = check(&[("app/scram.hl", scram)], "pin_spell_scram");
    assert!(ok && !out.contains("allowed"), "{out}");
    // a `scram.hl` that reproduces the module's identity (both pinned
    // names) is taken for the module: its bodies are held to the pins
    let module = "fn salted_password(@secret password: String, salt: String) -> String { return salt; }\nfn compute_client_final(a: String) -> String { return a; }\nfn main() { println(compute_client_final(salted_password(\"p\", \"s\"))); }\n";
    let (ok, out) = check(&[("app/scram.hl", module)], "pin_spell_module");
    assert!(!ok && out.contains("this one has changed"), "{out}");
}

/// The fingerprint strips positions and identity fields outside string
/// literals only: a literal that spells `id: NodeId(123), ` or `Pos(12)`
/// is body text, so changing it changes the pin (outside review,
/// finding 2).
#[test]
fn a_pinned_body_counts_its_string_literals() {
    let site = real_role_password();
    let fp = |out: &str| -> String {
        let at = out.find("(fingerprint ").expect("a fingerprint in the refusal");
        out[at + 13..at + 29].to_string()
    };
    let a = site.replace("vault: role_vault_name(role)", "vault: role_vault_name(role + \"id: NodeId(123), \")");
    let b = site.replace("vault: role_vault_name(role)", "vault: role_vault_name(role + \"Pos(12)\")");
    let c = site.replace("vault: role_vault_name(role)", "vault: role_vault_name(role + \"Pos(13)\")");
    assert!(a != site && b != site && c != site, "the edits apply");
    let (ok_a, out_a) = check(&[("dna/core/memory_schema.hl", &a)], "pq_lit_field");
    let (ok_b, out_b) = check(&[("dna/core/memory_schema.hl", &b)], "pq_lit_pos12");
    let (ok_c, out_c) = check(&[("dna/core/memory_schema.hl", &c)], "pq_lit_pos13");
    assert!(!ok_a && out_a.contains("this one has changed"), "{out_a}");
    assert!(!ok_b && out_b.contains("this one has changed"), "{out_b}");
    assert!(!ok_c && out_c.contains("this one has changed"), "{out_c}");
    assert_ne!(fp(&out_b), fp(&out_c), "the literal's digits count: {out_b}\n{out_c}");
    assert_ne!(fp(&out_a), fp(&out_b), "{out_a}\n{out_b}");
}

/// A pin's companion is declared in the candidate's own module: two
/// unrelated libraries, each a `scram.hl` holding one pinned name, do not
/// vouch for each other when one app imports both; a genuine module still
/// gets its pins however many aliases or copies it arrives under (outside
/// review of #1277, finding 2).
#[test]
fn a_pins_companion_comes_from_its_own_module() {
    let liba = "fn salted_password(@secret password: String, salt: String) -> String { return salt; }\n";
    let libb = "fn compute_client_final(a: String) -> String { return a; }\n";
    let app = "import \"../liba\" as a;\nimport \"../libb\" as b;\nfn main() { println(b::compute_client_final(a::salted_password(\"p\", \"s\"))); }\n";
    let (ok, out) = check(&[("app/main.hl", app), ("liba/scram.hl", liba), ("libb/scram.hl", libb)], "pin_two_libs");
    assert!(ok && !out.contains("allowed") && !out.contains("pq::"), "{out}");
    // each alone is no pin either
    let alone_a = "import \"../liba\" as a;\nfn main() { println(a::salted_password(\"p\", \"s\")); }\n";
    let (ok, out) = check(&[("app/main.hl", alone_a), ("liba/scram.hl", liba)], "pin_lib_a");
    assert!(ok && !out.contains("allowed") && !out.contains("pq::"), "{out}");
    let alone_b = "import \"../libb\" as b;\nfn main() { println(b::compute_client_final(\"x\")); }\n";
    let (ok, out) = check(&[("app/main.hl", alone_b), ("libb/scram.hl", libb)], "pin_lib_b");
    assert!(ok && !out.contains("allowed") && !out.contains("pq::"), "{out}");

    // the genuine module, imported under two aliases and as two copies:
    // its pins hold (the reviewed bodies pass), and a changed body is
    // refused AS a pin, which only a recognized pin says
    let real = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dna/core/pond/pq/scram.hl")).unwrap();
    let use_both = |x: &str, y: &str| {
        format!("{x}\n{y}\nfn main() {{ println(x::gs2_header() + y::gs2_header()); }}\n")
    };
    let two_aliases = use_both("import \"../lib\" as x;", "import \"../lib\" as y;");
    let (ok, out) = check(&[("app/main.hl", &two_aliases), ("lib/scram.hl", &real)], "pin_two_aliases");
    assert!(ok && !out.contains("this one has changed"), "{out}");
    let two_copies = use_both("import \"../lib1\" as x;", "import \"../lib2\" as y;");
    let (ok, out) =
        check(&[("app/main.hl", &two_copies), ("lib1/scram.hl", &real), ("lib2/scram.hl", &real)], "pin_two_copies");
    assert!(ok && !out.contains("this one has changed"), "{out}");
    let changed = real.replace("fn salted_password(@secret password: String, salt: Bytes, iters: Int) -> Bytes {", "fn salted_password(@secret password: String, salt: Bytes, iters: Int) -> Bytes {\n    println(\"edited\");");
    assert_ne!(changed, real, "the edit applies");
    let (ok, out) =
        check(&[("app/main.hl", &two_copies), ("lib1/scram.hl", &real), ("lib2/scram.hl", &changed)], "pin_copy_changed");
    assert!(!ok && out.contains("`pq::salted_password` is allowed by name only with the body that was reviewed"), "{out}");
}

/// A copied, unchanged SCRAM module beside a `main.hl` is the module on
/// every entry point: `check`, `build` and `run` of the directory each
/// read the pin's file from the source map (a directory target's program
/// key is the directory, which names no file), so each allows the pinned
/// bodies with the deferral warning (outside review of #1277, finding 1).
#[test]
fn a_directory_target_reads_a_pin_the_same_on_check_build_and_run() {
    let d: PathBuf = std::env::temp_dir().join(format!("hale_secret_reveal_{}_dir_verbs", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let client = d.join("client");
    std::fs::create_dir_all(&client).unwrap();
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dna/core/pond/pq/scram.hl"), client.join("scram.hl"))
        .unwrap();
    std::fs::write(client.join("main.hl"), "fn main() { println(\"client ran\"); }\n").unwrap();
    for verb in ["check", "build", "run"] {
        let out = Command::new(env!("CARGO_BIN_EXE_hale"))
            .args([verb, &client.to_string_lossy()])
            .current_dir(&d)
            .output()
            .expect("hale");
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(
            out.status.success() && text.contains("It is allowed here by name, in `pq::salted_password`"),
            "hale {verb} client:\n{text}"
        );
        if verb == "run" {
            assert!(text.contains("client ran"), "{text}");
        }
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// `hale check` over an in-tree seed directory; returns (success, output).
fn check_tree(dir: &str) -> (bool, String) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["check", &root.join(dir).to_string_lossy()])
        .current_dir(&root)
        .output()
        .expect("hale");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

/// The four pinned bodies are in the tree, and a pin is compared on every
/// check that reaches its declaration, whether or not the body reveals
/// anything the rule catches (a stale pin is refused at the declaration).
/// So the pins must match the tree they gate, as their own seed and as an
/// imported one: `dna/host` holds `role_password` and the host's
/// `knowledge_database` and imports `dna/core`; `dna/core/pond/pq` holds
/// `salted_password` and `compute_client_final` and is imported by both.
#[test]
fn the_pinned_bodies_in_the_tree_match_their_pins() {
    const CHANGED: &str = "is allowed by name only with the body that was reviewed";
    for dir in ["dna/host", "dna/core", "dna/core/pond/pq"] {
        let (ok, out) = check_tree(dir);
        assert!(ok, "hale check {dir} failed:\n{out}");
        assert!(!out.contains(CHANGED), "a stale secret pin in {dir}:\n{out}");
    }
    // The pins are exercised, not skipped: the two bodies that reveal a
    // secret are allowed by name, which the check says as a warning in
    // the seed that declares them (an imported seed's warnings are the
    // importer's to ignore, so `dna/host` shows neither).
    let (_, core) = check_tree("dna/core");
    assert!(core.contains("It is allowed here by name, in `dna::role_password`"), "{core}");
    let (_, pq) = check_tree("dna/core/pond/pq");
    assert!(pq.contains("It is allowed here by name, in `pq::salted_password`"), "{pq}");
}
