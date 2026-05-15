//! m96 — Aperio std::ts substrate. Tree-sitter wrapper exposing
//! handle-based `extern "C"` entry points the codegen-emitted
//! binary links against. Mirrors the lotus_arena.c / lotus_tcp_*
//! shape (extern "C" symbols, lazy global state) but lives in
//! Rust because tree-sitter's primary surface is a Rust crate
//! and the per-language grammar packages (tree-sitter-go, etc.)
//! are distributed there.
//!
//! Compiled as a staticlib via the sibling `aperio-ts-shim`
//! workspace crate — cargo produces `libaperio_ts_shim.a` at the
//! workspace target dir; codegen's link step picks it up and
//! passes it to clang alongside the user program's object file.
//!
//! Handle model. Trees and nodes are addressed by 1-based int64
//! handles into per-kind `Mutex<Vec<Option<...>>>` slot tables.
//! Zero is the universal sentinel ("absent / parse failed /
//! out-of-bounds child"). Memory grows for the program's
//! lifetime; m96 v0 doesn't expose a free path. Acceptable for
//! short-lived parse-once workloads (parse N source files, walk
//! once, emit output; process exits afterward).
//!
//! Strings returned across the FFI boundary are allocated in the
//! lazy global payload arena (`lotus_bus_payload_arena_alloc`,
//! defined in lotus_arena.c) so they survive the call-site for
//! the program's lifetime — same lifetime mechanism as
//! `lotus_fs_read_file`'s String return.

use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;

use tree_sitter::{Node, Parser, Tree};

extern "C" {
    /// Defined in lotus_arena.c. Returns memory whose lifetime is
    /// the program; never freed by user code.
    fn lotus_bus_payload_arena_alloc(size: usize, align: usize) -> *mut c_void;
}

/// Copy `s` plus a NUL terminator into the lazy global payload
/// arena. Returns the resulting C-string pointer (`*const c_char`)
/// or `null` on alloc failure.
fn alloc_cstring(s: &str) -> *const c_char {
    let bytes = s.as_bytes();
    unsafe {
        let p = lotus_bus_payload_arena_alloc(bytes.len() + 1, 1) as *mut u8;
        if p.is_null() {
            return std::ptr::null();
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        *p.add(bytes.len()) = 0;
        p as *const c_char
    }
}

/// One parsed Tree, plus the source bytes it was parsed from.
/// Owning the source keeps `node_text` slicing safe — tree-sitter
/// nodes carry byte ranges into the original input, so the
/// originating bytes must outlive every Node access.
struct TreeSlot {
    tree: Tree,
    src: String,
}

/// Stable handle for a tree-sitter Node. The Node value itself
/// isn't stored — instead we record a path of `(child_index,
/// is_named)` steps from the tree's root and re-derive the Node
/// per access. This sidesteps the lifetime-laundering trick that
/// would be needed to store `Node<'tree>` in a `'static` slot
/// table; the cost is one walk per access, which is negligible
/// for the visualization workloads m96 enables.
#[derive(Clone)]
struct NodeRef {
    tree_id: u32,
    /// Each step: child index + whether it was a `.named_child(i)`
    /// (true) or `.child(i)` (false) traversal. Empty path = root.
    path: Vec<(u32, bool)>,
}

static TREES: Mutex<Vec<Option<TreeSlot>>> = Mutex::new(Vec::new());
static NODES: Mutex<Vec<Option<NodeRef>>> = Mutex::new(Vec::new());

fn push_tree(slot: TreeSlot) -> i64 {
    let mut t = TREES.lock().unwrap();
    t.push(Some(slot));
    t.len() as i64
}

fn push_node(n: NodeRef) -> i64 {
    let mut g = NODES.lock().unwrap();
    g.push(Some(n));
    g.len() as i64
}

fn get_node(node_id: i64) -> Option<NodeRef> {
    if node_id <= 0 {
        return None;
    }
    let g = NODES.lock().unwrap();
    let idx = (node_id as usize).checked_sub(1)?;
    g.get(idx).and_then(|s| s.clone())
}

/// Walk a `NodeRef`'s path from root and yield the corresponding
/// `tree_sitter::Node`. Returns `None` if any step misses (path
/// drifted out of bounds, or the tree handle is invalid). Calls
/// `f` with the resolved node while holding the trees lock so
/// the borrow stays valid.
fn with_node<R>(
    node_id: i64,
    f: impl FnOnce(&Node, &TreeSlot) -> R,
) -> Option<R> {
    let nref = get_node(node_id)?;
    let trees = TREES.lock().unwrap();
    let idx = (nref.tree_id as usize).checked_sub(1)?;
    let slot = trees.get(idx)?.as_ref()?;
    let mut cur = slot.tree.root_node();
    for (i, named) in &nref.path {
        let next = if *named {
            cur.named_child(*i as usize)
        } else {
            cur.child(*i as usize)
        };
        cur = next?;
    }
    Some(f(&cur, slot))
}

/// Parse a Go source string. Returns a tree handle (>= 1) on
/// success or 0 on language-init / parse failure / NUL-pointer
/// input. The handle is valid for the rest of the program — m96
/// v0 doesn't free trees.
#[no_mangle]
pub extern "C" fn lotus_ts_parse_go(src: *const c_char) -> i64 {
    if src.is_null() {
        return 0;
    }
    let cstr = unsafe { CStr::from_ptr(src) };
    let s = match cstr.to_str() {
        Ok(s) => s,
        Err(_) => return 0,
    };
    let mut parser = Parser::new();
    let lang: tree_sitter::Language = tree_sitter_go::language();
    if parser.set_language(&lang).is_err() {
        return 0;
    }
    let tree = match parser.parse(s.as_bytes(), None) {
        Some(t) => t,
        None => return 0,
    };
    let slot = TreeSlot {
        tree,
        src: s.to_string(),
    };
    push_tree(slot)
}

/// Return the root-node handle for `tree_id` (>= 1) or 0 if the
/// tree handle is invalid.
#[no_mangle]
pub extern "C" fn lotus_ts_root_node(tree_id: i64) -> i64 {
    if tree_id <= 0 {
        return 0;
    }
    let trees = TREES.lock().unwrap();
    let idx = match (tree_id as usize).checked_sub(1) {
        Some(i) => i,
        None => return 0,
    };
    if trees.get(idx).and_then(|s| s.as_ref()).is_none() {
        return 0;
    }
    drop(trees);
    push_node(NodeRef {
        tree_id: tree_id as u32,
        path: Vec::new(),
    })
}

/// Return the node's grammar kind (e.g. "function_declaration")
/// as a NUL-terminated C string allocated in the lazy global
/// payload arena. Returns an empty string ("") on invalid handle.
#[no_mangle]
pub extern "C" fn lotus_ts_node_kind(node_id: i64) -> *const c_char {
    let kind = with_node(node_id, |n, _| n.kind().to_string());
    alloc_cstring(kind.as_deref().unwrap_or(""))
}

/// Total child count (named + anonymous). Returns -1 on invalid.
#[no_mangle]
pub extern "C" fn lotus_ts_node_child_count(node_id: i64) -> i64 {
    with_node(node_id, |n, _| n.child_count() as i64).unwrap_or(-1)
}

/// Named-child count (skips anonymous tokens like punctuation).
/// Returns -1 on invalid.
#[no_mangle]
pub extern "C" fn lotus_ts_node_named_child_count(node_id: i64) -> i64 {
    with_node(node_id, |n, _| n.named_child_count() as i64).unwrap_or(-1)
}

/// Get the `idx`'th child handle. Returns 0 if out of bounds or
/// handle invalid. Counts include anonymous nodes; pair with
/// `lotus_ts_node_child_count`.
#[no_mangle]
pub extern "C" fn lotus_ts_node_child(node_id: i64, idx: i64) -> i64 {
    if idx < 0 {
        return 0;
    }
    let nref = match get_node(node_id) {
        Some(r) => r,
        None => return 0,
    };
    // Validate the child exists by walking — keeps invariants
    // (handed-out handles always resolve) honest.
    let exists = with_node(node_id, |n, _| n.child(idx as usize).is_some())
        .unwrap_or(false);
    if !exists {
        return 0;
    }
    let mut path = nref.path;
    path.push((idx as u32, false));
    push_node(NodeRef {
        tree_id: nref.tree_id,
        path,
    })
}

/// Get the `idx`'th named-child handle. Returns 0 on out-of-
/// bounds or invalid handle.
#[no_mangle]
pub extern "C" fn lotus_ts_node_named_child(node_id: i64, idx: i64) -> i64 {
    if idx < 0 {
        return 0;
    }
    let nref = match get_node(node_id) {
        Some(r) => r,
        None => return 0,
    };
    let exists =
        with_node(node_id, |n, _| n.named_child(idx as usize).is_some())
            .unwrap_or(false);
    if !exists {
        return 0;
    }
    let mut path = nref.path;
    path.push((idx as u32, true));
    push_node(NodeRef {
        tree_id: nref.tree_id,
        path,
    })
}

/// Start byte (inclusive) of the node's source range. Returns -1
/// on invalid handle.
#[no_mangle]
pub extern "C" fn lotus_ts_node_start_byte(node_id: i64) -> i64 {
    with_node(node_id, |n, _| n.start_byte() as i64).unwrap_or(-1)
}

/// End byte (exclusive) of the node's source range. Returns -1 on
/// invalid handle.
#[no_mangle]
pub extern "C" fn lotus_ts_node_end_byte(node_id: i64) -> i64 {
    with_node(node_id, |n, _| n.end_byte() as i64).unwrap_or(-1)
}

/// Source slice spanning the node, allocated in the lazy global
/// payload arena and NUL-terminated. Empty string on invalid
/// handle or zero-width node.
#[no_mangle]
pub extern "C" fn lotus_ts_node_text(node_id: i64) -> *const c_char {
    let text = with_node(node_id, |n, slot| {
        let lo = n.start_byte();
        let hi = n.end_byte().min(slot.src.len());
        if lo > hi {
            return String::new();
        }
        // Byte slice — tree-sitter ranges are byte-precise. Use
        // `from_utf8_lossy` to defang any mid-multibyte slicing if
        // a grammar ever produces such a range; in practice for Go
        // this won't fire.
        let bytes = &slot.src.as_bytes()[lo..hi];
        String::from_utf8_lossy(bytes).into_owned()
    });
    alloc_cstring(text.as_deref().unwrap_or(""))
}

/// Is the node an "extra" / error / missing node? Returns 0
/// (false) on invalid handle. Useful for filtering noise out of
/// import-graph extraction.
#[no_mangle]
pub extern "C" fn lotus_ts_node_is_named(node_id: i64) -> i64 {
    with_node(node_id, |n, _| if n.is_named() { 1 } else { 0 }).unwrap_or(0)
}
