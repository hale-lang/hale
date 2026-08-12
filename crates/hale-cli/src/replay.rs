//! GH #296: recording-file parsing and comparison for `hale replay`.
//!
//! Format v0.2 (PRE-STABLE — the runtime's `lotus_obs.c` is the
//! producer; this reader and the test-support reader in
//! `hale-codegen/tests/support/obs.rs` must track it): 64-byte
//! header, tagged entries (tag 0 = 24-byte ring record, tag 1 =
//! payload blob, tag 2 = journal blob; blobs are a 32-byte header +
//! bytes padded to 8), 16-byte clean-finalize trailer.

use std::path::Path;

pub const REC_MAGIC: u64 = 0x30434552454C4148; // "HALEREC0"
pub const REC_END: u64 = 0x30444E45454C4148; // "HALEEND0"

// Private recorder-event namespace (tag-0 entries whose ring
// carries PRIV_RING) — disjoint from iris protocol ekinds.
const REC_EV_CONSUMER: u32 = 1;
const REC_EV_CONSUME: u32 = 2;
const PRIV_RING: u32 = 0x8000_0000;

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

pub struct Recording {
    pub model_hash: u64,
    pub exec_digest: u64,
    pub clean: bool,
    pub ring_records: usize,
    /// (pub_id, topic, flags, bytes); flags bit 0 = ingress,
    /// bit 1 = raw in-process struct (NOT byte-comparable).
    pub payloads: Vec<(u64, u32, u64, Vec<u8>)>,
    /// (consumer, jkind, arghash, bytes) in file order.
    pub journal: Vec<(u64, u32, u32, Vec<u8>)>,
    /// Per consumer id: the ordered pub_id consume stream.
    pub consume_streams: Vec<(u64, Vec<u64>)>,
}

pub fn parse(path: &Path) -> Result<Recording, String> {
    let buf = std::fs::read(path)
        .map_err(|e| format!("could not read `{}`: {}", path.display(), e))?;
    if buf.len() < 80 || u64_at(&buf, 0) != REC_MAGIC {
        return Err(format!(
            "`{}` is not a hale recording (bad magic)",
            path.display()
        ));
    }
    let maj = u16::from_le_bytes(buf[8..10].try_into().unwrap());
    let min = u16::from_le_bytes(buf[10..12].try_into().unwrap());
    if maj != 0 || min < 2 {
        return Err(format!(
            "`{}` is a v{}.{} recording; this hale replays v0.2+ \
             (re-record with this toolchain)",
            path.display(),
            maj,
            min
        ));
    }
    let header_len = u32_at(&buf, 12) as usize;
    if header_len != 64 {
        return Err(format!(
            "`{}`: unexpected header length {}",
            path.display(),
            header_len
        ));
    }
    let model_hash = u64_at(&buf, 48);
    let exec_digest = u64_at(&buf, 56);
    let mut end = buf.len();
    let has_trailer =
        end >= header_len + 16 && u64_at(&buf, end - 16) == REC_END;
    let trailer_count = if has_trailer { u64_at(&buf, end - 8) } else { 0 };
    if has_trailer {
        end -= 16;
    }

    let mut ring_consumer = [0u64; 64];
    let mut streams: Vec<(u64, Vec<u64>)> = Vec::new();
    let mut payloads = Vec::new();
    let mut journal = Vec::new();
    let mut total_entries = 0u64;
    let mut ring_records = 0usize;
    let mut off = header_len;
    while off + 8 <= end {
        let tag = u32_at(&buf, off);
        let a = u32_at(&buf, off + 4);
        match tag {
            0 => {
                if off + 24 > end {
                    break;
                }
                total_entries += 1;
                ring_records += 1;
                let w0 = u64_at(&buf, off + 8);
                let w1 = u64_at(&buf, off + 16);
                let ekind = ((w0 >> 20) & 0x1F) as u32;
                if a & PRIV_RING != 0 {
                    let pr = (a & !PRIV_RING) as usize;
                    if pr < 64 {
                        if ekind == REC_EV_CONSUMER {
                            ring_consumer[pr] = w1;
                        } else if ekind == REC_EV_CONSUME {
                            let cid = ring_consumer[pr];
                            let pub_id = w1 & 0xFFF_FFFF_FFFF;
                            match streams
                                .iter_mut()
                                .find(|(c, _)| *c == cid)
                            {
                                Some((_, v)) => v.push(pub_id),
                                None => {
                                    streams.push((cid, vec![pub_id]))
                                }
                            }
                        }
                    }
                }
                off += 24;
            }
            1 | 2 => {
                if off + 32 > end {
                    break;
                }
                let b = u64_at(&buf, off + 8);
                let c = u64_at(&buf, off + 16);
                let size = u64_at(&buf, off + 24) as usize;
                let padded = (size + 7) & !7;
                if off + 32 + padded > end {
                    break;
                }
                total_entries += 1;
                if tag == 1 {
                    payloads.push((
                        b,
                        a,
                        c,
                        buf[off + 32..off + 32 + size].to_vec(),
                    ));
                } else {
                    journal.push((
                        b,
                        a,
                        (c >> 32) as u32,
                        buf[off + 32..off + 32 + size].to_vec(),
                    ));
                }
                off += 32 + padded;
            }
            other => {
                return Err(format!(
                    "`{}`: unknown entry tag {} at offset {} — \
                     recording from a newer hale?",
                    path.display(),
                    other,
                    off
                ));
            }
        }
    }
    // A clean recording means the WHOLE artifact validated: parse
    // must end exactly at the trailer and the trailer's count must
    // match what was parsed (review finding 5 — trailer magic at
    // EOF alone admits internally truncated files).
    let clean = has_trailer && off == end && trailer_count == total_entries;
    Ok(Recording {
        model_hash,
        exec_digest,
        clean,
        ring_records,
        payloads,
        journal,
        consume_streams: streams,
    })
}

/// Compare two recordings' per-consumer consume streams and payload
/// bytes. Returns None when equivalent, or a human-readable first
/// divergence.
pub fn diff(original: &Recording, replayed: &Recording) -> Option<String> {
    for (cid, orig) in &original.consume_streams {
        let rep = replayed
            .consume_streams
            .iter()
            .find(|(c, _)| c == cid)
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&[]);
        let n = orig.len().min(rep.len());
        for i in 0..n {
            if orig[i] != rep[i] {
                return Some(format!(
                    "consumer {}: consume #{} was delivery {:#x} in \
                     the recording, {:#x} in the replay",
                    cid,
                    i + 1,
                    orig[i],
                    rep[i]
                ));
            }
        }
        if orig.len() != rep.len() {
            return Some(format!(
                "consumer {}: {} consumes recorded, {} replayed",
                cid,
                orig.len(),
                rep.len()
            ));
        }
    }
    for (cid, _) in &replayed.consume_streams {
        if !original.consume_streams.iter().any(|(c, _)| c == cid) {
            return Some(format!(
                "consumer {} appeared in the replay but not in the \
                 recording",
                cid
            ));
        }
    }
    // Payloads, bidirectionally: every recorded publish must exist
    // in the replay and vice versa; topic and flags must agree; a
    // raw-struct payload (flag bit 1) is an ABI snapshot whose
    // bytes are ASLR-dependent, so it compares by size only.
    for (pub_id, topic, flags, bytes) in &original.payloads {
        match replayed.payloads.iter().find(|(p, _, _, _)| p == pub_id)
        {
            None => {
                return Some(format!(
                    "publish {:#x} (topic {}) has no counterpart \
                     in the replay",
                    pub_id, topic
                ));
            }
            Some((_, rt, rf, rb)) => {
                if rt != topic || rf != flags {
                    return Some(format!(
                        "publish {:#x}: topic/flags differ \
                         ({}/{:#x} vs {}/{:#x})",
                        pub_id, topic, flags, rt, rf
                    ));
                }
                let raw = flags & 2 != 0;
                if (raw && rb.len() != bytes.len())
                    || (!raw && rb != bytes)
                {
                    return Some(format!(
                        "publish {:#x}: payload {} differ ({} vs \
                         {} bytes)",
                        pub_id,
                        if raw { "sizes" } else { "bytes" },
                        bytes.len(),
                        rb.len()
                    ));
                }
            }
        }
    }
    for (pub_id, topic, _, _) in &replayed.payloads {
        if !original.payloads.iter().any(|(p, _, _, _)| p == pub_id) {
            return Some(format!(
                "replay produced publish {:#x} (topic {}) that the \
                 recording does not contain",
                pub_id, topic
            ));
        }
    }
    // Journal streams: same reads, same identities, same values,
    // in the same per-consumer order.
    if original.journal.len() != replayed.journal.len() {
        return Some(format!(
            "{} journaled reads recorded, {} in the replay",
            original.journal.len(),
            replayed.journal.len()
        ));
    }
    for (i, (o, r)) in original
        .journal
        .iter()
        .zip(replayed.journal.iter())
        .enumerate()
    {
        if o != r {
            return Some(format!(
                "journal read #{}: (consumer {}, kind {}, args \
                 {:#x}) recorded vs (consumer {}, kind {}, args \
                 {:#x}) replayed",
                i + 1,
                o.0,
                o.1,
                o.2,
                r.0,
                r.1,
                r.2
            ));
        }
    }
    None
}
