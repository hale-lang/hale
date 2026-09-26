# One image with voice's three programs: the api, the brain and the node.
# compose.yaml picks which one each service runs.
#
# The build stage uses hale's released Linux toolchain rather than building
# the compiler: `hale` statically links LLVM and needs only clang, libc and
# OpenSSL headers beside it (see hale's own Dockerfile).

ARG HALE_VERSION=v0.21.0

# The admin UI, built once and served by the api from its own origin.
FROM node:22-bookworm-slim AS ui
WORKDIR /ui
COPY ui/package.json ui/package-lock.json ui/tsconfig.base.json ./
COPY ui/packages ./packages
COPY ui/apps ./apps
COPY spec /spec
RUN npm ci && npm run build

FROM ubuntu:24.04 AS build
ARG HALE_VERSION
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl clang-18 libc6-dev libssl-dev \
      libffi8 libtinfo6 libzstd1 zlib1g-dev libstdc++6 \
    && ln -s /usr/bin/clang-18 /usr/bin/clang \
    && rm -rf /var/lib/apt/lists/*
RUN set -eu; \
    case "$TARGETARCH" in \
      arm64) arch=aarch64 ;; \
      amd64) arch=x86_64 ;; \
      *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    name="hale-${HALE_VERSION}-${arch}-unknown-linux-gnu.tar.gz"; \
    base="https://github.com/hale-lang/hale/releases/download/${HALE_VERSION}"; \
    cd /tmp; \
    curl -fsSLO "$base/$name"; \
    curl -fsSLO "$base/$name.sha256"; \
    sha256sum -c "$name.sha256"; \
    tar xzf "$name" -C /usr/local/bin ./hale ./libhale_ts_shim.a; \
    rm -f "$name" "$name.sha256"; \
    hale --version

WORKDIR /src
COPY api api
COPY brain brain
COPY node node
RUN hale test api && hale test node \
    && hale build api && hale build brain && hale build node

FROM ubuntu:24.04
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl libssl3 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/api/api /usr/local/bin/voice-api
COPY --from=build /src/brain/brain /usr/local/bin/voice-brain
COPY --from=build /src/node/node /usr/local/bin/voice-node
COPY api/canned /usr/share/voice/canned
COPY --from=ui /ui/apps/admin/dist /usr/share/voice/ui
