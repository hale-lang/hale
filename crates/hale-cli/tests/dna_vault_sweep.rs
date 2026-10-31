//! GH #1653: a test killed mid-run leaves nothing once the next sweep
//! runs, even when its vault went before its habitat did.
//!
//! Every toolchain before GH #1653 removes a gone process's test vault
//! without taking its habitat down, and every toolchain on the machine
//! shares the vault root, so a habitat can outlive its vault. The sweep
//! (`hale habitat sweep --test-vaults`, which `hale test` and the Rust
//! tests' vaults run) finds such a habitat by its compose working
//! directory, under a gone process's vault, and takes it down by its
//! project's name.

#[path = "support/scratch.rs"]
mod scratch;

use std::path::Path;
use std::process::Command;

fn docker_ok() -> bool {
    Command::new("docker").args(["compose", "version"]).output().is_ok_and(|o| o.status.success())
        && Command::new("docker").args(["info"]).output().is_ok_and(|o| o.status.success())
}

fn left_of(project: &str) -> String {
    let label = format!("label=com.docker.compose.project={project}");
    let mut left = String::new();
    for args in [vec!["ps", "-a", "-q", "--filter", &label], vec!["volume", "ls", "-q", "--filter", &label]] {
        let o = Command::new("docker").args(&args).output().expect("docker");
        left.push_str(&String::from_utf8_lossy(&o.stdout));
    }
    left.trim().to_string()
}

fn sweep(root: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_hale")).args(["habitat", "sweep", "--test-vaults"]).arg(root).output().expect("hale habitat sweep")
}

#[test]
fn a_killed_tests_habitat_is_swept_after_its_vault_went() {
    if !docker_ok() {
        eprintln!("dna_vault_sweep: not exercised (no docker compose)");
        return;
    }
    // the vault root, a scratch directory of the test's (whatever is left
    // under it comes down when it drops)
    let root = scratch::Scratch::new(format!("hale_vault_sweep_{}", std::process::id()));
    // the test process: alive while its habitat comes up, then killed
    let mut test = Command::new("sleep").arg("600").spawn().expect("a stand-in test process");
    let vault = root.join(format!("{}-killed.vault", test.id()));
    let alloc = vault.join("data/hale/habitat/local/allocations/0123456789ab");
    std::fs::create_dir_all(&alloc).unwrap();
    std::fs::write(alloc.join("compose.yaml"), "services:\n  nerves-0123456789ab:\n    image: nats:2\n    network_mode: bridge\n    volumes:\n      - nerves-0123456789ab:/data\nvolumes:\n  nerves-0123456789ab: {}\n").unwrap();
    let project = format!("hale-habitat-local-sweep{}", test.id());
    let up = Command::new("docker").args(["compose", "-p", &project, "up", "-d"]).current_dir(&alloc).output().expect("docker compose up");
    assert!(up.status.success(), "the killed test's habitat comes up: {}", String::from_utf8_lossy(&up.stderr));
    // a live test's habitat is left alone
    let alive = sweep(&root);
    assert!(alive.status.success(), "{}", String::from_utf8_lossy(&alive.stderr));
    assert!(!left_of(&project).is_empty() && vault.exists(), "a live test's vault and habitat are not swept");
    // killed mid-run, and its vault removed without its habitat, as a
    // toolchain before GH #1653 removes it
    let _ = test.kill();
    let _ = test.wait();
    std::fs::remove_dir_all(&vault).unwrap();
    let swept = sweep(&root);
    let left = left_of(&project);
    if !left.is_empty() {
        let _ = Command::new("docker").args(["compose", "-p", &project, "down", "-v"]).output();
    }
    assert!(swept.status.success(), "the sweep: {}", String::from_utf8_lossy(&swept.stderr));
    assert_eq!(left, "", "nothing of the killed test's habitat is left");
    assert!(!vault.exists(), "and no vault is made again");
}
