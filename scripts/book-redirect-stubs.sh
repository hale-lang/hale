#!/usr/bin/env bash
# Replace the built book's pages with redirects to the docs site.
#
# The book is rendered twice: mdBook builds it here (docs/book), and
# hale-lang.org syncs docs/src into its own site, where the sidebar
# follows SUMMARY.md and the examples are captured against the
# released toolchain. The site is the one readers are pointed at; the
# GitHub Pages copy stays only for what it alone serves (/play, the
# playground, and /api, the model's rustdoc). So every book page the
# Pages copy would publish becomes a stub that sends the reader to the
# same page on the site, and old deep links keep working.
#
# Usage: scripts/book-redirect-stubs.sh <built-book-dir> [site-docs-url]
set -euo pipefail
book=${1:?usage: book-redirect-stubs.sh <built-book-dir> [site-docs-url]}
site=${2:-https://hale-lang.org/docs}
site=${site%/}

stub() {
    local url=$1
    cat <<HTML
<!doctype html>
<meta charset="utf-8">
<title>Moved to hale-lang.org</title>
<link rel="canonical" href="$url">
<meta http-equiv="refresh" content="0; url=$url">
<p>The book lives at <a href="$url">$url</a>.</p>
HTML
}

count=0
while IFS= read -r -d '' page; do
    rel=${page#"$book"/}
    case "$rel" in
        play/*|api/*) continue ;;              # served here and nowhere else
    esac
    path=${rel%.html}
    case "$path" in
        index|introduction|print|404|toc) path="" ;;
        */index) path=${path%/index} ;;
    esac
    url="$site/${path}"
    url=${url%/}/
    stub "$url" > "$page"
    count=$((count + 1))
done < <(find "$book" -name '*.html' -not -path "$book/play/*" -not -path "$book/api/*" -print0)

stub "$site/" > "$book/404.html"
echo "book-redirect-stubs: $count page(s) now redirect to $site/"
