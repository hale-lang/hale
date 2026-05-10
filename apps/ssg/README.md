# ssg

A markdown-to-HTML static site generator written in Aperio.
Reads `.md` files from an input directory, renders each through
`std::text::md_to_html`, wraps the result in a plain HTML shell,
and writes the matching `.html` file to an output directory.
After the per-page pass, it emits an `index.html` linking to
every rendered page.

## What it does

1. Lists the input directory via `std::io::fs::list_dir`.
2. For each entry ending in `.md`, reads the file, renders it
   to HTML, wraps the rendered fragment in
   `<!doctype><html>...<body>...</body></html>`, and writes the
   output as `<stem>.html` in the output directory.
3. Emits an `index.html` listing each rendered page (only when
   at least one page rendered).
4. Prints one line per file: `rendered: <input> -> <output>`.

## How to run

Build:

```
./target/debug/aperio build apps/ssg/main.ap
```

Run with defaults (input `./content`, output `./out`):

```
cd apps/ssg
mkdir -p out
./main
```

Run with explicit paths:

```
mkdir -p /tmp/ssg-out
./apps/ssg/main apps/ssg/content /tmp/ssg-out
```

Sample output:

```
ssg: input=./content output=./out
rendered: ./content/hello.md -> ./out/hello.html
rendered: ./content/about.md -> ./out/about.html
rendered: ./content/notes.md -> ./out/notes.html
rendered: <index> -> ./out/index.html
ssg: 3 rendered, 0 failed
```

The `content/` subdirectory ships three example markdown files
so the default invocation produces a non-empty result.

## Argv

| Position  | Default       | Meaning                       |
|-----------|---------------|-------------------------------|
| `argv[1]` | `./content`   | Input directory of `.md` files |
| `argv[2]` | `./out`       | Output directory for `.html`  |

## What it doesn't do (yet)

- **Does not create the output directory.** The stdlib has no
  `mkdir` surface today (`std::io::fs` ships `read_file`,
  `write_file`, `read_bytes`, `list_dir`, `file_exists`,
  `file_size`). The output directory must already exist;
  `write_file` returns `-1` and the program reports
  `write failed` for every file otherwise. Logged in
  `FRICTION.md`.
- **Block-level markdown only.** `std::text::md_to_html`
  handles ATX headings, paragraphs, fenced code, and HTML
  escape, but not inline `**bold**`, `*italic*`, or links.
  Marked Blocked on `ready-today.md`.
- **No recursive walk.** `list_dir` returns one level of
  entries; nested content directories are not traversed.
- **No HTML escape on filenames.** The index links and the
  `<title>` tag use the file stem verbatim. Filenames with
  HTML metacharacters would corrupt the output.
- **Empty file is indistinguishable from a read error.**
  `read_file` collapses both to `""`, which renders as an
  empty body either way. Logged in `FRICTION.md`.

## Layout

```
apps/ssg/
  main.ap        — the program (top-level free fns + main)
  README.md      — this file
  FRICTION.md    — append-only friction log
  content/       — three sample .md fixtures
  out/           — written by the binary; gitignored if you wish
```
