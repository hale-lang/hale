//! The vault a test's `hale` runs under (GH #989's Deferred line, closed):
//! `hale dna init` and `upgrade` provision the organism's secrets and
//! `hale dna secret set` fills a slot, so a test that ran them against the
//! developer's vault left a throwaway organization's entries there, or a
//! fake key in a real slot. Every `hale` a DNA test starts goes through
//! `hale()`: the vault is this test process's own, under the temporary
//! directory, and no real vault (`HALE_VAULT_ADDR`) is reached.
//! `harness_vault.rs`'s guard holds the DNA tests to it. (A `.hl` fixture
//! gets one per test file from `hale test` itself.)

use std::path::PathBuf;
use std::process::Command;

/// This test process's vault directory, `<tmp>/hale-test-vaults/<pid>`
/// (mode 700). The first call sweeps the vaults of test processes that
/// are gone, as `hale test` does for its own beside them: nextest runs
/// each test as a process, and none outlives its test to remove one.
pub fn dir() -> PathBuf {
    static MADE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    MADE.get_or_init(|| {
        let root = std::env::temp_dir().join("hale-test-vaults");
        if let Ok(entries) = std::fs::read_dir(&root) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let Ok(pid) = name.split('-').next().unwrap_or("").parse::<i32>() else { continue };
                let gone = pid > 0
                    && unsafe { libc::kill(pid, 0) } != 0
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
                if gone {
                    let _ = std::fs::remove_dir_all(e.path());
                }
            }
        }
        // a gone process whose pid this one reuses left nothing behind
        let dir = root.join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        dir
    })
    .clone()
}

/// `hale`, under this test process's vault.
pub fn hale() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    c.env("HALE_VAULT_DIR", dir()).env_remove("HALE_VAULT_ADDR");
    c
}
