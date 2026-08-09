//! The target model.
//!
//! What a build produces is decided by the TARGET, not by the machine
//! doing the building. That sounds obvious; the compiler did not believe
//! it. `CompileTarget` had two variants — `Native` and `Wasm32` — where
//! "native" meant "whatever this host happens to be", and every question
//! that depended on the target's platform was answered by asking Rust's
//! `cfg!(target_os = ...)` about the HOST at the time the compiler itself
//! was compiled. On a Linux box building for Linux the two coincide, so
//! nothing was visibly wrong. They stop coinciding the moment a second
//! platform exists, and then the conflation is not a refactor away from
//! being a bug — it IS the bug, silently emitting host-shaped decisions
//! into a foreign artifact.
//!
//! So: a target is a triple with a parsed identity, and everything that
//! varies by platform — object and executable extensions, which system
//! libraries to link, whether `-Wl,--wrap` exists — is a question you ask
//! the `TargetSpec`. The host appears in exactly one role, discovering
//! the local toolchain, which is the one thing it legitimately decides.
//!
//! This module deliberately describes more than the compiler can build.
//! `x86_64-pc-windows-msvc` parses, names its files, and reports its
//! support tier here, while codegen for it does not exist yet. Being able
//! to name a target precisely is the prerequisite for implementing it,
//! and a target model that only admits what already works cannot be the
//! thing you build the next platform on top of.
//!
//! See GH #445 for the full Windows plan; this is its first step.

use std::fmt;

/// The instruction set an artifact is emitted for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetArch {
    X86_64,
    Aarch64,
    Wasm32,
}

impl TargetArch {
    /// The LLVM target-initialization family this architecture needs.
    /// `initialize_native()` is only correct when the target IS the host;
    /// anything else has to initialize its own architecture explicitly.
    pub fn llvm_name(self) -> &'static str {
        match self {
            TargetArch::X86_64 => "x86-64",
            TargetArch::Aarch64 => "aarch64",
            TargetArch::Wasm32 => "wasm32",
        }
    }
}

/// The operating system, which decides the object format, the system
/// libraries, and most of the linker dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetOs {
    Linux,
    MacOs,
    Windows,
    /// `wasm32-unknown-unknown`: no OS, and no POSIX to speak of.
    None,
}

/// The ABI/CRT flavour. On Windows this is the difference between two
/// incompatible worlds, so it is part of the target's identity rather
/// than a linker detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetEnv {
    Gnu,
    Msvc,
    None,
}

/// How far the compiler can actually take a target today.
///
/// Kept explicit and per-target so that "we cannot build this yet" is a
/// fact the target model states, rather than something a user discovers
/// from a link error three subsystems away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetSupport {
    /// Builds and links an executable.
    Supported,
    /// Emits a relocatable object; linking is a later step.
    ObjectOnly,
    /// Named and described, but no codegen path exists yet.
    Planned,
}

/// The file-naming conventions of a platform.
///
/// These were `.with_extension("wasm")` at one call site and an
/// extension-less path everywhere else — a convention spread across the
/// CLI rather than owned by the target. Windows needs four of these to
/// differ at once, so they live together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetFilenames {
    /// Relocatable object: `.o`, `.obj`, `.wasm`.
    pub object: &'static str,
    /// Executable suffix: empty on unix, `.exe` on Windows.
    pub executable: &'static str,
    /// Static library / import library.
    pub static_lib: &'static str,
    /// Shared library.
    pub dynamic_lib: &'static str,
    /// Separate debug-info file, where the platform has one.
    pub debug_info: Option<&'static str>,
}

/// A fully-identified compilation target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSpec {
    /// The canonical LLVM triple. This is the identity: it is what the
    /// module is stamped with, what the toolchain is asked for, and what
    /// distinguishes one cache entry from another.
    pub triple: String,
    pub arch: TargetArch,
    pub os: TargetOs,
    pub env: TargetEnv,
}

/// Why a `--target` value was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetParseError {
    pub input: String,
}

impl fmt::Display for TargetParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown target `{}`\n  known: {}\n  aliases: native (this host), wasm32",
            self.input,
            TargetSpec::known()
                .iter()
                .map(|t| t.triple.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl std::error::Error for TargetParseError {}

impl TargetSpec {
    fn new(triple: &str, arch: TargetArch, os: TargetOs, env: TargetEnv) -> Self {
        TargetSpec {
            triple: triple.to_string(),
            arch,
            os,
            env,
        }
    }

    /// Every triple the compiler can name. Naming is not building — see
    /// [`TargetSpec::support`].
    pub fn known() -> Vec<TargetSpec> {
        vec![
            Self::new(
                "x86_64-unknown-linux-gnu",
                TargetArch::X86_64,
                TargetOs::Linux,
                TargetEnv::Gnu,
            ),
            Self::new(
                "aarch64-unknown-linux-gnu",
                TargetArch::Aarch64,
                TargetOs::Linux,
                TargetEnv::Gnu,
            ),
            Self::new(
                "x86_64-apple-darwin",
                TargetArch::X86_64,
                TargetOs::MacOs,
                TargetEnv::None,
            ),
            Self::new(
                "aarch64-apple-darwin",
                TargetArch::Aarch64,
                TargetOs::MacOs,
                TargetEnv::None,
            ),
            Self::new(
                "x86_64-pc-windows-msvc",
                TargetArch::X86_64,
                TargetOs::Windows,
                TargetEnv::Msvc,
            ),
            Self::new(
                "aarch64-pc-windows-msvc",
                TargetArch::Aarch64,
                TargetOs::Windows,
                TargetEnv::Msvc,
            ),
            Self::new(
                "wasm32-unknown-unknown",
                TargetArch::Wasm32,
                TargetOs::None,
                TargetEnv::None,
            ),
        ]
    }

    /// The triple of the machine running the compiler.
    ///
    /// This is the ONE place a host `cfg!` is legitimate: it answers "what
    /// am I", not "what am I building". Everything downstream reads the
    /// returned spec.
    pub fn host() -> TargetSpec {
        let arch = if cfg!(target_arch = "aarch64") {
            TargetArch::Aarch64
        } else {
            TargetArch::X86_64
        };
        if cfg!(target_os = "macos") {
            let triple = match arch {
                TargetArch::Aarch64 => "aarch64-apple-darwin",
                _ => "x86_64-apple-darwin",
            };
            Self::new(triple, arch, TargetOs::MacOs, TargetEnv::None)
        } else if cfg!(target_os = "windows") {
            let triple = match arch {
                TargetArch::Aarch64 => "aarch64-pc-windows-msvc",
                _ => "x86_64-pc-windows-msvc",
            };
            Self::new(triple, arch, TargetOs::Windows, TargetEnv::Msvc)
        } else {
            let triple = match arch {
                TargetArch::Aarch64 => "aarch64-unknown-linux-gnu",
                _ => "x86_64-unknown-linux-gnu",
            };
            Self::new(triple, arch, TargetOs::Linux, TargetEnv::Gnu)
        }
    }

    /// Parse a `--target` value: a canonical triple, or one of the two
    /// aliases the CLI has always accepted.
    pub fn parse(value: &str) -> Result<TargetSpec, TargetParseError> {
        match value {
            "native" | "host" => return Ok(Self::host()),
            "wasm32" | "wasm" => {
                return Ok(Self::new(
                    "wasm32-unknown-unknown",
                    TargetArch::Wasm32,
                    TargetOs::None,
                    TargetEnv::None,
                ))
            }
            _ => {}
        }
        Self::known()
            .into_iter()
            .find(|t| t.triple == value)
            .ok_or_else(|| TargetParseError {
                input: value.to_string(),
            })
    }

    pub fn is_windows(&self) -> bool {
        self.os == TargetOs::Windows
    }
    pub fn is_macos(&self) -> bool {
        self.os == TargetOs::MacOs
    }
    pub fn is_linux(&self) -> bool {
        self.os == TargetOs::Linux
    }
    pub fn is_wasm(&self) -> bool {
        self.arch == TargetArch::Wasm32
    }

    /// Whether the target has a POSIX runtime under it. The lotus C
    /// runtime is POSIX-shaped today, which is precisely why Windows is
    /// `Planned` rather than merely unimplemented in the linker.
    pub fn is_posix(&self) -> bool {
        matches!(self.os, TargetOs::Linux | TargetOs::MacOs)
    }

    /// GNU-ld's `-Wl,--wrap`, which the allocator and syscall-count shims
    /// need. macOS's ld64 has no equivalent, and neither does link.exe.
    pub fn supports_wrap_linker_flag(&self) -> bool {
        self.os == TargetOs::Linux
    }

    /// POSIX shared memory lives in librt on Linux and in libc on macOS.
    pub fn needs_librt(&self) -> bool {
        self.os == TargetOs::Linux
    }

    pub fn filenames(&self) -> TargetFilenames {
        match self.os {
            TargetOs::Windows => TargetFilenames {
                object: "obj",
                executable: "exe",
                static_lib: "lib",
                dynamic_lib: "dll",
                debug_info: Some("pdb"),
            },
            TargetOs::MacOs => TargetFilenames {
                object: "o",
                executable: "",
                static_lib: "a",
                dynamic_lib: "dylib",
                debug_info: Some("dSYM"),
            },
            TargetOs::Linux => TargetFilenames {
                object: "o",
                executable: "",
                static_lib: "a",
                dynamic_lib: "so",
                debug_info: None,
            },
            TargetOs::None => TargetFilenames {
                object: "wasm",
                executable: "wasm",
                static_lib: "a",
                dynamic_lib: "wasm",
                debug_info: None,
            },
        }
    }

    pub fn support(&self) -> TargetSupport {
        match self.os {
            TargetOs::Linux | TargetOs::MacOs => TargetSupport::Supported,
            TargetOs::None => TargetSupport::ObjectOnly,
            TargetOs::Windows => TargetSupport::Planned,
        }
    }

    /// A one-line human description, used by `--target ... --describe-target`
    /// and by the error raised when a target parses but cannot be built.
    pub fn describe(&self) -> String {
        let f = self.filenames();
        let support = match self.support() {
            TargetSupport::Supported => "supported: builds and links",
            TargetSupport::ObjectOnly => "object-only: emits a relocatable object, no link",
            TargetSupport::Planned => "planned: named and described, no codegen yet (GH #445)",
        };
        format!(
            "{}\n  arch: {}   os: {:?}   env: {:?}\n  object: .{}   executable: {}   \
             static: .{}   dynamic: .{}\n  {}",
            self.triple,
            self.arch.llvm_name(),
            self.os,
            self.env,
            f.object,
            if f.executable.is_empty() {
                "(none)".to_string()
            } else {
                format!(".{}", f.executable)
            },
            f.static_lib,
            f.dynamic_lib,
            support,
        )
    }
}

impl Default for TargetSpec {
    fn default() -> Self {
        Self::host()
    }
}

impl fmt::Display for TargetSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.triple)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_alias_is_the_host_triple() {
        assert_eq!(TargetSpec::parse("native").unwrap(), TargetSpec::host());
        assert_eq!(TargetSpec::parse("host").unwrap(), TargetSpec::host());
    }

    #[test]
    fn wasm_alias_keeps_working() {
        for alias in ["wasm", "wasm32", "wasm32-unknown-unknown"] {
            let t = TargetSpec::parse(alias).unwrap();
            assert!(t.is_wasm(), "{alias}");
            assert_eq!(t.filenames().object, "wasm");
            assert_eq!(t.support(), TargetSupport::ObjectOnly);
        }
    }

    /// The exit criterion for GH #445 PR 1: the Windows triple can be
    /// parsed and described, without any Windows codegen existing.
    #[test]
    fn windows_triple_parses_and_describes() {
        let t = TargetSpec::parse("x86_64-pc-windows-msvc").unwrap();
        assert!(t.is_windows());
        assert!(!t.is_posix());
        assert_eq!(t.env, TargetEnv::Msvc);
        assert_eq!(t.support(), TargetSupport::Planned);

        let f = t.filenames();
        assert_eq!(f.object, "obj");
        assert_eq!(f.executable, "exe");
        assert_eq!(f.static_lib, "lib");
        assert_eq!(f.dynamic_lib, "dll");
        assert_eq!(f.debug_info, Some("pdb"));

        let d = t.describe();
        assert!(d.contains("x86_64-pc-windows-msvc"), "{d}");
        assert!(d.contains("planned"), "{d}");
    }

    /// Windows must not inherit the POSIX linker decisions the compiler
    /// used to make by asking the host.
    #[test]
    fn windows_claims_no_posix_linker_behaviour() {
        let w = TargetSpec::parse("x86_64-pc-windows-msvc").unwrap();
        assert!(!w.supports_wrap_linker_flag());
        assert!(!w.needs_librt());
    }

    #[test]
    fn platform_predicates_match_the_old_host_cfg() {
        let linux = TargetSpec::parse("x86_64-unknown-linux-gnu").unwrap();
        let mac = TargetSpec::parse("aarch64-apple-darwin").unwrap();

        // `-Wl,--wrap` and `-lrt` were both spelled `!cfg!(target_os = "macos")`.
        assert!(linux.supports_wrap_linker_flag() && linux.needs_librt());
        assert!(!mac.supports_wrap_linker_flag() && !mac.needs_librt());

        assert_eq!(linux.filenames().executable, "");
        assert_eq!(mac.filenames().dynamic_lib, "dylib");
        assert_eq!(linux.filenames().dynamic_lib, "so");
    }

    #[test]
    fn unknown_target_names_the_alternatives() {
        let e = TargetSpec::parse("x86_64-pc-windows-gnu").unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("unknown target"), "{msg}");
        assert!(msg.contains("x86_64-pc-windows-msvc"), "{msg}");
        assert!(msg.contains("native"), "{msg}");
    }

    #[test]
    fn every_known_triple_round_trips() {
        for t in TargetSpec::known() {
            assert_eq!(TargetSpec::parse(&t.triple).unwrap(), t, "{}", t.triple);
            assert!(!t.describe().is_empty());
        }
    }

    #[test]
    fn host_is_a_known_triple() {
        let h = TargetSpec::host();
        assert!(
            TargetSpec::known().iter().any(|t| t.triple == h.triple),
            "host triple {} is not in the known list",
            h.triple
        );
    }
}
