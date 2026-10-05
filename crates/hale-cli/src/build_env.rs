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
    o.lifecycle_trace = flag("HALE_LIFECYCLE_TRACE");
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

/// The build-options half of the execution identity. One spelling,
/// so `hale build` and `hale run` fingerprint the same options the
/// same way (they did not: the build path never computed a digest
/// at all — GH #476 Change 8 review).
pub(crate) fn options_fingerprint(o: &BuildOptions) -> String {
    // `debug` keeps its place with the constant `false` (F.40 phase 4,
    // I2): the DWARF line tables `hale build` adds by default change no
    // behaviour, and `run` and `replay` never add them, so fingerprinting
    // them made every recording of a built binary "different build
    // inputs". The constant keeps the string `run` has always stamped.
    let mut fp = format!(
        "target={:?};cpu={:?};dev={};debug=false",
        o.target, o.target_cpu, o.dev_profile
    );
    // GH #904: the FFI surface is part of what the executable IS —
    // two builds of one source that link different C are different
    // programs. Appended only when non-empty, so every recording
    // stamped before this (no `--link` / `--csrc`, which is every
    // recording `hale run` could make) keeps the identity it
    // carries.
    if !o.link_libs.is_empty() {
        fp.push_str(&format!(";link={}", o.link_libs.join(",")));
    }
    // GH #1106: an api binding is part of the program the binary is.
    if let Some(api) = &o.api {
        fp.push_str(&format!(";api={}", api));
    }
    // GH #1109: the role table is part of the binary too.
    if let Some(t) = &o.api_roles {
        fp.push_str(&format!(";roles={}", t));
    }
    if !o.csrc_files.is_empty() {
        let files: Vec<String> = o
            .csrc_files
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        fp.push_str(&format!(";csrc={}", files.join(",")));
    }
    // Every knob that changes the emitted binary, appended only when
    // set so each identity stamped before them (none of them was in
    // it: they were read from the environment inside codegen) keeps the
    // string it had. Not here: what only narrates or times a build
    // (`dump_ir*`, `di_trace`, `dispatch_trace`, `time_phases`), the
    // C warnings (`cc_warnings`), which linker runs (`no_lld`), and
    // where the cache lives (`cache_dir`).
    if o.asan {
        fp.push_str(";asan");
    }
    if o.tsan {
        fp.push_str(";tsan");
    }
    if o.ubsan {
        fp.push_str(";ubsan");
    }
    if let Some(l) = o.lto {
        if l != LtoMode::Off {
            fp.push_str(&format!(";lto={l:?}"));
        }
    }
    if o.disable_prefetch {
        fp.push_str(";no_prefetch");
    }
    if o.no_bus_devirt {
        fp.push_str(";no_bus_devirt");
    }
    if o.lifecycle_trace {
        fp.push_str(";lifecycle_trace");
    }
    if o.no_ownership_bubble {
        fp.push_str(";no_ownership_bubble");
    }
    if o.no_ts_shim {
        fp.push_str(";no_ts_shim");
    }
    if let Some(p) = &o.ts_shim {
        fp.push_str(&format!(";ts_shim={}", p.display()));
    }
    if let Some(z) = &o.zig {
        fp.push_str(&format!(";zig={z}"));
    }
    if let Some(g) = &o.target_glibc {
        fp.push_str(&format!(";glibc={g}"));
    }
    if let Some(p) = &o.target_sysroot {
        fp.push_str(&format!(";sysroot={}", p.display()));
    }
    if let Some(p) = &o.openssl_prefix {
        fp.push_str(&format!(";openssl={}", p.display()));
    }
    fp
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

    fn base() -> BuildOptions {
        BuildOptions::new(PathBuf::from("/cache"))
    }

    /// Every identity stamped before the knobs were options was made
    /// without them, so a default build must keep the string it had.
    #[test]
    fn a_default_builds_identity_is_what_it_always_was() {
        assert_eq!(options_fingerprint(&base()), "target=Native;cpu=Native;dev=false;debug=false");
    }

    /// Toggling any knob that changes the emitted binary changes the
    /// execution identity; each moves it to a string of its own.
    #[test]
    fn a_knob_that_changes_the_binary_changes_the_identity() {
        let knobs: Vec<(&str, Box<dyn Fn(&mut BuildOptions)>)> = vec![
            ("asan", Box::new(|o| o.asan = true)),
            ("tsan", Box::new(|o| o.tsan = true)),
            ("ubsan", Box::new(|o| o.ubsan = true)),
            ("lto thin", Box::new(|o| o.lto = Some(LtoMode::Thin))),
            ("lto full", Box::new(|o| o.lto = Some(LtoMode::Full))),
            ("disable_prefetch", Box::new(|o| o.disable_prefetch = true)),
            ("no_bus_devirt", Box::new(|o| o.no_bus_devirt = true)),
            ("lifecycle_trace", Box::new(|o| o.lifecycle_trace = true)),
            ("no_ownership_bubble", Box::new(|o| o.no_ownership_bubble = true)),
            ("no_ts_shim", Box::new(|o| o.no_ts_shim = true)),
            ("ts_shim", Box::new(|o| o.ts_shim = Some(PathBuf::from("/a.a")))),
            ("zig", Box::new(|o| o.zig = Some("/opt/zig".into()))),
            ("target_glibc", Box::new(|o| o.target_glibc = Some("2.35".into()))),
            ("target_sysroot", Box::new(|o| o.target_sysroot = Some(PathBuf::from("/sys")))),
            ("openssl_prefix", Box::new(|o| o.openssl_prefix = Some(PathBuf::from("/ssl")))),
            ("dev_profile", Box::new(|o| o.dev_profile = true)),
        ];
        let plain = options_fingerprint(&base());
        let mut seen = std::collections::BTreeSet::new();
        for (name, set) in &knobs {
            let mut o = base();
            set(&mut o);
            let fp = options_fingerprint(&o);
            assert_ne!(fp, plain, "{name} must change the execution identity");
            assert!(seen.insert(fp), "{name} shares an identity with another knob");
        }
        let mut two = base();
        two.target_glibc = Some("2.35".into());
        let mut other = base();
        other.target_glibc = Some("2.31".into());
        assert_ne!(options_fingerprint(&two), options_fingerprint(&other), "the value is part of the identity");
    }

    /// `debug` adds DWARF line tables and changes no behaviour; `hale
    /// build` sets it and `run` and `replay` never do, so it is no part
    /// of the identity (F.40 phase 4, I2), and the default string is
    /// what `run` has always stamped.
    #[test]
    fn debug_leaves_the_identity() {
        let plain = options_fingerprint(&base());
        let mut o = base();
        o.debug = Some(hale_codegen::DebugSources { files: Vec::new() });
        assert_eq!(options_fingerprint(&o), plain);
        assert_eq!(plain, "target=Native;cpu=Native;dev=false;debug=false");
    }

    /// The options half's covered changes each move the identity (I2):
    /// the target, `dev`, an environment's role table, a link library.
    #[test]
    fn a_covered_option_moves_the_identity() {
        let plain = options_fingerprint(&base());
        let moved = |set: &dyn Fn(&mut BuildOptions)| {
            let mut o = base();
            set(&mut o);
            options_fingerprint(&o) != plain
        };
        assert!(moved(&|o| o.target = hale_codegen::CompileTarget::Wasm32), "the target");
        assert!(moved(&|o| o.dev_profile = true), "dev");
        assert!(moved(&|o| o.api_roles = Some("ops=uid:1000".into())), "a role table");
        assert!(moved(&|o| o.link_libs.push("m".into())), "a link library");
    }

    /// The dispatch plan's own frame moves the execution identity, with
    /// the sources and the options held: what the end-to-end plan test
    /// cannot isolate, since the knob that changes the plan is in the
    /// options too.
    #[test]
    fn the_dispatch_plan_alone_moves_the_execution_identity() {
        let entry = PathBuf::from("/w/app.hl");
        let sources: BTreeMap<PathBuf, String> = [(entry.clone(), "fn main() { }\n".to_string())].into();
        let fp = options_fingerprint(&base());
        let digest = |plan| crate::shared::options::exec_digest(&sources, &entry, &fp, plan);
        assert_eq!(digest(1), digest(1));
        assert_ne!(digest(1), digest(2));
    }

    /// What only narrates a build, times it, chooses its warnings or its
    /// linker, or says where the cache is leaves the binary's identity
    /// alone; so does an LTO of `off`, which is a build without LTO.
    #[test]
    fn a_knob_that_does_not_change_the_binary_leaves_the_identity() {
        let plain = options_fingerprint(&base());
        let mut o = base();
        o.dump_ir_beside_output = true;
        o.dump_ir = Some(PathBuf::from("/x.ll"));
        o.di_trace = true;
        o.dispatch_trace = true;
        o.time_phases = true;
        o.cc_warnings = true;
        o.no_lld = true;
        o.lto = Some(LtoMode::Off);
        o.cache_dir = PathBuf::from("/somewhere/else");
        assert_eq!(options_fingerprint(&o), plain);
    }
}
