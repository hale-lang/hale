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

const EK_BUS_CONSUME: u32 = 8;
const EK_CONSUMER: u32 = 12;

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

pub struct Recording {
    pub model_hash: u64,
    pub clean: bool,
    pub ring_records: usize,
    pub payloads: Vec<(u64, u64, Vec<u8>)>, // (pub_id, flags, bytes)
    pub journal_entries: usize,
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
    let model_hash = u64_at(&buf, 48);
    let mut end = buf.len();
    let clean = end >= header_len + 16 && u64_at(&buf, end - 16) == REC_END;
    if clean {
        end -= 16;
    }

    let mut ring_consumer = [0u64; 64];
    let mut streams: Vec<(u64, Vec<u64>)> = Vec::new();
    let mut payloads = Vec::new();
    let mut journal_entries = 0usize;
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
                ring_records += 1;
                let w0 = u64_at(&buf, off + 8);
                let w1 = u64_at(&buf, off + 16);
                let ekind = ((w0 >> 20) & 0x1F) as u32;
                let ring = a as usize;
                if ekind == EK_CONSUMER && ring < 64 {
                    ring_consumer[ring] = w1;
                } else if ekind == EK_BUS_CONSUME && ring < 64 {
                    let cid = ring_consumer[ring];
                    let pub_id = w1 & 0xFFF_FFFF_FFFF;
                    match streams.iter_mut().find(|(c, _)| *c == cid) {
                        Some((_, v)) => v.push(pub_id),
                        None => streams.push((cid, vec![pub_id])),
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
                if tag == 1 {
                    payloads.push((
                        b,
                        c,
                        buf[off + 32..off + 32 + size].to_vec(),
                    ));
                } else {
                    journal_entries += 1;
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
    Ok(Recording {
        model_hash,
        clean,
        ring_records,
        payloads,
        journal_entries,
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
    // Payload comparison: same pub_id → same bytes.
    for (pub_id, _, bytes) in &original.payloads {
        if let Some((_, _, rb)) =
            replayed.payloads.iter().find(|(p, _, _)| p == pub_id)
        {
            if rb != bytes {
                return Some(format!(
                    "delivery {:#x}: payload bytes differ ({} vs {} \
                     bytes)",
                    pub_id,
                    bytes.len(),
                    rb.len()
                ));
            }
        }
    }
    None
}
