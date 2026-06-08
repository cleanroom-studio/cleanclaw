#!/usr/bin/env bash
# Lint the Dockerfile for hardcoded arch assumptions + missing
# multi-arch plumbing. Catches regressions where a maintainer
# adds `RUN wget http://...linux-amd64.tar.gz` or hardcodes
# `/usr/lib/x86_64-linux-gnu/` paths.
#
# Run: bash scripts/tests/dockerfile_lint_test.sh
set -euo pipefail

FAIL=0
PASS=0

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
DOCKERFILE="${ROOT_DIR}/Dockerfile"
DOCKERIGNORE="${ROOT_DIR}/.dockerignore"

# Sanity: the file must exist.
if [ ! -f "$DOCKERFILE" ]; then
    printf "  \033[31m✗\033[0m Dockerfile missing at %s\n" "$DOCKERFILE" >&2
    exit 1
fi

pass() {
    PASS=$((PASS + 1))
    printf "  \033[32m✓\033[0m %s\n" "$1"
}
fail() {
    FAIL=$((FAIL + 1))
    printf "  \033[31m✗\033[0m %s\n" "$1"
    if [ -n "${2:-}" ]; then
        printf "    %s\n" "$2"
    fi
}

# 1. No hardcoded `linux/amd64` strings in the build stages.
#    Comments and docs are allowed (they often explain what
#    multi-arch means), so we strip `^#` lines before grepping.
if grep -v '^#' "$DOCKERFILE" | grep -n 'linux/amd64' >/dev/null 2>&1; then
    fail "Dockerfile contains hardcoded 'linux/amd64' reference"
    grep -n 'linux/amd64' "$DOCKERFILE" | sed 's/^/    /'
else
    pass "no hardcoded 'linux/amd64' references in Dockerfile (comments OK)"
fi

# 2. The runtime base image must be the multi-arch-friendly
# `debian:bookworm-slim` (or the equivalent `-slim` variant
# which has multi-arch manifests on Docker Hub). A bare
# `debian:bookworm` without -slim is also multi-arch but pulls
# a lot of unnecessary packages.
if grep -q '^FROM debian:bookworm-slim' "$DOCKERFILE"; then
    pass "runtime base is debian:bookworm-slim (multi-arch manifest)"
else
    fail "runtime base is not debian:bookworm-slim" \
         "use a multi-arch-manifest image so amd64 + arm64 builds both resolve"
fi

# 3. The builder base image must be the multi-arch
# `rust:1.83-bookworm` (which has both amd64 + arm64 manifests
# on Docker Hub).
if grep -q '^FROM --platform=\$BUILDPLATFORM rust:1.83-bookworm' "$DOCKERFILE"; then
    pass "builder base is --platform=\$BUILDPLATFORM rust:1.83-bookworm (multi-arch)"
elif grep -q '^FROM rust:1.83-bookworm' "$DOCKERFILE"; then
    pass "builder base is rust:1.83-bookworm (multi-arch; add --platform=\$BUILDPLATFORM for QEMU)"
else
    fail "builder base is not rust:1.83-bookworm"
fi

# 4. `cargo build` must be `--locked` so a Cargo.lock drift
# fails the build instead of silently updating the lockfile.
if grep -q 'cargo build --release --locked' "$DOCKERFILE"; then
    pass "cargo build uses --locked"
else
    fail "cargo build is missing --locked" \
         "the build would silently update Cargo.lock on a mismatch"
fi

# 5. The COPY --from=builder must reference target/release/
# (the cargo build's default output dir) — never a hardcoded
# arch path.
if grep -E 'COPY --from=builder.*target/[^ ]*/release' "$DOCKERFILE" >/dev/null; then
    fail "COPY --from=builder references a hardcoded arch dir under target/" \
         "use target/release/ (the host's cargo output dir)"
else
    pass "COPY --from=builder uses the arch-agnostic target/release/"
fi

# 6. The .dockerignore should exclude _ref/ (huge Go reference
# repo) and target/ (build artifacts). Without these the
# build context balloons to >100MB.
if [ -f "$DOCKERIGNORE" ]; then
    if grep -qx '_ref/' "$DOCKERIGNORE"; then
        pass ".dockerignore excludes _ref/"
    else
        fail ".dockerignore does not exclude _ref/"
    fi
    if grep -qx 'target/' "$DOCKERIGNORE"; then
        pass ".dockerignore excludes target/"
    else
        fail ".dockerignore does not exclude target/"
    fi
else
    fail ".dockerignore is missing" \
         "create one to keep the build context small (see existing pattern)"
fi

# 7. The build should set RUSTFLAGS or rely on the default
# target. With QEMU emulation (buildx's default), the default
# target is the emulated arch, so no RUSTFLAGS is needed. But
# if cross-compilation is added later, the RUSTFLAGS must use
# a target triple, not a path. This test is informational.
if grep -q 'RUSTFLAGS=' "$DOCKERFILE"; then
    if grep -q 'RUSTFLAGS=.*target/' "$DOCKERFILE"; then
        fail "RUSTFLAGS appears to set a target/ path" \
             "set the rustc --target triple, not a cargo target dir"
    else
        pass "RUSTFLAGS is set without a hardcoded target path"
    fi
else
    pass "RUSTFLAGS is not set (QEMU handles the cross-arch target)"
fi

echo
echo "Passed: $PASS    Failed: $FAIL"
[ "$FAIL" -eq 0 ]
