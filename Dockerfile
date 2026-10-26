# Public Hale toolchain image: compile `.hl` programs in any repo's CI.
#
# Two stages on purpose. The compiler statically links LLVM 18 (see
# `ldd hale` — no libLLVM.so.18), so the heavy `llvm-18-dev` libs are a
# *build*-time dependency only. The runtime stage ships just what's
# needed to invoke `hale build` on a foreign program:
#   - the `hale` binary
#   - `libhale_ts_shim.a` SITTING NEXT TO IT — codegen path-probes
#     `$(dirname current_exe)/libhale_ts_shim.a` (crates/hale-codegen/
#     src/codegen.rs); without it every link fails `undefined reference
#     to lotus_ts_*`.
#   - `clang` (bare, on PATH) — codegen shells out to `clang` to compile
#     the embedded C runtime and link the object file. The stdlib `.hl`
#     and `lotus_*.c` sources are include_str!'d into the binary, so they
#     aren't shipped; but clang needs libc dev headers + crt objects.
#
# Base is ubuntu:24.04 (noble) for BOTH stages: it's the only common base
# that carries llvm-18/clang-18 in apt, and it matches the `ubuntu-latest`
# runner the CI builds + tests on (tests.yml / release.yml). Debian
# bookworm ships LLVM 14-16 only, so it can't build or run this.

# ---- builder: full LLVM-18 dev to compile the compiler ----
FROM ubuntu:24.04 AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
      curl ca-certificates build-essential git \
      llvm-18-dev libpolly-18-dev libzstd-dev \
      clang-18 libclang-18-dev zlib1g-dev \
      libssl-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Stock Ubuntu rustc can lag the workspace; install the current stable
# toolchain via rustup instead.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain stable
ENV PATH="/root/.cargo/bin:${PATH}"
ENV LLVM_SYS_180_PREFIX=/usr/lib/llvm-18

WORKDIR /src
COPY . .
# Plain --workspace (no --bin filter): a `--bin hale` filter restricts the
# build to that one bin target and SKIPS the hale-ts-shim staticlib, so
# libhale_ts_shim.a never lands in target/release/. This mirrors CI.
RUN cargo build --release --workspace

# ---- runtime: NO llvm dev libs; LLVM is statically linked into hale ----
FROM ubuntu:24.04 AS runtime

# clang-18 compiles the embedded C runtime + links programs; libc6-dev
# supplies the headers/crt objects clang needs at link time. libssl-dev is
# required by EVERY native build: the embedded TLS runtime (lotus_tls.c)
# #includes <openssl/ssl.h> and codegen always links -lssl -lcrypto
# (codegen.rs:726); libssl-dev gives both the headers and the .so symlinks.
# lld-18 supplies `wasm-ld-18`, which codegen invokes to link `--target
# wasm32` builds (codegen.rs resolve_tool finds the `-18` suffix, so no bare
# symlink is needed). The rest are `hale`'s own dynamic deps (ldd): libffi,
# libtinfo, libzstd, zlib, libstdc++, plus git, curl, ssh, openssl and the
# docker + compose CLI so a habitat's body and face run inside compose.
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git openssh-client openssl \
      docker.io docker-compose-v2 \
      clang-18 lld-18 libc6-dev libssl-dev \
      libffi8 libtinfo6 libzstd1 zlib1g-dev libstdc++6 \
    && ln -s /usr/bin/clang-18 /usr/bin/clang \
    && ln -sf /usr/bin/clang-18 /usr/bin/cc \
    && mkdir -p /private/var/folders \
    && ln -s /private/var/folders /var/folders \
    && ln -s /tmp /private/tmp \
    && git config --system safe.directory '*' \
    && rm -rf /var/lib/apt/lists/*

# Both on PATH so std::env::current_exe finds the sibling .a.
COPY --from=builder /src/target/release/hale              /usr/local/bin/hale
COPY --from=builder /src/target/release/libhale_ts_shim.a /usr/local/bin/libhale_ts_shim.a

RUN printf '#!/bin/sh\nuid="$(id -u)"\nif [ "$uid" != "0" ]; then\n  grep -v "^[^:]*:[^:]*:$uid:" /etc/passwd > /tmp/.passwd.$$ 2>/dev/null && cat /tmp/.passwd.$$ > /etc/passwd 2>/dev/null; rm -f /tmp/.passwd.$$\n  printf "%%s:x:%%s:%%s::%%s:/bin/sh\\n" "${USER:-hale}" "$uid" "$(id -g)" "${HOME:-/tmp}" >> /etc/passwd 2>/dev/null || true\nfi\nexec hale "$@"\n' > /usr/local/bin/hale-entrypoint \
    && chmod +x /usr/local/bin/hale-entrypoint

# Fail the build if a trivial program can't compile+link+run in the final
# image. Uses the canonical locus form (println is a lifecycle builtin, not
# a bare free fn) so this exercises the real codegen + clang link path. Then
# build the same program for wasm32 to prove the lld-18/wasm-ld path works,
# and pre-build the embedded DNA host and head seeds into /var/cache/hale so
# `hale dna dev` and `hale dna head` inside a habitat container start without
# compiling on first boot.
ENV XDG_CACHE_HOME=/var/cache
RUN printf 'locus H {\n  birth() { println("ok"); }\n}\nfn main() { H { }; }\n' > /tmp/t.hl \
    && hale build /tmp/t.hl && /tmp/t \
    && hale build /tmp/t.hl --target wasm32 --wrap-main && test -f /tmp/t.wasm \
    && rm -f /tmp/t /tmp/t.hl /tmp/t.wasm /tmp/t.mjs \
    && hale iris --build-only >/dev/null \
    && cache="$(hale iris --where)" \
    && rm -rf /var/cache/hale/runtime \
    && find "$cache" -type f -executable -delete \
    && hale build --target-cpu baseline "$cache/iris/consumer/fuse-hl" \
    && pids="" \
    && for seed in dna/host dna/host/verb dna/api/practice_review dna/api/project_service dna/face/standard dna/oidc/serve dna/reflexes; do \
         hale build --target-cpu baseline "$cache/$seed" & pids="$pids $!"; \
       done \
    && for p in $pids; do wait "$p"; done \
    && chmod -R a+rwX /var/cache/hale \
    && chmod a+rw /etc/passwd \
    && printf '#!/bin/sh\nuid="$(id -u)"\nif [ "$uid" != "0" ]; then\n  grep -v "^[^:]*:[^:]*:$uid:" /etc/passwd > /tmp/.passwd.$$ 2>/dev/null && cat /tmp/.passwd.$$ > /etc/passwd 2>/dev/null; rm -f /tmp/.passwd.$$\n  printf "%%s:x:%%s:%%s::%%s:/bin/sh\\n" "${USER:-hale}" "$uid" "$(id -g)" "${HOME:-/tmp}" >> /etc/passwd 2>/dev/null || true\nfi\nif [ -z "$DOCKER_API_VERSION" ] && [ -S /var/run/docker.sock ]; then\n  api="$(curl --unix-socket /var/run/docker.sock -s http://localhost/version 2>/dev/null | sed -n '\''s/.*"ApiVersion":"\([^"]*\)".*/\\1/p'\'' | head -n 1)"\n  [ -n "$api" ] && export DOCKER_API_VERSION="$api"\nfi\nif [ "$1" = "hale" ]; then\n  shift\nfi\nexec hale "$@"\n' > /usr/local/bin/hale-entrypoint \
    && chmod +x /usr/local/bin/hale-entrypoint

ENTRYPOINT ["/usr/local/bin/hale-entrypoint"]
