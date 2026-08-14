//! GH #296: recording-file parsing and comparison for `hale replay`.
//!
//! Format v0.3 (PRE-STABLE — the runtime's `lotus_obs.c` is the
//! producer; this reader and the test-support reader in
//! `hale-codegen/tests/support/obs.rs` must track it): 96-byte
//! header, tagged entries (tag 0 = 24-byte ring record; tags 1–3 =
//! payload / journal / meta blobs, each a 32-byte header + bytes
//! padded to 8), 16-byte clean-finalize trailer. `clean` means the
//! WHOLE artifact validated: exact parse to the trailer and a
//! matching entry count — trailer magic at EOF alone proves nothing.
//!
//! GH #296 phase 5 (durable recording): a file WITHOUT the trailer is a
//! crash-truncated recording. The drain appends whole frames in
//! stream order, so the prefix is exact up to one torn frame at the
//! tail — parsing stops there (`clean: false`) instead of erroring,
//! and `hale replay --allow-truncated` replays that prefix.

use std::collections::BTreeMap;
use std::path::Path;

pub const REC_MAGIC: u64 = 0x30434552454C4148; // "HALEREC0"
pub const REC_END: u64 = 0x30444E45454C4148; // "HALEEND0"

// Private recorder-event namespace (tag-0 entries whose ring field
// carries PRIV_RING) — disjoint from iris protocol ekinds.
const REC_EV_CONSUMER: u32 = 1;
const REC_EV_CONSUME: u32 = 2;
const PRIV_RING: u32 = 0x8000_0000;

// Public iris ekinds the comparator aligns across runs.
const EK_BUS_PUBLISH: u32 = 1;
const EK_BUS_DELIVER: u32 = 2;

const META_TOPIC: u64 = 1;
const META_PUBRING: u64 = 2;
const META_SUBJHASH: u64 = 3;

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

pub struct RecPayload {
    pub pub_id: u64,
    pub topic: u32,
    /// bit 0 = external ingress; bit 1 = raw in-process struct
    /// (metadata only — no bytes stored, declared size in
    /// `raw_size`).
    pub flags: u32,
    pub raw_size: u64,
    pub bytes: Vec<u8>,
}

pub struct RecJournal {
    pub consumer: u64,
    pub jkind: u32,
    pub withheld: bool,
    pub args: Vec<u8>,
    pub result: Vec<u8>,
}

/// One public bus event, aligned across runs by subject name (the
/// manifest id is registration-order and races) and consumer id
/// (the ring index is claim-order and races).
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct PubEvent {
    pub subject: String,
    pub ekind: u32,
    pub locus: u32,
    pub seq: u64,
}

pub struct Recording {
    pub model_hash: u64,
    pub exec_digest: [u64; 4],
    pub env_redacted: bool,
    pub clean: bool,
    pub ring_records: usize,
    pub payloads: Vec<RecPayload>,
    pub journal: Vec<RecJournal>,
    /// Per consumer id: the ordered (target locus, msg_id) stream.
    pub consume_streams: Vec<(u64, Vec<(u32, u64)>)>,
    /// Per consumer id: the ordered PUBLIC bus stream (publish +
    /// deliver), subject-aligned — this is what makes synchronous
    /// direct dispatch visible to `--diff`.
    pub public_streams: Vec<(u64, Vec<PubEvent>)>,
}

pub fn parse(path: &Path) -> Result<Recording, String> {
    let buf = std::fs::read(path)
        .map_err(|e| format!("could not read `{}`: {}", path.display(), e))?;
    if buf.len() < 112 || u64_at(&buf, 0) != REC_MAGIC {
        return Err(format!(
            "`{}` is not a hale recording (bad magic or too small)",
            path.display()
        ));
    }
    let maj = u16::from_le_bytes(buf[8..10].try_into().unwrap());
    let min = u16::from_le_bytes(buf[10..12].try_into().unwrap());
    if maj != 0 || min < 3 {
        return Err(format!(
            "`{}` is a v{}.{} recording; this hale replays v0.3+ \
             (re-record with this toolchain)",
            path.display(),
            maj,
            min
        ));
    }
    let header_len = u32_at(&buf, 12) as usize;
    if header_len != 96 {
        return Err(format!(
            "`{}`: unexpected header length {}",
            path.display(),
            header_len
        ));
    }
    let model_hash = u64_at(&buf, 48);
    let mut exec_digest = [0u64; 4];
    for (i, part) in exec_digest.iter_mut().enumerate() {
        *part = u64_at(&buf, 56 + i * 8);
    }
    let env_redacted = u64_at(&buf, 88) & 1 != 0;

    let mut end = buf.len();
    let has_trailer =
        end >= header_len + 16 && u64_at(&buf, end - 16) == REC_END;
    let trailer_count = if has_trailer { u64_at(&buf, end - 8) } else { 0 };
    if has_trailer {
        end -= 16;
    }

    let mut priv_ring_consumer = [0u64; 64];
    let mut pub_ring_consumer: BTreeMap<u32, u64> = BTreeMap::new();
    let mut topic_names: BTreeMap<u32, String> = BTreeMap::new();
    let mut consume: Vec<(u64, Vec<(u32, u64)>)> = Vec::new();
    // Public records buffered raw (ring, w0, w1) and resolved into
    // streams at the end, once the meta maps are complete.
    let mut public_raw: Vec<(u32, u64, u64)> = Vec::new();
    let mut payloads = Vec::new();
    let mut journal = Vec::new();
    let mut total_entries = 0u64;
    let mut ring_records = 0usize;
    let mut off = header_len;
    while off + 8 <= end {
        let tag = u32_at(&buf, off);
        let a = u32_at(&buf, off + 4);
        total_entries += 1;
        match tag {
            0 => {
                if end - off < 24 {
                    if !has_trailer {
                        total_entries -= 1;
                        break;
                    }
                    return Err("truncated ring record".into());
                }
                ring_records += 1;
                let w0 = u64_at(&buf, off + 8);
                let w1 = u64_at(&buf, off + 16);
                let ekind = ((w0 >> 20) & 0x1F) as u32;
                if a & PRIV_RING != 0 {
                    let pr = (a & !PRIV_RING) as usize;
                    if pr >= 64 {
                        return Err("private ring out of range".into());
                    }
                    if ekind == REC_EV_CONSUMER {
                        priv_ring_consumer[pr] = w1;
                    } else if ekind == REC_EV_CONSUME {
                        let cid = priv_ring_consumer[pr];
                        let locus = (w0 & 0xFFFFF) as u32;
                        match consume.iter_mut().find(|(c, _)| *c == cid)
                        {
                            Some((_, v)) => v.push((locus, w1)),
                            None => {
                                consume.push((cid, vec![(locus, w1)]))
                            }
                        }
                    }
                } else {
                    public_raw.push((a, w0, w1));
                }
                off += 24;
            }
            1 | 2 | 3 => {
                if end - off < 32 {
                    if !has_trailer {
                        total_entries -= 1;
                        break;
                    }
                    return Err("truncated blob header".into());
                }
                let b = u64_at(&buf, off + 8);
                let c = u64_at(&buf, off + 16);
                let size = u64_at(&buf, off + 24) as usize;
                if size > end - off - 32 {
                    if !has_trailer {
                        total_entries -= 1;
                        break;
                    }
                    return Err("blob length out of range".into());
                }
                let padded = (size + 7) & !7;
                if padded > end - off - 32 {
                    if !has_trailer {
                        total_entries -= 1;
                        break;
                    }
                    return Err("blob padding out of range".into());
                }
                let bytes = &buf[off + 32..off + 32 + size];
                match tag {
                    1 => {
                        let flags = c as u32;
                        let raw = flags & 2 != 0;
                        payloads.push(RecPayload {
                            pub_id: b,
                            topic: a,
                            flags,
                            raw_size: if raw {
                                c >> 32
                            } else {
                                size as u64
                            },
                            bytes: if raw {
                                Vec::new()
                            } else {
                                bytes.to_vec()
                            },
                        });
                    }
                    2 => {
                        if size < 4 {
                            return Err("journal entry too small".into());
                        }
                        let args_len = u32_at(bytes, 0) as usize;
                        if args_len + 4 > size {
                            return Err(
                                "journal argument frame out of range"
                                    .into(),
                            );
                        }
                        journal.push(RecJournal {
                            consumer: b,
                            jkind: a,
                            withheld: c >> 63 != 0,
                            args: bytes[4..4 + args_len].to_vec(),
                            result: bytes[4 + args_len..].to_vec(),
                        });
                    }
                    _ => match b {
                        META_TOPIC => {
                            let name = bytes
                                .split(|x| *x == 0)
                                .next()
                                .unwrap_or(&[]);
                            topic_names.insert(
                                a,
                                String::from_utf8_lossy(name)
                                    .into_owned(),
                            );
                        }
                        META_PUBRING => {
                            pub_ring_consumer.insert(a, c);
                        }
                        META_SUBJHASH => {
                            // phase 5b: subject-hash → name map, a
                            // replay-runtime (injection) concern.
                            // Validated by shape, carried nowhere.
                        }
                        _ => {
                            return Err(format!(
                                "unknown meta subtype {}",
                                b
                            ));
                        }
                    },
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
    let clean = has_trailer && off == end && trailer_count == total_entries;

    // Resolve public bus records into per-consumer, subject-aligned
    // streams. A ring with no consumer mapping keys under 0.
    //
    // Sequence NORMALIZATION (round 3 CI finding): the per-topic
    // seq counter is shared by every publisher of a subject, so the
    // raw values assigned to two RACING publishers depend on their
    // interleaving — which is exactly the cross-consumer ordering
    // the recording never promises. Within one consumer's stream,
    // what is deterministic is the publish's rank, and what pairing
    // needs is only that a publish and its delivers share an
    // identity. So each raw seq maps to its first-appearance
    // ordinal within (consumer, subject): stable across runs,
    // pairing-preserving within the stream, and a genuinely
    // different publish count or order still diverges.
    let mut public: Vec<(u64, Vec<PubEvent>)> = Vec::new();
    let mut ranks: BTreeMap<(u64, u32, u64), u64> = BTreeMap::new();
    let mut next_rank: BTreeMap<(u64, u32), u64> = BTreeMap::new();
    for (ring, w0, w1) in public_raw {
        let ekind = ((w0 >> 20) & 0x1F) as u32;
        if ekind != EK_BUS_PUBLISH && ekind != EK_BUS_DELIVER {
            continue;
        }
        let id = (w0 & 0xFFFFF) as u32;
        let cid = pub_ring_consumer.get(&ring).copied().unwrap_or(0);
        let raw_seq = w1 & 0xFFF_FFFF_FFFF;
        let rank = *ranks.entry((cid, id, raw_seq)).or_insert_with(|| {
            let n = next_rank.entry((cid, id)).or_insert(0);
            let r = *n;
            *n += 1;
            r
        });
        let ev = PubEvent {
            subject: topic_names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| format!("<topic-{}>", id)),
            ekind,
            locus: ((w1 >> 44) & 0xFFFFF) as u32,
            seq: rank,
        };
        match public.iter_mut().find(|(c, _)| *c == cid) {
            Some((_, v)) => v.push(ev),
            None => public.push((cid, vec![ev])),
        }
    }

    Ok(Recording {
        model_hash,
        exec_digest,
        env_redacted,
        clean,
        ring_records,
        payloads,
        journal,
        consume_streams: consume,
        public_streams: public,
    })
}

/// Compare two recordings. Bidirectional over every surface the
/// artifact carries: per-consumer queued consume streams (target
/// locus + msg_id), per-consumer PUBLIC bus streams (subject-aligned
/// — synchronous direct dispatch shows up here), payloads both ways
/// (topic, flags, bytes; raw ABI snapshots by declared size), and
/// per-consumer journal streams (kind, args, withheld state, value).
/// Returns None when equivalent, or the first divergence.
///
/// `prefix_only` (a truncated baseline, GH #296 phase 5): the
/// recording is a crash-cut prefix, so the replay legitimately runs
/// PAST it — surplus replay events are not divergences, but anything
/// inside the recorded extent still must match exactly, and a replay
/// that produces LESS than the recording did still diverges.
pub fn diff(
    original: &Recording,
    replayed: &Recording,
    prefix_only: bool,
) -> Option<String> {
    fn stream_diff<T: PartialEq + std::fmt::Debug>(
        what: &str,
        a: &[(u64, Vec<T>)],
        b: &[(u64, Vec<T>)],
        prefix_only: bool,
    ) -> Option<String> {
        for (cid, orig) in a {
            let rep = b
                .iter()
                .find(|(c, _)| c == cid)
                .map(|(_, v)| v.as_slice())
                .unwrap_or(&[]);
            let n = orig.len().min(rep.len());
            for i in 0..n {
                if orig[i] != rep[i] {
                    return Some(format!(
                        "consumer {}: {} #{} was {:?} in the \
                         recording, {:?} in the replay",
                        cid,
                        what,
                        i + 1,
                        orig[i],
                        rep[i]
                    ));
                }
            }
            if orig.len() != rep.len()
                && !(prefix_only && rep.len() > orig.len())
            {
                return Some(format!(
                    "consumer {}: {} {}s recorded, {} replayed",
                    cid,
                    orig.len(),
                    what,
                    rep.len()
                ));
            }
        }
        if !prefix_only {
            for (cid, v) in b {
                if !v.is_empty() && !a.iter().any(|(c, _)| c == cid) {
                    return Some(format!(
                        "consumer {} produced {}s in the replay but \
                         not in the recording",
                        cid, what
                    ));
                }
            }
        }
        None
    }

    if let Some(d) = stream_diff(
        "queued consume",
        &original.consume_streams,
        &replayed.consume_streams,
        prefix_only,
    ) {
        return Some(d);
    }
    if let Some(d) = stream_diff(
        "public bus event",
        &original.public_streams,
        &replayed.public_streams,
        prefix_only,
    ) {
        return Some(d);
    }

    // Payloads, bidirectionally.
    for p in &original.payloads {
        match replayed.payloads.iter().find(|r| r.pub_id == p.pub_id) {
            None => {
                return Some(format!(
                    "publish {:#x} (topic {}) has no counterpart in \
                     the replay",
                    p.pub_id, p.topic
                ));
            }
            Some(r) => {
                if r.topic != p.topic || r.flags != p.flags {
                    return Some(format!(
                        "publish {:#x}: topic/flags differ \
                         ({}/{:#x} vs {}/{:#x})",
                        p.pub_id, p.topic, p.flags, r.topic, r.flags
                    ));
                }
                let raw = p.flags & 2 != 0;
                if (raw && r.raw_size != p.raw_size)
                    || (!raw && r.bytes != p.bytes)
                {
                    return Some(format!(
                        "publish {:#x}: payload {} differ ({} vs {})",
                        p.pub_id,
                        if raw { "declared sizes" } else { "bytes" },
                        p.raw_size,
                        r.raw_size
                    ));
                }
            }
        }
    }
    if !prefix_only {
        for r in &replayed.payloads {
            if !original.payloads.iter().any(|p| p.pub_id == r.pub_id) {
                return Some(format!(
                    "replay produced publish {:#x} (topic {}) that \
                     the recording does not contain",
                    r.pub_id, r.topic
                ));
            }
        }
    }

    // Journal, grouped PER CONSUMER (the drain interleaves
    // concurrent threads' blobs in incidental order — comparing
    // one global vector would report divergence for runs whose
    // every consumer saw exactly its recorded inputs; review
    // round 2, finding 6).
    fn by_consumer(
        j: &[RecJournal],
    ) -> Vec<(u64, Vec<(u32, bool, &[u8], &[u8])>)> {
        let mut out: Vec<(u64, Vec<(u32, bool, &[u8], &[u8])>)> =
            Vec::new();
        for e in j {
            let row = (
                e.jkind,
                e.withheld,
                e.args.as_slice(),
                e.result.as_slice(),
            );
            match out.iter_mut().find(|(c, _)| *c == e.consumer) {
                Some((_, v)) => v.push(row),
                None => out.push((e.consumer, vec![row])),
            }
        }
        out
    }
    let a = by_consumer(&original.journal);
    let b = by_consumer(&replayed.journal);
    if let Some(d) = stream_diff("journal read", &a, &b, prefix_only) {
        return Some(d);
    }
    None
}
