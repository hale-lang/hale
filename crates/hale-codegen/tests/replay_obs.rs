//! The `replay_obs` integration-test binary: 12 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "obs_emission.rs"]
mod obs_emission;
#[path = "obs_entity_ids_unstamped.rs"]
mod obs_entity_ids_unstamped;
#[path = "obs_fleet_contract.rs"]
mod obs_fleet_contract;
#[path = "obs_intra_tree_publish.rs"]
mod obs_intra_tree_publish;
#[path = "obs_net_seq.rs"]
mod obs_net_seq;
#[path = "obs_protocol_header.rs"]
mod obs_protocol_header;
#[path = "replay_asyncio.rs"]
mod replay_asyncio;
#[path = "replay_canceled_run.rs"]
mod replay_canceled_run;
#[path = "replay_determinism.rs"]
mod replay_determinism;
#[path = "replay_ingress.rs"]
mod replay_ingress;
#[path = "replay_recording.rs"]
mod replay_recording;
#[path = "replay_truncated.rs"]
mod replay_truncated;