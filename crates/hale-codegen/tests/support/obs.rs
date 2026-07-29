//! R6 (2026-07-29) — shared observation-segment reader for tests.
//!
//! The raw-consumer decoding of `/hale-obs-<pid>` segments (mmap,
//! header offsets, manifest walk, counter lines, ring records) was
//! copy-pasted across `obs_fleet_contract.rs`, `obs_net_seq.rs`, and
//! `obs_emission.rs` — three drifting copies of PROTOCOL v0.1, one
//! of which decoded BUS w1 with the emitter's own (wrong) layout for
//! four releases (handoff-7). One copy now, with the protocol.h
//! decodes vendored HERE and nowhere else.
//!
//! Usage from an integration test (each test file is its own crate):
//!   #[path = "support/obs.rs"]
//!   mod obs;
#![allow(dead_code)]

pub fn read_u64(seg: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(seg[off..off + 8].try_into().unwrap())
}
pub fn read_u32(seg: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(seg[off..off + 4].try_into().unwrap())
}

unsafe fn mmap_raw(
    f: &std::fs::File,
    len: usize,
    prot: i32,
) -> *mut u8 {
    use std::os::unix::io::AsRawFd;
    extern "C" {
        fn mmap(
            addr: *mut core::ffi::c_void,
            len: usize,
            prot: i32,
            flags: i32,
            fd: i32,
            off: i64,
        ) -> *mut core::ffi::c_void;
    }
    let p = mmap(
        core::ptr::null_mut(),
        len,
        prot,
        0x1, /* MAP_SHARED */
        f.as_raw_fd(),
        0,
    );
    if p as isize == -1 {
        core::ptr::null_mut()
    } else {
        p as *mut u8
    }
}

/// Snapshot a live process's obs segment into an owned buffer (must
/// run before the process exits — teardown shm_unlinks).
pub fn snapshot_shm(pid: u32) -> Option<Vec<u8>> {
    let f = std::fs::File::open(format!("/dev/shm/hale-obs-{}", pid))
        .ok()?;
    let len = f.metadata().ok()?.len() as usize;
    let p = unsafe { mmap_raw(&f, len, 0x1) };
    if p.is_null() {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(p as *const u8, len) }
        .to_vec())
}

/// Attach as an observer: bump `observer_count` on the control page
/// so ring emission turns on (requires a writable map).
pub fn attach_observer(pid: u32) {
    if let Ok(f) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(format!("/dev/shm/hale-obs-{}", pid))
    {
        let len = f.metadata().unwrap().len() as usize;
        let p = unsafe { mmap_raw(&f, len, 0x3) };
        if !p.is_null() {
            let seg =
                unsafe { std::slice::from_raw_parts(p as *const u8, len) };
            let control_off = read_u64(seg, 0x38) as usize;
            unsafe {
                std::ptr::write_volatile(
                    p.add(control_off) as *mut u32,
                    1,
                )
            };
        }
    }
}

/// The manifest topic id (join key) + counter line for a subject, or
/// None if the topic never registered in this segment.
pub fn topic_id_and_line(seg: &[u8], subject: &[u8]) -> Option<(u32, usize)> {
    let manifest_off = read_u64(seg, 0x40) as usize;
    let entry_count = read_u32(seg, manifest_off) as usize;
    let pool_off = read_u32(seg, manifest_off + 8) as usize;
    let entries = manifest_off + 16;
    let mut line = 0usize;
    for i in 0..entry_count {
        let e = entries + i * 32;
        let kind = seg[e + 28];
        // topic (0) and binding (2) entries each occupy a counter line.
        if kind == 0 || kind == 2 {
            line += 1;
            if kind == 0 {
                let name_off = read_u32(seg, e + 20) as usize;
                let name_len = seg[e + 24] as usize
                    | ((seg[e + 25] as usize) << 8);
                let base = manifest_off + pool_off + name_off;
                if &seg[base..base + name_len] == subject {
                    let id = read_u32(seg, e + 16);
                    return Some((id, line));
                }
            }
        }
    }
    None
}

/// (published, delivered, bytes) counters for a subject.
pub fn topic_counters(seg: &[u8], subject: &[u8]) -> Option<(u64, u64, u64)> {
    let (_, line) = topic_id_and_line(seg, subject)?;
    let counters_off = read_u64(seg, 0x58) as usize;
    let cline = counters_off + line * 64;
    Some((
        read_u64(seg, cline),
        read_u64(seg, cline + 8),
        read_u64(seg, cline + 16),
    ))
}

/// All records of a given ekind as (w0-id, w1). NET (3/4): w0-id is
/// the topic id. BUS (1/2): decode w1 with [`obs_bus_locus`].
/// LOCUS_BIRTH (5): w0-id = instance id.
pub fn records(seg: &[u8], want_ekind: u32) -> Vec<(u32, u64)> {
    let rings_off = read_u64(seg, 0x68) as usize;
    let ring_count = read_u32(seg, 0x1C) as usize;
    let ring_slots = read_u32(seg, 0x20) as usize;
    let mut out = Vec::new();
    for r in 0..ring_count {
        let rdesc = rings_off + r * 64;
        let data_off = read_u64(seg, rdesc) as usize;
        let head = read_u64(seg, rdesc + 8) as usize;
        let start = head.saturating_sub(ring_slots);
        for i in start..head {
            let slot = data_off + (i & (ring_slots - 1)) * 16;
            let w0 = read_u64(seg, slot);
            let w1 = read_u64(seg, slot + 8);
            let id = (w0 & 0xFFFFF) as u32;
            let ekind = ((w0 >> 20) & 0x1F) as u32;
            if ekind == want_ekind {
                out.push((id, w1));
            }
        }
    }
    out
}

/// Vendored from iris `emitter/protocol.h` (PROTOCOL §8, the
/// executable reference): BUS_PUBLISH / BUS_DELIVER pack
/// `w1 = locus:20 (bits 44..63) | seq:44 (low)`. The handoff-7 bug
/// was the emitter transposing these fields while its tests decoded
/// with the emitter's own layout — vendor THIS decode, never the
/// emitter's. If protocol.h changes, change both in one commit
/// (PROTOCOL.md's own rule).
pub fn obs_bus_locus(w1: u64) -> u32 {
    ((w1 >> 44) & 0xFFFFF) as u32
}

/// NET_SEND / NET_DELIVER w1 = origin:16 (low) | seq:48 (high) —
/// PROTOCOL §8, handoff-3 amendment.
pub fn net_origin_seq(w1: u64) -> (u32, u64) {
    ((w1 & 0xFFFF) as u32, (w1 >> 16) & 0xFFFF_FFFF_FFFF)
}
