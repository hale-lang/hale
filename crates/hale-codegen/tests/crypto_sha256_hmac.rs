//! C3 (pond follow-up) — `std::crypto::sha256` and
//! `std::crypto::hmac_sha256`. Mirrors the sha1 test pattern;
//! drives pond/crypto off its 140-line pure-Hale O(N²) workaround.
//!
//! Vectors:
//!   - FIPS 180-2 B.1: sha256("abc")
//!   - Empty input: sha256("")
//!   - FIPS 180-2 B.2: sha256(56-byte input)
//!   - RFC 4231 test 1: hmac_sha256(key=0x0b*20, msg="Hi There")
//!   - hashlib's digests at the 64-byte block and 56-byte padding
//!     boundaries, which hold whichever compression tier runs
//!
//! The .hl program prints each digest as space-separated decimal
//! byte values; this test crate converts to lowercase hex and
//! compares against the canonical strings.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let bin = harness::unique_bin(&format!("hale_test_crypto_sha256_{}", name));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    bin
}

/// Parse one line of the form `tag= b0 b1 b2 ...` (decimal bytes)
/// into a lowercase hex string. Skips lines that don't start with
/// `tag=`.
fn extract_hex(stdout: &str, tag: &str) -> String {
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix(&format!("{}=", tag)) {
            let mut hex = String::new();
            for token in rest.split_whitespace() {
                let v: u32 = token.parse().expect("decimal byte");
                hex.push_str(&format!("{:02x}", v));
            }
            return hex;
        }
    }
    panic!("tag {:?} not in stdout:\n{}", tag, stdout);
}

#[test]
fn sha256_fips_vectors() {
    // Print each digest as "tag= byte byte byte ..." for easy
    // hex reconstruction. 32 bytes per digest.
    let src = r#"
        fn print_digest(name: String, d: Bytes) {
            let mut s = name + "=";
            let mut i = 0;
            while i < 32 {
                let b = std::bytes::at(d, i) or 0;
                s = s + " " + b;
                i = i + 1;
            }
            println(s);
        }

        fn main() {
            // FIPS 180-2 B.1
            let abc = std::bytes::from_string("abc");
            print_digest("abc", std::crypto::sha256(abc));

            // Empty
            let empty = std::bytes::from_string("");
            print_digest("empty", std::crypto::sha256(empty));

            // FIPS 180-2 B.2 — 56-byte input that spans two SHA-256 blocks
            // after padding (forces the multi-block path).
            let b2 = std::bytes::from_string(
                "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            );
            print_digest("b2", std::crypto::sha256(b2));
        }
    "#;
    let bin = build("fips_vectors", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert_eq!(
        extract_hex(&stdout, "abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        "stdout: {}",
        stdout
    );
    assert_eq!(
        extract_hex(&stdout, "empty"),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "stdout: {}",
        stdout
    );
    assert_eq!(
        extract_hex(&stdout, "b2"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        "stdout: {}",
        stdout
    );
}

/// The digests of `data(n)` at the block and padding boundaries, from
/// Python's hashlib over the same bytes. Every entry point runs the one
/// compression tier the machine selects (the x86-64 SHA extensions, the
/// ARMv8 SHA-256 instructions, the portable rounds), so a tier that is
/// wrong on its own agrees with itself everywhere else: only a digest
/// computed elsewhere catches it, on whichever machine runs this.
const BOUNDARY_DIGESTS: &[(&str, &str)] = &[
    ("55", "2900465fcb533e05a158fd2b3be0e5e3b03740d83060aa3580e0d98a96bf2384"),
    ("56", "31454ff48ef36af2f08fd511bdc37d9d5855ac23e992e5ff5445cb6b7674a674"),
    ("63", "5f6401b96532c36de4e65beec0409b69b1d181864c8009b7a04f43e5d56350d1"),
    ("64", "94eb5de4943613fd048dc93393ab06877405faa39c11f53e9386083339833e7e"),
    ("65", "fc518669b6eb4b4dd91827ecacef86689c725bd5bab888fd3b26dbb196eec954"),
    ("119", "b0dc41b1a384e2f1203f0351b38fbeaafceef577ce1191d5bfc25da39f721eae"),
    ("120", "5df24dd802ac26132ce608dcb5f09841eef039ee0f152acf98d26d17fe4e88e6"),
    ("127", "0fe729ff19257bd6fec853acc2ea355f6b34b58e6c0f684c3e188fcdfcd9baae"),
    ("128", "0aedd4856f8eba0963627336ad5144a9a7dbe12498e6066f0165fc97d8ddee4c"),
    ("129", "4f1757ae4bffbae86d775b831765b75af154d52f7deaa46dd378051a2d3ad57f"),
    ("191", "af3693744312c85b48ece39c2b1826d4ce6cd8aa7ff0db2709f1e95b95364077"),
    ("192", "0dc88f7329df669b2e5baf95676c6a50dfb5c1f618c853eba2f9090ee6b03a0d"),
    ("1000", "0bcebe80fd1a990eaf1db275e6d284021de98baafae55bf96de5d67a68f7439b"),
    ("4096", "ddac8baa8df5bd98a019468948621d336514fa4007e399a6c3e4d2f8c20e6801"),
    ("65599", "3c943888cb3f79bad2acb70e38c5bafedfe290e71faf58999ab6b20e0fdf885f"),
    ("1048577", "a2a4c80e07ee2813148328dcdfab2cd7d06f5c6a85f26ee8275bc1529cefb96e"),
];

#[test]
fn sha256_at_the_block_boundaries_is_hashlibs() {
    let sizes: Vec<&str> = BOUNDARY_DIGESTS.iter().map(|(n, _)| *n).collect();
    let src = format!(
        r#"
        fn data(n: Int) -> Bytes {{
            let b = std::bytes::BytesBuilder {{ initial_cap: n + 1 }};
            let mut i = 0;
            while i < n {{
                b.append_u8((i * 37 + 11 + i / 251) % 256);
                i = i + 1;
            }}
            return b.snapshot() or b"";
        }}
        fn main() {{
            let sizes = [{}];
            let mut j = 0;
            while j < len(sizes) {{
                println(to_string(sizes[j]) + " " + std::hex::encode(std::crypto::sha256(data(sizes[j]))));
                j = j + 1;
            }}
        }}
    "#,
        sizes.join(", ")
    );
    let bin = build("boundary_digests", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let got: Vec<(String, String)> = stdout
        .lines()
        .filter_map(|l| l.split_once(' ').map(|(a, b)| (a.to_string(), b.to_string())))
        .collect();
    let want: Vec<(String, String)> = BOUNDARY_DIGESTS.iter().map(|(n, d)| (n.to_string(), d.to_string())).collect();
    assert_eq!(got, want, "stdout: {}", stdout);
}

#[test]
fn hmac_sha256_rfc4231_case_1() {
    // RFC 4231 test 1:
    //   key = 0x0b repeated 20 times
    //   data = "Hi There"
    //   HMAC-SHA-256 = b0344c61d8db38535ca8afceaf0bf12b
    //                  881dc200c9833da726e9376c2e32cff7
    //
    // Build the key by concatenating 20 single-byte Bytes blobs.
    let src = r#"
        fn print_digest(name: String, d: Bytes) {
            let mut s = name + "=";
            let mut i = 0;
            while i < 32 {
                let b = std::bytes::at(d, i) or 0;
                s = s + " " + b;
                i = i + 1;
            }
            println(s);
        }

        fn main() {
            let mut key = std::bytes::from_string("");
            let mut i = 0;
            while i < 20 {
                key = std::bytes::concat(key, std::bytes::from_int(0x0B));
                i = i + 1;
            }
            let msg = std::bytes::from_string("Hi There");
            print_digest("hmac1", std::crypto::hmac_sha256(key, msg));
        }
    "#;
    let bin = build("hmac_rfc1", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert_eq!(
        extract_hex(&stdout, "hmac1"),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
        "stdout: {}",
        stdout
    );
}
