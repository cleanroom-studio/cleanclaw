# --- Stage 1: Build the Rust workspace ---
# The Go pipeline used a two-stage build (node:22-alpine for the
# web UI, then a Go builder). The Rust pipeline collapses both
# into a single builder stage — the SSR frontend lives in
# `cleanclaw-web`, the backend in `cleanclaw-gateway` / `cleanclaw-api`,
# and the CLI glues them together. One builder, one binary.
#
# Multi-arch: this image builds for `linux/amd64` AND `linux/arm64`
# (Apple Silicon, AWS Graviton, RPi 4/5). Docker Hub publishes a
# multi-arch manifest for `rust:1.83-bookworm` and
# `debian:bookworm-slim`, so the FROM lines below resolve to the
# right arch for each buildx platform. CI uses
# `docker/setup-qemu-action@v3` to enable the cross-arch emulation
# under the hood. `TARGETARCH` is set by buildx and is exposed
# here for explicit cross-compilation logic (we don't currently
# need it because QEMU handles everything at the rustc level).
FROM --platform=$BUILDPLATFORM rust:1.83-bookworm AS builder

# Declare the buildx-provided args so downstream RUN steps can
# reference them. They're also useful when running the image
# interactively (`docker build --build-arg TARGETARCH=arm64`).
ARG TARGETARCH
ARG TARGETVARIANT

WORKDIR /src

# Pre-cache dependency layer: copy only manifests first so this
# layer is reused across builds when only source changes. We copy
# the full directory tree (not the source) so the manifest
# pre-build compiles every crate's transitive deps. The "real"
# sources come in the next COPY.
COPY Cargo.toml Cargo.lock ./
COPY crates/cleanclaw-core/Cargo.toml crates/cleanclaw-core/Cargo.toml
COPY crates/cleanclaw-config/Cargo.toml crates/cleanclaw-config/Cargo.toml
COPY crates/cleanclaw-store/Cargo.toml crates/cleanclaw-store/Cargo.toml
COPY crates/cleanclaw-bus/Cargo.toml crates/cleanclaw-bus/Cargo.toml
COPY crates/cleanclaw-taskqueue/Cargo.toml crates/cleanclaw-taskqueue/Cargo.toml
COPY crates/cleanclaw-webhook/Cargo.toml crates/cleanclaw-webhook/Cargo.toml
COPY crates/cleanclaw-agentcli/Cargo.toml crates/cleanclaw-agentcli/Cargo.toml
COPY crates/cleanclaw-auth/Cargo.toml crates/cleanclaw-auth/Cargo.toml
COPY crates/cleanclaw-provider/Cargo.toml crates/cleanclaw-provider/Cargo.toml
COPY crates/cleanclaw-toolprov/Cargo.toml crates/cleanclaw-toolprov/Cargo.toml
COPY crates/cleanclaw-workspace/Cargo.toml crates/cleanclaw-workspace/Cargo.toml
COPY crates/cleanclaw-skills/Cargo.toml crates/cleanclaw-skills/Cargo.toml
COPY crates/cleanclaw-session/Cargo.toml crates/cleanclaw-session/Cargo.toml
COPY crates/cleanclaw-scope/Cargo.toml crates/cleanclaw-scope/Cargo.toml
COPY crates/cleanclaw-usage/Cargo.toml crates/cleanclaw-usage/Cargo.toml
COPY crates/cleanclaw-privacy/Cargo.toml crates/cleanclaw-privacy/Cargo.toml
COPY crates/cleanclaw-policy/Cargo.toml crates/cleanclaw-policy/Cargo.toml
COPY crates/cleanclaw-sandbox/Cargo.toml crates/cleanclaw-sandbox/Cargo.toml
COPY crates/cleanclaw-mcp/Cargo.toml crates/cleanclaw-mcp/Cargo.toml
COPY crates/cleanclaw-plugin/Cargo.toml crates/cleanclaw-plugin/Cargo.toml
COPY crates/cleanclaw-cron/Cargo.toml crates/cleanclaw-cron/Cargo.toml
COPY crates/cleanclaw-channels/Cargo.toml crates/cleanclaw-channels/Cargo.toml
COPY crates/cleanclaw-agent/Cargo.toml crates/cleanclaw-agent/Cargo.toml
COPY crates/cleanclaw-api/Cargo.toml crates/cleanclaw-api/Cargo.toml
COPY crates/cleanclaw-setup/Cargo.toml crates/cleanclaw-setup/Cargo.toml
COPY crates/cleanclaw-gateway/Cargo.toml crates/cleanclaw-gateway/Cargo.toml
COPY crates/cleanclaw-daemon/Cargo.toml crates/cleanclaw-daemon/Cargo.toml
COPY crates/cleanclaw-web/Cargo.toml crates/cleanclaw-web/Cargo.toml
COPY crates/cleanclaw-embed/Cargo.toml crates/cleanclaw-embed/Cargo.toml
COPY crates/cleanclaw-plugin-runtime/Cargo.toml crates/cleanclaw-plugin-runtime/Cargo.toml
COPY crates/cleanclaw-plugins/plugin-demo/Cargo.toml crates/cleanclaw-plugins/plugin-demo/Cargo.toml
COPY crates/cleanclaw-plugins/mem0/Cargo.toml crates/cleanclaw-plugins/mem0/Cargo.toml
COPY crates/cleanclaw-plugins/post-turn-echo/Cargo.toml crates/cleanclaw-plugins/post-turn-echo/Cargo.toml
COPY crates/cleanclaw-plugins/openclaw-demo/Cargo.toml crates/cleanclaw-plugins/openclaw-demo/Cargo.toml

# Now copy the actual sources and build. The workspace is built
# in release mode with LTO; the resulting `cleanclaw` binary at
# target/${TARGETARCH}/release/cleanclaw is the same shape for
# every arch (the `uname -m` of the build host is what
# `cargo build` defaults to; with buildx + QEMU this is the
# emulated arch).
#
# We `--locked` so the build fails fast on a Cargo.lock drift
# instead of silently updating it.
RUN mkdir -p crates && \
    for d in crates/*/; do mkdir -p "$d/src"; done
COPY . .
RUN echo "==> building for arch: ${TARGETARCH:-$(uname -m)}" && \
    cargo build --release --locked --workspace

# Sanity: assert the resulting binary is the right arch. On
# linux/amd64, `file` should report x86-64; on linux/arm64,
# aarch64. The build fails immediately on a mismatch (e.g. a
# QEMU misconfig or a cached amd64 binary in a multi-arch
# pipeline).
RUN set -eux; \
    BIN=/src/target/release/cleanclaw; \
    file "$BIN"; \
    case "${TARGETARCH}" in \
        amd64) grep -q 'x86-64' <(file "$BIN") ;; \
        arm64) grep -q 'aarch64' <(file "$BIN") ;; \
        *) echo "unknown TARGETARCH: ${TARGETARCH}"; exit 1 ;; \
    esac

# --- Stage 2: Runtime image ---
# Mirrors the Go `FROM gcr.io/distroless/static-debian12:nonroot`
# stage: a minimal base with a non-root user, the gateway
# binary, and a /data volume mount. The multi-arch manifest for
# `debian:bookworm-slim` resolves to the same arch as the
# builder, so the binary is portable into the runtime stage
# without an intermediate manifest list.
FROM debian:bookworm-slim

ARG TARGETARCH

# CJK / emoji fonts + CA certs. The Go image inherited
# `gcr.io/distroless/static-debian12:nonroot` which doesn't carry
# fonts; we ship them in the runtime stage so the agent's web
# UI renders Unicode correctly and outbound HTTPS works.
#
# On arm64 hosts, `apt-get` reaches a different mirror pool than
# amd64. The `apt-get update` is the same on both; the package
# set we install is architecture-agnostic. The total layer size
# is comparable across arches.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates \
        fonts-noto-cjk \
        fonts-noto-color-emoji \
        tini \
 && rm -rf /var/lib/apt/lists/*

# Match the Go image's non-root user (uid 65532). The same
# numeric uid works on both arches — the kernel maps it to
# `/etc/passwd` regardless of the arch's passwd layout.
RUN groupadd -g 65532 cleanclaw \
 && useradd  -u 65532 -g 65532 -d /data -s /sbin/nologin cleanclaw \
 && mkdir -p /data/.cleanclaw \
 && chown -R cleanclaw:cleanclaw /data

USER cleanclaw:cleanclaw
WORKDIR /data

# Copy the binaries. The CLI is the entry point; the SSR
# frontend is statically linked in. Each binary is arch-specific
# (e.g. `cleanclaw` on arm64 is an aarch64 ELF); the multi-arch
# manifest pins the right copy per buildx platform.
COPY --from=builder --chown=cleanclaw:cleanclaw /src/target/release/cleanclaw /usr/local/bin/cleanclaw
COPY --from=builder --chown=cleanclaw:cleanclaw /src/target/release/cleanclaw-plugin-demo /usr/local/bin/cleanclaw-plugin-demo
COPY --from=builder --chown=cleanclaw:cleanclaw /src/target/release/cleanclaw-mem0 /usr/local/bin/cleanclaw-mem0
COPY --from=builder --chown=cleanclaw:cleanclaw /src/target/release/cleanclaw-post-turn-echo /usr/local/bin/cleanclaw-post-turn-echo
COPY --from=builder --chown=cleanclaw:cleanclaw /src/target/release/cleanclaw-openclaw-demo /usr/local/bin/cleanclaw-openclaw-demo

ENV CLEANCLAW_HOME=/data/.cleanclaw \
    CLEANCLAW_PORT=18953 \
    RUST_LOG=info

EXPOSE 18953

# tini reaps zombies + forwards signals, just like the Go image.
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["cleanclaw", "daemon", "run"]
