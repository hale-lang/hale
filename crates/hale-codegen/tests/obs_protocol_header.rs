//! GH #527 B1: `runtime/obs_protocol.h` is the observation protocol's
//! executable form, shared by the emitter (prepended to lotus_obs.c at
//! build) and, after B2, by iris's consumers. The Rust test decoder in
//! `support/obs.rs` mirrors a handful of its constants; this test reads
//! the header text and refuses a mismatch, replacing the old "if
//! protocol.h changes, change both in one commit" rule with a build
//! failure.

#[path = "support/obs.rs"]
#[allow(dead_code)]
mod obs;

const HEADER: &str = include_str!("../runtime/obs_protocol.h");

fn define(name: &str) -> Option<String> {
    HEADER
        .lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix(&format!("#define {name} ")).map(|v| v.trim().to_string()))
}

fn enum_value(name: &str) -> Option<u32> {
    HEADER.lines().map(str::trim).find_map(|l| {
        let rest = l.strip_prefix(name)?.trim_start();
        let rest = rest.strip_prefix('=')?.trim_start();
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().ok()
    })
}

/// The `/* 0x.. */` offset comment on a header field.
fn field_offset(field: &str) -> Option<usize> {
    HEADER.lines().find_map(|l| {
        let l = l.trim();
        if !l.contains(&format!(" {field};")) {
            return None;
        }
        let c = l.split("/*").nth(1)?.trim();
        let hex = c.strip_prefix("0x")?;
        let hex: String = hex.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        usize::from_str_radix(&hex, 16).ok()
    })
}

#[test]
fn version_and_magic_agree() {
    assert_eq!(define("OBS_PROTO_MAJOR").as_deref(), Some(obs::PROTO_MAJOR.to_string().as_str()));
    assert_eq!(define("OBS_PROTO_MINOR").as_deref(), Some(obs::PROTO_MINOR.to_string().as_str()));
    let magic = define("OBS_MAGIC").expect("OBS_MAGIC");
    assert_eq!(magic, format!("0x{:016X}ULL", obs::MAGIC), "magic literal");
}

#[test]
fn ekinds_agree() {
    for (name, v) in [
        ("OBS_EK_EPOCH", obs::EK_EPOCH),
        ("OBS_EK_BUS_PUBLISH", obs::EK_BUS_PUBLISH),
        ("OBS_EK_BUS_DELIVER", obs::EK_BUS_DELIVER),
        ("OBS_EK_NET_SEND", obs::EK_NET_SEND),
        ("OBS_EK_NET_DELIVER", obs::EK_NET_DELIVER),
        ("OBS_EK_LOCUS_BIRTH", obs::EK_LOCUS_BIRTH),
        ("OBS_EK_LOCUS_DISSOLVE", obs::EK_LOCUS_DISSOLVE),
        ("OBS_EK_RESTART", obs::EK_RESTART),
        ("OBS_EK_DROP_MARK", obs::EK_DROP_MARK),
    ] {
        assert_eq!(enum_value(name), Some(v), "{name}");
    }
}

#[test]
fn header_offsets_agree() {
    for (field, off) in [
        ("proto_minor", obs::OFF_PROTO_MINOR),
        ("ring_count", obs::OFF_RING_COUNT),
        ("ring_slots", obs::OFF_RING_SLOTS),
        ("manifest_off", obs::OFF_MANIFEST_OFF),
        ("counters_off", obs::OFF_COUNTERS_OFF),
        ("rings_off", obs::OFF_RINGS_OFF),
        ("model_hash", obs::OFF_MODEL_HASH),
        ("entity_id_digest", obs::OFF_ENTITY_ID_DIGEST),
    ] {
        assert_eq!(field_offset(field), Some(off), "{field}");
    }
}

/// The word packings the decoder reproduces by hand (`obs_bus_locus`,
/// `net_origin_seq`, `records`) are stated in the header as inline
/// fns; pin the shift constants they use.
#[test]
fn packing_shifts_are_the_headers() {
    assert!(HEADER.contains("((uint64_t)(ekind & 0x1Fu) << 20)"), "ekind at bits 20..25");
    assert!(HEADER.contains("((uint64_t)(locus & 0xFFFFFu) << 44)"), "bus locus at bits 44..64");
    assert!(HEADER.contains("((seq & 0xFFFFFFFFFFFFULL) << 16)"), "net seq at bits 16..64");
}
