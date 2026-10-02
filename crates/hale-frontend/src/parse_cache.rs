//! Parse reuse (F.40 phase 3, X1): a file's parse product, kept per path
//! and text, so a load that reads a text it has parsed before parses it
//! no second time.
//!
//! What the cache holds is the parser's product and nothing after it:
//! the program [`hale_syntax::parse_source_at_in`] returns for the text
//! at base 0 (unshifted), or its diagnostics, before the load merges,
//! mangles, shapes (the wasm entry wrap, an environment's constitutions,
//! the desugar sequence) or mints anything. Each load recreates the rest
//! from it: the file's base in its own source map, the product moved to
//! that base ([`hale_syntax::shift::shift_program`]), then the shaping
//! and the mint the snapshot runs over its own copy. A shaped or minted
//! program is never kept: shaping reads the whole load and its config,
//! and identities are snapshot-local (two snapshots share no result), so
//! neither is a fact about one file's text. The snapshot's key, which
//! names the whole load, is not a key for one member either.
//!
//! An entry is keyed by the file's path and its exact text, and by the
//! effect-class table the parse starts from: a load parses every file
//! through its one table (#345), interning the classes the file names,
//! so the table the parse starts from is an input of its product, and
//! the table it leaves is an output kept beside it. A hit is the same
//! text, byte for byte, parsed from an equal table; its product is the
//! one a fresh parse would give, which the snapshot test pins
//! (`a_reused_parse_is_the_parse`). Nothing else invalidates an entry —
//! the product is a function of those three inputs — and a path keeps
//! only its [`PER_PATH`] most recently used entries, so an edited file's
//! old texts fall out as it is typed.
//!
//! Reuse is a provider's choice ([`crate::source::SourceProvider::parses`]):
//! the editor's provider carries the language server's cache across its
//! loads; the disk's carries none, so a one-shot verb pays nothing for
//! a cache it would never hit.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use hale_syntax::ast::{EffectClasses, Program};
use hale_syntax::Diag;

/// How many entries a path keeps: the editor's last texts of a file
/// and, for an imported library, the two parses a load makes of it
/// (through a table of its own, then through the load's).
pub const PER_PATH: usize = 4;

/// One file's parse, as the parser left it.
struct Entry {
    text: String,
    /// The effect-class table the parse started from.
    classes_in: EffectClasses,
    /// The table it left.
    classes_out: EffectClasses,
    /// At base 0.
    product: Result<Program, Vec<Diag>>,
}

/// The parse products a process keeps, per path, most recently used
/// first; and how often a parse was reused or made.
pub struct ParseCache {
    entries: Mutex<BTreeMap<PathBuf, Vec<Entry>>>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl Default for ParseCache {
    fn default() -> Self {
        ParseCache::new()
    }
}

impl ParseCache {
    pub const fn new() -> Self {
        ParseCache { entries: Mutex::new(BTreeMap::new()), hits: AtomicU64::new(0), misses: AtomicU64::new(0) }
    }

    /// How many parses this cache answered from an entry.
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    /// How many parses it had to make.
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }

    /// [`hale_syntax::parse_source_at_in`] of `source`, the text of
    /// `path`, at `base` through `classes`: the kept product moved to
    /// `base` and the table it left, when this cache holds the text
    /// parsed from an equal table; otherwise a fresh parse at base 0,
    /// kept, then moved.
    pub fn parse_at(
        &self,
        path: &Path,
        source: &str,
        base: u32,
        classes: &mut EffectClasses,
    ) -> Result<Program, Vec<Diag>> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let kept = entries.entry(path.to_path_buf()).or_default();
        let found = kept.iter().position(|e| e.text == source && e.classes_in == *classes);
        let entry = match found {
            Some(i) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                let entry = kept.remove(i);
                kept.insert(0, entry);
                &kept[0]
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                let classes_in = classes.clone();
                let product = hale_syntax::parse_source_at_in(source, 0, classes);
                kept.insert(0, Entry { text: source.to_string(), classes_in, classes_out: classes.clone(), product });
                kept.truncate(PER_PATH);
                &kept[0]
            }
        };
        *classes = entry.classes_out.clone();
        match &entry.product {
            Ok(program) => {
                let mut program = program.clone();
                hale_syntax::shift::shift_program(&mut program, base);
                Ok(program)
            }
            Err(diags) => Err(diags.iter().map(|d| d.clone().shifted(base)).collect()),
        }
    }
}

/// Parse one file of a load as `src` reads it: through `src`'s cache
/// when it carries one, else directly — the same product either way.
pub(crate) fn parse_file(
    src: &dyn crate::source::SourceProvider,
    path: &Path,
    source: &str,
    base: u32,
    classes: &mut EffectClasses,
) -> Result<Program, Vec<Diag>> {
    match src.parses() {
        Some(cache) => cache.parse_at(path, source, base, classes),
        None => hale_syntax::parse_source_at_in(source, base, classes),
    }
}
