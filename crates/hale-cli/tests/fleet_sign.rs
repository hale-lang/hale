//! GH #408 Phase 7: signed fleet components and binary attestation.
//!
//! A signature makes the certificate portable — it proves the
//! artifacts a composition read are the ones a key-holder meant,
//! never that the code behaves. The covered thing is the artifact's
//! EXACT BYTES (sound because artifacts are byte-reproducible),
//! never the in-band FNV digest, which is a tripwire rather than a
//! trust anchor.
//!
//! Trust is strict when declared: passing `--trust` (or listing
//! `[fleet_trust]` keys) makes an unsigned or unverifiable
//! component a refusal. Declaring a trust set and then quietly
//! admitting unsigned artifacts would be law that looks bound and
//! binds nothing — so these tests care as much about what is
//! REFUSED as what verifies.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_fleet_sign_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn write(root: &Path, rel: &str, src: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(&p, src).expect("write");
}

fn hale(args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .output()
        .expect("run hale");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

/// `hale` with a working directory — the `[fleet_trust]` form reads
/// the manifest at and above the cwd.
fn hale_in(dir: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run hale");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

const APP: &str = r#"
type Ping { n: Int; }
topic Pings { payload: Ping; subject: "svc.ping"; }
locus Send {
    bus { publish Pings; }
    birth() { Pings <- Ping { n: 1 }; }
}
locus Recv {
    bus { subscribe Pings as on_ping; }
    fn on_ping(p: Ping) { println(p.n); }
}
main locus M {
    params { s: Send = Send { }; r: Recv = Recv { }; }
}
fn main() { M { }; }
"#;

const PLAN: &str = r#"{
  "schema": "1.1",
  "name": "signed",
  "instances": [
    {"id": "app-0", "artifact": "artifacts/app.json"}
  ]
}"#;

/// One seed, one artifact, one plan, one keypair.
fn fleet(tag: &str) -> PathBuf {
    let r = root(tag);
    write(&r, "app/main.hl", APP);
    write(&r, "hale.toml", "[deps]\n");
    let dst = r.join("artifacts/app.json");
    std::fs::create_dir_all(dst.parent().expect("parent")).expect("mkdir");
    let (out, code) = hale(&[
        "check",
        r.join("app").to_str().expect("utf8"),
        &format!("--dump-topology={}", dst.display()),
    ]);
    assert_eq!(code, 0, "component must check clean: {}", out);
    write(&r, "signed.plan.json", PLAN);
    let (out, code) = hale(&[
        "fleet",
        "keygen",
        r.join("ops").to_str().expect("utf8"),
    ]);
    assert_eq!(code, 0, "keygen: {}", out);
    r
}

fn p(r: &Path, rel: &str) -> String {
    r.join(rel).to_str().expect("utf8").to_string()
}

/// keygen → sign → check --trust verifies, and the fleet artifact
/// records which key admitted each component — `signed_by` is the
/// key's identity, not the key file's path.
#[test]
fn a_signed_component_verifies_and_the_artifact_records_the_key() {
    let r = fleet("verify");
    let (out, code) = hale(&[
        "fleet",
        "sign",
        &p(&r, "artifacts/app.json"),
        "--key",
        &p(&r, "ops.pem"),
    ]);
    assert_eq!(code, 0, "sign: {}", out);
    let key_id = out
        .split("key_id ")
        .nth(1)
        .expect("sign prints key_id")
        .trim()
        .to_string();

    let (out, code) = hale(&[
        "fleet",
        "dump",
        &p(&r, "signed.plan.json"),
        "--trust",
        &p(&r, "ops.pub.pem"),
    ]);
    assert_eq!(code, 0, "check with trust: {}", out);
    let v: serde_json::Value =
        serde_json::from_str(&out).expect("fleet artifact parses");
    assert_eq!(
        v["components"][0]["signed_by"].as_str(),
        Some(key_id.as_str()),
        "artifact records the admitting key: {}",
        out
    );
    assert_eq!(
        v["components"][0]["sha256"].as_str().map(str::len),
        Some(64),
        "artifact records the admitted bytes' sha256"
    );
    let _ = std::fs::remove_dir_all(&r);
}

/// The signature covers exact bytes: appending one byte of
/// whitespace — invisible to JSON, invisible to the FNV digest's
/// prefix — is a refusal that names the sidecar.
#[test]
fn a_tampered_artifact_is_refused_even_when_json_still_parses() {
    let r = fleet("tamper");
    let art = r.join("artifacts/app.json");
    let (_, code) = hale(&[
        "fleet",
        "sign",
        art.to_str().expect("utf8"),
        "--key",
        &p(&r, "ops.pem"),
    ]);
    assert_eq!(code, 0);

    let mut bytes = std::fs::read(&art).expect("read artifact");
    bytes.push(b' ');
    std::fs::write(&art, bytes).expect("append");

    let (out, code) = hale(&[
        "fleet",
        "check",
        &p(&r, "signed.plan.json"),
        "--trust",
        &p(&r, "ops.pub.pem"),
    ]);
    assert_eq!(code, 1, "tampered component must refuse: {}", out);
    assert!(
        out.contains("does not verify"),
        "refusal names the signature failure: {}",
        out
    );
    let _ = std::fs::remove_dir_all(&r);
}

/// Trust declared + no sidecar = refusal. Absence is not admission.
#[test]
fn an_unsigned_component_is_refused_when_trust_is_declared() {
    let r = fleet("unsigned");
    let (out, code) = hale(&[
        "fleet",
        "check",
        &p(&r, "signed.plan.json"),
        "--trust",
        &p(&r, "ops.pub.pem"),
    ]);
    assert_eq!(code, 1, "unsigned must refuse under trust: {}", out);
    assert!(
        out.contains("no signature at"),
        "refusal says the sidecar is missing, not that bytes differ: {}",
        out
    );
    // And WITHOUT trust the same plan composes — the pre-Phase-7
    // meaning of a composition is unchanged.
    let (out, code) = hale(&["fleet", "check", &p(&r, "signed.plan.json")]);
    assert_eq!(code, 0, "no trust declared, no signature required: {}", out);
    let _ = std::fs::remove_dir_all(&r);
}

/// A valid signature under a key OUTSIDE the trust set is exactly as
/// inadmissible as a broken one.
#[test]
fn a_signature_from_an_untrusted_key_is_refused() {
    let r = fleet("untrusted");
    let (_, code) =
        hale(&["fleet", "keygen", r.join("rogue").to_str().expect("utf8")]);
    assert_eq!(code, 0);
    let (_, code) = hale(&[
        "fleet",
        "sign",
        &p(&r, "artifacts/app.json"),
        "--key",
        &p(&r, "rogue.pem"),
    ]);
    assert_eq!(code, 0);

    let (out, code) = hale(&[
        "fleet",
        "check",
        &p(&r, "signed.plan.json"),
        "--trust",
        &p(&r, "ops.pub.pem"),
    ]);
    assert_eq!(code, 1, "untrusted signer must refuse: {}", out);
    assert!(out.contains("not in the trust set"), "{}", out);
    let _ = std::fs::remove_dir_all(&r);
}

/// `[fleet_trust]` in the manifest binds the all-fleets form the
/// same way `--trust` binds a named plan.
#[test]
fn fleet_trust_in_the_manifest_binds_every_declared_fleet() {
    let r = fleet("manifest");
    write(
        &r,
        "hale.toml",
        "[deps]\n\n[fleets]\nsigned = \"signed.plan.json\"\n\n\
         [fleet_trust]\nkeys = [\"ops.pub.pem\"]\n",
    );
    // Unsigned: the manifest's trust refuses it.
    let (out, code) = hale_in(&r, &["fleet", "check"]);
    assert_eq!(code, 1, "manifest trust must refuse unsigned: {}", out);
    assert!(out.contains("no signature at"), "{}", out);
    // Signed: the same form passes.
    let (_, code) = hale(&[
        "fleet",
        "sign",
        &p(&r, "artifacts/app.json"),
        "--key",
        &p(&r, "ops.pem"),
    ]);
    assert_eq!(code, 0);
    let (out, code) = hale_in(&r, &["fleet", "check"]);
    assert_eq!(code, 0, "signed fleet passes under manifest trust: {}", out);
    let _ = std::fs::remove_dir_all(&r);
}

/// attest: every instance carries `binary` + `binary_sha256`, and
/// the bytes on disk match — or the plan is not attested. A missing
/// row is a refusal, not a skip: partial coverage must not wear a
/// full answer's exit code.
#[test]
fn attest_matches_binaries_and_refuses_partial_coverage() {
    let r = fleet("attest");
    // A stand-in "binary": attest hashes bytes; it does not run them.
    write(&r, "bin/app", "not really an ELF but definitely bytes\n");
    let bytes = std::fs::read(r.join("bin/app")).expect("read");
    let digest = {
        use std::fmt::Write as _;
        // sha256 via the hale CLI would be circular; openssl's CLI may
        // be absent. Small and standard beats clever here.
        let d = sha256_ref(&bytes);
        let mut s = String::new();
        for b in d {
            let _ = write!(s, "{:02x}", b);
        }
        s
    };
    write(
        &r,
        "attest.plan.json",
        &format!(
            r#"{{"schema": "1.1", "name": "signed",
  "instances": [{{"id": "app-0", "artifact": "artifacts/app.json",
                  "binary": "bin/app", "binary_sha256": "{}"}}]}}"#,
            digest
        ),
    );
    let (out, code) = hale(&["fleet", "attest", &p(&r, "attest.plan.json")]);
    assert_eq!(code, 0, "matching binary attests: {}", out);

    // Flip the bytes: same path, different binary.
    write(&r, "bin/app", "different bytes\n");
    let (out, code) = hale(&["fleet", "attest", &p(&r, "attest.plan.json")]);
    assert_eq!(code, 1, "changed binary must refuse: {}", out);
    assert!(out.contains("is not the binary the plan names"), "{}", out);

    // An instance with no digest rows poisons the whole attestation.
    let (out, code) = hale(&["fleet", "attest", &p(&r, "signed.plan.json")]);
    assert_eq!(code, 1, "undeclared binary must refuse: {}", out);
    assert!(out.contains("attestable"), "{}", out);
    let _ = std::fs::remove_dir_all(&r);
}

/// A 1.0 plan still reads — Phase 7's fields are additive, and a
/// plan that predates them keeps its meaning.
#[test]
fn a_schema_1_0_plan_still_composes() {
    let r = fleet("compat");
    write(
        &r,
        "old.plan.json",
        &PLAN.replace("\"schema\": \"1.1\"", "\"schema\": \"1.0\""),
    );
    let (out, code) = hale(&["fleet", "check", &p(&r, "old.plan.json")]);
    assert_eq!(code, 0, "1.0 plan composes: {}", out);
    let _ = std::fs::remove_dir_all(&r);
}

/// Minimal SHA-256, test-local, so the expected digest does not
/// depend on the code under test. (FIPS 180-4; block-at-a-time.)
fn sha256_ref(msg: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b,
        0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
        0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7,
        0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
        0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152,
        0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
        0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
        0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
        0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f,
        0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut data = msg.to_vec();
    let bitlen = (msg.len() as u64) * 8;
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bitlen.to_be_bytes());
    for block in data.chunks(64) {
        let mut w = [0u32; 64];
        for (i, c) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7)
                ^ w[i - 15].rotate_right(18)
                ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17)
                ^ w[i - 2].rotate_right(19)
                ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) = (
            h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7],
        );
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}
