//! The `bus_topics` integration-test binary: 22 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "adapter_fanout_own_bytes.rs"]
mod adapter_fanout_own_bytes;
#[path = "binding_birth_fail.rs"]
mod binding_birth_fail;
#[path = "binding_ingest_468.rs"]
mod binding_ingest_468;
#[path = "binding_keyed_over_wire.rs"]
mod binding_keyed_over_wire;
#[path = "binding_listen_rearm.rs"]
mod binding_listen_rearm;
#[path = "binding_loss_supervision.rs"]
mod binding_loss_supervision;
#[path = "binding_multi_peer.rs"]
mod binding_multi_peer;
#[path = "binding_stream_framing.rs"]
mod binding_stream_framing;
#[path = "bindings_codec_clause.rs"]
mod bindings_codec_clause;
#[path = "bindings_transport_dissolve.rs"]
mod bindings_transport_dissolve;
#[path = "codec_dispatch_roundtrip.rs"]
mod codec_dispatch_roundtrip;
#[path = "codec_encode_oversize.rs"]
mod codec_encode_oversize;
#[path = "codec_instantiation.rs"]
mod codec_instantiation;
#[path = "gate_counters.rs"]
mod gate_counters;
#[path = "nested_offthread_delivery.rs"]
mod nested_offthread_delivery;
#[path = "nested_struct_bus_payload.rs"]
mod nested_struct_bus_payload;
#[path = "placement_occurrences.rs"]
mod placement_occurrences;
#[path = "replica_keys.rs"]
mod replica_keys;
#[path = "serializer_shape.rs"]
mod serializer_shape;
#[path = "sleep_drains_bus.rs"]
mod sleep_drains_bus;
#[path = "topic_declarations.rs"]
mod topic_declarations;
#[path = "topic_phase2.rs"]
mod topic_phase2;
#[path = "topic_phase2_adapter_binding.rs"]
mod topic_phase2_adapter_binding;
