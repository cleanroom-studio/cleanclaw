#!/usr/bin/env bash
# Builds a stamped release binary and (optionally) a Docker image.
#
# Usage:
#   scripts/release.sh v0.1.0                   # build + tarball
#   scripts/release.sh v0.1.0 --docker          # build + image (host arch)
#   scripts/release.sh v0.1.0 --docker --multi-arch   # build + linux/amd64+arm64
#   scripts/release.sh v0.1.0 --docker --push   # build + push (host arch)
#   scripts/release.sh v0.1.0 --docker --multi-arch --push
#
# Exit codes:
#   0   success
#   1   bad usage / missing binary / missing tool
#   2   docker buildx push failed
set -euo pipefail

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
    echo "Usage: $0 <version> [--docker] [--multi-arch] [--push]"
    echo "Example: $0 v0.1.0"
    echo "         $0 v0.1.0 --docker --multi-arch --push"
    exit 1
fi
shift

DOCKER=0
MULTI_ARCH=0
PUSH=0
for arg in "$@"; do
    case "$arg" in
        --docker)     DOCKER=1 ;;
        --multi-arch) MULTI_ARCH=1 ;;
        --push)       PUSH=1 ;;
        -h|--help)    sed -n '2,11p' "$0"; exit 0 ;;
        *)            echo "unknown arg: $arg" >&2; exit 1 ;;
    esac
done

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> release: $VERSION"
echo "    root:    $ROOT"
echo "    docker:  $DOCKER    multi-arch: $MULTI_ARCH    push: $PUSH"

# 1. Cargo release. The Makefile stamps the build identity via
#    CLEANCLAW_VERSION / CLEANCLAW_COMMIT / CLEANCLAW_DATE env
#    vars; cleanclaw-core reads them at compile time and
#    `cleanclaw version` prints them at runtime.
echo "==> cargo build --release --workspace"
make release-local

BIN="target/release/cleanclaw"
if [ ! -f "$BIN" ]; then
    echo "error: $BIN not found"
    exit 1
fi

# 2. Tarball. We stamp the arch on the filename so the install script can
#    pick the right one (aarch64 for arm64 hosts, x86_64 for
#    amd64).
HOST_ARCH="$(uname -m)"
case "$HOST_ARCH" in
    x86_64|amd64)   TARBALL_ARCH="x86_64" ;;
    aarch64|arm64)  TARBALL_ARCH="aarch64" ;;
    *)              TARBALL_ARCH="$HOST_ARCH" ;;
esac
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
TARBALL="cleanclaw_${VERSION}_${OS}_${TARBALL_ARCH}.tar.gz"
echo "==> tarball: $TARBALL (host arch: $TARBALL_ARCH)"
tar -czf "$TARBALL" -C target/release cleanclaw \
    cleanclaw-plugin-demo cleanclaw-mem0 \
    cleanclaw-post-turn-echo cleanclaw-openclaw-demo
sha256sum "$TARBALL" > "$TARBALL.sha256"
echo "    $(cat $TARBALL.sha256)"

# 3. Docker image. The Go pipeline did `docker buildx build
#    --platform linux/amd64,linux/arm64`; the Rust pipeline does
#    the same via `--multi-arch` (and uses QEMU emulation in CI
#    for the cross-arch step).
if [ "$DOCKER" -eq 1 ]; then
    IMAGE="cleanclaw/cleanclaw:${VERSION}"
    echo "==> docker image: $IMAGE"
    # If buildx isn't set up, fall back to legacy `docker build`
    # (single-arch only). The setup-buildx-action in CI handles
    # the multi-arch path; locally, `docker buildx create --use`
    # is the equivalent one-shot.
    if [ "$MULTI_ARCH" -eq 1 ]; then
        if ! command -v docker >/dev/null 2>&1; then
            echo "error: docker not installed" >&2
            exit 1
        fi
        if ! docker buildx version >/dev/null 2>&1; then
            echo "error: docker buildx not available" >&2
            echo "       install: https://docs.docker.com/go/buildx/" >&2
            exit 1
        fi
        PLATFORMS="linux/amd64,linux/arm64"
        echo "    platforms: $PLATFORMS"
        if [ "$PUSH" -eq 1 ]; then
            docker buildx build --push --platform "$PLATFORMS" \
                --tag "$IMAGE" --tag "cleanclaw/cleanclaw:latest" .
        else
            docker buildx build --load --platform "$PLATFORMS" \
                --tag "$IMAGE" .
        fi
    else
        if [ "$PUSH" -eq 1 ]; then
            docker buildx build --push -t "$IMAGE" .
        else
            docker build -t "$IMAGE" .
        fi
    fi
fi

echo
echo "Release done."
echo "  binary:  $BIN"
echo "  tarball: $TARBALL"
echo "  sha256:  $TARBALL.sha256"
[ "$DOCKER" -eq 1 ] && echo "  image:   $IMAGE"
