//! GH #408 Phase 7 — signed fleet components.
//!
//! A signature makes the certificate portable: `hale fleet check`
//! proves a composed world against artifacts it can read, and the
//! signature proves those artifacts are the ones some key-holder
//! meant. It certifies PROVENANCE AND INTEGRITY, never behavior —
//! the artifact never claims a message will arrive, and the
//! signature never claims the code is good.
//!
//! The scheme is ES256 (ECDSA P-256 over SHA-256), because the
//! system already speaks it: `std::crypto::ecdsa_p256_sign/verify`
//! (spec/stdlib.md), OpenSSL-backed in the runtime, PEM keys, raw
//! `r‖s` 64-byte signatures. One signature algorithm end to end
//! means a Hale program — a supervisor, a deploy gate — can verify
//! the same sidecar with the language's own stdlib.
//!
//! Signatures cover the artifact's EXACT BYTES. That is sound
//! because artifacts are byte-reproducible (schema 1.8: workspace-
//! relative paths, canonicalized sources), and it is necessary
//! because the in-band `artifact_digest` is FNV-1a — an integrity
//! tripwire, not a trust anchor. Nothing here signs a digest.
//!
//! Sidecar format: `<artifact>.sig`, one line, `es256:<128 hex>`.
//! The prefix is the format's version; an unknown prefix is a
//! refusal, not a skip.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use openssl::bn::BigNum;
use openssl::ec::{EcGroup, EcKey};
use openssl::ecdsa::EcdsaSig;
use openssl::nid::Nid;
use openssl::pkey::{PKey, Public};
use openssl::sha::sha256;

const SIG_PREFIX: &str = "es256:";

/// The trust roots one composition verifies against. Loaded once,
/// up front, so a bad key file is its own error at the boundary
/// rather than surfacing as a spurious per-component refusal.
pub struct Trust {
    /// `(key_id, key)` — key_id is the first 8 bytes of
    /// SHA-256(SPKI DER), hex. Identity of the KEY, so a rotated
    /// key changes the id even when the file path stays put.
    pub keys: Vec<(String, EcKey<Public>)>,
}

impl Trust {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Load trusted public keys (SPKI PEM). Every path must load —
    /// a trust root that fails to parse is a configuration error,
    /// and skipping it would silently narrow the trusted set.
    pub fn load(paths: &[PathBuf]) -> Result<Trust, String> {
        let mut keys = Vec::new();
        for p in paths {
            let pem = std::fs::read(p)
                .map_err(|e| format!("trust key {}: {}", p.display(), e))?;
            let pkey = PKey::public_key_from_pem(&pem).map_err(|e| {
                format!("trust key {}: not a public key PEM: {}", p.display(), e)
            })?;
            let ec = pkey.ec_key().map_err(|_| {
                format!(
                    "trust key {}: not an EC key — fleet signatures are \
                     ES256 (P-256), matching std::crypto",
                    p.display()
                )
            })?;
            keys.push((key_id(&pkey)?, ec));
        }
        Ok(Trust { keys })
    }

    /// Verify `bytes` against the sidecar signature at `sig_path`.
    /// Returns the key_id that verified. Every failure names what
    /// was checked — absence, format, and mismatch are three
    /// different repairs.
    pub fn verify(
        &self,
        bytes: &[u8],
        sig_path: &Path,
    ) -> Result<String, String> {
        let raw = std::fs::read_to_string(sig_path).map_err(|_| {
            format!(
                "no signature at {} — trust roots are declared, so an \
                 unsigned component is not admissible",
                sig_path.display()
            )
        })?;
        let hex = raw
            .trim()
            .strip_prefix(SIG_PREFIX)
            .ok_or_else(|| {
                format!(
                    "{}: unrecognized signature format (expected \
                     `{}<hex>`)",
                    sig_path.display(),
                    SIG_PREFIX
                )
            })?;
        let sig = decode_hex(hex).ok_or_else(|| {
            format!("{}: signature is not valid hex", sig_path.display())
        })?;
        if sig.len() != 64 {
            return Err(format!(
                "{}: signature is {} bytes, ES256 raw r‖s is 64",
                sig_path.display(),
                sig.len()
            ));
        }
        let digest = sha256(bytes);
        let r = BigNum::from_slice(&sig[..32]).map_err(es)?;
        let s = BigNum::from_slice(&sig[32..]).map_err(es)?;
        let esig = EcdsaSig::from_private_components(r, s).map_err(es)?;
        for (id, key) in &self.keys {
            if esig.verify(&digest, key).unwrap_or(false) {
                return Ok(id.clone());
            }
        }
        Err(format!(
            "{}: signature does not verify under any of the {} trusted \
             key(s) — either the artifact changed after signing or the \
             signer is not in the trust set",
            sig_path.display(),
            self.keys.len()
        ))
    }
}

/// `hale fleet keygen <prefix>` — write `<prefix>.pem` (PKCS#8
/// private) and `<prefix>.pub.pem` (SPKI public). Returns the
/// key_id for the operator to record.
pub fn keygen(prefix: &Path) -> Result<String, String> {
    let group =
        EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).map_err(es)?;
    let ec = EcKey::generate(&group).map_err(es)?;
    let pkey = PKey::from_ec_key(ec).map_err(es)?;

    let priv_path = with_ext(prefix, "pem");
    let pub_path = with_ext(prefix, "pub.pem");
    write_private(&priv_path, &pkey.private_key_to_pem_pkcs8().map_err(es)?)?;
    std::fs::write(&pub_path, pkey.public_key_to_pem().map_err(es)?)
        .map_err(|e| format!("write {}: {}", pub_path.display(), e))?;
    key_id(&pkey)
}

/// `hale fleet sign <file> --key <priv.pem>` — detached sidecar
/// over the file's exact bytes. Works on any file: a component
/// artifact and a fleet artifact are both just bytes here.
pub fn sign(file: &Path, key_path: &Path) -> Result<(PathBuf, String), String> {
    let bytes = std::fs::read(file)
        .map_err(|e| format!("read {}: {}", file.display(), e))?;
    let pem = std::fs::read(key_path)
        .map_err(|e| format!("read key {}: {}", key_path.display(), e))?;
    // PKCS#8 first (what keygen writes), SEC1 as the fallback —
    // both spellings are in spec/stdlib.md for std::crypto.
    let pkey = PKey::private_key_from_pem(&pem).map_err(|e| {
        format!("{}: not a private key PEM: {}", key_path.display(), e)
    })?;
    let ec = pkey.ec_key().map_err(|_| {
        format!(
            "{}: not an EC key — fleet signatures are ES256 (P-256)",
            key_path.display()
        )
    })?;

    let digest = sha256(&bytes);
    let esig = EcdsaSig::sign(&digest, &ec).map_err(es)?;
    let mut raw = [0u8; 64];
    let (r, s) = (esig.r().to_vec(), esig.s().to_vec());
    raw[32 - r.len()..32].copy_from_slice(&r);
    raw[64 - s.len()..].copy_from_slice(&s);

    let sig_path = sidecar(file);
    let mut line = String::with_capacity(6 + 128 + 1);
    line.push_str(SIG_PREFIX);
    for b in raw {
        let _ = write!(line, "{:02x}", b);
    }
    line.push('\n');
    std::fs::write(&sig_path, line)
        .map_err(|e| format!("write {}: {}", sig_path.display(), e))?;
    Ok((sig_path, key_id(&pkey)?))
}

/// SHA-256 of a file's bytes, hex — `hale fleet attest` compares
/// this against the plan's `binary_sha256`.
pub fn sha256_file(p: &Path) -> Result<String, String> {
    let bytes = std::fs::read(p)
        .map_err(|e| format!("read {}: {}", p.display(), e))?;
    Ok(hex(&sha256(&bytes)))
}

pub fn sidecar(artifact: &Path) -> PathBuf {
    let mut s = artifact.as_os_str().to_os_string();
    s.push(".sig");
    PathBuf::from(s)
}

/// First 8 bytes of SHA-256 over the SPKI DER, hex. Key identity,
/// not file identity: rotating the key changes the id even if the
/// path stays put.
fn key_id<T: openssl::pkey::HasPublic>(
    pkey: &PKey<T>,
) -> Result<String, String> {
    let der = pkey.public_key_to_der().map_err(es)?;
    Ok(hex(&sha256(&der)[..8]))
}

fn with_ext(prefix: &Path, ext: &str) -> PathBuf {
    let mut s = prefix.as_os_str().to_os_string();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

#[cfg(unix)]
fn write_private(path: &Path, pem: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("write {}: {}", path.display(), e))?;
    f.write_all(pem)
        .map_err(|e| format!("write {}: {}", path.display(), e))
}

#[cfg(not(unix))]
fn write_private(path: &Path, pem: &[u8]) -> Result<(), String> {
    std::fs::write(path, pem)
        .map_err(|e| format!("write {}: {}", path.display(), e))
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{:02x}", b);
    }
    s
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn es(e: openssl::error::ErrorStack) -> String {
    format!("openssl: {}", e)
}
