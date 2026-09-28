//! The `bus_core` integration-test binary: 19 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "bus_adapter_inbound.rs"]
mod bus_adapter_inbound;
#[path = "bus_array_payload.rs"]
mod bus_array_payload;
#[path = "bus_async_wire_payload_reclaim.rs"]
mod bus_async_wire_payload_reclaim;
#[path = "bus_backpressure.rs"]
mod bus_backpressure;
#[path = "bus_bounded_topics.rs"]
mod bus_bounded_topics;
#[path = "bus_bytes_payload.rs"]
mod bus_bytes_payload;
#[path = "bus_config.rs"]
mod bus_config;
#[path = "bus_cross_thread_drain.rs"]
mod bus_cross_thread_drain;
#[path = "bus_decimal_store.rs"]
mod bus_decimal_store;
#[path = "bus_devirt_differential.rs"]
mod bus_devirt_differential;
#[path = "bus_devirt_direct.rs"]
mod bus_devirt_direct;
#[path = "bus_devirt_no_pinned.rs"]
mod bus_devirt_no_pinned;
#[path = "bus_handler_freefn.rs"]
mod bus_handler_freefn;
#[path = "bus_large_payload.rs"]
mod bus_large_payload;
#[path = "bus_payload_arena_cap.rs"]
mod bus_payload_arena_cap;
#[path = "bus_publish_stack_alloca.rs"]
mod bus_publish_stack_alloca;
#[path = "bus_routing_keys.rs"]
mod bus_routing_keys;
#[path = "bus_subscriber.rs"]
mod bus_subscriber;
#[path = "bus_wildcards.rs"]
mod bus_wildcards;
