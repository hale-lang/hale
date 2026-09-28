//! The `async_io` integration-test binary: 17 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "async_io_park_resume.rs"]
mod async_io_park_resume;
#[path = "async_io_recv_into_park.rs"]
mod async_io_recv_into_park;
#[path = "async_io_shutdown_parked.rs"]
mod async_io_shutdown_parked;
#[path = "async_io_udp_multi_reader.rs"]
mod async_io_udp_multi_reader;
#[path = "async_io_udp_recv_in_handler.rs"]
mod async_io_udp_recv_in_handler;
#[path = "asyncio_sleep_park.rs"]
mod asyncio_sleep_park;
#[path = "coop_pool_basic.rs"]
mod coop_pool_basic;
#[path = "coop_pool_multi_locus.rs"]
mod coop_pool_multi_locus;
#[path = "coop_pool_run_dispatch.rs"]
mod coop_pool_run_dispatch;
#[path = "coop_to_pinned_mid_program.rs"]
mod coop_to_pinned_mid_program;
#[path = "recv_into.rs"]
mod recv_into;
#[path = "recv_stamped.rs"]
mod recv_stamped;
#[path = "recv_timeout_sentinel.rs"]
mod recv_timeout_sentinel;
#[path = "recv_zero_alloc.rs"]
mod recv_zero_alloc;
#[path = "stream_fallible_io.rs"]
mod stream_fallible_io;
#[path = "stream_fallible_unaddressed_rejects.rs"]
mod stream_fallible_unaddressed_rejects;
#[path = "udp_reader.rs"]
mod udp_reader;
