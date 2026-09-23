// The identity of the embedded DNA source set (GH #726).
//
// `hale dna new` / `init` / `upgrade` materialize `vendor/dna` from
// the source EMBEDDED in this binary, so an organism a fixture
// starts runs the core the binary carries, not the one in the
// working tree — and two binaries with the same `hale --version`
// can embed different source. This module is the deterministic
// name of what a binary embeds: a length-framed SHA-256 over the
// sorted `(path, content)` pairs of every embedded file.
//
// Not `hale_iris::toolchain_hash()`, which keys the toolchain cache:
// that is 64-bit FNV-1a over the DNA files AND the iris ones, so it
// changes when neither the core nor the host did, and it is not a
// digest anyone should publish as provenance. This one names exactly
// the DNA source set, cryptographically.
//
// One algorithm, two callers: `build.rs` `include!`s this file and
// digests the on-disk `dna/` tree of the repository being built
// (the digest becomes `hale_dna::EMBEDDED_DIGEST`), and the crate
// compiles it in so `digest_of_tree` can name any other checkout's
// tree and `digest_of_pairs` can name the compiled-in set. The
// unit tests hold the two derivations equal, which is what makes
// the build's snapshot coherent: a source file edited *during* a
// build, or added without updating this crate's file lists, makes
// them disagree and fails the build.

use std::path::Path;

/// The directories the toolchain embeds, with the extensions each
/// contributes, listed NON-recursively (`dna/pond/README.md` is not
/// source). Keep this in step with the `FILES` / `HOST_FILES` / `POND_FILES` …
/// lists in `lib.rs`; the unit tests fail if it drifts.
pub const EMBEDDED_DIRS: &[(&str, &[&str])] = &[
    ("dna/core", &["hl"]),
    ("dna/host", &["hl"]),
    ("dna/membrane", &["hl"]),
    ("dna/operations", &["hl"]),
    ("dna/organization_runtime", &["hl"]),
    ("dna/organization_source", &["hl"]),
    ("dna/pond/db", &["hl"]),
    ("dna/pond/pq", &["hl"]),
    ("dna/ui", &["hl", "html"]),
];

/// The framing tag: a digest says which algorithm produced it, so a
/// later change of shape cannot be mistaken for a source change.
const TAG: &[u8] = b"hale-dna-embedded-v1\n";

/// One digest over `(path, content)` pairs: sorted by path, each
/// field length-framed (no concatenation ambiguity), SHA-256, lower
/// hex. The same pairs in any order give the same digest; one byte
/// of any content, or one path, changes it.
pub fn digest_of_pairs(pairs: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = pairs.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut buf: Vec<u8> = Vec::from(TAG);
    buf.extend_from_slice(&(sorted.len() as u64).to_le_bytes());
    for (path, content) in sorted {
        buf.extend_from_slice(&(path.len() as u64).to_le_bytes());
        buf.extend_from_slice(path.as_bytes());
        buf.extend_from_slice(&(content.len() as u64).to_le_bytes());
        buf.extend_from_slice(content.as_bytes());
    }
    hex(&sha256(&buf))
}

/// The same digest over a directory holding the repository's layout
/// (`<root>/dna/core/…`): what the *working tree* would embed. A
/// missing directory is an error rather than a smaller set, so a
/// wrong root cannot pass as a matching one.
pub fn digest_of_tree(root: &Path) -> Result<String, String> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    for (dir, exts) in EMBEDDED_DIRS {
        let d = root.join(dir);
        let entries = std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        for e in entries {
            let p = e.map_err(|e| format!("{}: {e}", d.display()))?.path();
            if !p.is_file() {
                continue;
            }
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
            if !exts.contains(&ext) {
                continue;
            }
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or_default().to_string();
            let content = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            pairs.push((format!("{dir}/{name}"), content));
        }
    }
    Ok(digest_of_pairs(&pairs))
}

/// The files `digest_of_tree` would read under `root`, for a build
/// script's `rerun-if-changed`.
pub fn tree_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for (dir, exts) in EMBEDDED_DIRS {
        let d = root.join(dir);
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file() && exts.contains(&p.extension().and_then(|s| s.to_str()).unwrap_or("")) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Minimal SHA-256 (FIPS 180-4), std only: this crate carries the
/// DNA source and nothing else, and the build script shares this
/// exact code by `include!`.
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bitlen = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
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
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}
