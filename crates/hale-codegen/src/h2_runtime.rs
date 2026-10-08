//! nghttp2 as part of the runtime, for a program that serves HTTP/2.
//!
//! The library (MIT, vendored whole under `runtime/third_party/nghttp2`) is
//! plain C with no thread, socket or blocking call of its own: the runtime's
//! `lotus_h2.c` drives it as bytes in, bytes out and a queue of events. It
//! is compiled by the same cc step as every other runtime translation unit,
//! with the same flags (the sanitizers' included), each file once per flag
//! set into the content-addressed object cache, and linked only into a
//! program that reaches `lotus_h2_*`, as the tree-sitter shim is. A cross
//! build compiles the same files with the target's `zig cc`, so a release
//! asset links the library statically by construction.
//!
//! The files cannot be one translation unit (their `static` helpers share
//! names) and are embedded as text, so the headers they include are staged
//! into the cache directory once, under a name that is their content's hash,
//! and the compile is given `-I` there.

use crate::codegen::{compile_cached_runtime_object_with, BuildOptions, CodegenError};
use std::path::PathBuf;

/// The glue: the whole C surface `std::io::h2` lowers to.
const GLUE: &str = include_str!("../runtime/lotus_h2.c");

macro_rules! vendored {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_str!(concat!("../runtime/third_party/nghttp2/", $name)))),*]
    };
}

/// The library's translation units.
const SOURCES: &[(&str, &str)] = vendored![
    "nghttp2_alpn.c",
    "nghttp2_buf.c",
    "nghttp2_callbacks.c",
    "nghttp2_debug.c",
    "nghttp2_extpri.c",
    "nghttp2_frame.c",
    "nghttp2_hd.c",
    "nghttp2_hd_huffman.c",
    "nghttp2_hd_huffman_data.c",
    "nghttp2_helper.c",
    "nghttp2_http.c",
    "nghttp2_map.c",
    "nghttp2_mem.c",
    "nghttp2_option.c",
    "nghttp2_outbound_item.c",
    "nghttp2_pq.c",
    "nghttp2_priority_spec.c",
    "nghttp2_queue.c",
    "nghttp2_ratelim.c",
    "nghttp2_rcbuf.c",
    "nghttp2_session.c",
    "nghttp2_stream.c",
    "nghttp2_submit.c",
    "nghttp2_time.c",
    "nghttp2_version.c",
    "sfparse.c",
];

/// The headers they include.
const HEADERS: &[(&str, &str)] = vendored![
    "nghttp2_alpn.h",
    "nghttp2_buf.h",
    "nghttp2_callbacks.h",
    "nghttp2_debug.h",
    "nghttp2_extpri.h",
    "nghttp2_frame.h",
    "nghttp2_hd.h",
    "nghttp2_hd_huffman.h",
    "nghttp2_helper.h",
    "nghttp2_http.h",
    "nghttp2_int.h",
    "nghttp2_map.h",
    "nghttp2_mem.h",
    "nghttp2_net.h",
    "nghttp2_option.h",
    "nghttp2_outbound_item.h",
    "nghttp2_pq.h",
    "nghttp2_priority_spec.h",
    "nghttp2_queue.h",
    "nghttp2_ratelim.h",
    "nghttp2_rcbuf.h",
    "nghttp2_session.h",
    "nghttp2_stream.h",
    "nghttp2_submit.h",
    "nghttp2_time.h",
    "sfparse.h",
    "nghttp2/nghttp2.h",
    "nghttp2/nghttp2ver.h",
];

/// What the library is told of the platform in place of a generated
/// `config.h`: the POSIX headers and the monotonic clock, which every target
/// the runtime builds for has.
const DEFINES: &[&str] = &[
    "-DNGHTTP2_STATICLIB",
    "-DHAVE_ARPA_INET_H=1",
    "-DHAVE_NETINET_IN_H=1",
    "-DHAVE_CLOCK_GETTIME=1",
    "-DHAVE_DECL_CLOCK_MONOTONIC=1",
];

/// Whether this C symbol is the h2 surface's, so the link needs the objects.
pub(crate) fn is_h2_symbol(name: &str) -> bool {
    name.starts_with("lotus_h2_")
}

/// Whether `dir` holds every header, byte for byte.
fn staged(dir: &std::path::Path) -> bool {
    HEADERS.iter().all(|(name, text)| {
        std::fs::read_to_string(dir.join(name))
            .map(|t| t == *text)
            .unwrap_or(false)
    })
}

/// How many generations of the include directory are tried before the
/// staging gives up: the canonical name and seven beside it.
const GENERATIONS: u32 = 8;

/// The directory the headers are read from: `canonical` when it holds them
/// all, else the first generation beside it (`<canonical>-g1`, `-g2`, …)
/// that does, staged here if none did.
///
/// A complete directory is never moved or removed, by anyone: a build that
/// was handed one keeps reading it. So a directory that is there but
/// incomplete (a cache restored from a run whose staging was cut short, a
/// staging killed midway) is not repaired in place; it is left, and the
/// next generation's name is used. Every staging writes into a temporary
/// directory of the call's own and renames it into place, so the name only
/// ever appears complete, and two callers landing the same generation at
/// once both find it complete whichever rename won.
fn stage_headers(canonical: &std::path::Path) -> Result<PathBuf, CodegenError> {
    static STAGINGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let io = |what: &str, e: std::io::Error| {
        CodegenError::Link(format!("stage the nghttp2 headers ({what}): {e}"))
    };
    for generation in 0..GENERATIONS {
        let dir = if generation == 0 {
            canonical.to_path_buf()
        } else {
            canonical.with_file_name(format!(
                "{}-g{generation}",
                canonical
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("lotus-rt-h2-inc")
            ))
        };
        if staged(&dir) {
            return Ok(dir);
        }
        if dir.exists() {
            // there and incomplete: nobody's to move, so the next name
            continue;
        }
        let n = STAGINGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = dir.with_extension(format!("tmp{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("nghttp2")).map_err(|e| io("mkdir", e))?;
        for (name, text) in HEADERS {
            std::fs::write(tmp.join(name), text).map_err(|e| io(name, e))?;
        }
        // a concurrent staging may have landed the name first: it holds the
        // same bytes
        if std::fs::rename(&tmp, &dir).is_err() {
            let _ = std::fs::remove_dir_all(&tmp);
        }
        if staged(&dir) {
            return Ok(dir);
        }
    }
    Err(CodegenError::Link(
        "the nghttp2 headers were not staged".into(),
    ))
}

/// The library's objects and the glue's, compiled (or taken from the
/// cache) with `cflags`, in parallel: a cold cache compiles twenty-seven
/// files, once.
pub(crate) fn h2_objects(
    options: &BuildOptions,
    cc: &[String],
    cc_version: &str,
    cflags: &[String],
) -> Result<Vec<PathBuf>, CodegenError> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (name, text) in HEADERS {
        name.hash(&mut h);
        text.hash(&mut h);
    }
    let dir = options
        .cache_dir
        .join(format!("lotus-rt-h2-inc-{:016x}", h.finish()));
    let _ = std::fs::create_dir_all(&options.cache_dir);
    let include = stage_headers(&dir)?;
    let mut flags: Vec<String> = cflags.to_vec();
    flags.push(format!("-I{}", include.display()));
    flags.extend(DEFINES.iter().map(|d| d.to_string()));
    let mut units: Vec<(&str, &str)> = SOURCES.to_vec();
    units.push(("glue", GLUE));
    let results: Vec<Result<PathBuf, CodegenError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = units
            .iter()
            .map(|(name, source)| {
                let flags = &flags;
                let stem = format!("h2-{}", name.trim_end_matches(".c"));
                scope.spawn(move || {
                    compile_cached_runtime_object_with(
                        options, cc, cc_version, source, &stem, flags,
                    )
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|t| {
                t.join().unwrap_or_else(|_| {
                    Err(CodegenError::Link("an nghttp2 compile panicked".into()))
                })
            })
            .collect()
    });
    results.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two builds in one process stage the headers at once: each must find
    /// them complete, whichever rename won. The staging directory used to be
    /// named by the process alone, so the second call removed the first's
    /// files before its own rename.
    #[test]
    fn concurrent_staging_in_one_process_leaves_every_header() {
        let root = std::env::temp_dir().join(format!("hale-h2-stage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for round in 0..40 {
            let dir = root.join(format!("inc-{round}"));
            let threads: Vec<_> = (0..8)
                .map(|_| {
                    let dir = dir.clone();
                    std::thread::spawn(move || stage_headers(&dir))
                })
                .collect();
            for t in threads {
                match t.join().unwrap() {
                    Ok(used) => assert_eq!(used, dir, "round {round}: the canonical name"),
                    Err(e) => panic!("round {round}: {e:?}"),
                }
            }
            for (name, text) in HEADERS {
                assert_eq!(
                    std::fs::read_to_string(dir.join(name)).ok().as_deref(),
                    Some(*text),
                    "round {round}: {name}"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A directory that is there but incomplete, as a cache restored from a
    /// cut-short staging leaves it, is neither trusted nor touched: the next
    /// generation's name is staged and handed out, the same one to every
    /// caller, and the incomplete directory keeps its bytes.
    #[test]
    fn an_incomplete_directory_is_left_and_the_next_generation_used() {
        let root =
            std::env::temp_dir().join(format!("hale-h2-stage-broken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let canonical = root.join("inc");
        std::fs::create_dir_all(canonical.join("nghttp2")).unwrap();
        std::fs::write(canonical.join("nghttp2/nghttp2.h"), "not the header").unwrap();
        let used = stage_headers(&canonical).unwrap();
        assert_eq!(used, root.join("inc-g1"));
        assert!(staged(&used));
        assert_eq!(
            std::fs::read_to_string(canonical.join("nghttp2/nghttp2.h")).unwrap(),
            "not the header"
        );
        assert_eq!(
            stage_headers(&canonical).unwrap(),
            used,
            "the same generation again"
        );
        // the first generation poisoned too: the second
        let (name, _) = HEADERS[0];
        std::fs::remove_file(used.join(name)).unwrap();
        let next = stage_headers(&canonical).unwrap();
        assert_eq!(next, root.join("inc-g2"));
        assert!(staged(&next));
        assert!(!staged(&used), "a generation is never repaired in place");
        let leftovers: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "no temporary directory is left: {leftovers:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
