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
      curl ca-certificates build-essential \
      llvm-18-dev libpolly-18-dev libzstd-dev \
      clang-18 libclang-18-dev zlib1g-dev \
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
# libtinfo, libzstd, zlib, libstdc++.
RUN apt-get update && apt-get install -y --no-install-recommends \
      clang-18 lld-18 libc6-dev libssl-dev \
      libffi8 libtinfo6 libzstd1 zlib1g libstdc++6 \
    && ln -s /usr/bin/clang-18 /usr/bin/clang \
    && rm -rf /var/lib/apt/lists/*

# Both on PATH so std::env::current_exe finds the sibling .a.
COPY --from=builder /src/target/release/hale              /usr/local/bin/hale
COPY --from=builder /src/target/release/libhale_ts_shim.a /usr/local/bin/libhale_ts_shim.a

# Fail the build if a trivial program can't compile+link+run in the final
# image. Uses the canonical locus form (println is a lifecycle builtin, not
# a bare free fn) so this exercises the real codegen + clang link path. Then
# build the same program for wasm32 to prove the lld-18/wasm-ld path works.
RUN printf 'locus H {\n  birth() { println("ok"); }\n}\nfn main() { H { }; }\n' > /tmp/t.hl \
    && hale build /tmp/t.hl && /tmp/t \
    && hale build /tmp/t.hl --target wasm32 --wrap-main && test -f /tmp/t.wasm \
    && rm -f /tmp/t /tmp/t.hl /tmp/t.wasm /tmp/t.mjs

ENTRYPOINT ["hale"]
