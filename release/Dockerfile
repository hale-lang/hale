# Reproducible Linux RELEASE build for the `hale` compiler.
#
# Goal: produce a `hale-<triple>.tar.gz` whose binary launches and
# compiles programs on a machine that DOES NOT have LLVM 18 installed.
#
# How self-containment is achieved (LLVM): llvm-sys 180.0.0 defaults to
# `force-static` (its build.rs: force_static = !(prefer_static ||
# prefer_dynamic || force_dynamic)), so as long as the static LLVM `.a`
# component libs are present (Ubuntu's `llvm-18-dev` ships all 213 of
# them under /usr/lib/llvm-18/lib), the compiler links libLLVM
# STATICALLY. The shipped `hale` therefore has NO `libLLVM.so.18` in its
# `ldd` — verified in the `smoketest` stage below. No env knob or extra
# feature flag is needed; setting LLVM_SYS_180_PREFIX is enough.
#
# What is NOT (and cannot be) bundled — the end user STILL needs these to
# *use* the tarball, because `hale build`/`hale run` shell out at compile
# time (this is inherent to the codegen design, not a packaging gap):
#   - `clang` on PATH  — codegen recompiles the embedded C runtime and
#     links every build via `Command::new("clang")` (codegen.rs:1606).
#   - OpenSSL dev       — the embedded TLS runtime #includes
#     <openssl/ssl.h> and codegen always links `-lssl -lcrypto`.
#   - libc dev + crt objects for the final link.
# The `smoketest` stage documents exactly this minimal runtime set.
#
# Base is ubuntu:24.04 (noble): the only common base that carries
# llvm-18 / clang-18 in stock apt (no apt.llvm.org PPA), matching the
# `ubuntu-latest` CI runner (tests.yml / release.yml).

# =============================================================
# Stage 1 — builder: full LLVM-18 dev stack + pinned Rust
# =============================================================
FROM ubuntu:24.04 AS builder

ARG RUST_VERSION=1.95.0
ARG TARGET=x86_64-unknown-linux-gnu

# Build dependencies:
#   llvm-18-dev libpolly-18-dev libzstd-dev clang-18 libclang-18-dev
#   zlib1g-dev   — the exact set CI installs (tests.yml:102 / release.yml:87)
#   lld-18       — LTO builds link with `-fuse-ld=lld` (codegen.rs:1619)
#   libssl-dev   — links -lssl -lcrypto into every native build
#   pkg-config git curl build-essential ca-certificates — toolchain glue
RUN apt-get update && apt-get install -y --no-install-recommends \
      llvm-18-dev libpolly-18-dev libzstd-dev \
      clang-18 libclang-18-dev zlib1g-dev lld-18 \
      libssl-dev pkg-config git curl build-essential ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Stock Ubuntu rustc lags the workspace (needs 1.95+); install a PINNED
# stable toolchain via rustup for reproducibility.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain "${RUST_VERSION}"
ENV PATH="/root/.cargo/bin:${PATH}"

# The one env var llvm-sys needs to find LLVM 18.
ENV LLVM_SYS_180_PREFIX=/usr/lib/llvm-18

WORKDIR /src
COPY . .

# Plain --workspace (NOT `--bin hale`): a `--bin` filter skips the
# hale-ts-shim staticlib target, so `libhale_ts_shim.a` never lands in
# target/release/. That staticlib MUST ship next to `hale` or every
# user-program link fails `undefined reference to lotus_ts_*` (codegen
# path-probes `$(dirname current_exe)/libhale_ts_shim.a`). This mirrors CI.
RUN cargo build --release --workspace

# ---- assemble the release tarball (shape mirrors release.yml) ----
# release.yml ships {hale, README, LICENSE} + a sibling .sha256. We ADD
# `libhale_ts_shim.a`: without it the shipped binary can build trivial
# programs but fails to link anything that touches std::io (the shim
# resolves lotus_ts_* / __StdSourceWalk). A hale tarball without it is
# effectively non-functional for real programs.
RUN set -eux; \
    mkdir -p /out /stage; \
    cp target/release/hale                 /stage/; \
    cp target/release/libhale_ts_shim.a    /stage/; \
    cp README.md                           /stage/ 2>/dev/null || true; \
    cp LICENSE                             /stage/ 2>/dev/null || true; \
    tarball="hale-${TARGET}.tar.gz"; \
    tar -czf "/out/${tarball}" -C /stage .; \
    ( cd /out && sha256sum "${tarball}" | tee "${tarball}.sha256" ); \
    echo "=== ldd of shipped hale (expect NO libLLVM) ==="; \
    ldd /stage/hale; \
    echo "=== binary size ==="; \
    ls -lh /stage/hale; \
    ls -l /out

# =============================================================
# Stage 2 — package: minimal carrier for the built artifacts
# =============================================================
# This is the docker-compose `build` target. It carries ONLY /out (the
# tarball + .sha256). Its default command copies /out to the
# host-bind-mounted /dist — so `docker compose run --rm build` drops the
# tarball on the host. (ubuntu base, not scratch, only because we need a
# shell for the copy; this image is transient and never shipped.)
FROM ubuntu:24.04 AS package
COPY --from=builder /out/ /out/
CMD ["sh", "-c", "mkdir -p /dist && cp -v /out/* /dist/ && echo '--- dist ---' && ls -l /dist"]

# =============================================================
# Stage 3 — smoketest: CLEAN image with NO llvm-18-dev
# =============================================================
# ACCEPTANCE TEST. A fresh ubuntu:24.04 that installs ONLY the inherent
# runtime deps (clang + openssl + certs + libc dev) and NOT `llvm-18-dev`.
# It extracts the tarball and runs `hale run hello.hl`, proving the binary
# launches AND compiles+runs a program WITHOUT the LLVM 18 dev stack.
#
# Note: `clang-18` transitively pulls `libLLVM.so.18` as ITS OWN runtime
# dependency (clang is an LLVM tool). That is expected and unavoidable —
# the point is proven by `ldd hale` showing our binary does not link it,
# so `hale` needs no separate LLVM install. The user needs clang anyway.
FROM ubuntu:24.04 AS smoketest
ARG TARGET=x86_64-unknown-linux-gnu

RUN apt-get update && apt-get install -y --no-install-recommends \
      clang-18 libssl-dev libc6-dev ca-certificates \
    && ln -s /usr/bin/clang-18 /usr/bin/clang \
    && rm -rf /var/lib/apt/lists/*

# Prove the LLVM DEV stack is genuinely absent from this image. Note that
# `clang-18` drags in `libLLVM.so.18` (the runtime .so) as its own
# dependency, so /usr/lib/llvm-18/lib EXISTS and holds that shared lib —
# that's fine and expected. What must be absent is the DEV stack the build
# needs: the `llvm-18-dev` package, `llvm-config-18`, and the static
# component `.a` libs (all dev-only). If those are gone, this box could
# not itself build `hale`; that `hale` runs here anyway is the point.
RUN set -eux; \
    ! dpkg -s llvm-18-dev >/dev/null 2>&1 || { echo "FAIL: llvm-18-dev installed"; exit 1; }; \
    ! command -v llvm-config-18 >/dev/null 2>&1 || { echo "FAIL: llvm-config-18 present"; exit 1; }; \
    test -z "$(ls /usr/lib/llvm-18/lib/libLLVM*.a 2>/dev/null)" || { echo "FAIL: static LLVM libs present"; exit 1; }; \
    echo "confirmed: no llvm-18-dev / no static LLVM in smoke image (only clang's runtime libLLVM.so remains)"

COPY --from=builder /out/hale-${TARGET}.tar.gz /tmp/hale.tar.gz
RUN set -eux; \
    mkdir -p /opt/hale; \
    tar -xzf /tmp/hale.tar.gz -C /opt/hale; \
    rm /tmp/hale.tar.gz
ENV PATH="/opt/hale:${PATH}"

# Build-time acceptance assertion (fails the image build if broken):
#   1. hale's own ldd has NO libLLVM  -> self-contained re: LLVM
#   2. `hale run` compiles + links + executes a real program with no
#      LLVM dev stack installed.
RUN set -eux; \
    echo "=== ldd of extracted hale (must show NO libLLVM) ==="; \
    ldd /opt/hale/hale; \
    ldd /opt/hale/hale | grep -qi 'llvm' && { echo "FAIL: hale links libLLVM"; exit 1; } || echo "OK: hale does not link libLLVM"; \
    printf 'locus H {\n  birth() { println("hello from a no-LLVM box"); }\n}\nfn main() { H { }; }\n' > /tmp/hello.hl; \
    echo "=== hale run hello.hl ==="; \
    hale run /tmp/hello.hl

# Default command re-runs the program so `docker compose run --rm
# smoketest` prints the live output.
CMD ["sh", "-c", "printf 'locus H {\\n  birth() { println(\"hello from a no-LLVM box\"); }\\n}\\nfn main() { H { }; }\\n' > /tmp/hello.hl && ldd /opt/hale/hale && echo '--- run ---' && hale run /tmp/hello.hl"]
