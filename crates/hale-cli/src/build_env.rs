//! The one place the process environment becomes codegen's
//! `BuildOptions`.
//!
//! Codegen reads no environment variable (GH #843, finished): every
//! `LOTUS_*` / `HALE_*` build knob it used to look up itself is a field
//! of `hale_codegen::BuildOptions`, and this function is what fills them
//! in for the CLI. Every command that compiles starts from it, so the
//! same variable means the same thing to `hale build`, `run`, `test`
//! and `replay`. The table of the variables is in `spec/runtime.md`
//! ("Build-time environment").

use std::path::PathBuf;

use hale_codegen::{BuildOptions, LtoMode};

/// `BuildOptions` from the process environment.
pub(crate) fn build_options_from_env() -> BuildOptions {
    build_options_from(|name| std::env::var(name).ok())
}

/// The same, over any lookup: what the unit tests below feed it.
pub(crate) fn build_options_from(get: impl Fn(&str) -> Option<String>) -> BuildOptions {
    // A boolean knob: `1`, `true` or `TRUE`.
    let flag = |name: &str| get(name).map(|v| v == "1" || v == "true" || v == "TRUE").unwrap_or(false);
    // A knob that means "on" by being set to anything at all.
    let is_set = |name: &str| get(name).is_some();
    let non_empty = |name: &str| get(name).filter(|v| !v.is_empty());

    // The runtime-object cache is the caller's to choose (`BuildOptions`
    // has no default for it): `$XDG_CACHE_HOME/hale/runtime`, else
    // `~/.cache/hale/runtime`. With neither set there is no per-user
    // place, and the fallback is a directory this process names for
    // itself, never a fixed path in the shared temp dir.
    let cache_dir = non_empty("XDG_CACHE_HOME")
        .map(|x| PathBuf::from(x).join("hale").join("runtime"))
        .or_else(|| non_empty("HOME").map(|h| PathBuf::from(h).join(".cache").join("hale").join("runtime")))
        .unwrap_or_else(|| std::env::temp_dir().join(format!("hale-runtime-cache-{}", std::process::id())));

    let mut o = BuildOptions::new(cache_dir);
    o.dev_profile = is_set("HALE_DEV");
    o.dump_ir_beside_output = is_set("LOTUS_DUMP_IR");
    o.no_bus_devirt = flag("LOTUS_NO_BUS_DEVIRT");
    o.no_ownership_bubble = flag("LOTUS_NO_OWNERSHIP_BUBBLE");
    o.asan = flag("LOTUS_ASAN");
    o.tsan = flag("LOTUS_TSAN");
    o.ubsan = flag("LOTUS_UBSAN");
    o.lto = get("LOTUS_LTO").map(|v| LtoMode::parse(&v));
    o.disable_prefetch = flag("LOTUS_DISABLE_PREFETCH");
    o.di_trace = is_set("LOTUS_DI_TRACE");
    o.dispatch_trace = flag("HALE_DISPATCH_TRACE");
    o.time_phases = is_set("HALE_TIME");
    o.cc_warnings = flag("HALE_CC_WARNINGS");
    o.no_lld = flag("HALE_NO_LLD");
    o.no_ts_shim = flag("HALE_NO_TS_SHIM");
    o.ts_shim = non_empty("HALE_TS_SHIM_A").map(PathBuf::from);
    o.zig = non_empty("HALE_ZIG");
    o.target_glibc = non_empty("HALE_TARGET_GLIBC");
    o.target_sysroot = non_empty("HALE_TARGET_SYSROOT").map(PathBuf::from);
    o.openssl_prefix = ["LOTUS_OPENSSL_PREFIX", "OPENSSL_ROOT_DIR"]
        .iter()
        .filter_map(|v| non_empty(v))
        .map(PathBuf::from)
        .find(|p| p.join("include/openssl/ssl.h").exists());
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn from(pairs: &[(&str, &str)]) -> BuildOptions {
        let env: BTreeMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        build_options_from(|n| env.get(n).cloned())
    }

    #[test]
    fn an_empty_environment_is_the_default_build() {
        let o = from(&[]);
        assert!(!o.asan && !o.tsan && !o.ubsan && !o.dev_profile && !o.no_lld && !o.cc_warnings);
        assert!(o.lto.is_none() && o.zig.is_none() && o.ts_shim.is_none());
        assert!(!o.dump_ir_beside_output && !o.time_phases && !o.di_trace && !o.dispatch_trace);
    }

    #[test]
    fn a_boolean_knob_is_1_true_or_upper_true_and_nothing_else() {
        for on in ["1", "true", "TRUE"] {
            assert!(from(&[("LOTUS_ASAN", on)]).asan, "{on}");
        }
        for off in ["0", "yes", "", "True", "on"] {
            assert!(!from(&[("LOTUS_ASAN", off)]).asan, "{off:?}");
        }
    }

    #[test]
    fn a_set_only_knob_is_on_by_being_set() {
        assert!(from(&[("HALE_TIME", "")]).time_phases);
        assert!(from(&[("HALE_DEV", "0")]).dev_profile, "HALE_DEV=0 is still set");
        assert!(from(&[("LOTUS_DUMP_IR", "1")]).dump_ir_beside_output);
        assert!(from(&[("LOTUS_DI_TRACE", "")]).di_trace);
    }

    #[test]
    fn lto_takes_its_spellings_and_an_unknown_one_is_off() {
        assert_eq!(from(&[("LOTUS_LTO", "thin")]).lto, Some(LtoMode::Thin));
        assert_eq!(from(&[("LOTUS_LTO", "1")]).lto, Some(LtoMode::Full));
        assert_eq!(from(&[("LOTUS_LTO", "nonsense")]).lto, Some(LtoMode::Off));
        assert_eq!(from(&[]).lto, None);
    }

    #[test]
    fn the_runtime_cache_is_under_xdg_then_home_and_an_empty_one_is_skipped() {
        let rt = |p: &str| PathBuf::from(p).join("hale").join("runtime");
        assert_eq!(from(&[("XDG_CACHE_HOME", "/x"), ("HOME", "/h")]).cache_dir, rt("/x"));
        assert_eq!(from(&[("XDG_CACHE_HOME", ""), ("HOME", "/h")]).cache_dir, PathBuf::from("/h/.cache/hale/runtime"));
    }

    #[test]
    fn with_no_cache_home_the_fallback_is_this_processs_own_directory() {
        let own = std::env::temp_dir().join(format!("hale-runtime-cache-{}", std::process::id()));
        assert_eq!(from(&[("HOME", "")]).cache_dir, own);
        assert_eq!(from(&[]).cache_dir, own);
    }

    #[test]
    fn a_cross_build_knob_is_taken_when_not_empty() {
        let o = from(&[("HALE_ZIG", "/opt/zig"), ("HALE_TARGET_GLIBC", "2.35"), ("HALE_TARGET_SYSROOT", "/sys"), ("HALE_TS_SHIM_A", "/a.a")]);
        assert_eq!(o.zig.as_deref(), Some("/opt/zig"));
        assert_eq!(o.target_glibc.as_deref(), Some("2.35"));
        assert_eq!(o.target_sysroot, Some(PathBuf::from("/sys")));
        assert_eq!(o.ts_shim, Some(PathBuf::from("/a.a")));
        assert!(from(&[("HALE_ZIG", ""), ("HALE_TARGET_GLIBC", "")]).zig.is_none());
    }
}
