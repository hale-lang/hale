# ssg friction log

Append-only. Format per `notes/agent-onboarding/app-dev-brief.md`.

## 2026-05-10 no-mkdir

**Tried:** Make the SSG self-bootstrapping by creating the output
directory if it does not exist (`std::io::fs::mkdir(output_dir)`).
**Hit:** No `mkdir` / `create_dir` / `ensure_dir` surface in
`std::io::fs`. The capability matrix lists read / write / size /
exists / read_bytes / list_dir; nothing for directory creation.
**Workaround:** Documented the precondition in the README and
sample-run instructions (`mkdir -p out`) instead of doing it from
the program. `write_file` returns `-1` if the directory is
absent, which the program does report.
**Why it matters:** Almost every file-writing CLI wants to
ensure-or-create the destination directory. Forcing every caller
to shell out via the README is a papercut and makes the program
non-self-contained. Pairs naturally with a future
`std::io::fs::mkdir(path) -> Int` and/or
`write_file_p(path, content)` that creates intermediate dirs.

## 2026-05-10 read_file-empty-vs-error

**Tried:** Distinguish "the input markdown file is genuinely
empty" from "the read failed" so the program can warn rather
than render an empty `<body>`.
**Hit:** `std::io::fs::read_file` returns `""` for both cases —
the docs explicitly say the C-layer `-1` is clamped to `0` /
empty-string at the Aperio surface. There is no errno-style
disambiguation surface today, and `file_size` plus `file_exists`
still cannot tell me "the read I just did failed mid-way."
**Workaround:** None — the program treats empty-and-readable as
"render an empty page," which is the correct behavior for an
intentionally empty file. Errors during read are silently
treated the same way.
**Why it matters:** Minor papercut for an SSG (an empty `.md`
yields an empty `.html`, which is mostly fine) but harder for
programs that need to fail loudly on a partial read. Tied to the
broader "no errno surface" Blocked entry on `ready-today.md`.

## 2026-05-10 list_dir-newline-string

**Tried:** Iterate filenames with a clean
`for name in entries { ... }` shape after
`let entries = std::io::fs::list_dir(input_dir);`.
**Hit:** `list_dir` returns a `String` of newline-separated
names, not a `[String]`. The brief's counter-hallucination table
already flagged that there are no general arrays here yet, but
the cost of the substitute is that every caller hand-rolls a
`while start < total { index_of("\n") ... }` loop, and I had to
do it twice in this program (once for rendering, once for the
index).
**Workaround:** Copied the docs-server `__render_index` walk
shape into both call sites. Did not factor it into a helper
because (a) Aperio fns can't return a `[String]` either, and
(b) returning the same newline-separated `String` from a helper
gains nothing.
**Why it matters:** `ready-today.md` already lists "`[String]`
overload" as the unblock. Reaffirming: every list_dir caller in
the repo (docs-server, ssg, presumably the next one) writes the
same loop. A `List<T>` / split-on-char primitive would let the
helper land naturally.
