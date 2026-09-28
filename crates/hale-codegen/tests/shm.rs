//! The `shm` integration-test binary: 15 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "shm_drain_batch.rs"]
mod shm_drain_batch;
#[path = "shm_ring.rs"]
mod shm_ring;
#[path = "shm_ring_hale_subscriber.rs"]
mod shm_ring_hale_subscriber;
#[path = "shm_ring_layout_bytesview.rs"]
mod shm_ring_layout_bytesview;
#[path = "shm_ring_layout_bytesview_producer.rs"]
mod shm_ring_layout_bytesview_producer;
#[path = "shm_ring_layout_codegen.rs"]
mod shm_ring_layout_codegen;
#[path = "shm_ring_layout_lotus_dogfood.rs"]
mod shm_ring_layout_lotus_dogfood;
#[path = "shm_ring_layout_record_header.rs"]
mod shm_ring_layout_record_header;
#[path = "shm_ring_layout_zerocopy_write.rs"]
mod shm_ring_layout_zerocopy_write;
#[path = "shm_ring_nested_param_subscriber.rs"]
mod shm_ring_nested_param_subscriber;
#[path = "shm_ring_overflow.rs"]
mod shm_ring_overflow;
#[path = "shm_ring_publish.rs"]
mod shm_ring_publish;
#[path = "shm_ring_record_header_fields.rs"]
mod shm_ring_record_header_fields;
#[path = "shm_ring_torn_read_stress.rs"]
mod shm_ring_torn_read_stress;
#[path = "shm_write_topic_mangle.rs"]
mod shm_write_topic_mangle;
