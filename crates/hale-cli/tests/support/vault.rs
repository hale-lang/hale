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

/// The root every test vault sits under, this user's alone:
/// `<tmp>/hale-test-vaults-<uid>`, made mode 700 and refused (loudly)
/// when it is not a directory this user owns, so another user's root or
/// a planted symlink never becomes where a test's secrets go.
fn root() -> PathBuf {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let uid = unsafe { libc::getuid() };
    let root = std::env::temp_dir().join(format!("hale-test-vaults-{uid}"));
    let _ = std::fs::create_dir(&root);
    let meta = std::fs::symlink_metadata(&root).unwrap_or_else(|e| panic!("test vault root {}: {e}", root.display()));
    assert!(meta.is_dir() && meta.uid() == uid, "test vault root {} is not a directory of this user's", root.display());
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).expect("test vault root mode");
    root
}

/// This test module's vault directory, `<root>/<pid>-<module>` (mode
/// 700). Every test file includes this one, so an area binary holds one
/// per file: keyed by the module too, no file's first call empties a
/// vault another file's organism is running on when libtest runs them as
/// threads of one process. The first call sweeps the vaults of test
/// processes that are gone, as `hale test` does for its own beside them:
/// nextest runs each test as a process, and none outlives its test to
/// remove one.
pub fn dir() -> PathBuf {
    static MADE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    MADE.get_or_init(|| {
        let root = root();
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
        let module = module_path!().replace("::", ".");
        let dir = root.join(format!("{}-{module}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap_or_else(|e| panic!("test vault {}: {e}", dir.display()));
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("test vault mode");
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
