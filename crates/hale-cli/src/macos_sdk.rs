//! macOS: an SDK the selected linker can read, when `SDKROOT` does not
//! name one.
//!
//! clang links against the newest macOS SDK `xcrun` finds, wherever it
//! lives, with the linker of the developer tools `xcode-select` (or
//! `DEVELOPER_DIR`) selects. With an older Xcode selected beside newer
//! Command Line Tools those two disagree: the SDK is the Command Line
//! Tools' and the linker is Xcode's, too old to read the SDK's `.tbd`
//! stubs (`tapi error: malformed file … unknown architecture`), and every
//! link fails, a plain C program's as well.
//!
//! The rule: an explicit `SDKROOT` is the user's and is left alone. An
//! SDK under the linker's own developer directory is its own and is used
//! as it is, with nothing probed. Otherwise a trivial program is linked
//! against the default SDK; if the linker reads it, it stays. If the
//! linker refuses it, the SDKs installed under the linker's own developer
//! directory are tried, newest first, then the Command Line Tools' ones,
//! and the first that links is handed to every host clang the build runs
//! (`BuildOptions::macos_sdkroot`). The answer is kept on disk beside the
//! runtime-object cache, keyed by the linker (path, size, mtime) and the
//! default SDK, so the trial links run once per toolchain, and at most
//! once per process. When no SDK links, nothing is handed on and the
//! build's own link fails naming the SDKs tried and the fixes.
//!
//! Nothing here runs on any other host.

use std::path::{Path, PathBuf};

/// What a build hands its host clang.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Selection {
    /// The SDK to link against, when clang's own choice is unreadable.
    pub sdkroot: Option<PathBuf>,
    /// The SDKs the linker refused, in the order they were tried.
    pub unreadable: Vec<PathBuf>,
}

/// How a trial link against one SDK went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Probe {
    Links,
    /// The linker could not read the SDK's stubs.
    Unreadable,
    /// It failed some other way (no clang, say): not the SDK's doing.
    Failed,
}

/// The CommandLineTools SDK directory, which `xcrun` searches whatever
/// is selected.
const CLT_SDKS: &str = "/Library/Developer/CommandLineTools/SDKs";

/// The selection for this host, `sdkroot_env` being `SDKROOT` as the
/// environment has it. Computed once per process.
pub(crate) fn for_host(sdkroot_env: Option<&str>, cache_dir: &Path) -> Selection {
    if !cfg!(target_os = "macos") || sdkroot_env.is_some_and(|v| !v.is_empty()) {
        return Selection::default();
    }
    static ONCE: std::sync::OnceLock<Selection> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| select_on_this_mac(cache_dir)).clone()
}

fn select_on_this_mac(cache_dir: &Path) -> Selection {
    let Some(default_sdk) = xcrun(&["--show-sdk-path"]) else {
        return Selection::default();
    };
    let Some(linker) = xcrun(&["-f", "ld"]) else {
        return Selection::default();
    };
    let dev = developer_dir_of_linker(&linker);
    let default_sdk = canonical(&default_sdk);
    if default_sdk.starts_with(canonical(&dev)) {
        return Selection::default();
    }
    let cache = cache_file(cache_dir, &linker, &default_sdk);
    if let Some(sel) = cache.as_deref().and_then(read_cache) {
        return sel;
    }
    let sel = choose(&default_sdk, &own_sdk_dirs(&dev), &[PathBuf::from(CLT_SDKS)], &mut trial_link);
    if let Some(path) = cache {
        write_cache(&path, &sel);
    }
    sel
}

/// The developer directory a linker belongs to: Xcode's
/// `<Xcode.app>/Contents/Developer` for
/// `…/Toolchains/XcodeDefault.xctoolchain/usr/bin/ld`, the Command Line
/// Tools' root for `…/CommandLineTools/usr/bin/ld`.
pub(crate) fn developer_dir_of_linker(linker: &Path) -> PathBuf {
    // `<toolchain>/usr/bin/ld`
    let toolchain = linker.parent().and_then(Path::parent).and_then(Path::parent).unwrap_or(Path::new("/"));
    match toolchain.parent() {
        Some(p) if p.file_name().is_some_and(|n| n == "Toolchains") => p.parent().unwrap_or(p).to_path_buf(),
        _ => toolchain.to_path_buf(),
    }
}

/// Where a developer directory keeps its macOS SDKs: Xcode's platform
/// directory, or the Command Line Tools' `SDKs`.
pub(crate) fn own_sdk_dirs(dev: &Path) -> Vec<PathBuf> {
    vec![dev.join("Platforms/MacOSX.platform/Developer/SDKs"), dev.join("SDKs")]
}

/// The choice itself, over any trial link: the default SDK when the
/// linker reads it (or the trial failed for another reason, which the
/// build's own link will report), else the first SDK under `own` then
/// `others` that links, newest first.
pub(crate) fn choose(
    default_sdk: &Path,
    own: &[PathBuf],
    others: &[PathBuf],
    probe: &mut dyn FnMut(&Path) -> Probe,
) -> Selection {
    match probe(default_sdk) {
        Probe::Links | Probe::Failed => return Selection::default(),
        Probe::Unreadable => {}
    }
    let mut unreadable = vec![default_sdk.to_path_buf()];
    let mut seen = vec![canonical(default_sdk)];
    for dir in own.iter().chain(others) {
        for sdk in sdks_in(dir) {
            if seen.contains(&sdk) {
                continue;
            }
            seen.push(sdk.clone());
            match probe(&sdk) {
                Probe::Links => return Selection { sdkroot: Some(sdk), unreadable: Vec::new() },
                Probe::Unreadable => unreadable.push(sdk),
                Probe::Failed => {}
            }
        }
    }
    Selection { sdkroot: None, unreadable }
}

/// The macOS SDKs in one directory, symlinks resolved and each once,
/// newest version first.
pub(crate) fn sdks_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut sdks: Vec<PathBuf> = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("MacOSX") && name.ends_with(".sdk")) {
            continue;
        }
        let path = canonical(&e.path());
        if path.is_dir() && !sdks.contains(&path) {
            sdks.push(path);
        }
    }
    sdks.sort_by(|a, b| sdk_version(b).cmp(&sdk_version(a)).then_with(|| a.cmp(b)));
    sdks
}

/// `MacOSX26.5.sdk` → `[26, 5]`; an unversioned name sorts last.
fn sdk_version(sdk: &Path) -> Vec<u32> {
    let name = sdk.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let v = name.trim_start_matches("MacOSX").trim_end_matches(".sdk");
    v.split('.').map_while(|p| p.parse().ok()).collect()
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn xcrun(args: &[&str]) -> Option<PathBuf> {
    let out = std::process::Command::new("xcrun").args(args).stderr(std::process::Stdio::null()).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !s.is_empty()).then(|| PathBuf::from(s))
}

/// Link `int main` against `sdk` with the host clang.
fn trial_link(sdk: &Path) -> Probe {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NONCE: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "hale-sdk-probe-{}-{}",
        std::process::id(),
        NONCE.fetch_add(1, Ordering::Relaxed)
    ));
    if std::fs::create_dir_all(&dir).is_err() {
        return Probe::Failed;
    }
    let src = dir.join("probe.c");
    let probe = match std::fs::write(&src, "int main(void) { return 0; }\n") {
        Err(_) => Probe::Failed,
        Ok(()) => match std::process::Command::new("clang")
            .env("SDKROOT", sdk)
            .arg(&src)
            .arg("-o")
            .arg(dir.join("probe"))
            .output()
        {
            Ok(o) if o.status.success() => Probe::Links,
            Ok(o) if hale_codegen::macos_linker_cannot_read_sdk(&String::from_utf8_lossy(&o.stderr)) => Probe::Unreadable,
            _ => Probe::Failed,
        },
    };
    let _ = std::fs::remove_dir_all(&dir);
    probe
}

/// `<cache>/macos-sdk-<key>`, the key naming the linker binary (path,
/// size, mtime: a toolchain update moves it) and the default SDK.
fn cache_file(cache_dir: &Path, linker: &Path, default_sdk: &Path) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let meta = std::fs::metadata(linker).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (linker, meta.len(), mtime.as_nanos(), default_sdk).hash(&mut h);
    Some(cache_dir.join(format!("macos-sdk-{:016x}", h.finish())))
}

/// `use <sdk>` or `default`, then one `unreadable <sdk>` line per SDK
/// refused. A chosen SDK that has since gone is a miss.
pub(crate) fn parse_cache(text: &str) -> Option<Selection> {
    let mut lines = text.lines();
    let sdkroot = match lines.next()? {
        "default" => None,
        l => Some(PathBuf::from(l.strip_prefix("use ")?)),
    };
    let mut unreadable = Vec::new();
    for l in lines {
        unreadable.push(PathBuf::from(l.strip_prefix("unreadable ")?));
    }
    Some(Selection { sdkroot, unreadable })
}

pub(crate) fn render_cache(sel: &Selection) -> String {
    let mut s = match &sel.sdkroot {
        Some(p) => format!("use {}\n", p.display()),
        None => "default\n".to_string(),
    };
    for p in &sel.unreadable {
        s.push_str(&format!("unreadable {}\n", p.display()));
    }
    s
}

fn read_cache(path: &Path) -> Option<Selection> {
    let sel = parse_cache(&std::fs::read_to_string(path).ok()?)?;
    match &sel.sdkroot {
        Some(sdk) if !sdk.is_dir() => None,
        _ => Some(sel),
    }
}

fn write_cache(path: &Path, sel: &Selection) {
    let Some(dir) = path.parent() else { return };
    let _ = std::fs::create_dir_all(dir);
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    if std::fs::write(&tmp, render_cache(sel)).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A scratch tree of fake SDK directories, removed on drop.
    struct Tree(PathBuf);
    impl Tree {
        fn new(tag: &str) -> Tree {
            use std::sync::atomic::{AtomicU64, Ordering};
            static N: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "hale-macos-sdk-test-{tag}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&root).unwrap();
            Tree(canonical(&root))
        }
        fn sdk(&self, dir: &str, name: &str) -> PathBuf {
            let p = self.0.join(dir).join(name);
            std::fs::create_dir_all(&p).unwrap();
            p
        }
        fn link(&self, dir: &str, name: &str, to: &str) -> PathBuf {
            let p = self.0.join(dir).join(name);
            std::os::unix::fs::symlink(to, &p).unwrap();
            p
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A trial link that answers from a table and records what it was asked.
    fn table(answers: HashMap<PathBuf, Probe>, asked: &mut Vec<PathBuf>) -> impl FnMut(&Path) -> Probe + '_ {
        move |sdk| {
            asked.push(sdk.to_path_buf());
            *answers.get(sdk).unwrap_or(&Probe::Unreadable)
        }
    }

    #[test]
    fn the_linker_of_xcode_and_of_the_command_line_tools_names_its_developer_dir() {
        assert_eq!(
            developer_dir_of_linker(Path::new(
                "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/ld"
            )),
            PathBuf::from("/Applications/Xcode.app/Contents/Developer")
        );
        assert_eq!(
            developer_dir_of_linker(Path::new("/Library/Developer/CommandLineTools/usr/bin/ld")),
            PathBuf::from("/Library/Developer/CommandLineTools")
        );
        assert_eq!(
            own_sdk_dirs(Path::new("/X/Contents/Developer"))[0],
            PathBuf::from("/X/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs")
        );
    }

    #[test]
    fn sdks_are_listed_once_each_newest_first() {
        let t = Tree::new("list");
        let v265 = t.sdk("clt", "MacOSX26.5.sdk");
        let v27 = t.sdk("clt", "MacOSX27.0.sdk");
        let v264 = t.sdk("clt", "MacOSX26.4.sdk");
        t.link("clt", "MacOSX.sdk", "MacOSX27.0.sdk");
        t.link("clt", "MacOSX26.sdk", "MacOSX26.5.sdk");
        t.sdk("clt", "iPhoneOS27.0.sdk");
        assert_eq!(sdks_in(&t.0.join("clt")), vec![v27, v265, v264]);
        assert!(sdks_in(&t.0.join("missing")).is_empty());
    }

    #[test]
    fn a_default_sdk_the_linker_reads_is_kept_without_looking_further() {
        let t = Tree::new("keep");
        let default = t.sdk("clt", "MacOSX27.0.sdk");
        t.sdk("xcode", "MacOSX26.2.sdk");
        let mut asked = Vec::new();
        let sel = choose(&default, &[t.0.join("xcode")], &[t.0.join("clt")], &mut table(HashMap::from([(default.clone(), Probe::Links)]), &mut asked));
        assert_eq!(sel, Selection::default());
        assert_eq!(asked, vec![default]);
    }

    #[test]
    fn an_unreadable_default_falls_back_to_the_linkers_own_newest_sdk() {
        // The case on the machine that prompted this: Xcode 26.2 selected,
        // the Command Line Tools' 27.0 SDK the default.
        let t = Tree::new("own");
        let default = t.sdk("clt", "MacOSX27.0.sdk");
        t.link("clt", "MacOSX.sdk", "MacOSX27.0.sdk");
        let clt265 = t.sdk("clt", "MacOSX26.5.sdk");
        let x262 = t.sdk("xcode", "MacOSX26.2.sdk");
        let x261 = t.sdk("xcode", "MacOSX26.1.sdk");
        t.link("xcode", "MacOSX.sdk", "MacOSX26.2.sdk");
        let answers = HashMap::from([(x262.clone(), Probe::Links), (x261, Probe::Links), (clt265, Probe::Links)]);
        let mut asked = Vec::new();
        let sel = choose(&default, &[t.0.join("xcode")], &[t.0.join("clt")], &mut table(answers, &mut asked));
        assert_eq!(sel, Selection { sdkroot: Some(x262.clone()), unreadable: Vec::new() });
        assert_eq!(asked, vec![default, x262]);
    }

    #[test]
    fn with_no_own_sdk_that_links_the_other_tools_older_sdk_is_taken() {
        let t = Tree::new("other");
        let default = t.sdk("clt", "MacOSX27.0.sdk");
        let x = t.sdk("xcode", "MacOSX26.2.sdk");
        let clt265 = t.sdk("clt", "MacOSX26.5.sdk");
        let answers = HashMap::from([(x.clone(), Probe::Failed), (clt265.clone(), Probe::Links)]);
        let mut asked = Vec::new();
        let sel = choose(&default, &[t.0.join("xcode")], &[t.0.join("clt")], &mut table(answers, &mut asked));
        assert_eq!(sel.sdkroot, Some(clt265.clone()));
        // the default is never tried twice
        assert_eq!(asked, vec![default, x, clt265]);
    }

    #[test]
    fn when_no_sdk_links_none_is_chosen_and_every_refusal_is_named() {
        let t = Tree::new("none");
        let default = t.sdk("clt", "MacOSX27.0.sdk");
        let x = t.sdk("xcode", "MacOSX26.2.sdk");
        let mut asked = Vec::new();
        let sel = choose(&default, &[t.0.join("xcode")], &[t.0.join("clt")], &mut table(HashMap::new(), &mut asked));
        assert_eq!(sel, Selection { sdkroot: None, unreadable: vec![default, x] });
    }

    #[test]
    fn a_default_that_fails_for_another_reason_is_left_to_the_build() {
        let t = Tree::new("failed");
        let default = t.sdk("clt", "MacOSX27.0.sdk");
        t.sdk("xcode", "MacOSX26.2.sdk");
        let mut asked = Vec::new();
        let sel = choose(&default, &[t.0.join("xcode")], &[], &mut table(HashMap::from([(default.clone(), Probe::Failed)]), &mut asked));
        assert_eq!(sel, Selection::default());
        assert_eq!(asked.len(), 1);
    }

    #[test]
    fn the_cached_answer_reads_back_as_written() {
        for sel in [
            Selection::default(),
            Selection { sdkroot: Some(PathBuf::from("/x/MacOSX26.2.sdk")), unreadable: Vec::new() },
            Selection { sdkroot: None, unreadable: vec![PathBuf::from("/a/MacOSX27.0.sdk"), PathBuf::from("/b/MacOSX26.2.sdk")] },
        ] {
            assert_eq!(parse_cache(&render_cache(&sel)), Some(sel));
        }
        assert_eq!(parse_cache(""), None);
        assert_eq!(parse_cache("garbage\n"), None);
    }

    #[test]
    fn an_explicit_sdkroot_and_any_other_host_are_left_alone() {
        let nowhere = Path::new("/nonexistent-hale-cache");
        assert_eq!(for_host(Some("/Some/MacOSX.sdk"), nowhere), Selection::default());
        if !cfg!(target_os = "macos") {
            assert_eq!(for_host(None, nowhere), Selection::default());
        }
    }
}
