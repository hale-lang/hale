#!/usr/bin/env bash
# Assemble the target sysroot a cross build links against (GH #970).
#
# `hale build --target <linux triple>` from another host compiles the
# lotus runtime and links with `zig cc`, which brings its own libc for
# every Linux target. What zig does not bring is what the runtime needs
# beyond libc: OpenSSL (lotus_tls.c) and zlib (lotus_compress.c), and
# the tree-sitter shim (`std::ts`). This script builds those for one
# target into the directory codegen looks in:
#
#   ${XDG_CACHE_HOME:-~/.cache}/hale/sysroot/<triple>/
#       include/openssl/*.h  include/zlib.h  include/zconf.h
#       lib/libssl.a  lib/libcrypto.a  lib/libz.a
#       lib/libhale_ts_shim.a            (when built from a checkout)
#       bin/cc                           (zig wrapper for this target)
#       VERSIONS                         (what went in)
#
# OpenSSL and zlib are built FROM SOURCE, with zig, against the same
# glibc the compiler pins for the target. A distribution's archives
# cannot do that: Ubuntu 24.04's libcrypto.a is compiled against glibc
# 2.39 and reaches for `__isoc23_strtol`, which the 2.31 floor does not
# have — so a binary linked against it either fails to link or has to
# ask for a glibc newer than the machines it is meant for. Built here,
# the archives ask for exactly the pinned floor, and the emitted binary
# depends on the target's glibc and nothing else. Static, because a Mac
# cannot install a Linux libssl, and a program built here has to run on
# a machine that never saw this sysroot.
#
# The ts-shim is a Rust staticlib; run from a checkout with the target's
# Rust std installed (`rustup target add <triple>`) it is cross-built
# here too, with zig as the C compiler for tree-sitter.
#
#   scripts/target-sysroot.sh aarch64-unknown-linux-gnu
#   scripts/target-sysroot.sh x86_64-unknown-linux-gnu
#
# Needs: zig (brew install zig), curl, perl and make (OpenSSL's build).
# `HALE_TARGET_SYSROOT=<dir>` names another output directory, and is
# what codegen reads to find one somewhere else. `HALE_TARGET_GLIBC`
# moves the glibc floor; codegen reads the same variable, so the two
# always agree.
set -euo pipefail

# The pins. Both hashes were taken from the release tarballs on
# 2026-09-21; bump the version and the hash together.
OPENSSL_VERSION=3.5.8
OPENSSL_SHA256=a8f84a39918ec6415ce765d9b429d313ba97b8143169c172e734b9514464f5b2
ZLIB_VERSION=1.3.2
ZLIB_SHA256=bb329a0a2cd0274d05519d61c667c062e06990d72e125ee2dfa8de64f0119d16

triple=${1:-}
case "$triple" in
  aarch64-unknown-linux-gnu) zig_target=aarch64-linux-gnu; openssl_target=linux-aarch64 ;;
  x86_64-unknown-linux-gnu)  zig_target=x86_64-linux-gnu;  openssl_target=linux-x86_64 ;;
  "") echo "usage: $0 <aarch64-unknown-linux-gnu|x86_64-unknown-linux-gnu>" >&2; exit 2 ;;
  *)  echo "$0: no sysroot recipe for \`$triple\` (Linux gnu targets only)" >&2; exit 2 ;;
esac

# The glibc the emitted binary asks for. Old enough for the LTS
# distributions in service (Debian 11, Ubuntu 20.04; RHEL 8's 2.28 is
# the one it excludes). Codegen pins the same version.
glibc=${HALE_TARGET_GLIBC:-2.31}

for tool in zig curl perl make; do
  command -v "$tool" >/dev/null || { echo "$0: \`$tool\` is needed (zig: brew install zig / https://ziglang.org/download)" >&2; exit 1; }
done

out=${HALE_TARGET_SYSROOT:-${XDG_CACHE_HOME:-$HOME/.cache}/hale/sysroot/$triple}
mkdir -p "$out/include" "$out/lib" "$out/bin"
work=$(mktemp -d "${TMPDIR:-/tmp}/hale-sysroot-XXXXXX")
trap 'rm -rf "$work"' EXIT

# A C compiler for this target, for anything that takes `CC`: the
# `cc` crate adds a rustc-style `--target=<triple>` that zig does not
# parse, so the wrapper drops it and pins zig's own target.
cat > "$out/bin/cc" <<EOF
#!/bin/sh
# zig as the C compiler for $triple (scripts/target-sysroot.sh)
# Rotate the arguments through "\$@" so each survives verbatim — an
# \`eval\` would re-parse OpenSSL's -DOPENSSLDIR="..." and lose it.
n=\$#
while [ "\$n" -gt 0 ]; do
  a=\$1; shift; n=\$((n - 1))
  case "\$a" in --target=*) ;; *) set -- "\$@" "\$a" ;; esac
done
exec zig cc -target $zig_target.$glibc "\$@"
EOF
chmod +x "$out/bin/cc"
export CC="$out/bin/cc" AR="zig ar" RANLIB="zig ranlib"

fetch() { # url sha256 -> file in $work
  local url=$1 sha=$2 file="$work/${1##*/}"
  echo "fetching ${1##*/}" >&2
  curl -fsSL "$url" -o "$file"
  echo "$sha  $file" | shasum -a 256 -c - >/dev/null \
    || { echo "$0: checksum mismatch for ${1##*/}" >&2; exit 1; }
  echo "$file"
}

# zlib: fifteen C files and two headers. Its `configure` would detect
# the HOST (and reach for Apple's libtool on a Mac), so it is compiled
# directly, with the two feature macros configure would have written
# into zconf.h for a Linux target.
zlib_tgz=$(fetch "https://github.com/madler/zlib/releases/download/v$ZLIB_VERSION/zlib-$ZLIB_VERSION.tar.gz" "$ZLIB_SHA256")
tar xzf "$zlib_tgz" -C "$work"
(
  cd "$work/zlib-$ZLIB_VERSION"
  for f in adler32 compress crc32 deflate gzclose gzlib gzread gzwrite \
           infback inffast inflate inftrees trees uncompr zutil; do
    "$CC" -O2 -w -DHAVE_UNISTD_H -D_LARGEFILE64_SOURCE=1 -c "$f.c" -o "$f.o"
  done
  rm -f "$out/lib/libz.a"
  zig ar rcs "$out/lib/libz.a" ./*.o
  cp zlib.h zconf.h "$out/include/"
)
echo "built libz.a (zlib $ZLIB_VERSION)"

# OpenSSL: its Configure takes the target by name and honours CC/AR/
# RANLIB, so it cross-builds as it is. Libraries only — no apps, tests,
# docs or engines — which is what the lotus runtime links.
openssl_tgz=$(fetch "https://github.com/openssl/openssl/releases/download/openssl-$OPENSSL_VERSION/openssl-$OPENSSL_VERSION.tar.gz" "$OPENSSL_SHA256")
tar xzf "$openssl_tgz" -C "$work"
echo "building OpenSSL $OPENSSL_VERSION for $triple (a minute or two)"
(
  cd "$work/openssl-$OPENSSL_VERSION"
  ./Configure "$openssl_target" no-shared no-tests no-apps no-docs no-engine \
    --prefix="$out" --libdir=lib > "$work/openssl-configure.log" 2>&1 \
    || { cat "$work/openssl-configure.log" >&2; exit 1; }
  make -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)" build_libs \
    > "$work/openssl-build.log" 2>&1 || { tail -40 "$work/openssl-build.log" >&2; exit 1; }
  make install_dev > "$work/openssl-install.log" 2>&1 \
    || { tail -20 "$work/openssl-install.log" >&2; exit 1; }
)
echo "built libssl.a + libcrypto.a (OpenSSL $OPENSSL_VERSION)"

# The tree-sitter shim, from a checkout that has the target's Rust std.
repo=$(cd "$(dirname "$0")/.." && pwd)
shim=skipped
if [ -f "$repo/crates/hale-ts-shim/Cargo.toml" ] && rustup target list --installed 2>/dev/null | grep -qx "$triple"; then
  echo "building libhale_ts_shim.a for $triple"
  env_cc="CC_$(echo "$triple" | tr '-' '_')"
  env_ar="AR_$(echo "$triple" | tr '-' '_')"
  (cd "$repo" && env "$env_cc=$out/bin/cc" "$env_ar=zig ar" \
     cargo build --release -p hale-ts-shim --target "$triple" >/dev/null)
  cp "$repo/target/$triple/release/libhale_ts_shim.a" "$out/lib/"
  shim=built
else
  echo "skipping libhale_ts_shim.a: not in a checkout, or \`rustup target add $triple\` not done" >&2
  echo "(a program that never calls std::ts links without it)" >&2
fi

{
  echo "target   $triple"
  echo "glibc    $glibc"
  echo "zig      $(zig version)"
  echo "openssl  $OPENSSL_VERSION"
  echo "zlib     $ZLIB_VERSION"
  echo "ts-shim  $shim"
} > "$out/VERSIONS"

echo "sysroot for $triple at $out:"
cat "$out/VERSIONS"
ls "$out/lib"
