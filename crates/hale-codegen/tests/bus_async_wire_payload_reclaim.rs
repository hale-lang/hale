//! P0 leak regression (downstream handoff 2026-07-15) — a cross-thread
//! bus payload delivered to a long-lived `where async_io` subscriber must
//! be reclaimed PER DELIVERY, not accumulated in the subscriber's
//! program-lifetime locus arena.
//!
//! Shape of the bug: a cross-thread publish posts a WIRE cell (serialized
//! payload) that the owner thread deserializes just before the handler
//! runs (`lotus_bus_cell_materialize`). The deserializer allocates the
//! payload's owned fields (here a 16 KiB String) into an arena — and
//! routing that into the subscriber's LOCUS arena leaked one payload's
//! worth of heap on every delivery. On an async_io subscriber whose run()
//! parks (the canonical server loop), that arena is never reclaimed until
//! dissolve, so a steady publish stream grows RSS without bound. The fix
//! deserializes into a per-delivery subregion destroyed right after the
//! handler returns.
//!
//! This test pins the fix to measured RSS: an async_io subscriber flooded
//! with LARGE (16 KiB) payloads must end near the same RSS as the
//! identical subscriber flooded with TINY (1-byte) payloads. Pre-fix the
//! large run accumulated ~320 MiB over 20k deliveries; the tiny run stayed
//! at the floor. Comparing the two isolates the payload-reclaim path from
//! the build's baseline arena (which the alloc-model RSS tests note sits
//! tens of MB above the optimized CLI build), so the assertion is a
//! relative delta, not a fragile absolute bound.

use std::process::Command;

use hale_codegen::build_executable;

/// A flood program parameterized by the per-payload body. An async_io
/// subscriber (its run() parked forever on an accept — the server-loop
/// shape) counts deliveries and, at the target count, prints final RSS and
/// exits. The publisher is `pinned`, so every publish crosses a thread
/// boundary and posts a wire cell that the subscriber's pool worker
/// materializes — the exact path that leaked.
fn flood_src(body_expr: &str, n: u32) -> String {
    format!(
        r#"
        type Payload {{ seq: Int; body: String; }}
        locus Sink {{
            params {{ got: Int = 0; n: Int = {n}; acc: Int = 0; }}
            bus {{ subscribe "flood" as on_msg of type Payload; }}
            fn on_msg(m: Payload) {{
                self.got = self.got + 1;
                self.acc = self.acc + len(m.body);
                if self.got == self.n {{
                    print("acc="); println(self.acc);
                    print("final_rss_mb="); println(std::process::rss_bytes() / 1048576);
                    std::process::exit(0);
                }}
            }}
            run() {{
                // Park run() on an OS-chosen port (nobody connects) so the
                // pool worker spends its life draining bus cells — the
                // long-lived-subscriber shape where the leak accrued.
                let lfd = std::io::tcp::__listen_socket("127.0.0.1", 0);
                let _ = std::io::tcp::__accept_one(lfd);
            }}
        }}
        locus Flood {{
            params {{ n: Int = {n}; }}
            bus {{ publish "flood" of type Payload; }}
            run() {{
                std::time::sleep(200ms);
                let body = {body_expr};
                let mut i = 0;
                while i < self.n {{
                    "flood" <- Payload {{ seq: i, body: body }};
                    i = i + 1;
                    // Yield periodically so the async subscriber drains and
                    // publisher-side ring backpressure stays bounded.
                    if i - (i / 200) * 200 == 0 {{ std::time::sleep(1ms); }}
                }}
            }}
        }}
        main locus App {{
            params {{ sink: Sink = Sink {{ }}; flood: Flood = Flood {{ }}; }}
            placement {{ sink: cooperative(pool = io) where async_io; flood: pinned; }}
            run() {{ std::time::sleep(30s); }}
        }}
        fn main() {{ App {{ }}; }}
        "#,
    )
}

fn build_and_rss(name: &str, src: &str) -> i64 {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_bus_async_reclaim_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(output.status.success(), "{} crashed: {:?}", name, output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .find(|l| l.starts_with("final_rss_mb="))
        .and_then(|l| l.trim_start_matches("final_rss_mb=").trim().parse().ok())
        .unwrap_or_else(|| panic!("no final_rss_mb in {} stdout: {:?}", name, stdout))
}

#[test]
fn async_io_wire_payload_reclaimed_per_delivery() {
    const N: u32 = 20_000;

    // 16 KiB body per delivery: 20k × 16 KiB = 320 MiB of accumulation if
    // the payload leaks into the locus arena (the pre-fix behavior).
    let large_rss = build_and_rss("large", &flood_src(r#"std::str::repeat("0123456789abcdef", 1024)"#, N));
    // 1-byte body: the same delivery machinery, negligible payload — the
    // control that isolates baseline arena from payload accumulation.
    let tiny_rss = build_and_rss("tiny", &flood_src(r#""x""#, N));

    // Post-fix both sit at the runtime floor: the per-delivery region is
    // destroyed after each handler, so the 16 KiB body never persists. A
    // generous 96 MiB gate still fails hard on the pre-fix ~320 MiB leak
    // while absorbing arena/allocator slack between the two builds.
    assert!(
        large_rss <= tiny_rss + 96,
        "async_io subscriber flooded with 16 KiB payloads ended at {large_rss} MiB vs \
         {tiny_rss} MiB for 1-byte payloads — a >96 MiB gap means the cross-thread \
         wire payload is accumulating in the subscriber's locus arena again \
         (per-delivery reclaim regressed in lotus_bus_cell_materialize / the drain paths).",
    );
}
