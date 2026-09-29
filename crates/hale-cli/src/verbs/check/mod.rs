//! `hale check` and `hale verify`: the command line (`cli`), the per-target
//! implementation (`run_impl`) and the workspace and matrix modes (`matrix`).

use std::sync::atomic::AtomicBool;

pub(crate) mod cli;
pub(crate) mod matrix;
pub(crate) mod run_impl;

/// GH #476 Change 2: did `hale model dump` ask for the canonical
/// model? The command is a shim into the check pipeline, and the
/// pipeline's dump section reads PROCESS argv (it is a
/// top-level-command scope), so the demand cannot ride the rest-args
/// the shim forwards.
///
/// GH #887: it used to travel as `HALE_DUMP_MODEL` in the
/// environment. `std::env::set_var` is undefined behaviour in a
/// process that has threads — the environment is one table with no
/// lock, and every `getenv` in flight races it — and this CLI starts
/// them (the LSP, the observation reader, a child's pipes). The
/// demand is one process-global bit read by one in-process consumer,
/// so it is one process-global bit: set by the shim before anything
/// else runs, read where the flag is read, and (unlike the env var)
/// not inherited by any child.
pub(crate) static MODEL_DUMP_DEMANDED: AtomicBool = AtomicBool::new(false);
