//! The `topology` integration-test binary: 3 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "topology_artifact_contract.rs"]
mod topology_artifact_contract;
#[path = "topology_graph_cli.rs"]
mod topology_graph_cli;
#[path = "topology_v2.rs"]
mod topology_v2;
