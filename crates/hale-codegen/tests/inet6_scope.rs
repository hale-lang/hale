//! An IPv6 source address keeps its zone (`fe80::1%eth0`) through
//! `std::io::udp::recv_with_source`, so a reply to a link-local sender
//! reaches it: the scope id used to be dropped and the source read as a
//! bare `fe80::1`, which no interface routes. A link-local peer cannot be
//! arranged on a CI host's loopback, so this holds the two halves to each
//! other directly: the runtime's formatter writes the zone, by name when
//! the interface has one and by index when it has none, and the resolver
//! `send` and `connect` use reads it back to the same address and scope.

use std::path::PathBuf;
use std::process::Command;

#[path = "support/harness.rs"]
mod harness;

#[test]
fn a_scoped_ipv6_source_formats_with_its_zone_and_resolves_back() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bin = harness::unique_bin("inet6_scope_driver");
    let status = Command::new("clang")
        .arg(manifest.join("tests").join("inet6_scope_driver.c"))
        .arg("-O1")
        .arg("-lpthread")
        .arg("-lm")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building the driver");
    let out = Command::new(&bin).output().expect("run the driver");
    let _ = std::fs::remove_file(&bin);
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "{:?}\n{}", out.status, text);
    let lo = text.lines().next().and_then(|l| l.strip_prefix("loopback=")).expect("the loopback's name");
    assert!(
        text.contains(&format!("named text=fe80::1%{lo} roundtrip=same")),
        "a zone with a name is written by name and resolves back:\n{text}"
    );
    assert!(
        text.contains("index text=fe80::1%4000000 roundtrip=same"),
        "a zone with no interface name is written by index and resolves back:\n{text}"
    );
    assert!(text.contains("unscoped text=2001:db8::1 roundtrip=same"), "no zone, no suffix:\n{text}");
}
